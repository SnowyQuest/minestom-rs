//! `ItemStack`, data components, `Material`, `Inventory`, and window click processing for `minestom-rs`.

/// Errors returned by item and inventory operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ItemError {
    /// Slot index out of bounds for an inventory.
    #[error("inventory slot {slot} out of bounds (size {size})")]
    SlotOutOfBounds {
        /// Requested slot index.
        slot: usize,
        /// Total slot capacity.
        size: usize,
    },
}
