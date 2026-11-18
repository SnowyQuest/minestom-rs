# `minestom-rs`

A production-quality **Minecraft: Java Edition 26.2** (protocol **776**, data version **4903**) server library written from scratch in safe, idiomatic Rust with zero vanilla code—inspired by [Minestom](https://github.com/Minestom/Minestom).

## Highlights

- **API parity, idiomatic Rust:** Familiar `MinecraftServer`, `InstanceContainer`, `SharedInstance`, `GenerationUnit`, `EventNode`, `GlobalEventHandler`, `Player`, `Entity`, `Block`, `ItemStack`, `Command`, `Scheduler`, and `Tag<T>` concepts with zero hidden global mutable state.
- **Safety & reliability:** `#![forbid(unsafe_code)]` across all crates, typed errors (`thiserror`), structured `tracing` instrumentation, and property-tested protocol decoders.
- **High-throughput architecture:** Tokio TCP networking bridged via bounded channels to a deterministic, multi-threaded 20 TPS tick loop with generational arenas (`slotmap`) and cached paletted chunk packets.
- **Offline, deterministic builds:** Registry and constant tables generated from `MinestomDataGenerator` (`net.minestom:data:26.2-rv3`) via `cargo xtask codegen` and committed to version control.

## Workspace Documentation

- [Parity Matrix (`docs/PARITY.md`)](./docs/PARITY.md)
- [Migrating from Java Minestom (`docs/MIGRATING_FROM_MINESTOM.md`)](./docs/MIGRATING_FROM_MINESTOM.md)
- [Architecture Decision Records (`docs/adr/`)](./docs/adr/)

## Development Commands

```bash
# Regenerate static data from pinned 26.2-rv3 resources
cargo xtask codegen

# Run full quality gate (codegen check, rustfmt, clippy pedantic, nextest, rustdoc, cargo-deny)
cargo xtask ci
```

## License

Licensed under the [Apache License, Version 2.0](./LICENSE). See [NOTICE](./NOTICE) for upstream Minestom attribution.
