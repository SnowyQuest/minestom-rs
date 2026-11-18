//! Lossless-where-possible JSON ↔ NBT conversion used for vanilla data registries.

use serde_json::{Map, Number, Value};

use crate::{NbtCompound, NbtTag};

/// Converts a `serde_json::Value` (such as MinestomDataGenerator JSON) into an [`NbtTag`].
///
/// Booleans map to `TAG_Byte(0 | 1)`, integers fit into `TAG_Int` or `TAG_Long`,
/// floating-point values map to `TAG_Double` (or `TAG_Float` where representable when needed,
/// defaulting to `TAG_Double` / `TAG_Float` rules), arrays map to `TAG_List`, and objects
/// map to `TAG_Compound` omitting `null` fields.
#[must_use]
pub fn json_to_nbt(value: &Value) -> NbtTag {
    match value {
        Value::Null => NbtTag::End,
        Value::Bool(b) => NbtTag::Byte(i8::from(*b)),
        Value::Number(num) => {
            if let Some(i) = num.as_i64() {
                if let Ok(i32_val) = i32::try_from(i) {
                    NbtTag::Int(i32_val)
                } else {
                    NbtTag::Long(i)
                }
            } else if let Some(u) = num.as_u64() {
                NbtTag::Long(u as i64)
            } else if let Some(f) = num.as_f64() {
                // Note: Minecraft registry floats (e.g. ambient_light, exhaustion, scaling)
                // accept TAG_Float or TAG_Double via Mojang DynamicOps; using f32 when exact
                // or f64 otherwise preserves precision cleanly.
                let as_f32 = f as f32;
                if f64::from(as_f32).to_bits() == f.to_bits() {
                    NbtTag::Float(as_f32)
                } else {
                    NbtTag::Double(f)
                }
            } else {
                NbtTag::Int(0)
            }
        }
        Value::String(s) => NbtTag::String(s.clone()),
        Value::Array(arr) => {
            let items = arr
                .iter()
                .filter(|v| !v.is_null())
                .map(json_to_nbt)
                .collect();
            NbtTag::List(items)
        }
        Value::Object(obj) => {
            let mut compound = NbtCompound::with_capacity(obj.len());
            for (k, v) in obj {
                if !v.is_null() {
                    compound.insert(k.clone(), json_to_nbt(v));
                }
            }
            NbtTag::Compound(compound)
        }
    }
}

/// Converts an [`NbtTag`] into an equivalent `serde_json::Value`.
#[must_use]
pub fn nbt_to_json(tag: &NbtTag) -> Value {
    match tag {
        NbtTag::End => Value::Null,
        NbtTag::Byte(v) => Value::Number(Number::from(*v)),
        NbtTag::Short(v) => Value::Number(Number::from(*v)),
        NbtTag::Int(v) => Value::Number(Number::from(*v)),
        NbtTag::Long(v) => Value::Number(Number::from(*v)),
        NbtTag::Float(v) => Number::from_f64(f64::from(*v)).map_or(Value::Null, Value::Number),
        NbtTag::Double(v) => Number::from_f64(*v).map_or(Value::Null, Value::Number),
        NbtTag::ByteArray(arr) => Value::Array(
            arr.iter()
                .map(|&b| Value::Number(Number::from(b)))
                .collect(),
        ),
        NbtTag::String(s) => Value::String(s.clone()),
        NbtTag::List(list) => Value::Array(list.iter().map(nbt_to_json).collect()),
        NbtTag::Compound(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), nbt_to_json(v));
            }
            Value::Object(out)
        }
        NbtTag::IntArray(arr) => Value::Array(
            arr.iter()
                .map(|&n| Value::Number(Number::from(n)))
                .collect(),
        ),
        NbtTag::LongArray(arr) => Value::Array(
            arr.iter()
                .map(|&n| Value::Number(Number::from(n)))
                .collect(),
        ),
    }
}
