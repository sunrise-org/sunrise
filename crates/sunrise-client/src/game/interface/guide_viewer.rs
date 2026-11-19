//! The guide viewer (#2664; docs/cleanroom/guides-spec.md section 9): every installed mod's guide in one
//! screen, opened with W over an item on a container screen, the item panel or a guide page. It draws over the
//! container screen it was opened from, which stays open underneath and is back when the viewer closes.
//!
//! The pages, their layout and search are `sunrise_client::guides`; this module reads the resources for them,
//! draws the laid-out page (text, items, images, recipes with the item panel's recipe drawing, scenes with the
//! living-scene drawing) and takes the viewer's input. A Sunrise extra: built only with `sunrise-extras`.

use super::Game;
use super::Screen;
use super::native_hooks::{
    NativeRecipe, NativeRecipeKind, NativeRecipeLayout, NativeRecipeResult, NativeRecipeView,
    NativeRecipes,
};
use super::native_panel_recipes::Shown;
use super::native_scene::{SceneBlock, scene_matrix};
use mod_api::{SceneClock, SceneView};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use sunrise_client::guides::{
    Problem,
    layout::{self, Action, Env, Laid, Piece, RecipeQuery, Tip},
    library::{Entry, Library, Sources, Target},
    scene::Shape,
    search::{self, Hit},
};
use sunrise_client::hud::{HudBatch, rgba};
use sunrise_client::items::ItemStack;
use sunrise_client::ui::EditBox;
use sunrise_world::registry::BlockStateId;
use winit::event::{ElementState, KeyEvent, MouseButton};
use winit::keyboard::{KeyCode, PhysicalKey};

/// The dynamic textures' owner for guide images.
const IMAGES: &str = "sunrise_guide";
const ROW: f32 = 12.0;
const RESULT_ROW: f32 = 22.0;

/// The viewer's state: the library, the open view, and the history and open sidebar entries, which last while
/// the game runs (Q-046).
#[derive(Default)]
pub(super) struct Guides {
    library: Option<Arc<Library>>,
    /// The language the library was read in.
    language: String,
    view: Option<Box<View>>,
    back: Vec<(String, f32)>,
    forward: Vec<(String, f32)>,
    open: HashSet<String>,
    /// The page shown last, by id: the page kept across a reload.
    last: Option<String>,
}

struct View {
    page: usize,
    scroll: f32,
    sidebar_scroll: f32,
    laid: Laid,
    /// The page column's width the page was laid out for.
    width: f32,
    recipes: HashMap<RecipeQuery, Vec<Shown>>,
    scenes: Vec<SceneDraw>,
    /// Image resources to the sprite they were loaded as and its pixel size; none when it couldn't be.
    images: HashMap<String, Option<String>>,
    search: EditBox,
    hits: Vec<Hit>,
    since: Instant,
    drag: Option<Drag>,
}

struct SceneDraw {
    blocks: Vec<([i32; 3], BlockStateId)>,
    view: SceneView,
    /// The page's camera, for the reset control.
    home: SceneView,
    clock: u32,
    background: u32,
    interactive: bool,
    /// The problem shown in its box instead of the scene (a limit passed).
    refused: Option<Problem>,
    annotations: Vec<sunrise_client::guides::scene::Annotation>,
}

#[derive(Clone, Copy)]
enum Drag {
    Scrollbar,
    Scene { index: usize, last: (f32, f32) },
}

/// Where the viewer's parts are on a screen `w` by `h` GUI pixels.
struct Frame {
    back: [f32; 4],
    forward: [f32; 4],
    search: [f32; 4],
    close: [f32; 4],
    side: [f32; 4],
    page: [f32; 4],
}

fn frame(w: f32, h: f32) -> Frame {
    let m = 8.0;
    let side_w = (w * 0.3).clamp(90.0, 170.0);
    let page_x = m + side_w + 10.0;
    Frame {
        back: [m, 6.0, 14.0, 14.0],
        forward: [m + 16.0, 6.0, 14.0, 14.0],
        search: [m + 34.0, 6.0, side_w - 34.0, 14.0],
        close: [w - m - 14.0, 6.0, 14.0, 14.0],
        side: [m, 26.0, side_w, h - 26.0 - m],
        page: [page_x, 26.0, (w - page_x - m - 8.0).max(40.0), h - 26.0 - m],
    }
}

fn inside(p: (f32, f32), r: [f32; 4]) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

/// `0xAARRGGBB` as the batch's colour.
fn argb(c: u32) -> u32 {
    let [a, r, g, b] = c.to_be_bytes();
    rgba(r, g, b, a)
}

/// A PNG's size from its header.
fn png_size(bytes: &[u8]) -> Option<[f32; 2]> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some([w as f32, h as f32])
}

/// A recipe of the client's tables as the item panel's recipe view.
fn recipe_view(tables: &NativeRecipes, r: &NativeRecipe) -> NativeRecipeView {
    let stack = |s: &super::native_hooks::NativeRecipeStack| -> Option<mod_api::Stack> {
        let id = &tables.items.get(s.item as usize)?.id;
        Some(mod_api::Stack {
            item: mod_api::__private::item_key(id)?,
            count: s.count.max(1),
            components: s.components.clone(),
        })
    };
    let slots: Vec<Vec<mod_api::Stack>> = r
        .slots
        .iter()
        .map(|s| s.iter().filter_map(stack).collect())
        .collect();
    let (layout, width, height) = match r.kind {
        NativeRecipeKind::CraftingShaped => {
            (NativeRecipeLayout::Grid, r.width.max(1), r.height.max(1))
        }
        NativeRecipeKind::CraftingShapeless => {
            let n = slots.len() as u32;
            let w = match n {
                0 | 1 => 1,
                2..=4 => 2,
                _ => 3,
            };
            (NativeRecipeLayout::Grid, w, n.div_ceil(w).max(1))
        }
        NativeRecipeKind::Smelting
        | NativeRecipeKind::Blasting
        | NativeRecipeKind::Smoking
        | NativeRecipeKind::Campfire => (NativeRecipeLayout::Cooking, 1, 1),
        NativeRecipeKind::Stonecutting => (NativeRecipeLayout::Single, 1, 1),
        NativeRecipeKind::Smithing => (NativeRecipeLayout::Smithing, 1, 1),
        NativeRecipeKind::Brewing => (NativeRecipeLayout::Brewing, 1, 1),
    };
    NativeRecipeView {
        layout,
        width,
        height,
        slots,
        results: vec![NativeRecipeResult {
            stacks: r.results.iter().filter_map(stack).collect(),
            chance: None,
        }],
        fluid_inputs: Vec::new(),
        fluid_outputs: Vec::new(),
        note: String::new(),
        action: None,
    }
}

/// What layout asks, answered by the game.
struct GameEnv<'g, 'w> {
    game: &'g Game<'w>,
    tables: Option<Arc<NativeRecipes>>,
    room: f32,
    installed: HashSet<String>,
    recipes: RefCell<HashMap<RecipeQuery, Vec<Shown>>>,
}

impl Env for GameEnv<'_, '_> {
    fn text_width(&self, text: &str, bold: bool) -> f32 {
        let w = HudBatch::text_width(&self.game.gui.atlas, text);
        if bold {
            w + text.chars().count() as f32
        } else {
            w
        }
    }
    fn item_name(&self, id: &str, components: &str) -> Option<String> {
        let stack = self.game.guide_stack(id, components)?;
        Some(sunrise_client::stations::hover_name(
            &self.game.pack.names,
            &stack,
        ))
    }
    fn block_exists(&self, id: &str) -> bool {
        self.game.pack.assets.registry.default_state(id).is_some()
    }
    fn image_size(&self, resource: &str) -> Option<[f32; 2]> {
        // Q-017: PNG; a JPEG shows its alt text for now.
        png_size(&self.read(resource)?)
    }
    fn key_name(&self, mapping: &str) -> Option<String> {
        let bound = self.game.settings.keys.get(mapping);
        (bound != "key.keyboard.unknown")
            .then(|| sunrise_client::keybinds::display_name(bound).plain())
    }
    fn player_name(&self) -> String {
        self.game.own_profile_name()
    }
    fn installed(&self, mod_id: &str) -> bool {
        self.installed.contains(mod_id)
    }
    fn recipes(&self, query: &RecipeQuery) -> Vec<[f32; 2]> {
        let Some(tables) = &self.tables else {
            return Vec::new();
        };
        let makes = |r: &&NativeRecipe, item: &str| {
            r.results.iter().any(|s| {
                tables
                    .items
                    .get(s.item as usize)
                    .is_some_and(|i| i.id == item)
            })
        };
        // Q-026: the first in the recipe book's order, then campfire, stonecutting, smithing, brewing.
        let found: Vec<&NativeRecipe> = match query {
            RecipeQuery::First(item) => tables
                .recipes
                .iter()
                .filter(|r| makes(r, item.as_str()))
                .take(1)
                .collect(),
            RecipeQuery::All(item) => tables
                .recipes
                .iter()
                .filter(|r| makes(r, item.as_str()))
                .collect(),
            RecipeQuery::Id(id) => tables
                .recipes
                .iter()
                .filter(|r| r.id == *id)
                .take(1)
                .collect(),
        };
        let items = self.game.items();
        let shown: Vec<Shown> = found
            .into_iter()
            .map(|r| Shown::new(items, &recipe_view(tables, r)))
            .collect();
        let sizes = shown
            .iter()
            .map(|s| {
                let (w, h) = s.size(self.room);
                [w, h]
            })
            .collect();
        self.recipes.borrow_mut().insert(query.clone(), shown);
        sizes
    }
    fn read(&self, resource: &str) -> Option<Vec<u8>> {
        self.game.pack.source.read(resource).ok()
    }
    fn problem(&self, problem: &Problem) -> String {
        let english = sunrise_client::guides::english(problem.key);
        self.game
            .translate_with_fallback(problem.key, english)
            .replacen("%s", &problem.arg, 1)
    }
}

impl Game<'_> {
    pub(super) fn guides_open(&self) -> bool {
        self.guides.view.is_some()
    }

    /// An item stack from an id and `/give`-style components; none for an item the game doesn't have.
    pub(super) fn guide_stack(&self, id: &str, components: &str) -> Option<ItemStack> {
        self.items().get(id)?;
        let (damage, extra) = if components.is_empty() {
            (0, None)
        } else {
            sunrise_client::item_data::command_components(components).map_or((0, None), |tag| {
                sunrise_client::item_data::decode_components(Some(&tag))
            })
        };
        Some(ItemStack {
            item: id.into(),
            count: 1,
            damage,
            extra,
        })
    }

    /// The mods and namespaces that count as installed (spec 7.2, Q-040).
    fn guide_installed(&self) -> HashSet<String> {
        let mut installed: HashSet<String> = sunrise_protocol::mod_list::local()
            .into_iter()
            .map(|e| e.id)
            .collect();
        for id in &self.items().registry_order {
            if let Some((ns, _)) = id.split_once(':') {
                installed.insert(ns.to_owned());
            }
        }
        installed.insert("minecraft".to_owned());
        installed
    }

    /// The guides in the loaded resources, read again after a reload or a language change.
    fn guide_library(&mut self) -> Arc<Library> {
        let language = sunrise_client::lang::code();
        if let Some(lib) = &self.guides.library
            && self.guides.language == language
        {
            return lib.clone();
        }
        let names = self
            .pack
            .source
            .list_under("assets", &["ae2guide", "guideme_guides", "guides"]);
        let installed = self.guide_installed();
        let source = &self.pack.source;
        let lib = Arc::new(Library::build(&Sources {
            names: &names,
            read: &|name| source.read(name).ok(),
            language: &language,
            installed: &|m| installed.contains(m),
            mod_name: &|_| None,
        }));
        for e in &lib.errors {
            log::warn!("guides: {e}");
        }
        self.guides.library = Some(lib.clone());
        self.guides.language = language;
        lib
    }

    /// Resources were reloaded (F3+T, packs): the guides are read again, and an open viewer stays on its page
    /// if it is still there (spec 9.8, 10.3).
    pub(super) fn guides_reloaded(&mut self) {
        self.guides.library = None;
        let Some(scroll) = self.guides.view.as_ref().map(|v| v.scroll) else {
            return;
        };
        let id = self.guides.last.clone();
        let lib = self.guide_library();
        match id.and_then(|id| lib.page(&id)) {
            Some(page) => {
                self.guide_show(page, None);
                if let Some(v) = &mut self.guides.view {
                    v.scroll = scroll;
                }
            }
            None => self.guide_close(),
        }
    }

    /// The item under the pointer on a container screen: the item panel's, a slot's, or the creative list's.
    fn guide_pointed_item(&mut self) -> Option<String> {
        if let Some(stack) = self.mod_browser_pointed() {
            return mod_api::__private::item_key_id(stack.item).map(|id| id.to_string());
        }
        if self.screen != Screen::Creative {
            return None;
        }
        let r = self.renderer.as_ref()?;
        let batch = HudBatch::new(
            [r.config.width, r.config.height],
            self.settings.gui_scale_inputs(),
        );
        let slot = self.slot_at(&batch, self.cursor.0, self.cursor.1)?;
        let display = sunrise_client::creative::Creative::DISPLAY;
        if slot < display {
            return None;
        }
        let first = self.creative_screen.first_row(&self.pack.names);
        self.creative_screen
            .shown(&self.pack.names, first * 9 + slot - display)
            .map(|s| s.item.to_string())
    }

    /// W pressed (its physical key, spec 9.1.2) on a container screen: opens the page of the item under the
    /// pointer. False when nothing opened, so W goes on as usual.
    pub(super) fn guide_key_w(&mut self, key: KeyCode) -> bool {
        if !cfg!(feature = "sunrise-extras")
            || key != KeyCode::KeyW
            || !matches!(self.screen, Screen::Inventory | Screen::Creative)
            || self.title.is_some()
            || self.text_field_focused()
        {
            return false;
        }
        let Some(item) = self.guide_pointed_item() else {
            return false;
        };
        self.guide_open_item(&item)
    }

    /// Opens the page of `item`; false when it has none (Q-042: nothing happens).
    fn guide_open_item(&mut self, item: &str) -> bool {
        let lib = self.guide_library();
        let Some(page) = lib.item_page(item) else {
            return false;
        };
        if self.guides.view.as_ref().is_some_and(|v| v.page == page) {
            return true;
        }
        self.guide_go(page, None);
        true
    }

    /// Follows a link or a lookup to `page`: the current page goes into the history, the forward part is
    /// dropped (spec 9.3).
    fn guide_go(&mut self, page: usize, anchor: Option<String>) {
        if let Some(v) = &self.guides.view
            && let Some(lib) = &self.guides.library
        {
            let here = (lib.pages[v.page].id.clone(), v.scroll);
            self.guides.back.push(here);
            self.guides.forward.clear();
        }
        self.guide_show(page, anchor);
    }

    /// Shows `page` (at `anchor`), opening the viewer if needed.
    fn guide_show(&mut self, page: usize, anchor: Option<String>) {
        let lib = self.guide_library();
        if page >= lib.pages.len() {
            return;
        }
        for key in lib.path_keys(page) {
            self.guides.open.insert(key);
        }
        self.guides.last = Some(lib.pages[page].id.clone());
        let (search, hits, sidebar_scroll) = match self.guides.view.take() {
            Some(old) => {
                self.guide_free_images(&old);
                (old.search, old.hits, old.sidebar_scroll)
            }
            None => (EditBox::new(0.0, 0.0, 10.0, 14.0), Vec::new(), 0.0),
        };
        self.guides.view = Some(Box::new(View {
            page,
            scroll: 0.0,
            sidebar_scroll,
            laid: Laid::default(),
            width: -1.0,
            recipes: HashMap::new(),
            scenes: Vec::new(),
            images: HashMap::new(),
            search,
            hits,
            since: Instant::now(),
            drag: None,
        }));
        self.guide_relayout();
        if let Some(v) = &mut self.guides.view
            && let Some(a) = anchor
        {
            // An unknown anchor opens the page at the top (spec 5.2.4).
            v.scroll = v.laid.anchors.get(&a).copied().unwrap_or(0.0);
        }
    }

    fn guide_close(&mut self) {
        if let Some(v) = self.guides.view.take() {
            self.guide_free_images(&v);
        }
    }

    fn guide_free_images(&mut self, view: &View) {
        let atlas = &mut self.gui.atlas;
        for sprite in view.images.values().flatten() {
            if let Ok(id) = mod_api::Identifier::parse(sprite) {
                let _ = self.dynamic_textures.free(
                    atlas.dynamic_texture_pool.as_ref(),
                    &mut atlas.sprites,
                    Some(IMAGES),
                    &id,
                );
            }
        }
    }

    fn guide_frame(&self) -> Option<Frame> {
        let r = self.renderer.as_ref()?;
        let b = HudBatch::new(
            [r.config.width, r.config.height],
            self.settings.gui_scale_inputs(),
        );
        Some(frame(b.width, b.height))
    }

    /// Lays the page out again for the page column's width: its recipes, scenes and images with it.
    fn guide_relayout(&mut self) {
        let Some(f) = self.guide_frame() else {
            return;
        };
        let Some(page) = self.guides.view.as_ref().map(|v| v.page) else {
            return;
        };
        let lib = self.guide_library();
        let tables = self.made_recipes();
        let width = f.page[2];
        let env = GameEnv {
            game: self,
            tables,
            room: width,
            installed: self.guide_installed(),
            recipes: RefCell::new(HashMap::new()),
        };
        let laid = layout::layout(&lib, page, &env, width);
        let recipes = env.recipes.into_inner();
        for p in &laid.errors {
            log::warn!("guide page {}: {}", lib.pages[page].id, p.english());
        }
        let scenes: Vec<SceneDraw> = laid
            .scenes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let registry = &self.pack.assets.registry;
                let blocks = s
                    .model
                    .blocks
                    .iter()
                    .filter_map(|(pos, (state, _))| {
                        let id =
                            sunrise_client::block_sprites::parse_state(registry, &state.command());
                        if id.is_none() {
                            log::warn!("guide scene: unknown block state {}", state.command());
                        }
                        Some((*pos, id?))
                    })
                    .collect();
                // Q-037's roll is not drawn: the scene drawing turns about two axes.
                let view = SceneView::new(s.center, s.model.camera[0], s.model.camera[1], s.scale);
                SceneDraw {
                    blocks,
                    view,
                    home: view,
                    clock: ((page as u32) << 6) | (i as u32 & 63),
                    background: s.model.background,
                    interactive: s.model.interactive,
                    refused: s
                        .model
                        .refused()
                        .then(|| {
                            s.model
                                .errors
                                .iter()
                                .find(|p| p.key.contains("too_many"))
                                .cloned()
                        })
                        .flatten(),
                    annotations: s.model.annotations.clone(),
                }
            })
            .collect();
        let images: Vec<String> = laid
            .pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Image { resource, .. } => Some(resource.clone()),
                _ => None,
            })
            .collect();
        let mut loaded = HashMap::new();
        for (n, resource) in images.into_iter().enumerate() {
            if loaded.contains_key(&resource) {
                continue;
            }
            let sprite = self.guide_load_image(&resource, n);
            loaded.insert(resource, sprite);
        }
        if let Some(v) = &mut self.guides.view {
            let old: Vec<String> = v.images.values().flatten().cloned().collect();
            v.laid = laid;
            v.width = width;
            v.recipes = recipes;
            v.scenes = scenes;
            v.images = loaded;
            v.scroll = v.scroll.min((v.laid.height - f.page[3]).max(0.0));
            let kept: HashSet<String> = v.images.values().flatten().cloned().collect();
            let stale: Vec<String> = old.into_iter().filter(|s| !kept.contains(s)).collect();
            let atlas = &mut self.gui.atlas;
            for sprite in stale {
                if let Ok(id) = mod_api::Identifier::parse(&sprite) {
                    let _ = self.dynamic_textures.free(
                        atlas.dynamic_texture_pool.as_ref(),
                        &mut atlas.sprites,
                        Some(IMAGES),
                        &id,
                    );
                }
            }
        }
    }

    /// A guide image as a sprite of the GUI atlas's dynamic texture pool.
    fn guide_load_image(&mut self, resource: &str, n: usize) -> Option<String> {
        let name = format!("{IMAGES}:page/{n}");
        if self.gui.atlas.sprites.contains_key(&name) {
            return Some(name);
        }
        let bytes = self.pack.source.read(resource).ok()?;
        let (w, h, pixels) = sunrise_client::hud::decode_image_bytes(&bytes).ok()?;
        let id = mod_api::Identifier::parse(&name).ok()?;
        let atlas = &mut self.gui.atlas;
        match self.dynamic_textures.create(
            atlas.dynamic_texture_pool.as_ref(),
            &mut atlas.sprites,
            Some(IMAGES),
            &id,
            [w, h],
            &pixels,
        ) {
            Ok(()) => Some(name),
            Err(e) => {
                log::warn!("guide image {resource}: {e:?}");
                None
            }
        }
    }

    // ---- Drawing ----

    /// Draws the viewer over the screen it was opened on.
    pub(super) fn draw_guide(&mut self, batch: &mut HudBatch) {
        if self.guides.view.is_none() {
            return;
        }
        if !matches!(self.screen, Screen::Inventory | Screen::Creative) {
            // The container screen closed underneath (the server closed it): the viewer goes with it.
            self.guide_close();
            return;
        }
        if self.guides.language != sunrise_client::lang::code() {
            // The language changed: the page again in the new language (spec 8.6).
            self.guides_reloaded();
        }
        let f = frame(batch.width, batch.height);
        if self
            .guides
            .view
            .as_ref()
            .is_some_and(|v| v.width != f.page[2])
        {
            self.guide_relayout();
        }
        let Some(lib) = self.guides.library.clone() else {
            return;
        };
        let cursor = self.cursor;
        let atlas_bg = rgba(12, 12, 16, 235);
        batch.fill(
            &self.gui.atlas,
            0.0,
            0.0,
            batch.width,
            batch.height,
            atlas_bg,
        );
        batch.fill(
            &self.gui.atlas,
            f.side[0] - 2.0,
            f.side[1] - 2.0,
            f.side[2] + 4.0,
            f.side[3] + 4.0,
            rgba(0, 0, 0, 120),
        );
        self.guide_draw_bar(batch, &f, &lib);
        self.guide_draw_sidebar(batch, &f, &lib);
        let tip = self.guide_draw_page(batch, &f, &lib);
        // Tooltips last, over everything.
        match tip {
            Some(Tooltip::Stack(stack)) => {
                self.draw_stack_tooltip(batch, &stack, cursor.0, cursor.1)
            }
            Some(Tooltip::Lines(lines)) => {
                let lines: Vec<(String, u32)> = lines.into_iter().map(|l| (l, u32::MAX)).collect();
                self.draw_tooltip_lines(batch, &lines, cursor.0, cursor.1);
            }
            None => {}
        }
    }

    fn guide_button(&self, batch: &mut HudBatch, rect: [f32; 4], label: &str, active: bool) {
        let hover = active && inside(self.cursor, rect);
        let sprite = match (active, hover) {
            (false, _) => "gui/sprites/widget/button_disabled",
            (true, true) => "gui/sprites/widget/button_highlighted",
            (true, false) => "gui/sprites/widget/button",
        };
        let atlas = &self.gui.atlas;
        sunrise_client::ui::nine_slice(
            batch,
            atlas,
            sprite,
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            u32::MAX,
        );
        let w = HudBatch::text_width(atlas, label);
        let color = if active {
            u32::MAX
        } else {
            rgba(160, 160, 160, 255)
        };
        batch.text(
            atlas,
            label,
            rect[0] + ((rect[2] - w) / 2.0).floor(),
            rect[1] + 3.0,
            color,
            true,
        );
    }

    fn guide_draw_bar(&mut self, batch: &mut HudBatch, f: &Frame, lib: &Library) {
        let (can_back, can_forward) = (
            !self.guides.back.is_empty(),
            !self.guides.forward.is_empty(),
        );
        self.guide_button(batch, f.back, "<", can_back);
        self.guide_button(batch, f.forward, ">", can_forward);
        self.guide_button(batch, f.close, "x", true);
        for (rect, key, fallback) in [
            (f.back, "sunrise.guide.back", "Back"),
            (f.forward, "sunrise.guide.forward", "Forward"),
            (f.close, "sunrise.guide.close", "Close"),
        ] {
            let label = self.translate_with_fallback(key, fallback);
            sunrise_client::widgets::record(&label, rect, true);
        }
        let hint = self.translate_with_fallback("sunrise.guide.search", "Search guides...");
        let Some(v) = self.guides.view.as_mut() else {
            return;
        };
        let title = &lib.pages[v.page].title;
        let guide = &lib.guides[lib.pages[v.page].guide].name;
        let atlas = &self.gui.atlas;
        let heading = format!("{guide} \u{203a} {title}");
        let room = f.close[0] - f.page[0] - 6.0;
        let shown = if atlas.font.fit(&heading, room) >= heading.chars().count() {
            heading
        } else {
            let fit = atlas.font.fit(&heading, room - atlas.font.width("..."));
            format!(
                "{}...",
                heading.chars().take(fit).collect::<String>().trim_end()
            )
        };
        batch.text(
            atlas,
            &shown,
            f.page[0],
            9.0,
            rgba(255, 255, 255, 255),
            true,
        );
        let search = &mut v.search;
        search.x = f.search[0];
        search.y = f.search[1];
        search.width = f.search[2];
        search.height = f.search[3];
        search.suggestion = (search.value.is_empty() && !search.focused).then_some(hint);
        search.text_color = rgba(224, 224, 224, 255);
        search.draw_with_skin(batch, atlas, None, Some("sunrise/jei/search_background_v2"));
    }

    fn guide_draw_sidebar(&mut self, batch: &mut HudBatch, f: &Frame, lib: &Library) {
        let cursor = self.cursor;
        let Some(v) = self.guides.view.as_ref() else {
            return;
        };
        let start = batch.vertices.len();
        let atlas = &self.gui.atlas;
        let mut y = f.side[1] - v.sidebar_scroll;
        let fit = |text: &str, room: f32| -> String {
            if atlas.font.fit(text, room) >= text.chars().count() {
                return text.to_owned();
            }
            let n = atlas.font.fit(text, room - atlas.font.width("..."));
            format!("{}...", text.chars().take(n).collect::<String>().trim_end())
        };
        let mut icons: Vec<(f32, f32, String, String)> = Vec::new();
        if !v.search.value.trim().is_empty() {
            if v.hits.is_empty() {
                let none =
                    self.translate_with_fallback("sunrise.guide.no_results", "No pages found");
                batch.text(
                    atlas,
                    &none,
                    f.side[0] + 2.0,
                    y + 2.0,
                    rgba(160, 160, 160, 255),
                    false,
                );
            }
            for hit in &v.hits {
                let row = [f.side[0], y, f.side[2], RESULT_ROW];
                if hit.page == v.page {
                    batch.fill(
                        atlas,
                        row[0],
                        row[1],
                        row[2],
                        row[3],
                        rgba(255, 255, 255, 40),
                    );
                } else if inside(cursor, row) && inside(cursor, f.side) {
                    batch.fill(
                        atlas,
                        row[0],
                        row[1],
                        row[2],
                        row[3],
                        rgba(255, 255, 255, 20),
                    );
                }
                let page = &lib.pages[hit.page];
                let title = format!("{} \u{2013} {}", page.title, lib.guides[page.guide].name);
                batch.text(
                    atlas,
                    &fit(&title, f.side[2] - 4.0),
                    row[0] + 2.0,
                    y + 1.0,
                    u32::MAX,
                    false,
                );
                batch.text(
                    atlas,
                    &fit(&hit.excerpt, f.side[2] - 4.0),
                    row[0] + 2.0,
                    y + 11.0,
                    rgba(150, 150, 150, 255),
                    false,
                );
                y += RESULT_ROW;
            }
        } else {
            for row in lib.rows(&self.guides.open) {
                let x = f.side[0] + row.depth as f32 * 8.0;
                let rect = [f.side[0], y, f.side[2], ROW];
                let (label, current) = match row.entry {
                    Entry::Guide(g) => (lib.guides[g].name.clone(), false),
                    Entry::Page(p) => (lib.pages[p].title.clone(), p == v.page),
                };
                if current {
                    batch.fill(
                        atlas,
                        rect[0],
                        rect[1],
                        rect[2],
                        rect[3],
                        rgba(255, 255, 255, 40),
                    );
                } else if inside(cursor, rect) && inside(cursor, f.side) {
                    batch.fill(
                        atlas,
                        rect[0],
                        rect[1],
                        rect[2],
                        rect[3],
                        rgba(255, 255, 255, 20),
                    );
                }
                if row.has_children {
                    let arrow = if row.open { "\u{25be}" } else { "\u{25b8}" };
                    batch.text(atlas, arrow, x, y + 2.0, rgba(180, 180, 180, 255), false);
                }
                let mut tx = x + 8.0;
                if let Entry::Page(p) = row.entry
                    && let Some((id, components)) = &lib.pages[p].icon
                {
                    icons.push((tx, y, id.clone(), components.clone()));
                    tx += 13.0;
                }
                let color = match row.entry {
                    Entry::Guide(_) => rgba(255, 220, 120, 255),
                    Entry::Page(_) => rgba(224, 224, 224, 255),
                };
                let room = f.side[0] + f.side[2] - tx - 2.0;
                batch.text(atlas, &fit(&label, room), tx, y + 2.0, color, false);
                y += ROW;
            }
        }
        for (x, y, id, components) in icons {
            if let Some(stack) = self.guide_stack(&id, &components) {
                let s = batch.vertices.len();
                self.draw_item_model(batch, &stack, x, y);
                batch.scale_since(s, x, y, 0.75, 0.75);
            }
        }
        batch.clip_since(
            start,
            f.side[0],
            f.side[1],
            f.side[0] + f.side[2],
            f.side[1] + f.side[3],
        );
    }

    /// Draws the page: its pieces, recipes and scenes, clipped to the page column. Returns the tooltip under
    /// the pointer.
    fn guide_draw_page(
        &mut self,
        batch: &mut HudBatch,
        f: &Frame,
        lib: &Library,
    ) -> Option<Tooltip> {
        let cursor = self.cursor;
        let v = self.guides.view.as_ref()?;
        let (px, py) = (f.page[0], f.page[1] - v.scroll);
        let (top, bottom) = (f.page[1], f.page[1] + f.page[3]);
        let visible = |y: f32, h: f32| py + y + h >= top && py + y <= bottom;
        let start = batch.vertices.len();
        let since = v.since;
        let mut tip = None;
        let over_page = inside(cursor, f.page);
        for piece in &v.laid.pieces {
            match piece {
                Piece::Text { x, y, text, style } => {
                    if !visible(*y, layout::LINE * style.scale) {
                        continue;
                    }
                    let mut codes = String::new();
                    if style.bold {
                        codes.push_str("\u{a7}l");
                    }
                    if style.italic {
                        codes.push_str("\u{a7}o");
                    }
                    if style.underline {
                        codes.push_str("\u{a7}n");
                    }
                    if style.strike {
                        codes.push_str("\u{a7}m");
                    }
                    let s = batch.vertices.len();
                    let (tx, ty) = (px + x, py + y);
                    batch.text(
                        &self.gui.atlas,
                        &format!("{codes}{text}"),
                        tx,
                        ty,
                        argb(style.color),
                        false,
                    );
                    if style.scale != 1.0 {
                        batch.scale_since(s, tx, ty, style.scale, style.scale);
                    }
                }
                Piece::Fill { rect, color } => {
                    if visible(rect[1], rect[3]) {
                        batch.fill(
                            &self.gui.atlas,
                            px + rect[0],
                            py + rect[1],
                            rect[2],
                            rect[3],
                            argb(*color),
                        );
                    }
                }
                Piece::Slot { x, y } => {
                    if visible(*y, 18.0) {
                        batch.sprite(
                            &self.gui.atlas,
                            "gui/sprites/container/slot",
                            px + x,
                            py + y,
                        );
                    }
                }
                Piece::Item {
                    x,
                    y,
                    size,
                    id,
                    components,
                } => {
                    if !visible(*y, *size) {
                        continue;
                    }
                    if let Some(stack) = self.guide_stack(id, components) {
                        let (ix, iy) = (px + x, py + y);
                        let s = batch.vertices.len();
                        self.draw_item_model(batch, &stack, ix, iy);
                        if *size != 16.0 {
                            batch.scale_since(s, ix, iy, size / 16.0, size / 16.0);
                        }
                    }
                }
                Piece::Image { rect, resource } => {
                    if !visible(rect[1], rect[3]) {
                        continue;
                    }
                    let dst = [px + rect[0], py + rect[1], rect[2], rect[3]];
                    let sprite = v.images.get(resource).and_then(|s| s.as_deref());
                    let drawn = sprite.is_some_and(|name| {
                        let size = self
                            .gui
                            .atlas
                            .sprites
                            .get(name)
                            .map(|r| [r.w as f32, r.h as f32]);
                        size.is_some_and(|s| {
                            self.native_sprite(batch, name, [0.0, 0.0, s[0], s[1]], dst, None)
                        })
                    });
                    if !drawn {
                        batch.fill(
                            &self.gui.atlas,
                            dst[0],
                            dst[1],
                            dst[2],
                            dst[3],
                            rgba(80, 80, 80, 160),
                        );
                    }
                }
                Piece::Recipe { rect, query, index } => {
                    if !visible(rect[1], rect[3]) {
                        continue;
                    }
                    if let Some(r) = v.recipes.get(query).and_then(|list| list.get(*index)) {
                        let (x, y) = (px + rect[0], py + rect[1]);
                        self.draw_shown_recipe(batch, r, since, x, y, v.width, *index);
                        if over_page
                            && let Some(stack) = self.shown_slot_at(r, since, x, y, v.width)
                        {
                            tip = Some(Tooltip::Stack(stack));
                        }
                    }
                }
                Piece::Scene { rect, scene } => {
                    if !visible(rect[1], rect[3]) {
                        continue;
                    }
                    let screen = [px + rect[0], py + rect[1], rect[2], rect[3]];
                    if let Some(s) = v.scenes.get(*scene)
                        && let Some(t) = self.guide_draw_scene(batch, s, screen, since, over_page)
                    {
                        tip = Some(t);
                    }
                }
            }
        }
        batch.clip_since(start, f.page[0], top, f.page[0] + f.page[2] + 8.0, bottom);
        if v.laid.height > f.page[3] {
            sunrise_client::ui::scrollbar(
                batch,
                &self.gui.atlas,
                f.page[0] + f.page[2] + 2.0,
                top,
                bottom,
                v.laid.height,
                v.scroll,
            );
        }
        if tip.is_some() || !over_page {
            return tip;
        }
        let at = (cursor.0 - px, cursor.1 - py);
        let hot = v.laid.hotspots.iter().rev().find(|h| inside(at, h.rect))?;
        Some(match hot.tip.clone()? {
            Tip::Item { id, components } => Tooltip::Stack(self.guide_stack(&id, &components)?),
            Tip::Page(p) => Tooltip::Lines(vec![lib.pages.get(p)?.title.clone()]),
            Tip::Missing(id) => Tooltip::Lines(vec![
                self.translate_with_fallback("sunrise.guide.error.missing_page", "Missing page %s")
                    .replacen("%s", &id, 1),
            ]),
            Tip::Hidden(m) => Tooltip::Lines(vec![
                self.translate_with_fallback("sunrise.guide.hidden_page", "Needs the mod %s")
                    .replacen("%s", &m, 1),
            ]),
            Tip::External(url) => Tooltip::Lines(vec![url]),
            Tip::Lines(lines) => Tooltip::Lines(lines),
        })
    }

    /// One scene in its box: background, blocks (living: their particles), annotations, the reset control.
    fn guide_draw_scene(
        &self,
        batch: &mut HudBatch,
        s: &SceneDraw,
        rect: [f32; 4],
        since: Instant,
        over_page: bool,
    ) -> Option<Tooltip> {
        let atlas = &self.gui.atlas;
        if s.background != 0 {
            batch.fill(
                atlas,
                rect[0],
                rect[1],
                rect[2],
                rect[3],
                argb(s.background),
            );
        }
        if let Some(problem) = &s.refused {
            let english = sunrise_client::guides::english(problem.key);
            let text =
                self.translate_with_fallback(problem.key, english)
                    .replacen("%s", &problem.arg, 1);
            batch.text(
                atlas,
                &text,
                rect[0] + 4.0,
                rect[1] + 4.0,
                rgba(255, 85, 85, 255),
                false,
            );
            return None;
        }
        let corners = [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
        let blocks: Vec<SceneBlock<'_>> = s
            .blocks
            .iter()
            .map(|&(pos, state)| SceneBlock {
                pos,
                state,
                entity: None,
            })
            .collect();
        // The scene's clock: 20 ticks a second from when the page opened (spec 6.8).
        let ticks = since.elapsed().as_secs_f64() * 20.0;
        let clock =
            SceneClock::new(s.clock, ticks as u64, ticks.fract() as f32).animate_blocks(true);
        let scene = mod_api::Scene::new(Vec::new()).with_clock(clock);
        if let Err(e) = self.native_draw_scene(batch, corners, &blocks, &scene, s.view) {
            log::debug!("guide scene not drawn whole: {e:?}");
        }
        let m = scene_matrix(corners, s.view);
        let project = |p: [f32; 3]| {
            let v = m.transform_point3(glam::Vec3::from_array(p));
            (v.x, v.y)
        };
        let mut tip = None;
        let cursor = self.cursor;
        let near =
            |a: (f32, f32), b: (f32, f32), d: f32| (a.0 - b.0).abs() <= d && (a.1 - b.1).abs() <= d;
        for ann in &s.annotations {
            let color = argb(ann.color);
            let hovered = match &ann.shape {
                Shape::Diamond { pos } => {
                    let (x, y) = project(*pos);
                    for (dy, half) in [
                        (-3.0, 0.0),
                        (-2.0, 1.0),
                        (-1.0, 2.0),
                        (0.0, 3.0),
                        (1.0, 2.0),
                        (2.0, 1.0),
                        (3.0, 0.0),
                    ] {
                        batch.fill(
                            atlas,
                            (x - half).floor(),
                            (y + dy).floor(),
                            2.0 * half + 1.0,
                            1.0,
                            color,
                        );
                    }
                    near(cursor, (x, y), 4.0)
                }
                Shape::Box { min, max, .. } => {
                    let c = |i: usize| {
                        [
                            if i & 1 == 0 { min[0] } else { max[0] },
                            if i & 2 == 0 { min[1] } else { max[1] },
                            if i & 4 == 0 { min[2] } else { max[2] },
                        ]
                    };
                    let pts: Vec<(f32, f32)> = (0..8).map(|i| project(c(i))).collect();
                    for (a, b) in [
                        (0, 1),
                        (2, 3),
                        (4, 5),
                        (6, 7),
                        (0, 2),
                        (1, 3),
                        (4, 6),
                        (5, 7),
                        (0, 4),
                        (1, 5),
                        (2, 6),
                        (3, 7),
                    ] {
                        line(batch, atlas, pts[a], pts[b], color);
                    }
                    let (x0, x1) = pts
                        .iter()
                        .fold((f32::MAX, f32::MIN), |(l, r), p| (l.min(p.0), r.max(p.0)));
                    let (y0, y1) = pts
                        .iter()
                        .fold((f32::MAX, f32::MIN), |(l, r), p| (l.min(p.1), r.max(p.1)));
                    inside(cursor, [x0, y0, x1 - x0, y1 - y0])
                }
                Shape::Line { from, to, .. } => {
                    let (a, b) = (project(*from), project(*to));
                    line(batch, atlas, a, b, color);
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let len2 = (dx * dx + dy * dy).max(1e-6);
                    let t =
                        (((cursor.0 - a.0) * dx + (cursor.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
                    near(cursor, (a.0 + t * dx, a.1 + t * dy), 3.0)
                }
            };
            if hovered && over_page && inside(cursor, rect) && !ann.tooltip.is_empty() {
                let doc = sunrise_client::guides::markdown::Document {
                    blocks: ann.tooltip.clone(),
                };
                let lines: Vec<String> = doc
                    .sections()
                    .into_iter()
                    .flat_map(|(_, t)| t.lines().map(str::to_owned).collect::<Vec<_>>())
                    .filter(|l| !l.trim().is_empty())
                    .collect();
                tip = Some(Tooltip::Lines(lines));
            }
        }
        if s.interactive {
            let reset = [rect[0] + rect[2] - 11.0, rect[1] + 1.0, 10.0, 10.0];
            let hover = inside(cursor, reset);
            batch.fill(
                atlas,
                reset[0],
                reset[1],
                reset[2],
                reset[3],
                rgba(0, 0, 0, if hover { 180 } else { 110 }),
            );
            batch.text(
                atlas,
                "\u{21ba}",
                reset[0] + 2.0,
                reset[1] + 1.0,
                u32::MAX,
                false,
            );
            if hover {
                tip = Some(Tooltip::Lines(vec![self.translate_with_fallback(
                    "sunrise.guide.reset_view",
                    "Reset view",
                )]));
            }
        }
        tip
    }

    // ---- Input ----

    /// A mouse button while the viewer is open: it takes them all.
    pub(super) fn guide_mouse(&mut self, state: ElementState, button: MouseButton) {
        let pressed = state == ElementState::Pressed;
        if !pressed {
            if let Some(v) = &mut self.guides.view {
                v.drag = None;
            }
            return;
        }
        match button {
            MouseButton::Back => return self.guide_history(true),
            MouseButton::Forward => return self.guide_history(false),
            MouseButton::Left => {}
            _ => return,
        }
        let Some(f) = self.guide_frame() else {
            return;
        };
        let cursor = self.cursor;
        let Some(lib) = self.guides.library.clone() else {
            return;
        };
        if let Some(v) = &mut self.guides.view {
            let focus = inside(cursor, f.search);
            v.search.focused = focus;
            if focus {
                let shift = false;
                v.search.click(&self.gui.atlas, cursor.0, shift, false);
                return;
            }
        }
        if inside(cursor, f.close) {
            return self.guide_close();
        }
        if inside(cursor, f.back) {
            return self.guide_history(true);
        }
        if inside(cursor, f.forward) {
            return self.guide_history(false);
        }
        if inside(cursor, f.side) {
            return self.guide_sidebar_click(&f, &lib);
        }
        if inside(cursor, f.page) {
            self.guide_page_click(&f);
        }
    }

    fn guide_sidebar_click(&mut self, f: &Frame, lib: &Library) {
        let Some(v) = &self.guides.view else {
            return;
        };
        let y = self.cursor.1 - f.side[1] + v.sidebar_scroll;
        if y < 0.0 {
            return;
        }
        if !v.search.value.trim().is_empty() {
            let Some(hit) = v.hits.get((y / RESULT_ROW) as usize).cloned() else {
                return;
            };
            return self.guide_go(hit.page, hit.anchor);
        }
        let Some(row) = lib.rows(&self.guides.open).get((y / ROW) as usize).cloned() else {
            return;
        };
        let x = f.side[0] + row.depth as f32 * 8.0;
        let key = match row.entry {
            Entry::Guide(g) => lib.guides[g].id.clone(),
            Entry::Page(p) => lib.pages[p].id.clone(),
        };
        let on_arrow = self.cursor.0 < x + 8.0;
        if row.has_children && (on_arrow || matches!(row.entry, Entry::Guide(_)) || !row.open) {
            if !self.guides.open.remove(&key) {
                self.guides.open.insert(key);
            }
            if on_arrow || matches!(row.entry, Entry::Guide(_)) {
                return;
            }
        }
        if let Entry::Page(p) = row.entry
            && self.guides.view.as_ref().is_some_and(|v| v.page != p)
        {
            self.guide_go(p, None);
        }
    }

    fn guide_page_click(&mut self, f: &Frame) {
        let cursor = self.cursor;
        let hit = {
            let Some(v) = &self.guides.view else {
                return;
            };
            self.guide_page_hit(v, f, cursor)
        };
        match hit {
            PageHit::Scrollbar => {
                if let Some(v) = &mut self.guides.view {
                    v.drag = Some(Drag::Scrollbar);
                }
                self.guide_drag_scrollbar(f);
            }
            PageHit::Item(id) => {
                self.guide_open_item(&id);
            }
            PageHit::SceneReset(index) => {
                if let Some(s) = self
                    .guides
                    .view
                    .as_mut()
                    .and_then(|v| v.scenes.get_mut(index))
                {
                    s.view = s.home;
                }
            }
            PageHit::SceneDrag(index) => {
                if let Some(v) = &mut self.guides.view {
                    v.drag = Some(Drag::Scene {
                        index,
                        last: cursor,
                    });
                }
            }
            PageHit::Act(action) => self.guide_act(action),
            PageHit::Nothing => {}
        }
    }

    /// What a click at `cursor` on the page meets.
    fn guide_page_hit(&self, v: &View, f: &Frame, cursor: (f32, f32)) -> PageHit {
        if v.laid.height > f.page[3] && cursor.0 >= f.page[0] + f.page[2] {
            return PageHit::Scrollbar;
        }
        let (px, py) = (f.page[0], f.page[1] - v.scroll);
        let at = (cursor.0 - px, cursor.1 - py);
        for piece in &v.laid.pieces {
            match piece {
                // A recipe slot opens its item's page (spec 5.14).
                Piece::Recipe { rect, query, index } if inside(at, *rect) => {
                    let stack = v
                        .recipes
                        .get(query)
                        .and_then(|l| l.get(*index))
                        .and_then(|r| {
                            self.shown_slot_at(r, v.since, px + rect[0], py + rect[1], v.width)
                        });
                    return stack.map_or(PageHit::Nothing, |s| PageHit::Item(s.item.to_string()));
                }
                Piece::Scene { rect, scene } if inside(at, *rect) => {
                    if !v.scenes.get(*scene).is_some_and(|s| s.interactive) {
                        break;
                    }
                    let reset = [rect[0] + rect[2] - 11.0, rect[1] + 1.0, 10.0, 10.0];
                    return if inside(at, reset) {
                        PageHit::SceneReset(*scene)
                    } else {
                        PageHit::SceneDrag(*scene)
                    };
                }
                _ => {}
            }
        }
        v.laid
            .hotspots
            .iter()
            .rev()
            .find(|h| inside(at, h.rect))
            .and_then(|h| h.action.clone())
            .map_or(PageHit::Nothing, PageHit::Act)
    }

    fn guide_act(&mut self, action: Action) {
        match action {
            Action::Go(Target::Page { page, anchor }) => {
                if self.guides.view.as_ref().is_some_and(|v| v.page == page) {
                    // An anchor on this page: scroll to it.
                    if let Some(v) = &mut self.guides.view
                        && let Some(y) = anchor.and_then(|a| v.laid.anchors.get(&a).copied())
                    {
                        v.scroll = y;
                    }
                } else {
                    self.guide_go(page, anchor);
                }
            }
            // Minecraft's confirmation, under the chat-links options (spec 5.1.3).
            Action::Go(Target::External(url)) => self.click_url(url),
            Action::Go(_) => {}
            Action::Item { id, .. } => {
                self.guide_open_item(&id);
            }
            Action::Command { command, close } => {
                // Q-024: no confirmation; the server checks permissions as for a typed command.
                if close {
                    self.guide_close();
                }
                self.click_command(&command);
            }
        }
    }

    fn guide_history(&mut self, back: bool) {
        let (from, to) = if back {
            (&mut self.guides.back, &mut self.guides.forward)
        } else {
            (&mut self.guides.forward, &mut self.guides.back)
        };
        let Some((id, scroll)) = from.pop() else {
            return;
        };
        let Some(lib) = self.guides.library.clone() else {
            return;
        };
        if let Some(v) = &self.guides.view {
            to.push((lib.pages[v.page].id.clone(), v.scroll));
        }
        let Some(page) = lib.page(&id) else {
            return;
        };
        self.guide_show(page, None);
        if let Some(v) = &mut self.guides.view {
            v.scroll = scroll.min((v.laid.height - 1.0).max(0.0));
        }
    }

    /// The wheel: the sidebar under the pointer scrolls, an interactive scene zooms, else the page scrolls.
    pub(super) fn guide_wheel(&mut self, notches: f32) {
        let Some(f) = self.guide_frame() else {
            return;
        };
        let cursor = self.cursor;
        let rows = match (&self.guides.library, &self.guides.view) {
            (Some(lib), Some(v)) if v.search.value.trim().is_empty() => {
                lib.rows(&self.guides.open).len() as f32 * ROW
            }
            (_, Some(v)) => v.hits.len() as f32 * RESULT_ROW,
            _ => return,
        };
        let Some(v) = &mut self.guides.view else {
            return;
        };
        if inside(cursor, f.side) {
            v.sidebar_scroll =
                (v.sidebar_scroll - notches * 3.0 * ROW).clamp(0.0, (rows - f.side[3]).max(0.0));
            return;
        }
        let at = (cursor.0 - f.page[0], cursor.1 - f.page[1] + v.scroll);
        let zoomed = v.laid.pieces.iter().find_map(|p| match p {
            Piece::Scene { rect, scene } if inside(at, *rect) => Some(*scene),
            _ => None,
        });
        if let Some(i) = zoomed
            && let Some(s) = v.scenes.get_mut(i)
            && s.interactive
        {
            // Q-037: 10% a notch, from a quarter to four times the page's zoom.
            let k = 1.1f32.powf(notches);
            s.view.scale = (s.view.scale * k).clamp(s.home.scale * 0.25, s.home.scale * 4.0);
            return;
        }
        let max = (v.laid.height - f.page[3]).max(0.0);
        v.scroll = (v.scroll - notches * 3.0 * layout::LINE).clamp(0.0, max);
    }

    fn guide_drag_scrollbar(&mut self, f: &Frame) {
        let cursor = self.cursor;
        let Some(v) = &mut self.guides.view else {
            return;
        };
        let max = (v.laid.height - f.page[3]).max(0.0);
        let t = ((cursor.1 - f.page[1]) / f.page[3]).clamp(0.0, 1.0);
        v.scroll = (t * max).round();
    }

    /// The pointer moved: a drag of the scrollbar or of an interactive scene goes on.
    pub(super) fn guide_cursor_moved(&mut self) {
        let Some(f) = self.guide_frame() else {
            return;
        };
        let cursor = self.cursor;
        let Some(drag) = self.guides.view.as_ref().and_then(|v| v.drag) else {
            return;
        };
        match drag {
            Drag::Scrollbar => self.guide_drag_scrollbar(&f),
            Drag::Scene { index, last } => {
                let Some(v) = &mut self.guides.view else {
                    return;
                };
                if let Some(s) = v.scenes.get_mut(index) {
                    // Q-037: a degree a pixel; pitch from straight below to straight above.
                    s.view.yaw += cursor.0 - last.0;
                    s.view.pitch = (s.view.pitch + cursor.1 - last.1).clamp(-90.0, 90.0);
                }
                v.drag = Some(Drag::Scene {
                    index,
                    last: cursor,
                });
            }
        }
    }

    /// A key while the viewer is open: it takes them all.
    pub(super) fn guide_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        let focused = self.guides.view.as_ref().is_some_and(|v| v.search.focused);
        if code == KeyCode::Escape {
            if focused && let Some(v) = &mut self.guides.view {
                v.search.focused = false;
            } else {
                self.guide_close();
            }
            return;
        }
        if focused {
            let shortcut = self.edit_shortcut(event);
            let ctrl = self.has_control_down();
            let shift = self.has_shift_down();
            let types_text = self.types_text();
            let Some(v) = &mut self.guides.view else {
                return;
            };
            if let Some(shortcut) = shortcut {
                v.search.shortcut(shortcut, &mut self.clipboard);
            } else if !v.search.key(code, ctrl, shift)
                && types_text
                && let Some(text) = &event.text
            {
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                v.search.insert_text(&text);
            }
            v.search.reset_blink();
            let query = v.search.value.clone();
            let lib = self.guide_library();
            if let Some(v) = &mut self.guides.view {
                v.hits = search::search(&lib, &query);
                v.sidebar_scroll = 0.0;
            }
            return;
        }
        match code {
            KeyCode::KeyW if !event.repeat => {
                if let Some(item) = self.guide_item_under_pointer() {
                    self.guide_open_item(&item);
                }
            }
            KeyCode::Backspace => self.guide_history(true),
            KeyCode::PageDown | KeyCode::PageUp | KeyCode::Home | KeyCode::End => {
                let Some(f) = self.guide_frame() else {
                    return;
                };
                if let Some(v) = &mut self.guides.view {
                    let max = (v.laid.height - f.page[3]).max(0.0);
                    v.scroll = match code {
                        KeyCode::PageDown => v.scroll + f.page[3] - layout::LINE,
                        KeyCode::PageUp => v.scroll - f.page[3] + layout::LINE,
                        KeyCode::Home => 0.0,
                        _ => max,
                    }
                    .clamp(0.0, max);
                }
            }
            _ => {}
        }
    }

    /// The item under the pointer on the page: an item image or link, or a recipe slot (spec 9.1.1).
    fn guide_item_under_pointer(&self) -> Option<String> {
        let f = self.guide_frame()?;
        let v = self.guides.view.as_ref()?;
        if !inside(self.cursor, f.page) {
            return None;
        }
        let (px, py) = (f.page[0], f.page[1] - v.scroll);
        let at = (self.cursor.0 - px, self.cursor.1 - py);
        for piece in &v.laid.pieces {
            if let Piece::Recipe { rect, query, index } = piece
                && inside(at, *rect)
            {
                let r = v.recipes.get(query)?.get(*index)?;
                return self
                    .shown_slot_at(r, v.since, px + rect[0], py + rect[1], v.width)
                    .map(|s| s.item.to_string());
            }
        }
        v.laid
            .hotspots
            .iter()
            .rev()
            .find(|h| inside(at, h.rect))
            .and_then(|h| match &h.tip {
                Some(Tip::Item { id, .. }) => Some(id.clone()),
                _ => None,
            })
    }
}

enum Tooltip {
    Stack(ItemStack),
    Lines(Vec<String>),
}

/// What a click on the page meets.
enum PageHit {
    Scrollbar,
    Item(String),
    SceneReset(usize),
    SceneDrag(usize),
    Act(Action),
    Nothing,
}

/// A line from `a` to `b` a pixel wide, as small fills along it.
fn line(
    batch: &mut HudBatch,
    atlas: &sunrise_client::hud::HudAtlas,
    a: (f32, f32),
    b: (f32, f32),
    color: u32,
) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let steps = dx.abs().max(dy.abs()).ceil().clamp(1.0, 2048.0) as usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        batch.fill(
            atlas,
            (a.0 + dx * t).floor(),
            (a.1 + dy * t).floor(),
            1.0,
            1.0,
            color,
        );
    }
}
