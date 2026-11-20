//! Projection ownership/poses over the negotiated engine channel. The actual local
//! blocks and block entities are the existing viewed-level client world, never a mod map.
use super::*;
use mod_api::{PayloadBytes, PayloadId, PayloadSendError, WorldEpoch};
use sunrise_client::{
    lan::viewed_levels::areas::{ChunkArea, ChunkAreaMode},
    raw_payloads::Endpoint,
};
use sunrise_protocol::live_world_sync::{self as wire, Message};

#[derive(Default)]
pub(super) struct Sync {
    sent: BTreeMap<(u128, u64), Sent>,
    pub(super) replicas: BTreeMap<u64, Replica>,
    retired: BTreeSet<u64>,
    epoch: Option<WorldEpoch>,
}
#[derive(Clone, Copy, PartialEq)]
struct Sent {
    source_revision: u64,
    state_hash: [u8; 32],
    membership: u64,
    sample: LiveWorldSample,
}
pub(super) struct Replica {
    source: String,
    destination: String,
    collider: u32,
    boxes: u32,
    pub(super) source_revision: u64,
    state_hash: [u8; 32],
    cells: Vec<[i32; 3]>,
    chunks: Vec<(i32, i32)>,
    sample: LiveWorldSample,
    membership: u64,
    frame: u64,
    epoch: WorldEpoch,
    entry: Option<Entry>,
    observed: Option<(LevelRef, u64, [u8; 32])>,
}

impl Sync {
    pub(super) fn clear(&mut self) {
        self.sent.clear();
        self.replicas.clear();
        self.retired.clear();
        self.epoch = None;
    }
}
fn wire_pose(p: LiveWorldPose) -> wire::Pose {
    wire::Pose {
        translation: p.translation,
        pivot: p.pivot,
        rotation: p.rotation,
    }
}
fn wire_sample(
    s: LiveWorldSample,
    frame: u64,
    source_revision: u64,
    state_hash: [u8; 32],
) -> wire::Sample {
    wire::Sample {
        frame,
        source_revision,
        state_hash,
        previous: wire_pose(s.previous),
        current: wire_pose(s.current),
        revision: s.revision,
    }
}
fn api_pose(p: wire::Pose) -> LiveWorldPose {
    LiveWorldPose {
        translation: p.translation,
        pivot: p.pivot,
        rotation: p.rotation,
    }
}
fn api_sample(s: wire::Sample) -> LiveWorldSample {
    LiveWorldSample::new(api_pose(s.previous), api_pose(s.current), s.revision)
}

/// Prepared once per membership revision, never by a per-guest pose flush.
pub(super) fn stream_areas(source: LevelRef, cells: &[[i32; 3]]) -> Result<Vec<ChunkArea>, Error> {
    let chunks: BTreeSet<_> = cells.iter().map(|p| (p[0] >> 4, p[2] >> 4)).collect();
    let area = |center, radius| ChunkArea {
        level: source.dimension_id().into(),
        center,
        radius,
        mode: ChunkAreaMode::Loaded,
    };
    if chunks.len() <= 32 {
        return Ok(chunks.into_iter().map(|p| area(p, 0)).collect());
    }
    let mut tiles: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
    for chunk in chunks {
        tiles
            .entry((chunk.0.div_euclid(32), chunk.1.div_euclid(32)))
            .or_default()
            .push(chunk);
    }
    if tiles.len() > 32 {
        return Err(Error::Budget);
    }
    Ok(tiles
        .into_values()
        .map(|chunks| {
            let min = [
                chunks.iter().map(|p| p.0).min().unwrap(),
                chunks.iter().map(|p| p.1).min().unwrap(),
            ];
            let max = [
                chunks.iter().map(|p| p.0).max().unwrap(),
                chunks.iter().map(|p| p.1).max().unwrap(),
            ];
            let radius = ((max[0] - min[0]).max(max[1] - min[1]) + 1) / 2;
            area((min[0] + radius, min[1] + radius), radius as u32)
        })
        .collect())
}

impl Game<'_> {
    /// Admit the precise cached sparse stream selection before any binding/membership mutation.
    pub(super) fn check_projection_views(
        &self,
        destination: LevelRef,
        new_areas: &[ChunkArea],
    ) -> Result<(), Error> {
        self.check_projection_views_replacing(destination, new_areas, None)
    }
    pub(super) fn check_projection_views_replacing(
        &self,
        destination: LevelRef,
        new_areas: &[ChunkArea],
        replacing: Option<LiveWorldRef>,
    ) -> Result<(), Error> {
        if new_areas.len() > 32 {
            return Err(Error::Budget);
        }
        let table = self.live_worlds.borrow();
        let mut requested: Vec<_> = table
            .entries
            .values()
            .filter(|e| e.destination == destination && Some(e.handle) != replacing)
            .flat_map(|e| e.stream_areas.iter().cloned())
            .collect();
        requested.extend_from_slice(new_areas);
        if requested.len() > 32 {
            return Err(Error::Budget);
        }
        let Some(host) = self.lan.as_ref() else {
            return Ok(());
        };
        for guest in host
            .hub
            .guests
            .iter()
            .filter(|g| g.participates_online() && g.dimension == destination.dimension_id())
        {
            if !sunrise_client::raw_payloads::negotiated(
                guest.sender.payload_transport(),
                wire::CHANNEL,
            ) || guest.sender.viewed_levels.revision
                < sunrise_protocol::viewed_levels::REVISION_4
            {
                return Err(Error::Unsupported);
            }
            host.hub
                .can_set_viewed_areas(guest.uuid, "sunrise:live_worlds", &requested)
                .map_err(|_| Error::Budget)?;
        }
        Ok(())
    }

    /// Snapshot before absolute pose samples; source chunks continue on the ordinary view protocol.
    pub(in crate::game) fn flush_live_world_sync(&mut self) {
        if !self.is_logical_server() {
            return;
        }
        if !sunrise_client::join_seq::visibility::passes_settled() {
            return;
        }
        if self.live_worlds.borrow().entries.is_empty()
            && self.live_worlds.borrow().sync.sent.is_empty()
        {
            return;
        }
        let guests: Vec<_> = self
            .lan
            .as_ref()
            .map(|host| {
                host.hub
                    .guests
                    .iter()
                    .filter(|g| g.participates_online())
                    .map(|g| {
                        (
                            g.uuid,
                            g.dimension.clone(),
                            g.sender.clone(),
                            g.sender.viewed_levels,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let online: BTreeSet<_> = guests.iter().map(|g| g.0).collect();
        self.live_worlds
            .borrow_mut()
            .sync
            .sent
            .retain(|(uuid, _), _| online.contains(uuid));
        for (uuid, dimension, sender, viewed) in guests {
            let (wanted, requested, messages) = {
                let table = self.live_worlds.borrow();
                let mut wanted = BTreeSet::new();
                let mut requested = Vec::new();
                let mut messages = Vec::new();
                let mut refused = false;
                for entry in table.entries.values().filter(|e| {
                    e.destination.dimension_id() == dimension
                        && self.level_handle_current(e.handle.level())
                        && self.level_handle_current(e.destination)
                }) {
                    if !sunrise_client::raw_payloads::negotiated(
                        sender.payload_transport(),
                        wire::CHANNEL,
                    ) || viewed.revision < sunrise_protocol::viewed_levels::REVISION_4
                    {
                        refused = true;
                        break;
                    }
                    requested.extend(entry.stream_areas.iter().cloned());
                    let id = entry.handle.generation();
                    wanted.insert(id);
                    let Ok(geometry) = self.projection_geometry(entry) else {
                        refused = true;
                        break;
                    };
                    let source_revision = geometry.source_revision;
                    let state_hash = geometry.state_hash;
                    let stamp = Sent {
                        source_revision,
                        state_hash,
                        membership: entry.membership,
                        sample: entry.sample,
                    };
                    let message = match table.sync.sent.get(&(uuid, id)) {
                        None => Some(Message::Snapshot {
                            id,
                            collider: entry.collider,
                            boxes: entry.limits.boxes,
                            source: entry.handle.level().dimension_id().into(),
                            destination: entry.destination.dimension_id().into(),
                            cells: entry.cells.clone(),
                            membership: entry.membership,
                            sample: wire_sample(
                                entry.sample,
                                table.sequence,
                                source_revision,
                                state_hash,
                            ),
                        }),
                        Some(old) if old.membership != entry.membership => {
                            Some(Message::Snapshot {
                                id,
                                collider: entry.collider,
                                boxes: entry.limits.boxes,
                                source: entry.handle.level().dimension_id().into(),
                                destination: entry.destination.dimension_id().into(),
                                cells: entry.cells.clone(),
                                membership: entry.membership,
                                sample: wire_sample(
                                    entry.sample,
                                    table.sequence,
                                    source_revision,
                                    state_hash,
                                ),
                            })
                        }
                        Some(old) if *old != stamp => Some(Message::Pose {
                            id,
                            sample: wire_sample(
                                entry.sample,
                                table.sequence,
                                source_revision,
                                state_hash,
                            ),
                        }),
                        _ => None,
                    };
                    if let Some(message) = message {
                        messages.push((id, stamp, message));
                    }
                }
                if refused {
                    sender.close();
                    continue;
                }
                for &(player, id) in table.sync.sent.keys() {
                    if player == uuid && !wanted.contains(&id) {
                        messages.push((
                            id,
                            Sent {
                                source_revision: 0,
                                state_hash: [0; 32],
                                membership: 0,
                                sample: LiveWorldSample::new(
                                    LiveWorldPose::at([0.0; 3]),
                                    LiveWorldPose::at([0.0; 3]),
                                    0,
                                ),
                            },
                            Message::Clear { id },
                        ));
                    }
                }
                (wanted, requested, messages)
            };
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.viewed_areas(uuid, "sunrise:live_worlds") != requested
                && host
                    .hub
                    .set_viewed_areas(uuid, "sunrise:live_worlds", requested)
                    .is_err()
            {
                sender.close();
                continue;
            }
            for (id, stamp, message) in messages {
                let Ok(bytes) = wire::encode(&message) else {
                    sender.close();
                    break;
                };
                match self.queue_current_payload(
                    Endpoint::server(sender.clone()),
                    &PayloadId(wire::CHANNEL.into()),
                    PayloadBytes(bytes),
                ) {
                    Ok(_) => {
                        let mut table = self.live_worlds.borrow_mut();
                        if wanted.contains(&id) {
                            table.sync.sent.insert((uuid, id), stamp);
                        } else {
                            table.sync.sent.remove(&(uuid, id));
                        }
                    }
                    Err(PayloadSendError::Backpressure) => {
                        sender.close();
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
    }

    pub(in crate::game) fn remote_live_world_sync(
        &mut self,
        data: &[u8],
    ) -> sunrise_protocol::Result<()> {
        let message = wire::decode(data)?;
        let epoch = self
            .native_lifecycle
            .epoch()
            .ok_or_else(|| sunrise_protocol::Error::new("live world: no client level"))?;
        let mut table = self.live_worlds.borrow_mut();
        let sync = &mut table.sync;
        if sync.epoch != Some(epoch) {
            sync.clear();
            sync.epoch = Some(epoch);
        }
        match message {
            Message::Snapshot {
                id,
                collider,
                boxes,
                source,
                destination,
                cells,
                sample,
                membership,
            } => {
                if sync.retired.contains(&id) {
                    return Ok(());
                }
                if sync.replicas.len() >= 64 && !sync.replicas.contains_key(&id) {
                    return Err(sunrise_protocol::Error::new("live world: replica budget"));
                }
                let incoming = api_sample(sample);
                let total: usize = sync
                    .replicas
                    .iter()
                    .filter(|(key, _)| **key != id)
                    .map(|(_, r)| r.cells.len())
                    .sum();
                if total.checked_add(cells.len()).is_none_or(|n| n > 262144) {
                    return Err(sunrise_protocol::Error::new(
                        "live world: aggregate cell budget",
                    ));
                }
                let reserved: u64 = sync
                    .replicas
                    .iter()
                    .filter(|(key, _)| **key != id)
                    .map(|(_, r)| u64::from(r.boxes))
                    .sum();
                if reserved + u64::from(boxes) > 1048576 {
                    return Err(sunrise_protocol::Error::new(
                        "live world: aggregate shape budget",
                    ));
                }
                if let Some(old) = sync.replicas.get(&id) {
                    if membership < old.membership
                        || sample.frame < old.frame
                        || sample.revision < old.sample.revision
                    {
                        return Ok(());
                    }
                    if sample.frame == old.frame {
                        if membership == old.membership
                            && cells == old.cells
                            && incoming == old.sample
                            && source == old.source
                            && destination == old.destination
                            && collider == old.collider
                            && boxes == old.boxes
                            && sample.source_revision == old.source_revision
                            && sample.state_hash == old.state_hash
                        {
                            return Ok(());
                        }
                        return Err(sunrise_protocol::Error::new(
                            "live world: divergent equal-frame snapshot",
                        ));
                    }
                    if sample.revision == old.sample.revision
                        && incoming.current != old.sample.current
                    {
                        return Err(sunrise_protocol::Error::new(
                            "live world: divergent equal-pose snapshot",
                        ));
                    }
                    if old.source != source
                        || old.destination != destination
                        || old.collider != collider
                    {
                        return Err(sunrise_protocol::Error::new("live world: owner changed"));
                    }
                }
                let old_entry = sync.replicas.remove(&id).and_then(|r| r.entry);
                let chunks = cells
                    .iter()
                    .map(|p| (p[0] >> 4, p[2] >> 4))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                sync.replicas.insert(
                    id,
                    Replica {
                        source,
                        destination,
                        collider,
                        boxes,
                        source_revision: sample.source_revision,
                        state_hash: sample.state_hash,
                        cells,
                        chunks,
                        sample: incoming,
                        membership,
                        frame: sample.frame,
                        epoch,
                        entry: old_entry,
                        observed: None,
                    },
                );
            }
            Message::Pose { id, sample } => {
                if let Some(replica) = sync.replicas.get_mut(&id) {
                    let incoming = api_sample(sample);
                    if sample.frame < replica.frame || sample.revision < replica.sample.revision {
                        return Ok(());
                    }
                    if sample.frame == replica.frame {
                        if incoming == replica.sample
                            && sample.source_revision == replica.source_revision
                            && sample.state_hash == replica.state_hash
                        {
                            return Ok(());
                        }
                        return Err(sunrise_protocol::Error::new(
                            "live world: divergent equal-frame pose",
                        ));
                    }
                    if sample.revision == replica.sample.revision
                        && incoming.current != replica.sample.current
                    {
                        return Err(sunrise_protocol::Error::new(
                            "live world: divergent equal-pose sample",
                        ));
                    }
                    replica.source_revision = sample.source_revision;
                    replica.state_hash = sample.state_hash;
                    replica.frame = sample.frame;
                    replica.sample = incoming;
                }
            }
            Message::Clear { id } => {
                sync.replicas.remove(&id);
                if sync.retired.len() >= 65536 {
                    return Err(sunrise_protocol::Error::new(
                        "live world: retirement budget",
                    ));
                }
                sync.retired.insert(id);
            }
        }
        Ok(())
    }

    pub(super) fn replica_entry<'a>(&self, id: u64, replica: &'a mut Replica) -> Option<&'a Entry> {
        if self.native_lifecycle.epoch() != Some(replica.epoch)
            || self.dim.id() != replica.destination
        {
            return None;
        }
        let source = self.level(&replica.source)?;
        let destination = self.level(&replica.destination)?;
        let world = self.native_level_view_world(source)?;
        let loaded = replica.chunks.iter().all(|p| world.chunks.contains_key(p));
        if loaded
            && replica
                .observed
                .as_ref()
                .is_none_or(|(level, revision, _)| *level != source || *revision != world.revision)
        {
            replica.observed = Some((source, world.revision, state_hash(world, &replica.cells)));
        }
        if !loaded
            || replica
                .observed
                .as_ref()
                .is_none_or(|(_, _, hash)| *hash != replica.state_hash)
        {
            if let Some(entry) = replica.entry.as_mut() {
                entry.frozen = true;
            }
            return replica.entry.as_ref();
        }
        if replica
            .entry
            .as_ref()
            .is_none_or(|e| e.handle.level() != source || e.destination != destination)
        {
            replica.entry = Some(Entry {
                handle: LiveWorldRef::new(source, id),
                owner: "sunrise:live_worlds",
                destination,
                cells: replica.cells.clone(),
                limits: LiveWorldLimits {
                    cells: 65536,
                    chunks: 4096,
                    boxes: replica.boxes,
                    ..Default::default()
                },
                sample: replica.sample,
                collider: replica.collider,
                accepted_tick: None,
                tick_start: replica.sample.previous,
                carrying: true,
                frozen: false,
                geometry: RefCell::new(None),
                mesh: RefCell::new(None),
                membership: replica.membership,
                retained: replica.chunks.clone(),
                stream_areas: Vec::new(),
            });
        }
        let entry = replica.entry.as_mut()?;
        if entry.membership != replica.membership {
            entry.cells.clone_from(&replica.cells);
            entry.retained.clone_from(&replica.chunks);
            entry.membership = replica.membership;
        }
        entry.sample = replica.sample;
        entry.tick_start = replica.sample.previous;
        entry.frozen = false;
        Some(entry)
    }

    pub(super) fn client_projection_colliders(&self, base: Prepared) -> Prepared {
        let mut table = self.live_worlds.borrow_mut();
        if table.sync.replicas.is_empty() {
            return base;
        }
        let mut placed: Vec<_> = base
            .placed()
            .map(|p| Placed {
                id: p.id,
                collider: p.collider.clone(),
                exempt: p.exempt.clone(),
            })
            .collect();
        for (&id, replica) in &mut table.sync.replicas {
            let Some(entry) = self.replica_entry(id, replica) else {
                continue;
            };
            let Ok(geometry) = self.projection_geometry(entry) else {
                continue;
            };
            placed.push(Placed {
                id: entry.collider,
                collider: OrientedCollider::new(
                    frame(entry.sample.current),
                    geometry.boxes.clone(),
                ),
                exempt: Vec::new(),
            });
        }
        Prepared::new(placed)
    }

    pub(super) fn client_projection_frame(&self, id: u32) -> Option<Frame> {
        let mut table = self.live_worlds.borrow_mut();
        for (&generation, replica) in &mut table.sync.replicas {
            if replica.collider == id {
                return Some(frame(
                    self.replica_entry(generation, replica)?.sample.current,
                ));
            }
        }
        None
    }

    pub(super) fn client_projection_geometry(
        &self,
        camera: glam::DVec3,
        partial: f64,
        out: &mut Vec<Vertex>,
    ) {
        let mut table = self.live_worlds.borrow_mut();
        for (&id, replica) in &mut table.sync.replicas {
            if let Some(entry) = self.replica_entry(id, replica) {
                self.draw_projection(entry, camera, partial, out);
            }
        }
    }
}
