#![cfg(test)]
//! `RedStoneOreBlock.useItemOn` returns `PASS` only when the held item is a
//! `BlockItem` and `new BlockPlaceContext(...).canPlace()` is true, and
//! `SUCCESS` otherwise — on both the server (`use_hand`) and client prediction
//! (`remote_block_reacts` / `client_block_answer`).
//!
//! The server cases here are also compared against vanilla 26.3 itself by
//! `tools/test_use_item_on_parity.py` (`redstone_ore_named_cases` and a random
//! stream), which prints the hand that ended the click and whether it failed.
//! This test keeps what that harness cannot reach: client prediction, and that
//! the item's `placement_context` hook is never asked.

use crate::game::native_entity_refs::tests::with_game;
use crate::game::{GuestUse, Hit};
use mod_api::{
    Hand, Hooks, ItemPart, LevelSide, PlacementContext, PlacementContextEvent, Registrar,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_client::{chunk::Chunk, items::ItemStack, motion::PlayerMotion};
use sunrise_core::{geometry::Vec3, position::ChunkPos};
use sunrise_world::level::InteractionResult;

#[derive(Clone, Default)]
struct PlacementProbe(Arc<AtomicUsize>);

#[mod_api::sunrise_part]
impl ItemPart for PlacementProbe {
    fn placement_context(
        &self,
        _: LevelSide<'_>,
        _: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<PlacementContext>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        None
    }
}

#[mod_api::__subscriptions]
impl Hooks for PlacementProbe {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for item in [
            "minecraft:cobblestone",
            "minecraft:sweet_berries",
            "minecraft:short_grass",
            "minecraft:oak_slab",
        ] {
            let key = r.item_key(item);
            r.item_part(key, self.clone());
        }
    }
}

/// A click on the centre of `face` of the block at `pos`.
fn hit_on(pos: [i32; 3], face: [i32; 3]) -> Hit {
    Hit {
        pos,
        face,
        point: std::array::from_fn(|i| f64::from(pos[i]) + 0.5 + f64::from(face[i]) * 0.5),
        inside: false,
    }
}

fn top_hit(pos: [i32; 3]) -> Hit {
    hit_on(pos, [0, 1, 0])
}

const EAST: [i32; 3] = [1, 0, 0];

#[test]
#[ignore = "daily"]
fn redstone_ore_use_checks_block_place_context_can_place() {
    crate::game::in_own_process(
        "game::sculk::redstone_ore_use_tests::redstone_ore_use_checks_block_place_context_can_place",
        || {
            let probe = PlacementProbe::default();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "redstone_ore_probe",
                Box::new(probe.clone()),
            )]
            .into();
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(0, 0))
                        .unwrap();
                    let registry = game.pack.assets.registry;
                    let default = |name| registry.default_state(name).unwrap();
                    let ore = default("minecraft:redstone_ore");
                    let deepslate_ore = default("minecraft:deepslate_redstone_ore");
                    let stone = default("minecraft:stone");
                    let cobble = default("minecraft:cobblestone");
                    let grass = default("minecraft:short_grass");
                    let dirt = default("minecraft:dirt");
                    let air = default("minecraft:air");
                    let bottom_slab = game
                        .with_props(default("minecraft:oak_slab"), &[("type", "bottom")])
                        .unwrap();
                    let lit_ore = game.with_props(ore, &[("lit", "true")]).unwrap();
                    let lit_deepslate = game.with_props(deepslate_ore, &[("lit", "true")]).unwrap();

                    let mut player =
                        PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap();
                    // Hungry so a consumable block item (`sweet_berries`) would start eating
                    // if `use_redstone_ore` mistakenly passed and let placement fail.
                    player.health.food.food = 6;
                    game.player = Some(player);
                    game.set_mode_id(0);

                    let click_server =
                        |game: &mut crate::game::Game<'_>, hit: Hit, held: Option<ItemStack>| {
                            *game.held_mut() = held;
                            game.guest_use = Some(GuestUse::on_block(Hand::Main));
                            let consumed = game.use_hand(Some(&hit)).unwrap();
                            let fell_through = matches!(
                                game.guest_use,
                                Some(GuestUse::OnBlock {
                                    block_fell_through: true,
                                    ..
                                })
                            );
                            game.guest_use = None;
                            (consumed, fell_through)
                        };

                    // 1. Block item aimed at a valid destination (air above): PASS + placed + lit.
                    game.world.set(2, 64, 4, ore).unwrap();
                    game.world.set(2, 65, 4, air).unwrap();
                    let before_attempts = probe.0.load(Ordering::SeqCst);
                    let (consumed, fell_through) = click_server(
                        game,
                        top_hit([2, 64, 4]),
                        Some(ItemStack::new("minecraft:cobblestone", 2)),
                    );
                    assert!(consumed);
                    assert!(fell_through, "valid destination passes to BlockItem.place");
                    assert!(!game.use_failed);
                    assert_eq!(probe.0.load(Ordering::SeqCst), before_attempts + 1);
                    assert_eq!(game.world.block(2, 64, 4), lit_ore);
                    assert_eq!(game.world.block(2, 65, 4), cobble);
                    assert_eq!(game.held().unwrap().count, 1);

                    // 2. Block item aimed at an invalid destination (solid stone above):
                    //    SUCCESS + lit + no placement attempt.
                    game.world.set(4, 64, 4, ore).unwrap();
                    game.world.set(4, 65, 4, stone).unwrap();
                    let before_attempts = probe.0.load(Ordering::SeqCst);
                    let (consumed, fell_through) = click_server(
                        game,
                        top_hit([4, 64, 4]),
                        Some(ItemStack::new("minecraft:cobblestone", 2)),
                    );
                    assert!(consumed);
                    assert!(!fell_through, "blocked destination is consumed as SUCCESS");
                    assert!(!game.use_failed, "SUCCESS must not set use_failed");
                    assert_eq!(
                        probe.0.load(Ordering::SeqCst),
                        before_attempts,
                        "blocked destination must not attempt BlockItem.place"
                    );
                    assert_eq!(game.world.block(4, 64, 4), lit_ore);
                    assert_eq!(game.world.block(4, 65, 4), stone);
                    assert_eq!(game.held().unwrap().count, 2);

                    // 3. Consumable block item aimed at a blocked destination:
                    //    SUCCESS + no placement attempt + does not start eating.
                    game.world.set(5, 64, 4, deepslate_ore).unwrap();
                    game.world.set(5, 65, 4, stone).unwrap();
                    let before_attempts = probe.0.load(Ordering::SeqCst);
                    let (consumed, fell_through) = click_server(
                        game,
                        top_hit([5, 64, 4]),
                        Some(ItemStack::new("minecraft:sweet_berries", 2)),
                    );
                    assert!(consumed);
                    assert!(!fell_through);
                    assert!(!game.use_failed);
                    assert!(
                        game.using.is_none(),
                        "SUCCESS must not fall through to eating"
                    );
                    assert_eq!(probe.0.load(Ordering::SeqCst), before_attempts);
                    assert_eq!(game.world.block(5, 64, 4), lit_deepslate);
                    assert_eq!(game.held().unwrap().count, 2);

                    // 4. Replaceable-by-same-item rules (`BlockState.canBeReplaced`):
                    //    - `short_grass` into `short_grass` cannot replace -> SUCCESS, no attempt.
                    //    - `oak_slab` into bottom `oak_slab` can replace -> PASS + double slab.
                    //    The grass stands beside the ore on dirt and the click is on the ore's
                    //    east face: `RedStoneOreBlock.interact` lights the ore with
                    //    `setBlockAndUpdate` before the gate, and grass standing on the ore
                    //    itself would break in that shape update (case 4c).
                    game.world.set(6, 64, 8, ore).unwrap();
                    game.world.set(7, 63, 8, dirt).unwrap();
                    game.world.set(7, 64, 8, grass).unwrap();
                    let before_attempts = probe.0.load(Ordering::SeqCst);
                    let (consumed, fell_through) = click_server(
                        game,
                        hit_on([6, 64, 8], EAST),
                        Some(ItemStack::new("minecraft:short_grass", 2)),
                    );
                    assert!(consumed);
                    assert!(!fell_through, "short_grass cannot replace short_grass");
                    assert!(!game.use_failed);
                    assert_eq!(probe.0.load(Ordering::SeqCst), before_attempts);
                    assert_eq!(game.world.block(6, 64, 8), lit_ore);
                    assert_eq!(game.world.block(7, 64, 8), grass);
                    assert_eq!(game.held().unwrap().count, 2);

                    game.world.set(7, 64, 4, ore).unwrap();
                    game.world.set(7, 65, 4, bottom_slab).unwrap();
                    let before_attempts = probe.0.load(Ordering::SeqCst);
                    let (consumed, fell_through) = click_server(
                        game,
                        top_hit([7, 64, 4]),
                        Some(ItemStack::new("minecraft:oak_slab", 2)),
                    );
                    assert!(consumed);
                    assert!(fell_through, "oak_slab merges into adjacent bottom slab");
                    assert!(!game.use_failed);
                    assert_eq!(probe.0.load(Ordering::SeqCst), before_attempts + 1);
                    assert_eq!(game.world.block(7, 64, 4), lit_ore);
                    assert_eq!(
                        registry
                            .state(game.world.block(7, 65, 4))
                            .unwrap()
                            .properties["type"],
                        "double"
                    );
                    assert_eq!(game.held().unwrap().count, 1);

                    // 4c. Grass standing on the ore: lighting it updates the grass's shape,
                    //     the grass can't survive on ore and breaks, and `canPlace` then
                    //     sees air: PASS, as in vanilla. Placing grass on ore then fails.
                    game.world.set(11, 64, 4, ore).unwrap();
                    game.world.set(11, 65, 4, grass).unwrap();
                    let before_attempts = probe.0.load(Ordering::SeqCst);
                    let (_, fell_through) = click_server(
                        game,
                        top_hit([11, 64, 4]),
                        Some(ItemStack::new("minecraft:short_grass", 2)),
                    );
                    assert!(
                        fell_through,
                        "the grass broke before BlockPlaceContext.canPlace"
                    );
                    assert_eq!(probe.0.load(Ordering::SeqCst), before_attempts + 1);
                    assert_eq!(game.world.block(11, 64, 4), lit_ore);
                    assert_eq!(game.world.block(11, 65, 4), air);
                    assert_eq!(game.held().unwrap().count, 2);

                    // 5. Non-block item and empty hand aimed at a valid spot: SUCCESS + lit.
                    game.world.set(8, 64, 4, ore).unwrap();
                    game.world.set(8, 65, 4, air).unwrap();
                    let (consumed, fell_through) = click_server(
                        game,
                        top_hit([8, 64, 4]),
                        Some(ItemStack::new("minecraft:stick", 1)),
                    );
                    assert!(consumed);
                    assert!(!fell_through);
                    assert!(!game.use_failed);
                    assert_eq!(game.world.block(8, 64, 4), lit_ore);
                    assert_eq!(game.world.block(8, 65, 4), air);

                    game.world.set(9, 64, 4, ore).unwrap();
                    game.world.set(9, 65, 4, air).unwrap();
                    let (consumed, fell_through) = click_server(game, top_hit([9, 64, 4]), None);
                    assert!(consumed);
                    assert!(!fell_through);
                    assert!(!game.use_failed);
                    assert_eq!(game.world.block(9, 64, 4), lit_ore);

                    // 6. Client prediction (`remote_block_reacts` / `client_block_answer`):
                    //    must follow the same `BlockPlaceContext.canPlace()` and `BlockItem` check.
                    game.options.execution_role =
                        crate::game::execution_role::ExecutionRole::Client;
                    game.options.dedicated = None;
                    let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(pipe, "RedstoneOreClient").unwrap();
                    game.remote.as_mut().unwrap().logged_in = true;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.sync_tick_chunks();

                    let client_click =
                        |game: &mut crate::game::Game<'_>, hit: Hit, item: Option<&str>| {
                            let held = item.map(|id| ItemStack::new(id, 1));
                            game.remote_block_reacts(&hit, held.as_ref(), true).0
                        };

                    // Air above redstone ore: cobblestone and redstone (item != block name) PASS;
                    // stick and empty hand return SUCCESS.
                    game.world.set(2, 64, 4, ore).unwrap();
                    game.world.set(2, 65, 4, air).unwrap();
                    assert_eq!(
                        client_click(game, top_hit([2, 64, 4]), Some("minecraft:cobblestone")),
                        InteractionResult::Pass
                    );
                    assert_eq!(
                        client_click(game, top_hit([2, 64, 4]), Some("minecraft:redstone")),
                        InteractionResult::Pass
                    );
                    assert_eq!(
                        client_click(game, top_hit([2, 64, 4]), Some("minecraft:stick")),
                        InteractionResult::Success
                    );
                    assert_eq!(
                        client_click(game, top_hit([2, 64, 4]), None),
                        InteractionResult::Success
                    );

                    // Solid stone above: cobblestone returns SUCCESS on both redstone ores.
                    game.world.set(2, 65, 4, stone).unwrap();
                    assert_eq!(
                        client_click(game, top_hit([2, 64, 4]), Some("minecraft:cobblestone")),
                        InteractionResult::Success
                    );
                    game.world.set(2, 64, 4, deepslate_ore).unwrap();
                    assert_eq!(
                        client_click(game, top_hit([2, 64, 4]), Some("minecraft:cobblestone")),
                        InteractionResult::Success
                    );

                    // Replaceability nuances on client: slab merges (PASS), same grass refuses (SUCCESS).
                    game.world.set(2, 64, 4, ore).unwrap();
                    game.world.set(2, 65, 4, bottom_slab).unwrap();
                    assert_eq!(
                        client_click(game, top_hit([2, 64, 4]), Some("minecraft:oak_slab")),
                        InteractionResult::Pass
                    );
                    assert_eq!(
                        client_click(game, top_hit([2, 64, 4]), Some("minecraft:cobblestone")),
                        InteractionResult::Success
                    );
                    // The client never lights the ore (only `spawnParticles`), so grounded
                    // grass beside it is what `canPlace` reads.
                    game.world.set(2, 64, 8, ore).unwrap();
                    game.world.set(3, 63, 8, dirt).unwrap();
                    game.world.set(3, 64, 8, grass).unwrap();
                    assert_eq!(
                        client_click(
                            game,
                            hit_on([2, 64, 8], EAST),
                            Some("minecraft:short_grass")
                        ),
                        InteractionResult::Success
                    );
                    assert_eq!(
                        client_click(
                            game,
                            hit_on([2, 64, 8], EAST),
                            Some("minecraft:cobblestone")
                        ),
                        InteractionResult::Pass
                    );

                    // Off-hand client prediction (`remote_use_item_on` -> `client_block_answer`):
                    // blocked destination consumes as SUCCESS without attempting placement.
                    game.world.set(2, 65, 4, stone).unwrap();
                    game.offhand = Some(ItemStack::new("minecraft:cobblestone", 2));
                    let before_swings = game.swings_started;
                    let before_attempts = probe.0.load(Ordering::SeqCst);
                    assert!(game.remote_use_item_on(
                        Hand::Off,
                        mod_api::InteractionAttempt::Press,
                        Some(top_hit([2, 64, 4])),
                    ));
                    assert_eq!(game.swings_started, before_swings + 1);
                    assert_eq!(probe.0.load(Ordering::SeqCst), before_attempts);
                    assert_eq!(game.offhand.as_ref().unwrap().count, 2);
                    assert_eq!(game.world.block(2, 65, 4), stone);
                });
            });
        },
    );
}
