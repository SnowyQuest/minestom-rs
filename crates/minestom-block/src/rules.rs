//! Vanilla and custom [`BlockPlacementRule`] implementations (`net.minestom.server.instance.block.rule`).

use std::sync::Arc;

use minestom_core::{BlockFace, BlockVec, Pos, Vec3};
use minestom_data::generated::block_states::BLOCK_INFOS;

use crate::{Block, BlockGetter, BlockManager};

/// Input parameters passed to [`BlockPlacementRule::block_place`].
pub struct PlacementState<'a> {
    /// Read-only world block accessor.
    pub world: &'a dyn BlockGetter,
    /// Candidate default block being placed.
    pub block: Block,
    /// Face of the supporting block that was clicked.
    pub block_face: BlockFace,
    /// Target block position where the block will be placed.
    pub place_position: BlockVec,
    /// Sub-block cursor click offset (`0.0..=1.0`).
    pub cursor_position: Vec3,
    /// Placing player's position and look angles.
    pub player_position: Pos,
    /// Whether the player is holding sneak.
    pub is_player_shifting: bool,
}

/// Input parameters passed to [`BlockPlacementRule::is_self_replaceable`]
/// (e.g. merging a top slab onto a bottom slab into `type=double`, or stacking snow layers).
pub struct ReplacementContext<'a> {
    /// Read-only world block accessor.
    pub world: &'a dyn BlockGetter,
    /// Existing block at the clicked position.
    pub current_block: Block,
    /// Block item being placed by the player.
    pub placing_block: Block,
    /// Clicked face on `current_block`.
    pub block_face: BlockFace,
    /// Sub-block cursor click offset (`0.0..=1.0`).
    pub cursor_position: Vec3,
}

/// Input parameters passed to [`BlockPlacementRule::block_update`].
pub struct UpdateState<'a> {
    /// Read-only world block accessor.
    pub world: &'a dyn BlockGetter,
    /// Position of the block receiving a neighbor update.
    pub block_position: BlockVec,
    /// Current state of this block.
    pub current_block: Block,
    /// Face toward the neighbor that changed.
    pub from_face: BlockFace,
}

/// Custom state computation rule when placing or neighbor-updating a block type
/// (`BlockPlacementRule` in Minestom).
pub trait BlockPlacementRule: Send + Sync + 'static {
    /// Computes the placed [`Block`] state, or returns `None` to disallow placement.
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block>;

    /// Computes an updated [`Block`] state when an adjacent neighbor changes.
    fn block_update(&self, state: &UpdateState<'_>) -> Block {
        state.current_block
    }

    /// Returns `true` if clicking `replacement.current_block` should merge/replace in-place
    /// instead of offsetting to the adjacent block (`Slab`, `Snow`, `Candle`, `SeaPickle`).
    fn is_self_replaceable(&self, _replacement: &ReplacementContext<'_>) -> bool {
        false
    }

    /// Maximum Manhattan propagation depth for chained neighbor updates.
    fn max_update_distance(&self) -> usize {
        10
    }
}

/// Applies `waterlogged=true` if `block` supports `"waterlogged"` and the displaced block at
/// `place_pos` is source `minecraft:water`.
#[inline]
#[must_use]
pub fn apply_waterlogged(world: &dyn BlockGetter, place_pos: BlockVec, block: Block) -> Block {
    if !block.has_property("waterlogged") {
        return block;
    }
    let existing = world.get_block(place_pos);
    let in_water = existing.same_block(Block::WATER) && existing.get_property("level") == Some("0");
    block
        .with_property("waterlogged", if in_water { "true" } else { "false" })
        .unwrap_or(block)
}

/// Pillar `axis=x|y|z` placement rule (logs, wood, stems, quartz/purpur pillars, chains, hay, basalt).
pub struct AxisPlacementRule;

impl BlockPlacementRule for AxisPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let placed = state
            .block
            .with_property("axis", state.block_face.axis_str())
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }
}

/// Horizontal `facing` toward the placing player (chests, furnaces, smokers, blast furnaces,
/// lecterns, looms, stonecutters, carved pumpkins, beehives, ender chests).
pub struct HorizontalFacingPlacementRule;

impl BlockPlacementRule for HorizontalFacingPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let facing = BlockFace::from_horizontal_yaw(state.player_position.yaw).opposite();
        let placed = state
            .block
            .with_property("facing", facing.as_str())
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }
}

/// Horizontal `facing` away from the placing player (fence gates, glazed terracotta, calibrated sculk sensors).
pub struct AwayFacingPlacementRule;

impl BlockPlacementRule for AwayFacingPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let facing = BlockFace::from_horizontal_yaw(state.player_position.yaw);
        let placed = state
            .block
            .with_property("facing", facing.as_str())
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }
}

/// Anvil placement rule (`facing` rotated 90° clockwise from player look direction).
pub struct AnvilPlacementRule;

impl BlockPlacementRule for AnvilPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let facing = BlockFace::from_horizontal_yaw(state.player_position.yaw).rotate_y_cw();
        Some(
            state
                .block
                .with_property("facing", facing.as_str())
                .unwrap_or(state.block),
        )
    }
}

/// 6-way `facing=down|up|north|south|west|east` rule (dispensers, droppers, pistons, barrels,
/// observers, end rods, lightning rods, amethyst clusters).
pub struct Directional6PlacementRule {
    /// If `true`, faces the clicked `block_face` directly (end rod, lightning rod, amethyst).
    /// Otherwise faces toward (`invert = true`: pistons/dispensers/barrels) or away (`invert = false`: observer) from player.
    pub use_clicked_face: bool,
    /// Invert look direction so front faces player.
    pub face_player: bool,
}

impl BlockPlacementRule for Directional6PlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let dir = if self.use_clicked_face {
            state.block_face
        } else {
            let look = if state.player_position.pitch > 60.0 {
                BlockFace::Down
            } else if state.player_position.pitch < -60.0 {
                BlockFace::Up
            } else {
                BlockFace::from_horizontal_yaw(state.player_position.yaw)
            };
            if self.face_player {
                look.opposite()
            } else {
                look
            }
        };
        let placed = state
            .block
            .with_property("facing", dir.as_str())
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }
}

/// Vanilla slab rule (`type=bottom|top|double` + `waterlogged`), including merging two half-slabs
/// of the same block into `type=double`!
pub struct SlabPlacementRule;

impl BlockPlacementRule for SlabPlacementRule {
    fn is_self_replaceable(&self, rep: &ReplacementContext<'_>) -> bool {
        if !rep.current_block.same_block(rep.placing_block) {
            return false;
        }
        match rep.current_block.get_property("type") {
            Some("bottom") => {
                rep.block_face == BlockFace::Up
                    || (rep.block_face.is_horizontal() && rep.cursor_position.y > 0.5)
            }
            Some("top") => {
                rep.block_face == BlockFace::Down
                    || (rep.block_face.is_horizontal() && rep.cursor_position.y <= 0.5)
            }
            _ => false,
        }
    }

    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let existing = state.world.get_block(state.place_position);
        if existing.same_block(state.block) && existing.get_property("type") != Some("double") {
            let merged = state
                .block
                .with_property("type", "double")
                .unwrap_or(state.block);
            return Some(
                merged
                    .with_property("waterlogged", "false")
                    .unwrap_or(merged),
            );
        }
        let slab_type = match state.block_face {
            BlockFace::Down => "top",
            BlockFace::Up => "bottom",
            _ => {
                if state.cursor_position.y > 0.5 {
                    "top"
                } else {
                    "bottom"
                }
            }
        };
        let placed = state
            .block
            .with_property("type", slab_type)
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }
}

fn is_stairs_block(block: Block) -> bool {
    block.has_property("shape") && block.has_property("half") && block.has_property("facing")
}

fn can_take_stair_shape(
    world: &dyn BlockGetter,
    pos: BlockVec,
    facing: BlockFace,
    half: &str,
    check_dir: BlockFace,
) -> bool {
    let other = world.get_block(pos.relative(check_dir));
    !is_stairs_block(other)
        || other.get_property("facing") != Some(facing.as_str())
        || other.get_property("half") != Some(half)
}

fn compute_stair_shape(
    world: &dyn BlockGetter,
    pos: BlockVec,
    facing: BlockFace,
    half: &str,
) -> &'static str {
    // 1. Check front neighbor (`pos + facing`) for outer corners
    let front = world.get_block(pos.relative(facing));
    if is_stairs_block(front)
        && front.get_property("half") == Some(half)
        && let Some(f_str) = front.get_property("facing")
        && let Ok(front_facing) = f_str.parse::<BlockFace>()
        && front_facing.axis_str() != facing.axis_str()
        && can_take_stair_shape(world, pos, facing, half, front_facing.opposite())
    {
        return if front_facing == facing.rotate_y_ccw() {
            "outer_left"
        } else {
            "outer_right"
        };
    }

    // 2. Check back neighbor (`pos + facing.opposite()`) for inner corners
    let back = world.get_block(pos.relative(facing.opposite()));
    if is_stairs_block(back)
        && back.get_property("half") == Some(half)
        && let Some(b_str) = back.get_property("facing")
        && let Ok(back_facing) = b_str.parse::<BlockFace>()
        && back_facing.axis_str() != facing.axis_str()
        && can_take_stair_shape(world, pos, facing, half, back_facing)
    {
        return if back_facing == facing.rotate_y_ccw() {
            "inner_left"
        } else {
            "inner_right"
        };
    }

    "straight"
}

/// Vanilla stairs rule (`facing`, `half=bottom|top`, `shape=straight|inner_*|outer_*`, `waterlogged`).
pub struct StairsPlacementRule;

impl BlockPlacementRule for StairsPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let facing = BlockFace::from_horizontal_yaw(state.player_position.yaw);
        let half = match state.block_face {
            BlockFace::Down => "top",
            BlockFace::Up => "bottom",
            _ if state.cursor_position.y > 0.5 => "top",
            _ => "bottom",
        };
        let shape = compute_stair_shape(state.world, state.place_position, facing, half);
        let placed = state
            .block
            .with_properties([
                ("facing", facing.as_str()),
                ("half", half),
                ("shape", shape),
            ])
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }

    fn block_update(&self, state: &UpdateState<'_>) -> Block {
        if state.from_face.is_horizontal()
            && let Some(f_str) = state.current_block.get_property("facing")
            && let Ok(facing) = f_str.parse::<BlockFace>()
            && let Some(half) = state.current_block.get_property("half")
        {
            let shape = compute_stair_shape(state.world, state.block_position, facing, half);
            return state
                .current_block
                .with_property("shape", shape)
                .unwrap_or(state.current_block);
        }
        state.current_block
    }
}

/// Vanilla trapdoor rule (`facing`, `half=bottom|top`, `waterlogged`).
pub struct TrapdoorPlacementRule;

impl BlockPlacementRule for TrapdoorPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let (facing, half) = if state.block_face.is_horizontal() {
            (
                state.block_face,
                if state.cursor_position.y > 0.5 {
                    "top"
                } else {
                    "bottom"
                },
            )
        } else {
            (
                BlockFace::from_horizontal_yaw(state.player_position.yaw).opposite(),
                if state.block_face == BlockFace::Down {
                    "top"
                } else {
                    "bottom"
                },
            )
        };
        let placed = state
            .block
            .with_properties([("facing", facing.as_str()), ("half", half)])
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }
}

/// Vanilla 2-block door placement rule (`facing`, `half=lower`, `hinge=left|right`).
pub struct DoorPlacementRule;

impl BlockPlacementRule for DoorPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let upper_pos = state.place_position.offset(0, 1, 0);
        if !state.world.get_block(upper_pos).is_replaceable() {
            return None;
        }
        let facing = BlockFace::from_horizontal_yaw(state.player_position.yaw);
        let left_neighbor = state
            .world
            .get_block(state.place_position.relative(facing.rotate_y_ccw()));
        let right_neighbor = state
            .world
            .get_block(state.place_position.relative(facing.rotate_y_cw()));

        let hinge = if left_neighbor.same_block(state.block) {
            "right"
        } else if right_neighbor.same_block(state.block) {
            "left"
        } else {
            let cursor_right = match facing {
                BlockFace::North => state.cursor_position.x < 0.5,
                BlockFace::South => state.cursor_position.x > 0.5,
                BlockFace::West => state.cursor_position.z > 0.5,
                _ => state.cursor_position.z < 0.5,
            };
            if cursor_right { "right" } else { "left" }
        };

        state
            .block
            .with_properties([
                ("facing", facing.as_str()),
                ("half", "lower"),
                ("hinge", hinge),
            ])
            .ok()
    }
}

fn connects_fence_or_pane(self_block: Block, neighbor: Block, toward_neighbor: BlockFace) -> bool {
    if neighbor.is_air() {
        return false;
    }
    if neighbor.same_block(self_block) {
        return true;
    }
    let self_is_pane = self_block.key().ends_with("_pane") || self_block.key().ends_with("_bars");
    let neighbor_is_pane = neighbor.key().ends_with("_pane") || neighbor.key().ends_with("_bars");
    if self_is_pane && neighbor_is_pane {
        return true;
    }
    let self_is_wooden_fence =
        self_block.key().ends_with("_fence") && self_block != Block::NETHER_BRICK_FENCE;
    let neighbor_is_wooden_fence =
        neighbor.key().ends_with("_fence") && neighbor != Block::NETHER_BRICK_FENCE;
    if self_is_wooden_fence && neighbor_is_wooden_fence {
        return true;
    }
    if neighbor.key().ends_with("_fence_gate")
        && let Some(f_str) = neighbor.get_property("facing")
        && let Ok(gate_facing) = f_str.parse::<BlockFace>()
    {
        return gate_facing.axis_str() != toward_neighbor.axis_str();
    }
    neighbor.is_solid() && neighbor.occludes()
}

/// Vanilla 4-way horizontal connectivity rule (`north`/`south`/`east`/`west` booleans)
/// for fences, glass panes, and iron/copper bars.
pub struct FenceConnectPlacementRule;

impl FenceConnectPlacementRule {
    fn resolve(world: &dyn BlockGetter, pos: BlockVec, mut block: Block) -> Block {
        for face in BlockFace::HORIZONTALS {
            let neighbor = world.get_block(pos.relative(face));
            let connected = connects_fence_or_pane(block, neighbor, face);
            block = block
                .with_property(face.as_str(), if connected { "true" } else { "false" })
                .unwrap_or(block);
        }
        block
    }
}

impl BlockPlacementRule for FenceConnectPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let connected = Self::resolve(state.world, state.place_position, state.block);
        Some(apply_waterlogged(
            state.world,
            state.place_position,
            connected,
        ))
    }

    fn block_update(&self, state: &UpdateState<'_>) -> Block {
        if state.from_face.is_horizontal() {
            Self::resolve(state.world, state.block_position, state.current_block)
        } else {
            state.current_block
        }
    }
}

/// Vanilla wall connectivity rule (`north`/`south`/`east`/`west` = `none|low|tall` + `up`).
pub struct WallConnectPlacementRule;

impl WallConnectPlacementRule {
    fn resolve(world: &dyn BlockGetter, pos: BlockVec, mut block: Block) -> Block {
        let above = world.get_block(pos.offset(0, 1, 0));
        let above_solid = !above.is_air() && above.is_solid();
        let mut conns = [false; 4];
        for (idx, face) in BlockFace::HORIZONTALS.iter().enumerate() {
            let neighbor = world.get_block(pos.relative(*face));
            let conn = !neighbor.is_air()
                && (neighbor.key().ends_with("_wall")
                    || (neighbor.is_solid() && neighbor.occludes())
                    || neighbor.key().ends_with("_pane")
                    || neighbor.key().ends_with("_bars"));
            conns[idx] = conn;
            let side_val = if !conn {
                "none"
            } else if above_solid {
                "tall"
            } else {
                "low"
            };
            block = block
                .with_property(face.as_str(), side_val)
                .unwrap_or(block);
        }
        // Straight N-S or W-E walls without a block above omit the center post (`up=false`).
        let straight_ns = conns[0] && conns[1] && !conns[2] && !conns[3];
        let straight_we = !conns[0] && !conns[1] && conns[2] && conns[3];
        let up = above.key().ends_with("_wall") || !(straight_ns || straight_we);
        block
            .with_property("up", if up { "true" } else { "false" })
            .unwrap_or(block)
    }
}

impl BlockPlacementRule for WallConnectPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let connected = Self::resolve(state.world, state.place_position, state.block);
        Some(apply_waterlogged(
            state.world,
            state.place_position,
            connected,
        ))
    }

    fn block_update(&self, state: &UpdateState<'_>) -> Block {
        Self::resolve(state.world, state.block_position, state.current_block)
    }
}

/// Vanilla `face=floor|wall|ceiling` + `facing` rule (buttons, levers, grindstones).
pub struct FaceAttachedPlacementRule;

impl BlockPlacementRule for FaceAttachedPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let (face, facing) = match state.block_face {
            BlockFace::Up => (
                "floor",
                BlockFace::from_horizontal_yaw(state.player_position.yaw),
            ),
            BlockFace::Down => (
                "ceiling",
                BlockFace::from_horizontal_yaw(state.player_position.yaw),
            ),
            wall_face => ("wall", wall_face),
        };
        Some(
            state
                .block
                .with_properties([("face", face), ("facing", facing.as_str())])
                .unwrap_or(state.block),
        )
    }
}

/// Vanilla 16-angle `rotation=0..15` rule (standing signs, standing banners, floor skulls)
/// with automatic redirect to `wall_block` (`*_wall_sign`, `*_wall_banner`, `*_wall_skull`, `wall_torch`)
/// when placed on a horizontal wall face!
pub struct Rotation16PlacementRule {
    /// Optional wall-mounted counterpart block when placed against a horizontal face.
    pub wall_counterpart: Option<Block>,
}

impl BlockPlacementRule for Rotation16PlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        if state.block_face.is_horizontal()
            && let Some(wall_blk) = self.wall_counterpart
        {
            let placed = wall_blk
                .with_property("facing", state.block_face.as_str())
                .unwrap_or(wall_blk);
            return Some(apply_waterlogged(state.world, state.place_position, placed));
        }
        if state.block_face == BlockFace::Down && self.wall_counterpart.is_some() {
            return None;
        }
        let rot = (((f64::from(state.player_position.yaw) + 180.0) * 16.0 / 360.0 + 0.5).floor()
            as i32)
            .rem_euclid(16);
        let rot_s = rot.to_string();
        let placed = state
            .block
            .with_property("rotation", &rot_s)
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }
}

/// Vanilla lantern placement rule (`hanging=true` on `BlockFace::Down`, `waterlogged`).
pub struct LanternPlacementRule;

impl BlockPlacementRule for LanternPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        if state.block_face.is_horizontal() {
            return None;
        }
        let hanging = state.block_face == BlockFace::Down;
        let placed = state
            .block
            .with_property("hanging", if hanging { "true" } else { "false" })
            .unwrap_or(state.block);
        Some(apply_waterlogged(state.world, state.place_position, placed))
    }
}

/// Vanilla snow layer stacking rule (`layers=1..=8`).
pub struct SnowLayerPlacementRule;

impl BlockPlacementRule for SnowLayerPlacementRule {
    fn is_self_replaceable(&self, rep: &ReplacementContext<'_>) -> bool {
        if !rep.current_block.same_block(Block::SNOW) {
            return false;
        }
        let layers = rep
            .current_block
            .get_property("layers")
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(1);
        layers < 8 && (rep.block_face == BlockFace::Up || rep.placing_block.same_block(Block::SNOW))
    }

    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let existing = state.world.get_block(state.place_position);
        if existing.same_block(Block::SNOW) {
            let layers = existing
                .get_property("layers")
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(1);
            let next = (layers + 1).min(8).to_string();
            return state.block.with_property("layers", &next).ok();
        }
        Some(state.block)
    }
}

/// Generic waterlogging rule for blocks whose only placement-time property is `waterlogged`.
pub struct WaterloggedOnlyPlacementRule;

impl BlockPlacementRule for WaterloggedOnlyPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        Some(apply_waterlogged(
            state.world,
            state.place_position,
            state.block,
        ))
    }
}

fn find_wall_counterpart(key: &str) -> Option<Block> {
    let bare = key.strip_prefix("minecraft:").unwrap_or(key);
    if bare == "torch" {
        return Some(Block::WALL_TORCH);
    }
    if bare == "soul_torch" {
        return Some(Block::SOUL_WALL_TORCH);
    }
    if bare == "copper_torch" {
        return Block::from_key("minecraft:copper_wall_torch");
    }
    if bare == "redstone_torch" {
        return Some(Block::REDSTONE_WALL_TORCH);
    }
    if let Some(stem) = bare.strip_suffix("_sign")
        && !stem.ends_with("_hanging")
        && !stem.ends_with("_wall")
    {
        return Block::from_key(&format!("minecraft:{stem}_wall_sign"));
    }
    if let Some(stem) = bare.strip_suffix("_banner")
        && !stem.ends_with("_wall")
    {
        return Block::from_key(&format!("minecraft:{stem}_wall_banner"));
    }
    if let Some(stem) = bare.strip_suffix("_skull")
        && !stem.ends_with("_wall")
    {
        return Block::from_key(&format!("minecraft:{stem}_wall_skull"));
    }
    if let Some(stem) = bare.strip_suffix("_head")
        && !stem.ends_with("_wall")
    {
        return Block::from_key(&format!("minecraft:{stem}_wall_head"));
    }
    None
}

/// Registers built-in vanilla [`BlockPlacementRule`]s across all matching blocks in `BLOCK_INFOS`.
pub fn register_vanilla_placement_rules(manager: &BlockManager) {
    let axis_rule: Arc<dyn BlockPlacementRule> = Arc::new(AxisPlacementRule);
    let horiz_toward_rule: Arc<dyn BlockPlacementRule> = Arc::new(HorizontalFacingPlacementRule);
    let horiz_away_rule: Arc<dyn BlockPlacementRule> = Arc::new(AwayFacingPlacementRule);
    let anvil_rule: Arc<dyn BlockPlacementRule> = Arc::new(AnvilPlacementRule);
    let dir6_player_rule: Arc<dyn BlockPlacementRule> = Arc::new(Directional6PlacementRule {
        use_clicked_face: false,
        face_player: true,
    });
    let dir6_away_rule: Arc<dyn BlockPlacementRule> = Arc::new(Directional6PlacementRule {
        use_clicked_face: false,
        face_player: false,
    });
    let dir6_face_rule: Arc<dyn BlockPlacementRule> = Arc::new(Directional6PlacementRule {
        use_clicked_face: true,
        face_player: false,
    });
    let slab_rule: Arc<dyn BlockPlacementRule> = Arc::new(SlabPlacementRule);
    let stairs_rule: Arc<dyn BlockPlacementRule> = Arc::new(StairsPlacementRule);
    let trapdoor_rule: Arc<dyn BlockPlacementRule> = Arc::new(TrapdoorPlacementRule);
    let door_rule: Arc<dyn BlockPlacementRule> = Arc::new(DoorPlacementRule);
    let fence_rule: Arc<dyn BlockPlacementRule> = Arc::new(FenceConnectPlacementRule);
    let wall_rule: Arc<dyn BlockPlacementRule> = Arc::new(WallConnectPlacementRule);
    let face_attached_rule: Arc<dyn BlockPlacementRule> = Arc::new(FaceAttachedPlacementRule);
    let lantern_rule: Arc<dyn BlockPlacementRule> = Arc::new(LanternPlacementRule);
    let snow_rule: Arc<dyn BlockPlacementRule> = Arc::new(SnowLayerPlacementRule);
    let waterlogged_rule: Arc<dyn BlockPlacementRule> = Arc::new(WaterloggedOnlyPlacementRule);

    for info in BLOCK_INFOS {
        let block = Block(info.default_state_id);
        let key = info.key;

        if block == Block::SNOW {
            manager.register_shared_placement_rule(info.id, Arc::clone(&snow_rule));
            continue;
        }
        if key.ends_with("anvil") {
            manager.register_shared_placement_rule(info.id, Arc::clone(&anvil_rule));
            continue;
        }
        // Torch -> wall_torch redirect even though standing torch has no rotation property
        if matches!(
            key,
            "minecraft:torch"
                | "minecraft:soul_torch"
                | "minecraft:redstone_torch"
                | "minecraft:copper_torch"
        ) {
            let wall_counterpart = find_wall_counterpart(key);
            manager.register_shared_placement_rule(
                info.id,
                Arc::new(Rotation16PlacementRule { wall_counterpart }),
            );
            continue;
        }
        // Slab (`type = top | bottom | double`)
        if info
            .properties
            .iter()
            .any(|(n, vals)| *n == "type" && vals.contains(&"double") && vals.contains(&"bottom"))
        {
            manager.register_shared_placement_rule(info.id, Arc::clone(&slab_rule));
            continue;
        }
        // Stairs (`facing`, `half`, `shape` with `inner_left`)
        if is_stairs_block(block) {
            manager.register_shared_placement_rule(info.id, Arc::clone(&stairs_rule));
            continue;
        }
        // Doors (`facing`, `half = upper | lower`, `hinge`)
        if block.has_property("hinge") && block.has_property("half") {
            manager.register_shared_placement_rule(info.id, Arc::clone(&door_rule));
            continue;
        }
        // Trapdoors (`facing`, `half`, `open` without `hinge`)
        if block.has_property("open")
            && block.has_property("half")
            && block.has_property("facing")
            && !block.has_property("hinge")
        {
            manager.register_shared_placement_rule(info.id, Arc::clone(&trapdoor_rule));
            continue;
        }
        // Walls (`up` + `north=none|low|tall`)
        if key.ends_with("_wall") && block.has_property("up") {
            manager.register_shared_placement_rule(info.id, Arc::clone(&wall_rule));
            continue;
        }
        // Fences, glass panes, iron/copper bars (`north`/`south`/`east`/`west` = `true|false`)
        if (key.ends_with("_fence") || key.ends_with("_pane") || key.ends_with("_bars"))
            && block.has_property("north")
            && block.has_property("east")
        {
            manager.register_shared_placement_rule(info.id, Arc::clone(&fence_rule));
            continue;
        }
        // Buttons, levers, grindstones (`face = floor | wall | ceiling` + `facing`)
        if block.has_property("face") && block.has_property("facing") {
            manager.register_shared_placement_rule(info.id, Arc::clone(&face_attached_rule));
            continue;
        }
        // Lanterns (`hanging`)
        if block.has_property("hanging") && key.ends_with("lantern") {
            manager.register_shared_placement_rule(info.id, Arc::clone(&lantern_rule));
            continue;
        }
        // Standing signs, banners, skulls (`rotation = 0..15`)
        if block.has_property("rotation") {
            let wall_counterpart = find_wall_counterpart(key);
            manager.register_shared_placement_rule(
                info.id,
                Arc::new(Rotation16PlacementRule { wall_counterpart }),
            );
            continue;
        }
        // Axis pillars (`axis = x | y | z`)
        if info
            .properties
            .iter()
            .any(|(n, vals)| *n == "axis" && vals.len() == 3)
        {
            manager.register_shared_placement_rule(info.id, Arc::clone(&axis_rule));
            continue;
        }
        // 6-way directional vs 4-way horizontal `facing`
        if let Some((_, vals)) = info.properties.iter().find(|(n, _)| *n == "facing") {
            if vals.len() == 6 {
                let rule = if key == "minecraft:observer" {
                    &dir6_away_rule
                } else if key.ends_with("_rod")
                    || key.ends_with("_cluster")
                    || key.ends_with("_bud")
                {
                    &dir6_face_rule
                } else {
                    &dir6_player_rule
                };
                manager.register_shared_placement_rule(info.id, Arc::clone(rule));
                continue;
            } else if vals.len() == 4 {
                let rule = if key.ends_with("_fence_gate")
                    || key.ends_with("_glazed_terracotta")
                    || key == "minecraft:calibrated_sculk_sensor"
                {
                    &horiz_away_rule
                } else {
                    &horiz_toward_rule
                };
                manager.register_shared_placement_rule(info.id, Arc::clone(rule));
                continue;
            }
        }
        // Remaining waterloggable blocks (e.g. roots, grates, conduits, scaffolding, coral)
        if block.has_property("waterlogged") {
            manager.register_shared_placement_rule(info.id, Arc::clone(&waterlogged_rule));
        }
    }
}
