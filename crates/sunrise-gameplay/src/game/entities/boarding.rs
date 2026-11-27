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
}
