//! `ClientRenderClientHost::create_dynamic_texture`, `update_dynamic_texture`,
//! `update_dynamic_texture_region` and `release_dynamic_texture`: a client mod's dynamic textures
//! (vanilla's `DynamicTexture` in the `TextureManager`), made for the mod whose hook or part is
//! running (`crash::current_native_mod`). Each call is checked here. A create makes its GPU
//! texture within the call; the others are queued as commands the render thread applies, in
//! order, before the next frame (`hud::dynamic_textures`).
use crate::game::ClientGame;
use mod_api::{DynamicTextureError, Identifier};
use sunrise_client::hud::dynamic_textures::Command;

impl ClientGame<'_> {
    /// The calling mod, when `id` is in a namespace it owns (its manifest namespaces or a served
    /// package's namespaces via `served_by`), excluding vanilla's `minecraft`.
    fn dynamic_texture_owner(id: &Identifier) -> Result<&'static str, DynamicTextureError> {
        mod_api::content::crash::current_native_mod()
            .filter(|owner| {
                id.namespace() != "minecraft"
                    && mod_api::__private::owns_namespace(owner, id.namespace())
            })
            .ok_or(DynamicTextureError::NotOwned)
    }

    /// Tell the table which device's textures it is naming, as the renderer holds them now.
    pub fn bind_dynamic_textures(&mut self) {
        if let Some(renderer) = &self.client.renderer.0 {
            self.client
                .dynamic_textures
                .bind(renderer.hud.dynamic().id(), renderer.max_texture_side());
        }
    }

    /// The calling mod's texture `id` of `mip_levels` levels made from `rgba` (`width * height`
    /// pixels of four bytes, level 0).
    pub fn create_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        mip_levels: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        self.bind_dynamic_textures();
        let owner = Self::dynamic_texture_owner(id)?;
        let client = &mut self.client;
        let gpu = client
            .mod_gpu
            .as_ref()
            .ok_or(DynamicTextureError::Unavailable)?;
        let size = [width, height];
        // The GPU texture is made within the call, so a device refusal is the call's answer
        // and leaves the id free.
        client
            .dynamic_textures
            .create(owner, id, size, mip_levels, rgba, |handle| {
                gpu.create_dynamic_texture(handle, size, mip_levels, rgba)
                    .map_err(|error| {
                        log::warn!("dynamic texture {id} was not made: {error}");
                        DynamicTextureError::DeviceRefused
                    })
            })
    }
    /// The calling mod's texture `id` given new pixels.
    pub fn update_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        self.bind_dynamic_textures();
        let owner = Self::dynamic_texture_owner(id)?;
        self.client
            .dynamic_textures
            .update(owner, id, [width, height], rgba)
    }
    /// A region of mip level `mip_level` of the calling mod's texture `id` given new pixels.
    pub fn update_dynamic_texture_region(
        &mut self,
        id: &Identifier,
        mip_level: u32,
        origin: [u32; 2],
        size: [u32; 2],
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        self.bind_dynamic_textures();
        let owner = Self::dynamic_texture_owner(id)?;
        self.client
            .dynamic_textures
            .update_region(owner, id, mip_level, origin, size, rgba)
    }
    /// The calling mod's texture `id` released.
    pub fn release_dynamic_texture(&mut self, id: &Identifier) -> Result<(), DynamicTextureError> {
        self.bind_dynamic_textures();
        let owner = Self::dynamic_texture_owner(id)?;
        self.client.dynamic_textures.release(owner, id)
    }
    /// The commands made since the last frame, for this frame's batch: first the textures of
    /// a mod that was switched off since are released.
    pub fn dynamic_texture_commands(&mut self, commands: &mut Vec<Command>) {
        self.bind_dynamic_textures();
        let mods = &self.common.mods;
        self.client
            .dynamic_textures
            .release_unloaded(mod_api::__private::mods_switched(), |owner| {
                mods.code_owner_enabled(owner)
            });
        self.client.dynamic_textures.take_commands(commands);
    }
}
