# ADR 0001: Workspace Layout and Acyclic Crate Boundaries

- **Status:** Accepted
- **Date:** 2026-11-18
- **Target Upstream:** Minestom (`net.minestom:data:26.2-rv3`, Minecraft: Java Edition **26.2**, Protocol **776**, Data Version **4903**)

## Context

Minestom in Java is published primarily as a single monolithic artifact (`net.minestom:minestom`) where packages (`instance`, `entity`, `block`, `item`, `event`, `network`, `command`) freely reference one another in cycles. In Rust (Cargo workspace, edition 2024, resolver 3), crate dependencies must form a strict Directed Acyclic Graph (DAG). Naive 1:1 package-to-crate splitting causes circular dependencies (e.g., `BlockPlacementRule` referencing `Player` while `Player` references `Instance` and `Block`).

At the same time, modular crates dramatically speed up incremental compilation, enforce architectural boundaries, allow headless protocol/world/NBT reuse, and keep unsafe/macro surface areas isolated.

## Decision

We adopt the 16-crate workspace + `xtask` layout below, organized into strict layers so all dependency edges point downward:

1. **Layer 0 (Leaf primitives & proc-macros):**
   - `minestom-macros`: Procedural macros (`#[derive(Packet)]`, `#[derive(Event)]`) when justified; zero runtime dependencies.
   - `minestom-nbt`: Complete safe-Rust NBT + SNBT implementation supporting both named-root (Anvil/disk) and nameless-root (MC 1.20.2+ network) compounds.
2. **Layer 1 (Core value types & engines):**
   - `minestom-core`: Coordinates (`Point`, `Vec3`, `Pos`, `BlockVec`, `ChunkPos`), `Key`/`NamespaceId`, handle IDs (`EntityId`, `PlayerId`, `InstanceId`), common enums (`GameMode`, `Difficulty`, `Hand`, `BlockFace`), and typed `Tag<T>` / `TagMap` backed by `minestom-nbt`.
   - `minestom-text`: Adventure-style `Component`, `Style`, `TextColor`, `ClickEvent`, `HoverEvent`, plus JSON, NBT, legacy (`§`/`&`), plain, and MiniMessage-subset serializers.
   - `minestom-data`: **Generated**, committed static constants and vanilla registry tables produced by `cargo xtask codegen` from pinned `net.minestom:data:26.2-rv3`.
   - `minestom-event`: Generic `Event`, `CancellableEvent`, `EventNode<E>` tree, `ListenerHandle<E>` cache, and `GlobalEventHandler`. Domain crates define concrete event structs implementing `minestom_event::Event`.
   - `minestom-scheduler`: Tick-deterministic `Scheduler`, `SchedulerManager`, `Task`, `TaskSchedule`, and `ExecutionType`.
3. **Layer 2 (Registries, Blocks, Items):**
   - `minestom-registry`: Static & dynamic `Registry<T>` / `DynamicRegistry<T>` (`Registries`, `DimensionType`, `Biome`, `ChatType`, `DamageType`, `Dialog`, `Timeline`, `WorldClock`, variants, and registry tags).
   - `minestom-block`: `Block` value type (state ID, property map, block-entity NBT/tags), `BlockHandler`, `BlockPlacementRule`, and abstract `BlockGetter`/`BlockSetter` traits implemented by `minestom-world` to invert cyclic dependencies.
   - `minestom-item`: `Material`, `ItemStack` with 1.20.5+/26.2 data components, `Inventory`, window types, and click state machines.
4. **Layer 3 (Protocol & Network):**
   - `minestom-protocol`: Wire codecs (`VarInt`, `VarLong`, packed `BlockPos`, `BitSet`, `Encode`/`Decode`), packet framing, and typed clientbound/serverbound packets across `Handshake`, `Status`, `Login`, `Configuration`, and `Play` for protocol 776.
   - `minestom-network`: Tokio TCP listener, per-connection read/write tasks, zlib compression, AES/CFB8 stream encryption, Mojang session auth, Velocity/BungeeCord forwarding, rate limiting, and bounded channel bridges.
5. **Layer 4 (World, Entities, Commands):**
   - `minestom-world`: Paletted `Section` and `Chunk` with cached packet buffers, parallel `Generator`/`GenerationUnit`/`UnitModifier`, `ChunkLoader` + `AnvilLoader`, opt-in `LightingChunk`, `InstanceContainer`, `SharedInstance`, and `InstanceManager`.
   - `minestom-entity`: Compositional `Entity` (`EntityCore` + `LivingData` + `EntityKind`/`Player`), metadata, AABB physics & collision, spatial `EntityTracker`/`EntityView`, `EntityBehavior`/`Goal`/`Target` AI traits, attributes, effects, and damage.
   - `minestom-command`: Brigadier-compatible `Command` tree, typed `Argument` parsers, suggestions, conditions, `CommandManager`, and `DeclareCommands` packet graph serialization.
6. **Layer 5 (Server Runtime & Facade):**
   - `minestom-server`: `MinecraftServer`, `Server` handle, `ServerConfig`, `ThreadDispatcher`, fixed-rate 20 TPS tick loop, and full protocol state machine wiring.
   - `minestom`: Public facade crate re-exporting unified `Error`, `prelude::*`, and subsystem modules.

### Breaking Cycles Cleanly

- **Traits in lower crates:** `minestom-block` defines `BlockGetter` rather than importing `Instance`.
- **Opaque generational IDs in `minestom-core`:** `EntityId`, `PlayerId`, and `InstanceId` live in `minestom-core`, allowing lower-level crates (`minestom-block`, `minestom-item`, `minestom-event`) to carry strongly typed references without depending on `minestom-entity` or `minestom-world`.

## Consequences

- Zero circular dependencies across all 16 crates.
- Shared `[workspace.dependencies]` and strict `[workspace.lints]` (`#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`, `clippy::pedantic`) apply uniformly everywhere.
- `cargo xtask codegen` writes deterministic Rust modules to `crates/minestom-data` from committed, checksum-verified `26.2-rv3` JSON resources so `cargo build` never touches the network.
