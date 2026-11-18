# ADR 0007: Error Handling, Structured Logging, and Configuration

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

As a reusable server library (not a standalone application), `minestom-rs` must never crash on malformed client traffic, hijack stdout with `println!`, or force a single global logger implementation onto downstream embedders.

## Decision

1. **Typed Errors (`thiserror`), Zero Panics on Untrusted Input:**
   - Every library crate defines its own domain error enum (`NbtError`, `ProtocolError`, `NetworkError`, `WorldError`, `CommandError`, `ServerError`) derived with `thiserror::Error` (`#![forbid(unsafe_code)]`, never `anyhow`).
   - The facade crate `minestom` exposes `minestom::Error` wrapping all subsystem errors via `#[from]`.
   - `clippy::unwrap_used` and `clippy::expect_used` are audited and avoided in packet decoding, NBT parsing, compression, and network handlers. Any invalid `VarInt`, truncated frame, oversized string, invalid state ID, or bad compressed length returns `Err(...)`, logs at `debug!` / `warn!`, and cleanly disconnects **only that client**.
2. **Structured Instrumentation (`tracing`):**
   - Library crates emit structured events and spans via `tracing` (`connection{peer_addr, state, username}`, `tick{tick_num, duration_us}`, `instance{id}`).
   - `tracing-subscriber` is strictly a dev/example dependency—library crates never initialize global subscribers.
3. **Typed `ServerConfig` + Builder + Optional TOML:**
   - Replace Minestom's JVM system properties (`-Dminestom.tps=20`, `-Dminestom.chunk-view-distance=8`, etc.) with an explicit, strongly typed `ServerConfig` struct and `MinecraftServer::builder()`:
     - `ticks_per_second: u32` (default `20`)
     - `chunk_view_distance: u8` (default `8`)
     - `entity_view_distance: u8` (default `5`)
     - `compression_threshold: Option<usize>` (default `Some(256)`)
     - `brand_name: String` (default `"minestom-rs"`)
     - `difficulty: Difficulty` (default `Difficulty::Normal`)
     - `max_players: u32` (default `100`)
     - `motd: Component`
     - `auth_mode: AuthMode` (`Offline`, `Online`, `Velocity { secret }`, `BungeeCord`)
     - `rate_limit_packets_per_sec: u32` (default `500`)
   - `ServerConfig` implements `serde::Serialize` and `serde::Deserialize` so applications can load configuration from TOML (`ServerConfig::from_toml_str(...)`) or construct it fluently in code.

## Consequences

- Downstream applications retain full control over logging sinks, telemetry, and multi-server hosting within a single process (including parallel integration tests on ephemeral ports `127.0.0.1:0`).
- Malformed or fuzzed network bytes are guaranteed by `proptest` and decoder bounds checks never to panic the server.
