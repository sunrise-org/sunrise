//! Sorting (`sort`, `sortKey`; docs/extras.md): buttons by the storage and inventory, and a key.
use crate::plan::{Item, plan};
use crate::settings;
use sunrise_platform::containers::{Button, Click, Container, Role, SlotId};
use sunrise_platform::hud::Canvas;
use sunrise_platform::items::creative_order;

/// A sort button's side in GUI pixels.
const BUTTON: f32 = 9.0;
const EDGE: u32 = 0xFF37_3737;
const FACE: u32 = 0xFFA0_A0A0;
const FACE_LIT: u32 = 0xFFDC_DCDC;

fn chest_like(menu: &str) -> bool {
    menu.starts_with("minecraft:generic_9x") || menu == "minecraft:shulker_box"
}

/// The open screen's sort buttons, by the title and "Inventory" lines: corner and slots sorted.
fn buttons(c: &dyn Container) -> Vec<((f32, f32), Vec<SlotId>)> {
    let menu = c.menu();
    if !settings::sort() || !(menu == "inventory" || chest_like(menu)) {
        return Vec::new();
    }
    let slots = c.slots();
    let of = |role: Role| -> Vec<SlotId> {
        let slots = slots.iter().filter(|s| s.role == role);
        slots.map(|s| s.id).collect()
    };
    let panel = c.panel();
    let mut buttons = Vec::new();
    let storage = of(Role::Storage);
    if chest_like(menu) && !storage.is_empty() {
        buttons.push(((160.0, panel.title_y - 1.0), storage));
    }
    buttons.push(((160.0, panel.inventory_y - 1.0), of(Role::Inventory)));
    buttons
}

fn under(c: &dyn Container, (x, y): (f32, f32)) -> bool {
    let (cx, cy) = c.cursor();
    cx >= x && cx < x + BUTTON && cy >= y && cy < y + BUTTON
}

/// The buttons: a bordered square with three bars, lighter under the cursor.
pub(crate) fn draw(c: &dyn Container, canvas: &mut dyn Canvas) {
    for (at @ (x, y), _) in buttons(c) {
        let face = if under(c, at) { FACE_LIT } else { FACE };
        canvas.fill(x, y, BUTTON, BUTTON, EDGE);
        canvas.fill(x + 1.0, y + 1.0, BUTTON - 2.0, BUTTON - 2.0, face);
        for (i, w) in [5.0, 4.0, 3.0].into_iter().enumerate() {
            canvas.fill(x + 2.0, y + 2.0 + 2.0 * i as f32, w, 1.0, EDGE);
        }
    }
}

/// A mouse button as the key mappings name it.
fn mouse_name(button: Button) -> String {
    match button {
        Button::Left => "key.mouse.left".into(),
        Button::Right => "key.mouse.right".into(),
        Button::Middle => "key.mouse.middle".into(),
        Button::Other(n) => format!("key.mouse.{}", u32::from(n) + 1),
    }
}

/// A left press on a sort button, or the sort key as a mouse button: true when it sorted.
pub(crate) fn press(c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool {
    if button == Button::Left
        && let Some((_, slots)) = buttons(c).into_iter().find(|(at, _)| under(c, *at))
    {
        sort(c, &slots);
        return true;
    }
    key(c, slot, &mouse_name(button))
}

/// The sort key: sorts the part under the cursor (the container's when it is over no slot).
pub(crate) fn key(c: &mut dyn Container, slot: Option<SlotId>, key: &str) -> bool {
    if !settings::sort() || settings::sort_key() != key {
        return false;
    }
    let buttons = buttons(c);
    let slots = buttons
        .iter()
        .find(|(_, slots)| slot.is_some_and(|s| slots.contains(&s)))
        .or(buttons.first());
    let Some((_, slots)) = slots else {
        return false;
    };
    let slots = slots.clone();
    sort(c, &slots);
    true
}

/// Sorts `slots` with ordinary left clicks; it needs an empty hand.
fn sort(c: &mut dyn Container, slots: &[SlotId]) {
    if c.carried().is_some() {
        return;
    }
    let items: Vec<Option<Item>> = slots
        .iter()
        .map(|&slot| {
            let stack = c.stack(slot)?;
            let order = creative_order(&stack).unwrap_or(u32::MAX);
            Some(Item {
                kind: stack.kind,
                count: stack.count,
                max: stack.max,
                rank: (order, c.name(slot).unwrap_or_default()),
            })
        })
        .collect();
    for i in plan(&items) {
        c.click(slots[i], Click::Pick { right: false });
    }
}
