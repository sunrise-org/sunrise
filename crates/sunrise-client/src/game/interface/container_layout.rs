//! Container geometry constructed by the screen build seam.
use super::screen_build::ContainerLayout;
use super::{Creative, Game, Screen, beacon, stations};
use sunrise_client::{hud::HudBatch, inventory_effects::EffectsLayout};

#[cfg(test)]
thread_local! {
    pub(in crate::game) static LAYOUT_CONSTRUCTIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Selectors read by slot_position; item contents and scroll do not move slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct SlotInputs {
    count: usize,
    craft_size: usize,
    creative_inventory: bool,
    creative_tab: bool,
    station: Option<(std::mem::Discriminant<super::stations::Kind>, usize)>,
    container: Option<std::mem::Discriminant<sunrise_client::containers::BlockEntity>>,
    machine: Option<bool>,
    crafter: bool,
    mount: Option<(usize, bool, bool)>,
    rows: Option<usize>,
    native: Option<(u64, u32, Option<f32>, Option<f32>)>,
}

impl Game<'_> {
    /// Inventory origin and image rectangle from a panel size and explicit layout shifts.
    /// Opening's staged panel passes zero shifts; only a live screen asks for room.
    pub(super) fn inventory_panel(
        batch: &HudBatch,
        width: Option<f32>,
        height: f32,
        station: Option<stations::Kind>,
        [book, room]: [f32; 2],
    ) -> ((f32, f32), [f32; 4]) {
        let width = width.unwrap_or(176.0);
        let (x, y) = batch.centered_origin(width, height);
        let x = x + book - room;
        let (offset, width) = match station {
            Some(stations::Kind::Merchant) => (-50.0, 276.0),
            Some(stations::Kind::Beacon) => (beacon::SHIFT, 230.0),
            _ => (0.0, width),
        };
        ((x, y), [x + offset, y, width, height])
    }

    /// Geometry of the live container, including its book and mod room shifts.
    pub(super) fn container_panel(&self, batch: &HudBatch) -> ((f32, f32), [f32; 4]) {
        if self.screen == Screen::Creative {
            let (x, y) = Self::creative_origin(batch);
            return ((x, y), [x, y, Creative::WIDTH, Creative::HEIGHT]);
        }
        Self::inventory_panel(
            batch,
            self.native_screen_width(),
            self.screen_height(),
            self.station.as_ref().map(|s| s.kind),
            [
                self.recipe_book_shift(batch.width),
                self.screen_room_shift(batch),
            ],
        )
    }

    fn native_slot_inputs(&self) -> Option<(u64, u32, Option<f32>, Option<f32>)> {
        self.shown_native_screen().map(|s| {
            (
                s.layout_revision,
                s.slot_move_count(),
                self.native_screen_inventory_x(),
                self.native_screen_inventory_y(),
            )
        })
    }

    pub(super) fn container_layout(
        &self,
        batch: &sunrise_client::hud::HudBatch,
    ) -> ContainerLayout {
        self.build_container_layout(batch, self.container_layout_inputs(batch))
    }

    pub(super) fn build_container_layout(
        &self,
        batch: &sunrise_client::hud::HudBatch,
        mut inputs: ContainerLayout,
    ) -> ContainerLayout {
        #[cfg(test)]
        LAYOUT_CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
        // Slot moves made since the last build apply from this one, to the
        // boxes below and to everything drawn until the next.
        if let Some(screen) = self.shown_native_screen() {
            screen.commit_slot_moves();
        }
        let origin = inputs.origin;
        let slots = self
            .slots()
            .into_iter()
            .map(|slot| {
                let (x, y) = self.slot_position(slot);
                (slot, [origin.0 + x - 1.0, origin.1 + y - 1.0, 18.0, 18.0])
            })
            .collect();
        inputs.effects = inputs
            .shows_effects
            .then(|| {
                self.inventory_effects_rect(
                    batch,
                    [inputs.origin.0, inputs.origin.1],
                    inputs.image[2],
                    &inputs.effects_layout,
                )
            })
            .flatten();
        inputs.slots = slots;
        inputs
    }

    /// The open container screen's layout from the game's state: the panel it
    /// draws, the book and effect panels beside it, the creative tab rows and
    /// every slot's hover box. The build calls it when it makes the layout;
    /// nothing else does.
    pub(super) fn container_layout_inputs(
        &self,
        batch: &sunrise_client::hud::HudBatch,
    ) -> ContainerLayout {
        let (origin, image) = self.container_panel(batch);
        // A title over the inventory draws no book or effects of its own.
        let covered = self.screen == Screen::Inventory && self.title.is_some();
        let book = (!covered).then(|| self.recipe_book_rect(batch)).flatten();
        let shows_effects = !covered
            && match self.screen {
                Screen::Creative => true,
                Screen::Inventory => {
                    self.container.is_none() && self.craft_size == 2 && !self.mount_active()
                }
                _ => false,
            };
        let space = Self::effects_space(batch, [origin.0, origin.1], image[2]);
        let effects_layout = if shows_effects {
            self.inventory_effects_layout(&space, image)
        } else {
            EffectsLayout::vanilla(space.available())
        };
        let tabs = (self.screen == Screen::Creative).then(|| {
            [
                [
                    image[0],
                    image[1] - Creative::TAB_ROW_DEPTH,
                    image[2],
                    Creative::TAB_ROW_DEPTH,
                ],
                [
                    image[0],
                    image[1] + image[3],
                    image[2],
                    Creative::TAB_ROW_DEPTH,
                ],
            ]
        });
        ContainerLayout {
            slot_inputs: SlotInputs {
                count: self.slot_count(),
                craft_size: self.craft_size,
                creative_inventory: self.creative_inventory_open(),
                creative_tab: self.creative_tab_open(),
                station: self
                    .station
                    .as_ref()
                    .map(|s| (std::mem::discriminant(&s.kind), s.inputs.len())),
                container: self.container_kind().map(std::mem::discriminant),
                machine: self.machine_layout_selector(),
                crafter: self.crafter_open(),
                mount: self.mount_layout_inputs(),
                rows: self.chest_rows(),
                native: self.native_slot_inputs(),
            },
            image,
            origin,
            book,
            effects: None,
            shows_effects,
            effects_revision: {
                use std::hash::{Hash, Hasher};
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                for effect in &self.potions.effects.list {
                    effect.id.hash(&mut hash);
                    effect.duration.hash(&mut hash);
                    effect.amplifier.hash(&mut hash);
                }
                hash.finish()
            },
            effects_layout,
            tabs,
            slots: Vec::new(),
        }
    }
}
