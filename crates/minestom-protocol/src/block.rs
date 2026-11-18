//! Block placement, digging/actions, single/multi block changes, and block-entity NBT packets
//! for protocol 776 (MC 26.2).

use bytes::{Buf, BufMut};
use minestom_core::{BlockFace, BlockVec, Hand};
use minestom_macros::PacketMeta;
use minestom_nbt::{NbtCompound, NbtTag};

use crate::{
    ConnectionState, Decode, Encode, PacketPayload, ProtocolError, read_var_int, read_var_long,
    write_var_int, write_var_long,
};

fn decode_block_face_i32(raw: i32) -> Result<BlockFace, ProtocolError> {
    match raw {
        0 => Ok(BlockFace::Down),
        1 => Ok(BlockFace::Up),
        2 => Ok(BlockFace::North),
        3 => Ok(BlockFace::South),
        4 => Ok(BlockFace::West),
        5 => Ok(BlockFace::East),
        other => Err(ProtocolError::InvalidDiscriminant {
            type_name: "BlockFace",
            value: other,
        }),
    }
}

fn decode_hand(raw: i32) -> Result<Hand, ProtocolError> {
    match raw {
        0 => Ok(Hand::Main),
        1 => Ok(Hand::Off),
        other => Err(ProtocolError::InvalidDiscriminant {
            type_name: "Hand",
            value: other,
        }),
    }
}

/// Clientbound `AcknowledgeBlockChangePacket` (`0x04` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct AcknowledgeBlockChangePacket {
    /// Sequence ID echoed from [`ClientPlayerActionPacket`] or [`ClientPlayerBlockPlacementPacket`].
    pub sequence: i32,
}

impl PacketPayload for AcknowledgeBlockChangePacket {
    const PACKET_ID: i32 = 0x04;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for AcknowledgeBlockChangePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.sequence);
        Ok(())
    }
}

impl Decode for AcknowledgeBlockChangePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            sequence: read_var_int(buf)?,
        })
    }
}

/// Clientbound `BlockEntityDataPacket` (`0x06` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct BlockEntityDataPacket {
    /// Block position.
    pub position: BlockVec,
    /// Protocol ID in `minecraft:block_entity_type`.
    pub block_entity_type: i32,
    /// Optional nameless network [`NbtCompound`] (`TAG_End` `0x00` on wire if `None`).
    pub data: Option<NbtCompound>,
}

impl PacketPayload for BlockEntityDataPacket {
    const PACKET_ID: i32 = 0x06;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for BlockEntityDataPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.position.encode(buf)?;
        write_var_int(buf, self.block_entity_type);
        if let Some(compound) = &self.data {
            compound.encode(buf)
        } else {
            buf.put_u8(0x00);
            Ok(())
        }
    }
}

impl Decode for BlockEntityDataPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let position = BlockVec::decode(buf)?;
        let block_entity_type = read_var_int(buf)?;
        let data = match NbtTag::decode(buf)? {
            NbtTag::End => None,
            NbtTag::Compound(c) => Some(c),
            other => {
                return Err(ProtocolError::InvalidField(format!(
                    "expected Compound or End in BlockEntityDataPacket, got {other:?}"
                )));
            }
        };
        Ok(Self {
            position,
            block_entity_type,
            data,
        })
    }
}

/// Clientbound `BlockChangePacket` (`0x08` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct BlockChangePacket {
    /// Target block coordinate.
    pub position: BlockVec,
    /// New protocol block state ID.
    pub block_state_id: i32,
}

impl PacketPayload for BlockChangePacket {
    const PACKET_ID: i32 = 0x08;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for BlockChangePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.position.encode(buf)?;
        write_var_int(buf, self.block_state_id);
        Ok(())
    }
}

impl Decode for BlockChangePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            position: BlockVec::decode(buf)?,
            block_state_id: read_var_int(buf)?,
        })
    }
}

/// Clientbound `MultiBlockChangePacket` (`0x54` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct MultiBlockChangePacket {
    /// Packed 64-bit `(chunk_x & 0x3FFFFF) << 42 | (chunk_z & 0x3FFFFF) << 20 | (section_y & 0xFFFFF)`.
    pub chunk_section_position: i64,
    /// `VarLong` entries `(state_id << 12) | (local_x << 8) | (local_z << 4) | local_y`.
    pub blocks: Vec<i64>,
}

impl MultiBlockChangePacket {
    /// Packs `(chunk_x, section_y, chunk_z)` and `(local_x, local_y, local_z, state_id)` entries
    /// into a [`MultiBlockChangePacket`].
    #[must_use]
    pub fn from_section_entries(
        chunk_x: i32,
        section_y: i32,
        chunk_z: i32,
        entries: &[(u8, u8, u8, u16)],
    ) -> Self {
        let pos = ((i64::from(chunk_x) & 0x3F_FFFF) << 42)
            | ((i64::from(chunk_z) & 0x3F_FFFF) << 20)
            | (i64::from(section_y) & 0x0F_FFFF);
        let blocks = entries
            .iter()
            .map(|&(lx, ly, lz, state_id)| {
                let local_idx =
                    (i64::from(lx & 15) << 8) | (i64::from(lz & 15) << 4) | i64::from(ly & 15);
                (i64::from(state_id) << 12) | local_idx
            })
            .collect();
        Self {
            chunk_section_position: pos,
            blocks,
        }
    }
}

impl PacketPayload for MultiBlockChangePacket {
    const PACKET_ID: i32 = 0x54;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for MultiBlockChangePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        if self.blocks.len() > 4096 {
            return Err(ProtocolError::InvalidLength {
                len: self.blocks.len() as i32,
                max: 4096,
            });
        }
        self.chunk_section_position.encode(buf)?;
        write_var_int(buf, self.blocks.len() as i32);
        for &entry in &self.blocks {
            write_var_long(buf, entry);
        }
        Ok(())
    }
}

impl Decode for MultiBlockChangePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let chunk_section_position = i64::decode(buf)?;
        let raw_len = read_var_int(buf)?;
        if !(0..=4096).contains(&raw_len) {
            return Err(ProtocolError::InvalidLength {
                len: raw_len,
                max: 4096,
            });
        }
        let mut blocks = Vec::with_capacity((raw_len as usize).min(buf.remaining()));
        for _ in 0..raw_len {
            blocks.push(read_var_long(buf)?);
        }
        Ok(Self {
            chunk_section_position,
            blocks,
        })
    }
}

/// Digging / item-action status inside [`ClientPlayerActionPacket`] (`0..=7` in MC 26.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PlayerActionStatus {
    /// `0` — Started digging block.
    StartedDigging = 0,
    /// `1` — Cancelled digging block.
    CancelledDigging = 1,
    /// `2` — Finished digging block.
    FinishedDigging = 2,
    /// `3` — Drop entire held item stack (`Ctrl+Q`).
    DropItemStack = 3,
    /// `4` — Drop single held item (`Q`).
    DropItem = 4,
    /// `5` — Shoot arrow / finish eating / release item use.
    UpdateItemState = 5,
    /// `6` — Swap main-hand and off-hand items (`F`).
    SwapItemHand = 6,
    /// `7` — MC 26.2 spear/weapon stab action.
    Stab = 7,
}

impl PlayerActionStatus {
    const fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::StartedDigging),
            1 => Ok(Self::CancelledDigging),
            2 => Ok(Self::FinishedDigging),
            3 => Ok(Self::DropItemStack),
            4 => Ok(Self::DropItem),
            5 => Ok(Self::UpdateItemState),
            6 => Ok(Self::SwapItemHand),
            7 => Ok(Self::Stab),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "PlayerActionStatus",
                value: other,
            }),
        }
    }
}

/// Serverbound `ClientPlayerActionPacket` (`0x29` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientPlayerActionPacket {
    /// Action discriminant.
    pub status: PlayerActionStatus,
    /// Block position (`BlockVec::ZERO` for non-block actions like `DropItem`).
    pub position: BlockVec,
    /// Clicked [`BlockFace`] (encoded as a single `BYTE` `0..=5` on wire!).
    pub block_face: BlockFace,
    /// Client block-prediction sequence number.
    pub sequence: i32,
}

impl PacketPayload for ClientPlayerActionPacket {
    const PACKET_ID: i32 = 0x29;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientPlayerActionPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.status as i32);
        self.position.encode(buf)?;
        (self.block_face as i8).encode(buf)?;
        write_var_int(buf, self.sequence);
        Ok(())
    }
}

impl Decode for ClientPlayerActionPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let status = PlayerActionStatus::from_id(read_var_int(buf)?)?;
        let position = BlockVec::decode(buf)?;
        let face_byte = i8::decode(buf)?;
        let block_face = decode_block_face_i32(i32::from(face_byte))?;
        let sequence = read_var_int(buf)?;
        Ok(Self {
            status,
            position,
            block_face,
            sequence,
        })
    }
}

/// Serverbound `ClientPlayerBlockPlacementPacket` (`0x42` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct ClientPlayerBlockPlacementPacket {
    /// Hand used (`Main` or `Off`).
    pub hand: Hand,
    /// Clicked block coordinate.
    pub position: BlockVec,
    /// Clicked [`BlockFace`] (encoded as `VarInt` `0..=5` on wire!).
    pub block_face: BlockFace,
    /// Sub-block cursor X (`0.0..=1.0`).
    pub cursor_x: f32,
    /// Sub-block cursor Y (`0.0..=1.0`).
    pub cursor_y: f32,
    /// Sub-block cursor Z (`0.0..=1.0`).
    pub cursor_z: f32,
    /// Whether player head is inside block.
    pub inside_block: bool,
    /// Whether raycast hit world border (MC 26.2).
    pub hit_world_border: bool,
    /// Client block-prediction sequence number.
    pub sequence: i32,
}

impl PacketPayload for ClientPlayerBlockPlacementPacket {
    const PACKET_ID: i32 = 0x42;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientPlayerBlockPlacementPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.hand as i32);
        self.position.encode(buf)?;
        write_var_int(buf, self.block_face as i32);
        self.cursor_x.encode(buf)?;
        self.cursor_y.encode(buf)?;
        self.cursor_z.encode(buf)?;
        self.inside_block.encode(buf)?;
        self.hit_world_border.encode(buf)?;
        write_var_int(buf, self.sequence);
        Ok(())
    }
}

impl Decode for ClientPlayerBlockPlacementPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let hand = decode_hand(read_var_int(buf)?)?;
        let position = BlockVec::decode(buf)?;
        let block_face = decode_block_face_i32(read_var_int(buf)?)?;
        let cursor_x = f32::decode(buf)?;
        let cursor_y = f32::decode(buf)?;
        let cursor_z = f32::decode(buf)?;
        let inside_block = bool::decode(buf)?;
        let hit_world_border = bool::decode(buf)?;
        let sequence = read_var_int(buf)?;
        Ok(Self {
            hand,
            position,
            block_face,
            cursor_x,
            cursor_y,
            cursor_z,
            inside_block,
            hit_world_border,
            sequence,
        })
    }
}

/// Serverbound `ClientUseItemPacket` (`0x43` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct ClientUseItemPacket {
    /// Hand used.
    pub hand: Hand,
    /// Prediction sequence number.
    pub sequence: i32,
    /// Yaw at use instant.
    pub yaw: f32,
    /// Pitch at use instant.
    pub pitch: f32,
}

impl PacketPayload for ClientUseItemPacket {
    const PACKET_ID: i32 = 0x43;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientUseItemPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.hand as i32);
        write_var_int(buf, self.sequence);
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)
    }
}

impl Decode for ClientUseItemPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            hand: decode_hand(read_var_int(buf)?)?,
            sequence: read_var_int(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload};

    #[test]
    fn block_packets_roundtrip() {
        let place = ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(4, 39, -2),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 7,
        };
        let mut b = encode_packet_payload(&place).unwrap();
        assert_eq!(read_var_int(&mut b).unwrap(), 0x42);
        assert_eq!(
            decode_packet_exact::<ClientPlayerBlockPlacementPacket>(b).unwrap(),
            place
        );

        let multi = MultiBlockChangePacket::from_section_entries(0, 2, 0, &[(1, 8, 2, 9)]);
        let mut mb = encode_packet_payload(&multi).unwrap();
        assert_eq!(read_var_int(&mut mb).unwrap(), 0x54);
        assert_eq!(
            decode_packet_exact::<MultiBlockChangePacket>(mb).unwrap(),
            multi
        );

        let mut nbt = NbtCompound::new();
        nbt.insert("Lock", NbtTag::String("secret".to_owned()));
        let be = BlockEntityDataPacket {
            position: BlockVec::new(1, 40, 1),
            block_entity_type: 1,
            data: Some(nbt),
        };
        let mut beb = encode_packet_payload(&be).unwrap();
        assert_eq!(read_var_int(&mut beb).unwrap(), 0x06);
        assert_eq!(
            decode_packet_exact::<BlockEntityDataPacket>(beb).unwrap(),
            be
        );
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_block_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<AcknowledgeBlockChangePacket>(b.clone());
            let _ = decode_packet_exact::<BlockEntityDataPacket>(b.clone());
            let _ = decode_packet_exact::<BlockChangePacket>(b.clone());
            let _ = decode_packet_exact::<MultiBlockChangePacket>(b.clone());
            let _ = decode_packet_exact::<ClientPlayerActionPacket>(b.clone());
            let _ = decode_packet_exact::<ClientPlayerBlockPlacementPacket>(b.clone());
            let _ = decode_packet_exact::<ClientUseItemPacket>(b);
        }
    }
}
