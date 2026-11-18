# ADR 0002: Concurrency Model and State Ownership

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

A Minecraft server has two fundamentally different concurrency workloads:
1. **Asynchronous I/O:** Hundreds or thousands of concurrent TCP sockets with variable latency, packet framing, compression, encryption, and HTTP session authentication.
2. **Deterministic Simulation:** A fixed-rate 20 TPS (50 ms budget) game tick updating chunks, entities, physics, AI, scheduled tasks, and viewer tracking.

In Java Minestom, global access goes through `MinecraftServer.get*()` static singletons, tick work is partitioned across threads by `ThreadDispatcher`, and cross-thread entity safety uses `Acquirable<T>` locks. Wrapping every Rust `Entity`, `Chunk`, and `Instance` in `Arc<Mutex<_>>` or `Arc<RwLock<_>>` creates lock contention, cache-unfriendly pointer chasing, and runtime lock-ordering deadlocks whenever two entities interact.

## Evaluated Options & Spike Results

We evaluated three state-ownership designs and ran a release-mode benchmark spike (`rustc 1.100.0 -O`, x86_64 Linux) simulating 10,000 entities per tick performing velocity integration plus 1,000 cross-entity targeted health updates:

| Option | Description | 10k Entities (µs / tick) | Deadlock Risk | API Ergonomics |
|---|---|---:|---|---|
| **(a) Instance-owned generational arenas (`slotmap`) + command queue** | Each `Instance` owns `SlotMap<EntityId, Entity>` and chunk storage; tick passes `&mut Instance` / `TickContext`; external/async callers obtain lightweight handles (`EntityRef`, `PlayerRef`, `InstanceHandle`) that enqueue typed commands applied at tick boundaries (with `ArcSwap`/atomic snapshots for lock-free reads where appropriate). | **36.22 µs** (4.24x faster) | **None** (borrow-checked single writer per instance partition) | Idiomatic Rust, familiar Minestom handle feel |
| **(b) Full internal ECS (`bevy_ecs`)** | Archetype SoA tables internally with handle wrapper API on top. | Fast linear iteration, slower random structural changes | None | Heavy dependency footprint, leaky abstraction for custom `EntityBehavior` state, large compile-time cost |
| **(c) `Acquirable<T>` via `Arc<parking_lot::RwLock<Entity>>`** | Direct port of Java Minestom's per-entity lock acquisition. | **153.56 µs** | **High** (lock ordering between entity A & B or re-entrant self-locking panics/deadlocks) | Verbose `.write()` guards everywhere |

## Decision

1. **No Global Mutable State:** `MinecraftServer::builder().build()?` produces an explicit, cheaply cloneable `Server` handle (`Arc<ServerShared>`). Users pass or capture `server` (and `InstanceHandle` / `PlayerRef`) instead of calling static singletons.
2. **Tokio for Networking Only:** Each client TCP connection runs on Tokio tasks (`minestom-network`) with `tokio_util::codec` and `bytes::BytesMut`. Decoded inbound packets enter the server through bounded `flume` channels; outbound packets leave through per-connection bounded `flume` channels (`try_send` / backpressure policy disconnecting hopelessly stalled clients rather than stalling tick threads). Blocking disk I/O (Anvil region reads/writes) runs via `tokio::task::spawn_blocking` or dedicated I/O workers—never on Tokio async worker threads or tick threads.
3. **Dedicated Tick Threads & `ThreadDispatcher`:** The 20 TPS tick loop runs on dedicated OS threads (`Ms-TickScheduler` + `rayon` / partitioned worker pool), never inside `tokio::spawn`. Each `Instance` forms an independent tick partition ticked in parallel across worker threads, matching Minestom's `ThreadDispatcher` guarantees without cross-instance locking.
4. **Option (a) — Generational Arena Ownership (`slotmap`) + Command Queue:**
   - Each `Instance` owns its chunks (`FxHashMap<ChunkPos, Chunk>`) and entities (`SlotMap<EntityId, Entity>`).
   - Within synchronous tick callbacks, event listeners, and entity behaviors, `TickContext` grants direct, zero-lock mutable access to instance state.
   - Outside the tick (e.g., async tasks, external handles like `InstanceHandle` or `PlayerRef`), mutations enqueue lightweight commands into a lock-free/bounded per-instance inbox drained deterministically at `ExecutionType::TickStart`, while read-mostly properties (e.g., player UUID, username, last known `Pos`, online status) are cached in a lightweight shared snapshot for instant non-blocking queries.

## Consequences

- 4.2x faster entity ticking than lock-per-entity, zero lock-ordering deadlocks, and `#![forbid(unsafe_code)]` compliance.
- A slow or malicious network peer cannot stall the 20 TPS tick loop.
- Users enjoy Minestom-familiar handles (`instance.set_generator(...)`, `player.send_message(...)`, `player.teleport(...)`) from both sync and async contexts.
