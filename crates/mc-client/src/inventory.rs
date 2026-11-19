//! The player's inventory as the client keeps it: `InventoryMenu`'s 46 slots and the item on
//! the cursor, and `InventoryScreen`'s mouse and keys. Each click is predicted as
//! `AbstractContainerMenu.doClick` does and sent as `MultiPlayerGameMode.handleContainerInput`
//! does, with the slots it changed, so the server only corrects what we got wrong.
//!
//! Slots: 0 the crafting result, 1 to 4 the grid, 5 to 8 armour (head first), 9 to 35 the
//! inventory, 36 to 44 the hotbar, 45 the offhand.

use mc_protocol::codec::Put;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub type Stack = Option<(String, i32)>;

/// `ContainerInput`'s ids.
const PICKUP: i32 = 0;
const QUICK_MOVE: i32 = 1;
const SWAP: i32 = 2;
const THROW: i32 = 4;
const QUICK_CRAFT: i32 = 5;
const PICKUP_ALL: i32 = 6;
/// The slot number of a click outside the screen.
const OUTSIDE: i32 = -999;
const SLOTS: usize = 46;
/// The armour slots' `EquipmentSlot`s, from slot 5.
const ARMOR: [&str; 4] = ["head", "chest", "legs", "feet"];

/// What the inventory needs of an item: its protocol id, stack size and equipment slot.
struct Item {
    id: u32,
    max: i32,
    slot: Option<String>,
}

fn with_count(name: &str, count: i32) -> Stack {
    (count > 0).then(|| (name.to_owned(), count))
}

pub struct Inventory {
    pub slots: Vec<Stack>,
    pub carried: Stack,
    /// The server's count of the menu's changes, which each click echoes.
    state_id: i32,
    items: HashMap<String, Item>,
    open: bool,
    /// `AbstractContainerMenu`'s drag (`quickcraftStatus`, `quickcraftType`, its slots).
    qc_status: i32,
    qc_type: i32,
    qc_slots: Vec<usize>,
    /// `AbstractContainerScreen`'s drag: the button it started with, its type and slots.
    dragging: Option<(u8, i32)>,
    drag_slots: Vec<usize>,
    skip_next_release: bool,
    /// The last press: its slot, button and when, for double clicks.
    last_click: Option<(Option<usize>, u8, Instant)>,
    double_click: bool,
}

impl Inventory {
    pub fn new(registry: &mc_registry::Registry) -> Self {
        let items = (0..registry.item_count())
            .filter_map(|id| {
                let name = registry.item_name(id)?;
                let info = registry.item_info(name)?;
                let item = Item {
                    id,
                    max: info.max_stack,
                    slot: info.slot.clone(),
                };
                Some((name.to_owned(), item))
            })
            .collect();
        Self {
            slots: vec![None; SLOTS],
            carried: None,
            state_id: 0,
            items,
            open: false,
            qc_status: 0,
            qc_type: 0,
            qc_slots: Vec::new(),
            dragging: None,
            drag_slots: Vec::new(),
            skip_next_release: false,
            last_click: None,
            double_click: false,
        }
    }

    /// The server set these slots (and maybe the state id).
    pub fn set(&mut self, state_id: Option<i32>, slots: Vec<(u8, Stack)>) {
        if let Some(id) = state_id {
            self.state_id = id;
        }
        for (slot, stack) in slots {
            if let Some(s) = self.slots.get_mut(slot as usize) {
                *s = stack;
            }
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.last_click = None;
    }

    /// `AbstractContainerScreen.onClose`: `ServerboundContainerClosePacket`, then
    /// `InventoryMenu.removed` drops what's on the cursor (the server drops it in the world)
    /// and clears the crafting result; the server moves the grid back into the inventory.
    pub fn close(&mut self, outbox: &mc_protocol::Outbox) -> anyhow::Result<()> {
        self.open = false;
        (self.carried, self.slots[0]) = (None, None);
        self.reset_drag();
        self.qc_status = 0;
        self.qc_slots.clear();
        let mut body = Vec::new();
        body.put_varint(0);
        outbox.send("container_close", &body)
    }

    fn max_of(&self, name: &str) -> i32 {
        self.items.get(name).map_or(64, |i| i.max)
    }

    /// `Slot.getMaxStackSize(stack)`: armour slots hold one, the rest the inventory's 99.
    fn max_in(&self, slot: usize, name: &str) -> i32 {
        let limit = if (5..9).contains(&slot) { 1 } else { 99 };
        self.max_of(name).min(limit)
    }

    /// `Slot.mayPlace`: nothing goes into the result, and armour only where it's worn.
    fn may_place(&self, slot: usize, name: &str) -> bool {
        match slot {
            0 => false,
            5..9 => {
                let worn = self.items.get(name).and_then(|i| i.slot.as_deref());
                worn == Some(ARMOR[slot - 5])
            }
            _ => true,
        }
    }

    /// `canItemQuickReplace` ignoring size: the slot is empty, or holds the same item and
    /// not more than a stack.
    fn can_quick_replace(&self, slot: usize, name: &str) -> bool {
        match &self.slots[slot] {
            None => true,
            Some((n, c)) => n == name && *c <= self.max_of(name),
        }
    }

    fn can_drag_to(&self, slot: usize, name: &str) -> bool {
        self.can_quick_replace(slot, name) && self.may_place(slot, name)
    }

    /// `getQuickCraftPlaceCount`: an even share, one each, or a full stack (creative).
    fn place_count(&self, slots: usize, kind: i32, name: &str, count: i32) -> i32 {
        match kind {
            0 => count / slots as i32,
            1 => 1,
            2 => self.max_of(name),
            _ => count,
        }
    }

    /// `Slot.safeInsert`: up to `amount` of the carried items into `slot`.
    fn insert(&mut self, slot: usize, amount: i32) {
        let Some((name, count)) = self.carried.clone() else {
            return;
        };
        if !self.may_place(slot, &name) {
            return;
        }
        let have = match &self.slots[slot] {
            None => 0,
            Some((n, c)) if *n == name => *c,
            Some(_) => return,
        };
        let moved = amount.min(count).min(self.max_in(slot, &name) - have);
        if moved > 0 {
            self.slots[slot] = Some((name.clone(), have + moved));
            self.carried = with_count(&name, count - moved);
        }
    }

    /// `AbstractContainerMenu.doClick`, as far as the inventory goes. Bundles, recipes and
    /// item behaviours on click aren't predicted; the server corrects them.
    fn do_click(&mut self, slot: i32, button: i32, input: i32, creative: bool) {
        if input == QUICK_CRAFT {
            let expected = self.qc_status;
            self.qc_status = button & 3;
            // Out of order, or nothing to spread.
            if (expected != 1 || self.qc_status != 2) && expected != self.qc_status
                || self.carried.is_none()
            {
                self.reset_quick_craft();
            } else if self.qc_status == 0 {
                self.qc_type = button >> 2 & 3;
                if self.qc_type < 2 || self.qc_type == 2 && creative {
                    self.qc_status = 1;
                    self.qc_slots.clear();
                } else {
                    self.reset_quick_craft();
                }
            } else if self.qc_status == 1 {
                let (name, count) = self.carried.clone().expect("checked above");
                let s = slot as usize;
                if s < SLOTS
                    && self.can_drag_to(s, &name)
                    && (self.qc_type == 2 || count > self.qc_slots.len() as i32)
                    && !self.qc_slots.contains(&s)
                {
                    self.qc_slots.push(s);
                }
            } else if self.qc_status == 2 {
                if self.qc_slots.len() == 1 {
                    let s = self.qc_slots[0];
                    let kind = self.qc_type;
                    self.reset_quick_craft();
                    self.do_click(s as i32, kind, PICKUP, creative);
                    return;
                }
                let (name, count) = self.carried.clone().expect("checked above");
                let mut remaining = count;
                let n = self.qc_slots.len();
                for s in self.qc_slots.clone() {
                    if self.can_drag_to(s, &name) && (self.qc_type == 2 || count >= n as i32) {
                        let carry = self.slots[s].as_ref().map_or(0, |(_, c)| *c);
                        let share = self.place_count(n, self.qc_type, &name, count);
                        let new = (share + carry).min(self.max_in(s, &name));
                        remaining -= new - carry;
                        self.slots[s] = Some((name.clone(), new));
                    }
                }
                self.carried = with_count(&name, remaining);
                self.reset_quick_craft();
            } else {
                self.reset_quick_craft();
            }
            return;
        }
        if self.qc_status != 0 {
            self.reset_quick_craft();
            return;
        }
        let primary = button == 0;
        match input {
            PICKUP | QUICK_MOVE if button == 0 || button == 1 => {
                if slot == OUTSIDE {
                    // Thrown out of the screen: all of it, or one.
                    if let Some((name, count)) = self.carried.clone() {
                        self.carried = if primary {
                            None
                        } else {
                            with_count(&name, count - 1)
                        };
                    }
                } else if slot < 0 || slot as usize >= SLOTS {
                } else if input == QUICK_MOVE {
                    let s = slot as usize;
                    let mut moved = self.quick_move_stack(s);
                    while let Some((name, _)) = &moved
                        && self.slots[s].as_ref().is_some_and(|(n, _)| n == name)
                    {
                        moved = self.quick_move_stack(s);
                    }
                } else {
                    self.pickup(slot as usize, primary);
                }
            }
            SWAP if (0..9).contains(&button) || button == 40 => {
                self.swap(
                    slot as usize,
                    if button == 40 {
                        45
                    } else {
                        36 + button as usize
                    },
                );
            }
            THROW if self.carried.is_none() && (0..SLOTS as i32).contains(&slot) => {
                let s = slot as usize;
                if let Some((name, count)) = self.slots[s].clone() {
                    self.slots[s] = if primary {
                        with_count(&name, count - 1)
                    } else {
                        None
                    };
                }
            }
            PICKUP_ALL if (0..SLOTS as i32).contains(&slot) => {
                self.pickup_all(slot as usize, button)
            }
            _ => {}
        }
    }

    /// A left (`primary`) or right click on a slot.
    fn pickup(&mut self, s: usize, primary: bool) {
        match (self.slots[s].clone(), self.carried.clone()) {
            (None, Some((_, count))) => self.insert(s, if primary { count } else { 1 }),
            (None, None) => {}
            // Take it all, or half rounded up.
            (Some((name, count)), None) => {
                let amount = if primary { count } else { (count + 1) / 2 };
                self.slots[s] = with_count(&name, count - amount);
                self.carried = Some((name, amount));
            }
            (Some((clicked, have)), Some((name, count))) => {
                if self.may_place(s, &name) {
                    if clicked == name {
                        self.insert(s, if primary { count } else { 1 });
                    } else if count <= self.max_in(s, &name) {
                        self.slots[s] = Some((name, count));
                        self.carried = Some((clicked, have));
                    }
                } else if clicked == name {
                    // Such as the crafting result: added to the cursor's stack.
                    let take = have.min(self.max_of(&name) - count);
                    if take > 0 {
                        self.slots[s] = with_count(&clicked, have - take);
                        self.carried = Some((name, count + take));
                    }
                }
            }
        }
    }

    /// `InventoryMenu.quickMoveStack`: where shift-clicking a slot sends its items. Returns
    /// what was in the slot, or `None` when nothing moved.
    fn quick_move_stack(&mut self, s: usize) -> Stack {
        let (name, count) = self.slots[s].clone()?;
        let worn = self.items.get(&name).and_then(|i| i.slot.clone());
        let armor = ARMOR
            .iter()
            .position(|a| Some(*a) == worn.as_deref())
            .map(|i| 5 + i);
        let mut left = count;
        let (start, end, backwards) = match s {
            0 => (9, 45, true),
            1..9 => (9, 45, false),
            _ if armor.is_some_and(|a| self.slots[a].is_none()) => {
                let a = armor.expect("checked");
                (a, a + 1, false)
            }
            _ if worn.as_deref() == Some("offhand") && self.slots[45].is_none() => (45, 46, false),
            9..36 => (36, 45, false),
            36..45 => (9, 36, false),
            _ => (9, 45, false),
        };
        if !self.move_to(&name, &mut left, start, end, backwards) {
            return None;
        }
        self.slots[s] = with_count(&name, left);
        (left != count).then_some((name, count))
    }

    /// `moveItemStackTo`: onto matching stacks in `start..end` first, then into the first
    /// empty slot that takes it.
    fn move_to(
        &mut self,
        name: &str,
        left: &mut i32,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> bool {
        let order: Vec<usize> = if backwards {
            (start..end).rev().collect()
        } else {
            (start..end).collect()
        };
        let mut changed = false;
        if self.max_of(name) > 1 {
            for &d in &order {
                if *left == 0 {
                    break;
                }
                let Some((n, c)) = self.slots[d].clone() else {
                    continue;
                };
                let max = self.max_in(d, name);
                if n == name && c < max {
                    let moved = (*left).min(max - c);
                    *left -= moved;
                    self.slots[d] = Some((n, c + moved));
                    changed = true;
                }
            }
        }
        if *left > 0 {
            for &d in &order {
                if self.slots[d].is_none() && self.may_place(d, name) {
                    let moved = (*left).min(self.max_in(d, name));
                    self.slots[d] = Some((name.to_owned(), moved));
                    *left -= moved;
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    /// A number key (or F for the offhand) over a slot: swaps it with that hotbar slot.
    fn swap(&mut self, s: usize, source: usize) {
        if s >= SLOTS {
            return;
        }
        match (self.slots[source].clone(), self.slots[s].clone()) {
            (None, None) => {}
            (None, Some(target)) => {
                (self.slots[source], self.slots[s]) = (Some(target), None);
            }
            (Some((name, count)), target) => {
                if !self.may_place(s, &name) {
                    return;
                }
                let max = self.max_in(s, &name);
                if count > max {
                    // Splits; a target that was there goes elsewhere in the inventory,
                    // which the server works out.
                    self.slots[s] = Some((name.clone(), max));
                    self.slots[source] = Some((name, count - max));
                } else {
                    self.slots[source] = target;
                    self.slots[s] = Some((name, count));
                }
            }
        }
    }

    /// A double click: gathers matching items onto the cursor, partial stacks first.
    fn pickup_all(&mut self, s: usize, button: i32) {
        let Some((name, mut count)) = self.carried.clone() else {
            return;
        };
        if self.slots[s].is_some() {
            return;
        }
        let max = self.max_of(&name);
        let order: Vec<usize> = if button == 0 {
            (0..SLOTS).collect()
        } else {
            (0..SLOTS).rev().collect()
        };
        for pass in 0..2 {
            for &i in &order {
                if count >= max {
                    break;
                }
                // `InventoryMenu.canTakeItemForPickAll`: not from the crafting result.
                if i == 0 {
                    continue;
                }
                if let Some((n, c)) = self.slots[i].clone()
                    && n == name
                    && c <= max
                    && (pass != 0 || c != max)
                {
                    let take = c.min(max - count);
                    self.slots[i] = with_count(&n, c - take);
                    count += take;
                }
            }
        }
        self.carried = Some((name, count));
    }

    fn reset_quick_craft(&mut self) {
        self.qc_status = 0;
        self.qc_slots.clear();
    }

    fn reset_drag(&mut self) {
        self.dragging = None;
        self.drag_slots.clear();
    }

    /// `HashedStack`: present, then the item, count and no component changes (stacks with
    /// components hash differently, so the server sends those slots back as they are).
    fn put_hashed(&self, body: &mut Vec<u8>, stack: &Stack) {
        match stack
            .as_ref()
            .and_then(|(n, c)| Some((self.items.get(n)?.id, *c)))
        {
            Some((id, count)) => {
                body.put_bool(true)
                    .put_varint(id as i32)
                    .put_varint(count)
                    .put_varint(0)
                    .put_varint(0);
            }
            None => {
                body.put_bool(false);
            }
        }
    }

    /// `MultiPlayerGameMode.handleContainerInput`: predicts the click, then sends it with
    /// the slots it changed and the cursor's item.
    fn click(
        &mut self,
        outbox: &mc_protocol::Outbox,
        slot: i32,
        button: i32,
        input: i32,
        creative: bool,
    ) -> anyhow::Result<()> {
        let before = self.slots.clone();
        self.do_click(slot, button, input, creative);
        let changed: Vec<usize> = (0..SLOTS).filter(|&i| before[i] != self.slots[i]).collect();
        let mut body = Vec::new();
        body.put_varint(0)
            .put_varint(self.state_id)
            .put_u16(slot as i16 as u16)
            .put_u8(button as u8)
            .put_varint(input)
            .put_varint(changed.len() as i32);
        for i in changed {
            body.put_u16(i as u16);
            self.put_hashed(&mut body, &self.slots[i]);
        }
        self.put_hashed(&mut body, &self.carried.clone());
        outbox.send("container_click", &body)
    }

    /// `AbstractContainerScreen.mouseClicked`: `button` 0 left, 1 right, 2 middle, over
    /// `hovered` (or `outside` the screen).
    #[allow(clippy::too_many_arguments)]
    pub fn mouse_down(
        &mut self,
        outbox: &mc_protocol::Outbox,
        button: u8,
        shift: bool,
        hovered: Option<usize>,
        outside: bool,
        creative: bool,
    ) -> anyhow::Result<()> {
        let now = Instant::now();
        // `MouseHandler`: a second press of the same button within 250 ms.
        self.double_click = self.last_click.is_some_and(|(slot, b, at)| {
            slot == hovered && b == button && now - at < Duration::from_millis(250)
        });
        self.last_click = Some((hovered, button, now));
        self.skip_next_release = false;
        let cloning = button == 2 && creative;
        if button > 1 && !cloning {
            return Ok(());
        }
        let slot = if outside {
            OUTSIDE
        } else {
            hovered.map_or(-1, |s| s as i32)
        };
        if slot == -1 || self.dragging.is_some() {
            return Ok(());
        }
        if self.carried.is_none() {
            let input = if cloning {
                3
            } else if shift && slot != OUTSIDE {
                QUICK_MOVE
            } else if slot == OUTSIDE {
                THROW
            } else {
                PICKUP
            };
            // `getContainerClickButton`: middle clicks go as button 2.
            self.click(outbox, slot, button as i32, input, creative)?;
            self.skip_next_release = true;
        } else {
            self.drag_slots.clear();
            self.dragging = Some((button, if cloning { 2 } else { button as i32 }));
        }
        Ok(())
    }

    /// `AbstractContainerScreen.mouseDragged`: a drag with items on the cursor adds each
    /// slot it passes over that can take them.
    pub fn mouse_moved(&mut self, hovered: Option<usize>) {
        let (Some((_, kind)), Some(s), Some((name, count))) =
            (self.dragging, hovered, self.carried.clone())
        else {
            return;
        };
        if (count > self.drag_slots.len() as i32 || kind == 2)
            && self.can_drag_to(s, &name)
            && !self.drag_slots.contains(&s)
        {
            self.drag_slots.push(s);
        }
    }

    /// `AbstractContainerScreen.mouseReleased`.
    pub fn mouse_up(
        &mut self,
        outbox: &mc_protocol::Outbox,
        button: u8,
        shift: bool,
        hovered: Option<usize>,
        outside: bool,
        creative: bool,
    ) -> anyhow::Result<()> {
        let slot = if outside {
            OUTSIDE
        } else {
            hovered.map_or(-1, |s| s as i32)
        };
        if self.double_click && hovered.is_some() && button == 0 {
            // Shift-double-click (moving every matching stack) isn't done yet.
            if !shift {
                self.click(outbox, slot, 0, PICKUP_ALL, creative)?;
            }
            self.double_click = false;
        } else {
            if let Some((b, _)) = self.dragging
                && b != button
            {
                self.reset_drag();
                self.skip_next_release = true;
                return Ok(());
            }
            if self.skip_next_release {
                self.skip_next_release = false;
                return Ok(());
            }
            if let (Some((_, kind)), false) = (self.dragging, self.drag_slots.is_empty()) {
                // `quickCraftToSlots`: start, each slot, end.
                let mask = |header: i32| header | kind << 2;
                self.click(outbox, OUTSIDE, mask(0), QUICK_CRAFT, creative)?;
                for s in self.drag_slots.clone() {
                    self.click(outbox, s as i32, mask(1), QUICK_CRAFT, creative)?;
                }
                self.click(outbox, OUTSIDE, mask(2), QUICK_CRAFT, creative)?;
            } else if self.carried.is_some() {
                let input = if shift && slot != OUTSIDE {
                    QUICK_MOVE
                } else {
                    PICKUP
                };
                self.click(outbox, slot, button.min(1) as i32, input, creative)?;
            }
        }
        self.reset_drag();
        Ok(())
    }

    /// `AbstractContainerScreen.keyPressed`'s slot keys over `hovered`: a hotbar number (0
    /// to 8) swaps with it, F (40) with the offhand, and Q throws one, or the stack with
    /// Ctrl.
    pub fn key(
        &mut self,
        outbox: &mc_protocol::Outbox,
        key: InventoryKey,
        hovered: Option<usize>,
        creative: bool,
    ) -> anyhow::Result<()> {
        let Some(s) = hovered else { return Ok(()) };
        match key {
            InventoryKey::Hotbar(n) if self.carried.is_none() => {
                self.click(outbox, s as i32, n as i32, SWAP, creative)
            }
            InventoryKey::Offhand if self.carried.is_none() => {
                self.click(outbox, s as i32, 40, SWAP, creative)
            }
            InventoryKey::Drop { all } if self.slots[s].is_some() => {
                self.click(outbox, s as i32, all as i32, THROW, creative)
            }
            _ => Ok(()),
        }
    }

    /// What the screen draws: during a drag over more than one slot, each slot with its
    /// share and the cursor with what's left (`extractSlot`, `recalculateQuickCraftRemaining`).
    pub fn view(&self, mouse: Option<[f32; 2]>) -> mc_render::hud::inventory::Inventory {
        let mut slots = self.slots.clone();
        let mut carried = self.carried.clone();
        let mut dragged = Vec::new();
        if let (Some((_, kind)), Some((name, count))) = (self.dragging, &self.carried)
            && self.drag_slots.len() > 1
        {
            let n = self.drag_slots.len();
            let mut remaining = if kind == 2 { self.max_of(name) } else { *count };
            for &s in &self.drag_slots {
                let carry = slots[s].as_ref().map_or(0, |(_, c)| *c);
                let share = self.place_count(n, kind, name, *count);
                let new = (share + carry).min(self.max_in(s, name));
                if kind != 2 {
                    remaining -= new - carry;
                }
                slots[s] = Some((name.clone(), new));
                dragged.push(s as u8);
            }
            carried = Some((name.clone(), remaining));
        }
        mc_render::hud::inventory::Inventory {
            slots,
            carried,
            dragged,
            mouse,
        }
    }
}

/// A key that acts on the hovered slot.
pub enum InventoryKey {
    Hotbar(u8),
    Offhand,
    Drop { all: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory() -> Inventory {
        Inventory::new(&mc_registry::Registry::vanilla().unwrap())
    }

    fn stack(name: &str, count: i32) -> Stack {
        Some((format!("minecraft:{name}"), count))
    }

    /// Clicks change the slots as vanilla's `doClick` does: a right click takes half rounded
    /// up, armour slots take only their own armour, one at a time, a shift-click moves a
    /// stack from the inventory onto the hotbar's matching stack first, and a drag spreads
    /// the cursor's items evenly, keeping the rest.
    #[test]
    fn predicts_clicks_like_vanilla() {
        let mut inv = inventory();
        inv.slots[9] = stack("stone", 7);
        inv.do_click(9, 1, PICKUP, false);
        assert_eq!(
            (&inv.slots[9], &inv.carried),
            (&stack("stone", 3), &stack("stone", 4))
        );

        // Stone won't go on the head; a helmet will, and a pumpkin too.
        inv.do_click(5, 0, PICKUP, false);
        assert_eq!((&inv.slots[5], &inv.carried), (&None, &stack("stone", 4)));
        inv.carried = stack("diamond_helmet", 1);
        inv.do_click(6, 0, PICKUP, false);
        assert_eq!(inv.slots[6], None);
        inv.do_click(5, 0, PICKUP, false);
        assert_eq!(
            (&inv.slots[5], &inv.carried),
            (&stack("diamond_helmet", 1), &None)
        );

        // 60 on the hotbar's second slot fills to 64, and the other 26 go to its first
        // empty slot.
        inv.slots[37] = stack("stone", 60);
        inv.slots[10] = stack("stone", 30);
        inv.do_click(10, 0, QUICK_MOVE, false);
        assert_eq!(
            (&inv.slots[10], &inv.slots[36], &inv.slots[37]),
            (&None, &stack("stone", 26), &stack("stone", 64))
        );

        // 10 ender pearls (16 a stack) dragged over three slots: 3 each, 1 left.
        inv.carried = stack("ender_pearl", 10);
        let mask = |header: i32| header;
        inv.do_click(OUTSIDE, mask(0), QUICK_CRAFT, false);
        for s in [20, 21, 22] {
            inv.do_click(s, mask(1), QUICK_CRAFT, false);
        }
        inv.do_click(OUTSIDE, mask(2), QUICK_CRAFT, false);
        assert_eq!(
            inv.slots[20..23],
            [
                stack("ender_pearl", 3),
                stack("ender_pearl", 3),
                stack("ender_pearl", 3)
            ]
        );
        assert_eq!(inv.carried, stack("ender_pearl", 1));
    }
}
