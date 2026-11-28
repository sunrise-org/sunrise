use super::{ServerGame, commands::execute::Source};
use mod_api::WorldEpoch;
use sunrise_gameplay_common::game::{Dim, lan_actions::TurnRefused};
use sunrise_level::{world::World, worldborder::WorldBorder};
use sunrise_sim::{
    join_seq::JoinSeq,
    lan::{
        guest::{Guest, GuestKind},
        guest_list::GuestList,
    },
    ticket_levels::ChunkTickets,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LevelId {
    pub(super) dim: Dim,
    epoch: Option<WorldEpoch>,
    opened: (u64, u64),
}

impl LevelId {
    #[inline(always)]
    pub(super) fn loaded(game: &ServerGame<'_>, dim: Dim) -> Option<Self> {
        let levels = &game.server.levels.0;
        let loaded = if dim == game.common.dim {
            !levels.active_closed
        } else {
            levels.parked.contains_key(&dim)
        };
        if !loaded {
            return None;
        }
        Some(Self {
            dim,
            epoch: game.common.native_lifecycle.epoch(),
            opened: levels.incarnation(dim)?,
        })
    }

    #[inline(always)]
    fn current(self, game: &ServerGame<'_>) -> bool {
        Self::loaded(game, self.dim) == Some(self)
    }

    #[inline(always)]
    pub(super) fn restore_dimension(self, game: &ServerGame<'_>, legacy: Dim) -> Dim {
        if self.current(game) { self.dim } else { legacy }
    }

    #[inline(always)]
    pub(super) fn view<'a>(self, game: &'a ServerGame<'_>) -> Option<LevelView<'a>> {
        (self.dim == game.common.dim && self.current(game)).then(|| LevelView::active(game))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlayerId {
    Owner(Option<WorldEpoch>, JoinSeq),
    Guest {
        conn: u32,
        kind: GuestKind,
        join: JoinSeq,
    },
}

impl PlayerId {
    #[inline(always)]
    pub(super) fn guest(guest: &Guest) -> Self {
        Self::Guest {
            conn: guest.conn,
            kind: guest.kind,
            join: guest.residency.join(),
        }
    }

    #[inline(always)]
    pub(super) fn acting(game: &ServerGame<'_>) -> Option<Self> {
        if let Some(conn) = game.server.lan_acting {
            return game.server.lan.as_ref()?.hub.guest(conn).map(Self::guest);
        }
        game.common.player.as_ref()?;
        Some(Self::Owner(
            game.common.native_lifecycle.epoch(),
            game.common.resident_entities.local_player.join(),
        ))
    }

    #[inline(always)]
    fn live(self, guest: &Guest) -> bool {
        Self::guest(guest) == self
            && if guest.is_fake() {
                !guest.disconnect_pending && guest.left_reason.is_empty()
            } else {
                guest.play_connection_open()
            }
    }

    #[inline(always)]
    fn live_slot(self, guests: &GuestList) -> Option<usize> {
        let Self::Guest { conn, .. } = self else {
            return None;
        };
        let index = guests.connection_index(conn)?;
        self.live(guests.get(index)?).then_some(index)
    }

    #[cfg(test)]
    #[inline(always)]
    fn view_at<'a>(self, game: &'a ServerGame<'_>, index: usize) -> Option<PlayerView<'a>> {
        let guests = &game.server.lan.as_ref()?.hub.guests;
        (self.live_slot(guests)? == index).then(|| PlayerView {
            guest: &guests[index],
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ActorInLevel {
    pub(super) player: PlayerId,
    pub(super) level: LevelId,
}

impl ActorInLevel {
    #[inline(always)]
    fn live_slot(self, game: &ServerGame<'_>) -> Option<usize> {
        self.level.view(game)?;
        let guests = &game.server.lan.as_ref()?.hub.guests;
        let index = self.player.live_slot(guests)?;
        (guests[index].dimension == self.level.dim.id()).then_some(index)
    }

    #[cfg(test)]
    #[inline(always)]
    fn view_at<'a>(self, game: &'a ServerGame<'_>, index: usize) -> Option<PlayerView<'a>> {
        (self.live_slot(game)? == index).then_some(())?;
        self.player.view_at(game, index)
    }

    #[inline(always)]
    pub(super) fn resume_connection(self, game: &ServerGame<'_>, legacy: u32) -> u32 {
        let slot = self.live_slot(game);
        #[cfg(test)]
        tests::record_resume(self, slot.is_some());
        slot.and_then(|index| {
            let conn = game.server.lan.as_ref()?.hub.guests[index].conn;
            (conn == legacy).then_some(conn)
        })
        .unwrap_or(legacy)
    }

    #[inline(always)]
    pub(super) fn resume_slot(self, game: &ServerGame<'_>, legacy: usize) -> usize {
        // A row change still takes the parent adapter's slot and outcome.
        let slot = self.live_slot(game).filter(|&slot| slot == legacy);
        #[cfg(test)]
        tests::record_resume(self, slot.is_some());
        slot.unwrap_or(legacy)
    }

    #[inline(always)]
    pub(super) fn resume_transfer_slot(
        self,
        game: &ServerGame<'_>,
        source: Option<LevelId>,
        legacy: usize,
    ) -> usize {
        let slot = self.level.view(game).and_then(|_| {
            let source = source.filter(|level| level.current(game))?;
            let guests = &game.server.lan.as_ref()?.hub.guests;
            let slot = self.player.live_slot(guests)?;
            (slot == legacy && guests[slot].dimension == source.dim.id()).then_some(slot)
        });
        slot.unwrap_or(legacy)
    }

    #[inline(always)]
    pub(super) fn resume_level<'a>(self, game: &'a ServerGame<'_>) -> LevelView<'a> {
        let slot = self.live_slot(game);
        #[cfg(test)]
        tests::record_resume(self, slot.is_some());
        LevelView::resume(game, slot.map(|_| self.level))
    }

    #[inline(always)]
    pub(super) fn with_guest<'g, R>(
        self,
        game: &mut ServerGame<'g>,
        legacy: usize,
        conn: u32,
        f: impl FnOnce(&mut ServerGame<'g>) -> R,
    ) -> Result<R, TurnRefused> {
        let slot = self.resume_slot(game, legacy);
        game.as_guest_at(slot, conn, f)
    }
}

pub(super) enum CommandOrigin<'a> {
    Function(&'a str),
    Block((i32, i32, i32), Dim),
    Cart(u32),
    Player(Option<PlayerId>),
    Console,
}

impl<'a> CommandOrigin<'a> {
    #[inline(always)]
    pub(super) fn read(game: &'a ServerGame<'_>, source: &Source) -> Self {
        if let Some(id) = game.server.exec.0.functions.last() {
            Self::Function(id)
        } else if let Some(pos) = source.block {
            Self::Block(pos, source.origin_dimension)
        } else if let Some(id) = source.cart {
            Self::Cart(id)
        } else if source.origin_player {
            Self::Player(PlayerId::acting(game))
        } else {
            Self::Console
        }
    }
}

#[inline(always)]
pub(super) fn active_views<'a>(
    game: &'a ServerGame<'_>,
    level: Option<LevelId>,
    index: usize,
) -> Option<(LevelView<'a>, PlayerView<'a>)> {
    let guests = &game.server.lan.as_ref()?.hub.guests;
    Some((
        LevelView::resume(game, level),
        PlayerView {
            guest: &guests[index],
        },
    ))
}

pub(super) struct LevelView<'a> {
    world: &'a World,
    border: &'a WorldBorder,
}

impl<'a> LevelView<'a> {
    #[inline(always)]
    pub(super) fn active(game: &'a ServerGame<'_>) -> Self {
        Self {
            world: &game.common.world,
            border: &game.common.border,
        }
    }

    #[inline(always)]
    pub(super) fn resume(game: &'a ServerGame<'_>, level: Option<LevelId>) -> Self {
        level
            .and_then(|id| id.view(game))
            .unwrap_or_else(|| Self::active(game))
    }

    #[inline(always)]
    pub(super) fn world(&self) -> &'a World {
        self.world
    }

    #[inline(always)]
    pub(super) fn build_limits(&self) -> (i32, i32) {
        (self.world.shape.min_y, self.world.shape.max_y() - 1)
    }

    #[inline(always)]
    pub(super) fn contains(&self, x: f64, z: f64) -> bool {
        self.border.contains(x, z)
    }
}

pub(super) struct LevelWrite<'a> {
    tickets: &'a mut ChunkTickets,
}

impl<'a> LevelWrite<'a> {
    #[inline(always)]
    pub(super) fn active(game: &'a mut ServerGame<'_>) -> Self {
        Self {
            tickets: &mut game.server.chunk_ticking.tickets,
        }
    }

    #[inline(always)]
    pub(super) fn add_portal(&mut self, chunk: (i32, i32)) {
        self.tickets.add_portal(chunk);
    }
}

pub(super) struct PlayerView<'a> {
    pub(super) guest: &'a Guest,
}

pub(super) struct PlayerWrite<'a> {
    pub(super) guest: &'a mut Guest,
}

impl PlayerWrite<'_> {
    #[inline(always)]
    pub(super) fn update<R>(
        guests: &mut GuestList,
        slot: usize,
        f: impl FnOnce(PlayerWrite<'_>) -> R,
    ) -> R {
        guests.update(slot, |guest| f(PlayerWrite { guest }))
    }
}

#[cfg(test)]
#[path = "state_seam/tests.rs"]
pub(super) mod tests;
