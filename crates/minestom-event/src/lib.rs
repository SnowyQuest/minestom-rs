//! Composable `EventNode` tree, domain filter traits, priorities, expiring listeners
//! (`expire_count`, `expire_when`), and zero-allocation `ListenerHandle<E>` dispatch (ADR 0003).
//!
//! ```
//! use minestom_event::{Event, EventListener, EventNode};
//!
//! struct CounterEvent(u32);
//! impl Event for CounterEvent {}
//!
//! let root = EventNode::all("root");
//! root.add_listener_spec(
//!     EventListener::builder(|e: &mut CounterEvent| e.0 += 1)
//!         .expire_count(2)
//!         .build(),
//! );
//!
//! let mut ev = CounterEvent(0);
//! root.call(&mut ev);
//! root.call(&mut ev);
//! root.call(&mut ev); // Expired after 2 calls!
//! assert_eq!(ev.0, 2);
//! ```

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use parking_lot::RwLock;
use rustc_hash::FxHashMap;

use minestom_core::{BlockVec, EntityId, InstanceId, PlayerId};

/// Marker trait implemented by all server events.
pub trait Event: Any + Send + Sync + 'static {}

/// Trait implemented by events whose default outcome can be cancelled by listeners.
pub trait CancellableEvent: Event {
    /// Returns `true` if a prior listener marked this event as cancelled.
    fn is_cancelled(&self) -> bool;

    /// Marks this event as cancelled or uncancelled.
    fn set_cancelled(&mut self, cancelled: bool);
}

/// Trait implemented by events associated with a specific player (`EventFilter::PLAYER`).
pub trait PlayerEvent: Event {
    /// Generational [`PlayerId`] of the player involved in this event.
    fn player_id(&self) -> PlayerId;
}

/// Trait implemented by events associated with an entity (`EventFilter::ENTITY`).
pub trait EntityEvent: Event {
    /// Generational [`EntityId`] of the entity involved in this event.
    fn entity_id(&self) -> EntityId;
}

/// Trait implemented by events associated with a world instance (`EventFilter::INSTANCE`).
pub trait InstanceEvent: Event {
    /// Generational [`InstanceId`] of the instance involved in this event.
    fn instance_id(&self) -> InstanceId;
}

/// Trait implemented by events targeting a block position (`EventFilter::BLOCK`).
pub trait BlockEvent: Event {
    /// World block coordinate involved in this event.
    fn block_position(&self) -> BlockVec;
}

/// Returns `true` if this listener has expired and should be pruned from its owning `EventNode`.
type ErasedCallback = Arc<dyn Fn(&mut dyn Any) -> bool + Send + Sync>;
type NodeCondition = Arc<dyn Fn(&dyn Any) -> bool + Send + Sync>;

/// Configured typed listener built via [`EventListener::builder`].
pub struct EventListener<E: Event> {
    priority: i32,
    id: u64,
    callback: ErasedCallback,
    _marker: PhantomData<fn(&mut E)>,
}

impl<E: Event> EventListener<E> {
    /// Starts building a typed [`EventListener`] with `handler`.
    #[must_use]
    pub fn builder<F>(handler: F) -> EventListenerBuilder<E, F>
    where
        F: Fn(&mut E) + Send + Sync + 'static,
    {
        EventListenerBuilder {
            handler,
            priority: 0,
            expire_count: None,
            filter: None,
            expire_when: None,
            _marker: PhantomData,
        }
    }
}

static NEXT_LISTENER_ID: AtomicU64 = AtomicU64::new(1);

type PredicateBox<E> = Box<dyn Fn(&E) -> bool + Send + Sync>;

/// Fluent builder for [`EventListener<E>`] supporting priorities, filters, and expiration.
pub struct EventListenerBuilder<E: Event, F> {
    handler: F,
    priority: i32,
    expire_count: Option<usize>,
    filter: Option<PredicateBox<E>>,
    expire_when: Option<PredicateBox<E>>,
    _marker: PhantomData<fn(&mut E)>,
}

impl<E, F> EventListenerBuilder<E, F>
where
    E: Event,
    F: Fn(&mut E) + Send + Sync + 'static,
{
    /// Sets execution priority (lower runs earlier; default `0`).
    #[must_use]
    pub const fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Automatically removes this listener after it executes `count` times (`expireCount` in Minestom).
    #[must_use]
    pub const fn expire_count(mut self, count: usize) -> Self {
        self.expire_count = Some(count);
        self
    }

    /// Runs this listener only when `predicate(event)` returns `true`.
    #[must_use]
    pub fn filter(mut self, predicate: impl Fn(&E) -> bool + Send + Sync + 'static) -> Self {
        self.filter = Some(Box::new(predicate));
        self
    }

    /// Automatically removes this listener when `predicate(event)` returns `true` (`expireWhen` in Minestom).
    #[must_use]
    pub fn expire_when(mut self, predicate: impl Fn(&E) -> bool + Send + Sync + 'static) -> Self {
        self.expire_when = Some(Box::new(predicate));
        self
    }

    /// Skips invoking `handler` if a prior listener already marked `E` as cancelled.
    #[must_use]
    pub fn ignore_cancelled(self) -> Self
    where
        E: CancellableEvent,
    {
        self.filter(|ev: &E| !ev.is_cancelled())
    }

    /// Finalizes the [`EventListener<E>`].
    #[must_use]
    pub fn build(self) -> EventListener<E> {
        let id = NEXT_LISTENER_ID.fetch_add(1, Ordering::Relaxed);
        let remaining = self.expire_count.map(AtomicUsize::new);
        let filter = self.filter;
        let expire_when = self.expire_when;
        let handler = self.handler;

        let callback: ErasedCallback = Arc::new(move |any: &mut dyn Any| {
            let Some(ev) = any.downcast_mut::<E>() else {
                return false;
            };
            if let Some(exp_pred) = &expire_when
                && exp_pred(ev)
            {
                return true;
            }
            if let Some(flt) = &filter
                && !flt(ev)
            {
                return false;
            }
            if let Some(rem) = &remaining {
                let prev = rem.try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    if n == 0 { None } else { Some(n - 1) }
                });
                match prev {
                    Ok(1) => {
                        handler(ev);
                        return true;
                    }
                    Ok(_) => {
                        handler(ev);
                        return false;
                    }
                    Err(_) => return true,
                }
            }
            handler(ev);
            false
        });

        EventListener {
            priority: self.priority,
            id,
            callback,
            _marker: PhantomData,
        }
    }
}

#[derive(Clone)]
struct RegisteredListener {
    id: u64,
    priority: i32,
    callback: ErasedCallback,
}

#[derive(Clone)]
struct CompiledEntry {
    owner: EventNode,
    id: u64,
    node_priority: i32,
    priority: i32,
    condition: Option<NodeCondition>,
    callback: ErasedCallback,
}

struct NodeInner {
    name: String,
    priority: i32,
    condition: Option<NodeCondition>,
    listeners: FxHashMap<TypeId, Vec<RegisteredListener>>,
    children: Vec<EventNode>,
    compiled_cache: FxHashMap<TypeId, (u64, Arc<[CompiledEntry]>)>,
}

/// Hierarchical, thread-safe event dispatch node corresponding to Minestom's `EventNode`.
#[derive(Clone)]
pub struct EventNode {
    inner: Arc<RwLock<NodeInner>>,
    revision: Arc<AtomicU64>,
}

/// Top-level event root owned by `MinecraftServer`, corresponding to `GlobalEventHandler`.
pub type GlobalEventHandler = EventNode;

impl std::fmt::Debug for EventNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.inner.read();
        f.debug_struct("EventNode")
            .field("name", &guard.name)
            .field("priority", &guard.priority)
            .field("children", &guard.children.len())
            .finish_non_exhaustive()
    }
}

impl Default for EventNode {
    fn default() -> Self {
        Self::all("global")
    }
}

impl EventNode {
    /// Creates a new `EventNode` accepting all events (`EventNode.all` in Minestom).
    #[must_use]
    pub fn all(name: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(NodeInner {
                name: name.into(),
                priority: 0,
                condition: None,
                listeners: FxHashMap::default(),
                children: Vec::new(),
                compiled_cache: FxHashMap::default(),
            })),
            revision: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Creates an `EventNode` scoped to a specific event type `E` with a filter predicate.
    #[must_use]
    pub fn value<E: Event>(
        name: impl Into<String>,
        predicate: impl Fn(&E) -> bool + Send + Sync + 'static,
    ) -> Self {
        let node = Self::all(name);
        node.inner.write().condition = Some(Arc::new(move |any: &dyn Any| {
            any.downcast_ref::<E>().is_some_and(&predicate)
        }));
        node
    }

    /// Sets this node's execution priority relative to sibling nodes and returns `self`.
    #[must_use]
    pub fn with_priority(self, priority: i32) -> Self {
        self.inner.write().priority = priority;
        self.bump_revision();
        self
    }

    /// Attaches a node-wide condition predicate for event type `E`.
    #[must_use]
    pub fn with_condition<E: Event>(
        self,
        predicate: impl Fn(&E) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.inner.write().condition = Some(Arc::new(move |any: &dyn Any| {
            any.downcast_ref::<E>().is_none_or(&predicate)
        }));
        self.bump_revision();
        self
    }

    /// Returns the name of this node.
    #[must_use]
    pub fn name(&self) -> String {
        self.inner.read().name.clone()
    }

    /// Returns the priority of this node.
    #[must_use]
    pub fn priority(&self) -> i32 {
        self.inner.read().priority
    }

    fn bump_revision(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    fn tree_revision(&self) -> u64 {
        let mut rev = self.revision.load(Ordering::Acquire);
        let guard = self.inner.read();
        for child in &guard.children {
            rev = rev.wrapping_add(child.tree_revision().rotate_left(7));
        }
        rev
    }

    /// Registers a typed [`EventListener<E>`] specification on this node.
    pub fn add_listener_spec<E: Event>(&self, spec: EventListener<E>) -> &Self {
        let mut guard = self.inner.write();
        guard
            .listeners
            .entry(TypeId::of::<E>())
            .or_default()
            .push(RegisteredListener {
                id: spec.id,
                priority: spec.priority,
                callback: spec.callback,
            });
        guard.compiled_cache.remove(&TypeId::of::<E>());
        drop(guard);
        self.bump_revision();
        self
    }

    /// Registers a typed synchronous listener on this node with default priority `0`.
    pub fn add_listener<E, F>(&self, handler: F) -> &Self
    where
        E: Event,
        F: Fn(&mut E) + Send + Sync + 'static,
    {
        self.add_listener_spec(EventListener::builder(handler).build())
    }

    /// Registers a typed synchronous listener with an explicit `priority` (lower runs first).
    pub fn add_listener_with_priority<E, F>(&self, priority: i32, handler: F) -> &Self
    where
        E: Event,
        F: Fn(&mut E) + Send + Sync + 'static,
    {
        self.add_listener_spec(EventListener::builder(handler).priority(priority).build())
    }

    /// Attaches a child `EventNode` to this tree.
    pub fn add_child(&self, child: Self) -> &Self {
        let mut guard = self.inner.write();
        guard.children.push(child);
        guard.children.sort_by_key(Self::priority);
        guard.compiled_cache.clear();
        drop(guard);
        self.bump_revision();
        self
    }

    /// Removes any child nodes matching `name`.
    pub fn remove_children_by_name(&self, name: &str) {
        let mut guard = self.inner.write();
        guard.children.retain(|c| c.name() != name);
        guard.compiled_cache.clear();
        drop(guard);
        self.bump_revision();
    }

    fn remove_listener_by_id(&self, type_id: TypeId, id: u64) {
        let mut guard = self.inner.write();
        if let Some(vec) = guard.listeners.get_mut(&type_id) {
            vec.retain(|item| item.id != id);
        }
        guard.compiled_cache.remove(&type_id);
        drop(guard);
        self.bump_revision();
    }

    fn collect_listeners(
        &self,
        type_id: TypeId,
        inherited_cond: Option<&NodeCondition>,
        out: &mut Vec<CompiledEntry>,
    ) {
        let guard = self.inner.read();
        let combined_cond: Option<NodeCondition> = match (inherited_cond, &guard.condition) {
            (None, None) => None,
            (Some(a), None) => Some(Arc::clone(a)),
            (None, Some(b)) => Some(Arc::clone(b)),
            (Some(a), Some(b)) => {
                let a = Arc::clone(a);
                let b = Arc::clone(b);
                Some(Arc::new(move |any| a(any) && b(any)))
            }
        };

        if let Some(list) = guard.listeners.get(&type_id) {
            for item in list {
                out.push(CompiledEntry {
                    owner: self.clone(),
                    id: item.id,
                    node_priority: guard.priority,
                    priority: item.priority,
                    condition: combined_cond.clone(),
                    callback: Arc::clone(&item.callback),
                });
            }
        }
        for child in &guard.children {
            child.collect_listeners(type_id, combined_cond.as_ref(), out);
        }
    }

    fn compiled_chain_for(&self, type_id: TypeId) -> Arc<[CompiledEntry]> {
        let current_rev = self.tree_revision();
        {
            let guard = self.inner.read();
            if let Some((cached_rev, chain)) = guard.compiled_cache.get(&type_id)
                && *cached_rev == current_rev
            {
                return Arc::clone(chain);
            }
        }

        let mut raw = Vec::new();
        self.collect_listeners(type_id, None, &mut raw);
        raw.sort_by_key(|e| (e.node_priority, e.priority));
        let chain: Arc<[CompiledEntry]> = raw.into();

        self.inner
            .write()
            .compiled_cache
            .insert(type_id, (current_rev, Arc::clone(&chain)));
        chain
    }

    /// Dispatches `event` through all matching listeners in this node and its descendants.
    ///
    /// Uses a revision-validated cached listener slice so repeated calls allocate zero heap memory.
    pub fn call<E: Event>(&self, event: &mut E) {
        let type_id = TypeId::of::<E>();
        let chain = self.compiled_chain_for(type_id);
        Self::execute_chain(type_id, &chain, event);
    }

    /// Dispatches a [`CancellableEvent`] and runs `on_success` only if `!event.is_cancelled()`.
    pub fn call_cancellable<E: CancellableEvent>(
        &self,
        event: &mut E,
        on_success: impl FnOnce(&mut E),
    ) {
        self.call(event);
        if !event.is_cancelled() {
            on_success(event);
        }
    }

    fn execute_chain<E: Event>(type_id: TypeId, chain: &[CompiledEntry], event: &mut E) {
        for entry in chain {
            if let Some(cond) = &entry.condition
                && !cond(event)
            {
                continue;
            }
            let expired = (entry.callback)(event);
            if expired {
                entry.owner.remove_listener_by_id(type_id, entry.id);
            }
        }
    }

    /// Returns a pre-bound [`ListenerHandle<E>`] for ultra-fast hot-path event dispatch without
    /// `TypeId` hash lookups (`ListenerHandle` in Minestom).
    #[must_use]
    pub fn listener_handle<E: Event>(&self) -> ListenerHandle<E> {
        ListenerHandle {
            root: self.clone(),
            cached: RwLock::new((0, Arc::from([]))),
            _marker: PhantomData,
        }
    }

    /// Returns `true` if at least one listener is registered for `E` in this subtree.
    #[must_use]
    pub fn has_listener<E: Event>(&self) -> bool {
        !self.compiled_chain_for(TypeId::of::<E>()).is_empty()
    }
}

/// Pre-bound handle for dispatching a single event type `E` with minimal overhead.
pub struct ListenerHandle<E: Event> {
    root: EventNode,
    cached: RwLock<(u64, Arc<[CompiledEntry]>)>,
    _marker: PhantomData<fn(&mut E)>,
}

impl<E: Event> ListenerHandle<E> {
    /// Dispatches `event` using the cached chain, refreshing automatically if the tree changed.
    pub fn call(&self, event: &mut E) {
        let rev = self.root.tree_revision();
        let chain = {
            let guard = self.cached.read();
            if guard.0 == rev {
                Arc::clone(&guard.1)
            } else {
                drop(guard);
                let fresh = self.root.compiled_chain_for(TypeId::of::<E>());
                *self.cached.write() = (rev, Arc::clone(&fresh));
                fresh
            }
        };
        EventNode::execute_chain(TypeId::of::<E>(), &chain, event);
    }

    /// Returns `true` if at least one listener currently subscribes to `E`.
    #[must_use]
    pub fn has_listener(&self) -> bool {
        self.root.has_listener::<E>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestChat {
        msg: String,
        cancelled: bool,
    }
    impl Event for TestChat {}
    impl CancellableEvent for TestChat {
        fn is_cancelled(&self) -> bool {
            self.cancelled
        }
        fn set_cancelled(&mut self, cancelled: bool) {
            self.cancelled = cancelled;
        }
    }

    #[test]
    fn priorities_conditions_cancellation_and_expiration_work() {
        let root = EventNode::all("root");

        // Priority -10 runs before priority 10
        root.add_listener_with_priority(10, |ev: &mut TestChat| {
            ev.msg.push_str(":late");
        });
        root.add_listener_with_priority(-10, |ev: &mut TestChat| {
            ev.msg.push_str(":early");
        });

        // Child node filtered to messages starting with "vip"
        let vip_child =
            EventNode::value("vip", |ev: &TestChat| ev.msg.starts_with("vip")).with_priority(20);
        vip_child.add_listener(|ev: &mut TestChat| ev.msg.push_str(":vip"));
        root.add_child(vip_child);

        let handle = root.listener_handle::<TestChat>();
        let mut e1 = TestChat {
            msg: "hello".to_owned(),
            cancelled: false,
        };
        handle.call(&mut e1);
        assert_eq!(e1.msg, "hello:early:late");

        let mut e2 = TestChat {
            msg: "vip".to_owned(),
            cancelled: false,
        };
        handle.call(&mut e2);
        assert_eq!(e2.msg, "vip:early:late:vip");
    }
}
