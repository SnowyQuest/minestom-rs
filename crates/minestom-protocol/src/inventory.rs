//! MC 26.2 `ItemStack` Data Component wire codec (trusted & untrusted creative),
//! `HashedStack` (`ItemStack.Hash`), container windows, equipment, and item collection packets
//! for protocol 776.

use std::collections::{BTreeMap, BTreeSet};

use bytes::{Buf, BufMut, BytesMut};
use minestom_item::{
    DataComponentValue, EquipmentSlot, ItemRarity, ItemStack, Material, WindowClickMode,
    component_ids,
};
use minestom_macros::PacketMeta;
use minestom_nbt::NbtCompound;
use minestom_text::Component;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_byte_array, read_prefixed_vec, read_string, read_var_int, write_byte_array,
    write_prefixed_vec, write_string, write_var_int,
};

fn encode_component_payload<B: BufMut>(
    buf: &mut B,
    val: &DataComponentValue,
) -> Result<(), ProtocolError> {
    match val {
        DataComponentValue::CustomData(nbt) => nbt.encode(buf),
        DataComponentValue::MaxStackSize(v)
        | DataComponentValue::MaxDamage(v)
        | DataComponentValue::Damage(v)
        | DataComponentValue::RepairCost(v) => {
            write_var_int(buf, *v);
            Ok(())
        }
        DataComponentValue::Unbreakable | DataComponentValue::CreativeSlotLock => Ok(()),
        DataComponentValue::CustomName(c) | DataComponentValue::ItemName(c) => c.encode(buf),
        DataComponentValue::ItemModel(s) => write_string(buf, s, DEFAULT_MAX_STRING_CHARS),
        DataComponentValue::Lore(lines) => write_prefixed_vec(buf, lines, 256),
        DataComponentValue::Rarity(r) => {
            write_var_int(buf, *r as i32);
            Ok(())
        }
        DataComponentValue::Enchantments(map) => {
            if map.len() > 256 {
                return Err(ProtocolError::InvalidLength {
                    len: map.len() as i32,
                    max: 256,
                });
            }
            write_var_int(buf, map.len() as i32);
            for (&ench_id, &lvl) in map {
                write_var_int(buf, ench_id);
                write_var_int(buf, lvl);
            }
            Ok(())
        }
        DataComponentValue::EnchantmentGlintOverride(b) => b.encode(buf),
        DataComponentValue::RawOpaque { bytes, .. } => {
            buf.put_slice(bytes);
            Ok(())
        }
    }
}

fn decode_known_component_payload<B: Buf>(
    buf: &mut B,
    component_id: i32,
) -> Result<DataComponentValue, ProtocolError> {
    match component_id {
        component_ids::CUSTOM_DATA => Ok(DataComponentValue::CustomData(NbtCompound::decode(buf)?)),
        component_ids::MAX_STACK_SIZE => Ok(DataComponentValue::MaxStackSize(read_var_int(buf)?)),
        component_ids::MAX_DAMAGE => Ok(DataComponentValue::MaxDamage(read_var_int(buf)?)),
        component_ids::DAMAGE => Ok(DataComponentValue::Damage(read_var_int(buf)?)),
        component_ids::UNBREAKABLE => Ok(DataComponentValue::Unbreakable),
        component_ids::CUSTOM_NAME => Ok(DataComponentValue::CustomName(Component::decode(buf)?)),
        component_ids::ITEM_NAME => Ok(DataComponentValue::ItemName(Component::decode(buf)?)),
        component_ids::ITEM_MODEL => Ok(DataComponentValue::ItemModel(read_string(
            buf,
            DEFAULT_MAX_STRING_CHARS,
        )?)),
        component_ids::LORE => Ok(DataComponentValue::Lore(read_prefixed_vec(buf, 256)?)),
        component_ids::RARITY => {
            let r = match read_var_int(buf)? {
                0 => ItemRarity::Common,
                1 => ItemRarity::Uncommon,
                2 => ItemRarity::Rare,
                3 => ItemRarity::Epic,
                other => {
                    return Err(ProtocolError::InvalidDiscriminant {
                        type_name: "ItemRarity",
                        value: other,
                    });
                }
            };
            Ok(DataComponentValue::Rarity(r))
        }
        component_ids::ENCHANTMENTS => {
            let len = read_var_int(buf)?;
            if !(0..=256).contains(&len) {
                return Err(ProtocolError::InvalidLength { len, max: 256 });
            }
            let mut map = BTreeMap::new();
            for _ in 0..len {
                let k = read_var_int(buf)?;
                let v = read_var_int(buf)?;
                map.insert(k, v);
            }
            Ok(DataComponentValue::Enchantments(map))
        }
        component_ids::REPAIR_COST => Ok(DataComponentValue::RepairCost(read_var_int(buf)?)),
        component_ids::CREATIVE_SLOT_LOCK => Ok(DataComponentValue::CreativeSlotLock),
        component_ids::ENCHANTMENT_GLINT_OVERRIDE => Ok(
            DataComponentValue::EnchantmentGlintOverride(bool::decode(buf)?),
        ),
        other => Err(ProtocolError::InvalidDiscriminant {
            type_name: "DataComponent",
            value: other,
        }),
    }
}

impl Encode for ItemStack {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        if self.is_air() {
            write_var_int(buf, 0);
            return Ok(());
        }
        write_var_int(buf, self.amount() as i32);
        write_var_int(buf, self.material().id() as i32);
        write_var_int(buf, self.added_components().len() as i32);
        write_var_int(buf, self.removed_components().len() as i32);
        for (&cid, val) in self.added_components() {
            write_var_int(buf, cid);
            encode_component_payload(buf, val)?;
        }
        for &rem_id in self.removed_components() {
            write_var_int(buf, rem_id);
        }
        Ok(())
    }
}

impl Decode for ItemStack {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let amount = read_var_int(buf)?;
        if amount <= 0 {
            return Ok(Self::AIR);
        }
        let raw_mat = read_var_int(buf)?;
        if raw_mat < 0 {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "Material",
                value: raw_mat,
            });
        }
        let material =
            Material::from_id(raw_mat as u32).map_err(|_| ProtocolError::InvalidDiscriminant {
                type_name: "Material",
                value: raw_mat,
            })?;
        let added_len = read_var_int(buf)?;
        let removed_len = read_var_int(buf)?;
        if !(0..=256).contains(&added_len) || !(0..=256).contains(&removed_len) {
            return Err(ProtocolError::InvalidLength {
                len: added_len.max(removed_len),
                max: 256,
            });
        }
        let mut added = BTreeMap::new();
        for _ in 0..added_len {
            let cid = read_var_int(buf)?;
            let val = decode_known_component_payload(buf, cid)?;
            added.insert(cid, val);
        }
        let mut removed = BTreeSet::new();
        for _ in 0..removed_len {
            removed.insert(read_var_int(buf)?);
        }
        Ok(Self::from_parts(material, amount as u32, added, removed))
    }
}

/// Encodes `stack` using MC 26.2's `ItemStack.UNTRUSTED_NETWORK_TYPE` (length-prefixed component
/// payloads used in [`ClientCreativeInventoryActionPacket`]).
pub fn encode_untrusted_item_stack<B: BufMut>(
    buf: &mut B,
    stack: &ItemStack,
) -> Result<(), ProtocolError> {
    if stack.is_air() {
        write_var_int(buf, 0);
        return Ok(());
    }
    write_var_int(buf, stack.amount() as i32);
    write_var_int(buf, stack.material().id() as i32);
    write_var_int(buf, stack.added_components().len() as i32);
    write_var_int(buf, stack.removed_components().len() as i32);
    for (&cid, val) in stack.added_components() {
        write_var_int(buf, cid);
        let mut tmp = BytesMut::new();
        encode_component_payload(&mut tmp, val)?;
        write_byte_array(buf, &tmp, 65_536)?;
    }
    for &rem_id in stack.removed_components() {
        write_var_int(buf, rem_id);
    }
    Ok(())
}

/// Decodes `ItemStack.UNTRUSTED_NETWORK_TYPE`, safely preserving any unknown creative component
/// payloads inside [`DataComponentValue::RawOpaque`]!
pub fn decode_untrusted_item_stack<B: Buf>(buf: &mut B) -> Result<ItemStack, ProtocolError> {
    let amount = read_var_int(buf)?;
    if amount <= 0 {
        return Ok(ItemStack::AIR);
    }
    let raw_mat = read_var_int(buf)?;
    if raw_mat < 0 {
        return Err(ProtocolError::InvalidDiscriminant {
            type_name: "Material",
            value: raw_mat,
        });
    }
    let material =
        Material::from_id(raw_mat as u32).map_err(|_| ProtocolError::InvalidDiscriminant {
            type_name: "Material",
            value: raw_mat,
        })?;
    let added_len = read_var_int(buf)?;
    let removed_len = read_var_int(buf)?;
    if !(0..=256).contains(&added_len) || !(0..=256).contains(&removed_len) {
        return Err(ProtocolError::InvalidLength {
            len: added_len.max(removed_len),
            max: 256,
        });
    }
    let mut added = BTreeMap::new();
    for _ in 0..added_len {
        let cid = read_var_int(buf)?;
        let raw_bytes = read_byte_array(buf, 65_536)?;
        let mut slice = raw_bytes.as_slice();
        let val = match decode_known_component_payload(&mut slice, cid) {
            Ok(parsed) if slice.is_empty() => parsed,
            _ => DataComponentValue::RawOpaque {
                id: cid,
                bytes: raw_bytes,
            },
        };
        added.insert(cid, val);
    }
    let mut removed = BTreeSet::new();
    for _ in 0..removed_len {
        removed.insert(read_var_int(buf)?);
    }
    Ok(ItemStack::from_parts(
        material,
        amount as u32,
        added,
        removed,
    ))
}

/// Hashed item summary (`ItemStack.Hash` in MC 26.2 [`ClientClickWindowPacket`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HashedStack {
    /// Empty slot (`false` on wire).
    #[default]
    Air,
    /// Non-empty hashed stack (`true` on wire).
    Item {
        /// Material ID.
        material: Material,
        /// Stack count.
        amount: i32,
        /// Added `(component_id -> crc32_hash)` entries.
        added_hashes: BTreeMap<i32, i32>,
        /// Removed component IDs.
        removed_components: BTreeSet<i32>,
    },
}

impl Encode for HashedStack {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        match self {
            Self::Air => false.encode(buf),
            Self::Item {
                material,
                amount,
                added_hashes,
                removed_components,
            } => {
                true.encode(buf)?;
                write_var_int(buf, material.id() as i32);
                write_var_int(buf, *amount);
                write_var_int(buf, added_hashes.len() as i32);
                for (&cid, &hash) in added_hashes {
                    write_var_int(buf, cid);
                    hash.encode(buf)?;
                }
                write_var_int(buf, removed_components.len() as i32);
                for &cid in removed_components {
                    write_var_int(buf, cid);
                }
                Ok(())
            }
        }
    }
}

impl Decode for HashedStack {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        if !bool::decode(buf)? {
            return Ok(Self::Air);
        }
        let raw_mat = read_var_int(buf)?;
        if raw_mat < 0 {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "Material",
                value: raw_mat,
            });
        }
        let material =
            Material::from_id(raw_mat as u32).map_err(|_| ProtocolError::InvalidDiscriminant {
                type_name: "Material",
                value: raw_mat,
            })?;
        let amount = read_var_int(buf)?;
        let added_len = read_var_int(buf)?;
        if !(0..=256).contains(&added_len) {
            return Err(ProtocolError::InvalidLength {
                len: added_len,
                max: 256,
            });
        }
        let mut added_hashes = BTreeMap::new();
        for _ in 0..added_len {
            let cid = read_var_int(buf)?;
            let h = i32::decode(buf)?;
            added_hashes.insert(cid, h);
        }
        let rem_len = read_var_int(buf)?;
        if !(0..=256).contains(&rem_len) {
            return Err(ProtocolError::InvalidLength {
                len: rem_len,
                max: 256,
            });
        }
        let mut removed_components = BTreeSet::new();
        for _ in 0..rem_len {
            removed_components.insert(read_var_int(buf)?);
        }
        Ok(Self::Item {
            material,
            amount,
            added_hashes,
            removed_components,
        })
    }
}

/// Clientbound `CloseWindowPacket` (`0x11` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct CloseWindowPacket {
    /// Window ID (`0` = player inventory, `1..=100` = open container).
    pub window_id: i32,
}

impl PacketPayload for CloseWindowPacket {
    const PACKET_ID: i32 = 0x11;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for CloseWindowPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.window_id);
        Ok(())
    }
}

impl Decode for CloseWindowPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            window_id: read_var_int(buf)?,
        })
    }
}

/// Clientbound `WindowItemsPacket` (`0x12` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct WindowItemsPacket {
    /// Window ID.
    pub window_id: i32,
    /// State revision ID.
    pub state_id: i32,
    /// Full list of window slots (max `128`).
    pub items: Vec<ItemStack>,
    /// Cursor carried item.
    pub carried_item: ItemStack,
}

impl PacketPayload for WindowItemsPacket {
    const PACKET_ID: i32 = 0x12;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for WindowItemsPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.window_id);
        write_var_int(buf, self.state_id);
        write_prefixed_vec(buf, &self.items, 128)?;
        self.carried_item.encode(buf)
    }
}

impl Decode for WindowItemsPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            window_id: read_var_int(buf)?,
            state_id: read_var_int(buf)?,
            items: read_prefixed_vec(buf, 128)?,
            carried_item: ItemStack::decode(buf)?,
        })
    }
}

/// Clientbound `WindowPropertyPacket` (`0x13` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct WindowPropertyPacket {
    /// Window ID.
    pub window_id: i32,
    /// Property index (e.g. furnace burn progress).
    pub property: i16,
    /// Property value.
    pub value: i16,
}

impl PacketPayload for WindowPropertyPacket {
    const PACKET_ID: i32 = 0x13;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for WindowPropertyPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.window_id);
        self.property.encode(buf)?;
        self.value.encode(buf)
    }
}

impl Decode for WindowPropertyPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            window_id: read_var_int(buf)?,
            property: i16::decode(buf)?,
            value: i16::decode(buf)?,
        })
    }
}

/// Clientbound `SetSlotPacket` (`0x14` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SetSlotPacket {
    /// Window ID (`-1` with `slot = -1` updates cursor).
    pub window_id: i32,
    /// State revision ID.
    pub state_id: i32,
    /// Target slot index.
    pub slot: i16,
    /// New item stack.
    pub item_stack: ItemStack,
}

impl PacketPayload for SetSlotPacket {
    const PACKET_ID: i32 = 0x14;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SetSlotPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.window_id);
        write_var_int(buf, self.state_id);
        self.slot.encode(buf)?;
        self.item_stack.encode(buf)
    }
}

impl Decode for SetSlotPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            window_id: read_var_int(buf)?,
            state_id: read_var_int(buf)?,
            slot: i16::decode(buf)?,
            item_stack: ItemStack::decode(buf)?,
        })
    }
}

/// Clientbound `OpenWindowPacket` (`0x3B` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct OpenWindowPacket {
    /// Positive window ID (`1..=100`).
    pub window_id: i32,
    /// Container `InventoryType` ordinal (`0..=24`).
    pub window_type: i32,
    /// Window title [`Component`].
    pub title: Component,
}

impl PacketPayload for OpenWindowPacket {
    const PACKET_ID: i32 = 0x3B;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for OpenWindowPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.window_id);
        write_var_int(buf, self.window_type);
        self.title.encode(buf)
    }
}

impl Decode for OpenWindowPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            window_id: read_var_int(buf)?,
            window_type: read_var_int(buf)?,
            title: Component::decode(buf)?,
        })
    }
}

/// Clientbound `SetCursorItemPacket` (`0x60` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SetCursorItemPacket {
    /// New cursor carried item.
    pub item_stack: ItemStack,
}

impl PacketPayload for SetCursorItemPacket {
    const PACKET_ID: i32 = 0x60;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SetCursorItemPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.item_stack.encode(buf)
    }
}

impl Decode for SetCursorItemPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            item_stack: ItemStack::decode(buf)?,
        })
    }
}

/// Clientbound `EntityEquipmentPacket` (`0x66` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct EntityEquipmentPacket {
    /// Entity ID whose visible equipment is being updated.
    pub entity_id: i32,
    /// Non-empty map of `(EquipmentSlot, ItemStack)`.
    pub equipments: BTreeMap<EquipmentSlot, ItemStack>,
}

impl PacketPayload for EntityEquipmentPacket {
    const PACKET_ID: i32 = 0x66;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityEquipmentPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        if self.equipments.is_empty() {
            return Err(ProtocolError::InvalidLength { len: 0, max: 8 });
        }
        write_var_int(buf, self.entity_id);
        let total = self.equipments.len();
        for (i, (&slot, item)) in self.equipments.iter().enumerate() {
            let mut slot_byte = slot as u8;
            if i + 1 < total {
                slot_byte |= 0x80;
            }
            buf.put_u8(slot_byte);
            item.encode(buf)?;
        }
        Ok(())
    }
}

impl Decode for EntityEquipmentPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let entity_id = read_var_int(buf)?;
        let mut equipments = BTreeMap::new();
        loop {
            let raw = u8::decode(buf)?;
            let slot = match raw & 0x7F {
                0 => EquipmentSlot::MainHand,
                1 => EquipmentSlot::OffHand,
                2 => EquipmentSlot::Boots,
                3 => EquipmentSlot::Leggings,
                4 => EquipmentSlot::Chestplate,
                5 => EquipmentSlot::Helmet,
                6 => EquipmentSlot::Body,
                7 => EquipmentSlot::Saddle,
                other => {
                    return Err(ProtocolError::InvalidDiscriminant {
                        type_name: "EquipmentSlot",
                        value: i32::from(other),
                    });
                }
            };
            let item = ItemStack::decode(buf)?;
            equipments.insert(slot, item);
            if (raw & 0x80) == 0 {
                break;
            }
            if equipments.len() >= 16 {
                return Err(ProtocolError::InvalidLength { len: 16, max: 8 });
            }
        }
        Ok(Self {
            entity_id,
            equipments,
        })
    }
}

/// Clientbound `HeldItemChangePacket` (`0x69` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct HeldItemChangePacket {
    /// Selected hotbar slot (`0..=8`).
    pub slot: i8,
}

impl PacketPayload for HeldItemChangePacket {
    const PACKET_ID: i32 = 0x69;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for HeldItemChangePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.slot.encode(buf)
    }
}

impl Decode for HeldItemChangePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            slot: i8::decode(buf)?,
        })
    }
}

/// Clientbound `CollectItemPacket` (`0x7C` in [`ConnectionState::Play`], item pickup animation!).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct CollectItemPacket {
    /// Picked-up `ItemEntity` network ID.
    pub collected_entity_id: i32,
    /// Collector player/entity network ID.
    pub collector_entity_id: i32,
    /// Item count visually flying into collector.
    pub pickup_item_count: i32,
}

impl PacketPayload for CollectItemPacket {
    const PACKET_ID: i32 = 0x7C;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for CollectItemPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.collected_entity_id);
        write_var_int(buf, self.collector_entity_id);
        write_var_int(buf, self.pickup_item_count);
        Ok(())
    }
}

impl Decode for CollectItemPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            collected_entity_id: read_var_int(buf)?,
            collector_entity_id: read_var_int(buf)?,
            pickup_item_count: read_var_int(buf)?,
        })
    }
}

/// Serverbound `ClientClickWindowButtonPacket` (`0x11` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientClickWindowButtonPacket {
    /// Window ID.
    pub window_id: i32,
    /// Button ID (enchanting table option, lectern page, etc.).
    pub button_id: i32,
}

impl PacketPayload for ClientClickWindowButtonPacket {
    const PACKET_ID: i32 = 0x11;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientClickWindowButtonPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.window_id);
        write_var_int(buf, self.button_id);
        Ok(())
    }
}

impl Decode for ClientClickWindowButtonPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            window_id: read_var_int(buf)?,
            button_id: read_var_int(buf)?,
        })
    }
}

/// Serverbound `ClientClickWindowPacket` (`0x12` in [`ConnectionState::Play`], MC 26.2 `HashedStack`!).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientClickWindowPacket {
    /// Window ID (`0` = player inventory).
    pub window_id: i32,
    /// Last seen window `state_id`.
    pub state_id: i32,
    /// Clicked slot (`-999` = outside window).
    pub slot: i16,
    /// Mouse/key button.
    pub button: i8,
    /// Click mode (`0..=6`).
    pub click_type: WindowClickMode,
    /// Client-predicted changed slots (`slot -> HashedStack`, max `128`).
    pub changed_slots: BTreeMap<i16, HashedStack>,
    /// Client-predicted cursor (`HashedStack`).
    pub clicked_item: HashedStack,
}

impl PacketPayload for ClientClickWindowPacket {
    const PACKET_ID: i32 = 0x12;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientClickWindowPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        if self.changed_slots.len() > 128 {
            return Err(ProtocolError::InvalidLength {
                len: self.changed_slots.len() as i32,
                max: 128,
            });
        }
        write_var_int(buf, self.window_id);
        write_var_int(buf, self.state_id);
        self.slot.encode(buf)?;
        self.button.encode(buf)?;
        write_var_int(buf, self.click_type as i32);
        write_var_int(buf, self.changed_slots.len() as i32);
        for (&s, h) in &self.changed_slots {
            s.encode(buf)?;
            h.encode(buf)?;
        }
        self.clicked_item.encode(buf)
    }
}

impl Decode for ClientClickWindowPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let window_id = read_var_int(buf)?;
        let state_id = read_var_int(buf)?;
        let slot = i16::decode(buf)?;
        let button = i8::decode(buf)?;
        let click_type = match read_var_int(buf)? {
            0 => WindowClickMode::Pickup,
            1 => WindowClickMode::QuickMove,
            2 => WindowClickMode::Swap,
            3 => WindowClickMode::Clone,
            4 => WindowClickMode::Throw,
            5 => WindowClickMode::QuickCraft,
            6 => WindowClickMode::PickupAll,
            other => {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "WindowClickMode",
                    value: other,
                });
            }
        };
        let map_len = read_var_int(buf)?;
        if !(0..=128).contains(&map_len) {
            return Err(ProtocolError::InvalidLength {
                len: map_len,
                max: 128,
            });
        }
        let mut changed_slots = BTreeMap::new();
        for _ in 0..map_len {
            let s = i16::decode(buf)?;
            let h = HashedStack::decode(buf)?;
            changed_slots.insert(s, h);
        }
        let clicked_item = HashedStack::decode(buf)?;
        Ok(Self {
            window_id,
            state_id,
            slot,
            button,
            click_type,
            changed_slots,
            clicked_item,
        })
    }
}

/// Serverbound `ClientCloseWindowPacket` (`0x13` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientCloseWindowPacket {
    /// Closed window ID.
    pub window_id: i32,
}

impl PacketPayload for ClientCloseWindowPacket {
    const PACKET_ID: i32 = 0x13;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientCloseWindowPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.window_id);
        Ok(())
    }
}

impl Decode for ClientCloseWindowPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            window_id: read_var_int(buf)?,
        })
    }
}

/// Serverbound `ClientHeldItemChangePacket` (`0x35` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientHeldItemChangePacket {
    /// Selected hotbar slot (`0..=8`).
    pub slot: i16,
}

impl PacketPayload for ClientHeldItemChangePacket {
    const PACKET_ID: i32 = 0x35;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientHeldItemChangePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.slot.encode(buf)
    }
}

impl Decode for ClientHeldItemChangePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            slot: i16::decode(buf)?,
        })
    }
}

/// Serverbound `ClientCreativeInventoryActionPacket` (`0x38` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ClientCreativeInventoryActionPacket {
    /// Window-0 slot (`-1` drops item into world).
    pub slot: i16,
    /// Spawned item stack (decoded via `UNTRUSTED_NETWORK_TYPE`!).
    pub item: ItemStack,
}

impl PacketPayload for ClientCreativeInventoryActionPacket {
    const PACKET_ID: i32 = 0x38;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientCreativeInventoryActionPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.slot.encode(buf)?;
        encode_untrusted_item_stack(buf, &self.item)
    }
}

impl Decode for ClientCreativeInventoryActionPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            slot: i16::decode(buf)?,
            item: decode_untrusted_item_stack(buf)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use minestom_core::Tag;
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload};

    #[test]
    fn item_stack_components_windows_equipment_and_click_packets_roundtrip() {
        let soul_tag: Tag<i32> = Tag::new("souls");
        let blade = ItemStack::single(Material::DIAMOND_SWORD)
            .with_custom_name(Component::text("Soulblade"))
            .with_lore([Component::text("Line 1"), Component::text("Line 2")])
            .with_unbreakable()
            .with_glowing(true)
            .with_enchantment(14, 5)
            .with_tag(&soul_tag, 99);

        let set_slot = SetSlotPacket {
            window_id: 0,
            state_id: 3,
            slot: 36,
            item_stack: blade.clone(),
        };
        let mut b = encode_packet_payload(&set_slot).unwrap();
        assert_eq!(read_var_int(&mut b).unwrap(), 0x14);
        let decoded = decode_packet_exact::<SetSlotPacket>(b).unwrap();
        assert_eq!(decoded, set_slot);
        assert_eq!(decoded.item_stack.get_tag(&soul_tag), Some(99));

        let creative = ClientCreativeInventoryActionPacket {
            slot: 36,
            item: blade.clone(),
        };
        let mut cb = encode_packet_payload(&creative).unwrap();
        assert_eq!(read_var_int(&mut cb).unwrap(), 0x38);
        assert_eq!(
            decode_packet_exact::<ClientCreativeInventoryActionPacket>(cb).unwrap(),
            creative
        );

        let mut eq_map = BTreeMap::new();
        eq_map.insert(EquipmentSlot::MainHand, blade);
        eq_map.insert(
            EquipmentSlot::Helmet,
            ItemStack::single(Material::DIAMOND_HELMET),
        );
        let eq_pkt = EntityEquipmentPacket {
            entity_id: 12,
            equipments: eq_map,
        };
        let mut eqb = encode_packet_payload(&eq_pkt).unwrap();
        assert_eq!(read_var_int(&mut eqb).unwrap(), 0x66);
        assert_eq!(
            decode_packet_exact::<EntityEquipmentPacket>(eqb).unwrap(),
            eq_pkt
        );

        let click = ClientClickWindowPacket {
            window_id: 1,
            state_id: 2,
            slot: 0,
            button: 0,
            click_type: WindowClickMode::QuickMove,
            changed_slots: BTreeMap::from([(
                0,
                HashedStack::Item {
                    material: Material::DIAMOND,
                    amount: 16,
                    added_hashes: BTreeMap::new(),
                    removed_components: BTreeSet::new(),
                },
            )]),
            clicked_item: HashedStack::Air,
        };
        let mut clb = encode_packet_payload(&click).unwrap();
        assert_eq!(read_var_int(&mut clb).unwrap(), 0x12);
        assert_eq!(
            decode_packet_exact::<ClientClickWindowPacket>(clb).unwrap(),
            click
        );
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_inventory_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<CloseWindowPacket>(b.clone());
            let _ = decode_packet_exact::<WindowItemsPacket>(b.clone());
            let _ = decode_packet_exact::<WindowPropertyPacket>(b.clone());
            let _ = decode_packet_exact::<SetSlotPacket>(b.clone());
            let _ = decode_packet_exact::<OpenWindowPacket>(b.clone());
            let _ = decode_packet_exact::<SetCursorItemPacket>(b.clone());
            let _ = decode_packet_exact::<EntityEquipmentPacket>(b.clone());
            let _ = decode_packet_exact::<HeldItemChangePacket>(b.clone());
            let _ = decode_packet_exact::<CollectItemPacket>(b.clone());
            let _ = decode_packet_exact::<ClientClickWindowButtonPacket>(b.clone());
            let _ = decode_packet_exact::<ClientClickWindowPacket>(b.clone());
            let _ = decode_packet_exact::<ClientCloseWindowPacket>(b.clone());
            let _ = decode_packet_exact::<ClientHeldItemChangePacket>(b.clone());
            let _ = decode_packet_exact::<ClientCreativeInventoryActionPacket>(b);
        }
    }
}
