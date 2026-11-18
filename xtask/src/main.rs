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

fn rust_const_ident(namespace_key: &str) -> String {
    let stripped = namespace_key
        .strip_prefix("minecraft:")
        .or_else(|| namespace_key.strip_prefix("brigadier:"))
        .unwrap_or(namespace_key);
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

fn parse_major_minor(version_str: &str) -> (u32, u32) {
    if let Some((maj, min)) = version_str.split_once('.') {
        (maj.parse().unwrap_or(0), min.parse().unwrap_or(0))
    } else {
        (version_str.parse().unwrap_or(0), 0)
    }
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

    let mut seen_idents = HashSet::new();
    for (id, key) in &entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{id}`).")?;
            writeln!(out, "pub const {ident}: u32 = {id};")?;
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct EntityTypeJson {
    id: u32,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    height: f64,
    #[serde(rename = "eyeHeight", default)]
    eye_height: f64,
    #[serde(rename = "packetType", default)]
    packet_type: String,
    #[serde(rename = "clientTrackingRange", default)]
    client_tracking_range: u8,
}

fn generate_entity_types(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
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

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `entity_type.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Static hitbox and tracking metadata for a vanilla entity type."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(out, "pub struct EntityTypeInfo {{")?;
    writeln!(out, "    /// Numeric protocol ID.")?;
    writeln!(out, "    pub id: u32,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Default bounding box width (X and Z).")?;
    writeln!(out, "    pub width: f64,")?;
    writeln!(out, "    /// Default bounding box height (Y).")?;
    writeln!(out, "    pub height: f64,")?;
    writeln!(out, "    /// Eye height offset from feet.")?;
    writeln!(out, "    pub eye_height: f64,")?;
    writeln!(
        out,
        "    /// Whether this entity type extends `LivingEntity`."
    )?;
    writeln!(out, "    pub living: bool,")?;
    writeln!(out, "    /// Default client tracking range in chunks.")?;
    writeln!(out, "    pub tracking_range_chunks: u8,")?;
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
        let living = e.packet_type == "LIVING" || e.packet_type == "PLAYER";
        let tr = e.client_tracking_range.max(4);
        writeln!(
            out,
            "    EntityTypeInfo {{ id: {}, key: {key:?}, width: {:.6}, height: {:.6}, eye_height: {:.6}, living: {living}, tracking_range_chunks: {tr} }},",
            e.id, e.width, e.height, e.eye_height
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    let mut seen_idents = HashSet::new();
    for (key, e) in &entries {
        let ident = rust_const_ident(key);
        if seen_idents.insert(ident.clone()) {
            writeln!(out, "/// Protocol ID for `{key}` (`{}`).", e.id)?;
            writeln!(out, "pub const {ident}: u32 = {};", e.id)?;
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct BlockStateEntryJson {
    #[serde(rename = "stateId")]
    state_id: u16,
}

#[derive(Debug, Deserialize)]
struct BlockJson {
    id: u16,
    #[serde(rename = "defaultStateId")]
    default_state_id: u16,
    #[serde(default)]
    solid: bool,
    #[serde(default)]
    air: bool,
    #[serde(default)]
    liquid: bool,
    #[serde(rename = "lightBlock", default)]
    light_block: u8,
    #[serde(rename = "lightEmission", default)]
    light_emission: u8,
    #[serde(default)]
    states: HashMap<String, BlockStateEntryJson>,
}

fn generate_block_states(files: &BTreeMap<String, Vec<u8>>) -> Result<String, XtaskError> {
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

    let total_states = usize::from(max_state_id) + 1;
    let mut state_to_block = vec![0_u16; total_states];
    let mut state_is_liquid = vec![false; total_states];
    for (key, b) in &blocks {
        let base_liquid = b.liquid || key == "minecraft:water" || key == "minecraft:lava";
        state_to_block[usize::from(b.default_state_id)] = b.id;
        for (prop_str, st) in &b.states {
            let idx = usize::from(st.state_id);
            state_to_block[idx] = b.id;
            if base_liquid || prop_str.contains("waterlogged=true") {
                state_is_liquid[idx] = true;
            }
        }
    }

    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `cargo xtask codegen` from `block.json` (`26.2-rv3`). Do not edit!"
    )?;
    writeln!(out)?;
    writeln!(out, "/// Metadata describing a vanilla block definition.")?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]")?;
    writeln!(out, "pub struct BlockInfo {{")?;
    writeln!(out, "    /// Numeric block registry ID.")?;
    writeln!(out, "    pub id: u16,")?;
    writeln!(out, "    /// Canonical `minecraft:...` resource key.")?;
    writeln!(out, "    pub key: &'static str,")?;
    writeln!(out, "    /// Default block state ID.")?;
    writeln!(out, "    pub default_state_id: u16,")?;
    writeln!(out, "    /// Whether the block is solid.")?;
    writeln!(out, "    pub solid: bool,")?;
    writeln!(
        out,
        "    /// Whether the block is air (`air`, `cave_air`, `void_air`)."
    )?;
    writeln!(out, "    pub air: bool,")?;
    writeln!(out, "    /// Light opacity (`0..=15`).")?;
    writeln!(out, "    pub light_block: u8,")?;
    writeln!(out, "    /// Emitted block light (`0..=15`).")?;
    writeln!(out, "    pub light_emission: u8,")?;
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
        "/// Indexed table of all `{}` vanilla blocks by `id`.",
        blocks.len()
    )?;
    writeln!(out, "pub static BLOCK_INFOS: &[BlockInfo] = &[")?;
    for (key, b) in &blocks {
        writeln!(
            out,
            "    BlockInfo {{ id: {}, key: {key:?}, default_state_id: {}, solid: {}, air: {}, light_block: {}, light_emission: {} }},",
            b.id, b.default_state_id, b.solid, b.air, b.light_block, b.light_emission
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
        "/// O(1) lookup indicating whether `state_id` contains fluid (`water`, `lava`, or `waterlogged=true`)."
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

    // Maps registry short name (e.g. "block" or "worldgen/biome") -> (entry_key -> numeric_id)
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
            let id = idx as u32;
            key_to_id.insert(entry_key.clone(), id);
            if reg_short == "dimension_type" && entry_key == "minecraft:overworld" {
                overworld_dim_id = id;
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
        "/// Numeric ID of `minecraft:plains` inside `CONFIGURATION_REGISTRIES` (`{plains_biome_id}`)."
    )?;
    writeln!(
        out_or(&mut dyn_out),
        "pub const PLAINS_BIOME_ID: u32 = {plains_biome_id};"
    )?;
    writeln!(
        dyn_out,
        "/// Total number of vanilla biomes (`{biome_count}`)."
    )?;
    writeln!(dyn_out, "pub const BIOME_COUNT: usize = {biome_count};")?;

    // Build all 32 client tag registries in exact `RegistriesImpl.tagRegistries` order:
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

const fn out_or(s: &mut String) -> &mut String {
    s
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

    let block_states_rs = generate_block_states(&files)?;
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

    let entity_types_rs = generate_entity_types(&files)?;
    write_or_check(
        &gen_dir.join("entity_types.rs"),
        &entity_types_rs,
        check_only,
    )?;

    let specs = [
        ("block.json", "vanilla block", "BLOCK_COUNT", "blocks.rs"),
        (
            "item.json",
            "vanilla item material",
            "MATERIAL_COUNT",
            "materials.rs",
        ),
        (
            "sound_event.json",
            "vanilla sound event",
            "SOUND_EVENT_COUNT",
            "sounds.rs",
        ),
        (
            "particle_type.json",
            "vanilla particle type",
            "PARTICLE_COUNT",
            "particles.rs",
        ),
        ("fluid.json", "vanilla fluid", "FLUID_COUNT", "fluids.rs"),
        (
            "attribute.json",
            "vanilla entity attribute",
            "ATTRIBUTE_COUNT",
            "attributes.rs",
        ),
        (
            "mob_effect.json",
            "vanilla potion/mob effect",
            "POTION_EFFECT_COUNT",
            "potion_effects.rs",
        ),
        (
            "game_event.json",
            "vanilla game event",
            "GAME_EVENT_COUNT",
            "game_events.rs",
        ),
    ];

    for (resource, doc, count_const, filename) in specs {
        let code = generate_id_constants(&files, resource, doc, count_const)?;
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
