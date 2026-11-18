# ADR 0005: Paletted Chunk Storage, Packet Caching, Generation, and Lighting

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

World chunk storage and chunk packet serialization dominate both memory footprint and join/movement bandwidth in Minecraft servers. In Minecraft 26.2 (protocol 776), each 16x16x16 `Section` encodes:
- Non-air block count (`i16`)
- Block states `PalettedContainer` (4,096 entries: single-value 0 bpe, indirect palette 4..=8 bpe, or direct global palette `ceil(log2(NUM_BLOCK_STATES))` = 15 bpe)
- Biomes `PalettedContainer` (64 entries of 4x4x4 cells: single-value 0 bpe, indirect palette 1..=3 bpe, or direct palette)
Entries are bit-packed into `i64`/`u64` words **without spanning across 64-bit word boundaries** (since MC 1.16+).

## Decision

1. **Wire-Native `Palette` Representation:**
   - `Palette` stores entries in the exact vanilla non-spanning 64-bit word layout:
     - `PaletteStorage::Single(u32)` — 2 bytes in memory! An empty air section or solid stone section uses virtually zero RAM.
     - `PaletteStorage::Indirect { bits_per_entry: u8, palette: SmallVec<[u32; 16]>, data: Box<[u64]> }` — automatically resizes upward when distinct values exceed `1 << bits_per_entry`, and compacts on request.
     - `PaletteStorage::Direct { bits_per_entry: u8, data: Box<[u64]> }` — direct global state IDs.
   - Because in-memory bit packing matches protocol 776 verbatim, serializing a `Section` copies `data` words directly to the packet buffer without per-block bit-twiddling!
2. **Cached `ChunkDataPacket` Buffer:**
   - Each `Chunk` holds `cached_packet: Option<Bytes>` invalidated atomically/mutably whenever `set_block`, `set_biome`, block-entity NBT, or lighting mutates the chunk.
   - Multiple players entering the same unmodified chunk share the exact same `Bytes` reference-counted slice (`O(1)` clone), matching Minestom's `CachedPacket` optimization.
3. **Parallel `Generator` API:**
   - `Generator` (`Fn(&mut GenerationUnit) + Send + Sync`) operates on chunk-aligned or multi-chunk `GenerationUnit`s via `UnitModifier` (`set_block`, `set_relative`, `fill`, `fill_height`, `fill_biome`), executed in parallel on `rayon` workers when generating chunk regions.
4. **Pluggable `ChunkLoader` & Anvil Format:**
   - `ChunkLoader` async-friendly trait (`load_chunk`, `save_chunk`), with `AnvilLoader` reading/writing vanilla `.mca` region files (`r.x.z.mca`, 4 KiB sector tables, Zlib/GZip chunk NBT, MC 26.2 `sections` format) on blocking I/O worker threads. The trait stays open for Polar-style compact formats.
5. **Opt-in `LightingChunk`:**
   - Sky light and block light nibble arrays (`2048` bytes per lit section) are computed via BFS flood-fill using `minestom-data` `light_block` opacity and light emission values when lighting is enabled on the `InstanceContainer`, defaulting to full skylight when disabled for lightweight minigame instances.

## Consequences

- Flat or sparse worlds consume minimal memory thanks to `PaletteStorage::Single`.
- Chunk streaming to joining/sprinting players avoids redundant serialization and zlib work via `CachedPacket` (`Bytes`).
- Vanilla Anvil worlds can be loaded without blocking Tokio network tasks or game tick threads.
