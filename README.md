# `minestom-rs`

A production-quality, modular **Minecraft: Java Edition 26.2** (protocol **776**, data version **4903**) server library written from scratch in **100% safe, idiomatic Rust** (`#![forbid(unsafe_code)]`) with zero vanilla code—inspired by [Minestom](https://github.com/Minestom/Minestom).

| Target Property | Value |
|---|---|
| **Minecraft Version** | `26.2` |
| **Protocol Version** | `776` |
| **World Data Version** | `4903` |
| **Resource / Data Pack Format** | `88.0` / `107.1` |
| **Pinned Data Artifact** | `net.minestom:data:26.2-rv3` (offline deterministic codegen) |
| **Rust Edition / Toolchain** | Edition `2024`, Rust `1.100.0`, `#![forbid(unsafe_code)]` |

---

## Key Features

- **Zero Global Mutable State:** `MinecraftServer::builder().build()?` produces a cheaply cloneable handle—spin up multiple isolated servers in parallel integration tests on `127.0.0.1:0`.
- **Deterministic 20 TPS Architecture:** Async Tokio TCP sessions (`PacketReader` / `PacketWriter` with Zlib framing and streaming AES-128-CFB8 encryption) bridge via bounded `flume` channels into a dedicated `Ms-TickScheduler` thread with `SlotMap` generational arenas and lock-free `ArcSwap<ServerSnapshot>` publishing.
- **Full World & Persistence Stack:** Paletted `Chunk`/`Section`s (`Single`, `Indirect`, `Direct` MC 26.2 `RAW_LONGS` format) with zero-copy `Bytes` packet caching (`13.9 ns` clone), parallel `rayon` `Generator`, `SharedInstance`, `.mca` `AnvilLoader` (MC 26.2 `dimensions/<ns>/<val>/region` + classic fallback), 3D BFS sky/block `LightingChunk`, `WorldBorder`, multi-clock `WorldTime`, and `Weather`.
- **Production Authentication & Proxies:** Offline mode, Online mode (RSA-1024 PKCS#1 v1.5, AES-128-CFB8, Mojang SHA-1 two's-complement session verification), Velocity modern forwarding (`velocity:player_info` HMAC-SHA256), BungeeCord/BungeeGuard forwarding, and O(1) token-bucket packet rate limiting.
- **Rich Gameplay, UI & Media Parity:**
  - Hierarchical `EventNode` tree, domain filters, expiring listeners, and cached `ListenerHandle<E>` (`47 ns` dispatch).
  - Brigadier-compatible `CommandManager`, typed `Argument<T>`, conditions, `DeclareCommandsPacket`, and live `TabCompletePacket`.
  - Compositional `Entity` (`158` generated `EntityType`s, dirty-tracked MC 26.2 `MetadataValue` `0..=42`, `LpVec3`, AABB gravity/collision, chunk-bucketed `EntityTracker`).
  - `EntityAIGroup` (`GoalSelector` & `TargetSelector`), bounded 3D grid A* `Navigator` (`58.5 µs` obstacle routing), 40 `Attribute`s with 3-stage modifiers, 40 `PotionEffect`s, melee combat/knockback, and death/respawn.
  - All `1,458` `Material`s, MC 26.2 `ItemStack` Data Components, `HashedStack` (`ItemStack.Hash`), 46-slot `PlayerInventory`, 25-type shared `Inventory` windows, full `process_inventory_click`, and `ItemEntity` drop/pickup.
  - `Sidebar` scoreboards, `TeamManager`, `BossBar`, `Title` & `ActionBar`, `TabList` header/footer, all `125` `Particle`s, `1,968` `Sound`s, `ToastNotification` & `AdvancementTab`, `RecipeManager`, `ResourcePack` push/pop, MC 26.2 modal `Dialog`s, and `ServerSnapshot`.

---

## Crate Architecture (17-Crate Acyclic Workspace)

```text
minestom (umbrella facade & prelude)
 └─► minestom-server (MinecraftServer, 20 TPS tick loop, AuthMode, Sidebar, BossBar, Snapshot)
      ├─► minestom-network (Tokio PacketStream, RSA/AES-128-CFB8, Velocity HMAC, BungeeGuard, RateLimiter)
      ├─► minestom-world (InstanceContainer, SharedInstance, Chunk/Palette, AnvilLoader, BFS Lighting, Batch)
      ├─► minestom-entity (Entity, EntityTracker, BoundingBox, Attributes, Potions, Damage, A* Navigator, AI)
      ├─► minestom-command (CommandManager, Brigadier tree, typed Arguments, TabComplete)
      ├─► minestom-scheduler (20 TPS SchedulerManager, TickStart/TickEnd, TaskHandle)
      └─► minestom-protocol (MC 26.2 / Proto 776 codecs, Zlib FrameCodec, all Handshake..Play packets)
           ├─► minestom-item (Material, ItemStack Data Components, HashedStack, InventoryClickProcessor)
           ├─► minestom-block (Mixed-radix Block states, BlockHandler, BlockPlacementRule, BlockManager)
           ├─► minestom-registry (27 Dynamic Registries, 32 Tag Registries, DimensionType)
           ├─► minestom-event (EventNode, EventListener, ListenerHandle, CancellableEvent)
           ├─► minestom-text (Component, Style, JSON / MC 26.2 NBT / Legacy / MiniMessage)
           ├─► minestom-nbt (Named & nameless network NBT, GZip/Zlib, SNBT, ReadLimiter)
           ├─► minestom-data (Offline xtask-generated MC 26.2 tables)
           ├─► minestom-core (Pos, Vec3, BlockVec, ChunkPos, Key, Tag<T>, GameMode)
           └─► minestom-macros (#[derive(PacketMeta)])
```

---

## Quickstart

```rust
use minestom::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = MinecraftServer::builder()
        .motd(Component::text("Hello from minestom-rs 26.2!").color(NamedTextColor::Gold))
        .build()?;

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
        unit.modifier().fill_height(40, 41, Block::GRASS_BLOCK);
    });

    let inst_cfg = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&inst_cfg);
            ev.player().set_respawn_point(Pos::new(0.5, 42.0, 0.5));
            ev.player().set_game_mode(GameMode::Creative);
        });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
```

---

## Runnable Examples

| Example | Milestone | Command |
|---|---|---|
| `status_server` | M1 | `cargo run -p minestom --example status_server` |
| `flat_world` | M2 | `cargo run -p minestom --example flat_world` |
| `commands_and_events` | M3 | `cargo run -p minestom --example commands_and_events` |
| `multiplayer_entities` | M4 | `cargo run -p minestom --example multiplayer_entities` |
| `blocks_and_batches` | M5 | `cargo run -p minestom --example blocks_and_batches` |
| `inventory_demo` | M6 | `cargo run -p minestom --example inventory_demo` |
| `world_features_demo` | M7 | `cargo run -p minestom --example world_features_demo` |
| `production_network` | M8 | `cargo run -p minestom --example production_network` |
| `entity_ai_and_combat` | M9 | `cargo run -p minestom --example entity_ai_and_combat` |
| `full_showcase` | M10 | `cargo run -p minestom --example full_showcase` |

---

## Microbenchmarks (`divan`, Release Profile)

| Benchmark | Median Latency |
|---|---|
| `cached_chunk_packet_clone_hit` (`Bytes` shallow clone) | **`13.92 ns`** |
| `listener_handle_hot_dispatch` (`ListenerHandle<E>`) | **`47.25 ns`** |
| `event_node_cached_dispatch` (`EventNode` hierarchy) | **`67.50 ns`** |
| `inventory_quick_move_and_pickup_cycle` | **`92.72 ns`** |
| `block_with_property_mixed_radix` (`Block::with_property` ×2 + `get_property`) | **`103.0 ns`** |
| `varint_encode_decode_batch` (8 values) | **`104.7 ns`** |
| `command_manager_parse_and_execute` | **`396.2 ns`** |
| `network_nbt_compound_roundtrip` | **`825.2 ns`** |
| `scheduler_tick_phase_100_tasks` (100 repeating tasks) | **`2.01 µs`** |
| `item_stack_data_components_roundtrip` | **`3.47 µs`** |
| `relative_batch_apply_125_blocks` (5×5×5 batch + `MultiBlockChange`) | **`3.88 µs`** |
| `spatial_entity_tracker_reconcile_1000_entities` | **`5.25 µs`** |
| `aes128_cfb8_stream_roundtrip_1kb` | **`56.71 µs`** |
| `astar_pathfinding_around_obstacle_wall` (3D A* across 2 chunks) | **`58.54 µs`** |
| `flat_chunk_generate_and_serialize_first_time` | **`83.75 µs`** |
| `anvil_chunk_nbt_roundtrip` (`.mca` chunk NBT encode + decode) | **`184.7 µs`** |
| `bfs_chunk_lighting_sky_and_block` (full 24-section 3D BFS light) | **`600.4 µs`** |

---

## Documentation & Quality Gate

- [Parity Matrix (`docs/PARITY.md`)](./docs/PARITY.md)
- [Migrating from Java Minestom (`docs/MIGRATING_FROM_MINESTOM.md`)](./docs/MIGRATING_FROM_MINESTOM.md)
- [Architecture Decision Records (`docs/adr/`)](./docs/adr/)

```bash
# Regenerate static MC 26.2 data from pinned xtask/data/minestom-data-26.2-rv3.tar.gz
cargo xtask codegen

# Run full CI quality gate (codegen --check, rustfmt, clippy pedantic -D warnings,
# cargo nextest, doctests, rustdoc -D warnings, and cargo deny check)
cargo xtask ci

# Run Divan microbenchmarks
cargo bench -p minestom --bench protocol_bench
cargo bench -p minestom --bench chunk_bench
```

## License

Licensed under the [Apache License, Version 2.0](./LICENSE). See [NOTICE](./NOTICE) for upstream Minestom attribution.
