//! The old `ServerPlayer` retained across `restoreFrom`, independently of its replacement.
use super::Game;
use mod_api::__private::game::Fields;
use mod_api::{Identifier, LevelRef, OriginalPlayerRef};
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-player attachment state and the stack of originals retained during clone hooks.
#[derive(Default)]
pub(super) struct PlayerState {
    pub(super) attachments: sunrise_mod_runtime::attachments::ResidentAttachments,
    pub(super) originals: Vec<OriginalPlayerSnapshot>,
}

pub(super) struct OriginalPlayerSnapshot {
    token: u64,
    uuid: u128,
    position: [f64; 3],
    dimension: String,
    level: Option<LevelRef>,
    inventory: Vec<Option<sunrise_items::items::ItemStack>>,
    armor: [Option<sunrise_items::items::ItemStack>; 4],
    offhand: Option<sunrise_items::items::ItemStack>,
    health: f32,
    experience: mod_api::PlayerExperience,
    effects: Vec<mod_api::ActiveEffect>,
    pub(super) attachments: mod_api::Attachments,
}

impl OriginalPlayerSnapshot {
    pub(super) fn token(&self) -> u64 {
        self.token
    }
    pub(super) fn uuid(&self) -> u128 {
        self.uuid
    }
}

impl Game<'_> {
    /// `ServerPlayer.restoreFrom`: retain the old object before replacing any data.
    pub(in crate::game) fn capture_original_player(
        &self,
        conn: u32,
    ) -> Option<OriginalPlayerSnapshot> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if !mod_api::__private::listened(mod_api::HookId::PlayerClone) {
            return None;
        }
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        let token = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("original player identity exhausted");
        Some(OriginalPlayerSnapshot {
            token,
            uuid: guest.uuid,
            position: guest.position,
            dimension: guest.dimension.clone(),
            level: self.level(&guest.dimension),
            inventory: guest.inventory.clone(),
            armor: guest.armor.clone(),
            offhand: guest.offhand.clone(),
            health: guest.health,
            experience: mod_api::PlayerExperience::from(
                mod_api::__private::built::PlayerExperience {
                    level: guest.xp_level,
                    progress: guest.xp_progress,
                    total: guest.xp_total,
                },
            ),
            effects: guest
                .effects
                .list
                .iter()
                .map(super::native_living_effects::active_effect)
                .collect(),
            attachments: (*guest.attachments).clone(),
        })
    }

    pub(in crate::game) fn with_original_player(
        &mut self,
        original: OriginalPlayerSnapshot,
        call: impl FnOnce(&mut Self, OriginalPlayerRef),
    ) {
        let handle = mod_api::__private::game::original_player_ref(original.token);
        self.player_state.originals.push(original);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(self, handle)));
        self.player_state.originals.pop();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    fn original_player_snapshot(
        &self,
        original: OriginalPlayerRef,
    ) -> Option<&OriginalPlayerSnapshot> {
        self.player_state
            .originals
            .iter()
            .rev()
            .find(|snapshot| snapshot.token == original.fields())
    }

    pub(super) fn original_player_uuid(&self, original_player: OriginalPlayerRef) -> Option<u128> {
        Some(self.original_player_snapshot(original_player)?.uuid)
    }
    pub(super) fn original_player_position(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<[f64; 3]> {
        Some(self.original_player_snapshot(original_player)?.position)
    }
    pub(super) fn original_player_dimension(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<Identifier> {
        Identifier::parse(&self.original_player_snapshot(original_player)?.dimension).ok()
    }
    pub(super) fn original_player_level(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<LevelRef> {
        self.original_player_snapshot(original_player)?.level
    }
    pub(super) fn original_player_health(&self, original_player: OriginalPlayerRef) -> Option<f32> {
        Some(self.original_player_snapshot(original_player)?.health)
    }
    pub(super) fn original_player_experience(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<mod_api::PlayerExperience> {
        Some(self.original_player_snapshot(original_player)?.experience)
    }
    pub(super) fn original_player_effects(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<Vec<mod_api::ActiveEffect>> {
        Some(
            self.original_player_snapshot(original_player)?
                .effects
                .clone(),
        )
    }
    pub(super) fn original_player_inventory_slot(
        &self,
        original_player: OriginalPlayerRef,
        slot: i32,
    ) -> Option<sunrise_items::items::ItemStack> {
        let snapshot = self.original_player_snapshot(original_player)?;
        let item = match slot {
            0..=35 => snapshot.inventory.get(slot as usize)?.as_ref(),
            100..=103 => snapshot.armor[(103 - slot) as usize].as_ref(),
            -106 => snapshot.offhand.as_ref(),
            _ => None,
        }?;
        Some(item.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Dim, native_guest_turn_tests::with_guests};
    use mod_api::{AttachmentType, Holder, Hooks, PlayerCloneEvent, ServerHost};
    use std::sync::{Arc, Mutex};

    const VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "unsaved",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    const RETAINED: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "retained",
        default: || 0,
        codec: Some(mod_api::AttachmentCodec {
            encode: |value| Ok([("value".into(), sunrise_core::nbt::Tag::Int(*value))].into()),
            decode: |value| match value.get(&sunrise_core::nbt::JavaString::from("value")) {
                Some(sunrise_core::nbt::Tag::Int(value)) => Ok(*value),
                _ => Err(mod_api::AttachmentError::Decode),
            },
        }),
        copy_on_death: true,
        sync: false,
    };

    const SAVED: AttachmentType<i32> = AttachmentType {
        name: "saved",
        copy_on_death: false,
        ..RETAINED
    };

    struct Listener {
        death: bool,
        keep: bool,
        heard: Arc<Mutex<Vec<OriginalPlayerRef>>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_attachment_type("clone:unsaved", false).unwrap();
            r.register_attachment_type("clone:retained", false).unwrap();
            r.register_attachment_type("clone:saved", false).unwrap();
        }
        fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
            assert_eq!(event.was_death, self.death);
            let original = event.original;
            let holder = Holder::OriginalPlayer(original.fields());
            assert_eq!(original.uuid(host), Some(12));
            assert_eq!(original.position(host), Some([7.0, 64.0, -9.0]));
            let dimension = if self.death { Dim::Nether } else { Dim::End };
            assert_eq!(
                original.dimension(host).unwrap().to_string(),
                dimension.id()
            );
            assert_eq!(original.level(host).unwrap().dimension_id(), dimension.id());
            assert_eq!(VALUE.get(host, holder), Ok(73));
            const MISSING: AttachmentType<i32> = AttachmentType {
                name: "missing",
                ..VALUE
            };
            // Never declared, so refused by name (#1448), not read as its default.
            assert_eq!(
                original.attachment(host, &MISSING),
                Err(mod_api::AttachmentError::Undeclared {
                    package_id: "clone".into(),
                    attachment_id: "clone:missing".into(),
                })
            );
            assert_eq!(original.attachment(host, &VALUE), Ok(73));
            assert_eq!(original.attachment(host, &RETAINED), Ok(17));
            assert_eq!(original.attachment(host, &SAVED), Ok(31));
            assert_eq!(
                SAVED.get(host, Holder::Player(event.player)),
                Ok(if self.death { 0 } else { 31 })
            );
            assert_eq!(
                VALUE.get(host, Holder::Player(event.player)),
                Ok(if self.death { 0 } else { 73 })
            );
            assert_eq!(RETAINED.get(host, Holder::Player(event.player)), Ok(17));
            assert_eq!(
                VALUE.set(host, holder, 100),
                Err(mod_api::AttachmentError::MissingHolder)
            );
            assert_eq!(
                VALUE.remove(host, holder),
                Err(mod_api::AttachmentError::MissingHolder)
            );
            if self.death && !self.keep {
                assert_eq!(original.inventory_slot(host, 2), None);
                assert_eq!(original.inventory_slot(host, 100), None);
                assert_eq!(original.inventory_slot(host, -106), None);
            } else {
                assert_eq!(original.inventory_slot(host, 2).unwrap().count(), 4);
                assert_eq!(original.inventory_slot(host, 100).unwrap().count(), 1);
                assert_eq!(original.inventory_slot(host, -106).unwrap().count(), 2);
            }
            assert_eq!(
                original.health(host),
                Some(if self.death { 0.0 } else { 13.0 })
            );
            assert_eq!(original.experience(host).unwrap().level, 7);
            let effects = original.effects(host).unwrap();
            assert_eq!(effects.len(), 1);
            assert_eq!(effects[0].duration, 200);
            assert_eq!(effects[0].amplifier, 1);
            let replacement = host.player(event.player).unwrap();
            let replacement_effects = mod_api::LivingRef::new(replacement.entity())
                .effects(host)
                .unwrap();
            assert_eq!(replacement_effects.len(), usize::from(!self.death));
            assert_eq!(original.inventory_slot(host, 99), None);
            // Replacement writes cannot alter the captured old values.
            VALUE.set(host, Holder::Player(event.player), 99).unwrap();
            assert_eq!(original.attachment(host, &VALUE), Ok(73));
            self.heard.lock().unwrap().push(original);
        }
    }

    fn clone_case(death: bool, keep: bool) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "clone",
            Box::new(Listener {
                death,
                keep,
                heard: heard.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                let dimension = if death { Dim::Nether } else { Dim::End };
                game.enter_level(dimension).unwrap();
                game.ui.rules.set(
                    "keep_inventory",
                    sunrise_client::gamerules::Value::Bool(keep),
                );
                let host = game.lan.as_mut().unwrap();
                host.hub.guests.update(0, |guest| {
                    guest.dimension = dimension.id().to_owned();
                    guest.position = [7.0, 64.0, -9.0];
                    guest.dead = death;
                    guest.health = if death { 0.0 } else { 13.0 };
                    guest.xp_level = 7;
                    guest
                        .effects
                        .list
                        .push(sunrise_client::effects::EffectInstance::from_def(
                            sunrise_client_core::potions::effect_by_index(
                                mod_api::__private::effect_index(
                                    mod_platform::__private::part_tests::effect_key(
                                        "minecraft:speed",
                                    ),
                                )
                                .unwrap(),
                            )
                            .unwrap(),
                            200,
                            1,
                        ));
                    // Respawn starts after death drops; keep-inventory leaves these intact.
                    if !death || keep {
                        guest.inventory[2] =
                            Some(sunrise_items::items::ItemStack::new("minecraft:diamond", 4));
                        guest.armor[3] = Some(sunrise_items::items::ItemStack::new(
                            "minecraft:diamond_boots",
                            1,
                        ));
                        guest.offhand =
                            Some(sunrise_items::items::ItemStack::new("minecraft:apple", 2));
                    }
                });
                let attachments = host.hub.guests.attachments_mut(12).unwrap();
                RETAINED.set(attachments, Holder::Level, 17).unwrap();
                SAVED.set(attachments, Holder::Level, 31).unwrap();
                // Keep codec-backed entries undecoded, beside an unserialized binding.
                let saved = mod_api::Attachments::from_nbt(attachments.to_nbt());
                attachments.replace_contents(saved);
                VALUE.set(attachments, Holder::Level, 73).unwrap();
                if death {
                    game.guest_respawn(12);
                } else {
                    let play = host.play.entry(12).or_default();
                    play.won_game = true;
                    play.respawn_requested = true;
                    game.handle_server_portal(12);
                }
                let heard = heard.lock().unwrap();
                assert_eq!(heard.len(), 1);
                let original = heard[0];
                assert_eq!(original.position(game), None);
                assert_eq!(original.inventory_slot(game, 2), None);
                assert_eq!(original.health(game), None);
                assert_eq!(original.experience(game), None);
                assert_eq!(original.effects(game), None);
                assert_eq!(
                    original.attachment(game, &VALUE),
                    Err(mod_api::AttachmentError::MissingHolder)
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn death_clone_retains_original_and_expires_after_dispatch() {
        crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
            clone_case(true, false);
            clone_case(true, true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn end_return_clone_retains_end_state_and_expires_after_dispatch() {
        crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
            clone_case(false, true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn unsubscribed_clone_does_not_capture() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            with_guests(|game| assert!(game.capture_original_player(12).is_none()));
        });
    }
}
