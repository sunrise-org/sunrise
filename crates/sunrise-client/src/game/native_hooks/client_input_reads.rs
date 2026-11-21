use super::*;
use crate::game::api_values::TryIntoApi;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl Game<'_> {
    /// Playing, with nothing over the world: keys and zoom apply.
    pub(in crate::game) fn playing(&self) -> bool {
        self.screen == Screen::Playing
            && self.title.is_none()
            && self.sign_edit.is_none()
            && self.book.is_none()
    }
    /// Milliseconds on the game's own clock (`Util.getMillis`), counting from the game's start:
    /// the clock the HUD's scrolling labels and the screens' text fields read too.
    pub(in crate::game) fn millis(&self) -> u64 {
        sunrise_client::hud_overlays::now_ms().max(0) as u64
    }
    /// The current window modifier keys in the same GLFW encoding as input hooks.
    /// Cleared when the window loses focus; available even outside a world.
    pub(in crate::game) fn modifiers(&self) -> ClientModifiers {
        ClientModifiers::new(crate::game::glfw_modifiers(self.modifiers))
    }
    /// A vanilla mapping (`key.forward`, `key.jump`, `key.attack`…): whether
    /// it is held (`KeyMapping.isDown`), and its key's name as Controls shows
    /// it. None for a mapping it doesn't answer for.
    pub(in crate::game) fn mapping(
        &self,
        name: &str,
    ) -> Result<Option<mod_api::KeyMapping>, mod_api::TextFormError> {
        let Some(down) = self.mapping_held(name) else {
            return Ok(None);
        };
        Ok(Some(mod_api::KeyMapping {
            down,
            label: keybinds::display_name(self.settings.keys.get(name)).try_into_api()?,
        }))
    }
    /// `mapping` without its label: whether it is held, and a hash of the key it is bound to.
    pub(in crate::game) fn mapping_state(&self, name: &str) -> Option<(bool, u64)> {
        use std::hash::{Hash, Hasher};
        let down = self.mapping_held(name)?;
        let mut binding = std::collections::hash_map::DefaultHasher::new();
        self.settings.keys.get(name).hash(&mut binding);
        Some((down, binding.finish()))
    }
    /// Whether the key or mouse button the vanilla mapping `name` is bound to is physically down.
    pub(in crate::game) fn mapping_physical_down(
        &self,
        name: &str,
    ) -> Result<Option<bool>, mod_api::LevelReadError> {
        // The mappings `mapping` and `mapping_state` answer for, and no others.
        if self.mapping_held(name).is_none() {
            return Ok(None);
        }
        Ok(Some(self.is_key_down(self.settings.keys.get(name))))
    }
    /// Whether the vanilla mapping `name` is held, for `mapping` and `mapping_state`.
    fn mapping_held(&self, name: &str) -> Option<bool> {
        let &(_, key) = MAPPINGS.iter().find(|m| m.0 == name)?;
        Some(match key {
            Some(k) => self.movement_key_held(k),
            // Attack and use read the logical mapping, not the physical mouse's state.
            None => {
                if name == "key.use" {
                    self.right
                } else {
                    self.left
                }
            }
        })
    }
}
