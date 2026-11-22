//! #1278: the clear of a departed guest's target is the mob's own goal's, so
//! a target listener sees it and can cancel or redirect it on the hosted tick.
use super::*;
use crate::game::creatures::departed_guest_tests::{
    BYSTANDER, LEAVER, disconnect, scene, targets, until_full_pass, zombie,
};
use std::sync::{Arc, Mutex};

/// What the listener does to a clear: keep the old target, or name another.
#[derive(Clone, Copy)]
enum OnClear {
    Cancel,
    Redirect(LivingRef),
}

struct Listener(Arc<Mutex<Option<OnClear>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn living_change_target(
        &self,
        _host: &dyn mod_api::Host,
        event: &mut mod_api::LivingChangeTargetEvent<'_>,
    ) -> mod_api::Flow {
        if event.original_target.is_none() {
            match *self.0.lock().unwrap() {
                Some(OnClear::Cancel) => event.cancelled = true,
                Some(OnClear::Redirect(to)) => *event.new_target = Some(to),
                None => {}
            }
        }
        mod_api::Flow::Continue
    }
}

fn with_listener(f: impl FnOnce(&Arc<Mutex<Option<OnClear>>>)) {
    let on_clear = Arc::new(Mutex::new(None));
    let mods = vec![mod_api::ModEntry::new(
        "departed-listener",
        Box::new(Listener(on_clear.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || f(&on_clear));
}

#[test]
#[ignore = "daily"]
fn a_listener_can_cancel_the_clear_of_a_departed_target() {
    with_listener(|on_clear| {
        let mut resources = None;
        let (mut game, id) = scene(&mut resources);
        *on_clear.lock().unwrap() = Some(OnClear::Cancel);
        disconnect(&mut game, LEAVER);
        until_full_pass(&mut game, id);
        assert_eq!(
            targets(&game, id),
            Some(LEAVER),
            "cancelled: the departed target stays, not the bystander"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_listener_can_redirect_the_clear_to_the_bystander() {
    with_listener(|on_clear| {
        let mut resources = None;
        let (mut game, id) = scene(&mut resources);
        let (_, entity) = game.guest_identity(BYSTANDER).unwrap();
        let to = LivingRef::new(game.entity(entity).unwrap());
        *on_clear.lock().unwrap() = Some(OnClear::Redirect(to));
        disconnect(&mut game, LEAVER);
        until_full_pass(&mut game, id);
        assert_eq!(
            zombie(&game, id).ai.target,
            Some(Who::Player),
            "redirected: it still targets a player"
        );
        // The redirect is the listener's choice: the next tick's binding
        // names the bystander it chose.
        crate::game::creatures::departed_guest_tests::step(&mut game);
        assert_eq!(targets(&game, id), Some(BYSTANDER));
    });
}
