//! Better Grass and Better Snow while a cell is meshed (`crate::better_grass` has what they
//! draw; `sunrise_optifine::bettergrass` says when). Better Grass swaps the faces a grass-like
//! block has toward a side for the full faces of another texture; Better Snow draws a snow
//! layer under a plant with snow beside it.

use super::{
    CellFaces, Chunk, FaceCell, GroupCells, Mesher, Padded, Pass, SectionMesh, Tints, step_index,
    visibility_plan,
};
use crate::{
    assets::RenderAssets,
    better_grass::{SNOW_BLOCK, SNOW_LAYER, TAKES_SNOW, UPSIDE_DOWN, side_index},
    graphics::BetterGrassMode,
    models::{StateModels, catalog::BakedFace},
};
use sunrise_optifine::bettergrass::{Probe, Surface};
use sunrise_world::registry::BlockStateId;

/// The blocks beside a side, as `[dx, dz]`, by the side's index (north, south, west, east).
const BESIDE: [[i32; 2]; 4] = [[0, -1], [0, 1], [-1, 0], [1, 0]];

/// The faces drawn over the sides of one block instead of its own.
pub(super) struct Replace<'r> {
    sides: [Option<&'r [BakedFace]>; 4],
    /// Only a face of this sprite layer is replaced; every face toward the side when `None`.
    layer: Option<u32>,
}

impl<'r> Replace<'r> {
    /// What to draw in place of `face`, if it is one this replaces: the side's faces for the
    /// first of them (`done` has the sides already drawn), nothing for the others.
    pub(super) fn replacing(
        &self,
        face: &BakedFace,
        done: &mut [bool; 4],
    ) -> Option<&'r [BakedFace]> {
        let side = face.quad.cull.and_then(side_index)?;
        let faces = self.sides[side]?;
        if self.layer.is_some_and(|layer| face.layer != layer) {
            return None;
        }
        let faces: &'r [BakedFace] = if std::mem::replace(&mut done[side], true) {
            &[]
        } else {
            faces
        };
        Some(faces)
    }
}

/// The faces Better Grass draws over the sides of the block `id` at `cell` of `pad`; `None`
/// when it leaves all of them as they are.
pub(super) fn sides<'r>(
    render: &'r RenderAssets,
    mode: BetterGrassMode,
    pad: &Padded,
    cell: usize,
    id: u32,
) -> Option<Replace<'r>> {
    if mode == BetterGrassMode::Off {
        return None;
    }
    let assets = &render.better;
    let surface = assets.kind(id).surface?;
    let fancy = mode == BetterGrassMode::Fancy;
    let kind_at =
        |step: [i32; 3]| assets.kind(pad.blocks[(cell as isize + step_index(step)) as usize]);
    let mut replace = Replace {
        sides: [None; 4],
        layer: None,
    };
    let mut any = false;
    for (side, [dx, dz]) in BESIDE.into_iter().enumerate() {
        let probe = |probe: Probe| match probe {
            Probe::SnowAbove => kind_at([0, 1, 0]).flags & (SNOW_LAYER | SNOW_BLOCK) != 0,
            Probe::SnowBeside => kind_at([dx, 0, dz]).flags & SNOW_LAYER != 0,
            Probe::SameBelowBeside => kind_at([dx, -1, dz]).surface == Some(surface),
            Probe::PathAbove => kind_at([0, 1, 0]).surface == Some(Surface::DirtPath),
            Probe::PathBeside => kind_at([dx, 0, dz]).surface == Some(Surface::DirtPath),
        };
        let faces: &[BakedFace] = if surface == Surface::DirtPath {
            match assets.path(side) {
                Some((layer, faces)) if assets.rules.path_side(fancy, probe) => {
                    replace.layer = Some(layer);
                    faces
                }
                _ => &[],
            }
        } else {
            match assets.rules.cube(fancy, surface, probe) {
                Some(cube) => assets.cube(cube, side),
                None => &[],
            }
        };
        if !faces.is_empty() {
            replace.sides[side] = Some(faces);
            any = true;
        }
    }
    any.then_some(replace)
}

impl Mesher<'_> {
    /// Better Snow: a snow layer is drawn at the cell `at` when its block is a plant, a fence or
    /// the like, a snow layer is beside it, and what is below it can hold snow beside it too.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn snow_under<M: visibility_plan::CellMasks>(
        &mut self,
        catalog: &StateModels,
        slots: &mut Vec<u32>,
        region: &[[Option<&Chunk>; 3]; 3],
        pad: &Padded,
        groups: Option<&GroupCells>,
        tints: &mut Tints,
        out: &mut SectionMesh,
        at: &CellFaces,
        base_y: i32,
        masks: &M,
    ) {
        let render = self.render;
        let assets = &render.better;
        let FaceCell {
            pos,
            cell,
            id,
            world,
            ..
        } = at.sample;
        if assets.kind(id).flags & TAKES_SNOW == 0 {
            return;
        }
        let Some(snow) = assets.snow_state() else {
            return;
        };
        let block_at = |step: [i32; 3]| pad.blocks[(cell as isize + step_index(step)) as usize];
        let snow_beside = [[0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]]
            .into_iter()
            .any(|step| assets.kind(block_at(step)).flags & SNOW_LAYER != 0);
        if !snow_beside {
            return;
        }
        // What the plant stands on: a full opaque cube, or a stair or slab turned upside down.
        let below = block_at([0, -1, 0]);
        if !(self.tables.info[below as usize].solid || assets.kind(below).flags & UPSIDE_DOWN != 0)
        {
            return;
        }
        if catalog.slots_at(BlockStateId(snow), world, slots).is_err() {
            return;
        }
        let Some(first) = slots.first().and_then(|&slot| catalog.model_by_slot(slot)) else {
            return;
        };
        let visible = self.visible_faces_as(snow, region, pad, base_y, pos, masks);
        let state = &self.tables.info[snow as usize];
        let ao = self.smooth_lighting && state.emission == 0 && first.ambient_occlusion;
        // The layer lies where a block of its own would, with none of the plant's offset.
        let at = CellFaces {
            sample: FaceCell {
                id: snow,
                visible,
                ..at.sample
            },
            origin: [pos[0] as f32, pos[1] as f32, pos[2] as f32],
            ao,
        };
        let pass = Pass::Own { force_solid: false };
        for &slot in slots.iter() {
            let Some(model) = catalog.model_by_slot(slot) else {
                continue;
            };
            for face in &model.faces {
                self.model_face(pad, groups, tints, out, &at, state, face, pass);
            }
        }
    }
}
