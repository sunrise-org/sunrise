//! `AbstractHorse.aiStep`'s grazing: a horse loaded with `EatingHaystack:1b`
//! clears it after 51 ticks of the counter (`eatingCounter` passes 50), and a
//! llama — `canEatGrass` is false, so vanilla never runs the counter — keeps
//! whatever it was loaded with.
use super::*;
use crate::mobs::world::with_test_world;
use std::sync::Arc;

/// A horse or llama as a save with `EatingHaystack:1b` leaves it, standing on
/// plain ground so only the loaded state can have started the graze.
fn loaded_eating(kind_id: &str, types: &super::MobTypes) -> Mob {
    let mut m = Mob::new(
        1,
        Arc::new(types.types[kind_id].clone()),
        Vec3::new(0.5, 64.0, 0.5),
        0.0,
    );
    equine::register(&mut m);
    m.persistent = true;
    m.on_ground = true;
    m.extra.flag = true;
    m
}

#[test]
#[ignore = "daily"]
fn a_grazing_horse_clears_eating_haystack_after_51_ticks_while_a_llama_keeps_it() {
    let mut outcomes = Vec::new();
    if !with_test_world(&[], |w, types| {
        for kind in ["minecraft:horse", "minecraft:llama"] {
            let mut m = loaded_eating(kind, types);
            let mut ate = Vec::new();
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            for _ in 0..51 {
                crate::mobs::ai::tick(&mut m, w, &mut test_level.level(), &mut events);
                ate.push(m.extra.flag);
            }
            outcomes.push((kind, ate));
        }
    }) {
        return;
    }
    assert_eq!(
        outcomes[0],
        (
            "minecraft:horse",
            [true; 50].into_iter().chain([false]).collect::<Vec<_>>()
        ),
        "the horse grazes 50 ticks, and the 51st clears EatingHaystack"
    );
    assert!(
        outcomes[1].1.iter().all(|ate| *ate),
        "the llama keeps the loaded EatingHaystack"
    );
}
