//! Axis-aligned `BoundingBox` (`AABB`) geometry, intersection queries, and gravity/drag
//! block collision resolution over [`BlockGetter`].

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

    /// Returns `true` if any solid block in `world` overlaps this bounding box at `foot_pos`.
    #[must_use]
    pub fn collides_with_blocks(&self, world: &dyn BlockGetter, foot_pos: Vec3) -> bool {
        if self.width() <= 0.0 || self.height() <= 0.0 || self.depth() <= 0.0 {
            return false;
        }
        let (min, max) = self.world_bounds(foot_pos);
        let x0 = (min.x + COLLISION_EPS).floor() as i32;
        let x1 = (max.x - COLLISION_EPS).floor() as i32;
        let y0 = (min.y + COLLISION_EPS).floor() as i32;
        let y1 = (max.y - COLLISION_EPS).floor() as i32;
        let z0 = (min.z + COLLISION_EPS).floor() as i32;
        let z1 = (max.z - COLLISION_EPS).floor() as i32;

        for by in y0..=y1 {
            for bz in z0..=z1 {
                for bx in x0..=x1 {
                    if world.get_block(BlockVec::new(bx, by, bz)).is_solid() {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Outcome of a single 20 TPS entity physics & collision step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsStepOutcome {
    /// Resolved post-collision position.
    pub position: Pos,
    /// Post-drag velocity for the next tick.
    pub velocity: Vec3,
    /// `true` if resting on a solid block below (`dy < 0` clamped).
    pub on_ground: bool,
    /// `true` if X or Z motion was blocked by a solid wall.
    pub collided_horizontally: bool,
}

/// Steps gravity, axis-by-axis solid-block collision (`Y` -> `X` -> `Z`), and aerodynamic drag
/// for one 50ms server tick (`Aerodynamics` + `CollisionUtils` in Minestom).
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

    // 1. Resolve vertical Y movement
    if vel.y.abs() > 0.0 {
        let candidate_y = Vec3::new(cur.x, cur.y + vel.y, cur.z);
        if bbox.collides_with_blocks(world, candidate_y) {
            if vel.y < 0.0 {
                // Snap cleanly onto top face of supporting block
                let min_y = cur.y + bbox.rel_min.y + vel.y;
                let block_top = (min_y.floor() + 1.0) - bbox.rel_min.y;
                if block_top <= cur.y + 0.001
                    && !bbox.collides_with_blocks(world, Vec3::new(cur.x, block_top, cur.z))
                {
                    cur.y = block_top;
                }
                on_ground = true;
            }
            vel.y = 0.0;
        } else {
            cur.y = candidate_y.y;
        }
    } else {
        // Check if standing directly atop a solid block
        on_ground = bbox.collides_with_blocks(world, Vec3::new(cur.x, cur.y - 0.001, cur.z));
    }

    // 2. Resolve X movement
    if vel.x.abs() > 0.0 {
        let candidate_x = Vec3::new(cur.x + vel.x, cur.y, cur.z);
        if bbox.collides_with_blocks(world, candidate_x) {
            vel.x = 0.0;
            collided_h = true;
        } else {
            cur.x = candidate_x.x;
        }
    }

    // 3. Resolve Z movement
    if vel.z.abs() > 0.0 {
        let candidate_z = Vec3::new(cur.x, cur.y, cur.z + vel.z);
        if bbox.collides_with_blocks(world, candidate_z) {
            vel.z = 0.0;
            collided_h = true;
        } else {
            cur.z = candidate_z.z;
        }
    }

    // Standard Minecraft ground/air drag
    let h_drag = if on_ground { 0.546 } else { 0.91 };
    vel.x *= h_drag;
    vel.z *= h_drag;
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

    use super::*;

    struct FlatFloor;
    impl BlockGetter for FlatFloor {
        fn get_block(&self, pos: BlockVec) -> Block {
            if pos.y < 40 {
                Block::GRASS_BLOCK
            } else if pos.x == 5 && pos.y == 40 && pos.z == 0 {
                Block::STONE
            } else {
                Block::AIR
            }
        }
    }

    #[test]
    fn falling_entity_lands_on_floor_and_stops_against_wall() {
        let world = FlatFloor;
        let bbox = BoundingBox::PLAYER;
        let mut pos = Pos::new(0.5, 42.0, 0.5);
        let mut vel = Vec3::ZERO;
        let mut grounded = false;

        for _ in 0..40 {
            let step = step_entity_physics(&world, bbox, pos, vel, false);
            pos = step.position;
            vel = step.velocity;
            grounded = step.on_ground;
            if grounded {
                break;
            }
        }
        assert!(grounded);
        assert!((pos.y - 40.0).abs() < 1e-4);

        // Push horizontally into the stone pillar at (5, 40, 0)
        pos = Pos::new(4.65, 40.0, 0.5);
        let wall_step = step_entity_physics(&world, bbox, pos, Vec3::new(0.5, 0.0, 0.0), false);
        assert!(wall_step.collided_horizontally);
        assert!((wall_step.velocity.x - 0.0).abs() < f64::EPSILON);
    }
}
