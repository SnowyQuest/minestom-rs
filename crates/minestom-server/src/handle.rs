//! Ergonomic, cheaply cloneable [`PlayerRef`] and [`EntityRef`] handles
//! (`net.minestom.server.entity.Player` / `Entity` runtime handles).
//!
//! Unlike raw IDs (`PlayerId`, `i32`), [`PlayerRef`] and [`EntityRef`] pair an identifier with a
//! [`MinecraftServer`] handle so event listeners, command executors, and async tasks can invoke
//! `player.send_message(...)`, `player.teleport(...)`, `player.give_item(...)`, or
//! `entity.set_meta(...)` directly in strict FIFO order without passing `&MinecraftServer` everywhere.

use std::fmt;

use minestom_core::{GameMode, InstanceId, PlayerId, Pos, Vec3};
use minestom_entity::metadata_def::{BitMaskKey, MetadataKey};
use minestom_entity::{
    Attribute, AttributeModifier, Damage, DamageType, EntityType, Potion, PotionEffect,
};
use minestom_item::{Inventory, ItemStack};
use minestom_protocol::ui_media::SoundSource;
use minestom_text::Component;
use uuid::Uuid;

use crate::MinecraftServer;
use crate::features::{
    AdvancementTab, BossBar, Dialog, ResourcePackRequest, Sidebar, Sound, Title, ToastNotification,
};
use crate::snapshot::{EntitySnapshot, PlayerSnapshot};

/// Ergonomic handle to an online (or disconnecting) player bound to a [`MinecraftServer`].
///
/// Cloning a `PlayerRef` is a cheap `Arc` refcount bump. All mutating methods enqueue commands
/// onto the server's unified bounded FIFO channel so sequential calls (`player.set_slot(...)`
/// followed by `player.open_inventory(...)`, or `player.send_message(...)` followed by
/// `player.kick(...)`) execute in exact causal order on the 20 TPS tick loop.
#[derive(Clone)]
pub struct PlayerRef {
    id: PlayerId,
    server: MinecraftServer,
}

impl fmt::Debug for PlayerRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlayerRef")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for PlayerRef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for PlayerRef {}

impl From<PlayerRef> for PlayerId {
    fn from(value: PlayerRef) -> Self {
        value.id
    }
}

impl From<&PlayerRef> for PlayerId {
    fn from(value: &PlayerRef) -> Self {
        value.id
    }
}

impl PlayerRef {
    pub(crate) const fn new(id: PlayerId, server: MinecraftServer) -> Self {
        Self { id, server }
    }

    /// Generational [`PlayerId`].
    #[inline]
    #[must_use]
    pub const fn id(&self) -> PlayerId {
        self.id
    }

    /// Borrows the owning [`MinecraftServer`].
    #[inline]
    #[must_use]
    pub const fn server(&self) -> &MinecraftServer {
        &self.server
    }

    /// Returns a point-in-time [`PlayerSnapshot`] from the latest tick snapshot, if online.
    #[must_use]
    pub fn snapshot(&self) -> Option<PlayerSnapshot> {
        self.server.snapshot().player(self.id).cloned()
    }

    /// Returns `true` if the player is present in the latest server tick snapshot.
    #[must_use]
    pub fn is_online(&self) -> bool {
        self.server.snapshot().player(self.id).is_some()
    }

    /// Protocol entity ID (`Some` if present in the latest tick snapshot).
    #[must_use]
    pub fn entity_id(&self) -> Option<i32> {
        self.server.snapshot().player(self.id).map(|p| p.entity_id)
    }

    /// Player [`Uuid`] (`Some` if present in the latest tick snapshot).
    #[must_use]
    pub fn uuid(&self) -> Option<Uuid> {
        self.server.snapshot().player(self.id).map(|p| p.uuid)
    }

    /// Player username (`Some` if present in the latest tick snapshot).
    #[must_use]
    pub fn username(&self) -> Option<String> {
        self.server
            .snapshot()
            .player(self.id)
            .map(|p| p.username.clone())
    }

    /// Current world [`Pos`] (`Some` if present in the latest tick snapshot).
    #[must_use]
    pub fn position(&self) -> Option<Pos> {
        self.server.snapshot().player(self.id).map(|p| p.position)
    }

    /// Current [`InstanceId`] (`Some` if present in the latest tick snapshot).
    #[must_use]
    pub fn instance_id(&self) -> Option<InstanceId> {
        self.server
            .snapshot()
            .player(self.id)
            .map(|p| p.instance_id)
    }

    /// Current [`GameMode`] (`Some` if present in the latest tick snapshot).
    #[must_use]
    pub fn game_mode(&self) -> Option<GameMode> {
        self.server.snapshot().player(self.id).map(|p| p.game_mode)
    }

    /// Current health points (`Some` if present in the latest tick snapshot).
    #[must_use]
    pub fn health(&self) -> Option<f32> {
        self.server.snapshot().player(self.id).map(|p| p.health)
    }

    /// Maximum health points (`Some` if present in the latest tick snapshot).
    #[must_use]
    pub fn max_health(&self) -> Option<f32> {
        self.server.snapshot().player(self.id).map(|p| p.max_health)
    }

    /// Sends a system chat [`Component`] to this player.
    pub fn send_message(&self, message: impl Into<Component>) {
        self.server.send_message(self.id, message);
    }

    /// Sends an above-hotbar action bar [`Component`] to this player.
    pub fn send_action_bar(&self, text: impl Into<Component>) {
        self.server.send_action_bar(self.id, text);
    }

    /// Sends a full-screen [`Title`] to this player.
    pub fn send_title(&self, title: &Title) {
        self.server.send_title(self.id, title);
    }

    /// Clears active titles on this player.
    pub fn clear_titles(&self, reset_times: bool) {
        self.server.clear_titles(self.id, reset_times);
    }

    /// Sends a pop-up [`ToastNotification`] to this player.
    pub fn send_toast(&self, toast: &ToastNotification) {
        self.server.send_toast(self.id, toast);
    }

    /// Synchronizes an [`AdvancementTab`] to this player.
    pub fn send_advancement_tab(&self, tab: &AdvancementTab, reset: bool) {
        self.server.send_advancement_tab(self.id, tab, reset);
    }

    /// Opens a modal [`Dialog`] on this player.
    pub fn show_dialog(&self, dialog: &Dialog) {
        self.server.show_dialog(self.id, dialog);
    }

    /// Closes any active modal dialog on this player.
    pub fn clear_dialog(&self) {
        self.server.clear_dialog(self.id);
    }

    /// Sets the Tab-list header and footer [`Component`]s for this player.
    pub fn set_tab_header_footer(
        &self,
        header: impl Into<Component>,
        footer: impl Into<Component>,
    ) {
        self.server
            .set_player_list_header_footer(self.id, header, footer);
    }

    /// Plays `sound` at `origin` exclusively to this player.
    pub fn play_sound(&self, sound: &Sound, origin: Vec3) {
        self.server.play_sound_to_player(self.id, sound, origin);
    }

    /// Stops matching sounds on this player.
    pub fn stop_sound(&self, source: Option<SoundSource>, sound_key: Option<String>) {
        self.server.stop_sound(self.id, source, sound_key);
    }

    /// Teleports this player to `position` and synchronizes chunk streaming & entity viewers.
    pub fn teleport(&self, position: Pos) {
        self.server.teleport_player(self.id, position);
    }

    /// Sets velocity on this player and sends `EntityVelocityPacket`.
    pub fn set_velocity(&self, velocity: Vec3) {
        self.server.set_player_velocity(self.id, velocity);
    }

    /// Changes this player's [`GameMode`] and syncs abilities + tab list across all players.
    pub fn set_game_mode(&self, mode: GameMode) {
        self.server.set_player_game_mode(self.id, mode);
    }

    /// Updates this player's respawn [`Pos`].
    pub fn set_respawn_point(&self, position: Pos) {
        self.server.set_player_respawn_point(self.id, position);
    }

    /// Adds `item` to this player's inventory (dropping any overflow into the world).
    pub fn give_item(&self, item: ItemStack) {
        self.server.give_player_item(self.id, item);
    }

    /// Sets window-0 `slot` (`0..46`) in this player's inventory.
    pub fn set_slot(&self, slot: usize, item: ItemStack) {
        self.server.set_player_slot(self.id, slot, item);
    }

    /// Opens a custom container [`Inventory`] window for this player.
    pub fn open_inventory(&self, inventory: &Inventory) {
        self.server.open_inventory(self.id, inventory);
    }

    /// Closes any open container window on this player.
    pub fn close_inventory(&self) {
        self.server.close_inventory(self.id);
    }

    /// Applies [`Damage`] to this player.
    pub fn damage(&self, damage: Damage) {
        self.server.damage_player(self.id, damage);
    }

    /// Restores this player to full maximum health.
    pub fn heal(&self) {
        self.server.heal_player(self.id);
    }

    /// Kills this player immediately using [`DamageType::GENERIC_KILL`].
    pub fn kill(&self) {
        self.server
            .damage_player(self.id, Damage::of(DamageType::GENERIC_KILL, 1_000_000.0));
    }

    /// Applies `potion` to this player.
    pub fn add_effect(&self, potion: Potion) {
        self.server.add_player_effect(self.id, potion);
    }

    /// Removes `effect` from this player.
    pub fn remove_effect(&self, effect: PotionEffect) {
        self.server.remove_player_effect(self.id, effect);
    }

    /// Sets the base value of `attribute` on this player.
    pub fn set_attribute(&self, attribute: Attribute, base_value: f64) {
        self.server
            .set_player_attribute(self.id, attribute, base_value);
    }

    /// Adds or replaces an [`AttributeModifier`] on this player.
    pub fn add_attribute_modifier(&self, attribute: Attribute, modifier: AttributeModifier) {
        self.server
            .add_player_attribute_modifier(self.id, attribute, modifier);
    }

    /// Displays `sidebar` to this player.
    pub fn show_sidebar(&self, sidebar: &Sidebar) {
        self.server.add_sidebar_viewer(sidebar, self.id);
    }

    /// Hides `sidebar` from this player.
    pub fn hide_sidebar(&self, sidebar: &Sidebar) {
        self.server.remove_sidebar_viewer(sidebar, self.id);
    }

    /// Displays `boss_bar` to this player.
    pub fn show_boss_bar(&self, boss_bar: &BossBar) {
        self.server.add_boss_bar_viewer(boss_bar, self.id);
    }

    /// Hides `boss_bar` from this player.
    pub fn hide_boss_bar(&self, boss_bar: &BossBar) {
        self.server.remove_boss_bar_viewer(boss_bar, self.id);
    }

    /// Pushes a server [`ResourcePackRequest`] to this player.
    pub fn push_resource_pack(&self, request: &ResourcePackRequest) {
        self.server.push_resource_pack(self.id, request);
    }

    /// Pops one (`Some(id)`) or all (`None`) server resource packs from this player.
    pub fn pop_resource_pack(&self, id: Option<Uuid>) {
        self.server.pop_resource_pack(self.id, id);
    }

    /// Disconnects this player with `reason`.
    pub fn kick(&self, reason: impl Into<Component>) {
        self.server.kick_player(self.id, reason);
    }
}

/// Ergonomic handle to a spawned world [`minestom_entity::Entity`] bound to a [`MinecraftServer`].
///
/// Returned immediately by [`MinecraftServer::spawn_entity`] and [`MinecraftServer::spawn_item`].
/// Implements [`fmt::Display`] (printing `network_id`), `PartialEq<i32>`, and `From<EntityRef> for i32`
/// so it works seamlessly both as a rich object handle and wherever a protocol `entity_id: i32` is expected.
#[derive(Clone)]
pub struct EntityRef {
    network_id: i32,
    entity_type: EntityType,
    instance_id: InstanceId,
    server: MinecraftServer,
}

impl fmt::Debug for EntityRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntityRef")
            .field("network_id", &self.network_id)
            .field("entity_type", &self.entity_type)
            .field("instance_id", &self.instance_id)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for EntityRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.network_id)
    }
}

impl PartialEq for EntityRef {
    fn eq(&self, other: &Self) -> bool {
        self.network_id == other.network_id
    }
}

impl Eq for EntityRef {}

impl PartialEq<i32> for EntityRef {
    fn eq(&self, other: &i32) -> bool {
        self.network_id == *other
    }
}

impl PartialEq<EntityRef> for i32 {
    fn eq(&self, other: &EntityRef) -> bool {
        *self == other.network_id
    }
}

impl From<EntityRef> for i32 {
    fn from(value: EntityRef) -> Self {
        value.network_id
    }
}

impl From<&EntityRef> for i32 {
    fn from(value: &EntityRef) -> Self {
        value.network_id
    }
}

impl EntityRef {
    pub(crate) const fn new(
        network_id: i32,
        entity_type: EntityType,
        instance_id: InstanceId,
        server: MinecraftServer,
    ) -> Self {
        Self {
            network_id,
            entity_type,
            instance_id,
            server,
        }
    }

    /// Assigned runtime protocol `entity_id`.
    #[inline]
    #[must_use]
    pub const fn network_id(&self) -> i32 {
        self.network_id
    }

    /// [`EntityType`] of this entity.
    #[inline]
    #[must_use]
    pub const fn entity_type(&self) -> EntityType {
        self.entity_type
    }

    /// [`InstanceId`] in which this entity was spawned.
    #[inline]
    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    /// Returns a point-in-time [`EntitySnapshot`] from the latest tick snapshot, if alive.
    #[must_use]
    pub fn snapshot(&self) -> Option<EntitySnapshot> {
        self.server.snapshot().entity(self.network_id).cloned()
    }

    /// Returns `true` if the entity is present in the latest server tick snapshot.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.server.snapshot().entity(self.network_id).is_some()
    }

    /// Current [`Pos`] from the latest tick snapshot, if alive.
    #[must_use]
    pub fn position(&self) -> Option<Pos> {
        self.server
            .snapshot()
            .entity(self.network_id)
            .map(|e| e.position)
    }

    /// Current velocity from the latest tick snapshot, if alive.
    #[must_use]
    pub fn velocity(&self) -> Option<Vec3> {
        self.server
            .snapshot()
            .entity(self.network_id)
            .map(|e| e.velocity)
    }

    /// Current health from the latest tick snapshot, if living and alive.
    #[must_use]
    pub fn health(&self) -> Option<f32> {
        self.server
            .snapshot()
            .entity(self.network_id)
            .and_then(|e| e.health)
    }

    /// Teleports this entity to `position` and syncs all viewers.
    pub fn teleport(&self, position: Pos) {
        self.server.teleport_entity(self.network_id, position);
    }

    /// Sets velocity on this entity and broadcasts `EntityVelocityPacket` to viewers.
    pub fn set_velocity(&self, velocity: Vec3) {
        self.server.set_entity_velocity(self.network_id, velocity);
    }

    /// Updates a typed [`MetadataKey`] (`minestom_entity::metadata_def`) on this live entity
    /// and broadcasts `EntityMetaDataPacket` to all viewers.
    pub fn set_meta<T>(&self, key: MetadataKey<T>, value: T) {
        self.server
            .set_entity_metadata_key(self.network_id, key, value);
    }

    /// Updates a [`BitMaskKey`] (`minestom_entity::metadata_def`) on this live entity
    /// and broadcasts `EntityMetaDataPacket` to all viewers.
    pub fn set_flag(&self, mask: BitMaskKey, enabled: bool) {
        self.server
            .set_entity_metadata_flag(self.network_id, mask, enabled);
    }

    /// Updates this entity's custom nameplate and syncs viewers.
    pub fn set_custom_name(&self, name: Option<Component>) {
        self.set_meta(minestom_entity::metadata_def::entity::CUSTOM_NAME, name);
    }

    /// Updates whether this entity's custom nameplate is always visible and syncs viewers.
    pub fn set_custom_name_visible(&self, visible: bool) {
        self.set_meta(
            minestom_entity::metadata_def::entity::CUSTOM_NAME_VISIBLE,
            visible,
        );
    }

    /// Applies [`Damage`] to this entity.
    pub fn damage(&self, damage: Damage) {
        self.server.damage_entity(self.network_id, damage);
    }

    /// Applies `potion` to this entity.
    pub fn add_effect(&self, potion: Potion) {
        self.server.add_entity_effect(self.network_id, potion);
    }

    /// Removes `effect` from this entity.
    pub fn remove_effect(&self, effect: PotionEffect) {
        self.server.remove_entity_effect(self.network_id, effect);
    }

    /// Sets the base value of `attribute` on this entity.
    pub fn set_attribute(&self, attribute: Attribute, base_value: f64) {
        self.server
            .set_entity_attribute(self.network_id, attribute, base_value);
    }

    /// Adds or replaces an [`AttributeModifier`] on this entity.
    pub fn add_attribute_modifier(&self, attribute: Attribute, modifier: AttributeModifier) {
        self.server
            .add_entity_attribute_modifier(self.network_id, attribute, modifier);
    }

    /// Despawns and removes this entity from its world instance.
    pub fn remove(&self) {
        self.server.remove_entity(self.network_id);
    }
}
