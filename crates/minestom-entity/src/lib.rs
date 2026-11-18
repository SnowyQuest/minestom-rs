//! Compositional `Entity`, `LivingEntity`, `Player`, `EntityCreature`, metadata,
//! AABB collision/physics, spatial `EntityTracker`, and `Goal`/`Target` AI traits.

/// Errors returned by entity operations.
#[derive(Debug, thiserror::Error)]
pub enum EntityError {
    /// Referenced entity handle is no longer alive.
    #[error("entity no longer exists")]
    Despawned,
}
