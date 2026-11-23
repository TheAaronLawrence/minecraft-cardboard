//! The open screen's menu as the client keeps it: the player's own (`InventoryMenu`'s 46
//! slots), or a chest, crafting table or furnace the server opened, whose own slots come
//! before the player's inventory and hotbar; the item on the cursor; and
//! `AbstractContainerScreen`'s mouse and keys. Each click is predicted as
//! `AbstractContainerMenu.doClick` does and sent as `MultiPlayerGameMode.handleContainerInput`
//! does, with the slots it changed, so the server only corrects what we got wrong.
//!
//! The player's slots: 0 the crafting result, 1 to 4 the grid, 5 to 8 armour (head first),
//! 9 to 35 the inventory, 36 to 44 the hotbar, 45 the offhand. A container's slot `own + j`
//! is the player's slot `9 + j`, as `addStandardInventorySlots` adds them.

use mc_protocol::codec::Put;
use mc_render::hud::inventory::Menu;
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
/// The player's first inventory slot, which a container's slot `own` is.
const INVENTORY: usize = 9;
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

/// A menu the server opened (`ClientboundOpenScreenPacket`).
struct Container {
    id: i32,
    menu: Menu,
    title: String,
    /// The menu's own slots; the rest are the player's.
    own: Vec<Stack>,
    state_id: i32,
    /// `ContainerData`: a furnace's lit time, lit duration, cooking progress and cooking
    /// total time.
    data: [i32; 4],
}

pub struct Inventory {
    /// The player's `InventoryMenu` slots, which a container shows from slot 9 on.
    pub slots: Vec<Stack>,
    pub carried: Stack,
    /// The server's count of the player menu's changes, which each click echoes.
    state_id: i32,
    items: HashMap<String, Item>,
    open: bool,
    /// The container open, if it isn't the player's own menu.
    container: Option<Container>,
    /// The server closed the screen, so the game takes the mouse back.
    closed_by_server: bool,
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
            container: None,
            closed_by_server: false,
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

    /// The server set these slots of menu `container` (and maybe its state id): the
    /// player's (0), or the open container's. Another menu's are stale and go.
    pub fn set(&mut self, container: i32, state_id: Option<i32>, slots: Vec<(u8, Stack)>) {
        if container == 0 {
            if let Some(id) = state_id {
                self.state_id = id;
            }
            for (slot, stack) in slots {
                if let Some(s) = self.slots.get_mut(slot as usize) {
                    *s = stack;
                }
            }
            return;
        }
        let Some(c) = self.container.as_mut().filter(|c| c.id == container) else {
            return;
        };
        if let Some(id) = state_id {
            c.state_id = id;
        }
        let len = c.menu.slot_count();
        for (slot, stack) in slots {
            if (slot as usize) < len {
                self.put(slot as usize, stack);
            }
        }
    }

    /// Which menu is open (the player's when none is).
    pub fn menu(&self) -> Menu {
        self.container.as_ref().map_or(Menu::Player, |c| c.menu)
    }

    fn len(&self) -> usize {
        self.menu().slot_count()
    }

    /// The open menu's slot `i`.
    fn get(&self, i: usize) -> &Stack {
        match &self.container {
            Some(c) if i < c.own.len() => &c.own[i],
            Some(c) => &self.slots[INVENTORY + i - c.own.len()],
            None => &self.slots[i],
        }
    }

    fn put(&mut self, i: usize, stack: Stack) {
        match &mut self.container {
            Some(c) if i < c.own.len() => c.own[i] = stack,
            Some(c) => self.slots[INVENTORY + i - c.own.len()] = stack,
            None => self.slots[i] = stack,
        }
    }

    /// The open menu's container id and state id, which clicks and closing send.
    fn ids(&self) -> (i32, i32) {
        self.container
            .as_ref()
            .map_or((0, self.state_id), |c| (c.id, c.state_id))
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.container = None;
        self.last_click = None;
    }

    /// `ClientPacketListener.handleOpenScreen`: the server opened a container, over any
    /// screen that was open (`MenuScreens.create`).
    pub fn open_container(&mut self, id: i32, menu: Menu, title: String) {
        self.reset_drag();
        self.reset_quick_craft();
        self.container = Some(Container {
            id,
            menu,
            title,
            own: vec![None; menu.own()],
            state_id: 0,
            data: [0; 4],
        });
        self.open = true;
        self.last_click = None;
    }

    /// `handleContainerSetData`.
    pub fn set_data(&mut self, container: i32, id: i16, value: i16) {
        if let Some(c) = self.container.as_mut().filter(|c| c.id == container)
            && let Some(d) = c.data.get_mut(id as usize)
        {
            *d = value as i32;
        }
    }

    /// `handleContainerClose`: `LocalPlayer.clientSideCloseContainer`, which closes the
    /// screen without telling the server.
    pub fn server_close(&mut self, container: i32) {
        if self.open && self.ids().0 == container {
            self.open = false;
            self.carried = None;
            self.container = None;
            self.reset_drag();
            self.reset_quick_craft();
            self.closed_by_server = true;
        }
    }

    /// Whether the server closed the screen since this was last asked.
    pub fn take_closed_by_server(&mut self) -> bool {
        std::mem::take(&mut self.closed_by_server)
    }

    /// `AbstractContainerScreen.onClose`: `ServerboundContainerClosePacket`, then the menu's
    /// `removed` drops what's on the cursor (the server drops it in the world); the player's
    /// clears its crafting result, and the server moves crafting grids back into the
    /// inventory.
    pub fn close(&mut self, outbox: &mc_protocol::Outbox) -> anyhow::Result<()> {
        let id = self.ids().0;
        self.open = false;
        self.carried = None;
        if self.container.take().is_none() {
            self.slots[0] = None;
        }
        self.reset_drag();
        self.reset_quick_craft();
        let mut body = Vec::new();
        body.put_varint(id);
        outbox.send("container_close", &body)
    }

    /// Closes a menu the server opened that this game can't show yet.
    pub fn refuse(outbox: &mc_protocol::Outbox, id: i32) -> anyhow::Result<()> {
        let mut body = Vec::new();
        body.put_varint(id);
        outbox.send("container_close", &body)
    }

    fn max_of(&self, name: &str) -> i32 {
        self.items.get(name).map_or(64, |i| i.max)
    }

    /// `Slot.getMaxStackSize(stack)`: armour slots hold one, the rest a container's 99.
    fn max_in(&self, slot: usize, name: &str) -> i32 {
        let armour = self.menu() == Menu::Player && (5..9).contains(&slot);
        self.max_of(name).min(if armour { 1 } else { 99 })
    }

    /// The menu's result slot, which takes nothing (`ResultSlot`, `FurnaceResultSlot`).
    fn is_result(&self, slot: usize) -> bool {
        match self.menu() {
            Menu::Player | Menu::Crafting => slot == 0,
            Menu::Furnace => slot == 2,
            Menu::Chest(_) => false,
        }
    }

    /// `Slot.mayPlace`: nothing goes into a result, and armour only where it's worn. A
    /// furnace's fuel slot takes only fuel, which isn't known here; the server corrects it.
    fn may_place(&self, slot: usize, name: &str) -> bool {
        match slot {
            _ if self.is_result(slot) => false,
            5..9 if self.menu() == Menu::Player => {
                let worn = self.items.get(name).and_then(|i| i.slot.as_deref());
                worn == Some(ARMOR[slot - 5])
            }
            _ => true,
        }
    }

    /// The open menu's slot for hotbar slot `n` (0 to 8), or 40 for the offhand, which only
    /// the player's menu has.
    fn hotbar_slot(&self, n: i32) -> Option<usize> {
        match (self.menu(), n) {
            (Menu::Player, 40) => Some(45),
            (Menu::Player, _) => Some(36 + n as usize),
            (_, 40) => None,
            (menu, _) => Some(menu.own() + 27 + n as usize),
        }
    }

    /// `canItemQuickReplace` ignoring size: the slot is empty, or holds the same item and
    /// not more than a stack.
    fn can_quick_replace(&self, slot: usize, name: &str) -> bool {
        match self.get(slot) {
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
        let have = match self.get(slot) {
            None => 0,
            Some((n, c)) if *n == name => *c,
            Some(_) => return,
        };
        let moved = amount.min(count).min(self.max_in(slot, &name) - have);
        if moved > 0 {
            self.put(slot, Some((name.clone(), have + moved)));
            self.carried = with_count(&name, count - moved);
        }
    }

    /// `AbstractContainerMenu.doClick` on the open menu. Bundles, recipes and item
    /// behaviours on click aren't predicted; the server corrects them.
    fn do_click(&mut self, slot: i32, button: i32, input: i32, creative: bool) {
        let len = self.len();
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
                if s < len
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
                        let carry = self.get(s).as_ref().map_or(0, |(_, c)| *c);
                        let share = self.place_count(n, self.qc_type, &name, count);
                        let new = (share + carry).min(self.max_in(s, &name));
                        remaining -= new - carry;
                        self.put(s, Some((name.clone(), new)));
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
                } else if slot < 0 || slot as usize >= len {
                } else if input == QUICK_MOVE {
                    let s = slot as usize;
                    let mut moved = self.quick_move_stack(s);
                    while let Some((name, _)) = &moved
                        && self.get(s).as_ref().is_some_and(|(n, _)| n == name)
                    {
                        moved = self.quick_move_stack(s);
                    }
                } else {
                    self.pickup(slot as usize, primary);
                }
            }
            SWAP if (0..9).contains(&button) || button == 40 => {
                if let Some(source) = self.hotbar_slot(button)
                    && (0..len as i32).contains(&slot)
                {
                    self.swap(slot as usize, source);
                }
            }
            THROW if self.carried.is_none() && (0..len as i32).contains(&slot) => {
                let s = slot as usize;
                if let Some((name, count)) = self.get(s).clone() {
                    let left = if primary {
                        with_count(&name, count - 1)
                    } else {
                        None
                    };
                    self.put(s, left);
                }
            }
            PICKUP_ALL if (0..len as i32).contains(&slot) => self.pickup_all(slot as usize, button),
            _ => {}
        }
    }

    /// A left (`primary`) or right click on a slot.
    fn pickup(&mut self, s: usize, primary: bool) {
        match (self.get(s).clone(), self.carried.clone()) {
            (None, Some((_, count))) => self.insert(s, if primary { count } else { 1 }),
            (None, None) => {}
            // Take it all, or half rounded up.
            (Some((name, count)), None) => {
                let amount = if primary { count } else { (count + 1) / 2 };
                self.put(s, with_count(&name, count - amount));
                self.carried = Some((name, amount));
            }
            (Some((clicked, have)), Some((name, count))) => {
                if self.may_place(s, &name) {
                    if clicked == name {
                        self.insert(s, if primary { count } else { 1 });
                    } else if count <= self.max_in(s, &name) {
                        self.put(s, Some((name, count)));
                        self.carried = Some((clicked, have));
                    }
                } else if clicked == name {
                    // Such as the crafting result: added to the cursor's stack.
                    let take = have.min(self.max_of(&name) - count);
                    if take > 0 {
                        self.put(s, with_count(&clicked, have - take));
                        self.carried = Some((name, count + take));
                    }
                }
            }
        }
    }

    /// The open menu's `quickMoveStack`: where shift-clicking a slot sends its items.
    /// Returns what was in the slot, or `None` when nothing moved.
    fn quick_move_stack(&mut self, s: usize) -> Stack {
        let (name, count) = self.get(s).clone()?;
        let mut left = count;
        let menu = self.menu();
        let own = menu.own();
        let mut to = |inv: &mut Self, start: usize, end: usize, backwards: bool| {
            inv.move_to(&name, &mut left, start, end, backwards)
        };
        let moved = match menu {
            // `InventoryMenu`.
            Menu::Player => {
                let worn = self.items.get(&name).and_then(|i| i.slot.clone());
                let armor = ARMOR
                    .iter()
                    .position(|a| Some(*a) == worn.as_deref())
                    .map(|i| 5 + i);
                match s {
                    0 => to(self, 9, 45, true),
                    1..9 => to(self, 9, 45, false),
                    _ if armor.is_some_and(|a| self.slots[a].is_none()) => {
                        let a = armor.expect("checked");
                        to(self, a, a + 1, false)
                    }
                    _ if worn.as_deref() == Some("offhand") && self.slots[45].is_none() => {
                        to(self, 45, 46, false)
                    }
                    9..36 => to(self, 36, 45, false),
                    36..45 => to(self, 9, 36, false),
                    _ => to(self, 9, 45, false),
                }
            }
            // `ChestMenu`: out of the chest into the player's slots, hotbar end first;
            // from the player's into the chest.
            Menu::Chest(_) if s < own => to(self, own, menu.slot_count(), true),
            Menu::Chest(_) => to(self, 0, own, false),
            // `CraftingMenu`: the result into the player's slots; theirs into the grid,
            // failing that between the inventory and the hotbar.
            Menu::Crafting => match s {
                0 => to(self, 10, 46, true),
                10..37 => to(self, 1, 10, false) || to(self, 37, 46, false),
                37..46 => to(self, 1, 10, false) || to(self, 10, 37, false),
                _ => to(self, 10, 46, false),
            },
            // `AbstractFurnaceMenu`: whether the player's item smelts or burns isn't known
            // here, so the server moves those.
            Menu::Furnace => match s {
                2 => to(self, 3, 39, true),
                0 | 1 => to(self, 3, 39, false),
                _ => return None,
            },
        };
        if !moved {
            return None;
        }
        self.put(s, with_count(&name, left));
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
                let Some((n, c)) = self.get(d).clone() else {
                    continue;
                };
                let max = self.max_in(d, name);
                if n == name && c < max {
                    let moved = (*left).min(max - c);
                    *left -= moved;
                    self.put(d, Some((n, c + moved)));
                    changed = true;
                }
            }
        }
        if *left > 0 {
            for &d in &order {
                if self.get(d).is_none() && self.may_place(d, name) {
                    let moved = (*left).min(self.max_in(d, name));
                    self.put(d, Some((name.to_owned(), moved)));
                    *left -= moved;
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    /// A number key (or F for the offhand) over a slot: swaps it with the menu's slot
    /// `source` for that hotbar slot.
    fn swap(&mut self, s: usize, source: usize) {
        match (self.get(source).clone(), self.get(s).clone()) {
            (None, None) => {}
            (None, Some(target)) => {
                self.put(source, Some(target));
                self.put(s, None);
            }
            (Some((name, count)), target) => {
                if !self.may_place(s, &name) {
                    return;
                }
                let max = self.max_in(s, &name);
                if count > max {
                    // Splits; a target that was there goes elsewhere in the inventory,
                    // which the server works out.
                    self.put(s, Some((name.clone(), max)));
                    self.put(source, Some((name, count - max)));
                } else {
                    self.put(source, target);
                    self.put(s, Some((name, count)));
                }
            }
        }
    }

    /// A double click: gathers matching items onto the cursor, partial stacks first.
    fn pickup_all(&mut self, s: usize, button: i32) {
        let Some((name, mut count)) = self.carried.clone() else {
            return;
        };
        if self.get(s).is_some() {
            return;
        }
        let max = self.max_of(&name);
        let len = self.len();
        let order: Vec<usize> = if button == 0 {
            (0..len).collect()
        } else {
            (0..len).rev().collect()
        };
        let crafted = matches!(self.menu(), Menu::Player | Menu::Crafting);
        for pass in 0..2 {
            for &i in &order {
                if count >= max {
                    break;
                }
                // `canTakeItemForPickAll`: not from a crafting result.
                if crafted && i == 0 {
                    continue;
                }
                if let Some((n, c)) = self.get(i).clone()
                    && n == name
                    && c <= max
                    && (pass != 0 || c != max)
                {
                    let take = c.min(max - count);
                    self.put(i, with_count(&n, c - take));
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
        let len = self.len();
        let before: Vec<Stack> = (0..len).map(|i| self.get(i).clone()).collect();
        self.do_click(slot, button, input, creative);
        let changed: Vec<usize> = (0..len).filter(|&i| before[i] != *self.get(i)).collect();
        let (container, state_id) = self.ids();
        let mut body = Vec::new();
        body.put_varint(container)
            .put_varint(state_id)
            .put_u16(slot as i16 as u16)
            .put_u8(button as u8)
            .put_varint(input)
            .put_varint(changed.len() as i32);
        for i in changed {
            body.put_u16(i as u16);
            self.put_hashed(&mut body, self.get(i));
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
            InventoryKey::Drop { all } if self.get(s).is_some() => {
                self.click(outbox, s as i32, all as i32, THROW, creative)
            }
            _ => Ok(()),
        }
    }

    /// What the screen draws: during a drag over more than one slot, each slot with its
    /// share and the cursor with what's left (`extractSlot`, `recalculateQuickCraftRemaining`).
    pub fn view(
        &self,
        mouse: Option<[f32; 2]>,
        effects: Vec<mc_render::hud::ActiveEffect>,
    ) -> mc_render::hud::inventory::Inventory {
        let mut slots: Vec<Stack> = (0..self.len()).map(|i| self.get(i).clone()).collect();
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
        // `AbstractFurnaceMenu.getLitProgress` and `getBurnProgress`.
        let [lit, duration, progress, total] = self.container.as_ref().map_or([0; 4], |c| c.data);
        let fraction = |a: i32, b: i32| (a as f32 / b as f32).clamp(0.0, 1.0);
        mc_render::hud::inventory::Inventory {
            menu: self.menu(),
            title: self
                .container
                .as_ref()
                .map_or(String::new(), |c| c.title.clone()),
            slots,
            carried,
            dragged,
            mouse,
            lit: (lit > 0).then(|| fraction(lit, if duration == 0 { 200 } else { duration })),
            burn: if total != 0 && progress != 0 {
                fraction(progress, total)
            } else {
                0.0
            },
            effects,
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

    /// In a three-row chest the player's slot 9 is the menu's 27: shift-clicking it fills
    /// the chest's first slot, and shift-clicking that sends it to the hotbar's last slot
    /// first, as `ChestMenu.quickMoveStack` moves backwards. In a crafting table the
    /// player's items go into the grid.
    #[test]
    fn predicts_container_clicks_like_vanilla() {
        let mut inv = inventory();
        inv.slots[9] = stack("stone", 10);
        inv.open_container(3, Menu::Chest(3), "Chest".into());
        assert_eq!(inv.get(27), &stack("stone", 10));
        inv.do_click(27, 0, QUICK_MOVE, false);
        assert_eq!((inv.get(0), &inv.slots[9]), (&stack("stone", 10), &None));
        inv.do_click(0, 0, QUICK_MOVE, false);
        assert_eq!((inv.get(0), &inv.slots[44]), (&None, &stack("stone", 10)));
        // The hotbar key 1 over a chest slot swaps with the hotbar's first slot (menu 54).
        inv.slots[36] = stack("dirt", 2);
        inv.do_click(5, 0, SWAP, false);
        assert_eq!((inv.get(5), &inv.slots[36]), (&stack("dirt", 2), &None));

        // The server's slot updates for the chest land in the player's slots past its own.
        inv.set(3, Some(7), vec![(30, stack("oak_log", 4))]);
        assert_eq!(inv.slots[12], stack("oak_log", 4));

        inv.open_container(4, Menu::Crafting, "Crafting".into());
        inv.do_click(10 + 3, 0, QUICK_MOVE, false);
        assert_eq!((inv.get(1), &inv.slots[12]), (&stack("oak_log", 4), &None));
    }
}
