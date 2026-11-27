//! The fight's level's `emptyTime`, and the fight while that level is idle:
//! `ServerLevel.tick` runs every level on every server tick, and runs
//! `EnderDragonFight.tick` while `emptyTime < 300`, whether or not a player
//! is in the level. Its player scan in that time is what takes a player who
//! left the End off the dragon's boss bar (`updatePlayers`,
//! `ServerBossEvent.removePlayer`).
use sunrise_gameplay_common::game::Dim;

/// `ServerLevel.tick`: the level's entities and its fight tick while
/// `emptyTime < 300`.
pub(super) const EMPTY_LEVEL_TICKS: i32 = 300;

impl crate::game::ServerGame<'_> {
    /// `ServerLevel.tick`'s `emptyTime` for the fight's level, from the server
    /// tick's own pass over it, idle or not: a level holding a ticket that
    /// keeps a dimension active (`chunkSource.hasActiveTickets()`) resets it,
    /// and a tick that `runs` counts it up. Nothing for another level.
    pub(in crate::game) fn count_fight_empty_time(&mut self, level: Dim, runs: bool) {
        if !self.owns_fight(level) {
            return;
        }
        if self.keeps_level_active(level) {
            self.server.fight.empty_time = 0;
        }
        if runs {
            self.server.fight.empty_time = self.server.fight.empty_time.saturating_add(1);
        }
    }

    /// Whether the fight ticks this tick (`emptyTime < 300`).
    pub(in crate::game) fn fight_ticks(&self) -> bool {
        self.server.fight.empty_time < EMPTY_LEVEL_TICKS
    }

    /// `TicketStorage.shouldKeepDimensionActive`: whether `level` holds a
    /// ticket of a type with `FLAG_KEEP_DIMENSION_ACTIVE`, as Sunrise has
    /// them: a player's simulation ticket (`DistanceManager.addPlayer`, which
    /// `ChunkMap.skipPlayer` leaves out for a spectator while
    /// `spectators_generate_chunks` is off), a forced chunk, or a portal
    /// ticket.
    fn keeps_level_active(&self, level: Dim) -> bool {
        let rule = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPECTATORS_GENERATE_CHUNKS);
        let loads = |spectator| sunrise_gameplay_common::game_modes::loads_chunks(rule, spectator);
        let forced = if level == self.common.dim {
            !self.server.streaming.forced.is_empty()
        } else {
            self.server
                .levels
                .0
                .parked
                .get(&level)
                .is_some_and(|p| !p.streaming.forced.is_empty())
        };
        let host = level == self.common.dim
            && self.common.player.is_some()
            && loads(self.common.spectator());
        let guests = self.server.lan.as_ref().is_some_and(|h| {
            h.hub
                .guests
                .iter()
                .any(|g| g.placed_in(level.id()) && loads(g.spectator()))
        });
        forced || host || guests || !self.portal_tickets_in(level).is_empty()
    }

    /// `EnderDragonFight.tick` for the fight's `level` on a running server
    /// tick whose pass does not tick that level (it has no player, forced
    /// chunk or ticket that asks for one), while `emptyTime < 300`: the same
    /// start as the level's own pass, the tick hook (with the level in place)
    /// and the player scan every 20 ticks. The rest of the fight works on the
    /// level's chunks and entities, which Sunrise does not tick in an idle
    /// level.
    pub(in crate::game) fn tick_idle_fight(&mut self, level: Dim) {
        if !self.owns_fight(level) || !self.server.fight.loaded || !self.fight_ticks() {
            return;
        }
        self.fight_tick_start(level);
    }

    /// `ServerLevel`'s constructor: a level whose type has the fight
    /// initializes it as the level opens, so it ticks from the server's
    /// start. `level` is in place.
    pub(in crate::game) fn open_level_fight(&mut self, level: Dim) {
        if self.owns_fight(level) && level == self.common.dim {
            self.load_fight();
        }
    }
}
