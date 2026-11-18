//! Opt-in 3D BFS sky & block light propagation (`LightingChunk` / `LightCompute` equivalent)
//! producing packed 2048-byte nibble arrays and protocol [`LightData`] / [`UpdateLightPacket`]s.

use std::collections::VecDeque;

use minestom_block::Block;
use minestom_protocol::play::{LightData, UpdateLightPacket};

use crate::Chunk;

/// Packs 4,096 4-bit light levels (`0..=15`) indexed by `(y << 8) | (z << 4) | x` into a
/// 2,048-byte Minecraft nibble array (even index in low nibble, odd index in high nibble).
#[must_use]
pub fn pack_light_nibbles(levels: &[u8]) -> Vec<u8> {
    let mut out = vec![0_u8; 2048];
    for (byte_idx, slot) in out.iter_mut().enumerate() {
        let lo = levels.get(byte_idx * 2).copied().unwrap_or(0) & 0x0F;
        let hi = levels.get(byte_idx * 2 + 1).copied().unwrap_or(0) & 0x0F;
        *slot = lo | (hi << 4);
    }
    out
}

/// Unpacks a 2,048-byte Minecraft nibble array into 4,096 4-bit light levels (`0..=15`).
#[must_use]
pub fn unpack_light_nibbles(bytes: &[u8]) -> [u8; 4096] {
    let mut levels = [0_u8; 4096];
    if bytes.len() < 2048 {
        return levels;
    }
    for (byte_idx, &b) in bytes[..2048].iter().enumerate() {
        levels[byte_idx * 2] = b & 0x0F;
        levels[byte_idx * 2 + 1] = (b >> 4) & 0x0F;
    }
    levels
}

#[inline]
const fn flat_idx(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

/// Computes full 3D sky light and block light across all sections of `chunk` using 6-neighbor BFS
/// propagation, returning protocol-ready [`LightData`].
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn compute_chunk_light(chunk: &Chunk) -> LightData {
    let sec_count = chunk.sections().len();
    let total_y = sec_count * 16;
    let total_voxels = total_y * 256;

    // Pre-extract opacity (`0..=15`) & emission (`0..=15`) in a single flat pass for fast BFS!
    let mut opacities = vec![0_u8; total_voxels];
    let mut sky = vec![0_u8; total_voxels];
    let mut block_light = vec![0_u8; total_voxels];
    let mut block_queue: VecDeque<(usize, usize, usize, u8)> = VecDeque::new();

    for (sec_idx, sec) in chunk.sections().iter().enumerate() {
        let base_y = sec_idx * 16;
        if sec.block_count() == 0 {
            continue;
        }
        if let Some(single_id) = sec.blocks().single_value() {
            let b = Block::from_state_id(single_id as u16).unwrap_or(Block::AIR);
            let op = b.light_opacity();
            let em = b.light_emission();
            if op > 0 || em > 0 {
                let start = base_y * 256;
                let end = start + 4096;
                if op > 0 {
                    opacities[start..end].fill(op);
                }
                if em > 0 {
                    block_light[start..end].fill(em);
                    for ly in 0..16 {
                        for lz in 0..16 {
                            for lx in 0..16 {
                                block_queue.push_back((lx, base_y + ly, lz, em));
                            }
                        }
                    }
                }
            }
        } else {
            for ly in 0..16 {
                let wy = base_y + ly;
                for lz in 0..16 {
                    for lx in 0..16 {
                        let b = sec.get_block(lx, ly, lz);
                        let v_idx = flat_idx(lx, wy, lz);
                        let op = b.light_opacity();
                        let em = b.light_emission();
                        opacities[v_idx] = op;
                        if em > 0 {
                            block_light[v_idx] = em;
                            block_queue.push_back((lx, wy, lz, em));
                        }
                    }
                }
            }
        }
    }

    // 1. Sky Light: cast vertical sunlight (`15`) down each `(x, z)` column, then 6-neighbor BFS!
    let mut sky_queue: VecDeque<(usize, usize, usize, u8)> = VecDeque::new();
    for lz in 0..16 {
        for lx in 0..16 {
            let mut level = 15_u8;
            for wy in (0..total_y).rev() {
                let v_idx = flat_idx(lx, wy, lz);
                let op = opacities[v_idx];
                if op >= 15 {
                    break;
                }
                level = level.saturating_sub(op);
                if level == 0 {
                    break;
                }
                sky[v_idx] = level;
                // Only enqueue column voxels where horizontal neighbors might be shadowed or semi-opaque
                if level < 15
                    || (lx > 0 && opacities[flat_idx(lx - 1, wy, lz)] > 0)
                    || (lx < 15 && opacities[flat_idx(lx + 1, wy, lz)] > 0)
                    || (lz > 0 && opacities[flat_idx(lx, wy, lz - 1)] > 0)
                    || (lz < 15 && opacities[flat_idx(lx, wy, lz + 1)] > 0)
                    || (wy > 0 && opacities[flat_idx(lx, wy - 1, lz)] >= 15)
                {
                    sky_queue.push_back((lx, wy, lz, level));
                }
            }
        }
    }

    // Propagate sky light into overhangs/caves
    bfs_propagate(&mut sky, &opacities, total_y, &mut sky_queue);
    // Propagate block light from emissive blocks (torches, glowstone, lanterns, lava)
    bfs_propagate(&mut block_light, &opacities, total_y, &mut block_queue);

    // Pack into LightData (`section_count + 2` light sections: 0 = below, 1..=sec_count, sec_count+1 = above)
    let total_light_sections = sec_count + 2;
    let num_words = total_light_sections.div_ceil(64);
    let mut sky_mask = vec![0_u64; num_words];
    let mut block_mask = vec![0_u64; num_words];
    let mut empty_sky_mask = vec![0_u64; num_words];
    let mut empty_block_mask = vec![0_u64; num_words];
    let mut sky_arrays = Vec::new();
    let mut block_arrays = Vec::new();

    // Light section 0 (below min_y): empty sky & block light
    empty_sky_mask[0] |= 1;
    empty_block_mask[0] |= 1;

    for sec_idx in 0..sec_count {
        let light_bit = sec_idx + 1;
        let slice_start = sec_idx * 4096;
        let slice_end = slice_start + 4096;

        let sky_sec = &sky[slice_start..slice_end];
        if sky_sec.iter().any(|&v| v > 0) {
            sky_mask[light_bit / 64] |= 1_u64 << (light_bit % 64);
            sky_arrays.push(pack_light_nibbles(sky_sec));
        } else {
            empty_sky_mask[light_bit / 64] |= 1_u64 << (light_bit % 64);
        }

        let blk_sec = &block_light[slice_start..slice_end];
        if blk_sec.iter().any(|&v| v > 0) {
            block_mask[light_bit / 64] |= 1_u64 << (light_bit % 64);
            block_arrays.push(pack_light_nibbles(blk_sec));
        } else {
            empty_block_mask[light_bit / 64] |= 1_u64 << (light_bit % 64);
        }
    }

    // Light section `sec_count + 1` (above max_y): full sky light (15), empty block light
    let top_bit = sec_count + 1;
    sky_mask[top_bit / 64] |= 1_u64 << (top_bit % 64);
    sky_arrays.push(vec![0xFF_u8; 2048]);
    empty_block_mask[top_bit / 64] |= 1_u64 << (top_bit % 64);

    LightData {
        sky_mask,
        block_mask,
        empty_sky_mask,
        empty_block_mask,
        sky_light: sky_arrays,
        block_light: block_arrays,
    }
}

fn bfs_propagate(
    grid: &mut [u8],
    opacities: &[u8],
    total_y: usize,
    queue: &mut VecDeque<(usize, usize, usize, u8)>,
) {
    const DIRS: [(isize, isize, isize); 6] = [
        (-1, 0, 0),
        (1, 0, 0),
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
    ];
    while let Some((x, y, z, cur)) = queue.pop_front() {
        if cur <= 1 {
            continue;
        }
        let v_cur = grid[flat_idx(x, y, z)];
        if cur < v_cur {
            continue;
        }
        for (dx, dy, dz) in DIRS {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            let nz = z as isize + dz;
            if !(0..16).contains(&nx)
                || !(0..total_y as isize).contains(&ny)
                || !(0..16).contains(&nz)
            {
                continue;
            }
            let (ux, uy, uz) = (nx as usize, ny as usize, nz as usize);
            let n_idx = flat_idx(ux, uy, uz);
            let step_cost = opacities[n_idx].max(1);
            if cur <= step_cost {
                continue;
            }
            let next_lvl = cur - step_cost;
            if next_lvl > grid[n_idx] {
                grid[n_idx] = next_lvl;
                if next_lvl > 1 {
                    queue.push_back((ux, uy, uz, next_lvl));
                }
            }
        }
    }
}

/// Builds an [`UpdateLightPacket`] (`0x30`) by running [`compute_chunk_light`] on `chunk`.
#[must_use]
pub fn build_update_light_packet(chunk: &Chunk) -> UpdateLightPacket {
    UpdateLightPacket {
        chunk_x: chunk.pos().x,
        chunk_z: chunk.pos().z,
        light_data: compute_chunk_light(chunk),
    }
}

#[cfg(test)]
mod tests {
    use minestom_core::ChunkPos;
    use minestom_registry::DimensionType;

    use super::*;

    #[test]
    fn bfs_sky_and_block_light_attenuates_under_roof_and_around_glowstone() {
        let dim = DimensionType::overworld();
        let mut chunk = Chunk::new(ChunkPos::new(0, 0), &dim);
        // Solid stone floor at y = 0 and opaque roof at y = 10
        chunk.fill_height(0, 1, Block::STONE);
        chunk.fill_height(10, 11, Block::STONE);
        // Place a GLOWSTONE (emission = 15) at (8, 2, 8) under the roof!
        chunk.set_block(8, 2, 8, Block::GLOWSTONE);

        let light = compute_chunk_light(&chunk);
        // Section index for y = 0..16 in Overworld (min_y = -64) is `sec_idx = 4` -> light bit `5`.
        let bit = 5_usize;
        assert_ne!(light.block_mask[bit / 64] & (1_u64 << (bit % 64)), 0);

        // Find which entry in `light.block_light` corresponds to light_bit 5
        let mut arr_idx = 0;
        for b in 0..bit {
            if (light.block_mask[b / 64] & (1_u64 << (b % 64))) != 0 {
                arr_idx += 1;
            }
        }
        let unpacked = unpack_light_nibbles(&light.block_light[arr_idx]);
        // At (8, 2, 8) emission = 15; 1 block away (9, 2, 8) = 14; 5 blocks away (13, 2, 8) = 10!
        assert_eq!(unpacked[flat_idx(8, 2, 8)], 15);
        assert_eq!(unpacked[flat_idx(9, 2, 8)], 14);
        assert_eq!(unpacked[flat_idx(13, 2, 8)], 10);
        // Solid floor at (8, 0, 8) has opacity 15 -> block light = 0
        assert_eq!(unpacked[flat_idx(8, 0, 8)], 0);
    }
}
