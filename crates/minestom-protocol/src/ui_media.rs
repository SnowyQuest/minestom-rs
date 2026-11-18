//! UI, HUD, media, scoreboard, teams, bossbar, titles, particles, sounds,
//! resource packs, advancements, recipes, and MC 26.2 dialog packets (protocol 776).

use std::collections::BTreeMap;

use bytes::{Buf, BufMut};
use minestom_core::{BlockVec, Vec3};
use minestom_item::ItemStack;
use minestom_macros::PacketMeta;
use minestom_nbt::NbtTag;
use minestom_text::Component;
use uuid::Uuid;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_prefixed_vec, read_string, read_var_int, write_prefixed_vec, write_string, write_var_int,
};

// ============================================================================
// BossBar (`0x09` Play)
// ============================================================================

/// Color of a [`BossBarPacket`] bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum BossBarColor {
    /// Pink (`0`).
    Pink = 0,
    /// Blue (`1`).
    Blue = 1,
    /// Red (`2`).
    Red = 2,
    /// Green (`3`).
    Green = 3,
    /// Yellow (`4`).
    Yellow = 4,
    /// Purple (`5`).
    Purple = 5,
    /// White (`6`).
    White = 6,
}

impl BossBarColor {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Pink),
            1 => Ok(Self::Blue),
            2 => Ok(Self::Red),
            3 => Ok(Self::Green),
            4 => Ok(Self::Yellow),
            5 => Ok(Self::Purple),
            6 => Ok(Self::White),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "BossBarColor",
                value: other,
            }),
        }
    }
}

/// Division overlay style of a [`BossBarPacket`] bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum BossBarOverlay {
    /// Continuous bar (`0`).
    Progress = 0,
    /// 6 segments (`1`).
    Notched6 = 1,
    /// 10 segments (`2`).
    Notched10 = 2,
    /// 12 segments (`3`).
    Notched12 = 3,
    /// 20 segments (`4`).
    Notched20 = 4,
}

impl BossBarOverlay {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Progress),
            1 => Ok(Self::Notched6),
            2 => Ok(Self::Notched10),
            3 => Ok(Self::Notched12),
            4 => Ok(Self::Notched20),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "BossBarOverlay",
                value: other,
            }),
        }
    }
}

/// Action payload inside [`BossBarPacket`].
#[derive(Debug, Clone, PartialEq)]
pub enum BossBarAction {
    /// Create/show boss bar (`0`).
    Add {
        /// Title text.
        title: Component,
        /// Progress fraction (`0.0..=1.0`).
        health: f32,
        /// Bar color.
        color: BossBarColor,
        /// Segment overlay.
        overlay: BossBarOverlay,
        /// Flags (`0x01` darken sky, `0x02` dragon music, `0x04` create fog).
        flags: u8,
    },
    /// Remove boss bar (`1`).
    Remove,
    /// Update progress (`2`).
    UpdateHealth(f32),
    /// Update title (`3`).
    UpdateTitle(Component),
    /// Update color and overlay (`4`).
    UpdateStyle {
        /// Bar color.
        color: BossBarColor,
        /// Segment overlay.
        overlay: BossBarOverlay,
    },
    /// Update sky/music/fog flags (`5`).
    UpdateFlags(u8),
}

/// Clientbound `BossBarPacket` (`0x09` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct BossBarPacket {
    /// Unique boss bar UUID.
    pub uuid: Uuid,
    /// Boss bar action.
    pub action: BossBarAction,
}

impl PacketPayload for BossBarPacket {
    const PACKET_ID: i32 = 0x09;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for BossBarPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.uuid.encode(buf)?;
        match &self.action {
            BossBarAction::Add {
                title,
                health,
                color,
                overlay,
                flags,
            } => {
                write_var_int(buf, 0);
                title.encode(buf)?;
                health.encode(buf)?;
                write_var_int(buf, *color as i32);
                write_var_int(buf, *overlay as i32);
                flags.encode(buf)
            }
            BossBarAction::Remove => {
                write_var_int(buf, 1);
                Ok(())
            }
            BossBarAction::UpdateHealth(h) => {
                write_var_int(buf, 2);
                h.encode(buf)
            }
            BossBarAction::UpdateTitle(t) => {
                write_var_int(buf, 3);
                t.encode(buf)
            }
            BossBarAction::UpdateStyle { color, overlay } => {
                write_var_int(buf, 4);
                write_var_int(buf, *color as i32);
                write_var_int(buf, *overlay as i32);
                Ok(())
            }
            BossBarAction::UpdateFlags(f) => {
                write_var_int(buf, 5);
                f.encode(buf)
            }
        }
    }
}

impl Decode for BossBarPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let uuid = Uuid::decode(buf)?;
        let action = match read_var_int(buf)? {
            0 => BossBarAction::Add {
                title: Component::decode(buf)?,
                health: f32::decode(buf)?,
                color: BossBarColor::from_id(read_var_int(buf)?)?,
                overlay: BossBarOverlay::from_id(read_var_int(buf)?)?,
                flags: u8::decode(buf)?,
            },
            1 => BossBarAction::Remove,
            2 => BossBarAction::UpdateHealth(f32::decode(buf)?),
            3 => BossBarAction::UpdateTitle(Component::decode(buf)?),
            4 => BossBarAction::UpdateStyle {
                color: BossBarColor::from_id(read_var_int(buf)?)?,
                overlay: BossBarOverlay::from_id(read_var_int(buf)?)?,
            },
            5 => BossBarAction::UpdateFlags(u8::decode(buf)?),
            other => {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "BossBarAction",
                    value: other,
                });
            }
        };
        Ok(Self { uuid, action })
    }
}

// ============================================================================
// Titles, ActionBar, and TabList Header/Footer
// ============================================================================

/// Clientbound `ClearTitlesPacket` (`0x0E` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClearTitlesPacket {
    /// Whether to also reset fade/stay timings to defaults.
    pub reset: bool,
}

impl PacketPayload for ClearTitlesPacket {
    const PACKET_ID: i32 = 0x0E;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClearTitlesPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.reset.encode(buf)
    }
}

impl Decode for ClearTitlesPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            reset: bool::decode(buf)?,
        })
    }
}

/// Clientbound `ActionBarPacket` (`0x57` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ActionBarPacket {
    /// Action bar text component.
    pub text: Component,
}

impl PacketPayload for ActionBarPacket {
    const PACKET_ID: i32 = 0x57;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ActionBarPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.text.encode(buf)
    }
}

impl Decode for ActionBarPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            text: Component::decode(buf)?,
        })
    }
}

/// Clientbound `SetTitleSubTitlePacket` (`0x70` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SetTitleSubTitlePacket {
    /// Subtitle text component.
    pub subtitle: Component,
}

impl PacketPayload for SetTitleSubTitlePacket {
    const PACKET_ID: i32 = 0x70;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SetTitleSubTitlePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.subtitle.encode(buf)
    }
}

impl Decode for SetTitleSubTitlePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            subtitle: Component::decode(buf)?,
        })
    }
}

/// Clientbound `SetTitleTextPacket` (`0x72` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SetTitleTextPacket {
    /// Main title text component.
    pub title: Component,
}

impl PacketPayload for SetTitleTextPacket {
    const PACKET_ID: i32 = 0x72;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SetTitleTextPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.title.encode(buf)
    }
}

impl Decode for SetTitleTextPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            title: Component::decode(buf)?,
        })
    }
}

/// Clientbound `SetTitleTimePacket` (`0x73` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct SetTitleTimePacket {
    /// Fade-in duration in ticks.
    pub fade_in: i32,
    /// Stay duration in ticks.
    pub stay: i32,
    /// Fade-out duration in ticks.
    pub fade_out: i32,
}

impl PacketPayload for SetTitleTimePacket {
    const PACKET_ID: i32 = 0x73;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SetTitleTimePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.fade_in.encode(buf)?;
        self.stay.encode(buf)?;
        self.fade_out.encode(buf)
    }
}

impl Decode for SetTitleTimePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            fade_in: i32::decode(buf)?,
            stay: i32::decode(buf)?,
            fade_out: i32::decode(buf)?,
        })
    }
}

/// Clientbound `PlayerListHeaderAndFooterPacket` (`0x7A` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct PlayerListHeaderAndFooterPacket {
    /// Tab list header component.
    pub header: Component,
    /// Tab list footer component.
    pub footer: Component,
}

impl PacketPayload for PlayerListHeaderAndFooterPacket {
    const PACKET_ID: i32 = 0x7A;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for PlayerListHeaderAndFooterPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.header.encode(buf)?;
        self.footer.encode(buf)
    }
}

impl Decode for PlayerListHeaderAndFooterPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            header: Component::decode(buf)?,
            footer: Component::decode(buf)?,
        })
    }
}

// ============================================================================
// Scoreboard & Teams (`0x4F`, `0x62`, `0x6A`, `0x6D`, `0x6E`)
// ============================================================================

/// Scoreboard score number formatting (`Sidebar.NumberFormat` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub enum SidebarNumberFormat {
    /// Hide score number completely (`0`).
    Blank,
    /// Style numeric score using `Component`'s formatting (`1`).
    Styled(Component),
    /// Replace numeric score with custom `Component` text (`2`).
    Fixed(Component),
}

impl Encode for SidebarNumberFormat {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        match self {
            Self::Blank => {
                write_var_int(buf, 0);
                Ok(())
            }
            Self::Styled(c) => {
                write_var_int(buf, 1);
                c.encode(buf)
            }
            Self::Fixed(c) => {
                write_var_int(buf, 2);
                c.encode(buf)
            }
        }
    }
}

impl Decode for SidebarNumberFormat {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        match read_var_int(buf)? {
            0 => Ok(Self::Blank),
            1 => Ok(Self::Styled(Component::decode(buf)?)),
            2 => Ok(Self::Fixed(Component::decode(buf)?)),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "SidebarNumberFormat",
                value: other,
            }),
        }
    }
}

/// Objective display render type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ObjectiveRenderType {
    /// Numeric integer scores (`0`).
    Integer = 0,
    /// Heart icons (`1`).
    Hearts = 1,
}

/// Mode inside [`ScoreboardObjectivePacket`].
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectiveAction {
    /// Create objective (`0`).
    Create {
        /// Display title.
        title: Component,
        /// Render type.
        render_type: ObjectiveRenderType,
        /// Optional default number format.
        number_format: Option<SidebarNumberFormat>,
    },
    /// Remove objective (`1`).
    Remove,
    /// Update objective title/format (`2`).
    Update {
        /// Display title.
        title: Component,
        /// Render type.
        render_type: ObjectiveRenderType,
        /// Optional default number format.
        number_format: Option<SidebarNumberFormat>,
    },
}

/// Clientbound `ScoreboardObjectivePacket` (`0x6A` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ScoreboardObjectivePacket {
    /// Internal objective identifier.
    pub objective_name: String,
    /// Create, remove, or update payload.
    pub action: ObjectiveAction,
}

impl PacketPayload for ScoreboardObjectivePacket {
    const PACKET_ID: i32 = 0x6A;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ScoreboardObjectivePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.objective_name, DEFAULT_MAX_STRING_CHARS)?;
        match &self.action {
            ObjectiveAction::Create {
                title,
                render_type,
                number_format,
            } => {
                0_i8.encode(buf)?;
                title.encode(buf)?;
                write_var_int(buf, *render_type as i32);
                number_format.encode(buf)
            }
            ObjectiveAction::Remove => 1_i8.encode(buf),
            ObjectiveAction::Update {
                title,
                render_type,
                number_format,
            } => {
                2_i8.encode(buf)?;
                title.encode(buf)?;
                write_var_int(buf, *render_type as i32);
                number_format.encode(buf)
            }
        }
    }
}

impl Decode for ScoreboardObjectivePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let objective_name = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let mode = i8::decode(buf)?;
        let action = match mode {
            0 | 2 => {
                let title = Component::decode(buf)?;
                let render_type = match read_var_int(buf)? {
                    0 => ObjectiveRenderType::Integer,
                    1 => ObjectiveRenderType::Hearts,
                    other => {
                        return Err(ProtocolError::InvalidDiscriminant {
                            type_name: "ObjectiveRenderType",
                            value: other,
                        });
                    }
                };
                let number_format = Option::<SidebarNumberFormat>::decode(buf)?;
                if mode == 0 {
                    ObjectiveAction::Create {
                        title,
                        render_type,
                        number_format,
                    }
                } else {
                    ObjectiveAction::Update {
                        title,
                        render_type,
                        number_format,
                    }
                }
            }
            1 => ObjectiveAction::Remove,
            other => {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "ObjectiveAction",
                    value: i32::from(other),
                });
            }
        };
        Ok(Self {
            objective_name,
            action,
        })
    }
}

/// Clientbound `DisplayScoreboardPacket` (`0x62` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct DisplayScoreboardPacket {
    /// Display slot (`0` = player list, `1` = sidebar, `2` = below name).
    pub position: i8,
    /// Objective name (`""` clears the slot).
    pub score_name: String,
}

impl PacketPayload for DisplayScoreboardPacket {
    const PACKET_ID: i32 = 0x62;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for DisplayScoreboardPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.position.encode(buf)?;
        write_string(buf, &self.score_name, DEFAULT_MAX_STRING_CHARS)
    }
}

impl Decode for DisplayScoreboardPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            position: i8::decode(buf)?,
            score_name: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
        })
    }
}

/// Clientbound `UpdateScorePacket` (`0x6E` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct UpdateScorePacket {
    /// Score holder identifier (player name or synthetic sidebar entry).
    pub entity_name: String,
    /// Target objective identifier.
    pub objective_name: String,
    /// Numeric score value.
    pub score: i32,
    /// Optional custom line display component (MC 1.20.3+ native sidebar line content!).
    pub display_name: Option<Component>,
    /// Optional per-entry score number format.
    pub number_format: Option<SidebarNumberFormat>,
}

impl PacketPayload for UpdateScorePacket {
    const PACKET_ID: i32 = 0x6E;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for UpdateScorePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.entity_name, DEFAULT_MAX_STRING_CHARS)?;
        write_string(buf, &self.objective_name, DEFAULT_MAX_STRING_CHARS)?;
        write_var_int(buf, self.score);
        self.display_name.encode(buf)?;
        self.number_format.encode(buf)
    }
}

impl Decode for UpdateScorePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_name: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            objective_name: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            score: read_var_int(buf)?,
            display_name: Option::<Component>::decode(buf)?,
            number_format: Option::<SidebarNumberFormat>::decode(buf)?,
        })
    }
}

/// Clientbound `ResetScorePacket` (`0x4F` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ResetScorePacket {
    /// Score holder identifier.
    pub owner: String,
    /// Specific objective name, or `None` to reset across all objectives.
    pub objective: Option<String>,
}

impl PacketPayload for ResetScorePacket {
    const PACKET_ID: i32 = 0x4F;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ResetScorePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.owner, DEFAULT_MAX_STRING_CHARS)?;
        match &self.objective {
            Some(obj) => {
                true.encode(buf)?;
                write_string(buf, obj, DEFAULT_MAX_STRING_CHARS)
            }
            None => false.encode(buf),
        }
    }
}

impl Decode for ResetScorePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let owner = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let objective = if bool::decode(buf)? {
            Some(read_string(buf, DEFAULT_MAX_STRING_CHARS)?)
        } else {
            None
        };
        Ok(Self { owner, objective })
    }
}

/// Legacy 16-color team color enum (`TeamColor` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TeamColor {
    /// Black (`0`).
    Black = 0,
    /// Dark blue (`1`).
    DarkBlue = 1,
    /// Dark green (`2`).
    DarkGreen = 2,
    /// Dark aqua (`3`).
    DarkAqua = 3,
    /// Dark red (`4`).
    DarkRed = 4,
    /// Dark purple (`5`).
    DarkPurple = 5,
    /// Gold (`6`).
    Gold = 6,
    /// Gray (`7`).
    Gray = 7,
    /// Dark gray (`8`).
    DarkGray = 8,
    /// Blue (`9`).
    Blue = 9,
    /// Green (`10`).
    Green = 10,
    /// Aqua (`11`).
    Aqua = 11,
    /// Red (`12`).
    Red = 12,
    /// Light purple (`13`).
    LightPurple = 13,
    /// Yellow (`14`).
    Yellow = 14,
    /// White (`15`).
    White = 15,
}

impl TeamColor {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Black),
            1 => Ok(Self::DarkBlue),
            2 => Ok(Self::DarkGreen),
            3 => Ok(Self::DarkAqua),
            4 => Ok(Self::DarkRed),
            5 => Ok(Self::DarkPurple),
            6 => Ok(Self::Gold),
            7 => Ok(Self::Gray),
            8 => Ok(Self::DarkGray),
            9 => Ok(Self::Blue),
            10 => Ok(Self::Green),
            11 => Ok(Self::Aqua),
            12 => Ok(Self::Red),
            13 => Ok(Self::LightPurple),
            14 => Ok(Self::Yellow),
            15 => Ok(Self::White),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "TeamColor",
                value: other,
            }),
        }
    }
}

/// Name-tag visibility rule on a [`TeamSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum NameTagVisibility {
    /// Always visible (`0`).
    Always = 0,
    /// Never visible (`1`).
    Never = 1,
    /// Hidden for other teams (`2`).
    HideForOtherTeams = 2,
    /// Hidden for own team (`3`).
    HideForOwnTeam = 3,
}

impl NameTagVisibility {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Always),
            1 => Ok(Self::Never),
            2 => Ok(Self::HideForOtherTeams),
            3 => Ok(Self::HideForOwnTeam),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "NameTagVisibility",
                value: other,
            }),
        }
    }
}

/// Entity push collision rule on a [`TeamSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum CollisionRule {
    /// Always push (`0`).
    Always = 0,
    /// Never push (`1`).
    Never = 1,
    /// Push other teams (`2`).
    PushOtherTeams = 2,
    /// Push own team (`3`).
    PushOwnTeam = 3,
}

impl CollisionRule {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Always),
            1 => Ok(Self::Never),
            2 => Ok(Self::PushOtherTeams),
            3 => Ok(Self::PushOwnTeam),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "CollisionRule",
                value: other,
            }),
        }
    }
}

/// Display and rule settings of a scoreboard team (`TeamsPacket.Settings` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct TeamSettings {
    /// Team display name.
    pub display_name: Component,
    /// Prefix prepended before member names.
    pub prefix: Component,
    /// Suffix appended after member names.
    pub suffix: Component,
    /// Name-tag visibility rule.
    pub name_tag_visibility: NameTagVisibility,
    /// Entity collision rule.
    pub collision_rule: CollisionRule,
    /// Optional team glow/name color (`None` = reset/no color, encoded as `21` in `VarInt`? Wait:
    /// in MC 26.2 `TeamColor.NETWORK_TYPE.optional()` encodes `bool` + `VarInt`! Verified from `TeamsPacket.java` line 794!).
    pub color: Option<TeamColor>,
    /// Friendly flags (`0x01` allow friendly fire, `0x02` see invisible teammates).
    pub friendly_flags: u8,
}

impl Default for TeamSettings {
    fn default() -> Self {
        Self {
            display_name: Component::empty(),
            prefix: Component::empty(),
            suffix: Component::empty(),
            name_tag_visibility: NameTagVisibility::Always,
            collision_rule: CollisionRule::Always,
            color: Some(TeamColor::White),
            friendly_flags: 0x03,
        }
    }
}

impl Encode for TeamSettings {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.display_name.encode(buf)?;
        self.prefix.encode(buf)?;
        self.suffix.encode(buf)?;
        write_var_int(buf, self.name_tag_visibility as i32);
        write_var_int(buf, self.collision_rule as i32);
        match self.color {
            Some(c) => {
                true.encode(buf)?;
                write_var_int(buf, c as i32);
            }
            None => false.encode(buf)?,
        }
        self.friendly_flags.encode(buf)
    }
}

impl Decode for TeamSettings {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let display_name = Component::decode(buf)?;
        let prefix = Component::decode(buf)?;
        let suffix = Component::decode(buf)?;
        let name_tag_visibility = NameTagVisibility::from_id(read_var_int(buf)?)?;
        let collision_rule = CollisionRule::from_id(read_var_int(buf)?)?;
        let color = if bool::decode(buf)? {
            Some(TeamColor::from_id(read_var_int(buf)?)?)
        } else {
            None
        };
        let friendly_flags = u8::decode(buf)?;
        Ok(Self {
            display_name,
            prefix,
            suffix,
            name_tag_visibility,
            collision_rule,
            color,
            friendly_flags,
        })
    }
}

fn write_string_list<B: BufMut>(buf: &mut B, items: &[String]) -> Result<(), ProtocolError> {
    if items.len() > 16_384 {
        return Err(ProtocolError::InvalidLength {
            len: items.len() as i32,
            max: 16_384,
        });
    }
    write_var_int(buf, items.len() as i32);
    for s in items {
        write_string(buf, s, DEFAULT_MAX_STRING_CHARS)?;
    }
    Ok(())
}

fn read_string_list<B: Buf>(buf: &mut B, max: usize) -> Result<Vec<String>, ProtocolError> {
    let len = read_var_int(buf)?;
    if len < 0 || (len as usize) > max {
        return Err(ProtocolError::InvalidLength { len, max });
    }
    let mut out = Vec::with_capacity((len as usize).min(256));
    for _ in 0..len {
        out.push(read_string(buf, DEFAULT_MAX_STRING_CHARS)?);
    }
    Ok(out)
}

/// Action inside [`TeamsPacket`].
#[derive(Debug, Clone, PartialEq)]
pub enum TeamsAction {
    /// Create team and add initial members (`0`).
    Create {
        /// Team display & rule settings.
        settings: TeamSettings,
        /// Initial member identifiers (usernames or entity UUID strings).
        entities: Vec<String>,
    },
    /// Remove team (`1`).
    Remove,
    /// Update team settings (`2`).
    Update(TeamSettings),
    /// Add members to team (`3`).
    AddEntities(Vec<String>),
    /// Remove members from team (`4`).
    RemoveEntities(Vec<String>),
}

/// Clientbound `TeamsPacket` (`0x6D` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct TeamsPacket {
    /// Unique team identifier.
    pub team_name: String,
    /// Team mutation action.
    pub action: TeamsAction,
}

impl PacketPayload for TeamsPacket {
    const PACKET_ID: i32 = 0x6D;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for TeamsPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.team_name, DEFAULT_MAX_STRING_CHARS)?;
        match &self.action {
            TeamsAction::Create { settings, entities } => {
                0_i8.encode(buf)?;
                settings.encode(buf)?;
                write_string_list(buf, entities)
            }
            TeamsAction::Remove => 1_i8.encode(buf),
            TeamsAction::Update(settings) => {
                2_i8.encode(buf)?;
                settings.encode(buf)
            }
            TeamsAction::AddEntities(entities) => {
                3_i8.encode(buf)?;
                write_string_list(buf, entities)
            }
            TeamsAction::RemoveEntities(entities) => {
                4_i8.encode(buf)?;
                write_string_list(buf, entities)
            }
        }
    }
}

impl Decode for TeamsPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let team_name = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let action = match i8::decode(buf)? {
            0 => TeamsAction::Create {
                settings: TeamSettings::decode(buf)?,
                entities: read_string_list(buf, 16_384)?,
            },
            1 => TeamsAction::Remove,
            2 => TeamsAction::Update(TeamSettings::decode(buf)?),
            3 => TeamsAction::AddEntities(read_string_list(buf, 16_384)?),
            4 => TeamsAction::RemoveEntities(read_string_list(buf, 16_384)?),
            other => {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "TeamsAction",
                    value: i32::from(other),
                });
            }
        };
        Ok(Self { team_name, action })
    }
}

// ============================================================================
// Particles (`0x2F` Play — exact MC 26.2 wire order!)
// ============================================================================

/// Extra data payload carried by parameterized MC 26.2 particle types.
#[derive(Debug, Clone, PartialEq)]
pub enum ParticleData {
    /// Simple particle with no extra payload.
    None,
    /// Block state ID (`block`, `block_marker`, `falling_dust`, `dust_pillar`, `block_crumble`).
    BlockState(u16),
    /// RGB + scale (`dust`, `effect`, `instant_effect`).
    RgbScale {
        /// 24-bit RGB integer (`0xRRGGBB`).
        rgb: i32,
        /// Scale or power scalar.
        scale: f32,
    },
    /// Dust color transition (`dust_color_transition`).
    DustColorTransition {
        /// Start 24-bit RGB.
        from_rgb: i32,
        /// End 24-bit RGB.
        to_rgb: i32,
        /// Scale.
        scale: f32,
    },
    /// 32-bit ARGB color (`entity_effect`, `tinted_leaves`, `flash`).
    ArgbColor(i32),
    /// Single float scalar (`dragon_breath`, `sculk_charge`).
    ScalarFloat(f32),
    /// Single `VarInt` (`shriek` delay).
    VarIntValue(i32),
    /// Single `i32` (`geyser`, `geyser_plume` water blocks).
    IntValue(i32),
    /// `i32` + `f32` (`geyser_base`, `geyser_poof`).
    IntAndFloat(i32, f32),
    /// Item stack (`item`).
    Item(ItemStack),
    /// Trail particle (`trail`: target `Vec3`, `i32` RGB, `VarInt` duration).
    Trail {
        /// Target coordinate.
        target: Vec3,
        /// 24-bit RGB color.
        rgb: i32,
        /// Duration ticks.
        duration: i32,
    },
    /// Vibration particle (`vibration`).
    VibrationBlock {
        /// Destination block position.
        block_pos: BlockVec,
        /// Travel ticks.
        travel_ticks: i32,
    },
    /// Entity-targeted vibration (`vibration`).
    VibrationEntity {
        /// Target entity ID.
        entity_id: i32,
        /// Eye height offset.
        eye_height: f32,
        /// Travel ticks.
        travel_ticks: i32,
    },
}

/// Clientbound `ParticlePacket` (`0x2F` in [`ConnectionState::Play`], MC 26.2).
///
/// Note: On the MC 26.2 wire, `override_limiter`, `long_distance`, `position`, `offset`,
/// `max_speed`, and `particle_count` precede `particle_id` and `data`!
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ParticlePacket {
    /// Particle protocol ID (`0..125`, `minestom_data::generated::particles`).
    pub particle_id: u32,
    /// Optional particle-specific extra data.
    pub data: ParticleData,
    /// Bypass client minimal-particle limiter.
    pub override_limiter: bool,
    /// Render up to 512 blocks away instead of 32.
    pub long_distance: bool,
    /// World spawn origin.
    pub position: Vec3,
    /// Gaussian spread offset `(dx, dy, dz)`.
    pub offset: (f32, f32, f32),
    /// Maximum particle speed scalar.
    pub max_speed: f32,
    /// Number of particles to spawn.
    pub particle_count: i32,
}

impl ParticlePacket {
    /// Constructs a simple (data-free) [`ParticlePacket`].
    #[must_use]
    pub const fn simple(
        particle_id: u32,
        position: Vec3,
        offset: (f32, f32, f32),
        max_speed: f32,
        particle_count: i32,
    ) -> Self {
        Self {
            particle_id,
            data: ParticleData::None,
            override_limiter: false,
            long_distance: false,
            position,
            offset,
            max_speed,
            particle_count,
        }
    }
}

impl PacketPayload for ParticlePacket {
    const PACKET_ID: i32 = 0x2F;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ParticlePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.override_limiter.encode(buf)?;
        self.long_distance.encode(buf)?;
        self.position.x.encode(buf)?;
        self.position.y.encode(buf)?;
        self.position.z.encode(buf)?;
        self.offset.0.encode(buf)?;
        self.offset.1.encode(buf)?;
        self.offset.2.encode(buf)?;
        self.max_speed.encode(buf)?;
        self.particle_count.encode(buf)?;
        write_var_int(buf, self.particle_id as i32);
        match &self.data {
            ParticleData::None => Ok(()),
            ParticleData::BlockState(state) => {
                write_var_int(buf, i32::from(*state));
                Ok(())
            }
            ParticleData::RgbScale { rgb, scale } => {
                rgb.encode(buf)?;
                scale.encode(buf)
            }
            ParticleData::DustColorTransition {
                from_rgb,
                to_rgb,
                scale,
            } => {
                from_rgb.encode(buf)?;
                to_rgb.encode(buf)?;
                scale.encode(buf)
            }
            ParticleData::ArgbColor(argb) => argb.encode(buf),
            ParticleData::ScalarFloat(v) => v.encode(buf),
            ParticleData::VarIntValue(v) => {
                write_var_int(buf, *v);
                Ok(())
            }
            ParticleData::IntValue(v) => v.encode(buf),
            ParticleData::IntAndFloat(i, f) => {
                i.encode(buf)?;
                f.encode(buf)
            }
            ParticleData::Item(stack) => stack.encode(buf),
            ParticleData::Trail {
                target,
                rgb,
                duration,
            } => {
                target.x.encode(buf)?;
                target.y.encode(buf)?;
                target.z.encode(buf)?;
                rgb.encode(buf)?;
                write_var_int(buf, *duration);
                Ok(())
            }
            ParticleData::VibrationBlock {
                block_pos,
                travel_ticks,
            } => {
                write_var_int(buf, 0);
                block_pos.encode(buf)?;
                write_var_int(buf, *travel_ticks);
                Ok(())
            }
            ParticleData::VibrationEntity {
                entity_id,
                eye_height,
                travel_ticks,
            } => {
                write_var_int(buf, 1);
                write_var_int(buf, *entity_id);
                eye_height.encode(buf)?;
                write_var_int(buf, *travel_ticks);
                Ok(())
            }
        }
    }
}

impl Decode for ParticlePacket {
    #[allow(clippy::too_many_lines)]
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let override_limiter = bool::decode(buf)?;
        let long_distance = bool::decode(buf)?;
        let position = Vec3::new(f64::decode(buf)?, f64::decode(buf)?, f64::decode(buf)?);
        let offset = (f32::decode(buf)?, f32::decode(buf)?, f32::decode(buf)?);
        let max_speed = f32::decode(buf)?;
        let particle_count = i32::decode(buf)?;
        let raw_id = read_var_int(buf)?;
        if !(0..125).contains(&raw_id) {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "Particle",
                value: raw_id,
            });
        }
        let particle_id = raw_id as u32;
        let data = match particle_id {
            1 | 2 | 36 | 118 | 122 => {
                let st = read_var_int(buf)?;
                let st_u16 = u16::try_from(st).map_err(|_| ProtocolError::InvalidDiscriminant {
                    type_name: "BlockState",
                    value: st,
                })?;
                ParticleData::BlockState(st_u16)
            }
            7 | 10 => ParticleData::IntValue(i32::decode(buf)?),
            8 | 9 => ParticleData::IntAndFloat(i32::decode(buf)?, f32::decode(buf)?),
            15 | 45 => ParticleData::ScalarFloat(f32::decode(buf)?),
            21 | 23 | 53 => ParticleData::RgbScale {
                rgb: i32::decode(buf)?,
                scale: f32::decode(buf)?,
            },
            22 => ParticleData::DustColorTransition {
                from_rgb: i32::decode(buf)?,
                to_rgb: i32::decode(buf)?,
                scale: f32::decode(buf)?,
            },
            28 | 43 | 49 => ParticleData::ArgbColor(i32::decode(buf)?),
            54 => ParticleData::Item(ItemStack::decode(buf)?),
            55 => match read_var_int(buf)? {
                0 => ParticleData::VibrationBlock {
                    block_pos: BlockVec::decode(buf)?,
                    travel_ticks: read_var_int(buf)?,
                },
                1 => ParticleData::VibrationEntity {
                    entity_id: read_var_int(buf)?,
                    eye_height: f32::decode(buf)?,
                    travel_ticks: read_var_int(buf)?,
                },
                other => {
                    return Err(ProtocolError::InvalidDiscriminant {
                        type_name: "VibrationSource",
                        value: other,
                    });
                }
            },
            56 => ParticleData::Trail {
                target: Vec3::new(f64::decode(buf)?, f64::decode(buf)?, f64::decode(buf)?),
                rgb: i32::decode(buf)?,
                duration: read_var_int(buf)?,
            },
            112 => ParticleData::VarIntValue(read_var_int(buf)?),
            _ => ParticleData::None,
        };
        Ok(Self {
            particle_id,
            data,
            override_limiter,
            long_distance,
            position,
            offset,
            max_speed,
            particle_count,
        })
    }
}

// ============================================================================
// Sounds (`0x75`, `0x77` Play)
// ============================================================================

/// Audio mixer category (`Sound.Source` in Adventure / `sound_sources.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SoundSource {
    /// Master (`0`).
    Master = 0,
    /// Music (`1`).
    Music = 1,
    /// Jukebox/records (`2`).
    Record = 2,
    /// Weather (`3`).
    Weather = 3,
    /// Blocks (`4`).
    Block = 4,
    /// Hostile mobs (`5`).
    Hostile = 5,
    /// Neutral mobs (`6`).
    Neutral = 6,
    /// Players (`7`).
    Player = 7,
    /// Ambient (`8`).
    Ambient = 8,
    /// Voice (`9`).
    Voice = 9,
    /// UI (`10`).
    Ui = 10,
}

impl SoundSource {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Master),
            1 => Ok(Self::Music),
            2 => Ok(Self::Record),
            3 => Ok(Self::Weather),
            4 => Ok(Self::Block),
            5 => Ok(Self::Hostile),
            6 => Ok(Self::Neutral),
            7 => Ok(Self::Player),
            8 => Ok(Self::Ambient),
            9 => Ok(Self::Voice),
            10 => Ok(Self::Ui),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "SoundSource",
                value: other,
            }),
        }
    }
}

/// Builtin or custom resource-pack `SoundEvent` reference (`SoundEvent.NETWORK_TYPE` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub enum SoundHolder {
    /// Builtin `minecraft:sound_event` protocol ID (`0..1968`, encoded on wire as `id + 1`).
    Builtin(u32),
    /// Custom resource-pack sound key (`VarInt 0` + `key` + optional fixed attenuation `range`).
    Custom {
        /// Sound resource key.
        key: String,
        /// Optional fixed attenuation distance.
        range: Option<f32>,
    },
}

impl Encode for SoundHolder {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        match self {
            Self::Builtin(id) => {
                write_var_int(buf, (*id as i32).saturating_add(1));
                Ok(())
            }
            Self::Custom { key, range } => {
                write_var_int(buf, 0);
                write_string(buf, key, DEFAULT_MAX_STRING_CHARS)?;
                range.encode(buf)
            }
        }
    }
}

impl Decode for SoundHolder {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let raw = read_var_int(buf)?;
        if raw < 0 {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "SoundHolder",
                value: raw,
            });
        }
        if raw == 0 {
            Ok(Self::Custom {
                key: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
                range: Option::<f32>::decode(buf)?,
            })
        } else {
            Ok(Self::Builtin((raw - 1) as u32))
        }
    }
}

/// Clientbound `SoundEffectPacket` (`0x75` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SoundEffectPacket {
    /// Sound event holder.
    pub sound: SoundHolder,
    /// Mixer category.
    pub source: SoundSource,
    /// Fixed-point `x * 8` coordinate.
    pub x_fixed8: i32,
    /// Fixed-point `y * 8` coordinate.
    pub y_fixed8: i32,
    /// Fixed-point `z * 8` coordinate.
    pub z_fixed8: i32,
    /// Volume (`1.0` = normal).
    pub volume: f32,
    /// Pitch (`1.0` = normal).
    pub pitch: f32,
    /// Variation seed.
    pub seed: i64,
}

impl SoundEffectPacket {
    /// Constructs a [`SoundEffectPacket`] at world `origin`.
    #[must_use]
    pub fn at(
        sound: SoundHolder,
        source: SoundSource,
        origin: Vec3,
        volume: f32,
        pitch: f32,
        seed: i64,
    ) -> Self {
        Self {
            sound,
            source,
            x_fixed8: (origin.x * 8.0) as i32,
            y_fixed8: (origin.y * 8.0) as i32,
            z_fixed8: (origin.z * 8.0) as i32,
            volume,
            pitch,
            seed,
        }
    }
}

impl PacketPayload for SoundEffectPacket {
    const PACKET_ID: i32 = 0x75;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SoundEffectPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.sound.encode(buf)?;
        write_var_int(buf, self.source as i32);
        self.x_fixed8.encode(buf)?;
        self.y_fixed8.encode(buf)?;
        self.z_fixed8.encode(buf)?;
        self.volume.encode(buf)?;
        self.pitch.encode(buf)?;
        self.seed.encode(buf)
    }
}

impl Decode for SoundEffectPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            sound: SoundHolder::decode(buf)?,
            source: SoundSource::from_id(read_var_int(buf)?)?,
            x_fixed8: i32::decode(buf)?,
            y_fixed8: i32::decode(buf)?,
            z_fixed8: i32::decode(buf)?,
            volume: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
            seed: i64::decode(buf)?,
        })
    }
}

/// Clientbound `StopSoundPacket` (`0x77` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct StopSoundPacket {
    /// Optional category filter.
    pub source: Option<SoundSource>,
    /// Optional sound key filter.
    pub sound: Option<String>,
}

impl PacketPayload for StopSoundPacket {
    const PACKET_ID: i32 = 0x77;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for StopSoundPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        let mut flags = 0_u8;
        if self.source.is_some() {
            flags |= 0x01;
        }
        if self.sound.is_some() {
            flags |= 0x02;
        }
        flags.encode(buf)?;
        if let Some(src) = self.source {
            write_var_int(buf, src as i32);
        }
        if let Some(snd) = &self.sound {
            write_string(buf, snd, DEFAULT_MAX_STRING_CHARS)?;
        }
        Ok(())
    }
}

impl Decode for StopSoundPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let flags = u8::decode(buf)?;
        if flags > 0x03 {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "StopSoundFlags",
                value: i32::from(flags),
            });
        }
        let source = if (flags & 0x01) != 0 {
            Some(SoundSource::from_id(read_var_int(buf)?)?)
        } else {
            None
        };
        let sound = if (flags & 0x02) != 0 {
            Some(read_string(buf, DEFAULT_MAX_STRING_CHARS)?)
        } else {
            None
        };
        Ok(Self { source, sound })
    }
}

// ============================================================================
// Resource Packs (`0x50` / `0x51` / `0x31` Play, `0x08` / `0x09` / `0x06` Config)
// ============================================================================

/// Client resource pack download/load status (`ResourcePackStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ResourcePackStatus {
    /// Pack loaded and applied (`0`).
    SuccessfullyLoaded = 0,
    /// User declined prompt (`1`).
    Declined = 1,
    /// Download HTTP/hash failed (`2`).
    FailedDownload = 2,
    /// User accepted prompt (`3`).
    Accepted = 3,
    /// Download finished (`4`).
    Downloaded = 4,
    /// Malformed URL (`5`).
    InvalidUrl = 5,
    /// Client reload failed (`6`).
    FailedReload = 6,
    /// Discarded by server/client (`7`).
    Discarded = 7,
}

impl ResourcePackStatus {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::SuccessfullyLoaded),
            1 => Ok(Self::Declined),
            2 => Ok(Self::FailedDownload),
            3 => Ok(Self::Accepted),
            4 => Ok(Self::Downloaded),
            5 => Ok(Self::InvalidUrl),
            6 => Ok(Self::FailedReload),
            7 => Ok(Self::Discarded),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "ResourcePackStatus",
                value: other,
            }),
        }
    }
}

/// Clientbound `ResourcePackPushPacket` (`0x51` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ResourcePackPushPacket {
    /// Pack UUID.
    pub id: Uuid,
    /// HTTPS download URL.
    pub url: String,
    /// 40-char lowercase SHA-1 hex string.
    pub hash: String,
    /// Disconnect player if declined.
    pub forced: bool,
    /// Optional custom prompt message.
    pub prompt: Option<Component>,
}

impl PacketPayload for ResourcePackPushPacket {
    const PACKET_ID: i32 = 0x51;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ResourcePackPushPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)?;
        write_string(buf, &self.url, DEFAULT_MAX_STRING_CHARS)?;
        write_string(buf, &self.hash, 40)?;
        self.forced.encode(buf)?;
        self.prompt.encode(buf)
    }
}

impl Decode for ResourcePackPushPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: Uuid::decode(buf)?,
            url: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            hash: read_string(buf, 40)?,
            forced: bool::decode(buf)?,
            prompt: Option::<Component>::decode(buf)?,
        })
    }
}

/// Clientbound `ResourcePackPopPacket` (`0x50` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ResourcePackPopPacket {
    /// Specific pack UUID to unload, or `None` to pop all server packs.
    pub id: Option<Uuid>,
}

impl PacketPayload for ResourcePackPopPacket {
    const PACKET_ID: i32 = 0x50;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ResourcePackPopPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)
    }
}

impl Decode for ResourcePackPopPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: Option::<Uuid>::decode(buf)?,
        })
    }
}

/// Serverbound `ClientResourcePackStatusPacket` (`0x31` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientResourcePackStatusPacket {
    /// Pack UUID.
    pub id: Uuid,
    /// Reported status.
    pub status: ResourcePackStatus,
}

impl PacketPayload for ClientResourcePackStatusPacket {
    const PACKET_ID: i32 = 0x31;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientResourcePackStatusPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)?;
        write_var_int(buf, self.status as i32);
        Ok(())
    }
}

impl Decode for ClientResourcePackStatusPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: Uuid::decode(buf)?,
            status: ResourcePackStatus::from_id(read_var_int(buf)?)?,
        })
    }
}

// ============================================================================
// Advancements & Toast Notifications (`0x82` Play)
// ============================================================================

/// Border frame style of an advancement icon (`FrameType` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum AdvancementFrameType {
    /// Standard square task frame (`0`).
    Task = 0,
    /// Spiked challenge frame (`1`).
    Challenge = 1,
    /// Rounded goal frame (`2`).
    Goal = 2,
}

impl AdvancementFrameType {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Task),
            1 => Ok(Self::Challenge),
            2 => Ok(Self::Goal),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "AdvancementFrameType",
                value: other,
            }),
        }
    }
}

/// Visual display info inside an [`AdvancementEntry`] (`AdvancementsPacket.DisplayData` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct AdvancementDisplayData {
    /// Title component.
    pub title: Component,
    /// Description component.
    pub description: Component,
    /// Icon item stack (`ItemStackTemplate`: non-empty item stack!).
    pub icon: ItemStack,
    /// Frame border style.
    pub frame_type: AdvancementFrameType,
    /// Bitflags (`0x01` has background texture, `0x02` show toast, `0x04` hidden).
    pub flags: i32,
    /// Optional tab background texture key (present iff `(flags & 0x01) != 0`).
    pub background_texture: Option<String>,
    /// Tree X coordinate.
    pub x: f32,
    /// Tree Y coordinate.
    pub y: f32,
}

impl Encode for AdvancementDisplayData {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.title.encode(buf)?;
        self.description.encode(buf)?;
        self.icon.encode(buf)?;
        write_var_int(buf, self.frame_type as i32);
        let effective_flags = if self.background_texture.is_some() {
            self.flags | 0x01
        } else {
            self.flags & !0x01
        };
        effective_flags.encode(buf)?;
        if let Some(bg) = &self.background_texture {
            write_string(buf, bg, DEFAULT_MAX_STRING_CHARS)?;
        }
        self.x.encode(buf)?;
        self.y.encode(buf)
    }
}

impl Decode for AdvancementDisplayData {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let title = Component::decode(buf)?;
        let description = Component::decode(buf)?;
        let icon = ItemStack::decode(buf)?;
        let frame_type = AdvancementFrameType::from_id(read_var_int(buf)?)?;
        let flags = i32::decode(buf)?;
        let background_texture = if (flags & 0x01) != 0 {
            Some(read_string(buf, DEFAULT_MAX_STRING_CHARS)?)
        } else {
            None
        };
        let x = f32::decode(buf)?;
        let y = f32::decode(buf)?;
        Ok(Self {
            title,
            description,
            icon,
            frame_type,
            flags,
            background_texture,
            x,
            y,
        })
    }
}

/// Single advancement definition inside [`AdvancementsPacket`].
#[derive(Debug, Clone, PartialEq)]
pub struct AdvancementEntry {
    /// Identifier key (e.g. `"minestom:welcome"`).
    pub key: String,
    /// Optional parent advancement key.
    pub parent: Option<String>,
    /// Optional display data.
    pub display: Option<AdvancementDisplayData>,
    /// Conjunctive list of disjunctive criterion groups (`Vec<Vec<String>>`).
    pub requirements: Vec<Vec<String>>,
    /// Telemetry flag.
    pub send_telemetry: bool,
}

impl Encode for AdvancementEntry {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.key, DEFAULT_MAX_STRING_CHARS)?;
        match &self.parent {
            Some(p) => {
                true.encode(buf)?;
                write_string(buf, p, DEFAULT_MAX_STRING_CHARS)?;
            }
            None => false.encode(buf)?,
        }
        self.display.encode(buf)?;
        write_var_int(buf, self.requirements.len() as i32);
        for group in &self.requirements {
            write_string_list(buf, group)?;
        }
        self.send_telemetry.encode(buf)
    }
}

impl Decode for AdvancementEntry {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let key = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let parent = if bool::decode(buf)? {
            Some(read_string(buf, DEFAULT_MAX_STRING_CHARS)?)
        } else {
            None
        };
        let display = Option::<AdvancementDisplayData>::decode(buf)?;
        let req_len = read_var_int(buf)?;
        if !(0..=1024).contains(&req_len) {
            return Err(ProtocolError::InvalidLength {
                len: req_len,
                max: 1024,
            });
        }
        let mut requirements = Vec::with_capacity(req_len as usize);
        for _ in 0..req_len {
            requirements.push(read_string_list(buf, 1024)?);
        }
        let send_telemetry = bool::decode(buf)?;
        Ok(Self {
            key,
            parent,
            display,
            requirements,
            send_telemetry,
        })
    }
}

/// Criterion completion map for an advancement inside [`AdvancementsPacket`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancementProgressEntry {
    /// Advancement identifier key.
    pub key: String,
    /// `(criterion_id, achieved_epoch_millis)` pairs.
    pub criteria: Vec<(String, Option<i64>)>,
}

impl Encode for AdvancementProgressEntry {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.key, DEFAULT_MAX_STRING_CHARS)?;
        write_var_int(buf, self.criteria.len() as i32);
        for (cid, achieved) in &self.criteria {
            write_string(buf, cid, DEFAULT_MAX_STRING_CHARS)?;
            achieved.encode(buf)?;
        }
        Ok(())
    }
}

impl Decode for AdvancementProgressEntry {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let key = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let len = read_var_int(buf)?;
        if !(0..=1024).contains(&len) {
            return Err(ProtocolError::InvalidLength { len, max: 1024 });
        }
        let mut criteria = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let cid = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
            let ts = Option::<i64>::decode(buf)?;
            criteria.push((cid, ts));
        }
        Ok(Self { key, criteria })
    }
}

/// Clientbound `AdvancementsPacket` (`0x82` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct AdvancementsPacket {
    /// Clear existing client advancements first.
    pub reset: bool,
    /// Added/updated advancements.
    pub advancements: Vec<AdvancementEntry>,
    /// Advancement keys to remove.
    pub identifiers_to_remove: Vec<String>,
    /// Criterion progress updates.
    pub progress: Vec<AdvancementProgressEntry>,
    /// Show advancements in pause menu (MC 26.2).
    pub show_advancements: bool,
}

impl PacketPayload for AdvancementsPacket {
    const PACKET_ID: i32 = 0x82;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for AdvancementsPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.reset.encode(buf)?;
        write_prefixed_vec(buf, &self.advancements, 4096)?;
        write_string_list(buf, &self.identifiers_to_remove)?;
        write_prefixed_vec(buf, &self.progress, 4096)?;
        self.show_advancements.encode(buf)
    }
}

impl Decode for AdvancementsPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            reset: bool::decode(buf)?,
            advancements: read_prefixed_vec(buf, 4096)?,
            identifiers_to_remove: read_string_list(buf, 4096)?,
            progress: read_prefixed_vec(buf, 4096)?,
            show_advancements: bool::decode(buf)?,
        })
    }
}

// ============================================================================
// Recipes (`0x85` Play — MC 26.2 DeclareRecipesPacket)
// ============================================================================

/// Stonecutter recipe entry inside [`DeclareRecipesPacket`].
#[derive(Debug, Clone, PartialEq)]
pub struct StonecutterRecipeEntry {
    /// Accepted input item protocol IDs (encoded as inline `RegistryTag`: `VarInt(len + 1)` + IDs).
    pub input_item_ids: Vec<u32>,
    /// Result item stack (`SlotDisplay.ItemStack` = discriminant `5` + `ItemStack`).
    pub result: ItemStack,
}

impl Encode for StonecutterRecipeEntry {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        // Inline RegistryTag.direct: VarInt(size + 1) followed by VarInt item IDs
        write_var_int(buf, (self.input_item_ids.len() as i32).saturating_add(1));
        for &id in &self.input_item_ids {
            write_var_int(buf, id as i32);
        }
        // SlotDisplay::ItemStack (`5` in `slot_display_types.json`)
        write_var_int(buf, 5);
        self.result.encode(buf)
    }
}

impl Decode for StonecutterRecipeEntry {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let raw_tag_len = read_var_int(buf)?;
        if raw_tag_len <= 0 || raw_tag_len > 4097 {
            return Err(ProtocolError::InvalidLength {
                len: raw_tag_len,
                max: 4096,
            });
        }
        let count = (raw_tag_len - 1) as usize;
        let mut input_item_ids = Vec::with_capacity(count.min(256));
        for _ in 0..count {
            let id = read_var_int(buf)?;
            if id < 0 {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "Material",
                    value: id,
                });
            }
            input_item_ids.push(id as u32);
        }
        let display_type = read_var_int(buf)?;
        let result = match display_type {
            4 => {
                let mat_id = read_var_int(buf)?;
                let mat = minestom_item::Material::from_id(mat_id as u32).map_err(|_| {
                    ProtocolError::InvalidDiscriminant {
                        type_name: "Material",
                        value: mat_id,
                    }
                })?;
                ItemStack::single(mat)
            }
            5 => ItemStack::decode(buf)?,
            other => {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "SlotDisplayType",
                    value: other,
                });
            }
        };
        Ok(Self {
            input_item_ids,
            result,
        })
    }
}

/// Clientbound `DeclareRecipesPacket` (`0x85` in [`ConnectionState::Play`], MC 26.2).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct DeclareRecipesPacket {
    /// Map from `RecipeProperty` minimal key (e.g. `"furnace_input"`, `"smithing_base"`)
    /// to accepted `Material` protocol IDs.
    pub item_properties: BTreeMap<String, Vec<u32>>,
    /// Stonecutter recipes.
    pub stonecutter_recipes: Vec<StonecutterRecipeEntry>,
}

impl PacketPayload for DeclareRecipesPacket {
    const PACKET_ID: i32 = 0x85;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for DeclareRecipesPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.item_properties.len() as i32);
        for (prop_key, items) in &self.item_properties {
            write_string(buf, prop_key, DEFAULT_MAX_STRING_CHARS)?;
            write_var_int(buf, items.len() as i32);
            for &id in items {
                write_var_int(buf, id as i32);
            }
        }
        write_prefixed_vec(buf, &self.stonecutter_recipes, 4096)
    }
}

impl Decode for DeclareRecipesPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let map_len = read_var_int(buf)?;
        if !(0..=32).contains(&map_len) {
            return Err(ProtocolError::InvalidLength {
                len: map_len,
                max: 32,
            });
        }
        let mut item_properties = BTreeMap::new();
        for _ in 0..map_len {
            let key = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
            let list_len = read_var_int(buf)?;
            if !(0..=4096).contains(&list_len) {
                return Err(ProtocolError::InvalidLength {
                    len: list_len,
                    max: 4096,
                });
            }
            let mut ids = Vec::with_capacity((list_len as usize).min(256));
            for _ in 0..list_len {
                let id = read_var_int(buf)?;
                if id < 0 {
                    return Err(ProtocolError::InvalidDiscriminant {
                        type_name: "Material",
                        value: id,
                    });
                }
                ids.push(id as u32);
            }
            item_properties.insert(key, ids);
        }
        let stonecutter_recipes = read_prefixed_vec(buf, 4096)?;
        Ok(Self {
            item_properties,
            stonecutter_recipes,
        })
    }
}

// ============================================================================
// Dialogs (`0x8B` / `0x8C` Play)
// ============================================================================

/// Registry-referenced or inline-NBT MC 26.2 `Holder<Dialog>`.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogHolder {
    /// Entry index in `minecraft:dialog` dynamic registry (`VarInt(id + 1)` on wire).
    RegistryId(u32),
    /// Inline nameless network NBT compound (`VarInt(0)` + NBT on wire).
    Direct(NbtTag),
}

/// Clientbound `ClearDialogPacket` (`0x8B` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PacketMeta)]
pub struct ClearDialogPacket;

impl PacketPayload for ClearDialogPacket {
    const PACKET_ID: i32 = 0x8B;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClearDialogPacket {
    fn encode<B: BufMut>(&self, _buf: &mut B) -> Result<(), ProtocolError> {
        Ok(())
    }
}

impl Decode for ClearDialogPacket {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self)
    }
}

/// Clientbound `ShowDialogPacket` (`0x8C` in [`ConnectionState::Play`], MC 26.2).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ShowDialogPacket {
    /// Dialog holder (`RegistryId` or inline `Direct` NBT).
    pub dialog: DialogHolder,
}

impl PacketPayload for ShowDialogPacket {
    const PACKET_ID: i32 = 0x8C;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ShowDialogPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        match &self.dialog {
            DialogHolder::RegistryId(id) => {
                write_var_int(buf, (*id as i32).saturating_add(1));
                Ok(())
            }
            DialogHolder::Direct(tag) => {
                write_var_int(buf, 0);
                tag.encode(buf)
            }
        }
    }
}

impl Decode for ShowDialogPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let raw = read_var_int(buf)?;
        if raw < 0 {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "DialogHolder",
                value: raw,
            });
        }
        if raw == 0 {
            let tag = NbtTag::decode(buf)?;
            Ok(Self {
                dialog: DialogHolder::Direct(tag),
            })
        } else {
            Ok(Self {
                dialog: DialogHolder::RegistryId((raw - 1) as u32),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use minestom_item::Material;
    use minestom_nbt::NbtCompound;
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload};

    #[test]
    #[allow(clippy::too_many_lines)]
    fn ui_media_packets_roundtrip() {
        let boss = BossBarPacket {
            uuid: Uuid::from_u128(99),
            action: BossBarAction::Add {
                title: Component::text("Raid Boss"),
                health: 0.75,
                color: BossBarColor::Purple,
                overlay: BossBarOverlay::Notched10,
                flags: 0x01,
            },
        };
        let mut b = encode_packet_payload(&boss).unwrap();
        assert_eq!(read_var_int(&mut b).unwrap(), 0x09);
        assert_eq!(decode_packet_exact::<BossBarPacket>(b).unwrap(), boss);

        let title = SetTitleTextPacket {
            title: Component::text("Victory!"),
        };
        let mut tb = encode_packet_payload(&title).unwrap();
        assert_eq!(read_var_int(&mut tb).unwrap(), 0x72);
        assert_eq!(
            decode_packet_exact::<SetTitleTextPacket>(tb).unwrap(),
            title
        );

        let obj = ScoreboardObjectivePacket {
            objective_name: "sb-1".to_owned(),
            action: ObjectiveAction::Create {
                title: Component::text("Stats"),
                render_type: ObjectiveRenderType::Integer,
                number_format: Some(SidebarNumberFormat::Blank),
            },
        };
        let mut ob = encode_packet_payload(&obj).unwrap();
        assert_eq!(read_var_int(&mut ob).unwrap(), 0x6A);
        assert_eq!(
            decode_packet_exact::<ScoreboardObjectivePacket>(ob).unwrap(),
            obj
        );

        let team = TeamsPacket {
            team_name: "red".to_owned(),
            action: TeamsAction::Create {
                settings: TeamSettings {
                    display_name: Component::text("Red"),
                    prefix: Component::text("[RED] "),
                    suffix: Component::empty(),
                    name_tag_visibility: NameTagVisibility::Always,
                    collision_rule: CollisionRule::Never,
                    color: Some(TeamColor::Red),
                    friendly_flags: 0x02,
                },
                entities: vec!["Alice".to_owned()],
            },
        };
        let mut tmb = encode_packet_payload(&team).unwrap();
        assert_eq!(read_var_int(&mut tmb).unwrap(), 0x6D);
        assert_eq!(decode_packet_exact::<TeamsPacket>(tmb).unwrap(), team);

        let part = ParticlePacket {
            particle_id: 21,
            data: ParticleData::RgbScale {
                rgb: 0xFF_00_55,
                scale: 1.5,
            },
            override_limiter: true,
            long_distance: false,
            position: Vec3::new(1.0, 42.0, -3.0),
            offset: (0.2, 0.4, 0.2),
            max_speed: 0.05,
            particle_count: 12,
        };
        let mut pb = encode_packet_payload(&part).unwrap();
        assert_eq!(read_var_int(&mut pb).unwrap(), 0x2F);
        assert_eq!(decode_packet_exact::<ParticlePacket>(pb).unwrap(), part);

        let snd = SoundEffectPacket::at(
            SoundHolder::Builtin(42),
            SoundSource::Master,
            Vec3::new(4.0, 40.0, 8.0),
            1.0,
            1.2,
            777,
        );
        let mut sb = encode_packet_payload(&snd).unwrap();
        assert_eq!(read_var_int(&mut sb).unwrap(), 0x75);
        assert_eq!(decode_packet_exact::<SoundEffectPacket>(sb).unwrap(), snd);

        let rp = ResourcePackPushPacket {
            id: Uuid::from_u128(1234),
            url: "https://example.com/pack.zip".to_owned(),
            hash: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            forced: true,
            prompt: Some(Component::text("Required pack")),
        };
        let mut rpb = encode_packet_payload(&rp).unwrap();
        assert_eq!(read_var_int(&mut rpb).unwrap(), 0x51);
        assert_eq!(
            decode_packet_exact::<ResourcePackPushPacket>(rpb).unwrap(),
            rp
        );

        let adv = AdvancementsPacket {
            reset: false,
            advancements: vec![AdvancementEntry {
                key: "minestom:toast".to_owned(),
                parent: None,
                display: Some(AdvancementDisplayData {
                    title: Component::text("First Steps"),
                    description: Component::text("Welcome!"),
                    icon: ItemStack::single(Material::DIAMOND),
                    frame_type: AdvancementFrameType::Goal,
                    flags: 0x02,
                    background_texture: None,
                    x: 0.0,
                    y: 0.0,
                }),
                requirements: vec![vec!["done".to_owned()]],
                send_telemetry: false,
            }],
            identifiers_to_remove: Vec::new(),
            progress: vec![AdvancementProgressEntry {
                key: "minestom:toast".to_owned(),
                criteria: vec![("done".to_owned(), Some(1_700_000_000))],
            }],
            show_advancements: true,
        };
        let mut ab = encode_packet_payload(&adv).unwrap();
        assert_eq!(read_var_int(&mut ab).unwrap(), 0x82);
        assert_eq!(decode_packet_exact::<AdvancementsPacket>(ab).unwrap(), adv);

        let mut props = BTreeMap::new();
        props.insert("furnace_input".to_owned(), vec![Material::DIAMOND.id()]);
        let recipes = DeclareRecipesPacket {
            item_properties: props,
            stonecutter_recipes: vec![StonecutterRecipeEntry {
                input_item_ids: vec![Material::COBBLESTONE.id()],
                result: ItemStack::single(Material::STONE),
            }],
        };
        let mut rec_b = encode_packet_payload(&recipes).unwrap();
        assert_eq!(read_var_int(&mut rec_b).unwrap(), 0x85);
        assert_eq!(
            decode_packet_exact::<DeclareRecipesPacket>(rec_b).unwrap(),
            recipes
        );

        let mut dlg_nbt = NbtCompound::new();
        dlg_nbt.insert("type", NbtTag::String("minecraft:notice".to_owned()));
        let dlg = ShowDialogPacket {
            dialog: DialogHolder::Direct(NbtTag::Compound(dlg_nbt)),
        };
        let mut db = encode_packet_payload(&dlg).unwrap();
        assert_eq!(read_var_int(&mut db).unwrap(), 0x8C);
        assert_eq!(decode_packet_exact::<ShowDialogPacket>(db).unwrap(), dlg);
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_ui_media_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<BossBarPacket>(b.clone());
            let _ = decode_packet_exact::<ClearTitlesPacket>(b.clone());
            let _ = decode_packet_exact::<ActionBarPacket>(b.clone());
            let _ = decode_packet_exact::<SetTitleSubTitlePacket>(b.clone());
            let _ = decode_packet_exact::<SetTitleTextPacket>(b.clone());
            let _ = decode_packet_exact::<SetTitleTimePacket>(b.clone());
            let _ = decode_packet_exact::<PlayerListHeaderAndFooterPacket>(b.clone());
            let _ = decode_packet_exact::<ScoreboardObjectivePacket>(b.clone());
            let _ = decode_packet_exact::<DisplayScoreboardPacket>(b.clone());
            let _ = decode_packet_exact::<UpdateScorePacket>(b.clone());
            let _ = decode_packet_exact::<ResetScorePacket>(b.clone());
            let _ = decode_packet_exact::<TeamsPacket>(b.clone());
            let _ = decode_packet_exact::<ParticlePacket>(b.clone());
            let _ = decode_packet_exact::<SoundEffectPacket>(b.clone());
            let _ = decode_packet_exact::<StopSoundPacket>(b.clone());
            let _ = decode_packet_exact::<ResourcePackPushPacket>(b.clone());
            let _ = decode_packet_exact::<ResourcePackPopPacket>(b.clone());
            let _ = decode_packet_exact::<ClientResourcePackStatusPacket>(b.clone());
            let _ = decode_packet_exact::<AdvancementsPacket>(b.clone());
            let _ = decode_packet_exact::<DeclareRecipesPacket>(b.clone());
            let _ = decode_packet_exact::<ClearDialogPacket>(b.clone());
            let _ = decode_packet_exact::<ShowDialogPacket>(b);
        }
    }
}
