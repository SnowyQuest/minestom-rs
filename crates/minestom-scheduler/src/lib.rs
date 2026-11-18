//! Deterministic tick-phase `Scheduler`, `SchedulerManager`, `TaskHandle`, and `TaskSchedule`
//! for `minestom-rs` (`net.minestom.server.timer` equivalent).
//!
//! Tasks execute deterministically on the 20 TPS tick loop during either [`ExecutionType::TickStart`]
//! or [`ExecutionType::TickEnd`], never blocking Tokio network workers.
//!
//! ```
//! use std::sync::Arc;
//! use std::sync::atomic::{AtomicUsize, Ordering};
//! use minestom_scheduler::{ExecutionType, Scheduler, TaskSchedule};
//!
//! let scheduler = Scheduler::new();
//! let counter = Arc::new(AtomicUsize::new(0));
//! let c = Arc::clone(&counter);
//!
//! let handle = scheduler
//!     .build_task(move || {
//!         c.fetch_add(1, Ordering::SeqCst);
//!     })
//!     .delay(TaskSchedule::Tick(2))
//!     .repeat(TaskSchedule::Tick(3))
//!     .execution_type(ExecutionType::TickEnd)
//!     .schedule();
//!
//! scheduler.process_tick_end(); // tick 1 -> waiting on delay
//! assert_eq!(counter.load(Ordering::SeqCst), 0);
//! scheduler.process_tick_end(); // tick 2 -> fires!
//! assert_eq!(counter.load(Ordering::SeqCst), 1);
//! handle.cancel();
//! assert!(!handle.is_alive());
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

/// Phase of a server tick during which a scheduled task executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExecutionType {
    /// Run at the start of the tick before packet/entity/chunk updates.
    #[default]
    TickStart,
    /// Run at the end of the tick after entity physics and world updates.
    TickEnd,
}

/// Scheduling interval for task delays and repeats (`TaskSchedule` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskSchedule {
    /// Execute on the current tick pass (`0` tick delay).
    Immediate,
    /// Execute on the next tick (`1` tick).
    NextTick,
    /// Execute after `n` server ticks (`0` behaves like [`Self::Immediate`]).
    Tick(u32),
    /// Execute after a wall-clock [`Duration`], quantized up to 20 TPS server ticks (`50ms` per tick).
    Duration(Duration),
    /// Do not reschedule; terminate the task cleanly.
    Stop,
}

impl TaskSchedule {
    /// Schedules after `ticks` server ticks.
    #[must_use]
    pub const fn tick(ticks: u32) -> Self {
        Self::Tick(ticks)
    }

    /// Schedules after `seconds` seconds (at 20 TPS = `seconds * 20` ticks).
    #[must_use]
    pub const fn seconds(seconds: u64) -> Self {
        Self::Duration(Duration::from_secs(seconds))
    }

    /// Schedules after `millis` milliseconds.
    #[must_use]
    pub const fn millis(millis: u64) -> Self {
        Self::Duration(Duration::from_millis(millis))
    }

    /// Converts this schedule into a tick count given `ticks_per_second` (`None` if [`Self::Stop`]).
    #[must_use]
    pub fn to_ticks(self, ticks_per_second: u32) -> Option<u64> {
        let tps = u64::from(ticks_per_second.max(1));
        match self {
            Self::Immediate => Some(0),
            Self::NextTick => Some(1),
            Self::Tick(n) => Some(u64::from(n)),
            Self::Duration(dur) => {
                if dur.is_zero() {
                    return Some(0);
                }
                let nanos = dur.as_nanos();
                let tick_nanos = u128::from(1_000_000_000_u64 / tps.max(1));
                let ticks = nanos.div_ceil(tick_nanos.max(1)) as u64;
                Some(ticks.max(1))
            }
            Self::Stop => None,
        }
    }
}

/// Unique numeric identifier of a scheduled task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(pub u64);

/// Cloneable handle for inspecting or cancelling a scheduled task (`Task` in Minestom).
#[derive(Debug, Clone)]
pub struct TaskHandle {
    id: TaskId,
    alive: Arc<AtomicBool>,
}

impl TaskHandle {
    /// Returns this task's [`TaskId`].
    #[must_use]
    pub const fn id(&self) -> TaskId {
        self.id
    }

    /// Returns `true` if this task is still scheduled and has not finished or been cancelled.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Cancels this task immediately so it will not run again.
    pub fn cancel(&self) {
        self.alive.store(false, Ordering::Release);
    }
}

type DynamicTaskFn = Box<dyn FnMut() -> TaskSchedule + Send + 'static>;

struct ScheduledEntry {
    id: TaskId,
    execution_type: ExecutionType,
    remaining_ticks: u64,
    alive: Arc<AtomicBool>,
    callback: DynamicTaskFn,
}

struct SchedulerInner {
    ticks_per_second: u32,
    pending: Vec<ScheduledEntry>,
    active: Vec<ScheduledEntry>,
    shutdown_tasks: Vec<Box<dyn FnOnce() + Send + 'static>>,
}

/// Deterministic tick-aligned task scheduler (`Scheduler` in Minestom).
#[derive(Clone)]
pub struct Scheduler {
    next_id: Arc<AtomicU64>,
    inner: Arc<Mutex<SchedulerInner>>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.lock();
        f.debug_struct("Scheduler")
            .field("tps", &g.ticks_per_second)
            .field("active_tasks", &(g.active.len() + g.pending.len()))
            .finish_non_exhaustive()
    }
}

impl Scheduler {
    /// Creates a new [`Scheduler`] targeting 20 TPS.
    #[must_use]
    pub fn new() -> Self {
        Self::with_tps(20)
    }

    /// Creates a new [`Scheduler`] quantized to `ticks_per_second`.
    #[must_use]
    pub fn with_tps(ticks_per_second: u32) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(1)),
            inner: Arc::new(Mutex::new(SchedulerInner {
                ticks_per_second: ticks_per_second.max(1),
                pending: Vec::new(),
                active: Vec::new(),
                shutdown_tasks: Vec::new(),
            })),
        }
    }

    /// Returns the number of currently queued/active tasks.
    #[must_use]
    pub fn active_task_count(&self) -> usize {
        let g = self.inner.lock();
        g.active
            .iter()
            .chain(g.pending.iter())
            .filter(|e| e.alive.load(Ordering::Acquire))
            .count()
    }

    /// Starts building a task with fluent `.delay(...)`, `.repeat(...)`, and `.execution_type(...)`.
    #[must_use]
    pub fn build_task<F>(&self, action: F) -> TaskBuilder<F>
    where
        F: FnMut() + Send + 'static,
    {
        TaskBuilder {
            scheduler: self.clone(),
            action,
            delay: TaskSchedule::Immediate,
            repeat: TaskSchedule::Stop,
            execution_type: ExecutionType::TickStart,
        }
    }

    /// Schedules a one-shot closure to run on the next [`ExecutionType::TickStart`].
    pub fn schedule_next_tick<F>(&self, action: F) -> TaskHandle
    where
        F: FnOnce() + Send + 'static,
    {
        let mut opt = Some(action);
        self.submit_task(
            ExecutionType::TickStart,
            TaskSchedule::NextTick,
            move || {
                if let Some(f) = opt.take() {
                    f();
                }
                TaskSchedule::Stop
            },
        )
    }

    /// Submits a dynamic task whose callback returns the next [`TaskSchedule`] after each run!
    pub fn submit_task<F>(
        &self,
        execution_type: ExecutionType,
        initial_delay: TaskSchedule,
        callback: F,
    ) -> TaskHandle
    where
        F: FnMut() -> TaskSchedule + Send + 'static,
    {
        let id = TaskId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let mut g = self.inner.lock();
        let Some(initial_ticks) = initial_delay.to_ticks(g.ticks_per_second) else {
            return TaskHandle {
                id,
                alive: Arc::new(AtomicBool::new(false)),
            };
        };
        let alive = Arc::new(AtomicBool::new(true));
        let handle = TaskHandle {
            id,
            alive: Arc::clone(&alive),
        };
        g.pending.push(ScheduledEntry {
            id,
            execution_type,
            remaining_ticks: initial_ticks,
            alive,
            callback: Box::new(callback),
        });
        handle
    }

    /// Registers a callback executed once when [`Self::shutdown`] is invoked (`buildShutdownTask` in Minestom).
    pub fn add_shutdown_task<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.inner.lock().shutdown_tasks.push(Box::new(task));
    }

    /// Advances and executes all due [`ExecutionType::TickStart`] tasks by 1 tick.
    pub fn process_tick_start(&self) {
        self.process_phase(ExecutionType::TickStart);
    }

    /// Advances and executes all due [`ExecutionType::TickEnd`] tasks by 1 tick.
    pub fn process_tick_end(&self) {
        self.process_phase(ExecutionType::TickEnd);
    }

    /// Advances and executes all due tasks matching `phase` by 1 tick without holding the scheduler
    /// mutex while user callbacks run (so tasks can safely schedule new tasks re-entrantly!).
    pub fn process_phase(&self, phase: ExecutionType) {
        let (tps, mut batch) = {
            let mut g = self.inner.lock();
            let pending = std::mem::take(&mut g.pending);
            g.active.extend(pending);
            g.active.retain(|e| e.alive.load(Ordering::Acquire));

            let mut due_and_staying = Vec::with_capacity(g.active.len());
            let mut other_phase = Vec::new();
            for mut entry in g.active.drain(..) {
                if entry.execution_type == phase {
                    entry.remaining_ticks = entry.remaining_ticks.saturating_sub(1);
                    due_and_staying.push(entry);
                } else {
                    other_phase.push(entry);
                }
            }
            g.active = other_phase;
            (g.ticks_per_second, due_and_staying)
        };

        let mut keep = Vec::with_capacity(batch.len());
        for mut entry in batch.drain(..) {
            if !entry.alive.load(Ordering::Acquire) {
                continue;
            }
            if entry.remaining_ticks == 0 {
                let next_sched = (entry.callback)();
                if !entry.alive.load(Ordering::Acquire) {
                    continue;
                }
                match next_sched.to_ticks(tps) {
                    Some(next_ticks) => {
                        // A repeat of 0/1 ticks runs on the next tick!
                        entry.remaining_ticks = next_ticks.max(1);
                        keep.push(entry);
                    }
                    None => {
                        entry.alive.store(false, Ordering::Release);
                    }
                }
            } else {
                keep.push(entry);
            }
        }

        let mut g = self.inner.lock();
        g.active.extend(keep);
        g.active.sort_by_key(|e| e.id);
    }

    /// Cancels all active tasks and runs every registered shutdown task once.
    pub fn shutdown(&self) {
        let shutdown_fns = {
            let mut g = self.inner.lock();
            let act = std::mem::take(&mut g.active);
            let pend = std::mem::take(&mut g.pending);
            for e in act.into_iter().chain(pend) {
                e.alive.store(false, Ordering::Release);
            }
            std::mem::take(&mut g.shutdown_tasks)
        };
        for f in shutdown_fns {
            f();
        }
    }
}

/// Fluent builder returned by [`Scheduler::build_task`].
pub struct TaskBuilder<F> {
    scheduler: Scheduler,
    action: F,
    delay: TaskSchedule,
    repeat: TaskSchedule,
    execution_type: ExecutionType,
}

impl<F> std::fmt::Debug for TaskBuilder<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskBuilder")
            .field("delay", &self.delay)
            .field("repeat", &self.repeat)
            .field("execution_type", &self.execution_type)
            .finish_non_exhaustive()
    }
}

impl<F> TaskBuilder<F>
where
    F: FnMut() + Send + 'static,
{
    /// Sets initial delay before the first execution (default [`TaskSchedule::Immediate`]).
    #[must_use]
    pub const fn delay(mut self, delay: TaskSchedule) -> Self {
        self.delay = delay;
        self
    }

    /// Sets repeating interval after each execution (default [`TaskSchedule::Stop`]).
    #[must_use]
    pub const fn repeat(mut self, repeat: TaskSchedule) -> Self {
        self.repeat = repeat;
        self
    }

    /// Selects [`ExecutionType::TickStart`] or [`ExecutionType::TickEnd`].
    #[must_use]
    pub const fn execution_type(mut self, execution_type: ExecutionType) -> Self {
        self.execution_type = execution_type;
        self
    }

    /// Registers the configured task onto the [`Scheduler`] and returns its [`TaskHandle`].
    pub fn schedule(self) -> TaskHandle {
        let mut action = self.action;
        let repeat = self.repeat;
        self.scheduler
            .submit_task(self.execution_type, self.delay, move || {
                action();
                repeat
            })
    }
}

/// Server-wide scheduler manager (`SchedulerManager` in Minestom) delegating to a root [`Scheduler`].
#[derive(Debug, Clone, Default)]
pub struct SchedulerManager {
    scheduler: Scheduler,
}

impl SchedulerManager {
    /// Creates a new `SchedulerManager` quantized to `ticks_per_second`.
    #[must_use]
    pub fn with_tps(ticks_per_second: u32) -> Self {
        Self {
            scheduler: Scheduler::with_tps(ticks_per_second),
        }
    }

    /// Borrows the underlying [`Scheduler`].
    #[must_use]
    pub const fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    /// Starts building a task via [`TaskBuilder`].
    #[must_use]
    pub fn build_task<F>(&self, action: F) -> TaskBuilder<F>
    where
        F: FnMut() + Send + 'static,
    {
        self.scheduler.build_task(action)
    }

    /// Schedules a one-shot task on the next [`ExecutionType::TickStart`].
    pub fn schedule_next_tick<F>(&self, action: F) -> TaskHandle
    where
        F: FnOnce() + Send + 'static,
    {
        self.scheduler.schedule_next_tick(action)
    }

    /// Submits a dynamic self-rescheduling task.
    pub fn submit_task<F>(
        &self,
        execution_type: ExecutionType,
        initial_delay: TaskSchedule,
        callback: F,
    ) -> TaskHandle
    where
        F: FnMut() -> TaskSchedule + Send + 'static,
    {
        self.scheduler
            .submit_task(execution_type, initial_delay, callback)
    }

    /// Registers a server shutdown callback.
    pub fn add_shutdown_task<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.scheduler.add_shutdown_task(task);
    }

    /// Processes [`ExecutionType::TickStart`] tasks.
    pub fn process_tick_start(&self) {
        self.scheduler.process_tick_start();
    }

    /// Processes [`ExecutionType::TickEnd`] tasks.
    pub fn process_tick_end(&self) {
        self.scheduler.process_tick_end();
    }

    /// Cancels active tasks and runs shutdown callbacks.
    pub fn shutdown(&self) {
        self.scheduler.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[test]
    fn delay_repeat_phase_isolation_reentrancy_and_shutdown_work() {
        let mgr = SchedulerManager::with_tps(20);
        let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

        let l1 = Arc::clone(&log);
        let mgr_reentrant = mgr.clone();
        mgr.build_task(move || {
            l1.lock().push("start_1");
            let l_nested = Arc::clone(&l1);
            mgr_reentrant.schedule_next_tick(move || {
                l_nested.lock().push("nested_next_tick");
            });
        })
        .execution_type(ExecutionType::TickStart)
        .schedule();

        let l2 = Arc::clone(&log);
        let rep_handle = mgr
            .build_task(move || {
                l2.lock().push("end_repeat");
            })
            .delay(TaskSchedule::Tick(2))
            .repeat(TaskSchedule::Tick(2))
            .execution_type(ExecutionType::TickEnd)
            .schedule();

        let shutdown_called = Arc::new(AtomicUsize::new(0));
        let sc = Arc::clone(&shutdown_called);
        mgr.add_shutdown_task(move || {
            sc.fetch_add(1, Ordering::SeqCst);
        });

        // Tick 1
        mgr.process_tick_start();
        mgr.process_tick_end();
        assert_eq!(*log.lock(), vec!["start_1"]);

        // Tick 2 -> nested_next_tick runs at TickStart; end_repeat fires at TickEnd!
        mgr.process_tick_start();
        mgr.process_tick_end();
        assert_eq!(
            *log.lock(),
            vec!["start_1", "nested_next_tick", "end_repeat"]
        );

        // Tick 3 -> end_repeat has 1 tick remaining
        mgr.process_tick_start();
        mgr.process_tick_end();
        assert_eq!(log.lock().len(), 3);

        // Tick 4 -> end_repeat fires a second time, then we cancel it!
        mgr.process_tick_start();
        mgr.process_tick_end();
        assert_eq!(log.lock().len(), 4);
        rep_handle.cancel();
        assert!(!rep_handle.is_alive());

        // Tick 5 -> cancelled task does not fire
        mgr.process_tick_end();
        assert_eq!(log.lock().len(), 4);

        mgr.shutdown();
        assert_eq!(shutdown_called.load(Ordering::SeqCst), 1);
    }
}
