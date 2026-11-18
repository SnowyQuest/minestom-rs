//! Deterministic tick-phase `Scheduler`, `Task`, and `TaskSchedule` for `minestom-rs`.

/// Phase of a server tick during which a scheduled task executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExecutionType {
    /// Run at the start of the tick before entity and chunk updates.
    #[default]
    TickStart,
    /// Run at the end of the tick after entity and chunk updates.
    TickEnd,
}
