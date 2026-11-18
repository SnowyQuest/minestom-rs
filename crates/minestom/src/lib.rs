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
    pub use crate::core::{
        BlockFace, BlockVec, ChunkPos, Color, Difficulty, Direction, EntityId, GameMode, Hand,
        InstanceId, Key, MainHand, NamespaceId, PlayerId, Point, Pos, Tag, TagMap, Vec3,
    };
    pub use crate::data::{DATA_VERSION, PROTOCOL_VERSION, VERSION_NAME};
    pub use crate::event::{CancellableEvent, Event};
    pub use crate::nbt::{NbtCompound, NbtTag};
    pub use crate::protocol::ConnectionState;
    pub use crate::scheduler::ExecutionType;
    pub use crate::server::{BoundServer, MinecraftServer, ServerConfig};
    pub use crate::text::{ClickEvent, Component, HoverEvent, NamedTextColor, Style, TextColor};
}
