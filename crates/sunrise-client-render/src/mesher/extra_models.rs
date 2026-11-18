//! The models a client mod draws as part of a block (`Chunk::extra_models`,
//! `ClientRenderClientHost::set_block_extra_models`): drawn after the section's own geometry and
//! held to the section's quad and byte budget (`mod_geometry::MAX_QUADS`, `MAX_SECTION_BYTES`),
//! so no list of models can grow a section's mesh past what its upload holds.
use super::mod_geometry::{Budget, QUAD_BYTES};
use super::{CellFaces, MAX_OVERLAYS, Mesher, Padded, SectionMesh, TerrainSource, Tints};
use std::sync::Arc;

/// The most quads one model face becomes: its `ctm_compact` pieces (up to 3 by 3), and as many
/// again for each connected-texture overlay drawn over it.
pub(super) const FACE_QUADS: usize = 9 * (1 + MAX_OVERLAYS);
/// The most mesh bytes one quad takes: [`QUAD_BYTES`] and its shader-pack source.
pub(super) const QUAD_MAX_BYTES: usize = QUAD_BYTES + size_of::<TerrainSource>();

/// Draws `cells`' extra models into `out`, after the block geometry of the whole section, in
/// cell order and then list order. A face is drawn only while the section's budget still holds
/// [`FACE_QUADS`] more quads of [`QUAD_MAX_BYTES`], whatever it expands to; at the first face
/// that could pass it, it and every extra model after it are dropped. The block geometry is
/// always kept, and the same section and lists always keep the same models.
pub(super) fn add(
    mesher: &Mesher<'_>,
    pad: &Padded,
    tints: &mut Tints<'_>,
    out: &mut SectionMesh,
    cells: &[(CellFaces<'_>, &Arc<[mod_api::ModelRef]>)],
) {
    for (at, models) in cells {
        let start = [out.solid.len(), out.translucent.len()];
        let mut full = false;
        'models: for &model in models.iter() {
            // A handle a resource reload made stale draws nothing.
            let Ok(model) = mesher.render.resolve_model(model) else {
                continue;
            };
            for face in &model.faces {
                if !fits(out, start, mesher.shader_pack) {
                    full = true;
                    break 'models;
                }
                mesher.model_face(pad, tints, out, at, face);
            }
        }
        if mesher.shader_pack {
            let source = TerrainSource::new(at.id, at.pos, at.state.emission, false);
            out.record_source(start, source);
        }
        if full {
            return;
        }
    }
}

/// Whether the section `out` has room for one more face at its largest. With a shader pack, the
/// quads drawn since `start` take their sources once the cell is done: counted now.
fn fits(out: &SectionMesh, start: [usize; 2], shader_pack: bool) -> bool {
    let mut left = Budget::left_after(out);
    if shader_pack {
        let pending = (out.solid.len() - start[0] + out.translucent.len() - start[1]) / 4;
        left.bytes = left
            .bytes
            .saturating_sub(pending * size_of::<TerrainSource>());
    }
    left.quads >= FACE_QUADS && left.bytes >= FACE_QUADS * QUAD_MAX_BYTES
}
