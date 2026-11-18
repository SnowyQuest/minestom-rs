//! 3D spatial coordinates (`Point`, `Vec3`, `Pos`, `BlockVec`, `ChunkPos`) and chunk/packed conversions.

use std::ops::{Add, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

/// Common coordinate accessors shared by [`Vec3`], [`Pos`], and [`BlockVec`].
pub trait Point: Copy {
    /// X coordinate in world units.
    fn x(&self) -> f64;
    /// Y coordinate in world units.
    fn y(&self) -> f64;
    /// Z coordinate in world units.
    fn z(&self) -> f64;

    /// Floored block X coordinate (`floor(x)`).
    fn block_x(&self) -> i32 {
        self.x().floor() as i32
    }

    /// Floored block Y coordinate (`floor(y)`).
    fn block_y(&self) -> i32 {
        self.y().floor() as i32
    }

    /// Floored block Z coordinate (`floor(z)`).
    fn block_z(&self) -> i32 {
        self.z().floor() as i32
    }

    /// Chunk X coordinate (`block_x >> 4`).
    fn chunk_x(&self) -> i32 {
        self.block_x().div_euclid(16)
    }

    /// Section Y coordinate (`block_y >> 4`).
    fn section_y(&self) -> i32 {
        self.block_y().div_euclid(16)
    }

    /// Chunk Z coordinate (`block_z >> 4`).
    fn chunk_z(&self) -> i32 {
        self.block_z().div_euclid(16)
    }

    /// Returns the [`ChunkPos`] containing this point.
    fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new(self.chunk_x(), self.chunk_z())
    }

    /// Returns the floored [`BlockVec`] containing this point.
    fn to_block_vec(&self) -> BlockVec {
        BlockVec::new(self.block_x(), self.block_y(), self.block_z())
    }

    /// Squared Euclidean distance to `other`.
    fn distance_squared(&self, other: &impl Point) -> f64 {
        let dx = self.x() - other.x();
        let dy = self.y() - other.y();
        let dz = self.z() - other.z();
        dx.mul_add(dx, dy.mul_add(dy, dz * dz))
    }

    /// Euclidean distance to `other`.
    fn distance(&self, other: &impl Point) -> f64 {
        self.distance_squared(other).sqrt()
    }
}

/// Immutable 3D vector (`x`, `y`, `z`) corresponding to Minestom's `Vec`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
}

impl Vec3 {
    /// Zero vector `(0.0, 0.0, 0.0)`.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    /// Unit vector `(0.0, 1.0, 0.0)`.
    pub const UP: Self = Self::new(0.0, 1.0, 0.0);

    /// Constructs a new `Vec3`.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Squared vector magnitude.
    pub fn length_squared(self) -> f64 {
        self.x
            .mul_add(self.x, self.y.mul_add(self.y, self.z * self.z))
    }

    /// Euclidean vector magnitude.
    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    /// Returns a normalized unit vector, or [`Self::ZERO`] if near zero.
    #[must_use]
    pub fn normalize(self) -> Self {
        let len = self.length();
        if len <= 1e-12 {
            Self::ZERO
        } else {
            Self::new(self.x / len, self.y / len, self.z / len)
        }
    }

    /// Dot product of `self` and `other`.
    pub fn dot(self, other: Self) -> f64 {
        self.x
            .mul_add(other.x, self.y.mul_add(other.y, self.z * other.z))
    }

    /// Cross product `self × other`.
    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y.mul_add(other.z, -(self.z * other.y)),
            self.z.mul_add(other.x, -(self.x * other.z)),
            self.x.mul_add(other.y, -(self.y * other.x)),
        )
    }
}

impl Point for Vec3 {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn z(&self) -> f64 {
        self.z
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// 3D world position with `yaw` and `pitch` rotation (in degrees), corresponding to Minestom's `Pos`.
///
/// ```
/// use minestom_core::{Point, Pos};
///
/// let spawn = Pos::new(0.5, 42.0, -3.2).with_yaw(90.0);
/// assert_eq!(spawn.block_x(), 0);
/// assert_eq!(spawn.block_z(), -4);
/// assert!((spawn.yaw - 90.0).abs() < f32::EPSILON);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Pos {
    /// World X coordinate.
    pub x: f64,
    /// World Y coordinate.
    pub y: f64,
    /// World Z coordinate.
    pub z: f64,
    /// Horizontal rotation in degrees (`-180.0..180.0`).
    pub yaw: f32,
    /// Vertical rotation in degrees (`-90.0..=90.0`).
    pub pitch: f32,
}

impl Pos {
    /// Origin `(0, 0, 0)` facing south (`yaw = 0, pitch = 0`).
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    /// Creates a `Pos` with zero yaw and pitch.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self {
            x,
            y,
            z,
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    /// Creates a `Pos` with explicit `yaw` and `pitch` angles in degrees.
    #[must_use]
    pub const fn with_look(x: f64, y: f64, z: f64, yaw: f32, pitch: f32) -> Self {
        Self {
            x,
            y,
            z,
            yaw,
            pitch,
        }
    }

    /// Returns a copy with updated `x`.
    #[must_use]
    pub const fn with_x(mut self, x: f64) -> Self {
        self.x = x;
        self
    }

    /// Returns a copy with updated `y`.
    #[must_use]
    pub const fn with_y(mut self, y: f64) -> Self {
        self.y = y;
        self
    }

    /// Returns a copy with updated `z`.
    #[must_use]
    pub const fn with_z(mut self, z: f64) -> Self {
        self.z = z;
        self
    }

    /// Returns a copy with updated `yaw`.
    #[must_use]
    pub const fn with_yaw(mut self, yaw: f32) -> Self {
        self.yaw = yaw;
        self
    }

    /// Returns a copy with updated `pitch`.
    #[must_use]
    pub const fn with_pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }

    /// Translates this position by `(dx, dy, dz)`.
    #[must_use]
    pub fn add_xyz(self, dx: f64, dy: f64, dz: f64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            z: self.z + dz,
            ..self
        }
    }

    /// Computes the unit look-direction vector from `yaw` and `pitch`.
    #[must_use]
    pub fn direction(self) -> Vec3 {
        let yaw_rad = f64::from(self.yaw).to_radians();
        let pitch_rad = f64::from(self.pitch).to_radians();
        let xz = pitch_rad.cos();
        Vec3::new(-xz * yaw_rad.sin(), -pitch_rad.sin(), xz * yaw_rad.cos())
    }

    /// Returns `true` if `self` and `other` lie within the same 16×16 chunk column.
    pub fn same_chunk(self, other: Self) -> bool {
        self.chunk_pos() == other.chunk_pos()
    }

    /// Extracts `(x, y, z)` as a [`Vec3`].
    #[must_use]
    pub const fn as_vec(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
}

impl Point for Pos {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn z(&self) -> f64 {
        self.z
    }
}

impl Add<Vec3> for Pos {
    type Output = Self;
    fn add(self, rhs: Vec3) -> Self::Output {
        self.add_xyz(rhs.x, rhs.y, rhs.z)
    }
}

/// Integer 3D block coordinate (`x`, `y`, `z`), corresponding to Minestom's `BlockVec`.
///
/// Supports lossless conversion to/from Minecraft's packed 64-bit network `Position`
/// (`26-bit signed X | 26-bit signed Z | 12-bit signed Y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct BlockVec {
    /// Block X coordinate.
    pub x: i32,
    /// Block Y coordinate.
    pub y: i32,
    /// Block Z coordinate.
    pub z: i32,
}

impl BlockVec {
    /// Origin block `(0, 0, 0)`.
    pub const ZERO: Self = Self::new(0, 0, 0);

    /// Creates a new `BlockVec`.
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Translates this block coordinate by `(dx, dy, dz)`.
    #[must_use]
    pub const fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }

    /// Encodes `(x, y, z)` into Minecraft's packed 64-bit `BLOCK_POSITION` integer:
    /// `((x & 0x3FF_FFFF) << 38) | ((z & 0x3FF_FFFF) << 12) | (y & 0xFFF)`.
    #[must_use]
    pub const fn to_packed_i64(self) -> i64 {
        (((self.x as i64) & 0x03FF_FFFF) << 38)
            | (((self.z as i64) & 0x03FF_FFFF) << 12)
            | ((self.y as i64) & 0xFFF)
    }

    /// Decodes a [`BlockVec`] from Minecraft's packed 64-bit `BLOCK_POSITION` integer
    /// with proper two's-complement sign extension.
    #[must_use]
    pub const fn from_packed_i64(packed: i64) -> Self {
        let x = (packed >> 38) as i32;
        let z = ((packed << 26) >> 38) as i32;
        let y = ((packed << 52) >> 52) as i32;
        Self { x, y, z }
    }
}

impl Point for BlockVec {
    fn x(&self) -> f64 {
        f64::from(self.x)
    }
    fn y(&self) -> f64 {
        f64::from(self.y)
    }
    fn z(&self) -> f64 {
        f64::from(self.z)
    }
    fn block_x(&self) -> i32 {
        self.x
    }
    fn block_y(&self) -> i32 {
        self.y
    }
    fn block_z(&self) -> i32 {
        self.z
    }
}

/// 2D chunk column coordinate `(chunk_x, chunk_z)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ChunkPos {
    /// Chunk X coordinate (`block_x >> 4`).
    pub x: i32,
    /// Chunk Z coordinate (`block_z >> 4`).
    pub z: i32,
}

impl ChunkPos {
    /// Creates a `ChunkPos`.
    #[must_use]
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    /// Chebyshev (`max(|dx|, |dz|)`) distance between two chunk coordinates.
    #[must_use]
    pub fn chebyshev_distance(self, other: Self) -> u32 {
        self.x.abs_diff(other.x).max(self.z.abs_diff(other.z))
    }

    /// Returns all chunk coordinates within Chebyshev `radius` of `self`, ordered
    /// outward from the center chunk so closest chunks stream to players first.
    #[must_use]
    pub fn chunks_in_range(self, radius: u8) -> Vec<Self> {
        let r = i32::from(radius);
        let side = (2 * r + 1) as usize;
        let mut out = Vec::with_capacity(side * side);
        out.push(self);
        for ring in 1..=r {
            for dx in -ring..=ring {
                out.push(Self::new(self.x + dx, self.z - ring));
                out.push(Self::new(self.x + dx, self.z + ring));
            }
            for dz in (-ring + 1)..ring {
                out.push(Self::new(self.x - ring, self.z + dz));
                out.push(Self::new(self.x + ring, self.z + dz));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn packed_block_vec_roundtrip(
            x in -33_554_432_i32..=33_554_431_i32,
            y in -2_048_i32..=2_047_i32,
            z in -33_554_432_i32..=33_554_431_i32,
        ) {
            let orig = BlockVec::new(x, y, z);
            let packed = orig.to_packed_i64();
            let decoded = BlockVec::from_packed_i64(packed);
            prop_assert_eq!(decoded, orig);
        }
    }

    #[test]
    fn chunk_range_count_and_center_first() {
        let center = ChunkPos::new(3, -5);
        let chunks = center.chunks_in_range(2);
        assert_eq!(chunks.len(), 25);
        assert_eq!(chunks[0], center);
    }
}
