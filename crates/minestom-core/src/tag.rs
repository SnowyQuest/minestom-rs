//! Strongly typed `Tag<T>` keys and NBT-backed `TagMap` (`TagHandler` equivalent).

use std::marker::PhantomData;

use minestom_nbt::{NbtCompound, NbtTag};
use uuid::Uuid;

/// Trait implemented by value types that can be stored in and retrieved from a [`TagMap`].
pub trait TagSerializable: Sized {
    /// Converts `self` into an [`NbtTag`].
    fn into_nbt_tag(self) -> NbtTag;
    /// Attempts to decode `Self` from an [`NbtTag`].
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self>;
}

impl TagSerializable for bool {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::Byte(i8::from(self))
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_byte().map(|b| b != 0)
    }
}

impl TagSerializable for i8 {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::Byte(self)
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_byte()
    }
}

impl TagSerializable for i16 {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::Short(self)
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_short()
    }
}

impl TagSerializable for i32 {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::Int(self)
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_int()
    }
}

impl TagSerializable for i64 {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::Long(self)
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_long()
    }
}

impl TagSerializable for f32 {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::Float(self)
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_float()
    }
}

impl TagSerializable for f64 {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::Double(self)
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_double()
    }
}

impl TagSerializable for String {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::String(self)
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_str().map(str::to_owned)
    }
}

impl TagSerializable for Uuid {
    fn into_nbt_tag(self) -> NbtTag {
        let b = self.as_u128();
        NbtTag::IntArray(vec![
            (b >> 96) as i32,
            (b >> 64) as i32,
            (b >> 32) as i32,
            b as i32,
        ])
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        if let NbtTag::IntArray(arr) = tag
            && arr.len() == 4
        {
            let hi = ((u128::from(arr[0] as u32)) << 96) | ((u128::from(arr[1] as u32)) << 64);
            let lo = ((u128::from(arr[2] as u32)) << 32) | u128::from(arr[3] as u32);
            Some(Self::from_u128(hi | lo))
        } else {
            None
        }
    }
}

impl TagSerializable for NbtTag {
    fn into_nbt_tag(self) -> NbtTag {
        self
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        Some(tag.clone())
    }
}

impl TagSerializable for NbtCompound {
    fn into_nbt_tag(self) -> NbtTag {
        NbtTag::Compound(self)
    }
    fn from_nbt_tag(tag: &NbtTag) -> Option<Self> {
        tag.as_compound().cloned()
    }
}

/// Zero-cost typed key for reading/writing metadata on blocks, items, entities, and instances.
///
/// ```
/// use minestom_core::{Tag, TagMap};
///
/// const SCORE: Tag<i32> = Tag::new("score");
/// let mut tags = TagMap::new();
/// tags.set_tag(&SCORE, 150);
/// assert_eq!(tags.get_tag(&SCORE), Some(150));
/// ```
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Tag<T> {
    key: &'static str,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Tag<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Tag<T> {}

impl<T> Tag<T> {
    /// Declares a typed `Tag<T>` with the given NBT key.
    #[must_use]
    pub const fn new(key: &'static str) -> Self {
        Self {
            key,
            _marker: PhantomData,
        }
    }

    /// Returns the string key of this tag.
    pub const fn key(&self) -> &'static str {
        self.key
    }
}

/// NBT-backed container implementing Minestom's `TagHandler` semantics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagMap {
    nbt: NbtCompound,
}

impl TagMap {
    /// Creates an empty `TagMap`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wraps an existing [`NbtCompound`].
    #[must_use]
    pub const fn from_compound(nbt: NbtCompound) -> Self {
        Self { nbt }
    }

    /// Returns `true` if no tags are stored.
    pub fn is_empty(&self) -> bool {
        self.nbt.is_empty()
    }

    /// Reads the typed value associated with `tag`.
    pub fn get_tag<T: TagSerializable>(&self, tag: &Tag<T>) -> Option<T> {
        let raw = self.nbt.get(tag.key())?;
        T::from_nbt_tag(raw)
    }

    /// Writes `value` under `tag`.
    pub fn set_tag<T: TagSerializable>(&mut self, tag: &Tag<T>, value: T) {
        self.nbt.insert(tag.key(), value.into_nbt_tag());
    }

    /// Removes `tag` and returns its previous value if it matched `T`.
    pub fn remove_tag<T: TagSerializable>(&mut self, tag: &Tag<T>) -> Option<T> {
        let old = self.nbt.remove(tag.key())?;
        T::from_nbt_tag(&old)
    }

    /// Returns `true` if `tag` is present and decodable as `T`.
    pub fn has_tag<T: TagSerializable>(&self, tag: &Tag<T>) -> bool {
        self.get_tag(tag).is_some()
    }

    /// Borrows the underlying [`NbtCompound`].
    pub const fn as_compound(&self) -> &NbtCompound {
        &self.nbt
    }
}
