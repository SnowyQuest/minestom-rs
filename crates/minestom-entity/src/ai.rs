//! Entity AI `GoalSelector`, `TargetSelector`, and `EntityAIGroup`
//! (`net.minestom.server.entity.ai` equivalent).

use minestom_block::BlockGetter;
use minestom_core::{BlockVec, PlayerId, Point, Pos};

use crate::attribute::Attribute;
use crate::pathfinding::{Navigator, is_standable};
use crate::{EntityBehavior, EntityCore, EntityType, LivingState};

/// Snapshot of a nearby living entity or player visible to [`TargetSelector`]s on the current tick.
#[derive(Debug, Clone, Copy)]
pub struct TargetCandidate {
    /// Target protocol entity ID.
    pub network_id: i32,
    /// Target [`PlayerId`] if this candidate is an online player.
    pub player_id: Option<PlayerId>,
    /// Target [`EntityType`].
    pub entity_type: EntityType,
    /// Current world position.
    pub position: Pos,
    /// Whether the candidate is dead (`health <= 0.0`).
    pub dead: bool,
}

/// Outgoing melee strike queued by an AI goal (e.g. [`MeleeAttackGoal`]) to be resolved by the
/// server tick loop into `EntityDamageEvent`, knockback, and combat packets.
#[derive(Debug, Clone, Copy)]
pub struct PendingMeleeHit {
    /// Attacked target's protocol entity ID.
    pub target_network_id: i32,
    /// Raw melee damage amount.
    pub damage: f32,
}

/// Mutable AI execution context passed into [`GoalSelector`] methods.
pub struct GoalContext<'a> {
    /// Owning creature's [`EntityCore`].
    pub core: &'a mut EntityCore,
    /// Owning creature's [`LivingState`].
    pub living: &'a mut LivingState,
    /// Instance block accessor for A* navigation.
    pub world: &'a dyn BlockGetter,
    /// Shared [`Navigator`] for this AI group.
    pub navigator: &'a mut Navigator,
    /// Currently selected target, if any.
    pub target: Option<TargetCandidate>,
}

/// Target acquisition strategy (`TargetSelector` in Minestom).
pub trait TargetSelector: Send + Sync + 'static {
    /// Evaluates nearby candidates and returns a target if one matches.
    fn find_target(&mut self, core: &EntityCore, living: &LivingState) -> Option<TargetCandidate>;
}

/// Selects the nearest alive entity (optionally restricted to `filter_type`, e.g. [`EntityType::PLAYER`])
/// within `range` blocks (`ClosestEntityTarget` in Minestom).
#[derive(Debug, Clone)]
pub struct ClosestEntityTarget {
    range_sq: f64,
    filter_type: Option<EntityType>,
}

impl ClosestEntityTarget {
    /// Targets any alive entity within `range` blocks, optionally filtered to `filter_type`.
    #[must_use]
    pub fn new(range: f64, filter_type: Option<EntityType>) -> Self {
        Self {
            range_sq: range * range,
            filter_type,
        }
    }

    /// Convenience constructor targeting the closest alive player within `range` blocks.
    #[must_use]
    pub fn players(range: f64) -> Self {
        Self::new(range, Some(EntityType::PLAYER))
    }
}

impl TargetSelector for ClosestEntityTarget {
    fn find_target(&mut self, core: &EntityCore, living: &LivingState) -> Option<TargetCandidate> {
        let mut best: Option<(f64, TargetCandidate)> = None;
        for &cand in &living.nearby_targets {
            if cand.dead || cand.network_id == core.network_id {
                continue;
            }
            if let Some(ft) = self.filter_type
                && cand.entity_type != ft
            {
                continue;
            }
            let d2 = core.position.distance_squared(&cand.position);
            if d2 <= self.range_sq && best.is_none_or(|(b_d2, _)| d2 < b_d2) {
                best = Some((d2, cand));
            }
        }
        best.map(|(_, c)| c)
    }
}

/// Retaliation target selector choosing `living.last_damager_network_id` if alive and within `range`
/// (`LastEntityDamagerTarget` in Minestom).
#[derive(Debug, Clone)]
pub struct LastEntityDamagerTarget {
    range_sq: f64,
}

impl LastEntityDamagerTarget {
    /// Constructs a [`LastEntityDamagerTarget`] with maximum pursuit `range` in blocks.
    #[must_use]
    pub fn new(range: f64) -> Self {
        Self {
            range_sq: range * range,
        }
    }
}

impl TargetSelector for LastEntityDamagerTarget {
    fn find_target(&mut self, core: &EntityCore, living: &LivingState) -> Option<TargetCandidate> {
        let damager_id = living.last_damager_network_id?;
        living.nearby_targets.iter().copied().find(|c| {
            c.network_id == damager_id
                && !c.dead
                && core.position.distance_squared(&c.position) <= self.range_sq
        })
    }
}

/// Autonomous behavior goal inside an [`EntityAIGroup`] (`GoalSelector` in Minestom).
pub trait GoalSelector: Send + Sync + 'static {
    /// Returns `true` when this goal wants to become active.
    fn should_start(&mut self, ctx: &GoalContext<'_>) -> bool;

    /// Invoked once when this goal becomes the active goal.
    fn start(&mut self, _ctx: &mut GoalContext<'_>) {}

    /// Invoked every server tick while active.
    fn tick(&mut self, ctx: &mut GoalContext<'_>);

    /// Returns `true` when this goal has completed or can no longer continue.
    fn should_end(&mut self, ctx: &GoalContext<'_>) -> bool;

    /// Invoked once when this goal stops running.
    fn end(&mut self, _ctx: &mut GoalContext<'_>) {}
}

/// Pursues the active target via A* pathfinding and strikes in melee range (`MeleeAttackGoal` in Minestom).
#[derive(Debug, Clone)]
pub struct MeleeAttackGoal {
    speed_blocks_per_tick: f64,
    attack_range_sq: f64,
    cooldown_ticks: u32,
    repath_interval_ticks: u32,
    cooldown_remaining: u32,
    ticks_since_repath: u32,
}

impl MeleeAttackGoal {
    /// Constructs a [`MeleeAttackGoal`].
    ///
    /// * `speed_blocks_per_tick`: horizontal chase speed (e.g. `0.23` for ~4.6 blocks/sec).
    /// * `attack_range`: melee reach in blocks (e.g. `2.0`).
    /// * `cooldown_ticks`: minimum ticks between strikes (e.g. `20` = 1 second).
    #[must_use]
    pub fn new(speed_blocks_per_tick: f64, attack_range: f64, cooldown_ticks: u32) -> Self {
        Self {
            speed_blocks_per_tick,
            attack_range_sq: attack_range * attack_range,
            cooldown_ticks: cooldown_ticks.max(1),
            repath_interval_ticks: 10,
            cooldown_remaining: 0,
            ticks_since_repath: 0,
        }
    }
}

impl GoalSelector for MeleeAttackGoal {
    fn should_start(&mut self, ctx: &GoalContext<'_>) -> bool {
        ctx.target.is_some_and(|t| !t.dead)
    }

    fn start(&mut self, ctx: &mut GoalContext<'_>) {
        self.ticks_since_repath = self.repath_interval_ticks;
        if let Some(target) = ctx.target {
            let goal = BlockVec::new(
                target.position.x.floor() as i32,
                target.position.y.round() as i32,
                target.position.z.floor() as i32,
            );
            ctx.navigator
                .set_path_to(ctx.world, ctx.core.position, goal);
        }
    }

    fn tick(&mut self, ctx: &mut GoalContext<'_>) {
        self.cooldown_remaining = self.cooldown_remaining.saturating_sub(1);
        let Some(target) = ctx.target else {
            ctx.navigator.reset(ctx.core);
            return;
        };

        self.ticks_since_repath += 1;
        if self.ticks_since_repath >= self.repath_interval_ticks || !ctx.navigator.is_navigating() {
            self.ticks_since_repath = 0;
            let goal = BlockVec::new(
                target.position.x.floor() as i32,
                target.position.y.round() as i32,
                target.position.z.floor() as i32,
            );
            ctx.navigator
                .set_path_to(ctx.world, ctx.core.position, goal);
        }

        let dist_sq = ctx.core.position.distance_squared(&target.position);
        if dist_sq > self.attack_range_sq * 0.64 {
            ctx.navigator.advance(ctx.core, self.speed_blocks_per_tick);
        } else {
            ctx.core.velocity.x = 0.0;
            ctx.core.velocity.z = 0.0;
            let dx = target.position.x - ctx.core.position.x;
            let dz = target.position.z - ctx.core.position.z;
            #[allow(clippy::cast_possible_truncation)]
            let yaw = (-dx).atan2(dz).to_degrees() as f32;
            ctx.core.position.yaw = yaw;
            ctx.core.head_yaw = yaw;
        }

        if dist_sq <= self.attack_range_sq && self.cooldown_remaining == 0 {
            self.cooldown_remaining = self.cooldown_ticks;
            #[allow(clippy::cast_possible_truncation)]
            let dmg = ctx.living.attributes.get_value(Attribute::ATTACK_DAMAGE) as f32;
            ctx.living.pending_melee_hits.push(PendingMeleeHit {
                target_network_id: target.network_id,
                damage: dmg.max(0.5),
            });
        }
    }

    fn should_end(&mut self, ctx: &GoalContext<'_>) -> bool {
        ctx.target.is_none_or(|t| t.dead)
    }

    fn end(&mut self, ctx: &mut GoalContext<'_>) {
        ctx.navigator.reset(ctx.core);
    }
}

/// Follows the selected target while keeping at least `stop_distance` blocks away (`FollowTargetGoal` in Minestom).
#[derive(Debug, Clone)]
pub struct FollowTargetGoal {
    speed_blocks_per_tick: f64,
    stop_dist_sq: f64,
    repath_interval_ticks: u32,
    ticks: u32,
}

impl FollowTargetGoal {
    /// Constructs a [`FollowTargetGoal`].
    #[must_use]
    pub fn new(speed_blocks_per_tick: f64, stop_distance: f64) -> Self {
        Self {
            speed_blocks_per_tick,
            stop_dist_sq: stop_distance * stop_distance,
            repath_interval_ticks: 12,
            ticks: 0,
        }
    }
}

impl GoalSelector for FollowTargetGoal {
    fn should_start(&mut self, ctx: &GoalContext<'_>) -> bool {
        ctx.target.is_some_and(|t| {
            !t.dead && ctx.core.position.distance_squared(&t.position) > self.stop_dist_sq
        })
    }

    fn tick(&mut self, ctx: &mut GoalContext<'_>) {
        let Some(target) = ctx.target else {
            return;
        };
        if ctx.core.position.distance_squared(&target.position) <= self.stop_dist_sq {
            ctx.navigator.reset(ctx.core);
            return;
        }
        self.ticks += 1;
        if self.ticks >= self.repath_interval_ticks || !ctx.navigator.is_navigating() {
            self.ticks = 0;
            let goal = BlockVec::new(
                target.position.x.floor() as i32,
                target.position.y.round() as i32,
                target.position.z.floor() as i32,
            );
            ctx.navigator
                .set_path_to(ctx.world, ctx.core.position, goal);
        }
        ctx.navigator.advance(ctx.core, self.speed_blocks_per_tick);
    }

    fn should_end(&mut self, ctx: &GoalContext<'_>) -> bool {
        ctx.target.is_none_or(|t| {
            t.dead || ctx.core.position.distance_squared(&t.position) <= self.stop_dist_sq
        })
    }

    fn end(&mut self, ctx: &mut GoalContext<'_>) {
        ctx.navigator.reset(ctx.core);
    }
}

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Periodically picks a random standable block within `radius` and walks to it (`RandomStrollGoal` in Minestom).
#[derive(Debug, Clone)]
pub struct RandomStrollGoal {
    radius: i32,
    speed_blocks_per_tick: f64,
    interval_ticks: u64,
}

impl RandomStrollGoal {
    /// Constructs a [`RandomStrollGoal`].
    #[must_use]
    pub fn new(radius: i32, speed_blocks_per_tick: f64, interval_ticks: u64) -> Self {
        Self {
            radius: radius.max(1),
            speed_blocks_per_tick,
            interval_ticks: interval_ticks.max(10),
        }
    }
}

impl GoalSelector for RandomStrollGoal {
    fn should_start(&mut self, ctx: &GoalContext<'_>) -> bool {
        ctx.target.is_none()
            && ctx.core.alive_ticks > 0
            && ctx.core.alive_ticks.is_multiple_of(self.interval_ticks)
    }

    fn start(&mut self, ctx: &mut GoalContext<'_>) {
        let seed = (ctx.core.uuid.as_u128() as u64) ^ ctx.core.alive_ticks;
        let base_x = ctx.core.position.x.floor() as i32;
        let base_y = ctx.core.position.y.round() as i32;
        let base_z = ctx.core.position.z.floor() as i32;
        let span = (self.radius * 2 + 1) as u64;
        for attempt in 0..8_u64 {
            let r1 = splitmix64(seed.wrapping_add(attempt * 2));
            let r2 = splitmix64(seed.wrapping_add(attempt * 2 + 1));
            let dx = (r1 % span) as i32 - self.radius;
            let dz = (r2 % span) as i32 - self.radius;
            if dx == 0 && dz == 0 {
                continue;
            }
            for dy in [0, 1, -1] {
                let cand = BlockVec::new(base_x + dx, base_y + dy, base_z + dz);
                if is_standable(ctx.world, cand, 2)
                    && ctx
                        .navigator
                        .set_path_to(ctx.world, ctx.core.position, cand)
                {
                    return;
                }
            }
        }
    }

    fn tick(&mut self, ctx: &mut GoalContext<'_>) {
        ctx.navigator.advance(ctx.core, self.speed_blocks_per_tick);
    }

    fn should_end(&mut self, ctx: &GoalContext<'_>) -> bool {
        ctx.target.is_some() || !ctx.navigator.is_navigating()
    }

    fn end(&mut self, ctx: &mut GoalContext<'_>) {
        ctx.navigator.reset(ctx.core);
    }
}

/// Periodically glances toward a random yaw angle while idle (`RandomLookAroundGoal` in Minestom).
#[derive(Debug, Clone)]
pub struct RandomLookAroundGoal {
    interval_ticks: u64,
    remaining_ticks: u32,
    target_yaw: f32,
}

impl RandomLookAroundGoal {
    /// Constructs a [`RandomLookAroundGoal`].
    #[must_use]
    pub const fn new(interval_ticks: u64) -> Self {
        Self {
            interval_ticks: if interval_ticks < 10 {
                10
            } else {
                interval_ticks
            },
            remaining_ticks: 0,
            target_yaw: 0.0,
        }
    }
}

impl GoalSelector for RandomLookAroundGoal {
    fn should_start(&mut self, ctx: &GoalContext<'_>) -> bool {
        ctx.target.is_none()
            && !ctx.navigator.is_navigating()
            && ctx.core.alive_ticks > 0
            && ctx.core.alive_ticks.is_multiple_of(self.interval_ticks)
    }

    fn start(&mut self, ctx: &mut GoalContext<'_>) {
        let r = splitmix64((ctx.core.uuid.as_u128() as u64) ^ ctx.core.alive_ticks);
        self.target_yaw = ((r % 360) as f32) - 180.0;
        self.remaining_ticks = 15;
    }

    fn tick(&mut self, ctx: &mut GoalContext<'_>) {
        self.remaining_ticks = self.remaining_ticks.saturating_sub(1);
        ctx.core.position.yaw = self.target_yaw;
        ctx.core.head_yaw = self.target_yaw;
    }

    fn should_end(&mut self, ctx: &GoalContext<'_>) -> bool {
        ctx.target.is_some() || self.remaining_ticks == 0
    }
}

/// Priority-ordered AI group combining [`TargetSelector`]s, [`GoalSelector`]s, and a [`Navigator`]
/// (`EntityAIGroup` in Minestom). Implements [`EntityBehavior`] so it attaches directly via
/// [`crate::Entity::with_behavior`].
#[derive(Default)]
pub struct EntityAIGroup {
    target_selectors: Vec<Box<dyn TargetSelector>>,
    goal_selectors: Vec<Box<dyn GoalSelector>>,
    active_goal: Option<usize>,
    navigator: Navigator,
}

impl EntityAIGroup {
    /// Creates an empty [`EntityAIGroup`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            target_selectors: Vec::new(),
            goal_selectors: Vec::new(),
            active_goal: None,
            navigator: Navigator::new(),
        }
    }

    /// Appends a [`TargetSelector`] in priority order (first match wins).
    #[must_use]
    pub fn with_target_selector(mut self, selector: impl TargetSelector) -> Self {
        self.target_selectors.push(Box::new(selector));
        self
    }

    /// Appends a [`GoalSelector`] in priority order (earlier goals preempt later goals).
    #[must_use]
    pub fn with_goal_selector(mut self, goal: impl GoalSelector) -> Self {
        self.goal_selectors.push(Box::new(goal));
        self
    }
}

impl EntityBehavior for EntityAIGroup {
    fn tick(
        &mut self,
        core: &mut EntityCore,
        living: Option<&mut LivingState>,
        world: &dyn BlockGetter,
    ) {
        let Some(living) = living else {
            return;
        };
        if living.dead {
            self.navigator.reset(core);
            return;
        }

        // 1. Acquire highest-priority target
        let mut selected_target = None;
        for ts in &mut self.target_selectors {
            if let Some(cand) = ts.find_target(core, living) {
                selected_target = Some(cand);
                break;
            }
        }

        let mut ctx = GoalContext {
            core,
            living,
            world,
            navigator: &mut self.navigator,
            target: selected_target,
        };

        // 2. Check if active goal should end or be preempted by a higher-priority goal
        if let Some(active_idx) = self.active_goal {
            if self.goal_selectors[active_idx].should_end(&ctx) {
                self.goal_selectors[active_idx].end(&mut ctx);
                self.active_goal = None;
            } else {
                for higher_idx in 0..active_idx {
                    if self.goal_selectors[higher_idx].should_start(&ctx) {
                        self.goal_selectors[active_idx].end(&mut ctx);
                        self.goal_selectors[higher_idx].start(&mut ctx);
                        self.active_goal = Some(higher_idx);
                        break;
                    }
                }
            }
        }

        // 3. If no goal is active, pick the first ready goal
        if self.active_goal.is_none() {
            for (idx, goal) in self.goal_selectors.iter_mut().enumerate() {
                if goal.should_start(&ctx) {
                    goal.start(&mut ctx);
                    self.active_goal = Some(idx);
                    break;
                }
            }
        }

        // 4. Tick active goal
        if let Some(active_idx) = self.active_goal {
            self.goal_selectors[active_idx].tick(&mut ctx);
        }
    }
}
