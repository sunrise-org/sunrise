use super::*;

impl Game<'_> {
    /// Collect F3 text declarations and renderer toggles with their registering mods, once at
    /// client startup (NeoForge's `RegisterDebugEntriesEvent`).
    pub(in crate::game) fn native_register_debug_entries(
        context: &mod_api::RegistrationContext,
        entries: &mut Vec<Answered<DebugEntry>>,
    ) {
        Self::fire_register_debug_entries(context, entries);
    }
}

#[cfg(test)]
mod tests {
    use super::{DebugEntry, Game, mod_api};
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::__private::{LoadAccess, ModPhase, PhaseOwner, check_access};
    use mod_api::Hooks;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn phase_refusal_discards_only_the_refused_mods_owned_callback() {
        struct DropCount(Arc<AtomicUsize>);
        impl Drop for DropCount {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        struct Registers {
            id: &'static str,
            drops: Arc<AtomicUsize>,
            refuse: bool,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Registers {
            fn register_debug_entries(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::RegisterDebugEntriesEvent<'_>,
            ) {
                assert!(event.entries.is_empty());
                let owned = DropCount(Arc::clone(&self.drops));
                event.entries.push(DebugEntry::text(
                    self.id.into(),
                    mod_api::DebugGroup::Memory,
                    move |_, _| {
                        let _ = std::hint::black_box(&owned);
                    },
                ));
                if self.refuse {
                    check_access("block_state", LoadAccess::WorldOnly);
                }
            }
        }

        let drops_a = Arc::new(AtomicUsize::new(0));
        let drops_b = Arc::new(AtomicUsize::new(0));
        let mods = [
            ("a", "a:lines", &drops_a, true),
            ("b", "b:lines", &drops_b, false),
        ]
        .into_iter()
        .map(|(mod_id, id, drops, refuse)| {
            mod_api::ModEntry::new(
                mod_id,
                Box::new(Registers {
                    id,
                    drops: Arc::clone(drops),
                    refuse,
                }),
            )
        })
        .collect::<Vec<_>>()
        .into();
        let _phase = PhaseOwner::scope(ModPhase::Registration);
        let mut entries = Vec::new();
        mod_platform::testing::with_mods(mods, || {
            Game::native_register_debug_entries(&mod_api::RegistrationContext::new(), &mut entries);
        });
        let [entry] = entries.as_slice() else {
            panic!("expected only the accepted callback");
        };
        assert_eq!((entry.mod_id, entry.answer.id()), ("b", "b:lines"));
        assert!(entry.answer.text_lines().is_some());
        assert_eq!(drops_a.load(Ordering::Relaxed), 1);
        assert_eq!(drops_b.load(Ordering::Relaxed), 0);
        drop(entries);
        assert_eq!(drops_b.load(Ordering::Relaxed), 1);
    }
}
