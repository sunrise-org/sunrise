//! Prepared multi-level ownership handoffs. Tokens hold read witnesses, never a second live inventory.
use super::transfer_cells::PreparedCells;
use super::*;
use mod_api::{
    LiveWorldTransferCell as Cell, LiveWorldTransferLimits as Limits,
    LiveWorldTransferPending as Pending, LiveWorldTransferReceipt as Receipt,
    LiveWorldTransferRef as Token, TransactionContext, __private::TransactionFrame,
};

pub(super) enum State {
    Prepared(PreparedCells),
    Staged { root: TransactionFrame, plan: PreparedCells },
    Done(Receipt),
    Refused(Error),
}
pub(super) struct Transfer {
    owner: &'static str,
    pub(super) cells: Vec<Cell>,
    pub(super) state: State,
}
impl Game<'_> {
    fn transfer_authority(&self, token: Token) -> Result<&Transfer, Error> {
        let owner = self.projection_owner()?;
        if Some(token.epoch()) != self.native_lifecycle.epoch() {
            return Err(Error::Stale);
        }
        let entry = self
            .live_world_durability
            .transfers
            .get(&token.generation())
            .ok_or(Error::Stale)?;
        if owner != entry.owner {
            return Err(Error::Denied);
        }
        Ok(entry)
    }
    fn transfer_frontier(&self) -> Result<(), Error> {
        if self.live_world_session_failed() {
            return Err(Error::Storage);
        }
        if !self.live_world_commit_frontier()
            || self.live_world_checkpoint_running()
            || matches!(
                self.transaction_lifecycle(),
                mod_api::TransactionLifecycle::Closing | mod_api::TransactionLifecycle::RootClosing
            )
        {
            #[cfg(test)]
            eprintln!(
                "LIVE_WORLD_TRANSFER busy frontier current={:?} lifecycle={:?} checkpoint={}",
                self.dim.id(),
                self.transaction_lifecycle(),
                self.live_world_checkpoint_running()
            );
            return Err(Error::Busy);
        }
        Ok(())
    }
    fn transfer_levels_admitted(&self, owner: &str, cells: &[Cell]) -> Result<(), Error> {
        // Ordinary menus keep their one real storage owner. This bounded bulk handoff waits for
        // all current screens to close; it never transfers a still-borrowed menu backing store.
        if self.container.is_some()
            || self
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.guests.iter().any(|guest| guest.menu.is_some()))
        {
            #[cfg(test)]
            eprintln!("LIVE_WORLD_TRANSFER actual menu borrow active");
            return Err(Error::Busy);
        }
        for level in cells.iter().flat_map(|c| [c.source, c.destination]) {
            if !self.live_world_lifecycle_current(level) {
                return Err(Error::Stale);
            }
            if level.generation() != 0
                && self.levels.runtime.owner_of(level.dimension_id()) != owner
            {
                return Err(Error::Denied);
            }
        }
        let table = self.live_worlds.borrow();
        for source in cells
            .iter()
            .flat_map(|c| [c.source, c.destination])
            .filter(|level| level.generation() != 0)
        {
            if !table
                .entries
                .values()
                .any(|entry| entry.handle.level() == source)
                && let Some(bytes) = self.read_projection_bytes(source)?
            {
                let saved: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|_| Error::Storage)?;
                match saved.get("schema").and_then(serde_json::Value::as_u64) {
                    Some(1) => return Err(Error::MigrationRequired),
                    Some(2) if saved.get("binding").is_some_and(|value| !value.is_null()) => {
                        return Err(Error::AlreadyBound);
                    }
                    Some(2) => {}
                    _ => return Err(Error::Storage),
                }
            }
        }
        for entry in table.entries.values() {
            let incoming = cells.iter().any(|c| c.destination == entry.handle.level());
            if incoming {
                return Err(Error::AlreadyBound);
            }
            if cells.iter().any(|c| c.source == entry.handle.level())
                && self.projection_has_open_menu(entry.handle)
            {
                #[cfg(test)]
                eprintln!(
                    "LIVE_WORLD_TRANSFER actual projected menu borrow {:?}",
                    entry.handle
                );
                return Err(Error::Busy);
            }
        }
        Ok(())
    }
    pub(in crate::game) fn level_prepare_transfer(
        &mut self,
        anchor: LevelRef,
        cells: &[Cell],
        limits: Limits,
    ) -> Result<Token, Error> {
        self.transfer_frontier()?;
        // A custom serializer may rewrite block states/inventory after a frozen stock payload is
        // selected. This bounded handoff refuses that unsupported codec policy before preparation.
        if mod_api::__private::listened(mod_api::HookId::ChunkDataSave)
            || mod_api::__private::listened(mod_api::HookId::ChunkDataLoad)
            || mod_api::__private::has_part_hook(mod_api::HookId::BlockEntitySaveAdditional)
            || mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityLoadAdditional)
        {
            return Err(Error::Unsupported);
        }
        if self.has_open_transaction() {
            #[cfg(test)]
            eprintln!(
                "LIVE_WORLD_TRANSFER open actual transaction at prepare: {:?}",
                self.transaction_lifecycle()
            );
            return Err(Error::Busy);
        }
        let owner = self.projection_owner()?;
        if !cells
            .iter()
            .any(|c| c.source == anchor || c.destination == anchor)
        {
            return Err(Error::Invalid);
        }
        if limits.cells == 0
            || limits.cells > 65536
            || limits.levels == 0
            || limits.levels > 64
            || limits.chunks == 0
            || limits.chunks > 8192
            || limits.boxes == 0
            || limits.boxes > 1048576
            || limits.scheduled_ticks > 1048576
            || limits.bytes == 0
            || limits.bytes > 512 * 1024 * 1024
        {
            return Err(Error::Budget);
        }
        self.transfer_levels_admitted(owner, cells)?;
        if self
            .live_world_durability
            .transfers
            .values()
            .filter(|t| matches!(t.state, State::Prepared(_) | State::Staged { .. }))
            .count()
            >= 64
        {
            return Err(Error::Budget);
        }
        let back = self.dim;
        let levels: BTreeSet<_> = cells
            .iter()
            .flat_map(|c| [c.source, c.destination])
            .collect();
        for level in levels {
            self.enter_level(super::super::Dim::from_id(level.dimension_id()))
                .map_err(|_| Error::NotResident)?;
        }
        if self.dim != back {
            self.enter_level(back).map_err(|_| Error::NotResident)?;
        }
        let plan = self.prepare_transfer_cells(cells, limits)?;
        self.live_world_durability.next_transfer = self
            .live_world_durability
            .next_transfer
            .checked_add(1)
            .ok_or(Error::Budget)?;
        let generation = self.live_world_durability.next_transfer;
        // Completed receipts remain bounded. Eviction makes only that old token stale.
        if self.live_world_durability.transfers.len() >= 256 {
            let old = self
                .live_world_durability
                .transfers
                .iter()
                .find(|(_, t)| matches!(t.state, State::Done(_) | State::Refused(_)))
                .map(|(id, _)| *id)
                .ok_or(Error::Budget)?;
            self.live_world_durability.transfers.remove(&old);
        }
        self.live_world_durability.transfers.insert(
            generation,
            Transfer {
                owner,
                cells: cells.to_vec(),
                state: State::Prepared(plan),
            },
        );
        Ok(Token::new(
            self.native_lifecycle.epoch().ok_or(Error::Unavailable)?,
            generation,
        ))
    }
    pub(in crate::game) fn live_world_transfer_stage(
        &mut self,
        token: Token,
        tx: &TransactionContext,
    ) -> Result<Pending, Error> {
        self.transfer_frontier()?;
        let entry = self.transfer_authority(token)?;
        let State::Prepared(plan) = &entry.state else {
            return Err(Error::Busy);
        };
        let root = self.current_storage_frame().ok_or(Error::Busy)?;
        if root.depth() != 0 || !root.matches(tx) {
            return Err(Error::Busy);
        }
        if self
            .live_world_durability
            .transfers
            .values()
            .any(|t| matches!(t.state, State::Staged {root: r, ..} if r == root))
        {
            return Err(Error::Busy);
        }
        self.transfer_levels_admitted(entry.owner, &entry.cells)?;
        self.validate_transfer_cells(plan)?;
        let levels: BTreeSet<_> = plan.affected_columns().keys().copied().collect();
        self.retain_live_world_root_targets(&levels)?;
        let epoch = sunrise_client::save::live_world_journal::next_epoch(&self.world_root)
            .map_err(|_| Error::Storage)?;
        let entry = self
            .live_world_durability
            .transfers
            .get_mut(&token.generation())
            .ok_or(Error::Stale)?;
        let State::Prepared(plan) =
            std::mem::replace(&mut entry.state, State::Refused(Error::NotCommitted))
        else {
            return Err(Error::Busy);
        };
        entry.state = State::Staged { root, plan };
        Ok(Pending::new(epoch))
    }
    pub(in crate::game) fn live_world_transfer_receipt(
        &self,
        token: Token,
    ) -> Result<Receipt, Error> {
        match &self.transfer_authority(token)?.state {
            State::Done(receipt) => Ok(receipt.clone()),
            State::Refused(error) => Err(*error),
            State::Prepared(_) => Err(Error::NotCommitted),
            State::Staged { .. } => Err(Error::Busy),
        }
    }
    pub(in crate::game) fn live_world_transfer_cancel(
        &mut self,
        token: Token,
    ) -> Result<(), Error> {
        let entry = self.transfer_authority(token)?;
        if matches!(entry.state, State::Staged { .. }) {
            return Err(Error::Busy);
        }
        self.live_world_durability
            .transfers
            .remove(&token.generation());
        Ok(())
    }
    pub(in crate::game) fn live_world_transfer_commit(
        &mut self,
        token: Token,
    ) -> Result<Receipt, Error> {
        self.transfer_frontier()?;
        if self.has_open_transaction() {
            return Err(Error::Busy);
        }
        self.transfer_authority(token)?;
        let result = self.storage_transaction_settled(|game, tx| {
            game.live_world_transfer_stage(token, tx)
        });
        if self.live_world_session_failed() {
            return Err(Error::Storage);
        }
        if let Err(mod_api::TransactionError::Aborted(error)) = result {
            return Err(error);
        }
        self.live_world_transfer_receipt(token)
    }
    pub(in crate::game) fn has_staged_live_world_transfer(&self, root: TransactionFrame) -> bool {
        self.live_world_durability
            .transfers
            .values()
            .any(|t| matches!(t.state, State::Staged {root: r, ..} if r == root))
    }
    pub(in crate::game) fn finish_live_world_transfer_root(
        &mut self,
        root: TransactionFrame,
        admitted: bool,
    ) -> Result<bool, Error> {
        let id = self
            .live_world_durability
            .transfers
            .iter()
            .find(|(_, t)| matches!(t.state, State::Staged {root: r, ..} if r == root))
            .map(|(id, _)| *id);
        let Some(id) = id else {
            return Ok(false);
        };
        let entry = self
            .live_world_durability
            .transfers
            .get_mut(&id)
            .ok_or(Error::Stale)?;
        let State::Staged { plan, .. } =
            std::mem::replace(&mut entry.state, State::Refused(Error::NotCommitted))
        else {
            return Err(Error::Busy);
        };
        let cells = entry.cells.clone();
        let owner = entry.owner;
        if !admitted {
            return Err(Error::NotCommitted);
        }
        let result = self
            .transfer_levels_admitted(owner, &cells)
            .and_then(|()| self.checkpoint_live_world_transfer(plan, &cells));
        let entry = self
            .live_world_durability
            .transfers
            .get_mut(&id)
            .ok_or(Error::Stale)?;
        match result {
            Ok(mut receipt) => {
                receipt.levels.retain(|row| {
                    row.level.generation() == 0
                        || self.levels.runtime.owner_of(row.level.dimension_id()) == owner
                });
                entry.state = State::Done(receipt);
                Ok(true)
            }
            Err(error) => {
                entry.state = State::Refused(error);
                Err(error)
            }
        }
    }
    pub(super) fn apply_transfer_membership(
        &mut self,
        cells: &[Cell],
    ) -> Result<Vec<LiveWorldRef>, Error> {
        let sources: BTreeSet<_> = cells.iter().map(|c| c.source).collect();
        let mut retired = Vec::new();
        for source in sources {
            let handle = self
                .live_worlds
                .borrow()
                .entries
                .values()
                .find(|e| e.handle.level() == source)
                .map(|e| e.handle);
            let Some(handle) = handle else {
                continue;
            };
            let removed: BTreeSet<_> = cells
                .iter()
                .filter(|c| c.source == source)
                .map(|c| c.from)
                .collect();
            let mut table = self.live_worlds.borrow_mut();
            let entry = table
                .entries
                .get_mut(&handle.generation())
                .ok_or(Error::Stale)?;
            entry.cells.retain(|cell| !removed.contains(cell));
            entry.membership = entry.membership.checked_add(1).ok_or(Error::Budget)?;
            entry.geometry.get_mut().take();
            entry.mesh.get_mut().take();
            let empty = entry.cells.is_empty();
            if !empty {
                entry.retained = retention::checked_retention(&entry.cells, entry.limits)?;
            }
            if empty {
                table.entries.remove(&handle.generation());
            }
            drop(table);
            if empty {
                self.record_projection_retirement(handle, 0)?;
                retired.push(handle);
            }
            self.refresh_live_world_retention(source);
        }
        Ok(retired)
    }
}
