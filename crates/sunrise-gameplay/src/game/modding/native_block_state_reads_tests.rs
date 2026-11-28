//! `BlockState.getLightBlock`, `isSuffocating` and `isValidSpawn` through `BlockStates` (#6035).
use crate::game::native_entity_refs::tests::with_game;
use mod_api::HostBlockStates;

/// Stone stops all light and suffocates, glass neither; a shulker box's suffocation depends on
/// where it is; leaves take ocelots and parrots only, ice polar bears only, and a magma block
/// fire-immune mobs only.
#[test]
#[ignore = "daily"]
fn a_state_answers_its_light_block_suffocation_and_spawn_rules() {
    with_game(|game| {
        let id =
            |name: &str| mod_api::prelude::BlockStatesHost::block_state_id(&*game, name).unwrap();
        let states = game.block_states();
        assert_eq!(states.light_block(id("minecraft:stone")), Some(15));
        assert_eq!(states.light_block(id("minecraft:glass")), Some(0));
        assert_eq!(states.light_block(id("minecraft:air")), Some(0));

        assert_eq!(states.is_suffocating(id("minecraft:stone")), Some(true));
        assert_eq!(states.is_suffocating(id("minecraft:glass")), Some(false));
        assert_eq!(states.is_suffocating(id("minecraft:shulker_box")), None);

        let spawn = |block: &str, mob: &str| states.is_valid_spawn(id(block), mob);
        assert_eq!(spawn("minecraft:stone", "minecraft:zombie"), Some(true));
        assert_eq!(spawn("minecraft:glass", "minecraft:zombie"), Some(false));
        assert_eq!(
            spawn("minecraft:oak_leaves", "minecraft:ocelot"),
            Some(true)
        );
        assert_eq!(
            spawn("minecraft:oak_leaves", "minecraft:zombie"),
            Some(false)
        );
        assert_eq!(spawn("minecraft:ice", "minecraft:polar_bear"), Some(true));
        assert_eq!(spawn("minecraft:ice", "minecraft:zombie"), Some(false));
        assert_eq!(
            spawn("minecraft:magma_block", "minecraft:blaze"),
            Some(true)
        );
        assert_eq!(
            spawn("minecraft:magma_block", "minecraft:zombie"),
            Some(false)
        );
        assert_eq!(
            spawn("minecraft:stone", "zombie"),
            Some(true),
            "a bare id is minecraft's"
        );
        assert_eq!(spawn("minecraft:stone", "minecraft:no_such_mob"), None);
    });
}

/// `is_suffocating` reads the open world's suffocation rules, its data packs'
/// `#minecraft:causes_suffocation`, not the built-in ones: a pack that replaces the tag with dirt
/// alone stops stone suffocating once its tables are installed, and dirt suffocates. Both the
/// host's `block_states()` and a level view's read them.
#[test]
#[ignore = "daily"]
fn suffocation_follows_the_worlds_causes_suffocation_tag() {
    with_game(|game| {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pack = root.join("datapacks/tags");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"Suffocation tag fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        write(
            "data/minecraft/tags/block/causes_suffocation.json",
            r#"{"replace":true,"values":["minecraft:dirt"]}"#,
        );
        std::fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/tags"],"disabled":[]}"#,
        )
        .unwrap();
        let id = |game: &crate::game::ServerGame<'_>, name: &str| {
            mod_api::prelude::BlockStatesHost::block_state_id(game, name).unwrap()
        };
        let (stone, dirt) = (id(game, "minecraft:stone"), id(game, "minecraft:dirt"));
        assert_eq!(game.block_states().is_suffocating(stone), Some(true));

        let resources = game.common.options.resources.clone();
        let stack = crate::game::functions::pack_stack(
            root,
            &resources,
            &game.common.pack.data_source,
            &game.common.pack.data_changes,
            false,
            &[],
            &[],
        );
        let prepared = game.prepare_world_reload(&stack, &resources).unwrap();
        game.install_world_tables(prepared);
        let states = game.block_states();
        assert_eq!(states.is_suffocating(stone), Some(false));
        assert_eq!(states.is_suffocating(dirt), Some(true));
        // A level view's tables (a part's level callback) read the same rules.
        let side = server_game_side!(game);
        let level = mod_api::BlockStates::of(&side.tables);
        assert_eq!(level.is_suffocating(stone), Some(false));
        assert_eq!(level.is_suffocating(dirt), Some(true));
    });
}
