use super::*;
use crate::game::boats::Boat;
use crate::game::native_entity_refs::tests::{with_client_game, with_game};
use mod_api::{
    BlockPart, BounceRestitutionEvent, Flow, Hooks, LevelSide, Registrar, RegistrationContext,
};
use std::sync::{Arc, Mutex};
use sunrise_client::chunk::Chunk;

#[derive(Clone, Default)]
struct PushBounce(Arc<Mutex<Vec<(bool, i32)>>>);

#[mod_api::sunrise_part]
impl BlockPart for PushBounce {
    fn bounce_restitution(&self, side: LevelSide<'_>, event: &mut BounceRestitutionEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((matches!(side, LevelSide::Client(_)), event.entity));
        event.factor = 1.0;
    }
}

#[mod_api::__subscriptions]
impl Hooks for PushBounce {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let glass = r.block_key("minecraft:glass");
        r.block_part(glass, self.clone());
    }
}

struct RemoveLaterBoat;

#[mod_api::__subscriptions]
impl Hooks for RemoveLaterBoat {
    fn vanilla_game_event(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        if event.event == "minecraft:bounce" && event.cause == Some(71) {
            assert!(host.entity(72).unwrap().discard(host));
        }
        Flow::Continue
    }
}

fn boats_over_glass(game: &mut Game<'_>, boats: &[(u32, i32)]) {
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let glass = game
        .pack
        .assets
        .registry
        .default_state("minecraft:glass")
        .unwrap();
    for &(id, x) in boats {
        game.world.set(x, 63, 4, glass).unwrap();
        let mut boat = Boat::new(
            id,
            "oak".into(),
            Vec3::new(f64::from(x) + 0.5, 64.25, 4.5),
            0.0,
        );
        boat.velocity.y = -1.0;
        game.boats.push(boat);
    }
}

#[test]
#[ignore = "daily"]
fn a_bounce_listener_can_remove_a_later_lid_target() {
    crate::game::in_own_process(
        "game::piston_push::restitution_tests::a_bounce_listener_can_remove_a_later_lid_target",
        || {
            let part = PushBounce::default();
            let mods: mod_api::Mods = vec![
                mod_api::ModEntry::new("bounce", Box::new(part.clone())),
                mod_api::ModEntry::new("discard", Box::new(RemoveLaterBoat)),
            ]
            .into();
            mod_platform::testing::with_mods(mods.clone(), || {
                crate::game::api_values::install_parts(&mods).unwrap();
                crate::game::native_hooks::install_block_contact_hooks();
                with_game(|game| {
                    boats_over_glass(game, &[(71, 4), (72, 7), (73, 10)]);
                    let area = Aabb::new(Vec3::new(3.0, 64.5, 3.0), Vec3::new(12.0, 65.0, 6.0));
                    sunrise_client::block_contact::in_level(game.tick, Some(game.dim.id()), || {
                        game.lid_push(Lid::Box, area, Vec3::new(0.0, -0.5, 0.0));
                    });
                    // Discard retires the handle synchronously; the tick sweep owns
                    // physical removal. The lid must not push that departed row.
                    assert!(game.entity(72).is_none());
                    for boat in game.boats.iter() {
                        if boat.id == 72 {
                            assert_eq!(boat.position.y, 64.25);
                            assert_eq!(boat.velocity.y, -1.0);
                        } else {
                            assert_eq!(boat.position.y, 64.0);
                            assert!(boat.velocity.y > 0.0);
                        }
                    }
                    assert_eq!(*part.0.lock().unwrap(), [(false, 71), (false, 73)]);
                });
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_downward_shulker_box_lid_bounces_only_the_locally_driven_boat() {
    crate::game::in_own_process(
        "game::piston_push::restitution_tests::a_downward_shulker_box_lid_bounces_only_the_locally_driven_boat",
        || {
            let part = PushBounce::default();
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("bounce", Box::new(part.clone()))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                crate::game::api_values::install_parts(&mods).unwrap();
                crate::game::native_hooks::install_block_contact_hooks();
                with_client_game(|game| {
                    game.init_remote_for_replay().unwrap();
                    game.remote.as_mut().unwrap().first_passenger = true;
                    game.riding = Some(71);
                    boats_over_glass(game, &[(71, 4), (72, 7)]);
                    let facing = Direction::ALL
                        .iter()
                        .position(|d| *d == Direction::Down)
                        .unwrap() as u8;
                    sunrise_client::block_contact::in_level(game.tick, Some(game.dim.id()), || {
                        game.push_shulker_local(vec![(
                            [4, 65, 4],
                            facing,
                            [3.0, 64.5, 3.0, 9.0, 65.0, 6.0],
                        )]);
                    });
                    assert_eq!(game.boats[0].position.y, 64.0);
                    assert!(game.boats[0].velocity.y > 0.0);
                    assert_eq!(game.boats[1].position.y, 64.25);
                    assert_eq!(game.boats[1].velocity.y, -1.0);
                    assert_eq!(*part.0.lock().unwrap(), [(true, 71)]);
                    assert!(!game.persist.sync_body_position.contains(&71));
                });
            });
        },
    );
}
