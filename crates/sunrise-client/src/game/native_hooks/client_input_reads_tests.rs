#[cfg(test)]
mod tests {
    use crate::game::{Screen, native_entity_refs::tests::with_client_game};
    use mod_api::__private::{ModPhase, PhaseOwner};
    use mod_api::MappingError;
    use sunrise_client::motion::PlayerMotion;
    use sunrise_core::geometry::Vec3;
    use winit::{event::MouseButton, keyboard::KeyCode};

    #[test]
    #[ignore = "daily"]
    fn mapping_physical_down_follows_the_binding_and_ignores_the_logical_state() {
        with_client_game(|game| {
            let _phase = PhaseOwner::scope(ModPhase::World);
            let forward_mapping = game.vanilla_key_mapping("key.forward").unwrap();
            let attack_mapping = game.vanilla_key_mapping("key.attack").unwrap();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            assert_eq!(
                game.vanilla_key_mapping("key.not_a_mapping"),
                Err(MappingError::Unknown {
                    key: "key.not_a_mapping".into(),
                }),
            );
            assert_eq!(forward_mapping.physical_down(game), Ok(false));
            game.physical_keys.insert(KeyCode::KeyW);
            assert_eq!(forward_mapping.physical_down(game), Ok(true));
            // A write to the logical mapping does not move the physical read.
            game.keys.insert(KeyCode::KeyW);
            assert_eq!(forward_mapping.is_down(game), Ok(true));
            forward_mapping.set_down(game, false).unwrap();
            assert!(!forward_mapping.is_down(game).unwrap());
            assert_eq!(forward_mapping.physical_down(game), Ok(true));
            // Rebound, it reads the new key, and the old one no longer counts.
            game.settings.keys.set("key.forward", "key.keyboard.up");
            assert_eq!(forward_mapping.physical_down(game), Ok(false));
            game.physical_keys.insert(KeyCode::ArrowUp);
            assert_eq!(forward_mapping.physical_down(game), Ok(true));
            // A mapping on a mouse button reads the button, by name too.
            assert_eq!(attack_mapping.physical_down(game), Ok(false));
            game.physical_buttons.insert(MouseButton::Left);
            assert_eq!(attack_mapping.physical_down(game), Ok(true));
            assert!(game.is_key_down("key.mouse.left"));
            assert!(!game.is_key_down("key.mouse.right"));
            game.physical_buttons.clear();
            assert!(!game.is_key_down("key.mouse.left"));
            // A mapping bound to no key is never down.
            game.settings.keys.set("key.attack", "key.keyboard.unknown");
            game.physical_buttons.insert(MouseButton::Left);
            assert_eq!(attack_mapping.physical_down(game), Ok(false));

            // The physical read is available even for a key with no tracked logical state.
            let chat = game.vanilla_key_mapping("key.chat").unwrap();
            game.physical_keys.insert(KeyCode::KeyT);
            assert_eq!(chat.physical_down(game), Ok(true));
            assert_eq!(chat.is_down(game), Err(MappingError::NoLogicalState));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn stale_vanilla_mapping_ref_refuses_foreign_client() {
        with_client_game(|game| {
            let _phase = PhaseOwner::scope(ModPhase::World);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.screen = Screen::Playing;
            game.title = None;
            let old = game.vanilla_key_mapping("key.attack").unwrap();
            assert_eq!(old.binding(game), Ok("key.mouse.left"));

            // A new client input store resolves the same fixed vanilla slot with a new identity.
            game.mod_keys = sunrise_client::keymapping::ModKeys::default();
            game.settings.keys.set("key.attack", "key.keyboard.r");
            assert_eq!(old.binding(game), Err(MappingError::Stale));
            assert_eq!(old.is_down(game), Err(MappingError::Stale));
            assert_eq!(old.physical_down(game), Err(MappingError::Stale));
            assert_eq!(old.translated_key_message(game), Err(MappingError::Stale));
            assert_eq!(old.set_down(game, true), Err(MappingError::Stale));
            assert_eq!(old.click(game), Err(MappingError::Stale));

            let current = game.vanilla_key_mapping("key.attack").unwrap();
            assert_eq!(current.binding(game), Ok("key.keyboard.r"));
            assert_eq!(current.is_down(game), Ok(false));
            assert_eq!(game.attack_clicks.count(), 0);
            // A live handle can still write with the same player and phase.
            current.set_down(game, true).unwrap();
            current.click(game).unwrap();
            assert_eq!(current.is_down(game), Ok(true));
            assert_eq!(game.attack_clicks.count(), 1);
        });
    }
}
