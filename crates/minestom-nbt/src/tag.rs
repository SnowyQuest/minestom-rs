//! Core `NbtTag`, `NbtCompound`, and `TagType` definitions.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::NbtError;

/// Discriminant byte IDs for the 13 Minecraft NBT tag types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TagType {
    /// `0x00` — Marks the end of a compound or an empty network tag.
    End = 0,
    /// `0x01` — Signed 8-bit integer (`i8`).
    Byte = 1,
    /// `0x02` — Signed 16-bit integer (`i16`).
    Short = 2,
    /// `0x03` — Signed 32-bit integer (`i32`).
    Int = 3,
    /// `0x04` — Signed 64-bit integer (`i64`).
    Long = 4,
    /// `0x05` — 32-bit IEEE-754 float (`f32`).
    Float = 5,
    /// `0x06` — 64-bit IEEE-754 float (`f64`).
    Double = 6,
    /// `0x07` — Length-prefixed `Vec<i8>`.
    ByteArray = 7,
    /// `0x08` — UTF-8 string (`u16` byte length prefix).
    String = 8,
    /// `0x09` — Homogeneous (or compound-wrapped) sequence of tags.
    List = 9,
    /// `0x0A` — Ordered string-keyed map of tags.
    Compound = 10,
    /// `0x0B` — Length-prefixed `Vec<i32>`.
    IntArray = 11,
    /// `0x0C` — Length-prefixed `Vec<i64>`.
    LongArray = 12,
}

impl TagType {
    /// Converts a wire discriminant byte into a [`TagType`].
    pub const fn from_id(id: u8) -> Result<Self, NbtError> {
        match id {
            0 => Ok(Self::End),
            1 => Ok(Self::Byte),
            2 => Ok(Self::Short),
            3 => Ok(Self::Int),
            4 => Ok(Self::Long),
            5 => Ok(Self::Float),
            6 => Ok(Self::Double),
            7 => Ok(Self::ByteArray),
            8 => Ok(Self::String),
            9 => Ok(Self::List),
            10 => Ok(Self::Compound),
            11 => Ok(Self::IntArray),
            12 => Ok(Self::LongArray),
            other => Err(NbtError::InvalidTagId(other)),
        }
    }

    /// Returns the wire discriminant byte for this tag type.
    pub const fn id(self) -> u8 {
        self as u8
    }

    /// Minimum wire byte size of a single payload of this tag type.
    pub const fn min_wire_bytes(self) -> usize {
        match self {
            Self::End => 0,
            Self::Byte | Self::Compound => 1,
            Self::Short | Self::String => 2,
            Self::Int | Self::Float | Self::ByteArray | Self::IntArray | Self::LongArray => 4,
            Self::List => 5,
            Self::Long | Self::Double => 8,
        }
    }
}

/// Ordered string-keyed NBT map (`TAG_Compound`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NbtCompound {
    entries: IndexMap<String, NbtTag>,
}

impl NbtCompound {
    /// Creates an empty `NbtCompound`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty `NbtCompound` with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: IndexMap::with_capacity(capacity),
        }
    }

    /// Returns the number of entries in this compound.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if this compound contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Inserts a key-value pair into the compound, returning any previous tag.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<NbtTag>) -> Option<NbtTag> {
        self.entries.insert(key.into(), value.into())
    }

    /// Builder-style helper that inserts a key-value pair and returns `self`.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<NbtTag>) -> Self {
        self.insert(key, value);
        self
    }

    /// Looks up a tag by key.
    pub fn get(&self, key: &str) -> Option<&NbtTag> {
        self.entries.get(key)
    }

    /// Looks up a mutable tag by key.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut NbtTag> {
        self.entries.get_mut(key)
    }

    /// Removes a key from the compound, preserving order of remaining keys via shift.
    pub fn remove(&mut self, key: &str) -> Option<NbtTag> {
        self.entries.shift_remove(key)
    }

    /// Returns `true` if `key` exists in this compound.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Iterates over `(key, value)` pairs in insertion order.
    pub fn iter(&self) -> indexmap::map::Iter<'_, String, NbtTag> {
        self.entries.iter()
    }

    /// Reads a boolean (`TAG_Byte != 0`).
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_byte().map(|b| b != 0)
    }

    /// Reads an `i8` (`TAG_Byte`).
    pub fn get_byte(&self, key: &str) -> Option<i8> {
        self.get(key)?.as_byte()
    }

    /// Reads an `i16` (`TAG_Short`).
    pub fn get_short(&self, key: &str) -> Option<i16> {
        self.get(key)?.as_short()
    }

    /// Reads an `i32` (`TAG_Int`).
    pub fn get_int(&self, key: &str) -> Option<i32> {
        self.get(key)?.as_int()
    }

    /// Reads an `i64` (`TAG_Long`).
    pub fn get_long(&self, key: &str) -> Option<i64> {
        self.get(key)?.as_long()
    }

    /// Reads an `f32` (`TAG_Float`).
    pub fn get_float(&self, key: &str) -> Option<f32> {
        self.get(key)?.as_float()
    }

    /// Reads an `f64` (`TAG_Double`).
    pub fn get_double(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_double()
    }

    /// Reads a `&str` (`TAG_String`).
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    /// Reads a nested `&NbtCompound` (`TAG_Compound`).
    pub fn get_compound(&self, key: &str) -> Option<&NbtCompound> {
        self.get(key)?.as_compound()
    }

    /// Reads a nested `&[NbtTag]` (`TAG_List`).
    pub fn get_list(&self, key: &str) -> Option<&[NbtTag]> {
        self.get(key)?.as_list()
    }
}

impl<'a> IntoIterator for &'a NbtCompound {
    type Item = (&'a String, &'a NbtTag);
    type IntoIter = indexmap::map::Iter<'a, String, NbtTag>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl IntoIterator for NbtCompound {
    type Item = (String, NbtTag);
    type IntoIter = indexmap::map::IntoIter<String, NbtTag>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

/// Owned value representation of any Minecraft NBT tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NbtTag {
    /// Empty / end marker (`0x00`), used on the network wire for null optional NBT.
    End,
    /// `TAG_Byte` (`i8`).
    Byte(i8),
    /// `TAG_Short` (`i16`).
    Short(i16),
    /// `TAG_Int` (`i32`).
    Int(i32),
    /// `TAG_Long` (`i64`).
    Long(i64),
    /// `TAG_Float` (`f32`).
    Float(f32),
    /// `TAG_Double` (`f64`).
    Double(f64),
    /// `TAG_Byte_Array` (`Vec<i8>`).
    ByteArray(Vec<i8>),
    /// `TAG_String` (`String`).
    String(String),
    /// `TAG_List` (`Vec<NbtTag>`).
    List(Vec<NbtTag>),
    /// `TAG_Compound` (`NbtCompound`).
    Compound(NbtCompound),
    /// `TAG_Int_Array` (`Vec<i32>`).
    IntArray(Vec<i32>),
    /// `TAG_Long_Array` (`Vec<i64>`).
    LongArray(Vec<i64>),
}

impl NbtTag {
    /// Returns the [`TagType`] discriminant of this tag.
    pub const fn tag_type(&self) -> TagType {
        match self {
            Self::End => TagType::End,
            Self::Byte(_) => TagType::Byte,
            Self::Short(_) => TagType::Short,
            Self::Int(_) => TagType::Int,
            Self::Long(_) => TagType::Long,
            Self::Float(_) => TagType::Float,
            Self::Double(_) => TagType::Double,
            Self::ByteArray(_) => TagType::ByteArray,
            Self::String(_) => TagType::String,
            Self::List(_) => TagType::List,
            Self::Compound(_) => TagType::Compound,
            Self::IntArray(_) => TagType::IntArray,
            Self::LongArray(_) => TagType::LongArray,
        }
    }

    /// Extracts an `i8` if this is `NbtTag::Byte`.
    pub const fn as_byte(&self) -> Option<i8> {
        if let Self::Byte(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extracts an `i16` if this is `NbtTag::Short`.
    pub const fn as_short(&self) -> Option<i16> {
        if let Self::Short(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extracts an `i32` if this is `NbtTag::Int`.
    pub const fn as_int(&self) -> Option<i32> {
        if let Self::Int(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extracts an `i64` if this is `NbtTag::Long`.
    pub const fn as_long(&self) -> Option<i64> {
        if let Self::Long(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extracts an `f32` if this is `NbtTag::Float`.
    pub const fn as_float(&self) -> Option<f32> {
        if let Self::Float(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extracts an `f64` if this is `NbtTag::Double`.
    pub const fn as_double(&self) -> Option<f64> {
        if let Self::Double(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extracts a `&str` if this is `NbtTag::String`.
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// Extracts a `&NbtCompound` if this is `NbtTag::Compound`.
    pub const fn as_compound(&self) -> Option<&NbtCompound> {
        if let Self::Compound(c) = self {
            Some(c)
        } else {
            None
        }
    }

    /// Extracts a `&[NbtTag]` if this is `NbtTag::List`.
    pub fn as_list(&self) -> Option<&[NbtTag]> {
        if let Self::List(l) = self {
            Some(l)
        } else {
            None
        }
    }
}

impl From<bool> for NbtTag {
    fn from(v: bool) -> Self {
        Self::Byte(i8::from(v))
    }
}

impl From<i8> for NbtTag {
    fn from(v: i8) -> Self {
        Self::Byte(v)
    }
}

impl From<i16> for NbtTag {
    fn from(v: i16) -> Self {
        Self::Short(v)
    }
}

impl From<i32> for NbtTag {
    fn from(v: i32) -> Self {
        Self::Int(v)
    }
}

impl From<i64> for NbtTag {
    fn from(v: i64) -> Self {
        Self::Long(v)
    }
}

impl From<f32> for NbtTag {
    fn from(v: f32) -> Self {
        Self::Float(v)
    }
}

impl From<f64> for NbtTag {
    fn from(v: f64) -> Self {
        Self::Double(v)
    }
}

impl From<String> for NbtTag {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<&str> for NbtTag {
    fn from(v: &str) -> Self {
        Self::String(v.to_owned())
    }
}

impl From<NbtCompound> for NbtTag {
    fn from(v: NbtCompound) -> Self {
        Self::Compound(v)
    }
}

impl From<Vec<NbtTag>> for NbtTag {
    fn from(v: Vec<NbtTag>) -> Self {
        Self::List(v)
    }
}
