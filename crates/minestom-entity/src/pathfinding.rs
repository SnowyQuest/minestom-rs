//! Bounded 3D grid A* pathfinder and waypoint `Navigator`
//! (`net.minestom.server.entity.pathfinding` equivalent).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use minestom_block::BlockGetter;
use minestom_core::{BlockVec, Pos, Vec3};
use rustc_hash::FxHashMap;

use crate::EntityCore;

/// Configuration parameters for [`find_path`].
#[derive(Debug, Clone, Copy)]
pub struct PathSearchConfig {
    /// Required vertical clearance in whole blocks (`2` for player/zombie-sized mobs, `1` for spiders/chickens).
    pub entity_height_blocks: i32,
    /// Maximum safe downward drop in blocks (`3` in vanilla).
    pub max_fall_distance: i32,
    /// Maximum A* nodes expanded before aborting (`512` default; bounds worst-case CPU cost).
    pub max_visited_nodes: usize,
    /// If `true` and the exact goal is unreachable within `max_visited_nodes`, returns the path
    /// to the closest explored node to the goal.
    pub allow_best_effort: bool,
}

impl Default for PathSearchConfig {
    fn default() -> Self {
        Self {
            entity_height_blocks: 2,
            max_fall_distance: 3,
            max_visited_nodes: 512,
            allow_best_effort: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OpenNode {
    pos: BlockVec,
    f_milli: i64,
    g_milli: i64,
}

impl Eq for OpenNode {}

impl Ord for OpenNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_milli
            .cmp(&self.f_milli)
            .then_with(|| other.g_milli.cmp(&self.g_milli))
    }
}

impl PartialOrd for OpenNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[inline]
fn is_passable_space(world: &dyn BlockGetter, base: BlockVec, height: i32) -> bool {
    for dy in 0..height.max(1) {
        let b = world.get_block(BlockVec::new(base.x, base.y + dy, base.z));
        if b.blocks_motion() || b.has_fluid() {
            return false;
        }
    }
    true
}

/// Returns `true` if an entity of `height` blocks can stand with its feet in `pos`
/// (motion-blocking floor at `pos.y - 1` and passable, non-fluid space in `pos.y .. pos.y + height`).
#[must_use]
pub fn is_standable(world: &dyn BlockGetter, pos: BlockVec, height: i32) -> bool {
    let floor = world.get_block(BlockVec::new(pos.x, pos.y - 1, pos.z));
    floor.blocks_motion() && is_passable_space(world, pos, height)
}

#[inline]
fn octile_heuristic_milli(a: BlockVec, b: BlockVec) -> i64 {
    let dx = i64::from((a.x - b.x).abs());
    let dy = i64::from((a.y - b.y).abs());
    let dz = i64::from((a.z - b.z).abs());
    let dmin = dx.min(dz);
    let dmax = dx.max(dz);
    dmin * 1414 + (dmax - dmin) * 1000 + dy * 1200
}

fn reconstruct_waypoints(
    came_from: &FxHashMap<BlockVec, BlockVec>,
    start: BlockVec,
    end: BlockVec,
) -> Vec<Pos> {
    let mut blocks = Vec::new();
    let mut cur = end;
    blocks.push(cur);
    while cur != start {
        let Some(&prev) = came_from.get(&cur) else {
            break;
        };
        cur = prev;
        if cur != start {
            blocks.push(cur);
        }
    }
    blocks.reverse();
    blocks
        .into_iter()
        .map(|b| Pos::new(f64::from(b.x) + 0.5, f64::from(b.y), f64::from(b.z) + 0.5))
        .collect()
}

/// Computes a 3D grid A* path of block-centered [`Pos`] waypoints from `start` to `goal`
/// using [`PathSearchConfig::default()`].
#[must_use]
pub fn find_path(world: &dyn BlockGetter, start: BlockVec, goal: BlockVec) -> Option<Vec<Pos>> {
    find_path_with_config(world, start, goal, PathSearchConfig::default())
}

/// Computes a bounded 3D grid A* path supporting flat walking, diagonal movement without
/// corner-cutting, 1-block step-ups, and safe multi-block drops.
#[must_use]
pub fn find_path_with_config(
    world: &dyn BlockGetter,
    start: BlockVec,
    goal: BlockVec,
    config: PathSearchConfig,
) -> Option<Vec<Pos>> {
    const DIRS: [(i32, i32, i64); 8] = [
        (1, 0, 1000),
        (-1, 0, 1000),
        (0, 1, 1000),
        (0, -1, 1000),
        (1, 1, 1414),
        (1, -1, 1414),
        (-1, 1, 1414),
        (-1, -1, 1414),
    ];

    if start == goal {
        return Some(Vec::new());
    }
    let h = config.entity_height_blocks.max(1);

    let mut open = BinaryHeap::new();
    let mut g_score: FxHashMap<BlockVec, i64> = FxHashMap::default();
    let mut came_from: FxHashMap<BlockVec, BlockVec> = FxHashMap::default();

    let h0 = octile_heuristic_milli(start, goal);
    g_score.insert(start, 0);
    open.push(OpenNode {
        pos: start,
        f_milli: h0,
        g_milli: 0,
    });

    let mut best_node = start;
    let mut best_h = h0;
    let mut visited = 0_usize;

    while let Some(current) = open.pop() {
        if current.pos == goal {
            return Some(reconstruct_waypoints(&came_from, start, goal));
        }
        if current
            .g_milli
            .gt(g_score.get(&current.pos).unwrap_or(&i64::MAX))
        {
            continue;
        }
        visited += 1;
        if visited > config.max_visited_nodes {
            break;
        }

        let cur = current.pos;
        for (dx, dz, base_cost) in DIRS {
            let is_diag = dx != 0 && dz != 0;
            if is_diag {
                // Prevent cutting diagonally across solid wall corners
                let side_x = BlockVec::new(cur.x + dx, cur.y, cur.z);
                let side_z = BlockVec::new(cur.x, cur.y, cur.z + dz);
                if !is_passable_space(world, side_x, h) || !is_passable_space(world, side_z, h) {
                    continue;
                }
            }

            let flat = BlockVec::new(cur.x + dx, cur.y, cur.z + dz);
            let mut next_opt: Option<(BlockVec, i64)> = None;

            if is_standable(world, flat, h) {
                next_opt = Some((flat, base_cost));
            } else if !is_diag {
                // Check 1-block step-up (requires overhead jump clearance above `cur`)
                let step_up = BlockVec::new(cur.x + dx, cur.y + 1, cur.z + dz);
                let headroom = BlockVec::new(cur.x, cur.y + h, cur.z);
                if !world.get_block(headroom).blocks_motion() && is_standable(world, step_up, h) {
                    next_opt = Some((step_up, base_cost + 600));
                } else if is_passable_space(world, flat, h) {
                    // Check safe downward drop (`1..=max_fall_distance`)
                    for fall in 1..=config.max_fall_distance.max(0) {
                        let candidate = BlockVec::new(cur.x + dx, cur.y - fall, cur.z + dz);
                        if is_standable(world, candidate, h) {
                            next_opt = Some((candidate, base_cost + i64::from(fall) * 300));
                            break;
                        }
                        if !is_passable_space(world, candidate, 1) {
                            break;
                        }
                    }
                }
            }

            if let Some((next_pos, step_cost)) = next_opt {
                let tentative_g = current.g_milli + step_cost;
                if tentative_g < *g_score.get(&next_pos).unwrap_or(&i64::MAX) {
                    came_from.insert(next_pos, cur);
                    g_score.insert(next_pos, tentative_g);
                    let h_est = octile_heuristic_milli(next_pos, goal);
                    if h_est < best_h {
                        best_h = h_est;
                        best_node = next_pos;
                    }
                    open.push(OpenNode {
                        pos: next_pos,
                        f_milli: tentative_g + h_est,
                        g_milli: tentative_g,
                    });
                }
            }
        }
    }

    if config.allow_best_effort && best_node != start {
        Some(reconstruct_waypoints(&came_from, start, best_node))
    } else {
        None
    }
}

/// Stateful waypoint follower steering an [`EntityCore`] along A* paths (`Navigator` in Minestom).
#[derive(Debug, Clone, Default)]
pub struct Navigator {
    waypoints: Vec<Pos>,
    cursor: usize,
    goal: Option<BlockVec>,
    /// Path search configuration used by [`Navigator::set_path_to`].
    pub search_config: PathSearchConfig,
}

impl Navigator {
    /// Creates an idle [`Navigator`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            waypoints: Vec::new(),
            cursor: 0,
            goal: None,
            search_config: PathSearchConfig {
                allow_best_effort: true,
                ..PathSearchConfig::default()
            },
        }
    }

    /// Computes and assigns an A* path from `start_pos` to `goal_block`, returning `true` if a
    /// non-empty path was found.
    pub fn set_path_to(
        &mut self,
        world: &dyn BlockGetter,
        start_pos: Pos,
        goal_block: BlockVec,
    ) -> bool {
        let start_block = BlockVec::new(
            start_pos.x.floor() as i32,
            start_pos.y.round() as i32,
            start_pos.z.floor() as i32,
        );
        self.goal = Some(goal_block);
        self.cursor = 0;
        if let Some(path) =
            find_path_with_config(world, start_block, goal_block, self.search_config)
        {
            self.waypoints = path;
            !self.waypoints.is_empty()
        } else {
            self.waypoints.clear();
            false
        }
    }

    /// Clears the current path and stops horizontal motion on `core`.
    pub fn reset(&mut self, core: &mut EntityCore) {
        self.waypoints.clear();
        self.cursor = 0;
        self.goal = None;
        core.velocity.x = 0.0;
        core.velocity.z = 0.0;
    }

    /// Returns `true` if the navigator still has unvisited waypoints.
    #[must_use]
    pub fn is_navigating(&self) -> bool {
        self.cursor < self.waypoints.len()
    }

    /// Remaining waypoints slice.
    #[must_use]
    pub fn remaining_waypoints(&self) -> &[Pos] {
        self.waypoints.get(self.cursor..).unwrap_or(&[])
    }

    /// Current target [`BlockVec`], if any.
    #[must_use]
    pub const fn goal(&self) -> Option<BlockVec> {
        self.goal
    }

    /// Steers `core` toward the next waypoint at `speed_blocks_per_tick`, returning `true` while
    /// still moving along the path.
    pub fn advance(&mut self, core: &mut EntityCore, speed_blocks_per_tick: f64) -> bool {
        while let Some(&wp) = self.waypoints.get(self.cursor) {
            let dx = wp.x - core.position.x;
            let dz = wp.z - core.position.z;
            let horiz_dist = dx.hypot(dz);
            let dy = wp.y - core.position.y;
            if horiz_dist <= 0.42 && dy.abs() <= 1.1 {
                self.cursor += 1;
                continue;
            }
            let dir_x = if horiz_dist > 1e-6 {
                dx / horiz_dist
            } else {
                0.0
            };
            let dir_z = if horiz_dist > 1e-6 {
                dz / horiz_dist
            } else {
                0.0
            };
            #[allow(clippy::cast_possible_truncation)]
            let yaw = (-dir_x).atan2(dir_z).to_degrees() as f32;
            core.position.yaw = yaw;
            core.head_yaw = yaw;

            let mut vy = core.velocity.y;
            if dy > 0.35 && core.on_ground {
                vy = 0.42;
            }
            let step = speed_blocks_per_tick.min(horiz_dist);
            core.velocity = Vec3::new(dir_x * step, vy, dir_z * step);
            return true;
        }
        core.velocity.x = 0.0;
        core.velocity.z = 0.0;
        false
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use minestom_block::Block;

    use super::*;

    struct TestGrid {
        blocks: HashMap<BlockVec, Block>,
    }

    impl TestGrid {
        fn flat_floor(half_extent: i32, y_floor: i32) -> Self {
            let mut blocks = HashMap::new();
            for x in -half_extent..=half_extent {
                for z in -half_extent..=half_extent {
                    blocks.insert(BlockVec::new(x, y_floor, z), Block::STONE);
                }
            }
            Self { blocks }
        }
    }

    impl BlockGetter for TestGrid {
        fn get_block(&self, pos: BlockVec) -> Block {
            self.blocks.get(&pos).copied().unwrap_or(Block::AIR)
        }
    }

    #[test]
    fn astar_routes_around_two_block_tall_wall_and_steps_up_stairs() {
        let mut grid = TestGrid::flat_floor(10, 39);
        // Build a 2-block-high wall across x = 3 for z in -3..=3
        for z in -3..=3 {
            grid.blocks.insert(BlockVec::new(3, 40, z), Block::STONE);
            grid.blocks.insert(BlockVec::new(3, 41, z), Block::STONE);
        }
        let path = find_path(&grid, BlockVec::new(0, 40, 0), BlockVec::new(6, 40, 0))
            .expect("path must route around wall");
        assert_ne!(path.len(), 0);
        for wp in &path {
            let bx = wp.x.floor() as i32;
            let bz = wp.z.floor() as i32;
            assert!(
                !(bx == 3 && (-3..=3).contains(&bz)),
                "waypoint {wp:?} passed through solid wall!"
            );
        }
        let last = path.last().copied().unwrap();
        assert_eq!(
            (last.x.floor() as i32, last.y as i32, last.z.floor() as i32),
            (6, 40, 0)
        );

        // Build a 1-block raised platform at x = 8, y = 40 -> standable at y = 41
        grid.blocks.insert(BlockVec::new(7, 40, 0), Block::STONE);
        grid.blocks.insert(BlockVec::new(8, 40, 0), Block::STONE);
        let stair_path = find_path(&grid, BlockVec::new(6, 40, 0), BlockVec::new(8, 41, 0))
            .expect("path must climb 1-block step");
        assert_eq!(stair_path.last().unwrap().y as i32, 41);
    }
}
