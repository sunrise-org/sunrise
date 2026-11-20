//! Derived ephemeral source residency, using ordinary forced streaming/ticking.
use super::*;

/// Charge the real existing forced-streaming terrain ring, not just selected chunks.
/// The source has one binding; memberships are sparse and never expand to their outer AABB.
pub(super) fn checked_retention(
    cells: &[[i32; 3]],
    limits: LiveWorldLimits,
) -> Result<Vec<(i32, i32)>, Error> {
    retention_footprint(cells, limits).map(|(members, _)| members)
}
fn retention_footprint(
    cells: &[[i32; 3]],
    limits: LiveWorldLimits,
) -> Result<(Vec<(i32, i32)>, u32), Error> {
    let members: BTreeSet<_> = cells.iter().map(|p| (p[0] >> 4, p[2] >> 4)).collect();
    let mut cover = BTreeSet::new();
    let ring = sunrise_client::streaming::FORCED_TERRAIN_RING;
    for &(x, z) in &members {
        for dx in -ring..=ring {
            for dz in -ring..=ring {
                cover.insert((x + dx, z + dz));
                if cover.len() > limits.chunks as usize {
                    return Err(Error::Budget);
                }
            }
        }
    }
    Ok((members.into_iter().collect(), cover.len() as u32))
}
impl Game<'_> {
    pub(in crate::game) fn live_world_lifecycle_current(&self, level: LevelRef) -> bool {
        !self.live_world_session_failed()
            && self.level_handle_current(level)
            && !self.levels.runtime.is_closing(level.dimension_id())
    }

    pub(super) fn refresh_live_world_retention(&mut self, source: LevelRef) {
        let chunks = self
            .live_worlds
            .borrow()
            .entries
            .values()
            .filter(|entry| entry.handle.level() == source)
            .flat_map(|entry| entry.retained.iter().copied())
            .collect();
        if self.dim.key() == source.dimension() && self.level_handle_current(source) {
            self.streaming.set_runtime_forced(chunks);
        } else if let Some(streaming) = self.levels.parked_streaming_of_mut(source.dimension_id()) {
            streaming.set_runtime_forced(chunks);
        }
    }

    /// Internal lifecycle cleanup never requires a mod context or a still-current source handle.
    pub(in crate::game) fn live_world_level_closed(&mut self, dim: super::super::Dim) {
        let sources: Vec<_> = self
            .live_worlds
            .borrow()
            .entries
            .values()
            .filter(|entry| {
                entry.handle.level().dimension() == dim.key()
                    || entry.destination.dimension() == dim.key()
            })
            .map(|entry| entry.handle.level())
            .collect();
        self.live_worlds.get_mut().entries.retain(|_, entry| {
            entry.handle.level().dimension() != dim.key()
                && entry.destination.dimension() != dim.key()
        });
        for source in sources {
            self.refresh_live_world_retention(source);
        }
        self.live_world_durability
            .members
            .retain(|level, _| level.dimension() != dim.key());
        self.live_world_durability
            .dirty
            .retain(|level| level.dimension() != dim.key());
        self.live_world_durability
            .retired
            .retain(|level, _| level.dimension() != dim.key());
    }
}

pub(super) fn selection_footprint(
    source: LevelRef,
    cells: &[[i32; 3]],
) -> Result<mod_api::LiveWorldFootprint, Error> {
    let limits = LiveWorldLimits {
        min: [-29_999_999; 3],
        max: [29_999_999; 3],
        cells: 65536,
        chunks: 4096,
        ..Default::default()
    };
    let cells = checked_cells(cells, limits)?;
    let (members, columns) = retention_footprint(&cells, limits)?;
    let areas = sync::stream_areas(source, &cells)?;
    Ok(mod_api::LiveWorldFootprint::new(
        cells.len() as u32,
        members.len() as u32,
        columns,
        areas.len() as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_charges_sparse_real_streaming_rings_without_an_outer_box() {
        let mut limits = LiveWorldLimits {
            chunks: 162,
            ..Default::default()
        };
        assert_eq!(
            checked_retention(&[[0, 64, 0], [2047, 64, 0]], limits).unwrap(),
            [(0, 0), (127, 0)]
        );
        limits.chunks = 161;
        assert_eq!(
            checked_retention(&[[0, 64, 0], [2047, 64, 0]], limits),
            Err(Error::Budget)
        );
        limits.chunks = 90;
        assert_eq!(
            checked_retention(&[[0, 64, 0], [17, 64, 0]], limits).unwrap(),
            [(0, 0), (1, 0)]
        );
        limits.chunks = 89;
        assert_eq!(
            checked_retention(&[[0, 64, 0], [17, 64, 0]], limits),
            Err(Error::Budget)
        );
    }
}
