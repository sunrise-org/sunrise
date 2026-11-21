//! The `sprite_source` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entry of an atlas definition whose `type` a mod registered (`register_sprite_sources`,
    /// vanilla's `SpriteSource.run`), asked at the entry's place in the definition's `sources`,
    /// once per atlas each time the client prepares its resources (a load or a reload), on the
    /// resource loader; the answer is kept for every user of that load. It is asked for the game's
    /// blocks, items and GUI atlases as well as the atlases a mod registered. `config` is the entry's JSON object
    /// converted to a `Tag`, `atlas` the atlas being listed, and `resources` the load's resources,
    /// every pack's, read-only and valid for this call. Push `SpriteSourceOutput`s to `output`
    /// only for a source your mod owns; they apply in order once every mod has answered. What
    /// the entries before it listed is not shown: output is write-only, as vanilla's is.
    #[hook(
        side = "client",
        version = 2,
        contract = "collect",
        out = "output",
        context = "registration",
        neoforge = "none (vanilla SpriteSource.run of a type RegisterSpriteSourcesEvent registers; NeoForge adds no event for it)",
        target = "sunrise_client_render::mod_sprite_sources::native_sprite_source"
    )]
    fn sprite_source();
}
