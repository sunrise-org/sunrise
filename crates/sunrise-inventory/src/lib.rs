//! Sunrise's inventory extras on the container hook: Mouse Tweaks-style controls and sorting.
#![forbid(unsafe_code)]

mod plan;
mod settings;
mod sorting;
mod tweaks;

pub use settings::{SECTION, SETTING, Section, Setting};
use sunrise_platform::containers::{Button, Container, ContainerHooks, SlotId};
use sunrise_platform::hooks::Hooks;
use sunrise_platform::hud::Canvas;

/// The extras' hooks, for `sunrise_platform::hooks::register`.
pub static HOOKS: Inventory = Inventory;

pub struct Inventory;

impl Hooks for Inventory {
    fn containers(&self) -> Option<&dyn ContainerHooks> {
        Some(&CONTAINERS)
    }
}

static CONTAINERS: Containers = Containers;

/// Sorting first: its buttons and key beat a drag starting.
struct Containers;

impl ContainerHooks for Containers {
    fn press(&self, c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool {
        sorting::press(c, slot, button) || tweaks::press(c, slot, button)
    }
    fn release(&self, _: &mut dyn Container, _: Option<SlotId>, _: Button) -> bool {
        tweaks::release();
        false
    }
    fn drag(&self, c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool {
        tweaks::drag(c, slot, button)
    }
    fn wheel(&self, c: &mut dyn Container, slot: Option<SlotId>, steps: f32) -> bool {
        tweaks::wheel(c, slot, steps)
    }
    fn key(&self, c: &mut dyn Container, slot: Option<SlotId>, key: &str) -> bool {
        sorting::key(c, slot, key)
    }
    fn draw(&self, c: &dyn Container, canvas: &mut dyn Canvas) {
        sorting::draw(c, canvas);
    }
}
