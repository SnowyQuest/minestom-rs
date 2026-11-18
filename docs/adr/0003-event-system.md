# ADR 0003: Typed Event Tree (`EventNode`) and Cached Dispatch

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

Minestom's event system is one of its signature features: rather than flat annotation-based listeners (Bukkit/Spigot), Minestom organizes listeners into a composable `EventNode<E>` tree rooted at `GlobalEventHandler`. Nodes support:
- Event-trait scoping (`EventFilter`: `ALL`, `PLAYER`, `ENTITY`, `INSTANCE`, `ITEM`, `INVENTORY`, etc.)
- Value filtering and custom predicates (`condition`)
- Priority ordering (`priority: i32`)
- Expiring listeners (`expire_count`, `expire_when`)
- Pre-compiled `ListenerHandle<E>` chains so firing an event on hot paths (movement, block interactions, ticks) performs zero heap allocations when no tree topology change occurred.

Additionally, Minestom has a small set of **asynchronous pre-play events** (`AsyncPlayerPreLoginEvent`, `AsyncPlayerConfigurationEvent`) that run off the tick thread during login/configuration so servers can query databases before assigning a spawning instance.

## Decision

1. **Core Traits:**
   - `pub trait Event: Any + Send + Sync + 'static`
   - `pub trait CancellableEvent: Event { fn is_cancelled(&self) -> bool; fn set_cancelled(&mut self, cancelled: bool); }`
   - Domain filter traits (`PlayerEvent`, `EntityEvent`, `InstanceEvent`, `BlockEvent`, `ItemEvent`, `InventoryEvent`) provide typed accessors (e.g., `fn player_id(&self) -> PlayerId`) usable by `EventFilter`.
2. **Composable `EventNode` Tree:**
   - `EventNode` stores listeners keyed by `TypeId`, child `EventNode`s sorted by `priority`, optional filter predicates, and an atomic structural revision counter (`AtomicU64`).
   - Builder/fluent API for listeners (`EventListener::builder::<MyEvent>(handler).priority(p).expire_count(n).expire_when(pred).filter(cond)`), plus ergonomic `node.add_listener(|event: &mut MyEvent| { ... })`.
3. **Zero-Allocation Hot-Path Dispatch (`ListenerHandle`):**
   - When `call(&mut event)` or `ListenerHandle::<MyEvent>::call(&mut event)` runs, it checks whether the cached flattened listener list matches the root node's current revision counter.
   - If unchanged (the common case on 99.99% of ticks), it iterates a pre-sorted `Arc<[CompiledListener]>` slice with **zero heap allocation**.
   - If a listener expired (`expire_count` reached 0 or `expire_when` returned `true`), it removes the listener and bumps the revision counter cleanly.
4. **Synchronous vs. Asynchronous Events:**
   - Gameplay events run synchronously on the owning tick thread (`FnMut(&mut E)` / `Fn(&mut E)`).
   - Login/configuration events (`AsyncPlayerPreLoginEvent`, `AsyncPlayerConfigurationEvent`) support both synchronous closures (like the target API `add_listener(move |event: &mut AsyncPlayerConfigurationEvent| { ... })`) AND async listeners (`add_async_listener(|event| Box::pin(async move { ... }))`) awaited on the connection's Tokio task before transitioning states.

## Consequences

- Preserves Minestom's exact `EventNode` composition model (`all`, `type_`, `value`, `priority`, `add_child`, `remove_child`, `ListenerHandle`).
- Both concise sync configuration listeners and database-backed async configuration listeners work seamlessly.
- Event dispatch overhead is a handful of nanoseconds per listener with zero per-call allocation.
