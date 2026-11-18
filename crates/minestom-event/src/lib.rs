//! Composable `EventNode` tree, event filters, priorities, expiring listeners,
//! and zero-allocation cached `ListenerHandle` dispatch for `minestom-rs`.

use std::any::Any;

/// Marker trait implemented by all synchronous and asynchronous server events.
pub trait Event: Any + Send + Sync + 'static {}

/// Trait implemented by events whose default outcome can be cancelled by listeners.
pub trait CancellableEvent: Event {
    /// Returns `true` if a prior listener marked this event as cancelled.
    fn is_cancelled(&self) -> bool;

    /// Marks this event as cancelled or uncancelled.
    fn set_cancelled(&mut self, cancelled: bool);
}
