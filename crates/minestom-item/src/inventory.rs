//! `PlayerInventory`, custom container `Inventory` windows, `InventoryType`, and full
//! `InventoryClickProcessor` (left/right pickup, shift-click `QuickMove`, hotbar/offhand `Swap`,
//! creative `Clone`, `Throw`, `PickupAll` double-click, and `QuickCraft` paint-drag!).

use std::sync::Arc;

use minestom_text::Component;
use parking_lot::RwLock;
use rustc_hash::FxHashSet;

use crate::{ItemError, ItemStack};

/// Container window types (`InventoryType` in Minestom 26.2, ordinal `0..=24`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum InventoryType {
    /// `0` — 9-slot 1-row chest (`generic_9x1`).
    Chest1Row = 0,
    /// `1` — 18-slot 2-row chest (`generic_9x2`).
    Chest2Row = 1,
    /// `2` — 27-slot 3-row chest (`generic_9x3`).
    Chest3Row = 2,
    /// `3` — 36-slot 4-row chest (`generic_9x4`).
    Chest4Row = 3,
    /// `4` — 45-slot 5-row chest (`generic_9x5`).
    Chest5Row = 4,
    /// `5` — 54-slot 6-row double chest (`generic_9x6`).
    Chest6Row = 5,
    /// `6` — 9-slot dispenser/dropper (`generic_3x3`).
    Window3x3 = 6,
    /// `7` — 9-slot crafter (`crafter_3x3`).
    Crafter3x3 = 7,
    /// `8` — 3-slot anvil.
    Anvil = 8,
    /// `9` — 1-slot beacon.
    Beacon = 9,
    /// `10` — 3-slot blast furnace.
    BlastFurnace = 10,
    /// `11` — 5-slot brewing stand.
    BrewingStand = 11,
    /// `12` — 10-slot crafting table.
    Crafting = 12,
    /// `13` — 2-slot enchanting table.
    Enchantment = 13,
    /// `14` — 3-slot furnace.
    Furnace = 14,
    /// `15` — 3-slot grindstone.
    Grindstone = 15,
    /// `16` — 5-slot hopper.
    Hopper = 16,
    /// `17` — 1-slot lectern.
    Lectern = 17,
    /// `18` — 4-slot loom.
    Loom = 18,
    /// `19` — 3-slot villager merchant.
    Merchant = 19,
    /// `20` — 27-slot shulker box.
    ShulkerBox = 20,
    /// `21` — 4-slot smithing table.
    Smithing = 21,
    /// `22` — 3-slot smoker.
    Smoker = 22,
    /// `23` — 3-slot cartography table.
    Cartography = 23,
    /// `24` — 2-slot stonecutter.
    StoneCutter = 24,
}

impl InventoryType {
    /// Returns the protocol `window_type` ID for `OpenWindowPacket`.
    #[must_use]
    pub const fn window_type_id(self) -> i32 {
        self as i32
    }

    /// Number of container slots in this window (excluding the player's 36 main/hotbar slots).
    #[must_use]
    pub const fn size(self) -> usize {
        match self {
            Self::Beacon | Self::Lectern => 1,
            Self::Enchantment | Self::StoneCutter => 2,
            Self::Anvil
            | Self::BlastFurnace
            | Self::Furnace
            | Self::Grindstone
            | Self::Merchant
            | Self::Smoker
            | Self::Cartography => 3,
            Self::Loom | Self::Smithing => 4,
            Self::BrewingStand | Self::Hopper => 5,
            Self::Chest1Row | Self::Window3x3 | Self::Crafter3x3 => 9,
            Self::Crafting => 10,
            Self::Chest2Row => 18,
            Self::Chest3Row | Self::ShulkerBox => 27,
            Self::Chest4Row => 36,
            Self::Chest5Row => 45,
            Self::Chest6Row => 54,
        }
    }
}

/// Raw click operation inside `ClientClickWindowPacket`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum WindowClickMode {
    /// `0` — Left (`button = 0`) or right (`button = 1`) click pickup/place, or click outside (`slot = -999`).
    Pickup = 0,
    /// `1` — Shift + left/right click quick transfer (`QUICK_MOVE`).
    QuickMove = 1,
    /// `2` — Number key `0..=8` hotbar swap or `40` (`F` offhand swap).
    Swap = 2,
    /// `3` — Middle-click creative clone (`CLONE`).
    Clone = 3,
    /// `4` — Drop single (`button = 0`) or full stack (`button = 1`) from slot (`THROW`).
    Throw = 4,
    /// `5` — Paint-drag gesture (`QUICK_CRAFT`: start, add slot, finish).
    QuickCraft = 5,
    /// `6` — Double-click gather matching items into cursor (`PICKUP_ALL`).
    PickupAll = 6,
}

struct InventoryInner {
    inventory_type: InventoryType,
    title: Component,
    slots: Vec<ItemStack>,
    state_id: i32,
}

/// Shared, thread-safe custom container window (`Inventory` in Minestom) that one or more
/// players can open simultaneously.
#[derive(Clone)]
pub struct Inventory {
    inner: Arc<RwLock<InventoryInner>>,
}

impl std::fmt::Debug for Inventory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.read();
        f.debug_struct("Inventory")
            .field("type", &g.inventory_type)
            .field("size", &g.slots.len())
            .field("state_id", &g.state_id)
            .finish()
    }
}

impl Inventory {
    /// Creates a new empty container [`Inventory`] of `inventory_type` with `title`.
    #[must_use]
    pub fn new(inventory_type: InventoryType, title: impl Into<Component>) -> Self {
        let cap = inventory_type.size();
        Self {
            inner: Arc::new(RwLock::new(InventoryInner {
                inventory_type,
                title: title.into(),
                slots: vec![ItemStack::AIR; cap],
                state_id: 1,
            })),
        }
    }

    /// Returns `true` if `self` and `other` point to the exact same shared container instance.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Container [`InventoryType`].
    #[must_use]
    pub fn inventory_type(&self) -> InventoryType {
        self.inner.read().inventory_type
    }

    /// Window title [`Component`].
    #[must_use]
    pub fn title(&self) -> Component {
        self.inner.read().title.clone()
    }

    /// Number of container slots.
    #[must_use]
    pub fn size(&self) -> usize {
        self.inner.read().slots.len()
    }

    /// Current revision `state_id` for `WindowItemsPacket` / `SetSlotPacket`.
    #[must_use]
    pub fn state_id(&self) -> i32 {
        self.inner.read().state_id
    }

    /// Bumps and returns the next positive `state_id`.
    pub fn bump_state_id(&self) -> i32 {
        let mut g = self.inner.write();
        g.state_id = (g.state_id.wrapping_add(1)) & 0x7FFF;
        if g.state_id == 0 {
            g.state_id = 1;
        }
        g.state_id
    }

    /// Reads the [`ItemStack`] in `slot`.
    pub fn get_item(&self, slot: usize) -> Result<ItemStack, ItemError> {
        let g = self.inner.read();
        g.slots
            .get(slot)
            .cloned()
            .ok_or(ItemError::SlotOutOfBounds {
                slot,
                size: g.slots.len(),
            })
    }

    /// Writes `stack` into `slot` and increments `state_id`.
    pub fn set_item(&self, slot: usize, stack: ItemStack) -> Result<(), ItemError> {
        let mut g = self.inner.write();
        let len = g.slots.len();
        let target = g
            .slots
            .get_mut(slot)
            .ok_or(ItemError::SlotOutOfBounds { slot, size: len })?;
        *target = stack.normalized();
        g.state_id = (g.state_id.wrapping_add(1)).max(1);
        Ok(())
    }

    /// Snapshots all container slots.
    #[must_use]
    pub fn items(&self) -> Vec<ItemStack> {
        self.inner.read().slots.clone()
    }

    /// Inserts as much of `stack` as fits into existing similar stacks then empty slots,
    /// returning any leftover [`ItemStack`].
    pub fn add_item(&self, stack: ItemStack) -> ItemStack {
        let mut g = self.inner.write();
        let rem = merge_stack_into_slice(&mut g.slots, stack);
        g.state_id = (g.state_id.wrapping_add(1)).max(1);
        rem
    }

    /// Clears all slots to [`ItemStack::AIR`].
    pub fn clear(&self) {
        let mut g = self.inner.write();
        g.slots.fill(ItemStack::AIR);
        g.state_id = (g.state_id.wrapping_add(1)).max(1);
    }
}

fn merge_stack_into_slice(slots: &mut [ItemStack], mut incoming: ItemStack) -> ItemStack {
    incoming = incoming.normalized();
    if incoming.is_air() {
        return ItemStack::AIR;
    }
    let max_stack = incoming.max_stack_size();

    // Pass 1: stack onto existing similar non-full stacks
    for existing in slots.iter_mut() {
        if incoming.is_air() {
            break;
        }
        if !existing.is_air() && existing.is_similar(&incoming) && existing.amount() < max_stack {
            let space = max_stack - existing.amount();
            let xfer = space.min(incoming.amount());
            let new_ex = existing.amount() + xfer;
            let new_in = incoming.amount() - xfer;
            *existing = existing.clone().with_amount(new_ex);
            incoming = incoming.with_amount(new_in);
        }
    }

    // Pass 2: place into empty AIR slots
    for existing in slots.iter_mut() {
        if incoming.is_air() {
            break;
        }
        if existing.is_air() {
            let xfer = max_stack.min(incoming.amount());
            let new_in = incoming.amount() - xfer;
            *existing = incoming.clone().with_amount(xfer);
            incoming = incoming.with_amount(new_in);
        }
    }

    incoming
}

#[derive(Debug, Clone, Default)]
struct DragSession {
    active: bool,
    button_kind: u8, // 0 = left even split, 1 = right 1-each, 2 = creative clone
    slots: Vec<usize>,
    visited: FxHashSet<usize>,
}

/// 46-slot player inventory (`window_id == 0`) plus carried cursor item, selected hotbar slot,
/// and paint-drag session state (`PlayerInventory` in Minestom).
///
/// Protocol window `0` slot layout:
/// - `0`: 2×2 crafting output
/// - `1..=4`: 2×2 crafting grid
/// - `5..=8`: armor (`5` helmet, `6` chestplate, `7` leggings, `8` boots)
/// - `9..=35`: main storage (`27` slots)
/// - `36..=44`: hotbar `0..=8` (`hotbar_to_window_slot(h) = 36 + h`)
/// - `45`: off-hand
#[derive(Debug, Clone)]
pub struct PlayerInventory {
    slots: [ItemStack; Self::SIZE],
    cursor: ItemStack,
    held_slot: u8,
    state_id: i32,
    drag: DragSession,
}

impl Default for PlayerInventory {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerInventory {
    /// Total protocol slot count in window `0` (`46`).
    pub const SIZE: usize = 46;
    /// First main-storage protocol slot (`9`).
    pub const MAIN_START: usize = 9;
    /// First hotbar protocol slot (`36`).
    pub const HOTBAR_START: usize = 36;
    /// Off-hand protocol slot (`45`).
    pub const OFFHAND_SLOT: usize = 45;

    /// Creates an empty [`PlayerInventory`] with hotbar slot `0` selected.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| ItemStack::AIR),
            cursor: ItemStack::AIR,
            held_slot: 0,
            state_id: 1,
            drag: DragSession::default(),
        }
    }

    /// Current revision `state_id` for window `0`.
    #[must_use]
    pub const fn state_id(&self) -> i32 {
        self.state_id
    }

    /// Bumps and returns the next positive `state_id`.
    pub fn bump_state_id(&mut self) -> i32 {
        self.state_id = (self.state_id.wrapping_add(1)) & 0x7FFF;
        if self.state_id == 0 {
            self.state_id = 1;
        }
        self.state_id
    }

    /// Selected hotbar index (`0..=8`).
    #[must_use]
    pub const fn held_slot(&self) -> u8 {
        self.held_slot
    }

    /// Updates selected hotbar index (`0..=8`).
    pub fn set_held_slot(&mut self, slot: u8) -> Result<(), ItemError> {
        if slot > 8 {
            return Err(ItemError::SlotOutOfBounds {
                slot: usize::from(slot),
                size: 9,
            });
        }
        self.held_slot = slot;
        Ok(())
    }

    /// Protocol window-0 slot index (`36..=44`) of the currently held hotbar slot.
    #[must_use]
    pub const fn held_window_slot(&self) -> usize {
        Self::HOTBAR_START + (self.held_slot as usize)
    }

    /// Borrows the item in the player's active main hand.
    #[must_use]
    pub fn item_in_main_hand(&self) -> &ItemStack {
        &self.slots[self.held_window_slot()]
    }

    /// Replaces the item in the player's active main hand.
    pub fn set_item_in_main_hand(&mut self, stack: ItemStack) {
        let idx = self.held_window_slot();
        self.slots[idx] = stack.normalized();
        let _ = self.bump_state_id();
    }

    /// Borrows the item in the player's off-hand (`slot 45`).
    #[must_use]
    pub fn item_in_off_hand(&self) -> &ItemStack {
        &self.slots[Self::OFFHAND_SLOT]
    }

    /// Replaces the item in the player's off-hand (`slot 45`).
    pub fn set_item_in_off_hand(&mut self, stack: ItemStack) {
        self.slots[Self::OFFHAND_SLOT] = stack.normalized();
        let _ = self.bump_state_id();
    }

    /// Borrows the item currently carried on the mouse cursor.
    #[must_use]
    pub const fn cursor_item(&self) -> &ItemStack {
        &self.cursor
    }

    /// Sets the item carried on the mouse cursor.
    pub fn set_cursor_item(&mut self, stack: ItemStack) {
        self.cursor = stack.normalized();
        let _ = self.bump_state_id();
    }

    /// Reads protocol window-0 `slot` (`0..46`).
    pub fn get_slot(&self, slot: usize) -> Result<&ItemStack, ItemError> {
        self.slots.get(slot).ok_or(ItemError::SlotOutOfBounds {
            slot,
            size: Self::SIZE,
        })
    }

    /// Writes `stack` into protocol window-0 `slot` (`0..46`).
    pub fn set_slot(&mut self, slot: usize, stack: ItemStack) -> Result<(), ItemError> {
        let s = self.slots.get_mut(slot).ok_or(ItemError::SlotOutOfBounds {
            slot,
            size: Self::SIZE,
        })?;
        *s = stack.normalized();
        let _ = self.bump_state_id();
        Ok(())
    }

    /// Snapshots all 46 protocol slots in window `0` order.
    #[must_use]
    pub fn window_zero_items(&self) -> Vec<ItemStack> {
        self.slots.to_vec()
    }

    /// Builds the combined `(container_slots + 27 main_storage + 9 hotbar)` item list
    /// expected by `WindowItemsPacket` when an external `container` is open!
    #[must_use]
    pub fn combined_window_items(&self, container: &Inventory) -> Vec<ItemStack> {
        let mut out = container.items();
        out.extend_from_slice(&self.slots[Self::MAIN_START..Self::HOTBAR_START + 9]);
        out
    }

    /// Adds `stack` preferring hotbar (`36..45`) first, then main inventory (`9..36`),
    /// returning any leftover [`ItemStack`] that did not fit!
    pub fn add_item(&mut self, mut stack: ItemStack) -> ItemStack {
        stack = stack.normalized();
        if stack.is_air() {
            return ItemStack::AIR;
        }
        // Pass 1: stack onto existing similar items in hotbar, then main storage
        let max_stack = stack.max_stack_size();
        for idx in
            (Self::HOTBAR_START..Self::HOTBAR_START + 9).chain(Self::MAIN_START..Self::HOTBAR_START)
        {
            if stack.is_air() {
                break;
            }
            let existing = &mut self.slots[idx];
            if !existing.is_air() && existing.is_similar(&stack) && existing.amount() < max_stack {
                let xfer = (max_stack - existing.amount()).min(stack.amount());
                let new_ex = existing.amount() + xfer;
                let new_st = stack.amount() - xfer;
                *existing = existing.clone().with_amount(new_ex);
                stack = stack.with_amount(new_st);
            }
        }
        // Pass 2: empty slots in hotbar, then main storage
        for idx in
            (Self::HOTBAR_START..Self::HOTBAR_START + 9).chain(Self::MAIN_START..Self::HOTBAR_START)
        {
            if stack.is_air() {
                break;
            }
            if self.slots[idx].is_air() {
                let xfer = max_stack.min(stack.amount());
                let new_st = stack.amount() - xfer;
                self.slots[idx] = stack.clone().with_amount(xfer);
                stack = stack.with_amount(new_st);
            }
        }
        let _ = self.bump_state_id();
        stack
    }

    /// Cancels any in-progress paint-drag gesture.
    pub fn cancel_drag(&mut self) {
        self.drag = DragSession::default();
    }
}

/// Summary outcome produced by [`process_inventory_click`].
#[derive(Debug, Clone, Default)]
pub struct ClickOutcome {
    /// Any items thrown out into the world during this click (`Throw` or clicking outside `-999`).
    pub dropped_items: Vec<ItemStack>,
    /// Whether inventory/cursor contents were mutated and should be resynced to viewers.
    pub changed: bool,
}

struct CombinedView<'a> {
    container_slots: Option<&'a mut [ItemStack]>,
    player: &'a mut PlayerInventory,
}

impl CombinedView<'_> {
    fn total_len(&self) -> usize {
        match &self.container_slots {
            Some(c) => c.len() + 36,
            None => PlayerInventory::SIZE,
        }
    }

    fn container_len(&self) -> usize {
        self.container_slots.as_ref().map_or(0, |c| c.len())
    }

    fn get(&self, idx: usize) -> Option<&ItemStack> {
        if let Some(c) = &self.container_slots {
            if idx < c.len() {
                c.get(idx)
            } else {
                let p_idx = PlayerInventory::MAIN_START + (idx - c.len());
                (p_idx < PlayerInventory::HOTBAR_START + 9).then(|| &self.player.slots[p_idx])
            }
        } else {
            self.player.slots.get(idx)
        }
    }

    fn set(&mut self, idx: usize, val: ItemStack) -> bool {
        let val = val.normalized();
        if let Some(c) = &mut self.container_slots {
            if idx < c.len() {
                c[idx] = val;
                true
            } else {
                let p_idx = PlayerInventory::MAIN_START + (idx - c.len());
                if p_idx < PlayerInventory::HOTBAR_START + 9 {
                    self.player.slots[p_idx] = val;
                    true
                } else {
                    false
                }
            }
        } else if let Some(slot_ref) = self.player.slots.get_mut(idx) {
            *slot_ref = val;
            true
        } else {
            false
        }
    }

    fn transfer_into_range(
        &mut self,
        mut stack: ItemStack,
        range: impl Iterator<Item = usize>,
    ) -> ItemStack {
        stack = stack.normalized();
        if stack.is_air() {
            return ItemStack::AIR;
        }
        let indices: Vec<usize> = range.collect();
        let max_stack = stack.max_stack_size();

        for &idx in &indices {
            if stack.is_air() {
                break;
            }
            if let Some(existing) = self.get(idx).cloned()
                && !existing.is_air()
                && existing.is_similar(&stack)
                && existing.amount() < max_stack
            {
                let xfer = (max_stack - existing.amount()).min(stack.amount());
                let new_ex = existing.amount() + xfer;
                let new_st = stack.amount() - xfer;
                self.set(idx, existing.with_amount(new_ex));
                stack = stack.with_amount(new_st);
            }
        }
        for &idx in &indices {
            if stack.is_air() {
                break;
            }
            if let Some(existing) = self.get(idx)
                && existing.is_air()
            {
                let xfer = max_stack.min(stack.amount());
                let new_st = stack.amount() - xfer;
                self.set(idx, stack.clone().with_amount(xfer));
                stack = stack.with_amount(new_st);
            }
        }
        stack
    }
}

/// Processes a full vanilla inventory click (`Pickup`, `QuickMove`, `Swap`, `Clone`, `Throw`,
/// `QuickCraft`, `PickupAll`) against `player_inv` and an optional open `container`!
pub fn process_inventory_click(
    player_inv: &mut PlayerInventory,
    container: Option<&Inventory>,
    slot: i16,
    button: i8,
    mode: WindowClickMode,
    creative_mode: bool,
) -> ClickOutcome {
    if let Some(cont) = container {
        let mut g = cont.inner.write();
        let out = execute_click_inner(
            CombinedView {
                container_slots: Some(&mut g.slots),
                player: player_inv,
            },
            slot,
            button,
            mode,
            creative_mode,
        );
        if out.changed {
            g.state_id = (g.state_id.wrapping_add(1)).max(1);
            let _ = player_inv.bump_state_id();
        }
        out
    } else {
        let out = execute_click_inner(
            CombinedView {
                container_slots: None,
                player: player_inv,
            },
            slot,
            button,
            mode,
            creative_mode,
        );
        if out.changed {
            let _ = player_inv.bump_state_id();
        }
        out
    }
}

fn execute_click_inner(
    mut view: CombinedView<'_>,
    slot: i16,
    button: i8,
    mode: WindowClickMode,
    creative_mode: bool,
) -> ClickOutcome {
    let mut outcome = ClickOutcome::default();

    if mode != WindowClickMode::QuickCraft {
        view.player.cancel_drag();
    }

    match mode {
        WindowClickMode::Pickup => {
            if slot == -999 {
                let cursor = view.player.cursor.clone();
                if !cursor.is_air() {
                    if button == 0 {
                        outcome.dropped_items.push(cursor);
                        view.player.cursor = ItemStack::AIR;
                    } else if button == 1 {
                        outcome.dropped_items.push(cursor.clone().with_amount(1));
                        view.player.cursor = cursor.clone().with_amount(cursor.amount() - 1);
                    }
                    outcome.changed = true;
                }
                return outcome;
            }
            let Ok(idx) = usize::try_from(slot) else {
                return outcome;
            };
            let Some(clicked) = view.get(idx).cloned() else {
                return outcome;
            };
            let cursor = view.player.cursor.clone();

            if button == 0 {
                // Left click: stack if similar, else swap cursor & slot!
                if !cursor.is_air() && !clicked.is_air() && clicked.is_similar(&cursor) {
                    let max_s = clicked.max_stack_size();
                    let space = max_s.saturating_sub(clicked.amount());
                    if space > 0 {
                        let xfer = space.min(cursor.amount());
                        view.set(idx, clicked.clone().with_amount(clicked.amount() + xfer));
                        view.player.cursor = cursor.clone().with_amount(cursor.amount() - xfer);
                        outcome.changed = true;
                    }
                } else if !cursor.is_air() || !clicked.is_air() {
                    view.set(idx, cursor);
                    view.player.cursor = clicked;
                    outcome.changed = true;
                }
            } else if button == 1 {
                // Right click: split half if cursor empty, or place 1 if compatible/empty!
                if cursor.is_air() && !clicked.is_air() {
                    let take = clicked.amount().div_ceil(2);
                    let remain = clicked.amount() - take;
                    view.set(idx, clicked.clone().with_amount(remain));
                    view.player.cursor = clicked.with_amount(take);
                    outcome.changed = true;
                } else if !cursor.is_air() {
                    if clicked.is_air() {
                        view.set(idx, cursor.clone().with_amount(1));
                        view.player.cursor = cursor.clone().with_amount(cursor.amount() - 1);
                        outcome.changed = true;
                    } else if clicked.is_similar(&cursor)
                        && clicked.amount() < clicked.max_stack_size()
                    {
                        view.set(idx, clicked.clone().with_amount(clicked.amount() + 1));
                        view.player.cursor = cursor.clone().with_amount(cursor.amount() - 1);
                        outcome.changed = true;
                    } else if !clicked.is_similar(&cursor) {
                        view.set(idx, cursor);
                        view.player.cursor = clicked;
                        outcome.changed = true;
                    }
                }
            }
        }
        WindowClickMode::QuickMove => {
            let Ok(idx) = usize::try_from(slot) else {
                return outcome;
            };
            let Some(clicked) = view.get(idx).cloned() else {
                return outcome;
            };
            if clicked.is_air() {
                return outcome;
            }
            let clen = view.container_len();
            let leftover = if clen > 0 {
                if idx < clen {
                    // Shift-click from container -> player hotbar (reversed) then main inventory (reversed), like vanilla!
                    let total = view.total_len();
                    view.transfer_into_range(clicked, (clen..total).rev())
                } else {
                    // Shift-click from player inventory -> container `0..clen`
                    view.transfer_into_range(clicked, 0..clen)
                }
            } else if (PlayerInventory::MAIN_START..PlayerInventory::HOTBAR_START).contains(&idx) {
                // Player main -> hotbar
                view.transfer_into_range(
                    clicked,
                    PlayerInventory::HOTBAR_START..PlayerInventory::HOTBAR_START + 9,
                )
            } else if (PlayerInventory::HOTBAR_START..PlayerInventory::HOTBAR_START + 9)
                .contains(&idx)
            {
                // Player hotbar -> main
                view.transfer_into_range(
                    clicked,
                    PlayerInventory::MAIN_START..PlayerInventory::HOTBAR_START,
                )
            } else {
                // Armor / crafting / offhand -> main + hotbar
                view.transfer_into_range(
                    clicked,
                    PlayerInventory::MAIN_START..PlayerInventory::HOTBAR_START + 9,
                )
            };
            view.set(idx, leftover);
            outcome.changed = true;
        }
        WindowClickMode::Swap => {
            let Ok(idx) = usize::try_from(slot) else {
                return outcome;
            };
            let Some(clicked) = view.get(idx).cloned() else {
                return outcome;
            };
            if (0..=8).contains(&button) {
                let hb_p_idx = PlayerInventory::HOTBAR_START + (button as usize);
                let hb_item = view.player.slots[hb_p_idx].clone();
                view.player.slots[hb_p_idx] = clicked;
                // Note: if `idx` itself aliases `hb_p_idx`, setting `idx` to `hb_item` restores cleanly!
                view.set(idx, hb_item);
                outcome.changed = true;
            } else if button == 40 {
                let off_item = view.player.slots[PlayerInventory::OFFHAND_SLOT].clone();
                view.player.slots[PlayerInventory::OFFHAND_SLOT] = clicked;
                view.set(idx, off_item);
                outcome.changed = true;
            }
        }
        WindowClickMode::Clone => {
            if creative_mode
                && view.player.cursor.is_air()
                && let Ok(idx) = usize::try_from(slot)
                && let Some(clicked) = view.get(idx).cloned()
                && !clicked.is_air()
            {
                let max_s = clicked.max_stack_size();
                view.player.cursor = clicked.with_amount(max_s);
                outcome.changed = true;
            }
        }
        WindowClickMode::Throw => {
            if let Ok(idx) = usize::try_from(slot)
                && let Some(clicked) = view.get(idx).cloned()
                && !clicked.is_air()
            {
                if button == 0 {
                    outcome.dropped_items.push(clicked.clone().with_amount(1));
                    view.set(idx, clicked.clone().with_amount(clicked.amount() - 1));
                    outcome.changed = true;
                } else if button == 1 {
                    outcome.dropped_items.push(clicked);
                    view.set(idx, ItemStack::AIR);
                    outcome.changed = true;
                }
            }
        }
        WindowClickMode::QuickCraft => {
            // Vanilla drag button protocol:
            // 0 = start left drag, 1 = add left slot, 2 = end left drag
            // 4 = start right drag, 5 = add right slot, 6 = end right drag
            // 8 = start middle drag, 9 = add middle slot, 10 = end middle drag
            match button {
                0 | 4 | 8 => {
                    view.player.drag = DragSession {
                        active: true,
                        button_kind: (button / 4) as u8,
                        slots: Vec::new(),
                        visited: FxHashSet::default(),
                    };
                }
                1 | 5 | 9 => {
                    if view.player.drag.active
                        && let Ok(idx) = usize::try_from(slot)
                        && idx < view.total_len()
                        && view.player.drag.visited.insert(idx)
                    {
                        view.player.drag.slots.push(idx);
                    }
                }
                2 | 6 | 10 => {
                    if view.player.drag.active && !view.player.cursor.is_air() {
                        let cursor = view.player.cursor.clone();
                        let targets = std::mem::take(&mut view.player.drag.slots);
                        let kind = view.player.drag.button_kind;
                        view.player.cancel_drag();

                        let eligible: Vec<usize> = targets
                            .into_iter()
                            .filter(|&idx| {
                                view.get(idx).is_some_and(|s| {
                                    s.is_air()
                                        || (s.is_similar(&cursor)
                                            && s.amount() < s.max_stack_size())
                                })
                            })
                            .collect();

                        if !eligible.is_empty() {
                            if kind == 2 && creative_mode {
                                let max_s = cursor.max_stack_size();
                                for idx in eligible {
                                    view.set(idx, cursor.clone().with_amount(max_s));
                                }
                                view.player.cursor = ItemStack::AIR;
                                outcome.changed = true;
                            } else {
                                let per_slot = if kind == 0 {
                                    cursor.amount() / (eligible.len() as u32)
                                } else {
                                    1
                                };
                                if per_slot > 0 {
                                    let mut rem = cursor.amount();
                                    for idx in eligible {
                                        if rem == 0 {
                                            break;
                                        }
                                        let cur = view.get(idx).cloned().unwrap_or(ItemStack::AIR);
                                        let cur_amt = if cur.is_air() { 0 } else { cur.amount() };
                                        let add = per_slot
                                            .min(rem)
                                            .min(cursor.max_stack_size() - cur_amt);
                                        if add > 0 {
                                            view.set(
                                                idx,
                                                cursor.clone().with_amount(cur_amt + add),
                                            );
                                            rem -= add;
                                        }
                                    }
                                    view.player.cursor = cursor.with_amount(rem);
                                    outcome.changed = true;
                                }
                            }
                        }
                    } else {
                        view.player.cancel_drag();
                    }
                }
                _ => view.player.cancel_drag(),
            }
        }
        WindowClickMode::PickupAll => {
            let cursor = view.player.cursor.clone();
            if !cursor.is_air() {
                let max_s = cursor.max_stack_size();
                let mut gathered = cursor.amount();
                let total = view.total_len();
                // Gather from non-full stacks first, then full stacks, just like vanilla!
                for full_pass in [false, true] {
                    for idx in 0..total {
                        if gathered >= max_s {
                            break;
                        }
                        if let Some(s) = view.get(idx).cloned()
                            && !s.is_air()
                            && s.is_similar(&cursor)
                            && ((s.amount() == max_s) == full_pass)
                        {
                            let take = (max_s - gathered).min(s.amount());
                            gathered += take;
                            view.set(idx, s.clone().with_amount(s.amount() - take));
                            outcome.changed = true;
                        }
                    }
                }
                view.player.cursor = cursor.with_amount(gathered);
            }
        }
    }

    outcome
}

#[cfg(test)]
mod tests {
    use crate::Material;

    use super::*;

    #[test]
    fn pickup_shift_click_swap_double_click_and_paint_drag_work() {
        let mut pinv = PlayerInventory::new();
        let chest = Inventory::new(InventoryType::Chest3Row, "Storage");

        // Put 32 diamonds in player hotbar slot 0 (window-0 slot 36 -> combined slot 27 + 27 = 54)
        pinv.set_slot(36, ItemStack::of(Material::DIAMOND, 32))
            .unwrap();

        // Shift-click combined slot 54 into chest -> moves 32 diamonds into chest slot 0!
        let r1 = process_inventory_click(
            &mut pinv,
            Some(&chest),
            54,
            0,
            WindowClickMode::QuickMove,
            false,
        );
        assert!(r1.changed);
        assert!(pinv.get_slot(36).unwrap().is_air());
        assert_eq!(chest.get_item(0).unwrap().amount(), 32);

        // Left-click chest slot 0 -> picks 32 diamonds onto cursor; paint-drag evenly across chest slots 1, 2, 3, 4 (8 each)!
        process_inventory_click(
            &mut pinv,
            Some(&chest),
            0,
            0,
            WindowClickMode::Pickup,
            false,
        );
        assert_eq!(pinv.cursor_item().amount(), 32);

        process_inventory_click(
            &mut pinv,
            Some(&chest),
            -999,
            0,
            WindowClickMode::QuickCraft,
            false,
        );
        for s in 1..=4 {
            process_inventory_click(
                &mut pinv,
                Some(&chest),
                s,
                1,
                WindowClickMode::QuickCraft,
                false,
            );
        }
        process_inventory_click(
            &mut pinv,
            Some(&chest),
            -999,
            2,
            WindowClickMode::QuickCraft,
            false,
        );
        assert!(pinv.cursor_item().is_air());
        for s in 1..=4 {
            assert_eq!(chest.get_item(s).unwrap().amount(), 8);
        }

        // Pick up slot 1 (8 diamonds) and double-click (`PickupAll`) -> gathers all 32 diamonds back onto cursor!
        process_inventory_click(
            &mut pinv,
            Some(&chest),
            1,
            0,
            WindowClickMode::Pickup,
            false,
        );
        process_inventory_click(
            &mut pinv,
            Some(&chest),
            1,
            0,
            WindowClickMode::PickupAll,
            false,
        );
        assert_eq!(pinv.cursor_item().amount(), 32);
    }
}
