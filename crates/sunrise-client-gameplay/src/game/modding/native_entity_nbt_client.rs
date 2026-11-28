//! Read-only saves of materialised client entities. The family encoders also
//! serve the authoritative save paths; no saved server compound is requested.
use crate::game::{
    ClientGame,
    client_side::{remote_client::Remote, text_displays::Shows},
};
use mod_api::{Compound, EntityRef};
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_gameplay_common::game::{
    commands_common::Target, dragon_common::Breath, general::general_type_catalog,
    interactions_common::InteractionState, potions_common::area_effect_cloud_to_nbt,
    tables::GameItems as _,
};
use sunrise_items::{item_nbt::item_to_nbt, items::ItemStack};
use sunrise_sim::{
    entity_nbt::{self, Base, EntityTagWriter, MobSave},
    lan::playerdata::uuid_ints,
    mobs::leash::{self, Holder},
};

impl ClientGame<'_> {
    pub(in crate::game) fn entity_client_nbt(&self, entity: EntityRef) -> Option<Compound> {
        let (remote, _) = self.native_client_entity_context(entity).ok()?;
        let target = self.native_client_entity_target(entity);
        let mut tag = self.client_entity_own_nbt(remote, entity.id(), target)?;
        // Match ServerGame::entity_tag_in_level, including its stand exclusion.
        if matches!(
            target,
            Some(Target::Mob(_) | Target::Boat(_) | Target::Cart(_))
        ) {
            let mut game = self;
            entity_nbt::write_passenger_tree(
                &mut game,
                &mut tag,
                entity.id(),
                |_, vehicle| {
                    let mut seats: Vec<_> = remote.passenger_seats(vehicle).collect();
                    seats.sort_unstable_by_key(|&(_, index)| index);
                    seats.into_iter().map(|(id, _)| id).collect()
                },
                |game, id| {
                    let rider = EntityRef::new(entity.epoch(), id);
                    let target = game.native_client_entity_target(rider)?;
                    // Server put_passengers_for writes only mob and saved general rows.
                    match target {
                        Target::Mob(_) => {}
                        Target::General(_) => {
                            let row = game.native_client_general(rider)?;
                            if general_type_catalog().save_policy(row.type_slot)
                                == mod_api::GeneralSavePolicy::NoSave
                            {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                    game.client_entity_own_nbt(remote, id, Some(target))
                },
            );
        }
        let Tag::Compound(mut fields) = tag else {
            return None;
        };
        fields.remove(&"id".into());
        Some(fields)
    }

    fn client_entity_own_nbt(
        &self,
        remote: &Remote,
        id: u32,
        target: Option<Target>,
    ) -> Option<Tag> {
        let wire_id = id as i32;
        if wire_id == remote.entity_id
            || matches!(
                target,
                Some(Target::Player | Target::Guest(_) | Target::DragonPart(_) | Target::Hook(_))
            )
        {
            return None;
        }
        let c = &self.common;
        let tracked = remote.entities.get(&wire_id);
        let kind = tracked.map(|t| t.kind.as_str());
        if kind == Some("minecraft:player") {
            return None;
        }
        // AddEntity already stored this identity; the general spawn route has
        // its own UUID, also recorded at admission. Missing identity is not a
        // reason to invent UUIDs for packet-only tracking entries.
        let uuid = if let Some(row) = remote.general.rows.get(&wire_id) {
            uuid_ints(row.uuid)
        } else {
            uuid_ints(*remote.level_entity_uuids.get(&wire_id)?)
        };
        let items = self.items();
        let registry = c.pack.assets.registry;
        let stored_uuid = |id: u32| c.persist.uuids.get(&id).copied();
        let holder_tag = |holder| {
            leash::to_nbt(
                holder,
                match holder {
                    Holder::Mob(id) | Holder::Boat(id) => stored_uuid(id),
                    Holder::Player => Some(uuid_ints(remote.profile)),
                    Holder::Knot(_) => None,
                },
            )
        };

        let mut tag = match target {
            Some(Target::Mob(_)) => {
                let m = c.mob_index.get_ref(&c.mobs, id)?;
                let mut out = EntityTagWriter::new(None);
                if let Some(value) = m.extra.leash.and_then(holder_tag) {
                    out.insert("leash".into(), value);
                }
                entity_nbt::write_mob(
                    m,
                    uuid,
                    &mut out,
                    Some(registry),
                    stored_uuid,
                    items,
                    MobSave {
                        include_id: true,
                        game_time: c.world.game_time,
                        variants: Some(self.world_variants()),
                    },
                );
                out.finish()
            }
            Some(Target::Stand(_)) => entity_nbt::armor_stand_to_nbt(
                c.armor_stands.iter().find(|a| a.id == id)?,
                uuid,
                None,
                items,
            ),
            Some(Target::Frame(_)) => entity_nbt::item_frame_to_nbt(
                c.item_frames.iter().find(|f| f.id == id)?,
                uuid,
                None,
                items,
            ),
            Some(Target::Painting(_)) => {
                entity_nbt::painting_to_nbt(c.paintings.iter().find(|p| p.id == id)?, uuid, None)
            }
            Some(Target::Item(_)) => entity_nbt::item_entity_to_nbt(
                c.dropped.iter().find(|e| e.id == id)?,
                uuid,
                None,
                items,
            ),
            Some(Target::Boat(_)) => {
                let b = c.boats.iter().find(|b| b.id == id)?;
                let raw = b
                    .leash
                    .and_then(holder_tag)
                    .map(|v| Tag::compound([("leash", v)]));
                b.to_nbt(uuid, raw.as_ref(), items)
            }
            Some(Target::Cart(_)) => c
                .carts
                .iter()
                .find(|c| c.id == id)?
                .to_nbt(uuid, None, items),
            Some(Target::Orb(_)) => {
                entity_nbt::orb_to_nbt(c.orbs.iter().find(|o| o.id == id)?, uuid)
            }
            Some(Target::Projectile(_)) => {
                let p = c.creatures.projectiles.iter().find(|p| p.id == id)?;
                let block = p
                    .arrow
                    .as_ref()
                    .and_then(|a| a.last_state)
                    .map(|s| c.state_to_nbt(s));
                let owner = p.owner_uuid;
                entity_nbt::arrow_to_nbt(p, uuid, owner, block, None, items)
                    .or_else(|| entity_nbt::fireball_to_nbt(p, uuid, owner, None, items))
                    .or_else(|| entity_nbt::throwable_to_nbt(p, uuid, owner, None, items))
                    .or_else(|| entity_nbt::spit_or_bullet_to_nbt(p, uuid, owner, None, None))?
            }
            Some(Target::Trident(_)) => {
                let t = c.tridents.iter().find(|t| t.id == id)?;
                let mut tag = t.to_nbt(uuid, registry, None, items);
                if let Some(owner) = stored_uuid(t.owner).or_else(|| {
                    (t.owner as i32 == remote.entity_id).then(|| uuid_ints(remote.profile))
                }) && let Tag::Compound(fields) = &mut tag
                {
                    fields.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
                }
                tag
            }
            Some(Target::Rocket(_)) => c
                .rockets
                .iter()
                .find(|r| r.id == id)?
                .to_nbt(uuid, None, None, items),
            Some(Target::Tnt(_)) => {
                let t = c.tnts.iter().find(|t| t.id == id)?;
                entity_nbt::tnt_to_nbt(
                    t.position,
                    t.velocity,
                    t.ground.on_ground,
                    t.fall_distance,
                    t.fuse,
                    t.power,
                    t.block_state_nbt(registry),
                    t.rotation,
                    uuid,
                    None,
                    None,
                )
            }
            Some(Target::Falling(_)) => {
                let f = c.falling.iter().find(|f| f.id == id)?;
                let mut tag = entity_nbt::falling_block_to_nbt(
                    c.state_to_nbt(f.state),
                    f.position,
                    f.velocity,
                    f.age,
                    f.drop_item,
                    f.damage,
                    f.fall_distance,
                    f.ground.on_ground,
                    f.rotation,
                    uuid,
                );
                if let Tag::Compound(fields) = &mut tag {
                    fields.insert("Fire".into(), Tag::Short(f.fire_ticks as i16));
                }
                tag
            }
            Some(Target::Fangs(_)) => {
                let f = c.fangs.list.iter().find(|f| f.id == id)?;
                entity_nbt::evoker_fangs_to_nbt(
                    vec3(f.pos),
                    f.motion,
                    f.fire_ticks,
                    f.yaw,
                    f.warmup,
                    stored_uuid(f.owner),
                    uuid,
                    None,
                )
            }
            Some(Target::Crystal(_)) => c.crystals.iter().find(|e| e.id == id)?.to_nbt(uuid, None),
            Some(Target::Dragon(_)) => c
                .dragon
                .as_ref()
                .filter(|d| d.id == id)?
                .to_nbt(uuid, None, items),
            Some(Target::Cushion(_)) => {
                entity_nbt::cushion_to_nbt(c.cushions.iter().find(|e| e.id == id)?, uuid, None)
            }
            Some(Target::Spawner(_)) => {
                let s = c.ominous_spawners.iter().find(|s| s.id == id)?;
                entity_nbt::ominous_item_spawner_to_nbt(
                    &s.item,
                    vec3(s.position),
                    s.spawn_item_after,
                    s.fire_ticks,
                    uuid,
                    None,
                    items,
                )
            }
            Some(Target::Bolt(_)) => {
                let b = c.bolts.iter().find(|b| b.id == id)?;
                b.to_nbt(b.at, uuid, true, None)
            }
            Some(Target::Cloud(_)) => {
                area_effect_cloud_to_nbt(c.breath_clouds.iter().find(|c| c.id == id)?, uuid, None)
            }
            Some(Target::General(_)) => {
                let row = remote.general.rows.get(&wire_id)?;
                entity_nbt::entity_base_tag(
                    row.kind(),
                    vec3(row.position),
                    Vec3::ZERO,
                    row.rotation,
                    uuid,
                    None,
                )
            }
            Some(Target::Other(_)) => {
                let i = self.client.interactions.get(&wire_id)?;
                let mut out = EntityTagWriter::new(None);
                Base {
                    id: "minecraft:interaction",
                    uuid,
                    position: i.position,
                    motion: Vec3::ZERO,
                    rotation: [i.yaw, i.pitch],
                    on_ground: false,
                    fall_distance: 0.0,
                }
                .write(&mut out, true);
                InteractionState {
                    width: i.width,
                    height: i.height,
                    response: i.response,
                    attack: None,
                    interaction: None,
                }
                .write(&mut out);
                out.finish()
            }
            Some(Target::Knot(_)) | None => {
                // These families use specialised client stores instead of a shared Target.
                let t = tracked?;
                let rotation = [t.yaw, t.pitch];
                match t.kind.as_str() {
                    kind if Shows::from_kind(kind).is_some() => {
                        let display = self.client.text_displays.0.get(&wire_id)?;
                        let mut tag = entity_nbt::entity_base_tag(
                            kind,
                            vec3(display.position().to_array()),
                            Vec3::ZERO,
                            display.rotation,
                            uuid,
                            None,
                        );
                        display.save_state().write_nbt(&mut tag, registry, items);
                        tag
                    }
                    "minecraft:area_effect_cloud" => {
                        let cloud = self.client.remote_clouds.iter().find(|e| e.id == id)?;
                        // The synced particle is the rendered particle, not the saved custom
                        // particle or potion contents which produced it on the server.
                        area_effect_cloud_to_nbt(
                            &Breath {
                                attachment_lifetime: Default::default(),
                                join: Default::default(),
                                id,
                                at: cloud.at,
                                motion: [0.0; 3],
                                fall_distance: 0.0,
                                fire_ticks: 0,
                                first_tick: t.tick_count == 0,
                                radius: cloud.radius,
                                age: t.tick_count,
                                duration: -1,
                                from_dragon: false,
                                victims: Vec::new(),
                                potion: None,
                            },
                            uuid,
                            None,
                        )
                    }
                    "minecraft:splash_potion"
                    | "minecraft:lingering_potion"
                    | "minecraft:experience_bottle" => {
                        let p = c.potions.thrown.iter().find(|p| p.id == id)?;
                        entity_nbt::thrown_item_to_nbt(
                            Base {
                                id: &t.kind,
                                uuid,
                                position: p.position,
                                motion: p.velocity,
                                rotation,
                                on_ground: false,
                                fall_distance: 0.0,
                            },
                            &p.stack,
                            t.tick_count > 0,
                            None,
                            None,
                            None,
                            items,
                        )
                    }
                    "minecraft:eye_of_ender" => {
                        let e = c.end.eyes.iter().find(|e| e.id == id)?;
                        let mut tag = entity_nbt::entity_base_tag(
                            &t.kind,
                            vec3(e.position.to_array()),
                            vec3(e.velocity.to_array()),
                            rotation,
                            uuid,
                            None,
                        );
                        if let Tag::Compound(fields) = &mut tag {
                            fields.insert(
                                "Item".into(),
                                item_to_nbt(&ItemStack::new("minecraft:ender_eye", 1), items),
                            );
                        }
                        tag
                    }
                    "minecraft:leash_knot" => {
                        let block = *self.client.remote_knots.get(&id)?;
                        let at = leash::knot_state(block).position;
                        entity_nbt::entity_base_tag(&t.kind, at, Vec3::ZERO, rotation, uuid, None)
                    }
                    _ => return None,
                }
            }
            Some(Target::Player | Target::Guest(_) | Target::DragonPart(_) | Target::Hook(_)) => {
                return None;
            }
        };

        if let Tag::Compound(fields) = &mut tag {
            // These are the inherited metadata values independent of body
            // family. Pose is not saved; shared fire/glow bits are visual
            // state, not Fire, Glowing or HasVisualFire.
            if let Some(t) = tracked {
                if let Some(air) = t.air_supply {
                    fields.insert("Air".into(), Tag::Short(air as i16));
                }
                if let Some(name) = &t.custom_name {
                    fields.insert("CustomName".into(), name.clone());
                }
                for (key, on) in [
                    ("CustomNameVisible", t.custom_name_visible),
                    ("Silent", t.silent),
                    ("NoGravity", t.no_gravity),
                ] {
                    if on {
                        fields.insert(key.into(), Tag::Byte(1));
                    }
                }
                if t.ticks_frozen > 0 {
                    fields.insert("TicksFrozen".into(), Tag::Int(t.ticks_frozen));
                }
            }
            // Entity.saveWithoutId writes the vehicle's horizontal position
            // for a rider while retaining the rider's vertical position.
            if let Some(vehicle) = remote.passenger_vehicle(id) {
                let position = if vehicle as i32 == remote.entity_id {
                    c.player
                        .as_ref()
                        .map(|p| [p.position.x, p.position.y, p.position.z])
                } else {
                    self.remote_simulated_entity_position(vehicle as i32)
                };
                if let Some([x, _, z]) = position
                    && let Some(Tag::List(pos)) = fields.get_mut(&"Pos".into())
                    && let [px, _, pz] = pos.as_mut_slice()
                {
                    *px = Tag::Double(x.to_bits());
                    *pz = Tag::Double(z.to_bits());
                }
            }
        }
        Some(tag)
    }
}

fn vec3([x, y, z]: [f64; 3]) -> Vec3 {
    Vec3::new(x, y, z)
}
