//! Natural textures (`optifine/natural.properties`): block textures that turn and flip, a
//! different way for each block face by its position, so a wall of one block shows no grid.
//! The file lists a texture a line: `<texture> = <value>`, with the value `4` (any of four
//! turns), `2` (0 or 180 degrees), `F` (flips), `4F` or `2F`.
//!
//! A pack's own file replaces OptiFine's list, which applies when the pack has none
//! ([`load`]). Which block faces turn is the game's: this reads the file, and says how a face
//! at a position turns ([`Natural::variant`]) and which of a quad's corners passes its texture
//! coordinates to which ([`Variant::source`]).
//!
//! Written from OptiFine's public documentation (OptiDocs: `natural_textures`, which lists the
//! values and the default list); the choice by position follows OptiFine's own, so a world
//! turns where OptiFine's does.

use sunrise_assets::resource_source::ResourceSource;

use crate::properties::Properties;
use crate::random_textures::{MAX_PROPERTIES, decode};

/// Where a pack's list is.
pub const FILE: &str = "assets/minecraft/optifine/natural.properties";

/// The list OptiFine uses without a pack's own: OptiDocs' default list.
const BUILT_IN: &str = include_str!("../data/natural.properties");

/// How far a texture may turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turns {
    /// Not at all (`F`: it only flips).
    None,
    /// By 0 or 180 degrees (`2`).
    Half,
    /// By 0, 90, 180 or 270 degrees (`4`).
    Quarter,
}

/// What a texture does by position: how far it turns, and whether it flips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Natural {
    pub turns: Turns,
    pub flip: bool,
}

/// One face of a block: its quarter turns (0 to 3) and whether it flips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Variant {
    pub turns: u8,
    pub flip: bool,
}

/// A line of the file: the block sprite (`namespace:path`, `minecraft:block/stone`) and what it
/// does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub sprite: String,
    pub natural: Natural,
}

impl Natural {
    /// A value of the file; `None` for any other (`0`, which OptiDocs lists for "off", included).
    pub fn parse(value: &str) -> Option<Self> {
        let (turns, flip) = match value {
            "4" => (Turns::Quarter, false),
            "2" => (Turns::Half, false),
            "F" => (Turns::None, true),
            "4F" => (Turns::Quarter, true),
            "2F" => (Turns::Half, true),
            _ => return None,
        };
        Some(Self { turns, flip })
    }

    /// What the face of a block at `pos` does, by its direction `face` (0 to 5: down, up, north,
    /// south, west, east). The same every time, whatever the order the faces are drawn in.
    pub fn variant(self, pos: [i32; 3], face: u8) -> Variant {
        let h = position_hash(pos, face);
        let turns = match self.turns {
            Turns::None => 0,
            Turns::Half => (h & 3) as u8 / 2 * 2,
            Turns::Quarter => (h & 3) as u8,
        };
        Variant {
            turns,
            flip: self.flip && h & 4 != 0,
        }
    }
}

impl Variant {
    /// Whether the face is left as it is.
    pub fn is_identity(self) -> bool {
        self.turns == 0 && !self.flip
    }

    /// The corner (0 to 3) of a quad whose texture coordinates `vertex` takes: a turn passes
    /// each corner's coordinates on to the one that many places before it, and a flip passes
    /// them on in the other order.
    pub fn source(self, vertex: usize) -> usize {
        let turns = usize::from(self.turns & 3);
        if self.flip {
            (7 - turns - vertex) % 4
        } else {
            (vertex + turns) % 4
        }
    }
}

/// A 32-bit integer hash (Thomas Wang's).
fn int_hash(i: i32) -> i32 {
    let mut i = (i ^ 61) ^ (i >> 16);
    i = i.wrapping_add(i << 3);
    i ^= i >> 4;
    i = i.wrapping_mul(668_265_261);
    i ^ (i >> 15)
}

/// The number a block face is chosen by: its direction, then x, z and y in turn, each hashed
/// into the last.
fn position_hash(pos: [i32; 3], face: u8) -> i32 {
    let h = int_hash(i32::from(face) + 37);
    let h = int_hash(h.wrapping_add(pos[0]));
    let h = int_hash(h.wrapping_add(pos[2]));
    int_hash(h.wrapping_add(pos[1]))
}

/// The sprite a key names: a bare name is a block sprite, `block/x` and `textures/block/x` are
/// the path under `textures/`, and `namespace:` puts either in that namespace. `None` for a path
/// that leaves the resource root.
fn sprite(key: &str) -> Option<String> {
    let (namespace, path) = key.split_once(':').unwrap_or(("minecraft", key));
    // `Identifier`: an empty namespace is `minecraft`.
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let path = path.strip_suffix(".png").unwrap_or(path);
    let path = path.strip_prefix("textures/").unwrap_or(path);
    let path = if path.contains('/') {
        path.to_owned()
    } else {
        format!("block/{path}")
    };
    crate::paths::valid(&path).then(|| format!("{namespace}:{path}"))
}

/// The entries of the file `text`, in file order. A texture listed twice keeps its last value, and
/// a line whose value is not one of the five is left out, as is a `0`.
pub fn parse(text: &str) -> Vec<Entry> {
    Properties::parse_equals_first(text)
        .entries
        .into_iter()
        .filter_map(|(key, value)| {
            let Some(natural) = Natural::parse(&value) else {
                if value != "0" {
                    log::warn!("Sunrise: natural textures: {key}: unknown value {value}");
                }
                return None;
            };
            Some(Entry {
                sprite: sprite(&key)?,
                natural,
            })
        })
        .collect()
}

/// OptiFine's own list, which applies where the pack has no file ([`load`]).
pub fn built_in() -> Vec<Entry> {
    parse(BUILT_IN)
}

/// The entries `root`'s pack stack gives: its [`FILE`] when it has one (with no entries when that
/// holds none, or can't be read), else [`built_in`].
pub fn load(root: &ResourceSource) -> Vec<Entry> {
    if !root.contains(FILE) {
        return built_in();
    }
    match root.read_limited(FILE, MAX_PROPERTIES) {
        Ok(bytes) => parse(&decode(bytes)),
        Err(e) => {
            log::warn!("Sunrise: natural textures {FILE}: {e}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    fn natural(value: &str) -> Natural {
        Natural::parse(value).unwrap()
    }

    #[test]
    fn the_five_values_and_nothing_else() {
        let n = |turns, flip| Some(Natural { turns, flip });
        assert_eq!(Natural::parse("4"), n(Turns::Quarter, false));
        assert_eq!(Natural::parse("2"), n(Turns::Half, false));
        assert_eq!(Natural::parse("F"), n(Turns::None, true));
        assert_eq!(Natural::parse("4F"), n(Turns::Quarter, true));
        assert_eq!(Natural::parse("2F"), n(Turns::Half, true));
        for other in ["0", "3", "FF", "f", "4f", "", "F4", "8"] {
            assert_eq!(Natural::parse(other), None, "{other:?}");
        }
    }

    #[test]
    fn keys_name_block_sprites() {
        let sprite = |key| sprite(key).unwrap();
        assert_eq!(sprite("stone"), "minecraft:block/stone");
        assert_eq!(sprite("stone.png"), "minecraft:block/stone");
        assert_eq!(sprite("block/stone"), "minecraft:block/stone");
        assert_eq!(sprite("textures/block/stone.png"), "minecraft:block/stone");
        assert_eq!(sprite("mymod:ruby_ore"), "mymod:block/ruby_ore");
        assert_eq!(sprite("mymod:block/ruby_ore"), "mymod:block/ruby_ore");
        assert_eq!(sprite(":stone"), "minecraft:block/stone");
        for bad in ["../x", "block/../../x", "a//b", "", "block/"] {
            assert_eq!(super::sprite(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_file_lists_entries_in_order() {
        let entries = parse(
            "# comment\ngrass_block_side = F\nobsidian=4F\n\nhalf = 2\nbad = 3\nworse = FF\noff = 0\nobsidian = 2F\n",
        );
        let got: Vec<_> = entries
            .iter()
            .map(|e| (e.sprite.as_str(), e.natural))
            .collect();
        // A texture listed twice is one entry, with the later value, where it was first listed.
        assert_eq!(
            got,
            [
                ("minecraft:block/grass_block_side", natural("F")),
                ("minecraft:block/obsidian", natural("2F")),
                ("minecraft:block/half", natural("2")),
            ]
        );
    }

    #[test]
    fn a_later_zero_turns_a_texture_off() {
        assert!(parse("stone = 2F\nstone = 0\n").is_empty());
    }

    #[test]
    fn the_built_in_list_has_every_line() {
        let lines = BUILT_IN
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .count();
        let list = built_in();
        assert_eq!(list.len(), lines, "a line was not read");
        let find = |name: &str| {
            list.iter()
                .find(|e| e.sprite == format!("minecraft:block/{name}"))
                .map(|e| e.natural)
        };
        assert_eq!(find("grass_block_side"), Some(natural("F")));
        assert_eq!(find("obsidian"), Some(natural("4F")));
        assert_eq!(find("jungle_leaves"), Some(natural("2")));
        assert_eq!(find("dirt_path_top"), Some(natural("4")));
        assert_eq!(find("stone"), None);
    }

    #[test]
    fn a_face_is_chosen_by_its_position_and_direction() {
        let quarter_flip = natural("4F");
        let at = |pos, face| quarter_flip.variant(pos, face);
        let v = |turns, flip| Variant { turns, flip };
        // The numbers OptiFine's own choice gives these faces.
        assert_eq!(at([0, 0, 0], 0), v(0, true));
        assert_eq!(at([0, 0, 0], 1), v(2, true));
        assert_eq!(at([1, 0, 0], 1), v(3, false));
        assert_eq!(at([0, 64, 0], 1), v(3, true));
        assert_eq!(at([12, 70, -5], 2), v(0, true));
        assert_eq!(at([-3, 5, 9], 5), v(1, true));
        assert_eq!(at([100, -20, -100], 3), v(0, false));
        assert_eq!(at([i32::MAX, 0, 0], 4), v(3, true));
        // Asking again changes nothing.
        assert_eq!(at([12, 70, -5], 2), at([12, 70, -5], 2));
    }

    #[test]
    fn only_what_the_value_allows_changes() {
        let positions =
            || (-8..8).flat_map(|x| (-4..4).flat_map(move |z| (0..6).map(move |f| ([x, 5, z], f))));
        let seen = |value: &str| -> (HashSet<u8>, HashSet<bool>) {
            let n = natural(value);
            positions()
                .map(|(pos, face)| n.variant(pos, face))
                .map(|v| (v.turns, v.flip))
                .unzip()
        };
        let (turns, flips) = seen("4");
        assert_eq!(
            (turns, flips),
            (HashSet::from([0, 1, 2, 3]), HashSet::from([false]))
        );
        let (turns, flips) = seen("2");
        assert_eq!(
            (turns, flips),
            (HashSet::from([0, 2]), HashSet::from([false]))
        );
        let (turns, flips) = seen("F");
        assert_eq!(
            (turns, flips),
            (HashSet::from([0]), HashSet::from([false, true]))
        );
        let (turns, flips) = seen("2F");
        assert_eq!(
            (turns, flips),
            (HashSet::from([0, 2]), HashSet::from([false, true]))
        );
        let (turns, flips) = seen("4F");
        assert_eq!(
            (turns, flips),
            (HashSet::from([0, 1, 2, 3]), HashSet::from([false, true]))
        );
    }

    #[test]
    fn a_turn_passes_each_corners_coordinates_on() {
        let order = |turns, flip| {
            let v = Variant { turns, flip };
            [0, 1, 2, 3].map(|vertex| v.source(vertex))
        };
        // The corner `vertex` takes the coordinates of `source(vertex)`: a turn of `t` moves them
        // `t` corners back, so `a b c d` on corners 0 to 3 becomes `b c d a` after one turn.
        assert_eq!(order(0, false), [0, 1, 2, 3]);
        assert_eq!(order(1, false), [1, 2, 3, 0]);
        assert_eq!(order(2, false), [2, 3, 0, 1]);
        assert_eq!(order(3, false), [3, 0, 1, 2]);
        // A flip reads them the other way round, and a turn after it shifts that.
        assert_eq!(order(0, true), [3, 2, 1, 0]);
        assert_eq!(order(1, true), [2, 1, 0, 3]);
        assert_eq!(order(2, true), [1, 0, 3, 2]);
        assert_eq!(order(3, true), [0, 3, 2, 1]);
        assert!(Variant::default().is_identity());
        assert!(
            !Variant {
                turns: 0,
                flip: true
            }
            .is_identity()
        );
    }

    fn pack(name: &str, file: Option<&[u8]>) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("sunrise-natural-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        if let Some(bytes) = file {
            let path = root.join(FILE);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        root
    }

    #[test]
    fn a_packs_file_replaces_the_built_in_list() {
        let root = pack("own", Some(b"stone = 2F\nmymod:ruby_ore = 4\n"));
        let entries = load(&ResourceSource::directory(root.clone()));
        let _ = fs::remove_dir_all(root);
        let sprites: Vec<_> = entries.iter().map(|e| e.sprite.as_str()).collect();
        assert_eq!(sprites, ["minecraft:block/stone", "mymod:block/ruby_ore"]);
    }

    #[test]
    fn without_a_file_the_built_in_list_applies() {
        let root = pack("none", None);
        let entries = load(&ResourceSource::directory(root.clone()));
        let _ = fs::remove_dir_all(root);
        assert_eq!(entries, built_in());
        assert!(!entries.is_empty());
    }

    #[test]
    fn an_empty_file_turns_nothing() {
        let root = pack("empty", Some(b"# nothing here\n"));
        let entries = load(&ResourceSource::directory(root.clone()));
        let _ = fs::remove_dir_all(root);
        assert!(entries.is_empty());
    }
}
