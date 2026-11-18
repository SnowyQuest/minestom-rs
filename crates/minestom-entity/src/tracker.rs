//! Chunk-bucketed spatial `EntityTracker` and per-entity viewer set (`EntityView` in Minestom).

use minestom_core::{ChunkPos, InstanceId, PlayerId, Point, Pos};
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Clone)]
struct TrackedSubject {
    instance_id: InstanceId,
    chunk_pos: ChunkPos,
    position: Pos,
    viewers: FxHashSet<PlayerId>,
}

/// Viewer set delta when reconciling which entities a player can see.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ViewerReconcileDiff {
    /// Network entity IDs that just entered the player's view range.
    pub entered: Vec<i32>,
    /// Network entity IDs that just left the player's view range.
    pub exited: Vec<i32>,
}

/// Spatial chunk-indexed tracker mapping `(InstanceId, ChunkPos)` to active network entity IDs
/// (`EntityTrackerImpl` in Minestom, zero full-world scans!).
#[derive(Debug, Default)]
pub struct EntityTracker {
    subjects: FxHashMap<i32, TrackedSubject>,
    chunk_buckets: FxHashMap<(InstanceId, ChunkPos), FxHashSet<i32>>,
    player_viewed_entities: FxHashMap<PlayerId, FxHashSet<i32>>,
}

impl EntityTracker {
    /// Creates an empty [`EntityTracker`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a newly spawned entity or player `entity_id` at `(instance_id, pos)`.
    pub fn register(&mut self, entity_id: i32, instance_id: InstanceId, pos: Pos) {
        let cpos = pos.chunk_pos();
        self.subjects.insert(
            entity_id,
            TrackedSubject {
                instance_id,
                chunk_pos: cpos,
                position: pos,
                viewers: FxHashSet::default(),
            },
        );
        self.chunk_buckets
            .entry((instance_id, cpos))
            .or_default()
            .insert(entity_id);
    }

    /// Removes `entity_id` from spatial buckets and returns all [`PlayerId`]s that were viewing it
    /// (so callers can send `DestroyEntitiesPacket`).
    pub fn unregister(&mut self, entity_id: i32) -> Vec<PlayerId> {
        let Some(removed) = self.subjects.remove(&entity_id) else {
            return Vec::new();
        };
        if let Some(bucket) = self
            .chunk_buckets
            .get_mut(&(removed.instance_id, removed.chunk_pos))
        {
            bucket.remove(&entity_id);
            if bucket.is_empty() {
                self.chunk_buckets
                    .remove(&(removed.instance_id, removed.chunk_pos));
            }
        }
        let mut viewers = Vec::with_capacity(removed.viewers.len());
        for pid in removed.viewers {
            if let Some(set) = self.player_viewed_entities.get_mut(&pid) {
                set.remove(&entity_id);
            }
            viewers.push(pid);
        }
        viewers
    }

    /// Removes a disconnecting viewer `player_id` from all entity viewer sets.
    pub fn remove_viewer(&mut self, player_id: PlayerId) {
        if let Some(viewed) = self.player_viewed_entities.remove(&player_id) {
            for eid in viewed {
                if let Some(sub) = self.subjects.get_mut(&eid) {
                    sub.viewers.remove(&player_id);
                }
            }
        }
    }

    /// Updates `entity_id`'s position and moves it across chunk buckets in `O(1)` if needed.
    /// Returns `true` if its `(InstanceId, ChunkPos)` bucket changed.
    pub fn update_position(
        &mut self,
        entity_id: i32,
        instance_id: InstanceId,
        new_pos: Pos,
    ) -> bool {
        let new_chunk = new_pos.chunk_pos();
        let Some(sub) = self.subjects.get_mut(&entity_id) else {
            return false;
        };
        sub.position = new_pos;
        if sub.instance_id == instance_id && sub.chunk_pos == new_chunk {
            return false;
        }

        let old_key = (sub.instance_id, sub.chunk_pos);
        if let Some(bucket) = self.chunk_buckets.get_mut(&old_key) {
            bucket.remove(&entity_id);
            if bucket.is_empty() {
                self.chunk_buckets.remove(&old_key);
            }
        }

        sub.instance_id = instance_id;
        sub.chunk_pos = new_chunk;
        self.chunk_buckets
            .entry((instance_id, new_chunk))
            .or_default()
            .insert(entity_id);
        true
    }

    /// Borrows the current [`PlayerId`] viewers watching `entity_id`.
    #[must_use]
    pub fn viewers(&self, entity_id: i32) -> Option<&FxHashSet<PlayerId>> {
        self.subjects.get(&entity_id).map(|s| &s.viewers)
    }

    /// Returns all entity IDs inside `chunk_pos` of `instance_id`.
    #[must_use]
    pub fn entities_in_chunk(&self, instance_id: InstanceId, chunk_pos: ChunkPos) -> Vec<i32> {
        self.chunk_buckets
            .get(&(instance_id, chunk_pos))
            .map_or_else(Vec::new, |set| set.iter().copied().collect())
    }

    /// Returns all entity IDs within Chebyshev `chunk_radius` around `center` in `instance_id`.
    #[must_use]
    pub fn entities_in_chunk_range(
        &self,
        instance_id: InstanceId,
        center: ChunkPos,
        chunk_radius: u8,
    ) -> Vec<i32> {
        let r = i32::from(chunk_radius);
        let mut out = Vec::new();
        for dz in -r..=r {
            for dx in -r..=r {
                let c = ChunkPos::new(center.x + dx, center.z + dz);
                if let Some(bucket) = self.chunk_buckets.get(&(instance_id, c)) {
                    out.extend(bucket.iter().copied());
                }
            }
        }
        out
    }

    /// Returns all entity IDs within Euclidean `max_distance` blocks of `origin` in `instance_id`.
    #[must_use]
    pub fn nearby_entities(
        &self,
        instance_id: InstanceId,
        origin: Pos,
        max_distance: f64,
    ) -> Vec<i32> {
        let chunk_radius = ((max_distance / 16.0).ceil() as u8).saturating_add(1);
        let max_sq = max_distance * max_distance;
        let center = origin.chunk_pos();
        let mut out = Vec::new();
        for eid in self.entities_in_chunk_range(instance_id, center, chunk_radius) {
            if let Some(sub) = self.subjects.get(&eid)
                && sub.position.distance_squared(&origin) <= max_sq
            {
                out.push(eid);
            }
        }
        out
    }

    /// Reconciles which entities `viewer_pid` (having own network ID `self_entity_id`) should see
    /// around `(instance_id, center_chunk)` within `view_distance` chunks, updating bidirectional
    /// viewer sets and returning newly entered & exited entity IDs!
    pub fn reconcile_viewer(
        &mut self,
        viewer_pid: PlayerId,
        self_entity_id: i32,
        instance_id: InstanceId,
        center_chunk: ChunkPos,
        view_distance: u8,
    ) -> ViewerReconcileDiff {
        let candidates: FxHashSet<i32> = self
            .entities_in_chunk_range(instance_id, center_chunk, view_distance)
            .into_iter()
            .filter(|&eid| eid != self_entity_id)
            .collect();

        let prev = self.player_viewed_entities.entry(viewer_pid).or_default();

        let mut entered = Vec::new();
        let mut exited = Vec::new();

        for &eid in prev.iter() {
            if !candidates.contains(&eid) {
                exited.push(eid);
            }
        }
        for &eid in &candidates {
            if !prev.contains(&eid) {
                entered.push(eid);
            }
        }

        for &eid in &exited {
            prev.remove(&eid);
            if let Some(sub) = self.subjects.get_mut(&eid) {
                sub.viewers.remove(&viewer_pid);
            }
        }
        for &eid in &entered {
            prev.insert(eid);
            if let Some(sub) = self.subjects.get_mut(&eid) {
                sub.viewers.insert(viewer_pid);
            }
        }

        ViewerReconcileDiff { entered, exited }
    }
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use super::*;

    #[test]
    fn spatial_tracker_reconciles_enter_and_exit_without_world_scan() {
        let mut tracker = EntityTracker::new();
        let inst = InstanceId::default();
        let mut dummy: SlotMap<PlayerId, ()> = SlotMap::with_key();
        let p1 = dummy.insert(());

        tracker.register(1, inst, Pos::new(0.0, 42.0, 0.0));
        tracker.register(2, inst, Pos::new(8.0, 42.0, 8.0));
        tracker.register(3, inst, Pos::new(200.0, 42.0, 200.0));

        let d1 = tracker.reconcile_viewer(p1, 1, inst, ChunkPos::new(0, 0), 2);
        assert_eq!(d1.entered, vec![2]);
        assert_eq!(d1.exited, Vec::<i32>::new());
        assert!(tracker.viewers(2).unwrap().contains(&p1));

        // Entity 2 walks far away into chunk (20, 20)
        assert!(tracker.update_position(2, inst, Pos::new(320.0, 42.0, 320.0)));
        let d2 = tracker.reconcile_viewer(p1, 1, inst, ChunkPos::new(0, 0), 2);
        assert_eq!(d2.entered, Vec::<i32>::new());
        assert_eq!(d2.exited, vec![2]);
    }
}
