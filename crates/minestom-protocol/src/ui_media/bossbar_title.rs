//! `BossBar`, Titles, `ActionBar`, and `TabList` header/footer packets (protocol 776, MC 26.2).

use bytes::{Buf, BufMut};
use minestom_macros::PacketMeta;
use minestom_text::Component;
use uuid::Uuid;

use crate::{
    ConnectionState, Decode, Encode, PacketPayload, ProtocolError, read_var_int, write_var_int,
};

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
