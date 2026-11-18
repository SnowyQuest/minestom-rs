//! Workspace automation binary (`cargo xtask codegen`, `cargo xtask ci`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tar::Archive;

/// Pinned `net.minestom:data:26.2-rv3` JSON archive SHA-256 checksum.
const PINNED_DATA_SHA256: &str = "4c3b278f110c2c388982c530bc9de9096e0ad9dca0864fb145cde0f6343301df";

/// Exact 27 configuration dynamic registries in `RegistriesImpl.configurationRegistries` order.
const CONFIGURATION_DYNAMIC_REGISTRIES: &[&str] = &[
    "chat_type",
    "worldgen/biome",
    "dialog",
    "damage_type",
    "trim_material",
    "trim_pattern",
    "banner_pattern",
    "enchantment",
    "painting_variant",
    "jukebox_song",
    "instrument",
    "wolf_variant",
    "wolf_sound_variant",
    "cat_variant",
    "cat_sound_variant",
    "chicken_variant",
    "chicken_sound_variant",
    "cow_variant",
    "cow_sound_variant",
    "frog_variant",
    "pig_variant",
    "pig_sound_variant",
    "zombie_nautilus_variant",
    "world_clock",
    "timeline",
    "dimension_type",
    "sulfur_cube_archetype",
];

#[derive(Debug, thiserror::Error)]
enum XtaskError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error in `{file}`: {source}")]
    Json {
        file: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Formatting error: {0}")]
    Fmt(#[from] std::fmt::Error),
    #[error("Data archive SHA-256 mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("Missing JSON resource `{0}` in data archive")]
    MissingResource(String),
    #[error("Invalid shape literal `{0}` in `block.json`")]
    InvalidShape(String),
    #[error("Generated file `{0}` is out of date; run `cargo xtask codegen`")]
    OutOfDate(String),
    #[error("Command `{0}` exited with failure")]
    CommandFailed(String),
    #[error("Unknown xtask subcommand `{0}` (expected `codegen` or `ci`)")]
    UnknownCommand(String),
}

#[derive(Debug, Deserialize)]
struct ConstantsJson {
    name: String,
    friendly_name: String,
    protocol: i32,
    world: i32,
    resourcepack: String,
    datapack: String,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits inside workspace root")
        .to_path_buf()
}

fn load_data_archive(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, XtaskError> {
    let archive_path = root.join("xtask/data/minestom-data-26.2-rv3.tar.gz");
    let raw = fs::read(&archive_path)?;

    let mut hasher = Sha256::new();
    hasher.update(&raw);
    let digest = hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        });
    if digest != PINNED_DATA_SHA256 {
        return Err(XtaskError::ChecksumMismatch {
            expected: PINNED_DATA_SHA256.to_owned(),
            actual: digest,
        });
    }

    let gz = GzDecoder::new(raw.as_slice());
    let mut tar = Archive::new(gz);
    let mut files = BTreeMap::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        files.insert(path, buf);
    }
    Ok(files)
}

fn parse_json<T: for<'de> Deserialize<'de>>(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
) -> Result<T, XtaskError> {
    let bytes = files
        .get(name)
        .ok_or_else(|| XtaskError::MissingResource(name.to_owned()))?;
    serde_json::from_slice(bytes).map_err(|source| XtaskError::Json {
        file: name.to_owned(),
        source,
    })
}

fn bare_mc_key(namespace_key: &str) -> &str {
    namespace_key
        .strip_prefix("minecraft:")
        .or_else(|| namespace_key.strip_prefix("brigadier:"))
        .unwrap_or(namespace_key)
}

fn rust_const_ident(namespace_key: &str) -> String {
    let stripped = bare_mc_key(namespace_key);
    let mut ident = stripped
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    if ident.starts_with(|c: char| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    ident
}

fn fmt_f32(v: f32) -> String {
    let clean = if v == 0.0 { 0.0 } else { v };
    format!("{clean:?}_f32")
}

fn fmt_f64(v: f64) -> String {
    let clean = if v == 0.0 { 0.0 } else { v };
    format!("{clean:?}_f64")
}

fn parse_major_minor(version_str: &str) -> (u32, u32) {
    if let Some((maj, min)) = version_str.split_once('.') {
        (maj.parse().unwrap_or(0), min.parse().unwrap_or(0))
    } else {
        (version_str.parse().unwrap_or(0), 0)
    }
}

fn write_sorted_key_lookup(
    out: &mut String,
    entries: &[(u32, String)],
    id_ty: &str,
) -> Result<(), XtaskError> {
    let mut sorted: Vec<(&str, u32)> = entries
        .iter()
        .map(|(id, key)| (bare_mc_key(key.as_str()), *id))
        .collect();
    sorted.sort_by_key(|&(bare, _)| bare);

    writeln!(
        out,
        "/// Sorted `(bare_key, id)` slice for zero-allocation O(log N) lookup."
    )?;
    writeln!(out, "pub static SORTED_KEYS: &[(&str, {id_ty})] = &[")?;
    for (bare, id) in &sorted {
        writeln!(out, "    ({bare:?}, {id}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Zero-allocation O(log N) key lookup accepting `\"minecraft:foo\"` or `\"foo\"`."
    )?;
    writeln!(out, "#[must_use]")?;
    writeln!(out, "pub fn lookup_key(key: &str) -> Option<{id_ty}> {{")?;
    writeln!(
        out,
        "    let bare = key.strip_prefix(\"minecraft:\").or_else(|| (!key.contains(':')).then_some(key))?;"
    )?;
    writeln!(
        out,
        "    SORTED_KEYS.binary_search_by_key(&bare, |&(k, _)| k).ok().map(|idx| SORTED_KEYS[idx].1)"
    )?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    Ok(())
}

fn write_const_impl_macro(
    out: &mut String,
    macro_name: &str,
    entries: &[(u32, String)],
) -> Result<(), XtaskError> {
    writeln!(
        out,
        "/// Generates `pub const <IDENT>: Self = Self(<id>);` for all `{}` entries.",
        entries.len()
    )?;
    writeln!(out, "#[macro_export]")?;
    writeln!(out, "macro_rules! {macro_name} {{")?;
    writeln!(out, "    ($ty:ident) => {{")?;
    writeln!(out, "        impl $ty {{")?;
    let mut seen = HashSet::new();
    for (id, key) in entries {
        let ident = rust_const_ident(key);
        if seen.insert(ident.clone()) {
            writeln!(out, "            /// `{key}` (`{id}`).")?;
            writeln!(out, "            pub const {ident}: Self = Self({id});")?;
        }
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }};")?;
    writeln!(out, "}}")?;
    Ok(())
}

fn generate_constants(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
    let c: ConstantsJson = parse_json(files, "constants.json")?;
    let (rp_maj, rp_min) = parse_major_minor(&c.resourcepack);
    let (dp_maj, dp_min) = parse_major_minor(&c.datapack);

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `net.minestom:data:26.2-rv3`. Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(out, "/// Target Minecraft version name (`\"{}\"`).", c.name)?;
    writeln!(out, "pub const VERSION_NAME: &str = {:?};", c.name)?;
    writeln!(
        out,
        "/// Friendly display name for the target Minecraft version (`\"{}\"`).",
        c.friendly_name
    )?;
    writeln!(
        out,
        "pub const FRIENDLY_VERSION_NAME: &str = {:?};",
        c.friendly_name
    )?;
    writeln!(
        out,
        "/// Target Minecraft Java Edition network protocol version (`{}`).",
        c.protocol
    )?;
    writeln!(out, "pub const PROTOCOL_VERSION: i32 = {};", c.protocol)?;
    writeln!(
        out,
        "/// Target Minecraft Anvil/world data version (`{}`).",
        c.world
    )?;
    writeln!(out, "pub const DATA_VERSION: i32 = {};", c.world)?;
    writeln!(
        out,
        "/// Target resource pack `(major, minor)` format version (`({rp_maj}, {rp_min})`).",
    )?;
    writeln!(
        out,
        "pub const RESOURCE_PACK_VERSION: (u32, u32) = ({rp_maj}, {rp_min});"
    )?;
    writeln!(
        out,
        "/// Target data pack `(major, minor)` format version (`({dp_maj}, {dp_min})`).",
    )?;
    writeln!(
        out,
        "pub const DATA_PACK_VERSION: (u32, u32) = ({dp_maj}, {dp_min});"
    )?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct SimpleIdEntry {
    id: u32,
}

fn ordered_static_entries(
    files: &BTreeMap<String, Vec<u8>>,
    resource: &str,
) -> Result<Vec<(u32, String)>, XtaskError> {
    let map: serde_json::Map<String, serde_json::Value> = parse_json(files, resource)?;
    let mut entries: Vec<(u32, String)> = Vec::with_capacity(map.len());
    for (key, val) in map {
        let parsed: SimpleIdEntry =
            serde_json::from_value(val).map_err(|source| XtaskError::Json {
                file: resource.to_owned(),
                source,
            })?;
        entries.push((parsed.id, key));
    }
    entries.sort_by_key(|(id, _)| *id);
    Ok(entries)
}

fn generate_id_constants(
    files: &BTreeMap<String, Vec<u8>>,
    resource: &str,
    type_doc: &str,
    count_const: &str,
    macro_name: &str,
) -> Result<String, XtaskError> {
    let entries = ordered_static_entries(files, resource)?;

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `{resource}` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(out, "/// Total number of {type_doc} entries.")?;
    writeln!(out, "pub const {count_const}: usize = {};", entries.len())?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table for {type_doc}."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (id, key) in &entries {
        writeln!(out, "    ({id}, {key:?}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, macro_name, &entries)?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct AttributeJson {
    id: u32,
    #[serde(rename = "translationKey")]
    translation_key: String,
    #[serde(rename = "defaultValue")]
    default_value: f64,
    #[serde(rename = "minValue")]
    min_value: f64,
    #[serde(rename = "maxValue")]
    max_value: f64,
    #[serde(rename = "clientSync", default)]
    client_sync: bool,
}

fn generate_attributes(
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(String, HashMap<String, u16>), XtaskError> {
    let raw_map: serde_json::Map<String, serde_json::Value> = parse_json(files, "attribute.json")?;
    let mut entries: Vec<(String, AttributeJson)> = Vec::with_capacity(raw_map.len());
    for (key, val) in raw_map {
        let parsed: AttributeJson =
            serde_json::from_value(val).map_err(|source| XtaskError::Json {
                file: "attribute.json".to_owned(),
                source,
            })?;
        entries.push((key, parsed));
    }
    entries.sort_by_key(|(_, a)| a.id);

    let mut attr_key_to_id = HashMap::with_capacity(entries.len());
    let mut id_entries = Vec::with_capacity(entries.len());
    for (key, a) in &entries {
        let id_u16 = u16::try_from(a.id).expect("attribute ID fits u16");
        attr_key_to_id.insert(key.clone(), id_u16);
        id_entries.push((a.id, key.clone()));
    }

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `attribute.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static specification of a vanilla MC 26.2 entity attribute."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(out, "pub struct AttributeSpecInfo {{")?;
    writeln!(out, "    /// Numeric protocol ID (`0..40`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Vanilla translation key.")?;
    writeln!(out, "    pub translation_key: &'static str,")?;
    writeln!(out, "    /// Default base value.")?;
    writeln!(out, "    pub default_value: f64,")?;
    writeln!(out, "    /// Minimum clamped final value.")?;
    writeln!(out, "    pub min_value: f64,")?;
    writeln!(out, "    /// Maximum clamped final value.")?;
    writeln!(out, "    pub max_value: f64,")?;
    writeln!(
        out,
        "    /// Whether changes are synchronized to clients via `EntityAttributesPacket`."
    )?;
    writeln!(out, "    pub client_sync: bool,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla entity attribute entries.")?;
    writeln!(out, "pub const ATTRIBUTE_COUNT: usize = {};", entries.len())?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table for vanilla entity attribute."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (key, a) in &entries {
        writeln!(out, "    ({}, {key:?}),", a.id)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Complete MC 26.2 attribute specification table (`{}` entries).",
        entries.len()
    )?;
    writeln!(
        out,
        "pub static ATTRIBUTE_SPECS: [AttributeSpecInfo; ATTRIBUTE_COUNT] = ["
    )?;
    for (key, a) in &entries {
        writeln!(
            out,
            "    AttributeSpecInfo {{ id: {}, key: {key:?}, translation_key: {:?}, default_value: {}, min_value: {}, max_value: {}, client_sync: {} }},",
            a.id,
            a.translation_key,
            fmt_f64(a.default_value),
            fmt_f64(a.min_value),
            fmt_f64(a.max_value),
            a.client_sync
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &id_entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_attribute_constants", &id_entries)?;
    Ok((out, attr_key_to_id))
}

#[derive(Debug, Deserialize)]
struct MobEffectJson {
    id: u32,
    #[serde(rename = "translationKey")]
    translation_key: String,
    #[serde(default)]
    color: u32,
    #[serde(default)]
    instantaneous: bool,
}

fn generate_potion_effects(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
    let raw_map: serde_json::Map<String, serde_json::Value> = parse_json(files, "mob_effect.json")?;
    let mut entries: Vec<(String, MobEffectJson)> = Vec::with_capacity(raw_map.len());
    for (key, val) in raw_map {
        let parsed: MobEffectJson =
            serde_json::from_value(val).map_err(|source| XtaskError::Json {
                file: "mob_effect.json".to_owned(),
                source,
            })?;
        entries.push((key, parsed));
    }
    entries.sort_by_key(|(_, e)| e.id);
    let id_entries: Vec<(u32, String)> = entries.iter().map(|(k, e)| (e.id, k.clone())).collect();

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `mob_effect.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static metadata for a vanilla MC 26.2 potion / mob effect."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]")?;
    writeln!(out, "pub struct PotionEffectInfo {{")?;
    writeln!(out, "    /// Numeric protocol ID (`0..40`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Vanilla translation key.")?;
    writeln!(out, "    pub translation_key: &'static str,")?;
    writeln!(out, "    /// Particle RGB color (`0xRRGGBB`).")?;
    writeln!(out, "    pub color: u32,")?;
    writeln!(
        out,
        "    /// Whether this effect applies instantaneously (`instant_health`, `instant_damage`, `saturation`)."
    )?;
    writeln!(out, "    pub instantaneous: bool,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Total number of vanilla potion/mob effect entries."
    )?;
    writeln!(
        out,
        "pub const POTION_EFFECT_COUNT: usize = {};",
        entries.len()
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table for vanilla potion/mob effect."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (key, e) in &entries {
        writeln!(out, "    ({}, {key:?}),", e.id)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `PotionEffectInfo` table for all `{}` effects.",
        entries.len()
    )?;
    writeln!(
        out,
        "pub static POTION_EFFECT_INFOS: [PotionEffectInfo; POTION_EFFECT_COUNT] = ["
    )?;
    for (key, e) in &entries {
        writeln!(
            out,
            "    PotionEffectInfo {{ id: {}, key: {key:?}, translation_key: {:?}, color: {}, instantaneous: {} }},",
            e.id, e.translation_key, e.color, e.instantaneous
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &id_entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_potion_effect_constants", &id_entries)?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct ParticleJson {
    id: u32,
    #[serde(rename = "hasData", default)]
    has_data: bool,
}

fn generate_particles(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
    let raw_map: serde_json::Map<String, serde_json::Value> =
        parse_json(files, "particle_type.json")?;
    let mut entries: Vec<(String, ParticleJson)> = Vec::with_capacity(raw_map.len());
    for (key, val) in raw_map {
        let parsed: ParticleJson =
            serde_json::from_value(val).map_err(|source| XtaskError::Json {
                file: "particle_type.json".to_owned(),
                source,
            })?;
        entries.push((key, parsed));
    }
    entries.sort_by_key(|(_, p)| p.id);
    let id_entries: Vec<(u32, String)> = entries.iter().map(|(k, p)| (p.id, k.clone())).collect();

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `particle_type.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(out, "/// Static metadata for a vanilla particle type.")?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]")?;
    writeln!(out, "pub struct ParticleInfo {{")?;
    writeln!(out, "    /// Numeric protocol ID (`0..125`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(
        out,
        "    /// Whether this particle carries extra wire data."
    )?;
    writeln!(out, "    pub has_data: bool,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla particle type entries.")?;
    writeln!(out, "pub const PARTICLE_COUNT: usize = {};", entries.len())?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table for vanilla particle type."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (key, p) in &entries {
        writeln!(out, "    ({}, {key:?}),", p.id)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `ParticleInfo` table for all `{}` particle types.",
        entries.len()
    )?;
    writeln!(
        out,
        "pub static PARTICLE_INFOS: [ParticleInfo; PARTICLE_COUNT] = ["
    )?;
    for (key, p) in &entries {
        writeln!(
            out,
            "    ParticleInfo {{ id: {}, key: {key:?}, has_data: {} }},",
            p.id, p.has_data
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &id_entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_particle_constants", &id_entries)?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct FluidJson {
    id: u32,
    #[serde(rename = "bucketId", default)]
    bucket_id: Option<String>,
}

fn generate_fluids(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
    let raw_map: serde_json::Map<String, serde_json::Value> = parse_json(files, "fluid.json")?;
    let mut entries: Vec<(String, FluidJson)> = Vec::with_capacity(raw_map.len());
    for (key, val) in raw_map {
        let parsed: FluidJson = serde_json::from_value(val).map_err(|source| XtaskError::Json {
            file: "fluid.json".to_owned(),
            source,
        })?;
        entries.push((key, parsed));
    }
    entries.sort_by_key(|(_, f)| f.id);
    let id_entries: Vec<(u32, String)> = entries.iter().map(|(k, f)| (f.id, k.clone())).collect();

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `fluid.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(out, "/// Static metadata for a vanilla fluid.")?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]")?;
    writeln!(out, "pub struct FluidInfo {{")?;
    writeln!(out, "    /// Numeric protocol ID (`0..5`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Corresponding bucket item key, if any.")?;
    writeln!(out, "    pub bucket_key: Option<&'static str>,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla fluid entries.")?;
    writeln!(out, "pub const FLUID_COUNT: usize = {};", entries.len())?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table for vanilla fluid."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (key, f) in &entries {
        writeln!(out, "    ({}, {key:?}),", f.id)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `FluidInfo` table for all `{}` fluids.",
        entries.len()
    )?;
    writeln!(out, "pub static FLUID_INFOS: [FluidInfo; FLUID_COUNT] = [")?;
    for (key, f) in &entries {
        writeln!(
            out,
            "    FluidInfo {{ id: {}, key: {key:?}, bucket_key: {:?} }},",
            f.id,
            f.bucket_id.as_deref()
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &id_entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_fluid_constants", &id_entries)?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct GameEventJson {
    id: u32,
    #[serde(rename = "notificationRadius", default)]
    notification_radius: u16,
}

fn generate_game_events(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
    let raw_map: serde_json::Map<String, serde_json::Value> = parse_json(files, "game_event.json")?;
    let mut entries: Vec<(String, GameEventJson)> = Vec::with_capacity(raw_map.len());
    for (key, val) in raw_map {
        let parsed: GameEventJson =
            serde_json::from_value(val).map_err(|source| XtaskError::Json {
                file: "game_event.json".to_owned(),
                source,
            })?;
        entries.push((key, parsed));
    }
    entries.sort_by_key(|(_, g)| g.id);
    let id_entries: Vec<(u32, String)> = entries.iter().map(|(k, g)| (g.id, k.clone())).collect();

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `game_event.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static metadata for a vanilla sculk/world game event."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]")?;
    writeln!(out, "pub struct GameEventInfo {{")?;
    writeln!(out, "    /// Numeric protocol ID.")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Sculk vibration notification radius.")?;
    writeln!(out, "    pub notification_radius: u16,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla game event entries.")?;
    writeln!(
        out,
        "pub const GAME_EVENT_COUNT: usize = {};",
        entries.len()
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table for vanilla game event."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (key, g) in &entries {
        writeln!(out, "    ({}, {key:?}),", g.id)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `GameEventInfo` table for all `{}` game events.",
        entries.len()
    )?;
    writeln!(
        out,
        "pub static GAME_EVENT_INFOS: [GameEventInfo; GAME_EVENT_COUNT] = ["
    )?;
    for (key, g) in &entries {
        writeln!(
            out,
            "    GameEventInfo {{ id: {}, key: {key:?}, notification_radius: {} }},",
            g.id, g.notification_radius
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &id_entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_game_event_constants", &id_entries)?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct DamageTypeJson {
    exhaustion: f32,
    message_id: String,
    scaling: String,
}

fn generate_damage_types(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
    let map: BTreeMap<String, DamageTypeJson> = parse_json(files, "damage_type.json")?;
    let mut id_entries = Vec::with_capacity(map.len());
    for (idx, key) in map.keys().enumerate() {
        id_entries.push((idx as u32, key.clone()));
    }

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `damage_type.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static metadata for a vanilla `minecraft:damage_type` entry."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(out, "pub struct DamageTypeInfo {{")?;
    writeln!(out, "    /// Dynamic registry protocol ID (`0..51`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Death message translation stem.")?;
    writeln!(out, "    pub message_id: &'static str,")?;
    writeln!(out, "    /// Hunger exhaustion added on hit.")?;
    writeln!(out, "    pub exhaustion: f32,")?;
    writeln!(out, "    /// Difficulty scaling mode.")?;
    writeln!(out, "    pub scaling: &'static str,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla damage type entries.")?;
    writeln!(out, "pub const DAMAGE_TYPE_COUNT: usize = {};", map.len())?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table matching `CONFIGURATION_REGISTRIES`."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (id, key) in &id_entries {
        writeln!(out, "    ({id}, {key:?}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `DamageTypeInfo` table for all `{}` damage types.",
        map.len()
    )?;
    writeln!(
        out,
        "pub static DAMAGE_TYPE_INFOS: [DamageTypeInfo; DAMAGE_TYPE_COUNT] = ["
    )?;
    for (idx, (key, dt)) in map.iter().enumerate() {
        writeln!(
            out,
            "    DamageTypeInfo {{ id: {idx}, key: {key:?}, message_id: {:?}, exhaustion: {}, scaling: {:?} }},",
            dt.message_id,
            fmt_f32(dt.exhaustion),
            dt.scaling
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &id_entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Dynamic registry ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_damage_type_constants", &id_entries)?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct EnchantmentJson {
    #[serde(default)]
    anvil_cost: u8,
    #[serde(default)]
    max_level: u8,
    #[serde(default)]
    weight: u8,
}

fn generate_enchantments(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
    let map: BTreeMap<String, EnchantmentJson> = parse_json(files, "enchantment.json")?;
    let mut id_entries = Vec::with_capacity(map.len());
    for (idx, key) in map.keys().enumerate() {
        id_entries.push((idx as u32, key.clone()));
    }

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `enchantment.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static metadata for a vanilla `minecraft:enchantment` entry."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]")?;
    writeln!(out, "pub struct EnchantmentInfo {{")?;
    writeln!(out, "    /// Dynamic registry protocol ID (`0..43`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Maximum vanilla enchantment level.")?;
    writeln!(out, "    pub max_level: u8,")?;
    writeln!(out, "    /// Base anvil XP cost.")?;
    writeln!(out, "    pub anvil_cost: u8,")?;
    writeln!(out, "    /// Enchanting table selection weight.")?;
    writeln!(out, "    pub weight: u8,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla enchantment entries.")?;
    writeln!(out, "pub const ENCHANTMENT_COUNT: usize = {};", map.len())?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table matching `CONFIGURATION_REGISTRIES`."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (id, key) in &id_entries {
        writeln!(out, "    ({id}, {key:?}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `EnchantmentInfo` table for all `{}` enchantments.",
        map.len()
    )?;
    writeln!(
        out,
        "pub static ENCHANTMENT_INFOS: [EnchantmentInfo; ENCHANTMENT_COUNT] = ["
    )?;
    for (idx, (key, en)) in map.iter().enumerate() {
        writeln!(
            out,
            "    EnchantmentInfo {{ id: {idx}, key: {key:?}, max_level: {}, anvil_cost: {}, weight: {} }},",
            en.max_level, en.anvil_cost, en.weight
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &id_entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Dynamic registry ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_enchantment_constants", &id_entries)?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct PaintingVariantJson {
    width: u8,
    height: u8,
    asset_id: String,
}

fn generate_painting_variants(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
    let map: BTreeMap<String, PaintingVariantJson> = parse_json(files, "painting_variant.json")?;
    let mut id_entries = Vec::with_capacity(map.len());
    for (idx, key) in map.keys().enumerate() {
        id_entries.push((idx as u32, key.clone()));
    }

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `painting_variant.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static metadata for a vanilla `minecraft:painting_variant` entry."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]")?;
    writeln!(out, "pub struct PaintingVariantInfo {{")?;
    writeln!(out, "    /// Dynamic registry protocol ID (`0..51`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Texture asset ID.")?;
    writeln!(out, "    pub asset_id: &'static str,")?;
    writeln!(out, "    /// Painting width in blocks.")?;
    writeln!(out, "    pub width: u8,")?;
    writeln!(out, "    /// Painting height in blocks.")?;
    writeln!(out, "    pub height: u8,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla painting variant entries.")?;
    writeln!(
        out,
        "pub const PAINTING_VARIANT_COUNT: usize = {};",
        map.len()
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table matching `CONFIGURATION_REGISTRIES`."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (id, key) in &id_entries {
        writeln!(out, "    ({id}, {key:?}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `PaintingVariantInfo` table for all `{}` painting variants.",
        map.len()
    )?;
    writeln!(
        out,
        "pub static PAINTING_VARIANT_INFOS: [PaintingVariantInfo; PAINTING_VARIANT_COUNT] = ["
    )?;
    for (idx, (key, pv)) in map.iter().enumerate() {
        writeln!(
            out,
            "    PaintingVariantInfo {{ id: {idx}, key: {key:?}, asset_id: {:?}, width: {}, height: {} }},",
            pv.asset_id, pv.width, pv.height
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (id, key) in &id_entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Dynamic registry ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_painting_variant_constants", &id_entries)?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct EntityTypeJson {
    id: u32,
    #[serde(rename = "translationKey", default)]
    translation_key: String,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    height: f64,
    #[serde(rename = "eyeHeight", default)]
    eye_height: f64,
    #[serde(default)]
    drag: Option<f64>,
    #[serde(default)]
    acceleration: Option<f64>,
    #[serde(rename = "fireImmune", default)]
    fire_immune: bool,
    #[serde(rename = "packetType", default)]
    packet_type: String,
    #[serde(rename = "clientTrackingRange", default)]
    client_tracking_range: u8,
    #[serde(rename = "defaultAttributes", default)]
    default_attributes: BTreeMap<String, f64>,
}

fn generate_entity_types(
    files: &BTreeMap<String, Vec<u8>>,
    attr_key_to_id: &HashMap<String, u16>,
) -> Result<String, XtaskError> {
    let raw_map: serde_json::Map<String, serde_json::Value> =
        parse_json(files, "entity_type.json")?;
    let mut entries: Vec<(String, EntityTypeJson)> = Vec::with_capacity(raw_map.len());
    for (key, val) in raw_map {
        let parsed: EntityTypeJson =
            serde_json::from_value(val).map_err(|source| XtaskError::Json {
                file: "entity_type.json".to_owned(),
                source,
            })?;
        entries.push((key, parsed));
    }
    entries.sort_by_key(|(_, e)| e.id);
    let id_entries: Vec<(u32, String)> = entries.iter().map(|(k, e)| (e.id, k.clone())).collect();

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `entity_type.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Spawn packet category for a vanilla entity type (`packetType` in `entity_type.json`)."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]")?;
    writeln!(out, "pub enum EntityPacketType {{")?;
    writeln!(out, "    /// Standard non-living entity (`BASE`).")?;
    writeln!(out, "    Base,")?;
    writeln!(out, "    /// Living entity or mob (`LIVING`).")?;
    writeln!(out, "    Living,")?;
    writeln!(out, "    /// Player avatar (`PLAYER`).")?;
    writeln!(out, "    Player,")?;
    writeln!(out, "    /// Experience orb (`EXPERIENCE_ORB`).")?;
    writeln!(out, "    ExperienceOrb,")?;
    writeln!(out, "    /// Wall painting (`PAINTING`).")?;
    writeln!(out, "    Painting,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static hitbox, aerodynamics, tracking, and attribute metadata for a vanilla entity type."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(out, "pub struct EntityTypeInfo {{")?;
    writeln!(out, "    /// Numeric protocol ID (`0..158`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Vanilla translation key.")?;
    writeln!(out, "    pub translation_key: &'static str,")?;
    writeln!(out, "    /// Network spawn category.")?;
    writeln!(out, "    pub packet_type: EntityPacketType,")?;
    writeln!(out, "    /// Default bounding box width (X and Z).")?;
    writeln!(out, "    pub width: f64,")?;
    writeln!(out, "    /// Default bounding box height (Y).")?;
    writeln!(out, "    pub height: f64,")?;
    writeln!(out, "    /// Eye height offset from feet.")?;
    writeln!(out, "    pub eye_height: f64,")?;
    writeln!(out, "    /// Per-tick aerodynamic drag factor.")?;
    writeln!(out, "    pub drag: f64,")?;
    writeln!(
        out,
        "    /// Per-tick gravity / downward acceleration (`0.08` for living mobs)."
    )?;
    writeln!(out, "    pub acceleration: f64,")?;
    writeln!(
        out,
        "    /// Whether this entity type extends `LivingEntity`."
    )?;
    writeln!(out, "    pub living: bool,")?;
    writeln!(
        out,
        "    /// Whether this entity type is immune to fire and lava damage."
    )?;
    writeln!(out, "    pub fire_immune: bool,")?;
    writeln!(
        out,
        "    /// Exact vanilla client tracking range in chunks (`0` for markers)."
    )?;
    writeln!(out, "    pub tracking_range_chunks: u8,")?;
    writeln!(
        out,
        "    /// Default `(attribute_id, base_value)` overrides for living entities."
    )?;
    writeln!(out, "    pub default_attributes: &'static [(u16, f64)],")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla entity type entries.")?;
    writeln!(
        out,
        "pub const ENTITY_TYPE_COUNT: usize = {};",
        entries.len()
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table for vanilla entity type."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (key, e) in &entries {
        writeln!(out, "    ({}, {key:?}),", e.id)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `EntityTypeInfo` table for all `{}` vanilla entity types.",
        entries.len()
    )?;
    writeln!(out, "pub static ENTITY_INFOS: &[EntityTypeInfo] = &[")?;
    for (key, e) in &entries {
        let (pkt_variant, living) = match e.packet_type.as_str() {
            "LIVING" => ("EntityPacketType::Living", true),
            "PLAYER" => ("EntityPacketType::Player", true),
            "EXPERIENCE_ORB" => ("EntityPacketType::ExperienceOrb", false),
            "PAINTING" => ("EntityPacketType::Painting", false),
            _ => ("EntityPacketType::Base", false),
        };
        let drag = e.drag.unwrap_or(if living { 0.02 } else { 0.0 });
        let accel = e.acceleration.unwrap_or(if living { 0.08 } else { 0.0 });

        let mut attrs: Vec<(u16, f64)> = e
            .default_attributes
            .iter()
            .filter_map(|(ak, &av)| attr_key_to_id.get(ak).map(|&aid| (aid, av)))
            .collect();
        attrs.sort_by_key(|&(aid, _)| aid);

        let mut attrs_lit = String::from("&[");
        for (i, (aid, av)) in attrs.iter().enumerate() {
            if i > 0 {
                attrs_lit.push_str(", ");
            }
            let _ = write!(attrs_lit, "({aid}, {})", fmt_f64(*av));
        }
        attrs_lit.push(']');

        writeln!(
            out,
            "    EntityTypeInfo {{ id: {}, key: {key:?}, translation_key: {:?}, packet_type: {pkt_variant}, width: {}, height: {}, eye_height: {}, drag: {}, acceleration: {}, living: {living}, fire_immune: {}, tracking_range_chunks: {}, default_attributes: {attrs_lit} }},",
            e.id,
            e.translation_key,
            fmt_f64(e.width),
            fmt_f64(e.height),
            fmt_f64(e.eye_height),
            fmt_f64(drag),
            fmt_f64(accel),
            e.fire_immune,
            e.client_tracking_range
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (key, e) in &entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{}`).", e.id)?;
            writeln!(out, "pub const {ident}: u32 = {};", e.id)?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_entity_type_constants", &id_entries)?;
    Ok(out)
}

#[derive(Debug, Default, Deserialize)]
struct EquippableJson {
    slot: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AttrModEntryJson {
    #[serde(rename = "type")]
    attr_type: String,
    amount: f64,
    operation: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AttrModsJson {
    List(Vec<AttrModEntryJson>),
    Wrapped { modifiers: Vec<AttrModEntryJson> },
}

impl AttrModsJson {
    fn as_slice(&self) -> &[AttrModEntryJson] {
        match self {
            Self::List(v) => v,
            Self::Wrapped { modifiers } => modifiers,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ItemComponentsJson {
    #[serde(rename = "minecraft:max_stack_size", default)]
    max_stack_size: Option<u8>,
    #[serde(rename = "minecraft:max_damage", default)]
    max_damage: Option<u16>,
    #[serde(rename = "minecraft:equippable", default)]
    equippable: Option<EquippableJson>,
    #[serde(rename = "minecraft:attribute_modifiers", default)]
    attribute_modifiers: Option<AttrModsJson>,
}

#[derive(Debug, Deserialize)]
struct ItemJson {
    id: u32,
    #[serde(rename = "translationKey", default)]
    translation_key: String,
    #[serde(default)]
    components: ItemComponentsJson,
    #[serde(rename = "correspondingBlock", default)]
    corresponding_block: Option<String>,
}

fn generate_materials(
    files: &BTreeMap<String, Vec<u8>>,
    block_key_to_default_sid: &HashMap<String, u16>,
) -> Result<(String, HashMap<String, u16>), XtaskError> {
    let raw_map: serde_json::Map<String, serde_json::Value> = parse_json(files, "item.json")?;
    let mut entries: Vec<(String, ItemJson)> = Vec::with_capacity(raw_map.len());
    for (key, val) in raw_map {
        let parsed: ItemJson = serde_json::from_value(val).map_err(|source| XtaskError::Json {
            file: "item.json".to_owned(),
            source,
        })?;
        entries.push((key, parsed));
    }
    entries.sort_by_key(|(_, item)| item.id);

    let mut item_key_to_id = HashMap::with_capacity(entries.len());
    let mut id_entries = Vec::with_capacity(entries.len());
    for (key, item) in &entries {
        let id_u16 = u16::try_from(item.id).expect("item ID fits u16");
        item_key_to_id.insert(key.clone(), id_u16);
        id_entries.push((item.id, key.clone()));
    }

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `item.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static metadata for a vanilla item material (`minecraft:item`)."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(out, "pub struct MaterialInfo {{")?;
    writeln!(out, "    /// Numeric protocol ID (`0..1537`).")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Vanilla translation key.")?;
    writeln!(out, "    pub translation_key: &'static str,")?;
    writeln!(
        out,
        "    /// Default maximum stack size (`1`, `16`, or `64`)."
    )?;
    writeln!(out, "    pub max_stack_size: u8,")?;
    writeln!(
        out,
        "    /// Default durability (`0` if unbreakable/non-damageable)."
    )?;
    writeln!(out, "    pub max_damage: u16,")?;
    writeln!(
        out,
        "    /// Corresponding block key when placed in the world, if any."
    )?;
    writeln!(out, "    pub corresponding_block: Option<&'static str>,")?;
    writeln!(
        out,
        "    /// Default block state ID when placed in the world (`O(1)` lookup)."
    )?;
    writeln!(out, "    pub corresponding_block_state_id: Option<u16>,")?;
    writeln!(
        out,
        "    /// Default equippable armor/saddle slot (`\"head\"`, `\"chest\"`, `\"legs\"`, `\"feet\"`, `\"body\"`, `\"offhand\"`), if any."
    )?;
    writeln!(out, "    pub equippable_slot: Option<&'static str>,")?;
    writeln!(
        out,
        "    /// Additive main-hand `minecraft:attack_damage` modifier from `item.json`."
    )?;
    writeln!(out, "    pub attack_damage_bonus: f32,")?;
    writeln!(
        out,
        "    /// Additive main-hand `minecraft:attack_speed` modifier from `item.json`."
    )?;
    writeln!(out, "    pub attack_speed_bonus: f32,")?;
    writeln!(
        out,
        "    /// Additive `minecraft:armor` defense points from `item.json`."
    )?;
    writeln!(out, "    pub armor: f32,")?;
    writeln!(
        out,
        "    /// Additive `minecraft:armor_toughness` from `item.json`."
    )?;
    writeln!(out, "    pub armor_toughness: f32,")?;
    writeln!(
        out,
        "    /// Additive `minecraft:knockback_resistance` from `item.json`."
    )?;
    writeln!(out, "    pub knockback_resistance: f32,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Total number of vanilla item material entries.")?;
    writeln!(out, "pub const MATERIAL_COUNT: usize = {};", entries.len())?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(protocol_id, namespace_id)` table for vanilla item material."
    )?;
    writeln!(out, "pub static ENTRIES: &[(u32, &str)] = &[")?;
    for (key, item) in &entries {
        writeln!(out, "    ({}, {key:?}),", item.id)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed `MaterialInfo` table for all `{}` vanilla item materials.",
        entries.len()
    )?;
    writeln!(out, "pub static MATERIAL_INFOS: &[MaterialInfo] = &[")?;
    for (key, item) in &entries {
        let max_stack = item.components.max_stack_size.unwrap_or(64);
        let max_dmg = item.components.max_damage.unwrap_or(0);
        let blk = item.corresponding_block.as_deref();
        let blk_sid = blk.and_then(|bk| block_key_to_default_sid.get(bk).copied());
        let eq_slot = item
            .components
            .equippable
            .as_ref()
            .and_then(|eq| eq.slot.as_deref());

        let mut atk_dmg = 0.0_f32;
        let mut atk_spd = 0.0_f32;
        let mut armor = 0.0_f32;
        let mut toughness = 0.0_f32;
        let mut kb_res = 0.0_f32;
        if let Some(mods) = &item.components.attribute_modifiers {
            for m in mods.as_slice() {
                if m.operation == "add_value" {
                    let amt = m.amount as f32;
                    match m.attr_type.as_str() {
                        "minecraft:attack_damage" => atk_dmg += amt,
                        "minecraft:attack_speed" => atk_spd += amt,
                        "minecraft:armor" => armor += amt,
                        "minecraft:armor_toughness" => toughness += amt,
                        "minecraft:knockback_resistance" => kb_res += amt,
                        _ => {}
                    }
                }
            }
        }

        writeln!(
            out,
            "    MaterialInfo {{ id: {}, key: {key:?}, translation_key: {:?}, max_stack_size: {max_stack}, max_damage: {max_dmg}, corresponding_block: {blk:?}, corresponding_block_state_id: {blk_sid:?}, equippable_slot: {eq_slot:?}, attack_damage_bonus: {}, attack_speed_bonus: {}, armor: {}, armor_toughness: {}, knockback_resistance: {} }},",
            item.id,
            item.translation_key,
            fmt_f32(atk_dmg),
            fmt_f32(atk_spd),
            fmt_f32(armor),
            fmt_f32(toughness),
            fmt_f32(kb_res)
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    write_sorted_key_lookup(&mut out, &id_entries, "u32")?;

    let mut seen_idents = HashSet::new();
    for (key, item) in &entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{}`).", item.id)?;
            writeln!(out, "pub const {ident}: u32 = {};", item.id)?;
        }
    }
    writeln!(out)?;
    write_const_impl_macro(&mut out, "impl_material_constants", &id_entries)?;
    Ok((out, item_key_to_id))
}

#[derive(Debug, Deserialize)]
struct BlockEntityRefJson {
    id: u16,
}

#[derive(Debug, Deserialize)]
struct BlockStateEntryJson {
    #[serde(rename = "stateId")]
    state_id: u16,
    #[serde(default)]
    solid: Option<bool>,
    #[serde(rename = "blocksMotion", default)]
    blocks_motion: Option<bool>,
    #[serde(rename = "canRespawnIn", default)]
    can_respawn_in: Option<bool>,
    #[serde(default)]
    flammable: Option<bool>,
    #[serde(default)]
    fluid: Option<bool>,
    #[serde(rename = "lightBlock", default)]
    light_block: Option<u8>,
    #[serde(rename = "lightEmission", default)]
    light_emission: Option<u8>,
    #[serde(rename = "mapColorId", default)]
    map_color_id: Option<u8>,
    #[serde(rename = "redstoneConductor", default)]
    redstone_conductor: Option<bool>,
    #[serde(rename = "collisionShape", default)]
    collision_shape: Option<String>,
    #[serde(default)]
    shape: Option<String>,
}

fn default_one_f32() -> f32 {
    1.0
}

fn default_friction_f32() -> f32 {
    0.6
}

#[derive(Debug, Deserialize)]
struct BlockJson {
    id: u16,
    #[serde(rename = "translationKey", default)]
    translation_key: String,
    #[serde(rename = "correspondingItem", default)]
    corresponding_item: Option<String>,
    #[serde(rename = "defaultStateId")]
    default_state_id: u16,
    #[serde(default)]
    hardness: f32,
    #[serde(rename = "explosionResistance", default)]
    explosion_resistance: f32,
    #[serde(default = "default_friction_f32")]
    friction: f32,
    #[serde(rename = "speedFactor", default = "default_one_f32")]
    speed_factor: f32,
    #[serde(rename = "jumpFactor", default = "default_one_f32")]
    jump_factor: f32,
    #[serde(rename = "pushReaction", default)]
    push_reaction: String,
    #[serde(rename = "mapColorId", default)]
    map_color_id: u8,
    #[serde(default)]
    solid: bool,
    #[serde(rename = "blocksMotion", default)]
    blocks_motion: bool,
    #[serde(default)]
    air: bool,
    #[serde(default)]
    liquid: bool,
    #[serde(default)]
    fluid: bool,
    #[serde(default)]
    replaceable: bool,
    #[serde(default)]
    occludes: bool,
    #[serde(rename = "requiresTool", default)]
    requires_tool: bool,
    #[serde(default)]
    flammable: bool,
    #[serde(default)]
    gravity: bool,
    #[serde(rename = "redstoneConductor", default)]
    redstone_conductor: bool,
    #[serde(rename = "signalSource", default)]
    signal_source: bool,
    #[serde(rename = "canRespawnIn", default)]
    can_respawn_in: bool,
    #[serde(rename = "lightBlock", default)]
    light_block: u8,
    #[serde(rename = "lightEmission", default)]
    light_emission: u8,
    #[serde(rename = "collisionShape", default)]
    collision_shape: String,
    #[serde(default)]
    shape: String,
    #[serde(rename = "blockEntity", default)]
    block_entity: Option<BlockEntityRefJson>,
    #[serde(default)]
    states: BTreeMap<String, BlockStateEntryJson>,
}

type ShapeBoxBits = [u32; 6];

fn parse_shape_boxes(raw: &str) -> Result<Vec<ShapeBoxBits>, XtaskError> {
    let s = raw.trim();
    if s.is_empty() || s == "[]" {
        return Ok(Vec::new());
    }
    let inner = s
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .ok_or_else(|| XtaskError::InvalidShape(raw.to_owned()))?;

    let mut out = Vec::new();
    let mut rest = inner.trim();
    while !rest.is_empty() {
        let after_prefix = rest
            .strip_prefix("AABB[")
            .ok_or_else(|| XtaskError::InvalidShape(raw.to_owned()))?;
        let (min_part, after_min) = after_prefix
            .split_once("] -> [")
            .ok_or_else(|| XtaskError::InvalidShape(raw.to_owned()))?;
        let (max_part, tail) = after_min
            .split_once(']')
            .ok_or_else(|| XtaskError::InvalidShape(raw.to_owned()))?;

        let mut coords = [0_u32; 6];
        let mut idx = 0;
        for part in [min_part, max_part] {
            for num_s in part.split(',') {
                if idx >= 6 {
                    return Err(XtaskError::InvalidShape(raw.to_owned()));
                }
                let val: f32 = num_s
                    .trim()
                    .parse()
                    .map_err(|_| XtaskError::InvalidShape(raw.to_owned()))?;
                let clean = if val == 0.0 { 0.0_f32 } else { val };
                coords[idx] = clean.to_bits();
                idx += 1;
            }
        }
        if idx != 6 {
            return Err(XtaskError::InvalidShape(raw.to_owned()));
        }
        out.push(coords);
        rest = tail.trim().strip_prefix(',').unwrap_or(tail).trim();
    }
    Ok(out)
}

fn intern_shape(
    raw: &str,
    map: &mut HashMap<Vec<ShapeBoxBits>, u16>,
    list: &mut Vec<Vec<ShapeBoxBits>>,
) -> Result<u16, XtaskError> {
    let boxes = parse_shape_boxes(raw)?;
    if let Some(&idx) = map.get(&boxes) {
        return Ok(idx);
    }
    let idx = u16::try_from(list.len()).expect("shape count fits u16");
    list.push(boxes.clone());
    map.insert(boxes, idx);
    Ok(idx)
}

fn recover_ordered_block_properties(b: &BlockJson) -> Vec<(String, Vec<String>)> {
    let Some((sample_key, _)) = b.states.iter().next() else {
        return Vec::new();
    };
    let trimmed = sample_key.trim_matches(['[', ']']);
    if trimmed.is_empty() {
        return Vec::new();
    }
    let prop_names: Vec<String> = trimmed
        .split(',')
        .filter_map(|kv| kv.split_once('=').map(|(k, _)| k.to_owned()))
        .collect();

    let mut min_sid_per_val: Vec<BTreeMap<String, u16>> = vec![BTreeMap::new(); prop_names.len()];
    for (st_str, st_obj) in &b.states {
        let inner = st_str.trim_matches(['[', ']']);
        for (idx, pair) in inner.split(',').enumerate() {
            if let Some((_, val)) = pair.split_once('=')
                && let Some(map) = min_sid_per_val.get_mut(idx)
            {
                map.entry(val.to_owned())
                    .and_modify(|cur| *cur = (*cur).min(st_obj.state_id))
                    .or_insert(st_obj.state_id);
            }
        }
    }

    prop_names
        .into_iter()
        .zip(min_sid_per_val)
        .map(|(pname, vmap)| {
            let mut vals: Vec<(String, u16)> = vmap.into_iter().collect();
            vals.sort_by_key(|(_, sid)| *sid);
            (pname, vals.into_iter().map(|(v, _)| v).collect())
        })
        .collect()
}

fn parse_raw_block_default_map(
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<HashMap<String, u16>, XtaskError> {
    #[derive(Deserialize)]
    struct MinimalBlock {
        #[serde(rename = "defaultStateId")]
        default_state_id: u16,
    }
    let raw_map: serde_json::Map<String, serde_json::Value> = parse_json(files, "block.json")?;
    let mut out = HashMap::with_capacity(raw_map.len());
    for (k, v) in raw_map {
        let mb: MinimalBlock = serde_json::from_value(v).map_err(|source| XtaskError::Json {
            file: "block.json".to_owned(),
            source,
        })?;
        out.insert(k, mb.default_state_id);
    }
    Ok(out)
}

#[allow(clippy::too_many_lines)]
fn generate_block_states(
    files: &BTreeMap<String, Vec<u8>>,
    item_key_to_id: &HashMap<String, u16>,
) -> Result<String, XtaskError> {
    #[derive(Clone, Copy, Default)]
    struct RawStateRec {
        block_id: u16,
        collision_idx: u16,
        outline_idx: u16,
        flags: u16,
        light_block: u8,
        light_emission: u8,
        map_color_id: u8,
    }

    const FLAG_SOLID: u16 = 1 << 0;
    const FLAG_BLOCKS_MOTION: u16 = 1 << 1;
    const FLAG_AIR: u16 = 1 << 2;
    const FLAG_LIQUID: u16 = 1 << 3;
    const FLAG_FLUID: u16 = 1 << 4;
    const FLAG_REPLACEABLE: u16 = 1 << 5;
    const FLAG_OCCLUDES: u16 = 1 << 6;
    const FLAG_REQUIRES_TOOL: u16 = 1 << 7;
    const FLAG_FLAMMABLE: u16 = 1 << 8;
    const FLAG_REDSTONE_CONDUCTOR: u16 = 1 << 9;
    const FLAG_SIGNAL_SOURCE: u16 = 1 << 10;
    const FLAG_CAN_RESPAWN_IN: u16 = 1 << 11;

    let raw_map: serde_json::Map<String, serde_json::Value> = parse_json(files, "block.json")?;
    let mut blocks: Vec<(String, BlockJson)> = Vec::with_capacity(raw_map.len());
    let mut max_state_id = 0_u16;

    for (key, val) in raw_map {
        let parsed: BlockJson = serde_json::from_value(val).map_err(|source| XtaskError::Json {
            file: "block.json".to_owned(),
            source,
        })?;
        max_state_id = max_state_id.max(parsed.default_state_id);
        for st in parsed.states.values() {
            max_state_id = max_state_id.max(st.state_id);
        }
        blocks.push((key, parsed));
    }
    blocks.sort_by_key(|(_, b)| b.id);

    let mut shape_map: HashMap<Vec<ShapeBoxBits>, u16> = HashMap::new();
    let mut shape_list: Vec<Vec<ShapeBoxBits>> = Vec::new();
    // Ensure index 0 is always the empty shape `&[]`.
    let empty_idx = intern_shape("[]", &mut shape_map, &mut shape_list)?;
    assert_eq!(empty_idx, 0);

    let total_states = usize::from(max_state_id) + 1;
    let mut state_to_block = vec![0_u16; total_states];
    let mut state_is_liquid = vec![false; total_states];
    let mut state_recs = vec![RawStateRec::default(); total_states];

    for (key, b) in &blocks {
        let base_liquid = b.liquid || key == "minecraft:water" || key == "minecraft:lava";
        for (prop_str, st) in &b.states {
            let idx = usize::from(st.state_id);
            state_to_block[idx] = b.id;

            let solid = st.solid.unwrap_or(b.solid);
            let blocks_motion = st.blocks_motion.unwrap_or(b.blocks_motion);
            let fluid =
                st.fluid.unwrap_or(b.fluid) || base_liquid || prop_str.contains("waterlogged=true");
            let flammable = st.flammable.unwrap_or(b.flammable);
            let redstone_cond = st.redstone_conductor.unwrap_or(b.redstone_conductor);
            let can_respawn = st.can_respawn_in.unwrap_or(b.can_respawn_in);
            let light_blk = st.light_block.unwrap_or(b.light_block);
            let light_emit = st.light_emission.unwrap_or(b.light_emission);
            let map_col = st.map_color_id.unwrap_or(b.map_color_id);

            let col_raw = st
                .collision_shape
                .as_deref()
                .unwrap_or(b.collision_shape.as_str());
            let out_raw = st.shape.as_deref().unwrap_or(b.shape.as_str());
            let col_idx = intern_shape(col_raw, &mut shape_map, &mut shape_list)?;
            let out_idx = intern_shape(out_raw, &mut shape_map, &mut shape_list)?;

            let mut flags = 0_u16;
            if solid {
                flags |= FLAG_SOLID;
            }
            if blocks_motion {
                flags |= FLAG_BLOCKS_MOTION;
            }
            if b.air {
                flags |= FLAG_AIR;
            }
            if base_liquid {
                flags |= FLAG_LIQUID;
            }
            if fluid {
                flags |= FLAG_FLUID;
            }
            if b.replaceable {
                flags |= FLAG_REPLACEABLE;
            }
            if b.occludes {
                flags |= FLAG_OCCLUDES;
            }
            if b.requires_tool {
                flags |= FLAG_REQUIRES_TOOL;
            }
            if flammable {
                flags |= FLAG_FLAMMABLE;
            }
            if redstone_cond {
                flags |= FLAG_REDSTONE_CONDUCTOR;
            }
            if b.signal_source {
                flags |= FLAG_SIGNAL_SOURCE;
            }
            if can_respawn {
                flags |= FLAG_CAN_RESPAWN_IN;
            }

            state_is_liquid[idx] = fluid;
            state_recs[idx] = RawStateRec {
                block_id: b.id,
                collision_idx: col_idx,
                outline_idx: out_idx,
                flags,
                light_block: light_blk,
                light_emission: light_emit,
                map_color_id: map_col,
            };
        }
    }

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `block.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Ordered `(property_name, allowed_values)` schema slice for a vanilla block."
    )?;
    writeln!(
        out,
        "pub type BlockPropertiesSlice = &'static [(&'static str, &'static [&'static str])];"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Sub-voxel axis-aligned bounding box in block-local `[0.0, 1.0]` coordinates."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(out, "pub struct BlockAabb {{")?;
    writeln!(out, "    /// Minimum local X.")?;
    writeln!(out, "    pub min_x: f32,")?;
    writeln!(out, "    /// Minimum local Y.")?;
    writeln!(out, "    pub min_y: f32,")?;
    writeln!(out, "    /// Minimum local Z.")?;
    writeln!(out, "    pub min_z: f32,")?;
    writeln!(out, "    /// Maximum local X.")?;
    writeln!(out, "    pub max_x: f32,")?;
    writeln!(out, "    /// Maximum local Y.")?;
    writeln!(out, "    pub max_y: f32,")?;
    writeln!(out, "    /// Maximum local Z.")?;
    writeln!(out, "    pub max_z: f32,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Piston push reaction behavior (`pushReaction` in `block.json`)."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]")?;
    writeln!(out, "pub enum PushReaction {{")?;
    writeln!(out, "    /// Pushed and pulled normally (`NORMAL`).")?;
    writeln!(out, "    Normal,")?;
    writeln!(out, "    /// Destroyed when pushed (`DESTROY`).")?;
    writeln!(out, "    Destroy,")?;
    writeln!(out, "    /// Immovable by pistons (`BLOCK`).")?;
    writeln!(out, "    Block,")?;
    writeln!(
        out,
        "    /// Pushed only; not pulled by sticky pistons (`PUSH_ONLY`)."
    )?;
    writeln!(out, "    PushOnly,")?;
    writeln!(out, "    /// Ignored (`IGNORE`).")?;
    writeln!(out, "    Ignore,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "/// Per-state bitflag: `solid == true`.")?;
    writeln!(out, "pub const STATE_FLAG_SOLID: u16 = {FLAG_SOLID};")?;
    writeln!(out, "/// Per-state bitflag: `blocksMotion == true`.")?;
    writeln!(
        out,
        "pub const STATE_FLAG_BLOCKS_MOTION: u16 = {FLAG_BLOCKS_MOTION};"
    )?;
    writeln!(out, "/// Per-state bitflag: `air == true`.")?;
    writeln!(out, "pub const STATE_FLAG_AIR: u16 = {FLAG_AIR};")?;
    writeln!(
        out,
        "/// Per-state bitflag: pure liquid block (`water` or `lava`)."
    )?;
    writeln!(out, "pub const STATE_FLAG_LIQUID: u16 = {FLAG_LIQUID};")?;
    writeln!(
        out,
        "/// Per-state bitflag: contains fluid (`water`, `lava`, `kelp`, `seagrass`, or `waterlogged=true`)."
    )?;
    writeln!(out, "pub const STATE_FLAG_FLUID: u16 = {FLAG_FLUID};")?;
    writeln!(
        out,
        "/// Per-state bitflag: replaceable by block placement (`air`, `short_grass`, `snow`, `water`, etc.)."
    )?;
    writeln!(
        out,
        "pub const STATE_FLAG_REPLACEABLE: u16 = {FLAG_REPLACEABLE};"
    )?;
    writeln!(out, "/// Per-state bitflag: `occludes == true`.")?;
    writeln!(out, "pub const STATE_FLAG_OCCLUDES: u16 = {FLAG_OCCLUDES};")?;
    writeln!(
        out,
        "/// Per-state bitflag: `requiresTool == true` for item drops."
    )?;
    writeln!(
        out,
        "pub const STATE_FLAG_REQUIRES_TOOL: u16 = {FLAG_REQUIRES_TOOL};"
    )?;
    writeln!(out, "/// Per-state bitflag: `flammable == true`.")?;
    writeln!(
        out,
        "pub const STATE_FLAG_FLAMMABLE: u16 = {FLAG_FLAMMABLE};"
    )?;
    writeln!(out, "/// Per-state bitflag: `redstoneConductor == true`.")?;
    writeln!(
        out,
        "pub const STATE_FLAG_REDSTONE_CONDUCTOR: u16 = {FLAG_REDSTONE_CONDUCTOR};"
    )?;
    writeln!(out, "/// Per-state bitflag: `signalSource == true`.")?;
    writeln!(
        out,
        "pub const STATE_FLAG_SIGNAL_SOURCE: u16 = {FLAG_SIGNAL_SOURCE};"
    )?;
    writeln!(out, "/// Per-state bitflag: `canRespawnIn == true`.")?;
    writeln!(
        out,
        "pub const STATE_FLAG_CAN_RESPAWN_IN: u16 = {FLAG_CAN_RESPAWN_IN};"
    )?;
    writeln!(out)?;
    writeln!(out, "/// Metadata describing a vanilla block definition.")?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(out, "pub struct BlockInfo {{")?;
    writeln!(out, "    /// Numeric block registry ID (`0..1196`).")?;
    writeln!(out, "    pub id: u16,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Vanilla translation key.")?;
    writeln!(out, "    pub translation_key: &'static str,")?;
    writeln!(
        out,
        "    /// Corresponding item `Material` numeric ID, if any."
    )?;
    writeln!(out, "    pub corresponding_item_id: Option<u16>,")?;
    writeln!(
        out,
        "    /// Lowest block state ID belonging to this block."
    )?;
    writeln!(out, "    pub first_state_id: u16,")?;
    writeln!(
        out,
        "    /// Number of contiguous block states belonging to this block."
    )?;
    writeln!(out, "    pub state_count: u16,")?;
    writeln!(out, "    /// Default block state ID.")?;
    writeln!(out, "    pub default_state_id: u16,")?;
    writeln!(
        out,
        "    /// Optional vanilla block-entity type ID (`minecraft:block_entity_type`)."
    )?;
    writeln!(out, "    pub block_entity_id: Option<u16>,")?;
    writeln!(out, "    /// Mining hardness (`-1.0` for unbreakable).")?;
    writeln!(out, "    pub hardness: f32,")?;
    writeln!(out, "    /// Blast resistance.")?;
    writeln!(out, "    pub explosion_resistance: f32,")?;
    writeln!(
        out,
        "    /// Slipperiness friction (`0.6` default, `0.98` ice)."
    )?;
    writeln!(out, "    pub friction: f32,")?;
    writeln!(
        out,
        "    /// Movement speed multiplier (`0.4` soul sand/honey, `1.0` default)."
    )?;
    writeln!(out, "    pub speed_factor: f32,")?;
    writeln!(
        out,
        "    /// Jump velocity multiplier (`0.5` honey, `1.0` default)."
    )?;
    writeln!(out, "    pub jump_factor: f32,")?;
    writeln!(out, "    /// Piston push reaction.")?;
    writeln!(out, "    pub push_reaction: PushReaction,")?;
    writeln!(out, "    /// Default solid flag.")?;
    writeln!(out, "    pub solid: bool,")?;
    writeln!(
        out,
        "    /// Whether the block is air (`air`, `cave_air`, `void_air`)."
    )?;
    writeln!(out, "    pub air: bool,")?;
    writeln!(
        out,
        "    /// Whether the block falls under gravity (`sand`, `gravel`, `anvil`)."
    )?;
    writeln!(out, "    pub gravity: bool,")?;
    writeln!(out, "    /// Default light opacity (`0..=15`).")?;
    writeln!(out, "    pub light_block: u8,")?;
    writeln!(out, "    /// Default emitted block light (`0..=15`).")?;
    writeln!(out, "    pub light_emission: u8,")?;
    writeln!(
        out,
        "    /// Mixed-radix property definitions `(name, values)`."
    )?;
    writeln!(out, "    pub properties: BlockPropertiesSlice,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Compact 12-byte per-state record (`STATE_INFOS[state_id]`) with exact state overrides."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]")?;
    writeln!(out, "pub struct StateInfo {{")?;
    writeln!(out, "    /// Owning `BlockInfo` ID (`0..1196`).")?;
    writeln!(out, "    pub block_id: u16,")?;
    writeln!(
        out,
        "    /// Index into `BLOCK_SHAPES` for physical collision AABBs."
    )?;
    writeln!(out, "    pub collision_shape_idx: u16,")?;
    writeln!(
        out,
        "    /// Index into `BLOCK_SHAPES` for selection/outline AABBs."
    )?;
    writeln!(out, "    pub outline_shape_idx: u16,")?;
    writeln!(out, "    /// Bitmask of `STATE_FLAG_*` constants.")?;
    writeln!(out, "    pub flags: u16,")?;
    writeln!(out, "    /// Per-state light opacity (`0..=15`).")?;
    writeln!(out, "    pub light_block: u8,")?;
    writeln!(out, "    /// Per-state emitted block light (`0..=15`).")?;
    writeln!(out, "    pub light_emission: u8,")?;
    writeln!(out, "    /// Per-state map color ID.")?;
    writeln!(out, "    pub map_color_id: u8,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Total number of vanilla block states (`{total_states}`)."
    )?;
    writeln!(out, "pub const TOTAL_BLOCK_STATES: usize = {total_states};")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Deduplicated voxel AABB shape table (`{}` unique shapes across all `{total_states}` states).",
        shape_list.len()
    )?;
    writeln!(out, "pub static BLOCK_SHAPES: &[&[BlockAabb]] = &[")?;
    for boxes in &shape_list {
        if boxes.is_empty() {
            writeln!(out, "    &[],")?;
        } else {
            write!(out, "    &[")?;
            for (bi, coords) in boxes.iter().enumerate() {
                if bi > 0 {
                    write!(out, ", ")?;
                }
                write!(
                    out,
                    "BlockAabb {{ min_x: {}, min_y: {}, min_z: {}, max_x: {}, max_y: {}, max_z: {} }}",
                    fmt_f32(f32::from_bits(coords[0])),
                    fmt_f32(f32::from_bits(coords[1])),
                    fmt_f32(f32::from_bits(coords[2])),
                    fmt_f32(f32::from_bits(coords[3])),
                    fmt_f32(f32::from_bits(coords[4])),
                    fmt_f32(f32::from_bits(coords[5]))
                )?;
            }
            writeln!(out, "],")?;
        }
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Indexed table of all `{}` vanilla blocks by `id`.",
        blocks.len()
    )?;
    writeln!(out, "pub static BLOCK_INFOS: &[BlockInfo] = &[")?;
    for (key, b) in &blocks {
        let first_sid = b
            .states
            .values()
            .map(|s| s.state_id)
            .min()
            .unwrap_or(b.default_state_id);
        let state_count = u16::try_from(b.states.len().max(1)).expect("state count fits u16");
        let be_opt = b.block_entity.as_ref().map(|be| be.id);
        let item_opt = b
            .corresponding_item
            .as_deref()
            .and_then(|ik| item_key_to_id.get(ik).copied());
        let push_variant = match b.push_reaction.as_str() {
            "DESTROY" => "PushReaction::Destroy",
            "BLOCK" => "PushReaction::Block",
            "PUSH_ONLY" => "PushReaction::PushOnly",
            "IGNORE" => "PushReaction::Ignore",
            _ => "PushReaction::Normal",
        };
        let props = recover_ordered_block_properties(b);

        for (st_str, st_obj) in &b.states {
            let inner = st_str.trim_matches(['[', ']']);
            if !inner.is_empty() {
                let mut offset = 0_usize;
                for ((_, allowed), pair) in props.iter().zip(inner.split(',')) {
                    let (_, val) = pair.split_once('=').expect("prop=val");
                    let pos = allowed
                        .iter()
                        .position(|item| item == val)
                        .expect("value in allowed");
                    offset = offset * allowed.len() + pos;
                }
                assert_eq!(first_sid as usize + offset, st_obj.state_id as usize);
            }
        }

        let mut props_lit = String::from("&[");
        for (pi, (pname, pvals)) in props.iter().enumerate() {
            if pi > 0 {
                props_lit.push_str(", ");
            }
            let _ = write!(props_lit, "({pname:?}, &[");
            for (vi, pv) in pvals.iter().enumerate() {
                if vi > 0 {
                    props_lit.push_str(", ");
                }
                let _ = write!(props_lit, "{pv:?}");
            }
            props_lit.push_str("])");
        }
        props_lit.push(']');

        writeln!(
            out,
            "    BlockInfo {{ id: {}, key: {key:?}, translation_key: {:?}, corresponding_item_id: {item_opt:?}, first_state_id: {first_sid}, state_count: {state_count}, default_state_id: {}, block_entity_id: {be_opt:?}, hardness: {}, explosion_resistance: {}, friction: {}, speed_factor: {}, jump_factor: {}, push_reaction: {push_variant}, solid: {}, air: {}, gravity: {}, light_block: {}, light_emission: {}, properties: {props_lit} }},",
            b.id,
            b.translation_key,
            b.default_state_id,
            fmt_f32(b.hardness),
            fmt_f32(b.explosion_resistance),
            fmt_f32(b.friction),
            fmt_f32(b.speed_factor),
            fmt_f32(b.jump_factor),
            b.solid,
            b.air,
            b.gravity,
            b.light_block,
            b.light_emission
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// O(1) per-state table for all `{total_states}` vanilla block states."
    )?;
    writeln!(out, "pub static STATE_INFOS: &[StateInfo] = &[")?;
    for r in &state_recs {
        writeln!(
            out,
            "    StateInfo {{ block_id: {}, collision_shape_idx: {}, outline_shape_idx: {}, flags: {}, light_block: {}, light_emission: {}, map_color_id: {} }},",
            r.block_id,
            r.collision_idx,
            r.outline_idx,
            r.flags,
            r.light_block,
            r.light_emission,
            r.map_color_id
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// O(1) lookup mapping `state_id` -> owning `block_id`."
    )?;
    writeln!(out, "pub static STATE_TO_BLOCK_ID: &[u16] = &[")?;
    for chunk in state_to_block.chunks(24) {
        write!(out, "   ")?;
        for id in chunk {
            write!(out, " {id},")?;
        }
        writeln!(out)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// O(1) lookup indicating whether `state_id` contains fluid (`water`, `lava`, `kelp`, or `waterlogged=true`)."
    )?;
    writeln!(out, "pub static STATE_IS_LIQUID: &[bool] = &[")?;
    for chunk in state_is_liquid.chunks(24) {
        write!(out, "   ")?;
        for flag in chunk {
            write!(out, " {flag},")?;
        }
        writeln!(out)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    // Sorted `(bare_key, default_state_id)` for zero-alloc O(log N) `Block::from_key`!
    let mut sorted_blocks: Vec<(&str, u16)> = blocks
        .iter()
        .map(|(k, b)| (bare_mc_key(k.as_str()), b.default_state_id))
        .collect();
    sorted_blocks.sort_by_key(|&(bare, _)| bare);
    writeln!(
        out,
        "/// Sorted `(bare_key, default_state_id)` table for zero-allocation O(log N) `Block::from_key`."
    )?;
    writeln!(out, "pub static SORTED_BLOCK_KEYS: &[(&str, u16)] = &[")?;
    for (bare, sid) in &sorted_blocks {
        writeln!(out, "    ({bare:?}, {sid}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Zero-allocation O(log N) lookup returning the default state ID for `\"minecraft:foo\"` or `\"foo\"`."
    )?;
    writeln!(out, "#[must_use]")?;
    writeln!(
        out,
        "pub fn lookup_block_default_state(key: &str) -> Option<u16> {{"
    )?;
    writeln!(
        out,
        "    let bare = key.strip_prefix(\"minecraft:\").or_else(|| (!key.contains(':')).then_some(key))?;"
    )?;
    writeln!(
        out,
        "    SORTED_BLOCK_KEYS.binary_search_by_key(&bare, |&(k, _)| k).ok().map(|idx| SORTED_BLOCK_KEYS[idx].1)"
    )?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    let mut seen = HashSet::new();
    for (key, b) in &blocks {
        let ident = rust_const_ident(key);
        if seen.insert(ident.clone()) {
            writeln!(
                out,
                "/// Default state ID for `{key}` (`{}`).",
                b.default_state_id
            )?;
            writeln!(
                out,
                "pub const STATE_{ident}: u16 = {};",
                b.default_state_id
            )?;
        }
    }
    writeln!(out)?;

    writeln!(
        out,
        "/// Generates `pub const <BLOCK>: Self = Self(<default_state_id>);` for all `{}` vanilla blocks.",
        blocks.len()
    )?;
    writeln!(out, "#[macro_export]")?;
    writeln!(out, "macro_rules! impl_block_constants {{")?;
    writeln!(out, "    ($ty:ident) => {{")?;
    writeln!(out, "        impl $ty {{")?;
    let mut seen_macro = HashSet::new();
    for (key, b) in &blocks {
        let ident = rust_const_ident(key);
        if seen_macro.insert(ident.clone()) {
            writeln!(
                out,
                "            /// `{key}` (default state `{}`).",
                b.default_state_id
            )?;
            writeln!(
                out,
                "            pub const {ident}: Self = Self({});",
                b.default_state_id
            )?;
        }
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }};")?;
    writeln!(out, "}}")?;

    Ok(out)
}

#[derive(Debug, Deserialize)]
struct TagEntryJson {
    values: Vec<String>,
}

fn resolve_tag_recursive<'a>(
    tag_name: &'a str,
    raw_tags: &'a BTreeMap<String, TagEntryJson>,
    key_to_id: &HashMap<String, u32>,
    visiting: &mut HashSet<&'a str>,
    out_ids: &mut Vec<u32>,
    seen_ids: &mut HashSet<u32>,
) {
    if !visiting.insert(tag_name) {
        return;
    }
    if let Some(tag_def) = raw_tags.get(tag_name) {
        for item in &tag_def.values {
            if let Some(sub_tag) = item.strip_prefix('#') {
                resolve_tag_recursive(sub_tag, raw_tags, key_to_id, visiting, out_ids, seen_ids);
            } else if let Some(&id) = key_to_id.get(item)
                && seen_ids.insert(id)
            {
                out_ids.push(id);
            }
        }
    }
    visiting.remove(tag_name);
}

#[allow(clippy::too_many_lines)]
fn generate_dynamic_registries_and_tags(
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(String, String), XtaskError> {
    let mut dyn_out = String::new();
    writeln!(
        dyn_out,
        "// @generated by `cargo xtask codegen` from `net.minestom:data:26.2-rv3`. Do not edit!"
    )?;
    writeln!(dyn_out)?;
    writeln!(
        dyn_out,
        "/// Static descriptor of a vanilla configuration dynamic registry."
    )?;
    writeln!(dyn_out, "#[derive(Debug, Clone, Copy)]")?;
    writeln!(dyn_out, "pub struct VanillaDynamicRegistry {{")?;
    writeln!(
        dyn_out,
        "    /// Full registry key (e.g. `\"minecraft:dimension_type\"`)."
    )?;
    writeln!(dyn_out, "    pub registry_id: &'static str,")?;
    writeln!(
        dyn_out,
        "    /// Ordered `(entry_key, compact_json_payload)` slice matching numeric IDs `0..len`."
    )?;
    writeln!(
        dyn_out,
        "    pub entries: &'static [(&'static str, &'static str)],"
    )?;
    writeln!(dyn_out, "}}")?;
    writeln!(dyn_out)?;

    let mut registry_key_maps: HashMap<String, HashMap<String, u32>> = HashMap::new();

    for (short_name, res_file) in [
        ("block", "block.json"),
        ("entity_type", "entity_type.json"),
        ("fluid", "fluid.json"),
        ("game_event", "game_event.json"),
        ("item", "item.json"),
    ] {
        let entries = ordered_static_entries(files, res_file)?;
        let map = entries.into_iter().map(|(id, k)| (k, id)).collect();
        registry_key_maps.insert(short_name.to_owned(), map);
    }

    let mut overworld_dim_id = 0_u32;
    let mut overworld_caves_dim_id = 0_u32;
    let mut the_nether_dim_id = 0_u32;
    let mut the_end_dim_id = 0_u32;
    let mut plains_biome_id = 0_u32;
    let mut biome_count = 0_usize;

    writeln!(
        dyn_out,
        "/// All 27 MC 26.2 configuration dynamic registries in exact wire synchronization order."
    )?;
    writeln!(
        dyn_out,
        "pub static CONFIGURATION_REGISTRIES: &[VanillaDynamicRegistry] = &["
    )?;

    for &reg_short in CONFIGURATION_DYNAMIC_REGISTRIES {
        let res_name = format!("{reg_short}.json");
        let obj: BTreeMap<String, serde_json::Value> = parse_json(files, &res_name)?;

        let mut key_to_id = HashMap::with_capacity(obj.len());
        writeln!(dyn_out, "    VanillaDynamicRegistry {{")?;
        writeln!(dyn_out, "        registry_id: \"minecraft:{reg_short}\",")?;
        writeln!(dyn_out, "        entries: &[")?;
        for (idx, (entry_key, entry_val)) in obj.iter().enumerate() {
            let id = u32::try_from(idx).expect("registry index fits u32");
            key_to_id.insert(entry_key.clone(), id);
            if reg_short == "dimension_type" {
                match entry_key.as_str() {
                    "minecraft:overworld" => overworld_dim_id = id,
                    "minecraft:overworld_caves" => overworld_caves_dim_id = id,
                    "minecraft:the_nether" => the_nether_dim_id = id,
                    "minecraft:the_end" => the_end_dim_id = id,
                    _ => {}
                }
            }
            if reg_short == "worldgen/biome" && entry_key == "minecraft:plains" {
                plains_biome_id = id;
            }
            let json_str = serde_json::to_string(entry_val).map_err(|source| XtaskError::Json {
                file: res_name.clone(),
                source,
            })?;
            writeln!(dyn_out, "            ({entry_key:?}, {json_str:?}),")?;
        }
        if reg_short == "worldgen/biome" {
            biome_count = obj.len();
        }
        writeln!(dyn_out, "        ],")?;
        writeln!(dyn_out, "    }},")?;
        registry_key_maps.insert(reg_short.to_owned(), key_to_id);
    }
    writeln!(dyn_out, "];")?;
    writeln!(dyn_out)?;
    writeln!(
        dyn_out,
        "/// Numeric ID of `minecraft:overworld` inside `CONFIGURATION_REGISTRIES` (`{overworld_dim_id}`)."
    )?;
    writeln!(
        dyn_out,
        "pub const OVERWORLD_DIMENSION_TYPE_ID: u32 = {overworld_dim_id};"
    )?;
    writeln!(
        dyn_out,
        "/// Numeric ID of `minecraft:overworld_caves` inside `CONFIGURATION_REGISTRIES` (`{overworld_caves_dim_id}`)."
    )?;
    writeln!(
        dyn_out,
        "pub const OVERWORLD_CAVES_DIMENSION_TYPE_ID: u32 = {overworld_caves_dim_id};"
    )?;
    writeln!(
        dyn_out,
        "/// Numeric ID of `minecraft:the_nether` inside `CONFIGURATION_REGISTRIES` (`{the_nether_dim_id}`)."
    )?;
    writeln!(
        dyn_out,
        "pub const THE_NETHER_DIMENSION_TYPE_ID: u32 = {the_nether_dim_id};"
    )?;
    writeln!(
        dyn_out,
        "/// Numeric ID of `minecraft:the_end` inside `CONFIGURATION_REGISTRIES` (`{the_end_dim_id}`)."
    )?;
    writeln!(
        dyn_out,
        "pub const THE_END_DIMENSION_TYPE_ID: u32 = {the_end_dim_id};"
    )?;
    writeln!(
        dyn_out,
        "/// Numeric ID of `minecraft:plains` inside `CONFIGURATION_REGISTRIES` (`{plains_biome_id}`)."
    )?;
    writeln!(
        dyn_out,
        "pub const PLAINS_BIOME_ID: u32 = {plains_biome_id};"
    )?;
    writeln!(
        dyn_out,
        "/// Total number of vanilla biomes (`{biome_count}`)."
    )?;
    writeln!(dyn_out, "pub const BIOME_COUNT: usize = {biome_count};")?;

    let mut tag_reg_order = vec!["block", "entity_type", "fluid", "game_event", "item"];
    tag_reg_order.extend_from_slice(CONFIGURATION_DYNAMIC_REGISTRIES);

    let mut tag_out = String::new();
    writeln!(
        tag_out,
        "// @generated by `cargo xtask codegen` from `tags/*.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(tag_out)?;
    writeln!(tag_out, "/// Type alias for a static registry tag table.")?;
    writeln!(
        tag_out,
        "pub type TagGroupSlice = [(&'static str, &'static [u32])];"
    )?;
    writeln!(tag_out)?;
    writeln!(
        tag_out,
        "/// Pre-resolved `(registry_id, &[(tag_identifier, &[protocol_entry_ids])])` for `TagsPacket`."
    )?;
    writeln!(
        tag_out,
        "pub static CLIENT_TAG_REGISTRIES: &[(&str, &TagGroupSlice)] = &["
    )?;

    for reg_short in tag_reg_order {
        let tag_file = format!("tags/{reg_short}.json");
        let full_reg = format!("minecraft:{reg_short}");
        write!(tag_out, "    ({full_reg:?}, &[")?;
        if files.contains_key(&tag_file) {
            let raw_tags: BTreeMap<String, TagEntryJson> = parse_json(files, &tag_file)?;
            let empty_map = HashMap::new();
            let key_to_id = registry_key_maps.get(reg_short).unwrap_or(&empty_map);
            writeln!(tag_out)?;
            for tag_key in raw_tags.keys() {
                let mut visiting = HashSet::new();
                let mut ids = Vec::new();
                let mut seen_ids = HashSet::new();
                resolve_tag_recursive(
                    tag_key,
                    &raw_tags,
                    key_to_id,
                    &mut visiting,
                    &mut ids,
                    &mut seen_ids,
                );
                write!(tag_out, "        ({tag_key:?}, &[")?;
                for (i, id) in ids.iter().enumerate() {
                    if i > 0 {
                        write!(tag_out, ", ")?;
                    }
                    write!(tag_out, "{id}")?;
                }
                writeln!(tag_out, "]),")?;
            }
            writeln!(tag_out, "    ]),")?;
        } else {
            writeln!(tag_out, "]),")?;
        }
    }
    writeln!(tag_out, "];")?;

    Ok((dyn_out, tag_out))
}

fn write_or_check(path: &Path, content: &str, check_only: bool) -> Result<(), XtaskError> {
    if check_only {
        let existing = fs::read_to_string(path).unwrap_or_default();
        if existing != content {
            return Err(XtaskError::OutOfDate(path.display().to_string()));
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(())
}

fn run_codegen(check_only: bool) -> Result<(), XtaskError> {
    let root = workspace_root();
    let files = load_data_archive(&root)?;
    let gen_dir = root.join("crates/minestom-data/src/generated");

    let constants_rs = generate_constants(&files)?;
    write_or_check(&gen_dir.join("constants.rs"), &constants_rs, check_only)?;

    let block_default_map = parse_raw_block_default_map(&files)?;
    let (materials_rs, item_key_to_id) = generate_materials(&files, &block_default_map)?;
    write_or_check(&gen_dir.join("materials.rs"), &materials_rs, check_only)?;

    let block_states_rs = generate_block_states(&files, &item_key_to_id)?;
    write_or_check(
        &gen_dir.join("block_states.rs"),
        &block_states_rs,
        check_only,
    )?;

    let (dyn_regs_rs, reg_tags_rs) = generate_dynamic_registries_and_tags(&files)?;
    write_or_check(
        &gen_dir.join("dynamic_registries.rs"),
        &dyn_regs_rs,
        check_only,
    )?;
    write_or_check(&gen_dir.join("registry_tags.rs"), &reg_tags_rs, check_only)?;

    let (attributes_rs, attr_key_to_id) = generate_attributes(&files)?;
    write_or_check(&gen_dir.join("attributes.rs"), &attributes_rs, check_only)?;

    let entity_types_rs = generate_entity_types(&files, &attr_key_to_id)?;
    write_or_check(
        &gen_dir.join("entity_types.rs"),
        &entity_types_rs,
        check_only,
    )?;

    let potion_effects_rs = generate_potion_effects(&files)?;
    write_or_check(
        &gen_dir.join("potion_effects.rs"),
        &potion_effects_rs,
        check_only,
    )?;

    let particles_rs = generate_particles(&files)?;
    write_or_check(&gen_dir.join("particles.rs"), &particles_rs, check_only)?;

    let fluids_rs = generate_fluids(&files)?;
    write_or_check(&gen_dir.join("fluids.rs"), &fluids_rs, check_only)?;

    let game_events_rs = generate_game_events(&files)?;
    write_or_check(&gen_dir.join("game_events.rs"), &game_events_rs, check_only)?;

    let damage_types_rs = generate_damage_types(&files)?;
    write_or_check(
        &gen_dir.join("damage_types.rs"),
        &damage_types_rs,
        check_only,
    )?;

    let enchantments_rs = generate_enchantments(&files)?;
    write_or_check(
        &gen_dir.join("enchantments.rs"),
        &enchantments_rs,
        check_only,
    )?;

    let painting_variants_rs = generate_painting_variants(&files)?;
    write_or_check(
        &gen_dir.join("painting_variants.rs"),
        &painting_variants_rs,
        check_only,
    )?;

    let specs = [
        (
            "block.json",
            "vanilla block",
            "BLOCK_COUNT",
            "impl_block_id_constants",
            "blocks.rs",
        ),
        (
            "block_entity_type.json",
            "vanilla block entity type",
            "BLOCK_ENTITY_TYPE_COUNT",
            "impl_block_entity_type_constants",
            "block_entity_types.rs",
        ),
        (
            "sound_event.json",
            "vanilla sound event",
            "SOUND_EVENT_COUNT",
            "impl_sound_event_constants",
            "sounds.rs",
        ),
        (
            "villager_profession.json",
            "vanilla villager profession",
            "VILLAGER_PROFESSION_COUNT",
            "impl_villager_profession_constants",
            "villager_professions.rs",
        ),
    ];

    for (resource, doc, count_const, macro_name, filename) in specs {
        let code = generate_id_constants(&files, resource, doc, count_const, macro_name)?;
        write_or_check(&gen_dir.join(filename), &code, check_only)?;
    }

    Ok(())
}

fn run_cmd(
    root: &Path,
    prog: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<(), XtaskError> {
    let label = format!("{prog} {}", args.join(" "));
    let mut cmd = Command::new(prog);
    cmd.current_dir(root).args(args);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err(XtaskError::CommandFailed(label));
    }
    Ok(())
}

fn run_ci() -> Result<(), XtaskError> {
    let root = workspace_root();
    run_codegen(true)?;
    run_cmd(&root, "cargo", &["fmt", "--all", "--", "--check"], &[])?;
    run_cmd(
        &root,
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
        &[],
    )?;
    run_cmd(
        &root,
        "cargo",
        &["nextest", "run", "--workspace", "--all-features"],
        &[],
    )?;
    run_cmd(&root, "cargo", &["test", "--workspace", "--doc"], &[])?;
    run_cmd(
        &root,
        "cargo",
        &["doc", "--workspace", "--no-deps", "--all-features"],
        &[("RUSTDOCFLAGS", "-D warnings")],
    )?;
    run_cmd(&root, "cargo", &["deny", "check"], &[])?;
    Ok(())
}

fn try_main() -> Result<(), XtaskError> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("codegen") => {
            let check = args.any(|a| a == "--check");
            run_codegen(check)
        }
        Some("ci") => run_ci(),
        Some(other) => Err(XtaskError::UnknownCommand(other.to_owned())),
        None => Err(XtaskError::UnknownCommand(String::new())),
    }
}

fn main() -> ExitCode {
    if let Err(err) = try_main() {
        eprintln!("xtask error: {err}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
