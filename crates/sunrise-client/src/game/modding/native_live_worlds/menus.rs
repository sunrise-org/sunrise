//! Continuing ordinary block menus in their actual source World. Only identity/context is stored here.
use super::*;
use crate::game::{Dim, lan_actions::MenuOpen, transactions::StorageSettlement};
use mod_api::__private::game::Fields as _;
use sunrise_client::lan::{guest::Guest, menu::Slots};

#[derive(Clone)]
pub(super) struct ProjectedMenu {
    handle: LiveWorldRef,
    destination: LevelRef,
    uuid: u128,
    window: i32,
    registry_id: Box<str>,
    slots: Slots,
    membership: u64,
    holders: Vec<mod_api::BlockEntityRef>,
}

pub(in crate::game) struct PreparedMenu(ProjectedMenu);

#[derive(Clone, Copy)]
pub(super) struct PendingOpen {
    action: actions::ProjectedAction,
    pos: (i32, i32, i32),
    destination: LevelRef,
    canceled: bool,
}

#[derive(Clone, Copy)]
pub(super) struct MenuScope {
    pub conn: u32,
    pub handle: LiveWorldRef,
    pub destination: LevelRef,
    uuid: u128,
    window: i32,
    pose_revision: u64,
    membership: u64,
    source_revision: u64,
    physical_eye: [f64; 3],
}

fn keys(slots: Slots, pos: Option<(i32, i32, i32)>) -> Vec<[i32; 3]> {
    match slots {
        Slots::Block(k) => vec![[k.0, k.1, k.2]],
        Slots::Double(a, b) => vec![[a.0, a.1, a.2], [b.0, b.1, b.2]],
        Slots::Ender => pos.into_iter().map(|k| [k.0, k.1, k.2]).collect(),
        Slots::Native => Vec::new(),
    }
}

impl Game<'_> {
    /// The ordinary guest turn hands Game.container to LAN only after restoring the body.
    /// Carry identity across that boundary, never slots or a second player inventory.
    pub(super) fn capture_projection_menu_open(&mut self) {
        let (Some(conn), Some(pos)) = (self.lan_acting, self.container) else {
            return;
        };
        let Some(action) = self.live_worlds.borrow().action else {
            return;
        };
        let destination = {
            self.resolved_projection_entry(action.handle)
                .ok()
                .map(|entry| entry.destination)
        };
        let Some(destination) = destination else {
            self.close_ender_chest();
            self.container = None;
            return;
        };
        // A guest's temporary ender inventory must be closed in its actual source frame.
        self.close_ender_chest();
        self.live_worlds.get_mut().pending_opens.insert(
            conn,
            PendingOpen {
                action,
                pos,
                destination,
                canceled: false,
            },
        );
    }

    pub(in crate::game) fn finish_projection_menu_open(
        &mut self,
        conn: u32,
        pos: (i32, i32, i32),
    ) -> Option<Result<bool, Error>> {
        let pending = self.live_worlds.get_mut().pending_opens.remove(&conn)?;
        let action = pending.action;
        Some(if pending.canceled || pending.pos != pos {
            Err(Error::Revision)
        } else {
            self.with_projected_actor(action.handle, action.player, action.hit, |game, _| {
                let scope = game.storage_scope_guard(true);
                if !scope.game.projection_actor_target_fresh(action) {
                    return Err(Error::Revision);
                }
                let opened = scope.game.lan_open_container(conn, pos) == MenuOpen::Opened;
                let settled = scope.close(opened);
                if opened && !matches!(settled, Some(StorageSettlement::Committed(_))) {
                    game.forget_projection_menu(conn);
                    return Err(Error::Unavailable);
                }
                Ok(opened)
            })
        })
    }

    /// Prepare after all open/lock/start callbacks, before any OpenScreen publication.
    /// This bounded slice supports only stock chests/ender chests and furnace menus.
    pub(in crate::game) fn prepare_projection_menu(
        &self,
        conn: u32,
        registry_id: &str,
        slots: Slots,
        pos: (i32, i32, i32),
    ) -> Result<Option<PreparedMenu>, Error> {
        let table = self.live_worlds.borrow();
        let Some(action) = table.action else {
            return Ok(None);
        };
        if !matches!(
            registry_id,
            "minecraft:generic_9x3"
                | "minecraft:generic_9x6"
                | "minecraft:furnace"
                | "minecraft:blast_furnace"
                | "minecraft:smoker"
        ) || !self.projection_menu_effect_fresh()
        {
            return Err(Error::Unsupported);
        }
        let entry = table
            .entries
            .get(&action.handle.generation())
            .ok_or(Error::Stale)?;
        let guest = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .ok_or(Error::Stale)?;
        if guest.uuid != action.player.uuid() || guest.dimension != entry.destination.dimension_id()
        {
            return Err(Error::Stale);
        }
        let cells = keys(slots, Some(pos));
        if cells.is_empty() || cells.len() > 2 {
            return Err(Error::Unsupported);
        }
        let holders = cells
            .into_iter()
            .map(|cell| {
                if entry.cells.binary_search(&cell).is_err() {
                    return Err(Error::Revision);
                }
                self.block_entity(entry.handle.level(), cell)
                    .ok_or(Error::Stale)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(PreparedMenu(ProjectedMenu {
            handle: entry.handle,
            destination: entry.destination,
            uuid: guest.uuid,
            window: 0,
            registry_id: registry_id.into(),
            slots,
            membership: entry.membership,
            holders,
        })))
    }

    /// Hub open has no gameplay callbacks between prepare and installation.
    pub(in crate::game) fn install_projection_menu(
        &mut self,
        conn: u32,
        seed: Option<PreparedMenu>,
    ) {
        let Some(PreparedMenu(mut binding)) = seed else {
            return;
        };
        let Some(menu) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .and_then(|guest| guest.menu.as_ref())
        else {
            return;
        };
        binding.window = menu.window;
        debug_assert_eq!(menu.source_dimension, binding.handle.level().dimension_id());
        self.live_worlds.get_mut().menus.insert(conn, binding);
    }

    pub(in crate::game) fn projection_menu_bound(&self, conn: u32) -> bool {
        self.live_worlds.borrow().menus.contains_key(&conn)
    }
    pub(in crate::game) fn projection_menu_in_scope(&self, conn: u32) -> bool {
        self.live_worlds.borrow().menu_scope.is_some_and(|scope| {
            scope.conn == conn && self.dim.id() == scope.handle.level().dimension_id()
        })
    }
    pub(in crate::game) fn forget_projection_menu(&mut self, conn: u32) {
        self.live_worlds.get_mut().menus.remove(&conn);
    }

    pub(in crate::game) fn projection_menu_valid(&self, guest: &Guest) -> Option<bool> {
        let table = self.live_worlds.borrow();
        let binding = table.menus.get(&guest.conn)?;
        let Some(entry) = table.entries.get(&binding.handle.generation()) else {
            return Some(false);
        };
        if entry.handle != binding.handle
            || entry.destination != binding.destination
            || entry.membership != binding.membership
            || !self.live_world_lifecycle_current(binding.handle.level())
            || !self.live_world_lifecycle_current(binding.destination)
            || !guest.participates_online()
            || !guest.placed
            || guest.dead
            || guest.uuid != binding.uuid
            || guest.dimension != binding.destination.dimension_id()
        {
            return Some(false);
        }
        let Some(menu) = guest.menu.as_ref() else {
            return Some(false);
        };
        if menu.window != binding.window
            || menu.registry_id != binding.registry_id
            || menu.slots != binding.slots
            || menu.source_dimension != binding.handle.level().dimension_id()
        {
            return Some(false);
        }
        let Some(world) = self.native_level_world(binding.handle.level()) else {
            return Some(false);
        };
        if binding.holders.iter().any(|holder| {
            let cell = holder.fields().position;
            entry.cells.binary_search(&cell).is_err()
                || self.block_entity(binding.handle.level(), cell) != Some(*holder)
                || !world.is_loaded_at(cell[0], cell[1], cell[2])
        }) {
            return Some(false);
        }
        let Ok(pose) = checked_pose(entry.sample.current) else {
            return Some(false);
        };
        let eye = pose.to_local(vector(guest.eye())).components();
        Some(menu.still_valid_with_eye(&self.lan.as_ref()?.hub.loop_context, world, guest, eye))
    }

    /// Before the decided click applies, after the existing ordinary callbacks. A stale scope
    /// refuses the prepared action; it never recomputes a plan or replays a callback.
    pub(in crate::game) fn projection_menu_apply_fresh(&self, conn: u32) -> bool {
        let table = self.live_worlds.borrow();
        let Some(scope) = table.menu_scope.filter(|scope| scope.conn == conn) else {
            return !table.menus.contains_key(&conn);
        };
        let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return false;
        };
        let Some(entry) = table.entries.get(&scope.handle.generation()) else {
            return false;
        };
        entry.handle == scope.handle
            && entry.destination == scope.destination
            && entry.membership == scope.membership
            && entry.sample.revision == scope.pose_revision
            && self.dim.id() == scope.handle.level().dimension_id()
            && self.world.revision == scope.source_revision
            && guest.uuid == scope.uuid
            && guest.eye() == scope.physical_eye
            && guest
                .menu
                .as_ref()
                .is_some_and(|menu| menu.window == scope.window)
            && self.projection_menu_valid(guest) == Some(true)
    }

    /// Cleanup intentionally needs only the captured source identity, not current reach or
    /// membership: invalidating a menu must still run its ordinary removed/lid/cursor path.
    pub(in crate::game) fn with_projection_menu<T>(
        &mut self,
        conn: u32,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> Result<T, Error> {
        if self.projection_menu_in_scope(conn) {
            return Ok(operation(self));
        }
        let binding = self
            .live_worlds
            .borrow()
            .menus
            .get(&conn)
            .cloned()
            .ok_or(Error::Stale)?;
        if !self.projection_action_transaction_allowed(binding.handle.level()) {
            return Err(Error::Busy);
        }
        if !self.level_handle_current(binding.handle.level())
            || !self.level_handle_current(binding.destination)
            || self.native_level_world(binding.destination).is_none()
        {
            return Err(Error::NotResident);
        }
        let guest = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .ok_or(Error::Stale)?;
        if guest.uuid != binding.uuid {
            return Err(Error::Stale);
        }
        let physical_eye = guest.eye();
        let (pose_revision, membership) = self
            .live_worlds
            .borrow()
            .entries
            .get(&binding.handle.generation())
            .filter(|entry| entry.handle == binding.handle)
            .map_or((0, binding.membership), |entry| {
                (entry.sample.revision, entry.membership)
            });
        let source_revision = self
            .native_level_world(binding.handle.level())
            .ok_or(Error::NotResident)?
            .revision;
        let target = Dim::from_id(binding.handle.level().dimension_id());
        let back = self.enter_command_level(target);
        if self.dim != target {
            return Err(Error::NotResident);
        }
        let previous = self.live_worlds.get_mut().menu_scope.replace(MenuScope {
            conn,
            handle: binding.handle,
            destination: binding.destination,
            uuid: binding.uuid,
            window: binding.window,
            pose_revision,
            membership,
            source_revision,
            physical_eye,
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(self)));
        self.live_worlds.get_mut().menu_scope = previous;
        if let Some(back) = back {
            self.enter_level(back).map_err(|_| Error::Unavailable)?;
        }
        match result {
            Ok(value) => Ok(value),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// Exercise the automatic projected menu close without running the full player tick.
    /// Slots/progress sync itself stays in Hub::sync_containers on the ordinary SOURCE pass.
    #[cfg(test)]
    pub(in crate::game) fn tick_projection_menus(&mut self) {
        if self.live_worlds.borrow().menus.is_empty() {
            return;
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let close: Vec<_> = host
            .hub
            .guests
            .iter()
            .filter(|guest| {
                guest.dimension == self.dim.id()
                    && !self.player_tick_canceled(guest.conn)
                    && self.projection_menu_valid(guest) == Some(false)
            })
            .map(|guest| guest.conn)
            .collect();
        for conn in close {
            self.guest_close_menus_and_tell(conn);
        }
    }

    pub(in crate::game) fn close_projection_menus(
        &mut self,
        handle: LiveWorldRef,
    ) -> Result<(), Error> {
        for pending in self.live_worlds.get_mut().pending_opens.values_mut() {
            if pending.action.handle == handle {
                pending.canceled = true;
            }
        }
        let conns: Vec<_> = self
            .live_worlds
            .borrow()
            .menus
            .iter()
            .filter_map(|(&conn, binding)| (binding.handle == handle).then_some(conn))
            .collect();
        for conn in conns {
            self.with_projection_menu(conn, |game| game.guest_close_menus_and_tell(conn))?;
            self.forget_projection_menu(conn);
        }
        Ok(())
    }
    pub(in crate::game) fn close_projection_menus_for_level(
        &mut self,
        dim: Dim,
    ) -> Result<(), Error> {
        let mut handles: Vec<_> = self
            .live_worlds
            .borrow()
            .menus
            .values()
            .filter(|binding| {
                binding.handle.level().dimension() == dim.key()
                    || binding.destination.dimension() == dim.key()
            })
            .map(|binding| binding.handle)
            .collect();
        handles.extend(
            self.live_worlds
                .borrow()
                .pending_opens
                .values()
                .filter(|pending| {
                    pending.action.handle.level().dimension() == dim.key()
                        || pending.destination.dimension() == dim.key()
                })
                .map(|pending| pending.action.handle),
        );
        handles.sort_by_key(|handle| handle.generation());
        handles.dedup();
        for handle in handles {
            self.close_projection_menus(handle)?;
        }
        Ok(())
    }

    /// Ordinary cursor overflow/disconnect drops belong to the physical level. This scope only
    /// changes that gameplay helper's route; explicit LevelRef spawns retain their explicit level.
    pub(in crate::game) fn projection_drop_destination(&self, conn: u32) -> Option<LevelRef> {
        let table = self.live_worlds.borrow();
        if let Some(scope) = table.menu_scope.filter(|scope| scope.conn == conn) {
            return Some(scope.destination);
        }
        let action = table.action?;
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        (guest.uuid == action.player.uuid())
            .then(|| {
                table
                    .entries
                    .get(&action.handle.generation())
                    .map(|entry| entry.destination)
            })
            .flatten()
    }
}

impl Game<'_> {
    pub(super) fn projection_has_open_menu(&self, handle: LiveWorldRef) -> bool {
        let table = self.live_worlds.borrow();
        table.menus.values().any(|m| m.handle == handle)
            || table
                .pending_opens
                .values()
                .any(|p| p.action.handle == handle && !p.canceled)
    }
}
