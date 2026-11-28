//! Phase-scoped client resource reload and client setup views.
use crate::ReloadContext;

/// Fallback empty client render-assets / instance-groups service returned by default
/// [`ClientReloadHost`] implementations (such as startup reload registration before a game exists).
struct EmptyClientReloadServices;

impl crate::RenderAssets for EmptyClientReloadServices {
    fn model(&self, _: &crate::Identifier) -> Result<crate::ModelRef, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
    fn render_pipeline(
        &self,
        _: &crate::Identifier,
    ) -> Result<crate::RenderPipelineRef, crate::RenderPipelineError> {
        Err(crate::RenderPipelineError::Unknown)
    }
    fn render_type(
        &self,
        _: &crate::Identifier,
    ) -> Result<crate::RenderTypeRef, crate::RenderTypeError> {
        Err(crate::RenderTypeError::Unknown)
    }
    fn entity_solid(
        &self,
        _: &crate::Identifier,
    ) -> Result<crate::RenderTypeRef, crate::RenderTypeError> {
        Err(crate::RenderTypeError::Unknown)
    }
    fn entity_cutout(
        &self,
        _: &crate::Identifier,
    ) -> Result<crate::RenderTypeRef, crate::RenderTypeError> {
        Err(crate::RenderTypeError::Unknown)
    }
    fn entity_translucent(
        &self,
        _: &crate::Identifier,
    ) -> Result<crate::RenderTypeRef, crate::RenderTypeError> {
        Err(crate::RenderTypeError::Unknown)
    }
    fn compute_pipeline(
        &self,
        _: &crate::Identifier,
    ) -> Result<crate::ComputePipelineRef, crate::ComputePipelineError> {
        Err(crate::ComputePipelineError::Unknown)
    }
    fn pipeline_modifier(
        &self,
        _: &crate::Identifier,
    ) -> Result<crate::PipelineModifierRef, crate::RenderPipelineError> {
        Err(crate::RenderPipelineError::Unknown)
    }
    fn animation(&self, _: &crate::Identifier) -> Option<crate::AnimationHolder> {
        None
    }
    fn block_state_model(
        &self,
        _: crate::BlockState,
    ) -> Result<crate::ModelRef, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
    fn block_state_models(
        &self,
        _: crate::BlockState,
    ) -> Result<crate::BakedModelKeys<'_>, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
    fn baked_model(
        &self,
        _: crate::BakedModelKey,
    ) -> Result<crate::BakedModelView<'_>, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
    fn atlas_sprite_pixels(
        &self,
        _: crate::SpriteRef,
    ) -> Result<crate::AtlasSpritePixels, crate::AtlasSpriteError> {
        Err(crate::AtlasSpriteError::NotReady)
    }
    fn shader_pack_block_id(&self, _: crate::BlockState) -> Option<i32> {
        None
    }
    fn pipeline_pack_bound(&self, _: crate::RenderPipelineRef) -> bool {
        false
    }
}

impl crate::InstanceGroups for EmptyClientReloadServices {
    fn create_group(
        &self,
        _: crate::ModelRef,
        _: u32,
    ) -> Result<crate::InstanceGroupRef, crate::GroupError> {
        Err(crate::GroupError::Stale)
    }
}

const EMPTY_CLIENT_RELOAD_SERVICES: EmptyClientReloadServices = EmptyClientReloadServices;

/// Phase-scoped read-only host for client resource reload hooks
/// (`client_resources_reloaded`, and [`ClientReloadListenerContext::published`] on later
/// reloads).
///
/// Named `ClientReloadHost` rather than `ClientReloadContext` because [`crate::ClientReloadContext`]
/// is the per-listener asset file reader passed to `ClientReloadListener::prepare`.
pub trait ClientReloadHost:
    ReloadContext + crate::prelude::ItemParticlesClientHost + crate::InstanceGroupContext
{
    /// Published client render assets: standalone models, render pipelines, animations,
    /// and baked block-state models.
    fn render_assets(&self) -> &dyn crate::RenderAssets {
        &EMPTY_CLIENT_RELOAD_SERVICES
    }
    /// Persistent model instance groups.
    fn instance_groups(&self) -> &dyn crate::InstanceGroups {
        &EMPTY_CLIENT_RELOAD_SERVICES
    }
    /// Whether `stack`'s resolved model uses block lighting in `context`.
    fn item_uses_block_light(
        &self,
        _stack: &crate::ItemStack,
        _context: crate::ItemDisplayContext,
    ) -> bool {
        false
    }
    /// Resolve `stack`'s item model once for `display` and `seed`, returning a retained
    /// [`crate::PreparedItemRef`] for [`crate::BlockEntityDrawContext::prepared_item`].
    fn prepare_item(
        &self,
        _stack: &crate::ItemStack,
        _display: crate::ItemDisplayContext,
        _seed: i32,
    ) -> Result<crate::PreparedItemRef, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
    /// Resolved block sprites for each requested block state.
    fn block_sprites(&self, blocks: &[crate::BlockState]) -> Vec<Option<crate::BlockSprites>> {
        vec![None; blocks.len()]
    }
    /// Resolved block-atlas sprites for each requested texture name.
    fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<crate::SpriteRef>> {
        vec![None; textures.len()]
    }
    /// GUI atlas region for `sprite`.
    fn gui_sprite(&self, _sprite: &crate::Identifier) -> Option<crate::AtlasRegion> {
        None
    }
    /// Monotonic version of the published block-sprites table.
    fn block_sprites_version(&self) -> u64 {
        0
    }
}

/// Phase-scoped context for `Hooks::add_client_reload_listeners`.
///
/// Initial client resource loading runs on a loader thread before any `Game` or published
/// asset tables exist, so this trait directly exposes only the process, mod-roster,
/// settings, math-profile, and local text facts available on both startup and later
/// reloads. On later reloads, [`published`](Self::published) returns `Some(&dyn ClientReloadHost)`
/// for reading the currently published client assets and registries; on initial startup it
/// returns `None`.
pub trait ClientReloadListenerContext {
    /// Physical process, not the logical side of this hook.
    fn dist(&self) -> crate::Dist {
        crate::Dist::Client
    }
    /// Exact Minecraft version supported by this process.
    fn game_version(&self) -> &'static str {
        ""
    }
    /// The engine's `DataVersion` written into level data and chunks.
    fn server_data_version(&self) -> i32 {
        0
    }
    /// Sunrise version printed in crash reports.
    fn sunrise_version(&self) -> &'static str {
        ""
    }
    /// Startup opt-in for test-only content.
    fn game_tests_enabled(&self) -> bool {
        false
    }
    /// Whether the player left mod `mod_id` enabled.
    fn on(&self, _mod_id: &str) -> bool {
        false
    }
    /// Whether mod `id` is loaded in this process.
    fn is_loaded(&self, _id: &str) -> bool {
        false
    }
    /// Read a registered setting through its owner-bound [`crate::SettingKey`].
    fn setting(
        &self,
        _key: crate::SettingKey,
    ) -> Result<crate::SettingValue, crate::SettingAccessError> {
        Err(crate::SettingAccessError::Unavailable)
    }
    /// Math backend profile active in this process.
    fn math_profile(&self) -> crate::MathProfile;
    /// Plain string form of `text` using the currently active client language (or fallback
    /// keys during initial startup before language assets are published).
    fn text_string(&self, _text: &crate::Text) -> String {
        String::new()
    }
    /// Published client assets and registries from the previous completed load.
    ///
    /// Returns `None` on initial startup before any client resources have been published,
    /// and `Some` on later reloads (F3+T, resource-pack changes, server resource packs).
    fn published(&self) -> Option<&dyn ClientReloadHost> {
        None
    }
}

/// Phase-scoped context for `Hooks::client_setup`.
///
/// Extends [`ClientReloadHost`] with client setup services (options, key mappings, audio,
/// local sound handles, text measurement, translations, client registries, config/data files,
/// and non-world window/view/screen options) without exposing world-only level, entity, or
/// gameplay mutation methods.
pub trait ClientSetupContext:
    ClientReloadHost
    + crate::prelude::AudioClientHost
    + crate::prelude::ClientInputClientHost
    + crate::prelude::ClientNarratorClientHost
    + crate::prelude::ClientOptionsClientHost
    + crate::prelude::ClientTextClientHost
    + crate::prelude::DrawClientHost
    + crate::prelude::PayloadsClientHost
    + crate::prelude::ScoreboardClientHost
{
    /// The game's shared `config/` directory, as in `ConfigContext::config_dir`.
    fn config_dir(&self) -> &std::path::Path;
    /// Translate a language key in the client's current language.
    fn translate(&self, _key: &str) -> Option<String> {
        None
    }
    /// Registered enchantments in registry order, including selection `weight` and `anvil_cost`
    /// (the base anvil fee per enchantment level). This and the other enchantment reads use the
    /// table built from the server's synced entries when available, and the local table otherwise.
    fn enchantments(&self) -> Vec<crate::EnchantmentInfo> {
        Vec::new()
    }
    /// Registered potions, in registry order.
    fn potions(&self) -> Vec<crate::PotionInfo> {
        Vec::new()
    }
    /// `Enchantment.getMinCost(level)` for `id`.
    fn enchantment_min_cost(&self, _id: &str, _level: u32) -> Option<i32> {
        None
    }
    /// Whether `item` is in enchantment `id`'s `supported_items` (`Enchantment.isSupportedItem`).
    /// Does not consult Item Part overrides.
    fn enchantment_supports(&self, _id: &str, _item: crate::ItemKey) -> bool {
        false
    }
    /// Every sound event the client's resources have (`SoundManager.getAvailableSounds`).
    fn sound_events(&self) -> Vec<crate::Identifier> {
        Vec::new()
    }
    /// The output devices sound can play on (`SoundEngine.getAvailableSoundDevices`).
    fn audio_devices(&self) -> Vec<String> {
        Vec::new()
    }
    /// The output device the options ask for (`Options.soundDevice`).
    fn audio_device(&self) -> String {
        String::new()
    }
    /// Play a local client sound (`SoundEngine.playDelayed`), returning its control handle.
    fn play_local_sound(&self, _sound: &crate::SoundPlay, _after: u32) -> Option<crate::SoundRef> {
        None
    }
    /// Muffle client sounds over `ticks` client ticks, leaving `exempt` events clear.
    fn muffle_sounds(
        &mut self,
        _amount: f32,
        _lowpass: f32,
        _ticks: u32,
        _exempt: &[&str],
    ) -> bool {
        false
    }
    /// The particles alive in the client's particle engine (`ParticleEngine.countParticles`).
    fn particle_count(&self) -> u32 {
        0
    }
    /// Clear active client particles and emitters (`ParticleEngine.clearParticles`).
    fn clear_particles(&self) -> bool {
        false
    }
    /// Current playback state of a local sound handle started with `play_local_sound`.
    fn sound_state(&self, _sound: crate::SoundRef) -> Option<crate::SoundState> {
        None
    }
    /// Stop a local sound handle.
    fn sound_stop(&self, _sound: crate::SoundRef) -> bool {
        false
    }
    /// Fade a local sound handle's volume to `volume` over `ticks` unpaused ticks.
    fn sound_fade(&self, _sound: crate::SoundRef, _volume: f32, _ticks: u32) -> bool {
        false
    }
    /// Move a playing positional local sound handle to `position`.
    fn sound_move_to(&self, _sound: crate::SoundRef, _position: [f64; 3]) -> bool {
        false
    }
    /// Set a playing local sound handle's pitch.
    fn sound_set_pitch(&self, _sound: crate::SoundRef, _pitch: f32) -> bool {
        false
    }
    /// Current binding name for a resolved vanilla key mapping handle.
    fn vanilla_key_mapping_binding(
        &self,
        _mapping: crate::VanillaKeyMappingRef,
    ) -> Result<&str, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Logical held state for a resolved vanilla gameplay key mapping handle.
    fn vanilla_key_mapping_is_down(
        &self,
        _mapping: crate::VanillaKeyMappingRef,
    ) -> Result<bool, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Physical key/button held state for a resolved vanilla key mapping handle.
    fn vanilla_key_mapping_physical_down(
        &self,
        _mapping: crate::VanillaKeyMappingRef,
    ) -> Result<bool, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Translated Controls label for a resolved vanilla key mapping handle.
    fn vanilla_key_mapping_translated_key_message(
        &self,
        _mapping: crate::VanillaKeyMappingRef,
    ) -> Result<crate::Text, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Current binding name and modifiers for a resolved mod key mapping handle.
    fn mod_key_mapping_binding(
        &self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<(&str, crate::ClientModifiers), crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// Held state for a resolved mod key mapping handle.
    fn mod_key_mapping_is_down(
        &self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<bool, crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// Translated Controls label for a resolved mod key mapping handle.
    fn mod_key_mapping_translated_key_message(
        &self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<crate::Text, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Consume one pending click on a resolved mod key mapping handle.
    fn mod_key_mapping_consume_click(
        &mut self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<bool, crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// Rebind an owned mod key mapping handle and persist it to `sunrise-mods.txt`.
    fn mod_key_mapping_set_binding(
        &mut self,
        _mapping: crate::ModKeyMappingRef,
        _key: &str,
        _modifiers: crate::ClientModifiers,
    ) -> Result<(), crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// Reset an owned mod key mapping handle to its declared default binding.
    fn mod_key_mapping_reset_binding(
        &mut self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<(), crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// The HUD tick counter (`Hud.getGuiTicks`).
    fn gui_ticks(&self) -> i32 {
        0
    }
    /// Effective HUD and hand visibility mode.
    fn hud_mode(&self) -> crate::HudMode {
        crate::HudMode::AllVisible
    }
    /// Select HUD and hand visibility mode.
    fn set_hud_mode(&mut self, _mode: crate::HudMode) {}
    /// Current camera perspective (`Options.getCameraType`).
    fn camera_type(&self) -> crate::CameraType {
        crate::CameraType::FirstPerson
    }
    /// Request the window's camera perspective.
    fn set_camera_type(&self, _camera_type: crate::CameraType) {}
    /// Whether the client is currently paused.
    fn is_paused(&self) -> bool {
        false
    }
    /// Configured menu background blur radius (`menuBackgroundBlurriness`).
    fn menu_blur_radius(&self) -> f32 {
        0.0
    }
    /// Last window focus state reported by the window system.
    fn window_focused(&self) -> bool {
        false
    }
    /// Whether the debug overlay is currently visible.
    fn debug_overlay_visible(&self) -> bool {
        false
    }
    /// Whether a physical key or mouse button is currently down.
    fn is_key_down(&self, _key: &str) -> bool {
        false
    }
    /// The same screen identifier provided to screen hooks, if a screen is open.
    fn observed_screen_id(&self) -> Option<crate::ScreenId> {
        None
    }
    /// The mod whose custom screen is currently open, if any.
    fn mod_screen(&self) -> Option<&str> {
        None
    }
    /// The open screen's title as plain text, if any.
    fn screen_title(&self) -> Option<String> {
        None
    }
}
