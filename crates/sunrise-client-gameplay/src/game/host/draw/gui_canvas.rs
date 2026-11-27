//! Direct mod GUI drawing on the hook's HUD batch.

use super::text_layout::LabelLayout;
use crate::game::ClientGame;
use mod_api::{
    FluidStack, GuiBlitRegion, GuiCanvas, GuiDrawError, GuiSpriteRegion, GuiVertex, ItemStack,
    Text, TextLayoutSize, TextLayoutSpec,
};
use sunrise_client::{
    hud::{GuiSprite, HudAtlas, HudBatch},
    hud_layout::{
        Blend, IDENTITY, Pose, PoseOp, ResolvedGuiQuad, abgr, paint_fill_gradient,
        paint_player_face, pose_apply, pose_point,
    },
    items::ItemStack as EngineStack,
    skins::HeadSkinJournal,
};
use sunrise_gameplay_common::game::api_values::{TryIntoEngine, stack_value};

pub(in crate::game) struct GameGuiCanvas<'a, 'g> {
    batch: &'a mut HudBatch,
    atlas: &'a HudAtlas,
    /// Absent only for the renderer-only canvas fixture.
    game: Option<&'a ClientGame<'g>>,
    head_skins: &'a HeadSkinJournal,
    pose: Pose,
    /// Screen-space edges, not affected by future changes of pose.
    scissor: Option<[f32; 4]>,
    /// A decorator may paint the standard suffix, but never enter the decorator walk again.
    decorating: bool,
}

impl ClientGame<'_> {
    pub(in crate::game) fn draw_item_decorator(
        &self,
        batch: &mut HudBatch,
        stack: &EngineStack,
        x: f32,
        y: f32,
    ) {
        let _ = GameGuiCanvas::draw_root(
            batch,
            &self.client.gui.atlas,
            Some(self),
            None,
            true,
            &mut |draw| {
                self.native_item_decorations(stack, x, y, draw);
                Ok(())
            },
        );
    }
}

impl GameGuiCanvas<'_, '_> {
    pub(in crate::game) fn draw(
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        game: Option<&ClientGame<'_>>,
        scissor: Option<[f32; 4]>,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        Self::draw_root(batch, atlas, game, scissor, false, draw)
    }

    fn draw_root(
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        game: Option<&ClientGame<'_>>,
        scissor: Option<[f32; 4]>,
        decorating: bool,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        let head_skins = HeadSkinJournal::default();
        GameGuiCanvas {
            batch,
            atlas,
            game,
            head_skins: &head_skins,
            pose: IDENTITY,
            scissor,
            decorating,
        }
        .group(draw)?;
        if let Some(game) = game
            && !head_skins.is_empty()
        {
            game.commit_head_skin_journal(head_skins);
        }
        Ok(())
    }

    fn group(
        self,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        let GameGuiCanvas {
            batch,
            atlas,
            game,
            head_skins,
            pose,
            scissor,
            decorating,
        } = self;
        head_skins.scope(|| {
            batch.draw_scope(&atlas.font, |batch| {
                let start = batch.vertices.len();
                draw(&mut GameGuiCanvas {
                    batch,
                    atlas,
                    game,
                    head_skins,
                    pose,
                    scissor,
                    decorating,
                })?;
                if let Some([left, top, right, bottom]) = scissor {
                    batch.clip_since(start, left, top, right, bottom);
                }
                Ok(())
            })
        })
    }

    /// Validate the posed slot before asking its model or bar hooks to run. Oversized models
    /// and count text may extend beyond the slot; `paint` validates their emitted vertices.
    fn check_item_geometry(&self, [x, y]: [f32; 2]) -> Result<(), GuiDrawError> {
        let [right, bottom] = rect_end([x, y, 16.0, 16.0])?;
        if self.pose == IDENTITY
            || [[x, y], [right, y], [right, bottom], [x, bottom]]
                .into_iter()
                .flat_map(|point| pose_point(self.pose, point))
                .all(f32::is_finite)
        {
            Ok(())
        } else {
            Err(GuiDrawError::NotFinite)
        }
    }

    /// An individual call can be refused even when the caller chooses to commit its group.
    fn paint(&mut self, emit: impl FnOnce(&mut HudBatch, &HudAtlas)) -> Result<(), GuiDrawError> {
        self.paint_in_pose(self.pose, |batch, atlas| {
            emit(batch, atlas);
            Ok(())
        })
    }

    fn paint_in_pose<T>(
        &mut self,
        pose: Pose,
        emit: impl FnOnce(&mut HudBatch, &HudAtlas) -> Result<T, GuiDrawError>,
    ) -> Result<T, GuiDrawError> {
        let atlas = self.atlas;
        self.batch.draw_scope(&atlas.font, |batch| {
            let start = batch.vertices.len();
            let output = emit(batch, atlas)?;
            for vertex in &mut batch.vertices[start..] {
                if pose != IDENTITY {
                    vertex.pos = pose_point(pose, vertex.pos);
                }
                if !vertex.pos.into_iter().chain(vertex.uv).all(f32::is_finite) {
                    return Err(GuiDrawError::NotFinite);
                }
            }
            Ok(output)
        })
    }
}

impl GameGuiCanvas<'_, '_> {
    /// [`GuiCanvas::fluid`] once the fluid's still sprite and `0xAARRGGBB` tint are known: the
    /// sprite (a packed block atlas name, `minecraft:` optional, else a registered atlas's; the
    /// missing sprite when the atlas has neither) tiled at its own pixel size from the top left
    /// of the sorted `rect`, the last column and row cut, as JEI's `drawTiledSprite` does. Only
    /// what can show through the canvas's clip (its scissor, else the screen) is emitted, on the
    /// grid anchored at `rect`'s top left, so the work is bounded by the clip's GUI pixels, not by
    /// `rect` or the pose's scale: see [`fluid_tiles`]. Tile phases are taken relative to the
    /// clip, and corners are posed in `f64` before narrowing, so a far-off `rect` keeps its tiles.
    fn paint_fluid(&mut self, still: &str, tint: u32, rect: [f32; 4]) -> Result<(), GuiDrawError> {
        let [x1, y1] = rect_end(rect)?;
        let atlas = self.atlas;
        let Some(tile) = atlas
            .texture_rect(still)
            .or_else(|| atlas.sprites.get(sunrise_client::hud::MISSING_SPRITE))
            .copied()
            .filter(|tile| tile.w > 0 && tile.h > 0)
        else {
            return Ok(());
        };
        let page = atlas.texture_atlas_page(still);
        let (left, right) = (rect[0].min(x1), rect[0].max(x1));
        let (top, bottom) = (rect[1].min(y1), rect[1].max(y1));
        let [sx0, sy0, sx1, sy1] =
            self.scissor
                .unwrap_or([0.0, 0.0, self.batch.width, self.batch.height]);
        let clip = [
            sx0.max(0.0),
            sy0.max(0.0),
            sx1.min(self.batch.width),
            sy1.min(self.batch.height),
        ];
        let (tw, th) = (f64::from(tile.w), f64::from(tile.h));
        let fill = [left, top, right, bottom].map(f64::from);
        let pose = self.pose;
        let Some(plan) = fluid_tiles(pose, clip, fill, [tw, th]) else {
            return Ok(());
        };
        // The corners below are already posed, in `f64`.
        self.paint_in_pose(IDENTITY, |batch, _| {
            let color = abgr(tint);
            let start = batch.vertices.len();
            let (du, dv) = (tile.u1 - tile.u0, tile.v1 - tile.v0);
            let posed = |x: f64, y: f64| {
                let [x, y] = pose_f64(pose, [x, y]);
                [x as f32, y as f32]
            };
            match plan {
                FluidTiles::Tiles([columns, rows]) => {
                    // Integer counters over the visible tiles only.
                    for row in 0..rows.count {
                        let (y, h) = rows.tile(row);
                        for column in 0..columns.count {
                            let (x, w) = columns.tile(column);
                            let (u1, v1) = (
                                tile.u0 + du * (w / tw) as f32,
                                tile.v0 + dv * (h / th) as f32,
                            );
                            batch.quad(
                                [
                                    posed(x, y),
                                    posed(x + w, y),
                                    posed(x + w, y + h),
                                    posed(x, y + h),
                                ],
                                [[tile.u0, tile.v0], [u1, tile.v0], [u1, v1], [tile.u0, v1]],
                                color,
                            );
                        }
                    }
                }
                FluidTiles::Pixels([x0, y0, x1, y1]) => {
                    for py in y0..y1 {
                        for px in x0..x1 {
                            let centre = [px as f64 + 0.5, py as f64 + 0.5];
                            let [lx, ly] = unpose_f64(pose, centre);
                            if !(lx >= fill[0] && lx < fill[2] && ly >= fill[1] && ly < fill[3]) {
                                continue;
                            }
                            let uv = [
                                tile.u0 + du * (tile_phase(lx, fill[0], tw) / tw) as f32,
                                tile.v0 + dv * (tile_phase(ly, fill[1], th) / th) as f32,
                            ];
                            let [x, y] = [px as f32, py as f32];
                            batch.quad(
                                [[x, y], [x + 1.0, y], [x + 1.0, y + 1.0], [x, y + 1.0]],
                                [uv; 4],
                                color,
                            );
                        }
                    }
                }
            }
            if let Some(page) = page {
                batch.blend_since(
                    start,
                    sunrise_client::hud::Blend::AtlasPage(
                        page,
                        sunrise_client::hud::AtlasBlend::Alpha,
                    ),
                );
            }
            Ok(())
        })
    }
}

/// Multiplies the vertices from `start` on by `tint` (packed ABGR), as `item_tinted` does. An
/// OptiFine CIT glint layer's vertices (`Blend::CitGlint`) carry the layer's slot in red and its
/// intensity in alpha, so only their alpha is multiplied, by the tint's alpha: the slot stays,
/// and the layer fades with the item.
fn tint_vertices(batch: &mut HudBatch, start: usize, tint: u32) {
    let cit: Vec<std::ops::Range<usize>> = batch
        .blends
        .iter()
        .filter(|(range, blend)| {
            matches!(blend, sunrise_client::hud::Blend::CitGlint(_)) && range.end as usize > start
        })
        .map(|(range, _)| range.start as usize..range.end as usize)
        .collect();
    let alpha = tint | 0x00FF_FFFF;
    for (i, vertex) in batch.vertices.iter_mut().enumerate().skip(start) {
        let tint = if cit.iter().any(|range| range.contains(&i)) {
            alpha
        } else {
            tint
        };
        vertex.color = multiply_abgr(vertex.color, tint);
    }
}

/// What a fluid fill emits to show through a clip, from [`fluid_tiles`].
enum FluidTiles {
    /// The visible tiles, columns then rows, each quad posed corner by corner.
    Tiles([TileSpan; 2]),
    /// More visible tiles than GUI pixel cells (tiles under a pixel on average, from a
    /// minifying pose): one screen-space quad per GUI pixel cell `[x0, y0, x1, y1)` whose centre
    /// is inside the fill, with the uv of the texel the tiling puts at that centre, as point
    /// sampling the full tiling there would.
    Pixels([i64; 4]),
}

/// One axis of the visible tiles: the first's local start, how many, where the fill ends, and
/// the tile size.
#[derive(Clone, Copy)]
struct TileSpan {
    start: f64,
    count: u64,
    end: f64,
    size: f64,
}

impl TileSpan {
    /// The `n`th visible tile's local start and extent, the fill's last one cut.
    fn tile(self, n: u64) -> (f64, f64) {
        let at = self.start + n as f64 * self.size;
        (at, (self.end - at).clamp(0.0, self.size))
    }
}

/// How far into its tile `x` is, on a grid of `size` starting at `origin`. Each remainder is
/// exact, so a far-off `origin` (`-1e30`) keeps the phase.
fn tile_phase(x: f64, origin: f64, size: f64) -> f64 {
    let phase = (x.rem_euclid(size) - origin.rem_euclid(size)).rem_euclid(size);
    if phase < size { phase } else { 0.0 }
}

/// `pose_point` in `f64`: (a x + c y + e, b x + d y + f).
fn pose_f64(pose: Pose, [x, y]: [f64; 2]) -> [f64; 2] {
    let [a, b, c, d, e, f] = pose.map(f64::from);
    [a * x + c * y + e, b * x + d * y + f]
}

/// The inverse of [`pose_f64`]; the translation is taken off first so a far-off one stays exact.
fn unpose_f64(pose: Pose, [x, y]: [f64; 2]) -> [f64; 2] {
    let [a, b, c, d, e, f] = pose.map(f64::from);
    let det = a * d - c * b;
    let (x, y) = (x - e, y - f);
    [(d * x - c * y) / det, (a * y - b * x) / det]
}

/// What a fill over the sorted local `[left, top, right, bottom]`, tiled every `tile` GUI pixels
/// from its top left, emits to show through the screen-space `clip` under `pose`. The visible
/// part is the fill meeting the bounding box of the clip's corners mapped back through the
/// inverse pose; the first tile on each axis comes from the phase of that box's edge, not from
/// an index counted from a distant origin. When those tiles outnumber the GUI pixel cells of the
/// clip meeting the posed fill, [`FluidTiles::Pixels`] covers the cells instead, so either way
/// the quads are bounded by the clip's GUI pixels. A mod's `rect` and pose are any finite `f32`
/// (`1e30` wide, a scale of `1 / 256`). `None` when nothing can show: an empty clip, a fill
/// outside it, or a singular pose (which flattens the fill to zero area).
fn fluid_tiles(
    pose: Pose,
    [cx0, cy0, cx1, cy1]: [f32; 4],
    [left, top, right, bottom]: [f64; 4],
    [tw, th]: [f64; 2],
) -> Option<FluidTiles> {
    if !(cx1 > cx0 && cy1 > cy0) {
        return None;
    }
    let [a, b, c, d, ..] = pose.map(f64::from);
    let det = a * d - c * b;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let bounds = |points: [[f64; 2]; 4]| {
        let min = |axis: usize| points.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
        let max = |axis: usize| {
            points
                .iter()
                .map(|p| p[axis])
                .fold(f64::NEG_INFINITY, f64::max)
        };
        [min(0), min(1), max(0), max(1)]
    };
    let corners = |[x0, y0, x1, y1]: [f64; 4]| [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let clip = [cx0, cy0, cx1, cy1].map(f64::from);
    let local = bounds(corners(clip).map(|p| unpose_f64(pose, p)));
    let (x0, x1) = (local[0].max(left), local[2].min(right));
    let (y0, y1) = (local[1].max(top), local[3].min(bottom));
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    let posed = bounds(corners([left, top, right, bottom]).map(|p| pose_f64(pose, p)));
    let cells = [
        posed[0].max(clip[0]).floor(),
        posed[1].max(clip[1]).floor(),
        posed[2].min(clip[2]).ceil(),
        posed[3].min(clip[3]).ceil(),
    ];
    if !(cells[2] > cells[0] && cells[3] > cells[1]) {
        return None;
    }
    let span = |lo: f64, hi: f64, origin: f64, end: f64, size: f64| {
        let start = (lo - tile_phase(lo, origin, size)).max(origin);
        TileSpan {
            start,
            count: ((hi - start) / size).ceil() as u64,
            end,
            size,
        }
    };
    let columns = span(x0, x1, left, right, tw);
    let rows = span(y0, y1, top, bottom, th);
    let area = (cells[2] - cells[0]) * (cells[3] - cells[1]);
    Some(if columns.count as f64 * rows.count as f64 > area {
        FluidTiles::Pixels(cells.map(|edge| edge as i64))
    } else {
        FluidTiles::Tiles([columns, rows])
    })
}

/// Two packed colours multiplied channel by channel, as a shader multiplies a vertex colour by
/// `ColorModulator`: `u32::MAX` leaves `color` unchanged.
fn multiply_abgr(color: u32, tint: u32) -> u32 {
    let channel = |shift: u32| {
        let product = ((color >> shift) & 0xFF) * ((tint >> shift) & 0xFF);
        // Rounded division by 255, exact for every product of two bytes.
        ((product + 128 + ((product + 128) >> 8)) >> 8) << shift
    };
    channel(0) | channel(8) | channel(16) | channel(24)
}

/// The opposite corner of a finite `[x, y, w, h]`, including finite endpoint arithmetic.
fn rect_end([x, y, w, h]: [f32; 4]) -> Result<[f32; 2], GuiDrawError> {
    let end = [x + w, y + h];
    if [x, y].into_iter().chain(end).all(f32::is_finite) {
        Ok(end)
    } else {
        Err(GuiDrawError::NotFinite)
    }
}

/// The checks `blit` and `texture` share, before either looks its texture up: a finite
/// destination rectangle and `uv`, and no zero `tex` side (the divisor of the UVs).
fn check_blit_region(dst: [f32; 4], uv: [f32; 2], tex: [i32; 2]) -> Result<(), GuiDrawError> {
    rect_end(dst)?;
    if !uv.into_iter().all(f32::is_finite) {
        return Err(GuiDrawError::NotFinite);
    }
    if tex.contains(&0) {
        return Err(GuiDrawError::BadBlitRegion);
    }
    Ok(())
}

/// The API count is nonzero, but vanilla also treats air as an empty stack.
fn drawable_stack(stack: &ItemStack) -> Result<Option<EngineStack>, GuiDrawError> {
    let stack = stack_value(stack).map_err(GuiDrawError::Stack)?;
    Ok((&*stack.item != "minecraft:air").then_some(stack))
}

impl GuiCanvas for GameGuiCanvas<'_, '_> {
    fn size(&self) -> [f32; 2] {
        [self.batch.width, self.batch.height]
    }

    fn scope(
        &mut self,
        ops: &[PoseOp],
        scissor: Option<[f32; 4]>,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        let mut pose = self.pose;
        for &op in ops {
            let finite = match op {
                PoseOp::Translate(x, y) | PoseOp::Scale(x, y) => x.is_finite() && y.is_finite(),
                PoseOp::Rotate(angle) => angle.is_finite(),
            };
            if !finite {
                return Err(GuiDrawError::NotFinite);
            }
            pose_apply(&mut pose, op);
            if !pose.into_iter().all(f32::is_finite) {
                return Err(GuiDrawError::NotFinite);
            }
        }
        let clip = if let Some(rect @ [x, y, w, h]) = scissor {
            let end = rect_end(rect)?;
            let [x, y] = pose_point(pose, [x, y]);
            let [right, bottom] = pose_point(pose, end);
            // ScreenRectangle transforms the two ordered corners, not an enclosing box;
            // it floors the origin and extents separately.
            let [left, top] = [x.floor(), y.floor()];
            let [width, height] = [(right - x).floor(), (bottom - y).floor()];
            let mut clip = [left, top, left + width, top + height];
            if !clip.into_iter().all(f32::is_finite) {
                return Err(GuiDrawError::NotFinite);
            }
            let [x0, y0, x1, y1] =
                self.scissor
                    .unwrap_or([0.0, 0.0, self.batch.width, self.batch.height]);
            clip = [
                clip[0].max(x0),
                clip[1].max(y0),
                clip[2].min(x1),
                clip[3].min(y1),
            ];
            if w <= 0.0 || h <= 0.0 || clip[2] <= clip[0] || clip[3] <= clip[1] {
                clip = [0.0; 4];
            }
            Some(clip)
        } else {
            self.scissor
        };
        GameGuiCanvas {
            batch: &mut *self.batch,
            pose,
            scissor: clip,
            ..*self
        }
        .group(draw)
    }

    fn fill(&mut self, rect @ [x, y, ..]: [f32; 4], argb: u32) -> Result<(), GuiDrawError> {
        let [x1, y1] = rect_end(rect)?;
        let (left, right) = (x.min(x1), x.max(x1));
        let (top, bottom) = (y.min(y1), y.max(y1));
        self.paint(|batch, atlas| {
            batch.fill_quad(
                atlas,
                [[left, top], [right, top], [right, bottom], [left, bottom]],
                abgr(argb),
            );
        })
    }

    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) -> Result<(), GuiDrawError> {
        rect_end(rect)?;
        self.paint(|batch, atlas| paint_fill_gradient(batch, atlas, rect, top, bottom))
    }

    fn triangles(&mut self, triangles: &[[GuiVertex; 3]]) -> Result<(), GuiDrawError> {
        // `paint` refuses a corner that is not finite once posed, the identity pose included.
        self.paint(|batch, atlas| {
            for triangle in triangles {
                batch.fill_triangle_shaded(
                    atlas,
                    triangle.map(|corner| corner.pos),
                    triangle.map(|corner| abgr(corner.argb)),
                );
            }
        })
    }

    fn text(
        &mut self,
        text: &Text,
        at @ [x, y]: [f32; 2],
        argb: u32,
        shadow: bool,
    ) -> Result<(), GuiDrawError> {
        if !at.into_iter().all(f32::is_finite) {
            return Err(GuiDrawError::NotFinite);
        }
        let text = text.try_into_engine(&()).map_err(GuiDrawError::Text)?;
        // GuiGraphicsExtractor.text skips extraction when the default alpha is zero.
        if argb >> 24 == 0 {
            return Ok(());
        }
        self.paint(|batch, atlas| {
            batch.text_styled(atlas, &text, x, y, abgr(argb), shadow);
        })
    }

    fn text_with_word_wrap(
        &mut self,
        text: &Text,
        at: [f32; 2],
        spec: TextLayoutSpec,
        argb: u32,
        shadow: bool,
    ) -> Result<TextLayoutSize, GuiDrawError> {
        let layout = LabelLayout::try_from(spec).map_err(GuiDrawError::TextLayout)?;
        if !at.into_iter().all(f32::is_finite) {
            return Err(GuiDrawError::NotFinite);
        }
        let mut pose = self.pose;
        pose_apply(&mut pose, PoseOp::Translate(at[0], at[1]));
        pose_apply(&mut pose, PoseOp::Scale(spec.scale, spec.scale));
        if !pose.into_iter().all(f32::is_finite) {
            return Err(GuiDrawError::NotFinite);
        }
        let text = text.try_into_engine(&()).map_err(GuiDrawError::Text)?;
        self.paint_in_pose(pose, |batch, atlas| {
            layout
                .visit_visual(&atlas.font, &text, |line, [x, y]| {
                    if argb >> 24 != 0 {
                        batch.text_visual(atlas, line, x, y, abgr(argb), shadow);
                    }
                })
                .map_err(GuiDrawError::TextLayout)
        })
    }

    fn item(&mut self, stack: &ItemStack, at: [f32; 2], seed: i32) -> Result<(), GuiDrawError> {
        let Some(stack) = drawable_stack(stack)? else {
            return Ok(());
        };
        self.check_item_geometry(at)?;
        let game = self.game.ok_or(GuiDrawError::Unavailable)?;
        let head_skins = self.head_skins;
        head_skins.scope(|| {
            self.paint(|batch, _| {
                game.draw_item_model_with_skins(batch, &stack, at, seed, Some(head_skins));
            })
        })
    }

    fn item_tinted(
        &mut self,
        stack: &ItemStack,
        at: [f32; 2],
        seed: i32,
        argb: u32,
    ) -> Result<(), GuiDrawError> {
        if argb >> 24 == 0 {
            return Ok(());
        }
        let start = self.batch.vertices.len();
        self.item(stack, at, seed)?;
        let tint = abgr(argb);
        if tint != u32::MAX {
            tint_vertices(self.batch, start, tint);
        }
        Ok(())
    }

    fn item_decorations(
        &mut self,
        stack: &ItemStack,
        at @ [x, y]: [f32; 2],
        count: Option<&str>,
    ) -> Result<(), GuiDrawError> {
        let Some(stack) = drawable_stack(stack)? else {
            return Ok(());
        };
        self.check_item_geometry(at)?;
        let game = self.game.ok_or(GuiDrawError::Unavailable)?;
        self.paint(|batch, _| {
            game.draw_vanilla_item_decorations(batch, &stack, x, y, count);
        })?;
        // Its draws already use this pose and scissor. Only the vanilla suffix above needs
        // transforming, not the whole decorator call.
        if !self.decorating {
            game.native_item_decorations(
                &stack,
                x,
                y,
                &mut GameGuiCanvas {
                    batch: &mut *self.batch,
                    decorating: true,
                    ..*self
                },
            );
        }
        Ok(())
    }

    fn blit_sprite(
        &mut self,
        name: &str,
        dst: [f32; 4],
        region: Option<GuiSpriteRegion>,
        argb: u32,
        blend: Blend,
    ) -> Result<(), GuiDrawError> {
        rect_end(dst)?;
        if matches!(blend, Blend::Vignette | Blend::Nausea) {
            return Err(GuiDrawError::UnsupportedSpriteBlend);
        }
        let region = if let Some(GuiSpriteRegion {
            size: [w, h],
            offset: [u, v],
        }) = region
        {
            if w <= 0 || h <= 0 {
                return Err(GuiDrawError::BadSpriteRegion);
            }
            Some([w, h, u, v])
        } else {
            None
        };
        let Some(GuiSprite::Present { rect, .. }) = self.atlas.gui_lookup(name) else {
            return Err(GuiDrawError::UnknownSprite);
        };
        let quad = ResolvedGuiQuad {
            rect,
            dst,
            color: argb,
        };
        self.paint(|batch, atlas| quad.paint_sprite(batch, atlas, name, region, blend))
    }

    fn atlas_sprite(&mut self, name: &str, dst: [f32; 4], argb: u32) -> Result<(), GuiDrawError> {
        rect_end(dst)?;
        let rect = *self
            .atlas
            .sprites
            .get(name)
            .ok_or(GuiDrawError::UnknownSprite)?;
        let quad = ResolvedGuiQuad {
            rect,
            dst,
            color: argb,
        };
        self.paint(|batch, _| quad.paint(batch))
    }

    fn fluid(&mut self, fluid: &FluidStack, rect: [f32; 4]) -> Result<(), GuiDrawError> {
        rect_end(rect)?;
        if fluid.key().is_none() {
            return Err(GuiDrawError::UnknownFluid);
        }
        let id = mod_api::__private::fluid_identity_name(&fluid.fluid);
        // `FluidStateModelSet.get(fluid)` as an item's fluid layer reads it: the engine's one
        // appearance (a registered model, the look, vanilla's table). A fluid no model knows gets
        // `ModelBakery.MissingModels.fluid`, `missingno` untinted; the empty fluid draws nothing.
        match sunrise_client::fluid_models::item_appearance(&id) {
            Some(appearance) => self.paint_fluid(&appearance.still, appearance.tint as u32, rect),
            None if id == "minecraft:empty" => Ok(()),
            None => self.paint_fluid(sunrise_client::hud::MISSING_SPRITE, u32::MAX, rect),
        }
    }

    fn blit(
        &mut self,
        texture: &str,
        dst: [f32; 4],
        GuiBlitRegion { uv, src, tex }: GuiBlitRegion,
        argb: u32,
        blend: Blend,
    ) -> Result<(), GuiDrawError> {
        check_blit_region(dst, uv, tex)?;
        let rect = *self
            .atlas
            .texture_rect(texture)
            .ok_or(GuiDrawError::UnknownTexture)?;
        let quad = ResolvedGuiQuad {
            rect,
            dst,
            color: argb,
        };
        self.paint(|batch, atlas| {
            quad.paint_blit(
                batch,
                atlas.texture_atlas_page(texture),
                uv,
                src,
                tex,
                blend,
            )
        })
    }

    fn texture(
        &mut self,
        id: &mod_api::Identifier,
        dst: [f32; 4],
        GuiBlitRegion { uv, src, tex }: GuiBlitRegion,
        argb: u32,
    ) -> Result<(), GuiDrawError> {
        check_blit_region(dst, uv, tex)?;
        // A live dynamic texture first, then the resource stack, as one id space; an id in
        // neither is refused here, and only one released after this call draws the missing texture.
        let dynamic = self
            .game
            .and_then(|game| game.client.dynamic_textures.get(id));
        if dynamic.is_none() && !self.atlas.has_texture(id) {
            return Err(GuiDrawError::UnknownTexture);
        }
        self.paint(|batch, atlas| match dynamic {
            Some((handle, size)) => {
                batch.dynamic_texture(handle, size, dst, uv, [src, tex], abgr(argb))
            }
            None => batch.texture(atlas, id, dst, uv, [src, tex], abgr(argb)),
        })
    }

    fn player_face(
        &mut self,
        uuid: u128,
        at: [f32; 2],
        size: f32,
        hat: bool,
        flip: bool,
        argb: u32,
    ) -> Result<(), GuiDrawError> {
        let dst = [at[0], at[1], size, size];
        rect_end(dst)?;
        let game = self.game.ok_or(GuiDrawError::Unavailable)?;
        let head_skins = self.head_skins;
        head_skins.scope(|| {
            let mut skin_name = String::new();
            let skin = game.player_face_skin_with_journal(uuid, Some(head_skins), &mut skin_name);
            self.paint(|batch, atlas| {
                paint_player_face(batch, atlas, skin, dst, hat, flip, argb);
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use sunrise_assets::resource_source::ResourceSource;
    use sunrise_client::hud::{Blend as HudBlend, Rect};

    /// The multiply rounds to the nearest byte for every pair of channels, and white is neutral.
    #[test]
    fn multiplying_packed_colours_rounds_each_channel() {
        for a in 0..=255u32 {
            for b in 0..=255u32 {
                let want = ((a * b) as f64 / 255.0).round() as u32;
                assert_eq!(multiply_abgr(a, b), want, "{a} * {b}");
                assert_eq!(multiply_abgr(a << 24, b << 24), want << 24, "{a} * {b}");
            }
        }
        assert_eq!(multiply_abgr(0x8040_20FF, u32::MAX), 0x8040_20FF);
        assert_eq!(multiply_abgr(0xFFFF_FFFF, 0x80FF_0000), 0x80FF_0000);
    }

    /// A CIT glint layer's vertices keep their slot (red) and take only the tint's alpha on their
    /// intensity; every other vertex is multiplied by the whole tint.
    #[test]
    fn a_tint_keeps_a_cit_glint_layers_slot() {
        let mut batch = HudBatch::scratch([320, 240]);
        let quad = |batch: &mut HudBatch, color: u32| {
            batch.quad_glint(
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                [[0.0; 2]; 4],
                [[0.0; 2]; 4],
                color,
            );
        };
        // Drawn before the item: not tinted.
        quad(&mut batch, 0xFF00_00FF);
        let start = batch.vertices.len();
        quad(&mut batch, u32::MAX);
        let cit = batch.vertices.len();
        // Slot 7 at intensity 200: red 7, alpha 200 (packed ABGR).
        quad(&mut batch, 0xC800_0007);
        batch.blend_since(cit, HudBlend::CitGlint(0));
        let tint = abgr(0x80FF_4000);
        tint_vertices(&mut batch, start, tint);
        let colors: Vec<u32> = batch.vertices.iter().map(|v| v.color).collect();
        assert!(colors[..start].iter().all(|&c| c == 0xFF00_00FF));
        assert!(colors[start..cit].iter().all(|&c| c == tint));
        // 200 * 128 / 255 rounds to 100; the slot stays 7.
        assert!(
            colors[cit..].iter().all(|&c| c == 0x6400_0007),
            "{colors:x?}"
        );
    }

    /// `item_tinted` multiplies every vertex `item` draws, the enchantment glint's included (the
    /// glint shader multiplies by its white vertex colour): opaque white draws `item` itself and
    /// a zero alpha draws nothing.
    #[test]
    #[ignore = "daily"]
    fn a_tinted_item_multiplies_the_vertices_the_item_draws() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            // The glint options a frame captures (full strength), as a frame drawing items has.
            game.client.glint_frame = sunrise_client::item_render::GlintFrame {
                millis: 0,
                speed: 1.0,
                alpha: 1.0,
                sprite: game
                    .client
                    .gui
                    .atlas
                    .sprites
                    .contains_key("misc/enchanted_glint_item"),
            };
            assert!(
                game.client.glint_frame.sprite,
                "the glint's sprite is in the atlas"
            );
            // A plain item, and one that always has the glint.
            for (item, foil) in [
                ("minecraft:diamond_sword", false),
                ("minecraft:enchanted_golden_apple", true),
            ] {
                let stack = mod_api::ItemStack::new(mod_api::__private::item_key(item).unwrap(), 1)
                    .unwrap();
                let draw = |argb: Option<u32>| {
                    let mut batch = HudBatch::scratch([320, 240]);
                    GameGuiCanvas::draw(
                        &mut batch,
                        &game.client.gui.atlas,
                        Some(&*game),
                        None,
                        &mut |root| match argb {
                            None => root.item(&stack, [4.0, 4.0], 0),
                            Some(argb) => root.item_tinted(&stack, [4.0, 4.0], 0, argb),
                        },
                    )
                    .unwrap();
                    batch
                        .vertices
                        .iter()
                        .map(|v| (v.pos, v.uv, v.color))
                        .collect::<Vec<_>>()
                };
                let plain = draw(None);
                assert!(!plain.is_empty());
                assert_eq!(draw(Some(0xFFFF_FFFF)), plain);
                assert!(draw(Some(0x00FF_FFFF)).is_empty());
                let tint = 0x80FF_4000;
                let tinted = draw(Some(tint));
                assert_eq!(tinted.len(), plain.len());
                for ((pos, uv, color), (plain_pos, plain_uv, plain_color)) in
                    tinted.iter().zip(&plain)
                {
                    assert_eq!((pos, uv), (plain_pos, plain_uv));
                    assert_eq!(*color, multiply_abgr(*plain_color, abgr(tint)));
                }
                // The glint's quads (atlas U at 2 or more, the shader's glint marker) are drawn white,
                // so tinted they carry the tint itself, which the glint shader multiplies by.
                let glint: Vec<u32> = tinted
                    .iter()
                    .filter(|(_, uv, _)| uv[0] >= 2.0)
                    .map(|&(_, _, color)| color)
                    .collect();
                assert_eq!(!glint.is_empty(), foil, "{item}");
                assert!(glint.iter().all(|&color| color == abgr(tint)), "{item}");
            }
        });
    }

    /// `blit` and `texture` refuse a bad region the same way and before their lookup (#5994),
    /// and `blit` refuses a name outside the atlas, which `texture` is for, as documented.
    #[test]
    fn blit_and_texture_share_their_region_checks() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let id = mod_api::Identifier::parse("test:dynamic/panel").unwrap();
        let region = |uv, tex| GuiBlitRegion {
            uv,
            src: [4, 4],
            tex,
        };
        let cases = [
            (
                [0.0, 0.0, f32::NAN, 4.0],
                region([0.0, 0.0], [4, 4]),
                GuiDrawError::NotFinite,
            ),
            (
                [0.0; 4],
                region([f32::INFINITY, 0.0], [4, 4]),
                GuiDrawError::NotFinite,
            ),
            (
                [0.0; 4],
                region([0.0, 0.0], [4, 0]),
                GuiDrawError::BadBlitRegion,
            ),
            (
                [0.0; 4],
                region([0.0, 0.0], [4, 4]),
                GuiDrawError::UnknownTexture,
            ),
        ];
        for (dst, region, error) in cases {
            let mut batch = HudBatch::scratch([320, 240]);
            let blit = GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                root.blit("test:dynamic/panel", dst, region, u32::MAX, Blend::Textured)
            });
            assert_eq!(blit, Err(error.clone()), "blit {dst:?}");
            let texture = GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                root.texture(&id, dst, region, u32::MAX)
            });
            assert_eq!(texture, Err(error), "texture {dst:?}");
        }
    }

    #[test]
    fn gui_groups_clip_posed_leaves_and_restore_vertices_and_blends_on_abort() {
        let mut atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        atlas.sprites.insert(
            "test:canvas".into(),
            Rect {
                u0: 0.25,
                v0: 0.125,
                u1: 0.75,
                v1: 0.625,
                w: 4,
                h: 4,
            },
        );
        let crosshair = |canvas: &mut dyn GuiCanvas, dst| {
            canvas.blit(
                "test:canvas",
                dst,
                GuiBlitRegion {
                    uv: [0.0, 0.0],
                    src: [4, 4],
                    tex: [4, 4],
                },
                0x8011_2233,
                Blend::Crosshair,
            )
        };
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            crosshair(root, [2.0, 3.0, 2.0, 2.0])
        })
        .unwrap();

        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            assert_eq!(root.size(), [320.0, 240.0]);
            root.scope(
                &[PoseOp::Translate(10.75, 20.25)],
                Some([0.0, 0.0, 5.75, 5.75]),
                &mut |parent| {
                    parent.scope(
                        &[PoseOp::Translate(1.0, 1.0), PoseOp::Scale(2.0, 3.0)],
                        Some([0.25, 0.25, 3.0, 3.0]),
                        &mut |child| crosshair(child, [0.0, 0.0, 4.0, 4.0]),
                    )?;
                    for angle in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                        assert_eq!(
                            parent.scope(
                                &[PoseOp::Translate(2.0, 0.0), PoseOp::Rotate(angle)],
                                None,
                                &mut |_| panic!("a refused rotation must not invoke the callback"),
                            ),
                            Err(GuiDrawError::NotFinite),
                        );
                    }
                    assert_eq!(
                        parent.scope(
                            &[PoseOp::Scale(f32::MAX, 1.0), PoseOp::Scale(2.0, 1.0)],
                            None,
                            &mut |_| {
                                panic!("a scope with a non-finite composed pose must not run")
                            }
                        ),
                        Err(GuiDrawError::NotFinite)
                    );
                    assert_eq!(
                        parent.scope(&[], Some([f32::MAX, 0.0, f32::MAX, 1.0]), &mut |_| {
                            panic!("a scope with a non-finite clip edge must not run")
                        }),
                        Err(GuiDrawError::NotFinite)
                    );

                    // Both discarded scopes append with the existing blend, extending its run.
                    assert_eq!(
                        parent.scope(&[], None, &mut |child| {
                            crosshair(child, [0.0, 0.0, 1.0, 1.0])?;
                            Err(GuiDrawError::Abort)
                        }),
                        Err(GuiDrawError::Abort)
                    );
                    let unwind = catch_unwind(AssertUnwindSafe(|| {
                        parent.scope(&[PoseOp::Translate(1.0, 0.0)], None, &mut |child| {
                            crosshair(child, [0.0, 0.0, 1.0, 1.0])?;
                            panic!("discard on unwind")
                        })
                    }));
                    assert!(unwind.is_err());

                    // Catch a leaf refusal instead of discarding its entire scope.
                    parent.scope(&[PoseOp::Scale(2.0, 1.0)], None, &mut |child| {
                        assert_eq!(
                            crosshair(child, [f32::MAX, 0.0, 1.0, 1.0]),
                            Err(GuiDrawError::NotFinite)
                        );
                        Ok(())
                    })?;
                    parent.fill_gradient([2.0, 2.0, 1.0, 1.0], 0x80AA_1234, 0xFF00_0000)
                },
            )?;
            root.fill([30.0, 40.0, 2.0, 2.0], 0xFFFF_FFFF)
        })
        .unwrap();
        assert_eq!(batch.vertices.len(), 24);
        assert_eq!(batch.vertices[0].pos, [2.0, 3.0]);
        assert_eq!(batch.vertices[6].pos, [11.75, 21.25]);
        assert_eq!(batch.vertices[8].pos, [19.75, 33.25]);
        assert_eq!(batch.vertices[6].uv, [0.25, 0.125]);
        assert_eq!(batch.vertices[6].color, 0x8033_2211);
        assert_eq!(batch.vertices[12].pos, [12.75, 22.25]);
        assert_eq!(batch.vertices[12].color, 0x8034_12AA);
        assert_eq!(batch.vertices[14].color, 0xFF00_0000);
        assert_eq!(batch.vertices[18].pos, [30.0, 40.0]);
        assert_eq!(batch.blends, [(0..12, HudBlend::Invert)]);

        let scissors = [
            (0..6, None),
            (6..12, Some([12.0, 22.0, 15.0, 25.0])),
            (12..18, Some([10.0, 20.0, 15.0, 25.0])),
            (18..24, None),
        ];
        assert_eq!(batch.scissors_in(0..24).collect::<Vec<_>>(), scissors);
        let before = bytemuck::cast_slice::<_, u8>(&batch.vertices).to_vec();
        assert_eq!(
            GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                root.scope(&[], Some([4.0, 7.0, 1.0, 1.0]), &mut |child| {
                    crosshair(child, [4.0, 7.0, 1.0, 1.0])
                })?;
                Err(GuiDrawError::Abort)
            }),
            Err(GuiDrawError::Abort)
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&batch.vertices),
            before.as_slice()
        );
        assert_eq!(batch.blends, [(0..12, HudBlend::Invert)]);
        assert_eq!(batch.scissors_in(0..24).collect::<Vec<_>>(), scissors);
    }

    #[test]
    fn a_refused_blit_leaves_the_batch_as_it_was_before_the_group() {
        const DST: [f32; 4] = [10.0, 20.0, 4.0, 4.0];
        const WHITE: u32 = u32::MAX;
        const REGION: GuiBlitRegion = GuiBlitRegion {
            uv: [0.0, 0.0],
            src: [4, 4],
            tex: [4, 4],
        };
        let mut atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let rect = Rect {
            u0: 0.25,
            v0: 0.5,
            u1: 0.5,
            v1: 0.75,
            w: 4,
            h: 4,
        };
        atlas.gui.insert(
            "test:present".into(),
            GuiSprite::Present {
                rect,
                scaling: None,
            },
        );
        atlas.gui.insert("test:removed".into(), GuiSprite::Missing);
        atlas.sprites.insert("test:removed".into(), rect);
        atlas.sprites.insert("test:live".into(), rect);

        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.atlas_sprite("test:removed", DST, WHITE)?;
            root.blit_sprite("test:present", DST, None, WHITE, Blend::Crosshair)
        })
        .unwrap();
        assert_eq!(batch.vertices.len(), 12);
        assert_eq!(batch.vertices[0].uv, [0.25, 0.5]);
        assert_eq!(batch.vertices[6].uv, [0.25, 0.5]);
        assert_eq!(batch.blends, [(6..12, HudBlend::Invert)]);
        let before = bytemuck::cast_slice::<_, u8>(&batch.vertices).to_vec();

        let mut refuses = |draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
                           expected| {
            assert_eq!(
                GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                    root.scope(&[PoseOp::Translate(2.0, 0.0)], None, &mut |child| {
                        child.blit("test:live", DST, REGION, WHITE, Blend::Crosshair)
                    })?;
                    draw(root)
                }),
                Err(expected)
            );
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&batch.vertices),
                before.as_slice()
            );
            assert_eq!(batch.blends, [(6..12, HudBlend::Invert)]);
        };
        for name in [
            "test:unknown",
            "test:removed",
            "mob_effect/test_missing",
            "test:mob_effect/missing",
        ] {
            refuses(
                &mut |gui| gui.blit_sprite(name, DST, None, WHITE, Blend::Textured),
                GuiDrawError::UnknownSprite,
            );
        }
        refuses(
            &mut |gui| gui.atlas_sprite("test:unknown", DST, WHITE),
            GuiDrawError::UnknownSprite,
        );
        for blend in [Blend::Nausea, Blend::Vignette] {
            refuses(
                &mut |gui| gui.blit_sprite("test:present", DST, None, WHITE, blend),
                GuiDrawError::UnsupportedSpriteBlend,
            );
        }
        for (size, blend) in [
            ([0, 4], Blend::Textured),
            ([4, 0], Blend::Textured),
            ([-1, 4], Blend::Crosshair),
        ] {
            let region = Some(GuiSpriteRegion {
                size,
                offset: [0; 2],
            });
            refuses(
                &mut |gui| gui.blit_sprite("test:present", DST, region, WHITE, blend),
                GuiDrawError::BadSpriteRegion,
            );
        }
        refuses(
            &mut |gui| gui.blit("test:unknown", DST, REGION, WHITE, Blend::Vignette),
            GuiDrawError::UnknownTexture,
        );
        let standalone = mod_api::Identifier::parse("test:textures/gui/standalone.png").unwrap();
        refuses(
            &mut |gui| gui.texture(&standalone, DST, REGION, WHITE),
            GuiDrawError::UnknownTexture,
        );
        for (region, error) in [
            (
                GuiBlitRegion {
                    tex: [0, 4],
                    ..REGION
                },
                GuiDrawError::BadBlitRegion,
            ),
            (
                GuiBlitRegion {
                    uv: [f32::NAN, 0.0],
                    ..REGION
                },
                GuiDrawError::NotFinite,
            ),
        ] {
            refuses(
                &mut |gui| gui.blit("test:live", DST, region, WHITE, Blend::Textured),
                error.clone(),
            );
            refuses(
                &mut |gui| gui.texture(&standalone, DST, region, WHITE),
                error,
            );
        }
    }

    #[test]
    fn raw_blit_negative_source_extent_mirrors_the_texture_region() {
        let mut atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        atlas.sprites.insert(
            "test:mirror".into(),
            Rect {
                u0: 0.25,
                v0: 0.125,
                u1: 0.75,
                v1: 0.625,
                w: 8,
                h: 8,
            },
        );
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |canvas| {
            canvas.blit(
                "test:mirror",
                [10.0, 20.0, 4.0, 4.0],
                GuiBlitRegion {
                    uv: [4.0, 0.0],
                    src: [-4, 4],
                    tex: [8, 8],
                },
                u32::MAX,
                Blend::Textured,
            )
        })
        .unwrap();
        assert_eq!(
            batch
                .vertices
                .iter()
                .map(|v| (v.pos, v.uv))
                .collect::<Vec<_>>(),
            [
                ([10.0, 20.0], [0.50, 0.125]),
                ([14.0, 20.0], [0.25, 0.125]),
                ([14.0, 24.0], [0.25, 0.375]),
                ([10.0, 20.0], [0.50, 0.125]),
                ([14.0, 24.0], [0.25, 0.375]),
                ([10.0, 24.0], [0.50, 0.375]),
            ]
        );
    }

    #[test]
    fn scoped_text_keeps_its_style_and_a_late_conversion_failure_discards_it() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut styled = Text::literal("A");
        styled.style.color = Some("#112233".into());
        styled.style.underlined = Some(true);
        let mut invalid = Text::literal("Not representable");
        invalid.metadata =
            mod_api::Tag::compound([("test:nonfinite", mod_api::Tag::Double(f64::NAN.to_bits()))]);
        let at = [10.0, 20.0];
        let mut batch = HudBatch::scratch([320, 240]);
        assert_eq!(
            GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                root.text(&styled, at, u32::MAX, false)?;
                root.text(&invalid, at, u32::MAX, false)
            }),
            Err(GuiDrawError::Text(mod_api::text::TextFormError::NonFinite))
        );
        assert!(batch.vertices.is_empty());
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.text(&styled, at, u32::MAX, false)
        })
        .unwrap();
        assert!(!batch.vertices.is_empty());
        assert!(
            batch
                .vertices
                .iter()
                .all(|vertex| vertex.color == 0xFF33_2211)
        );
    }

    #[test]
    fn wrapped_text_refuses_nonfinite_scaled_geometry_without_visiting_or_painting() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        for (label, argb, scale) in [
            ("\n", u32::MAX, f32::MAX),
            ("A\nA", 0, f32::MAX / 16.0),
            ("A\nA", u32::MAX, f32::MAX / 16.0),
        ] {
            let text = Text::literal(label);
            let spec = TextLayoutSpec {
                max_width: 100.0,
                align: mod_api::TextAlign::Left,
                scale,
            };
            let layout = LabelLayout::try_from(spec).unwrap();
            assert_eq!(
                layout.visit_components(&atlas.font, &text, &mut |_| {
                    panic!("an invalid label must not publish even its first line")
                }),
                Err(mod_api::TextLayoutError::GeometryNotFinite)
            );

            let mut batch = HudBatch::scratch([320, 240]);
            GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                assert_eq!(
                    root.text_with_word_wrap(&text, [0.0; 2], spec, argb, false),
                    Err(GuiDrawError::TextLayout(
                        mod_api::TextLayoutError::GeometryNotFinite
                    ))
                );
                root.fill([1.0, 2.0, 3.0, 4.0], u32::MAX)
            })
            .unwrap();
            assert_eq!(batch.vertices.len(), 6);
            assert_eq!(batch.vertices[0].pos, [1.0, 2.0]);
            assert!(batch.patches.is_empty());
        }
    }

    #[test]
    fn transparent_styled_text_does_not_extract_inline_profiles_or_glyphs() {
        use mod_api::text::{TextContent, TextObject, TextProfile};
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut profile = TextProfile::default();
        profile.name = Some("CanvasHead".into());
        let mut text = Text::new(TextContent::Object(TextObject::Player {
            profile,
            hat: true,
            fallback: None,
        }));
        text.children.push(Text::literal("Visible only with alpha"));
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.text(&text, [10.0, 20.0], 0x00FF_FFFF, false)
        })
        .unwrap();
        assert!(batch.vertices.is_empty());
        assert!(batch.inline_profiles.is_empty());
        assert!(batch.patches.is_empty());
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.text(&text, [10.0, 20.0], u32::MAX, false)
        })
        .unwrap();
        assert!(!batch.vertices.is_empty());
        assert_eq!(batch.inline_profiles.len(), 1);
    }

    #[test]
    fn gui_scissors_are_empty_on_reflection_inversion_and_disjoint_intersection() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            let mut over_scissor =
                |canvas: &mut dyn GuiCanvas| canvas.fill([-32.0, -32.0, 64.0, 64.0], 0xFFFF_FFFF);
            root.scope(
                &[PoseOp::Scale(-1.0, 1.0)],
                Some([1.0, 1.0, 4.0, 4.0]),
                &mut over_scissor,
            )?;
            root.scope(
                &[PoseOp::Scale(-1.0, 1.0)],
                Some([5.0, 1.0, -4.0, 4.0]),
                &mut over_scissor,
            )?;
            root.scope(&[], Some([0.0, 0.0, 4.0, 4.0]), &mut |parent| {
                parent.scope(&[], Some([6.0, 6.0, 4.0, 4.0]), &mut over_scissor)
            })?;
            // A singular, finite pose is allowed. A scissor with zero transformed width is empty.
            root.scope(
                &[PoseOp::Scale(0.0, 1.0)],
                Some([0.0, 0.0, 4.0, 4.0]),
                &mut over_scissor,
            )
        })
        .unwrap();
        assert_eq!(batch.vertices.len(), 24);
        assert_eq!(
            batch.scissors_in(0..24).collect::<Vec<_>>(),
            [(0..24, Some([0.0; 4]))]
        );
        assert!(batch.blends.is_empty());
    }

    #[test]
    fn player_face_rejects_non_finite_geometry_and_missing_game() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut batch = HudBatch::scratch([320, 240]);
        let uuid = 0x853c_80ef_3c37_49fd_aa49_938b_674a_dae6;
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            assert_eq!(
                root.player_face(uuid, [f32::NAN, 0.0], 8.0, true, false, 0xFFFF_FFFF),
                Err(GuiDrawError::NotFinite)
            );
            assert_eq!(
                root.player_face(uuid, [f32::MAX, 0.0], f32::MAX, true, false, 0xFFFF_FFFF),
                Err(GuiDrawError::NotFinite)
            );
            assert_eq!(
                root.player_face(uuid, [4.0, 4.0], 8.0, true, false, 0xFFFF_FFFF),
                Err(GuiDrawError::Unavailable)
            );
            Ok(())
        })
        .unwrap();
        assert!(batch.vertices.is_empty());
    }

    #[test]
    fn triangles_are_posed_with_their_corner_colours_and_a_non_finite_corner_draws_none() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut batch = HudBatch::scratch([320, 240]);
        let corner = |pos, argb| GuiVertex { pos, argb };
        let white = |pos| corner(pos, 0xFFFF_FFFF);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.scope(
                &[PoseOp::Translate(10.0, 20.0), PoseOp::Scale(2.0, 2.0)],
                None,
                &mut |panel| {
                    panel.triangles(&[[
                        corner([0.0, 0.0], 0xFF11_2233),
                        corner([4.0, 0.0], 0x8044_5566),
                        corner([0.0, 3.0], 0x0077_8899),
                    ]])
                },
            )?;
            // One bad corner refuses the whole call, the finite triangle before it included.
            assert_eq!(
                root.triangles(&[
                    [white([0.0, 0.0]), white([1.0, 0.0]), white([0.0, 1.0])],
                    [white([f32::NAN, 0.0]), white([1.0, 0.0]), white([0.0, 1.0])],
                ]),
                Err(GuiDrawError::NotFinite)
            );
            assert_eq!(
                root.scope(&[PoseOp::Scale(f32::MAX, 1.0)], None, &mut |wide| {
                    wide.triangles(&[[white([0.0, 0.0]), white([2.0, 0.0]), white([0.0, 1.0])]])
                }),
                Err(GuiDrawError::NotFinite)
            );
            Ok(())
        })
        .unwrap();
        let corners = batch.vertices.iter().map(|v| (v.pos, v.color));
        assert_eq!(
            corners.collect::<Vec<_>>(),
            [
                ([10.0, 20.0], 0xFF33_2211),
                ([18.0, 20.0], 0x8066_5544),
                ([10.0, 26.0], 0x0099_8877),
                ([10.0, 20.0], 0xFF33_2211),
                ([10.0, 26.0], 0x0099_8877),
                ([10.0, 26.0], 0x0099_8877),
            ]
        );
    }

    fn fluid_stack(id: &str) -> mod_api::FluidStack {
        mod_platform::__private::part_tests::game_thread();
        mod_api::FluidStack::new(mod_api::__private::fluid_key(id).unwrap(), 1000)
    }

    /// A quarter of the atlas, 16 px square.
    const TILE: Rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 0.5,
        v1: 0.5,
        w: 16,
        h: 16,
    };

    /// An atlas holding `names` as packed sprites only, as block models' sprites are: no GUI
    /// entry, so [`HudAtlas::gui_lookup`] does not answer them.
    fn block_atlas(names: &[&str]) -> HudAtlas {
        let mut atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        for name in names {
            atlas.sprites.insert((*name).into(), TILE);
        }
        atlas
    }

    /// `draw` on a canvas in `pose` over `batch`, outside any group, for
    /// [`GameGuiCanvas::paint_fluid`].
    fn on_canvas(
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        pose: Pose,
        draw: impl FnOnce(&mut GameGuiCanvas<'_, '_>) -> Result<(), GuiDrawError>,
    ) {
        let head_skins = HeadSkinJournal::default();
        draw(&mut GameGuiCanvas {
            batch,
            atlas,
            game: None,
            head_skins: &head_skins,
            pose,
            scissor: None,
            decorating: false,
        })
        .unwrap();
    }

    /// Each six-vertex quad's position and uv extents, `[min x, min y, max x, max y]`.
    fn quad_extents(vertices: &[sunrise_client::hud::HudVertex]) -> Vec<([f32; 4], [f32; 4])> {
        fn extent(
            q: &[sunrise_client::hud::HudVertex],
            f: fn(&sunrise_client::hud::HudVertex) -> [f32; 2],
        ) -> [f32; 4] {
            let (xs, ys) = (q.iter().map(|v| f(v)[0]), q.iter().map(|v| f(v)[1]));
            [
                xs.clone().fold(f32::MAX, f32::min),
                ys.clone().fold(f32::MAX, f32::min),
                xs.fold(f32::MIN, f32::max),
                ys.fold(f32::MIN, f32::max),
            ]
        }
        vertices
            .chunks(6)
            .map(|q| (extent(q, |v| v.pos), extent(q, |v| v.uv)))
            .collect()
    }

    /// Vanilla water and lava draw the block atlas's `block/water_still` and `block/lava_still`,
    /// packed sprites no GUI entry owns, as an item's fluid layer does: water times its item
    /// colour 0xFF3F76E4, lava white. Their appearance is vanilla's fixed table, which every
    /// load of looks rebuilds unchanged, so no other test can alter it.
    #[test]
    fn vanilla_water_and_lava_draw_their_block_still_sprites_as_an_items_fluid_layer_does() {
        let mut atlas = block_atlas(&["block/water_still"]);
        let lava = Rect {
            u0: 0.5,
            u1: 1.0,
            ..TILE
        };
        atlas.sprites.insert("block/lava_still".into(), lava);
        for (fluid, tint, uv) in [
            ("minecraft:water", 0xFF3F_76E4, [0.0, 0.0, 0.5, 0.5]),
            ("minecraft:lava", u32::MAX, [0.5, 0.0, 1.0, 0.5]),
        ] {
            let stack = fluid_stack(fluid);
            let mut batch = HudBatch::scratch([320, 240]);
            GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |canvas| {
                canvas.fluid(&stack, [0.0, 0.0, 16.0, 16.0])
            })
            .unwrap();
            assert_eq!(
                quad_extents(&batch.vertices),
                [([0.0, 0.0, 16.0, 16.0], uv)],
                "{fluid}: one whole tile of its own sprite"
            );
            assert!(
                batch.vertices.iter().all(|v| v.color == abgr(tint)),
                "{fluid}"
            );
        }
    }

    /// A declared fluid draws the engine's one appearance of it (`look_appearance`, which
    /// `item_appearance` answers from): its look's model-atlas sprite tiled at its own pixel size
    /// from the top left, the right and bottom tiles cut, in the canvas pose, times the look's
    /// tint made opaque. The look is given to the canvas, not installed process-wide.
    #[test]
    fn a_declared_fluid_tiles_its_looks_sprite_from_the_top_left_and_cuts_the_last_tiles() {
        use sunrise_mod_package::content::{FluidLook, FluidModel};
        let look = |tint| FluidLook {
            id: "a:oil".into(),
            sprite: Some("a:block/oil".into()),
            tint,
            world: None,
            bucket_item: None,
            measures: Default::default(),
        };
        // A zero alpha draws opaque, as in the world.
        let oil = sunrise_client::fluid_models::look_appearance(&look(Some(0x0011_2233)), &[]);
        assert_eq!(
            (oil.still.as_str(), oil.tint as u32),
            ("a:block/oil", 0xFF11_2233)
        );
        let atlas = block_atlas(&["a:block/oil"]);
        let mut batch = HudBatch::scratch([320, 240]);
        let mut pose = IDENTITY;
        pose_apply(&mut pose, PoseOp::Translate(100.0, 0.0));
        on_canvas(&mut batch, &atlas, pose, |canvas| {
            canvas.paint_fluid(&oil.still, oil.tint as u32, [0.0, 0.0, 24.0, 20.0])
        });
        assert!(batch.vertices.iter().all(|v| v.color == abgr(0xFF11_2233)));
        // 2 columns by 2 rows; the second column is cut to 8 of 16 pixels, half the u span.
        assert_eq!(
            quad_extents(&batch.vertices),
            [
                ([100.0, 0.0, 116.0, 16.0], [0.0, 0.0, 0.5, 0.5]),
                ([116.0, 0.0, 124.0, 16.0], [0.0, 0.0, 0.25, 0.5]),
                ([100.0, 16.0, 116.0, 20.0], [0.0, 0.0, 0.5, 0.125]),
                ([116.0, 16.0, 124.0, 20.0], [0.0, 0.0, 0.25, 0.125]),
            ]
        );

        // A sprite with no tint draws white, and a registered model's still sprite wins.
        let plain = sunrise_client::fluid_models::look_appearance(&look(None), &[]);
        assert_eq!(plain.tint, -1);
        let modelled = sunrise_client::fluid_models::look_appearance(
            &look(None),
            &[FluidModel {
                fluid: "a:oil".into(),
                still: Some("a:block/oil_still".into()),
                ..FluidModel::default()
            }],
        );
        assert_eq!(modelled.still, "a:block/oil_still");
    }

    /// Only the tiles that can show through the clip are emitted, on the grid anchored at the
    /// rectangle's top left, each a whole repeat of the sprite where the rectangle goes on: a
    /// rectangle a million pixels square under a 40 x 20 scissor adds that scissor's six tiles, one
    /// starting above and left of the scissor keeps its grid, and one outside the clip adds
    /// nothing. A rectangle from -1e30 across the screen, and one so far off that `x + 16.0`
    /// rounds back to `x` posed back on screen, keep whole tiles at their exact posed corners.
    #[test]
    fn a_huge_or_far_off_fluid_rectangle_tiles_only_where_the_clip_shows_it() {
        let atlas = block_atlas(&["block/water_still"]);
        let stack = fluid_stack("minecraft:water");
        let whole = [0.0, 0.0, 0.5, 0.5];
        let fill = |scissor: Option<[f32; 4]>, rect: [f32; 4]| {
            let mut batch = HudBatch::scratch([320, 240]);
            GameGuiCanvas::draw(&mut batch, &atlas, None, scissor, &mut |canvas| {
                canvas.fluid(&stack, rect)
            })
            .unwrap();
            batch
        };

        let batch = fill(Some([0.0, 0.0, 40.0, 20.0]), [0.0, 0.0, 1e6, 1e6]);
        assert_eq!(
            quad_extents(&batch.vertices),
            [
                ([0.0, 0.0, 16.0, 16.0], whole),
                ([16.0, 0.0, 32.0, 16.0], whole),
                ([32.0, 0.0, 48.0, 16.0], whole),
                ([0.0, 16.0, 16.0, 32.0], whole),
                ([16.0, 16.0, 32.0, 32.0], whole),
                ([32.0, 16.0, 48.0, 32.0], whole),
            ],
            "3 x 2 repeats of the sprite, not one stretched over the rectangle"
        );

        let batch = fill(Some([0.0, 0.0, 20.0, 20.0]), [-20.0, -20.0, 1e6, 1e6]);
        assert_eq!(
            quad_extents(&batch.vertices),
            [
                ([-4.0, -4.0, 12.0, 12.0], whole),
                ([12.0, -4.0, 28.0, 12.0], whole),
                ([-4.0, 12.0, 12.0, 28.0], whole),
                ([12.0, 12.0, 28.0, 28.0], whole),
            ],
            "the grid starts at the rectangle's corner, not the scissor's"
        );

        // With no scissor the screen clips: 320 / 16 columns of one row, cut to 8 pixels.
        let batch = fill(None, [0.0, 0.0, 1e30, 8.0]);
        assert_eq!(batch.vertices.len(), 6 * 20);
        assert!(fill(None, [400.0, 0.0, 1e30, 8.0]).vertices.is_empty());
        assert!(
            fill(Some([0.0, 0.0, 0.0, 0.0]), [0.0, 0.0, 16.0, 16.0])
                .vertices
                .is_empty()
        );

        // A rectangle from -1e30 to 1e30 crossing the screen: its 16 px phase there is 0.
        let row = |n: usize| -> Vec<_> {
            (0..n)
                .map(|i| {
                    let x = 16.0 * i as f32;
                    ([x, 0.0, x + 16.0, 16.0], whole)
                })
                .collect()
        };
        let batch = fill(None, [-1e30, 0.0, 2e30, 16.0]);
        assert_eq!(quad_extents(&batch.vertices), row(20));

        // 2^29 + 16 rounds back to 2^29 in `f32`; posed back on screen in `f64` first, the
        // four tiles are whole.
        let mut batch = HudBatch::scratch([320, 240]);
        let mut pose = IDENTITY;
        pose_apply(&mut pose, PoseOp::Translate(-536_870_912.0, 0.0));
        on_canvas(&mut batch, &atlas, pose, |canvas| {
            canvas.paint_fluid(
                "block/water_still",
                u32::MAX,
                [536_870_912.0, 0.0, 64.0, 16.0],
            )
        });
        assert_eq!(quad_extents(&batch.vertices), row(4));
    }

    /// A minifying pose puts many tiles in each GUI pixel; the fill then emits one quad per GUI
    /// pixel of the clip, sampling the texel the tiling puts at its centre, not every tile: a
    /// 100,000 px square at a scale of 1 / 256 on a 320 x 240 canvas adds 76,800 quads, not
    /// 19,660,800. Each centre `(p + 0.5) * 256` is 8 px into a tile of the grid starting at 8.
    #[test]
    fn a_minified_fluid_rectangle_emits_at_most_one_quad_per_gui_pixel_of_the_clip() {
        let atlas = block_atlas(&["block/water_still"]);
        let mut batch = HudBatch::scratch([320, 240]);
        let mut pose = IDENTITY;
        pose_apply(&mut pose, PoseOp::Scale(1.0 / 256.0, 1.0 / 256.0));
        on_canvas(&mut batch, &atlas, pose, |canvas| {
            canvas.paint_fluid(
                "block/water_still",
                u32::MAX,
                [8.0, 8.0, 100_000.0, 100_000.0],
            )
        });
        let quads = quad_extents(&batch.vertices);
        assert_eq!(quads.len(), 320 * 240);
        assert_eq!(quads[0], ([0.0, 0.0, 1.0, 1.0], [0.25, 0.25, 0.25, 0.25]));
        assert_eq!(quads[321], ([1.0, 1.0, 2.0, 2.0], [0.25, 0.25, 0.25, 0.25]));
        assert!(quads.iter().all(|(_, uv)| *uv == [0.25, 0.25, 0.25, 0.25]));
    }

    /// A sprite the atlas lacks draws the missing sprite; an unregistered fluid and a non-finite
    /// rectangle draw nothing.
    #[test]
    fn a_missing_fluid_sprite_draws_missingno_and_unknown_fluids_are_refused() {
        let mut atlas = block_atlas(&[]);
        let missing = Rect {
            u0: 0.5,
            v0: 0.5,
            u1: 1.0,
            v1: 1.0,
            ..TILE
        };
        atlas
            .sprites
            .insert(sunrise_client::hud::MISSING_SPRITE.into(), missing);
        let mut batch = HudBatch::scratch([320, 240]);
        on_canvas(&mut batch, &atlas, IDENTITY, |canvas| {
            canvas.paint_fluid("a:block/absent", u32::MAX, [0.0, 0.0, 16.0, 16.0])
        });
        assert_eq!(
            quad_extents(&batch.vertices),
            [([0.0, 0.0, 16.0, 16.0], [0.5, 0.5, 1.0, 1.0])]
        );

        let stack = fluid_stack("minecraft:water");
        let unknown = mod_api::FluidStack {
            fluid: mod_api::FluidIdentity::of_name("missing:oil").unwrap(),
            amount: 1000,
            components: None,
        };
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |canvas| {
            assert_eq!(
                canvas.fluid(&unknown, [0.0, 0.0, 4.0, 4.0]),
                Err(GuiDrawError::UnknownFluid)
            );
            assert_eq!(
                canvas.fluid(&stack, [0.0, f32::NAN, 4.0, 4.0]),
                Err(GuiDrawError::NotFinite)
            );
            assert_eq!(
                canvas.fluid(&stack, [f32::MAX, f32::MAX, f32::MAX, f32::MAX]),
                Err(GuiDrawError::NotFinite)
            );
            Ok(())
        })
        .unwrap();
        assert!(batch.vertices.is_empty());
    }
}
