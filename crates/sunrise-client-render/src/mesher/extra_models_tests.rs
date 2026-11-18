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

fn bytes(vertices: &[TerrainVertex]) -> &[u8] {
    bytemuck::cast_slice(vertices)
}

/// A checkerboard section, every face of its 2,048 stones open but those toward the absent
/// neighbouring columns, each stone drawing `BLOCK_EXTRA_MODELS_MAX` gold cubes: 753,664 extra
/// faces asked for, nearly three times the section's quads. The mesh stays within the section's quad and byte limits, keeps every stone face as
/// it was, keeps the extra models in cell order and then list order up to the limit, and is the
/// same on every build.
#[test]
fn extra_models_stay_within_the_sections_limits() {
    use super::extra_models::FACE_QUADS;
    use super::mod_geometry::{MAX_QUADS, MAX_SECTION_BYTES};

    let Some(world) = World::load() else {
        return;
    };
    let tables = world.tables(Vec::new());
    let mut mesher = world.mesher(&tables);
    let stone = world
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    let gold = model_of(&world, "minecraft:gold_block", world.render.generation);
    let mut plain = Chunk::empty(0, 0);
    let mut cells = Vec::new();
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                if (x + y + z) % 2 == 0 {
                    plain.set(x, MIN_Y + y, z, stone.0);
                    cells.push([x, y, z]);
                }
            }
        }
    }
    let mut chunk = plain.clone();
    let heavy: Arc<[_]> = vec![gold; mod_api::BLOCK_EXTRA_MODELS_MAX].into();
    for &[x, y, z] in &cells {
        let cell = chunk.dynamic_cell(x, MIN_Y + y, z).unwrap();
        chunk.extra_models.insert(cell, heavy.clone());
    }
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&plain);
    let base = mesher.mesh(&region, 0);
    assert_eq!(quads(&base.solid), cells.len() * 6 - 4 * 128);
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);

    let vertices = mesh.solid.len() + mesh.translucent.len();
    let used = vertices * size_of::<TerrainVertex>()
        + mesh.solid_tints.len() * size_of::<u32>()
        + (mesh.solid_sources.len() + mesh.translucent_sources.len()) * size_of::<TerrainSource>();
    assert!(vertices / 4 <= MAX_QUADS, "{} quads", vertices / 4);
    assert!(used <= MAX_SECTION_BYTES, "{used} bytes");
    assert!(
        vertices / 4 > MAX_QUADS - 2 * FACE_QUADS,
        "the extra models fill the section up to its limit: {} quads",
        vertices / 4
    );
    // Every stone face, unchanged and first.
    assert_eq!(
        bytes(&mesh.solid[..base.solid.len()]),
        bytes(&base.solid),
        "the block geometry is kept whole"
    );
    // The extra models after it, from the first cell on: its whole list, and none for the last.
    let extra = &mesh.solid[base.solid.len()..];
    let gold_layer = layer(&world, GOLD);
    assert!(extra.iter().all(|v| v.layer & !OPAQUE_LAYER == gold_layer));
    // A quad of a cell's cube: its four corners on it (a neighbour's meets it at an edge at most).
    let of = |quad: &[TerrainVertex], [x, y, z]: [i32; 3]| {
        let lo = [x as f32, y as f32, z as f32];
        quad.iter()
            .all(|v| (0..3).all(|k| (lo[k]..=lo[k] + 1.0).contains(&v.pos[k])))
    };
    let faces =
        |vertices: &[TerrainVertex], cell| vertices.chunks(4).filter(|quad| of(quad, cell)).count();
    let first = 4 * faces(&base.solid, cells[0]) * mod_api::BLOCK_EXTRA_MODELS_MAX;
    assert!(first > 0);
    assert!(extra[..first].chunks(4).all(|quad| of(quad, cells[0])));
    assert_eq!(faces(extra, *cells.last().unwrap()), 0);
    // The same section meshes the same, quad for quad.
    let again = mesher.mesh(&region, 0);
    assert_eq!(bytes(&again.solid), bytes(&mesh.solid));
    assert_eq!(bytes(&again.translucent), bytes(&mesh.translucent));
}
