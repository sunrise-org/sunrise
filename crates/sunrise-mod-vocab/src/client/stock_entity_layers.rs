//! Draw-local controls for stock living-entity layers.

use crate::Hand;
use std::cell::Cell;

/// The slot of a humanoid armor layer, not the item's equipment slot.
// exhaustive: these are the four slots drawn by the humanoid armor layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmorLayerSlot {
    /// The helmet layer.
    Head,
    /// The chestplate layer.
    Chest,
    /// The leggings layer.
    Legs,
    /// The boots layer.
    Feet,
}

/// The independent passes of a worn humanoid armor piece.
// exhaustive: the base material, foil and trim are the stock armor passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmorLayerPass {
    /// The armor material, including its base texture layers.
    Base,
    /// The enchantment foil, whether carried by the base or by the trim.
    Glint,
    /// The armor-trim material.
    Trim,
}

/// A stock layer of the current living-entity renderer. Not a model part or a mod-added layer.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockEntityLayer {
    /// The avatar's cape.
    Cape,
    /// The wings rendered as a layer over the body.
    Wings,
    /// A non-armor item drawn on the head, such as a skull.
    HeadItem,
    /// The stock item layer for the given hand.
    HeldItem(Hand),
    /// One material or effect pass of a worn humanoid armor piece.
    Armor(ArmorLayerSlot, ArmorLayerPass),
}

impl StockEntityLayer {
    const fn bit(self) -> u32 {
        1 << match self {
            Self::Cape => 0,
            Self::Wings => 1,
            Self::HeadItem => 2,
            Self::HeldItem(Hand::Main) => 3,
            Self::HeldItem(Hand::Off) => 4,
            Self::Armor(slot, pass) => 5 + 3 * slot as u32 + pass as u32,
        }
    }
}

/// Why a stock layer hide made no edit.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HideLayerError {
    /// The stock renderer has no such layer or pass. An empty equipment slot is not an error.
    Unsupported(StockEntityLayer),
}

// hidden: the renderer's fixed-size mask, used for both supported and hidden layers.
#[doc(hidden)]
#[derive(Clone, Copy, Default)]
pub struct StockLayerMask(u32);

impl StockLayerMask {
    pub const PLAYER: Self = Self((1 << 17) - 1);

    pub fn insert(&mut self, layer: StockEntityLayer) {
        self.0 |= layer.bit();
    }

    pub const fn contains(self, layer: StockEntityLayer) -> bool {
        self.0 & layer.bit() != 0
    }
}

/// Hide stock layers for one world draw in `render_player_pre` or `render_living_pre`.
/// Successful edits accumulate across listeners and both hooks. They cannot show a layer or
/// affect the inventory preview, another draw, equipment, or the entity's base model.
pub struct HiddenLayers<'draw> {
    supported: StockLayerMask,
    hidden: &'draw Cell<StockLayerMask>,
}

impl<'draw> HiddenLayers<'draw> {
    /// Hide `layer` if the stock renderer supports it, even if its usual draw conditions aren't
    /// met right now. Repeating a hide is harmless. A refusal does not change any prior edit.
    pub fn hide(&self, layer: StockEntityLayer) -> Result<(), HideLayerError> {
        if !self.supported.contains(layer) {
            return Err(HideLayerError::Unsupported(layer));
        }
        self.hidden
            .set(StockLayerMask(self.hidden.get().0 | layer.bit()));
        Ok(())
    }

    // hidden: the engine issues a fresh editor for each world draw.
    #[doc(hidden)]
    pub fn __new(supported: StockLayerMask, hidden: &'draw Cell<StockLayerMask>) -> Self {
        Self { supported, hidden }
    }

    // hidden: only the stock draw consumes the resulting mask.
    #[doc(hidden)]
    pub fn __mask(&self) -> StockLayerMask {
        self.hidden.get()
    }

    pub(crate) fn guard<T, E>(&self, call: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        struct Restore<'a>(&'a Cell<StockLayerMask>, StockLayerMask);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let mut restore = Restore(self.hidden, self.hidden.get());
        let answer = call();
        if answer.is_ok() {
            restore.1 = self.hidden.get();
        }
        answer
    }
}
