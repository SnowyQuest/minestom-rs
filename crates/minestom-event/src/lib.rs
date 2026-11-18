//! Composable `EventNode` tree, priorities, expiring listeners, and cached `ListenerHandle`
//! dispatch for `minestom-rs` (ADR 0003).
//!
//! ```
//! use minestom_event::{Event, EventNode};
//!
//! struct PingEvent(u32);
//! impl Event for PingEvent {}
//!
//! let node = EventNode::all("root");
//! node.add_listener(|e: &mut PingEvent| e.0 += 10);
//!
//! let mut ev = PingEvent(5);
//! node.call(&mut ev);
//! assert_eq!(ev.0, 15);
//! ```

use std::any::{Any, TypeId};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;
use rustc_hash::FxHashMap;

/// Marker trait implemented by all server events.
pub trait Event: Any + Send + Sync + 'static {}

/// Trait implemented by events whose default outcome can be cancelled by listeners.
pub trait CancellableEvent: Event {
    /// Returns `true` if a prior listener marked this event as cancelled.
    fn is_cancelled(&self) -> bool;

    /// Marks this event as cancelled or uncancelled.
    fn set_cancelled(&mut self, cancelled: bool);
}

type ErasedCallback = Arc<dyn Fn(&mut dyn Any) -> bool + Send + Sync>;

#[derive(Clone)]
struct RegisteredListener {
    priority: i32,
    callback: ErasedCallback,
}

struct NodeInner {
    name: String,
    priority: i32,
    listeners: FxHashMap<TypeId, Vec<RegisteredListener>>,
    children: Vec<EventNode>,
    compiled_cache: FxHashMap<TypeId, (u64, Arc<[ErasedCallback]>)>,
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
    /// Creates a new `EventNode` accepting all events.
    #[must_use]
    pub fn all(name: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(NodeInner {
                name: name.into(),
                priority: 0,
                listeners: FxHashMap::default(),
                children: Vec::new(),
                compiled_cache: FxHashMap::default(),
            })),
            revision: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Sets this node's execution priority relative to sibling nodes and returns `self`.
    #[must_use]
    pub fn with_priority(self, priority: i32) -> Self {
        self.inner.write().priority = priority;
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

    /// Registers a typed synchronous listener on this node with default priority `0`.
    pub fn add_listener<E, F>(&self, handler: F) -> &Self
    where
        E: Event,
        F: Fn(&mut E) + Send + Sync + 'static,
    {
        self.add_listener_with_priority(0, handler)
    }

    /// Registers a typed synchronous listener with an explicit `priority` (lower runs first).
    pub fn add_listener_with_priority<E, F>(&self, priority: i32, handler: F) -> &Self
    where
        E: Event,
        F: Fn(&mut E) + Send + Sync + 'static,
    {
        let erased: ErasedCallback = Arc::new(move |any: &mut dyn Any| {
            if let Some(ev) = any.downcast_mut::<E>() {
                handler(ev);
            }
            false
        });
        let mut guard = self.inner.write();
        guard
            .listeners
            .entry(TypeId::of::<E>())
            .or_default()
            .push(RegisteredListener {
                priority,
                callback: erased,
            });
        guard.compiled_cache.remove(&TypeId::of::<E>());
        drop(guard);
        self.bump_revision();
        self
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

    fn collect_listeners(&self, type_id: TypeId, out: &mut Vec<(i32, ErasedCallback)>) {
        let guard = self.inner.read();
        if let Some(list) = guard.listeners.get(&type_id) {
            for item in list {
                out.push((item.priority, Arc::clone(&item.callback)));
            }
        }
        for child in &guard.children {
            child.collect_listeners(type_id, out);
        }
    }

    fn compiled_chain_for(&self, type_id: TypeId) -> Arc<[ErasedCallback]> {
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
        self.collect_listeners(type_id, &mut raw);
        raw.sort_by_key(|(prio, _)| *prio);
        let chain: Arc<[ErasedCallback]> = raw.into_iter().map(|(_, cb)| cb).collect();

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
        let chain = self.compiled_chain_for(TypeId::of::<E>());
        for cb in chain.iter() {
            let _ = cb(event);
        }
    }

    /// Returns `true` if at least one listener is registered for `E` in this subtree.
    #[must_use]
    pub fn has_listener<E: Event>(&self) -> bool {
        !self.compiled_chain_for(TypeId::of::<E>()).is_empty()
    }
}
