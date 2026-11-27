//! Better Grass (`optifine/bettergrass.properties`) and Better Snow.
//!
//! Better Grass draws the top texture on the side of a grass block (also mycelium, podzol,
//! the two nylium blocks, and the dirt beside a dirt path) where the blocks around it say so: in
//! the fast mode always, in the fancy mode only where the block below that side is of the same
//! kind. Snow on top of such a block shows the snow texture on its side instead. The file picks
//! which blocks take part and which textures are used ([`BetterGrass::parse`]); which neighbour
//! each test looks at is the game's ([`Probe`]), so these rules hold no game types.
//!
//! Better Snow has no file: a snow layer is drawn under a plant, fence or the like that has snow
//! beside it ([`takes_snow`], [`holds_snow`]).
//!
//! Written from OptiFine's public documentation (OptiDocs: `better_grass`).

use sunrise_assets::resource_source::ResourceSource;

use crate::properties::Properties;
use crate::random_textures::{MAX_PROPERTIES, decode};

/// Where a pack's file is.
pub const FILE: &str = "assets/minecraft/optifine/bettergrass.properties";

/// The sprites a cube is drawn with (`texture.*`), as sprite names (`minecraft:block/...`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Textures {
    /// `texture.grass`: the top of a grass block, which the biome tints.
    pub grass: String,
    /// `texture.grass_side`: drawn under the top with `grass.multilayer`.
    pub grass_side: String,
    /// `texture.dirt_path`.
    pub dirt_path: String,
    /// `texture.dirt_path_side`: the side of a dirt path that `texture.dirt_path` replaces.
    pub dirt_path_side: String,
    /// `texture.mycelium`.
    pub mycelium: String,
    /// `texture.podzol`.
    pub podzol: String,
    /// `texture.crimson_nylium`.
    pub crimson_nylium: String,
    /// `texture.warped_nylium`.
    pub warped_nylium: String,
    /// `texture.snow`: the side of a block with snow on top.
    pub snow: String,
}

impl Default for Textures {
    fn default() -> Self {
        let block = |name: &str| format!("minecraft:block/{name}");
        Self {
            grass: block("grass_block_top"),
            grass_side: block("grass_block_side"),
            dirt_path: block("dirt_path_top"),
            dirt_path_side: block("dirt_path_side"),
            mycelium: block("mycelium_top"),
            podzol: block("podzol_top"),
            crimson_nylium: block("crimson_nylium"),
            warped_nylium: block("warped_nylium"),
            snow: block("snow"),
        }
    }
}

impl Textures {
    /// The sprite of `cube`; for `Cube::Grass` the top, whose side is [`Self::grass_side`].
    pub fn of(&self, cube: Cube) -> &str {
        match cube {
            Cube::Grass => &self.grass,
            Cube::Mycelium => &self.mycelium,
            Cube::Podzol => &self.podzol,
            Cube::Snow => &self.snow,
            Cube::DirtPath => &self.dirt_path,
            Cube::CrimsonNylium => &self.crimson_nylium,
            Cube::WarpedNylium => &self.warped_nylium,
        }
    }

    /// Every sprite a cube or a replaced side may show, for the game to load.
    pub fn all(&self) -> [&str; 9] {
        [
            &self.grass,
            &self.grass_side,
            &self.dirt_path,
            &self.dirt_path_side,
            &self.mycelium,
            &self.podzol,
            &self.crimson_nylium,
            &self.warped_nylium,
            &self.snow,
        ]
    }
}

/// The blocks Better Grass changes the sides of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Grass,
    Mycelium,
    Podzol,
    CrimsonNylium,
    WarpedNylium,
    /// A dirt block: it shows a dirt path's top where a path lies above it and beside it.
    Dirt,
    /// A dirt path: its own side shows its top.
    DirtPath,
}

impl Surface {
    /// The surface a block (its name without a namespace) is.
    pub fn of(block: &str) -> Option<Self> {
        Some(match block {
            "grass_block" => Self::Grass,
            "mycelium" => Self::Mycelium,
            "podzol" => Self::Podzol,
            "crimson_nylium" => Self::CrimsonNylium,
            "warped_nylium" => Self::WarpedNylium,
            "dirt" => Self::Dirt,
            "dirt_path" => Self::DirtPath,
            _ => return None,
        })
    }
}

/// Which full block face Better Grass draws over a side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cube {
    Grass,
    Mycelium,
    Podzol,
    Snow,
    DirtPath,
    CrimsonNylium,
    WarpedNylium,
}

impl Cube {
    pub const ALL: [Cube; 7] = [
        Cube::Grass,
        Cube::Mycelium,
        Cube::Podzol,
        Cube::Snow,
        Cube::DirtPath,
        Cube::CrimsonNylium,
        Cube::WarpedNylium,
    ];

    /// The biome tints a cube's picture (tint index 0): the grass top; the podzol's asks too,
    /// and gets none, as podzol has no tint.
    pub fn tinted(self) -> bool {
        matches!(self, Cube::Grass | Cube::Podzol)
    }
}

/// What a rule asks about the blocks around the one it is drawing a side of. The side looks
/// toward "beside"; "below beside" is the block under that one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// Snow, a layer or a block, is on top of the block.
    SnowAbove,
    /// A snow layer is beside it.
    SnowBeside,
    /// The block below beside is the same kind of block as this one.
    SameBelowBeside,
    /// A dirt path is on top of the block.
    PathAbove,
    /// A dirt path is beside it.
    PathBeside,
}

/// The settings of `bettergrass.properties`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BetterGrass {
    pub grass: bool,
    pub dirt_path: bool,
    pub mycelium: bool,
    pub podzol: bool,
    pub crimson_nylium: bool,
    pub warped_nylium: bool,
    /// `grass.snow`: with snow on top.
    pub grass_snow: bool,
    /// `mycelium.snow`.
    pub mycelium_snow: bool,
    /// `podzol.snow`.
    pub podzol_snow: bool,
    /// `grass.multilayer`: the grass' side picture stays under the top's.
    pub multilayer: bool,
    pub textures: Textures,
}

impl Default for BetterGrass {
    /// The file's defaults: everything on but `grass.multilayer`.
    fn default() -> Self {
        Self {
            grass: true,
            dirt_path: true,
            mycelium: true,
            podzol: true,
            crimson_nylium: true,
            warped_nylium: true,
            grass_snow: true,
            mycelium_snow: true,
            podzol_snow: true,
            multilayer: false,
            textures: Textures::default(),
        }
    }
}

impl BetterGrass {
    /// Reads a file. `exists` says whether a sprite (`minecraft:block/...`) has a texture: a
    /// `texture.*` naming one that has none keeps its default, as in OptiFine.
    pub fn parse(text: &str, mut exists: impl FnMut(&str) -> bool) -> Self {
        let props = Properties::parse(text);
        let flag = |key: &str, default: bool| match props.get(key) {
            Some(value) => value.trim().eq_ignore_ascii_case("true"),
            None => default,
        };
        let mut sprite = |key: &str, default: String| {
            props
                .get(key)
                .and_then(sprite_name)
                .filter(|name| exists(name.as_str()))
                .unwrap_or(default)
        };
        let d = Self::default();
        let t = d.textures.clone();
        Self {
            grass: flag("grass", d.grass),
            dirt_path: flag("dirt_path", d.dirt_path),
            mycelium: flag("mycelium", d.mycelium),
            podzol: flag("podzol", d.podzol),
            crimson_nylium: flag("crimson_nylium", d.crimson_nylium),
            warped_nylium: flag("warped_nylium", d.warped_nylium),
            grass_snow: flag("grass.snow", d.grass_snow),
            mycelium_snow: flag("mycelium.snow", d.mycelium_snow),
            podzol_snow: flag("podzol.snow", d.podzol_snow),
            multilayer: flag("grass.multilayer", d.multilayer),
            textures: Textures {
                grass: sprite("texture.grass", t.grass),
                grass_side: sprite("texture.grass_side", t.grass_side),
                dirt_path: sprite("texture.dirt_path", t.dirt_path),
                dirt_path_side: sprite("texture.dirt_path_side", t.dirt_path_side),
                mycelium: sprite("texture.mycelium", t.mycelium),
                podzol: sprite("texture.podzol", t.podzol),
                crimson_nylium: sprite("texture.crimson_nylium", t.crimson_nylium),
                warped_nylium: sprite("texture.warped_nylium", t.warped_nylium),
                snow: sprite("texture.snow", t.snow),
            },
        }
    }

    /// What a side of a block of `surface` shows: the cube Better Grass draws over it, or `None`
    /// for the block's own faces. `fancy` is the fancy mode; `probe` answers the questions the
    /// rule has of the blocks around it.
    pub fn cube(
        &self,
        fancy: bool,
        surface: Surface,
        probe: impl Fn(Probe) -> bool,
    ) -> Option<Cube> {
        let (on, on_snow, cube) = match surface {
            Surface::Grass => (self.grass, self.grass_snow, Cube::Grass),
            Surface::Mycelium => (self.mycelium, self.mycelium_snow, Cube::Mycelium),
            Surface::Podzol => (self.podzol, self.podzol_snow, Cube::Podzol),
            Surface::CrimsonNylium => (self.crimson_nylium, false, Cube::CrimsonNylium),
            Surface::WarpedNylium => (self.warped_nylium, false, Cube::WarpedNylium),
            // Dirt under a path, beside another: the path's top, in either mode.
            Surface::Dirt => {
                return (self.dirt_path && probe(Probe::PathAbove) && probe(Probe::PathBeside))
                    .then_some(Cube::DirtPath);
            }
            Surface::DirtPath => return None,
        };
        let nylium = matches!(cube, Cube::CrimsonNylium | Cube::WarpedNylium);
        if !nylium && probe(Probe::SnowAbove) {
            // With snow on top the side shows snow, and the block's own kind stays out of it.
            return (on_snow && (!fancy || probe(Probe::SnowBeside))).then_some(Cube::Snow);
        }
        (on && (!fancy || probe(Probe::SameBelowBeside))).then_some(cube)
    }

    /// Whether the side of a dirt path that shows `texture.dirt_path_side` shows
    /// `texture.dirt_path` instead: always in the fast mode; in the fancy mode where the block
    /// below that side is a dirt path too.
    pub fn path_side(&self, fancy: bool, probe: impl Fn(Probe) -> bool) -> bool {
        self.dirt_path && (!fancy || probe(Probe::SameBelowBeside))
    }
}

/// The sprite a `texture.*` value names: a path in the `minecraft` namespace unless it says
/// another, without `textures/` or `.png`.
fn sprite_name(value: &str) -> Option<String> {
    let value = value.trim();
    let (namespace, path) = value.split_once(':').unwrap_or(("minecraft", value));
    let path = path.trim_start_matches('/');
    let path = path.strip_prefix("textures/").unwrap_or(path);
    let path = path.strip_suffix(".png").unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    Some(format!("{namespace}:{path}"))
}

/// The pack stack's file, else the defaults. `exists` is as for [`BetterGrass::parse`].
pub fn load(root: &ResourceSource, exists: impl FnMut(&str) -> bool) -> BetterGrass {
    if !root.contains(FILE) {
        return BetterGrass::default();
    }
    match root.read_limited(FILE, MAX_PROPERTIES) {
        Ok(bytes) => BetterGrass::parse(&decode(bytes), exists),
        Err(e) => {
            log::warn!("Sunrise: better grass {FILE}: {e}");
            BetterGrass::default()
        }
    }
}

/// Blocks Better Snow draws a snow layer under, whatever their state: the plants, flowers,
/// saplings, potted plants, and a few small blocks.
const SNOW_BLOCKS: &[&str] = &[
    "acacia_sapling",
    "allium",
    "azure_bluet",
    "birch_sapling",
    "blue_orchid",
    "brown_mushroom",
    "cherry_sapling",
    "closed_eyeblossom",
    "cornflower",
    "dandelion",
    "dark_oak_sapling",
    "fern",
    "flower_pot",
    "glass_pane",
    "golden_dandelion",
    "hopper",
    "jungle_sapling",
    "ladder",
    "large_fern",
    "lever",
    "lilac",
    "lily_of_the_valley",
    "mangrove_propagule",
    "oak_sapling",
    "open_eyeblossom",
    "orange_tulip",
    "oxeye_daisy",
    "pale_oak_sapling",
    "peony",
    "pink_tulip",
    "pitcher_crop",
    "pitcher_plant",
    "poplar_sapling",
    "poppy",
    "red_mushroom",
    "red_tulip",
    "redstone_torch",
    "redstone_wall_torch",
    "rose_bush",
    "short_grass",
    "small_dripleaf",
    "spruce_sapling",
    "sugar_cane",
    "sunflower",
    "tall_grass",
    "tall_seagrass",
    "torchflower",
    "turtle_egg",
    "vine",
    "white_tulip",
    "wither_rose",
];

/// A stair or a slab turned upside down: its top is flat, so snow lies on it.
fn upside_down(block: &str, property_is: &impl Fn(&str, &str) -> bool) -> bool {
    (block.ends_with("_stairs") && property_is("half", "top"))
        || (block.ends_with("_slab") && property_is("type", "top"))
}

/// Whether Better Snow draws a snow layer under a block (a plant, a fence, a wall, a pane, a
/// torch, a lever, a button on a wall, an upside-down stair or slab, a flower pot, ...), when
/// the blocks around it let it ([`holds_snow`]). A block that is a full opaque cube takes none:
/// that is the game's to say. `block` is the name without a namespace; `property_is(key,
/// value)` says whether the block's state has that property at that value.
pub fn takes_snow(block: &str, property_is: impl Fn(&str, &str) -> bool) -> bool {
    if block.ends_with("_stairs") || block.ends_with("_slab") {
        return upside_down(block, &property_is);
    }
    if block.ends_with("_button") {
        return !property_is("face", "floor");
    }
    block.starts_with("potted_")
        || [
            "_fence",
            "_fence_gate",
            "_wall",
            "_stained_glass_pane",
            "_bars",
        ]
        .iter()
        .any(|suffix| block.ends_with(suffix))
        || SNOW_BLOCKS.binary_search(&block).is_ok()
}

/// Whether the block under a block that [`takes_snow`] has a top snow can lie on beside it:
/// any full opaque cube (`opaque_cube`, the game's to say), or an upside-down stair or slab.
pub fn holds_snow(
    block: &str,
    opaque_cube: bool,
    property_is: impl Fn(&str, &str) -> bool,
) -> bool {
    opaque_cube || upside_down(block, &property_is)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str, _: &str) -> bool {
        false
    }

    #[test]
    fn no_file_is_the_defaults() {
        let g = BetterGrass::parse("", |_| true);
        assert_eq!(g, BetterGrass::default());
        assert!(g.grass && g.dirt_path && g.mycelium && g.podzol);
        assert!(g.crimson_nylium && g.warped_nylium);
        assert!(g.grass_snow && g.mycelium_snow && g.podzol_snow);
        assert!(!g.multilayer);
        assert_eq!(g.textures.grass, "minecraft:block/grass_block_top");
        assert_eq!(g.textures.grass_side, "minecraft:block/grass_block_side");
        assert_eq!(g.textures.dirt_path_side, "minecraft:block/dirt_path_side");
        assert_eq!(g.textures.snow, "minecraft:block/snow");
    }

    #[test]
    fn flags_read_true_and_anything_else_is_false() {
        let g = BetterGrass::parse(
            "grass=false\nmycelium = TRUE\npodzol=no\ndirt_path=false\n\
             grass.snow=false\ngrass.multilayer=true\ncrimson_nylium=false\n",
            |_| true,
        );
        assert!(!g.grass);
        assert!(g.mycelium);
        assert!(!g.podzol);
        assert!(!g.dirt_path);
        assert!(!g.grass_snow);
        assert!(g.multilayer);
        assert!(!g.crimson_nylium);
        assert!(
            g.warped_nylium,
            "a key the file leaves out keeps its default"
        );
    }

    #[test]
    fn textures_name_sprites_and_fall_back_where_there_is_none() {
        let text = "texture.grass=block/my_grass\n\
                    texture.snow=minecraft:textures/block/my_snow.png\n\
                    texture.podzol=block/missing\n\
                    texture.mycelium=other:block/mycelium\n\
                    texture.grass_side=\n";
        let g = BetterGrass::parse(text, |name| name != "minecraft:block/missing");
        assert_eq!(g.textures.grass, "minecraft:block/my_grass");
        assert_eq!(g.textures.snow, "minecraft:block/my_snow");
        assert_eq!(g.textures.podzol, "minecraft:block/podzol_top");
        assert_eq!(g.textures.mycelium, "other:block/mycelium");
        assert_eq!(g.textures.grass_side, "minecraft:block/grass_block_side");
        assert_eq!(g.textures.of(Cube::Grass), "minecraft:block/my_grass");
        assert_eq!(
            g.textures.of(Cube::DirtPath),
            "minecraft:block/dirt_path_top"
        );
        assert_eq!(g.textures.all().len(), 9);
    }

    #[test]
    fn surfaces_are_the_blocks_better_grass_changes() {
        assert_eq!(Surface::of("grass_block"), Some(Surface::Grass));
        assert_eq!(Surface::of("dirt_path"), Some(Surface::DirtPath));
        assert_eq!(Surface::of("dirt"), Some(Surface::Dirt));
        assert_eq!(Surface::of("warped_nylium"), Some(Surface::WarpedNylium));
        for other in ["stone", "grass", "coarse_dirt", "rooted_dirt", "snow"] {
            assert_eq!(Surface::of(other), None, "{other}");
        }
    }

    #[test]
    fn fast_always_shows_the_top_and_fancy_asks_the_block_below_beside() {
        let g = BetterGrass::default();
        let same = |same: bool| move |p: Probe| p == Probe::SameBelowBeside && same;
        assert_eq!(
            g.cube(false, Surface::Grass, same(false)),
            Some(Cube::Grass)
        );
        assert_eq!(g.cube(true, Surface::Grass, same(false)), None);
        assert_eq!(g.cube(true, Surface::Grass, same(true)), Some(Cube::Grass));
        assert_eq!(
            g.cube(true, Surface::Mycelium, same(true)),
            Some(Cube::Mycelium)
        );
        assert_eq!(
            g.cube(true, Surface::Podzol, same(true)),
            Some(Cube::Podzol)
        );
        assert_eq!(
            g.cube(false, Surface::CrimsonNylium, same(false)),
            Some(Cube::CrimsonNylium)
        );
        assert_eq!(
            g.cube(true, Surface::WarpedNylium, same(true)),
            Some(Cube::WarpedNylium)
        );
    }

    #[test]
    fn a_block_a_rule_turns_off_keeps_its_own_faces() {
        let g = BetterGrass {
            grass: false,
            ..BetterGrass::default()
        };
        assert_eq!(g.cube(false, Surface::Grass, |_| true), None);
        assert!(g.cube(false, Surface::Mycelium, |_| false).is_some());
    }

    #[test]
    fn snow_on_top_shows_the_snow_cube_and_not_the_blocks_own() {
        let g = BetterGrass::default();
        let snow = |above: bool, beside: bool| {
            move |p: Probe| match p {
                Probe::SnowAbove => above,
                Probe::SnowBeside => beside,
                Probe::SameBelowBeside => true,
                _ => false,
            }
        };
        assert_eq!(
            g.cube(false, Surface::Grass, snow(true, false)),
            Some(Cube::Snow)
        );
        // Fancy wants a snow layer beside, whatever the block below is.
        assert_eq!(g.cube(true, Surface::Grass, snow(true, false)), None);
        assert_eq!(
            g.cube(true, Surface::Grass, snow(true, true)),
            Some(Cube::Snow)
        );
        assert_eq!(
            g.cube(true, Surface::Grass, snow(false, true)),
            Some(Cube::Grass)
        );
        // Nylium has no snowy side.
        assert_eq!(
            g.cube(false, Surface::CrimsonNylium, snow(true, true)),
            Some(Cube::CrimsonNylium)
        );
        let off = BetterGrass {
            mycelium_snow: false,
            ..BetterGrass::default()
        };
        assert_eq!(off.cube(false, Surface::Mycelium, snow(true, true)), None);
        assert_eq!(
            off.cube(false, Surface::Podzol, snow(true, true)),
            Some(Cube::Snow)
        );
    }

    #[test]
    fn dirt_shows_a_paths_top_where_a_path_lies_above_and_beside() {
        let g = BetterGrass::default();
        let path = |above: bool, beside: bool| {
            move |p: Probe| match p {
                Probe::PathAbove => above,
                Probe::PathBeside => beside,
                _ => false,
            }
        };
        assert_eq!(
            g.cube(false, Surface::Dirt, path(true, true)),
            Some(Cube::DirtPath)
        );
        assert_eq!(
            g.cube(true, Surface::Dirt, path(true, true)),
            Some(Cube::DirtPath)
        );
        assert_eq!(g.cube(false, Surface::Dirt, path(true, false)), None);
        assert_eq!(g.cube(false, Surface::Dirt, path(false, true)), None);
        let off = BetterGrass {
            dirt_path: false,
            ..BetterGrass::default()
        };
        assert_eq!(off.cube(false, Surface::Dirt, path(true, true)), None);
    }

    #[test]
    fn a_path_side_shows_the_top_always_in_fast_and_over_a_path_in_fancy() {
        let g = BetterGrass::default();
        assert!(g.path_side(false, |_| false));
        assert!(!g.path_side(true, |_| false));
        assert!(g.path_side(true, |p| p == Probe::SameBelowBeside));
        let off = BetterGrass {
            dirt_path: false,
            ..BetterGrass::default()
        };
        assert!(!off.path_side(false, |_| true));
        assert_eq!(g.cube(false, Surface::DirtPath, |_| true), None);
    }

    #[test]
    fn the_snow_table_is_sorted_for_binary_search() {
        assert!(SNOW_BLOCKS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn plants_fences_and_the_like_take_snow() {
        for block in [
            "short_grass",
            "tall_grass",
            "poppy",
            "oak_sapling",
            "potted_oak_sapling",
            "potted_cactus",
            "flower_pot",
            "oak_fence",
            "nether_brick_fence",
            "birch_fence_gate",
            "cobblestone_wall",
            "glass_pane",
            "red_stained_glass_pane",
            "iron_bars",
            "waxed_oxidized_copper_bars",
            "redstone_torch",
            "redstone_wall_torch",
            "lever",
            "ladder",
            "hopper",
            "vine",
            "sugar_cane",
            "turtle_egg",
            "golden_dandelion",
            "pitcher_crop",
            "small_dripleaf",
            "tall_seagrass",
            "mangrove_propagule",
        ] {
            assert!(takes_snow(block, none), "{block}");
        }
    }

    #[test]
    fn blocks_that_are_not_such_things_take_none() {
        for block in [
            "stone",
            "grass_block",
            "snow",
            "torch",
            "bamboo_sapling",
            "oak_planks",
            "oak_door",
            "cactus",
            "kelp",
            "oak_wall_sign",
            "stripped_oak_log",
        ] {
            assert!(!takes_snow(block, none), "{block}");
        }
    }

    #[test]
    fn stairs_slabs_and_buttons_ask_their_state() {
        let is =
            |key: &'static str, value: &'static str| move |k: &str, v: &str| k == key && v == value;
        assert!(takes_snow("oak_stairs", is("half", "top")));
        assert!(!takes_snow("oak_stairs", is("half", "bottom")));
        assert!(takes_snow("stone_slab", is("type", "top")));
        assert!(!takes_snow("stone_slab", is("type", "double")));
        assert!(!takes_snow("stone_slab", none));
        assert!(takes_snow("stone_button", is("face", "wall")));
        assert!(takes_snow("stone_button", is("face", "ceiling")));
        assert!(!takes_snow("stone_button", is("face", "floor")));
    }

    #[test]
    fn what_holds_snow_is_a_full_cube_or_an_upside_down_stair_or_slab() {
        let is =
            |key: &'static str, value: &'static str| move |k: &str, v: &str| k == key && v == value;
        assert!(holds_snow("stone", true, none));
        assert!(!holds_snow("glass", false, none));
        assert!(holds_snow("oak_stairs", false, is("half", "top")));
        assert!(!holds_snow("oak_stairs", false, is("half", "bottom")));
        assert!(holds_snow("stone_slab", false, is("type", "top")));
        assert!(!holds_snow("stone_slab", false, is("type", "bottom")));
    }
}
