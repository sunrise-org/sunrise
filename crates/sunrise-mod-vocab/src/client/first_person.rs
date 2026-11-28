//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;

/// The posed first-person draw context passed to [`crate::RenderHandEvent`].
pub trait HandDrawContext: BlockEntityDrawContext {
    /// Appends the local avatar's physical left or right arm in its rest pose with the vanilla
    /// outward roll, using the frame's skin, model width and that side's sleeve setting. The
    /// current draw pose and fixed light apply, not the default hand's item, swing or equip pose.
    /// Either arm can be submitted on either hand callback. This does not fire `render_arm`,
    /// change draw state or cancel the default. If the avatar arm is unavailable, draws nothing.
    fn render_arm(&mut self, arm: MainHand);
}
