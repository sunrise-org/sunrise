//! An entity's sounds, game events, entity events and spectator state for
//! native server mods, through an `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity
//! and level handles"): `Entity.playSound`, `gameEvent`,
//! `Level.broadcastEntityEvent`, `getSoundSource` and `isSpectator`. They
//! return None or false for a stale handle, on a client, and for an entity
//! that is not in the level in place (a player in a parked level).
use super::commands::Target;
use mod_api::{EntityRef, SoundSource};
use sunrise_sim::mobs::Species;

/// `Monster`'s sound source is hostile, and so are a few mobs that are not
/// `Monster`s: `Hoglin`, `Ghast`, `Phantom`, `Shulker` and the slimes
/// (`AbstractCubeMob`).
pub(in crate::game::server_side) fn hostile_species(species: Species) -> bool {
    sunrise_sim::mobs::ai::is_monster(species)
        || matches!(
            species,
            Species::Hoglin
                | Species::Ghast
                | Species::Phantom
                | Species::Shulker
                | Species::Slime
                | Species::MagmaCube
        )
}

impl crate::game::ServerGame<'_> {
    /// The level in place holds the entity: events and game events are
    /// queued in it.
    pub(in crate::game::server_side) fn native_entity_here(&self, entity: EntityRef) -> bool {
        self.entity_level(entity)
            .is_some_and(|level| level.dimension() == self.native_level_key())
    }
    /// `Entity.getSoundSource`: the group an entity's sounds play under.
    /// Players are `Players`; `Monster`s, the dragon, hoglins, ghasts,
    /// phantoms, shulkers, slimes and a killer bunny are `Hostile`; dropped
    /// items and experience orbs `Ambient`; every other entity `Neutral`.
    pub fn entity_sound_source(&self, entity: EntityRef) -> Option<SoundSource> {
        self.target_sound_source(self.native_entity_target(entity)?)
    }

    pub(in crate::game::server_side) fn target_sound_source(
        &self,
        target: Target,
    ) -> Option<SoundSource> {
        Some(match target {
            Target::Player | Target::Guest(_) => SoundSource::Players,
            Target::Mob(id) => {
                let m = self.common.mobs.iter().find(|m| m.id == id)?;
                let evil = m.species() == Species::Rabbit
                    && m.extra.variant == sunrise_sim::mobs::rabbit::EVIL;
                if evil || hostile_species(m.species()) {
                    SoundSource::Hostile
                } else {
                    SoundSource::Neutral
                }
            }
            Target::Item(_) | Target::Orb(_) => SoundSource::Ambient,
            Target::Dragon(_) => SoundSource::Hostile,
            t => match self.target_type(t).as_str() {
                "minecraft:shulker_bullet" => SoundSource::Hostile,
                "minecraft:lightning_bolt" => SoundSource::Weather,
                _ => SoundSource::Neutral,
            },
        })
    }
    /// `Entity.playSound(sound, volume, pitch)`: plays sound event `sound` at
    /// the entity under its sound source, for the players within range. A
    /// silent mob plays nothing, as vanilla's check is `isSilent`; a player
    /// (`Player.playSound`) doesn't hear it themself, their own client plays
    /// it. True means queued, not heard. Registered events go by ID and other valid keys by name.
    pub fn entity_play_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let (Some(level), Some(at), Some(source)) = (
            self.entity_level(entity),
            self.target_position(target),
            self.entity_sound_source(entity),
        ) else {
            return false;
        };
        let except = match target {
            Target::Player => self.server.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => {
                if self.entity_is_silent(entity) == Some(true) {
                    return false;
                }
                None
            }
        };
        self.native_queue_sound(super::native_level_effects::NativeSoundRequest {
            level,
            sound,
            at,
            source,
            volume,
            pitch,
            except,
            entity: None,
            inline_range: None,
            seed: None,
        })
    }
    /// `Entity.gameEvent(event)`: game event `event` (`minecraft:step`...)
    /// at the entity's position with the entity as its source, which sculk
    /// sensors, shriekers, wardens and allays in range hear. A player or mob
    /// is the event's source entity; other entities retain their own id.
    /// False for an event vanilla doesn't have, a dead guest and a guest while
    /// another one acts.
    pub fn entity_game_event(&mut self, entity: EntityRef, event: &str) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(at) = self.target_position(target) else {
            return false;
        };
        let Some(level) = self.level(self.common.dim.id()) else {
            return false;
        };
        self.level_game_event(
            level,
            event,
            at,
            mod_api::GameEventContext {
                source_entity: Some(entity),
                affected_state: None,
            },
        )
    }
    fn resolve_entity_broadcast_event(&self, entity: EntityRef) -> Option<(i32, [f64; 3])> {
        let target = self.native_entity_target(entity)?;
        let at = self.target_position(target)?;
        if !self.native_entity_here(entity) {
            return None;
        }
        let entity_id = sunrise_gameplay_common::game::entity_ids::wire_entity_id(entity.id());
        Some((entity_id, at))
    }
    /// `Level.broadcastEntityEvent(entity, id)`: sends entity event `id` (the
    /// byte `Entity.handleEntityEvent` reads: `2` is a hurt flash, `3` a
    /// death, `35` a totem...) to the players tracking the entity and to the
    /// entity itself. True means queued; false for an entity outside the
    /// level in place.
    pub fn entity_broadcast_event(&mut self, entity: EntityRef, id: u8) -> bool {
        let Some((entity_id, at)) = self.resolve_entity_broadcast_event(entity) else {
            return false;
        };
        self.emit_level_effect(
            sunrise_gameplay_common::game::journal_level_effects::PendingWrite::EntityEvent {
                level: self.common.dim,
                entity_id,
                event: id as i8,
                at,
            },
        );
        true
    }
    /// Immediate counterpart to `entity_broadcast_event` for unjournaled direct writes whose
    /// mutation takes effect immediately even inside an open transaction (such as non-player
    /// `living_hurt_and_break`), keeping the write and its notification on the same side of the
    /// rollback boundary.
    pub(in crate::game::server_side) fn broadcast_entity_event_immediate(
        &mut self,
        entity: EntityRef,
        id: u8,
    ) -> bool {
        let Some((entity_id, at)) = self.resolve_entity_broadcast_event(entity) else {
            return false;
        };
        self.lan_entity_event(entity_id, id as i8, at);
        true
    }
    /// `Entity.isSpectator`: true for a player in spectator mode, false for
    /// every other entity.
    pub fn entity_spectator(&self, entity: EntityRef) -> Option<bool> {
        Some(match self.native_entity_target(entity)? {
            Target::Player => self.common.spectator(),
            Target::Guest(conn) => self.command_guest(conn)?.game_mode == 3,
            _ => false,
        })
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

#[cfg(test)]
mod issue_6335_tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    use mod_api::{Flow, HostSide, ModEntry, Mods};
    use sunrise_gameplay_common::game::entity_ids::{entity_id_bits, wire_entity_id};
    use sunrise_protocol::packets::play::Clientbound as C;
    use sunrise_sim::lan::{Out, PlayerSender};

    struct RecordSound {
        seen: Arc<Mutex<Vec<u32>>>,
        calls: Arc<AtomicUsize>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RecordSound {
        fn play_level_sound_at_entity(
            &self,
            _: HostSide<'_>,
            e: &mut mod_api::PlayLevelSoundAtEntityEvent,
        ) -> Flow {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(e.entity.id());
            Flow::Continue
        }
    }

    fn decode_packets(rx: &mpsc::Receiver<Out>) -> Vec<C> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                _ => Vec::new(),
            })
            .filter_map(|body| {
                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                C::decode(id, reader.rest()).ok()
            })
            .collect()
    }

    /// A live entity whose stored id has the high bit set keeps those bits
    /// through a queued bound sound (flush) and both entity-event broadcasts.
    #[test]
    #[ignore = "daily"]
    fn issue_6335_a_live_high_bit_entity_keeps_its_sound_and_event_bits() {
        const NAME: &str = "game::server_side::native_entity_sounds::issue_6335_tests::issue_6335_a_live_high_bit_entity_keeps_its_sound_and_event_bits";
        sunrise_gameplay_common::game::in_own_process(NAME, || {
            mod_platform::__private::test_platform::install();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: Mods = vec![ModEntry::new(
                "record-sound",
                Box::new(RecordSound {
                    seen: Arc::clone(&seen),
                    calls: Arc::clone(&calls),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                crate::game::native_guest_turn_tests::with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let (tx, rx) = mpsc::channel();
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.sender = PlayerSender::new(tx, Arc::default());
                    });
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    game.server.next_entity = 0x8000_0001 - 1;
                    let id = game
                        .spawn_mob(
                            "minecraft:zombie",
                            [3.5, 64.0, 5.5],
                            true,
                            sunrise_gameplay_common::game::creatures::SpawnSite::Raw,
                        )
                        .unwrap();
                    assert_eq!(id, 0x8000_0001, "the mob's stored id has the high bit");
                    let entity =
                        mod_api::EntityRef::new(game.common.native_lifecycle.epoch().unwrap(), id);
                    let wire = wire_entity_id(id);

                    // The queued bound sound keeps the bits and the flush delivers a
                    // SoundEntity packet for the same id.
                    assert!(game.entity_play_bound_sound(
                        entity,
                        "minecraft:entity.player.burp",
                        mod_api::SoundSource::Players,
                        1.0,
                        1.0,
                    ));
                    assert_eq!(
                        game.server
                            .pending_native_sounds
                            .last()
                            .and_then(|s| s.entity),
                        Some(wire)
                    );
                    game.flush_native_sounds();
                    assert!(game.server.pending_native_sounds.is_empty());
                    let packets = decode_packets(&rx);
                    assert!(
                        packets
                            .iter()
                            .any(|p| matches!(p, C::SoundEntity(s) if s.id == wire)),
                        "a SoundEntity packet carries the high-bit id"
                    );
                    assert!(
                        !packets.iter().any(|p| matches!(p, C::Sound(_))),
                        "only the entity hook hears the bound sound"
                    );
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    assert_eq!(seen.lock().unwrap().as_slice(), [id]);

                    // Both entity-event paths accept the live entity and keep the bits.
                    assert!(game.entity_broadcast_event(entity, 2));
                    assert!(game.broadcast_entity_event_immediate(entity, 2));
                    assert_eq!(game.server.lan_entity_events, vec![(wire, 2), (wire, 2)]);
                    assert_eq!(entity_id_bits(wire), id);
                });
            });
        });
    }
}
