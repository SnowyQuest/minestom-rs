# Minestom → `minestom-rs` Parity Matrix

- **Upstream Target:** Minestom (`master` @ `b58fe548e5`)
- **Minecraft Version:** `26.2`
- **Protocol Version:** `776`
- **Data Version:** `4903`
- **Resource Pack Format:** `88.0`
- **Data Pack Format:** `107.1`
- **Data Artifact:** `net.minestom:data:26.2-rv3`

Status legend:
- `done` — Implemented, tested, and documented with parity.
- `partial` — Core types/subset working; remaining items tracked in notes.
- `changed` — Intentionally redesigned for idiomatic Rust (see [MIGRATING_FROM_MINESTOM.md](./MIGRATING_FROM_MINESTOM.md) and `docs/adr/`).
- `todo` — Planned for upcoming milestones (M1–M10).
- `won't port` — Java/JVM-specific internal detail not applicable in Rust.

---

## Package Mapping

| Upstream Java Package (`net.minestom.server.*`) | Key Upstream Classes | Rust Crate & Module | Status | Notes & Milestone |
|---|---|---|---|---|
| `net.minestom.server` | `MinecraftServer`, `MinecraftConstants`, `ServerProcess`, `Auth`, `FeatureFlag`, `ServerFlag` | `minestom_server`, `minestom_data::constants`, `minestom` | `partial` | `MinecraftServer` builder/handle, `InstanceManager`, `GlobalEventHandler`, 20 TPS tick loop, Offline Login → Config → Play done (M2); Online/Proxy Auth in M8 |
| `advancements` | `Advancement`, `AdvancementManager`, `AdvancementRoot`, `AdvancementTab`, `FrameType` | `minestom_server::advancement` | `todo` | M10 |
| `adventure`, `adventure.audience`, `adventure.serializer.nbt`, `adventure.provider` | `Component`, `Audiences`, `NbtComponentSerializer`, `ClickCallbackManager` | `minestom_text` | `partial` | `Component`, `Style`, `TextColor`, `ClickEvent`, `HoverEvent`, JSON, MC 26.2 NBT, legacy (`§`/`&`), plain, MiniMessage subset done (M1); audiences/callbacks in M3/M10 |
| `adventure.bossbar` | `BossBar`, `BossBarManager`, `BossBarHolder` | `minestom_server::bossbar` | `todo` | M10 |
| `codec` | `Codec`, `StructCodec`, `Transcoder`, `Result` | `minestom_nbt`, `minestom_protocol::codec` | `done` | Idiomatic Rust `Encode`/`Decode` traits + JSON↔NBT & `serde` transcoders (M1) |
| `collision` | `BoundingBox`, `Aerodynamics`, `CollisionUtils`, `BlockCollision`, `EntityCollision`, `Shape` | `minestom_entity::collision` | `done` | `BoundingBox` (`AABB`), overlap queries, and 20 TPS gravity/drag + solid-block collision resolution done (M4) |
| `color` | `Color`, `AlphaColor`, `TeamColor` | `minestom_core::types`, `minestom_text` | `partial` | `Color`, `AlphaColor`, `NamedTextColor` done (M1); `DyeColor` / `TeamColor` in M10 |
| `command` | `CommandManager`, `CommandSender`, `ConsoleSender`, `ServerSender` | `minestom_command` | `done` | Thread-safe `CommandManager`, `CommandSender` (`console`/`player`), reply & `SenderEffect` pipeline done (M3) |
| `command.builder`, `parser`, `suggestion`, `condition`, `exception` | `Command`, `CommandSyntax`, `CommandContext`, `CommandExecutor`, `Suggestion`, `CommandCondition` | `minestom_command` | `done` | Brigadier-compatible `Command`, `CommandSyntax`, `CommandCondition`, `SuggestionCallback`, `DeclareCommandsPacket` & `TabCompletePacket` done (M3) |
| `command.builder.arguments.*` | `Argument`, `ArgumentLiteral`, `ArgumentNumber`, `ArgumentString`, `ArgumentEntity`, `ArgumentBlockState`, `ArgumentRelativeVec3`, ... | `minestom_command` | `done` | Typed `Argument<T>` (`literal`, `word`, `string`, `greedy_string`, `boolean`, `integer`, `long`, `float`, `double`, `choices`, `game_mode`, relative `block_pos` & `vec3`) done (M3) |
| `component` | `DataComponent`, `DataComponentMap`, `DataComponents` | `minestom_item::DataComponentValue` | `done` | Typed MC 26.2 Data Components (`CustomData`, `MaxStackSize`, `MaxDamage`, `Damage`, `Unbreakable`, `CustomName`, `ItemName`, `ItemModel`, `Lore`, `Rarity`, `Enchantments`, `RepairCost`, `CreativeSlotLock`, `EnchantmentGlintOverride`, `RawOpaque`) + trusted/untrusted & `HashedStack` codecs done (M6) |
| `condition` | `DataPredicate` | `minestom_item::predicate` | `todo` | M6/M10 |
| `coordinate` | `Point`, `Vec`, `Pos`, `BlockVec`, `ChunkRange`, `CoordConversion`, `Area` | `minestom_core::coord` | `done` | `Point`, `Vec3`, `Pos`, `BlockVec` (packed 64-bit), `ChunkPos` (spiral range) done & proptested (M1) |
| `crypto` | `ChatSession`, `MessageSignature`, `PlayerPublicKey`, `SignedMessageBody`, `FilterMask` | `minestom_network::crypto`, `minestom_protocol::chat` | `todo` | Chat signing & session crypto (M3/M8) |
| `dialog` | `Dialog`, `DialogBody`, `DialogInput`, `DialogAction` | `minestom_registry::dialog` | `todo` | MC 26.2 dialog dynamic registry & packets (M2/M10) |
| `entity` | `Entity`, `LivingEntity`, `Player`, `EntityCreature`, `EntityProjectile`, `EntityType`, `GameMode`, `PlayerSkin`, `EquipmentSlot` | `minestom_entity`, `minestom_data::entity_types`, `minestom_core` | `done` | Compositional `Entity` (`EntityCore` + `LivingState` + `EntityRole` + `EntityBehavior`), `EntityType` (158 generated types w/ hitboxes), spatial `EntityTracker` & multiplayer sync done (M4) |
| `entity.ai`, `entity.ai.goal`, `entity.ai.target` | `EntityAIGroup`, `GoalSelector`, `TargetSelector`, `MeleeAttackGoal`, `RandomStrollGoal`, `FollowTargetGoal`, `ClosestEntityTarget`, ... | `minestom_entity::ai` | `todo` | `Goal` and `Target` traits (M4/M9) |
| `entity.attribute` | `Attribute`, `AttributeInstance`, `AttributeModifier`, `AttributeOperation` | `minestom_entity::attribute`, `minestom_data::attribute` | `todo` | M9 |
| `entity.damage` | `Damage`, `DamageType`, `EntityDamage`, `EntityProjectileDamage`, `PositionalDamage` | `minestom_entity::damage`, `minestom_registry::damage_type` | `todo` | M2 (registry) / M9 (gameplay) |
| `entity.metadata.**` | `EntityMeta`, `LivingEntityMeta`, `PlayerMeta`, animal/monster/display/projectile/minecart/villager variants & metadata | `minestom_entity::metadata`, `minestom_protocol::entity` | `done` | Dirty-tracked `EntityMetadata` + MC 26.2 `MetadataValue` (`0..=42` incl. `Avatar` `MainHand`/`SkinParts`) + all variant dynamic registries done (M4) |
| `entity.pathfinding.**` | `Navigator`, `PNode`, `PPath`, `PathGenerator`, ground/water/flying followers & generators | `minestom_entity::pathfinding` | `todo` | A* pathfinder (M9) |
| `entity.vehicle` | `PlayerInputs` | `minestom_protocol::entity` | `done` | `ClientInputPacket` (`0x2B`) movement/sneak/sprint input flags done (M4) |
| `event`, `event.trait` | `Event`, `EventNode`, `GlobalEventHandler`, `EventFilter`, `EventListener`, `ListenerHandle`, `CancellableEvent`, `AsyncEvent` | `minestom_event` | `done` | `EventNode` tree, domain filter traits (`PlayerEvent`, `EntityEvent`, `InstanceEvent`, `BlockEvent`), `EventListener` (`expire_count`, `expire_when`, `filter`), `ListenerHandle<E>` done (M3) |
| `event.server` | `ServerListPingEvent`, `ClientPingServerEvent`, `ServerTickMonitorEvent` | `minestom_server::event` | `done` | Cancellable `ServerListPingEvent` & `ClientPingServerEvent` done (M3) |
| `event.player` | `AsyncPlayerPreLoginEvent`, `AsyncPlayerConfigurationEvent`, `PlayerSpawnEvent`, `PlayerMoveEvent`, `PlayerChatEvent`, `PlayerBlockPlaceEvent`, `PlayerBlockBreakEvent`, `PlayerDisconnectEvent`, ... | `minestom_server::event`, `minestom_entity` | `done` | `AsyncPlayerConfigurationEvent`, `PlayerSpawnEvent`, `PlayerMoveEvent`, `PlayerChatEvent`, `PlayerCommandEvent`, `PlayerBlockPlaceEvent`, `PlayerBlockBreakEvent`, `PlayerAttackEntityEvent`, `PlayerEntityInteractEvent`, `PlayerHandAnimationEvent`, `PlayerDisconnectEvent` done (M3–M6) |
| `event.entity`, `event.entity.projectile` | `EntitySpawnEvent`, `EntityDespawnEvent`, `EntityTickEvent`, `EntityAttackEvent`, `EntityDamageEvent`, `EntityDeathEvent`, ... | `minestom_entity` | `partial` | `EntitySpawnEvent`, `EntityDespawnEvent`, `PlayerAttackEntityEvent` done (M4); `EntityDamageEvent` & `EntityDeathEvent` in M9 |
| `event.instance` | `InstanceTickEvent`, `InstanceChunkLoadEvent`, `InstanceChunkUnloadEvent`, `InstanceBlockUpdateEvent`, ... | `minestom_world::event` | `todo` | M2/M5/M7 |
| `event.inventory`, `event.item`, `event.book` | `InventoryPreClickEvent`, `InventoryClickEvent`, `InventoryCloseEvent`, `ItemDropEvent`, `PickupItemEvent`, `EditBookEvent` | `minestom_item` | `done` | Cancellable `InventoryPreClickEvent`, `InventoryCloseEvent`, `ItemDropEvent`, `PickupItemEvent` done (M6) |
| `exception` | `ExceptionManager`, `ExceptionHandler` | `minestom::Error`, `tracing` | `changed` | Replaced by typed `Result<T, E>` (`thiserror`) and `tracing` spans (ADR 0007) |
| `extras.mojangAuth`, `extras.lan` | `MojangCrypt`, `OpenToLAN` | `minestom_network::auth`, `minestom_network::lan` | `todo` | AES/CFB8, RSA, SHA-1 Mojang hex digest, Velocity & BungeeCord forwarding (M8) |
| `game`, `gamedata` | `GameEvent`, `DataPack` | `minestom_data::game_event`, `minestom_protocol::config` | `done` | `GameEvent` IDs + `KnownPack` (`minecraft:core` `26.2`) done (M2) |
| `instance` | `Instance`, `InstanceContainer`, `SharedInstance`, `InstanceManager`, `Chunk`, `DynamicChunk`, `LightingChunk`, `Section`, `WorldBorder`, `EntityTracker` | `minestom_world`, `minestom_entity::tracker` | `done` | `InstanceContainer`, `SharedInstance`, `InstanceManager`, `Chunk`, `Section` (`liquidCount`), `LightingChunk`, `WorldBorder`, `WorldTime`, `Weather`, spatial `EntityTracker` done (M2–M7) |
| `instance.anvil` | `AnvilLoader`, `RegionFile` | `minestom_world::anvil` | `done` | `ChunkLoader` + `.mca` `AnvilLoader` (MC 26.2 `dimensions/<ns>/<val>/region` + classic fallback, Zlib/GZip/Uncompressed, `level.dat`, non-blocking `save_instance_async`) done (M7) |
| `instance.batch` | `Batch`, `AbsoluteBlockBatch`, `RelativeBlockBatch`, `ChunkBatch` | `minestom_world::batch` | `done` | `Batch`, `AbsoluteBlockBatch`, `RelativeBlockBatch` + per-section `MultiBlockChangePacket` sync done (M5) |
| `instance.block`, `rule`, `banner`, `jukebox`, `predicate` | `Block`, `BlockFace`, `BlockHandler`, `BlockManager`, `BlockPlacementRule`, `BlockEntityType`, `BannerPattern`, `JukeboxSong` | `minestom_block`, `minestom_data::block_states`, `minestom_registry` | `done` | Mixed-radix `Block::get_property`/`with_property`, `BlockHandler`, `BlockManager`, `BlockPlacementRule`, block-entity NBT & `Tag<T>` done (M5) |
| `instance.fluid`, `instance.gamerule` | `Fluid`, `GameRule` | `minestom_data::{fluid, game_rule}` | `partial` | Fluid IDs & block liquid flags generated (M2); `GameRule` runtime in M7 |
| `instance.generator` | `Generator`, `GenerationUnit`, `UnitModifier` | `minestom_world::generator` | `done` | Parallel `rayon` chunk generation + `UnitModifier` (`fill_height`, `fill`, `set_block`) done (M2) |
| `instance.heightmap` | `Heightmap`, `MotionBlockingHeightmap`, `WorldSurfaceHeightmap` | `minestom_world::chunk` | `done` | Packed 9-bit `WORLD_SURFACE` & `MOTION_BLOCKING` heightmaps for `ChunkDataPacket` done (M2) |
| `instance.light` | `Light`, `SkyLight`, `BlockLight`, `LightCompute` | `minestom_world::light` | `done` | Opt-in 3D BFS sky & block light propagation (`compute_chunk_light`, `UpdateLightPacket` `0x30`) + nibble packing done (M7) |
| `instance.palette` | `Palette`, `Palettes` | `minestom_world::palette` | `done` | Single, Indirect, and Direct non-spanning 64-bit word `Palette` (MC 26.2 `RAW_LONGS` wire format) done (M2) |
| `inventory`, `inventory.click`, `inventory.type` | `AbstractInventory`, `PlayerInventory`, `Inventory`, `InventoryType`, `ClickType`, `InventoryClickProcessor`, specialized inventories | `minestom_item::inventory` | `done` | 46-slot `PlayerInventory`, shared 25-type `Inventory` windows (`OpenWindow`/`WindowItems`/`CloseWindow`), full `process_inventory_click` (`Pickup`, `QuickMove`, `Swap`, `Clone`, `Throw`, `QuickCraft` paint-drag, `PickupAll`) done (M6) |
| `item`, `armor`, `book`, `crossbow`, `enchant`, `instrument`, `predicate` | `ItemStack`, `Material`, `TrimMaterial`, `TrimPattern`, `Enchantment`, `Instrument`, `ItemPredicate` | `minestom_item`, `minestom_data::materials`, `minestom_registry` | `done` | All 1,458 MC 26.2 `Material`s w/ max stack/damage/corresponding block, `ItemStack` Data Components & `Tag<T>`, `EntityEquipmentPacket`, `ItemEntity` drop & `CollectItemPacket` pickup done (M6) |
| `listener`, `listener.common`, `listener.preplay`, `listener.manager` | `HandshakeListener`, `StatusListener`, `LoginListener`, `KeepAliveListener`, `SettingsListener`, play packet listeners | `minestom_network`, `minestom_server` | `partial` | `Handshake` + `Status` + outdated-protocol `LoginDisconnect` done (M1); Login/Config/Play in M2–M8 |
| `message` | `ChatType`, `ChatPosition`, `Messenger` | `minestom_registry`, `minestom_protocol::chat_command` | `done` | `minecraft:chat_type` dynamic registry + `SystemChatPacket` (`0x79`) & `DisguisedChatPacket` (`0x21`) broadcasting done (M3) |
| `monitoring` | `TickMonitor`, `EventsJFR` | `minestom_server::monitor`, `tracing` | `changed` | `TickMonitor` event + `tracing` spans instead of JVM JFR (`EventsJFR` won't port) |
| `network`, `network.socket`, `network.player`, `network.plugin` | `NetworkBuffer`, `ConnectionState`, `ConnectionManager`, `PlayerConnection`, `Server`, `GameProfile`, `ClientSettings` | `minestom_protocol`, `minestom_network` | `partial` | `FrameCodec` (uncompressed + Zlib), `PacketStream`, Tokio accept loop done (M1); Play/Auth in M2/M8 |
| `network.debug.**` | `DebugSubscription`, `DebugBrainDump`, ... | `minestom_protocol::debug` | `todo` | Optional debug subscriptions (M10) |
| `network.packet.client.**` | Handshake, Status, Login, Configuration, Common, Play serverbound packets | `minestom_protocol::{handshake, status, login, config, play}` | `partial` | `Handshake` (`0x00`) & `Status` (`0x00`, `0x01`) done & proptested (M1); Login/Config/Play in M2–M10 |
| `network.packet.server.**` | Status, Login, Configuration, Common, Play clientbound packets + `CachedPacket`, `FramedPacket` | `minestom_protocol::{status, login, config, play}` | `partial` | `StatusResponse` (`0x00`), `StatusPong` (`0x01`), `LoginDisconnect` (`0x00`) done (M1); Login/Config/Play in M2–M10 |
| `particle` | `Particle` | `minestom_data::particle`, `minestom_protocol::play` | `partial` | Static IDs generated (M0); payload codecs in M10 |
| `ping` | `Status`, `ServerListPingType` | `minestom_protocol::status` | `done` | `ServerStatus`, `StatusVersion`, `StatusPlayers`, base64 PNG favicon & 64×64 PNG encoder done (M1) |
| `potion` | `PotionEffect`, `PotionType`, `Potion`, `TimedPotion` | `minestom_data::potion`, `minestom_entity::effect` | `partial` | Static IDs generated (M0); active entity effects in M9 |
| `property` | `ServerProperties`, `ServerProperty` | `minestom_server::config::ServerConfig` | `done` | Typed `ServerConfig` + `MinecraftServer::builder()` + TOML `serde` done (M1, ADR 0007) |
| `recipe`, `recipe.display` | `Recipe`, `RecipeManager`, `Ingredient`, `RecipeDisplay`, `SlotDisplay` | `minestom_server::recipe` | `todo` | M10 |
| `registry` | `Registry`, `StaticRegistry`, `DynamicRegistry`, `Registries`, `VanillaRegistries`, `RegistryKey`, `TagKey`, `RegistryTag` | `minestom_registry` | `partial` | All 27 MC 26.2 configuration dynamic registries + 32 resolved client tag registries + `DimensionType` done (M2); custom runtime mutation in M7/M10 |
| `scoreboard` | `Scoreboard`, `Sidebar`, `Team`, `TeamManager`, `TabList`, `BelowNameTag` | `minestom_server::scoreboard` | `todo` | M10 |
| `snapshot` | `ServerSnapshot`, `InstanceSnapshot`, `ChunkSnapshot`, `EntitySnapshot`, `PlayerSnapshot` | `minestom_server::snapshot` | `todo` | Immutable point-in-time read snapshots (M7/M10) |
| `sound` | `SoundEvent`, `BuiltinSoundEvent`, `CustomSoundEvent`, `BlockSoundType`, `Music` | `minestom_data::sound`, `minestom_core::sound` | `partial` | Static IDs generated (M0); packets in M10 |
| `statistic` | `StatisticType`, `StatisticCategory`, `PlayerStatistic` | `minestom_data::statistic`, `minestom_server::statistic` | `todo` | M10 |
| `tag` | `Tag`, `TagHandler`, `TagReadable`, `TagWritable`, `TagSerializer` | `minestom_core::tag` | `done` | Generic `Tag<T: TagSerializable>` + `TagMap` backed by `minestom_nbt` done (M1) |
| `thread` | `ThreadDispatcher`, `TickThread`, `TickSchedulerThread`, `Acquirable` | `minestom_server::tick` | `changed` | Dedicated tick pool + generational arenas replace `Acquirable` locks (ADR 0002, M2) |
| `timer` | `Scheduler`, `SchedulerManager`, `Task`, `TaskSchedule`, `ExecutionType` | `minestom_scheduler` | `done` | Deterministic 20 TPS `Scheduler`, `SchedulerManager`, `TaskBuilder`, `TaskHandle`, `TaskSchedule` (`TickStart`/`TickEnd` + shutdown hooks) done (M7) |
| `utils.nbt` | `BinaryTagReader`, `BinaryTagWriter`, `BinaryTagUtil` | `minestom_nbt` | `done` | Safe named + nameless network NBT + GZip/Zlib + SNBT + JSON↔NBT done & proptested (M1) |
| `utils.**` | `NamespaceID`/`Key`, `Direction`, `MathUtils`, `UUIDUtils`, `MojangUtils`, `BlockIterator`, ... | `minestom_core`, `std`, ecosystem crates | `partial` | `Key`/`NamespaceId`, `Direction`/`BlockFace`, `offline_player_uuid` done (M1) |
| `world`, `world.biome`, `world.attribute`, `world.clock`, `world.timeline` | `DimensionType`, `Difficulty`, `MoonPhase`, `Biome`, `EnvironmentAttribute`, `WorldClock`, `ClockTimeMarker`, `Timeline` | `minestom_registry`, `minestom_world`, `minestom_protocol::play` | `done` | MC 26.2 `DimensionType`, `world_clock` & `timeline` dynamic registries, multi-clock `SetTimePacket` (`0x71`), `WorldTime` & `Weather` done (M2/M7) |
