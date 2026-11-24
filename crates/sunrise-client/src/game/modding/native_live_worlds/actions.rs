//! Ordinary player turns in a transformed source context. The physical player body stays put.
use super::*;
use crate::game::{Dim, GuestUse, Hit, transactions::StorageSettlement};
use mod_api::{Hand, PlayerRef};

#[derive(Clone, Copy)]
pub(super) struct ProjectedAction {
    pub handle: LiveWorldRef,
    pub player: PlayerRef,
    pub local_eye: [f64; 3],
    pub(super) hit: LiveWorldHit,
    pub(super) membership: u64,
    physical_eye: [f64; 3],
    physical_level: LevelRef,
    physical_conn: Option<u32>,
    player_writes: u64,
    action_writes: u64,
    mode: sunrise_client::command::args::GameMode,
    selected: usize,
    permission: Option<u8>,
    spawn_protection: Option<i32>,
    break_writes: Option<u64>,
}

impl Game<'_> {
    pub(in crate::game) fn live_world_action_eye(&self) -> Option<[f64; 3]> {
        let table = self.live_worlds.borrow();
        let action = table.action?;
        (self.dim.key() == action.handle.level().dimension()
            && self.native_acting_player() == Some(action.player))
        .then_some(action.local_eye)
    }
    pub(in crate::game) fn live_world_action_in_level(&self) -> bool {
        self.live_world_action_eye().is_some()
    }

    /// Runs after ordinary callbacks and again immediately before the real removal.
    /// It never re-selects a target or replays an event. Vanilla actions have no projection scope.
    pub(in crate::game) fn projection_break_fresh(&self, pos: [i32; 3]) -> bool {
        if !self.projection_action_fresh() {
            return false;
        }
        let table = self.live_worlds.borrow();
        let Some(action) = table.action else {
            return true;
        };
        let Some(writes) = action.break_writes else {
            return true;
        };
        let Some(entry) = table.entries.get(&action.handle.generation()) else {
            return false;
        };
        entry.handle == action.handle
            && self.level_handle_current(action.handle.level())
            && self.level_handle_current(entry.destination)
            && pos == action.hit.cell
            && self.world.revision == action.hit.cell_revision
            && entry.sample.revision == action.hit.pose_revision
            && entry.membership == action.membership
            && entry.cells.binary_search(&pos).is_ok()
            && self.transactions.writes() == writes
            && self.native_acting_player() == Some(action.player)
    }

    fn physical_projection_eye(&self) -> Option<[f64; 3]> {
        let player = self.player.as_ref()?;
        Some([
            player.position.x,
            player.position.y + f64::from(player.dimensions.eye_height),
            player.position.z,
        ])
    }

    /// Called after callbacks and immediately before real ordinary mutations. The physical body
    /// and its permissions remain authoritative even while the source eye is contextual.
    pub(in crate::game) fn projection_action_fresh(&self) -> bool {
        let Some(action) = self.live_worlds.borrow().action else {
            return true;
        };
        self.projection_action_inputs_fresh(action)
    }
    pub(super) fn projection_action_inputs_fresh(&self, action: ProjectedAction) -> bool {
        self.transactions.writes() == action.action_writes
            && self.projection_actor_target_fresh(action)
    }

    /// Menu effects may legitimately journal their source BEs; player mutation and target
    /// identity still cannot change across the ordinary effect or guest handoff.
    pub(super) fn projection_menu_effect_fresh(&self) -> bool {
        self.live_worlds
            .borrow()
            .action
            .is_none_or(|action| self.projection_actor_target_fresh(action))
    }

    pub(super) fn projection_actor_target_fresh(&self, action: ProjectedAction) -> bool {
        self.native_acting_player() == Some(action.player)
            && self
                .player
                .as_ref()
                .is_some_and(|player| !player.health.dead_or_dying())
            && self.physical_projection_eye() == Some(action.physical_eye)
            && self.transactions.player_writes() == action.player_writes
            && self.mode == action.mode
            && self.selected == action.selected
            && self.lan_acting_permission == action.permission
            && self.lan_acting_spawn_protection == action.spawn_protection
            && self
                .live_worlds
                .borrow()
                .entries
                .get(&action.handle.generation())
                .is_some_and(|entry| entry.membership == action.membership)
            && self
                .check_projected_actor(action.handle, action.player, action.hit)
                .is_ok()
    }

    pub(in crate::game) fn projection_physical_player_level(
        &self,
        player: PlayerRef,
    ) -> Option<LevelRef> {
        let table = self.live_worlds.borrow();
        let action = table.action?;
        let entry = table.entries.get(&action.handle.generation())?;
        if action.player != player
            || self.native_acting_player() != Some(player)
            || action.physical_conn != self.lan_acting
            || self.dim.key() != action.handle.level().dimension()
            || entry.handle != action.handle
            || entry.destination != action.physical_level
            || !self.live_world_lifecycle_current(action.handle.level())
            || !self.live_world_lifecycle_current(action.physical_level)
            || self.native_lifecycle.epoch() != Some(player.entity().epoch())
        {
            return None;
        }
        if let Some(conn) = action.physical_conn {
            let guest = self.lan.as_ref()?.hub.guest(conn)?;
            if guest.uuid != player.uuid()
                || u32::try_from(guest.entity_id).ok() != Some(player.entity().id())
                || guest.dimension != action.physical_level.dimension_id()
                || !guest.placed
                || guest.dead
                || !guest.participates_in_player_simulation()
            {
                return None;
            }
        }
        Some(action.physical_level)
    }

    /// Authoritative token + current real player eye/reach + ordinary static occlusion.
    fn check_projected_actor(
        &self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
    ) -> Result<LiveWorldPose, Error> {
        self.live_world_validate_hit_resolved(handle, hit)?;
        let entry = self.resolved_projection_entry(handle)?;
        if self.player_level(player) != Some(entry.destination)
            || !self.native_player_current(player)
        {
            return Err(Error::Stale);
        }
        let eye = self.physical_projection_eye().ok_or(Error::Unavailable)?;
        let direction = vector(hit.point).subtract(vector(eye));
        let range = self.reach() + 1.0;
        if !range.is_finite() || direction.length_squared() >= range * range {
            return Err(Error::OutOfReach);
        }
        if direction.length_squared() <= 1.0e-12 {
            return Err(Error::Invalid);
        }
        let end = vector(hit.point).add_vector(direction.normalize().scale(1.0e-5));
        let geometry = self.projection_geometry(&entry)?;
        let actual = geometry
            .contact
            .clip(&checked_pose(entry.sample.current)?, vector(eye), end)
            .ok_or(Error::Revision)?;
        if actual.local_cell != position(hit.cell)
            || actual.local_direction != hit.face
            || actual.location.distance_squared(vector(hit.point)) > 1.0e-10
        {
            return Err(Error::Revision);
        }
        // The caller cannot select a farther owner through a nearer live wall. The same
        // bounded real geometry used for drawing/contact is authoritative on the server.
        let mut remaining = entry.limits.contacts as usize;
        remaining = remaining
            .checked_sub(geometry.boxes.boxes().len())
            .ok_or(Error::Budget)?;
        let table = self.live_worlds.borrow();
        for other in table
            .entries
            .values()
            .filter(|other| other.handle != handle && other.destination == entry.destination)
        {
            let geometry = self.projection_geometry(other)?;
            remaining = remaining
                .checked_sub(geometry.boxes.boxes().len())
                .ok_or(Error::Budget)?;
            if let Some(nearer) =
                geometry
                    .contact
                    .clip(&checked_pose(other.sample.current)?, vector(eye), end)
                && nearer.location.distance_squared(vector(eye)) + 1.0e-10
                    < direction.length_squared()
            {
                return Err(Error::Obstructed);
            }
        }
        drop(table);
        let obstruction = self
            .level_clip(
                entry.destination,
                eye,
                hit.point,
                mod_api::ClipBlock::Collider,
                mod_api::ClipFluid::None,
            )
            .ok_or(Error::NotResident)?;
        if !obstruction.miss
            && vector(obstruction.location).distance_squared(vector(eye)) + 1.0e-10
                < direction.length_squared()
        {
            return Err(Error::Obstructed);
        }
        let physical = hit.point.map(|v| v.floor() as i32);
        if self.lan_acting_spawn_protection.is_some_and(|radius| {
            self.spawn_protects(radius, entry.destination.dimension_id(), physical)
        }) {
            return Err(Error::Denied);
        }
        Ok(entry.sample.current)
    }

    pub(super) fn with_projected_actor<T>(
        &mut self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        operation: impl FnOnce(&mut Self, Hit) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if !self.projection_action_transaction_allowed(handle.level()) {
            return Err(Error::Busy);
        }
        if self.live_worlds.borrow().action.is_some() {
            return Err(Error::Busy);
        }
        // Resolve ownership before borrowing the player. No client or foreign owner gets this turn.
        self.resolved_projection_entry(handle)?;
        let mut answer = Err(Error::Unavailable);
        let ran = self.native_as_player_handle(player, |game| {
            answer = (|| {
                let pose = game.check_projected_actor(handle, player, hit)?;
                let actual_eye = game.physical_projection_eye().ok_or(Error::Unavailable)?;
                let physical_level = game.player_level(player).ok_or(Error::Stale)?;
                let building = game.capture_projection_building(handle, pose)?;
                let local_eye = checked_pose(pose)?
                    .to_local(vector(actual_eye))
                    .components();
                let membership = game.resolved_projection_entry(handle)?.membership;
                let target = Dim::from_id(handle.level().dimension_id());
                let back = game.enter_command_level(target);
                if game.dim != target {
                    return Err(Error::Unavailable);
                }
                let view = vector(crate::game::view_vector(game.yaw, game.pitch));
                let rotation = checked_pose(pose)?.rotation();
                let local_view = rotation.inverse_rotate(view);
                let old_angles = (game.yaw, game.pitch);
                game.yaw = (-local_view.x).atan2(local_view.z).to_degrees() as f32;
                game.pitch = (-local_view.y)
                    .atan2((local_view.x * local_view.x + local_view.z * local_view.z).sqrt())
                    .to_degrees() as f32;
                game.live_worlds.get_mut().action = Some(ProjectedAction {
                    handle,
                    player,
                    local_eye,
                    hit,
                    membership,
                    break_writes: None,
                    physical_eye: actual_eye,
                    physical_level,
                    physical_conn: game.lan_acting,
                    player_writes: game.transactions.player_writes(),
                    action_writes: game.transactions.writes(),
                    mode: game.mode,
                    selected: game.selected,
                    permission: game.lan_acting_permission,
                    spawn_protection: game.lan_acting_spawn_protection,
                });
                game.live_worlds.get_mut().building = Some(building);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    operation(
                        game,
                        Hit {
                            pos: hit.cell,
                            face: hit.face.normal(),
                            point: hit.local_point,
                            inside: false,
                        },
                    )
                }));
                let result = match result {
                    Ok(answer) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if answer.is_ok() {
                            game.capture_projection_menu_open();
                        }
                        answer
                    })),
                    Err(panic) => Err(panic),
                };
                game.live_worlds.get_mut().building = None;
                game.live_worlds.get_mut().action = None;
                (game.yaw, game.pitch) = old_angles;
                if let Some(back) = back {
                    game.enter_level(back).map_err(|_| Error::Unavailable)?;
                }
                match result {
                    Ok(result) => result,
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            })();
        });
        if !ran {
            return Err(Error::Stale);
        }
        answer
    }

    pub(in crate::game) fn live_world_destroy_block(
        &mut self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        into_inventory: bool,
    ) -> Result<bool, Error> {
        if self.has_open_transaction() {
            return Err(Error::Busy);
        }
        self.projection_entry(handle)?;
        self.live_world_destroy_block_resolved(handle, player, hit, into_inventory)
    }

    pub(in crate::game) fn live_world_destroy_block_resolved(
        &mut self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        into_inventory: bool,
    ) -> Result<bool, Error> {
        self.with_projected_actor(handle, player, hit, |game, hit| {
            let scope = game.storage_scope_guard(true);
            let writes = scope.game.transactions.writes();
            scope
                .game
                .live_worlds
                .get_mut()
                .action
                .as_mut()
                .expect("scoped action")
                .break_writes = Some(writes);
            let removed = scope
                .game
                .player_destroy_block(player, hit.pos, into_inventory);
            let settled = scope.close(removed);
            if removed && !matches!(settled, Some(StorageSettlement::Committed(_))) {
                return Err(Error::Unavailable);
            }
            Ok(removed)
        })
    }

    pub(in crate::game) fn live_world_use_block(
        &mut self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        hand: Hand,
    ) -> Result<bool, Error> {
        self.projection_entry(handle)?;
        self.live_world_use_block_resolved(handle, player, hit, hand)
    }

    pub(in crate::game) fn live_world_use_block_resolved(
        &mut self,
        handle: LiveWorldRef,
        player: PlayerRef,
        hit: LiveWorldHit,
        hand: Hand,
    ) -> Result<bool, Error> {
        // Ordinary menu screens are published by this operation's own source turn. A caller's
        // outer savepoint could later abort after that publication, so refuse before any effect.
        if self.has_open_transaction() {
            return Err(Error::Busy);
        }
        self.with_projected_actor(handle, player, hit, |game, hit| {
            let old_hand = game.hand;
            let old_use = game.guest_use;
            game.hand = hand;
            game.guest_use = Some(GuestUse::on_block(hand));
            let target = game.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let name = &game.res.facts().get(target).name;
            let menu = matches!(
                &**name,
                "minecraft:chest"
                    | "minecraft:trapped_chest"
                    | "minecraft:ender_chest"
                    | "minecraft:furnace"
                    | "minecraft:blast_furnace"
                    | "minecraft:smoker"
            );
            let item_supported = game.held().is_none_or(|stack| {
                game.res
                    .items()
                    .get(&stack.item)
                    .is_some_and(|item| item.block.is_some())
            });
            if !item_supported && !(menu && !game.keys.held(sunrise_client::actions::Action::Sneak))
            {
                game.hand = old_hand;
                game.guest_use = old_use;
                return Err(Error::Unsupported);
            }
            if !menu
                && (game.block_opens_menu(&hit)
                    || game
                        .mod_menu_at((hit.pos[0], hit.pos[1], hit.pos[2]))
                        .is_some())
                && !game.keys.held(sunrise_client::actions::Action::Sneak)
            {
                game.hand = old_hand;
                game.guest_use = old_use;
                return Err(Error::Unsupported);
            }
            let scope = game.storage_scope_guard(true);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                scope.game.use_hand(Some(&hit))
            }));
            if scope.game.container.is_some() && !scope.game.projection_menu_effect_fresh() {
                scope.game.use_failed = true;
            }
            let denied = scope.game.use_failed;
            let accepted = result
                .as_ref()
                .is_ok_and(|result| result.as_ref().is_ok_and(|used| *used))
                && !denied;
            if !accepted {
                scope.game.close_ender_chest();
                scope.game.container = None;
            }
            let settled = scope.close(accepted);
            game.hand = old_hand;
            game.guest_use = old_use;
            if accepted && !matches!(settled, Some(StorageSettlement::Committed(_))) {
                game.close_ender_chest();
                game.container = None;
                return Err(Error::Unavailable);
            }
            match result {
                Ok(Ok(_)) if denied => Err(Error::Denied),
                Ok(result) => result.map_err(|_| Error::Unavailable),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        })
    }
}
