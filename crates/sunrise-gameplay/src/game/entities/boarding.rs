//! A player's `Entity.boardingCooldown` is always 0: `Player.removeVehicle`
//! zeroes the 60 ticks `Entity.removePassenger` just set (#11158), so
//! `Entity.canRide` only asks that the player is not sneaking, which each
//! vehicle's interaction checks. Mobs keep theirs in `extra.boarding_cooldown`
//! (`boat_passengers::BOARDING_COOLDOWN`).
use crate::game::ServerGame;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::entity_memory::OffLevel;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt2 as _;
use sunrise_sim::{lan::guest::Guest, mobs::Mob};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) struct SeatedPassenger {
    pub order: i64,
    pub id: u32,
    pub player: bool,
}

struct SeatIterators<'a, G, RM, OM, GP, B> {
    acting: Option<u32>,
    riding: Option<u32>,
    has_player: bool,
    riding_player: Option<(u32, i64)>,
    placed_guests: G,
    resident_mobs: RM,
    lent_mobs: &'a [Mob],
    off_level_mobs: OM,
    general_passengers: GP,
    bolts: B,
}

fn visit_player_seat_sources<'a>(
    vehicle: u32,
    acting: Option<u32>,
    riding: Option<u32>,
    has_player: bool,
    riding_player: Option<(u32, i64)>,
    placed_guests: impl Iterator<Item = &'a Guest>,
    mut matches: impl FnMut(i64, u32) -> bool,
) -> bool {
    let mut acting_listed = false;
    for g in placed_guests {
        let rides = if acting == Some(g.conn) {
            acting_listed = true;
            riding
        } else {
            g.vehicle
        };
        if rides == Some(vehicle) && matches(g.vehicle_order, g.entity_id as u32) {
            return true;
        }
    }
    // An unlisted acting guest goes first. This game's own player keeps its seat order.
    riding_player.is_some_and(|(me, place)| {
        !acting_listed && has_player && riding == Some(vehicle) && matches(place, me)
    })
}

fn visit_seat_sources<'a, G, RM, OM, GP, B>(
    vehicle: u32,
    sources: SeatIterators<'a, G, RM, OM, GP, B>,
    mut matches: impl FnMut(SeatedPassenger) -> bool,
) -> bool
where
    G: Iterator<Item = &'a Guest>,
    RM: Iterator<Item = &'a Mob>,
    OM: Iterator<Item = &'a Mob>,
    GP: Iterator<Item = (i64, u32)>,
    B: Iterator<Item = (Option<u32>, i64, u32)>,
{
    if visit_player_seat_sources(
        vehicle,
        sources.acting,
        sources.riding,
        sources.has_player,
        sources.riding_player,
        sources.placed_guests,
        |order, id| {
            matches(SeatedPassenger {
                order,
                id,
                player: true,
            })
        },
    ) {
        return true;
    }
    let other_mobs = sources
        .lent_mobs
        .iter()
        .chain(sources.off_level_mobs)
        .filter(|m| m.extra.mount == Some(vehicle));
    sources
        .resident_mobs
        .chain(other_mobs)
        .filter(|m| !m.removed)
        .any(|m| {
            matches(SeatedPassenger {
                order: m.extra.vehicle_order,
                id: m.id,
                player: false,
            })
        })
        || sources.general_passengers.any(|(order, id)| {
            matches(SeatedPassenger {
                order,
                id,
                player: false,
            })
        })
        || sources
            .bolts
            .filter(|&(mount, ..)| mount == Some(vehicle))
            .any(|(_, order, id)| {
                matches(SeatedPassenger {
                    order,
                    id,
                    player: false,
                })
            })
}

fn collect_ordered_seats(
    visit: impl FnOnce(&mut dyn FnMut(SeatedPassenger) -> bool),
) -> Vec<SeatedPassenger> {
    let mut seats = Vec::new();
    visit(&mut |seat| {
        seats.push(seat);
        false
    });
    seats.sort_by_key(|seat| seat.order);
    seats
}

fn visit_game_passenger_seats(
    game: &ServerGame<'_>,
    vehicle: u32,
    lent_mobs: &[Mob],
    matches: impl FnMut(SeatedPassenger) -> bool,
) -> bool {
    visit_seat_sources(
        vehicle,
        SeatIterators {
            acting: game
                .server
                .lan_acting
                .filter(|_| game.common.player.is_some()),
            riding: game.common.riding,
            has_player: game.common.player.is_some(),
            riding_player: game.riding_player(),
            placed_guests: game.placed_seat_guests(),
            resident_mobs: game.resident_mob_riders(vehicle).map(|(_, m)| m),
            lent_mobs,
            off_level_mobs: game
                .common
                .persist
                .off_level
                .values()
                .filter_map(OffLevel::as_mob),
            general_passengers: game
                .common
                .general_passengers(vehicle)
                .map(|row| (row.vehicle_order, row.id)),
            bolts: game
                .common
                .bolts
                .iter()
                .map(|b| (b.mount, b.vehicle_order, b.id)),
        },
        matches,
    )
}

/// The passengers of `vehicle` in `Entity.passengers` order across all server
/// seat sources, preserving source encounter order when places tie.
pub(in crate::game) fn ordered_passengers(
    game: &ServerGame<'_>,
    vehicle: u32,
) -> Vec<SeatedPassenger> {
    collect_ordered_seats(|visit| {
        visit_game_passenger_seats(game, vehicle, &[], visit);
    })
}

/// The vehicle `rider` directly sits in across server seat sources.
pub(in crate::game) fn vehicle_of_entity(game: &ServerGame<'_>, rider: u32) -> Option<u32> {
    if game.common.player.is_some() && game.riding_player().is_some_and(|(id, _)| id == rider) {
        return game.common.riding;
    }
    let acting = game
        .server
        .lan_acting
        .filter(|_| game.common.player.is_some());
    if let Some(guest) = game
        .placed_seat_guests()
        .find(|g| g.entity_id as u32 == rider)
    {
        return if acting == Some(guest.conn) {
            game.common.riding
        } else {
            guest.vehicle
        };
    }
    match game.player_vehicle(rider) {
        Some(player) => game.native_vehicle_id(player),
        None => game
            .native_vehicle_id(Target::Mob(rider))
            .or_else(|| game.native_vehicle_id(Target::General(rider))),
    }
}

pub(super) fn next_boarded_rider(
    after: Option<u32>,
    mut seats: impl FnMut(&mut dyn FnMut(i64, u32) -> bool),
) -> Option<u32> {
    let mut after_key = None;
    if let Some(after) = after {
        let mut ordinal = 0usize;
        seats(&mut |place, rider| {
            if rider == after {
                after_key = Some((place, ordinal));
                return true;
            }
            ordinal += 1;
            false
        });
        after_key?;
    }
    let mut ordinal = 0usize;
    let mut next = None;
    seats(&mut |place, rider| {
        let key = (place, ordinal);
        ordinal += 1;
        if after_key.is_none_or(|after| key > after) && next.is_none_or(|(best, _)| key < best) {
            next = Some((key, rider));
        }
        false
    });
    next.map(|(_, rider)| rider)
}

/// `Entity.getIndirectPassengersStream` for `vehicle`: the vehicle and every
/// vehicle riding it, directly or through another rider. `mounts` holds
/// `(vehicle, its vehicle)` for every mob.
pub(in crate::game::server_side) fn passenger_vehicles(
    vehicle: u32,
    mounts: &[(u32, Option<u32>)],
) -> Vec<u32> {
    let mut ids = vec![vehicle];
    let mut i = 0;
    while i < ids.len() {
        let v = ids[i];
        i += 1;
        for &(id, mount) in mounts {
            if mount == Some(v) && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

impl ServerGame<'_> {
    /// `Entity.passengers` of `vehicle`, first passenger first: each one's place, id and whether
    /// it is a player. Places tie only for those that never boarded through `addPassenger` (a
    /// load keeps its list order), players ahead.
    pub fn seated_passengers(&self, vehicle: u32) -> Vec<(i64, u32, bool)> {
        ordered_passengers(self, vehicle)
            .into_iter()
            .map(|seat| (seat.order, seat.id, seat.player))
            .collect()
    }

    /// `Entity.isVehicle`: any kind of rider makes this a vehicle.
    pub fn has_seated_passengers(&self, vehicle: u32) -> bool {
        self.has_seated_passengers_with_lent_mobs(vehicle, &[])
    }

    /// The same check while the mob tick holds the resident list out of `Game`.
    pub(in crate::game) fn has_seated_passengers_with_lent_mobs(
        &self,
        vehicle: u32,
        lent_mobs: &[Mob],
    ) -> bool {
        self.any_passenger_seat(vehicle, lent_mobs, |_| true)
    }

    /// Shared seat enumeration; visit only until `matches` returns true. The
    /// enumeration order breaks ties for passengers with the same seat order.
    fn any_passenger_seat(
        &self,
        vehicle: u32,
        lent_mobs: &[Mob],
        mut matches: impl FnMut((i64, u32, bool)) -> bool,
    ) -> bool {
        visit_game_passenger_seats(self, vehicle, lent_mobs, |seat| {
            matches((seat.order, seat.id, seat.player))
        })
    }

    /// Player seats in source encounter order, shared with the mob tick's live loan.
    pub(super) fn any_player_passenger_seat(
        &self,
        vehicle: u32,
        matches: impl FnMut(i64, u32) -> bool,
    ) -> bool {
        visit_player_seat_sources(
            vehicle,
            self.server
                .lan_acting
                .filter(|_| self.common.player.is_some()),
            self.common.riding,
            self.common.player.is_some(),
            self.riding_player(),
            self.placed_seat_guests(),
            matches,
        )
    }

    fn placed_seat_guests(&self) -> impl Iterator<Item = &Guest> {
        self.server
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(move |g| g.placed && g.dimension == self.dim.id())
    }

    /// The parent behind a boarded seat. A displaced player's handle can be hidden during a
    /// nested guest turn, but the player's seat still has a parent in the hub.
    pub(super) fn seated_passenger_vehicle(&self, rider: u32) -> Option<u32> {
        vehicle_of_entity(self, rider)
    }

    /// The first actual rider, or the rider directly after `after`, even off-level.
    pub(super) fn next_passenger_seat(&self, vehicle: u32, after: Option<u32>) -> Option<u32> {
        next_boarded_rider(after, |visit| {
            self.any_passenger_seat(vehicle, &[], |(place, id, _)| visit(place, id));
        })
    }

    /// `Entity.startRiding` for the acting player: the rider link, then
    /// `CriteriaTriggers.START_RIDING_TRIGGER` for every server player the
    /// vehicle carries.
    pub fn start_riding(&mut self, vehicle: u32) {
        self.common.riding = Some(vehicle);
        self.seat_player(vehicle);
        self.note_started_riding(vehicle);
    }

    /// The first seat, preserving enumeration order on a tie without sorting or allocating.
    pub(in crate::game::server_side) fn first_seated_passenger(
        &self,
        vehicle: u32,
    ) -> Option<(i64, u32, bool)> {
        let mut first: Option<(i64, u32, bool)> = None;
        self.any_passenger_seat(vehicle, &[], |seat| {
            if first.is_none_or(|old| seat.0 < old.0) {
                first = Some(seat);
            }
            false
        });
        first
    }

    /// `getPassengers().size()` without allocating or sorting the seats.
    pub fn seated_passenger_count(&self, vehicle: u32) -> usize {
        let mut count = 0;
        self.any_passenger_seat(vehicle, &[], |_| {
            count += 1;
            false
        });
        count
    }

    /// `Entity.addPassenger`: the place of `rider` getting on `vehicle`. On a server a player
    /// goes ahead of a first passenger that is no player; everything else goes last.
    pub fn boarding_order(&self, vehicle: u32, rider: u32, player: bool) -> i64 {
        let mut aboard = ordered_passengers(self, vehicle);
        aboard.retain(|seat| seat.id != rider);
        match (aboard.first(), aboard.last()) {
            (Some(first), _) if player && !first.player => first.order - 1,
            (_, Some(last)) => last.order + 1,
            _ => 0,
        }
    }
    /// `addPassenger` for the mob `id`, linked to its vehicle just now.
    pub fn seat_mob(&mut self, id: u32) {
        let mount = self.common.mob_object(id);
        let Some(vehicle) = mount.and_then(|m| m.extra.mount) else {
            return;
        };
        let order = self.boarding_order(vehicle, id, false);
        if let Some(m) = self.mob_object_mut(id) {
            m.extra.vehicle_order = order;
        }
    }
    /// `addPassenger` for the player riding through `riding`, now in `vehicle`: the acting
    /// guest's place goes to the hub, this game's own player's to `riding_order`.
    pub(in crate::game::server_side) fn seat_player(&mut self, vehicle: u32) {
        match (self.server.lan_acting, self.server.lan_acting_entity) {
            (Some(conn), Some(me)) => {
                let order = self.boarding_order(vehicle, me, true);
                if let Some(host) = self.server.lan.as_mut() {
                    host.hub.update_guest(conn, |g| g.vehicle_order = order);
                }
            }
            (None, _) => {
                if let Some((me, _)) = self.riding_player() {
                    self.server.riding_order = self.boarding_order(vehicle, me, true);
                }
            }
            _ => {}
        }
    }

    /// A rider link changed on `vehicle` (`sendEventAndTriggers`). Every
    /// player aboard hears the trigger; a host's turn can fire it now, while
    /// a guest's turn (the hub held by its action) leaves it for
    /// [`Self::flush_started_riding`] once every player is back in place.
    pub(in crate::game) fn note_started_riding(&mut self, vehicle: u32) {
        self.server.started_riding.push(vehicle);
        if self.server.lan.is_some() && self.server.lan_acting.is_none() {
            self.flush_started_riding();
        }
    }

    /// Fires the queued `started_riding` triggers with every player in place.
    pub(in crate::game::server_side) fn flush_started_riding(&mut self) {
        for vehicle in std::mem::take(&mut self.server.started_riding) {
            self.trigger_started_riding(vehicle);
        }
    }
}

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt2 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::lan::{Joined, PlayerSender};

    fn test_guest(conn: u32, entity_id: i32, vehicle: Option<u32>, order: i64) -> Guest {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: format!("guest-{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            entity_id,
            [0.0, 64.0, 0.0],
            10,
        );
        guest.placed = true;
        guest.dimension = "minecraft:overworld".to_owned();
        guest.vehicle = vehicle;
        guest.vehicle_order = order;
        guest
    }

    fn test_mob(id: u32, mount: Option<u32>, order: i64, removed: bool) -> Mob {
        let mut mob = Mob::new(
            id,
            sunrise_sim::mobs::sensor_scan_tests::kind(),
            Vec3::new(0.0, 64.0, 0.0),
            0.0,
        );
        mob.extra.mount = mount;
        mob.extra.vehicle_order = order;
        mob.removed = removed;
        mob
    }

    fn test_general_row(
        id: u32,
        mount: Option<u32>,
        order: i64,
    ) -> sunrise_gameplay_common::game::general::GeneralRow {
        sunrise_gameplay_common::game::general::GeneralRow {
            id,
            uuid: [id as i32, 0, 0, 1],
            join: Default::default(),
            type_slot: 0,
            position: Vec3::new(0.0, 64.0, 0.0),
            previous: Vec3::new(0.0, 64.0, 0.0),
            velocity: Vec3::ZERO,
            rotation: [0.0; 2],
            previous_rotation: [0.0; 2],
            owner: None,
            fall_distance: 0.0,
            on_ground: false,
            tick_count: 0,
            no_physics: false,
            horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fluids: Default::default(),
            under_water: false,
            custom_name: None,
            custom_name_visible: false,
            base: Default::default(),
            payload_version: 0,
            payload_bytes: Vec::new(),
            mount,
            vehicle_order: order,
            boarding_cooldown: 0,
            attachment_lifetime: Default::default(),
        }
    }

    #[test]
    fn started_riding_reaches_every_rider_of_the_vehicle() {
        // Entity.startRiding: START_RIDING_TRIGGER for every ServerPlayer in
        // entityToRide.getIndirectPassengersStream(). A mob between two
        // riders is still part of that stream, and another vehicle is not.
        let mounts = [(7, Some(5)), (8, Some(7)), (9, Some(4))];
        let aboard = passenger_vehicles(5, &mounts);
        assert!(aboard.contains(&5), "the rider on the vehicle itself");
        assert!(aboard.contains(&7), "the mob in the vehicle's seat");
        assert!(aboard.contains(&8), "the rider of that mob");
        assert!(!aboard.contains(&9), "a mob riding another vehicle");
        assert!(!aboard.contains(&6), "a mob riding nothing");
    }

    #[test]
    fn ordered_passengers_preserves_source_tie_order_for_equal_places() {
        let vehicle = 500;
        let other = 999;

        // Acting guest rides `vehicle` through `riding`, overriding its stale hub vehicle.
        let guests = [
            test_guest(1, 101, Some(other), 0),
            test_guest(2, 102, Some(vehicle), 0),
            test_guest(3, 103, Some(other), 0),
        ];
        let resident_mobs = [
            test_mob(201, Some(vehicle), 0, false),
            test_mob(202, Some(vehicle), 0, true),
        ];
        let lent_mobs = [
            test_mob(203, Some(vehicle), 0, false),
            test_mob(204, Some(vehicle), 0, true),
            test_mob(205, Some(other), 0, false),
        ];
        let off_level_mobs = [
            test_mob(206, Some(vehicle), 0, false),
            test_mob(207, Some(vehicle), 0, true),
        ];
        let general = [(0, 301)];
        let bolts = [(Some(vehicle), 0, 401), (Some(other), 0, 402)];

        let with_acting_guest = collect_ordered_seats(|visit| {
            visit_seat_sources(
                vehicle,
                SeatIterators {
                    acting: Some(1),
                    riding: Some(vehicle),
                    has_player: true,
                    riding_player: Some((101, i64::MIN)),
                    placed_guests: guests.iter(),
                    resident_mobs: resident_mobs.iter(),
                    lent_mobs: &lent_mobs,
                    off_level_mobs: off_level_mobs.iter(),
                    general_passengers: general.iter().copied(),
                    bolts: bolts.iter().copied(),
                },
                visit,
            );
        });
        assert_eq!(
            with_acting_guest,
            vec![
                SeatedPassenger {
                    order: 0,
                    id: 101,
                    player: true,
                },
                SeatedPassenger {
                    order: 0,
                    id: 102,
                    player: true,
                },
                SeatedPassenger {
                    order: 0,
                    id: 201,
                    player: false,
                },
                SeatedPassenger {
                    order: 0,
                    id: 203,
                    player: false,
                },
                SeatedPassenger {
                    order: 0,
                    id: 206,
                    player: false,
                },
                SeatedPassenger {
                    order: 0,
                    id: 301,
                    player: false,
                },
                SeatedPassenger {
                    order: 0,
                    id: 401,
                    player: false,
                },
            ]
        );

        // Unlisted own player ties after placed guests and ahead of mobs.
        let with_own_player = collect_ordered_seats(|visit| {
            visit_seat_sources(
                vehicle,
                SeatIterators {
                    acting: None,
                    riding: Some(vehicle),
                    has_player: true,
                    riding_player: Some((110, 0)),
                    placed_guests: guests.iter(),
                    resident_mobs: resident_mobs.iter(),
                    lent_mobs: &lent_mobs,
                    off_level_mobs: off_level_mobs.iter(),
                    general_passengers: general.iter().copied(),
                    bolts: bolts.iter().copied(),
                },
                visit,
            );
        });
        let expected_ids = vec![102, 110, 201, 203, 206, 301, 401];
        assert_eq!(
            with_own_player.iter().map(|s| s.id).collect::<Vec<_>>(),
            expected_ids
        );

        // Cursor stepping matches the ordered list verbatim.
        let mut cursor_ids = Vec::new();
        let mut after = None;
        while let Some(id) = next_boarded_rider(after, |visit| {
            visit_seat_sources(
                vehicle,
                SeatIterators {
                    acting: None,
                    riding: Some(vehicle),
                    has_player: true,
                    riding_player: Some((110, 0)),
                    placed_guests: guests.iter(),
                    resident_mobs: resident_mobs.iter(),
                    lent_mobs: &lent_mobs,
                    off_level_mobs: off_level_mobs.iter(),
                    general_passengers: general.iter().copied(),
                    bolts: bolts.iter().copied(),
                },
                |seat| visit(seat.order, seat.id),
            );
        }) {
            cursor_ids.push(id);
            after = Some(id);
        }
        assert_eq!(cursor_ids, expected_ids);
    }

    #[test]
    fn ordered_passengers_sorts_mixed_and_tied_places_across_sources() {
        let vehicle = 600;
        let guests = [
            test_guest(1, 101, Some(vehicle), 2),
            test_guest(2, 102, Some(vehicle), 0),
        ];
        let resident_mobs = [
            test_mob(201, Some(vehicle), 2, false),
            test_mob(202, Some(vehicle), -1, false),
        ];
        let off_level_mobs = [test_mob(203, Some(vehicle), 0, false)];
        let general = [(0, 301), (-1, 302)];
        let bolts = [(Some(vehicle), 2, 401), (Some(vehicle), -2, 402)];

        let ordered = collect_ordered_seats(|visit| {
            visit_seat_sources(
                vehicle,
                SeatIterators {
                    acting: None,
                    riding: Some(vehicle),
                    has_player: true,
                    riding_player: Some((110, 0)),
                    placed_guests: guests.iter(),
                    resident_mobs: resident_mobs.iter(),
                    lent_mobs: &[],
                    off_level_mobs: off_level_mobs.iter(),
                    general_passengers: general.iter().copied(),
                    bolts: bolts.iter().copied(),
                },
                visit,
            );
        });
        assert_eq!(
            ordered,
            vec![
                SeatedPassenger {
                    order: -2,
                    id: 402,
                    player: false,
                },
                SeatedPassenger {
                    order: -1,
                    id: 202,
                    player: false,
                },
                SeatedPassenger {
                    order: -1,
                    id: 302,
                    player: false,
                },
                SeatedPassenger {
                    order: 0,
                    id: 102,
                    player: true,
                },
                SeatedPassenger {
                    order: 0,
                    id: 110,
                    player: true,
                },
                SeatedPassenger {
                    order: 0,
                    id: 203,
                    player: false,
                },
                SeatedPassenger {
                    order: 0,
                    id: 301,
                    player: false,
                },
                SeatedPassenger {
                    order: 2,
                    id: 101,
                    player: true,
                },
                SeatedPassenger {
                    order: 2,
                    id: 201,
                    player: false,
                },
                SeatedPassenger {
                    order: 2,
                    id: 401,
                    player: false,
                },
            ]
        );
    }

    #[test]
    fn ordered_passengers_matches_seated_passengers_on_live_game() {
        if std::env::var_os("SUNRISE_TEST_RESOURCES").is_none() {
            return;
        }
        use sunrise_gameplay_common::game::lightning_common::Bolt;
        use sunrise_sim::lan::{NetConfig, hub::HubConfig};

        crate::game::native_entity_refs::tests::with_game(|game| {
            let vehicle = 500;
            game.lan_host(
                crate::game::server_side::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..NetConfig::lan(0, String::new(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: game.dim.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 4,
                    integrated: true,
                    levels: game.server.level_shapes.clone(),
                    level_seeds: game.server.levels.0.runtime.seeds(),
                },
            )
            .unwrap();

            game.common.player.as_mut().unwrap().entity_id = 511;
            game.server.lan_acting = None;
            game.server.lan_acting_entity = None;
            game.server.lan_acting_uuid = None;
            game.common.riding = Some(vehicle);
            game.server.riding_order = 0;

            let guest_index = game.server.lan.as_ref().unwrap().hub.guests.len();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .push(test_guest(1, 510, Some(vehicle), 0));
            let mob_index = game.common.mobs.len();
            game.common
                .mobs
                .push(test_mob(520, Some(vehicle), 0, false));
            game.common.mobs.push(test_mob(521, Some(vehicle), 0, true));
            game.common
                .persist
                .off_level
                .insert(522, OffLevel::Mob(test_mob(522, Some(vehicle), 0, false)));
            let general_index = game.common.resident_entities.general.rows.len();
            game.common
                .resident_entities
                .general
                .rows
                .push(test_general_row(530, Some(vehicle), 0));
            let bolt_index = game.common.bolts.len();
            let (mut bolt, _) = Bolt::new(540, [0.0, 64.0, 0.0], false);
            bolt.mount = Some(vehicle);
            bolt.vehicle_order = 0;
            game.common.bolts.push(bolt);

            let seats = ordered_passengers(game, vehicle);
            let as_tuples: Vec<(i64, u32, bool)> =
                seats.iter().map(|s| (s.order, s.id, s.player)).collect();
            assert_eq!(as_tuples, game.seated_passengers(vehicle));
            assert_eq!(
                seats.iter().map(|s| s.id).collect::<Vec<_>>(),
                vec![510, 511, 520, 522, 530, 540]
            );
            assert_eq!(
                game.passenger_order(vehicle),
                vec![510, 511, 520, 522, 530, 540]
            );
            assert_eq!(game.first_seated_passenger(vehicle), Some((0, 510, true)));
            assert_eq!(game.seated_passenger_count(vehicle), 6);
            assert!(game.has_seated_passengers(vehicle));

            for rider in [510, 511, 520, 522, 530] {
                assert_eq!(vehicle_of_entity(game, rider), Some(vehicle));
            }
            assert_eq!(vehicle_of_entity(game, 521), None);

            // Reassign places with mixed and equal orders across sources.
            game.server.riding_order = 1;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(guest_index, |g| g.vehicle_order = 1);
            game.common
                .mobs
                .update(mob_index, |m| m.extra.vehicle_order = -1);
            game.common
                .resident_entities
                .general
                .rows
                .update(general_index, |r| r.vehicle_order = -1);
            game.common
                .bolts
                .update(bolt_index, |b| b.vehicle_order = 0);

            let mixed = ordered_passengers(game, vehicle);
            let mixed_tuples: Vec<(i64, u32, bool)> =
                mixed.iter().map(|s| (s.order, s.id, s.player)).collect();
            assert_eq!(mixed_tuples, game.seated_passengers(vehicle));
            assert_eq!(
                mixed.iter().map(|s| s.id).collect::<Vec<_>>(),
                vec![520, 530, 522, 540, 510, 511]
            );
        });
    }
}
