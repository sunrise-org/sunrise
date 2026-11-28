//! `Entity.saveWithoutId`: the fields every saved entity has, whoever simulates it.
use sunrise_core::geometry::Vec3;

/// The fields `Entity.saveWithoutId` writes, in one record.
///
/// A simulator embeds this (or lends it through [`HasEntityFields`]), so the NBT
/// codec reads and writes one struct instead of the simulator's own body. Units are
/// vanilla's: blocks for positions and distances, degrees for the rotation, ticks for
/// the timers.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityFields {
    /// `Pos`: the entity's position in its level.
    pub position: Vec3,
    /// `Motion`: the velocity the entity moves with each tick.
    pub motion: Vec3,
    /// `Rotation`'s first float: `Entity.getYRot`, turning left.
    pub yaw: f32,
    /// `Rotation`'s second float: `Entity.getXRot`, positive looking down.
    pub pitch: f32,
    /// `fall_distance`: blocks fallen since the entity last landed (vanilla stores a double; the
    /// NBT key is the lowercase `fall_distance`, as the field is named).
    pub fall_distance: f64,
    /// `Fire`: remaining fire ticks; negative while the entity is in fire but not burning.
    pub fire: i32,
    /// `Air`: remaining air ticks before drowning damage.
    pub air: i32,
    /// `OnGround`: whether the entity last moved while touching the ground.
    pub on_ground: bool,
    /// `Invulnerable`: whether non-`#bypasses_invulnerability` damage is ignored.
    pub invulnerable: bool,
    /// `PortalCooldown`: ticks before the entity may use a portal again.
    pub portal_cooldown: i32,
    /// `UUID`: the entity's identity, as the four ints vanilla writes.
    pub uuid: u128,
}

/// The entity fields of a simulator's own body, lent to the codecs.
///
/// A simulator keeps its state as it likes; a save codec asks for this and reads or
/// writes the record. Both directions are needed: loading writes the fields, saving
/// reads them.
pub trait HasEntityFields {
    fn entity_fields(&self) -> &EntityFields;
    fn entity_fields_mut(&mut self) -> &mut EntityFields;
}
