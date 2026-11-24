//! Session fencing and common persistent coverage over the existing writers.
use super::*;
use sunrise_client::save::{LiveWorldSaveLease, SaveDir};

#[derive(Default)]
pub(in crate::game) struct Durability {
    pub(super) failed: bool,
    pub(super) coverage_epoch: u64,
    pub(super) last_barrier_epoch: u64,
    pub(super) saved_owner_profile: Option<[i32; 4]>,
    pub(super) last_capture: Option<CaptureUsage>,
    in_server_tick: bool,
    global_frontier: bool,
    save_requested: bool,
    pub(super) saving: bool,
    pub(super) retiring_streams: bool,
    pub(super) members: BTreeMap<LevelRef, DurableLevel>,
    pub(super) dirty: BTreeSet<LevelRef>,
    pub(super) retired: BTreeMap<LevelRef, super::binding_journal::RetiredBinding>,
    pub(super) migration: Option<(LevelRef, Vec<u8>, super::persistence_codec::SavedLevel)>,
    pub(super) next_transfer: u64,
    pub(super) transfers: BTreeMap<u64, super::transfer::Transfer>,
}
pub(super) struct CaptureUsage {
    pub(super) epoch: u64,
    /// None for an early payload-cap refusal, where only a lower bound is known.
    pub(super) usage: Option<sunrise_client::save::live_world_journal::Usage>,
    pub(super) payload_bound: Option<(u64, u32)>,
    pub(super) limit: u64,
    /// Size admission only, never a durable-success receipt.
    pub(super) fits_byte_budget: bool,
}
pub(super) struct DurableLevel {
    pub(super) save: Arc<SaveDir>,
    _lease: LiveWorldSaveLease,
}
impl Durability {
    pub(in crate::game) fn reset_for_recovered_world(&mut self) {
        // The preceding session lock still protected every pending decision; only a new open
        // can recover it. Clearing this context cannot make that old Game save again.
        *self = Self::default();
    }
}
impl Game<'_> {
    pub(in crate::game) fn live_world_session_failed(&self) -> bool {
        self.live_world_durability.failed
    }
    pub(in crate::game) fn admit_live_world_session(&self) -> sunrise_client::Result<()> {
        if self.live_world_session_failed() {
            return Err(
                "live-world durable decision requires recovery before the session can continue"
                    .into(),
            );
        }
        Ok(())
    }
    pub(in crate::game) fn live_world_commit_frontier(&self) -> bool {
        (!self.live_world_durability.in_server_tick || self.live_world_durability.global_frontier)
            && self.lan_acting.is_none()
            && self.lan_acting_uuid.is_none()
            && self.item_columns.is_none()
            && self.pass_mobs.is_none()
            && {
                let table = self.live_worlds.borrow();
                table.action.is_none() && table.menu_scope.is_none()
            }
    }
    pub(in crate::game) fn live_world_tick_enter(&mut self) {
        self.live_world_durability.in_server_tick = true;
        self.live_world_durability.global_frontier = true;
    }
    pub(in crate::game) fn live_world_tick_levels(&mut self) {
        self.live_world_durability.global_frontier = false;
    }
    pub(in crate::game) fn live_world_tick_end_frontier(&mut self) {
        self.live_world_durability.global_frontier = true;
    }
    pub(in crate::game) fn live_world_tick_leave(&mut self) {
        self.live_world_durability.in_server_tick = false;
        self.live_world_durability.global_frontier = false;
    }
    pub(in crate::game) fn hold_live_world_level(
        &mut self,
        requested: LevelRef,
    ) -> Result<(), Error> {
        if self.live_world_session_failed() {
            return Err(Error::Storage);
        }
        if !self.level_handle_current(requested) {
            return Err(Error::Stale);
        }
        // Once player files join a shared decision, no currently open world's independent writer
        // may race them. This operational mode lasts for the recovered world session, including
        // an aborted first binding or the disposal of its final projection.
        let mut levels: BTreeSet<_> = self
            .levels
            .parked_dims()
            .into_iter()
            .filter_map(|dim| self.level(dim.id()))
            .collect();
        if let Some(current) = self.level(self.dim.id()) {
            levels.insert(current);
        }
        levels.insert(requested);
        if self.live_world_durability.members.len()
            + levels
                .iter()
                .filter(|level| !self.live_world_durability.members.contains_key(level))
                .count()
            > 64
        {
            return Err(Error::Budget);
        }
        if self.player_autosaves.enable_common_checkpoint().is_err() {
            self.fence_live_world_session();
            return Err(Error::Storage);
        }
        for level in levels {
            if let Err(error) = self.hold_live_world_store(level) {
                // A partially installed terminal common mode cannot fall back to independent writers.
                self.fence_live_world_session();
                return Err(error);
            }
        }
        Ok(())
    }
    fn hold_live_world_store(&mut self, level: LevelRef) -> Result<(), Error> {
        if self.live_world_durability.members.contains_key(&level) {
            return Ok(());
        }
        let save = if self.dim.key() == level.dimension() {
            self.save.clone()
        } else {
            self.levels
                .parked_save_of(level.dimension_id())
                .ok_or(Error::NotResident)?
        };
        let next = self
            .live_world_durability
            .coverage_epoch
            .checked_add(1)
            .ok_or(Error::Budget)?;
        let lease = save
            .require_live_world_barrier()
            .map_err(|_| Error::Storage)?;
        self.live_world_durability.members.insert(
            level,
            DurableLevel {
                save,
                _lease: lease,
            },
        );
        self.live_world_durability.coverage_epoch = next;
        Ok(())
    }
    pub(super) fn dirty_live_world_level(&mut self, level: LevelRef) -> Result<(), Error> {
        self.hold_live_world_level(level)?;
        self.live_world_durability.dirty.insert(level);
        Ok(())
    }
    pub(in crate::game) fn fence_live_world_session(&mut self) {
        self.live_world_durability.failed = true;
        for member in self.live_world_durability.members.values() {
            member.save.fail_live_world_save();
        }
        // Ordinary current/parked writers cannot overwrite WAL-owned files on crash or close.
        self.save.fail_live_world_save();
        self.player_autosaves.fail_common();
        self.levels.fence_live_world_saves();
        self.agent.hold_server = true;
    }
}

impl Game<'_> {
    pub(in crate::game) fn live_world_durability_active(&self) -> bool {
        self.player_autosaves.common_required() || !self.live_world_durability.members.is_empty()
    }
    pub(in crate::game) fn live_world_common_level_count(&self) -> usize {
        self.live_world_durability.members.len()
    }
    pub(in crate::game) fn request_live_world_save(&mut self) {
        self.live_world_durability.save_requested = true;
    }
    pub(in crate::game) fn live_world_checkpoint_running(&self) -> bool {
        self.live_world_durability.saving
    }
    pub(in crate::game) fn service_live_world_saves(&mut self) -> sunrise_client::Result<()> {
        if !self.live_world_durability_active() || self.live_world_checkpoint_running() {
            return Ok(());
        }
        self.admit_live_world_session()?;
        let requested = self.live_world_durability.save_requested
            || !self.player_autosaves.writes_finished()?;
        let pending: Vec<_> = self
            .live_world_durability
            .members
            .iter()
            .filter(|(_, member)| member.save.live_world_barrier_pending())
            .map(|(level, _)| *level)
            .collect();
        if !requested && pending.is_empty() {
            return Ok(());
        }
        if !self.live_world_commit_frontier() {
            return Ok(());
        }
        if requested {
            match self.checkpoint_live_world_levels(&[]) {
                Ok(receipt) => {
                    self.live_world_durability.save_requested = false;
                    self.finish_common_game_save(&receipt, false)?;
                }
                Err(Error::Busy | Error::Revision) => return Ok(()),
                Err(error) => return Err(format!("common save refused: {error:?}").into()),
            }
        }
        // A held Streaming plan can retire only immediately after its exact live readset joined
        // a common decision. Planning and its checkpoint stay in this completed server frontier.
        let back = self.dim;
        self.live_world_durability.retiring_streams = true;
        for level in pending {
            if self.level_handle_current(level) {
                self.enter_level(super::super::Dim::from_id(level.dimension_id()))?;
                self.update_world();
                if self.live_world_session_failed() {
                    break;
                }
            }
        }
        self.live_world_durability.retiring_streams = false;
        if self.dim != back {
            self.enter_level(back)?;
        }
        self.admit_live_world_session()
    }
    pub(in crate::game) fn projection_level_for_generation(
        &self,
        generation: u64,
    ) -> Option<LevelRef> {
        self.live_worlds
            .borrow()
            .entries
            .get(&generation)
            .map(|entry| entry.handle.level())
    }
    pub(in crate::game) fn retain_live_world_root_targets(
        &mut self,
        targets: &BTreeSet<LevelRef>,
    ) -> Result<(), Error> {
        for &target in targets {
            // Leases belong to the recovered session, not a tentative mod binding. Aborting
            // caller metadata must not re-enable an independent writer beside common player files.
            self.hold_live_world_level(target)?;
        }
        if !targets.is_empty() {
            self.live_world_durability.coverage_epoch = self
                .live_world_durability
                .coverage_epoch
                .checked_add(1)
                .ok_or(Error::Budget)?;
        }
        self.live_world_durability
            .dirty
            .extend(targets.iter().copied());
        Ok(())
    }
}

impl Game<'_> {
    pub(in crate::game) fn validate_common_metadata_profile(&self) -> sunrise_client::Result<()> {
        if self.live_world_durability.last_barrier_epoch == 0
            || self.level.player_uuid != self.live_world_durability.saved_owner_profile
        {
            return Err("owner profile is not covered by the common checkpoint".into());
        }
        if let Some(owner) = self.lan.as_ref().and_then(|host| host.owner.as_ref())
            && self.level.player_uuid
                != Some(sunrise_client::lan::playerdata::uuid_ints(owner.uuid))
        {
            return Err("owner profile changed after the common checkpoint".into());
        }
        Ok(())
    }
}
