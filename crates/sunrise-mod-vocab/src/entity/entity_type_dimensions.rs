//! An entity type's registered size, as `ItemsHost::entity_type_dimensions` answers it.

/// An entity type's registered size (`EntityType.getDimensions()`), what
/// `ItemsHost::entity_type_dimensions` answers before any entity of the type exists. It does not
/// carry `EntityType.spawnDimensionsScale`, which `getSpawnAABB` multiplies the size by.
// no wire: a local read of the game's entity type table
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct EntityTypeDimensions {
    /// `EntityDimensions.width()`: the box's side on x and z.
    pub width: f32,
    /// `EntityDimensions.height()`.
    pub height: f32,
    /// `EntityDimensions.eyeHeight()`.
    pub eye_height: f32,
}
impl EntityTypeDimensions {
    /// `EntityDimensions.scalable(width, height).withEyeHeight(eye_height)`.
    pub fn new(width: f32, height: f32, eye_height: f32) -> Self {
        Self {
            width,
            height,
            eye_height,
        }
    }
}
