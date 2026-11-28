//! Absolute model arm rotations for custom humanoid arm-pose transforms.

/// Absolute model arm rotations in radians. Each non-finite component leaves
/// that component of the ordinary humanoid pose unchanged.
// no wire: used by the local arm-pose callback.
// exhaustive: mods write either arm's three model rotation components
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmAngles {
    /// Right arm pitch, yaw and roll.
    pub right: [f32; 3],
    /// Left arm pitch, yaw and roll.
    pub left: [f32; 3],
}
