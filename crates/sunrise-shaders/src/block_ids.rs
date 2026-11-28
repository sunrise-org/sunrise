//! `block.properties` (spec 4.1): `block.<id>=<block> <block> ...`, each block
//! `[namespace:]name` with optional state filters `:<property>=<v1>,<v2>`. Values in one filter
//! are alternatives; separate filters must all match. Names the game does not know are simply
//! never asked for. A word `%<tag>` (tier E) names every block of a block tag, with the same
//! filters; a rule naming the block wins over one naming its tag. `item.properties` and
//! `entity.properties` read the same words, `%<tag>` naming an item or entity type tag (D-9).
//!
//! The same file's `layer.<layer>=<block> ...` lines (4.2) move block states to another terrain
//! layer ([`BlockLayers`]): their words are read by the same parser, and they also take `%tag`.

use crate::Properties;
use std::collections::{HashMap, HashSet};

/// One word's state filters: `(property, values)`, each matching one of its values.
type Filters = Vec<(String, Vec<String>)>;

/// A tag's members by its full name (`minecraft:logs`), as the world's tags hold them.
pub type TagMembers = HashMap<String, HashSet<String>>;

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    id: i32,
    /// A full name, `minecraft:stone`, or a tag's for `%tag` (`minecraft:logs`).
    name: String,
    tag: bool,
    filters: Filters,
}

/// The ids `block.properties` gives block states.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockIds {
    rules: Vec<Rule>,
    /// Lines skipped and why.
    pub notes: Vec<String>,
}

/// A block word, `[namespace:]name[:<property>=<v1>,<v2>...]...`, as its full name and filters;
/// `None` for a word with more than two name parts.
fn block_word(word: &str) -> Option<(String, Filters)> {
    let (names, filters): (Vec<&str>, Vec<&str>) = word.split(':').partition(|p| !p.contains('='));
    let name = match names[..] {
        [name] => format!("minecraft:{name}"),
        [namespace, name] => format!("{namespace}:{name}"),
        _ => return None,
    };
    let filters = filters
        .iter()
        .filter_map(|f| f.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.split(',').map(str::to_owned).collect()))
        .collect();
    Some((name, filters))
}

/// A rule's word, `[%]<block word>`: whether it names a tag, then [`block_word`]'s name and
/// filters.
fn rule_word(word: &str) -> Option<(bool, String, Filters)> {
    let (tag, block) = match word.strip_prefix('%') {
        Some(tag) => (true, tag),
        None => (false, word),
    };
    let (name, filters) = block_word(block)?;
    Some((tag, name, filters))
}

/// How a matching rule ranks (the least wins): a rule naming the block before one naming its
/// tag (4.1), then the rule with the most filters, then the first in the file (`index`).
fn rank(tag: bool, filters: &Filters, index: usize) -> (bool, std::cmp::Reverse<usize>, usize) {
    (tag, std::cmp::Reverse(filters.len()), index)
}

/// Whether a state with `properties` passes every filter.
fn filters_match<'a>(filters: &Filters, properties: &impl Fn(&str) -> Option<&'a str>) -> bool {
    filters
        .iter()
        .all(|(k, vs)| properties(k).is_some_and(|v| vs.iter().any(|x| x == v)))
}

/// The rules of `subjects` (index, name, whether a tag) by the name they reach: a name's own,
/// and each member `tags` lists under a tag. A tag `tags` lacks reaches nothing.
fn by_name<'a>(
    subjects: impl Iterator<Item = (usize, &'a str, bool)>,
    tags: &'a TagMembers,
) -> HashMap<&'a str, Vec<usize>> {
    let mut out: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, subject, tag) in subjects {
        if !tag {
            out.entry(subject).or_default().push(i);
        } else if let Some(members) = tags.get(subject) {
            for member in members {
                out.entry(member.as_str()).or_default().push(i);
            }
        }
    }
    out
}

impl BlockIds {
    pub fn parse(properties: &Properties) -> Self {
        Self::parse_kind(properties, "block")
    }

    /// The same for `<kind>.<id>=` lines: `entity` in `entity.properties`, `item` in
    /// `item.properties` (4.1); names there carry no state filters.
    pub fn parse_kind(properties: &Properties, kind: &str) -> Self {
        let mut out = Self::default();
        let prefix = format!("{kind}.");
        for e in &properties.entries {
            let Some(id) = e.key.strip_prefix(prefix.as_str()) else {
                continue;
            };
            let Ok(id) = id.parse::<i32>() else {
                // `layer.*` (4.2) and other keys are not ids.
                continue;
            };
            for word in e.value.split_whitespace() {
                let Some((tag, name, filters)) = rule_word(word) else {
                    out.notes
                        .push(format!("line {}: not a block name: {word}", e.line));
                    continue;
                };
                out.rules.push(Rule {
                    id,
                    name,
                    tag,
                    filters,
                });
            }
        }
        out
    }

    /// Each rule's index, the name it names (a block's, or a tag's for `%tag`) and whether that
    /// is a tag, in file order: what a caller indexes the rules of a block by.
    pub fn subjects(&self) -> impl Iterator<Item = (usize, &str, bool)> {
        self.rules
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.name.as_str(), r.tag))
    }

    /// Whether a rule names a tag: only then do the ids follow the world's tags.
    pub fn names_tags(&self) -> bool {
        self.rules.iter().any(|r| r.tag)
    }

    /// The rules of [`Self::subjects`] by the block name they reach, `tags` giving each tag's
    /// members: what [`Self::pick`] takes for a state of that block.
    pub fn by_name<'a>(&'a self, tags: &'a TagMembers) -> HashMap<&'a str, Vec<usize>> {
        by_name(self.subjects(), tags)
    }

    /// Of the rules `candidates` (indices from [`Self::subjects`] that reach the block), the id
    /// of the state with `properties`: a rule naming the block wins over one naming its tag
    /// (4.1), then the rule with the most filters that all match (one name belongs to one id; a
    /// filtered entry is the more specific one), then the first in the file.
    pub fn pick<'p>(
        &self,
        candidates: impl IntoIterator<Item = usize>,
        properties: impl Fn(&str) -> Option<&'p str>,
    ) -> Option<i32> {
        candidates
            .into_iter()
            .filter_map(|i| Some((i, self.rules.get(i)?)))
            .filter(|(_, r)| filters_match(&r.filters, &properties))
            .min_by_key(|&(i, r)| rank(r.tag, &r.filters, i))
            .map(|(_, r)| r.id)
    }

    /// [`Self::pick`] among every rule for block `name` (`minecraft:stone`), `in_tag` telling
    /// whether the block is in a tag.
    pub fn id<'a>(
        &self,
        name: &str,
        properties: impl Fn(&str) -> Option<&'a str>,
        in_tag: impl Fn(&str) -> bool,
    ) -> Option<i32> {
        self.pick(candidates(self.subjects(), name, in_tag), properties)
    }

    /// Every name without state filters and its id: what `entity.properties` and
    /// `item.properties` hold. A name's own entry wins over a tag's (4.1), then the first line
    /// naming it; a tag `tags` lacks gives nothing.
    pub fn plain(&self, tags: &TagMembers) -> HashMap<String, i32> {
        let mut out = HashMap::new();
        let unfiltered = || self.rules.iter().filter(|r| r.filters.is_empty());
        for r in unfiltered().filter(|r| !r.tag) {
            out.entry(r.name.clone()).or_insert(r.id);
        }
        for r in unfiltered().filter(|r| r.tag) {
            for member in tags.get(&r.name).into_iter().flatten() {
                out.entry(member.clone()).or_insert(r.id);
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// The indices of `subjects` that reach block `name`: its own rules, and the rules of the tags
/// `in_tag` says it is in.
fn candidates<'a>(
    subjects: impl Iterator<Item = (usize, &'a str, bool)>,
    name: &'a str,
    in_tag: impl Fn(&str) -> bool,
) -> impl Iterator<Item = usize> {
    subjects
        .filter(move |&(_, subject, tag)| {
            if tag {
                in_tag(subject)
            } else {
                subject == name
            }
        })
        .map(|(i, ..)| i)
}

/// A terrain layer `layer.<layer>` names (4.2). `layer.cutout_mipped` is [`RenderLayer::Cutout`]:
/// the terrain has one alpha-tested pass (the mesher's solid quads without `OPAQUE_LAYER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderLayer {
    /// No alpha test, no blending.
    Solid,
    /// Alpha test.
    Cutout,
    /// Blending, drawn with the translucent terrain (`gbuffers_water`).
    Translucent,
}

impl RenderLayer {
    /// The layer of `layer.<name>`, or `None` for a name 4.2 doesn't list.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "solid" => Some(Self::Solid),
            "cutout" | "cutout_mipped" => Some(Self::Cutout),
            "translucent" => Some(Self::Translucent),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct LayerRule {
    layer: RenderLayer,
    /// A block's full name, or a tag's for `%tag` (`minecraft:leaves`).
    name: String,
    tag: bool,
    filters: Filters,
    /// The line and the word that wrote it, for notes.
    line: u32,
    word: String,
}

/// The `layer.*` rule a state matched (`BlockLayers::pick`): its layer, and the line and the
/// word that wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerMatch<'a> {
    pub layer: RenderLayer,
    pub line: u32,
    pub word: &'a str,
}

/// The terrain layers `block.properties`' `layer.*` lines (4.2) give block states.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockLayers {
    /// In file order.
    rules: Vec<LayerRule>,
    /// Lines and words skipped and why.
    pub notes: Vec<String>,
}

impl BlockLayers {
    pub fn parse(properties: &Properties) -> Self {
        let mut out = Self::default();
        for e in &properties.entries {
            let Some(name) = e.key.strip_prefix("layer.") else {
                continue;
            };
            let Some(layer) = RenderLayer::named(name) else {
                out.notes
                    .push(format!("line {}: not a render layer: {}", e.line, e.key));
                continue;
            };
            for word in e.value.split_whitespace() {
                let Some((tag, name, filters)) = rule_word(word) else {
                    out.notes
                        .push(format!("line {}: not a block name: {word}", e.line));
                    continue;
                };
                out.rules.push(LayerRule {
                    layer,
                    name,
                    tag,
                    filters,
                    line: e.line,
                    word: word.to_owned(),
                });
            }
        }
        out
    }

    /// Each rule's index, the name it names (a block's, or a tag's for `%tag`) and whether that
    /// is a tag, in file order: what a caller indexes the rules of a block by.
    pub fn subjects(&self) -> impl Iterator<Item = (usize, &str, bool)> {
        self.rules
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.name.as_str(), r.tag))
    }

    /// The rules of [`Self::subjects`] by the block name they reach, `tags` giving each tag's
    /// members: what [`Self::pick`] takes for a state of that block.
    pub fn by_name<'a>(&'a self, tags: &'a TagMembers) -> HashMap<&'a str, Vec<usize>> {
        by_name(self.subjects(), tags)
    }

    /// Of the rules `candidates` (indices from [`Self::subjects`] whose subject names the block),
    /// the one for the state with `properties`: a rule naming the block wins over one naming its
    /// tag (4.1), then the rule with the most filters that all match, then the first in the file.
    /// `None` when none matches, and the block keeps its own layer.
    pub fn pick<'a, 'p>(
        &'a self,
        candidates: impl IntoIterator<Item = usize>,
        properties: impl Fn(&str) -> Option<&'p str>,
    ) -> Option<LayerMatch<'a>> {
        candidates
            .into_iter()
            .filter_map(|i| Some((i, self.rules.get(i)?)))
            .filter(|(_, r)| filters_match(&r.filters, &properties))
            .min_by_key(|&(i, r)| rank(r.tag, &r.filters, i))
            .map(|(_, r)| LayerMatch {
                layer: r.layer,
                line: r.line,
                word: &r.word,
            })
    }

    /// [`Self::pick`] among every rule, `in_tag` telling whether block `name` is in a tag.
    pub fn layer<'a>(
        &self,
        name: &str,
        properties: impl Fn(&str) -> Option<&'a str>,
        in_tag: impl Fn(&str) -> bool,
    ) -> Option<RenderLayer> {
        self.pick(candidates(self.subjects(), name, in_tag), properties)
            .map(|m| m.layer)
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Macros;

    #[test]
    fn names_states_and_precedence() {
        let text = "block.10=stone minecraft:dirt mod:thing \\\n grass_block:snowy=false\n\
                    block.11=grass_block\nblock.12=oak_leaves:persistent=true:distance=1,2 %leaves\n\
                    layer.translucent=glass\n";
        let ids = BlockIds::parse(&Properties::parse(text, &Macros::standard()));
        let none = |_: &str| None;
        let no_tag = |_: &str| false;
        assert_eq!(ids.id("minecraft:stone", none, no_tag), Some(10));
        assert_eq!(ids.id("mod:thing", none, no_tag), Some(10));
        assert_eq!(ids.id("minecraft:glass", none, no_tag), None);
        let snowy = |s: &'static str| move |k: &str| (k == "snowy").then_some(s);
        // The filtered entry is the more specific one.
        assert_eq!(
            ids.id("minecraft:grass_block", snowy("false"), no_tag),
            Some(10)
        );
        assert_eq!(
            ids.id("minecraft:grass_block", snowy("true"), no_tag),
            Some(11)
        );
        let leaves = |p: &'static str, d: &'static str| {
            move |k: &str| match k {
                "persistent" => Some(p),
                "distance" => Some(d),
                _ => None,
            }
        };
        assert_eq!(
            ids.id("minecraft:oak_leaves", leaves("true", "2"), no_tag),
            Some(12)
        );
        assert_eq!(
            ids.id("minecraft:oak_leaves", leaves("true", "3"), no_tag),
            None
        );
        assert_eq!(
            ids.id("minecraft:oak_leaves", leaves("false", "1"), no_tag),
            None
        );
        // `%leaves` is a rule now, which no tag reaches here.
        assert!(ids.notes.is_empty(), "{:?}", ids.notes);
        assert!(ids.names_tags());
        let named = BlockIds::parse(&Properties::parse("block.5=stone\n", &Macros::standard()));
        assert!(!named.names_tags());
    }

    fn members(tags: &[(&str, &[&str])]) -> TagMembers {
        tags.iter()
            .map(|(tag, members)| {
                let members = members.iter().map(|m| (*m).to_owned()).collect();
                ((*tag).to_owned(), members)
            })
            .collect()
    }

    /// The issue's case, `block.42=%minecraft:logs`, and every form a tag word takes: with or
    /// without a namespace, with filters, beside a rule naming the block, beside another tag,
    /// a tag the world lacks, an empty `%` and a word of three name parts.
    #[test]
    fn tag_words_give_their_members_ids() {
        let text = "block.42=%minecraft:logs\n\
                    block.43=oak_log:axis=x\n\
                    block.44=%leaves:persistent=true %c:ores\n\
                    block.45=%logs:axis=y %leaves\n\
                    block.46=%no_such_tag %a:b:c %\n";
        let ids = BlockIds::parse(&Properties::parse(text, &Macros::standard()));
        let tags = members(&[
            (
                "minecraft:logs",
                &["minecraft:oak_log", "minecraft:birch_log"],
            ),
            ("minecraft:leaves", &["minecraft:oak_leaves"]),
            ("c:ores", &["mod:ruby_ore"]),
            ("minecraft:empty", &[]),
        ]);
        let by_name = ids.by_name(&tags);
        let id = |name: &str, property: Option<(&'static str, &'static str)>| {
            let properties = move |k: &str| property.filter(|p| p.0 == k).map(|p| p.1);
            let in_tag = |t: &str| tags.get(t).is_some_and(|m| m.contains(name));
            let picked = by_name
                .get(name)
                .and_then(|rules| ids.pick(rules.iter().copied(), properties));
            // The index and the per-block lookup read the rules one way.
            assert_eq!(ids.id(name, properties, in_tag), picked, "{name}");
            picked
        };
        // Only the tag reaches birch logs; on axis=y its filtered rule is the more specific.
        assert_eq!(id("minecraft:birch_log", Some(("axis", "z"))), Some(42));
        assert_eq!(id("minecraft:birch_log", Some(("axis", "y"))), Some(45));
        // A rule naming the block wins over its tags, even with fewer filters; where it
        // doesn't match, the tags' rules decide.
        assert_eq!(id("minecraft:oak_log", Some(("axis", "x"))), Some(43));
        assert_eq!(id("minecraft:oak_log", Some(("axis", "y"))), Some(45));
        assert_eq!(id("minecraft:oak_log", Some(("axis", "z"))), Some(42));
        assert_eq!(
            id("minecraft:oak_leaves", Some(("persistent", "true"))),
            Some(44)
        );
        assert_eq!(
            id("minecraft:oak_leaves", Some(("persistent", "false"))),
            Some(45)
        );
        assert_eq!(id("mod:ruby_ore", None), Some(44));
        assert_eq!(id("minecraft:stone", None), None);
        // A tag the world lacks and the empty `%` reach nothing, without a note; three name
        // parts are no name.
        let subjects: Vec<_> = ids.subjects().filter(|s| s.0 >= 6).collect();
        assert_eq!(
            subjects,
            [(6, "minecraft:no_such_tag", true), (7, "minecraft:", true)]
        );
        assert!(by_name.values().flatten().all(|&i| i < 6));
        assert_eq!(ids.notes, ["line 5: not a block name: %a:b:c"]);
        // Without the world's tags, only the named rule gives an id.
        let none = TagMembers::new();
        let by_name = ids.by_name(&none);
        assert_eq!(by_name.len(), 1);
        let axis = |a: &'static str| move |k: &str| (k == "axis").then_some(a);
        let oak = || by_name["minecraft:oak_log"].iter().copied();
        assert_eq!(ids.pick(oak(), axis("x")), Some(43));
        assert_eq!(ids.pick(oak(), axis("z")), None);
    }

    /// Rank among matching rules is the rule's kind before its filters: an unfiltered rule
    /// naming the block wins over a tag's filtered rule that also matches, and two tag rules of
    /// equal rank reaching one block give the first in the file, whichever tag it names.
    #[test]
    fn a_name_outranks_a_more_filtered_tag_and_equal_tags_take_the_first() {
        let tags = members(&[
            ("minecraft:logs", &["minecraft:oak_log"]),
            ("minecraft:planks", &["minecraft:oak_planks"]),
            ("c:wood", &["minecraft:oak_planks"]),
        ]);
        let id = |text: &str, name: &str, property: Option<(&'static str, &'static str)>| {
            let ids = BlockIds::parse(&Properties::parse(text, &Macros::standard()));
            let properties = move |k: &str| property.filter(|p| p.0 == k).map(|p| p.1);
            let in_tag = |t: &str| tags.get(t).is_some_and(|m| m.contains(name));
            let by_name = ids.by_name(&tags);
            let picked = by_name
                .get(name)
                .and_then(|rules| ids.pick(rules.iter().copied(), properties));
            assert_eq!(ids.id(name, properties, in_tag), picked, "{name}");
            picked
        };
        let text = "block.50=%minecraft:logs:axis=y\nblock.51=oak_log\n";
        assert_eq!(id(text, "minecraft:oak_log", Some(("axis", "y"))), Some(51));
        let planks_first = "block.52=%minecraft:planks\nblock.53=%c:wood\n";
        let wood_first = "block.53=%c:wood\nblock.52=%minecraft:planks\n";
        assert_eq!(id(planks_first, "minecraft:oak_planks", None), Some(52));
        assert_eq!(id(wood_first, "minecraft:oak_planks", None), Some(53));
    }

    /// `item.properties` and `entity.properties` take `%tag` too (D-9): a name's own entry wins
    /// over a tag's, then the first line naming it; a tag the world lacks gives nothing.
    #[test]
    fn plain_names_take_tags() {
        let text = "item.10=%swords\nitem.11=diamond_sword %axes\nitem.12=%swords %axes %missing\n\
                    item.13=%swords:a=b\n";
        let ids = BlockIds::parse_kind(&Properties::parse(text, &Macros::standard()), "item");
        let tags = members(&[
            (
                "minecraft:swords",
                &["minecraft:diamond_sword", "minecraft:iron_sword"],
            ),
            (
                "minecraft:axes",
                &["minecraft:iron_axe", "minecraft:diamond_sword"],
            ),
        ]);
        let plain = ids.plain(&tags);
        assert_eq!(
            plain,
            HashMap::from([
                ("minecraft:diamond_sword".to_owned(), 11),
                ("minecraft:iron_sword".to_owned(), 10),
                ("minecraft:iron_axe".to_owned(), 11),
            ])
        );
        assert_eq!(
            ids.plain(&TagMembers::new()),
            HashMap::from([("minecraft:diamond_sword".to_owned(), 11)])
        );
        let text = "entity.20=%minecraft:skeletons zombie\nentity.21=stray\n";
        let ids = BlockIds::parse_kind(&Properties::parse(text, &Macros::standard()), "entity");
        let tags = members(&[(
            "minecraft:skeletons",
            &["minecraft:skeleton", "minecraft:stray"],
        )]);
        assert_eq!(
            ids.plain(&tags),
            HashMap::from([
                ("minecraft:zombie".to_owned(), 20),
                ("minecraft:skeleton".to_owned(), 20),
                ("minecraft:stray".to_owned(), 21),
            ])
        );
    }

    #[test]
    fn entity_lines_by_name() {
        let text = "entity.10=zombie minecraft:husk\nentity.11=zombie mod:thing\nblock.5=stone\n";
        let ids = BlockIds::parse_kind(&Properties::parse(text, &Macros::standard()), "entity");
        let plain = ids.plain(&TagMembers::new());
        assert_eq!(
            plain["minecraft:zombie"], 10,
            "the first line naming it wins"
        );
        assert_eq!(plain["minecraft:husk"], 10);
        assert_eq!(plain["mod:thing"], 11);
        assert!(!plain.contains_key("minecraft:stone"));
    }

    /// The issue's control: one property's value moves a state, the other states of the same
    /// block keep theirs.
    #[test]
    fn layer_rules_move_named_states_and_tags() {
        let text = "block.10=oak_log\n\
                    layer.translucent=minecraft:oak_log:axis=x %logs:axis=z\n\
                    layer.cutout=birch_log:axis=x mod:glass %minecraft:leaves\n\
                    layer.cutout_mipped=dirt\n\
                    layer.solid=stained:a:b oak_leaves:persistent=true\n\
                    layer.water=stone\n\
                    layer.=stone\n";
        let layers = BlockLayers::parse(&Properties::parse(text, &Macros::standard()));
        let axis = |a: &'static str| move |k: &str| (k == "axis").then_some(a);
        let no_tag = |_: &str| false;
        let logs = |t: &str| t == "minecraft:logs";
        use RenderLayer::*;
        assert_eq!(
            layers.layer("minecraft:oak_log", axis("x"), no_tag),
            Some(Translucent)
        );
        assert_eq!(layers.layer("minecraft:oak_log", axis("y"), no_tag), None);
        assert_eq!(
            layers.layer("minecraft:birch_log", axis("x"), no_tag),
            Some(Cutout)
        );
        assert_eq!(layers.layer("minecraft:birch_log", axis("y"), no_tag), None);
        // Through `%logs`, with its filter: `birch_log` axis=z is in the tag.
        assert_eq!(
            layers.layer("minecraft:birch_log", axis("z"), logs),
            Some(Translucent)
        );
        assert_eq!(layers.layer("minecraft:birch_log", axis("y"), logs), None);
        assert_eq!(layers.layer("mod:glass", |_| None, no_tag), Some(Cutout));
        assert_eq!(
            layers.layer("minecraft:dirt", |_| None, no_tag),
            Some(Cutout)
        );
        // A rule naming the block wins over its tag, even with fewer filters.
        let leaves = |t: &str| t == "minecraft:leaves";
        let persistent = |k: &str| (k == "persistent").then_some("true");
        assert_eq!(
            layers.layer("minecraft:oak_leaves", persistent, leaves),
            Some(Solid)
        );
        assert_eq!(
            layers.layer("minecraft:oak_leaves", |_| Some("false"), leaves),
            Some(Cutout)
        );
        // Unknown layer names and a word of three name parts are refused, by name.
        assert_eq!(layers.layer("minecraft:stone", |_| None, no_tag), None);
        assert_eq!(
            layers.notes,
            [
                "line 5: not a block name: stained:a:b",
                "line 6: not a render layer: layer.water",
                "line 7: not a render layer: layer.",
            ]
        );
        let tags: Vec<_> = layers.subjects().filter(|s| s.2).map(|s| s.1).collect();
        assert_eq!(tags, ["minecraft:logs", "minecraft:leaves"]);
    }

    /// The most filters win among rules naming the block, then the first in the file.
    #[test]
    fn layer_rule_precedence() {
        let text = "layer.cutout=glass\nlayer.translucent=glass glass:a=1 glass:a=1:b=2\n\
                    layer.solid=glass:b=2:a=1\n";
        let layers = BlockLayers::parse(&Properties::parse(text, &Macros::standard()));
        let props = |a: &'static str, b: &'static str| {
            move |k: &str| match k {
                "a" => Some(a),
                "b" => Some(b),
                _ => None,
            }
        };
        let no_tag = |_: &str| false;
        use RenderLayer::*;
        assert_eq!(
            layers.layer("minecraft:glass", props("0", "0"), no_tag),
            Some(Cutout)
        );
        assert_eq!(
            layers.layer("minecraft:glass", props("1", "0"), no_tag),
            Some(Translucent)
        );
        assert_eq!(
            layers.layer("minecraft:glass", props("1", "2"), no_tag),
            Some(Translucent)
        );
        // The winning rule is named by its line and word, for a note.
        let glass = layers
            .subjects()
            .filter(|s| s.1 == "minecraft:glass")
            .map(|s| s.0);
        assert_eq!(
            layers.pick(glass, props("1", "2")),
            Some(LayerMatch {
                layer: Translucent,
                line: 2,
                word: "glass:a=1:b=2"
            })
        );
        assert!(layers.notes.is_empty());
        // A file without `layer.*` lines has no rules; its id lines are not layers.
        let ids = Properties::parse("block.5=stone\n", &Macros::standard());
        assert!(BlockLayers::parse(&ids).is_empty());
    }
}
