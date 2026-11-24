//! Internal stored projection pick and authenticated action transport; no mod dispatch authority is borrowed.
use super::*;
#[cfg(test)]
#[path = "picking/tests.rs"]
mod tests;
use mod_api::{Hand, PayloadBytes, PayloadId, WorldEpoch};
use sunrise_protocol::live_world_actions::{self as wire, Action, Request};

#[derive(Clone, Copy)]
pub(in crate::game) struct Pick {
    epoch: WorldEpoch,
    destination: LevelRef,
    pub handle: LiveWorldRef,
    pub hit: LiveWorldHit,
    membership: u64,
    actionable: bool,
}

impl Game<'_> {
    pub(in crate::game) fn clear_projection_pick(&self) {
        self.live_worlds.borrow_mut().pick = None;
    }
    /// A frozen replica consumes the projected target but cannot publish an action request.
    pub(in crate::game) fn refresh_projection_pick(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
        nearer: Option<f64>,
    ) {
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return;
        };
        let end = vector(origin).add_vector(vector(direction).scale(self.reach()));
        let mut table = self.live_worlds.borrow_mut();
        table.pick = None;
        let mut best = nearer.unwrap_or(self.reach());
        let mut selected = None;
        for (&id, replica) in &mut table.sync.replicas {
            let source_revision = replica.source_revision;
            let Some(entry) = self.replica_entry(id, replica) else {
                continue;
            };
            if entry.destination.dimension() != self.dim.key() {
                continue;
            }
            let Ok(geometry) = self.projection_geometry(entry) else {
                continue;
            };
            let Ok(pose) = checked_pose(entry.sample.current) else {
                continue;
            };
            let Some(hit) = geometry.contact.clip(&pose, vector(origin), end) else {
                continue;
            };
            let distance = hit.location.distance_squared(vector(origin)).sqrt();
            if distance >= best {
                continue;
            }
            best = distance;
            selected = Some(Pick {
                epoch,
                destination: entry.destination,
                handle: entry.handle,
                membership: entry.membership,
                actionable: !entry.frozen,
                hit: LiveWorldHit::new(
                    entry.handle,
                    [hit.local_cell.x, hit.local_cell.y, hit.local_cell.z],
                    entry.sample.revision,
                    source_revision,
                    hit.location.components(),
                    hit.local_direction,
                    hit.local_location.components(),
                ),
            });
        }
        table.pick = selected;
    }

    pub(in crate::game) fn projection_pick(&self) -> Option<Pick> {
        let pick = self.live_worlds.borrow().pick?;
        (self.native_lifecycle.epoch() == Some(pick.epoch)
            && pick.destination.dimension() == self.dim.key()
            && self.live_world_lifecycle_current(pick.destination)
            && self.live_world_lifecycle_current(pick.handle.level()))
        .then_some(pick)
    }

    /// A client-pick listener can choose a genuinely nearer ordinary target. A farther
    /// target must not turn a projected click into a vanilla action on the hidden parent.
    pub(in crate::game) fn finish_projection_pick(
        &self,
        ordinary: Option<mod_api::CrosshairTarget>,
    ) -> Option<mod_api::CrosshairTarget> {
        let Some(pick) = self.projection_pick() else {
            return ordinary;
        };
        let point = ordinary.and_then(|target| match target {
            mod_api::CrosshairTarget::Block { hit, .. } => Some(hit.point),
            mod_api::CrosshairTarget::Entity { point, .. } => Some(point),
            _ => None,
        });
        if point.is_some_and(|point| self.pick_distance(point) < self.pick_distance(pick.hit.point))
        {
            self.live_worlds.borrow_mut().pick = None;
            ordinary
        } else {
            None
        }
    }

    fn send_projection_action(&mut self, action: Action, hand: Hand) -> bool {
        let Some(pick) = self.projection_pick() else {
            return false;
        };
        if !pick.actionable {
            return false;
        }
        // Preserve held-slot ordering before a projected use/held destroy as for ordinary packets.
        self.remote_send_carried();
        let sequence = self.next_sequence();
        let request = Request {
            action,
            generation: pick.handle.generation(),
            membership: pick.membership,
            pose_revision: pick.hit.pose_revision,
            source_revision: pick.hit.cell_revision,
            cell: pick.hit.cell,
            face: crate::game::remote::face_direction(pick.hit.face.normal()),
            point: pick.hit.point,
            local_point: pick.hit.local_point,
            hand: u8::from(hand == Hand::Off),
            sequence,
        };
        let Ok(bytes) = wire::encode(&request) else {
            return false;
        };
        self.send_payload_to_server(&PayloadId(wire::CHANNEL.into()), PayloadBytes(bytes))
            .is_ok()
    }

    /// Some consumes this projected click even on refusal: no item/parent fallback.
    /// This first slice refuses survival mining rather than inventing a second dig clock.
    pub(in crate::game) fn remote_projection_mining(&mut self, start: bool) -> Option<bool> {
        self.projection_pick()?;
        self.stop_remote_mining();
        self.mining = None;
        if !self.creative || self.spectator() || self.using.is_some() {
            return Some(false);
        }
        if !self
            .res
            .items()
            .can_destroy_block(self.held(), self.creative)
        {
            return Some(false);
        }
        let remote = self.remote.as_mut()?;
        if !start && remote.destroy_delay > 0 {
            remote.destroy_delay -= 1;
            return Some(false);
        }
        let sent = self.send_projection_action(Action::StartDestroy, Hand::Main);
        if sent && let Some(remote) = self.remote.as_mut() {
            remote.destroy_delay = 5;
        }
        Some(sent)
    }

    pub(in crate::game) fn remote_projection_use(&mut self) -> bool {
        if self.projection_pick().is_none() {
            return false;
        }
        // Server retains use/lock/sneak/held semantics. No local block or inventory prediction.
        self.send_projection_action(Action::Use, Hand::Main);
        true
    }

    /// No level/player/owner handle from the wire is accepted. Generation is resolved from
    /// the current server table in this authenticated connection's physical dimension.
    pub(in crate::game) fn guest_projection_action(&mut self, conn: u32, data: &[u8]) {
        let Ok(request) = wire::decode(data) else {
            return;
        };
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return;
        };
        if guest.dead
            || !guest.placed
            || !guest.participates_online()
            || !guest
                .sender
                .payload_transport()
                .epoch()
                .is_some_and(|epoch| {
                    epoch.active()
                        && epoch.phase == sunrise_protocol::payload_negotiation::Phase::Play
                        && epoch.peer.engine_receives(epoch.phase, wire::CHANNEL)
                })
        {
            return;
        }
        let (uuid, dimension) = (guest.uuid, guest.dimension.clone());
        let Some(player) = self.player(uuid) else {
            return;
        };
        if guest.sender.payload_transport().player() != Some(player) {
            return;
        }
        let handle = {
            let table = self.live_worlds.borrow();
            let Some(entry) = table.entries.get(&request.generation) else {
                return;
            };
            if entry.destination.dimension_id() != dimension
                || entry.membership != request.membership
            {
                return;
            }
            entry.handle
        };
        if request.action == Action::Use {
            let Some(world) = self.native_level_world(handle.level()) else {
                return;
            };
            let state = world.block(request.cell[0], request.cell[1], request.cell[2]);
            let menu = matches!(
                &*self.res.facts().get(state).name,
                "minecraft:chest"
                    | "minecraft:trapped_chest"
                    | "minecraft:ender_chest"
                    | "minecraft:furnace"
                    | "minecraft:blast_furnace"
                    | "minecraft:smoker"
            );
            // Only the actual server inventory is authoritative. The request contains no item/state.
            let held = if request.hand == 1 {
                guest.offhand.as_ref()
            } else {
                guest.inventory.get(guest.selected).and_then(Option::as_ref)
            };
            let block_item = held.is_some_and(|stack| {
                self.res
                    .items()
                    .get(&stack.item)
                    .is_some_and(|item| item.block.is_some())
            });
            if !menu && !block_item {
                return;
            }
            // The ordinary source use path itself rejects unsupported screen providers
            // after resolving the actual held hand and the source block's menu policy.
        }
        let face = match request.face {
            0 => sunrise_core::geometry::Direction::Down,
            1 => sunrise_core::geometry::Direction::Up,
            2 => sunrise_core::geometry::Direction::North,
            3 => sunrise_core::geometry::Direction::South,
            4 => sunrise_core::geometry::Direction::West,
            5 => sunrise_core::geometry::Direction::East,
            _ => return,
        };
        let hit = LiveWorldHit::new(
            handle,
            request.cell,
            request.pose_revision,
            request.source_revision,
            request.point,
            face,
            request.local_point,
        );
        if let Some(host) = self.lan.as_mut() {
            host.hub.ack(conn, request.sequence);
        }
        match request.action {
            Action::Use => {
                let hand = if request.hand == 1 {
                    Hand::Off
                } else {
                    Hand::Main
                };
                let _ = self.live_world_use_block_resolved(handle, player, hit, hand);
            }
            Action::StartDestroy => {
                // Use the ordinary guest's cooldown, never a parallel projected clock.
                self.as_guest(conn, |game| {
                    if !game.creative
                        || game.spectator()
                        || game.destroy_delay > 0
                        || !game
                            .res
                            .items()
                            .can_destroy_block(game.held(), game.creative)
                    {
                        return;
                    }
                    let _ = game.live_world_destroy_block_resolved(handle, player, hit, false);
                    game.destroy_delay = 5;
                });
            }
        }
    }

    /// Ordinary parent packets are denied through a live wall, even if a client's cached
    /// projection was stale or a listener supplied a farther ordinary crosshair target.
    pub(in crate::game) fn projection_parent_obstructed(&self, conn: u32, point: [f64; 3]) -> bool {
        let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return true;
        };
        let eye = vector(guest.eye());
        let end = vector(point);
        if !end.is_finite() {
            return true;
        }
        let table = self.live_worlds.borrow();
        let mut remaining = None::<usize>;
        for entry in table
            .entries
            .values()
            .filter(|entry| entry.destination.dimension_id() == guest.dimension)
        {
            let budget = remaining.get_or_insert(entry.limits.contacts as usize);
            *budget = (*budget).min(entry.limits.contacts as usize);
            let Ok(geometry) = self.projection_geometry(entry) else {
                return true;
            };
            let Some(left) = budget.checked_sub(geometry.boxes.boxes().len()) else {
                return true;
            };
            *budget = left;
            let Ok(pose) = checked_pose(entry.sample.current) else {
                return true;
            };
            if geometry.contact.clip(&pose, eye, end).is_some_and(|hit| {
                hit.location.distance_squared(eye) + 1.0e-10 < end.distance_squared(eye)
            }) {
                return true;
            }
        }
        false
    }

    /// Same cell and interpolated frame as the real projection; only the existing edge renderer is used.
    pub(in crate::game) fn projection_pick_outline(&self, partial: f64) -> Vec<[f64; 6]> {
        let Some(pick) = self.projection_pick() else {
            return Vec::new();
        };
        let mut table = self.live_worlds.borrow_mut();
        let Some(replica) = table.sync.replicas.get_mut(&pick.handle.generation()) else {
            return Vec::new();
        };
        let Some(poses) = self.client_projection_draw_poses(pick.handle.generation(), replica)
        else {
            return Vec::new();
        };
        let Some(entry) = self.replica_entry(pick.handle.generation(), replica) else {
            return Vec::new();
        };
        if entry.frozen || entry.membership != pick.membership {
            return Vec::new();
        }
        let Some(world) = self.native_level_view_world(entry.handle.level()) else {
            return Vec::new();
        };
        let Ok(shape) = shape(self, world, pick.hit.cell) else {
            return Vec::new();
        };
        let pose = frame(poses[0]).interpolate(&frame(poses[1]), partial);
        let edges: Vec<_> = shape
            .edges()
            .into_iter()
            .map(|edge| {
                let point = |offset: usize| {
                    pose.to_world(Vec3::new(
                        edge[offset] + f64::from(pick.hit.cell[0]),
                        edge[offset + 1] + f64::from(pick.hit.cell[1]),
                        edge[offset + 2] + f64::from(pick.hit.cell[2]),
                    ))
                    .components()
                };
                let a = point(0);
                let b = point(3);
                [a[0], a[1], a[2], b[0], b[1], b[2]]
            })
            .collect();
        trace::emit("outline_draw", || {
            serde_json::json!({
                "generation": pick.handle.generation(), "collider": entry.collider,
                "client_tick": self.ticks, "coherent_sample": trace::sample(entry.sample),
                "membership": entry.membership, "cell": pick.hit.cell,
                "poses": poses.map(trace::pose), "partial": partial, "partial_bits": partial.to_bits(),
                "transform": trace::transform(&pose), "edges": edges.len(),
                "first_edge": edges.first(), "first_edge_bits": edges.first().map(|edge| edge.map(f64::to_bits))
            })
        });
        edges
    }
}
