//! Tests of the guides' parsing, discovery, navigation, search, structures, scenes and layout, from the spec's
//! statements and the decisions in docs/cleanroom/guides-decisions.md.

use super::Problem;
use super::front::{self, FrontMatter};
use super::layout::{self, Action, Env, Piece, RecipeQuery, Tip};
use super::library::{Entry, Library, Sources, Target};
use super::markdown::{Attr, Block, Inline, parse, parse_inlines};
use super::scene::{SceneModel, Shape};
use super::search::search;
use super::structure::{State, Structure};
use std::collections::HashSet;

fn text(s: &str) -> Inline {
    Inline::Text(s.into())
}

// ---- Front matter ----

#[test]
fn front_matter_reads_navigation_items_and_categories() {
    let page = "---\nnavigation:\n  title: Cables\n  icon: fluix_cable\n  parent: index.md\n  position: 20\n  \
                max_depth: 1\n  icon_components:\n    \"minecraft:rarity\": epic\nitem_ids:\n  - cable\n  - \
                minecraft:stick\ncategories: [Devices, Basics]\nunknown_key: 3\n---\n# Body\n";
    let (yaml, body) = front::split(page);
    assert_eq!(body, "# Body\n");
    let f = front::parse(yaml.unwrap()).unwrap();
    assert_eq!(
        f,
        FrontMatter {
            navigation: true,
            title: Some("Cables".into()),
            icon: Some("fluix_cable".into()),
            icon_components: "minecraft:rarity=\"epic\"".into(),
            parent: Some("index.md".into()),
            position: Some(20.0),
            max_depth: Some(1),
            item_ids: vec!["cable".into(), "minecraft:stick".into()],
            categories: vec!["Devices".into(), "Basics".into()],
            requires_mods: vec![],
        }
    );
}

#[test]
fn front_matter_absent_unclosed_or_broken() {
    assert_eq!(front::split("# Title\n---\n"), (None, "# Title\n---\n"));
    assert_eq!(front::split("---\ntitle: x\n").0, None);
    // A key at the wrong level is unknown: no navigation (spec 3.4, Q-007).
    let f = front::parse("icon: stick\n").unwrap();
    assert!(!f.navigation && f.icon.is_none());
    assert!(front::parse("navigation: [unclosed\n").is_err());
}

// ---- Markdown ----

#[test]
fn blocks_headings_lists_quotes_code_rules() {
    let doc = parse(
        "# Title\n\nSome *text*\nwrapped.\n\n- one\n- two\n  continued\n\n1. first\n2. second\n\n> quoted\n\n---\n\n```\ncode <Tag>\n```\n",
    );
    let b = &doc.blocks;
    assert!(matches!(&b[0], Block::Heading { level: 1, anchor, .. } if anchor == "title"));
    assert_eq!(
        b[1],
        Block::Paragraph(vec![
            text("Some "),
            Inline::Emph(vec![text("text")]),
            text(" wrapped.")
        ])
    );
    match &b[2] {
        Block::List {
            ordered: false,
            items,
            ..
        } => {
            assert_eq!(items.len(), 2);
            assert_eq!(
                items[1],
                vec![Block::Paragraph(vec![text("two continued")])]
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&b[3], Block::List { ordered: true, start: 1, items } if items.len() == 2));
    assert_eq!(
        b[4],
        Block::Quote(vec![Block::Paragraph(vec![text("quoted")])])
    );
    assert_eq!(b[5], Block::Rule);
    assert_eq!(b[6], Block::Code("code <Tag>".into()));
}

#[test]
fn inline_formatting_links_images_and_code() {
    assert_eq!(
        parse_inlines("**bold** __b__ ~~gone~~ ~x~ `a*b` [see](page.md#top) ![alt](img.png)"),
        vec![
            Inline::Strong(vec![text("bold")]),
            text(" "),
            Inline::Strong(vec![text("b")]),
            text(" "),
            Inline::Strike(vec![text("gone")]),
            text(" "),
            Inline::Strike(vec![text("x")]),
            text(" "),
            Inline::Code("a*b".into()),
            text(" "),
            Inline::Link {
                target: "page.md#top".into(),
                children: vec![text("see")]
            },
            text(" "),
            Inline::Image {
                src: "img.png".into(),
                alt: "alt".into()
            },
        ]
    );
    // An underscore inside a word is text; an unmatched star too.
    assert_eq!(
        parse_inlines("snake_case_name 2 * 3"),
        vec![text("snake_case_name 2 * 3")]
    );
}

#[test]
fn tables_with_alignment() {
    let doc = parse("| A | B |\n|:--|--:|\n| 1 | `x\\|y` |\n");
    match &doc.blocks[0] {
        Block::Table { align, head, rows } => {
            assert_eq!(
                align,
                &[super::markdown::Align::Left, super::markdown::Align::Right]
            );
            assert_eq!(head, &[vec![text("A")], vec![text("B")]]);
            assert_eq!(rows[0][1], vec![Inline::Code("x|y".into())]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn tags_blocks_inline_attributes_and_nesting() {
    let doc = parse(
        "<Row gap=\"8\" fullWidth>\n  <ItemImage id=\"stick\" scale={2} />\n\n  <Column>\n    Text **here**\n  \
         </Column>\n</Row>\n\nSee <ItemLink id=\"minecraft:stone\" optional /> now.\n",
    );
    let Block::Tag(row) = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks[0]);
    };
    assert_eq!(row.name, "Row");
    assert_eq!(row.attrs.num("gap"), Some(8.0));
    assert_eq!(row.attrs.flag("fullWidth"), Some(true));
    let Block::Tag(img) = &row.children[0] else {
        panic!()
    };
    assert_eq!(
        (img.name.as_str(), img.attrs.num("scale")),
        ("ItemImage", Some(2.0))
    );
    let Block::Tag(col) = &row.children[1] else {
        panic!()
    };
    assert_eq!(
        col.children,
        vec![Block::Paragraph(vec![
            text("Text "),
            Inline::Strong(vec![text("here")])
        ])]
    );
    match &doc.blocks[1] {
        Block::Paragraph(i) => {
            assert_eq!(i[0], text("See "));
            let Inline::Tag(t) = &i[1] else { panic!() };
            assert_eq!(t.attrs.get("optional"), Some(&Attr::Flag));
            assert_eq!(i[2], text(" now."));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn comments_unknown_lowercase_and_unclosed_tags() {
    let doc = parse(
        "Run `/x` with /ae2 channelmode <mode> {/* hidden\nstill hidden */}here.\n\n<Row>\nunclosed\n",
    );
    assert_eq!(
        doc.blocks[0],
        Block::Paragraph(vec![
            text("Run "),
            Inline::Code("/x".into()),
            text(" with /ae2 channelmode <mode> here.")
        ])
    );
    assert_eq!(doc.blocks[1], Block::Unclosed("Row".into()));
    assert_eq!(doc.blocks[2], Block::Paragraph(vec![text("unclosed")]));
}

#[test]
fn heading_anchors_drop_punctuation_and_number_repeats() {
    let doc = parse("# Crafting Storage\n## Dense Cable\n## What's *new*?\n## Dense Cable\n");
    let anchors: Vec<_> = doc
        .blocks
        .iter()
        .map(|b| match b {
            Block::Heading { anchor, .. } => anchor.clone(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(
        anchors,
        [
            "crafting-storage",
            "dense-cable",
            "whats-new",
            "dense-cable-1"
        ]
    );
}

// ---- Library ----

struct Files(Vec<(String, String)>);

impl Files {
    fn new(files: &[(&str, &str)]) -> Self {
        Files(
            files
                .iter()
                .map(|(n, c)| (n.to_string(), c.to_string()))
                .collect(),
        )
    }
    fn library(&self, lang: &str, installed: &[&str]) -> Library {
        let names: Vec<String> = self.0.iter().map(|(n, _)| n.clone()).collect();
        let read = |n: &str| {
            self.0
                .iter()
                .find(|(k, _)| k == n)
                .map(|(_, c)| c.clone().into_bytes())
        };
        let installed: HashSet<String> = installed.iter().map(|s| s.to_string()).collect();
        Library::build(&Sources {
            names: &names,
            read: &read,
            language: lang,
            installed: &|m| installed.contains(m),
            mod_name: &|ns| (ns == "ae2").then(|| "Applied Energistics 2".to_owned()),
        })
    }
}

fn nav(title: &str, extra: &str) -> String {
    format!("---\nnavigation:\n  title: {title}\n{extra}---\n# {title}\n")
}

fn sample() -> Files {
    Files::new(&[
        (
            "assets/ae2/ae2guide/index.md",
            &nav("Start", "  position: 0\nitem_ids: [controller]\n"),
        ),
        (
            "assets/ae2/ae2guide/mechanics/channels.md",
            &nav(
                "Channels",
                "  parent: index.md\n  position: 2\ncategories: [Basics]\n",
            ),
        ),
        (
            "assets/ae2/ae2guide/mechanics/power.md",
            &nav(
                "Power",
                "  parent: index.md\n  position: 1\ncategories: [Basics]\n",
            ),
        ),
        (
            "assets/ae2/ae2guide/mechanics/deep.md",
            &nav("Deep", "  parent: mechanics/power.md\n"),
        ),
        (
            "assets/ae2/ae2guide/_de_de/index.md",
            &nav("Anfang", "  position: 0\nitem_ids: [controller]\n"),
        ),
        ("assets/ae2/ae2guide/_de_de/only_de.md", "# Nur deutsch\n"),
        ("assets/ae2/ae2guide/assets/diagram.png", "png"),
        ("assets/ae2/ae2guide/_de_de/assets/diagram.png", "png-de"),
        ("assets/ae2/ae2guide/Bad Name.md", "# bad\n"),
        (
            "assets/ironfurnaces/guides/ironfurnaces/guide/with_ae2.md",
            "---\nnavigation:\n  title: With AE2\n  parent: ae2:index.md\nrequires_mods: [ae2]\n---\nText\n",
        ),
        (
            "assets/ironfurnaces/guides/ironfurnaces/guide/with_mek.md",
            "---\nnavigation:\n  title: With Mekanism\nrequires_mods: [mekanism]\nitem_ids: [minecraft:stick]\n---\n",
        ),
        (
            "assets/ironfurnaces/guides/ironfurnaces/guide/furnaces.md",
            "---\nnavigation:\n  title: Furnaces\nitem_ids: [iron_furnace, minecraft:furnace]\n---\n# Furnaces\n",
        ),
        (
            "assets/pack/guides/ironfurnaces/guide/furnaces.md",
            "# From a pack namespace\n",
        ),
        (
            "assets/other/guides/other/guide/furnace_too.md",
            "---\nitem_ids: [minecraft:furnace]\n---\n",
        ),
        (
            "assets/ironfurnaces/guideme_guides/guide.json",
            "{\"default_language\": \"en_us\", \"custom_colors\": {\"hot\": {\"dark_mode\": \"#FF8800\", \"light_mode\": \"#AA4400\"}}}",
        ),
    ])
}

#[test]
fn discovery_finds_both_layouts_and_skips_bad_names() {
    let lib = sample().library("en_us", &["ae2", "ironfurnaces"]);
    let ids: Vec<&str> = lib.guides.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids, ["ae2:guide", "ironfurnaces:guide", "other:guide"]);
    assert_eq!(lib.guides[0].name, "Applied Energistics 2");
    // Q-004: a guide with no mod name shows its namespace.
    assert_eq!(lib.guides[1].name, "ironfurnaces");
    assert!(lib.page("ae2:mechanics/channels.md").is_some());
    assert!(lib.errors.iter().any(|e| e.contains("Bad Name.md")));
    // Q-005: the guide's own namespace's copy wins over another namespace's.
    let furnaces = lib.page("ironfurnaces:furnaces.md").unwrap();
    assert_eq!(lib.pages[furnaces].title, "Furnaces");
    assert_eq!(
        lib.guides[1].colors.get("ironfurnaces:hot"),
        Some(&(0xff8800, 0xaa4400))
    );
}

#[test]
fn both_layouts_merge_and_untitled_pages_take_their_file_name() {
    let files = Files::new(&[
        (
            "assets/m/ae2guide/same.md",
            "# From ae2guide
",
        ),
        (
            "assets/m/ae2guide/only_old.md",
            "Body only
",
        ),
        (
            "assets/m/guides/m/guide/same.md",
            "# From guides
",
        ),
    ]);
    let lib = files.library("en_us", &[]);
    assert_eq!(lib.guides.len(), 1);
    assert_eq!(
        lib.pages[lib.page("m:same.md").unwrap()].title,
        "From guides"
    );
    assert_eq!(
        lib.pages[lib.page("m:only_old.md").unwrap()].title,
        "only_old"
    );
}

#[test]
fn translations_replace_pages_and_assets_with_fallback() {
    let en = sample().library("en_us", &["ae2"]);
    let de = sample().library("de_de", &["ae2"]);
    let start = |lib: &Library| lib.pages[lib.page("ae2:index.md").unwrap()].title.clone();
    assert_eq!(start(&en), "Start");
    assert_eq!(start(&de), "Anfang");
    // Untranslated pages fall back; a translation without a default page is ignored (Q-041).
    assert_eq!(
        de.pages[de.page("ae2:mechanics/power.md").unwrap()].title,
        "Power"
    );
    assert!(de.page("ae2:only_de.md").is_none());
    let index = de.page("ae2:index.md").unwrap();
    assert_eq!(
        de.asset(index, "assets/diagram.png").as_deref(),
        Some("assets/ae2/ae2guide/_de_de/assets/diagram.png")
    );
    let index = en.page("ae2:index.md").unwrap();
    assert_eq!(
        en.asset(index, "./assets/diagram.png").as_deref(),
        Some("assets/ae2/ae2guide/assets/diagram.png")
    );
    let power = en.page("ae2:mechanics/power.md").unwrap();
    assert_eq!(
        en.asset(power, "../assets/diagram.png").as_deref(),
        Some("assets/ae2/ae2guide/assets/diagram.png")
    );
}

#[test]
fn navigation_tree_order_depth_and_cross_guide_parents() {
    let lib = sample().library("en_us", &["ae2", "ironfurnaces"]);
    let id = |p: usize| lib.pages[p].id.as_str();
    let index = lib.page("ae2:index.md").unwrap();
    // Sorted by position; the cross-mod page (no position: 0) names its parent in the other guide (spec 7.4).
    let children: Vec<&str> = lib.pages[index].children.iter().map(|&p| id(p)).collect();
    assert_eq!(
        children,
        [
            "ironfurnaces:with_ae2.md",
            "ae2:mechanics/power.md",
            "ae2:mechanics/channels.md"
        ]
    );
    let mut open: HashSet<String> = ["ae2:guide", "ae2:index.md", "ae2:mechanics/power.md"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let rows = lib.rows(&open);
    let shown: Vec<String> = rows
        .iter()
        .map(|r| match r.entry {
            Entry::Guide(g) => format!("{}:{}", r.depth, lib.guides[g].id),
            Entry::Page(p) => format!("{}:{}", r.depth, id(p)),
        })
        .collect();
    // Two levels by default: power's child is past the limit, so power shows no children (spec 9.2.3).
    assert_eq!(
        shown,
        [
            "0:ae2:guide",
            "1:ae2:index.md",
            "2:ironfurnaces:with_ae2.md",
            "2:ae2:mechanics/power.md",
            "2:ae2:mechanics/channels.md",
            "0:ironfurnaces:guide",
        ]
    );
    assert!(!rows[3].has_children);
    open.insert("ironfurnaces:guide".into());
    assert_eq!(lib.rows(&open).len(), 7);
    assert_eq!(
        lib.path_keys(lib.page("ae2:mechanics/deep.md").unwrap()),
        ["ae2:mechanics/power.md", "ae2:index.md", "ae2:guide"]
    );
}

#[test]
fn missing_parents_and_cycles_are_reported_and_cut() {
    let files = Files::new(&[
        ("assets/m/ae2guide/a.md", &nav("A", "  parent: b.md\n")),
        ("assets/m/ae2guide/b.md", &nav("B", "  parent: a.md\n")),
        (
            "assets/m/ae2guide/c.md",
            &nav("C", "  parent: nowhere.md\n"),
        ),
    ]);
    let lib = files.library("en_us", &[]);
    assert!(lib.errors.iter().any(|e| e.contains("cycle")));
    assert!(lib.errors.iter().any(|e| e.contains("nowhere.md")));
    // Every page is still reachable from the top of the tree.
    let roots: Vec<&str> = lib.guides[0]
        .roots
        .iter()
        .map(|&p| lib.pages[p].id.as_str())
        .collect();
    assert!(roots.contains(&"m:c.md"));
    assert_eq!(roots.len(), 2);
}

#[test]
fn requires_mods_hides_pages_and_item_lookup_prefers_the_items_guide() {
    let lib = sample().library("en_us", &["ae2", "ironfurnaces"]);
    let with_mek = lib.page("ironfurnaces:with_mek.md").unwrap();
    assert_eq!(lib.pages[with_mek].hidden.as_deref(), Some("mekanism"));
    // The hidden page's items are not its own.
    assert_eq!(lib.item_page("minecraft:stick"), None);
    let index = lib.page("ae2:index.md").unwrap();
    assert_eq!(lib.item_page("ae2:controller"), Some(index));
    // Two pages claim minecraft:furnace, neither in minecraft's guide: the first by page id (Q-047).
    assert_eq!(
        lib.item_page("minecraft:furnace"),
        lib.page("ironfurnaces:furnaces.md")
    );
    assert_eq!(
        lib.item_page("ironfurnaces:iron_furnace"),
        lib.page("ironfurnaces:furnaces.md")
    );
    // Without AE2 the "With AE2" page is gone, and links to it say why (Q-015).
    let lib = sample().library("en_us", &["ironfurnaces"]);
    let furnaces = lib.page("ironfurnaces:furnaces.md").unwrap();
    match lib.resolve(furnaces, "with_ae2.md") {
        Target::Hidden { mod_id, .. } => assert_eq!(mod_id, "ae2"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn links_resolve_relative_full_ids_anchors_and_urls() {
    let lib = sample().library("en_us", &["ae2", "ironfurnaces"]);
    let channels = lib.page("ae2:mechanics/channels.md").unwrap();
    let power = lib.page("ae2:mechanics/power.md").unwrap();
    let index = lib.page("ae2:index.md").unwrap();
    assert_eq!(
        lib.resolve(channels, "power.md#cables"),
        Target::Page {
            page: power,
            anchor: Some("cables".into())
        }
    );
    assert_eq!(
        lib.resolve(channels, "./power.md"),
        Target::Page {
            page: power,
            anchor: None
        }
    );
    assert_eq!(
        lib.resolve(channels, "../index.md"),
        Target::Page {
            page: index,
            anchor: None
        }
    );
    assert_eq!(
        lib.resolve(channels, "#here"),
        Target::Page {
            page: channels,
            anchor: Some("here".into())
        }
    );
    assert_eq!(
        lib.resolve(channels, "ironfurnaces:furnaces.md"),
        Target::Page {
            page: lib.page("ironfurnaces:furnaces.md").unwrap(),
            anchor: None
        }
    );
    assert_eq!(
        lib.resolve(channels, "https://example.org/x"),
        Target::External("https://example.org/x".into())
    );
    assert_eq!(
        lib.resolve(channels, "gone.md"),
        Target::Missing("ae2:mechanics/gone.md".into())
    );
}

#[test]
fn categories_list_matching_pages_by_title() {
    let lib = sample().library("en_us", &["ae2"]);
    let titles: Vec<&str> = lib
        .category("Basics")
        .iter()
        .map(|&p| lib.pages[p].title.as_str())
        .collect();
    assert_eq!(titles, ["Channels", "Power"]);
    assert!(lib.category("basics").is_empty());
}

// ---- Search ----

#[test]
fn search_matches_word_prefixes_titles_first() {
    let files = Files::new(&[
        (
            "assets/m/ae2guide/a.md",
            "# Channels\n\nCables carry channels to devices.\n",
        ),
        (
            "assets/m/ae2guide/b.md",
            "# Devices\n\nSome text.\n\n## Wiring\n\nA dense cable carries 32 channels.\n",
        ),
        ("assets/m/ae2guide/c.md", "# Other\n\nNothing about it.\n"),
    ]);
    let lib = files.library("en_us", &[]);
    let hits = search(&lib, "CHANN");
    let pages: Vec<&str> = hits.iter().map(|h| lib.pages[h.page].id.as_str()).collect();
    assert_eq!(pages, ["m:a.md", "m:b.md"]);
    assert_eq!(hits[1].anchor.as_deref(), Some("wiring"));
    assert!(hits[1].excerpt.contains("dense cable"));
    // Every word must match.
    assert_eq!(search(&lib, "dense chann").len(), 1);
    assert!(search(&lib, "annels").is_empty());
}

// ---- Structures and scenes ----

#[test]
fn snbt_structures_in_both_layouts() {
    let data = "{DataVersion: 3465, size: [2, 1, 1], data: [{pos: [0, 0, 0], state: \"ae2:cable_bus{light_level:9,\
                waterlogged:false}\", nbt: {id: \"ae2:cable_bus\"}}, {pos: [1, 0, 0], state: \"minecraft:stone\"}], \
                entities: [], palette: [\"minecraft:stone\"]}";
    let s = Structure::read(data.as_bytes(), true).unwrap();
    assert_eq!(s.size, [2, 1, 1]);
    assert_eq!(s.data_version, Some(3465));
    assert_eq!(
        s.blocks[0].state.command(),
        "ae2:cable_bus[light_level=9,waterlogged=false]"
    );
    assert!(s.blocks[0].nbt.is_some());
    let index = "{size: [1, 2, 1], palette: [{Name: \"minecraft:furnace\", Properties: {facing: \"north\", lit: \"true\"}}, \
                 {Name: \"minecraft:dirt\"}], blocks: [{pos: [0, 0, 0], state: 1}, {pos: [0, 1, 0], state: 0}], \
                 entities: [{pos: [0.5d, 1.0d, 0.5d], blockPos: [0, 1, 0], nbt: {id: \"minecraft:sheep\"}}]}";
    let s = Structure::read(index.as_bytes(), true).unwrap();
    assert_eq!(s.blocks[0].state.command(), "minecraft:dirt");
    assert_eq!(
        s.blocks[1].state.command(),
        "minecraft:furnace[facing=north,lit=true]"
    );
    assert_eq!(s.entities[0].pos, [0.5, 1.0, 0.5]);
}

#[test]
fn binary_nbt_structures_gzipped() {
    use std::io::Write;
    use sunrise_core::nbt::{self, NamedTag, Tag};
    let root = Tag::compound([
        (
            "size",
            Tag::List(vec![Tag::Int(1), Tag::Int(1), Tag::Int(1)]),
        ),
        (
            "palettes",
            Tag::List(vec![Tag::List(vec![Tag::compound([(
                "Name",
                Tag::String("minecraft:glass".into()),
            )])])]),
        ),
        (
            "blocks",
            Tag::List(vec![Tag::compound([
                (
                    "pos",
                    Tag::List(vec![Tag::Int(0), Tag::Int(0), Tag::Int(0)]),
                ),
                ("state", Tag::Int(0)),
            ])]),
        ),
    ]);
    let raw = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag: root,
        },
        nbt::Limits::default(),
    )
    .unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).unwrap();
    let s = Structure::read(&gz.finish().unwrap(), false).unwrap();
    assert_eq!(s.blocks.len(), 1);
    assert_eq!(s.blocks[0].state, State::new("minecraft:glass"));
}

fn scene_of(markup: &str, files: &[(&str, &str)]) -> SceneModel {
    let doc = parse(markup);
    let Block::Tag(tag) = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks)
    };
    SceneModel::build(tag, "mod", &|src| {
        files
            .iter()
            .find(|(n, _)| *n == src)
            .map(|(_, c)| c.as_bytes().to_vec())
    })
}

#[test]
fn scene_children_run_in_document_order() {
    let structure = "{size: [3, 1, 1], data: [{pos: [0, 0, 0], state: \"minecraft:stone\"}, {pos: [1, 0, 0], state: \
                     \"minecraft:barrier\"}, {pos: [2, 0, 0], state: \"minecraft:redstone_wire{power:3}\"}]}";
    let scene = scene_of(
        "<GameScene zoom=\"4\" interactive={true} background=\"#102030\">\n  <ImportStructure src=\"a.snbt\" pos=\"0 1 0\" />\n  \
         <RemoveBlocks id=\"minecraft:barrier\" />\n  <Block id=\"lamp\" x=\"5\" p:lit=\"true\" />\n  <Block \
         id=\"minecraft:air\" x=\"2\" y=\"1\" />\n  <BlockAnnotationTemplate id=\"minecraft:stone\">\n    \
         <DiamondAnnotation pos=\"0.5 0.5 0.5\" color=\"#ff0000\">Stone here</DiamondAnnotation>\n  \
         </BlockAnnotationTemplate>\n  <IsometricCamera yaw=\"90\" />\n  <Entity id=\"minecraft:sheep\" />\n\
         </GameScene>\n",
        &[("a.snbt", structure)],
    );
    let blocks: Vec<(String, [i32; 3])> = scene
        .blocks
        .iter()
        .map(|(p, (s, _))| (s.command(), *p))
        .collect();
    assert_eq!(
        blocks,
        [
            ("minecraft:stone".to_owned(), [0, 1, 0]),
            ("mod:lamp[lit=true]".to_owned(), [5, 0, 0])
        ]
    );
    assert_eq!(scene.camera, [90.0, 0.0, 0.0]);
    assert_eq!(
        (scene.zoom, scene.interactive, scene.background),
        (4.0, true, 0xff10_2030)
    );
    assert_eq!(scene.annotations.len(), 1);
    assert_eq!(
        scene.annotations[0].shape,
        Shape::Diamond {
            pos: [0.5, 1.5, 0.5]
        }
    );
    assert_eq!(scene.annotations[0].color, 0xffff_0000);
    assert_eq!(scene.entities[0].pos, [0.5, 0.0, 0.5]);
    assert_eq!(scene.entities[0].yaw, -45.0);
    assert!(scene.errors.is_empty(), "{:?}", scene.errors);
}

#[test]
fn scenes_over_the_limit_or_missing_files_report() {
    let missing = scene_of(
        "<GameScene>\n<ImportStructure src=\"gone.snbt\" />\n</GameScene>\n",
        &[],
    );
    assert_eq!(
        missing.errors,
        [Problem::new(
            "sunrise.guide.error.missing_structure",
            "gone.snbt"
        )]
    );
    assert!(!missing.refused());
    let entities = "<Entity id=\"minecraft:pig\" />\n".repeat(17);
    let crowded = scene_of(&format!("<GameScene>\n{entities}</GameScene>\n"), &[]);
    assert_eq!(
        crowded.errors,
        [Problem::new("sunrise.guide.error.too_many_entities", "17")]
    );
    assert!(crowded.refused());
}

// ---- Layout ----

/// A font where every character is 6 px wide (7 bold), and a few known items, keys and recipes.
struct TestEnv {
    installed: Vec<&'static str>,
}

impl Env for TestEnv {
    fn text_width(&self, text: &str, bold: bool) -> f32 {
        text.chars().count() as f32 * if bold { 7.0 } else { 6.0 }
    }
    fn item_name(&self, id: &str, _: &str) -> Option<String> {
        match id {
            "minecraft:stick" => Some("Stick".into()),
            "ae2:controller" => Some("ME Controller".into()),
            "ae2:cable" => Some("Cable".into()),
            _ => None,
        }
    }
    fn block_exists(&self, id: &str) -> bool {
        id == "ae2:controller"
    }
    fn image_size(&self, resource: &str) -> Option<[f32; 2]> {
        resource.ends_with(".png").then_some([40.0, 30.0])
    }
    fn key_name(&self, mapping: &str) -> Option<String> {
        (mapping == "key.jump").then(|| "Space".into())
    }
    fn player_name(&self) -> String {
        "Alex".into()
    }
    fn installed(&self, mod_id: &str) -> bool {
        self.installed.contains(&mod_id)
    }
    fn recipes(&self, query: &RecipeQuery) -> Vec<[f32; 2]> {
        match query {
            RecipeQuery::All(id) if id == "minecraft:stick" => vec![[104.0, 60.0], [104.0, 60.0]],
            RecipeQuery::First(id) if id == "minecraft:stick" => vec![[104.0, 60.0]],
            _ => Vec::new(),
        }
    }
    fn read(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

fn laid(body: &str, width: f32, installed: Vec<&'static str>) -> (Library, layout::Laid) {
    let files = Files::new(&[
        ("assets/ae2/ae2guide/page.md", body),
        (
            "assets/ae2/ae2guide/other.md",
            "---\nitem_ids: [controller]\n---\n# Other page\n",
        ),
        ("assets/ae2/ae2guide/img.png", "png"),
    ]);
    let lib = files.library("en_us", &installed);
    let page = lib.page("ae2:page.md").unwrap();
    let out = layout::layout(&lib, page, &TestEnv { installed }, width);
    (lib, out)
}

fn texts(out: &layout::Laid) -> Vec<(f32, f32, String)> {
    out.pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Text { x, y, text, .. } => Some((*x, *y, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn text_wraps_to_the_width_by_words() {
    // 6 px a character: "aaaa bbbb cccc" is 84 px; 60 px holds "aaaa bbbb" (54 px) and then "cccc".
    let (_, out) = laid("aaaa bbbb cccc\n", 60.0, vec![]);
    let t = texts(&out);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].2, "aaaa bbbb");
    assert_eq!(t[1].2, "cccc");
    assert_eq!(t[1].1 - t[0].1, layout::LINE);
    assert_eq!(out.height, 2.0 * layout::LINE);
}

#[test]
fn links_get_hotspots_and_anchors_their_lines() {
    let (lib, out) = laid(
        "# Top\n\nGo [there](other.md) or [nowhere](missing.md).\n\n<a name=\"mark\"></a>Marked\n",
        400.0,
        vec![],
    );
    let other = lib.page("ae2:other.md").unwrap();
    let to_other = out
        .hotspots
        .iter()
        .find(|h| {
            h.action
                == Some(Action::Go(Target::Page {
                    page: other,
                    anchor: None,
                }))
        })
        .expect("link hotspot");
    assert_eq!(to_other.tip, Some(Tip::Page(other)));
    // "Go " is 3 characters of 6 px.
    assert_eq!(to_other.rect[0], 18.0);
    assert_eq!(to_other.rect[2], 30.0);
    let missing = out
        .hotspots
        .iter()
        .find(|h| matches!(h.tip, Some(Tip::Missing(_))))
        .unwrap();
    assert_eq!(missing.action, None);
    assert!(out.errors.contains(&Problem::new(
        "sunrise.guide.error.missing_page",
        "ae2:missing.md"
    )));
    assert_eq!(out.anchors["top"], 0.0);
    assert!(out.anchors["mark"] > out.anchors["top"]);
}

#[test]
fn item_links_name_items_and_link_their_pages() {
    let (lib, out) = laid(
        "<ItemLink id=\"controller\" /> <ItemLink id=\"minecraft:stick\" /> <ItemLink id=\"cable\" /> \
         <ItemLink id=\"x:y\" fallback=\"Gone\" /> <KeyBind id=\"key.jump\" /> <PlayerName />\n",
        800.0,
        vec![],
    );
    let all: String = texts(&out)
        .into_iter()
        .map(|t| t.2)
        .collect::<Vec<_>>()
        .join("|");
    // The stick has no page and is another namespace's: plain. The cable is AE2's own with no page: an error.
    assert!(all.contains("ME Controller"), "{all}");
    assert!(all.contains("Stick"));
    assert!(all.contains("No page for ae2:cable"));
    assert!(all.contains("Gone"));
    assert!(all.contains("Space"));
    assert!(all.contains("Alex"));
    let controller = out
        .hotspots
        .iter()
        .find(|h| matches!(&h.action, Some(Action::Item { id, .. }) if id == "ae2:controller"))
        .unwrap();
    assert_eq!(controller.rect[0], 0.0);
    assert_eq!(lib.item_page("ae2:controller"), lib.page("ae2:other.md"));
}

#[test]
fn optional_sections_follow_installed_mods() {
    let body = "<OptionalSection modId=\"mekanism\">\nWith Mekanism\n</OptionalSection>\n\n<OptionalSection \
                modId=\"!mekanism\">\nWithout Mekanism\n</OptionalSection>\n";
    let shown = |installed| {
        texts(&laid(body, 400.0, installed).1)
            .into_iter()
            .map(|t| t.2)
            .collect::<Vec<_>>()
    };
    assert_eq!(shown(vec!["mekanism"]), ["With Mekanism"]);
    assert_eq!(shown(vec![]), ["Without Mekanism"]);
}

#[test]
fn floating_images_push_text_aside_until_cleared() {
    let (_, out) = laid(
        "<FloatingImage src=\"img.png\" align=\"left\" />\n\nbeside\n\n<br clear=\"all\" />\n\nbelow\n",
        200.0,
        vec![],
    );
    let image = out.pieces.iter().find_map(|p| match p {
        Piece::Image { rect, .. } => Some(*rect),
        _ => None,
    });
    assert_eq!(image, Some([0.0, 0.0, 40.0, 30.0]));
    let t = texts(&out);
    // Beside the 40 px image with a 4 px gap; below it once cleared.
    assert_eq!((t[0].0, t[0].2.as_str()), (44.0, "beside"));
    assert_eq!(t[1].0, 0.0);
    assert!(t[1].1 >= 30.0);
}

#[test]
fn rows_place_children_side_by_side_and_recipes_wrap() {
    let (_, out) = laid(
        "<Row gap=\"10\">\n  <ItemImage id=\"minecraft:stick\" scale=\"2\" />\n  <ItemImage id=\"minecraft:stick\" />\n</Row>\n\n<RecipesFor id=\"minecraft:stick\" />\n\n<RecipeFor id=\"minecraft:dirt\" fallbackText=\"No dirt\" />\n",
        150.0,
        vec![],
    );
    let items: Vec<(f32, f32)> = out
        .pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Item { x, size, .. } => Some((*x, *size)),
            _ => None,
        })
        .collect();
    assert_eq!(items, [(0.0, 32.0), (42.0, 16.0)]);
    let recipes: Vec<[f32; 4]> = out
        .pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Recipe { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();
    // Two 104 px recipes don't fit 150 px side by side: the second goes under the first.
    assert_eq!(recipes.len(), 2);
    assert_eq!(recipes[0][0], recipes[1][0]);
    assert_eq!(recipes[1][1] - recipes[0][1], 60.0);
    assert!(texts(&out).iter().any(|t| t.2 == "No dirt"));
}

#[test]
fn unknown_tags_show_an_error_without_children() {
    let (_, out) = laid("<Fancy>\nsecret\n</Fancy>\n", 400.0, vec![]);
    let all: Vec<String> = texts(&out).into_iter().map(|t| t.2).collect();
    assert_eq!(all, ["Unknown tag <Fancy>"]);
    assert_eq!(
        out.errors,
        [Problem::new("sunrise.guide.error.unknown_tag", "Fancy")]
    );
}

#[test]
fn scenes_are_framed_and_fit_the_page() {
    let (_, out) = laid(
        "<GameScene zoom=\"4\">\n<Block id=\"minecraft:stone\" />\n</GameScene>\n",
        400.0,
        vec![],
    );
    assert_eq!(out.scenes.len(), 1);
    let s = &out.scenes[0];
    assert_eq!(s.scale, 32.0);
    assert_eq!(s.center, [0.5, 0.5, 0.5]);
    // A unit cube turned 225° shows its diagonal: sqrt(2) blocks wide, plus 8 px each side.
    assert!(
        (s.rect[2] - (32.0 * 2f32.sqrt() + 16.0)).abs() < 0.01,
        "{:?}",
        s.rect
    );
    let (_, narrow) = laid(
        "<GameScene zoom=\"40\">\n<Block id=\"minecraft:stone\" />\n</GameScene>\n",
        100.0,
        vec![],
    );
    assert!(narrow.scenes[0].rect[2] <= 100.0);
}

#[test]
fn unclosed_tags_dont_break_the_page() {
    let (_, out) = laid("Text before\n\n<Row>\n\nText after\n", 400.0, vec![]);
    let all: Vec<String> = texts(&out).into_iter().map(|t| t.2).collect();
    assert_eq!(all, ["Text before", "<Row> is not closed", "Text after"]);
}
