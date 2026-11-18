//! High-level server UI, media, and gameplay subsystems: `Sidebar` scoreboards, `TeamManager`,
//! `BossBar`, `Title` & `ActionBar`, `Particle` (`125` constants) & `Sound` (`1,968` `SoundEvent`s),
//! `ToastNotification` & `AdvancementTab`, `RecipeManager`, `ResourcePackRequest`, and MC 26.2 `Dialog` builders.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use minestom_block::Block;
use minestom_core::{PlayerId, Vec3};
use minestom_data::generated::{particles, sounds};
use minestom_event::{Event, PlayerEvent};
use minestom_item::{ItemStack, Material};
use minestom_nbt::{NbtCompound, NbtTag};
use minestom_protocol::ui_media::{
    AdvancementDisplayData, AdvancementEntry, AdvancementFrameType, AdvancementProgressEntry,
    AdvancementsPacket, BossBarAction, BossBarColor, BossBarOverlay, BossBarPacket,
    DeclareRecipesPacket, DialogHolder, DisplayScoreboardPacket, ObjectiveAction,
    ObjectiveRenderType, ParticleData, ParticlePacket, ResetScorePacket, ResourcePackPushPacket,
    ResourcePackStatus, ScoreboardObjectivePacket, SetTitleSubTitlePacket, SetTitleTextPacket,
    SetTitleTimePacket, ShowDialogPacket, SidebarNumberFormat, SoundEffectPacket, SoundHolder,
    SoundSource, StonecutterRecipeEntry, TeamSettings, TeamsAction, TeamsPacket, UpdateScorePacket,
};
pub use minestom_protocol::ui_media::{RecipePropertySet, SoundEvent};
use minestom_text::Component;
use parking_lot::RwLock;
use uuid::Uuid;

static NEXT_SIDEBAR_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_TOAST_ID: AtomicU64 = AtomicU64::new(1);

// ============================================================================
// Sidebar Scoreboard (`net.minestom.server.scoreboard.Sidebar`)
// ============================================================================

/// A single line inside a [`Sidebar`] (`Sidebar.ScoreboardLine` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarLine {
    /// Stable line identifier (also used as the synthetic score holder key).
    pub id: String,
    /// Rendered text component.
    pub content: Component,
    /// Sort order score (higher values appear higher in the sidebar).
    pub score: i32,
    /// Score number formatting (defaults to [`SidebarNumberFormat::Blank`] for clean custom lines).
    pub number_format: SidebarNumberFormat,
}

impl SidebarLine {
    /// Creates a [`SidebarLine`] with hidden score numbers (`SidebarNumberFormat::Blank`).
    #[must_use]
    pub fn new(id: impl Into<String>, content: impl Into<Component>, score: i32) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            score,
            number_format: SidebarNumberFormat::Blank,
        }
    }

    /// Overrides the right-aligned score number format.
    #[must_use]
    pub fn with_number_format(mut self, format: SidebarNumberFormat) -> Self {
        self.number_format = format;
        self
    }

    pub(crate) fn to_update_packet(&self, objective_name: &str) -> UpdateScorePacket {
        UpdateScorePacket {
            entity_name: self.id.clone(),
            objective_name: objective_name.to_owned(),
            score: self.score,
            display_name: Some(self.content.clone()),
            number_format: Some(self.number_format.clone()),
        }
    }
}

#[derive(Debug)]
struct SidebarInner {
    objective_name: String,
    title: Component,
    lines: BTreeMap<String, SidebarLine>,
    viewers: BTreeSet<PlayerId>,
}

/// Thread-safe custom right-hand HUD [`Sidebar`] (`Sidebar` in Minestom).
#[derive(Debug, Clone)]
pub struct Sidebar {
    inner: Arc<RwLock<SidebarInner>>,
}

impl Sidebar {
    /// Creates a new [`Sidebar`] with `title` (supports up to 15 lines).
    #[must_use]
    pub fn new(title: impl Into<Component>) -> Self {
        let id = NEXT_SIDEBAR_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::new(RwLock::new(SidebarInner {
                objective_name: format!("sb-{id}"),
                title: title.into(),
                lines: BTreeMap::new(),
                viewers: BTreeSet::new(),
            })),
        }
    }

    /// Unique protocol objective name (`"sb-<n>"`).
    #[must_use]
    pub fn objective_name(&self) -> String {
        self.inner.read().objective_name.clone()
    }

    /// Current title [`Component`].
    #[must_use]
    pub fn title(&self) -> Component {
        self.inner.read().title.clone()
    }

    /// Updates the sidebar title and returns the `(viewers, ScoreboardObjectivePacket)` to broadcast.
    pub fn set_title(
        &self,
        title: impl Into<Component>,
    ) -> (Vec<PlayerId>, ScoreboardObjectivePacket) {
        let mut g = self.inner.write();
        let title = title.into();
        g.title = title.clone();
        let pkt = ScoreboardObjectivePacket {
            objective_name: g.objective_name.clone(),
            action: ObjectiveAction::Update {
                title,
                render_type: ObjectiveRenderType::Integer,
                number_format: Some(SidebarNumberFormat::Blank),
            },
        };
        (g.viewers.iter().copied().collect(), pkt)
    }

    /// Inserts or replaces a [`SidebarLine`] (up to 15 lines) and returns `(viewers, UpdateScorePacket)`.
    pub fn set_line(&self, line: SidebarLine) -> Option<(Vec<PlayerId>, UpdateScorePacket)> {
        let mut g = self.inner.write();
        if g.lines.len() >= 15 && !g.lines.contains_key(&line.id) {
            return None;
        }
        let pkt = line.to_update_packet(&g.objective_name);
        g.lines.insert(line.id.clone(), line);
        Some((g.viewers.iter().copied().collect(), pkt))
    }

    /// Updates the text content of an existing line by `id`.
    pub fn update_line_content(
        &self,
        id: &str,
        content: impl Into<Component>,
    ) -> Option<(Vec<PlayerId>, UpdateScorePacket)> {
        let mut g = self.inner.write();
        let obj = g.objective_name.clone();
        let line = g.lines.get_mut(id)?;
        line.content = content.into();
        let pkt = line.to_update_packet(&obj);
        Some((g.viewers.iter().copied().collect(), pkt))
    }

    /// Removes a line by `id` and returns `(viewers, ResetScorePacket)`.
    pub fn remove_line(&self, id: &str) -> Option<(Vec<PlayerId>, ResetScorePacket)> {
        let mut g = self.inner.write();
        g.lines.remove(id)?;
        let pkt = ResetScorePacket {
            owner: id.to_owned(),
            objective: Some(g.objective_name.clone()),
        };
        Some((g.viewers.iter().copied().collect(), pkt))
    }

    pub(crate) fn register_viewer(
        &self,
        player_id: PlayerId,
    ) -> (
        ScoreboardObjectivePacket,
        DisplayScoreboardPacket,
        Vec<UpdateScorePacket>,
    ) {
        let mut g = self.inner.write();
        g.viewers.insert(player_id);
        let create = ScoreboardObjectivePacket {
            objective_name: g.objective_name.clone(),
            action: ObjectiveAction::Create {
                title: g.title.clone(),
                render_type: ObjectiveRenderType::Integer,
                number_format: Some(SidebarNumberFormat::Blank),
            },
        };
        let display = DisplayScoreboardPacket {
            position: 1,
            score_name: g.objective_name.clone(),
        };
        let scores = g
            .lines
            .values()
            .map(|l| l.to_update_packet(&g.objective_name))
            .collect();
        (create, display, scores)
    }

    pub(crate) fn unregister_viewer(
        &self,
        player_id: PlayerId,
    ) -> Option<ScoreboardObjectivePacket> {
        let mut g = self.inner.write();
        if g.viewers.remove(&player_id) {
            Some(ScoreboardObjectivePacket {
                objective_name: g.objective_name.clone(),
                action: ObjectiveAction::Remove,
            })
        } else {
            None
        }
    }
}

// ============================================================================
// Team & TeamManager (`net.minestom.server.scoreboard.TeamManager`)
// ============================================================================

#[derive(Debug, Clone)]
struct TeamEntry {
    name: String,
    settings: TeamSettings,
    members: BTreeSet<String>,
}

/// Thread-safe scoreboard team registry (`TeamManager` in Minestom).
#[derive(Debug, Clone, Default)]
pub struct TeamManager {
    teams: Arc<RwLock<BTreeMap<String, TeamEntry>>>,
}

impl TeamManager {
    /// Creates an empty [`TeamManager`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates or replaces a team and returns the [`TeamsPacket`] (`Create`) to broadcast.
    pub fn create_team(&self, name: impl Into<String>, settings: TeamSettings) -> TeamsPacket {
        let name = name.into();
        let mut g = self.teams.write();
        g.insert(
            name.clone(),
            TeamEntry {
                name: name.clone(),
                settings: settings.clone(),
                members: BTreeSet::new(),
            },
        );
        TeamsPacket {
            team_name: name,
            action: TeamsAction::Create {
                settings,
                entities: Vec::new(),
            },
        }
    }

    /// Updates a team's display/collision settings and returns the [`TeamsPacket`] (`Update`).
    pub fn update_team(&self, name: &str, settings: TeamSettings) -> Option<TeamsPacket> {
        let mut g = self.teams.write();
        let entry = g.get_mut(name)?;
        entry.settings = settings.clone();
        Some(TeamsPacket {
            team_name: name.to_owned(),
            action: TeamsAction::Update(settings),
        })
    }

    /// Adds a member identifier (username or entity UUID string) to `team_name`.
    pub fn add_member(&self, team_name: &str, entity: impl Into<String>) -> Option<TeamsPacket> {
        let entity = entity.into();
        let mut g = self.teams.write();
        let entry = g.get_mut(team_name)?;
        entry.members.insert(entity.clone());
        Some(TeamsPacket {
            team_name: team_name.to_owned(),
            action: TeamsAction::AddEntities(vec![entity]),
        })
    }

    /// Removes a member identifier from `team_name`.
    pub fn remove_member(&self, team_name: &str, entity: &str) -> Option<TeamsPacket> {
        let mut g = self.teams.write();
        let entry = g.get_mut(team_name)?;
        if !entry.members.remove(entity) {
            return None;
        }
        Some(TeamsPacket {
            team_name: team_name.to_owned(),
            action: TeamsAction::RemoveEntities(vec![entity.to_owned()]),
        })
    }

    /// Removes a team completely and returns the [`TeamsPacket`] (`Remove`).
    pub fn remove_team(&self, team_name: &str) -> Option<TeamsPacket> {
        let mut g = self.teams.write();
        g.remove(team_name)?;
        Some(TeamsPacket {
            team_name: team_name.to_owned(),
            action: TeamsAction::Remove,
        })
    }

    /// Builds `Create` [`TeamsPacket`]s for all registered teams (sent to newly joining players).
    #[must_use]
    pub fn all_creation_packets(&self) -> Vec<TeamsPacket> {
        let g = self.teams.read();
        g.values()
            .map(|t| TeamsPacket {
                team_name: t.name.clone(),
                action: TeamsAction::Create {
                    settings: t.settings.clone(),
                    entities: t.members.iter().cloned().collect(),
                },
            })
            .collect()
    }
}

// ============================================================================
// BossBar (`net.minestom.server.adventure.bossbar`)
// ============================================================================

#[derive(Debug)]
struct BossBarInner {
    uuid: Uuid,
    title: Component,
    progress: f32,
    color: BossBarColor,
    overlay: BossBarOverlay,
    flags: u8,
    viewers: BTreeSet<PlayerId>,
}

/// Thread-safe top-of-screen [`BossBar`] with live viewer tracking.
#[derive(Debug, Clone)]
pub struct BossBar {
    inner: Arc<RwLock<BossBarInner>>,
}

impl BossBar {
    /// Creates a new [`BossBar`].
    #[must_use]
    pub fn new(
        title: impl Into<Component>,
        progress: f32,
        color: BossBarColor,
        overlay: BossBarOverlay,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(BossBarInner {
                uuid: Uuid::new_v4(),
                title: title.into(),
                progress: progress.clamp(0.0, 1.0),
                color,
                overlay,
                flags: 0,
                viewers: BTreeSet::new(),
            })),
        }
    }

    /// Unique [`Uuid`] of this boss bar.
    #[must_use]
    pub fn uuid(&self) -> Uuid {
        self.inner.read().uuid
    }

    /// Updates progress (`0.0..=1.0`) and returns `(viewers, BossBarPacket)`.
    pub fn set_progress(&self, progress: f32) -> (Vec<PlayerId>, BossBarPacket) {
        let mut g = self.inner.write();
        let clamped = progress.clamp(0.0, 1.0);
        g.progress = clamped;
        let pkt = BossBarPacket {
            uuid: g.uuid,
            action: BossBarAction::UpdateHealth(clamped),
        };
        (g.viewers.iter().copied().collect(), pkt)
    }

    /// Updates title and returns `(viewers, BossBarPacket)`.
    pub fn set_title(&self, title: impl Into<Component>) -> (Vec<PlayerId>, BossBarPacket) {
        let mut g = self.inner.write();
        let title = title.into();
        g.title = title.clone();
        let pkt = BossBarPacket {
            uuid: g.uuid,
            action: BossBarAction::UpdateTitle(title),
        };
        (g.viewers.iter().copied().collect(), pkt)
    }

    /// Updates color and overlay and returns `(viewers, BossBarPacket)`.
    pub fn set_style(
        &self,
        color: BossBarColor,
        overlay: BossBarOverlay,
    ) -> (Vec<PlayerId>, BossBarPacket) {
        let mut g = self.inner.write();
        g.color = color;
        g.overlay = overlay;
        let pkt = BossBarPacket {
            uuid: g.uuid,
            action: BossBarAction::UpdateStyle { color, overlay },
        };
        (g.viewers.iter().copied().collect(), pkt)
    }

    pub(crate) fn register_viewer(&self, player_id: PlayerId) -> BossBarPacket {
        let mut g = self.inner.write();
        g.viewers.insert(player_id);
        BossBarPacket {
            uuid: g.uuid,
            action: BossBarAction::Add {
                title: g.title.clone(),
                health: g.progress,
                color: g.color,
                overlay: g.overlay,
                flags: g.flags,
            },
        }
    }

    pub(crate) fn unregister_viewer(&self, player_id: PlayerId) -> Option<BossBarPacket> {
        let mut g = self.inner.write();
        g.viewers.remove(&player_id).then_some(BossBarPacket {
            uuid: g.uuid,
            action: BossBarAction::Remove,
        })
    }
}

// ============================================================================
// Titles, Particles, Sounds, Toasts, Recipes, ResourcePacks, and Dialogs
// ============================================================================

/// Fade-in, stay, and fade-out durations (in ticks) for a [`Title`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleTimes {
    /// Fade-in ticks (`10` default).
    pub fade_in: i32,
    /// Stay ticks (`70` default).
    pub stay: i32,
    /// Fade-out ticks (`20` default).
    pub fade_out: i32,
}

impl Default for TitleTimes {
    fn default() -> Self {
        Self {
            fade_in: 10,
            stay: 70,
            fade_out: 20,
        }
    }
}

/// Full-screen title + subtitle banner (`Title` in Adventure).
#[derive(Debug, Clone, PartialEq)]
pub struct Title {
    /// Main title text.
    pub title: Component,
    /// Subtitle text.
    pub subtitle: Component,
    /// Optional custom fade/stay timings.
    pub times: Option<TitleTimes>,
}

impl Title {
    /// Constructs a [`Title`] with default timings (`10` / `70` / `20` ticks).
    #[must_use]
    pub fn new(title: impl Into<Component>, subtitle: impl Into<Component>) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            times: Some(TitleTimes::default()),
        }
    }

    /// Overrides the fade/stay [`TitleTimes`].
    #[must_use]
    pub const fn with_times(mut self, times: TitleTimes) -> Self {
        self.times = Some(times);
        self
    }

    pub(crate) fn to_packets(
        &self,
    ) -> (
        Option<SetTitleTimePacket>,
        SetTitleSubTitlePacket,
        SetTitleTextPacket,
    ) {
        (
            self.times.map(|t| SetTitleTimePacket {
                fade_in: t.fade_in,
                stay: t.stay,
                fade_out: t.fade_out,
            }),
            SetTitleSubTitlePacket {
                subtitle: self.subtitle.clone(),
            },
            SetTitleTextPacket {
                title: self.title.clone(),
            },
        )
    }
}

/// Strongly typed `Copy` handle for all 125 Minecraft 26.2 `Particle` types (`minecraft:particle_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Particle(pub u32);

minestom_data::impl_particle_constants!(Particle);

impl Particle {
    /// Total number of vanilla particle types (`125` in MC 26.2).
    pub const COUNT: usize = particles::ENTRIES.len();

    /// Returns the protocol registry ID (`0..125`).
    #[inline]
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Returns the canonical `"minecraft:..."` resource key.
    #[inline]
    #[must_use]
    pub const fn key(self) -> &'static str {
        if (self.0 as usize) < particles::ENTRIES.len() {
            particles::ENTRIES[self.0 as usize].1
        } else {
            "minecraft:poof"
        }
    }

    /// Looks up a [`Particle`] by `minecraft:...` or unprefixed key in `O(log N)` with zero allocation.
    #[inline]
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        particles::lookup_key(key).map(Self)
    }

    /// Looks up a [`Particle`] by protocol ID.
    #[inline]
    #[must_use]
    pub const fn from_id(id: u32) -> Option<Self> {
        if (id as usize) < Self::COUNT {
            Some(Self(id))
        } else {
            None
        }
    }

    /// Builds a simple [`ParticlePacket`] at `position`.
    #[must_use]
    pub const fn packet(
        self,
        position: Vec3,
        offset: (f32, f32, f32),
        max_speed: f32,
        count: i32,
    ) -> ParticlePacket {
        ParticlePacket::simple(self.0, position, offset, max_speed, count)
    }

    /// Builds a `minecraft:block` [`ParticlePacket`] using `block`'s protocol state ID.
    #[must_use]
    pub const fn block_packet(
        block: Block,
        position: Vec3,
        offset: (f32, f32, f32),
        max_speed: f32,
        count: i32,
    ) -> ParticlePacket {
        ParticlePacket {
            particle_id: particles::BLOCK,
            data: ParticleData::BlockState(block.state_id()),
            override_limiter: false,
            long_distance: false,
            position,
            offset,
            max_speed,
            particle_count: count,
        }
    }

    /// Builds a `minecraft:item` [`ParticlePacket`] carrying `item`.
    #[must_use]
    pub fn item_packet(
        item: ItemStack,
        position: Vec3,
        offset: (f32, f32, f32),
        max_speed: f32,
        count: i32,
    ) -> ParticlePacket {
        ParticlePacket {
            particle_id: particles::ITEM,
            data: ParticleData::Item(item),
            override_limiter: false,
            long_distance: false,
            position,
            offset,
            max_speed,
            particle_count: count,
        }
    }

    /// Builds a colored redstone `minecraft:dust` [`ParticlePacket`].
    #[must_use]
    pub const fn dust_packet(
        rgb: i32,
        scale: f32,
        position: Vec3,
        offset: (f32, f32, f32),
        count: i32,
    ) -> ParticlePacket {
        ParticlePacket {
            particle_id: particles::DUST,
            data: ParticleData::RgbScale { rgb, scale },
            override_limiter: false,
            long_distance: false,
            position,
            offset,
            max_speed: 0.0,
            particle_count: count,
        }
    }
}

/// Ergonomic sound descriptor (`Sound` / [`SoundEvent`] in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    /// Underlying [`SoundHolder`].
    pub holder: SoundHolder,
    /// Mixer category.
    pub source: SoundSource,
    /// Volume (`1.0` normal).
    pub volume: f32,
    /// Pitch (`1.0` normal).
    pub pitch: f32,
    /// Random variation seed.
    pub seed: i64,
}

impl Sound {
    /// `minecraft:entity.experience_orb.pickup`.
    pub const EXPERIENCE_ORB_PICKUP: u32 = sounds::ENTITY_EXPERIENCE_ORB_PICKUP;
    /// `minecraft:entity.player.levelup`.
    pub const PLAYER_LEVELUP: u32 = sounds::ENTITY_PLAYER_LEVELUP;
    /// `minecraft:block.note_block.pling`.
    pub const NOTE_BLOCK_PLING: u32 = sounds::BLOCK_NOTE_BLOCK_PLING;
    /// `minecraft:entity.generic.explode`.
    pub const GENERIC_EXPLODE: u32 = sounds::ENTITY_GENERIC_EXPLODE;
    /// `minecraft:ui.button.click`.
    pub const UI_BUTTON_CLICK: u32 = sounds::UI_BUTTON_CLICK;

    /// Constructs a builtin vanilla [`Sound`] from any [`SoundEvent`] (all 1,968 constants on
    /// [`SoundEvent`]) or raw `u32` ID.
    #[must_use]
    pub fn builtin(
        sound: impl Into<SoundEvent>,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> Self {
        Self::event(sound.into(), source, volume, pitch)
    }

    /// `const` constructor from a typed [`SoundEvent`].
    #[must_use]
    pub const fn event(event: SoundEvent, source: SoundSource, volume: f32, pitch: f32) -> Self {
        Self {
            holder: SoundHolder::Builtin(event.0),
            source,
            volume,
            pitch,
            seed: 0,
        }
    }

    /// Constructs a custom resource-pack [`Sound`].
    #[must_use]
    pub fn custom(
        key: impl Into<String>,
        range: Option<f32>,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> Self {
        Self {
            holder: SoundHolder::Custom {
                key: key.into(),
                range,
            },
            source,
            volume,
            pitch,
            seed: 0,
        }
    }

    /// Encodes as a [`SoundEffectPacket`] at `origin`.
    #[must_use]
    pub fn to_packet_at(&self, origin: Vec3) -> SoundEffectPacket {
        SoundEffectPacket::at(
            self.holder.clone(),
            self.source,
            origin,
            self.volume,
            self.pitch,
            self.seed,
        )
    }
}

/// Top-right pop-up advancement toast notification (`Notification` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct ToastNotification {
    /// Toast title text.
    pub title: Component,
    /// Toast icon item stack.
    pub icon: ItemStack,
    /// Border frame style (`Task`, `Challenge`, `Goal`).
    pub frame_type: AdvancementFrameType,
}

impl ToastNotification {
    /// Constructs a new [`ToastNotification`].
    #[must_use]
    pub fn new(
        title: impl Into<Component>,
        icon: ItemStack,
        frame_type: AdvancementFrameType,
    ) -> Self {
        Self {
            title: title.into(),
            icon,
            frame_type,
        }
    }

    pub(crate) fn to_add_and_remove_packets(&self) -> (AdvancementsPacket, AdvancementsPacket) {
        let id = NEXT_TOAST_ID.fetch_add(1, Ordering::Relaxed);
        let key = format!("minestom:toast_{id}");
        let add = AdvancementsPacket {
            reset: false,
            advancements: vec![AdvancementEntry {
                key: key.clone(),
                parent: None,
                display: Some(AdvancementDisplayData {
                    title: self.title.clone(),
                    description: Component::empty(),
                    icon: self.icon.clone(),
                    frame_type: self.frame_type,
                    // 0x02 = show toast, 0x04 = hidden from advancement screen
                    flags: 0x02 | 0x04,
                    background_texture: None,
                    x: 0.0,
                    y: 0.0,
                }),
                requirements: vec![vec!["trigger".to_owned()]],
                send_telemetry: false,
            }],
            identifiers_to_remove: Vec::new(),
            progress: vec![AdvancementProgressEntry {
                key: key.clone(),
                criteria: vec![("trigger".to_owned(), Some(1))],
            }],
            show_advancements: false,
        };
        let remove = AdvancementsPacket {
            reset: false,
            advancements: Vec::new(),
            identifiers_to_remove: vec![key],
            progress: Vec::new(),
            show_advancements: false,
        };
        (add, remove)
    }
}

#[derive(Debug, Default)]
struct AdvancementTabInner {
    advancements: Vec<AdvancementEntry>,
    progress: Vec<AdvancementProgressEntry>,
}

/// Persistent advancement tab tree (`AdvancementTab` in Minestom).
#[derive(Debug, Clone, Default)]
pub struct AdvancementTab {
    inner: Arc<RwLock<AdvancementTabInner>>,
}

impl AdvancementTab {
    /// Creates an empty [`AdvancementTab`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an advancement node (and optional completed criteria) in this tab.
    pub fn add_advancement(
        &self,
        entry: AdvancementEntry,
        completed_criteria: Vec<(String, Option<i64>)>,
    ) {
        let mut g = self.inner.write();
        let key = entry.key.clone();
        g.advancements.push(entry);
        if !completed_criteria.is_empty() {
            g.progress.push(AdvancementProgressEntry {
                key,
                criteria: completed_criteria,
            });
        }
    }

    /// Builds the [`AdvancementsPacket`] synchronizing this tab to a player.
    #[must_use]
    pub fn to_packet(&self, reset: bool) -> AdvancementsPacket {
        let g = self.inner.read();
        AdvancementsPacket {
            reset,
            advancements: g.advancements.clone(),
            identifiers_to_remove: Vec::new(),
            progress: g.progress.clone(),
            show_advancements: true,
        }
    }
}

#[derive(Debug, Default)]
struct RecipeManagerInner {
    item_properties: BTreeMap<String, Vec<u32>>,
    stonecutter_recipes: Vec<StonecutterRecipeEntry>,
}

/// Thread-safe server recipe registry (`RecipeManager` in Minestom, MC 26.2 `DeclareRecipesPacket`).
#[derive(Debug, Clone, Default)]
pub struct RecipeManager {
    inner: Arc<RwLock<RecipeManagerInner>>,
}

impl RecipeManager {
    /// Creates an empty [`RecipeManager`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers accepted [`Material`]s for a typed [`RecipePropertySet`].
    pub fn set_property_set(&self, property_set: RecipePropertySet, materials: &[Material]) {
        self.set_property_items(property_set.as_str(), materials);
    }

    /// Registers accepted `Material`s for a `RecipeProperty` (e.g. `"furnace_input"`, `"smithing_base"`).
    pub fn set_property_items(
        &self,
        property_minimal_key: impl Into<String>,
        materials: &[Material],
    ) {
        let mut g = self.inner.write();
        g.item_properties.insert(
            property_minimal_key.into(),
            materials.iter().map(|m| m.id()).collect(),
        );
    }

    /// Adds a stonecutter recipe converting any of `inputs` into `result`.
    pub fn add_stonecutter_recipe(&self, inputs: &[Material], result: ItemStack) {
        let mut g = self.inner.write();
        g.stonecutter_recipes.push(StonecutterRecipeEntry {
            input_item_ids: inputs.iter().map(|m| m.id()).collect(),
            result,
        });
    }

    /// Returns `true` if any recipes or property sets are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let g = self.inner.read();
        g.item_properties.is_empty() && g.stonecutter_recipes.is_empty()
    }

    /// Builds the MC 26.2 [`DeclareRecipesPacket`].
    #[must_use]
    pub fn to_declare_packet(&self) -> DeclareRecipesPacket {
        let g = self.inner.read();
        DeclareRecipesPacket {
            item_properties: g.item_properties.clone(),
            stonecutter_recipes: g.stonecutter_recipes.clone(),
        }
    }
}

/// Outbound server resource pack push request (`ResourcePackRequest` in Adventure/Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct ResourcePackRequest {
    /// Unique pack UUID.
    pub id: Uuid,
    /// HTTPS download URL.
    pub url: String,
    /// 40-char SHA-1 hex digest.
    pub sha1_hex: String,
    /// Whether declining disconnects the player.
    pub forced: bool,
    /// Optional custom prompt text.
    pub prompt: Option<Component>,
}

impl ResourcePackRequest {
    /// Creates a new [`ResourcePackRequest`].
    #[must_use]
    pub fn new(
        id: Uuid,
        url: impl Into<String>,
        sha1_hex: impl Into<String>,
        forced: bool,
        prompt: Option<Component>,
    ) -> Self {
        Self {
            id,
            url: url.into(),
            sha1_hex: sha1_hex.into(),
            forced,
            prompt,
        }
    }

    pub(crate) fn to_packet(&self) -> ResourcePackPushPacket {
        ResourcePackPushPacket {
            id: self.id,
            url: self.url.clone(),
            hash: self.sha1_hex.clone(),
            forced: self.forced,
            prompt: self.prompt.clone(),
        }
    }
}

/// Fired when a client reports [`ResourcePackStatus`] (`PlayerResourcePackStatusEvent` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerResourcePackStatusEvent {
    /// Reporting [`PlayerId`].
    pub player_id: PlayerId,
    /// Resource pack UUID.
    pub pack_id: Uuid,
    /// Reported [`ResourcePackStatus`].
    pub status: ResourcePackStatus,
}

impl Event for PlayerResourcePackStatusEvent {}

impl PlayerEvent for PlayerResourcePackStatusEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

/// MC 26.2 modal `Dialog` builder (`net.minestom.server.dialog.Dialog`).
#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    /// Underlying [`DialogHolder`].
    pub holder: DialogHolder,
}

impl Dialog {
    /// References a pre-registered `minecraft:dialog` dynamic registry entry by index.
    #[must_use]
    pub const fn from_registry_id(id: u32) -> Self {
        Self {
            holder: DialogHolder::RegistryId(id),
        }
    }

    /// Builds an inline MC 26.2 `minecraft:notice` modal dialog with `title`, `body_text`, and `button_label`.
    #[must_use]
    pub fn notice(
        title: impl Into<Component>,
        body_text: impl Into<Component>,
        button_label: impl Into<Component>,
    ) -> Self {
        let mut root = NbtCompound::new();
        root.insert("type", NbtTag::String("minecraft:notice".to_owned()));
        root.insert("title", title.into().to_nbt());
        root.insert("can_close_with_escape", NbtTag::Byte(1));

        let mut body_entry = NbtCompound::new();
        body_entry.insert("type", NbtTag::String("minecraft:plain_message".to_owned()));
        body_entry.insert("contents", body_text.into().to_nbt());
        root.insert("body", NbtTag::List(vec![NbtTag::Compound(body_entry)]));

        let mut action_btn = NbtCompound::new();
        action_btn.insert("label", button_label.into().to_nbt());
        root.insert("action", NbtTag::Compound(action_btn));

        Self {
            holder: DialogHolder::Direct(NbtTag::Compound(root)),
        }
    }

    pub(crate) fn to_packet(&self) -> ShowDialogPacket {
        ShowDialogPacket {
            dialog: self.holder.clone(),
        }
    }
}
