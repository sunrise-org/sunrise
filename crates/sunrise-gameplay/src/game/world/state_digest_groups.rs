use super::{random_words, relative, vec3};
use crate::game::ServerGame;
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    hash::{DefaultHasher, Hash, Hasher},
    mem::discriminant,
};
use sunrise_core::nbt::{JavaString, Tag, numeric_int};
use sunrise_gameplay_common::game::{
    FallingBlock, boats_common::Boat, commands_common::Target, dragon_common::dragon_part_id,
    entity::Entity, fireworks::Rocket, fishing_common::State, general::GeneralRow,
    minecarts::Minecart, native_entity_base::EntityBase, tnt_common::PrimedTnt,
};
use sunrise_items::{
    equipment::Equipment,
    item_nbt::item_to_nbt,
    items::{ItemStack, Items},
};
use sunrise_level::effects::{EffectInstance, PlayerEffects};
use sunrise_sim::{
    entities::{ArmorStand, ExperienceOrb, ItemEntity},
    join_seq::{JoinSeq, Joined},
    mobs::{Mob, leash::Holder},
    motion::PlayerMotion,
    nether_portal::PortalState,
    pose::SizeAnswer,
    projectiles::{Projectile, ProjectileKind},
    text_display_data::resident::{DisplayContents, DisplayState},
    thrown_trident::ThrownTrident,
};

#[derive(Default)]
pub(super) struct EntityGroups {
    pub(super) identity: DefaultHasher,
    pub(super) shared: DefaultHasher,
    visited: BTreeSet<u32>,
}

#[path = "state_digest_bodies.rs"]
mod bodies;

#[path = "state_digest_memory.rs"]
mod memory;

pub(super) use bodies::BodyDigest;

fn tag(h: &mut impl Hasher, value: &Tag) {
    match value {
        Tag::Compound(fields) => {
            value.id().hash(h);
            let mut entries: Vec<_> = fields.iter().collect();
            entries.sort_unstable_by_key(|&(key, _)| key);
            entries.len().hash(h);
            for (key, value) in entries {
                key.hash(h);
                tag(h, value);
            }
        }
        Tag::List(values) => {
            value.id().hash(h);
            values.len().hash(h);
            for value in values {
                tag(h, value);
            }
        }
        value => value.hash(h),
    }
}

fn optional_tag(h: &mut impl Hasher, value: Option<&Tag>) {
    value.is_some().hash(h);
    if let Some(value) = value {
        tag(h, value);
    }
}

fn stack(h: &mut impl Hasher, value: Option<&ItemStack>, items: &Items) {
    value.is_some().hash(h);
    if let Some(value) = value {
        tag(h, &item_to_nbt(value, items));
    }
}

fn vec_bits<const N: usize>(v: [f64; N]) -> [u64; N] {
    v.map(f64::to_bits)
}

fn rotation(v: [f32; 2]) -> [u32; 2] {
    v.map(f32::to_bits)
}

#[derive(Default)]
pub(super) struct Shared<'a> {
    uuid: Option<[i32; 4]>,
    fire: Option<i32>,
    air: Option<i32>,
    portal: Option<i32>,
    active_portal: Option<PortalState>,
    base: Option<EntityBase>,
    no_gravity: Option<bool>,
    silent: Option<bool>,
    no_physics: Option<bool>,
    invisible: Option<bool>,
    name_visible: Option<bool>,
    visual_fire: Option<bool>,
    invulnerable_time: Option<i32>,
    ticks_frozen: Option<i32>,
    fall_distance: Option<f64>,
    size: Option<SizeAnswer>,
    ground: Option<bool>,
    collisions: [Option<bool>; 4],
    name: Option<&'a Tag>,
    text: Option<&'a sunrise_game_data::text::Text>,
    literal_name: Option<&'a str>,
}

pub(super) trait SharedParts: Entity {
    fn target(&self) -> Target;
    fn shared(&self, _game: &ServerGame<'_>) -> Shared<'_> {
        Shared::default()
    }
}

macro_rules! shared_parts {
    ($ty:ty, $kind:ident $(, |$e:ident, $game:ident| $shared:expr)?) => {
        impl SharedParts for $ty {
            fn target(&self) -> Target {
                Target::$kind(self.id)
            }
            $(
                fn shared(&self, $game: &ServerGame<'_>) -> Shared<'_> {
                    let $e = self;
                    $shared
                }
            )?
        }
    };
}

shared_parts!(Mob, Mob, |e, _game| Shared {
    fire: Some(e.fire_ticks),
    air: Some(e.air),
    base: Some(EntityBase {
        fire_ticks: e.fire_ticks,
        invulnerable: e.invulnerable_flag,
        no_gravity: e.no_gravity,
        silent: e.silent,
        glowing: e.glowing,
    }),
    invisible: Some(e.invisible),
    name_visible: Some(e.name_visible),
    invulnerable_time: Some(e.invulnerable_time),
    ticks_frozen: Some(e.ticks_frozen),
    collisions: [None, None, Some(e.vertical_collision_below), None],
    text: e.custom_name_text.as_ref(),
    literal_name: e.custom_name.as_deref(),
    ..Shared::default()
});
shared_parts!(ItemEntity, Item, |e, _game| Shared {
    fire: Some(e.fire_ticks),
    collisions: [None, Some(e.vertical_collision), None, None],
    ..Shared::default()
});
shared_parts!(Boat, Boat, |e, _game| Shared {
    fire: Some(e.fire_ticks),
    ticks_frozen: Some(e.ticks_frozen),
    ..Shared::default()
});
shared_parts!(Minecart, Cart, |e, _game| Shared {
    ticks_frozen: Some(e.ticks_frozen),
    ..Shared::default()
});
shared_parts!(ArmorStand, Stand, |e, _game| Shared {
    fire: Some(e.fire_ticks),
    air: Some(e.air),
    text: e.custom_name.as_ref(),
    invisible: Some(e.invisible),
    name_visible: Some(e.name_visible),
    ..Shared::default()
});
shared_parts!(PrimedTnt, Tnt);
shared_parts!(FallingBlock, Falling, |e, _game| Shared {
    uuid: e.saved.as_ref().map(|s| s.0),
    fire: Some(e.fire_ticks),
    active_portal: Some(e.portal),
    ..Shared::default()
});
shared_parts!(ExperienceOrb, Orb, |e, _game| Shared {
    fire: Some(e.fire_ticks),
    air: Some(e.air),
    ..Shared::default()
});
shared_parts!(Projectile, Projectile, |e, game| Shared {
    uuid: e.uuid,
    fire: Some(e.arrow.as_ref().map_or(e.fire_ticks, |a| a.fire_ticks)),
    air: Some(e.air_supply),
    visual_fire: Some(e.has_visual_fire),
    portal: matches!(e.kind, ProjectileKind::EnderPearl).then(|| game.pearl_portal_cooldown(e.id)),
    ..Shared::default()
});
shared_parts!(Rocket, Rocket, |e, _game| Shared {
    silent: Some(e.silent),
    ..Shared::default()
});
shared_parts!(ThrownTrident, Trident, |e, _game| Shared {
    fire: Some(e.fire_ticks),
    ..Shared::default()
});
shared_parts!(GeneralRow, General, |e, _game| Shared {
    uuid: Some(e.uuid),
    fire: Some(e.base.fire_ticks),
    base: Some(e.base),
    name_visible: Some(e.custom_name_visible),
    ground: Some(e.on_ground),
    collisions: [
        Some(e.horizontal_collision),
        Some(e.vertical_collision),
        Some(e.vertical_collision_below),
        None,
    ],
    text: e.custom_name.as_ref(),
    ..Shared::default()
});

pub(super) fn typed_shared(game: &ServerGame<'_>, groups: &mut EntityGroups, e: &impl SharedParts) {
    let mut live = e.shared(game);
    let ground = e.ground_state();
    live.ground = live.ground.or_else(|| ground.map(|g| g.on_ground));
    live.collisions[0] = live.collisions[0].or_else(|| ground.map(|g| g.horizontal_collision));
    live.fall_distance = Some(e.fall_distance());
    live.size = e.size_answer();
    live.no_gravity = Some(
        live.base
            .map_or_else(|| e.is_no_gravity(), |b| b.no_gravity),
    );
    live.no_physics = Some(e.no_physics(game.common.persist.no_physics.get(&e.id()).copied()));
    hash_support(&mut groups.shared, ground);
    entity_shared(game, groups, e.id(), Some(e.target()), live);
}

fn entity_shared(
    game: &ServerGame<'_>,
    groups: &mut EntityGroups,
    id: u32,
    target: Option<Target>,
    live: Shared<'_>,
) {
    let uuid = live
        .uuid
        .or_else(|| game.common.persist.uuid_of(id))
        .or_else(|| {
            let Tag::IntArray(words) = game.saved_entity_compound(target?)?.get("UUID")? else {
                return None;
            };
            words.as_slice().try_into().ok()
        });
    groups.visited.insert(id);
    (id, uuid).hash(&mut groups.identity);
    entity_values(game, &mut groups.shared, id, target, live);
}

fn entity_values(
    game: &ServerGame<'_>,
    h: &mut DefaultHasher,
    id: u32,
    target: Option<Target>,
    live: Shared<'_>,
) {
    let raw = target.and_then(|t| game.saved_entity_compound(t));
    let number = |key| raw.and_then(|tag| tag.get(key)).and_then(numeric_int);
    id.hash(h);
    live.fire.or_else(|| number("Fire")).hash(h);
    live.air.or_else(|| number("Air")).hash(h);
    let portal = live
        .active_portal
        .or_else(|| game.server.portals.entities.get(&id).copied());
    live.portal
        .or_else(|| portal.map(|p| p.cooldown))
        .or_else(|| number("PortalCooldown"))
        .unwrap_or(0)
        .hash(h);
    portal.and_then(|p| p.process).map(process_values).hash(h);
    let base = live.base.unwrap_or_else(|| EntityBase::read(raw));
    (
        base.invulnerable,
        live.no_gravity.unwrap_or(base.no_gravity),
        live.silent.unwrap_or(base.silent),
        base.glowing,
    )
        .hash(h);
    (
        live.invisible,
        live.name_visible
            .or_else(|| number("CustomNameVisible").map(|n| n != 0)),
        live.visual_fire
            .or_else(|| number("HasVisualFire").map(|n| n != 0)),
        live.invulnerable_time
            .or_else(|| number("InvulnerableTime")),
        live.ticks_frozen.or_else(|| number("TicksFrozen")),
        live.no_physics
            .or_else(|| game.common.persist.no_physics.get(&id).copied()),
    )
        .hash(h);
    live.fall_distance
        .map(f64::to_bits)
        .or_else(|| {
            sunrise_sim::save::float(raw.and_then(|tag| tag.get("fall_distance"))).map(f64::to_bits)
        })
        .hash(h);
    size_values(live.size).hash(h);
    live.ground
        .or_else(|| number("OnGround").map(|n| n != 0))
        .hash(h);
    live.collisions.hash(h);
    if let Some(target) = target {
        let tags: Vec<_> = game.saved_entity_tags(target).iter_utf16().collect();
        tags.hash(h);
    } else {
        0usize.hash(h);
    }
    let text_tag = live
        .text
        .map(|text| sunrise_game_data::scoreboard::json_to_tag(&text.component_view()));
    let literal_tag = live.literal_name.map(|name| Tag::String(name.into()));
    optional_tag(
        h,
        live.name
            .or(text_tag.as_ref())
            .or(literal_tag.as_ref())
            .or_else(|| raw.and_then(|tag| tag.get("CustomName"))),
    );
}

fn size_values(
    size: Option<SizeAnswer>,
) -> Option<(
    std::mem::Discriminant<mod_api::Pose>,
    [u32; 3],
    Option<[u32; 3]>,
)> {
    size.map(|s| {
        (
            discriminant(&s.pose),
            s.vanilla.map(f32::to_bits),
            s.size.map(|size| size.map(f32::to_bits)),
        )
    })
}

fn hash_support(
    h: &mut DefaultHasher,
    ground: Option<&sunrise_world::ground_support::GroundState>,
) {
    ground
        .map(|g| (g.on_ground_no_blocks, g.supporting_block))
        .hash(h);
}

fn process_values(
    p: sunrise_sim::nether_portal::Process,
) -> (sunrise_core::position::BlockPos, i32, bool) {
    (p.entry, p.time, p.inside)
}

fn portal_values(p: PortalState) -> (i32, Option<(sunrise_core::position::BlockPos, i32, bool)>) {
    (p.cooldown, p.process.map(process_values))
}

fn text(h: &mut DefaultHasher, text: Option<&sunrise_game_data::text::Text>) {
    let tag = text.map(|text| sunrise_game_data::scoreboard::json_to_tag(&text.component_view()));
    optional_tag(h, tag.as_ref());
}

#[derive(Default)]
struct Group {
    hash: DefaultHasher,
    count: usize,
}

impl Group {
    fn row(&mut self, id: u32, join: JoinSeq) -> &mut DefaultHasher {
        self.count += 1;
        (id, join.is_joined().then(|| relative(join.get()))).hash(&mut self.hash);
        &mut self.hash
    }

    fn append(self, out: &mut String, name: &str) {
        write!(out, "{name}={}:{:016x} ", self.count, self.hash.finish()).unwrap();
    }
}

pub(super) fn level(
    game: &ServerGame<'_>,
    shared: &mut EntityGroups,
    details: Option<&mut String>,
) -> String {
    let mut out = String::new();
    let items = game.common.res.items();
    hangings(game, shared, items).append(&mut out, "hangings");
    decorations(game, shared, items, &mut out);
    transient(game, shared, items, &mut out);
    end_entities(game, shared, items, &mut out);
    server_projectiles(game, items, &mut out);
    seats(game).append(&mut out, "seats");
    players(game, items, details).append(&mut out, "players");
    memory::level(game, shared, items, &mut out);
    memory::server(game, items, &mut out);
    out.pop();
    out
}

fn hangings(game: &ServerGame<'_>, shared: &mut EntityGroups, items: &Items) -> Group {
    let mut group = Group::default();
    for e in &game.common.paintings {
        let h = group.row(e.id, e.join);
        "painting".hash(h);
        (
            e.pos,
            e.facing,
            &e.variant,
            &e.asset,
            e.width,
            e.height,
            e.check_interval,
            e.ticks,
        )
            .hash(h);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Painting(e.id)),
            Shared::default(),
        );
    }
    for e in &game.common.item_frames {
        let h = group.row(e.id, e.join);
        "item_frame".hash(h);
        (
            e.pos,
            e.facing,
            e.glow,
            e.rotation,
            e.drop_chance.to_bits(),
            e.fixed,
            e.invisible,
            e.check_interval,
            e.ticks,
            random_words(&e.random),
        )
            .hash(h);
        stack(h, e.item.as_ref(), items);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Frame(e.id)),
            Shared {
                name: e.custom_name.as_ref(),
                invisible: Some(e.invisible),
                ..Shared::default()
            },
        );
    }
    group
}

fn decorations(game: &ServerGame<'_>, shared: &mut EntityGroups, items: &Items, out: &mut String) {
    let mut cushions = Group::default();
    for e in &game.common.cushions {
        let h = cushions.row(e.id, e.join);
        vec3(h, e.position);
        (e.pos, e.yaw.to_bits(), e.color, e.check_interval).hash(h);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Cushion(e.id)),
            Shared {
                name: e.custom_name.as_ref(),
                ..Shared::default()
            },
        );
    }
    cushions.append(out, "cushions");

    let mut others = Group::default();
    for e in game.server.others.0.iter() {
        let h = others.row(e.id, e.join);
        e.kind.hash(h);
        vec3(h, e.position);
        (rotation([e.yaw, e.pitch]), e.knot_check).hash(h);
        if let Some(Tag::Compound(raw)) = game.common.persist.raw.get(&e.id) {
            let mut payload: Vec<_> = raw
                .iter()
                .filter(|(key, value)| !other_shared_key(key, value))
                .collect();
            payload.sort_unstable_by_key(|&(key, _)| key);
            payload.len().hash(h);
            for (key, value) in payload {
                key.hash(h);
                tag(h, value);
            }
        } else {
            0usize.hash(h);
        }
        e.display.is_some().hash(h);
        if let Some(display) = e.display.as_deref() {
            hash_display(h, display, items);
        }
        e.interaction
            .map(|i| {
                (
                    i.width.to_bits(),
                    i.height.to_bits(),
                    i.response,
                    i.attack.map(|a| (a.player, a.timestamp)),
                    i.interaction.map(|a| (a.player, a.timestamp)),
                )
            })
            .hash(h);
        e.eye.is_some().hash(h);
        if let Some(eye) = e.eye {
            vec3(h, eye.motion);
            (eye.life, eye.synced_fire, eye.has_visual_fire).hash(h);
        }
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Other(e.id)),
            Shared {
                fire: e.eye.map(|eye| eye.fire_ticks),
                size: e.size_answer,
                visual_fire: e.eye.map(|eye| eye.has_visual_fire),
                ..Shared::default()
            },
        );
    }
    others.append(out, "other_entities");

    let mut knots = Group::default();
    for e in &game.common.resident_entities.knots {
        let h = knots.row(e.id, e.join);
        (e.fence, e.invulnerable, e.no_gravity, e.silent, e.glowing).hash(h);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Knot(e.id)),
            Shared {
                fire: Some(e.fire_ticks),
                ..Shared::default()
            },
        );
    }
    let mut fence_ids: Vec<_> = game.server.knot_ids.iter().collect();
    fence_ids.sort_unstable();
    fence_ids.hash(&mut knots.hash);
    game.server.broken_knots.hash(&mut knots.hash);
    for e in game
        .common
        .mobs
        .iter()
        .filter(|e| e.extra.leash.is_some() || e.extra.leash_pending.is_some())
    {
        (0u8, e.id, e.extra.leash_player, e.extra.leash_knot_id).hash(&mut knots.hash);
        hash_leash(&mut knots.hash, e.extra.leash, e.extra.leash_pending);
    }
    for e in game
        .common
        .boats
        .iter()
        .filter(|e| e.leash.is_some() || e.leash_pending.is_some())
    {
        (1u8, e.id, e.leash_player, e.leash_knot_id).hash(&mut knots.hash);
        hash_leash(&mut knots.hash, e.leash, e.leash_pending);
    }
    knots.append(out, "leash_knots");
}

fn other_shared_key(key: &JavaString, value: &Tag) -> bool {
    let named = |name: &str| key.0.iter().copied().eq(name.encode_utf16());
    if named("UUID") {
        matches!(value, Tag::IntArray(words) if words.len() == 4)
    } else if named("Tags") {
        matches!(value, Tag::List(values) if values.iter().all(|v| matches!(v, Tag::String(_))))
    } else if named("CustomName") {
        true
    } else {
        ["Fire", "Air", "OnGround", "PortalCooldown"]
            .into_iter()
            .any(named)
            && numeric_int(value).is_some()
    }
}

fn hash_leash(
    h: &mut DefaultHasher,
    holder: Option<Holder>,
    pending: Option<sunrise_sim::mobs::leash::Pending>,
) {
    use sunrise_sim::mobs::leash::Pending;
    discriminant(&holder).hash(h);
    if let Some(holder) = holder {
        discriminant(&holder).hash(h);
        match holder {
            Holder::Player => {}
            Holder::Mob(id) | Holder::Boat(id) => id.hash(h),
            Holder::Knot(at) => at.hash(h),
        }
    }
    discriminant(&pending).hash(h);
    if let Some(pending) = pending {
        discriminant(&pending).hash(h);
        match pending {
            Pending::Uuid(uuid) => uuid.hash(h),
            Pending::Knot(at) => at.hash(h),
        }
    }
}

fn hash_display(h: &mut DefaultHasher, d: &DisplayState, items: &Items) {
    let t = d.transformation;
    for vector in [
        t.translation.as_slice(),
        &t.left_rotation,
        &t.scale,
        &t.right_rotation,
    ] {
        for value in vector {
            value.to_bits().hash(h);
        }
    }
    (
        d.billboard.name(),
        d.brightness,
        [
            d.view_range,
            d.shadow_radius,
            d.shadow_strength,
            d.width,
            d.height,
        ]
        .map(f32::to_bits),
        d.glow_color_override,
        d.interpolation_duration,
        d.start_interpolation,
        d.teleport_duration,
    )
        .hash(h);
    discriminant(&d.contents).hash(h);
    match &d.contents {
        DisplayContents::Block(state) => state.0.hash(h),
        DisplayContents::Item {
            stack: item,
            context,
        } => {
            context.name().hash(h);
            stack(h, item.as_deref(), items);
        }
        DisplayContents::Text(text) => {
            tag(h, &text.component.to_nbt());
            (
                text.line_width,
                text.background,
                text.opacity,
                text.shadow,
                text.see_through,
                text.default_background,
                text.alignment.name(),
            )
                .hash(h);
        }
    }
}

fn transient(game: &ServerGame<'_>, shared: &mut EntityGroups, items: &Items, out: &mut String) {
    let mut fangs = Group::default();
    for e in game.common.fangs.list.iter() {
        let h = fangs.row(e.id, e.join_seq());
        (
            vec_bits(e.pos),
            e.yaw.to_bits(),
            e.warmup,
            e.life,
            e.owner,
            e.started,
            e.synced_fire,
            e.has_visual_fire,
            random_words(&e.random),
        )
            .hash(h);
        vec3(h, e.motion);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Fangs(e.id)),
            Shared {
                fire: Some(e.fire_ticks),
                visual_fire: Some(e.has_visual_fire),
                ..Shared::default()
            },
        );
    }
    fangs.append(out, "fangs");

    let mut lightning = Group::default();
    for e in game.common.bolts.iter() {
        let h = lightning.row(e.id, e.join_seq());
        (
            vec_bits(e.at),
            rotation(e.rotation),
            e.fall_distance.to_bits(),
            e.life,
            e.flashes,
            e.seed,
            e.visual_only,
            e.fires,
            e.cause,
            random_words(&e.random),
        )
            .hash(h);
        vec3(h, e.velocity);
        (
            &e.strike.converted,
            &e.strike.hit,
            &e.strike.hit_items,
            &e.strike.spared,
        )
            .hash(h);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Bolt(e.id)),
            Shared {
                fire: Some(e.fire_ticks),
                portal: Some(e.portal_cooldown),
                air: Some(e.air),
                ground: Some(e.on_ground),
                name: e.custom_name.as_ref(),
                base: Some(EntityBase {
                    fire_ticks: e.fire_ticks,
                    invulnerable: e.invulnerable,
                    no_gravity: e.no_gravity,
                    silent: e.silent,
                    glowing: e.glowing,
                }),
                invulnerable_time: Some(e.invulnerable_time),
                visual_fire: Some(e.has_visual_fire),
                name_visible: Some(e.custom_name_visible),
                ticks_frozen: Some(e.ticks_frozen),
                fall_distance: Some(e.fall_distance),
                ..Shared::default()
            },
        );
    }
    lightning.append(out, "lightning");

    let mut spawners = Group::default();
    for e in game.common.ominous_spawners.iter() {
        let h = spawners.row(e.id, e.join);
        (
            vec_bits(e.position),
            e.spawn_item_after,
            e.tick_count,
            e.synced_fire,
            e.has_visual_fire,
        )
            .hash(h);
        stack(h, Some(&e.item), items);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Spawner(e.id)),
            Shared {
                uuid: Some(e.uuid),
                fire: Some(e.fire_ticks),
                visual_fire: Some(e.has_visual_fire),
                ..Shared::default()
            },
        );
    }
    spawners.append(out, "ominous_spawners");

    let mut hooks = Group::default();
    let mut entries: Vec<_> = game
        .fishing_hooks_here()
        .filter(|&(_, owner)| owner != Target::Player || local_player_here(game))
        .collect();
    entries.sort_unstable_by_key(|(hook, _)| hook.id);
    for (e, _) in entries {
        let h = hooks.row(e.id, e.join);
        vec3(h, e.position);
        vec3(h, e.velocity);
        hash_support(h, Some(&e.ground));
        discriminant(&e.state).hash(h);
        if let State::Hooked(id) = e.state {
            id.hash(h);
        }
        (
            e.caster,
            e.hooked_in,
            e.left_owner,
            rotation([e.yaw, e.pitch]),
            e.tick_count,
            e.life,
            e.nibble,
            e.time_until_lured,
            e.time_until_hooked,
            e.fish_angle.to_bits(),
            e.out_of_water,
            e.open_water,
        )
            .hash(h);
        (e.luck, e.lure, e.last_deflected_by, random_words(&e.random)).hash(h);
        e.medium
            .as_ref()
            .map(|medium| {
                (
                    medium.mod_id,
                    &medium.answered,
                    &medium.loot_table,
                    medium.lure_ticks,
                )
            })
            .hash(h);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Hook(e.id)),
            Shared {
                uuid: Some(e.uuid),
                fire: Some(e.base.fire_ticks),
                base: Some(e.base),
                ground: Some(e.ground.on_ground),
                ..Shared::default()
            },
        );
    }
    hooks.append(out, "fishing_hooks");
}

fn server_projectiles(game: &ServerGame<'_>, items: &Items, out: &mut String) {
    if !game.server.level_pass.last {
        out.push_str("server_thrown_potions=- server_thrown_eyes=- ");
        return;
    }

    let mut potions = Group::default();
    for e in &game.common.potions.thrown {
        let h = potions.row(e.id, JoinSeq::UNJOINED);
        vec3(h, e.position);
        vec3(h, e.velocity);
        (
            e.age,
            e.thrower,
            e.guest,
            e.left_owner,
            e.last_deflected_by,
            random_words(&e.random),
        )
            .hash(h);
        stack(h, Some(&e.stack), items);
        text(h, e.owner.as_ref());
    }
    potions.append(out, "server_thrown_potions");
    let mut eyes = Group::default();
    for e in &game.common.end.eyes {
        let h = eyes.row(e.id, JoinSeq::UNJOINED);
        (
            vec_bits(e.position.to_array()),
            vec_bits(e.velocity.to_array()),
            vec_bits(e.target.to_array()),
            e.life,
            e.survive,
        )
            .hash(h);
    }
    eyes.append(out, "server_thrown_eyes");
}

fn end_entities(game: &ServerGame<'_>, shared: &mut EntityGroups, items: &Items, out: &mut String) {
    let mut crystals = Group::default();
    for e in &game.common.crystals {
        let h = crystals.row(e.id, e.join);
        vec3(h, e.position);
        (
            rotation(e.rotation),
            e.show_bottom,
            e.time,
            e.beam,
            e.invulnerable,
            e.fall.1.to_bits(),
        )
            .hash(h);
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Crystal(e.id)),
            Shared {
                fire: Some(e.fire),
                ground: Some(e.fall.0),
                fall_distance: Some(e.fall.1),
                size: e.size_answer,
                ..Shared::default()
            },
        );
    }
    crystals.append(out, "end_crystals");

    let mut dragon = Group::default();
    for e in game.common.dragon.iter() {
        let h = dragon.row(e.id, e.join);
        vec3(h, e.position);
        vec3(h, e.velocity);
        (
            rotation([e.yaw, e.pitch]),
            e.body_yaw.to_bits(),
            e.head_yaw.to_bits(),
            e.yaw_accel.to_bits(),
            e.health.to_bits(),
            e.hurt_time,
            e.death_time,
            e.death_canceled,
            e.in_wall,
            e.ticks,
            e.no_ai,
            random_words(&e.random),
        )
            .hash(h);
        (
            e.flame_count,
            e.flame_ended,
            e.sitting_damage.to_bits(),
            e.invulnerable,
            e.last_hurt.to_bits(),
            e.target,
            e.aggressive,
            e.left_handed,
            e.persistent,
        )
            .hash(h);
        hash_phase(h, &e.phase);
        bodies::equipment(h, &e.equipment, items);
        bodies::living_attributes(h, &e.live);
        (
            e.hurt_by_player.map(|(who, tick)| (fight_who(who), tick)),
            e.last_source,
            e.last_hurt_mob,
            e.growl,
            e.ambient_time,
            e.nodes,
            e.strafe_request.map(fight_who),
        )
            .hash(h);
        e.nearest_crystal
            .and_then(|i| game.common.crystals.get(i))
            .map(|c| c.id)
            .hash(h);
        for delay in 0..e.history.len() {
            let (height, yaw) = e.sample_lerp(delay, 0.0);
            (height.to_bits(), yaw.to_bits()).hash(h);
        }
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Dragon(e.id)),
            Shared::default(),
        );
        if e.id == 0 {
            continue;
        }
        for (i, part) in e.parts.iter().enumerate() {
            let id = dragon_part_id(e.id, i);
            let h = dragon.row(id, JoinSeq::UNJOINED);
            vec3(h, part.0);
            vec3(h, part.1);
            entity_shared(
                game,
                shared,
                id,
                Some(Target::DragonPart(id)),
                Shared {
                    uuid: Some(part.3),
                    ..Shared::default()
                },
            );
        }
    }
    dragon.append(out, "dragon");

    let mut clouds = Group::default();
    for e in game.common.breath_clouds.iter() {
        let h = clouds.row(e.id, e.join_seq());
        (
            vec_bits(e.at),
            vec_bits(e.motion),
            e.fall_distance.to_bits(),
            e.radius.to_bits(),
            e.first_tick,
            e.age,
            e.duration,
            e.from_dragon,
        )
            .hash(h);
        hash_victims(h, &e.victims);
        e.potion.is_some().hash(h);
        if let Some(potion) = &e.potion {
            let s = &potion.settings;
            (
                s.wait_time,
                s.reapplication_delay,
                s.radius_on_use.to_bits(),
                s.duration_on_use,
                s.duration_scale.to_bits(),
                potion.radius_per_tick.to_bits(),
                potion.color,
                potion.owner,
                potion.guest,
            )
                .hash(h);
            (&potion.effects, &potion.hidden, &potion.ambient).hash(h);
            hash_victims(h, &potion.victims);
            text(h, Some(&potion.source));
            potion.custom_particle.is_some().hash(h);
            if let Some(particle) = &potion.custom_particle {
                particle_options(h, &particle.options, items);
            }
            optional_tag(
                h,
                potion
                    .custom_particle
                    .as_ref()
                    .and_then(|p| p.vanilla_tag.as_ref()),
            );
        }
        entity_shared(
            game,
            shared,
            e.id,
            Some(Target::Cloud(e.id)),
            Shared {
                fire: Some(e.fire_ticks),
                fall_distance: Some(e.fall_distance),
                ..Shared::default()
            },
        );
    }
    clouds.append(out, "breath_clouds");
}

fn fight_who(who: sunrise_gameplay_common::game::dragon_common::FightWho) -> Option<u32> {
    use sunrise_gameplay_common::game::dragon_common::FightWho;
    match who {
        FightWho::Host => None,
        FightWho::Guest(conn) => Some(conn),
    }
}

fn particle_options(
    h: &mut DefaultHasher,
    p: &sunrise_level::particle_options::ParticleOptions,
    items: &Items,
) {
    use mod_api::client::particles::PositionSource;
    (
        discriminant(&p.ty),
        &p.block,
        p.color.map(f32::to_bits),
        p.scale.to_bits(),
        p.to_color.map(f32::to_bits),
        p.trail.map(|(at, ticks)| (vec_bits(at), ticks)),
        &p.item,
        p.geyser
            .map(|(blocks, impulse)| (blocks, impulse.to_bits())),
        &p.mod_particle,
        p.typed_particle
            .as_ref()
            .map(|(typed, limit)| (typed.key(), limit)),
        &p.source,
    )
        .hash(h);
    stack(h, p.item_stack.as_ref(), items);
    p.vibration.is_some().hash(h);
    if let Some((listener, at, ticks)) = &p.vibration {
        (discriminant(listener), vec_bits(*at), ticks).hash(h);
        match listener {
            PositionSource::Block(at) => at.hash(h),
            PositionSource::Entity { id, y_offset } => (id, y_offset.to_bits()).hash(h),
            _ => (),
        }
    }
    p.polytone_seed.is_some().hash(h);
    if let Some(seed) = &p.polytone_seed {
        for map in [&seed.fields, &seed.customs] {
            map.len().hash(h);
            for (name, value) in map {
                (name, value.to_bits()).hash(h);
            }
        }
    }
}

fn hash_victims(
    h: &mut DefaultHasher,
    victims: &[(sunrise_gameplay_common::game::potions_common::Victim, i32)],
) {
    use sunrise_gameplay_common::game::{dragon_common::FightWho, potions_common::Victim};
    let mut victims: Vec<_> = victims
        .iter()
        .map(|&(who, until)| {
            let (kind, id) = match who {
                Victim::Mob(id) => (0, id),
                Victim::Player(FightWho::Host) => (1, 0),
                Victim::Player(FightWho::Guest(conn)) => (2, conn),
            };
            (kind, id, until)
        })
        .collect();
    victims.sort_unstable();
    victims.hash(h);
}

fn hash_phase(h: &mut DefaultHasher, phase: &sunrise_gameplay_common::game::dragon_common::Phase) {
    use sunrise_gameplay_common::game::dragon_common::{FightWho, Phase};
    phase.id().hash(h);
    let (target, path) = match phase {
        Phase::Holding {
            path,
            target,
            clockwise,
        } => {
            clockwise.hash(h);
            (target, path.as_ref())
        }
        Phase::Strafe {
            charge,
            path,
            target,
            clockwise,
            player,
        } => {
            (
                *charge,
                *clockwise,
                player.map(|who| match who {
                    FightWho::Host => None,
                    FightWho::Guest(conn) => Some(conn),
                }),
            )
                .hash(h);
            (target, path.as_ref())
        }
        Phase::Takeoff {
            first,
            path,
            target,
        } => {
            first.hash(h);
            (target, path.as_ref())
        }
        Phase::LandingApproach { path, target } => (target, path.as_ref()),
        Phase::Landing { target } | Phase::Hover { target } => (target, None),
        Phase::Charging { target, since } => {
            since.hash(h);
            (target, None)
        }
        Phase::Dying { target, time } => {
            time.hash(h);
            (target, None)
        }
        Phase::Scanning { time } => return time.hash(h),
        Phase::Attacking { ticks } | Phase::Flaming { ticks } => return ticks.hash(h),
    };
    target.is_some().hash(h);
    if let Some(target) = target {
        vec3(h, *target);
    }
    path.is_some().hash(h);
    if let Some(path) = path {
        (path.0.next, &path.0.nodes).hash(h);
    }
}

fn seats(game: &ServerGame<'_>) -> Group {
    let retained: Vec<_> = game
        .common
        .persist
        .off_level
        .values()
        .filter_map(|e| e.as_mob())
        .collect();
    let retained_ids: BTreeSet<_> = retained.iter().map(|e| e.id).collect();
    let mobs = game
        .common
        .mobs
        .iter()
        .chain(retained)
        .filter(|e| !e.removed)
        .filter_map(|e| Some((e.id, e.extra.mount?)));
    let rows = game
        .common
        .resident_entities
        .general
        .rows
        .iter()
        .filter(|e| {
            !game
                .common
                .persist
                .removals
                .contains(&Target::General(e.id))
        })
        .filter_map(|e| Some((e.id, e.mount?)));
    let bolts = game
        .common
        .bolts
        .iter()
        .filter_map(|e| Some((e.id, e.mount?)));
    let host = game
        .common
        .player
        .iter()
        .filter(|_| local_player_here(game))
        .filter_map(|p| Some((p.entity_id as u32, game.common.riding?)));
    let guests = level_guests(game).filter_map(|p| Some((p.entity_id as u32, p.vehicle?)));
    hash_seats(
        mobs.chain(rows).chain(bolts).chain(host).chain(guests),
        &retained_ids,
        |vehicle| game.seated_passengers(vehicle),
    )
}

fn hash_seats(
    edges: impl IntoIterator<Item = (u32, u32)>,
    retained: &BTreeSet<u32>,
    passengers: impl Fn(u32) -> Vec<(i64, u32, bool)>,
) -> Group {
    let mut edges: Vec<_> = edges.into_iter().collect();
    edges.sort_unstable();
    let vehicles: BTreeSet<_> = edges.iter().map(|&(_, vehicle)| vehicle).collect();
    let mut group = Group::default();
    for (rider, vehicle) in edges {
        vehicle.hash(group.row(rider, JoinSeq::UNJOINED));
    }
    vehicles.len().hash(&mut group.hash);
    for vehicle in vehicles {
        vehicle.hash(&mut group.hash);
        let mut seats = passengers(vehicle);
        seats.sort_by_key(|&(place, _, _)| place);
        let mut unordered: Vec<_> = seats
            .iter()
            .copied()
            .filter(|&(_, rider, player)| !player && retained.contains(&rider))
            .collect();
        unordered.sort_unstable();
        let mut canonical = unordered.into_iter();
        seats.len().hash(&mut group.hash);
        for (_, rider, player) in seats {
            let (rider, player) = if !player && retained.contains(&rider) {
                let (_, rider, player) = canonical.next().expect("retained seat");
                (rider, player)
            } else {
                (rider, player)
            };
            (rider, player).hash(&mut group.hash);
        }
    }
    group
}

fn local_player_here(game: &ServerGame<'_>) -> bool {
    game.common
        .resident_entities
        .local_player
        .location()
        .map_or(game.server.level_pass.last, |location| {
            location.owner.dimension == game.common.dim.key()
        })
}

fn level_guests<'a>(
    game: &'a ServerGame<'_>,
) -> impl Iterator<Item = &'a sunrise_sim::lan::guest::Guest> {
    let dim = game.common.dim.id();
    game.server
        .lan
        .iter()
        .flat_map(|host| host.hub.guests.iter())
        .filter(move |g| g.placed && g.dimension == dim)
}

struct PlayerRow<'a> {
    hash: &'a mut DefaultHasher,
    detail: Option<&'a mut String>,
}

impl<'a> PlayerRow<'a> {
    fn new(
        hash: &'a mut DefaultHasher,
        mut detail: Option<&'a mut String>,
        id: u32,
        name: &str,
    ) -> Self {
        if let Some(out) = &mut detail {
            write!(out, "  p\tid={id}\tname={}", name.escape_debug()).unwrap();
        }
        Self { hash, detail }
    }

    fn finish(self) {
        if let Some(detail) = self.detail {
            detail.push('\n');
        }
    }

    fn component(&mut self, label: &str, hash: impl Fn(&mut DefaultHasher)) {
        hash(self.hash);
        if let Some(detail) = &mut self.detail {
            let mut part = DefaultHasher::new();
            hash(&mut part);
            write!(detail, "\t{label}={:016x}", part.finish()).unwrap();
        }
    }

    fn value(&mut self, label: &str, value: impl Hash + std::fmt::Debug) {
        value.hash(self.hash);
        if let Some(detail) = &mut self.detail {
            write!(detail, "\t{label}={value:?}").unwrap();
        }
    }

    fn statistics(&mut self, values: &sunrise_gameplay_common::stats::StatValues) {
        self.component("statistics", |h| values.hash(h));
        if let Some(detail) = &mut self.detail {
            for (category, stats) in values {
                for (name, value) in stats.iter().filter(|&(_, &value)| value != 0) {
                    write!(
                        detail,
                        "\tstat:{}/{}={value}",
                        category.escape_debug(),
                        name.escape_debug()
                    )
                    .unwrap();
                }
            }
        }
    }
}

fn players(game: &ServerGame<'_>, items: &Items, mut details: Option<&mut String>) -> Group {
    let mut group = Group::default();
    let c = &game.common;
    if let Some(p) = c.player.as_ref().filter(|_| local_player_here(game)) {
        let id = p.entity_id as u32;
        let mut row = PlayerRow::new(
            group.row(id, JoinSeq::UNJOINED),
            details.as_deref_mut(),
            id,
            "local",
        );
        row.value(
            "position",
            (
                p.position.x.to_bits(),
                p.position.y.to_bits(),
                p.position.z.to_bits(),
            ),
        );
        row.value("uuid", game.saved_player_uuid());
        let food = &p.health.food;
        row.value("rotation", rotation([c.yaw, c.pitch]));
        row.value(
            "vitals",
            (
                p.health.health().to_bits(),
                p.health.absorption().to_bits(),
                food.food,
                food.saturation.to_bits(),
            ),
        );
        row.value(
            "experience",
            (
                c.experience.level,
                c.experience.progress.to_bits(),
                c.experience.total,
            ),
        );
        row.value("selected_and_mode", (c.selected, c.mode_id()));
        row.value("vehicle", c.riding);
        row.value("motion", Some(motion_values(p)));
        player_items(
            &mut row,
            (&c.inventory, &c.armor, c.offhand.as_ref(), &c.equipment),
            items,
        );
        row.component("ender_chest", |h| {
            bodies::slots(h, game.server.ender_items.iter(), items)
        });
        row.component("craft_grid", |h| {
            bodies::slots(h, c.craft_grid.iter(), items)
        });
        row.component("carried", |h| stack(h, c.carried.as_ref(), items));
        row.value("score_seed", (c.player_score, c.level.xp_seed));
        row.value("sleep", (c.sleeping, c.sleep_timer));
        row.value("spawn", c.spawn.map(|v| vec_bits([v.x, v.y, v.z])));
        row.value("item_use", (c.using, c.use_duration, c.use_delay));
        row.value(
            "attack_shield_xp_delay",
            (
                game.server.attack_ticks,
                game.server.shield_cooldown,
                game.server.xp_delay,
            ),
        );
        row.component("respawn", |h| {
            respawn(h, game.server.respawn_config.as_ref())
        });
        row.component("cooldowns", |h| cooldowns(h, &c.cooldowns));
        row.statistics(&c.stats.values);
        row.component("body", |h| player_body(h, p));
        effects(&mut row, &c.potions.effects);
        row.component("shared", |h| {
            entity_values(
                game,
                h,
                id,
                Some(Target::Player),
                Shared {
                    fire: Some(p.fire_ticks),
                    air: Some(p.air),
                    active_portal: Some(c.portal_player),
                    invulnerable_time: Some(p.health.invulnerable_time),
                    ticks_frozen: Some(p.ticks_frozen),
                    size: p.size_answer,
                    ground: Some(p.ground.on_ground),
                    collisions: player_collisions(p),
                    ..Shared::default()
                },
            )
        });
        row.finish();
    }
    let mut guests: Vec<_> = level_guests(game).collect();
    guests.sort_unstable_by_key(|p| p.entity_id);
    for p in guests {
        let id = p.entity_id as u32;
        let h = group.row(id, JoinSeq::UNJOINED);
        hash_guest(game, h, p, items, details.as_deref_mut());
    }
    group
}

fn hash_guest(
    game: &ServerGame<'_>,
    h: &mut DefaultHasher,
    p: &sunrise_sim::lan::guest::Guest,
    items: &Items,
    detail: Option<&mut String>,
) {
    let id = p.entity_id as u32;
    let mut row = PlayerRow::new(h, detail, id, &p.name);
    row.value("uuid", p.uuid);
    row.value("position", vec_bits(p.position));
    row.value("rotation", rotation([p.yaw, p.pitch]));
    row.value(
        "vitals",
        (
            p.health.to_bits(),
            p.motion
                .as_deref()
                .map(|m| m.health.absorption().to_bits())
                .or_else(|| p.saved_absorption.map(f32::to_bits)),
            p.food,
            p.saturation.to_bits(),
        ),
    );
    row.value(
        "experience",
        (p.xp_level, p.xp_progress.to_bits(), p.xp_total),
    );
    row.value("selected_and_mode", (p.selected, p.game_mode));
    row.value("vehicle", p.vehicle);
    row.value(
        "motion",
        p.motion.as_deref().map(motion_values).or_else(|| {
            p.loaded_body.map(|b| {
                (
                    vec_bits(b.velocity),
                    b.fall_distance.to_bits(),
                    b.food_exhaustion.to_bits(),
                    b.food_tick_timer,
                )
            })
        }),
    );
    player_items(
        &mut row,
        (&p.inventory, &p.armor, p.offhand.as_ref(), &p.equipment),
        items,
    );
    row.component("ender_chest", |h| {
        bodies::slots(h, p.ender_items.iter(), items)
    });
    row.component("craft_grid", |h| {
        bodies::slots(h, p.craft_grid.iter(), items)
    });
    row.component("carried", |h| stack(h, p.carried.as_ref(), items));
    row.component("craft_result", |h| stack(h, p.craft_result.as_ref(), items));
    row.value("score_seed", (p.score, p.xp_seed));
    row.value("sleep", (p.sleeping, p.bed, p.sleep_timer));
    row.value("spawn", p.spawn.map(vec_bits));
    row.value(
        "movement_flags",
        (
            p.sneaking,
            p.sprinting,
            p.fall_flying,
            p.pose().id(),
            p.dead,
        ),
    );
    row.value(
        "respawn",
        p.respawn_elsewhere
            .as_ref()
            .map(|(dim, at, yaw, pitch, forced)| (dim, at, rotation([*yaw, *pitch]), forced)),
    );
    let a = p.abilities;
    row.value(
        "abilities",
        (
            a.invulnerable,
            a.flying,
            a.may_fly,
            a.instabuild,
            a.may_build,
            rotation([a.flying_speed, a.walking_speed]),
        ),
    );
    effects(&mut row, &p.effects);
    let motion = p.motion.as_deref();
    row.component("body", |h| {
        if let Some(body) = motion {
            player_body(h, body);
        } else {
            optional_tag(h, p.carried_attributes.as_ref());
            p.loaded_body
                .map(|b| (b.ticks_frozen, b.invulnerable, b.invulnerable_time))
                .hash(h);
        }
    });
    let play = game
        .server
        .lan
        .as_ref()
        .and_then(|host| host.play.get(&p.conn));
    row.value("guest_play", play.is_some());
    if let Some(play) = play {
        row.value("item_use", (play.using, play.use_duration, play.use_delay));
        row.value(
            "attack_shield_xp_delay",
            (play.attack_ticks, play.shield_cooldown, play.xp_delay),
        );
        row.value(
            "digging",
            (play.digging, play.dig_start, play.destroy_pos, play.delayed),
        );
        row.component("cooldowns", |h| cooldowns(h, &play.cooldowns));
        row.statistics(&play.stats.values);
        row.component("guest_play_craft_grid", |h| {
            bodies::slots(h, play.craft_grid.iter(), items)
        });
    }
    row.component("shared", |h| {
        entity_values(
            game,
            h,
            id,
            Some(Target::Guest(p.conn)),
            Shared {
                fire: motion
                    .map(|m| m.fire_ticks)
                    .or_else(|| p.loaded_body.map(|b| b.fire_ticks)),
                air: motion
                    .map(|m| m.air)
                    .or_else(|| p.loaded_body.map(|b| b.air)),
                active_portal: Some(play.map_or_else(PortalState::default, |play| play.portal)),
                invulnerable_time: motion.map(|m| m.health.invulnerable_time),
                ticks_frozen: motion.map(|m| m.ticks_frozen),
                size: motion.and_then(|m| m.size_answer),
                ground: Some(p.on_ground),
                collisions: motion.map_or([None; 4], player_collisions),
                ..Shared::default()
            },
        )
    });
    row.finish();
}

fn respawn(
    h: &mut DefaultHasher,
    point: Option<&crate::game::server_side::respawn_anchor::RespawnConfig>,
) {
    point
        .map(|p| (p.dim.id(), p.pos, rotation([p.yaw, p.pitch]), p.forced))
        .hash(h);
}

fn cooldowns(h: &mut DefaultHasher, cooldowns: &sunrise_items::cooldowns::Cooldowns) {
    let mut groups: Vec<_> = cooldowns.remaining().collect();
    groups.sort_unstable_by_key(|&(group, _)| group);
    groups.len().hash(h);
    for (name, remaining) in groups {
        (name, remaining, cooldowns.percent(name, 0.0).to_bits()).hash(h);
    }
}

fn player_body(h: &mut DefaultHasher, p: &PlayerMotion) {
    (
        p.shift_key_down,
        p.sprinting,
        p.fall_flying,
        p.swimming,
        p.pose.id(),
        p.ticks_frozen,
        p.invulnerable,
        p.health.permanently_invulnerable,
        p.health.invulnerable_time,
        p.health.dead,
        random_words(&p.random),
    )
        .hash(h);
    hash_support(h, Some(&p.ground));
    bodies::attributes(h, &p.attributes);
}

fn player_collisions(p: &PlayerMotion) -> [Option<bool>; 4] {
    [
        Some(p.collision.horizontal),
        Some(p.collision.vertical),
        Some(p.collision.below),
        Some(p.collision.minor),
    ]
}

fn motion_values(p: &PlayerMotion) -> ([u64; 3], u64, u32, i32) {
    (
        vec_bits([p.velocity.x, p.velocity.y, p.velocity.z]),
        p.fall_distance.to_bits(),
        p.health.food.exhaustion.to_bits(),
        p.health.food.timer,
    )
}

type PlayerItems<'a> = (
    &'a [Option<ItemStack>],
    &'a [Option<ItemStack>; 4],
    Option<&'a ItemStack>,
    &'a Equipment,
);

fn player_items(
    row: &mut PlayerRow<'_>,
    (inventory, armor, offhand, extra): PlayerItems<'_>,
    items: &Items,
) {
    row.component("inventory", |h| {
        inventory.len().hash(h);
        for slot in inventory {
            stack(h, slot.as_ref(), items);
        }
    });
    row.component("armor", |h| {
        for slot in armor {
            stack(h, slot.as_ref(), items);
        }
    });
    row.component("equipment", |h| {
        for slot in extra.iter() {
            stack(h, slot.as_ref(), items);
        }
    });
    row.component("offhand", |h| stack(h, offhand, items));
}

fn effects(row: &mut PlayerRow<'_>, effects: &PlayerEffects) {
    row.value("effects_tick", effects.tick_count);
    row.component("effects", |h| {
        effects.list.len().hash(h);
        for active in &effects.list {
            effect(h, active);
        }
    });
}

fn effect(h: &mut DefaultHasher, active: &EffectInstance) {
    let mut next = Some(active);
    while let Some(EffectInstance {
        id,
        duration,
        amplifier,
        ambient,
        visible,
        show_icon,
        hidden,
        ..
    }) = next
    {
        (true, id, duration, amplifier, ambient, visible, show_icon).hash(h);
        next = hidden.as_deref();
    }
    false.hash(h);
}

#[cfg(test)]
mod tests {
    use super::{super::hasher, *};

    #[test]
    fn player_detail_preserves_hash_writes_and_lists_nonzero_stats() {
        let values = sunrise_gameplay_common::stats::StatValues::from([(
            "minecraft:custom".into(),
            std::collections::BTreeMap::from([
                ("minecraft:play_time".into(), 29),
                ("minecraft:deaths".into(), 0),
                ("minecraft:walk_one_cm".into(), 17),
            ]),
        )]);
        let expected = {
            let mut h = hasher();
            (2u32, 4u32, 20i32, -1i32).hash(&mut h);
            values.hash(&mut h);
            h.finish()
        };
        let digest = |detail| {
            let mut h = hasher();
            let mut row = PlayerRow {
                hash: &mut h,
                detail,
            };
            row.value("first", 2u32);
            row.value("rest", (4u32, 20i32, -1i32));
            row.statistics(&values);
            h.finish()
        };
        let mut detail = String::new();
        assert_eq!(digest(None), expected);
        assert_eq!(digest(Some(&mut detail)), expected);
        assert!(detail.ends_with(
            "\tstat:minecraft:custom/minecraft:play_time=29\tstat:minecraft:custom/minecraft:walk_one_cm=17"
        ));
    }

    #[test]
    fn seat_digest_preserves_boarding_order_but_ignores_edge_enumeration() {
        let digest = |edges: [(u32, u32); 3], order: [(i64, u32); 2]| {
            hash_seats(edges, &BTreeSet::from([11, 12]), |vehicle| match vehicle {
                10 => order
                    .into_iter()
                    .map(|(place, id)| (place, id, false))
                    .collect(),
                11 => vec![(0, 13, false)],
                _ => unreachable!(),
            })
            .hash
            .finish()
        };
        let edges = [(11, 10), (12, 10), (13, 11)];
        assert_eq!(
            digest(edges, [(0, 11), (0, 12)]),
            digest(edges, [(0, 12), (0, 11)])
        );
        assert_eq!(
            digest(edges, [(0, 11), (1, 12)]),
            digest([edges[2], edges[0], edges[1]], [(1, 12), (0, 11)])
        );
        assert_ne!(
            digest(edges, [(0, 11), (1, 12)]),
            digest(edges, [(0, 12), (1, 11)])
        );
        assert_ne!(
            digest(edges, [(0, 11), (1, 12)]),
            digest([(11, 10), (12, 10), (13, 10)], [(0, 11), (1, 12)])
        );
    }

    #[test]
    fn resident_seats_keep_gameplay_order_on_equal_places() {
        let seats = |order: [u32; 2]| {
            hash_seats([(11, 10), (12, 10)], &BTreeSet::new(), |_| {
                order.into_iter().map(|id| (0, id, false)).collect()
            })
            .hash
            .finish()
        };
        assert_ne!(seats([11, 12]), seats([12, 11]));
    }

    #[test]
    fn malformed_saved_shared_fields_remain_in_the_other_payload() {
        assert!(other_shared_key(&"UUID".into(), &Tag::IntArray(vec![0; 4])));
        assert!(!other_shared_key(
            &"UUID".into(),
            &Tag::IntArray(vec![0; 3])
        ));
        assert!(other_shared_key(&"Fire".into(), &Tag::Short(1)));
        assert!(!other_shared_key(
            &"Fire".into(),
            &Tag::String("bad".into())
        ));
        assert!(!other_shared_key(
            &"Tags".into(),
            &Tag::List(vec![Tag::Int(1)])
        ));
    }

    #[test]
    fn compound_digest_ignores_map_order_but_preserves_list_order() {
        let digest = |tag_value: Tag| {
            let mut h = hasher();
            tag(&mut h, &tag_value);
            h.finish()
        };
        let list = || Tag::List(vec![Tag::Int(1), Tag::Int(2)]);
        let expected = digest(Tag::compound([("a", list()), ("z", Tag::Int(0))]));
        assert_eq!(
            expected,
            digest(Tag::compound([("z", Tag::Int(0)), ("a", list())]))
        );
        assert_ne!(
            expected,
            digest(Tag::compound([
                ("a", Tag::List(vec![Tag::Int(2), Tag::Int(1)])),
                ("z", Tag::Int(0)),
            ]))
        );
    }
}
