//! The keys whose presses send a packet on a server's client (swap hands, drop, a mount's
//! inventory): queued by the key handling, acted on in the tick's `handleKeybinds` phase.
use super::{Game, S};
use sunrise_client::input_queue::QueueLimits;
use sunrise_protocol::packets::play;
use winit::keyboard::KeyCode;

/// The bounds of `Remote::pending_keys`: a tick acts on 64 presses, oldest first, and 1024 wait at
/// most. A player presses a few keys a tick.
pub(super) const PENDING_KEYS: QueueLimits = QueueLimits {
    name: "remote key presses",
    max_len: 1024,
    per_tick: 64,
};

/// `handleKeybinds`' order for the keys queued here: the inventory key, swap hands, then drop.
const KEY_ORDER: [KeyCode; 3] = [KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyQ];

impl Game<'_> {
    /// handleKeybinds' keybind polls, before its attack, use and pick: each key's presses
    /// (`consumeClick` until none is left) in the fixed order of `KEY_ORDER`, whatever order
    /// they were pressed in, and all within the tick, so no packet goes out between
    /// CLIENT_TICK_END and a pong.
    pub(super) fn remote_queued_keys(&mut self) {
        let pending = self
            .remote
            .as_mut()
            .map(|r| r.pending_keys.take_tick())
            .unwrap_or_default();
        for key in KEY_ORDER {
            for _ in pending.iter().filter(|&&pressed| pressed == key) {
                self.remote_key_press(key);
            }
        }
    }
    fn remote_key_press(&mut self, key: KeyCode) {
        match key {
            KeyCode::KeyF if !self.spectator() => self.remote_swap_hands(),
            KeyCode::KeyQ if !self.spectator() => {
                // Minecraft.hasControlDown: the physical keys, so a toggled
                // sprint on Left Control does not make Q drop the stack.
                let all = self.has_control_down() || self.input_takes_stack_drop();
                self.remote_drop(all);
            }
            KeyCode::KeyE
                if !self.in_portal_transition()
                    && (self.riding_mount_with_menu() || self.riding_chest_boat()) =>
            {
                let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
                self.send(S::PlayerCommand(play::PlayerCommand {
                    id,
                    action: 5,
                    data: 0,
                }));
            }
            _ => {}
        }
    }
    /// `Minecraft.tick`'s `missTime` count, after `handleKeybinds`.
    pub(super) fn remote_count_miss_time(&mut self) {
        if self.clicks.miss_time > 0 {
            self.clicks.miss_time -= 1;
        }
    }
}
