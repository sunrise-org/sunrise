use crate::{DynamicTextureError, Identifier};

/// The view that is submitting the player's built-in layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlayerLayerView {
    /// A player in the world, including the local third-person avatar.
    World,
    /// The local avatar in the survival or creative inventory.
    InventoryPreview,
}

/// A built-in player layer whose texture can be selected for a draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlayerLayer {
    /// The cape, which may have no profile image.
    Cape,
    /// The wings of worn equipment, such as an elytra.
    Wings,
}

/// A texture proposal for the current player, layer and view.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum PlayerLayerChoice<'a> {
    /// Restore the renderer's normal texture choice, even if a previous listener changed it.
    Keep,
    /// Draw with this mod's live dynamic texture. Both layers use a 64×32 layout;
    /// integer upscales of that layout are also accepted.
    UseTexture(&'a Identifier),
    /// For wings only: bypass the profile texture and use the worn equipment's texture path.
    UseEquipmentTexture,
}

/// Why a texture proposal did not replace the previous accepted choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlayerLayerRefusal {
    /// Only wings can use an equipment texture.
    WrongLayer,
    /// The image is not owned by the calling mod, does not exist, or has no slot in this atlas.
    Texture(DynamicTextureError),
    /// The image dimensions do not match the built-in UV layout or an integer upscale of it.
    IncompatibleLayout,
}

/// Engine context holding a stageable copy of the draw's texture decision.
#[doc(hidden)]
pub struct PlayerLayerSelection<'a> {
    layer: PlayerLayer,
    resolve: &'a dyn Fn(&str, &Identifier) -> Result<[f32; 4], PlayerLayerRefusal>,
    subscriber: &'static str,
    answer: ResolvedPlayerLayerTexture,
}

impl crate::ResolvePlayerLayerEvent<'_, '_> {
    /// Propose a choice. An error leaves the previous accepted choice intact; doing
    /// nothing leaves it intact too. The last accepted proposal from a successful
    /// listener wins. Only this draw is affected.
    pub fn choose(&mut self, choice: PlayerLayerChoice<'_>) -> Result<(), PlayerLayerRefusal> {
        let selection = &mut *self.selection;
        selection.answer = match choice {
            PlayerLayerChoice::Keep => ResolvedPlayerLayerTexture::Keep,
            PlayerLayerChoice::UseTexture(id) => {
                ResolvedPlayerLayerTexture::Texture((selection.resolve)(selection.subscriber, id)?)
            }
            PlayerLayerChoice::UseEquipmentTexture if selection.layer == PlayerLayer::Wings => {
                ResolvedPlayerLayerTexture::Equipment
            }
            PlayerLayerChoice::UseEquipmentTexture => return Err(PlayerLayerRefusal::WrongLayer),
        };
        Ok(())
    }
}

impl PlayerLayerSelection<'_> {
    pub(crate) fn __stage(&self, subscriber: &'static str) -> Self {
        Self {
            layer: self.layer,
            resolve: self.resolve,
            subscriber,
            answer: self.answer,
        }
    }
}

#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum ResolvedPlayerLayerTexture {
    /// Run the normal texture selection.
    Keep,
    /// Sample this atlas rectangle instead.
    Texture([f32; 4]),
    /// Select wings as if there were no profile texture.
    Equipment,
}

#[doc(hidden)]
pub fn player_layer_selection(
    layer: PlayerLayer,
    resolve: &dyn Fn(&str, &Identifier) -> Result<[f32; 4], PlayerLayerRefusal>,
) -> PlayerLayerSelection<'_> {
    PlayerLayerSelection {
        layer,
        resolve,
        subscriber: "",
        answer: ResolvedPlayerLayerTexture::Keep,
    }
}

#[doc(hidden)]
pub fn resolved_player_layer_texture(
    selection: PlayerLayerSelection<'_>,
) -> ResolvedPlayerLayerTexture {
    selection.answer
}
