//! A native mod's general rows as a Sunrise client sees them (#792, stage 5): the server's
//! `sunrise:general_entity` spawn adds one, the vanilla packets keyed by its network id move and
//! remove it as they do any tracked entity, and its declared fields are its synced attachments
//! (`Remote::attachments`). The server sends every row its first attachment snapshot (empty or
//! not) in another payload: a row is shown only once it has a published attachment generation.
//! Render extraction retains that generation, not the live synced holder. The row's behaviour is
//! the server's.
use super::{Game, Lerp, Remote};
use crate::game::general::general_type_catalog;
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use sunrise_client::join_seq::JoinSeq;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_mod_runtime::attachments::{Attachments, Holder};
use sunrise_protocol::{
    general_entity::{self, Message},
    retained::OwnedHeap,
};

// Bound retained native backing per complete generation, not packet size or individual fields.
// This also covers a generation assembled from multiple deltas.
const RENDER_SNAPSHOT_BYTES: usize = 64 << 20;

/// One general row the server shows the window.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) struct RemoteGeneral {
    /// Admission to the level, even if the first attachment snapshot has yet to arrive.
    pub join: JoinSeq,
    /// The client's frozen catalog slot for the row's type.
    pub type_slot: usize,
    pub uuid: u128,
    pub position: [f64; 3],
    /// Where it was when the last client tick began, for a frame's partial tick.
    pub previous: [f64; 3],
    /// Yaw, then pitch, in degrees.
    pub rotation: [f32; 2],
    pub previous_rotation: [f32; 2],
    /// Client ticks since the spawn.
    pub age: u32,
    // Deltas can form a complete generation only after the initial sync, even if that first
    // generation was refused for rendering.
    received_initial_snapshot: bool,
    /// The last whole generation published at spawn or sync. Until then the row is not shown.
    pub render_attachments: Option<mod_api::EntityRenderAttachments>,
}

impl RemoteGeneral {
    pub(in crate::game) fn new(
        type_slot: usize,
        uuid: u128,
        position: [f64; 3],
        rotation: [f32; 2],
    ) -> Self {
        Self {
            join: JoinSeq::next(),
            type_slot,
            uuid,
            position,
            previous: position,
            rotation,
            previous_rotation: rotation,
            age: 0,
            received_initial_snapshot: false,
            render_attachments: None,
        }
    }

    pub(in crate::game) fn is_shown(&self) -> bool {
        self.render_attachments.is_some()
    }

    fn attachments_arrived(&mut self, id: i32, replace: bool, attachments: &Attachments) {
        self.received_initial_snapshot |= replace;
        if !self.received_initial_snapshot {
            return;
        }
        let Some(generation) = mod_api::__private::game::capture_entity_render_attachments(
            attachments,
            RENDER_SNAPSHOT_BYTES,
            OwnedHeap::owned_heap_bytes,
        ) else {
            log::warn!(
                "Sunrise: synced render attachments for {} entity {id} not published: generation exceeds the {RENDER_SNAPSHOT_BYTES}-byte retention budget",
                self.kind()
            );
            return;
        };
        self.render_attachments = Some(generation);
    }

    pub(in crate::game) fn bounds(&self) -> Aabb {
        let [x, y, z] = self.position;
        general_type_catalog().bounds(self.type_slot, Vec3::new(x, y, z))
    }

    pub(in crate::game) fn kind(&self) -> &'static str {
        general_type_catalog().key(self.type_slot)
    }
}

/// The rows the server shows the window, by network id, and the warnings already given.
#[derive(Default)]
pub(in crate::game) struct RemoteGenerals {
    /// Client entity lookup order: inserts append; removing one keeps the others in order.
    pub rows: IndexMap<i32, RemoteGeneral>,
    /// Ids whose whole attachment snapshot arrived before an entity spawn.
    snapshots_before_spawn: HashSet<i32>,
    /// Type keys this client's catalog does not hold, each warned about once.
    unknown: std::collections::HashSet<String>,
    /// Whether a spawn for an id already held was refused before, warned about once.
    duplicate: bool,
    // Reuse the parent-before-child placement order while the window ticks.
    riders: Vec<(usize, i32, u32, usize)>,
}

impl RemoteGenerals {
    pub(in crate::game) fn shown(&self, id: i32) -> Option<&RemoteGeneral> {
        self.rows.get(&id).filter(|row| row.is_shown())
    }

    /// Forget every row: a world change or a respawn into another dimension.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.snapshots_before_spawn.clear();
    }

    /// The underlying replica was cleared. The last published generations can still draw,
    /// but deltas cannot publish a new one until there is a complete replica again.
    pub(super) fn reset_attachment_sync(&mut self) {
        self.snapshots_before_spawn.clear();
        for row in self.rows.values_mut() {
            row.received_initial_snapshot = false;
        }
    }

    /// Publish after the complete message has been applied. A delta before the initial sync
    /// cannot open the visibility gate; after it, a successful capture can.
    pub fn attachments_arrived(&mut self, id: i32, replace: bool, attachments: &Attachments) {
        match self.rows.get_mut(&id) {
            Some(row) => row.attachments_arrived(id, replace, attachments),
            None if replace => {
                self.snapshots_before_spawn.insert(id);
            }
            _ => {}
        }
    }
}

impl Remote {
    /// Discard the mirror and its initial sync state.
    pub(in crate::game) fn clear_attachments(&mut self) {
        self.attachments.clear();
        self.general.reset_attachment_sync();
    }
}

impl Game<'_> {
    /// The declared type key of general row `id`: the level's own row, or on a client the
    /// one its server shows it.
    pub(in crate::game) fn general_kind(&self, id: u32) -> Option<&'static str> {
        let slot = match self.resident_entities.general.row(id) {
            Some(row) => row.type_slot,
            None => self.shown_remote_general(id as i32)?.type_slot,
        };
        Some(crate::game::general::general_type_catalog().key(slot))
    }

    pub(in crate::game) fn shown_remote_general(&self, id: i32) -> Option<&RemoteGeneral> {
        self.remote.as_ref()?.general.shown(id)
    }

    /// A `sunrise:general_entity` payload the server sent the window.
    pub(super) fn remote_general_entity(&mut self, data: &[u8]) {
        let message = match general_entity::decode(data) {
            Ok(message) => message,
            Err(e) => return log::debug!("Sunrise: a bad general entity spawn: {e}"),
        };
        let Message::Spawn {
            id,
            uuid,
            type_key,
            position,
            rotation,
            velocity: _,
        } = message;
        // Both sides froze the same declarations (general types come from both-side
        // packages): a key this client lacks comes from a server that does not match it.
        let Some(type_slot) = general_type_catalog().slot(&type_key) else {
            if let Some(r) = self.remote.as_mut()
                && r.general.unknown.insert(type_key.clone())
            {
                log::warn!(
                    "Sunrise: the server sent general entity type {type_key}, not installed here"
                );
            }
            return;
        };
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if r.general.rows.contains_key(&id) {
            if !std::mem::replace(&mut r.general.duplicate, true) {
                log::warn!("Sunrise: the server sent general entity {id} twice; the first stays");
            }
            return;
        }
        let mut row = RemoteGeneral::new(type_slot, uuid, position, rotation);
        // A whole snapshot can already be here. A map made by a delta (or a local read) is
        // insufficient: neither establishes the row's initial synced state.
        if r.general.snapshots_before_spawn.remove(&id)
            && let Some(attachments) = r.attachments.get(&Holder::Entity(id as u32))
        {
            row.attachments_arrived(id, true, attachments);
        }
        r.general.rows.insert(id, row);
        self.persist
            .uuids
            .insert(id as u32, sunrise_client::lan::playerdata::uuid_ints(uuid));
        // Tracked: the id-keyed move, position sync, teleport and removal handlers apply.
        self.spawn_entity(
            id,
            &type_key,
            position,
            [rotation[0], rotation[1], rotation[0]],
        );
        if let Some(r) = self.remote.as_mut() {
            r.lerps.remove(&id);
        }
    }

    /// `ClientLevel.tickPassenger` for the general rows in an applied passenger
    /// list. Seat a nested general vehicle before its own general passengers.
    pub(in crate::game) fn position_remote_general_passengers(&mut self) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        if remote.general.rows.is_empty() || remote.passenger_vehicles.is_empty() {
            return;
        }
        let mut riders = std::mem::take(&mut remote.general.riders);
        riders.clear();
        riders.extend(remote.passenger_vehicles.iter().filter_map(|(&id, seat)| {
            let id = id as i32;
            if !remote.general.rows.contains_key(&id) {
                return None;
            }
            let depth = std::iter::successors(Some(seat.vehicle), |&v| remote.passenger_vehicle(v))
                .take(remote.passenger_vehicles.len() + 1)
                .count();
            Some((depth, id, seat.vehicle, seat.index))
        }));
        riders.sort_unstable();
        for &(_, id, vehicle, _) in &riders {
            if let Some(at) = self.general_passenger_position(vehicle, id as u32)
                && let Some(remote) = self.remote.as_mut()
                && let Some(row) = remote.general.rows.get_mut(&id)
            {
                row.position = [at.x, at.y, at.z];
            }
        }
        if let Some(remote) = self.remote.as_mut() {
            remote.general.riders = riders;
        }
    }

    /// The server removed general row `id` from the window.
    pub(super) fn forget_remote_general(r: &mut Remote, id: i32) {
        r.general.rows.shift_remove(&id);
        r.general.snapshots_before_spawn.remove(&id);
    }
}

/// One client tick of the rows: `setOldPosAndRot`, then a step of the non-living
/// `InterpolationHandler` (`Entity.moveOrInterpolateTo`, 3 steps) toward the server's last move.
pub(super) fn tick_remote_general(
    rows: &mut IndexMap<i32, RemoteGeneral>,
    lerps: &mut HashMap<i32, Lerp>,
) {
    for (id, row) in rows.iter_mut() {
        row.previous = row.position;
        row.previous_rotation = row.rotation;
        row.age = row.age.saturating_add(1);
        let Some(l) = lerps.get_mut(id).filter(|l| l.steps > 0) else {
            continue;
        };
        let n = f64::from(l.steps);
        for k in 0..3 {
            row.position[k] += (l.target[k] - row.position[k]) / n;
        }
        if let Some((yaw, pitch)) = l.rotation {
            row.rotation[0] =
                crate::game::remote_objects::lerp_yaw_step(row.rotation[0], yaw, l.steps);
            row.rotation[1] += (pitch - row.rotation[1]) / l.steps as f32;
        }
        l.steps -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    #[test]
    fn a_move_is_reached_in_three_ticks_and_each_tick_keeps_the_one_before() {
        let row = RemoteGeneral::new(0, 1, [0.0, 70.0, 0.0], [0.0, 0.0]);
        let mut rows = IndexMap::from([(5, row)]);
        let mut lerps = HashMap::from([(
            5,
            Lerp {
                target: [3.0, 70.0, 0.0],
                rotation: Some((90.0, 30.0)),
                steps: 3,
            },
        )]);
        let mut seen = Vec::new();
        for _ in 0..4 {
            tick_remote_general(&mut rows, &mut lerps);
            let r = &rows[&5];
            seen.push((r.previous[0], r.position[0], r.rotation, r.age));
        }
        assert_eq!(seen[0], (0.0, 1.0, [30.0, 10.0], 1));
        assert_eq!(seen[1], (1.0, 2.0, [60.0, 20.0], 2));
        assert_eq!(seen[2], (2.0, 3.0, [90.0, 30.0], 3));
        // Reached: it stays, and only ages.
        assert_eq!(seen[3], (3.0, 3.0, [90.0, 30.0], 4));
    }
    struct Declare;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Declare {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let types = [
                ("missile", 0.5, Some([1.0, 1.25, 0.0]), false),
                ("package", 2.0, None, true),
            ];
            for (name, height, passenger_attachment, pickable) in types {
                r.declare_general_entity(
                    name,
                    mod_api::GeneralEntityCategory::Misc,
                    0.5,
                    height,
                    passenger_attachment,
                    mod_api::GeneralSavePolicy::Save,
                    true,
                    pickable,
                    (!pickable).then_some(mod_api::PushReaction::IgnoreEntity),
                    8,
                    1,
                    1,
                    &[],
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn an_over_budget_generation_does_not_replace_or_open_visibility() {
        use mod_api::{AttachmentCodec, AttachmentError, AttachmentType};
        use sunrise_core::nbt::{Compound, Tag};
        use sunrise_mod_runtime::attachments::AttachmentSync;

        const FUEL: AttachmentType<i32> = AttachmentType {
            namespace: "mymod",
            name: "fuel",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
                decode: |c| match c.get(&"n".into()) {
                    Some(Tag::Int(n)) => Ok(*n),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            sync: true,
            copy_on_death: false,
        };
        mod_platform::__private::initialize().unwrap();
        let _phase =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            let slot = general_type_catalog().slot("mymod:missile").unwrap();
            let mut generals = RemoteGenerals::default();
            for id in [5, 8] {
                generals
                    .rows
                    .insert(id, RemoteGeneral::new(slot, 1, [0.0; 3], [0.0; 2]));
            }
            let mut replica = Attachments::default();
            let sync = |holder: &mut Attachments, name: &str, value: Option<Compound>| {
                holder.apply_sync(AttachmentSync {
                    namespace: "mymod".into(),
                    name: name.into(),
                    value,
                });
            };
            let fuel = |n| Some([("n".into(), Tag::Int(n))].into());
            // OwnedHeap charges retained capacity. Reserve it without filling a large payload.
            // Either binding fits in isolation; their combined generation exceeds 64 MiB.
            let large =
                || Some([("bytes".into(), Tag::ByteArray(Vec::with_capacity(40 << 20)))].into());
            sync(&mut replica, "fuel", fuel(7));
            sync(&mut replica, "large_a", large());
            generals.attachments_arrived(5, true, &replica);

            sync(&mut replica, "fuel", fuel(11));
            sync(&mut replica, "large_b", large());
            generals.attachments_arrived(5, false, &replica);
            generals.attachments_arrived(8, true, &replica);
            let published_fuel = |rows: &RemoteGenerals, id| {
                *rows
                    .shown(id)
                    .unwrap()
                    .render_attachments
                    .as_ref()
                    .unwrap()
                    .get(&FUEL)
                    .unwrap()
                    .unwrap()
            };
            assert_eq!(published_fuel(&generals, 5), 7);
            assert!(generals.shown(8).is_none());

            sync(&mut replica, "large_a", None);
            for id in [5, 8] {
                generals.attachments_arrived(id, false, &replica);
                assert_eq!(published_fuel(&generals, id), 11);
            }

            generals.reset_attachment_sync();
            let mut replica = Attachments::default();
            sync(&mut replica, "fuel", fuel(3));
            generals.attachments_arrived(5, false, &replica);
            assert_eq!(published_fuel(&generals, 5), 11);
            generals.attachments_arrived(5, true, &replica);
            assert_eq!(published_fuel(&generals, 5), 3);
        });
    }

    fn spawn(id: i32, key: &str, at: [f64; 3]) -> C {
        spawn_facing(id, key, at, 0.0)
    }

    fn spawn_facing(id: i32, key: &str, at: [f64; 3], yaw: f32) -> C {
        C::CustomPayload(sunrise_protocol::packets::configuration::CustomPayload {
            channel: general_entity::CHANNEL.into(),
            data: general_entity::encode(&Message::Spawn {
                id,
                uuid: 0x77,
                type_key: key.into(),
                position: at,
                rotation: [yaw, 0.0],
                velocity: [0.0; 3],
            })
            .unwrap(),
        })
    }

    /// The server's whole snapshot of entity `id`'s synced attachments: `value` in
    /// `mymod:fuel`, or nothing synced.
    fn snapshot(id: u32, value: Option<i32>) -> C {
        attachment_packet(id, true, value)
    }

    fn attachment_packet(id: u32, replace: bool, value: Option<i32>) -> C {
        use sunrise_core::nbt::Tag;
        use sunrise_mod_runtime::attachments::AttachmentSync;
        let changes = value.map_or_else(Vec::new, |n| {
            vec![AttachmentSync {
                namespace: "mymod".into(),
                name: "fuel".into(),
                value: Some([("n".into(), Tag::Int(n))].into()),
            }]
        });
        C::CustomPayload(sunrise_protocol::packets::configuration::CustomPayload {
            channel: sunrise_protocol::attachment_sync::CHANNEL.into(),
            data: sunrise_protocol::attachment_sync::encode(
                &sunrise_protocol::attachment_sync::Message {
                    holder: Holder::Entity(id),
                    replace,
                    changes,
                },
            )
            .unwrap(),
        })
    }

    #[test]
    #[ignore = "daily"]
    fn a_shown_pickable_general_row_receives_the_named_use_and_attack() {
        use crate::game::general::{GeneralEntityRecord, GeneralTypeEnvelope};
        use crate::game::native_entity_refs::tests::{with_client_game, with_game};
        use crate::game::native_hotbar_slot::hotbar_slot_tests::{logged_in, sent};
        use crate::game::native_player_death::tests::add_guest;
        use mod_api::{Flow, HostSide, InteractionResult};
        use std::sync::{Arc, Mutex};
        use sunrise_client::{chunk::Chunk, first_person::Hand, motion::PlayerMotion};
        use sunrise_protocol::packets::play::Serverbound as S;

        const PACKAGE: &str = "mymod:package";
        const AT: [f64; 3] = [0.5, 65.0, 2.5];

        #[derive(Debug, PartialEq)]
        enum Event {
            ClientUse,
            ServerUse,
            Attack,
        }
        type Events = Arc<Mutex<Vec<(Event, i32, String)>>>;
        struct Watch(Events);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Watch {
            fn entity_interact(
                &self,
                host: HostSide<'_>,
                event: &mut mod_api::EntityInteractEvent<'_>,
            ) -> Option<InteractionResult> {
                let side = if matches!(host, HostSide::Client(_)) {
                    Event::ClientUse
                } else {
                    Event::ServerUse
                };
                self.0
                    .lock()
                    .unwrap()
                    .push((side, event.entity, event.kind.to_owned()));
                Some(InteractionResult::Consume)
            }

            fn attack_entity(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::AttackEntityEvent<'_>,
            ) -> Flow {
                self.0
                    .lock()
                    .unwrap()
                    .push((Event::Attack, event.entity, event.kind.to_owned()));
                Flow::Continue
            }
        }

        mod_platform::__private::initialize().unwrap();
        let _registration =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        let calls = Events::default();
        let mods = vec![
            mod_api::ModEntry::new("mymod", Box::new(Declare)),
            mod_api::ModEntry::new("observer", Box::new(Watch(Arc::clone(&calls)))),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let _world =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let mut pipe = logged_in(game);
                // The two closer boxes also meet the ray: one cannot be picked, and one has
                // not received its snapshot. Neither may win over the ready package.
                game.handle_packet(spawn(40, "mymod:missile", [0.5, 66.5, 1.2]))
                    .unwrap();
                game.handle_packet(snapshot(40, None)).unwrap();
                assert_eq!(
                    game.entity(40).unwrap().piston_push_reaction(game),
                    Ok(Some(mod_api::PushReaction::IgnoreEntity))
                );
                game.handle_packet(spawn(41, PACKAGE, [0.5, 65.0, 1.8]))
                    .unwrap();
                game.handle_packet(spawn(42, PACKAGE, AT)).unwrap();
                game.handle_packet(snapshot(42, None)).unwrap();
                game.refresh_client_pick(1.0);
                assert!(
                    matches!(game.stored_client_pick(), Some(mod_api::CrosshairTarget::Entity { entity, .. }) if entity.id() == 42)
                );
                game.remote_use(mod_api::InteractionAttempt::Press);
                game.remote_start_attack();
                let packets: Vec<_> = sent(&mut pipe)
                    .into_iter()
                    .filter_map(|packet| match packet {
                        S::Interact(packet) => Some((Event::ClientUse, packet.id)),
                        S::Attack(packet) => Some((Event::Attack, packet.id)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(packets, [(Event::ClientUse, 42), (Event::Attack, 42)]);
                assert_eq!(
                    *calls.lock().unwrap(),
                    [(Event::ClientUse, 42, PACKAGE.into())]
                );
            });
            calls.lock().unwrap().clear();
            with_game(|game| {
                let record = GeneralEntityRecord {
                    position: AT,
                    velocity: [0.0; 3],
                    rotation: [0.0; 2],
                    owner: None,
                    uuid: [1, 0, 0, 4],
                    envelope: GeneralTypeEnvelope {
                        type_key: PACKAGE.into(),
                        payload_version: 1,
                        payload_bytes: Vec::new(),
                    },
                    passengers: Vec::new(),
                };
                game.load_listed(PACKAGE, None, None, record.to_tag())
                    .unwrap();
                let id = game.resident_entities.general.rows[0].id as i32;
                add_guest(game);
                let at = Vec3::new(0.5, 65.0, 0.5);
                let mut body = PlayerMotion::new(game.motion, at).unwrap();
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.position = at.components();
                    guest.load_timeout = 0;
                    body.entity_id = guest.entity_id;
                    guest.motion = Some(Box::new(body));
                });
                game.reset_entity_residency();
                let _world =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                assert_eq!(
                    game.guest_interact(1, 0, id, Hand::Main, [0.0, 1.65, -0.25]),
                    Ok(true)
                );
                // The named row is still in packet range, but a fresh ray can't see it.
                if !game.world.chunks.contains_key(&(0, 0)) {
                    game.world.chunks.insert((0, 0), Chunk::empty(0, 0).into());
                }
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 65, 1, stone).unwrap();
                game.world.set(0, 66, 1, stone).unwrap();
                game.as_guest(1, |guest| guest.attack_ticks = 20).unwrap();
                assert_eq!(
                    game.guest_attack(1, 0, id),
                    Ok(false),
                    "an unclaimed hit cannot hurt a plain row"
                );
                assert_eq!(game.as_guest(1, |guest| guest.attack_ticks), Some(0));
                assert!(game.resident_entities.general.row(id as u32).is_some());
                game.as_guest(1, |guest| guest.player.as_mut().unwrap().position.z = -64.5)
                    .unwrap();
                let too_far = Err(mod_api::ActionRefused::OutOfReach);
                assert_eq!(
                    game.guest_interact(1, 0, id, Hand::Main, [0.0, 1.65, -0.25]),
                    too_far
                );
                assert_eq!(game.guest_attack(1, 0, id), too_far);
                assert_eq!(
                    *calls.lock().unwrap(),
                    [
                        (Event::ServerUse, id, PACKAGE.into()),
                        (Event::Attack, id, PACKAGE.into())
                    ]
                );
                game.persist
                    .removals
                    .push(crate::game::commands::Target::General(id as u32));
                assert_eq!(
                    game.guest_attack(1, 0, id),
                    Err(mod_api::ActionRefused::InvalidTarget),
                    "the aim fallback must not recover a pending removal"
                );
            });
        });
    }

    /// The ids of the rows shown on the window.
    fn shown(game: &Game<'_>) -> Vec<i32> {
        let rows = &game.remote.as_ref().unwrap().general.rows;
        let mut ids: Vec<i32> = rows
            .iter()
            .filter(|(_, row)| row.is_shown())
            .map(|(&id, _)| id)
            .collect();
        ids.sort_unstable();
        ids
    }

    fn rows(game: &Game<'_>) -> IndexMap<i32, RemoteGeneral> {
        game.remote.as_ref().unwrap().general.rows.clone()
    }

    #[test]
    #[ignore = "daily"]
    fn riding_reads_follow_the_server_passenger_lists() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use mod_api::{ClientHost, EntityRef, WorldEpoch};

        if run_fresh_cases(
            "game::remote::general::tests::riding_reads_follow_the_server_passenger_lists",
            "SUNRISE_CLIENT_RIDING_TEST_CHILD",
            &["riding"],
        ) {
            return;
        }
        mod_platform::__private::initialize().unwrap();
        let _phase =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            crate::game::item_entities::thrown_items_tests::with_remote_game(|game| {
                let level = || play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 1,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                let position = |teleport| {
                    C::PlayerPosition(play::PlayerPosition {
                        teleport,
                        position: [0.5, 70.0, 0.5],
                        delta: [0.0; 3],
                        yaw: 0.0,
                        pitch: 0.0,
                        relatives: 0,
                    })
                };
                game.handle_packet(C::Login(play::Login {
                    player_id: 7,
                    hardcore: false,
                    levels: vec!["minecraft:overworld".into()],
                    max_players: 20,
                    chunk_radius: 8,
                    simulation_distance: 8,
                    reduced_debug_info: false,
                    show_death_screen: true,
                    limited_crafting: false,
                    spawn: level(),
                    online_mode: false,
                    enforces_secure_chat: false,
                }))
                .unwrap();
                game.handle_packet(position(1)).unwrap();
                let _world =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                game.handle_packet(spawn_facing(9, "mymod:missile", [2.5, 70.0, 0.5], 90.0))
                    .unwrap();
                game.handle_packet(spawn(12, "mymod:missile", [0.5, 70.0, 0.5]))
                    .unwrap();
                for id in [9, 12] {
                    game.handle_packet(snapshot(id, None)).unwrap();
                }
                game.spawn_entity(4, "minecraft:area_effect_cloud", [0.5, 70.0, 0.5], [0.0; 3]);
                game.spawn_entity(5, "minecraft:marker", [0.5, 70.0, 0.5], [0.0; 3]);
                game.spawn_entity(
                    6,
                    "minecraft:ominous_item_spawner",
                    [0.5, 70.0, 0.5],
                    [0.0; 3],
                );
                let set = |vehicle, passengers| {
                    C::SetPassengers(play::SetPassengers {
                        vehicle,
                        passengers,
                    })
                };
                let vehicle = game.entity(9).unwrap();
                let row = game.entity(12).unwrap();
                let cloud = game.entity(4).unwrap();
                let me = game.entity(7).unwrap();
                let unknown = EntityRef::new(vehicle.epoch(), 99);
                let stale = EntityRef::new(
                    WorldEpoch::new(vehicle.epoch().get().wrapping_add(1)),
                    vehicle.id(),
                );
                game.handle_packet(set(9, vec![99])).unwrap();
                let host: &dyn ClientHost = game;
                assert_eq!(vehicle.passengers(host), Some(vec![]));
                assert_eq!(vehicle.is_vehicle(host), Some(false));

                game.handle_packet(set(9, vec![12, 99, 7])).unwrap();
                game.handle_packet(set(12, vec![4])).unwrap();
                game.position_riders();
                assert_eq!(row.position(game), Some([2.5, 71.25, 1.5]));
                let host: &dyn ClientHost = game;
                assert_eq!(vehicle.passengers(host), Some(vec![row, me]));
                assert_eq!(vehicle.is_vehicle(host), Some(true));
                assert_eq!(row.vehicle(host), Some(vehicle));
                assert_eq!(me.vehicle(host), Some(vehicle));
                assert_eq!(cloud.is_passenger(host), Some(true));
                assert_eq!(cloud.root_vehicle(host), Some(vehicle));
                assert_eq!(unknown.vehicle(host), None);
                assert_eq!(unknown.passengers(host), None);
                assert_eq!(stale.passengers(host), None);
                assert_eq!(stale.is_vehicle(host), None);

                // An unknown vehicle can't displace a real relationship. Later tracking
                // doesn't apply either of its old packets or its mention as a passenger.
                game.handle_packet(set(99, vec![12])).unwrap();
                assert_eq!(row.vehicle(game), Some(vehicle));
                game.spawn_entity(99, "minecraft:marker", [0.5, 70.0, 0.5], [0.0; 3]);
                let late = game.entity(99).unwrap();
                assert_eq!(late.is_passenger(game), Some(false));
                assert_eq!(late.is_vehicle(game), Some(false));
                assert_eq!(vehicle.passengers(game), Some(vec![row, me]));

                game.handle_packet(set(4, vec![9])).unwrap();
                assert_eq!(vehicle.vehicle(game), None);
                assert_eq!(cloud.is_vehicle(game), Some(false));
                assert_eq!(cloud.root_vehicle(game), Some(vehicle));

                // Known entities that cannot accept a passenger do not displace
                // an existing ride either, even on a forced client mount.
                for refused in [5, 6] {
                    let cannot_carry = game.entity(refused).unwrap();
                    game.handle_packet(set(refused as i32, vec![12])).unwrap();
                    assert_eq!(cannot_carry.passengers(game), Some(vec![]));
                    assert_eq!(row.vehicle(game), Some(vehicle));
                }

                game.spawn_entity(20, "minecraft:happy_ghast", [2.5, 70.0, 0.5], [0.0; 3]);
                let ghast = game.entity(20).unwrap();
                game.handle_packet(set(20, vec![7, 12])).unwrap();
                game.position_riders();
                assert_eq!(row.position(game), Some([2.5 - 1.7, 74.0, 0.5]));
                game.handle_packet(set(9, vec![12])).unwrap();
                assert_eq!(me.vehicle(game), Some(ghast));
                game.handle_packet(set(7, vec![99])).unwrap();
                assert_eq!(late.vehicle(game), Some(me));
                // Replace the local player in the same level while it is both a rider and
                // a vehicle. A death never plays the ghast's voluntary dismount.
                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: level(),
                    keep: 0,
                }))
                .unwrap();
                assert_eq!(game.remote.as_ref().unwrap().ghast_dismount, None);
                game.handle_packet(position(2)).unwrap();
                assert_eq!(game.entity(7), Some(me));
                assert_eq!(ghast.position(game), Some([2.5, 70.0, 0.5]));
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|mob| mob.id == 20)
                        .unwrap()
                        .extra
                        .counter,
                    0
                );
                let host: &dyn ClientHost = game;
                assert_eq!(me.is_passenger(host), Some(false));
                assert_eq!(me.passengers(host), Some(vec![]));
                assert_eq!(late.is_passenger(host), Some(false));
                assert_eq!(vehicle.passengers(host), Some(vec![row]));
                assert_eq!(cloud.root_vehicle(host), Some(vehicle));

                game.handle_packet(set(9, vec![])).unwrap();
                assert_eq!(vehicle.passengers(game), Some(vec![]));
                assert_eq!(cloud.root_vehicle(game), Some(row));
            })
        });
    }

    /// The window's side of a general row: a spawn adds it in the client's catalog slot, an
    /// unknown key and a second spawn of one id change nothing; it is shown only once its first
    /// snapshot is here too, whichever comes first, with that snapshot's value; a position sync
    /// is reached over three ticks with nothing of the mod's run here, and a removal drops it
    /// with its synced attachments.
    #[test]
    #[ignore = "daily"]
    fn the_window_keeps_moves_and_drops_the_rows_it_is_shown() {
        mod_platform::__private::initialize().unwrap();
        let _phase =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            crate::game::item_entities::thrown_items_tests::with_remote_game(|game| {
                // Out-of-order deltas create a replica map, not an initial generation.
                game.handle_packet(attachment_packet(5, false, Some(99)))
                    .unwrap();
                game.handle_packet(spawn(5, "mymod:missile", [0.5, 70.0, 0.5]))
                    .unwrap();
                game.handle_packet(spawn(6, "other:unknown", [0.5, 70.0, 0.5]))
                    .unwrap();
                game.handle_packet(spawn(5, "mymod:missile", [9.5, 70.0, 0.5]))
                    .unwrap();
                let held = rows(game);
                assert_eq!(held.keys().collect::<Vec<_>>(), [&5]);
                assert_eq!(held[&5].position, [0.5, 70.0, 0.5]);
                assert_eq!(held[&5].uuid, 0x77);
                // A spawn and deltas still show nothing; the whole snapshot opens the gate.
                assert!(held[&5].render_attachments.is_none());
                game.handle_packet(attachment_packet(5, false, Some(12)))
                    .unwrap();
                assert!(shown(game).is_empty());
                game.handle_packet(snapshot(5, Some(9))).unwrap();
                assert_eq!(shown(game), [5]);
                assert!(rows(game)[&5].render_attachments.is_some());
                let fuel = |game: &Game<'_>, id: u32| {
                    let held = game.remote.as_ref().unwrap();
                    let map = held
                        .attachments
                        .get(&sunrise_mod_runtime::attachments::Holder::Entity(id))?;
                    // A synced value arrives raw, as `{value: <compound>}` (`apply_sync`).
                    let saved = map.entry("mymod", "fuel").ok()??.saved()?;
                    let Some(sunrise_core::nbt::Tag::Compound(value)) = saved.get(&"value".into())
                    else {
                        return None;
                    };
                    match value.get(&"n".into()) {
                        Some(sunrise_core::nbt::Tag::Int(n)) => Some(*n),
                        _ => None,
                    }
                };
                assert_eq!(fuel(game, 5), Some(9));
                // The snapshot first, an empty one: the spawn that follows is shown at once.
                game.handle_packet(snapshot(8, None)).unwrap();
                assert_eq!(shown(game), [5]);
                game.handle_packet(spawn(8, "mymod:missile", [0.5, 70.0, 0.5]))
                    .unwrap();
                assert_eq!(shown(game), [5, 8]);
                assert!(rows(game)[&8].render_attachments.is_some());
                game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
                    id: 5,
                    path: play::PositionPath::Linear([3.5, 70.0, 0.5]),
                    yaw: 0.0,
                    pitch: 0.0,
                    on_ground: false,
                }))
                .unwrap();
                let mut xs = Vec::new();
                for _ in 0..3 {
                    game.tick_remote_entities(true);
                    xs.push(rows(game)[&5].position[0]);
                }
                assert_eq!(xs, [1.5, 2.5, 3.5]);
                assert_eq!(rows(game)[&5].previous[0], 2.5);
                // A client holds rows only here: none is a resident row, ticked by mod code.
                assert!(game.resident_entities.general.rows.is_empty());
                let holder = sunrise_mod_runtime::attachments::Holder::Entity(5);
                game.remote
                    .as_mut()
                    .unwrap()
                    .attachments
                    .insert(holder, Default::default());
                game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![5, 8] }))
                    .unwrap();
                assert!(rows(game).is_empty());
                assert!(
                    !game
                        .remote
                        .as_ref()
                        .unwrap()
                        .attachments
                        .contains_key(&holder)
                );
            })
        });
    }
}
