//! Mouse Tweaks-style drags and wheel (`mouseTweaks`, `mouseTweaksWheel`; docs/extras.md).
use crate::settings;
use std::sync::Mutex;
use sunrise_platform::containers::{Button, Click, Container, Role, Slot, SlotId, Stack};

/// The menus the controls run in; not those whose slots cost or trigger something (anvils...).
fn runs_in(menu: &str) -> bool {
    menu == "inventory"
        || menu.starts_with("minecraft:generic_9x")
        || matches!(
            menu,
            "minecraft:shulker_box"
                | "minecraft:hopper"
                | "minecraft:generic_3x3"
                | "minecraft:furnace"
                | "minecraft:blast_furnace"
                | "minecraft:smoker"
                | "minecraft:crafting"
        )
}

/// A drag the controls follow, from the press the hooks heard to its release.
struct Drag {
    right: bool,
    /// The hand's kind, count and max, kept by the drag's own clicks since a server answers late.
    held: Option<(u64, u32, u32)>,
    /// The drag replaced vanilla's.
    active: bool,
    visited: Vec<SlotId>,
}

#[derive(Default)]
struct State {
    drag: Option<Drag>,
    /// Wheel steps not yet acted on (a trackpad scrolls in fractions).
    wheel: f32,
}

static STATE: Mutex<State> = Mutex::new(State {
    drag: None,
    wheel: 0.0,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn furnace(menu: &str) -> bool {
    matches!(
        menu,
        "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker"
    )
}

fn on(c: &dyn Container) -> bool {
    settings::mouse_tweaks() && runs_in(c.menu())
}

fn role(c: &dyn Container, slot: SlotId) -> Option<Role> {
    c.slots().into_iter().find(|s| s.id == slot).map(|s| s.role)
}

/// A slot a drag may start on or enter: listed, and no crafting result (a furnace's output is).
fn plain(c: &dyn Container, slot: SlotId) -> bool {
    role(c, slot).is_some_and(|r| r != Role::Result || furnace(c.menu()))
}

/// A slot of the open container's side: its storage, a furnace's slots, a crafting table's grid.
fn container_side(menu: &str, role: Role) -> bool {
    match role {
        Role::Storage => true,
        Role::Input | Role::Result if furnace(menu) => true,
        Role::Input => menu == "minecraft:crafting",
        _ => false,
    }
}

fn held(stack: &Stack) -> (u64, u32, u32) {
    (stack.kind, stack.count, stack.max)
}

/// Places one held item in `slot` when it takes it.
fn place(c: &mut dyn Container, drag: &mut Drag, slot: SlotId) {
    let Some((kind, count, max)) = drag.held else {
        return;
    };
    let Some(hand) = c.carried().filter(|h| h.kind == kind) else {
        return;
    };
    if c.takes_one(slot, &hand) && c.click(slot, Click::Put { one: true }) {
        drag.held = (count > 1).then_some((kind, count - 1, max));
    }
}

/// A press: true when a right-drag took it; else vanilla's press runs and the drag is watched.
pub(crate) fn press(c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool {
    let mut state = state();
    state.drag = None;
    let right = match button {
        Button::Left => false,
        Button::Right => true,
        _ => return false,
    };
    let Some(slot) = slot.filter(|&s| on(c) && plain(c, s)) else {
        return false;
    };
    let hand = c.carried();
    let shift = c.shift();
    if right {
        // A result slot takes nothing: its press stays vanilla's.
        let Some(hand) = hand.filter(|_| !shift && role(c, slot) != Some(Role::Result)) else {
            return false;
        };
        let mut drag = Drag {
            right,
            held: Some(held(&hand)),
            active: true,
            visited: vec![slot],
        };
        place(c, &mut drag, slot);
        state.drag = Some(drag);
        return true;
    }
    if hand.is_some() || shift {
        state.drag = Some(Drag {
            right,
            held: hand.as_ref().map(held),
            active: false,
            visited: vec![slot],
        });
    }
    false
}

/// The cursor moved with the press's button held: true when the controls own the drag.
pub(crate) fn drag(c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool {
    let mut state = state();
    let Some(mut drag) = state.drag.take() else {
        return false;
    };
    if drag.right != (button == Button::Right) {
        return false;
    }
    let was_active = drag.active;
    let entered = slot.filter(|s| !drag.visited.contains(s) && plain(c, *s));
    if let Some(slot) = entered {
        drag.visited.push(slot);
        let stack = c.stack(slot);
        let shift = c.shift();
        if drag.right {
            place(c, &mut drag, slot);
        } else if let Some((kind, count, max)) = drag.held {
            // Holding a stack: same-item slots only, while vanilla's drag hasn't spread.
            if let Some(s) = stack.filter(|s| s.kind == kind)
                && (drag.active || c.spread() <= 1)
            {
                drag.active = true;
                if shift {
                    c.click(slot, Click::QuickMove);
                } else if count + s.count <= max
                    && c.click(slot, Click::Pick { right: false })
                    && c.click(slot, Click::Pick { right: false })
                {
                    // Put the hand's stack in the slot, then take it all.
                    drag.held = Some((kind, count + s.count, max));
                }
            }
        } else if shift && stack.is_some() {
            drag.active = true;
            c.click(slot, Click::QuickMove);
        }
    }
    let owned = drag.active || was_active;
    state.drag = Some(drag);
    owned
}

/// The button went up: the drag ends.
pub(crate) fn release() {
    state().drag = None;
}

/// The wheel over the screen: true when the controls used it.
pub(crate) fn wheel(c: &mut dyn Container, slot: Option<SlotId>, steps: f32) -> bool {
    if !settings::wheel() || !runs_in(c.menu()) || c.carried().is_some() {
        return false;
    }
    let mut state = state();
    let Some(slot) = slot.filter(|&s| role(c, s).is_some()) else {
        state.wheel = 0.0;
        return false;
    };
    state.wheel += steps;
    let whole = state.wheel.trunc();
    state.wheel -= whole;
    drop(state);
    for _ in 0..whole.abs() as u32 {
        if whole < 0.0 {
            push(c, slot);
        } else {
            pull(c, slot);
        }
    }
    true
}

/// The slots across from `slot`: container side and player side face, else hotbar and inventory.
fn other_side(c: &dyn Container, slot: SlotId) -> Vec<SlotId> {
    let slots = c.slots();
    let menu = c.menu();
    let ids = |pick: &dyn Fn(&Slot) -> bool| -> Vec<SlotId> {
        slots.iter().filter(|s| pick(s)).map(|s| s.id).collect()
    };
    let own = ids(&|s| container_side(menu, s.role));
    let hotbar = ids(&|s| s.role == Role::Hotbar);
    let inventory = ids(&|s| s.role == Role::Inventory);
    let player = || {
        hotbar
            .iter()
            .rev()
            .chain(inventory.iter().rev())
            .copied()
            .collect()
    };
    match role(c, slot) {
        Some(r) if container_side(menu, r) => player(),
        Some(Role::Hotbar | Role::Inventory) if !own.is_empty() => own,
        Some(Role::Hotbar) => inventory,
        Some(Role::Inventory) => hotbar,
        _ => Vec::new(),
    }
}

/// Wheel down: one item where a quick move sends it; a result slot gives a whole craft or output.
fn push(c: &mut dyn Container, slot: SlotId) {
    let Some(stack) = c.stack(slot) else {
        return;
    };
    let landing = c.landing(slot);
    let Some(&target) = landing.first() else {
        return;
    };
    if role(c, slot) == Some(Role::Result) {
        if c.click(slot, Click::Pick { right: false }) {
            for target in landing {
                c.click(target, Click::Put { one: false });
            }
        }
        return;
    }
    if c.click(slot, Click::Pick { right: false }) {
        c.click(target, Click::Put { one: true });
        if stack.count > 1 {
            c.click(slot, Click::Put { one: false });
        }
    }
}

/// Wheel up: one item of the same kind from the other side into `slot`, never a result slot.
fn pull(c: &mut dyn Container, slot: SlotId) {
    let Some(stack) = c.stack(slot) else {
        return;
    };
    if !c.takes_one(slot, &stack) {
        return;
    }
    let source = other_side(c, slot).into_iter().find_map(|s| {
        c.stack(s)
            .filter(|v| v.kind == stack.kind && role(c, s) != Some(Role::Result))
            .map(|v| (s, v.count))
    });
    let Some((source, count)) = source else {
        return;
    };
    if c.click(source, Click::Pick { right: false }) {
        c.click(slot, Click::Put { one: true });
        if count > 1 {
            c.click(source, Click::Put { one: false });
        }
    }
}
