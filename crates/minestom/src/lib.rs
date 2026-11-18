//! `minestom` — A production-quality Minecraft: Java Edition **26.2** (protocol **776**)
//! server library written from scratch in safe, idiomatic Rust with zero vanilla code.
//!
//! Re-exports all workspace crates under ergonomic module names alongside [`prelude`]
//! and the unified [`Error`] type.

pub use minestom_block as block;
pub use minestom_command as command;
pub use minestom_core as core;
pub use minestom_data as data;
pub use minestom_entity as entity;
pub use minestom_event as event;
pub use minestom_item as item;
pub use minestom_macros as macros;
pub use minestom_nbt as nbt;
pub use minestom_network as network;
pub use minestom_protocol as protocol;
pub use minestom_registry as registry;
pub use minestom_scheduler as scheduler;
pub use minestom_server as server;
pub use minestom_text as text;
pub use minestom_world as world;

/// Unified top-level error type encompassing all `minestom-rs` subsystems.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Core coordinate/key error.
    #[error(transparent)]
    Core(#[from] minestom_core::CoreError),
    /// NBT codec error.
    #[error(transparent)]
    Nbt(#[from] minestom_nbt::NbtError),
    /// Text component error.
    #[error(transparent)]
    Text(#[from] minestom_text::TextError),
    /// Registry error.
    #[error(transparent)]
    Registry(#[from] minestom_registry::RegistryError),
    /// Block error.
    #[error(transparent)]
    Block(#[from] minestom_block::BlockError),
    /// Item/inventory error.
    #[error(transparent)]
    Item(#[from] minestom_item::ItemError),
    /// Protocol packet/framing error.
    #[error(transparent)]
    Protocol(#[from] minestom_protocol::ProtocolError),
    /// Network transport/auth error.
    #[error(transparent)]
    Network(#[from] minestom_network::NetworkError),
    /// World/chunk/Anvil error.
    #[error(transparent)]
    World(#[from] minestom_world::WorldError),
    /// Entity error.
    #[error(transparent)]
    Entity(#[from] minestom_entity::EntityError),
    /// Command error.
    #[error(transparent)]
    Command(#[from] minestom_command::CommandError),
    /// Server runtime/configuration error.
    #[error(transparent)]
    Server(#[from] minestom_server::ServerError),
}

/// Convenient prelude re-exporting the most frequently used `minestom-rs` types and traits.
pub mod prelude {
    pub use crate::Error;
    pub use crate::block::{
        AxisPlacementRule, Block, BlockDestroyContext, BlockGetter, BlockHandler,
        BlockInteractContext, BlockManager, BlockPlacementContext, BlockPlacementRule, BlockSetter,
        HorizontalFacingPlacementRule, PlacementState, UpdateState,
    };
    pub use crate::command::{
        Argument, Command, CommandContext, CommandError, CommandManager, CommandSender, Suggestion,
        SuggestionEntry,
    };
    pub use crate::core::{
        BlockFace, BlockVec, ChunkPos, Color, Difficulty, Direction, EntityId, GameMode, Hand,
        InstanceId, Key, MainHand, NamespaceId, PlayerId, Point, Pos, Tag, TagMap, Vec3,
    };
    pub use crate::data::{DATA_VERSION, PROTOCOL_VERSION, VERSION_NAME};
    pub use crate::entity::{
        BoundingBox, ConfiguringPlayer, Entity, EntityBehavior, EntityCore, EntityDespawnEvent,
        EntityMetadata, EntityRole, EntitySpawnEvent, EntityTracker, EntityType, LivingState,
        PlayerAttackEntityEvent, PlayerEntityInteractEvent, PlayerHandAnimationEvent,
    };
    pub use crate::event::{
        BlockEvent, CancellableEvent, EntityEvent, Event, EventListener, EventNode,
        GlobalEventHandler, InstanceEvent, ListenerHandle, PlayerEvent,
    };
    pub use crate::item::{
        ClickOutcome, DataComponentValue, EquipmentSlot, Inventory, InventoryCloseEvent,
        InventoryPreClickEvent, InventoryType, ItemDropEvent, ItemError, ItemRarity, ItemStack,
        Material, PickupItemEvent, PlayerInventory, WindowClickMode, process_inventory_click,
    };
    pub use crate::nbt::{NbtCompound, NbtTag};
    pub use crate::protocol::ConnectionState;
    pub use crate::registry::{DimensionType, Registries};
    pub use crate::scheduler::{
        ExecutionType, Scheduler, SchedulerManager, TaskBuilder, TaskHandle, TaskId, TaskSchedule,
    };
    pub use crate::server::{
        AsyncPlayerConfigurationEvent, BoundServer, ClientPingServerEvent, MinecraftServer,
        PlayerBlockBreakEvent, PlayerBlockPlaceEvent, PlayerChatEvent, PlayerCommandEvent,
        PlayerDisconnectEvent, PlayerMoveEvent, PlayerSpawnEvent, ServerConfig,
        ServerListPingEvent,
    };
    pub use crate::text::{ClickEvent, Component, HoverEvent, NamedTextColor, Style, TextColor};
    pub use crate::world::{
        AbsoluteBlockBatch, AnvilLoader, Batch, BlockEntityState, Chunk, ChunkLoader,
        GenerationUnit, Generator, InstanceContainer, InstanceManager, Palette, RelativeBlockBatch,
        Section, SharedInstance, UnitModifier, Weather, WorldBorder, WorldTime,
        compute_chunk_light,
    };
}
