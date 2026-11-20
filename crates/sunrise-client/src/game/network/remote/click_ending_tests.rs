use super::*;
use crate::game::remote::use_ends_click;

/// `FireworkRocketItem.use` is SUCCESS while gliding: the main hand's
/// rocket ends the click and the off hand's shield is not raised (#3424).
/// On foot it passes, so the off hand still gets its turn.
#[test]
#[ignore = "daily"]
fn a_rocket_used_while_gliding_ends_the_click() {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let registry = sunrise_world::level::data::load_registry(&root).unwrap();
    let facts = sunrise_client::blocks::BlockFacts::load(&root, &registry).unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = sunrise_client::items::Items::load(
        &root,
        &registry,
        &facts,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let rocket = items.get("minecraft:firework_rocket").unwrap();
    assert!(use_ends_click(rocket, true), "gliding");
    assert!(!use_ends_click(rocket, false), "on foot");
    // Other items answer the same whether or not the player glides.
    let shield = items.get("minecraft:shield").unwrap();
    let stick = items.get("minecraft:stick").unwrap();
    for gliding in [false, true] {
        assert!(use_ends_click(shield, gliding));
        assert!(!use_ends_click(stick, gliding));
        // FoodOnAStickItem.use passes on a client: no swing while the boost cools down.
        for rod in [
            "minecraft:carrot_on_a_stick",
            "minecraft:warped_fungus_on_a_stick",
        ] {
            let rod = items.get(rod).unwrap();
            assert!(!use_ends_click(rod, gliding), "{}", rod.id);
        }
        // A carved pumpkin has an equip_slot (head) but is not swappable, so it has no Item.use.
        let pumpkin = items.get("minecraft:carved_pumpkin").unwrap();
        assert!(!use_ends_click(pumpkin, gliding));
    }
}

/// #14773: Flint and steel and fire charge useOn on a client level ends the
/// click (returning true), so when held with an unusable off-hand item, the
/// click does not fall through to UseItem or try the off hand.
#[test]
#[ignore = "daily"]
fn flint_and_steel_use_on_ends_the_click() {
    use crate::game::Hit;
    use crate::game::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    use sunrise_client::{first_person::Hand, motion::PlayerMotion};
    use sunrise_core::geometry::Vec3;

    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "FlintTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.inventory[0] = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    game.offhand = Some(ItemStack::new("minecraft:stick", 1));
    game.selected = 0;
    game.forced_hit = Some(Hit {
        pos: [0, 63, 0],
        face: [0, 1, 0],
        point: [0.5, 64.0, 0.5],
        inside: false,
    });

    let done = game.remote_use_block(Hand::Main);
    assert!(done, "flint and steel useOn must end the click");
}

/// EndCrystalItem.useOn ends the click either way: SUCCESS (the arm swings) on obsidian with
/// air and nothing above, FAIL (no swing) on grass or with the player in the space above. Each
/// sends one use_item_on and tries neither UseItem nor the off hand, whose shield stays down.
/// On cooldown its useOn doesn't run, and the click goes on to the item.
#[test]
#[ignore = "daily"]
fn an_end_crystal_click_on_a_block_ends_there() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        use crate::game::Hit;
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, first_person::Hand, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play;
        let (mut peer, _writer) = connect_menu_test(game);
        peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        game.set_mode_id(0);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = &game.pack.assets.registry;
        let obsidian = registry.default_state("minecraft:obsidian").unwrap();
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        // Under the player (its own box fills the space above), clear, and grass.
        game.world.set(0, 63, 0, obsidian).unwrap();
        game.world.set(0, 63, 2, obsidian).unwrap();
        game.world.set(0, 63, 4, grass).unwrap();
        let crystal = ItemStack::new("minecraft:end_crystal", 4);
        game.inventory[0] = Some(crystal.clone());
        game.offhand = Some(ItemStack::new("minecraft:shield", 1));
        game.selected = 0;
        while peer.recv().is_ok() {}
        let aim = |game: &mut crate::game::Game<'_>, z: i32| {
            game.forced_hit = Some(Hit {
                pos: [0, 63, z],
                face: [0, 1, 0],
                point: [0.5, 64.0, f64::from(z) + 0.5],
                inside: false,
            });
        };
        // The whole click (Minecraft.startUseItem): it ends at the block, so neither the main
        // hand's air use nor the offhand's shield is tried, and nothing is held in use.
        let mut click = |game: &mut crate::game::Game<'_>, z: i32| {
            aim(game, z);
            let swings = game.swings_started;
            game.remote_use(mod_api::InteractionAttempt::Press);
            let mut sent = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                match play::Serverbound::decode(id, &bytes) {
                    Ok(play::Serverbound::UseItemOn(_)) => sent.push("use_item_on"),
                    Ok(play::Serverbound::UseItem(_)) => sent.push("use_item"),
                    _ => {}
                }
            }
            assert!(game.using.is_none(), "z={z}");
            (game.swings_started != swings, sent)
        };
        assert_eq!(click(game, 2), (true, vec!["use_item_on"]));
        assert_eq!(click(game, 4), (false, vec!["use_item_on"]));
        assert_eq!(click(game, 0), (false, vec!["use_item_on"]));
        // On cooldown the block use itself passes.
        game.cooldowns
            .add(sunrise_client::cooldowns::group(&crystal, None), 20);
        aim(game, 2);
        assert!(!game.remote_use_block(Hand::Main));
    });
}

/// EnderEyeItem.useOn is SUCCESS into a frame without an eye: the arm swings and the click ends
/// with one use_item_on. On a frame with an eye it passes, so Item.use follows (use_item).
/// MinecartItem.useOn ends the click either way: SUCCESS (swing) on a rail, FAIL (no swing) on
/// any other block. CompassItem.useOn on a lodestone is SUCCESS. After those, neither the main
/// hand's air use nor the off hand's torch is tried.
#[test]
#[ignore = "daily"]
fn an_ender_eye_and_a_minecart_end_the_click_where_vanilla_does() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        use crate::game::Hit;
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play;
        let (mut peer, _writer) = connect_menu_test(game);
        peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        game.set_mode_id(0);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let frame = |eye: &str| {
            registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:end_portal_frame".into(),
                    properties: [("eye", eye), ("facing", "north")]
                        .into_iter()
                        .map(|(key, value)| (key.into(), value.into()))
                        .collect(),
                })
                .unwrap()
        };
        game.world.set(0, 63, 2, frame("false")).unwrap();
        game.world.set(0, 63, 4, frame("true")).unwrap();
        game.world
            .set(0, 64, 6, registry.default_state("minecraft:rail").unwrap())
            .unwrap();
        game.world
            .set(0, 63, 8, registry.default_state("minecraft:stone").unwrap())
            .unwrap();
        game.world
            .set(
                0,
                63,
                10,
                registry.default_state("minecraft:lodestone").unwrap(),
            )
            .unwrap();
        game.offhand = Some(ItemStack::new("minecraft:torch", 8));
        game.selected = 0;
        while peer.recv().is_ok() {}
        let mut click = |game: &mut crate::game::Game<'_>, item: &str, pos: [i32; 3]| {
            game.inventory[0] = Some(ItemStack::new(item, 4));
            game.forced_hit = Some(Hit {
                pos,
                face: [0, 1, 0],
                point: [0.5, f64::from(pos[1]) + 1.0, f64::from(pos[2]) + 0.5],
                inside: false,
            });
            let swings = game.swings_started;
            game.remote_use(mod_api::InteractionAttempt::Press);
            let mut sent = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                match play::Serverbound::decode(id, &bytes) {
                    Ok(play::Serverbound::UseItemOn(p)) => sent.push(if p.hand == 0 {
                        "use_item_on"
                    } else {
                        "off_hand_use_item_on"
                    }),
                    Ok(play::Serverbound::UseItem(_)) => sent.push("use_item"),
                    _ => {}
                }
            }
            (game.swings_started != swings, sent)
        };
        assert_eq!(
            click(game, "minecraft:ender_eye", [0, 63, 2]),
            (true, vec!["use_item_on"])
        );
        // EnderEyeItem.use PASSes too while the player looks at a frame, so the off hand's
        // torch gets its turn.
        assert_eq!(
            click(game, "minecraft:ender_eye", [0, 63, 4]).1,
            ["use_item_on", "use_item", "off_hand_use_item_on"]
        );
        assert_eq!(
            click(game, "minecraft:minecart", [0, 64, 6]),
            (true, vec!["use_item_on"])
        );
        assert_eq!(
            click(game, "minecraft:minecart", [0, 63, 8]),
            (false, vec!["use_item_on"])
        );
        // CompassItem.useOn on a lodestone: SUCCESS, the click ends there.
        assert_eq!(
            click(game, "minecraft:compass", [0, 63, 10]),
            (true, vec!["use_item_on"])
        );
    });
}

/// `Player.interactOn` through the whole click (`Minecraft.startUseItem`) on a pig in front of
/// the player: the mob's own `mobInteract` answers first, and only when it passes does the held
/// stack's own `equippable` (`ItemStack.interactLivingEntity`, `Equippable.equipOnTarget`) get
/// its turn. A saddle on an adult pig with none is SUCCESS (a swing, the click ends at the
/// interact); a server's patch that removes the component, or one whose allowed entities leave
/// pigs out, passes on to the item; one that adds it to a stick equips like a saddle. A saddled
/// pig someone rides passes (`isVehicle`) and its occupied slot takes nothing. A carrot the
/// patch makes equippable is still the pig's food first: CONSUME, no swing. A cookie poisons
/// any parrot: SUCCESS.
#[test]
#[ignore = "daily"]
fn a_mob_answers_first_and_then_the_stacks_own_equippable() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, mobs::Mob, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::{buf::Writer, item::component, packets::play};
        let (mut peer, _writer) = connect_menu_test(game);
        peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        game.set_mode_id(0);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        (game.yaw, game.pitch) = (0.0, 0.0);
        (game.entity_reach, game.block_reach) = (3.0, 4.5);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.offhand = None;
        game.selected = 0;
        let entity_type = |game: &crate::game::Game<'_>, id: &str| {
            let types = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type");
            types.unwrap().id(id).unwrap()
        };
        let cow = entity_type(&*game, "minecraft:cow");
        // A server's patch adding `equippable` for the saddle slot with equip_on_interact, its
        // allowed entities `allowed` (registry ids) or every type: slot 7, a sound reference,
        // no asset or overlay, then the dispensable, swappable, damage-on-hurt and
        // equip-on-interact flags, no shearing, the shearing sound.
        let equips = |allowed: Option<&[i32]>| {
            let mut body = Writer::new();
            body.varint(7);
            body.varint(1);
            body.bool(false);
            body.bool(false);
            body.bool(allowed.is_some());
            if let Some(ids) = allowed {
                body.varint(ids.len() as i32 + 1);
                for id in ids {
                    body.varint(*id);
                }
            }
            for flag in [true, true, true, true, false] {
                body.bool(flag);
            }
            body.varint(1);
            let mut patch = Writer::new();
            patch.varint(1);
            patch.varint(0);
            patch.varint(component::EQUIPPABLE);
            patch.varint(body.data.len() as i32);
            patch.data.extend(body.data);
            patch.data
        };
        let removes = {
            let mut patch = Writer::new();
            patch.varint(0);
            patch.varint(1);
            patch.varint(component::EQUIPPABLE);
            patch.data
        };
        let patched = |item: &str, patch: Vec<u8>| {
            let mut stack = ItemStack::new(item, 1);
            stack.data_mut().wire =
                sunrise_client::item_data::Wire::new(stack.clone(), Arc::from(patch));
            stack
        };
        let mut next_id = 40;
        // The mob of `kind` standing at `y`, its box across the player's eye line (65.62), alone
        // in the level.
        let mut place = |game: &mut crate::game::Game<'_>, kind: &str, y: f64| {
            next_id += 1;
            let kind = game.res.mob_types().get(kind).unwrap().clone();
            game.mobs.clear();
            game.mobs
                .push(Mob::new(next_id, kind, Vec3::new(0.5, y, 2.5), 180.0));
            game.mobs.len() - 1
        };
        while peer.recv().is_ok() {}
        let mut click = |game: &mut crate::game::Game<'_>, stack: ItemStack| {
            game.inventory[0] = Some(stack);
            let swings = game.swings_started;
            game.remote_use(mod_api::InteractionAttempt::Press);
            let mut sent = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                match play::Serverbound::decode(id, &bytes) {
                    Ok(play::Serverbound::Interact(p)) if p.hand == 0 => sent.push("interact"),
                    Ok(play::Serverbound::UseItem(p)) if p.hand == 0 => sent.push("use_item"),
                    _ => {}
                }
            }
            (game.swings_started != swings, sent)
        };
        let saddle = || ItemStack::new("minecraft:saddle", 1);
        let ends = (true, vec!["interact"]);

        place(game, "minecraft:pig", 65.0);
        assert_eq!(click(game, saddle()), ends, "a saddle on an adult pig");
        place(game, "minecraft:pig", 65.0);
        assert_eq!(
            click(game, patched("minecraft:stick", equips(None))),
            ends,
            "a stick the server made a saddle"
        );
        for (label, stack) in [
            (
                "a saddle without its equippable",
                patched("minecraft:saddle", removes.clone()),
            ),
            (
                "one only cows may wear",
                patched("minecraft:saddle", equips(Some(&[cow][..]))),
            ),
        ] {
            place(game, "minecraft:pig", 65.0);
            let (swung, sent) = click(game, stack);
            assert!(!swung, "{label}");
            assert_eq!(sent.first(), Some(&"interact"), "{label}");
            assert!(sent.contains(&"use_item"), "{label} passes to the item");
        }
        // A baby's box is half as tall: it stands higher to cross the eye line.
        let baby = place(game, "minecraft:pig", 65.3);
        game.mobs[baby].extra.age = -24000;
        let (swung, sent) = click(game, saddle());
        assert!(!swung, "a baby pig can't wear it");
        assert_eq!(sent.first(), Some(&"interact"));
        assert!(sent.contains(&"use_item"));
        // A saddled pig another mob rides: the pig passes, and its saddle slot is taken.
        let ridden = place(game, "minecraft:pig", 65.0);
        game.mobs[ridden].extra.set_saddled(true);
        let rider = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut rider = Mob::new(90, rider, Vec3::new(6.5, 64.0, 6.5), 0.0);
        rider.extra.mount = Some(game.mobs[ridden].id);
        game.mobs.push(rider);
        let (swung, sent) = click(game, saddle());
        assert!(!swung, "the occupied slot takes nothing");
        assert_eq!(sent.first(), Some(&"interact"));
        assert!(sent.contains(&"use_item"));
        // The pig's own Animal.mobInteract takes its food first (CONSUME), so the patched
        // equippable never runs and nothing swings.
        place(game, "minecraft:pig", 65.0);
        assert_eq!(
            click(game, patched("minecraft:carrot", equips(None))),
            (false, vec!["interact"]),
            "the mob's answer decides the swing"
        );
        place(game, "minecraft:parrot", 65.0);
        assert_eq!(click(game, ItemStack::new("minecraft:cookie", 1)), ends);
    });
}
