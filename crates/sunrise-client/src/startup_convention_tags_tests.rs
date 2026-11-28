//! Exercise the startup tag readers on one materialized mod resource tree.
use super::AnimateTables;
use crate::{assets::Assets, blocks::BlockFacts, models, motion::MotionResources};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs};
use sunrise_core::geometry::Direction;
use sunrise_world::{
    fluid::FluidKind, heightmap::HeightmapState, worldgen::terrain::TerrainBlockStates,
};

#[test]
#[ignore = "daily"]
fn startup_binds_required_convention_tags() -> crate::Result<()> {
    // Assets::load sets the process-wide chunk palette width and interns the registry.
    const CHILD: &str = "SUNRISE_STARTUP_CONVENTION_TAGS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        crate::fresh_process::run_in_fresh_process(
            "animate_tick::startup_convention_tags_tests::startup_binds_required_convention_tags",
            CHILD,
            "1",
        );
        return Ok(());
    }

    let copied = [
        "attributes.json",
        "block-behaviours.json",
        "cached-collision.nbt",
        "damage-catalog.json",
        "damage-tags.json",
        "facts.json",
        "gameplay-facts.json",
        "landing-definitions.json",
        "movement-properties.nbt",
        "occlusion.nbt",
        "player-float-ids.json",
        "sunrise-resources.json",
    ];
    let fluid_dir = "data/minecraft/tags/fluid";
    let prepared = crate::prepared_test_root::prepared_root(&[
        &copied,
        &["block-tags.json", "render-definitions.json", fluid_dir],
    ]);
    let scratch = tempfile::tempdir()?;
    let root = scratch.path();
    for name in copied {
        fs::copy(prepared.join(name), root.join(name))
            .unwrap_or_else(|error| panic!("prepared resource {name}: {error}"));
    }

    let mut blocks: BTreeMap<String, String> =
        sunrise_world::json_file::read(&prepared.join("block-tags.json"))?;
    blocks.insert(
        "test:startup_barrels".into(),
        json!({"values": ["#c:barrels"]}).to_string(),
    );
    // Make the heightmap observation independent of its vanilla membership.
    blocks.insert(
        "minecraft:blocks_motion_in_heightmap".into(),
        json!({"replace": true, "values": ["#test:startup_barrels"]}).to_string(),
    );
    for tag in ["minecraft:inside_step_sound_blocks", "minecraft:terracotta"] {
        let mut file: Value = serde_json::from_str(&blocks[tag])?;
        file["values"]
            .as_array_mut()
            .expect("a prepared block tag has values")
            .push(json!("#test:startup_barrels"));
        blocks.insert(tag.into(), file.to_string());
    }
    let unresolved = [
        "minecraft:blocks_motion_in_heightmap_no_leaves",
        "minecraft:crystal_sound_blocks",
    ];
    for tag in unresolved {
        blocks.insert(
            tag.into(),
            json!({"replace": true, "values": ["minecraft:barrel", "test:missing_block"]})
                .to_string(),
        );
    }
    fs::write(root.join("block-tags.json"), serde_json::to_vec(&blocks)?)?;

    let mut render: BTreeMap<String, Value> =
        sunrise_world::json_file::read(&prepared.join("render-definitions.json"))?;
    render.get_mut("minecraft:stone").unwrap()["skip_rendering"] =
        json!([{"neighbor": "#test:startup_barrels"}]);
    fs::write(
        root.join("render-definitions.json"),
        serde_json::to_vec(&render)?,
    )?;

    fs::create_dir_all(root.join(fluid_dir))?;
    for file in fs::read_dir(prepared.join(fluid_dir))? {
        let file = file?;
        if file.file_type()?.is_file() {
            fs::copy(file.path(), root.join(fluid_dir).join(file.file_name()))?;
        }
    }
    let lily_pad = root.join(fluid_dir).join("supports_lily_pad.json");
    let mut support: Value = sunrise_world::json_file::read(&lily_pad)?;
    support["values"]
        .as_array_mut()
        .expect("the prepared lily pad support tag has values")
        .push(json!("#c:water"));
    fs::write(lily_pad, serde_json::to_vec(&support)?)?;

    let assets = Assets::load(root).unwrap_or_else(|error| panic!("Assets::load: {error}"));
    let registry = assets.registry;
    let barrel = registry.default_state("minecraft:barrel").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    assert!(
        assets
            .properties
            .heightmap_state(barrel)
            .blocks_motion_in_heightmap()
    );
    assert!(
        !assets
            .properties
            .heightmap_state(stone)
            .blocks_motion_in_heightmap()
    );

    assert!(
        !assets
            .properties
            .heightmap_state(barrel)
            .blocks_motion_in_heightmap_no_leaves()
    );

    let (facts, behaviours) = BlockFacts::load_pair(root, registry)?;
    for tag in unresolved {
        assert!(behaviours.tag(tag).is_none(), "{tag} must be omitted");
    }
    let tag = behaviours
        .tag("test:startup_barrels")
        .expect("the startup gameplay tag must bind");
    assert!(tag.contains(behaviours.table.block(barrel)));
    assert!(!tag.contains(behaviours.table.block(stone)));
    assert_eq!(
        behaviours.fluid_tags.supports_lily_pad,
        FluidKind::Water.mask() | FluidKind::FlowingWater.mask(),
    );

    let motion = MotionResources::load_with_facts(root, &assets, &facts)
        .unwrap_or_else(|error| panic!("MotionResources::load_with_facts: {error}"));
    assert!(motion.inside_step_sound.contains("minecraft:barrel"));
    assert!(!motion.inside_step_sound.contains("minecraft:stone"));
    assert!(motion.crystal_sound.is_empty());

    let animate = AnimateTables::load(root, registry)?;
    assert!(animate.terracotta[barrel.0 as usize]);
    assert!(!animate.terracotta[stone.0 as usize]);

    let visibility = models::load_visibility(root, registry)?;
    assert!(visibility.skip_rendering(stone, barrel, Direction::North, false)?);
    assert!(!visibility.skip_rendering(stone, stone, Direction::North, false)?);
    Ok(())
}
