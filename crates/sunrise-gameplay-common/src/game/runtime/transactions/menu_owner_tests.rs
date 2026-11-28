use super::*;
use sunrise_items::items::ItemStack;
use sunrise_sim::menu::{Kind, Menu};

#[test]
fn taking_and_returning_the_same_menu_invalidates_the_owner_read() {
    let mut owner = MenuOwner::<Option<Box<Menu>>>::default();
    *owner = Some(Box::new(Menu::new(Kind::Inventory)));
    let id = owner.id();
    let stamp = owner.stamp();
    let menu_stamp = owner.as_ref().unwrap().mutation_stamp();
    let menu = owner.take().unwrap();
    *owner = Some(menu);
    assert_eq!(owner.id(), id);
    assert_ne!(owner.stamp(), stamp);
    assert_eq!(owner.as_ref().unwrap().mutation_stamp(), menu_stamp);
}

#[test]
fn undo_restores_cursor_without_restoring_the_owner_revision() {
    let mut owner = MenuOwner::<Option<Box<Menu>>>::default();
    let mut menu = Box::new(Menu::new(Kind::Inventory));
    menu.carried = Some(ItemStack::new("minecraft:stone", 3));
    *owner = Some(menu);
    let undo = (*owner).clone();
    let stamp = owner.stamp();
    owner.as_mut().unwrap().carried = Some(ItemStack::new("minecraft:dirt", 2));
    let edited = owner.stamp();
    owner.restore(undo);
    assert_ne!(owner.stamp(), stamp);
    assert_ne!(owner.stamp(), edited);
    assert_eq!(
        owner.as_ref().unwrap().carried,
        Some(ItemStack::new("minecraft:stone", 3))
    );
}

#[test]
fn panic_during_an_owner_replacement_does_not_revalidate_an_old_read() {
    let mut owner = MenuOwner::<Option<Box<Menu>>>::default();
    *owner = Some(Box::new(Menu::new(Kind::Inventory)));
    let stamp = owner.stamp();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let menu = owner.take().unwrap();
        *owner = Some(menu);
        panic!("listener after replacement ABA");
    }));
    assert!(result.is_err());
    assert!(owner.is_some());
    assert_ne!(owner.stamp(), stamp);
}

#[test]
fn a_lifted_menu_stays_open_until_the_owner_is_written() {
    let mut owner = MenuOwner::<Option<u32>>::default();
    assert!(!owner.is_open());
    *owner = Some(7);
    assert_eq!(
        owner.lift_if(|&mut v| v == 8),
        None,
        "a refused lift takes nothing"
    );
    assert!(owner.is_open());
    let stamp = owner.stamp();
    assert_eq!(owner.lift_if(|_| true), Some(7));
    assert!(owner.is_none());
    assert!(owner.is_open(), "the click still has it open");
    assert_ne!(owner.stamp(), stamp);
    assert_eq!(owner.lift_if(|_| true), None);
    assert!(
        owner.is_open(),
        "a second lift that takes nothing keeps the first"
    );
    *owner = Some(7);
    assert!(owner.is_open());
    owner.lift_if(|_| true);
    // Closing it while lifted (any write) ends the lift.
    *owner = None;
    assert!(!owner.is_open());
    owner.restore(Some(7));
    owner.lift_if(|_| true);
    owner.restore(None);
    assert!(!owner.is_open(), "an undo ends the lift too");
}

#[test]
fn reads_keep_a_lift_and_the_next_write_ends_it() {
    let mut owner = MenuOwner::<Option<u32>>::default();
    *owner = Some(7);
    let stamp = owner.stamp();
    assert_eq!(owner.lift_if(|_| true), Some(7));
    // Shared reads (what a mod's `slot_holds` does) leave the lift standing.
    assert!(owner.is_none());
    assert!(owner.as_ref().is_none());
    assert!(owner.is_open());
    // Putting it back is an ordinary write: the menu is open through the value, the
    // lift is over, and the stamp moved on.
    *owner = Some(7);
    assert!(owner.is_open());
    assert_ne!(owner.stamp(), stamp);
    let _ = owner.take();
    assert!(!owner.is_open(), "the earlier lift ended at the put-back");
}
