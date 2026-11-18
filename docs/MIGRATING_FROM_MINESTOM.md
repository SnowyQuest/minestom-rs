# Migrating from Java Minestom to `minestom-rs`

`minestom-rs` preserves Minestom's core concepts, mental model, and type names (`MinecraftServer`, `InstanceManager`, `InstanceContainer`, `SharedInstance`, `GenerationUnit`, `EventNode`, `GlobalEventHandler`, `Player`, `Entity`, `Block`, `ItemStack`, `Command`, `Scheduler`, `Tag`), while adapting Java idioms into safe, idiomatic Rust.

Every intentional API difference from Java Minestom is recorded below.

---

## 1. Server Initialization & Static Singletons → Explicit `Server` Handle

**Why:** Hidden global mutable state (`MinecraftServer.getInstanceManager()`) prevents running multiple isolated servers in parallel tests, hides data dependencies, and requires global locks in Rust.

**Java (Minestom):**
```java
MinecraftServer server = MinecraftServer.init();
InstanceManager instanceManager = MinecraftServer.getInstanceManager();
GlobalEventHandler events = MinecraftServer.getGlobalEventHandler();
server.start("0.0.0.0", 25565);
```

**Rust (`minestom-rs`):**
```rust
let server = MinecraftServer::builder().build()?;
let instance_manager = server.instance_manager();
let events = server.global_event_handler();
server.start("0.0.0.0:25565").await?;
```

---

## 2. Getters / Setters, Nullability, and Checked Errors

**Why:** Rust naming conventions (RFC 430) omit the `get_` prefix on accessors, represent optional values explicitly via `Option<T>` instead of `@Nullable`, and return typed `Result<T, E>` instead of throwing unchecked exceptions.

| Java (Minestom) | Rust (`minestom-rs`) | Reason |
|---|---|---|
| `pos.x()`, `entity.getPosition()`, `entity.setPosition(pos)` | `pos.x()`, `entity.position()`, `entity.set_position(pos)` | Rust API guidelines (no `get_` prefix) |
| `@Nullable Instance getInstance()` | `fn instance(&self) -> Option<InstanceId>` | Explicit null safety via `Option<T>` |
| `throws InvalidPacketException` / `IllegalArgumentException` | `Result<T, minestom::Error>` | Typed error handling via `thiserror` |
| System properties (`-Dminestom.chunk-view-distance=8`) | `MinecraftServer::builder().chunk_view_distance(8)` / `ServerConfig` | Strongly typed config & TOML support (ADR 0007) |

---

## 3. Immutable Value Types (`Pos`, `Vec3`, `BlockVec`, `Block`, `ItemStack`)

**Why:** In Java, immutable records (`Pos`, `Vec`, `Block`, `ItemStack`) allocate on the heap unless scalar-replaced by JIT. In Rust, coordinate types (`Pos`, `Vec3`, `BlockVec`, `ChunkPos`) and compact IDs (`Material`, `EntityType`) are stack-allocated `Copy` value types, and `Block` / `ItemStack` use cheap value/copy-on-write semantics with fluent `with_*` builders.

**Java (Minestom):**
```java
Pos spawn = new Pos(0, 42, 0).withYaw(90f);
Block grass = Block.GRASS_BLOCK.withProperty("snowy", "true");
ItemStack sword = ItemStack.builder(Material.DIAMOND_SWORD).amount(1).build();
```

**Rust (`minestom-rs`):**
```rust
let spawn = Pos::new(0.0, 42.0, 0.0).with_yaw(90.0);
let grass = Block::GRASS_BLOCK.with_property("snowy", "true")?;
let sword = ItemStack::single(Material::DIAMOND_SWORD)
    .with_enchantment(Enchantment::SHARPNESS, 5);
```

---

## 4. Entity Hierarchy → Composition + Traits

**Why:** Rust favors composition and traits over OOP class hierarchies (`Entity → LivingEntity → EntityCreature → CustomZombie`). See ADR 0004.

**Java (Minestom):**
```java
public class CustomZombie extends EntityCreature {
    public CustomZombie() {
        super(EntityType.ZOMBIE);
        addAIGroup(List.of(new MeleeAttackGoal(this, 1.2, 20, TimeUnit.SERVER_TICK)),
                   List.of(new ClosestEntityTarget(this, 16, Player.class::isInstance)));
    }
}
```

**Rust (`minestom-rs`):**
```rust
let ai = EntityAIGroup::new()
    .with_target_selector(ClosestEntityTarget::players(16.0))
    .with_goal_selector(MeleeAttackGoal::new(0.24, 2.0, 20));
let zombie = Entity::creature(EntityType::ZOMBIE).with_behavior(ai);
server.spawn_entity(&instance, zombie, Pos::new(8.5, 40.0, 0.5));
```
Custom per-tick or spawn logic implements the `EntityBehavior` trait instead of overriding `Entity.tick(long)`.

---

## 5. `Acquirable<T>` → `PlayerRef`, `EntityRef`, and Unified FIFO Tick Channel

**Why:** Per-entity locks risk deadlocks and add synchronization overhead on every tick. Inside the 20 TPS tick thread, `SlotMap<PlayerId, ActivePlayer>` and world entities are owned and mutated without locks; from event listeners, command executors, or async tasks, `PlayerRef` (`ev.player`) and `EntityRef` (`server.spawn_entity(...)`) enqueue commands onto a single bounded FIFO channel and read consistent point-in-time state via `ArcSwap<ServerSnapshot>` (ADR 0002).

```rust
server.global_event_handler().add_listener(|ev: &mut PlayerSpawnEvent| {
    if ev.first_spawn {
        ev.player.send_message(Component::text("Welcome!"));
        ev.player.give_item(ItemStack::single(Material::DIAMOND_SWORD));
    }
});
```

---

## 6. Event Listeners & `EventNode`

**Why:** Rust closures infer or declare the event type directly in the parameter `&mut E` instead of passing `MyEvent.class` as a runtime token.

**Java (Minestom):**
```java
MinecraftServer.getGlobalEventHandler().addListener(AsyncPlayerConfigurationEvent.class, event -> {
    event.setSpawningInstance(instance);
    event.getPlayer().setRespawnPoint(new Pos(0, 42, 0));
});
```

**Rust (`minestom-rs`):**
```rust
server
    .global_event_handler()
    .add_listener(move |event: &mut AsyncPlayerConfigurationEvent| {
        event.set_spawning_instance(&instance);
        event.player().set_respawn_point(Pos::new(0.0, 42.0, 0.0));
    });
```

---

## 7. Typed `Tag<T>`

**Why:** Instead of separate static factories (`Tag.Integer("score")`, `Tag.String("team")`) backed by untyped `Object`, Rust uses a zero-cost phantom type `Tag<T: TagSerializable>`:

**Java (Minestom):**
```java
Tag<Integer> SCORE = Tag.Integer("score").defaultValue(0);
int score = entity.getTag(SCORE);
```

**Rust (`minestom-rs`):**
```rust
const SCORE: Tag<i32> = Tag::new("score");
let score = entity.get_tag(&SCORE).unwrap_or(0);
```
