use super::{
    BodyDigest, EntityGroups, Group, hash_guest, optional_tag, player_body, portal_values,
    random_words, respawn, rotation, stack, tag, typed_shared, vec_bits, vec3,
};
use crate::game::{
    ServerGame,
    server_side::nether_portal::{Plan, TransferredArrivalState},
};
use std::hash::{DefaultHasher, Hash};
use sunrise_core::{geometry::Vec3, nbt::Tag, random::LegacyRandom};
use sunrise_gameplay_common::game::{
    end_state::{Arriving, GatewayJob},
    entity::Entity,
    entity_memory::{OffLevel, RemovedVehicle},
};
use sunrise_items::items::Items;
use sunrise_sim::join_seq::JoinSeq;

fn ordered<K: Copy + Ord, V>(entries: impl Iterator<Item = (K, V)>) -> Vec<(K, V)> {
    let mut sorted: Vec<_> = entries.collect();
    sorted.sort_unstable_by_key(|&(key, _)| key);
    sorted
}

fn sorted<T: Ord>(entries: impl Iterator<Item = T>) -> Vec<T> {
    let mut sorted: Vec<_> = entries.collect();
    sorted.sort_unstable();
    sorted
}

pub(super) fn level(
    game: &ServerGame<'_>,
    shared: &mut EntityGroups,
    items: &Items,
    out: &mut String,
) {
    retained(game, shared, items).append(out, "retained_entities");
    saved(game, shared).append(out, "entity_memory");
    let mut portals = Group::default();
    for (&id, &state) in ordered(game.server.portals.entities.iter()) {
        portal_values(state).hash(portals.row(id, JoinSeq::UNJOINED));
    }
    portals.append(out, "entity_portals");
}

fn entity(h: &mut DefaultHasher, e: &impl Entity, random: &LegacyRandom) {
    e.native_type().hash(h);
    vec3(h, e.position());
    vec3(h, e.velocity());
    (rotation(e.rotation()), e.is_removed(), random_words(random)).hash(h);
}

fn retained(game: &ServerGame<'_>, shared: &mut EntityGroups, items: &Items) -> Group {
    let p = &game.common.persist;
    let mut group = Group::default();
    p.off_level.len().hash(&mut group.hash);
    for (&id, body) in ordered(p.off_level.iter()) {
        let h = group.row(id, JoinSeq::UNJOINED);
        std::mem::discriminant(body).hash(h);
        match body {
            OffLevel::Mob(mob) => {
                entity(h, mob, &mob.random);
                (
                    mob.health.to_bits(),
                    mob.extra.mount,
                    mob.extra.vehicle_order,
                )
                    .hash(h);
                mob.hash_body(h, items);
                typed_shared(game, shared, mob);
            }
            OffLevel::Boat(boat) => {
                entity(h, boat, &boat.random);
                boat.hash_body(h, items);
                typed_shared(game, shared, boat);
            }
            OffLevel::Cart(cart) => {
                entity(h, cart, &cart.random);
                cart.damage.to_bits().hash(h);
                cart.hash_body(h, items);
                typed_shared(game, shared, cart);
            }
        }
    }
    p.removed_vehicles.len().hash(&mut group.hash);
    for (&id, body) in ordered(p.removed_vehicles.iter()) {
        let h = group.row(id, JoinSeq::UNJOINED);
        std::mem::discriminant(body).hash(h);
        match body {
            RemovedVehicle::Mob { position, kind } => {
                vec3(h, *position);
                kind.hash(h);
            }
            RemovedVehicle::Boat(boat) => {
                entity(h, boat, &boat.random);
                boat.hash_body(h, items);
                typed_shared(game, shared, boat);
            }
            RemovedVehicle::Cart(cart) => {
                entity(h, cart, &cart.random);
                cart.damage.to_bits().hash(h);
                cart.hash_body(h, items);
                typed_shared(game, shared, cart);
            }
        }
    }
    group
}

fn saved(game: &ServerGame<'_>, shared: &EntityGroups) -> Group {
    let p = &game.common.persist;
    let mut group = Group::default();
    let h = &mut group.hash;
    sorted(p.transient.iter()).hash(h);
    ordered(p.no_physics.iter()).hash(h);
    ordered(p.forced_meta.iter()).hash(h);
    p.removals.hash(h);
    sorted(p.restored.iter()).hash(h);
    sorted(p.sync_body_position.iter()).hash(h);
    sorted(p.score_destroyed.iter()).hash(h);
    p.dragon_death_canceled
        .as_ref()
        .map(|ids| sorted(ids.iter()))
        .hash(h);
    sorted(p.hidden_entity_sections.iter()).hash(h);
    (p.unreadable_entity_sections, p.entity_metadata_on_disk).hash(h);

    let uuids = ordered(
        p.uuids
            .iter()
            .filter(|(id, _)| !shared.visited.contains(*id)),
    );
    uuids.hash(h);
    let raw = ordered(p.raw.iter().filter(|(id, _)| !shared.visited.contains(*id)));
    raw.len().hash(h);
    for (id, value) in raw {
        id.hash(h);
        tag(h, value);
    }
    let unknown = ordered(p.unknown.iter());
    unknown.len().hash(h);
    for (chunk, values) in unknown {
        chunk.hash(h);
        values.len().hash(h);
        for value in values {
            tag(h, value);
        }
    }
    let anonymous = ordered(p.anon.iter());
    anonymous.len().hash(h);
    for (key, (at, value)) in anonymous {
        key.hash(h);
        vec3(h, *at);
        tag(h, value);
    }
    let colliders = sorted(game.common.entity_attachments.colliders_in(game.common.dim));
    colliders.len().hash(h);
    for id in colliders {
        id.hash(h);
        let collider = game
            .common
            .entity_attachments
            .collider(id)
            .map(|c| Tag::Compound(c.to_nbt()));
        optional_tag(h, collider.as_ref());
    }
    group.count = p.unknown.len() + p.anon.len();
    group
}

pub(super) fn server(game: &ServerGame<'_>, items: &Items, out: &mut String) {
    if !game.server.level_pass.last {
        out.push_str("server_entity_state=- server_portal_travel=- server_player_transit=- ");
        return;
    }
    cross_level_entities(game).append(out, "server_entity_state");
    travel(game, items).append(out, "server_portal_travel");
    player_transit(game, items).append(out, "server_player_transit");
}

fn cross_level_entities(game: &ServerGame<'_>) -> Group {
    let server = &game.server;
    let mut group = Group::default();
    let h = &mut group.hash;
    let credits = ordered(server.last_hurt_by_player.iter());
    credits.len().hash(h);
    for (id, credit) in credits {
        (id, credit.0).hash(h);
    }
    let killers = ordered(server.lead_killers.iter());
    killers.len().hash(h);
    for (id, (uuid, dim)) in killers {
        (id, uuid, dim.id()).hash(h);
    }
    ordered(server.love_cause.iter()).hash(h);
    sorted(server.died.iter()).hash(h);
    let attachments = ordered(game.common.entity_attachments.iter());
    attachments.len().hash(h);
    group.count = attachments.len();
    for (id, data) in attachments {
        (id, data.is_empty(), data.has_live_persistence()).hash(h);
        tag(h, &Tag::Compound(data.to_staged_nbt()));
    }
    group
}

fn plan(h: &mut DefaultHasher, p: &Plan) {
    (
        p.dim.id(),
        p.approximate,
        p.entry_x_axis,
        p.width.to_bits(),
        p.height.to_bits(),
        p.end,
        p.from_end,
    )
        .hash(h);
    vec3(h, p.offset);
}

fn vec_map<'a>(h: &mut DefaultHasher, entries: impl Iterator<Item = (&'a u32, &'a Vec3)>) {
    let entries = ordered(entries);
    entries.len().hash(h);
    for (id, v) in entries {
        id.hash(h);
        vec3(h, *v);
    }
}

fn gateway_job(h: &mut DefaultHasher, job: GatewayJob) {
    std::mem::discriminant(&job).hash(h);
    match job {
        GatewayJob::Find { gateway, exact } => (gateway, exact).hash(h),
        GatewayJob::Known { exit, exact } => (exit, exact).hash(h),
    }
}

fn travel(game: &ServerGame<'_>, items: &Items) -> Group {
    let portals = &game.server.portals;
    let end = &game.common.end;
    let mut group = Group::default();
    let h = &mut group.hash;
    portals.arrivals.len().hash(h);
    for arrival in &portals.arrivals {
        plan(h, &arrival.plan);
        tag(h, &arrival.tag);
        arrival.tnt_used_portal.hash(h);
        match &arrival.transferred {
            None => 0u8.hash(h),
            Some(TransferredArrivalState::Entity(row)) => {
                1u8.hash(h);
                (row.initialized, row.died, row.transient, row.source).hash(h);
                row.attachments.is_some().hash(h);
                if let Some(data) = &row.attachments {
                    tag(h, &Tag::Compound(data.to_staged_nbt()));
                }
                row.accepted_hit.is_some().hash(h);
                if let Some(hit) = &row.accepted_hit {
                    let source = hit.source();
                    (
                        source.damage_type,
                        source.causing,
                        source.direct,
                        source.direct_type,
                        source.position.map(vec_bits),
                    )
                        .hash(h);
                    stack(h, hit.weapon.as_ref(), items);
                }
            }
            Some(TransferredArrivalState::VehicleTree(tree)) => {
                (2u8, &tree.name, tree.is_transient()).hash(h);
            }
        }
    }
    portals.carrying.len().hash(h);
    for (dim, id, entry) in &portals.carrying {
        (dim.id(), id, entry).hash(h);
    }
    vec_map(h, portals.before.iter());
    vec_map(h, portals.carried_motion.iter());
    vec_map(h, portals.carried_at.iter());
    ordered(portals.carried_cooldown.iter()).hash(h);

    let travellers = ordered(end.gateways.iter());
    travellers.len().hash(h);
    for (id, traveller) in travellers {
        // Traveller's debug view exposes its scalar portal process without advancing it.
        (id, format!("{traveller:?}")).hash(h);
    }
    end.arrivals.len().hash(h);
    for arrival in &end.arrivals {
        gateway_job(h, arrival.job);
        vec3(h, arrival.target);
        std::mem::discriminant(&arrival.traveller).hash(h);
        match &arrival.traveller {
            Arriving::Live(id) => id.hash(h),
            Arriving::Pearl(pearl) => {
                pearl.id.hash(h);
                entity(h, pearl.as_ref(), &pearl.random.0);
                (pearl.owner_uuid, pearl.uuid).hash(h);
            }
        }
    }
    (end.gateway_cooldown, &end.areas, &end.tickets).hash(h);

    let trips = &game.server.server_trips;
    trips.len().hash(h);
    for trip in trips {
        (
            trip.conn,
            trip.dim.id(),
            trip.source.id(),
            trip.vehicle,
            trip.world_spawn,
            trip.left.map(|dim| dim.id()),
            trip.spawn_search.as_ref().map(|s| s.waiting_for),
            trip.finder.is_some(),
        )
            .hash(h);
        vec3(h, trip.target);
        respawn(h, trip.respawn.as_ref());
    }
    let teleports = &game.server.level_teleports;
    teleports.len().hash(h);
    for teleport in teleports {
        (
            teleport.conn,
            teleport.dim.id(),
            vec_bits(teleport.at),
            teleport.rotation.map(rotation),
            teleport.vehicle,
        )
            .hash(h);
    }
    group.count = portals.arrivals.len() + end.arrivals.len() + trips.len() + teleports.len();
    group
}

fn player_transit(game: &ServerGame<'_>, items: &Items) -> Group {
    let mut group = Group::default();
    let guests = game
        .server
        .lan
        .iter()
        .flat_map(|host| host.hub.guests.iter())
        .filter(|g| !g.placed)
        .map(|g| (g.conn, g));
    for (_, guest) in ordered(guests) {
        let h = group.row(guest.entity_id as u32, JoinSeq::UNJOINED);
        (
            &guest.dimension,
            guest.held,
            guest.released,
            guest.respawning,
            guest.awaiting_teleport.map(|(id, at)| (id, vec_bits(at))),
        )
            .hash(h);
        optional_tag(h, guest.saved.as_ref());
        hash_guest(game, h, guest, items, None);
    }
    let displaced = &game.server.lan_displaced_players.0;
    displaced.len().hash(&mut group.hash);
    for player in displaced {
        let h = group.row(player.record_conn, JoinSeq::UNJOINED);
        (
            &player.name,
            player.uuid,
            player.fake,
            std::mem::discriminant(&player.mode),
            player.dimension.id(),
            player.input_sneak,
            player.selected,
            player.riding,
            rotation([player.yaw, player.pitch]),
            player.experience.level,
            player.experience.progress.to_bits(),
            player.experience.total,
        )
            .hash(h);
        player.stats.values.hash(h);
        player.body.is_some().hash(h);
        if let Some(body) = &player.body {
            (
                body.entity_id,
                body.health.health().to_bits(),
                body.health.absorption().to_bits(),
                body.health.food.food,
                body.health.food.saturation.to_bits(),
            )
                .hash(h);
            vec3(h, body.position);
            vec3(h, body.velocity);
            player_body(h, body);
        }
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn memory_maps_are_ordered_by_the_saved_key() {
        let a = HashMap::from([(3, "hidden"), (1, "resident"), (2, "retained")]);
        let b = HashMap::from([(2, "retained"), (3, "hidden"), (1, "resident")]);
        assert_eq!(ordered(a.iter()), ordered(b.iter()));
        assert_eq!(
            ordered(a.into_iter()),
            [(1, "resident"), (2, "retained"), (3, "hidden")]
        );
    }
}
