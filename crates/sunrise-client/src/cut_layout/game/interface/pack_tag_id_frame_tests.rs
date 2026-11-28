//! A `shaders/` pack's `%<tag>` id entries (spec 4.1, D-9) through the game's frames: the pack
//! loads through `shader_pack_inputs` (its pipelines on their own thread) from its own
//! `block.properties`, `item.properties` and `entity.properties`; the terrain carries the tag's
//! id as `mc_Entity` for the blocks the world's tags list under it, the held and rendered item
//! ids and the entity ids read the item and entity type tags, and all of them follow the tags
//! when they are replaced. Each way the terrain gets its ids runs in a process of its own (one
//! event loop, its own resources): flat draws and per-section draws reading the arena's pages
//! read them from the pack's terrain table; per-section draws on the normal stream read them from
//! the stream, meshed again with them.
use super::pack_layer_frame_tests::{
    GREEN, RED, drawn_frame, pack_frames, scene_of, settle, slabs_drawn, state, write_files,
};
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client_gameplay::game::ClientGame;
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_gameplay_common::game::test_fixtures::{run_fresh_cases, run_fresh_cases_with_env};
use sunrise_protocol::packets::play::Clientbound;

const NAME: &str =
    "cut_layout::game::pack_tag_id_frame_tests::a_tags_id_reaches_the_terrain_and_follows_the_tags";
const STREAM_NAME: &str = "cut_layout::game::pack_tag_id_frame_tests::a_tags_id_reaches_the_terrain_through_the_normal_stream";
const CASE: &str = "SUNRISE_PACK_TAG_ID_FRAME_CASE";

/// Green where the quad's `mc_Entity.x` is 42, red elsewhere.
const VERTEX: &str = "#version 120\nattribute vec4 mc_Entity;\nvarying vec4 glcolor;\n\
    void main() {\n    gl_Position = ftransform();\n\
    glcolor = mc_Entity.x == 42.0 ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0);\n}\n";
const FRAGMENT: &str =
    "#version 120\nvarying vec4 glcolor;\nvoid main() {\n    gl_FragData[0] = glcolor;\n}\n";

/// The issue's case through flat draws and per-section draws reading the arena's pages.
#[test]
#[ignore = "daily"]
fn a_tags_id_reaches_the_terrain_and_follows_the_tags() {
    if run_fresh_cases(NAME, CASE, &["flat", "pages"]) {
        return;
    }
    tag_id_frames(NAME);
}

/// The same through per-section draws on the normal stream: with no storage reads in vertex
/// shaders, the pages are no storage and no terrain table is built, so only meshing the
/// sections again gives the terrain the new ids.
#[test]
#[ignore = "daily"]
fn a_tags_id_reaches_the_terrain_through_the_normal_stream() {
    if run_fresh_cases_with_env(
        STREAM_NAME,
        CASE,
        &["normals"],
        1,
        &[("SUNRISE_SHADER_PACK_WITHOUT", "vertex-storage")],
    ) {
        return;
    }
    tag_id_frames(STREAM_NAME);
}

/// The world's tags replaced, as a server's tags packet (which `/reload` sends) replaces them:
/// `minecraft:logs` holds `logs`, `minecraft:swords` `swords` and `minecraft:skeletons`
/// `skeletons`.
fn retag(game: &mut ClientGame<'_>, logs: &[&str], swords: &[&str], skeletons: &[&str]) {
    let builtin = game.client.remote.as_ref().unwrap().builtin.clone();
    let update = crate::game::client_side::remote_tags::tests::update(
        &builtin,
        &[
            ("minecraft:block", "minecraft:logs", logs),
            ("minecraft:item", "minecraft:swords", swords),
            ("minecraft:entity_type", "minecraft:skeletons", skeletons),
        ],
    );
    game.handle_packet(Clientbound::UpdateTags(update)).unwrap();
}

/// The ids the pack gives the held or rendered oak log, stone, oak slab, iron sword and stick
/// (`currentRenderedItemId`: a block item's is its block's), then a skeleton and a zombie
/// (`entityId`).
fn item_and_entity_ids(game: &ClientGame<'_>) -> [i32; 7] {
    let entity = |name| game.entity_part_span(0, name, false).unwrap().id;
    [
        game.rendered_item_id("minecraft:oak_log"),
        game.rendered_item_id("minecraft:stone"),
        game.rendered_item_id("minecraft:oak_slab"),
        game.rendered_item_id("minecraft:iron_sword"),
        game.rendered_item_id("minecraft:stick"),
        entity("minecraft:skeleton"),
        entity("minecraft:zombie"),
    ]
}

/// The issue's own line, `block.42=%minecraft:logs`, beside `item.10=%minecraft:swords` and
/// `entity.20=%minecraft:skeletons`, over an oak log north of the player and an oak slab south
/// of it, in this process's case (`flat`, `pages` or `normals`):
/// - before any pack, neither block draws a program's colour; the server's tags packet lists
///   oak logs, iron swords and skeletons;
/// - once the pack runs, the log (in `minecraft:logs`) is 42 and the slab (in no tag the file
///   names) is not; the oak log's item renders as its block's 42, the iron sword is 10 and a
///   skeleton 20;
/// - the server's tags replaced (oak logs, iron swords and skeletons out; stone, sticks and
///   zombies in): the log is not 42, the stone item renders as 42, and the items and entities
///   follow;
/// - replaced again with the slab in `minecraft:logs`: the slab is 42;
/// - the pack picked again, and the tags replaced while its pipelines build: it runs with the
///   ids of the tags replaced, not of those it was loaded with.
fn tag_id_frames(name: &str) {
    let case = std::env::var(CASE).unwrap();
    let flat = case == "flat";
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("tagged");
    write_files(
        &pack,
        &[
            ("block.properties", "block.42=%minecraft:logs\n".into()),
            ("item.properties", "item.10=%minecraft:swords\n".into()),
            (
                "entity.properties",
                "entity.20=%minecraft:skeletons\n".into(),
            ),
            ("gbuffers_terrain.vsh", VERTEX.into()),
            ("gbuffers_terrain.fsh", FRAGMENT.into()),
        ],
    );
    with_client_game(|game| {
        let blocks = |game: &ClientGame<'_>| {
            [
                state(game, "minecraft:oak_log", &[("axis", "y")]),
                state(
                    game,
                    "minecraft:oak_slab",
                    &[("type", "bottom"), ("waterlogged", "false")],
                ),
            ]
        };
        let Some(_event_loop) = scene_of(game, flat, blocks) else {
            eprintln!("skipped: {name}: this adapter has no flat section draws");
            return;
        };
        let native = slabs_drawn(game);
        for pixel in native {
            assert!(pixel != RED && pixel != GREEN, "{native:?}");
        }
        // The server's tags: oak logs, iron swords and skeletons.
        retag(
            game,
            &["minecraft:oak_log"],
            &["minecraft:iron_sword"],
            &["minecraft:skeleton"],
        );
        let items = game.items();
        assert!(
            items.block_tags["minecraft:logs"].contains("minecraft:oak_log")
                && items.item_tags["minecraft:swords"].contains("minecraft:iron_sword")
                && (items.entity_type_tags).contains("minecraft:skeletons", "minecraft:skeleton"),
            "the fixture's tags list oak logs, iron swords and skeletons"
        );

        game.client.settings.sunrise.shader_pack = pack.to_str().unwrap().to_owned();
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [GREEN, RED],
            "the log through %minecraft:logs"
        );
        // The way this case's terrain reads its ids: the section arena has a normal stream only
        // where the pack's per-section draws read no pages (`follow_arena_layout`).
        let renderer = game.client.renderer.0.as_ref().unwrap();
        let (pages, normals) = (
            renderer.section_arena_pages().0,
            renderer.section_arena_normals().0,
        );
        match case.as_str() {
            "pages" => assert!(pages && !normals, "per-section draws read the pages"),
            "normals" => assert!(normals, "per-section draws read the stream"),
            _ => {}
        }
        assert_eq!(
            item_and_entity_ids(game),
            [42, 0, 0, 10, 65535, 20, 65535],
            "the files' tags"
        );

        retag(
            game,
            &["minecraft:stone"],
            &["minecraft:stick"],
            &["minecraft:zombie"],
        );
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [RED, RED],
            "oak logs left the tag"
        );
        assert_eq!(
            item_and_entity_ids(game),
            [0, 42, 0, 65535, 10, 65535, 20],
            "the replaced tags"
        );
        retag(
            game,
            &["minecraft:oak_slab"],
            &["minecraft:stick"],
            &["minecraft:zombie"],
        );
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [RED, GREEN],
            "the slab joined the tag"
        );
        assert_eq!(item_and_entity_ids(game), [0, 0, 42, 65535, 10, 65535, 20]);

        // Dropped, loaded again with oak logs in the tag, and the tags replaced while it builds.
        game.client.settings.sunrise.shader_pack = String::new();
        drawn_frame(game);
        settle(game);
        retag(
            game,
            &["minecraft:oak_log"],
            &["minecraft:iron_sword"],
            &["minecraft:skeleton"],
        );
        game.client.settings.sunrise.shader_pack = pack.to_str().unwrap().to_owned();
        drawn_frame(game);
        assert!(game.client.pack_ids.is_some(), "the pack loads");
        assert!(
            game.client
                .renderer
                .0
                .as_ref()
                .unwrap()
                .shader_pack_kind()
                .is_none(),
            "its pipelines are still building"
        );
        retag(
            game,
            &["minecraft:oak_slab"],
            &["minecraft:stick"],
            &["minecraft:zombie"],
        );
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [RED, GREEN],
            "the tags replaced while it built"
        );
        assert_eq!(item_and_entity_ids(game), [0, 0, 42, 65535, 10, 65535, 20]);
    });
}
