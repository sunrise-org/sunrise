//! Better Grass and Better Snow at load (`optifine/bettergrass.properties`, read by
//! `sunrise_optifine::bettergrass`). Load turns the rules into the full block faces Better
//! Grass draws over a side (one list for each cube and side), the dirt path's sides with its top
//! over them, and a kind for each block state; the mesher asks them for a block of a kind Better
//! Grass changes, and for the plants Better Snow draws a snow layer under.

use std::collections::BTreeMap;

use sunrise_assets::{model::CuboidModel, model::Direction, resolve::ModelLibrary};
use sunrise_core::position::BlockPos;
use sunrise_optifine::bettergrass::{self, BetterGrass, Cube, Surface};
use sunrise_world::registry::{BlockRegistry, BlockStateId};

use crate::{
    Result,
    graphics::BetterGrassMode,
    models::{StateModels, catalog::BakedFace, catalog::bake_resolved},
    resource_source::ResourceSource,
    sprites::Sprites,
};

/// The Better Grass and Better Snow options the mesher runs with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Better {
    pub grass: BetterGrassMode,
    pub snow: bool,
}

/// A snow layer (`minecraft:snow`).
pub const SNOW_LAYER: u8 = 1;
/// A snow block.
pub const SNOW_BLOCK: u8 = 2;
/// A block Better Snow draws a snow layer under, where the blocks around it let it
/// (`bettergrass::takes_snow`).
pub const TAKES_SNOW: u8 = 4;
/// An upside-down stair or slab, which has a flat top for snow to lie on.
pub const UPSIDE_DOWN: u8 = 8;

/// What Better Grass and Better Snow know of a block state.
#[derive(Clone, Copy, Debug, Default)]
pub struct StateKind {
    /// The block, if Better Grass changes its sides.
    pub surface: Option<Surface>,
    /// `SNOW_LAYER`, `SNOW_BLOCK`, `TAKES_SNOW` and `UPSIDE_DOWN`.
    pub flags: u8,
}

/// Faces for the four sides a block looks toward: north, south, west and east.
pub type Sides = [Vec<BakedFace>; 4];

/// The index of a side in [`Sides`]; `None` for up and down.
pub fn side_index(side: Direction) -> Option<usize> {
    match side {
        Direction::North => Some(0),
        Direction::South => Some(1),
        Direction::West => Some(2),
        Direction::East => Some(3),
        Direction::Down | Direction::Up => None,
    }
}

/// The path of the model the cubes are baked from.
const CUBE_MODEL: &str = "minecraft:better_grass/cube";

/// The rules, as the mesher needs them.
#[derive(Default)]
pub struct BetterAssets {
    pub rules: BetterGrass,
    /// By block state id; empty when nothing was loaded (a server).
    kinds: Vec<StateKind>,
    /// By `Cube as usize`.
    cubes: Vec<Sides>,
    /// The sprite layer of a dirt path's side, and the path's sides with its top over them.
    path: Option<(u32, Sides)>,
    /// The state of a single snow layer, which Better Snow draws.
    snow: Option<u32>,
}

impl BetterAssets {
    /// Assets of the given kinds, cubes and snow state, for tests of the mesher.
    #[cfg(test)]
    pub(crate) fn for_tests(kinds: Vec<StateKind>, cubes: Vec<Sides>, snow: Option<u32>) -> Self {
        Self {
            kinds,
            cubes,
            snow,
            ..Self::default()
        }
    }

    /// What is known of the state `state`.
    pub fn kind(&self, state: u32) -> StateKind {
        self.kinds.get(state as usize).copied().unwrap_or_default()
    }

    /// The faces `cube` draws over `side` (an index of [`Sides`]).
    pub fn cube(&self, cube: Cube, side: usize) -> &[BakedFace] {
        self.cubes
            .get(cube as usize)
            .map_or(&[], |sides| sides[side].as_slice())
    }

    /// The sprite layer a dirt path's side shows, and the faces its top draws over `side`.
    pub fn path(&self, side: usize) -> Option<(u32, &[BakedFace])> {
        let (layer, sides) = self.path.as_ref()?;
        Some((*layer, sides[side].as_slice()))
    }

    /// The state of the snow layer Better Snow draws under a plant.
    pub fn snow_state(&self) -> Option<u32> {
        self.snow
    }

    /// Reads the pack's rules and prepares what they draw. Sprites they name are loaded into
    /// `sprites`; `catalog` gives the dirt path's own faces.
    pub fn load(
        root: &ResourceSource,
        registry: &BlockRegistry,
        sprites: &mut Sprites,
        catalog: &StateModels,
    ) -> Result<Self> {
        // A texture naming nothing loadable keeps its default, as OptiFine does.
        let rules = bettergrass::load(root, |sprite| sprites.try_insert(sprite).is_ok());
        let kinds = (0..registry.len())
            .map(|id| match registry.state(BlockStateId(id as u32)) {
                Some(state) => kind(&state.name, |key, value| {
                    state.properties.get(key).is_some_and(|v| v == value)
                }),
                None => StateKind::default(),
            })
            .collect();
        let cubes = Cube::ALL
            .into_iter()
            .map(|cube| bake_cube(sprites, &rules, cube))
            .collect::<Result<Vec<_>>>()?;
        let path = path_sides(sprites, &rules, registry, catalog)?;
        let snow = registry.default_state("minecraft:snow").map(|id| id.0);
        Ok(Self {
            rules,
            kinds,
            cubes,
            path,
            snow,
        })
    }
}

/// The kind of a block state: `name` with its namespace, `property_is(key, value)` whether the
/// state has the property at the value.
fn kind(name: &str, property_is: impl Fn(&str, &str) -> bool) -> StateKind {
    let Some(block) = name.strip_prefix("minecraft:") else {
        return StateKind::default();
    };
    let mut flags = 0;
    if block == "snow" {
        flags |= SNOW_LAYER;
    }
    if block == "snow_block" {
        flags |= SNOW_BLOCK;
    }
    if bettergrass::takes_snow(block, &property_is) {
        flags |= TAKES_SNOW;
    }
    if bettergrass::holds_snow(block, false, &property_is) {
        flags |= UPSIDE_DOWN;
    }
    StateKind {
        surface: Surface::of(block),
        flags,
    }
}

/// The four sides of a block, each one full face of `sprite` that culls with its neighbour and
/// has the tint index `tint`, baked as a model would be.
fn bake_sides(sprites: &mut Sprites, sprite: &str, tint: i32) -> Result<Sides> {
    let face =
        |side: &str| serde_json::json!({ "texture": "#all", "cullface": side, "tintindex": tint });
    let model = serde_json::json!({
        "textures": { "all": sprite },
        "elements": [{
            "from": [0, 0, 0],
            "to": [16, 16, 16],
            "faces": {
                "north": face("north"),
                "south": face("south"),
                "west": face("west"),
                "east": face("east"),
            },
        }],
    });
    let library = ModelLibrary::new(BTreeMap::from([(
        CUBE_MODEL.to_owned(),
        CuboidModel::from_json(&model.to_string())?,
    )]))?;
    let baked = bake_resolved(sprites, &library.resolve(CUBE_MODEL)?, [0, 0, 0], false)?;
    let mut sides = Sides::default();
    for face in baked.faces {
        if let Some(side) = side_index(face.quad.facing) {
            sides[side].push(face);
        }
    }
    Ok(sides)
}

/// What `cube` draws over a side: its picture, which the biome tints for the grass (and the
/// podzol, which has no tint to give); with `grass.multilayer` the grass' side picture first.
fn bake_cube(sprites: &mut Sprites, rules: &BetterGrass, cube: Cube) -> Result<Sides> {
    let tint = if cube.tinted() { 0 } else { -1 };
    let mut sides = bake_sides(sprites, rules.textures.of(cube), tint)?;
    if cube == Cube::Grass && rules.multilayer {
        let mut under = bake_sides(sprites, &rules.textures.grass_side, -1)?;
        for (under, top) in under.iter_mut().zip(&mut sides) {
            under.append(top);
        }
        return Ok(under);
    }
    Ok(sides)
}

/// A dirt path's side faces that show `texture.dirt_path_side`, as they are but with
/// `texture.dirt_path`; with the layer they replace. `None` where the path has no such face.
fn path_sides(
    sprites: &mut Sprites,
    rules: &BetterGrass,
    registry: &BlockRegistry,
    catalog: &StateModels,
) -> Result<Option<(u32, Sides)>> {
    let Some(side_layer) = sprites.model_layer(&rules.textures.dirt_path_side) else {
        return Ok(None);
    };
    let Some(path) = registry.default_state("minecraft:dirt_path") else {
        return Ok(None);
    };
    let top = sprites.insert(&rules.textures.dirt_path)?;
    let mut found = Vec::new();
    if catalog
        .models_at(path, BlockPos::new(0, 0, 0), &mut found)
        .is_err()
    {
        return Ok(None);
    }
    let mut sides = Sides::default();
    for face in found.iter().flat_map(|model| &model.faces) {
        let Some(side) = face.quad.cull.and_then(side_index) else {
            continue;
        };
        if face.layer != side_layer {
            continue;
        }
        let mut face = face.clone();
        face.layer = top;
        face.translucent = sprites.translucent[top as usize];
        face.opaque = sprites.region_opaque(top, &face.quad.uv);
        sides[side].push(face);
    }
    Ok(sides
        .iter()
        .any(|faces| !faces.is_empty())
        .then_some((side_layer, sides)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is(key: &'static str, value: &'static str) -> impl Fn(&str, &str) -> bool {
        move |k, v| k == key && v == value
    }

    #[test]
    fn kinds_name_the_blocks_and_what_snow_does_with_them() {
        let grass = kind("minecraft:grass_block", is("snowy", "true"));
        assert_eq!(grass.surface, Some(Surface::Grass));
        assert_eq!(grass.flags, 0);
        assert_eq!(kind("minecraft:snow", |_, _| false).flags, SNOW_LAYER);
        assert_eq!(kind("minecraft:snow_block", |_, _| false).flags, SNOW_BLOCK);
        assert_eq!(kind("minecraft:poppy", |_, _| false).flags, TAKES_SNOW);
        let stairs = kind("minecraft:oak_stairs", is("half", "top"));
        assert_eq!(stairs.flags, TAKES_SNOW | UPSIDE_DOWN);
        let stairs = kind("minecraft:oak_stairs", is("half", "bottom"));
        assert_eq!(stairs.flags, 0);
        assert_eq!(
            kind("minecraft:dirt_path", |_, _| false).surface,
            Some(Surface::DirtPath)
        );
        assert_eq!(
            kind("minecraft:dirt", |_, _| false).surface,
            Some(Surface::Dirt)
        );
    }

    #[test]
    fn another_namespace_is_nothing() {
        let other = kind("modded:grass_block", |_, _| false);
        assert!(other.surface.is_none());
        assert_eq!(other.flags, 0);
        assert!(kind("modded:poppy", |_, _| false).flags == 0);
    }

    #[test]
    fn horizontal_sides_have_an_index_and_up_and_down_none() {
        let sides = [
            Direction::North,
            Direction::South,
            Direction::West,
            Direction::East,
        ];
        for (i, side) in sides.into_iter().enumerate() {
            assert_eq!(side_index(side), Some(i));
        }
        assert_eq!(side_index(Direction::Up), None);
        assert_eq!(side_index(Direction::Down), None);
    }

    #[test]
    fn nothing_loaded_draws_nothing() {
        let assets = BetterAssets::default();
        assert_eq!(assets.kind(0).flags, 0);
        assert!(assets.kind(9).surface.is_none());
        assert!(assets.cube(Cube::Grass, 0).is_empty());
        assert!(assets.path(0).is_none());
        assert!(assets.snow_state().is_none());
    }
}
