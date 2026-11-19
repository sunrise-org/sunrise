//! A page laid out to a width (guides spec 9.5): text reflowed with its styles and links, item icons, images,
//! floats, tables, rows and columns, recipe and scene boxes. Pure: the game answers sizes and names through
//! [`Env`] and draws the [`Piece`]s.

use super::Problem;
use super::library::{Library, Target, qualify, requirement_met};
use super::markdown::{Align, Attrs, Block, Inline, Tag};
use super::scene::SceneModel;
use super::structure::State;
use std::collections::HashMap;

/// The height of a line of text at scale 1, and of the font's glyphs.
pub const LINE: f32 = 10.0;
const GLYPH: f32 = 8.0;
/// The gap after a block.
const GAP: f32 = 5.0;
pub const TEXT: u32 = 0xffe0_e0e0;
pub const ERROR: u32 = 0xffff_5555;
pub const LINK: u32 = 0xff55_aaff;
const CODE: u32 = 0xffe8_c07a;
const MUTED: u32 = 0xff80_8080;
/// A scene's pixels per block at `zoom` 1 (Q-036).
pub const SCENE_PX: f32 = 8.0;

/// What layout asks the game.
pub trait Env {
    /// A string's width in GUI pixels at scale 1.
    fn text_width(&self, text: &str, bold: bool) -> f32;
    /// An item's name; none when the game has no such item.
    fn item_name(&self, id: &str, components: &str) -> Option<String>;
    /// Whether the game has a block of that id.
    fn block_exists(&self, id: &str) -> bool;
    /// An image's size in pixels; none when it can't be read.
    fn image_size(&self, resource: &str) -> Option<[f32; 2]>;
    /// The key a key mapping (`key.jump`) is bound to, as the Controls screen names it.
    fn key_name(&self, mapping: &str) -> Option<String>;
    fn player_name(&self) -> String;
    fn installed(&self, mod_id: &str) -> bool;
    /// The sizes of the recipes a recipe tag shows, in order.
    fn recipes(&self, query: &RecipeQuery) -> Vec<[f32; 2]>;
    fn read(&self, resource: &str) -> Option<Vec<u8>>;
    /// A problem's text in the player's language.
    fn problem(&self, problem: &Problem) -> String {
        problem.english()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub scale: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            color: TEXT,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            scale: 1.0,
        }
    }
}

/// Which recipes a recipe tag shows (spec 5.14).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RecipeQuery {
    /// `RecipeFor`: the first recipe making the item.
    First(String),
    /// `RecipesFor`: every recipe making it.
    All(String),
    /// `Recipe`: the recipe with that id.
    Id(String),
}

/// What a click does.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Go(Target),
    /// Opens the item's page.
    Item {
        id: String,
        components: String,
    },
    Command {
        command: String,
        close: bool,
    },
}

/// What hovering shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Tip {
    /// The page's title.
    Page(usize),
    Missing(String),
    Hidden(String),
    External(String),
    Item {
        id: String,
        components: String,
    },
    Lines(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hotspot {
    pub rect: [f32; 4],
    pub action: Option<Action>,
    pub tip: Option<Tip>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Text {
        x: f32,
        y: f32,
        text: String,
        style: Style,
    },
    Fill {
        rect: [f32; 4],
        color: u32,
    },
    /// A slot's background (item grids).
    Slot {
        x: f32,
        y: f32,
    },
    /// An item stack drawn `size` pixels square.
    Item {
        x: f32,
        y: f32,
        size: f32,
        id: String,
        components: String,
    },
    Image {
        rect: [f32; 4],
        resource: String,
    },
    /// The `index`th recipe of `query`.
    Recipe {
        rect: [f32; 4],
        query: RecipeQuery,
        index: usize,
    },
    Scene {
        rect: [f32; 4],
        scene: usize,
    },
}

/// A scene box: the scene and how it is framed.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneBox {
    pub model: SceneModel,
    pub rect: [f32; 4],
    /// Pixels per block.
    pub scale: f32,
    /// The blocks' centre, the point the view turns about.
    pub center: [f32; 3],
}

/// A laid-out page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Laid {
    pub pieces: Vec<Piece>,
    pub hotspots: Vec<Hotspot>,
    pub anchors: HashMap<String, f32>,
    pub scenes: Vec<SceneBox>,
    pub height: f32,
    /// The widest point reached.
    pub width: f32,
    pub errors: Vec<Problem>,
}

#[derive(Clone, Copy, Debug)]
struct Float {
    left: bool,
    x0: f32,
    x1: f32,
    top: f32,
    bottom: f32,
}

/// One unit of a paragraph's flow.
#[derive(Clone, Debug)]
enum Atom {
    Word {
        text: String,
        style: Style,
        link: Option<usize>,
    },
    Space(Style),
    Box {
        w: f32,
        h: f32,
        piece: Piece,
        link: Option<usize>,
    },
    Break,
    Clear,
    Anchor(String),
}

struct Ctx<'a> {
    lib: &'a Library,
    page: usize,
    env: &'a dyn Env,
    namespace: &'a str,
    out: Laid,
    links: Vec<(Option<Action>, Option<Tip>)>,
    y: f32,
    floats: Vec<Float>,
}

/// Lays out page `page` of `lib`, `width` wide.
pub fn layout(lib: &Library, page: usize, env: &dyn Env, width: f32) -> Laid {
    let namespace = lib.guides[lib.pages[page].guide].namespace.as_str();
    let mut ctx = Ctx::new(lib, page, env, namespace);
    ctx.blocks(&lib.pages[page].doc.blocks, 0.0, width);
    ctx.finish()
}

fn translate(p: &mut Piece, dx: f32, dy: f32) {
    let r = |r: &mut [f32; 4]| {
        r[0] += dx;
        r[1] += dy;
    };
    match p {
        Piece::Text { x, y, .. } | Piece::Slot { x, y } | Piece::Item { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Piece::Fill { rect, .. }
        | Piece::Image { rect, .. }
        | Piece::Recipe { rect, .. }
        | Piece::Scene { rect, .. } => r(rect),
    }
}

/// The 16 Minecraft text colours, as the viewer's dark page shows them (Q-025).
fn named_color(name: &str) -> Option<u32> {
    Some(
        0xff00_0000
            | match name {
                "black" => 0x000000,
                "dark_blue" => 0x0000aa,
                "dark_green" => 0x00aa00,
                "dark_aqua" => 0x00aaaa,
                "dark_red" => 0xaa0000,
                "dark_purple" => 0xaa00aa,
                "gold" => 0xffaa00,
                "gray" => 0xaaaaaa,
                "dark_gray" => 0x555555,
                "blue" => 0x5555ff,
                "green" => 0x55ff55,
                "aqua" => 0x55ffff,
                "red" => 0xff5555,
                "light_purple" => 0xff55ff,
                "yellow" => 0xffff55,
                "white" => 0xffffff,
                _ => return None,
            },
    )
}

/// The size of a scene's blocks seen with `camera`'s yaw and pitch, `scale` pixels per block.
pub fn projected_size(min: [f32; 3], max: [f32; 3], camera: [f32; 3], scale: f32) -> [f32; 2] {
    let (yaw, pitch) = (camera[0].to_radians(), camera[1].to_radians());
    let c = [0, 1, 2].map(|k| (max[k] - min[k]) / 2.0);
    let mut w: f32 = 0.0;
    let mut h: f32 = 0.0;
    for sx in [-1.0, 1.0] {
        for sy in [-1.0, 1.0] {
            for sz in [-1.0, 1.0] {
                let (x, y, z) = (c[0] * sx, c[1] * sy, c[2] * sz);
                let px = x * yaw.cos() + z * yaw.sin();
                let depth = -x * yaw.sin() + z * yaw.cos();
                let py = y * pitch.cos() - depth * pitch.sin();
                w = w.max(px.abs());
                h = h.max(py.abs());
            }
        }
    }
    [w * 2.0 * scale, h * 2.0 * scale]
}

impl<'a> Ctx<'a> {
    fn new(lib: &'a Library, page: usize, env: &'a dyn Env, namespace: &'a str) -> Self {
        Ctx {
            lib,
            page,
            env,
            namespace,
            out: Laid::default(),
            links: Vec::new(),
            y: 0.0,
            floats: Vec::new(),
        }
    }

    fn sub(&self) -> Ctx<'a> {
        Ctx::new(self.lib, self.page, self.env, self.namespace)
    }

    fn finish(mut self) -> Laid {
        let floats = self.floats.iter().map(|f| f.bottom).fold(0.0, f32::max);
        self.out.height = (self.y - GAP).max(floats).max(0.0);
        self.out
    }

    /// Merges a sub-layout placed at (`dx`, `dy`).
    fn merge(&mut self, mut sub: Laid, dx: f32, dy: f32) {
        for mut p in sub.pieces.drain(..) {
            if let Piece::Scene { scene, .. } = &mut p {
                *scene += self.out.scenes.len();
            }
            translate(&mut p, dx, dy);
            self.out.pieces.push(p);
        }
        for mut s in sub.scenes.drain(..) {
            s.rect[0] += dx;
            s.rect[1] += dy;
            self.out.scenes.push(s);
        }
        for mut h in sub.hotspots.drain(..) {
            h.rect[0] += dx;
            h.rect[1] += dy;
            self.out.hotspots.push(h);
        }
        for (k, v) in sub.anchors {
            self.out.anchors.entry(k).or_insert(v + dy);
        }
        self.out.errors.append(&mut sub.errors);
        self.out.width = self.out.width.max(sub.width + dx);
    }

    fn reach(&mut self, right: f32) {
        self.out.width = self.out.width.max(right);
    }

    fn error(&mut self, problem: Problem) -> Vec<Atom> {
        let msg = self.env.problem(&problem);
        self.out.errors.push(problem);
        let style = Style {
            color: ERROR,
            ..Style::default()
        };
        words(&msg, style, None)
    }

    fn link(&mut self, action: Option<Action>, tip: Option<Tip>) -> usize {
        self.links.push((action, tip));
        self.links.len() - 1
    }

    fn text_width(&self, text: &str, style: &Style) -> f32 {
        self.env.text_width(text, style.bold) * style.scale
    }

    // ---- Floats ----

    /// The free span of a line at `y`, `h` tall, within `x`..`x + w`.
    fn span(&self, x: f32, w: f32, y: f32, h: f32) -> (f32, f32) {
        let mut left = x;
        let mut right = x + w;
        for f in &self.floats {
            if f.top < y + h && f.bottom > y {
                if f.left && f.x1 > left && f.x0 < x + w {
                    left = left.max(f.x1 + 4.0);
                } else if !f.left && f.x0 < right && f.x1 > x {
                    right = right.min(f.x0 - 4.0);
                }
            }
        }
        (left, right.max(left))
    }

    fn clear(&mut self) {
        let bottom = self.floats.iter().map(|f| f.bottom).fold(self.y, f32::max);
        self.y = bottom;
        self.floats.clear();
    }

    /// Places a box floating left or right at the current line.
    fn float(
        &mut self,
        left: bool,
        w: f32,
        h: f32,
        x: f32,
        width: f32,
        mut piece: Piece,
        link: Option<usize>,
    ) {
        let w = w.min(width);
        let (l, r) = self.span(x, width, self.y, h);
        let x0 = if left { l } else { (r - w).max(l) };
        translate(&mut piece, x0, self.y);
        self.push_box(piece, [x0, self.y, w, h], link);
        self.floats.push(Float {
            left,
            x0,
            x1: x0 + w,
            top: self.y,
            bottom: self.y + h,
        });
    }

    fn push_box(&mut self, piece: Piece, rect: [f32; 4], link: Option<usize>) {
        if let Piece::Scene { scene, .. } = piece {
            self.out.scenes[scene].rect = rect;
        }
        if let Some(l) = link {
            let (action, tip) = self.links[l].clone();
            self.out.hotspots.push(Hotspot { rect, action, tip });
        }
        self.out.pieces.push(piece);
        self.reach(rect[0] + rect[2]);
    }

    // ---- Blocks ----

    fn blocks(&mut self, blocks: &[Block], x: f32, w: f32) {
        for b in blocks {
            self.block(b, x, w);
        }
    }

    fn block(&mut self, b: &Block, x: f32, w: f32) {
        match b {
            Block::Heading {
                level,
                text,
                anchor,
            } => {
                if self.y > 0.0 {
                    self.y += 3.0;
                }
                self.out.anchors.entry(anchor.clone()).or_insert(self.y);
                let style = Style {
                    bold: true,
                    scale: if *level == 1 { 2.0 } else { 1.0 },
                    color: if *level <= 2 { 0xffff_ffff } else { TEXT },
                    ..Style::default()
                };
                let atoms = self.inlines(text, style, None);
                self.flow(&atoms, x, w);
                if *level <= 2 {
                    self.out.pieces.push(Piece::Fill {
                        rect: [x, self.y, w, 1.0],
                        color: MUTED,
                    });
                    self.y += 2.0;
                }
                self.y += GAP;
            }
            Block::Paragraph(inlines) => {
                let atoms = self.inlines(inlines, Style::default(), None);
                self.flow(&atoms, x, w);
                self.y += GAP;
            }
            Block::Quote(inner) => {
                let top = self.y;
                self.blocks(inner, x + 8.0, w - 8.0);
                let bottom = (self.y - GAP).max(top + LINE);
                self.out.pieces.push(Piece::Fill {
                    rect: [x + 1.0, top, 2.0, bottom - top],
                    color: MUTED,
                });
                self.y = bottom + GAP;
            }
            Block::List {
                ordered,
                start,
                items,
            } => {
                for (i, item) in items.iter().enumerate() {
                    let marker = if *ordered {
                        format!("{}.", *start as usize + i)
                    } else {
                        "\u{2022}".to_owned()
                    };
                    let style = Style::default();
                    let mw = self.text_width(&marker, &style);
                    let indent = (mw + 4.0).max(10.0);
                    let (left, _) = self.span(x, w, self.y, LINE);
                    self.out.pieces.push(Piece::Text {
                        x: left,
                        y: self.y,
                        text: marker,
                        style,
                    });
                    let top = self.y;
                    self.blocks(item, left + indent, w - (left - x) - indent);
                    // Tight lists: no gap between items.
                    self.y = (self.y - GAP).max(top + LINE);
                }
                self.y += GAP;
            }
            Block::Code(code) => {
                self.clear();
                let style = Style {
                    color: CODE,
                    ..Style::default()
                };
                let top = self.y;
                let first = self.out.pieces.len();
                let mut y = top + 3.0;
                for line in code.split('\n') {
                    // Long lines wrap at the page's width (Q-010).
                    let mut rest = line;
                    loop {
                        let mut fit = rest.len();
                        while fit > 0 && self.text_width(&rest[..fit], &style) > w - 6.0 {
                            fit = rest[..fit].char_indices().next_back().map_or(0, |(i, _)| i);
                        }
                        if fit == 0 && !rest.is_empty() {
                            fit = rest.chars().next().map_or(0, char::len_utf8);
                        }
                        self.out.pieces.push(Piece::Text {
                            x: x + 3.0,
                            y,
                            text: rest[..fit].to_owned(),
                            style,
                        });
                        y += LINE;
                        rest = &rest[fit..];
                        if rest.is_empty() {
                            break;
                        }
                    }
                }
                let h = y - top + 2.0;
                self.out.pieces.insert(
                    first,
                    Piece::Fill {
                        rect: [x, top, w, h],
                        color: 0x6000_0000,
                    },
                );
                self.reach(x + w);
                self.y = top + h + GAP;
            }
            Block::Rule => {
                self.out.pieces.push(Piece::Fill {
                    rect: [x, self.y + 2.0, w, 1.0],
                    color: MUTED,
                });
                self.y += 5.0 + GAP;
            }
            Block::Table { align, head, rows } => self.table(align, head, rows, x, w),
            Block::Tag(tag) => self.block_tag(tag, x, w),
            Block::Unclosed(name) => {
                let atoms =
                    self.error(Problem::new("sunrise.guide.error.not_closed", name.clone()));
                self.flow(&atoms, x, w);
                self.y += GAP;
            }
        }
    }

    fn table(
        &mut self,
        align: &[Align],
        head: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
        x: f32,
        w: f32,
    ) {
        self.clear();
        const PAD: f32 = 3.0;
        let n = head.len().max(1);
        let all: Vec<&[Vec<Inline>]> = std::iter::once(head)
            .chain(rows.iter().map(Vec::as_slice))
            .collect();
        let bold = Style {
            bold: true,
            ..Style::default()
        };
        // Natural widths, then shrunk in proportion when they don't fit.
        let mut natural = vec![0.0f32; n];
        for (r, row) in all.iter().enumerate() {
            for (c, cell) in row.iter().enumerate().take(n) {
                let mut sub = self.sub();
                let atoms = sub.inlines(cell, if r == 0 { bold } else { Style::default() }, None);
                sub.flow(&atoms, 0.0, f32::MAX / 4.0);
                natural[c] = natural[c].max(sub.out.width + 2.0 * PAD);
            }
        }
        let total: f32 = natural.iter().sum();
        let widths: Vec<f32> = if total <= w {
            natural
        } else {
            natural
                .iter()
                .map(|c| (c * w / total).max(2.0 * PAD + 6.0))
                .collect()
        };
        let table_w: f32 = widths.iter().sum();
        let top = self.y;
        let mut y = top;
        let mut lines = vec![y];
        for (r, row) in all.iter().enumerate() {
            let mut cx = x;
            let mut row_h: f32 = LINE;
            let mut cells = Vec::new();
            for c in 0..n {
                let mut sub = self.sub();
                if let Some(cell) = row.get(c) {
                    let atoms =
                        sub.inlines(cell, if r == 0 { bold } else { Style::default() }, None);
                    sub.flow(&atoms, 0.0, widths[c] - 2.0 * PAD);
                }
                let laid = sub.finish();
                row_h = row_h.max(laid.height);
                let shift = match align.get(c) {
                    Some(Align::Center) => ((widths[c] - 2.0 * PAD - laid.width) / 2.0).max(0.0),
                    Some(Align::Right) => (widths[c] - 2.0 * PAD - laid.width).max(0.0),
                    _ => 0.0,
                };
                cells.push((laid, cx + PAD + shift));
                cx += widths[c];
            }
            for (laid, cx) in cells {
                self.merge(laid, cx, y + PAD);
            }
            if r == 0 {
                self.out.pieces.push(Piece::Fill {
                    rect: [x, y, table_w, row_h + 2.0 * PAD],
                    color: 0x3000_0000,
                });
            }
            y += row_h + 2.0 * PAD;
            lines.push(y);
        }
        for &ly in &lines {
            self.out.pieces.push(Piece::Fill {
                rect: [x, ly, table_w, 1.0],
                color: MUTED,
            });
        }
        let mut cx = x;
        for c in 0..=n {
            self.out.pieces.push(Piece::Fill {
                rect: [cx, top, 1.0, y - top + 1.0],
                color: MUTED,
            });
            if c < n {
                cx += widths[c];
            }
        }
        self.reach(x + table_w + 1.0);
        self.y = y + 1.0 + GAP;
    }

    /// Lays out a group of blocks alone, `w` wide: its layout.
    fn alone(&self, blocks: &[Block], w: f32) -> Laid {
        let mut sub = self.sub();
        sub.blocks(blocks, 0.0, w);
        sub.finish()
    }

    fn block_tag(&mut self, tag: &Tag<Block>, x: f32, w: f32) {
        let a = &tag.attrs;
        match tag.name.as_str() {
            "OptionalSection" => match a.str("modId") {
                Some(req) => {
                    if requirement_met(req, &|m| self.env.installed(m)) {
                        self.blocks(&tag.children, x, w);
                    }
                }
                None => {
                    let atoms = self.error(Problem::new(
                        "sunrise.guide.error.no_attribute",
                        "OptionalSection modId",
                    ));
                    self.flow(&atoms, x, w);
                    self.y += GAP;
                }
            },
            "div" => {
                // Q-012: a class makes a note box; without one the children are a plain block.
                if a.str("class").is_some() {
                    let laid = self.alone(&tag.children, w - 12.0);
                    let top = self.y;
                    let h = laid.height + 12.0;
                    self.out.pieces.push(Piece::Fill {
                        rect: [x, top, w, h],
                        color: 0x4040_6080,
                    });
                    self.out.pieces.push(Piece::Fill {
                        rect: [x, top, 2.0, h],
                        color: LINK,
                    });
                    self.merge(laid, x + 8.0, top + 6.0);
                    self.y = top + h + GAP;
                } else {
                    self.blocks(&tag.children, x, w);
                }
            }
            "br" if a.get("clear").is_some() => self.clear(),
            "Row" | "Column" => self.row(tag, x, w),
            "ItemGrid" => {
                let icons: Vec<&Tag<Block>> = tag
                    .children
                    .iter()
                    .filter_map(|c| match c {
                        Block::Tag(t) if t.name == "ItemIcon" => Some(t),
                        _ => None,
                    })
                    .collect();
                let per_row = ((w / 18.0).floor() as usize).max(1);
                let top = self.y;
                for (i, icon) in icons.iter().enumerate() {
                    let cx = x + (i % per_row) as f32 * 18.0;
                    let cy = top + (i / per_row) as f32 * 18.0;
                    self.out.pieces.push(Piece::Slot { x: cx, y: cy });
                    if let Some((piece, link)) = self.item_box(&icon.attrs, 1.0) {
                        let mut piece = piece;
                        translate(&mut piece, cx + 1.0, cy + 1.0);
                        self.push_box(piece, [cx + 1.0, cy + 1.0, 16.0, 16.0], link);
                    }
                    self.reach(cx + 18.0);
                }
                self.y = top + icons.len().div_ceil(per_row) as f32 * 18.0 + GAP;
            }
            "CategoryIndex" | "SubPages" => {
                let list: Vec<usize> = if tag.name == "CategoryIndex" {
                    self.lib.category(a.str("category").unwrap_or(""))
                } else {
                    let of = a
                        .str("id")
                        .and_then(|id| self.lib.page(&qualify(id, self.namespace)))
                        .unwrap_or(self.page);
                    let mut kids = self.lib.pages[of].children.clone();
                    if a.flag("alphabetical").unwrap_or(false) {
                        kids.sort_by_key(|&p| self.lib.pages[p].title.to_lowercase());
                    }
                    kids
                };
                let icons = a.flag("icons").unwrap_or(false);
                for p in list {
                    let link = self.link(
                        Some(Action::Go(Target::Page {
                            page: p,
                            anchor: None,
                        })),
                        Some(Tip::Page(p)),
                    );
                    let style = Style {
                        color: LINK,
                        underline: true,
                        ..Style::default()
                    };
                    let mut atoms = vec![
                        Atom::Word {
                            text: "\u{2022}".into(),
                            style: Style::default(),
                            link: None,
                        },
                        Atom::Space(Style::default()),
                    ];
                    if icons && let Some((id, components)) = self.lib.pages[p].icon.clone() {
                        atoms.push(Atom::Box {
                            w: 16.0,
                            h: 16.0,
                            piece: Piece::Item {
                                x: 0.0,
                                y: 0.0,
                                size: 16.0,
                                id,
                                components,
                            },
                            link: Some(link),
                        });
                        atoms.push(Atom::Space(style));
                    }
                    atoms.extend(words(&self.lib.pages[p].title, style, Some(link)));
                    self.flow(&atoms, x, w);
                }
                self.y += GAP;
            }
            "RecipeFor" | "RecipesFor" | "Recipe" => {
                let atoms = self.recipes(tag);
                self.flow(&atoms, x, w);
                self.y += GAP;
            }
            "GameScene" => self.scene(tag, x, w),
            "ItemImage" | "ItemIcon" | "BlockImage" | "FloatingImage" => {
                let float = a
                    .str(if tag.name == "FloatingImage" {
                        "align"
                    } else {
                        "float"
                    })
                    .filter(|f| matches!(*f, "left" | "right"));
                let atoms = self.inline_tag_atoms(&tag.name, a, &[], Style::default(), None);
                match (float, atoms.into_iter().next()) {
                    (
                        Some(side),
                        Some(Atom::Box {
                            w: bw,
                            h,
                            piece,
                            link,
                        }),
                    ) => {
                        self.float(side == "left", bw, h, x, w, piece, link);
                    }
                    (_, Some(atom)) => {
                        self.flow(&[atom], x, w);
                        self.y += GAP;
                    }
                    (_, None) => {}
                }
            }
            "a" | "br" | "ItemLink" | "Color" | "KeyBind" | "PlayerName" | "CommandLink" => {
                let atoms = self.inline_tag_atoms(&tag.name, a, &[], Style::default(), None);
                self.flow(&atoms, x, w);
                self.y += GAP;
            }
            other => {
                // Unknown tags show an error and not their children (Q-013).
                let atoms = self.error(Problem::new("sunrise.guide.error.unknown_tag", other));
                self.flow(&atoms, x, w);
                self.y += GAP;
            }
        }
    }

    fn row(&mut self, tag: &Tag<Block>, x: f32, w: f32) {
        let a = &tag.attrs;
        let column = tag.name == "Column";
        let gap = a.num("gap").unwrap_or(5.0).max(0.0);
        let align = a.str("alignItems").unwrap_or("start");
        let full = a.flag("fullWidth").unwrap_or(false);
        let children: Vec<Laid> = tag
            .children
            .iter()
            .map(|c| self.alone(std::slice::from_ref(c), w))
            .filter(|l| !l.pieces.is_empty())
            .collect();
        let top = self.y;
        if column {
            let width = if full {
                w
            } else {
                children.iter().map(|c| c.width).fold(0.0, f32::max)
            };
            let mut y = top;
            for c in children {
                let dx = match align {
                    "center" => ((width - c.width) / 2.0).max(0.0),
                    "end" => (width - c.width).max(0.0),
                    _ => 0.0,
                };
                let h = c.height;
                self.merge(c, x + dx, y);
                y += h + gap;
            }
            self.y = y - gap + GAP;
            return;
        }
        // Rows: side by side, wrapping; each line aligned across.
        let mut lines: Vec<Vec<Laid>> = vec![Vec::new()];
        let mut cx = 0.0;
        for c in children {
            let line = lines.last_mut().expect("one line");
            if !line.is_empty() && cx + c.width > w {
                lines.push(Vec::new());
                cx = 0.0;
            }
            cx += c.width + gap;
            lines.last_mut().expect("one line").push(c);
        }
        let mut y = top;
        for line in lines {
            let line_w: f32 = line.iter().map(|c| c.width).sum::<f32>()
                + gap * line.len().saturating_sub(1) as f32;
            let line_h = line.iter().map(|c| c.height).fold(0.0, f32::max);
            let mut cx = if full {
                x + ((w - line_w) / 2.0).max(0.0)
            } else {
                x
            };
            for c in line {
                let dy = match align {
                    "center" => (line_h - c.height) / 2.0,
                    "end" => line_h - c.height,
                    _ => 0.0,
                };
                let cw = c.width;
                self.merge(c, cx, y + dy);
                cx += cw + gap;
            }
            y += line_h + gap;
        }
        self.y = y - gap + GAP;
    }

    fn recipes(&mut self, tag: &Tag<Block>) -> Vec<Atom> {
        let a = &tag.attrs;
        let id = a
            .str("id")
            .map(|i| qualify(i.trim(), self.namespace))
            .unwrap_or_default();
        let query = match tag.name.as_str() {
            "RecipeFor" => RecipeQuery::First(id.clone()),
            "RecipesFor" => RecipeQuery::All(id.clone()),
            _ => RecipeQuery::Id(id.clone()),
        };
        let sizes = self.env.recipes(&query);
        if sizes.is_empty() {
            return match a.str("fallbackText") {
                Some(text) => words(text, Style::default(), None),
                None => self.error(Problem::new("sunrise.guide.error.no_recipe", id)),
            };
        }
        let mut atoms = Vec::new();
        for (index, [w, h]) in sizes.into_iter().enumerate() {
            if index > 0 {
                atoms.push(Atom::Space(Style {
                    scale: 0.5,
                    ..Style::default()
                }));
            }
            atoms.push(Atom::Box {
                w,
                h,
                piece: Piece::Recipe {
                    rect: [0.0, 0.0, w, h],
                    query: query.clone(),
                    index,
                },
                link: None,
            });
        }
        atoms
    }

    fn scene(&mut self, tag: &Tag<Block>, x: f32, w: f32) {
        let lib = self.lib;
        let page = self.page;
        let env = self.env;
        let model = SceneModel::build(tag, self.namespace, &|src| env.read(&lib.asset(page, src)?));
        self.scene_box(model, x, w, None, None);
    }

    /// Places a scene's box as a block (or a float on `side`), framing its blocks.
    fn scene_box(
        &mut self,
        model: SceneModel,
        x: f32,
        w: f32,
        side: Option<bool>,
        link: Option<usize>,
    ) {
        let (piece, bw, bh) = self.scene_piece(model, w);
        match side {
            Some(left) => self.float(left, bw, bh, x, w, piece, link),
            None => {
                self.clear();
                let mut piece = piece;
                translate(&mut piece, x, self.y);
                let rect = [x, self.y, bw, bh];
                if let Piece::Scene { scene, .. } = piece {
                    self.out.scenes[scene].rect = rect;
                }
                self.push_box(piece, rect, link);
                self.y += bh + GAP;
            }
        }
    }

    /// A scene's piece at the origin and its size.
    fn scene_piece(&mut self, model: SceneModel, w: f32) -> (Piece, f32, f32) {
        for e in &model.errors {
            self.out.errors.push(e.clone());
        }
        let (min, max) = model.bounds();
        let mut scale = SCENE_PX * model.zoom;
        let mut size = projected_size(min, max, model.camera, scale);
        const PAD: f32 = 8.0;
        if size[0] + 2.0 * PAD > w {
            let k = ((w - 2.0 * PAD) / size[0]).max(0.05);
            scale *= k;
            size = [size[0] * k, size[1] * k];
        }
        let (bw, bh) = (
            (size[0] + 2.0 * PAD).min(w),
            (size[1] + 2.0 * PAD).max(32.0),
        );
        let center = [0, 1, 2].map(|k| (min[k] + max[k]) / 2.0);
        let index = self.out.scenes.len();
        self.out.scenes.push(SceneBox {
            model,
            rect: [0.0, 0.0, bw, bh],
            scale,
            center,
        });
        (
            Piece::Scene {
                rect: [0.0, 0.0, bw, bh],
                scene: index,
            },
            bw,
            bh,
        )
    }

    /// An item icon `16 * scale` pixels square at the origin, with its tooltip and page link; none and an error
    /// for an unknown item.
    fn item_box(&mut self, a: &Attrs, scale: f32) -> Option<(Piece, Option<usize>)> {
        let id = qualify(a.str("id")?.trim(), self.namespace);
        let components = a.str("components").unwrap_or("").to_owned();
        self.env.item_name(&id, &components)?;
        let link = self.link(
            Some(Action::Item {
                id: id.clone(),
                components: components.clone(),
            }),
            Some(Tip::Item {
                id: id.clone(),
                components: components.clone(),
            }),
        );
        Some((
            Piece::Item {
                x: 0.0,
                y: 0.0,
                size: 16.0 * scale,
                id,
                components,
            },
            Some(link),
        ))
    }

    // ---- Inlines ----

    fn inlines(&mut self, inlines: &[Inline], style: Style, link: Option<usize>) -> Vec<Atom> {
        let mut atoms = Vec::new();
        for i in inlines {
            match i {
                Inline::Text(t) => atoms.extend(words(t, style, link)),
                Inline::Code(c) => atoms.extend(words(
                    c,
                    Style {
                        color: CODE,
                        ..style
                    },
                    link,
                )),
                Inline::Strong(c) => {
                    let inner = self.inlines(
                        c,
                        Style {
                            bold: true,
                            ..style
                        },
                        link,
                    );
                    atoms.extend(inner);
                }
                Inline::Emph(c) => {
                    let inner = self.inlines(
                        c,
                        Style {
                            italic: true,
                            ..style
                        },
                        link,
                    );
                    atoms.extend(inner);
                }
                Inline::Strike(c) => {
                    let inner = self.inlines(
                        c,
                        Style {
                            strike: true,
                            ..style
                        },
                        link,
                    );
                    atoms.extend(inner);
                }
                Inline::Link { target, children } => {
                    let (l, s) = self.target_link(target, style);
                    let inner = self.inlines(children, s, Some(l));
                    atoms.extend(inner);
                }
                Inline::Image { src, alt } => atoms.extend(self.image(src, alt, link)),
                Inline::Break => atoms.push(Atom::Break),
                Inline::Tag(t) => {
                    let inner = self.inline_tag_atoms(&t.name, &t.attrs, &t.children, style, link);
                    atoms.extend(inner);
                }
            }
        }
        atoms
    }

    /// A link to `target` and the style its text takes.
    fn target_link(&mut self, target: &str, style: Style) -> (usize, Style) {
        let resolved = self.lib.resolve(self.page, target);
        let (tip, color) = match &resolved {
            Target::Page { page, .. } => (Tip::Page(*page), LINK),
            Target::External(url) => (Tip::External(url.clone()), LINK),
            Target::Missing(id) => {
                self.out
                    .errors
                    .push(Problem::new("sunrise.guide.error.missing_page", id.clone()));
                (Tip::Missing(id.clone()), ERROR)
            }
            Target::Hidden { mod_id, .. } => (Tip::Hidden(mod_id.clone()), ERROR),
        };
        let action = matches!(resolved, Target::Page { .. } | Target::External(_))
            .then_some(Action::Go(resolved));
        let l = self.link(action, Some(tip));
        (
            l,
            Style {
                color,
                underline: color == LINK,
                ..style
            },
        )
    }

    fn image(&mut self, src: &str, alt: &str, link: Option<usize>) -> Vec<Atom> {
        let resource = self.lib.asset(self.page, src);
        let size = resource.as_ref().and_then(|r| self.env.image_size(r));
        let (Some(resource), Some([w, h])) = (resource, size) else {
            let shown = if alt.is_empty() { src } else { alt };
            return self.error(Problem::new("sunrise.guide.error.missing_image", shown));
        };
        let link = link.or_else(|| {
            (!alt.is_empty()).then(|| self.link(None, Some(Tip::Lines(vec![alt.to_owned()]))))
        });
        vec![Atom::Box {
            w,
            h,
            piece: Piece::Image {
                rect: [0.0, 0.0, w, h],
                resource,
            },
            link,
        }]
    }

    fn inline_tag_atoms(
        &mut self,
        name: &str,
        a: &Attrs,
        children: &[Inline],
        style: Style,
        link: Option<usize>,
    ) -> Vec<Atom> {
        match name {
            "ItemLink" => {
                let Some(id) = a.str("id").map(|i| qualify(i.trim(), self.namespace)) else {
                    return self.error(Problem::new(
                        "sunrise.guide.error.no_attribute",
                        "ItemLink id",
                    ));
                };
                let components = a.str("components").unwrap_or("").to_owned();
                let Some(name) = self.env.item_name(&id, &components) else {
                    return match a.str("fallback") {
                        Some(f) => words(f, style, link),
                        None => self.error(Problem::new("sunrise.guide.error.unknown_item", id)),
                    };
                };
                let own = id.split(':').next() == Some(self.namespace);
                let optional = a.flag("optional").unwrap_or(!own);
                let tip = Tip::Item {
                    id: id.clone(),
                    components: components.clone(),
                };
                match self.lib.item_page(&id) {
                    Some(p) if p == self.page => {
                        let l = self.link(None, Some(tip));
                        words(
                            &name,
                            Style {
                                bold: true,
                                ..style
                            },
                            Some(l),
                        )
                    }
                    Some(_) => {
                        let l = self.link(Some(Action::Item { id, components }), Some(tip));
                        words(
                            &name,
                            Style {
                                color: LINK,
                                underline: true,
                                ..style
                            },
                            Some(l),
                        )
                    }
                    None if optional => {
                        let l = self.link(None, Some(tip));
                        words(&name, style, Some(l))
                    }
                    None => self.error(Problem::new("sunrise.guide.error.no_page", id)),
                }
            }
            "Color" => {
                let color = match (a.str("color"), a.str("id")) {
                    (Some(c), _) => super::scene::color(c),
                    (None, Some(id)) => {
                        let id = id.trim();
                        named_color(id).or_else(|| {
                            let guide = &self.lib.guides[self.lib.pages[self.page].guide];
                            guide
                                .colors
                                .get(&qualify(id, self.namespace))
                                .map(|c| 0xff00_0000 | c.0)
                        })
                    }
                    (None, None) => None,
                };
                match color {
                    Some(color) => self.inlines(children, Style { color, ..style }, link),
                    None => self.error(Problem::new(
                        "sunrise.guide.error.unknown_color",
                        a.str("color").or(a.str("id")).unwrap_or(""),
                    )),
                }
            }
            "KeyBind" => {
                let id = a.str("id").unwrap_or("");
                match self.env.key_name(id) {
                    Some(key) => words(
                        &key,
                        Style {
                            bold: true,
                            ..style
                        },
                        link,
                    ),
                    None => self.error(Problem::new("sunrise.guide.error.unknown_key", id)),
                }
            }
            "PlayerName" => words(&self.env.player_name(), style, link),
            "CommandLink" => {
                let command = a.str("command").unwrap_or("").trim().to_owned();
                if !command.starts_with('/') {
                    return self.error(Problem::new("sunrise.guide.error.bad_command", command));
                }
                let mut tip = Vec::new();
                if let Some(title) = a.str("title") {
                    tip.push(title.to_owned());
                }
                tip.push(command.clone());
                let l = self.link(
                    Some(Action::Command {
                        command,
                        close: a.flag("close").unwrap_or(false),
                    }),
                    Some(Tip::Lines(tip)),
                );
                self.inlines(
                    children,
                    Style {
                        color: LINK,
                        underline: true,
                        ..style
                    },
                    Some(l),
                )
            }
            "a" => {
                let mut atoms = Vec::new();
                if let Some(anchor) = a.str("name") {
                    atoms.push(Atom::Anchor(anchor.to_owned()));
                }
                match a.str("href") {
                    Some(href) => {
                        let (l, s) = self.target_link(href, style);
                        atoms.extend(self.inlines(children, s, Some(l)));
                    }
                    None => atoms.extend(self.inlines(children, style, link)),
                }
                atoms
            }
            "br" => vec![if a.get("clear").is_some() {
                Atom::Clear
            } else {
                Atom::Break
            }],
            "ItemImage" | "ItemIcon" => {
                let scale = if name == "ItemImage" {
                    a.num("scale").filter(|s| *s > 0.0).unwrap_or(1.0)
                } else {
                    1.0
                };
                match self.item_box(a, scale) {
                    Some((piece, l)) => vec![Atom::Box {
                        w: 16.0 * scale,
                        h: 16.0 * scale,
                        piece,
                        link: l,
                    }],
                    None => {
                        let id = a.str("id").unwrap_or("");
                        self.error(Problem::new("sunrise.guide.error.unknown_item", id))
                    }
                }
            }
            "BlockImage" => {
                let Some(id) = a.str("id").map(|i| qualify(i.trim(), self.namespace)) else {
                    return self.error(Problem::new(
                        "sunrise.guide.error.no_attribute",
                        "BlockImage id",
                    ));
                };
                if !self.env.block_exists(&id) {
                    return self.error(Problem::new("sunrise.guide.error.unknown_block", id));
                }
                let scale = a.num("scale").filter(|s| *s > 0.0).unwrap_or(1.0);
                let mut state = State::new(&id);
                for (k, v) in a.props() {
                    state.set(&k, &v);
                }
                let yaw = match a.str("perspective") {
                    Some("isometric-north-west") => 135.0,
                    _ => 225.0,
                };
                let mut model = SceneModel {
                    blocks: Default::default(),
                    entities: Vec::new(),
                    camera: [yaw, 30.0, 0.0],
                    zoom: scale * 16.0 / SCENE_PX / 1.6,
                    background: 0,
                    interactive: false,
                    annotations: Vec::new(),
                    errors: Vec::new(),
                };
                model.blocks.insert([0, 0, 0], (state, None));
                // Q-021: the block's item tooltip, when it has an item.
                let l = self.env.item_name(&id, "").map(|_| {
                    self.link(
                        Some(Action::Item {
                            id: id.clone(),
                            components: String::new(),
                        }),
                        Some(Tip::Item {
                            id: id.clone(),
                            components: String::new(),
                        }),
                    )
                });
                let (piece, w, h) = self.scene_piece(model, f32::MAX);
                vec![Atom::Box {
                    w,
                    h,
                    piece,
                    link: l,
                }]
            }
            "FloatingImage" => match a.str("src") {
                Some(src) => {
                    let alt = a.str("title").or(a.str("alt")).unwrap_or("").to_owned();
                    self.image(src, &alt, link)
                }
                None => self.error(Problem::new(
                    "sunrise.guide.error.no_attribute",
                    "FloatingImage src",
                )),
            },
            other => self.error(Problem::new("sunrise.guide.error.unknown_tag", other)),
        }
    }

    // ---- Flow ----

    fn atom_size(&self, atom: &Atom) -> (f32, f32) {
        match atom {
            Atom::Word { text, style, .. } => (self.text_width(text, style), LINE * style.scale),
            Atom::Space(style) => (self.text_width(" ", style), LINE * style.scale),
            Atom::Box { w, h, .. } => (*w, *h),
            _ => (0.0, 0.0),
        }
    }

    /// Lays atoms out in lines from the current `y`, within `x`..`x + w` and around floats.
    fn flow(&mut self, atoms: &[Atom], x: f32, w: f32) {
        let mut i = 0;
        let mut pending_anchors: Vec<String> = Vec::new();
        while i < atoms.len() {
            // A line: as many atoms as fit.
            let mut line_h = LINE;
            let (mut left, mut right) = self.span(x, w, self.y, line_h);
            let mut line: Vec<(Atom, f32)> = Vec::new();
            let mut cx = left;
            let mut ended = false;
            while i < atoms.len() {
                let atom = &atoms[i];
                match atom {
                    Atom::Break => {
                        i += 1;
                        ended = true;
                        break;
                    }
                    Atom::Clear => {
                        if line.is_empty() {
                            i += 1;
                            self.clear();
                            (left, right) = self.span(x, w, self.y, line_h);
                            cx = left;
                            continue;
                        }
                        // Ends this line; the next one clears.
                        ended = true;
                        break;
                    }
                    Atom::Anchor(name) => {
                        pending_anchors.push(name.clone());
                        i += 1;
                        continue;
                    }
                    Atom::Space(_) if line.is_empty() => {
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
                let (aw, ah) = self.atom_size(atom);
                if ah > line_h {
                    // A taller line may meet a float the shorter one didn't.
                    let (l2, r2) = self.span(x, w, self.y, ah);
                    if line.is_empty() {
                        (left, right) = (l2, r2);
                        cx = left;
                    } else if l2 > left || r2 < right {
                        break;
                    }
                }
                if cx + aw > right + 0.01 && !line.is_empty() {
                    break;
                }
                if cx + aw > right + 0.01 && line.is_empty() {
                    if let Atom::Word { text, style, link } = atom
                        && text.chars().count() > 1
                    {
                        // Too wide for any line: split it.
                        let mut fit = text.len();
                        while fit > 0 && self.text_width(&text[..fit], style) > right - cx {
                            fit = text[..fit].char_indices().next_back().map_or(0, |(k, _)| k);
                        }
                        let fit = fit.max(text.chars().next().map_or(1, char::len_utf8));
                        let head = Atom::Word {
                            text: text[..fit].to_owned(),
                            style: *style,
                            link: *link,
                        };
                        let rest = Atom::Word {
                            text: text[fit..].to_owned(),
                            style: *style,
                            link: *link,
                        };
                        line.push((head, cx));
                        line_h = line_h.max(ah);
                        let mut tail = vec![rest];
                        tail.extend_from_slice(&atoms[i + 1..]);
                        self.place_line(&line, line_h, &mut pending_anchors);
                        self.flow(&tail, x, w);
                        return;
                    }
                    if let Some(f) = self
                        .floats
                        .iter()
                        .filter(|f| f.bottom > self.y)
                        .map(|f| f.bottom)
                        .reduce(f32::min)
                        && right - left < w
                    {
                        // Below the float that narrows the line.
                        self.y = f;
                        (left, right) = self.span(x, w, self.y, line_h);
                        cx = left;
                        continue;
                    }
                }
                line_h = line_h.max(ah);
                line.push((atom.clone(), cx));
                cx += aw;
                i += 1;
            }
            // Spaces at the end of a line are not drawn.
            while matches!(line.last(), Some((Atom::Space(_), _))) {
                line.pop();
            }
            if line.is_empty() && !ended {
                if !pending_anchors.is_empty() {
                    self.place_line(&line, 0.0, &mut pending_anchors);
                }
                continue;
            }
            self.place_line(&line, line_h, &mut pending_anchors);
        }
        if !pending_anchors.is_empty() {
            self.place_line(&[], 0.0, &mut pending_anchors);
        }
    }

    /// Emits one line's pieces at the current `y` and moves down.
    fn place_line(&mut self, line: &[(Atom, f32)], h: f32, anchors: &mut Vec<String>) {
        let y = self.y;
        for name in anchors.drain(..) {
            self.out.anchors.entry(name).or_insert(y);
        }
        // Text runs: neighbours with the same style and link become one piece.
        let mut run: Option<(String, f32, Style, Option<usize>, f32)> = None;
        let mut hot: Vec<(Option<usize>, [f32; 4])> = Vec::new();
        let flush = |ctx: &mut Self, run: &mut Option<(String, f32, Style, Option<usize>, f32)>| {
            if let Some((text, x0, style, _, _)) = run.take() {
                let th = GLYPH * style.scale;
                let ty = y + ((h - th) / 2.0).floor().max(0.0);
                ctx.out.pieces.push(Piece::Text {
                    x: x0,
                    y: ty,
                    text,
                    style,
                });
            }
        };
        for (atom, ax) in line {
            let (aw, ah) = self.atom_size(atom);
            let word = match atom {
                Atom::Word { text, style, link } => Some((text.as_str(), *style, *link)),
                Atom::Space(style) => Some((" ", *style, run.as_ref().and_then(|r| r.3))),
                _ => None,
            };
            if let Some((text, style, link)) = word {
                let same = run.as_ref().is_some_and(|r| r.2 == style && r.3 == link);
                if !same {
                    flush(self, &mut run);
                    run = Some((String::new(), *ax, style, link, *ax));
                }
                let r = run.as_mut().expect("run");
                r.0.push_str(text);
                r.4 = ax + aw;
                // Words make hotspots; a space only joins its neighbours' runs (and may carry the run's link).
                if link.is_some() && matches!(atom, Atom::Word { .. }) {
                    hot.push((link, [*ax, y, aw, h]));
                }
                self.reach(ax + aw);
                continue;
            }
            if let Atom::Box { piece, link, .. } = atom {
                flush(self, &mut run);
                let by = y + ((h - ah) / 2.0).floor().max(0.0);
                let mut piece = piece.clone();
                translate(&mut piece, *ax, by);
                if let Piece::Scene { scene, rect } = &piece {
                    self.out.scenes[*scene].rect = *rect;
                }
                self.out.pieces.push(piece);
                if link.is_some() {
                    hot.push((*link, [*ax, by, aw, ah]));
                }
            }
            self.reach(ax + aw);
        }
        flush(self, &mut run);
        // One hotspot per link per line, over its atoms.
        let mut merged: Vec<(usize, [f32; 4])> = Vec::new();
        for (link, r) in hot {
            let Some(l) = link else { continue };
            match merged.last_mut() {
                Some((ml, mr)) if *ml == l && (mr[0] + mr[2] - r[0]).abs() < 8.0 => {
                    let right = (r[0] + r[2]).max(mr[0] + mr[2]);
                    let top = mr[1].min(r[1]);
                    let bottom = (mr[1] + mr[3]).max(r[1] + r[3]);
                    *mr = [mr[0], top, right - mr[0], bottom - top];
                }
                _ => merged.push((l, r)),
            }
        }
        for (l, rect) in merged {
            let (action, tip) = self.links[l].clone();
            self.out.hotspots.push(Hotspot { rect, action, tip });
        }
        self.y += h;
    }
}

/// Text split into words and spaces.
fn words(text: &str, style: Style, link: Option<usize>) -> Vec<Atom> {
    let mut atoms = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        // A no-break space stays inside its word.
        if c.is_whitespace() && c != '\u{a0}' {
            if !word.is_empty() {
                atoms.push(Atom::Word {
                    text: std::mem::take(&mut word),
                    style,
                    link,
                });
            }
            if !matches!(atoms.last(), Some(Atom::Space(_))) {
                atoms.push(Atom::Space(style));
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        atoms.push(Atom::Word {
            text: word,
            style,
            link,
        });
    }
    atoms
}
