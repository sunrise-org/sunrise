#![cfg(test)]
// A native Host call the load phase refuses is contained and latched, not a crash (#3123).
use crate::*;
use mod_vocab::__private::{
    LoadAccess, ModPhase, PhaseOwner, PhaseRefusal, check_access, contain_refusal,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

fn refusal(mod_id: &'static str, operation: &'static str, phase: ModPhase) -> PhaseRefusal {
    PhaseRefusal {
        mod_id,
        operation,
        phase,
    }
}

#[test]
fn a_refused_call_comes_back_naming_the_mod_the_operation_and_the_phase() {
    crate::__private::initialize().unwrap();
    let _owner = PhaseOwner::new(ModPhase::Setup);
    assert_eq!(
        contain_refusal("m", || check_access("level", LoadAccess::WorldOnly)),
        Err(refusal("m", "level", ModPhase::Setup))
    );
    // What the phase admits is not refused.
    assert_eq!(
        contain_refusal("m", || {
            check_access("setting", LoadAccess::WorldOnly);
            check_access("block_state_default", LoadAccess::Metadata);
            7
        }),
        Ok(7)
    );
}

#[test]
fn the_latch_refuses_every_later_call_whatever_its_class_and_survives_a_mods_own_catch() {
    crate::__private::initialize().unwrap();
    let _owner = PhaseOwner::new(ModPhase::Setup);
    let result = contain_refusal("m", || {
        let swallowed = catch_unwind(|| check_access("first", LoadAccess::WorldOnly));
        assert!(swallowed.is_err());
        // Allowed in Setup by itself, refused now, and the first refusal stays the one reported.
        let again = catch_unwind(|| check_access("setting", LoadAccess::WorldOnly));
        assert!(again.unwrap_err().is::<PhaseRefusal>());
        let again = catch_unwind(|| check_access("block_state_default", LoadAccess::Metadata));
        assert!(again.unwrap_err().is::<PhaseRefusal>());
        "returned normally"
    });
    assert_eq!(result, Err(refusal("m", "first", ModPhase::Setup)));
    // The scope ended with the call: a later scope starts clean.
    assert_eq!(
        contain_refusal("m", || check_access("setting", LoadAccess::WorldOnly)),
        Ok(())
    );
}

#[test]
fn nested_scopes_save_and_restore_and_other_panics_go_on_unwinding() {
    crate::__private::initialize().unwrap();
    let _owner = PhaseOwner::new(ModPhase::Setup);
    let outer = contain_refusal("outer", || {
        let inner = contain_refusal("inner", || check_access("a", LoadAccess::WorldOnly));
        assert_eq!(inner, Err(refusal("inner", "a", ModPhase::Setup)));
        // The inner refusal did not latch the outer scope.
        check_access("setting", LoadAccess::WorldOnly);
    });
    assert_eq!(outer, Ok(()));
    let panic = catch_unwind(|| contain_refusal("m", || panic!("a real fault"))).unwrap_err();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"a real fault"));
}

#[test]
fn without_a_scope_the_gate_panics_with_the_named_message() {
    crate::__private::initialize().unwrap();
    let _owner = PhaseOwner::new(ModPhase::Reloading);
    let panic = catch_unwind(|| check_access("level", LoadAccess::WorldOnly)).unwrap_err();
    assert_eq!(
        panic.downcast_ref::<String>().unwrap(),
        "mod API level is unavailable during Reloading"
    );
}

#[test]
fn the_phase_is_per_thread_one_thread_reloading_another_in_world_is_allowed() {
    crate::__private::initialize().unwrap();
    let owner = PhaseOwner::new(ModPhase::Setup);
    let _reloading = owner.reloading().unwrap();
    assert!(catch_unwind(|| check_access("level", LoadAccess::WorldOnly)).is_err());
    // The integrated server's thread keeps its own phase while the window reloads.
    std::thread::spawn(|| check_access("level", LoadAccess::WorldOnly))
        .join()
        .unwrap();
}

struct Probe {
    seen: Arc<Mutex<Vec<&'static str>>>,
    name: &'static str,
    write: Option<usize>,
    refuse: bool,
    cancel: bool,
}
#[crate::__subscriptions]
impl Hooks for Probe {
    fn potion_brew_pre(&self, _: &mut dyn ServerHost, event: &mut PotionBrewPreEvent<'_>) -> Flow {
        self.seen.lock().unwrap().push(self.name);
        if let Some(slot) = self.write {
            event.stacks[slot] = Some(crate::__private::part_tests::stack(
                "minecraft:diamond",
                1,
                0,
            ));
        }
        if self.refuse {
            // A mod's own catch does not undo the refusal.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                check_access("entity", LoadAccess::WorldOnly)
            }));
            event.cancelled = true;
        }
        if self.cancel {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

struct NoHost;
impl Host for NoHost {}
crate::impl_host_areas!(NoHost);
impl ServerHost for NoHost {}
crate::impl_server_host_areas!(NoHost);

#[test]
fn a_persistence_write_without_a_backend_returns_unavailable() {
    let entity = EntityRef::new(WorldEpoch::new(1), 1);
    assert_eq!(
        entity.set_persistent(&mut NoHost, false),
        Err(LevelWriteError::Unavailable)
    );
}

fn brew(phase: ModPhase, listeners: Vec<Probe>) -> ([Option<ItemStack>; 5], bool) {
    crate::__private::initialize().unwrap();
    crate::__private::install_parts(&[], &crate::__private::part_tests::rule_schema(), |_| false)
        .unwrap();
    let mods = listeners
        .into_iter()
        .enumerate()
        .map(|(index, probe)| ModEntry::new(["first", "refused", "last"][index], Box::new(probe)))
        .collect::<Vec<_>>()
        .into();
    let _owner = PhaseOwner::new(phase);
    testing::with_mods(mods, || {
        let mut stacks = [None, None, None, None, None];
        let mut cancelled = false;
        dispatch_potion_brew_pre(&mut NoHost, [1, 2, 3], &mut stacks, &mut cancelled);
        (stacks, cancelled)
    })
}

#[test]
fn a_refused_listener_is_dropped_its_outputs_restored_and_the_hook_goes_on() {
    for phase in [ModPhase::Setup, ModPhase::Reloading] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let probe = |name, write, refuse, cancel| Probe {
            seen: seen.clone(),
            name,
            write,
            refuse,
            cancel,
        };
        let (stacks, cancelled) = brew(
            phase,
            vec![
                probe("first", Some(0), false, false),
                probe("refused", Some(1), true, true),
                probe("last", Some(2), false, false),
            ],
        );
        assert_eq!(*seen.lock().unwrap(), ["first", "refused", "last"]);
        // The earlier listener's answer stands, the refused one's write and cancel are undone, the
        // later listener still ran.
        assert!(stacks[0].is_some() && stacks[1].is_none() && stacks[2].is_some());
        assert!(!cancelled);
    }
}

#[test]
fn in_world_a_listener_runs_the_plain_call() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (stacks, cancelled) = brew(
        ModPhase::World,
        vec![Probe {
            seen,
            name: "only",
            write: Some(0),
            refuse: false,
            cancel: true,
        }],
    );
    assert!(stacks[0].is_some() && cancelled);
}
