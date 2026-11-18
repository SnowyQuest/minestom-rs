//! Axis-aligned `BoundingBox` (`AABB`) geometry, sub-voxel block shape sweeping, and
//! gravity/drag collision resolution over [`BlockGetter`].

use minestom_block::BlockGetter;
use minestom_core::{BlockVec, Point, Pos, Vec3};

const COLLISION_EPS: f64 = 1.0e-7;

/// Axis-aligned bounding box relative to an entity's foot position `(0, 0, 0)`
/// (`BoundingBox` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    /// Minimum relative corner (typically `(-width / 2, 0, -depth / 2)`).
    pub rel_min: Vec3,
    /// Maximum relative corner (typically `(+width / 2, height, +depth / 2)`).
    pub rel_max: Vec3,
}

impl Default for BoundingBox {
    fn default() -> Self {
        Self::PLAYER
    }
}

impl BoundingBox {
    /// Zero-volume bounding box (`0 × 0 × 0`).
    pub const ZERO: Self = Self {
        rel_min: Vec3::ZERO,
        rel_max: Vec3::ZERO,
    };

    /// Standing player hitbox (`0.6 × 1.8 × 0.6`).
    pub const PLAYER: Self = Self::symmetric(0.6, 1.8);

    /// Sneaking player hitbox (`0.6 × 1.5 × 0.6`).
    pub const PLAYER_SNEAKING: Self = Self::symmetric(0.6, 1.5);

    /// Constructs a horizontally centered hitbox (`width × height × width`) anchored at `min_y = 0`.
    #[must_use]
    pub const fn symmetric(width: f64, height: f64) -> Self {
        let half = width * 0.5;
        Self {
            rel_min: Vec3::new(-half, 0.0, -half),
            rel_max: Vec3::new(half, height, half),
        }
    }

    /// Constructs a centered hitbox with distinct `width` (X), `height` (Y), and `depth` (Z).
    #[must_use]
    pub const fn new(width: f64, height: f64, depth: f64) -> Self {
        Self {
            rel_min: Vec3::new(-width * 0.5, 0.0, -depth * 0.5),
            rel_max: Vec3::new(width * 0.5, height, depth * 0.5),
        }
    }

    /// X-axis span.
    #[must_use]
    pub fn width(&self) -> f64 {
        self.rel_max.x - self.rel_min.x
    }

    /// Y-axis span.
    #[must_use]
    pub fn height(&self) -> f64 {
        self.rel_max.y - self.rel_min.y
    }

    /// Z-axis span.
    #[must_use]
    pub fn depth(&self) -> f64 {
        self.rel_max.z - self.rel_min.z
    }

    /// Expands width, height, and depth by `(dx, dy, dz)` symmetrically around the foot center.
    #[must_use]
    pub fn expand(&self, dx: f64, dy: f64, dz: f64) -> Self {
        Self::new(self.width() + dx, self.height() + dy, self.depth() + dz)
    }

    /// Contracts width, height, and depth by `(dx, dy, dz)` (clamped to `>= 0.0`).
    #[must_use]
    pub fn contract(&self, dx: f64, dy: f64, dz: f64) -> Self {
        Self::new(
            (self.width() - dx).max(0.0),
            (self.height() - dy).max(0.0),
            (self.depth() - dz).max(0.0),
        )
    }

    /// World-space `(min, max)` corners when anchored at `foot_pos`.
    #[must_use]
    pub fn world_bounds(&self, foot_pos: Vec3) -> (Vec3, Vec3) {
        (
            Vec3::new(
                foot_pos.x + self.rel_min.x,
                foot_pos.y + self.rel_min.y,
                foot_pos.z + self.rel_min.z,
            ),
            Vec3::new(
                foot_pos.x + self.rel_max.x,
                foot_pos.y + self.rel_max.y,
                foot_pos.z + self.rel_max.z,
            ),
        )
    }

    /// Returns `true` if `self` at `pos_a` overlaps `other` at `pos_b`.
    #[must_use]
    pub fn intersects_at<P1: Point, P2: Point>(
        &self,
        pos_a: &P1,
        other: &Self,
        pos_b: &P2,
    ) -> bool {
        let (min_a, max_a) = self.world_bounds(Vec3::new(pos_a.x(), pos_a.y(), pos_a.z()));
        let (min_b, max_b) = other.world_bounds(Vec3::new(pos_b.x(), pos_b.y(), pos_b.z()));
        min_a.x < max_b.x - COLLISION_EPS
            && max_a.x > min_b.x + COLLISION_EPS
            && min_a.y < max_b.y - COLLISION_EPS
            && max_a.y > min_b.y + COLLISION_EPS
            && min_a.z < max_b.z - COLLISION_EPS
            && max_a.z > min_b.z + COLLISION_EPS
    }

    /// Returns `true` if any sub-voxel [`Block::collision_shape`](minestom_block::Block::collision_shape)
    /// box in `world` overlaps this bounding box at `foot_pos` (including 1.5-tall fences/walls below `min.y`).
    #[must_use]
    pub fn collides_with_blocks(&self, world: &dyn BlockGetter, foot_pos: Vec3) -> bool {
        if self.width() <= 0.0 || self.height() <= 0.0 || self.depth() <= 0.0 {
            return false;
        }
        let (min, max) = self.world_bounds(foot_pos);
        let x0 = (min.x + COLLISION_EPS).floor() as i32;
        let x1 = (max.x - COLLISION_EPS).floor() as i32;
        // Include `y0 - 1` so 1.5-tall fences, walls, and fence gates collide accurately!
        let y0 = (min.y + COLLISION_EPS).floor() as i32 - 1;
        let y1 = (max.y - COLLISION_EPS).floor() as i32;
        let z0 = (min.z + COLLISION_EPS).floor() as i32;
        let z1 = (max.z - COLLISION_EPS).floor() as i32;

        for by in y0..=y1 {
            for bz in z0..=z1 {
                for bx in x0..=x1 {
                    let bpos = BlockVec::new(bx, by, bz);
                    if world.get_block(bpos).collides_world_aabb(bpos, min, max) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn sweep_axis_y(world: &dyn BlockGetter, min: Vec3, max: Vec3, dy: f64) -> f64 {
    if dy.abs() < COLLISION_EPS {
        return 0.0;
    }
    let x0 = (min.x + COLLISION_EPS).floor() as i32;
    let x1 = (max.x - COLLISION_EPS).floor() as i32;
    let z0 = (min.z + COLLISION_EPS).floor() as i32;
    let z1 = (max.z - COLLISION_EPS).floor() as i32;
    let y0 = (min.y.min(min.y + dy) + COLLISION_EPS).floor() as i32 - 1;
    let y1 = (max.y.max(max.y + dy) - COLLISION_EPS).floor() as i32;

    let mut allowed = dy;
    for by in y0..=y1 {
        for bz in z0..=z1 {
            for bx in x0..=x1 {
                let b = world.get_block(BlockVec::new(bx, by, bz));
                for box_aabb in b.collision_shape() {
                    let b_min_x = f64::from(bx) + f64::from(box_aabb.min_x);
                    let b_max_x = f64::from(bx) + f64::from(box_aabb.max_x);
                    let b_min_z = f64::from(bz) + f64::from(box_aabb.min_z);
                    let b_max_z = f64::from(bz) + f64::from(box_aabb.max_z);
                    if max.x <= b_min_x + COLLISION_EPS
                        || min.x >= b_max_x - COLLISION_EPS
                        || max.z <= b_min_z + COLLISION_EPS
                        || min.z >= b_max_z - COLLISION_EPS
                    {
                        continue;
                    }
                    let b_min_y = f64::from(by) + f64::from(box_aabb.min_y);
                    let b_max_y = f64::from(by) + f64::from(box_aabb.max_y);
                    if allowed > 0.0 && max.y <= b_min_y + COLLISION_EPS {
                        let gap = b_min_y - max.y;
                        if gap < allowed {
                            allowed = gap.max(0.0);
                        }
                    } else if allowed < 0.0 && min.y >= b_max_y - COLLISION_EPS {
                        let gap = b_max_y - min.y;
                        if gap > allowed {
                            allowed = gap.min(0.0);
                        }
                    }
                }
            }
        }
    }
    allowed
}

fn sweep_axis_x(world: &dyn BlockGetter, min: Vec3, max: Vec3, dx: f64) -> f64 {
    if dx.abs() < COLLISION_EPS {
        return 0.0;
    }
    let y0 = (min.y + COLLISION_EPS).floor() as i32 - 1;
    let y1 = (max.y - COLLISION_EPS).floor() as i32;
    let z0 = (min.z + COLLISION_EPS).floor() as i32;
    let z1 = (max.z - COLLISION_EPS).floor() as i32;
    let x0 = (min.x.min(min.x + dx) + COLLISION_EPS).floor() as i32;
    let x1 = (max.x.max(max.x + dx) - COLLISION_EPS).floor() as i32;

    let mut allowed = dx;
    for by in y0..=y1 {
        for bz in z0..=z1 {
            for bx in x0..=x1 {
                let b = world.get_block(BlockVec::new(bx, by, bz));
                for box_aabb in b.collision_shape() {
                    let b_min_y = f64::from(by) + f64::from(box_aabb.min_y);
                    let b_max_y = f64::from(by) + f64::from(box_aabb.max_y);
                    let b_min_z = f64::from(bz) + f64::from(box_aabb.min_z);
                    let b_max_z = f64::from(bz) + f64::from(box_aabb.max_z);
                    if max.y <= b_min_y + COLLISION_EPS
                        || min.y >= b_max_y - COLLISION_EPS
                        || max.z <= b_min_z + COLLISION_EPS
                        || min.z >= b_max_z - COLLISION_EPS
                    {
                        continue;
                    }
                    let b_min_x = f64::from(bx) + f64::from(box_aabb.min_x);
                    let b_max_x = f64::from(bx) + f64::from(box_aabb.max_x);
                    if allowed > 0.0 && max.x <= b_min_x + COLLISION_EPS {
                        let gap = b_min_x - max.x;
                        if gap < allowed {
                            allowed = gap.max(0.0);
                        }
                    } else if allowed < 0.0 && min.x >= b_max_x - COLLISION_EPS {
                        let gap = b_max_x - min.x;
                        if gap > allowed {
                            allowed = gap.min(0.0);
                        }
                    }
                }
            }
        }
    }
    allowed
}

fn sweep_axis_z(world: &dyn BlockGetter, min: Vec3, max: Vec3, dz: f64) -> f64 {
    if dz.abs() < COLLISION_EPS {
        return 0.0;
    }
    let y0 = (min.y + COLLISION_EPS).floor() as i32 - 1;
    let y1 = (max.y - COLLISION_EPS).floor() as i32;
    let x0 = (min.x + COLLISION_EPS).floor() as i32;
    let x1 = (max.x - COLLISION_EPS).floor() as i32;
    let z0 = (min.z.min(min.z + dz) + COLLISION_EPS).floor() as i32;
    let z1 = (max.z.max(max.z + dz) - COLLISION_EPS).floor() as i32;

    let mut allowed = dz;
    for by in y0..=y1 {
        for bz in z0..=z1 {
            for bx in x0..=x1 {
                let b = world.get_block(BlockVec::new(bx, by, bz));
                for box_aabb in b.collision_shape() {
                    let b_min_y = f64::from(by) + f64::from(box_aabb.min_y);
                    let b_max_y = f64::from(by) + f64::from(box_aabb.max_y);
                    let b_min_x = f64::from(bx) + f64::from(box_aabb.min_x);
                    let b_max_x = f64::from(bx) + f64::from(box_aabb.max_x);
                    if max.y <= b_min_y + COLLISION_EPS
                        || min.y >= b_max_y - COLLISION_EPS
                        || max.x <= b_min_x + COLLISION_EPS
                        || min.x >= b_max_x - COLLISION_EPS
                    {
                        continue;
                    }
                    let b_min_z = f64::from(bz) + f64::from(box_aabb.min_z);
                    let b_max_z = f64::from(bz) + f64::from(box_aabb.max_z);
                    if allowed > 0.0 && max.z <= b_min_z + COLLISION_EPS {
                        let gap = b_min_z - max.z;
                        if gap < allowed {
                            allowed = gap.max(0.0);
                        }
                    } else if allowed < 0.0 && min.z >= b_max_z - COLLISION_EPS {
                        let gap = b_max_z - min.z;
                        if gap > allowed {
                            allowed = gap.min(0.0);
                        }
                    }
                }
            }
        }
    }
    allowed
}

/// Outcome of a single 20 TPS entity physics & collision step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsStepOutcome {
    /// Resolved post-collision position.
    pub position: Pos,
    /// Post-drag velocity for the next tick.
    pub velocity: Vec3,
    /// `true` if resting on a solid/sub-voxel block face below (`dy < 0` clamped).
    pub on_ground: bool,
    /// `true` if X or Z motion was blocked by a wall/fence/step.
    pub collided_horizontally: bool,
}

/// Steps gravity, exact sub-voxel AABB sweep (`Y` -> `X` -> `Z` over `Block::collision_shape()`),
/// and ground/air friction drag (`Aerodynamics` + `CollisionUtils` in Minestom).
#[must_use]
pub fn step_entity_physics(
    world: &dyn BlockGetter,
    bbox: BoundingBox,
    pos: Pos,
    mut vel: Vec3,
    no_gravity: bool,
) -> PhysicsStepOutcome {
    if !no_gravity {
        vel.y -= 0.08;
    }

    let mut cur = pos.as_vec();
    let mut on_ground = false;
    let mut collided_h = false;

    if bbox.width() > 0.0 && bbox.height() > 0.0 && bbox.depth() > 0.0 {
        // 1. Sweep vertical Y movement against sub-voxel boxes (slabs, stairs, fences, snow layers)
        if vel.y.abs() > 0.0 {
            let (min, max) = bbox.world_bounds(cur);
            let moved_y = sweep_axis_y(world, min, max, vel.y);
            cur.y += moved_y;
            if (moved_y - vel.y).abs() > COLLISION_EPS {
                if vel.y < 0.0 {
                    on_ground = true;
                }
                vel.y = 0.0;
            }
        } else {
            on_ground = bbox.collides_with_blocks(world, Vec3::new(cur.x, cur.y - 0.001, cur.z));
        }

        // 2. Sweep X movement
        if vel.x.abs() > 0.0 {
            let (min, max) = bbox.world_bounds(cur);
            let moved_x = sweep_axis_x(world, min, max, vel.x);
            cur.x += moved_x;
            if (moved_x - vel.x).abs() > COLLISION_EPS {
                vel.x = 0.0;
                collided_h = true;
            }
        }

        // 3. Sweep Z movement
        if vel.z.abs() > 0.0 {
            let (min, max) = bbox.world_bounds(cur);
            let moved_z = sweep_axis_z(world, min, max, vel.z);
            cur.z += moved_z;
            if (moved_z - vel.z).abs() > COLLISION_EPS {
                vel.z = 0.0;
                collided_h = true;
            }
        }
    } else {
        cur += vel;
    }

    // Apply block-specific ground slipperiness (`0.6` default -> `0.546`, `0.98` ice -> `0.8918`)
    let ground_friction = if on_ground {
        let support = world.get_block(BlockVec::new(
            cur.x.floor() as i32,
            (cur.y - 0.2).floor() as i32,
            cur.z.floor() as i32,
        ));
        let f = f64::from(if support.is_air() {
            0.6
        } else {
            support.friction()
        });
        f * 0.91
    } else {
        0.91
    };

    vel.x *= ground_friction;
    vel.z *= ground_friction;
    vel.y *= 0.98;
    if vel.x.abs() < 1.0e-6 {
        vel.x = 0.0;
    }
    if vel.y.abs() < 1.0e-6 {
        vel.y = 0.0;
    }
    if vel.z.abs() < 1.0e-6 {
        vel.z = 0.0;
    }

    PhysicsStepOutcome {
        position: Pos::with_look(cur.x, cur.y, cur.z, pos.yaw, pos.pitch),
        velocity: vel,
        on_ground,
        collided_horizontally: collided_h,
    }
}

#[cfg(test)]
mod tests {
    use minestom_block::Block;
    use rustc_hash::FxHashMap;

    use super::*;

    struct TestWorld(FxHashMap<BlockVec, Block>);
    impl BlockGetter for TestWorld {
        fn get_block(&self, pos: BlockVec) -> Block {
            self.0.get(&pos).copied().unwrap_or(Block::AIR)
        }
    }

    #[test]
    fn sub_voxel_bottom_slab_and_open_door_collision() {
        let mut map = FxHashMap::default();
        // Bottom oak slab at y=10 (`max_y = 0.5` -> top surface at `10.5`!)
        map.insert(BlockVec::new(0, 10, 0), Block::OAK_SLAB);
        let world = TestWorld(map);

        let mut pos = Pos::new(0.5, 11.2, 0.5);
        let mut vel = Vec3::ZERO;
        for _ in 0..20 {
            let out = step_entity_physics(&world, BoundingBox::PLAYER, pos, vel, false);
            pos = out.position;
            vel = out.velocity;
        }
        assert!(
            (pos.y - 10.5).abs() < 1e-4,
            "expected landing on bottom slab at 10.5, got {}",
            pos.y
        );

        // Open oak fence gate has empty collision shape, and open oak door rotates to a thin
        // 3/16 hinge panel so a player walks cleanly through the doorway!
        let open_gate = Block::OAK_FENCE_GATE.with_property("open", "true").unwrap();
        assert_eq!(open_gate.collision_shape().len(), 0);

        let open_door = Block::OAK_DOOR.with_property("open", "true").unwrap();
        assert_eq!(open_door.collision_shape().len(), 1);
        let mut door_map = FxHashMap::default();
        door_map.insert(BlockVec::new(0, 9, 0), Block::STONE);
        door_map.insert(BlockVec::new(0, 9, 1), Block::STONE);
        door_map.insert(BlockVec::new(0, 10, 0), open_door);
        let door_world = TestWorld(door_map);
        let walk = step_entity_physics(
            &door_world,
            BoundingBox::PLAYER,
            Pos::new(0.6, 10.0, -0.2),
            Vec3::new(0.0, 0.0, 0.8),
            true,
        );
        assert!(
            walk.position.z > 0.3,
            "player should walk through open door, got z={}",
            walk.position.z
        );
    }
}
