//! Stable storage identity for a menu's owning slot, including while lifted.
//!
//! A menu value's stamp cannot detect taking that same value out and putting it
//! back. This owner advances on every mutable access, even an equal replacement
//! or an unwind. Action admission uses its stable id before any take occurs.
//!
//! A click lifts its menu out of the owner ([`MenuOwner::lift_if`]) to lend it to the
//! game while mods decide the click. The menu is still the one the player has open,
//! so [`MenuOwner::is_open`] keeps answering yes until the owner is next written.
//! The lift records the owner revision it left behind, and every write already
//! advances that revision, so a write ends the lift without any extra bookkeeping on
//! ordinary mutable access (ticks, unmodded clicks).
use super::{Generation, MenuId};
use std::ops::{Deref, DerefMut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerStamp {
    pub id: MenuId,
    revision: Generation,
}

pub struct MenuOwner<T> {
    stamp: OwnerStamp,
    value: T,
    /// The owner revision a `lift_if` left behind when it took the value. The lift is
    /// live while the revision is still this one; any later write advances it.
    lifted_at: Option<Generation>,
}

impl<T: Default> Default for MenuOwner<T> {
    fn default() -> Self {
        Self {
            stamp: OwnerStamp {
                id: MenuId::fresh(),
                revision: Generation::default(),
            },
            value: T::default(),
            lifted_at: None,
        }
    }
}

impl<T> MenuOwner<T> {
    pub fn id(&self) -> MenuId {
        self.stamp.id
    }

    pub fn stamp(&self) -> OwnerStamp {
        self.stamp
    }

    /// Undo content only. The owner and its revision must never roll back.
    pub fn restore(&mut self, value: T) {
        self.stamp.revision.advance();
        self.value = value;
    }
}

impl<U> MenuOwner<Option<U>> {
    /// `Option::take_if` for a click that lends the menu out while hooks run: the owner
    /// is empty, but [`is_open`](Self::is_open) still counts the lifted menu until the
    /// next write (putting it back, replacing it or closing it) or [`restore`](Self::restore).
    /// A lift that finds nothing to take leaves an earlier, unfinished lift standing (a
    /// second click on a menu that is already lent out).
    pub fn lift_if(&mut self, keep: impl FnOnce(&mut U) -> bool) -> Option<U> {
        // Whether an earlier lift is still live, read before this access advances it.
        let earlier = self.lift_is_live();
        // A mutable access like any other, so the revision advances (the caller reads its
        // stamp after the lift).
        self.stamp.revision.advance();
        let lifted = self.value.take_if(keep);
        if lifted.is_some() || earlier {
            self.lifted_at = Some(self.stamp.revision);
        }
        lifted
    }

    fn lift_is_live(&self) -> bool {
        self.lifted_at == Some(self.stamp.revision)
    }

    /// The player has this menu open: it is in the owner, or lifted out by an
    /// unfinished click. A read during that click's hooks sees the menu still open.
    pub fn is_open(&self) -> bool {
        self.value.is_some() || self.lift_is_live()
    }
}

impl<T> Deref for MenuOwner<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for MenuOwner<T> {
    fn deref_mut(&mut self) -> &mut T {
        // Advancing the revision also ends any lift: the click put its menu back, or
        // something replaced or closed it, and the value now says which.
        self.stamp.revision.advance();
        &mut self.value
    }
}

#[cfg(test)]
#[path = "menu_owner_tests.rs"]
mod tests;
