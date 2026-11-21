//! What a mod's own atlas sprite source reads and answers (`sprite_source`).
use crate::Identifier;

/// The resources of the load that is listing an atlas, read-only (the `ResourceManager`
/// vanilla's `SpriteSource.run` receives): every selected pack of this load in order, the game's
/// own, the mods' and the player's. Names are full resource paths
/// (`assets/<namespace>/textures/<path>.png`). It is given to `Hooks::sprite_source` and valid
/// for that call only: a read after it returns finds nothing.
pub trait ResourceReader: Send + Sync + 'static {
    /// The winning copy of `path`, the topmost pack's (`ResourceManager.getResource`); `None`
    /// where no pack provides it.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// Every pack's copy of `path`, lowest first, the winning one last
    /// (`ResourceManager.getResourceStack`); empty where no pack provides it.
    fn read_stack(&self, path: &str) -> Vec<Vec<u8>>;
    /// The resource paths that start with `prefix`, one each, in name order
    /// (`ResourceManager.listResources`).
    fn list(&self, prefix: &str) -> Vec<String>;
}

/// One step a mod's sprite source takes on the atlas it is listed in, as vanilla's
/// `SpriteSource.Output` takes them. `Hooks::sprite_source` pushes them in order, and they apply
/// under the built-in sources' rule: a sprite of an id listed already replaces the earlier one.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods construct outputs and the game matches every kind
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SpriteSourceOutput {
    /// `add(Identifier, Resource)`: sprite `sprite` is the texture `texture`
    /// (`assets/<namespace>/textures/<path>.png`), with its `.mcmeta`. A texture the resources
    /// lack is logged and skipped.
    Texture {
        /// The sprite's id in the atlas (`mymod:gem`).
        sprite: Identifier,
        /// The texture it is (`mymod:item/gem`).
        texture: Identifier,
    },
    /// `add(Identifier, DiscardableLoader)`: sprite `sprite` is these pixels, `width` by `height`,
    /// rows top first, four bytes (red, green, blue, alpha) a pixel. Pixels whose length isn't
    /// `width * height * 4`, or a side over the largest a sprite can have, are logged and skipped.
    Pixels {
        /// The sprite's id in the atlas.
        sprite: Identifier,
        /// Its width in pixels.
        width: u32,
        /// Its height in pixels.
        height: u32,
        /// Its RGBA bytes.
        rgba: Vec<u8>,
    },
    /// `removeAll`: drops sprite `sprite` from what the sources before listed. An id not listed
    /// changes nothing. A source never reads what was listed: it names the ids it wants gone.
    Remove {
        /// The sprite to drop.
        sprite: Identifier,
    },
}
