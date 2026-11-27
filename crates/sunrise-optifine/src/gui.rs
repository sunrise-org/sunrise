//! Custom GUIs: the rules of `optifine/gui/container/**/*.properties`, which replace the textures
//! of a container's screen (the furnace's, the chest's, the beacon's...) where the container, its
//! biome, height, name and what its block or entity is allow it. A rule names one container and
//! the textures it replaces (`texture`, and `texture.<path>` for any picture under
//! `textures/gui/`); the first rule of the open screen's container whose conditions hold decides.
//! What the open screen belongs to is the client's to say ([`Subject`]): it knows the block, the
//! entity and their names.
//!
//! Written from OptiFine's public documentation of the format
//! (`OptiFineDoc/doc/custom_guis.properties`).

use std::collections::HashSet;

use sunrise_assets::resource_source::ResourceSource;

use crate::color::DYES;
use crate::matchers::{IdList, Name, RangeList};
use crate::paths::{properties_files, resolve};
use crate::properties::Properties;

/// The containers a rule can name (`container`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Anvil,
    Beacon,
    BrewingStand,
    Chest,
    Crafting,
    Dispenser,
    Enchantment,
    Furnace,
    Hopper,
    Horse,
    Villager,
    ShulkerBox,
    /// The creative inventory with its tabs.
    Creative,
    /// The survival inventory.
    Inventory,
}

const CONTAINERS: [(Container, &str); 14] = [
    (Container::Anvil, "anvil"),
    (Container::Beacon, "beacon"),
    (Container::BrewingStand, "brewing_stand"),
    (Container::Chest, "chest"),
    (Container::Crafting, "crafting"),
    (Container::Dispenser, "dispenser"),
    (Container::Enchantment, "enchantment"),
    (Container::Furnace, "furnace"),
    (Container::Hopper, "hopper"),
    (Container::Horse, "horse"),
    (Container::Villager, "villager"),
    (Container::ShulkerBox, "shulker_box"),
    (Container::Creative, "creative"),
    (Container::Inventory, "inventory"),
];

impl Container {
    fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_lowercase();
        CONTAINERS
            .iter()
            .find(|(_, name)| *name == value.as_str())
            .map(|(container, _)| *container)
    }

    /// The texture `texture=` replaces, by its name in the GUI atlas (`gui/container/furnace`
    /// for `textures/gui/container/furnace.png`). The creative inventory has none: its rules
    /// name theirs with `texture.<path>`.
    pub fn default_texture(self) -> Option<&'static str> {
        Some(match self {
            Self::Anvil => "gui/container/anvil",
            Self::Beacon => "gui/container/beacon",
            Self::BrewingStand => "gui/container/brewing_stand",
            Self::Chest => "gui/container/generic_54",
            Self::Crafting => "gui/container/crafting_table",
            Self::Dispenser => "gui/container/dispenser",
            Self::Enchantment => "gui/container/enchanting_table",
            Self::Furnace => "gui/container/furnace",
            Self::Hopper => "gui/container/hopper",
            Self::Horse => "gui/container/horse",
            Self::Villager => "gui/container/villager",
            Self::ShulkerBox => "gui/container/shulker_box",
            Self::Inventory => "gui/container/inventory",
            Self::Creative => return None,
        })
    }
}

/// What `variants` tells apart: a horse screen's mount, a dispenser screen's block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Horse,
    Donkey,
    Mule,
    Llama,
    Dispenser,
    Dropper,
}

const HORSE_VARIANTS: [(Variant, &str); 4] = [
    (Variant::Horse, "horse"),
    (Variant::Donkey, "donkey"),
    (Variant::Mule, "mule"),
    (Variant::Llama, "llama"),
];

const DISPENSER_VARIANTS: [(Variant, &str); 2] = [
    (Variant::Dispenser, "dispenser"),
    (Variant::Dropper, "dropper"),
];

/// The villager professions `professions` names (`minecraft:` optional).
const PROFESSIONS: [&str; 15] = [
    "armorer",
    "butcher",
    "cartographer",
    "cleric",
    "farmer",
    "fisherman",
    "fletcher",
    "leatherworker",
    "librarian",
    "mason",
    "nitwit",
    "none",
    "shepherd",
    "toolsmith",
    "weaponsmith",
];

/// What the open container screen belongs to: the facts the rules that ask more than the
/// container test. The containers that have such facts (the chest, beacon, dispenser, villager,
/// horse and shulker box) match no rule on a screen that gives none (a barrel's, a wandering
/// trader's): OptiFine's rules are for the block or the entity itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Detail<'a> {
    /// Nothing more to ask (the anvil, the furnace, the inventories...).
    None,
    /// A chest block's screen: next to another chest, a trapped chest, an ender chest, and
    /// whether it is Christmas (24 to 26 December).
    Chest {
        large: bool,
        trapped: bool,
        christmas: bool,
        ender: bool,
    },
    /// A beacon's pyramid levels.
    Beacon { levels: i32 },
    /// A dispenser block's screen, or a dropper's.
    Dispenser { dropper: bool },
    /// A villager's profession (`minecraft:farmer`) and trading level.
    Villager { profession: &'a str, level: i32 },
    /// A mount's screen: what it is (none for a zombie or skeleton horse) and the colour of a
    /// llama's carpet (`light_gray`).
    Horse {
        variant: Option<Variant>,
        carpet: Option<&'a str>,
    },
    /// A shulker box's colour (`light_gray`), none for an undyed one.
    Shulker { color: Option<&'a str> },
}

/// The open container screen as the rules see it: the container, where its block or entity is,
/// the name it shows and what it is ([`Detail`]).
#[derive(Clone, Copy, Debug)]
pub struct Subject<'a> {
    pub container: Container,
    /// The biome there (`minecraft:plains`).
    pub biome: &'a str,
    pub y: i32,
    /// The title the screen of a block shows, or the custom name of the entity; none when it
    /// has no name to give.
    pub name: Option<&'a str>,
    pub detail: Detail<'a>,
}

/// A replacement picture: its resource path, and the name it is packed under in the GUI atlas
/// ([`sprite_name`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub path: String,
    pub sprite: String,
}

/// The name the picture `assets/<namespace>/<path>.png` is packed under in the GUI atlas: its
/// path without the extension, behind the namespace unless that is `minecraft`.
pub fn sprite_name(path: &str) -> Option<String> {
    let (namespace, rest) = path.strip_prefix("assets/")?.split_once('/')?;
    let rest = rest.strip_suffix(".png")?;
    Some(if namespace == "minecraft" {
        rest.to_owned()
    } else {
        format!("{namespace}:{rest}")
    })
}

/// A profession of `professions`, with the levels it is limited to.
#[derive(Clone, Debug)]
struct Profession {
    id: &'static str,
    levels: Option<RangeList>,
}

/// One `.properties` file of `optifine/gui/container/`.
#[derive(Clone, Debug)]
pub struct Rule {
    /// The resource path of the file.
    pub file: String,
    pub container: Container,
    /// Each GUI texture replaced (`gui/container/furnace`) and the picture that replaces it.
    textures: Vec<(String, Picture)>,
    name: Option<Name>,
    biomes: Option<IdList>,
    heights: Option<RangeList>,
    large: Option<bool>,
    trapped: Option<bool>,
    christmas: Option<bool>,
    ender: Option<bool>,
    levels: Option<RangeList>,
    professions: Option<Vec<Profession>>,
    variants: Option<Vec<Variant>>,
    /// The dye colours of a shulker box or a llama's carpet.
    colors: Option<Vec<&'static str>>,
}

/// `true` or `false` in any case; anything else leaves the condition out.
fn flag(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The picture a value of `texture` or `texture.<path>` names: a path as the other OptiFine
/// files write them ([`resolve`]), with the extension `.png` when it has none.
fn picture(file: &str, value: &str) -> Result<Picture, String> {
    let bad = || format!("bad texture path {value}");
    let mut path = resolve(file, value).ok_or_else(bad)?;
    if !path.ends_with(".png") {
        path.push_str(".png");
    }
    let sprite = sprite_name(&path).ok_or_else(bad)?;
    Ok(Picture { path, sprite })
}

/// The GUI texture `texture.<key>` replaces: `key` is relative to `textures/gui`.
fn sub_texture(key: &str) -> String {
    let key = key.replace('\\', "/");
    let key = key.trim_start_matches('/');
    format!("gui/{}", key.strip_suffix(".png").unwrap_or(key))
}

/// Sets what `default` is replaced by; a later entry for the same texture wins.
fn replace(textures: &mut Vec<(String, Picture)>, default: String, picture: Picture) {
    match textures
        .iter()
        .position(|(replaced, _)| *replaced == default)
    {
        Some(i) => textures[i].1 = picture,
        None => textures.push((default, picture)),
    }
}

/// `variants` of a `container`: words of its table. An empty list leaves the condition out.
fn variants(container: Container, value: &str) -> Result<Option<Vec<Variant>>, String> {
    let table: &[(Variant, &str)] = match container {
        Container::Horse => &HORSE_VARIANTS,
        Container::Dispenser => &DISPENSER_VARIANTS,
        _ => &[],
    };
    let list = value
        .split_whitespace()
        .map(|word| {
            let word = word.to_lowercase();
            table
                .iter()
                .find(|(_, name)| *name == word.as_str())
                .map(|(variant, _)| *variant)
                .ok_or_else(|| format!("unknown variant {word}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((!list.is_empty()).then_some(list))
}

/// `colors`: dye colour names. An empty list leaves the condition out.
fn colors(value: &str) -> Result<Option<Vec<&'static str>>, String> {
    let list = value
        .split_whitespace()
        .map(|word| {
            let word = word.to_lowercase();
            DYES.iter()
                .find(|dye| **dye == word.as_str())
                .copied()
                .ok_or_else(|| format!("unknown colour {word}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((!list.is_empty()).then_some(list))
}

/// The levels of `profession:1,3-5`: the words that are no number or range of them (or are
/// negative) count for none.
fn level_list(value: &str) -> RangeList {
    RangeList(
        value
            .split(',')
            .filter_map(RangeList::parse)
            .flat_map(|ranges| ranges.0)
            .filter(|&(low, _)| low >= 0)
            .collect(),
    )
}

/// `professions`: words, each `profession` or `profession:levels`; what follows the last colon is
/// the levels when it is empty or starts with a digit. A profession OptiFine does not know makes
/// the list invalid; an empty list leaves the condition out.
fn professions(value: &str) -> Result<Option<Vec<Profession>>, String> {
    let mut list = Vec::new();
    for word in value.split_whitespace() {
        let (name, levels) = match word.rfind(':') {
            Some(i)
                if word[i + 1..].is_empty()
                    || word[i + 1..].starts_with(|c: char| c.is_ascii_digit()) =>
            {
                (&word[..i], Some(&word[i + 1..]))
            }
            _ => (word, None),
        };
        let name = name.to_lowercase();
        let bare = name.strip_prefix("minecraft:").unwrap_or(&name);
        let id = PROFESSIONS
            .iter()
            .find(|profession| **profession == bare)
            .copied()
            .ok_or_else(|| format!("unknown profession {word}"))?;
        list.push(Profession {
            id,
            levels: levels.map(level_list),
        });
    }
    Ok((!list.is_empty()).then_some(list))
}

impl Rule {
    /// The rule of the file at resource path `file`, or why OptiFine drops it: no or an unknown
    /// container, no texture, a condition it cannot read.
    pub fn parse(file: &str, props: &Properties) -> Result<Self, String> {
        let container = props.get("container").ok_or("no container")?;
        let container =
            Container::parse(container).ok_or_else(|| format!("unknown container {container}"))?;
        let mut textures = Vec::new();
        if let (Some(value), Some(default)) = (props.get("texture"), container.default_texture()) {
            replace(&mut textures, default.to_owned(), picture(file, value)?);
        }
        for (key, value) in props.with_prefix("texture.") {
            replace(&mut textures, sub_texture(key), picture(file, value)?);
        }
        if textures.is_empty() {
            return Err("no texture".into());
        }
        let ranges = |key: &str| {
            props
                .get(key)
                .map(|value| RangeList::parse(value).ok_or_else(|| format!("bad {key} {value}")))
                .transpose()
        };
        Ok(Self {
            file: file.to_owned(),
            container,
            textures,
            name: props
                .get("name")
                .map(|value| Name::parse(value).ok_or_else(|| format!("bad name {value}")))
                .transpose()?,
            biomes: props.get("biomes").map(IdList::parse),
            heights: ranges("heights")?,
            large: flag(props.get("large")),
            trapped: flag(props.get("trapped")),
            christmas: flag(props.get("christmas")),
            ender: flag(props.get("ender")),
            levels: ranges("levels")?,
            professions: props
                .get("professions")
                .map(professions)
                .transpose()?
                .flatten(),
            variants: props
                .get("variants")
                .map(|value| variants(container, value))
                .transpose()?
                .flatten(),
            colors: props.get("colors").map(colors).transpose()?.flatten(),
        })
    }

    /// Whether the rule holds for the screen: the container, the biome, height and name, then
    /// what the screen's block or entity is.
    pub fn matches(&self, subject: &Subject) -> bool {
        if self.container != subject.container {
            return false;
        }
        if self
            .biomes
            .as_ref()
            .is_some_and(|biomes| !biomes.matches(subject.biome))
        {
            return false;
        }
        if self
            .heights
            .as_ref()
            .is_some_and(|heights| !heights.contains(subject.y))
        {
            return false;
        }
        if self
            .name
            .as_ref()
            .is_some_and(|name| !subject.name.is_some_and(|shown| name.matches(shown)))
        {
            return false;
        }
        let wants = |wanted: Option<bool>, is: bool| wanted.is_none_or(|wanted| wanted == is);
        let color_in =
            |list: &[&str], color: Option<&str>| color.is_some_and(|color| list.contains(&color));
        match (subject.container, subject.detail) {
            (Container::Beacon, Detail::Beacon { levels }) => self
                .levels
                .as_ref()
                .is_none_or(|wanted| wanted.contains(levels)),
            (
                Container::Chest,
                Detail::Chest {
                    large,
                    trapped,
                    christmas,
                    ender,
                },
            ) => {
                wants(self.large, large)
                    && wants(self.trapped, trapped)
                    && wants(self.christmas, christmas)
                    && wants(self.ender, ender)
            }
            (Container::Dispenser, Detail::Dispenser { dropper }) => {
                let variant = if dropper {
                    Variant::Dropper
                } else {
                    Variant::Dispenser
                };
                self.variants
                    .as_ref()
                    .is_none_or(|wanted| wanted.contains(&variant))
            }
            (Container::Villager, Detail::Villager { profession, level }) => {
                let profession = profession.strip_prefix("minecraft:").unwrap_or(profession);
                self.professions.as_ref().is_none_or(|wanted| {
                    wanted.iter().any(|p| {
                        p.id == profession && p.levels.as_ref().is_none_or(|l| l.contains(level))
                    })
                })
            }
            (Container::Horse, Detail::Horse { variant, carpet }) => {
                self.variants
                    .as_ref()
                    .is_none_or(|wanted| variant.is_some_and(|variant| wanted.contains(&variant)))
                    && (variant != Some(Variant::Llama)
                        || self
                            .colors
                            .as_ref()
                            .is_none_or(|wanted| color_in(wanted, carpet)))
            }
            (Container::ShulkerBox, Detail::Shulker { color }) => self
                .colors
                .as_ref()
                .is_none_or(|wanted| color_in(wanted, color)),
            (
                Container::Beacon
                | Container::Chest
                | Container::Dispenser
                | Container::Villager
                | Container::Horse
                | Container::ShulkerBox,
                _,
            ) => false,
            _ => true,
        }
    }
}

/// The rules of a pack's `optifine/gui/container/` folders, in the order of their paths.
#[derive(Clone, Debug, Default)]
pub struct CustomGuis {
    rules: Vec<Rule>,
}

impl CustomGuis {
    /// Every rule `root` holds. Broken files are logged and left out.
    pub fn load(root: &ResourceSource) -> Self {
        Self::parse(
            properties_files(root, "gui/container")
                .into_iter()
                .filter_map(|path| {
                    let bytes = root
                        .read(&path)
                        .map_err(|e| log::warn!("Sunrise: custom GUIs {path}: {e}"))
                        .ok()?;
                    // Java's `Properties.load(InputStream)` reads ISO-8859-1; UTF-8 when it is valid.
                    let text = String::from_utf8(bytes).unwrap_or_else(|e| {
                        e.into_bytes().iter().map(|&b| char::from(b)).collect()
                    });
                    Some((path, text))
                }),
        )
    }

    /// The rules of files given as (resource path, text), in that order.
    fn parse(files: impl IntoIterator<Item = (String, String)>) -> Self {
        let rules = files
            .into_iter()
            .filter_map(|(path, text)| {
                Rule::parse(&path, &Properties::parse(&text))
                    .map_err(|e| log::warn!("Sunrise: custom GUIs {path}: {e}"))
                    .ok()
            })
            .collect();
        Self { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The pictures the rules draw, each once, with the GUI texture it first replaces (the size
    /// its picture is drawn at).
    pub fn pictures(&self) -> Vec<(&Picture, &str)> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (default, picture) in self.rules.iter().flat_map(|rule| &rule.textures) {
            if seen.insert(picture.sprite.as_str()) {
                out.push((picture, default.as_str()));
            }
        }
        out
    }

    /// The sprite to draw in place of the GUI texture `default` (`gui/container/furnace`) on the
    /// screen: the first rule of its container whose conditions hold decides, and answers only
    /// for the textures it replaces (a later rule is not tried for the others).
    pub fn texture(&self, subject: &Subject, default: &str) -> Option<&str> {
        let rule = self.rules.iter().find(|rule| rule.matches(subject))?;
        rule.textures
            .iter()
            .find(|(replaced, _)| replaced == default)
            .map(|(_, picture)| picture.sprite.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = "assets/minecraft/optifine/gui/container";

    fn guis(files: &[(&str, &str)]) -> CustomGuis {
        CustomGuis::parse(
            files
                .iter()
                .map(|(name, text)| (format!("{DIR}/{name}.properties"), (*text).to_owned())),
        )
    }

    fn rule(text: &str) -> Result<Rule, String> {
        Rule::parse(&format!("{DIR}/sub/x.properties"), &Properties::parse(text))
    }

    fn subject(container: Container, detail: Detail<'static>) -> Subject<'static> {
        Subject {
            container,
            biome: "minecraft:plains",
            y: 64,
            name: None,
            detail,
        }
    }

    fn chest(large: bool, trapped: bool) -> Detail<'static> {
        Detail::Chest {
            large,
            trapped,
            christmas: false,
            ender: false,
        }
    }

    #[test]
    fn textures_are_resolved_beside_the_file_or_under_the_pack() {
        let r = rule(
            "container=furnace\ntexture=dark\ntexture.container/blast_furnace=./b/blast.png\n\
             texture.\\\\container/smoker.png=~/gui/smoker\ntexture.container/hopper=mymod:textures/hopper.png\n",
        )
        .unwrap();
        let got: Vec<(&str, &str, &str)> = r
            .textures
            .iter()
            .map(|(default, p)| (default.as_str(), p.path.as_str(), p.sprite.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (
                    "gui/container/furnace",
                    "assets/minecraft/optifine/gui/container/sub/dark.png",
                    "optifine/gui/container/sub/dark"
                ),
                (
                    "gui/container/blast_furnace",
                    "assets/minecraft/optifine/gui/container/sub/b/blast.png",
                    "optifine/gui/container/sub/b/blast"
                ),
                (
                    "gui/container/smoker",
                    "assets/minecraft/optifine/gui/smoker.png",
                    "optifine/gui/smoker"
                ),
                (
                    "gui/container/hopper",
                    "assets/mymod/textures/hopper.png",
                    "mymod:textures/hopper"
                ),
            ]
        );
    }

    #[test]
    fn the_creative_inventory_needs_textures_of_its_own() {
        assert_eq!(
            rule("container=creative\ntexture=x").err().as_deref(),
            Some("no texture")
        );
        let r = rule(
            "container=Creative\ntexture.container/creative_inventory/tab_items=items\n\
             texture.container/creative_inventory/tab_inventory=inventory\n",
        )
        .unwrap();
        assert_eq!(r.container, Container::Creative);
        assert_eq!(r.textures.len(), 2);
    }

    #[test]
    fn a_later_texture_key_replaces_an_earlier_one() {
        let r = rule("container=hopper\ntexture=a\ntexture.container/hopper=b\n").unwrap();
        assert_eq!(r.textures.len(), 1);
        assert_eq!(r.textures[0].1.sprite, "optifine/gui/container/sub/b");
    }

    #[test]
    fn files_optifine_drops_are_dropped() {
        for (text, why) in [
            ("texture=a", "no container"),
            (
                "container=smithing\ntexture=a",
                "a container it does not have",
            ),
            ("container=furnace", "no texture"),
            (
                "container=villager\ntexture=a\nprofessions=wizard",
                "a profession it lacks",
            ),
            (
                "container=villager\ntexture=a\nprofessions=farmer mymod:farmer",
                "another namespace",
            ),
            (
                "container=shulker_box\ntexture=a\ncolors=silver",
                "the old dye name",
            ),
            (
                "container=chest\ntexture=a\nvariants=horse",
                "variants of a chest",
            ),
            (
                "container=horse\ntexture=a\nvariants=camel",
                "a mount it lacks",
            ),
            (
                "container=chest\ntexture=a\nname=regex:(",
                "a regex that does not compile",
            ),
            (
                "container=chest\ntexture=a\nheights=high",
                "heights that are no range",
            ),
            (
                "container=beacon\ntexture=a\nlevels=1-x",
                "levels that are no range",
            ),
            ("container=chest\ntexture=../../a", "a path out of the pack"),
        ] {
            assert!(rule(text).is_err(), "{why}");
        }
        let files = guis(&[
            ("a", "container=chest\ntexture=a"),
            ("b", "container=wizard\ntexture=b"),
            ("c", "container=hopper\ntexture=c"),
        ]);
        let order: Vec<&str> = files.rules.iter().map(|r| r.file.as_str()).collect();
        assert_eq!(
            order,
            [format!("{DIR}/a.properties"), format!("{DIR}/c.properties")]
        );
    }

    #[test]
    fn chests_ask_for_their_kind() {
        let big = rule("container=chest\ntexture=a\nlarge=true\ntrapped=false").unwrap();
        assert!(big.matches(&subject(Container::Chest, chest(true, false))));
        assert!(!big.matches(&subject(Container::Chest, chest(false, false))));
        assert!(!big.matches(&subject(Container::Chest, chest(true, true))));
        let ender = rule("container=chest\ntexture=a\nender=true").unwrap();
        let ender_chest = Detail::Chest {
            large: false,
            trapped: false,
            christmas: false,
            ender: true,
        };
        assert!(ender.matches(&subject(Container::Chest, ender_chest)));
        assert!(!ender.matches(&subject(Container::Chest, chest(false, false))));
        let christmas = rule("container=chest\ntexture=a\nchristmas=true").unwrap();
        let december = Detail::Chest {
            large: false,
            trapped: false,
            christmas: true,
            ender: false,
        };
        assert!(christmas.matches(&subject(Container::Chest, december)));
        assert!(!christmas.matches(&subject(Container::Chest, chest(false, false))));
        // A barrel, a minecart or a server's menu has no chest to ask about.
        assert!(!big.matches(&subject(Container::Chest, Detail::None)));
        let any = rule("container=chest\ntexture=a").unwrap();
        assert!(!any.matches(&subject(Container::Chest, Detail::None)));
        assert!(any.matches(&subject(Container::Chest, chest(true, true))));
    }

    #[test]
    fn biomes_heights_and_names_narrow_a_rule() {
        let r = rule(
            "container=furnace\ntexture=a\nbiomes=desert badlands\nheights=(-64)-0 70-\n\
             name=ipattern:*smelter*",
        )
        .unwrap();
        let at = |biome, y, name| Subject {
            biome,
            y,
            name,
            ..subject(Container::Furnace, Detail::None)
        };
        assert!(r.matches(&at("minecraft:desert", 80, Some("Iron SMELTER"))));
        assert!(r.matches(&at("minecraft:badlands", -10, Some("smelter"))));
        assert!(!r.matches(&at("minecraft:plains", 80, Some("smelter"))));
        assert!(!r.matches(&at("minecraft:desert", 30, Some("smelter"))));
        assert!(!r.matches(&at("minecraft:desert", 80, Some("Furnace"))));
        assert!(
            !r.matches(&at("minecraft:desert", 80, None)),
            "a name rule needs a name"
        );
        let not_ocean = rule("container=hopper\ntexture=a\nbiomes=!ocean !deep_ocean").unwrap();
        let hopper = subject(Container::Hopper, Detail::None);
        assert!(not_ocean.matches(&hopper));
        assert!(!not_ocean.matches(&Subject {
            biome: "minecraft:deep_ocean",
            ..hopper
        }));
    }

    #[test]
    fn a_rule_is_for_its_container_only() {
        let r = rule("container=hopper\ntexture=a").unwrap();
        assert!(r.matches(&subject(Container::Hopper, Detail::None)));
        assert!(!r.matches(&subject(Container::Furnace, Detail::None)));
    }

    #[test]
    fn beacons_dispensers_and_shulker_boxes() {
        let beacon = rule("container=beacon\ntexture=a\nlevels=3-4").unwrap();
        let levels = |levels| subject(Container::Beacon, Detail::Beacon { levels });
        assert!(beacon.matches(&levels(3)) && beacon.matches(&levels(4)));
        assert!(!beacon.matches(&levels(2)) && !beacon.matches(&levels(0)));
        assert!(!beacon.matches(&subject(Container::Beacon, Detail::None)));

        let dropper = rule("container=dispenser\ntexture=a\nvariants=dropper").unwrap();
        let block = |dropper| subject(Container::Dispenser, Detail::Dispenser { dropper });
        assert!(dropper.matches(&block(true)));
        assert!(!dropper.matches(&block(false)));
        let either = rule("container=dispenser\ntexture=a").unwrap();
        assert!(either.matches(&block(true)) && either.matches(&block(false)));

        let reds = rule("container=shulker_box\ntexture=a\ncolors=red Light_Gray").unwrap();
        let shulker = |color| subject(Container::ShulkerBox, Detail::Shulker { color });
        assert!(reds.matches(&shulker(Some("red"))));
        assert!(reds.matches(&shulker(Some("light_gray"))));
        assert!(!reds.matches(&shulker(Some("blue"))));
        assert!(
            !reds.matches(&shulker(None)),
            "an undyed box has no colour to list"
        );
        let any = rule("container=shulker_box\ntexture=a").unwrap();
        assert!(any.matches(&shulker(None)));
    }

    #[test]
    fn villagers_by_profession_and_level() {
        let r = rule(
            "container=villager\ntexture=a\nprofessions=farmer:1,3-4 minecraft:mason librarian:",
        )
        .unwrap();
        let at = |profession, level| {
            subject(Container::Villager, Detail::Villager { profession, level })
        };
        assert!(r.matches(&at("minecraft:farmer", 1)));
        assert!(r.matches(&at("farmer", 4)));
        assert!(!r.matches(&at("minecraft:farmer", 2)));
        assert!(
            r.matches(&at("minecraft:mason", 5)),
            "no levels means every level"
        );
        assert!(
            !r.matches(&at("minecraft:librarian", 1)),
            "an empty list of levels"
        );
        assert!(!r.matches(&at("minecraft:cleric", 1)));
        // A wandering trader is no villager.
        assert!(!r.matches(&subject(Container::Villager, Detail::None)));
        let any = rule("container=villager\ntexture=a").unwrap();
        assert!(any.matches(&at("minecraft:nitwit", 1)));
        assert!(!any.matches(&subject(Container::Villager, Detail::None)));
        // Levels count for what they name: bad words are skipped, negative ones too.
        let levels = level_list("2,x,5-3,-1,7");
        assert_eq!(levels.0, [(2, 2), (5, 3), (7, 7)]);
    }

    #[test]
    fn mounts_by_variant_and_llama_carpet() {
        let r = rule("container=horse\ntexture=a\nvariants=mule donkey").unwrap();
        let mount = |variant| {
            subject(
                Container::Horse,
                Detail::Horse {
                    variant,
                    carpet: None,
                },
            )
        };
        assert!(r.matches(&mount(Some(Variant::Mule))));
        assert!(r.matches(&mount(Some(Variant::Donkey))));
        assert!(!r.matches(&mount(Some(Variant::Horse))));
        assert!(!r.matches(&mount(None)), "a skeleton horse is none of them");
        let llama = rule("container=horse\ntexture=a\nvariants=llama\ncolors=white").unwrap();
        let carpet = |carpet| {
            subject(
                Container::Horse,
                Detail::Horse {
                    variant: Some(Variant::Llama),
                    carpet,
                },
            )
        };
        assert!(llama.matches(&carpet(Some("white"))));
        assert!(!llama.matches(&carpet(Some("red"))));
        assert!(
            !llama.matches(&carpet(None)),
            "a llama without a carpet has no colour"
        );
        // Colours count for llamas only.
        let colored = rule("container=horse\ntexture=a\ncolors=white").unwrap();
        assert!(colored.matches(&mount(Some(Variant::Horse))));
    }

    #[test]
    fn the_first_rule_that_matches_decides() {
        let files = guis(&[
            (
                "a_desert",
                "container=chest\ntexture=desert\nbiomes=desert\ntexture.container/shulker_box=box",
            ),
            ("b_large", "container=chest\ntexture=large\nlarge=true"),
            (
                "c_any",
                "container=chest\ntexture=plain\ntexture.container/hopper=other",
            ),
            ("d_hopper", "container=hopper\ntexture=hopper"),
        ]);
        let normal = chest(false, false);
        let plains = |detail| subject(Container::Chest, detail);
        let desert = Subject {
            biome: "minecraft:desert",
            ..plains(normal)
        };
        let generic = "gui/container/generic_54";
        assert_eq!(
            files.texture(&desert, generic),
            Some("optifine/gui/container/desert")
        );
        assert_eq!(
            files.texture(&plains(chest(true, false)), generic),
            Some("optifine/gui/container/large")
        );
        assert_eq!(
            files.texture(&plains(normal), generic),
            Some("optifine/gui/container/plain")
        );
        assert_eq!(
            files.texture(&plains(normal), "gui/container/hopper"),
            Some("optifine/gui/container/other")
        );
        // The desert rule decides for a desert chest, though it has nothing for the hopper's
        // texture: the rule after it is not tried.
        assert_eq!(files.texture(&desert, "gui/container/hopper"), None);
        assert_eq!(
            files.texture(&desert, "gui/container/shulker_box"),
            Some("optifine/gui/container/box")
        );
        assert_eq!(
            files.texture(
                &subject(Container::Furnace, Detail::None),
                "gui/container/furnace"
            ),
            None
        );
        assert!(CustomGuis::default().is_empty());
        assert!(!files.is_empty());
    }

    #[test]
    fn each_picture_is_listed_once_with_the_texture_it_replaces() {
        let files = guis(&[
            (
                "a",
                "container=furnace\ntexture=dark\ntexture.container/smoker=dark",
            ),
            (
                "b",
                "container=hopper\ntexture=./dark\ntexture.container/dispenser=light",
            ),
        ]);
        let got: Vec<(&str, &str)> = files
            .pictures()
            .into_iter()
            .map(|(picture, default)| (picture.sprite.as_str(), default))
            .collect();
        assert_eq!(
            got,
            [
                ("optifine/gui/container/dark", "gui/container/furnace"),
                ("optifine/gui/container/light", "gui/container/dispenser"),
            ]
        );
    }

    #[test]
    fn a_picture_outside_minecraft_is_named_by_its_namespace() {
        assert_eq!(
            sprite_name("assets/mymod/optifine/gui/container/a.png").as_deref(),
            Some("mymod:optifine/gui/container/a")
        );
        assert_eq!(
            sprite_name("assets/minecraft/textures/gui/a.png").as_deref(),
            Some("textures/gui/a")
        );
        assert_eq!(sprite_name("assets/minecraft/textures/gui/a.jpg"), None);
        assert_eq!(sprite_name("textures/gui/a.png"), None);
    }
}
