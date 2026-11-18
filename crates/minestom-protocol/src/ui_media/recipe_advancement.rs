//! Advancements, Toast notifications, Recipes (`RecipePropertySet`), and MC 26.2 Dialog packets.

use std::collections::BTreeMap;
use std::fmt;

use bytes::{Buf, BufMut};
use minestom_item::ItemStack;
use minestom_macros::PacketMeta;
use minestom_nbt::NbtTag;
use minestom_text::Component;

use super::scoreboard::{read_string_list, write_string_list};
use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_prefixed_vec, read_string, read_var_int, write_prefixed_vec, write_string, write_var_int,
};

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
    /// Icon item stack (`ItemStackTemplate`: non-empty item stack).
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

/// Strongly typed vanilla `RecipeProperty` set identifier (`RecipeProperty` in Minestom 26.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecipePropertySet {
    /// `"smithing_base"`.
    SmithingBase,
    /// `"smithing_template"`.
    SmithingTemplate,
    /// `"smithing_addition"`.
    SmithingAddition,
    /// `"furnace_input"`.
    FurnaceInput,
    /// `"blast_furnace_input"`.
    BlastFurnaceInput,
    /// `"smoker_input"`.
    SmokerInput,
    /// `"campfire_input"`.
    CampfireInput,
}

impl RecipePropertySet {
    /// Wire key string for [`DeclareRecipesPacket`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SmithingBase => "smithing_base",
            Self::SmithingTemplate => "smithing_template",
            Self::SmithingAddition => "smithing_addition",
            Self::FurnaceInput => "furnace_input",
            Self::BlastFurnaceInput => "blast_furnace_input",
            Self::SmokerInput => "smoker_input",
            Self::CampfireInput => "campfire_input",
        }
    }
}

impl fmt::Display for RecipePropertySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<RecipePropertySet> for String {
    fn from(value: RecipePropertySet) -> Self {
        value.as_str().to_owned()
    }
}

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
        write_var_int(buf, (self.input_item_ids.len() as i32).saturating_add(1));
        for &id in &self.input_item_ids {
            write_var_int(buf, id as i32);
        }
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
    /// Map from [`RecipePropertySet`] minimal key (e.g. `"furnace_input"`, `"smithing_base"`)
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
