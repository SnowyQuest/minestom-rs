//! Scoreboard objective, sidebar score, and scoreboard team packets (protocol 776, MC 26.2).

use bytes::{Buf, BufMut};
use minestom_macros::PacketMeta;
use minestom_text::Component;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_string, read_var_int, write_string, write_var_int,
};

pub(super) fn write_string_list<B: BufMut>(
    buf: &mut B,
    items: &[String],
) -> Result<(), ProtocolError> {
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

pub(super) fn read_string_list<B: Buf>(
    buf: &mut B,
    max: usize,
) -> Result<Vec<String>, ProtocolError> {
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
    /// Optional custom line display component (MC 1.20.3+ native sidebar line content).
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
    /// Optional team glow/name color.
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
