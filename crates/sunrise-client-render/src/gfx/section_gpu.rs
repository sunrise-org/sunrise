//! GPU representation and fading state of world sections.

use super::{FadeInputs, SectionKey, gpu_culling, packed, resident, section_fade, translucency};

use crate::mesher::{SectionMesh, Sides};
use glam::Vec3;
use section_fade::{section_instance, section_visibility};
use std::time::Instant;

pub(crate) struct SectionGpu {
    /// Arena page and first vertex of its solid quads, and after them its
    /// translucent quads unless those have an arena of their own; (0, 0),
    /// unused, when that leaves it no quads in `arena`.
    pub(super) page: u32,
    pub(super) start: u64,
    /// The page and first vertex of its translucent quads: in
    /// `translucent_arena` when there is one, else after the solid quads.
    /// Unused without translucent quads.
    pub(super) translucent_at: (u32, u64),
    pub(super) solid: u32,
    /// Slots its solid quads take in `arena`: four vertices each, or in the compact layout
    /// (`packed`, `mesher::packed`) a record each and the vertices of those without one.
    pub(super) solid_slots: u64,
    pub(super) packed: bool,
    /// Where the solid quads' buckets by facing side end (`mesher::Sides`).
    pub(super) sides: Sides,
    pub(super) translucent: u32,
    pub(super) sort: translucency::SortState,
    pub(super) sorted_indices: Option<(u32, u64)>,
    /// Its id for flat draws (`flat::Flat::add`), when they are on.
    pub(super) flat_id: Option<u32>,
    /// Its record's id in GPU culling's table (`gpu_culling::Records`), while
    /// that is on and it has solid quads.
    pub(super) gpu_id: Option<u32>,
    /// Its record is `FADING` (GPU culling on): the CPU draws its solid quads
    /// while its instance carries a fade (`drawn_faded`), and GPU culling's
    /// pass leaves it out.
    pub(super) gpu_fading: bool,
    /// Animated sprite layers present in this section.
    pub(super) animated_layers: Box<[u16]>,
}

/// Extracts the animated sprite layers present in a section mesh. Meshing
/// records the distinct layers, so uploading does not rescan every vertex.
pub(crate) fn collect_animated_layers(
    mesh: &SectionMesh,
    is_animated: impl Fn(u16) -> bool,
) -> Box<[u16]> {
    mesh.sprite_layers
        .iter()
        .copied()
        .filter(|&layer| is_animated(layer))
        .collect()
}

impl resident::ResidentSize for SectionGpu {
    fn census_bytes(&self) -> (u64, usize) {
        (
            self.vertex_bytes(),
            std::mem::size_of_val(&*self.animated_layers),
        )
    }
}

impl SectionGpu {
    /// Bytes in the vertex arena, before rounding up to the allocator's span.
    pub(super) fn vertex_bytes(&self) -> u64 {
        (self.solid_slots + u64::from(self.translucent) * 4)
            * std::mem::size_of::<super::TerrainVertex>() as u64
    }

    /// Its vertices in `arena`, and in `translucent_arena` when `split` (there
    /// is one): (page, first vertex, vertices) of each.
    pub(super) fn vertex_ranges(
        &self,
        split: bool,
    ) -> (Option<(u32, u64, u64)>, Option<(u32, u64, u64)>) {
        if !split {
            let vertices = self.solid_slots + u64::from(self.translucent) * 4;
            return (Some((self.page, self.start, vertices)), None);
        }
        let (page, start) = self.translucent_at;
        (
            (self.solid > 0).then_some((self.page, self.start, self.solid_slots)),
            (self.translucent > 0).then(|| (page, start, u64::from(self.translucent) * 4)),
        )
    }

    /// Whether GPU culling keeps a record of it at `key` while it is on (`gpu_id`):
    /// it has solid quads, in one of the level's `sections`, which the pass's
    /// tables span.
    pub(super) fn gpu_recorded(&self, key: SectionKey, sections: usize) -> bool {
        self.solid > 0 && (0..sections as i32).contains(&key.1)
    }

    /// The base vertex of its solid draws: quad `q`'s corners are `base + 4 * q` on (the
    /// quad indices). In a compact layout they name its record's slot past
    /// `PACKED_BASE`.
    pub(super) fn solid_base(&self) -> u32 {
        if !self.packed {
            return self.start as u32;
        }
        packed::PACKED_BASE + 4 * self.start as u32
    }

    /// GPU culling's record of it at `key`, its quads within `bounds`
    /// (`SectionMesh::bounds`) and in their buckets by facing side (`sides`),
    /// `FADING` while it fades (`gpu_fading`).
    pub(super) fn record(&self, key: SectionKey, bounds: Option<[Vec3; 2]>) -> gpu_culling::Record {
        let boxed = gpu_culling::rounded_box(bounds);
        gpu_culling::Record::new(
            key,
            self.page,
            u64::from(self.solid_base()),
            self.solid,
            boxed,
        )
        .facing(&self.sides)
        .fading(self.gpu_fading)
    }

    /// Whether GPU frames' walk lists it within the render distance
    /// (`Resident::walk`): its mark. Its translucent quads are the CPU's, or
    /// GPU culling can have no record of it, which holds whether GPU culling is
    /// on or not; or, while it is, its solid quads are the CPU's while it fades
    /// (`gpu_fading`).
    pub(super) fn listed(&self, key: SectionKey, sections: usize) -> bool {
        self.translucent > 0 || !self.gpu_recorded(key, sections) || self.gpu_fading
    }

    /// Whether GPU culling draws its solid quads in a GPU frame, `near`: its
    /// column within the render distance.
    pub(super) fn gpu_draws_solid(&self, near: bool) -> bool {
        near && self.gpu_id.is_some() && !self.gpu_fading
    }

    /// Whether the section is fading in at `f.now`.
    pub(super) fn drawn_faded(&self, key: SectionKey, f: FadeInputs<'_>) -> bool {
        let visibility = section_visibility(f.uploaded.get(&key), f.fade_ms, f.now);
        section_instance([0.0; 3], visibility)[3].to_bits()
            != section_instance([0.0; 3], 1.0)[3].to_bits()
    }

    /// The end of vanilla's fade-in, if a fade-in time is set.
    pub(super) fn fade_end(&self, key: SectionKey, f: FadeInputs<'_>) -> Option<Instant> {
        let vanilla = f.uploaded.get(&key).filter(|_| f.fade_ms > 0);
        vanilla.map(|&t| t + std::time::Duration::from_millis(f.fade_ms))
    }
}

/// Which of a mesh's ranges `move_off_page` moves: what it holds in the section
/// arena (`split`: its solid quads alone, `SectionGpu::vertex_ranges`), its
/// translucent quads in `translucent_arena`, or its sorted indices.
#[derive(Clone, Copy)]
pub(super) enum Part {
    Section { split: bool },
    Translucent,
    Indices,
}

impl Part {
    /// (page, first vertex or index, count), as the release paths free it.
    pub(super) fn of(self, s: &SectionGpu) -> Option<(u32, u64, u64)> {
        match self {
            Self::Section { split } => s.vertex_ranges(split).0,
            Self::Translucent => s.vertex_ranges(true).1,
            Self::Indices => s
                .sorted_indices
                .map(|(page, at)| (page, at, translucency::index_words(s.translucent))),
        }
    }

    /// Points the mesh where its range moved.
    pub(super) fn set(self, s: &mut SectionGpu, to: (u32, u64)) {
        match self {
            Self::Section { split } => {
                (s.page, s.start) = to;
                if !split {
                    s.translucent_at = (to.0, to.1 + s.solid_slots);
                }
            }
            Self::Translucent => s.translucent_at = to,
            Self::Indices => s.sorted_indices = Some(to),
        }
    }
}

/// Starts moving a page of a large arena far larger than its ranges in use
/// (`Arena::oversized`) into a new page of the size they get (`Arena::open_page`),
/// for `Renderer::shrink_translucent`: the page closes and `move_off_page` empties
/// it. None: no page to shrink. False, and nothing changes: no video memory for
/// the new page, or a range there without its mesh in `meshes`, which would keep
/// the page from ever emptying.
pub(super) fn begin_shrink<'a>(
    arena: &mut super::arena::Arena,
    device: &wgpu::Device,
    meshes: impl Iterator<Item = &'a mut SectionGpu>,
    part: Part,
) -> Option<bool> {
    let (page, capacity) = arena.oversized()?;
    let held: u64 = meshes
        .filter_map(|s| part.of(s).filter(|r| r.0 == page))
        .map(|(.., n)| super::arena::Arena::span(n))
        .sum();
    debug_assert_eq!(held, arena.used(page), "a range in use without its mesh");
    if held != arena.used(page) || !arena.open_page(device, capacity) {
        return Some(false);
    }
    arena.close(page);
    Some(true)
}

/// GPU culling's flag (`SectionGpu::gpu_fading`) for its recorded section `s`
/// at `key`, decided at `f.now`: whether it draws faded (`drawn_faded`), and if
/// time ends that, its end waited for in `fades` (`fade_end`).
pub(super) fn decide_fade(
    s: &SectionGpu,
    key: SectionKey,
    fades: &mut section_fade::Fades,
    f: FadeInputs<'_>,
) -> bool {
    let fading = s.drawn_faded(key, f);
    if let Some(end) = s.fade_end(key, f).filter(|_| fading) {
        fades.push(end, key);
    }
    fading
}

/// GPU culling switched off (`Renderer::set_gpu_culling`): no section keeps a
/// record or a flag, and those it flagged get back the walk marks they have
/// while it is off (`SectionGpu::listed`). `rows`: the level's sections.
pub(super) fn forget_records(sections: &mut resident::Resident<SectionGpu>, rows: usize) {
    let mut marks = Vec::new();
    for (key, s, _) in sections.iter_mut() {
        s.gpu_id = None;
        if std::mem::take(&mut s.gpu_fading) {
            marks.push((key, s.listed(key, rows)));
        }
    }
    for (key, marked) in marks {
        sections.set_marked(key, marked);
    }
}

/// GPU culling's fading sections brought up to `f.now`, before a frame's gate
/// and `prepare`: every recorded section anew when the flags were decided with
/// another chunk fade-in time (GPU culling just switched on, or Video Settings
/// changed it), else those whose fade was due to end, handed back to the GPU if
/// whole by then. A section's record (`FADING`) and its walk mark follow its
/// `gpu_fading`. `rows`: the level's sections.
pub(super) fn mark_fades(
    sections: &mut resident::Resident<SectionGpu>,
    records: &mut gpu_culling::Records,
    fades: &mut section_fade::Fades,
    rows: usize,
    f: FadeInputs<'_>,
) {
    let mut marks = Vec::new();
    if fades.fade_ms != Some(f.fade_ms) {
        fades.clear();
        fades.fade_ms = Some(f.fade_ms);
        for (key, s, _) in sections.iter_mut() {
            let Some(id) = s.gpu_id else {
                continue;
            };
            let fading = decide_fade(s, key, fades, f);
            if fading != s.gpu_fading {
                s.gpu_fading = fading;
                records.set_fading(id, fading);
                marks.push((key, s.listed(key, rows)));
            }
        }
    } else {
        while let Some(key) = fades.pop_due(f.now) {
            // Gone since, or no longer flagged.
            let Some(s) = sections.find_mut(key).filter(|s| s.gpu_fading) else {
                continue;
            };
            // Still fading: meshed again since, with its own end waited for
            // (`decide_fade`), or leaving or held, with none.
            if s.drawn_faded(key, f) {
                continue;
            }
            s.gpu_fading = false;
            records.set_fading(s.gpu_id.expect("flagged sections have records"), false);
            marks.push((key, s.listed(key, rows)));
        }
    }
    for (key, marked) in marks {
        sections.set_marked(key, marked);
    }
}
