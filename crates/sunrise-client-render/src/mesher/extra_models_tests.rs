//! A client mod's extra models (`Chunk::extra_models`) drawn as part of their block: placed,
//! lit and culled as its own faces, each quad in its own pass, and nothing for a stale handle.
use super::mod_geometry_tests::{World, quads};
use super::*;

const GOLD: &str = "minecraft:block/gold_block";
const GLASS: &str = "minecraft:block/white_stained_glass";

/// The handle of the first baked model block `name`'s default state draws, in `world`'s store
/// of `generation`.
fn model_of(world: &World, name: &str, generation: u64) -> mod_api::ModelRef {
    let state = world.assets.registry.default_state(name).unwrap();
    let slot = world.render.model_catalog.state_slots(state).unwrap()[0];
    mod_api::__private::built::model_ref(generation, slot)
}

/// `chunk` with `models` drawn as part of the block at (2, MIN_Y + 4, 2), `world.cube`'s stone.
fn with_extra(mut chunk: Chunk, models: &[mod_api::ModelRef]) -> Chunk {
    let cell = chunk.dynamic_cell(2, MIN_Y + 4, 2).unwrap();
    chunk.extra_models.insert(cell, models.into());
    chunk
}

fn layer(world: &World, sprite: &str) -> u16 {
    world.render.sprite_layers[sprite] as u16
}

fn on_layer(vertices: &[TerrainVertex], layer: u16) -> Vec<TerrainVertex> {
    vertices
        .iter()
        .filter(|v| v.layer & !OPAQUE_LAYER == layer)
        .copied()
        .collect()
}

#[test]
fn extra_models_draw_with_the_blocks_place_and_light() {
    let Some(world) = World::load() else {
        return;
    };
    let tables = world.tables(Vec::new());
    let mut mesher = world.mesher(&tables);
    let gold = model_of(&world, "minecraft:gold_block", world.render.generation);
    let plain = world.cube();
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&plain);
    let stone = mesher.mesh(&region, 0);
    assert_eq!(quads(&stone.solid), 6);
    let chunk = with_extra(world.cube(), &[gold]);
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);
    assert_eq!(
        quads(&mesh.solid),
        12,
        "the stone's six faces and the gold's six"
    );
    let added = on_layer(&mesh.solid, layer(&world, GOLD));
    assert_eq!(quads(&added), 6);
    // A full cube over the stone's: each gold corner where a stone corner is, lit as it is.
    let own: Vec<_> = stone.solid.iter().map(|v| (v.pos, v.light)).collect();
    assert!(added.iter().all(|v| own.contains(&(v.pos, v.light))));
    // The same model twice is drawn twice.
    let chunk = with_extra(world.cube(), &[gold, gold]);
    region[1][1] = Some(&chunk);
    assert_eq!(quads(&mesher.mesh(&region, 0).solid), 18);
}

#[test]
fn extra_faces_are_culled_by_neighbours_and_keep_their_pass() {
    let Some(world) = World::load() else {
        return;
    };
    let tables = world.tables(Vec::new());
    let mut mesher = world.mesher(&tables);
    let generation = world.render.generation;
    let gold = model_of(&world, "minecraft:gold_block", generation);
    let glass = model_of(&world, "minecraft:white_stained_glass", generation);
    let mut chunk = with_extra(world.cube(), &[gold, glass]);
    // Stone east of the block hides its east faces, the gold's and the glass's with them.
    let stone = world
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    chunk.set(3, MIN_Y + 4, 2, stone.0);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);
    assert_eq!(
        quads(&on_layer(&mesh.solid, layer(&world, GOLD))),
        5,
        "no gold face toward the neighbour"
    );
    let glass_quads = on_layer(&mesh.translucent, layer(&world, GLASS));
    assert_eq!(quads(&glass_quads), 5, "the glass in the translucent pass");
    assert!(on_layer(&mesh.solid, layer(&world, GLASS)).is_empty());
    assert_eq!(quads(&mesh.solid), 5 + 5 + 5, "both stones and the gold");
}

#[test]
fn a_stale_handle_draws_nothing() {
    let Some(world) = World::load() else {
        return;
    };
    let tables = world.tables(Vec::new());
    let mut mesher = world.mesher(&tables);
    let stale = model_of(&world, "minecraft:gold_block", world.render.generation + 1);
    let chunk = with_extra(world.cube(), &[stale]);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);
    assert_eq!(quads(&mesh.solid), 6, "the stone alone");
    assert!(on_layer(&mesh.solid, layer(&world, GOLD)).is_empty());
}
