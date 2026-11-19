//! Models a client mod attaches to a block position (`ClientRenderClientHost::set_block_extra_models`):
//! standalone models the chunk mesher draws as part of the block there, chosen at run time, and
//! why a call was refused.

/// The most models one block position holds: a longer list is
/// [`BlockExtraModelsError::TooMany`].
pub const BLOCK_EXTRA_MODELS_MAX: usize = 64;

/// Why `set_block_extra_models` changed nothing. The position keeps the models it had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockExtraModelsError {
    /// The client holds no column at the position, or the position is outside the level's height.
    NotLoaded,
    /// The list holds more than [`BLOCK_EXTRA_MODELS_MAX`] models.
    TooMany,
    /// A handle is stale or not a resolved standalone model.
    Model(crate::ModelError),
    /// Called outside the client's running game (the World phase): in a resource reload
    /// (`client_resources_reloaded`), setup or config loading. Native and wasm mods both get it.
    Unavailable,
}
