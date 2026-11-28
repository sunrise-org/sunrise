//! `SpecialModelRenderer`s: items drawn with entity models (chests, shulker
//! boxes, banners, heads, shields, tridents, decorated pots, conduits, bells,
//! books and copper golem statues). Each submits its model at the layer pose
//! as vanilla's `submit` does (no block-entity placement transform) and
//! reports `getExtents` over the same posed parts.
use super::{Foil, ItemQuad, ItemRenderer, ItemSink, SpecialHost, argb};
use crate::{block_entity_render::ModelPart, camera::joml, hud::Rect, mobs::EntityModelDocument};
use glam::{Mat4, Quat, Vec3};
use serde_json::Value;
use std::{collections::HashMap, f32::consts::PI, path::Path};
use sunrise_assets::item_model::{SpecialKind, SpecialModel};

/// Entity model layers the special renderers bake (`ModelLayers`).
const LAYERS: [(&str, &str); 22] = [
    ("chest", "minecraft:chest#main"),
    ("double_chest_left", "minecraft:double_chest_left#main"),
    ("double_chest_right", "minecraft:double_chest_right#main"),
    ("shulker_box", "minecraft:shulker_box#main"),
    ("standing_banner", "minecraft:standing_banner#main"),
    ("standing_banner_flag", "minecraft:standing_banner#flag"),
    ("wall_banner", "minecraft:wall_banner#main"),
    ("wall_banner_flag", "minecraft:wall_banner#flag"),
    ("skeleton", "minecraft:skeleton_skull#main"),
    ("wither_skeleton", "minecraft:wither_skeleton_skull#main"),
    ("zombie", "minecraft:zombie_head#main"),
    ("creeper", "minecraft:creeper_head#main"),
    ("dragon", "minecraft:dragon_skull#main"),
    ("piglin", "minecraft:piglin_head#main"),
    ("player", "minecraft:player_head#main"),
    ("bell", "minecraft:bell#main"),
    ("book", "minecraft:book#main"),
    ("conduit_shell", "minecraft:conduit#shell"),
    ("decorated_pot_base", "minecraft:decorated_pot_base#main"),
    ("decorated_pot_sides", "minecraft:decorated_pot_sides#main"),
    ("shield", "minecraft:shield#main"),
    ("trident", "minecraft:trident#main"),
];
const COPPER_GOLEM_LAYERS: [(&str, &str); 4] = [
    ("copper_golem", "minecraft:copper_golem#main"),
    (
        "copper_golem_running",
        "minecraft:copper_golem_running#main",
    ),
    (
        "copper_golem_sitting",
        "minecraft:copper_golem_sitting#main",
    ),
    ("copper_golem_star", "minecraft:copper_golem_star#main"),
];

pub(super) fn load_parts(root: &Path) -> HashMap<String, ModelPart> {
    let Ok(all) = std::fs::read(root.join("entity-models.json"))
        .map_err(|_| ())
        .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|_| ()))
    else {
        return HashMap::new();
    };
    load_parts_from_value(&all)
}

pub(super) fn load_parts_from_document(
    document: &EntityModelDocument,
) -> HashMap<String, ModelPart> {
    fn render_part(part: crate::mobs::Part) -> ModelPart {
        ModelPart {
            pose: part.pose,
            polygons: part
                .polygons
                .into_iter()
                .map(|poly| poly.vertices)
                .collect(),
            children: part
                .children
                .into_iter()
                .map(|mut child| (std::mem::take(&mut child.name), render_part(child)))
                .collect(),
        }
    }
    LAYERS
        .iter()
        .chain(COPPER_GOLEM_LAYERS.iter())
        .filter_map(|(key, layer)| Some(((*key).to_owned(), render_part(document.model(layer)?))))
        .collect()
}

fn load_parts_from_value(all: &Value) -> HashMap<String, ModelPart> {
    LAYERS
        .iter()
        .chain(COPPER_GOLEM_LAYERS.iter())
        .filter_map(|(key, layer)| {
            all.get(*layer)
                .map(|v| ((*key).to_owned(), ModelPart::parse(v)))
        })
        .collect()
}

/// Marks every quad passed on as drawn with a back-face-culling render type (`entitySolid`).
struct CullBack<'a>(&'a mut dyn ItemSink);
impl ItemSink for CullBack<'_> {
    fn quad(&mut self, quad: ItemQuad) {
        self.0.quad(ItemQuad {
            cull_back: true,
            ..quad
        });
    }
}

/// Marks every quad passed on as drawn with a render type that has an `OitPipelineSet`
/// (`RenderTypes.entityTranslucent`).
struct OitEntity<'a>(&'a mut dyn ItemSink);
impl ItemSink for OitEntity<'_> {
    fn quad(&mut self, quad: ItemQuad) {
        self.0.quad(ItemQuad { oit: true, ..quad });
    }
}

/// Marks every quad passed on as drawn with `RenderTypes.bannerPattern` (`cull = true`,
/// `BlendFunction.TRANSLUCENT`, no depth writes, no alpha cutout).
struct BannerPattern<'a>(&'a mut dyn ItemSink);
impl ItemSink for BannerPattern<'_> {
    fn quad(&mut self, quad: ItemQuad) {
        self.0.quad(ItemQuad {
            cull_back: true,
            banner_pattern: true,
            ..quad
        });
    }
}

/// A pose override for a named part (`setupAnim`).
type Adjust<'a> = &'a dyn Fn(&str, &mut [f32; 9]);
fn none(_: &str, _: &mut [f32; 9]) {}

/// `ModelPart.translateAndRotate`.
fn part_matrix(parent: Mat4, p: &[f32; 9]) -> Mat4 {
    parent
        * Mat4::from_translation(Vec3::new(p[0], p[1], p[2]) / 16.0)
        * Mat4::from_quat(Quat::from_euler(glam::EulerRot::ZYX, p[5], p[4], p[3]))
        * Mat4::from_scale(Vec3::new(p[6], p[7], p[8]))
}
/// `ModelPart.render` of `part` (named `name`) as item quads.
#[allow(clippy::too_many_arguments)]
fn draw(
    part: &ModelPart,
    name: &str,
    parent: Mat4,
    adjust: Adjust,
    sprite: Rect,
    color: u32,
    foil: Foil,
    entity_glint: bool,
    glint_rect: Option<Rect>,
    sink: &mut dyn ItemSink,
) {
    let mut p = part.pose;
    adjust(name, &mut p);
    let m = part_matrix(parent, &p);
    for poly in &part.polygons {
        let pos = poly.map(|v| m.transform_point3(Vec3::new(v[0], v[1], v[2]) / 16.0));
        let normal = (pos[1] - pos[0]).cross(pos[2] - pos[0]).normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            (pos[2] - pos[1])
                .cross(pos[3] - pos[1])
                .normalize_or(Vec3::Y)
        } else {
            normal
        };
        // A reflection reverses winding, not PoseStack normal directions.
        let normal = if m.determinant() < 0.0 {
            -normal
        } else {
            normal
        };
        sink.quad(ItemQuad {
            pos,
            uv: poly.map(|v| {
                [
                    sprite.u0 + (sprite.u1 - sprite.u0) * v[3],
                    sprite.v0 + (sprite.v1 - sprite.v0) * v[4],
                ]
            }),
            foil_uv: poly.map(|v| [v[3], v[4]]),
            gui_glint_uv: glint_rect
                .map(|r| poly.map(|v| [r.u0 + (r.u1 - r.u0) * v[3], r.v0 + (r.v1 - r.v0) * v[4]])),
            normal,
            color,
            foil,
            entity_glint,
            light_emission: 0,
            cull_back: false,
            translucent: false,
            oit: false,
            banner_pattern: false,
            packed_light: None,
            lightmap_only: false,
            explicit_alpha_cutoff: false,
            mod_pipeline: None,
        });
    }
    for (child_name, child) in &part.children {
        draw(
            child,
            child_name,
            m,
            adjust,
            sprite,
            color,
            foil,
            entity_glint,
            glint_rect,
            sink,
        );
    }
}
/// `ModelPart.getExtentsForGui` from an identity pose, posed and transformed as JOML does
/// (`translateAndRotate`: the rotation only when one is set, the scale only when not 1).
fn extents_of(
    part: &ModelPart,
    name: &str,
    parent: &joml::Mat4,
    adjust: Adjust,
    out: &mut Vec<Vec3>,
) {
    if part.polygons.is_empty() && part.children.is_empty() {
        return;
    }
    let mut p = part.pose;
    adjust(name, &mut p);
    let mut m = *parent;
    m.translate(p[0] / 16.0, p[1] / 16.0, p[2] / 16.0);
    if p[3] != 0.0 || p[4] != 0.0 || p[5] != 0.0 {
        m.rotate(joml::Quat::rotation_zyx(p[5], p[4], p[3]));
    }
    if p[6] != 1.0 || p[7] != 1.0 || p[8] != 1.0 {
        m.scale(p[6], p[7], p[8]);
    }
    for poly in &part.polygons {
        for v in poly {
            let q = Vec3::from_array(m.transform_position([v[0] / 16.0, v[1] / 16.0, v[2] / 16.0]));
            if !out.contains(&q) {
                out.push(q);
            }
        }
    }
    for (child_name, child) in &part.children {
        extents_of(child, child_name, &m, adjust, out);
    }
}
fn child<'a>(part: &'a ModelPart, name: &str) -> Option<&'a ModelPart> {
    part.child(name)
}

/// A texture identifier relative to `textures/` (`minecraft:entity/x` or a
/// full `minecraft:textures/entity/x.png`) as its atlas sprite name.
fn texture_sprite(id: &str) -> String {
    let path = id.strip_prefix("minecraft:").unwrap_or(id);
    let path = path.strip_prefix("textures/").unwrap_or(path);
    path.strip_suffix(".png").unwrap_or(path).to_owned()
}
fn mapped(directory: &str, id: &str) -> String {
    format!(
        "entity/{directory}/{}",
        id.strip_prefix("minecraft:").unwrap_or(id)
    )
}
/// `DyeColor.getTextureDiffuseColor` as ARGB.
fn dye(name: &str) -> u32 {
    0xFF00_0000 | crate::banner::dye_rgb(name)
}

/// The chest model of a chest type, with `ChestModel.setupAnim(openness)`.
fn chest_model(chest_type: &str) -> &'static str {
    match chest_type {
        "left" => "double_chest_left",
        "right" => "double_chest_right",
        _ => "chest",
    }
}
/// The head model of a skull `kind`: vanilla's layer, or a native mod's kind, under which the
/// renderer keeps the head that mod registered (`create_skull_models`).
fn skull_layer(kind: &str) -> &str {
    match kind {
        "skeleton" => "skeleton",
        "wither_skeleton" => "wither_skeleton",
        "zombie" => "zombie",
        "creeper" => "creeper",
        "dragon" => "dragon",
        "piglin" => "piglin",
        mod_kind if mod_kind.contains(':') => mod_kind,
        _ => "player",
    }
}
/// `SkullBlockRenderer.SKIN_BY_TYPE`, with the kinds native mods registered.
fn skull_texture<'a>(renderer: &ItemRenderer, kind: &'a str) -> &'a str {
    match kind {
        "skeleton" => "entity/skeleton/skeleton",
        "wither_skeleton" => "entity/skeleton/wither_skeleton",
        "zombie" => "entity/zombie/zombie",
        "creeper" => "entity/creeper/creeper",
        "dragon" => "entity/enderdragon/dragon",
        "piglin" => "entity/piglin/piglin",
        mod_kind if mod_kind.contains(':') => renderer
            .skull_textures
            .get(mod_kind)
            .copied()
            .unwrap_or_default(),
        _ => "entity/player/wide/steve",
    }
}
/// `SkullModelBase.setupAnim` for a head at `animation` (yRot = xRot = 0).
fn skull_pose(kind: &str, animation: f32) -> impl Fn(&str, &mut [f32; 9]) + '_ {
    move |name: &str, p: &mut [f32; 9]| match (kind, name) {
        (_, "head") => {
            p[3] = 0.0;
            p[4] = 0.0;
        }
        ("dragon", "jaw") => p[3] = (((animation * PI * 0.2) as f64).sin() as f32 + 1.0) * 0.2,
        ("piglin", "left_ear") => {
            p[5] = -((((animation * PI * 0.2 * 1.2) as f64).cos() + 2.5) as f32) * 0.2;
        }
        ("piglin", "right_ear") => {
            p[5] = ((((animation * PI * 0.2) as f64).cos() + 2.5) as f32) * 0.2;
        }
        _ => {}
    }
}
fn shulker_pose(progress: f32) -> impl Fn(&str, &mut [f32; 9]) {
    move |name: &str, p: &mut [f32; 9]| {
        if name == "lid" {
            p[0] = 0.0;
            p[1] = 24.0 - progress * 0.5 * 16.0;
            p[2] = 0.0;
            p[4] = 270.0 * progress * (PI / 180.0);
        }
    }
}
fn chest_pose(open: f32) -> impl Fn(&str, &mut [f32; 9]) {
    move |name: &str, p: &mut [f32; 9]| {
        if name == "lid" || name == "lock" {
            p[3] = -(open * (PI / 2.0));
        }
    }
}
/// `BannerFlagModel.setupAnim(0)`.
fn flag_pose(name: &str, p: &mut [f32; 9]) {
    if name == "flag" {
        p[3] = (-0.0125 + 0.01 * (2.0 * PI * 0.0f32).cos()) * PI;
    }
}
/// `BookModel.setupAnim`.
fn book_pose(openness: f32, flip1: f32, flip2: f32) -> impl Fn(&str, &mut [f32; 9]) {
    move |name: &str, p: &mut [f32; 9]| match name {
        "left_lid" => p[4] = PI + openness,
        "right_lid" => p[4] = -openness,
        "left_pages" => {
            p[4] = openness;
            p[0] = openness.sin();
        }
        "right_pages" => {
            p[4] = -openness;
            p[0] = openness.sin();
        }
        "flip_page1" => {
            p[4] = openness - openness * 2.0 * flip1;
            p[0] = openness.sin();
        }
        "flip_page2" => {
            p[4] = openness - openness * 2.0 * flip2;
            p[0] = openness.sin();
        }
        _ => {}
    }
}
/// `CopperGolemStatueModel.setupAnim`.
fn statue_pose(name: &str, p: &mut [f32; 9]) {
    if name == "root" {
        p[1] = 0.0;
        p[5] = PI;
    }
}
fn statue_layer(pose: &str) -> &'static str {
    match pose {
        "running" => "copper_golem_running",
        "sitting" => "copper_golem_sitting",
        "star" => "copper_golem_star",
        _ => "copper_golem",
    }
}

/// `SpecialModelRenderer.getExtents`.
pub(super) fn extents(renderer: &ItemRenderer, model: &SpecialModel) -> Vec<Vec3> {
    let mut out = Vec::new();
    let id = joml::Mat4::IDENTITY;
    let mut add = |key: &str, adjust: Adjust| {
        if let Some(part) = renderer.part(key) {
            extents_of(part, "root", &id, adjust, &mut out);
        }
    };
    match &model.kind {
        SpecialKind::Bell => add("bell", &none),
        SpecialKind::Banner { .. } => {
            // `BannerRenderer.getExtents`: the standing models only.
            add("standing_banner", &none);
            add("standing_banner_flag", &flag_pose);
        }
        SpecialKind::Book {
            open_angle,
            page1,
            page2,
        } => add(
            "book",
            &book_pose(open_angle * (PI / 180.0), *page1, *page2),
        ),
        SpecialKind::Conduit => add("conduit_shell", &none),
        SpecialKind::Chest {
            openness,
            chest_type,
            ..
        } => add(chest_model(chest_type), &chest_pose(*openness)),
        SpecialKind::CopperGolemStatue { pose, .. } => add(statue_layer(pose), &statue_pose),
        SpecialKind::DecoratedPot => {
            if let Some(base) = renderer.part("decorated_pot_base") {
                // `neck`, `top` and `bottom` are posed from an identity stack.
                for name in ["neck", "top", "bottom"] {
                    if let Some(part) = child(base, name) {
                        extents_of(part, name, &id, &none, &mut out);
                    }
                }
            }
        }
        SpecialKind::EndCube { .. } => {}
        SpecialKind::Head {
            kind, animation, ..
        } => add(skull_layer(kind), &skull_pose(kind, *animation)),
        SpecialKind::PlayerHead => add("player", &none),
        SpecialKind::ShulkerBox { openness, .. } => add("shulker_box", &shulker_pose(*openness)),
        SpecialKind::Shield => add("shield", &none),
        SpecialKind::Trident => add("trident", &none),
    }
    out
}

/// Banner pattern layers of a `minecraft:banner_patterns` value: the
/// pattern identifier (or inline asset) and the dye.
fn pattern_layers(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|layer| {
            let pattern = match &layer["pattern"] {
                Value::String(s) => s.clone(),
                Value::Object(o) => format!("#asset:{}", o.get("asset_id")?.as_str()?),
                _ => return None,
            };
            let color = layer["color"].as_str()?.to_owned();
            Some((pattern, color))
        })
        .collect()
}
fn pattern_texture(host: &dyn SpecialHost, banner: bool, pattern: &str) -> Option<String> {
    if let Some(asset) = pattern.strip_prefix("#asset:") {
        let path = asset.strip_prefix("minecraft:").unwrap_or(asset);
        return Some(format!(
            "entity/{}/{path}",
            if banner { "banner" } else { "shield" }
        ));
    }
    if banner {
        host.banner_texture(pattern)
    } else {
        host.shield_texture(pattern)
    }
}
/// `DecoratedPotPatterns.getPatternFromItem` for one side's sherd item.
fn pot_side(item: Option<&str>) -> String {
    let item = item.map(|i| i.strip_prefix("minecraft:").unwrap_or(i));
    match item.and_then(|i| i.strip_suffix("_pottery_sherd")) {
        Some(name) => format!("entity/decorated_pot/{name}_pottery_pattern"),
        None => "entity/decorated_pot/decorated_pot_side".to_owned(),
    }
}
/// `PotDecorations` as [back, left, right, front] item ids, from the list
/// codec or a keyed object.
fn pot_sides(v: &Value) -> [Option<String>; 4] {
    let id = |s: &Value| match s {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("id").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    };
    match v {
        Value::Array(a) => std::array::from_fn(|i| a.get(i).and_then(id)),
        Value::Object(o) => ["back", "left", "right", "front"].map(|k| o.get(k).and_then(id)),
        _ => Default::default(),
    }
}

/// `GuiBannerResultRenderer.renderToTexture` under `pose`: the standing
/// banner flag (`BannerFlagModel.setupAnim(0)`) in the banner base texture,
/// then `BannerRenderer.submitPatterns` (the base colour and up to 16 layers
/// of `(texture, dye)`).
pub(super) fn submit_banner_result(
    renderer: &ItemRenderer,
    pose: Mat4,
    base: &str,
    layers: &[(String, String)],
    sprites: &HashMap<String, Rect>,
    sink: &mut dyn ItemSink,
) {
    let Some(flag) = renderer.part("standing_banner_flag") else {
        return;
    };
    let part = |texture: &str, color: u32, target: &mut dyn ItemSink| {
        if let Some(r) = sprites.get(texture).copied() {
            draw(
                flag,
                "root",
                pose,
                &flag_pose,
                r,
                color,
                Foil::None,
                false,
                None,
                target,
            );
        }
    };
    part("entity/banner/banner_base", argb(-1), sink);
    let mut pattern_sink = BannerPattern(sink);
    part("entity/banner/base", dye(base), &mut pattern_sink);
    for (texture, color) in layers.iter().take(16) {
        part(texture, dye(color), &mut pattern_sink);
    }
}

/// `GuiBookModelRenderer.renderToTexture`'s book under `pose`: `BookModel`
/// set up for `BookModel.State.forAnimation(0, page1, page2, open)`.
pub(super) fn submit_book(
    renderer: &ItemRenderer,
    pose: Mat4,
    open: f32,
    page1: f32,
    page2: f32,
    sprites: &HashMap<String, Rect>,
    sink: &mut dyn ItemSink,
) {
    let (Some(book), Some(r)) = (
        renderer.part("book"),
        sprites
            .get("entity/enchantment/enchanting_table_book")
            .copied(),
    ) else {
        return;
    };
    // forAnimation at time 0: (sin(0) * 0.1 + 1.25) * open.
    let pose_for = book_pose(1.25 * open, page1, page2);
    draw(
        book,
        "root",
        pose,
        &pose_for,
        r,
        argb(-1),
        Foil::None,
        false,
        None,
        sink,
    );
}

#[derive(Clone, Copy)]
pub(super) enum PartAdjust<'a> {
    None,
    Book {
        openness: f32,
        flip1: f32,
        flip2: f32,
    },
    Chest {
        open: f32,
    },
    Shulker {
        progress: f32,
    },
    Statue,
    Skull {
        kind: &'a str,
        animation: f32,
    },
}

impl PartAdjust<'_> {
    fn apply(self, name: &str, p: &mut [f32; 9]) {
        match self {
            Self::None => {}
            Self::Book {
                openness,
                flip1,
                flip2,
            } => book_pose(openness, flip1, flip2)(name, p),
            Self::Chest { open } => chest_pose(open)(name, p),
            Self::Shulker { progress } => shulker_pose(progress)(name, p),
            Self::Statue => statue_pose(name, p),
            Self::Skull { kind, animation } => skull_pose(kind, animation)(name, p),
        }
    }
}

/// Resolved special-model state ready to submit at one or more poses without repeating
/// `SpecialHost` lookups (player skins, banner/shield patterns), atlas lookups, or glint
/// atlas locks.
pub(super) enum Prepared<'a> {
    Empty,
    Part {
        part: &'a ModelPart,
        adjust: PartAdjust<'a>,
        sprite: Rect,
        color: u32,
        foil: Foil,
        entity_glint: bool,
        oit: bool,
    },
    Banner {
        main: Option<(&'a ModelPart, Rect)>,
        flag_base: Option<(&'a ModelPart, Rect)>,
        flag_part: Option<&'a ModelPart>,
        pattern_base: Option<Rect>,
        base_color: u32,
        patterns: Vec<(Rect, u32)>,
    },
    Shield {
        part: &'a ModelPart,
        base_rect: Option<Rect>,
        has_patterns: bool,
        has_foil: bool,
        glint_rect: Option<Rect>,
        pattern_base: Option<Rect>,
        base_dye: u32,
        patterns: Vec<(Rect, u32)>,
    },
    DecoratedPot {
        base_parts: [Option<(&'a ModelPart, &'static str, Rect)>; 3],
        side_draws: [Option<(&'a ModelPart, &'static str, Rect)>; 4],
    },
}

/// Resolves a `SpecialModelRenderer`'s parts, sprites, skins, patterns, and glint rect once.
pub(super) fn prepare<'a>(
    renderer: &'a ItemRenderer,
    model: &'a SpecialModel,
    argument: &Value,
    has_foil: bool,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
) -> Prepared<'a> {
    let white = argb(-1);
    let part = |key: &str, adjust: PartAdjust<'a>, texture: &str, color: u32, foil: Foil| {
        if let (Some(part), Some(sprite)) = (renderer.part(key), sprites.get(texture).copied()) {
            Prepared::Part {
                part,
                adjust,
                sprite,
                color,
                foil,
                entity_glint: false,
                oit: false,
            }
        } else {
            Prepared::Empty
        }
    };
    match &model.kind {
        SpecialKind::Bell => part(
            "bell",
            PartAdjust::None,
            "entity/bell/bell_body",
            white,
            Foil::None,
        ),
        SpecialKind::Book {
            open_angle,
            page1,
            page2,
        } => part(
            "book",
            PartAdjust::Book {
                openness: open_angle * (PI / 180.0),
                flip1: *page1,
                flip2: *page2,
            },
            "entity/enchantment/enchanting_table_book",
            white,
            Foil::None,
        ),
        SpecialKind::Conduit => part(
            "conduit_shell",
            PartAdjust::None,
            "entity/conduit/base",
            white,
            Foil::None,
        ),
        SpecialKind::Chest {
            texture,
            openness,
            chest_type,
        } => part(
            chest_model(chest_type),
            PartAdjust::Chest { open: *openness },
            &mapped("chest", texture),
            white,
            Foil::None,
        ),
        SpecialKind::ShulkerBox { texture, openness } => part(
            "shulker_box",
            PartAdjust::Shulker {
                progress: *openness,
            },
            &mapped("shulker", texture),
            white,
            Foil::None,
        ),
        SpecialKind::CopperGolemStatue { texture, pose: p } => part(
            statue_layer(p),
            PartAdjust::Statue,
            &texture_sprite(texture),
            white,
            Foil::None,
        ),
        SpecialKind::Head {
            kind,
            texture,
            animation,
        } => {
            let skin = texture
                .as_deref()
                .map(|t| format!("entity/{}", t.strip_prefix("minecraft:").unwrap_or(t)))
                .map_or_else(
                    || std::borrow::Cow::Borrowed(skull_texture(renderer, kind)),
                    Into::into,
                );
            part(
                skull_layer(kind),
                PartAdjust::Skull {
                    kind,
                    animation: *animation,
                },
                &skin,
                white,
                Foil::None,
            )
        }
        SpecialKind::PlayerHead => {
            let skin = (!argument.is_null())
                .then(|| host.player_skin(argument))
                .flatten()
                .unwrap_or_else(|| "entity/player/slim/steve".to_owned());
            // `PlayerHeadSpecialRenderer`: the skin's `entityTranslucent` render type.
            if let (Some(part), Some(sprite)) =
                (renderer.part("player"), sprites.get(&skin).copied())
            {
                Prepared::Part {
                    part,
                    adjust: PartAdjust::None,
                    sprite,
                    color: white,
                    foil: Foil::None,
                    entity_glint: false,
                    oit: true,
                }
            } else {
                Prepared::Empty
            }
        }
        SpecialKind::Trident => {
            let foil = if has_foil { Foil::Standard } else { Foil::None };
            if let (Some(part), Some(sprite)) = (
                renderer.part("trident"),
                sprites.get("entity/trident/trident").copied(),
            ) {
                Prepared::Part {
                    part,
                    adjust: PartAdjust::None,
                    sprite,
                    color: white,
                    foil,
                    entity_glint: true,
                    oit: false,
                }
            } else {
                Prepared::Empty
            }
        }
        SpecialKind::Banner { color, wall } => {
            let (main_key, flag_key) = if *wall {
                ("wall_banner", "wall_banner_flag")
            } else {
                ("standing_banner", "standing_banner_flag")
            };
            let base_rect = sprites.get("entity/banner/banner_base").copied();
            let main = renderer.part(main_key).zip(base_rect);
            let flag_part = renderer.part(flag_key);
            let flag_base = flag_part.zip(base_rect);
            let pattern_base = sprites.get("entity/banner/base").copied();
            let base_color = dye(color);
            // `submitPatterns`: the base layer in the banner's colour, then up to 16 layers.
            let patterns: Vec<(Rect, u32)> = if flag_part.is_some() {
                pattern_layers(argument)
                    .into_iter()
                    .take(16)
                    .filter_map(|(pattern, color)| {
                        let t = pattern_texture(host, true, &pattern)?;
                        let r = sprites.get(&t).copied()?;
                        Some((r, dye(&color)))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Prepared::Banner {
                main,
                flag_base,
                flag_part,
                pattern_base,
                base_color,
                patterns,
            }
        }
        SpecialKind::Shield => {
            let Some(part) = renderer.part("shield") else {
                return Prepared::Empty;
            };
            let raw_patterns = pattern_layers(&argument["minecraft:banner_patterns"]);
            let base_color = argument["minecraft:base_color"].as_str();
            let has_patterns = !raw_patterns.is_empty() || base_color.is_some();
            let base = if has_patterns {
                "entity/shield/shield_base"
            } else {
                "entity/shield/shield_base_nopattern"
            };
            let glint_rect = has_foil
                .then(|| {
                    let atlas = renderer
                        .shield_glint_atlas
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    atlas.get_or_missing(base)
                })
                .flatten();
            let base_rect = sprites.get(base).copied();
            let (pattern_base, base_dye, patterns) = if has_patterns {
                let pattern_base = sprites.get("entity/shield/base").copied();
                let base_dye = dye(base_color.unwrap_or("white"));
                let patterns = raw_patterns
                    .into_iter()
                    .take(16)
                    .filter_map(|(pattern, color)| {
                        let t = pattern_texture(host, false, &pattern)?;
                        let r = sprites.get(&t).copied()?;
                        Some((r, dye(&color)))
                    })
                    .collect();
                (pattern_base, base_dye, patterns)
            } else {
                (None, white, Vec::new())
            };
            Prepared::Shield {
                part,
                base_rect,
                has_patterns,
                has_foil,
                glint_rect,
                pattern_base,
                base_dye,
                patterns,
            }
        }
        SpecialKind::DecoratedPot => {
            let sides = pot_sides(argument);
            let base_parts = renderer
                .part("decorated_pot_base")
                .zip(
                    sprites
                        .get("entity/decorated_pot/decorated_pot_base")
                        .copied(),
                )
                .map_or([None; 3], |(base, r)| {
                    ["neck", "top", "bottom"].map(|name| child(base, name).map(|p| (p, name, r)))
                });
            let side_draws = renderer
                .part("decorated_pot_sides")
                .map_or([None; 4], |side_parts| {
                    let mut draws = [None; 4];
                    for (i, name) in ["back", "left", "right", "front"].into_iter().enumerate() {
                        let texture = pot_side(sides[i].as_deref());
                        draws[i] = child(side_parts, name)
                            .zip(sprites.get(&texture).copied())
                            .map(|(p, r)| (p, name, r));
                    }
                    draws
                });
            Prepared::DecoratedPot {
                base_parts,
                side_draws,
            }
        }
        // `EndCubeSpecialRenderer` draws the end portal/gateway effect; no
        // bundled item uses it.
        SpecialKind::EndCube { .. } => Prepared::Empty,
    }
}

impl Prepared<'_> {
    /// Submits this prepared special model at `pose`.
    pub(super) fn submit(&self, pose: Mat4, sink: &mut dyn ItemSink) {
        let white = argb(-1);
        match self {
            Self::Empty => {}
            Self::Part {
                part,
                adjust,
                sprite,
                color,
                foil,
                entity_glint,
                oit,
            } => {
                let adjust_fn = |name: &str, p: &mut [f32; 9]| adjust.apply(name, p);
                if *oit {
                    let mut oit_sink = OitEntity(sink);
                    draw(
                        part,
                        "root",
                        pose,
                        &adjust_fn,
                        *sprite,
                        *color,
                        *foil,
                        *entity_glint,
                        None,
                        &mut oit_sink,
                    );
                } else {
                    draw(
                        part,
                        "root",
                        pose,
                        &adjust_fn,
                        *sprite,
                        *color,
                        *foil,
                        *entity_glint,
                        None,
                        sink,
                    );
                }
            }
            Self::Banner {
                main,
                flag_base,
                flag_part,
                pattern_base,
                base_color,
                patterns,
            } => {
                if let Some((p, r)) = *main {
                    draw(
                        p,
                        "root",
                        pose,
                        &none,
                        r,
                        white,
                        Foil::None,
                        false,
                        None,
                        sink,
                    );
                }
                if let Some((p, r)) = *flag_base {
                    draw(
                        p,
                        "root",
                        pose,
                        &flag_pose,
                        r,
                        white,
                        Foil::None,
                        false,
                        None,
                        sink,
                    );
                }
                if let Some(p) = *flag_part {
                    let mut pattern_sink = BannerPattern(sink);
                    if let Some(r) = *pattern_base {
                        draw(
                            p,
                            "root",
                            pose,
                            &flag_pose,
                            r,
                            *base_color,
                            Foil::None,
                            false,
                            None,
                            &mut pattern_sink,
                        );
                    }
                    for &(r, c) in patterns {
                        draw(
                            p,
                            "root",
                            pose,
                            &flag_pose,
                            r,
                            c,
                            Foil::None,
                            false,
                            None,
                            &mut pattern_sink,
                        );
                    }
                }
            }
            Self::Shield {
                part,
                base_rect,
                has_patterns,
                has_foil,
                glint_rect,
                pattern_base,
                base_dye,
                patterns,
            } => {
                if !*has_patterns {
                    let foil = if *has_foil {
                        Foil::Standard
                    } else {
                        Foil::None
                    };
                    if let Some(r) = *base_rect {
                        draw(
                            part,
                            "root",
                            pose,
                            &none,
                            r,
                            white,
                            foil,
                            true,
                            *glint_rect,
                            sink,
                        );
                    }
                } else {
                    if let Some(r) = *base_rect {
                        draw(
                            part,
                            "root",
                            pose,
                            &none,
                            r,
                            white,
                            Foil::None,
                            true,
                            None,
                            sink,
                        );
                    }
                    {
                        let mut pattern_sink = BannerPattern(sink);
                        if let Some(r) = *pattern_base {
                            draw(
                                part,
                                "root",
                                pose,
                                &none,
                                r,
                                *base_dye,
                                Foil::None,
                                true,
                                None,
                                &mut pattern_sink,
                            );
                        }
                        for &(r, c) in patterns {
                            draw(
                                part,
                                "root",
                                pose,
                                &none,
                                r,
                                c,
                                Foil::None,
                                true,
                                None,
                                &mut pattern_sink,
                            );
                        }
                    }
                    if *has_foil && let Some(r) = *base_rect {
                        // `ShieldSpecialRenderer.submit`: one `patternedShieldGlint` pass after patterns.
                        draw(
                            part,
                            "root",
                            pose,
                            &none,
                            r,
                            white,
                            Foil::GlintOnly,
                            true,
                            *glint_rect,
                            sink,
                        );
                    }
                }
            }
            Self::DecoratedPot {
                base_parts,
                side_draws,
            } => {
                // `DecoratedPotRenderer` draws every part with `entitySolid`.
                let sink = &mut CullBack(sink);
                for &(p, name, r) in base_parts.iter().flatten() {
                    draw(
                        p,
                        name,
                        pose,
                        &none,
                        r,
                        white,
                        Foil::None,
                        false,
                        None,
                        sink,
                    );
                }
                for &(p, name, r) in side_draws.iter().flatten() {
                    draw(
                        p,
                        name,
                        pose,
                        &none,
                        r,
                        white,
                        Foil::None,
                        false,
                        None,
                        sink,
                    );
                }
            }
        }
    }
}

/// `SpecialModelRenderer.submit` across one or more poses, resolving part, texture, skin,
/// pattern, and glint state once before drawing each pose.
#[allow(clippy::too_many_arguments)]
pub(super) fn submit(
    renderer: &ItemRenderer,
    model: &SpecialModel,
    argument: &Value,
    has_foil: bool,
    poses: &[Mat4],
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    sink: &mut dyn ItemSink,
) {
    if poses.is_empty() {
        return;
    }
    let prepared = prepare(renderer, model, argument, has_foil, sprites, host);
    for &pose in poses {
        prepared.submit(pose, sink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pot_sides_read_both_shapes() {
        let list = serde_json::json!(["minecraft:brick", "minecraft:angler_pottery_sherd"]);
        let sides = pot_sides(&list);
        assert_eq!(
            pot_side(sides[0].as_deref()),
            "entity/decorated_pot/decorated_pot_side"
        );
        assert_eq!(
            pot_side(sides[1].as_deref()),
            "entity/decorated_pot/angler_pottery_pattern"
        );
        assert_eq!(
            pot_side(sides[3].as_deref()),
            "entity/decorated_pot/decorated_pot_side"
        );
    }
    #[test]
    fn texture_ids_map_to_sprites() {
        assert_eq!(
            texture_sprite("minecraft:textures/entity/copper_golem/copper_golem.png"),
            "entity/copper_golem/copper_golem"
        );
        assert_eq!(
            mapped("chest", "minecraft:christmas"),
            "entity/chest/christmas"
        );
    }

    #[test]
    fn model_normals_follow_the_pose_not_the_mirrored_winding() {
        let part = ModelPart::parse(&serde_json::json!({
            "pose": [0, 0, 0, 0, 0, 0, 1, 1, 1],
            "polygons": [[[0, 0, 0, 0, 0], [1, 0, -1, 1, 0],
                          [0, 1, -1, 1, 1], [-1, 1, 0, 0, 1]]],
            "children": {}
        }));
        let sprite = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        };
        let local = Vec3::ONE.normalize();
        for scale in [
            Vec3::ONE,
            Vec3::new(16.0, -16.0, 16.0),
            Vec3::new(2.0, 3.0, -4.0),
            Vec3::new(-2.0, -3.0, 4.0),
        ] {
            let pose = Mat4::from_rotation_y(0.7) * Mat4::from_scale(scale);
            let mut quads = Vec::new();
            draw(
                &part,
                "root",
                pose,
                &none,
                sprite,
                argb(-1),
                Foil::None,
                false,
                None,
                &mut quads,
            );
            assert_eq!(quads.len(), 1);
            let expected = super::super::transform_normal(pose, local);
            assert!(
                (quads[0].normal - expected).length() < 1e-6,
                "scale {scale:?}: {:?} != {expected:?}",
                quads[0].normal
            );
        }
    }
    /// `ShieldSpecialRenderer.submit` of an enchanted shield with a banner:
    /// the glint goes over the patterns, and nothing covers them after.
    #[test]
    fn an_enchanted_banner_shield_keeps_its_patterns_under_the_glint() {
        let root =
            std::env::temp_dir().join(format!("sunrise-shield-glint-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        std::fs::write(
            root.join("entity-models.json"),
            serde_json::json!({"minecraft:shield#main": {
                "pose": [0, 0, 0, 0, 0, 0, 1, 1, 1],
                "polygons": [[[0, 0, 0, 0, 0], [1, 0, 0, 1, 0],
                              [1, 1, 0, 1, 1], [0, 1, 0, 0, 1]]],
                "children": {}
            }})
            .to_string(),
        )
        .unwrap();
        let library = crate::model_library::ParsedModels::default().into_item_library();
        let data = crate::test_resources::item_data(
            &root,
            sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
        )
        .unwrap();
        let renderer = ItemRenderer::load_with_library(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library,
            &data,
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let textures = [
            "entity/shield/shield_base",
            "entity/shield/shield_base_nopattern",
            "entity/shield/base",
            "entity/shield/stripe_bottom",
        ];
        // A sprite per texture, told apart by its u0.
        let sprites: HashMap<String, Rect> = textures
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let u0 = i as f32 / 8.0;
                let rect = Rect {
                    u0,
                    v0: 0.0,
                    u1: u0 + 0.1,
                    v1: 0.1,
                    w: 16,
                    h: 16,
                };
                ((*t).to_owned(), rect)
            })
            .collect();
        use super::super::{ItemRenderState, LayerGeometry, LayerRender, LayerTints, NoHost};
        use sunrise_assets::item_model::{DisplayContext, FoilType};
        let display = DisplayContext::ThirdPersonRightHand;
        let drawn = |argument: Value, has_foil: bool, tint: Option<u32>| {
            let mut state = ItemRenderState {
                display,
                layers: vec![LayerRender {
                    geometry: LayerGeometry::Special {
                        model: SpecialModel {
                            kind: SpecialKind::Shield,
                        },
                        argument,
                    },
                    tints: LayerTints::Owned(Vec::new()),
                    foil: if has_foil {
                        FoilType::Standard
                    } else {
                        FoilType::None
                    },
                    matrix: Mat4::IDENTITY,
                    particle: "".into(),
                    uses_block_light: false,
                    extents: Default::default(),
                    display,
                    submit_options: mod_api::SpecialModelSubmitOptions::default(),
                    render_type: None,
                }],
                oversized_in_gui: false,
                animated: false,
                cit_glint: None,
                hdr_tier: 0,
                hdr_light: 0,
                held_tint: None,
            };
            if let Some(rgb) = tint {
                state.set_held_tint(rgb);
            }
            let mut quads = Vec::new();
            state.submit(&renderer, &sprites, &NoHost, Mat4::IDENTITY, &mut quads);
            quads
        };
        let texture_and_foil = |quads: &[ItemQuad]| {
            quads
                .iter()
                .map(|q| {
                    let texture = textures
                        .iter()
                        .find(|t| sprites[**t].u0 == q.uv[0][0].min(q.uv[1][0]))
                        .unwrap();
                    (*texture, q.foil, q.banner_pattern)
                })
                .collect::<Vec<_>>()
        };
        let banner = serde_json::json!({
            "minecraft:base_color": "red",
            "minecraft:banner_patterns": [{
                "pattern": {"asset_id": "minecraft:stripe_bottom",
                            "translation_key": "block.minecraft.banner.stripe_bottom"},
                "color": "blue"
            }]
        });
        let untinted = drawn(banner.clone(), true, None);
        assert_eq!(
            texture_and_foil(&untinted),
            [
                ("entity/shield/shield_base", Foil::None, false),
                ("entity/shield/base", Foil::None, true),
                ("entity/shield/stripe_bottom", Foil::None, true),
                ("entity/shield/shield_base", Foil::GlintOnly, false),
            ]
        );
        assert_eq!(
            texture_and_foil(&drawn(banner.clone(), false, None)),
            [
                ("entity/shield/shield_base", Foil::None, false),
                ("entity/shield/base", Foil::None, true),
                ("entity/shield/stripe_bottom", Foil::None, true),
            ]
        );
        assert_eq!(
            texture_and_foil(&drawn(Value::Null, true, None)),
            [("entity/shield/shield_base_nopattern", Foil::Standard, false)]
        );
        // Removing green acts on the base and both pattern passes without altering the glint pass.
        let expected = untinted
            .into_iter()
            .map(|q| ItemQuad {
                color: if q.foil == Foil::GlintOnly {
                    q.color
                } else {
                    q.color & 0xFFFF_00FF
                },
                ..q
            })
            .collect::<Vec<_>>();
        assert_eq!(drawn(banner, true, Some(0xFF_00_FF)), expected);
    }
    /// `DecoratedPotRenderer` draws every part with `RenderTypes.entitySolid`,
    /// whose pipeline culls back faces: the pot's flat `top` and `bottom`
    /// planes would otherwise show both sides and z-fight in the hand too.
    #[test]
    fn the_decorated_pot_item_culls_its_back_faces() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-models.json").exists() {
            return;
        }
        let data = crate::test_resources::item_data(
            &root,
            sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
        )
        .unwrap();
        let renderer = ItemRenderer::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &data,
        )
        .unwrap();
        let sprite = Rect {
            u0: 0.25,
            v0: 0.125,
            u1: 0.375,
            v1: 0.25,
            w: 16,
            h: 16,
        };
        let sprites: HashMap<String, Rect> = [
            "entity/decorated_pot/decorated_pot_base",
            "entity/decorated_pot/decorated_pot_side",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), sprite))
        .collect();
        let model = SpecialModel {
            kind: SpecialKind::DecoratedPot,
        };
        let mut quads: Vec<ItemQuad> = Vec::new();
        submit(
            &renderer,
            &model,
            &Value::Null,
            false,
            &[Mat4::IDENTITY],
            &sprites,
            &super::super::NoHost,
            &mut quads,
        );
        assert!(!quads.is_empty(), "the pot item draws its parts");
        assert!(
            quads.iter().all(|q| q.cull_back),
            "entitySolid culls the pot's back faces"
        );
    }

    /// Submitting a composite (`Quads` + `PlayerHead`) held item or worn `PlayerHead` across
    /// multiple mirror poses resolves the profile's skin once through `SpecialHost::player_skin`
    /// while preserving per-pose layer submission order (`Q0, S0, Q1, S1, ...`) bit-for-bit.
    #[test]
    fn a_player_head_state_resolves_skin_once_across_multiple_poses() {
        use super::super::{ItemRenderState, LayerGeometry, LayerRender, LayerTints};
        use std::{cell::Cell, sync::Arc};
        use sunrise_assets::{
            bake::ModelQuad,
            item_bake::BakedItemModel,
            item_model::{DisplayContext, FoilType},
            model::{Direction, Material},
        };

        struct CountingHost {
            skin_calls: Cell<usize>,
        }
        impl SpecialHost for CountingHost {
            fn banner_texture(&self, _: &str) -> Option<String> {
                None
            }
            fn shield_texture(&self, _: &str) -> Option<String> {
                None
            }
            fn player_skin(&self, _profile: &Value) -> Option<String> {
                self.skin_calls.set(self.skin_calls.get() + 1);
                Some("entity/player/custom_test_skin".to_owned())
            }
        }

        let root =
            std::env::temp_dir().join(format!("sunrise-player-head-poses-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        std::fs::write(
            root.join("entity-models.json"),
            serde_json::json!({"minecraft:player_head#main": {
                "pose": [0, 0, 0, 0, 0, 0, 1, 1, 1],
                "polygons": [[[0, 0, 0, 0, 0], [1, 0, 0, 1, 0],
                              [1, 1, 0, 1, 1], [0, 1, 0, 0, 1]]],
                "children": {}
            }})
            .to_string(),
        )
        .unwrap();
        let library = crate::model_library::ParsedModels::default().into_item_library();
        let data = crate::test_resources::item_data(
            &root,
            sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
        )
        .unwrap();
        let renderer = ItemRenderer::load_with_library(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library,
            &data,
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);

        let quad_rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.25,
            v1: 0.25,
            w: 16,
            h: 16,
        };
        let custom_rect = Rect {
            u0: 0.25,
            v0: 0.5,
            u1: 0.5,
            v1: 0.75,
            w: 64,
            h: 64,
        };
        let mut sprites = HashMap::new();
        sprites.insert("item/test_quad_layer".to_owned(), quad_rect);
        sprites.insert("entity/player/custom_test_skin".to_owned(), custom_rect);

        let profile = serde_json::json!({"name": "IllusionerTest"});
        let poses = [
            Mat4::from_translation(Vec3::new(2.5, 1.0, -1.5)),
            Mat4::from_translation(Vec3::new(-2.0, 0.0, 2.0)),
            Mat4::from_translation(Vec3::new(1.5, 1.0, 2.5)),
            Mat4::from_translation(Vec3::new(-3.0, 0.0, -2.5)),
        ];
        let quad_material = Material {
            sprite: "minecraft:item/test_quad_layer".into(),
            force_translucent: false,
        };
        let baked_quad = Arc::new(BakedItemModel {
            quads: vec![ModelQuad {
                positions: [
                    [-0.5, 0.5, 0.0],
                    [-0.5, -0.5, 0.0],
                    [0.5, -0.5, 0.0],
                    [0.5, 0.5, 0.0],
                ],
                uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
                facing: Direction::South,
                cull: None,
                material: quad_material.clone(),
                tint_index: -1,
                shade_direction_override: None,
                light_emission: 0,
                light: [0; 2],
                color: None,
                ambient_occlusion: None,
                render_type: None,
                shade: true,
            }],
            uses_block_light: false,
            particle: quad_material,
            transforms: Default::default(),
        });
        let layer_matrix = Mat4::from_translation(Vec3::new(0.125, -0.25, 0.5));
        let model = SpecialModel {
            kind: SpecialKind::PlayerHead,
        };
        // Composite state (`minecraft:composite`): an ordinary quad layer `Q` followed by a
        // special `PlayerHead` layer `S`.
        let state = ItemRenderState {
            display: DisplayContext::ThirdPersonRightHand,
            layers: vec![
                LayerRender {
                    geometry: LayerGeometry::Quads(baked_quad),
                    tints: LayerTints::Owned(Vec::new()),
                    foil: FoilType::None,
                    matrix: Mat4::IDENTITY,
                    particle: "item/test_quad_layer".into(),
                    uses_block_light: false,
                    extents: Default::default(),
                    display: DisplayContext::ThirdPersonRightHand,
                    submit_options: mod_api::SpecialModelSubmitOptions::default(),
                    render_type: None,
                },
                LayerRender {
                    geometry: LayerGeometry::Special {
                        model: model.clone(),
                        argument: profile.clone(),
                    },
                    tints: LayerTints::Owned(Vec::new()),
                    foil: FoilType::None,
                    matrix: layer_matrix,
                    particle: "".into(),
                    uses_block_light: false,
                    extents: Default::default(),
                    display: DisplayContext::ThirdPersonRightHand,
                    submit_options: mod_api::SpecialModelSubmitOptions::default(),
                    render_type: None,
                },
            ],
            oversized_in_gui: false,
            animated: false,
            cit_glint: None,
            hdr_tier: 0,
            hdr_light: 0,
            held_tint: None,
        };

        // Composite held-item multi-pose path (`ItemRenderState::submit_poses`).
        let held_host = CountingHost {
            skin_calls: Cell::new(0),
        };
        let mut multi_held = Vec::new();
        state.submit_poses(&renderer, &sprites, &held_host, &poses, &mut multi_held);
        assert_eq!(
            held_host.skin_calls.get(),
            1,
            "held composite player head must resolve player_skin once across all four poses"
        );
        let mut single_held = Vec::new();
        for &pose in &poses {
            state.submit(&renderer, &sprites, &held_host, pose, &mut single_held);
        }
        assert_eq!(
            multi_held.iter().map(|q| q.oit).collect::<Vec<_>>(),
            [false, true, false, true, false, true, false, true],
            "composite item must preserve per-pose Q, S layer order"
        );
        assert_eq!(multi_held, single_held);

        // Worn-skull multi-pose path (`ItemRenderer::submit_special_head_poses`).
        let worn_host = CountingHost {
            skin_calls: Cell::new(0),
        };
        let mut multi_worn = Vec::new();
        renderer.submit_special_head_poses(
            &model,
            &profile,
            &poses,
            &sprites,
            &worn_host,
            &mut multi_worn,
        );
        assert_eq!(
            worn_host.skin_calls.get(),
            1,
            "worn player head must resolve player_skin once across all four poses"
        );
        let mut single_worn = Vec::new();
        for &pose in &poses {
            renderer.submit_special(
                &model,
                &profile,
                false,
                pose,
                &sprites,
                &worn_host,
                &mut single_worn,
            );
        }
        assert_eq!(multi_worn.len(), 4);
        assert_eq!(multi_worn, single_worn);
    }
}
