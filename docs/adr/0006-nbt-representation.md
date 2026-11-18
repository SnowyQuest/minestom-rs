# ADR 0006: NBT Representation (`minestom-nbt`)

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

NBT (Named Binary Tag) is pervasive in Minecraft 26.2 (protocol 776):
1. **Network NBT (nameless root, MC 1.20.2+):** Text `Component`s in Configuration and Play states, `RegistryData` entries, `ChunkData` heightmaps & block entities, and item data components omit the 2-byte root name string (`TagType::Compound (0x0A)` followed immediately by child tags).
2. **Disk / Anvil NBT (named root):** `.mca` region chunks, `.nbt` structures/schematics, and `level.dat` include the 2-byte modified-UTF8/UTF-8 root name string and use GZip or Zlib compression.
3. **SNBT (Stringified NBT):** Used in commands (`/give`, `/data`), Adventure hover events, debugging, and `Tag` serialization.
4. **Ergonomic Tree Manipulation:** Minestom's `Tag<T>` / `TagHandler`, `Block` entity data, and dynamic registries build, inspect, and mutate small-to-medium NBT compounds constantly.

## Evaluated Options

| Option | Pros | Cons |
|---|---|---|
| `simdnbt` | Very fast zero-copy read throughput on large `.mca` files | Heavy reliance on `unsafe` SIMD intrinsics (conflicts with workspace-wide `#![forbid(unsafe_code)]`), lifetime-infected borrowed views awkward for persistent `Block`/`ItemStack`/`TagMap` values |
| `valence_nbt` | Pure Rust, clean `Compound` API | Tied to older Valence release cycles; brings external trait/serde coupling that still requires wrapper types for protocol 776 nameless network tags + `Tag<T>` |
| **Custom `minestom-nbt` crate** | 100% safe Rust (`#![forbid(unsafe_code)]`), deterministic insertion order (`IndexMap`), native first-class support for **both** named-root and nameless network NBT, strict recursion/allocation bounds against zip-bombs/malformed packets, built-in SNBT parser & formatter, optional `serde` support, and JSON↔NBT transcoding for `MinestomData` registries | ~600–900 LOC to write and proptest |

## Decision

We implement **`minestom-nbt`** directly in the workspace as a pure, `#![forbid(unsafe_code)]` Layer-0 crate with:
- `NbtTag` (`Byte(i8)`, `Short(i16)`, `Int(i32)`, `Long(i64)`, `Float(f32)`, `Double(f64)`, `ByteArray(Vec<i8>)`, `String(String)`, `List(NbtList)`, `Compound(NbtCompound)`, `IntArray(Vec<i32>)`, `LongArray(Vec<i64>)`)
- `NbtCompound` backed by `IndexMap<String, NbtTag>` for deterministic serialization and round-trip equality
- Explicit `read_named` / `write_named` (disk/Anvil) and `read_network` / `write_network` (protocol 776 nameless root, supporting `Tag_End (0x00)` as empty/null network tag as well as arbitrary root `NbtTag`s used by text components!)
- Enforced `MAX_NBT_DEPTH = 512` and `MAX_NBT_BYTES` limits on untrusted network inputs so crafted nested lists/compounds return `Err(NbtError::DepthLimitExceeded)` instead of overflowing the stack or OOMing the server
- Built-in SNBT serializer/parser and GZip/Zlib helpers via `flate2`.

## Consequences

- Zero `unsafe` code anywhere in NBT parsing or encoding.
- Handles MC 26.2 network `Component` NBT (which can encode a bare `Tag_String (0x08)` for plain text OR a `Tag_Compound (0x0A)` for styled components!) cleanly with zero impedance mismatch.
- Comprehensive round-trip and `proptest` coverage directly inside `minestom-nbt`.
