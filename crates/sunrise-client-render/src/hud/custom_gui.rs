//! OptiFine's Custom GUIs on the GUI atlas: the pictures a pack's `optifine/gui/container` rules
//! draw in place of a container screen's textures are packed beside the vanilla ones, and a
//! screen asks which of them to draw ([`HudAtlas::container_sprite`]).
use super::*;
use sunrise_optifine::gui::{CustomGuis, Subject};

/// The most texels a replacement picture may have: an image beyond it is left out unread.
const MAX_TEXELS: u64 = 4096 * 4096;

/// The pictures the rules of `guis` draw, packed whole under the names the rules give them
/// ([`sunrise_optifine::gui::sprite_name`]), each with the size of the texture it replaces when
/// that differs from its own: a picture of a higher resolution keeps the screen's layout.
pub(super) fn custom_gui_sprites(
    root: &ResourceSource,
    defaults: &ResourceSource,
    guis: &CustomGuis,
) -> Vec<(PackedImage, Option<(u32, u32)>)> {
    guis.pictures()
        .into_iter()
        .filter_map(|(picture, default)| {
            let too_big = png_size(root, &picture.path)
                .is_some_and(|(w, h)| u64::from(w) * u64::from(h) > MAX_TEXELS);
            if too_big {
                log::warn!("skipped custom GUI texture {}: too big", picture.path);
                return None;
            }
            let (w, h, pixels) = decode_resource(root, &picture.path)
                .map_err(|e| log::warn!("skipped custom GUI texture {}: {e}", picture.path))
                .ok()?;
            let replaced = png_size(
                defaults,
                &crate::atlas_sources::texture_resource(default, "textures"),
            )
            .filter(|&size| size != (w, h));
            Some(((picture.sprite.clone(), w, h, pixels), replaced))
        })
        .collect()
}

impl HudAtlas {
    /// The sprite a container screen draws for its GUI texture `default` (`gui/container/furnace`):
    /// the picture the packs' Custom GUIs rules put there for what `screen` describes, else
    /// `default` itself, as when no rule holds or the picture did not pack. `screen` is only
    /// built when there are rules.
    pub fn container_sprite<'a, 's>(
        &'a self,
        default: &'a str,
        screen: impl FnOnce() -> Subject<'s>,
    ) -> &'a str {
        if self.custom_guis.is_empty() {
            return default;
        }
        match self.custom_guis.texture(&screen(), default) {
            Some(sprite) if self.sprites.contains_key(sprite) => sprite,
            _ => default,
        }
    }
}
