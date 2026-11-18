//! Brigadier-compatible declarative command tree, typed arguments, dynamic suggestions,
//! sender conditions, and `DeclareCommandsPacket` graph serialization.
//!
//! ```
//! use minestom_command::{Argument, Command, CommandManager, CommandSender};
//! use minestom_core::GameMode;
//!
//! let manager = CommandManager::new();
//! let mode_arg = Argument::game_mode("mode");
//! let mode_ref = mode_arg.clone();
//!
//! let gm = Command::new("gamemode")
//!     .with_aliases(["gm"])
//!     .add_syntax([mode_arg.untyped()], move |sender, ctx| {
//!         let mode = *ctx.get(&mode_ref).expect("validated GameMode");
//!         sender.set_game_mode(mode);
//!         sender.send_message(format!("Set game mode to {mode:?}"));
//!     });
//! manager.register(gm);
//!
//! let mut console = CommandSender::console();
//! manager.execute(&mut console, "gm creative").unwrap();
//! assert_eq!(console.messages().len(), 1);
//! ```

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use bytes::{BufMut, BytesMut};
use minestom_core::{BlockVec, GameMode, PlayerId, Point, Pos, Vec3};
use minestom_protocol::chat_command::{
    CommandGraphNode, DeclareCommandsPacket, TabCompleteMatch, node_flags, parser_ids,
};
use minestom_protocol::write_var_int;
use minestom_text::Component;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use uuid::Uuid;

/// Errors returned when parsing or executing commands.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// Command name was not registered.
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    /// Argument failed syntax validation.
    #[error("invalid command syntax at `{input}`: {reason}")]
    SyntaxError {
        /// Offending token or substring.
        input: String,
        /// Human-readable explanation.
        reason: String,
    },
    /// Sender failed the command's execution condition.
    #[error("insufficient permissions to execute command")]
    ConditionFailed,
}

/// Metadata describing an online player executing a command.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSenderInfo {
    /// Generational [`PlayerId`] handle.
    pub player_id: PlayerId,
    /// Player UUID.
    pub uuid: Uuid,
    /// Player username.
    pub username: String,
    /// Current world position (used for relative `~` coordinate arguments).
    pub position: Pos,
    /// Current [`GameMode`].
    pub game_mode: GameMode,
    /// Numeric permission level (`0..=4`).
    pub permission_level: u8,
}

/// Deferred side-effect requested on a player or server during command execution.
#[derive(Debug, Clone, PartialEq)]
pub enum SenderEffect {
    /// Change the executing player's [`GameMode`].
    SetGameMode(GameMode),
    /// Teleport the executing player to a new [`Pos`].
    Teleport(Pos),
    /// Broadcast a chat [`Component`] to all online players.
    Broadcast(Component),
    /// Disconnect the executing player with a reason [`Component`].
    Kick(Component),
}

#[derive(Debug, Clone, PartialEq)]
enum SenderIdentity {
    Console,
    Player(PlayerSenderInfo),
}

/// Command invoker (`Console` or `Player`) collecting reply messages and side-effects.
#[derive(Debug, Clone)]
pub struct CommandSender {
    identity: SenderIdentity,
    messages: Vec<Component>,
    effects: Vec<SenderEffect>,
}

impl CommandSender {
    /// Creates a privileged console [`CommandSender`].
    #[must_use]
    pub const fn console() -> Self {
        Self {
            identity: SenderIdentity::Console,
            messages: Vec::new(),
            effects: Vec::new(),
        }
    }

    /// Creates a player [`CommandSender`] from [`PlayerSenderInfo`].
    #[must_use]
    pub const fn player(info: PlayerSenderInfo) -> Self {
        Self {
            identity: SenderIdentity::Player(info),
            messages: Vec::new(),
            effects: Vec::new(),
        }
    }

    /// Returns `true` if invoked from the server console.
    #[must_use]
    pub const fn is_console(&self) -> bool {
        matches!(self.identity, SenderIdentity::Console)
    }

    /// Returns `true` if invoked by a connected player.
    #[must_use]
    pub const fn is_player(&self) -> bool {
        matches!(self.identity, SenderIdentity::Player(_))
    }

    /// Returns player details if this sender is a player.
    #[must_use]
    pub const fn as_player(&self) -> Option<&PlayerSenderInfo> {
        match &self.identity {
            SenderIdentity::Console => None,
            SenderIdentity::Player(p) => Some(p),
        }
    }

    /// Returns the display name of the sender (`"CONSOLE"` or player username).
    #[must_use]
    pub fn name(&self) -> &str {
        match &self.identity {
            SenderIdentity::Console => "CONSOLE",
            SenderIdentity::Player(p) => &p.username,
        }
    }

    /// Returns `true` if the sender is console or has `permission_level >= level`.
    #[must_use]
    pub const fn has_permission_level(&self, level: u8) -> bool {
        match &self.identity {
            SenderIdentity::Console => true,
            SenderIdentity::Player(p) => p.permission_level >= level,
        }
    }

    /// Origin position used to resolve relative `~` coordinates (`Pos::ZERO` for console).
    #[must_use]
    pub const fn origin_position(&self) -> Pos {
        match &self.identity {
            SenderIdentity::Console => Pos::ZERO,
            SenderIdentity::Player(p) => p.position,
        }
    }

    /// Queues a chat reply [`Component`] to this sender.
    pub fn send_message(&mut self, message: impl Into<Component>) {
        self.messages.push(message.into());
    }

    /// Requests changing this player's [`GameMode`] (no-op on console aside from recording effect).
    pub fn set_game_mode(&mut self, mode: GameMode) {
        if let SenderIdentity::Player(p) = &mut self.identity {
            p.game_mode = mode;
        }
        self.effects.push(SenderEffect::SetGameMode(mode));
    }

    /// Requests teleporting this player to `pos`.
    pub fn teleport(&mut self, pos: Pos) {
        if let SenderIdentity::Player(p) = &mut self.identity {
            p.position = pos;
        }
        self.effects.push(SenderEffect::Teleport(pos));
    }

    /// Requests broadcasting `message` to all online players.
    pub fn broadcast(&mut self, message: impl Into<Component>) {
        self.effects.push(SenderEffect::Broadcast(message.into()));
    }

    /// Requests disconnecting this player with `reason`.
    pub fn kick(&mut self, reason: impl Into<Component>) {
        self.effects.push(SenderEffect::Kick(reason.into()));
    }

    /// Borrows all reply messages queued during execution.
    #[must_use]
    pub fn messages(&self) -> &[Component] {
        &self.messages
    }

    /// Drains all reply messages queued during execution.
    pub fn take_messages(&mut self) -> Vec<Component> {
        std::mem::take(&mut self.messages)
    }

    /// Drains all [`SenderEffect`]s queued during execution.
    pub fn take_effects(&mut self) -> Vec<SenderEffect> {
        std::mem::take(&mut self.effects)
    }
}

/// Predicate controlling whether a [`CommandSender`] can see and execute a command or syntax.
///
/// The second parameter is `None` when building the `DeclareCommandsPacket` tree, and
/// `Some(command_string)` during execution or tab-completion.
pub type CommandCondition = Arc<dyn Fn(&CommandSender, Option<&str>) -> bool + Send + Sync>;

/// Single auto-completion entry with an optional hover tooltip.
#[derive(Debug, Clone, PartialEq)]
pub struct SuggestionEntry {
    /// Suggested replacement text.
    pub value: String,
    /// Optional tooltip [`Component`].
    pub tooltip: Option<Component>,
}

impl SuggestionEntry {
    /// Creates a plain [`SuggestionEntry`] without a tooltip.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            tooltip: None,
        }
    }

    /// Creates a [`SuggestionEntry`] with a hover tooltip [`Component`].
    #[must_use]
    pub fn with_tooltip(value: impl Into<String>, tooltip: impl Into<Component>) -> Self {
        Self {
            value: value.into(),
            tooltip: Some(tooltip.into()),
        }
    }
}

/// Mutable suggestion accumulator passed to dynamic [`SuggestionCallback`]s.
#[derive(Debug, Clone, Default)]
pub struct Suggestion {
    input_prefix: String,
    entries: Vec<SuggestionEntry>,
}

impl Suggestion {
    /// Creates a new [`Suggestion`] context for the current partial argument token `input_prefix`.
    #[must_use]
    pub fn new(input_prefix: impl Into<String>) -> Self {
        Self {
            input_prefix: input_prefix.into(),
            entries: Vec::new(),
        }
    }

    /// Returns the partial token currently typed by the user at the cursor.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input_prefix
    }

    /// Appends a [`SuggestionEntry`].
    pub fn add_entry(&mut self, entry: SuggestionEntry) {
        self.entries.push(entry);
    }

    /// Convenience helper appending a plain string suggestion.
    pub fn add(&mut self, value: impl Into<String>) {
        self.entries.push(SuggestionEntry::new(value));
    }

    /// Borrows collected entries.
    #[must_use]
    pub fn entries(&self) -> &[SuggestionEntry] {
        &self.entries
    }
}

/// Dynamic server-side tab-completion callback (`Argument.setSuggestionCallback` in Minestom).
pub type SuggestionCallback =
    Arc<dyn Fn(&CommandSender, &CommandContext, &mut Suggestion) + Send + Sync>;

/// Parsed runtime value stored inside [`CommandContext`].
#[derive(Debug, Clone, PartialEq)]
pub enum ArgumentValue {
    /// Parsed `String` (literal, word, quoted string, greedy string, or enum choice).
    Text(String),
    /// Parsed `bool`.
    Bool(bool),
    /// Parsed `i32`.
    Int(i32),
    /// Parsed `i64`.
    Long(i64),
    /// Parsed `f32`.
    Float(f32),
    /// Parsed `f64`.
    Double(f64),
    /// Parsed [`GameMode`].
    GameMode(GameMode),
    /// Parsed [`BlockVec`] block position.
    BlockPos(BlockVec),
    /// Parsed [`Vec3`] floating-point position.
    Vec3(Vec3),
}

/// Trait implemented by Rust types extractable from [`CommandContext::get`].
pub trait FromArgumentValue: Sized {
    /// Attempts to borrow `Self` from an [`ArgumentValue`].
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self>;
}

impl FromArgumentValue for String {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl FromArgumentValue for bool {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::Bool(v) => Some(v),
            _ => None,
        }
    }
}

impl FromArgumentValue for i32 {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::Int(v) => Some(v),
            _ => None,
        }
    }
}

impl FromArgumentValue for i64 {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::Long(v) => Some(v),
            _ => None,
        }
    }
}

impl FromArgumentValue for f32 {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::Float(v) => Some(v),
            _ => None,
        }
    }
}

impl FromArgumentValue for f64 {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::Double(v) => Some(v),
            _ => None,
        }
    }
}

impl FromArgumentValue for GameMode {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::GameMode(v) => Some(v),
            _ => None,
        }
    }
}

impl FromArgumentValue for BlockVec {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::BlockPos(v) => Some(v),
            _ => None,
        }
    }
}

impl FromArgumentValue for Vec3 {
    fn from_arg_value(val: &ArgumentValue) -> Option<&Self> {
        match val {
            ArgumentValue::Vec3(v) => Some(v),
            _ => None,
        }
    }
}

/// Wire & parser specification of a Brigadier command argument.
#[derive(Clone)]
pub enum ArgumentKind {
    /// Exact keyword literal (`node_flags::LITERAL`).
    Literal,
    /// Single unquoted word (`brigadier:string`, mode `0`).
    Word,
    /// Single word or `"quoted phrase"` (`brigadier:string`, mode `1`).
    QuotableString,
    /// Consumes the remainder of the command line (`brigadier:string`, mode `2`).
    GreedyString,
    /// `true` or `false` (`brigadier:bool`).
    Boolean,
    /// Bounded or unbounded `i32` (`brigadier:integer`).
    Integer {
        /// Optional inclusive lower bound.
        min: Option<i32>,
        /// Optional inclusive upper bound.
        max: Option<i32>,
    },
    /// Bounded or unbounded `i64` (`brigadier:long`).
    Long {
        /// Optional inclusive lower bound.
        min: Option<i64>,
        /// Optional inclusive upper bound.
        max: Option<i64>,
    },
    /// Bounded or unbounded `f32` (`brigadier:float`).
    Float {
        /// Optional inclusive lower bound.
        min: Option<f32>,
        /// Optional inclusive upper bound.
        max: Option<f32>,
    },
    /// Bounded or unbounded `f64` (`brigadier:double`).
    Double {
        /// Optional inclusive lower bound.
        min: Option<f64>,
        /// Optional inclusive upper bound.
        max: Option<f64>,
    },
    /// Fixed set of string choices with automatic validation and tab-completion.
    Choices {
        /// Allowed values.
        choices: Vec<String>,
    },
    /// Minecraft [`GameMode`] (`minecraft:gamemode`).
    GameMode,
    /// 3-token integer block coordinate supporting `~` relative offsets (`minecraft:block_pos`).
    BlockPos,
    /// 3-token floating-point coordinate supporting `~` relative offsets (`minecraft:vec3`).
    Vec3,
}

impl std::fmt::Debug for ArgumentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Literal => write!(f, "Literal"),
            Self::Word => write!(f, "Word"),
            Self::QuotableString => write!(f, "QuotableString"),
            Self::GreedyString => write!(f, "GreedyString"),
            Self::Boolean => write!(f, "Boolean"),
            Self::Integer { min, max } => f
                .debug_struct("Integer")
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::Long { min, max } => f
                .debug_struct("Long")
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::Float { min, max } => f
                .debug_struct("Float")
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::Double { min, max } => f
                .debug_struct("Double")
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::Choices { choices } => {
                f.debug_struct("Choices").field("choices", choices).finish()
            }
            Self::GameMode => write!(f, "GameMode"),
            Self::BlockPos => write!(f, "BlockPos"),
            Self::Vec3 => write!(f, "Vec3"),
        }
    }
}

/// Type-erased argument descriptor stored inside [`CommandSyntax`].
#[derive(Clone)]
pub struct UntypedArgument {
    id: String,
    kind: ArgumentKind,
    suggestion_callback: Option<SuggestionCallback>,
}

impl std::fmt::Debug for UntypedArgument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UntypedArgument")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("has_suggestions", &self.suggestion_callback.is_some())
            .finish()
    }
}

impl UntypedArgument {
    /// Argument identifier name.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Argument parser kind.
    #[must_use]
    pub const fn kind(&self) -> &ArgumentKind {
        &self.kind
    }

    fn requires_ask_server(&self) -> bool {
        self.suggestion_callback.is_some() || matches!(self.kind, ArgumentKind::Choices { .. })
    }

    fn parser_encoding(&self) -> (Option<i32>, Vec<u8>) {
        let mut props = BytesMut::new();
        match &self.kind {
            ArgumentKind::Literal => (None, Vec::new()),
            ArgumentKind::Word | ArgumentKind::Choices { .. } => {
                write_var_int(&mut props, 0);
                (Some(parser_ids::STRING), props.to_vec())
            }
            ArgumentKind::QuotableString => {
                write_var_int(&mut props, 1);
                (Some(parser_ids::STRING), props.to_vec())
            }
            ArgumentKind::GreedyString => {
                write_var_int(&mut props, 2);
                (Some(parser_ids::STRING), props.to_vec())
            }
            ArgumentKind::Boolean => (Some(parser_ids::BOOL), Vec::new()),
            ArgumentKind::Integer { min, max } => {
                let flags = u8::from(min.is_some()) | (u8::from(max.is_some()) << 1);
                props.put_u8(flags);
                if let Some(lo) = min {
                    props.put_i32(*lo);
                }
                if let Some(hi) = max {
                    props.put_i32(*hi);
                }
                (Some(parser_ids::INTEGER), props.to_vec())
            }
            ArgumentKind::Long { min, max } => {
                let flags = u8::from(min.is_some()) | (u8::from(max.is_some()) << 1);
                props.put_u8(flags);
                if let Some(lo) = min {
                    props.put_i64(*lo);
                }
                if let Some(hi) = max {
                    props.put_i64(*hi);
                }
                (Some(parser_ids::LONG), props.to_vec())
            }
            ArgumentKind::Float { min, max } => {
                let flags = u8::from(min.is_some()) | (u8::from(max.is_some()) << 1);
                props.put_u8(flags);
                if let Some(lo) = min {
                    props.put_f32(*lo);
                }
                if let Some(hi) = max {
                    props.put_f32(*hi);
                }
                (Some(parser_ids::FLOAT), props.to_vec())
            }
            ArgumentKind::Double { min, max } => {
                let flags = u8::from(min.is_some()) | (u8::from(max.is_some()) << 1);
                props.put_u8(flags);
                if let Some(lo) = min {
                    props.put_f64(*lo);
                }
                if let Some(hi) = max {
                    props.put_f64(*hi);
                }
                (Some(parser_ids::DOUBLE), props.to_vec())
            }
            ArgumentKind::GameMode => (Some(parser_ids::GAMEMODE), Vec::new()),
            ArgumentKind::BlockPos => (Some(parser_ids::BLOCK_POS), Vec::new()),
            ArgumentKind::Vec3 => (Some(parser_ids::VEC3), Vec::new()),
        }
    }

    fn node_signature_key(&self) -> String {
        let (pid, props) = self.parser_encoding();
        format!(
            "{}:{}:{pid:?}:{props:?}:{}",
            matches!(self.kind, ArgumentKind::Literal),
            self.id,
            self.requires_ask_server()
        )
    }

    fn parse<'a>(
        &self,
        cursor: &mut InputCursor<'a>,
        sender: &CommandSender,
    ) -> Result<(ArgumentValue, &'a str), CommandError> {
        let start_idx = cursor.pos;
        match &self.kind {
            ArgumentKind::Literal => {
                let tok = cursor.next_word()?;
                if tok == self.id {
                    Ok((ArgumentValue::Text(tok.to_owned()), tok))
                } else {
                    Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("expected literal `{}`", self.id),
                    })
                }
            }
            ArgumentKind::Word => {
                let tok = cursor.next_word()?;
                Ok((ArgumentValue::Text(tok.to_owned()), tok))
            }
            ArgumentKind::QuotableString => {
                let (val, raw) = cursor.next_quotable()?;
                Ok((ArgumentValue::Text(val), raw))
            }
            ArgumentKind::GreedyString => {
                let rest = cursor.consume_remaining()?;
                Ok((ArgumentValue::Text(rest.to_owned()), rest))
            }
            ArgumentKind::Boolean => {
                let tok = cursor.next_word()?;
                match tok {
                    "true" => Ok((ArgumentValue::Bool(true), tok)),
                    "false" => Ok((ArgumentValue::Bool(false), tok)),
                    _ => Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: "expected `true` or `false`".to_owned(),
                    }),
                }
            }
            ArgumentKind::Integer { min, max } => {
                let tok = cursor.next_word()?;
                let val = tok.parse::<i32>().map_err(|_| CommandError::SyntaxError {
                    input: tok.to_owned(),
                    reason: "expected 32-bit integer".to_owned(),
                })?;
                if let Some(lo) = min
                    && val < *lo
                {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("integer must not be less than {lo}"),
                    });
                }
                if let Some(hi) = max
                    && val > *hi
                {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("integer must not exceed {hi}"),
                    });
                }
                Ok((ArgumentValue::Int(val), tok))
            }
            ArgumentKind::Long { min, max } => {
                let tok = cursor.next_word()?;
                let val = tok.parse::<i64>().map_err(|_| CommandError::SyntaxError {
                    input: tok.to_owned(),
                    reason: "expected 64-bit integer".to_owned(),
                })?;
                if let Some(lo) = min
                    && val < *lo
                {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("long must not be less than {lo}"),
                    });
                }
                if let Some(hi) = max
                    && val > *hi
                {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("long must not exceed {hi}"),
                    });
                }
                Ok((ArgumentValue::Long(val), tok))
            }
            ArgumentKind::Float { min, max } => {
                let tok = cursor.next_word()?;
                let val = tok.parse::<f32>().map_err(|_| CommandError::SyntaxError {
                    input: tok.to_owned(),
                    reason: "expected float".to_owned(),
                })?;
                if !val.is_finite() {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: "float must be finite".to_owned(),
                    });
                }
                if let Some(lo) = min
                    && val < *lo
                {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("float must not be less than {lo}"),
                    });
                }
                if let Some(hi) = max
                    && val > *hi
                {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("float must not exceed {hi}"),
                    });
                }
                Ok((ArgumentValue::Float(val), tok))
            }
            ArgumentKind::Double { min, max } => {
                let tok = cursor.next_word()?;
                let val = tok.parse::<f64>().map_err(|_| CommandError::SyntaxError {
                    input: tok.to_owned(),
                    reason: "expected double".to_owned(),
                })?;
                if !val.is_finite() {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: "double must be finite".to_owned(),
                    });
                }
                if let Some(lo) = min
                    && val < *lo
                {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("double must not be less than {lo}"),
                    });
                }
                if let Some(hi) = max
                    && val > *hi
                {
                    return Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("double must not exceed {hi}"),
                    });
                }
                Ok((ArgumentValue::Double(val), tok))
            }
            ArgumentKind::Choices { choices } => {
                let tok = cursor.next_word()?;
                if let Some(matched) = choices.iter().find(|c| c.eq_ignore_ascii_case(tok)) {
                    Ok((ArgumentValue::Text(matched.clone()), tok))
                } else {
                    Err(CommandError::SyntaxError {
                        input: tok.to_owned(),
                        reason: format!("expected one of {}", choices.join(", ")),
                    })
                }
            }
            ArgumentKind::GameMode => {
                let tok = cursor.next_word()?;
                let mode = match tok.to_ascii_lowercase().as_str() {
                    "survival" | "s" | "0" => GameMode::Survival,
                    "creative" | "c" | "1" => GameMode::Creative,
                    "adventure" | "a" | "2" => GameMode::Adventure,
                    "spectator" | "sp" | "3" => GameMode::Spectator,
                    _ => {
                        return Err(CommandError::SyntaxError {
                            input: tok.to_owned(),
                            reason: "expected survival, creative, adventure, or spectator"
                                .to_owned(),
                        });
                    }
                };
                Ok((ArgumentValue::GameMode(mode), tok))
            }
            ArgumentKind::BlockPos => {
                let origin = sender.origin_position().to_block_vec();
                let x = parse_rel_i32(cursor.next_word()?, origin.x)?;
                let y = parse_rel_i32(cursor.next_word()?, origin.y)?;
                let z = parse_rel_i32(cursor.next_word()?, origin.z)?;
                let raw = &cursor.src[start_idx..cursor.pos];
                Ok((ArgumentValue::BlockPos(BlockVec::new(x, y, z)), raw.trim()))
            }
            ArgumentKind::Vec3 => {
                let origin = sender.origin_position();
                let x = parse_rel_f64(cursor.next_word()?, origin.x)?;
                let y = parse_rel_f64(cursor.next_word()?, origin.y)?;
                let z = parse_rel_f64(cursor.next_word()?, origin.z)?;
                let raw = &cursor.src[start_idx..cursor.pos];
                Ok((ArgumentValue::Vec3(Vec3::new(x, y, z)), raw.trim()))
            }
        }
    }

    fn suggest(
        &self,
        sender: &CommandSender,
        partial_ctx: &CommandContext,
        prefix: &str,
    ) -> Vec<SuggestionEntry> {
        let mut suggestion = Suggestion::new(prefix);
        if let Some(cb) = &self.suggestion_callback {
            cb(sender, partial_ctx, &mut suggestion);
        } else {
            match &self.kind {
                ArgumentKind::Literal => suggestion.add(&self.id),
                ArgumentKind::Boolean => {
                    suggestion.add("true");
                    suggestion.add("false");
                }
                ArgumentKind::Choices { choices } => {
                    for c in choices {
                        suggestion.add(c);
                    }
                }
                ArgumentKind::GameMode => {
                    for m in ["survival", "creative", "adventure", "spectator"] {
                        suggestion.add(m);
                    }
                }
                _ => {}
            }
        }
        let lower = prefix.to_ascii_lowercase();
        suggestion
            .entries
            .into_iter()
            .filter(|e| e.value.to_ascii_lowercase().starts_with(&lower))
            .collect()
    }
}

fn parse_rel_i32(tok: &str, base: i32) -> Result<i32, CommandError> {
    if let Some(rest) = tok.strip_prefix('~') {
        if rest.is_empty() {
            Ok(base)
        } else {
            let delta = rest.parse::<i32>().map_err(|_| CommandError::SyntaxError {
                input: tok.to_owned(),
                reason: "invalid relative block coordinate".to_owned(),
            })?;
            Ok(base.saturating_add(delta))
        }
    } else {
        tok.parse::<i32>().map_err(|_| CommandError::SyntaxError {
            input: tok.to_owned(),
            reason: "expected integer block coordinate".to_owned(),
        })
    }
}

fn parse_rel_f64(tok: &str, base: f64) -> Result<f64, CommandError> {
    if let Some(rest) = tok.strip_prefix('~') {
        if rest.is_empty() {
            Ok(base)
        } else {
            let delta = rest.parse::<f64>().map_err(|_| CommandError::SyntaxError {
                input: tok.to_owned(),
                reason: "invalid relative coordinate".to_owned(),
            })?;
            if !delta.is_finite() {
                return Err(CommandError::SyntaxError {
                    input: tok.to_owned(),
                    reason: "coordinate must be finite".to_owned(),
                });
            }
            Ok(base + delta)
        }
    } else {
        let v = tok.parse::<f64>().map_err(|_| CommandError::SyntaxError {
            input: tok.to_owned(),
            reason: "expected coordinate number".to_owned(),
        })?;
        if !v.is_finite() {
            return Err(CommandError::SyntaxError {
                input: tok.to_owned(),
                reason: "coordinate must be finite".to_owned(),
            });
        }
        Ok(v)
    }
}

/// Strongly typed handle to a command argument of type `T`.
///
/// Pass `arg.untyped()` into [`Command::add_syntax`] and use `&arg` inside the executor closure
/// with [`CommandContext::get`] for compile-time type-checked extraction.
#[derive(Clone, Debug)]
pub struct Argument<T> {
    inner: UntypedArgument,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Argument<T> {
    const fn from_untyped(inner: UntypedArgument) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }

    /// Argument identifier name.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// Attaches a dynamic server-side [`SuggestionCallback`] (`minecraft:ask_server`).
    #[must_use]
    pub fn with_suggestion_callback(
        mut self,
        cb: impl Fn(&CommandSender, &CommandContext, &mut Suggestion) + Send + Sync + 'static,
    ) -> Self {
        self.inner.suggestion_callback = Some(Arc::new(cb));
        self
    }

    /// Converts this typed handle into an [`UntypedArgument`] for syntax registration.
    #[must_use]
    pub fn untyped(&self) -> UntypedArgument {
        self.inner.clone()
    }
}

impl Argument<String> {
    /// Exact literal keyword argument (`ArgumentType.Literal` in Minestom).
    #[must_use]
    pub fn literal(keyword: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: keyword.into(),
            kind: ArgumentKind::Literal,
            suggestion_callback: None,
        })
    }

    /// Single whitespace-delimited word (`ArgumentType.Word`).
    #[must_use]
    pub fn word(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::Word,
            suggestion_callback: None,
        })
    }

    /// Single word or `"quoted phrase"` (`ArgumentType.String`).
    #[must_use]
    pub fn string(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::QuotableString,
            suggestion_callback: None,
        })
    }

    /// Greedy trailing string consuming the remainder of input (`ArgumentType.StringArray` / greedy).
    #[must_use]
    pub fn greedy_string(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::GreedyString,
            suggestion_callback: None,
        })
    }

    /// Fixed set of allowed string choices with automatic tab-completion (`ArgumentType.Enum` / `Word().from(...)`).
    #[must_use]
    pub fn choices<I, S>(id: impl Into<String>, choices: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::Choices {
                choices: choices.into_iter().map(Into::into).collect(),
            },
            suggestion_callback: None,
        })
    }
}

impl Argument<bool> {
    /// Boolean (`true` / `false`) argument (`ArgumentType.Boolean`).
    #[must_use]
    pub fn boolean(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::Boolean,
            suggestion_callback: None,
        })
    }
}

impl Argument<i32> {
    /// 32-bit signed integer argument (`ArgumentType.Integer`).
    #[must_use]
    pub fn integer(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::Integer {
                min: None,
                max: None,
            },
            suggestion_callback: None,
        })
    }

    /// Constrains this integer argument to `min..=max` (inclusive).
    #[must_use]
    pub fn between(mut self, min: i32, max: i32) -> Self {
        self.inner.kind = ArgumentKind::Integer {
            min: Some(min),
            max: Some(max),
        };
        self
    }

    /// Sets an inclusive minimum bound.
    #[must_use]
    pub fn min(mut self, min: i32) -> Self {
        if let ArgumentKind::Integer { min: lo, .. } = &mut self.inner.kind {
            *lo = Some(min);
        }
        self
    }

    /// Sets an inclusive maximum bound.
    #[must_use]
    pub fn max(mut self, max: i32) -> Self {
        if let ArgumentKind::Integer { max: hi, .. } = &mut self.inner.kind {
            *hi = Some(max);
        }
        self
    }
}

impl Argument<i64> {
    /// 64-bit signed integer argument (`ArgumentType.Long`).
    #[must_use]
    pub fn long(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::Long {
                min: None,
                max: None,
            },
            suggestion_callback: None,
        })
    }

    /// Constrains this `i64` argument to `min..=max` (inclusive).
    #[must_use]
    pub fn between(mut self, min: i64, max: i64) -> Self {
        self.inner.kind = ArgumentKind::Long {
            min: Some(min),
            max: Some(max),
        };
        self
    }
}

impl Argument<f32> {
    /// 32-bit float argument (`ArgumentType.Float`).
    #[must_use]
    pub fn float(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::Float {
                min: None,
                max: None,
            },
            suggestion_callback: None,
        })
    }

    /// Constrains this `f32` argument to `min..=max` (inclusive).
    #[must_use]
    pub fn between(mut self, min: f32, max: f32) -> Self {
        self.inner.kind = ArgumentKind::Float {
            min: Some(min),
            max: Some(max),
        };
        self
    }
}

impl Argument<f64> {
    /// 64-bit double argument (`ArgumentType.Double`).
    #[must_use]
    pub fn double(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::Double {
                min: None,
                max: None,
            },
            suggestion_callback: None,
        })
    }

    /// Constrains this `f64` argument to `min..=max` (inclusive).
    #[must_use]
    pub fn between(mut self, min: f64, max: f64) -> Self {
        self.inner.kind = ArgumentKind::Double {
            min: Some(min),
            max: Some(max),
        };
        self
    }
}

impl Argument<GameMode> {
    /// Minecraft [`GameMode`] argument (`minecraft:gamemode`).
    #[must_use]
    pub fn game_mode(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::GameMode,
            suggestion_callback: None,
        })
    }
}

impl Argument<BlockVec> {
    /// 3-integer block position argument supporting `~` relative coordinates (`ArgumentType.RelativeBlockPosition`).
    #[must_use]
    pub fn block_pos(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::BlockPos,
            suggestion_callback: None,
        })
    }
}

impl Argument<Vec3> {
    /// 3-float vector position argument supporting `~` relative coordinates (`ArgumentType.RelativeVec3`).
    #[must_use]
    pub fn vec3(id: impl Into<String>) -> Self {
        Self::from_untyped(UntypedArgument {
            id: id.into(),
            kind: ArgumentKind::Vec3,
            suggestion_callback: None,
        })
    }
}

/// Parsed argument container passed into command executors and [`SuggestionCallback`]s.
#[derive(Debug, Clone)]
pub struct CommandContext {
    input: String,
    command_name: String,
    values: HashMap<String, ArgumentValue>,
    raw_slices: HashMap<String, String>,
}

impl CommandContext {
    fn new(input: impl Into<String>, command_name: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            command_name: command_name.into(),
            values: HashMap::new(),
            raw_slices: HashMap::new(),
        }
    }

    /// Full raw command line without leading `/`.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Invoked command label or alias.
    #[must_use]
    pub fn command_name(&self) -> &str {
        &self.command_name
    }

    /// Retrieves a strongly typed reference to `arg`'s parsed value.
    #[must_use]
    pub fn get<T: FromArgumentValue>(&self, arg: &Argument<T>) -> Option<&T> {
        self.values.get(arg.id()).and_then(T::from_arg_value)
    }

    /// Retrieves the raw input substring matched by `arg`.
    #[must_use]
    pub fn raw<T>(&self, arg: &Argument<T>) -> Option<&str> {
        self.raw_slices.get(arg.id()).map(String::as_str)
    }
}

/// Synchronous command execution callback (`CommandExecutor` in Minestom).
pub type CommandExecutor = Arc<dyn Fn(&mut CommandSender, &CommandContext) + Send + Sync>;

/// Single syntax branch inside a [`Command`].
#[derive(Clone)]
pub struct CommandSyntax {
    arguments: Vec<UntypedArgument>,
    condition: Option<CommandCondition>,
    executor: CommandExecutor,
}

/// Declarative command definition (`Command` in Minestom) supporting aliases, conditions,
/// default executors, multiple typed syntaxes, and nested subcommands.
#[derive(Clone)]
pub struct Command {
    name: String,
    aliases: Vec<String>,
    condition: Option<CommandCondition>,
    default_executor: Option<CommandExecutor>,
    syntaxes: Vec<CommandSyntax>,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command")
            .field("name", &self.name)
            .field("aliases", &self.aliases)
            .field("syntaxes", &self.syntaxes.len())
            .finish_non_exhaustive()
    }
}

impl Command {
    /// Creates a new [`Command`] with primary `name` (do not include leading `/`).
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            aliases: Vec::new(),
            condition: None,
            default_executor: None,
            syntaxes: Vec::new(),
        }
    }

    /// Attaches one or more command aliases (e.g. `["gm"]` for `"gamemode"`).
    #[must_use]
    pub fn with_aliases<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.aliases.extend(aliases.into_iter().map(Into::into));
        self
    }

    /// Sets an execution/visibility condition for the entire command.
    #[must_use]
    pub fn with_condition(
        mut self,
        cond: impl Fn(&CommandSender, Option<&str>) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.condition = Some(Arc::new(cond));
        self
    }

    /// Restricts this command to senders with at least `permission_level` (console always passes).
    #[must_use]
    pub fn with_permission_level(self, permission_level: u8) -> Self {
        self.with_condition(move |sender, _| sender.has_permission_level(permission_level))
    }

    /// Sets the fallback executor run when invoked with zero arguments (or when no syntax matches,
    /// if zero arguments are passed).
    #[must_use]
    pub fn with_default_executor(
        mut self,
        exec: impl Fn(&mut CommandSender, &CommandContext) + Send + Sync + 'static,
    ) -> Self {
        self.default_executor = Some(Arc::new(exec));
        self
    }

    /// Adds a typed argument syntax branch.
    #[must_use]
    pub fn add_syntax<I>(
        mut self,
        args: I,
        exec: impl Fn(&mut CommandSender, &CommandContext) + Send + Sync + 'static,
    ) -> Self
    where
        I: IntoIterator<Item = UntypedArgument>,
    {
        self.syntaxes.push(CommandSyntax {
            arguments: args.into_iter().collect(),
            condition: None,
            executor: Arc::new(exec),
        });
        self
    }

    /// Adds a typed argument syntax branch guarded by an additional [`CommandCondition`].
    #[must_use]
    pub fn add_conditional_syntax<I>(
        mut self,
        args: I,
        cond: impl Fn(&CommandSender, Option<&str>) -> bool + Send + Sync + 'static,
        exec: impl Fn(&mut CommandSender, &CommandContext) + Send + Sync + 'static,
    ) -> Self
    where
        I: IntoIterator<Item = UntypedArgument>,
    {
        self.syntaxes.push(CommandSyntax {
            arguments: args.into_iter().collect(),
            condition: Some(Arc::new(cond)),
            executor: Arc::new(exec),
        });
        self
    }

    /// Mounts `subcommand` under this command by prepending literal nodes for its name and aliases.
    #[must_use]
    pub fn add_subcommand(mut self, subcommand: &Self) -> Self {
        let mut sub_labels = Vec::with_capacity(1 + subcommand.aliases.len());
        sub_labels.push(subcommand.name.clone());
        sub_labels.extend(subcommand.aliases.clone());

        for label in sub_labels {
            let lit = Argument::literal(&label).untyped();
            if let Some(def_exec) = &subcommand.default_executor {
                let exec_clone = Arc::clone(def_exec);
                let sub_cond = subcommand.condition.clone();
                self.syntaxes.push(CommandSyntax {
                    arguments: vec![lit.clone()],
                    condition: sub_cond,
                    executor: exec_clone,
                });
            }
            for syn in &subcommand.syntaxes {
                let mut combined_args = Vec::with_capacity(1 + syn.arguments.len());
                combined_args.push(lit.clone());
                combined_args.extend(syn.arguments.clone());

                let merged_cond =
                    merge_conditions(subcommand.condition.as_ref(), syn.condition.as_ref());
                self.syntaxes.push(CommandSyntax {
                    arguments: combined_args,
                    condition: merged_cond,
                    executor: Arc::clone(&syn.executor),
                });
            }
        }
        self
    }

    /// Primary command name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Registered aliases.
    #[must_use]
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }
}

fn merge_conditions(
    a: Option<&CommandCondition>,
    b: Option<&CommandCondition>,
) -> Option<CommandCondition> {
    match (a, b) {
        (None, None) => None,
        (Some(c), None) | (None, Some(c)) => Some(Arc::clone(c)),
        (Some(c1), Some(c2)) => {
            let c1 = Arc::clone(c1);
            let c2 = Arc::clone(c2);
            Some(Arc::new(move |s, txt| c1(s, txt) && c2(s, txt)))
        }
    }
}

struct InputCursor<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> InputCursor<'a> {
    const fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn skip_spaces(&mut self) {
        while let Some(b) = self.src.as_bytes().get(self.pos) {
            if *b == b' ' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn is_eof(&mut self) -> bool {
        self.skip_spaces();
        self.pos >= self.src.len()
    }

    fn next_word(&mut self) -> Result<&'a str, CommandError> {
        self.skip_spaces();
        if self.pos >= self.src.len() {
            return Err(CommandError::SyntaxError {
                input: String::new(),
                reason: "unexpected end of command".to_owned(),
            });
        }
        let start = self.pos;
        while let Some(&b) = self.src.as_bytes().get(self.pos) {
            if b == b' ' {
                break;
            }
            self.pos += 1;
        }
        Ok(&self.src[start..self.pos])
    }

    fn next_quotable(&mut self) -> Result<(String, &'a str), CommandError> {
        self.skip_spaces();
        if self.pos >= self.src.len() {
            return Err(CommandError::SyntaxError {
                input: String::new(),
                reason: "unexpected end of command".to_owned(),
            });
        }
        let bytes = self.src.as_bytes();
        let quote = bytes[self.pos];
        if quote == b'"' || quote == b'\'' {
            let start = self.pos;
            self.pos += 1;
            let mut unescaped = String::new();
            let mut closed = false;
            while self.pos < self.src.len() {
                let ch = self.src[self.pos..].chars().next().unwrap_or('\0');
                self.pos += ch.len_utf8();
                if ch == '\\' && self.pos < self.src.len() {
                    let next = self.src[self.pos..].chars().next().unwrap_or('\0');
                    self.pos += next.len_utf8();
                    unescaped.push(next);
                } else if ch as u8 == quote {
                    closed = true;
                    break;
                } else {
                    unescaped.push(ch);
                }
            }
            if !closed {
                return Err(CommandError::SyntaxError {
                    input: self.src[start..].to_owned(),
                    reason: "unterminated quoted string".to_owned(),
                });
            }
            Ok((unescaped, &self.src[start..self.pos]))
        } else {
            let word = self.next_word()?;
            Ok((word.to_owned(), word))
        }
    }

    fn consume_remaining(&mut self) -> Result<&'a str, CommandError> {
        self.skip_spaces();
        if self.pos >= self.src.len() {
            return Err(CommandError::SyntaxError {
                input: String::new(),
                reason: "expected trailing string".to_owned(),
            });
        }
        let start = self.pos;
        self.pos = self.src.len();
        Ok(&self.src[start..])
    }
}

/// Result of server-side tab completion ready to populate `TabCompletePacket`.
#[derive(Debug, Clone, PartialEq)]
pub struct TabCompleteResult {
    /// Character index (0-based in client input including leading `/`) where replacement begins.
    pub start: i32,
    /// Character length of the substring being replaced.
    pub length: i32,
    /// Candidate matches.
    pub matches: Vec<TabCompleteMatch>,
}

type UnknownCommandCallback = Arc<dyn Fn(&mut CommandSender, &str) + Send + Sync>;

struct ManagerInner {
    commands: Vec<Command>,
    lookup: FxHashMap<String, usize>,
    unknown_callback: Option<UnknownCommandCallback>,
}

/// Thread-safe Brigadier command registry, dispatcher, and `DeclareCommandsPacket` builder
/// (`CommandManager` in Minestom).
#[derive(Clone)]
pub struct CommandManager {
    inner: Arc<RwLock<ManagerInner>>,
}

impl std::fmt::Debug for CommandManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.inner.read();
        f.debug_struct("CommandManager")
            .field("commands", &guard.commands.len())
            .finish_non_exhaustive()
    }
}

impl Default for CommandManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandManager {
    /// Creates an empty [`CommandManager`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(ManagerInner {
                commands: Vec::new(),
                lookup: FxHashMap::default(),
                unknown_callback: None,
            })),
        }
    }

    /// Registers a [`Command`] and all of its aliases (case-insensitive lookup).
    pub fn register(&self, command: Command) {
        let mut guard = self.inner.write();
        let idx = guard.commands.len();
        guard.lookup.insert(command.name.to_ascii_lowercase(), idx);
        for alias in &command.aliases {
            guard.lookup.insert(alias.to_ascii_lowercase(), idx);
        }
        guard.commands.push(command);
    }

    /// Returns `true` if a command or alias matching `name` is registered.
    #[must_use]
    pub fn command_exists(&self, name: &str) -> bool {
        self.inner
            .read()
            .lookup
            .contains_key(&name.to_ascii_lowercase())
    }

    /// Sets an optional callback invoked when [`Self::execute`] encounters an unknown command.
    pub fn set_unknown_command_callback(
        &self,
        cb: impl Fn(&mut CommandSender, &str) + Send + Sync + 'static,
    ) {
        self.inner.write().unknown_callback = Some(Arc::new(cb));
    }

    /// Parses and executes `command_line` (leading `/` optional) as `sender`.
    pub fn execute(
        &self,
        sender: &mut CommandSender,
        command_line: &str,
    ) -> Result<(), CommandError> {
        let trimmed = command_line.trim_start();
        let clean = trimmed.strip_prefix('/').unwrap_or(trimmed);
        let mut root_cursor = InputCursor::new(clean);
        let Ok(invoked_label) = root_cursor.next_word() else {
            return Err(CommandError::UnknownCommand(String::new()));
        };

        let (cmd, unknown_cb) = {
            let guard = self.inner.read();
            let found = guard
                .lookup
                .get(&invoked_label.to_ascii_lowercase())
                .and_then(|&idx| guard.commands.get(idx).cloned());
            (found, guard.unknown_callback.clone())
        };

        let Some(cmd) = cmd else {
            if let Some(cb) = unknown_cb {
                cb(sender, clean);
            }
            return Err(CommandError::UnknownCommand(invoked_label.to_owned()));
        };

        if let Some(cond) = &cmd.condition
            && !cond(sender, Some(clean))
        {
            return Err(CommandError::ConditionFailed);
        }

        let args_tail = &clean[root_cursor.pos..];
        let mut check_empty = InputCursor::new(args_tail);
        if check_empty.is_eof()
            && let Some(def_exec) = &cmd.default_executor
        {
            // Still prefer an explicit 0-arg syntax if one exists, otherwise default executor
            if !cmd.syntaxes.iter().any(|s| s.arguments.is_empty()) {
                let ctx = CommandContext::new(clean, invoked_label);
                def_exec(sender, &ctx);
                return Ok(());
            }
        }

        let mut best_error: Option<(usize, CommandError)> = None;

        for syn in &cmd.syntaxes {
            let mut cursor = InputCursor::new(args_tail);
            let mut ctx = CommandContext::new(clean, invoked_label);
            let mut failed: Option<(usize, CommandError)> = None;

            for (depth, arg) in syn.arguments.iter().enumerate() {
                match arg.parse(&mut cursor, sender) {
                    Ok((val, raw_str)) => {
                        ctx.values.insert(arg.id.clone(), val);
                        ctx.raw_slices.insert(arg.id.clone(), raw_str.to_owned());
                    }
                    Err(err) => {
                        failed = Some((depth, err));
                        break;
                    }
                }
            }

            if let Some((depth, err)) = failed {
                if best_error.as_ref().is_none_or(|(d, _)| depth >= *d) {
                    best_error = Some((depth, err));
                }
                continue;
            }

            if !cursor.is_eof() {
                let trailing = cursor.src[cursor.pos..].trim().to_owned();
                let depth = syn.arguments.len();
                let err = CommandError::SyntaxError {
                    input: trailing,
                    reason: "unexpected trailing arguments".to_owned(),
                };
                if best_error.as_ref().is_none_or(|(d, _)| depth >= *d) {
                    best_error = Some((depth, err));
                }
                continue;
            }

            if let Some(syn_cond) = &syn.condition
                && !syn_cond(sender, Some(clean))
            {
                best_error = Some((syn.arguments.len(), CommandError::ConditionFailed));
                continue;
            }

            (syn.executor)(sender, &ctx);
            return Ok(());
        }

        if let Some((_, err)) = best_error {
            Err(err)
        } else if let Some(def_exec) = &cmd.default_executor {
            let ctx = CommandContext::new(clean, invoked_label);
            def_exec(sender, &ctx);
            Ok(())
        } else {
            Err(CommandError::SyntaxError {
                input: args_tail.trim().to_owned(),
                reason: "invalid arguments".to_owned(),
            })
        }
    }

    /// Computes server-side tab completions for `raw_input` (e.g. `"/gm cr"` or `"/tp "` ).
    #[must_use]
    pub fn complete(&self, sender: &CommandSender, raw_input: &str) -> TabCompleteResult {
        let has_slash = raw_input.starts_with('/');
        let body = raw_input.strip_prefix('/').unwrap_or(raw_input);
        let slash_offset = usize::from(has_slash);

        // Case 1: Still typing command name (no space encountered yet)
        if !body.contains(' ') {
            let lower = body.to_ascii_lowercase();
            let guard = self.inner.read();
            let mut candidates = Vec::new();
            for cmd in &guard.commands {
                if cmd
                    .condition
                    .as_ref()
                    .is_some_and(|c| !c(sender, Some(body)))
                {
                    continue;
                }
                for label in std::iter::once(&cmd.name).chain(cmd.aliases.iter()) {
                    if label.to_ascii_lowercase().starts_with(&lower) {
                        candidates.push(TabCompleteMatch {
                            match_str: label.clone(),
                            tooltip: None,
                        });
                    }
                }
            }
            candidates.sort_by(|a, b| a.match_str.cmp(&b.match_str));
            candidates.dedup_by(|a, b| a.match_str == b.match_str);
            return TabCompleteResult {
                start: slash_offset as i32,
                length: body.chars().count() as i32,
                matches: candidates,
            };
        }

        // Case 2: Completing an argument inside a known command
        let first_space = body.find(' ').unwrap_or(body.len());
        let cmd_label = &body[..first_space];
        let cmd = {
            let guard = self.inner.read();
            guard
                .lookup
                .get(&cmd_label.to_ascii_lowercase())
                .and_then(|&idx| guard.commands.get(idx).cloned())
        };
        let Some(cmd) = cmd else {
            return TabCompleteResult {
                start: raw_input.chars().count() as i32,
                length: 0,
                matches: Vec::new(),
            };
        };
        if cmd
            .condition
            .as_ref()
            .is_some_and(|c| !c(sender, Some(body)))
        {
            return TabCompleteResult {
                start: raw_input.chars().count() as i32,
                length: 0,
                matches: Vec::new(),
            };
        }

        let last_space_byte = body.rfind(' ').unwrap_or(first_space);
        let prior_args_slice = if last_space_byte > first_space {
            &body[first_space + 1..last_space_byte]
        } else {
            ""
        };
        let current_prefix = &body[last_space_byte + 1..];
        let replace_start_chars = raw_input[..=(slash_offset + last_space_byte)]
            .chars()
            .count() as i32;
        let replace_len_chars = current_prefix.chars().count() as i32;

        let mut collected: Vec<TabCompleteMatch> = Vec::new();

        for syn in &cmd.syntaxes {
            if syn
                .condition
                .as_ref()
                .is_some_and(|c| !c(sender, Some(body)))
            {
                continue;
            }
            let mut cursor = InputCursor::new(prior_args_slice);
            let mut partial_ctx = CommandContext::new(body, cmd_label);
            let mut next_arg_idx = 0_usize;
            let mut ok = true;

            while !cursor.is_eof() {
                let Some(arg) = syn.arguments.get(next_arg_idx) else {
                    ok = false;
                    break;
                };
                if let Ok((val, raw_str)) = arg.parse(&mut cursor, sender) {
                    partial_ctx.values.insert(arg.id.clone(), val);
                    partial_ctx
                        .raw_slices
                        .insert(arg.id.clone(), raw_str.to_owned());
                    next_arg_idx += 1;
                } else {
                    ok = false;
                    break;
                }
            }

            if ok && let Some(target_arg) = syn.arguments.get(next_arg_idx) {
                for entry in target_arg.suggest(sender, &partial_ctx, current_prefix) {
                    if !collected.iter().any(|m| m.match_str == entry.value) {
                        collected.push(TabCompleteMatch {
                            match_str: entry.value,
                            tooltip: entry.tooltip,
                        });
                    }
                }
            }
        }

        TabCompleteResult {
            start: replace_start_chars,
            length: replace_len_chars,
            matches: collected,
        }
    }

    /// Serializes all commands permitted for `sender` into a Brigadier [`DeclareCommandsPacket`].
    #[must_use]
    pub fn build_declare_commands_packet(&self, sender: &CommandSender) -> DeclareCommandsPacket {
        let guard = self.inner.read();
        let mut nodes = vec![CommandGraphNode {
            flags: node_flags::ROOT,
            children: Vec::new(),
            redirect_node: None,
            name: None,
            parser_id: None,
            properties: Vec::new(),
            suggestions_type: None,
        }];

        // Map `(parent_node_idx, node_signature)` -> `child_node_idx` to deduplicate shared prefixes
        let mut edge_dedup: FxHashMap<(usize, String), usize> = FxHashMap::default();

        for cmd in &guard.commands {
            if cmd.condition.as_ref().is_some_and(|c| !c(sender, None)) {
                continue;
            }

            let visible_syntaxes: Vec<&CommandSyntax> = cmd
                .syntaxes
                .iter()
                .filter(|s| s.condition.as_ref().is_none_or(|c| c(sender, None)))
                .collect();

            let root_executable = cmd.default_executor.is_some()
                || visible_syntaxes.iter().any(|s| s.arguments.is_empty());

            // Build primary command literal node under root (index 0)
            let primary_idx = nodes.len();
            let mut primary_flags = node_flags::LITERAL;
            if root_executable {
                primary_flags |= node_flags::EXECUTABLE;
            }
            nodes.push(CommandGraphNode {
                flags: primary_flags,
                children: Vec::new(),
                redirect_node: None,
                name: Some(cmd.name.clone()),
                parser_id: None,
                properties: Vec::new(),
                suggestions_type: None,
            });
            nodes[0].children.push(primary_idx as i32);

            for syn in &visible_syntaxes {
                if syn.arguments.is_empty() {
                    continue;
                }
                let mut parent = primary_idx;
                for (i, arg) in syn.arguments.iter().enumerate() {
                    let is_last = i + 1 == syn.arguments.len();
                    let sig = arg.node_signature_key();
                    if let Some(&existing_idx) = edge_dedup.get(&(parent, sig.clone())) {
                        if is_last {
                            nodes[existing_idx].flags |= node_flags::EXECUTABLE;
                        }
                        parent = existing_idx;
                    } else {
                        let (parser_id, properties) = arg.parser_encoding();
                        let mut flags = if matches!(arg.kind, ArgumentKind::Literal) {
                            node_flags::LITERAL
                        } else {
                            node_flags::ARGUMENT
                        };
                        if is_last {
                            flags |= node_flags::EXECUTABLE;
                        }
                        let suggestions_type = if !matches!(arg.kind, ArgumentKind::Literal)
                            && arg.requires_ask_server()
                        {
                            flags |= node_flags::HAS_SUGGESTIONS_TYPE;
                            Some("minecraft:ask_server".to_owned())
                        } else {
                            None
                        };
                        let new_idx = nodes.len();
                        nodes.push(CommandGraphNode {
                            flags,
                            children: Vec::new(),
                            redirect_node: None,
                            name: Some(arg.id.clone()),
                            parser_id,
                            properties,
                            suggestions_type,
                        });
                        nodes[parent].children.push(new_idx as i32);
                        edge_dedup.insert((parent, sig), new_idx);
                        parent = new_idx;
                    }
                }
            }

            // Emit alias nodes sharing the exact child list & executability of `primary_idx`
            // (avoids vanilla Brigadier MC-136555 redirect bug on 0-arg executable aliases)
            let primary_children = nodes[primary_idx].children.clone();
            for alias in &cmd.aliases {
                let alias_idx = nodes.len();
                nodes.push(CommandGraphNode {
                    flags: primary_flags,
                    children: primary_children.clone(),
                    redirect_node: None,
                    name: Some(alias.clone()),
                    parser_id: None,
                    properties: Vec::new(),
                    suggestions_type: None,
                });
                nodes[0].children.push(alias_idx as i32);
            }
        }

        DeclareCommandsPacket {
            nodes,
            root_index: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use minestom_core::Pos;

    use super::*;

    #[test]
    fn typed_arguments_relative_coords_conditions_and_suggestions_work() {
        let manager = CommandManager::new();

        let tp_pos = Argument::block_pos("target");
        let tp_pos_ref = tp_pos.clone();
        let tp = Command::new("tp").with_permission_level(2).add_syntax(
            [tp_pos.untyped()],
            move |sender, ctx| {
                let b = *ctx.get(&tp_pos_ref).unwrap();
                sender.teleport(Pos::new(f64::from(b.x), f64::from(b.y), f64::from(b.z)));
            },
        );
        manager.register(tp);

        let warp_arg = Argument::word("warp").with_suggestion_callback(|_, _, sug| {
            sug.add_entry(SuggestionEntry::with_tooltip("spawn", "Main hub"));
            sug.add("shop");
            sug.add("arena");
        });
        let warp_ref = warp_arg.clone();
        let warp = Command::new("warp")
            .with_default_executor(|sender, _| sender.send_message("Usage: /warp <name>"))
            .add_syntax([warp_arg.untyped()], move |sender, ctx| {
                let target = ctx.get(&warp_ref).unwrap();
                sender.send_message(format!("Warping to {target}"));
            });
        manager.register(warp);

        // Low-permission player fails /tp condition
        let mut p_low = CommandSender::player(PlayerSenderInfo {
            player_id: PlayerId::default(),
            uuid: Uuid::nil(),
            username: "Guest".to_owned(),
            position: Pos::new(10.0, 64.0, -5.0),
            game_mode: GameMode::Survival,
            permission_level: 0,
        });
        assert_eq!(
            manager.execute(&mut p_low, "/tp ~5 ~1 ~-2"),
            Err(CommandError::ConditionFailed)
        );

        // DeclareCommands for Guest hides /tp and exposes /warp with ask_server!
        let guest_decl = manager.build_declare_commands_packet(&p_low);
        assert_eq!(guest_decl.nodes[0].children.len(), 1);

        // Op player executes relative /tp ~5 ~1 ~-2 from (10, 64, -5) -> (15, 65, -7)
        let mut p_op = CommandSender::player(PlayerSenderInfo {
            player_id: PlayerId::default(),
            uuid: Uuid::nil(),
            username: "Admin".to_owned(),
            position: Pos::new(10.0, 64.0, -5.0),
            game_mode: GameMode::Creative,
            permission_level: 4,
        });
        manager.execute(&mut p_op, "/tp ~5 ~1 ~-2").unwrap();
        assert_eq!(p_op.origin_position(), Pos::new(15.0, 65.0, -7.0));

        // Tab-completion on "/warp s" returns ["spawn", "shop"] starting at index 6
        let comp = manager.complete(&p_low, "/warp s");
        assert_eq!(comp.start, 6);
        assert_eq!(comp.length, 1);
        assert_eq!(comp.matches.len(), 2);
        assert_eq!(comp.matches[0].match_str, "spawn");
        assert_eq!(comp.matches[1].match_str, "shop");
    }
}
