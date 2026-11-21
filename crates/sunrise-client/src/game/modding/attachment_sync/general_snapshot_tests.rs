//! A general row's first attachment snapshot: sent whenever a client starts tracking the row,
//! even with nothing synced, since the client shows the row only once it arrives
//! (`game/network/remote/general.rs`); and whole, so a client that pairs later reads the value
//! the row holds then.
use super::transport_tests::{VALUE, store};
use crate::game::general::{GeneralEntityRecord, GeneralTypeEnvelope};
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{GeneralEntityCategory, GeneralSavePolicy};
use sunrise_core::nbt::Tag;
use sunrise_mod_runtime::attachments::Holder;

struct Declare;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Declare {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_attachment_type("sync_route:value", true)
            .unwrap();
        r.declare_general_entity(
            "robot",
            GeneralEntityCategory::Misc,
            0.8,
            1.9,
            None,
            GeneralSavePolicy::Save,
            true,
            false,
            None,
            8,
            3,
            1,
            &[],
        )
        .unwrap();
    }
}

/// A server game with `sync_route:robot` declared and `sync_route:value` synced.
fn with_robot(test: impl FnOnce(&mut super::super::Game<'_>)) {
    mod_platform::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("sync_route", Box::new(Declare))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(crate::game::Dim::Overworld);
            game.build_registries();
            game.reset_entity_residency();
            let _world = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            test(game)
        })
    });
}

/// One live `sync_route:robot` row; its id.
fn robot(game: &mut super::super::Game<'_>) -> u32 {
    let record = GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0, 0.0],
        uuid: [1, 2, 3, 4],
        owner: None,
        envelope: GeneralTypeEnvelope {
            type_key: "sync_route:robot".into(),
            payload_version: 1,
            payload_bytes: Vec::new(),
        },
        passengers: Vec::new(),
    };
    game.load_listed("sync_route:robot", None, None, record.to_tag())
        .unwrap();
    game.resident_entities.general.rows[0].id
}

fn value_of(changes: &[sunrise_mod_runtime::attachments::AttachmentSync]) -> Option<i32> {
    changes
        .iter()
        .find_map(|change| match change.value.as_ref()?.get(&"n".into()) {
            Some(Tag::Int(n)) => Some(*n),
            _ => None,
        })
}

#[test]
#[ignore = "daily"]
fn a_general_row_always_has_a_first_snapshot_holding_what_it_holds_now() {
    with_robot(|game| {
        let id = robot(game);
        // First pairing, nothing synced yet: one whole, empty snapshot all the same.
        let first = game.attachment_snapshot(Holder::Entity(id));
        assert_eq!(first.len(), 1);
        assert!(first[0].replace && first[0].changes.is_empty());
        // A client that pairs later is sent the value the row holds then.
        let seal = game.transaction_open();
        VALUE.set(store(game), Holder::Entity(id), 7).unwrap();
        game.transaction_close(&seal, true);
        let late = game.attachment_snapshot(Holder::Entity(id));
        assert_eq!(late.len(), 1);
        assert!(late[0].replace);
        assert_eq!(value_of(&late[0].changes), Some(7));
        // Another entity with nothing synced still gets none.
        assert!(
            game.attachment_snapshot(Holder::Entity(id + 1000))
                .is_empty()
        );
    });
}
