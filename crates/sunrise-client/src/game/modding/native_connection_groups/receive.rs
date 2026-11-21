//! Give one completed bundle's group data to the packet that owns it. Nothing
//! here waits for another frame or survives the packet's adoption.
use sunrise_client::{
    chunk::{Chunk, LevelShape},
    fast_hash::FxHashMap,
    world::World,
};
use sunrise_protocol::{
    Error, Result, connection_groups::Section, packets::play::Clientbound as C,
};

#[derive(Clone, Copy)]
pub(in crate::game) struct ReceivedGroup(pub Option<u64>);

impl ReceivedGroup {
    pub(in crate::game) fn apply(self, world: &mut World, [x, y, z]: [i32; 3]) {
        world.set_connection_group(x, y, z, self.0, false);
    }
}

pub(in crate::game) type GroupUpdates = FxHashMap<[i32; 3], ReceivedGroup>;

/// Install the decoded map in the new column before the column, its remesh
/// diff or any of its load hooks can become visible.
pub(in crate::game) fn column_groups(
    chunk: &mut Chunk,
    shape: LevelShape,
    sections: &[Section],
) -> Result<()> {
    let mut seen = sunrise_client::fast_hash::FxHashSet::default();
    for section in sections {
        let [x, y, z] = section.pos;
        if (x, z) != (chunk.x, chunk.z) || !shape.contains_y(y << 4) || !seen.insert(y) {
            return Err(Error::new("connection groups: unexpected column section"));
        }
        for entry in &section.entries {
            let Some(group) = entry.group else {
                return Err(Error::new("connection groups: clear in full column"));
            };
            let [x, y, z] = entry.cell.at(section.pos);
            chunk.set_connection_group(x & 15, y, z & 15, Some(group));
        }
    }
    Ok(())
}

pub(in crate::game) fn block_groups(
    world: &World,
    packet: &C,
    sections: &[Section],
) -> Result<GroupUpdates> {
    let [section] = sections else {
        return Err(Error::new(
            "connection groups: block update needs one group section",
        ));
    };
    let mut updates = GroupUpdates::default();
    match packet {
        C::BlockUpdate(block) => {
            let [entry] = section.entries.as_slice() else {
                return Err(Error::new(
                    "connection groups: single block needs one group cell",
                ));
            };
            if entry.cell.at(section.pos) != block.pos {
                return Err(Error::new(
                    "connection groups: group cell not in block update",
                ));
            }
            updates.insert(block.pos, ReceivedGroup(entry.group));
        }
        C::SectionBlocksUpdate(blocks) if blocks.section == section.pos => {
            let mut positions = [0_u64; 4096 / 64];
            for &change in &blocks.changes {
                let index = (change & 0xFFF) as usize;
                positions[index / 64] |= 1 << (index % 64);
            }
            for entry in &section.entries {
                let index = entry.cell.index();
                if positions[index / 64] & (1 << (index % 64)) == 0 {
                    return Err(Error::new(
                        "connection groups: group cell not in section update",
                    ));
                }
                updates.insert(entry.cell.at(section.pos), ReceivedGroup(entry.group));
            }
        }
        _ => {
            return Err(Error::new(
                "connection groups: companion is not its block update",
            ));
        }
    }
    if updates
        .keys()
        .any(|&[x, y, z]| !world.is_loaded_at(x, y, z))
    {
        return Err(Error::new("connection groups: group cell is not loaded"));
    }
    Ok(updates)
}
