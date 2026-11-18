# ADR 0004: Compositional Entity Architecture, AI Traits, and Spatial Tracking

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

In Java Minestom, entities use deep OOP inheritance:
`Entity` → `LivingEntity` → `EntityCreature` (with `AIGroup`s of `GoalSelector` + `TargetSelector` and `Navigator` pathfinding) or `Player` / `ItemEntity` / `BlockDisplayMeta` etc.
Rust intentionally lacks struct inheritance. Meanwhile, users must still be able to:
1. Spawn any built-in `EntityType` with strongly typed metadata (`EntityMetadata`).
2. Attach custom behavior and AI goals/targets just as cleanly as subclassing `EntityCreature` in Java.
3. Track entity visibility across thousands of players and entities efficiently (`EntityTracker` + `EntityView`).

## Decision

1. **Composition over Inheritance:**
   Every `Entity` stored in an `Instance` arena is composed of orthogonal components:
   - `EntityCore`: `EntityId` (generational handle), network `i32` protocol ID, `Uuid`, `EntityType`, `Pos`, velocity `Vec3`, `BoundingBox`, on-ground/no-gravity flags, `EntityMetadata` dirty-tracked map, vehicle/passengers (`SmallVec<[EntityId; 2]>`), `TagMap`, and viewer set.
   - `Option<LivingState>`: health, max health, absorption, `AttributeMap`, active potion effects, equipment slots (`MainHand`, `OffHand`, `Helmet`, `Chestplate`, `Leggings`, `Boots`, `Body`), invulnerability ticks, and death state.
   - `EntityKind`: closed enum distinguishing core runtime categories (`Standard`, `Creature(CreatureBrain)`, `Player(PlayerData)`, `ItemDrop(ItemStack)`, `ExperienceOrb(u16)`), plus `Option<Box<dyn EntityBehavior>>` for user-defined custom entities.
2. **Custom Entities & AI via Traits:**
   - `EntityBehavior`: `on_spawn`, `on_tick(&mut EntityTickContext)`, `on_interact`, `on_damage`, `on_remove`.
   - `CreatureBrain`: holds `Navigator` (A* grid pathfinder over `BlockGetter`) and `Vec<AiGroup>`, where each `AiGroup` pairs `Vec<Box<dyn Goal>>` (`should_start`, `start`, `tick`, `should_end`, `end`) with `Vec<Box<dyn Target>>` (`find_target`). Built-in goals/targets (`RandomStrollGoal`, `MeleeAttackGoal`, `FollowTargetGoal`, `LookAtPlayerGoal`, `ClosestEntityTarget`, `LastEntityDamagerTarget`) implement these public traits.
3. **Chunk-Bucketed Spatial `EntityTracker`:**
   - `EntityTracker` indexes entity positions by `ChunkPos` (`FxHashMap<ChunkPos, SmallVec<[EntityId; 8]>>`), partitioned by target category (`Entities`, `Players`).
   - When an entity crosses a chunk boundary or spawns/despawns, `EntityView` computes set differences against `entity_view_distance` (in chunks) and automatically emits spawn/destroy/metadata/equipment packets to entering/leaving player viewers, while supporting manual viewer rules (`auto_viewable`, `add_viewer`, `remove_viewer`).

## Consequences

- Eliminates fragile OOP downcasting (`instanceof LivingEntity`) in favor of explicit `entity.living()`, `entity.as_player()`, and trait-based `EntityBehavior`.
- Dirty-tracked `EntityMetadata` only serializes changed metadata entries per tick, minimizing network bandwidth.
- Chunk-bucketed spatial queries run in $O(k)$ over nearby chunks rather than $O(N^2)$ over all entities in an instance.
