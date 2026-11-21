//! `/give`'s handing over of one stack.
use super::{Game, Target};
use sunrise_client::items::ItemStack;
use sunrise_core::random::RandomSource;

impl Game<'_> {
    /// `GiveCommand.giveItem`'s handing over of one stack to the acting player:
    /// `Inventory.add`, then what did not fit thrown from the eyes with no pickup delay and
    /// `owner` (the player's UUID) its only taker, or the pickup sound when it all went in.
    /// For a player in another level than the one in place nothing lands in the world: what
    /// did not fit is returned and no sound plays.
    pub(in crate::game) fn give_acting_stack(
        &mut self,
        stack: ItemStack,
        owner: Option<[i32; 4]>,
    ) -> Option<ItemStack> {
        let uuid = self.native_acting_id();
        self.journal_player(uuid);
        let left = self.add_stack_to_inventory(stack);
        if !self.acting_in_level_in_place() {
            return left;
        }
        // At the player who gets them, in the level in place now: a root commit may replay
        // this after the command put the caller's level back.
        let dim = self.dim;
        if let Some(rest) = left {
            let eye = self.give_eye();
            self.after_commit(crate::game::transactions::After::GiveRest {
                uuid,
                rest,
                owner,
                dim,
                eye,
            });
        } else {
            self.after_commit(crate::game::transactions::After::GivePickup(uuid, dim));
        }
        None
    }
    pub(in crate::game) fn give_pickup(&mut self) {
        let p = self
            .target_position(Target::Player)
            .unwrap_or(self.source_position());
        // GiveCommand uses the receiving player's random, not the level's.
        let mut r = sunrise_client::random_source::entity();
        let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
        self.audio
            .play("minecraft:entity.item.pickup", Some(p), 0.2, pitch);
    }
    /// The receiving player's eyes, where what did not fit is thrown from.
    fn give_eye(&self) -> [f64; 3] {
        let p = self
            .target_position(Target::Player)
            .unwrap_or(self.source_position());
        self.eye_position()
            .unwrap_or([p[0], p[1] + self.target_eye_height(Target::Player), p[2]])
    }
    pub(in crate::game) fn give_remainder(&mut self, rest: ItemStack, owner: Option<[i32; 4]>) {
        // Inventory full: Player.createItemStackToDrop (thrown from
        // the eyes), then setNoPickUpDelay and setTarget(player).
        let eye = self.give_eye();
        self.throw_from_eyes(rest, eye, false, false);
        if let Some(item) = self.just_spawned_item() {
            item.thrower = None;
            item.pickup_delay = 0;
            item.target = owner;
        }
    }
    /// What did not fit of a `/give` whose player died or left between taking the stack and
    /// the root commit that throws it: it lies where it would have been thrown from, at
    /// `eye` as it was when the stack was taken, still with no pickup delay and `owner` its
    /// only taker. The stack was already taken from its source, so it lands rather than
    /// being lost.
    pub(in crate::game) fn give_remainder_unheld(
        &mut self,
        rest: ItemStack,
        owner: Option<[i32; 4]>,
        eye: [f64; 3],
    ) {
        let at = [eye[0], eye[1] - f64::from(0.3f32), eye[2]];
        self.spawn_item(rest, at, [0.0; 3], 0);
        if let Some(item) = self.just_spawned_item() {
            item.target = owner;
        }
    }
}
