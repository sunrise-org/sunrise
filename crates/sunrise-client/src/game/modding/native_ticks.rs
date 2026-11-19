//! Entities ticking for native mods (`native_entity_tick_pre`, `native_entity_tick_post`:
//! NeoForge's `EntityTickEvent.Pre` and `.Post`), on the server and on a client. An entity of a
//! type a mod named (`entity_tick_each`) is told alone to the mods that named it, right before
//! and right after its own tick (`Listeners`: the dispatch goes only to them); the other mods
//! hear it with its family. The others are told by family: the loops of the mobs and the items hold their lists out
//! of `Game`, so those families hand their ticking entities to a `TickRound` before the loop, hear
//! one Pre for all of them before it and one Post after it; the minecarts, boats and general
//! entity rows, whose loops can call the hooks, are told one by one. The mob and item loops tell
//! a named entity's Pre and Post from inside, at a yield that gives their list back for the call.
//!
//! Each entity's position writes are filed for `entity_entering_section`
//! (`entity_sections.rs`) while its tick is still running: a round keeps its entities while
//! either hook is listened to.
use super::Game;
use std::sync::Arc;
use sunrise_client::join_seq::JoinSeq;
use sunrise_client::mobs::MobType;

/// The item entities' type, as the hooks name it.
pub(super) const ITEM: &str = "minecraft:item";
/// The players' type, as the hooks name it.
pub(super) const PLAYER: &str = "minecraft:player";

/// The type of an entity in a round, as the hooks name it.
pub(super) enum TickKind {
    Fixed(&'static str),
    Mob(Arc<MobType>),
}

impl TickKind {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Fixed(id) => id,
            Self::Mob(kind) => &kind.id,
        }
    }
}

/// The index in a family's list of the entity `is` picks out, looked for from `from` on first:
/// the list keeps its order, though a mod's hook may have taken entities out of it.
pub(super) fn place<T>(list: &[T], from: usize, is: impl Fn(&T) -> bool) -> Option<usize> {
    let from = from.min(list.len());
    list[from..]
        .iter()
        .position(&is)
        .map(|i| from + i)
        .or_else(|| list[..from].iter().position(is))
}

/// Where slot `slot`'s entity of a family's loop is in its list now: the slot itself until a hook
/// told during the loop changed the list (`places`, by id), then by its id (`slots`), if it is
/// still there.
pub(super) fn slot_place(
    places: &Option<sunrise_client::fast_hash::FxHashMap<u32, usize>>,
    slots: &[u32],
    slot: usize,
) -> Option<usize> {
    match places {
        None => Some(slot),
        Some(places) => places.get(&slots[slot]).copied(),
    }
}

/// Where each entity of `ids` is, by id: a family's loop finds its rows with it once a hook told
/// during the loop changed the list.
pub(super) fn places_of(
    ids: impl Iterator<Item = u32>,
) -> sunrise_client::fast_hash::FxHashMap<u32, usize> {
    ids.enumerate().map(|(i, id)| (id, i)).collect()
}

/// What a family's Pre and Post are told from, kept between ticks so that telling them
/// allocates nothing.
#[derive(Default)]
pub(in crate::game) struct TickBuffers {
    ids: Vec<u32>,
    /// Empty between calls: it borrows the round's kinds only while a hook runs.
    kinds: Vec<&'static str>,
    /// What one Pre's listeners canceled, its own for each call: what a listener does to it
    /// reaches the round only as the IDs that call was told.
    canceled: Vec<u32>,
    /// The types of entities whose rows hold them as their own strings, copied out for a call.
    names: KindNames,
}

/// Type names copied out of the rows that own them, so a hook is told them while the rows stay
/// in their list for handles to read: one string and where each name ends.
#[derive(Default)]
struct KindNames {
    text: String,
    ends: Vec<usize>,
}

impl KindNames {
    fn clear(&mut self) {
        self.text.clear();
        self.ends.clear();
    }
    fn push(&mut self, kind: &str) {
        self.text.push_str(kind);
        self.ends.push(self.text.len());
    }
    fn names(&self) -> impl Iterator<Item = &str> {
        let mut start = 0;
        self.ends.iter().map(move |&end| {
            let name = &self.text[start..end];
            start = end;
            name
        })
    }
}

/// Whether an entity the server keeps without simulating it (`others`: displays, markers,
/// interactions) is told its tick here. Leash knots and eyes of ender tick in loops of their own;
/// a missing mod's record is kept, not an entity of the level that ticks.
pub(super) fn other_ticks(kind: &str) -> bool {
    !matches!(kind, "minecraft:leash_knot" | "minecraft:eye_of_ender")
        && !super::general::quarantined_kind(kind)
}

/// Which entities of a round are told alone, around their own tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Alone {
    /// None: no mod named a type.
    #[default]
    None,
    /// Those of the types mods named (a mob type's `tick_alone`).
    Named,
    /// Every entity: a mod named every type.
    All,
}

/// Whether the tick hooks are heard on this side in this tick, and which entities are told alone:
/// asked once a tick (`native_tick_told`).
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::game) struct Told {
    /// Whether the tick hooks are listened to on this side: otherwise no Pre or Post is told.
    listening: bool,
    /// Which entities are told alone instead of with their family.
    alone: Alone,
}

impl Told {
    /// Whether the tick hooks are listened to on this side.
    pub(super) fn listening(&self) -> bool {
        self.listening
    }
    /// Whether a mob of type `kind` is told alone, around its own tick, not with its family.
    pub(super) fn alone_mob(&self, kind: &MobType) -> bool {
        self.listening
            && match self.alone {
                Alone::None => false,
                Alone::Named => kind.flags.tick_alone,
                Alone::All => true,
            }
    }
    /// Whether the entities of type `kind` are told alone. For a family of one type
    /// (`minecraft:item`) it is asked once for the family, not for each entity.
    pub(super) fn alone_kind(&self, kind: &str) -> bool {
        self.listening
            && match self.alone {
                Alone::None => false,
                Alone::Named => {
                    mod_api::__private::entity_tick_each().is_some_and(|each| each.names(kind))
                }
                Alone::All => true,
            }
    }
}

/// The entities of one family in one tick.
#[derive(Default)]
pub(in crate::game) struct TickRound {
    /// Whether the hooks are heard, and who is told alone.
    told: Told,
    /// Whether `entity_entering_section` is listened to on this server: each entity's position
    /// write is filed and told while its tick is still running.
    sections: bool,
    /// The entities about to tick with the family, in order, each with whether it is of a type
    /// a mod named (told alone to the mods that named it, with the family to the others); once
    /// the family's Pre is told, those it let through, for its Post.
    ticking: Vec<(u32, TickKind, bool)>,
    /// Whether `ticking` holds an entity of a named type.
    named: bool,
    /// The entities a Pre canceled.
    canceled: Vec<u32>,
    /// Scratch output owned by one hook dispatch, never shared with earlier turns.
    call_canceled: Vec<u32>,
    /// The items whose part ended their tick (`on_entity_item_update`): they are not
    /// canceled, so they still hear their Post.
    halted: Vec<u32>,
}

impl TickRound {
    /// Whether a mod canceled the tick of entity `id`.
    pub(super) fn canceled(&self, id: u32) -> bool {
        !self.canceled.is_empty() && self.canceled.contains(&id)
    }
    /// Whether a part ended the tick of item `id` (`on_entity_item_update`).
    pub(super) fn halted(&self, id: u32) -> bool {
        !self.halted.is_empty() && self.halted.contains(&id)
    }
    /// A part ended the tick of item `id`.
    pub(super) fn halt(&mut self, id: u32) {
        self.halted.push(id);
    }
    /// Whether `entity_entering_section` is listened to: each entity's position updates are filed.
    pub(super) fn sections(&self) -> bool {
        self.sections
    }
    /// Whether the round keeps the entities that tick: a mod hears their ticks or their moves.
    pub(super) fn listening(&self) -> bool {
        self.told.listening || self.sections
    }
    /// Whether a mob of type `kind` is told alone, around its own tick, not with its family.
    pub(super) fn alone_mob(&self, kind: &MobType) -> bool {
        self.told.alone_mob(kind)
    }
    /// Whether the entities of type `kind` are told alone ([`Told::alone_kind`]).
    pub(super) fn alone_kind(&self, kind: &str) -> bool {
        self.told.alone_kind(kind)
    }
    /// The entity `id` of `kind` is about to tick with its family, which hears its Pre and its
    /// Posts together (`native_tick_pre_batch`).
    pub(super) fn add(&mut self, id: u32, kind: TickKind) {
        if self.listening() {
            self.ticking.push((id, kind, false));
        }
    }
    /// The entity `id` of a type a mod named is about to tick, told alone to the mods that named
    /// it (`native_tick_pre_named`): the mods that didn't hear it with its family.
    pub(super) fn add_named(&mut self, id: u32, kind: TickKind) {
        if self.told.listening {
            self.ticking.push((id, kind, true));
            self.named = true;
        }
    }
    /// The `n`th entity, in order, that the family's Pre let through, of those told with it
    /// alone (not of a named type).
    pub(super) fn let_through(&self, n: usize) -> Option<u32> {
        if !self.named {
            return self.ticking.get(n).map(|(id, ..)| *id);
        }
        self.ticking
            .iter()
            .filter(|(_, _, named)| !named)
            .nth(n)
            .map(|(id, ..)| *id)
    }
    /// Adds to `canceled` the entities of `ticking` the calls of a Pre canceled (`call_canceled`,
    /// what each call's listeners pushed of what that call was told), each once and in their
    /// order, and takes them out of `ticking`.
    fn take_canceled(&mut self) {
        if self.call_canceled.is_empty() {
            return;
        }
        let mut kept = 0;
        for i in 0..self.ticking.len() {
            let id = self.ticking[i].0;
            if self.call_canceled.contains(&id) {
                self.canceled.push(id);
            } else {
                self.ticking.swap(kept, i);
                kept += 1;
            }
        }
        self.ticking.truncate(kept);
        self.call_canceled.clear();
    }
    /// Keeps for the family's Post only the entities whose tick ran (`ran`): one a hook took
    /// out of the level after the family's Pre, before its turn, never ticked.
    pub(super) fn keep_ran(&mut self, ran: impl Fn(u32) -> bool) {
        if self.told.listening {
            self.ticking.retain(|(id, ..)| ran(*id));
        }
    }
    /// The entity `id`, told its Pre, went before it ticked (an item merged into an earlier
    /// one): it hears no Post.
    pub(super) fn not_ticked(&mut self, id: u32) {
        if self.listening() {
            self.ticking.retain(|(other, ..)| *other != id);
        }
    }
}

impl Game<'_> {
    /// A round for a family about to tick, on the server or on a client, only while a mod
    /// listens (`native_entity_tick_listened`). Its list is kept between ticks.
    pub(super) fn native_tick_round(&mut self) -> TickRound {
        let told = self.native_tick_told();
        let sections = self.is_logical_server() && self.entity_section_scope().is_some();
        TickRound {
            told,
            sections,
            ticking: if told.listening || sections {
                std::mem::take(&mut self.creatures.tick_buffer)
            } else {
                Vec::new()
            },
            named: false,
            canceled: Vec::new(),
            call_canceled: Vec::new(),
            halted: Vec::new(),
        }
    }

    /// Whether the tick hooks are heard on this side (`native_entity_tick_listened`), and which
    /// entities are told alone.
    pub(super) fn native_tick_told(&mut self) -> Told {
        let mut listening = false;
        self.native_entity_tick_listened(&mut listening);
        let alone = match mod_api::__private::entity_tick_each() {
            Some(each) if listening && each.all() => Alone::All,
            Some(each) if listening && each.any() => Alone::Named,
            _ => Alone::None,
        };
        Told { listening, alone }
    }

    /// `native_entity_tick_pre`, once for the family's entities the round holds (`add`): those
    /// a mod cancels leave it and do not tick. With an entity of a named type among them
    /// (`add_named`), the mods that named nothing it holds hear the whole family, and each mod
    /// that named a type hears it without what it hears alone; a cancel from any of them stops
    /// the entity, and then no mod hears it alone. Every call is told the same entities.
    pub(super) fn native_tick_pre_batch(&mut self, round: &mut TickRound) {
        if !round.told.listening || round.ticking.is_empty() {
            return;
        }
        self.native_tell_family(
            || {
                round
                    .ticking
                    .iter()
                    .map(|(id, kind, named)| (*id, kind.id(), *named))
            },
            round.named,
            Some(&mut round.call_canceled),
        );
        round.take_canceled();
    }

    /// `native_entity_tick_post`, once for the family's entities the round let through, after
    /// their position writes have already filed and told their crossings, to the same mods as
    /// its Pre.
    pub(super) fn native_tick_post_batch(&mut self, mut round: TickRound) {
        if round.told.listening && !round.ticking.is_empty() {
            self.native_tick_family(&round, None);
        }
        if round.listening() {
            round.ticking.clear();
            self.creatures.tick_buffer = round.ticking;
        }
    }

    /// The family's Pre (gathering what each call canceled of what it was told into
    /// `canceled`) or its Post, to each group of listeners with what it hears of the family.
    fn native_tick_family(&mut self, round: &TickRound, canceled: Option<&mut Vec<u32>>) {
        self.native_tell_family(
            || {
                round
                    .ticking
                    .iter()
                    .map(|(id, kind, named)| (*id, kind.id(), *named))
            },
            round.named,
            canceled,
        );
    }

    /// The Pre (gathering what each call canceled of what it was told into `canceled`) or the
    /// Post of a family's `entries` (each entity, its type, and whether a mod named it): to every
    /// listener when `named` is false (none is of a named type); otherwise the whole family to
    /// the mods that named nothing, and to each mod that named a type the family without what it
    /// hears alone. Every call is told the same entities, so a cancel in one doesn't hide them
    /// from the next.
    fn native_tell_family<'k, I: Iterator<Item = (u32, &'k str, bool)>>(
        &mut self,
        entries: impl Fn() -> I,
        named: bool,
        mut canceled: Option<&mut Vec<u32>>,
    ) {
        use mod_api::__private::Listeners;
        let each = mod_api::__private::entity_tick_each().filter(|_| named);
        let buffers = std::mem::take(&mut self.creatures.tick_hooks);
        let (mut ids, mut kinds): (_, Vec<&str>) = (buffers.ids, buffers.kinds);
        let mut out = buffers.canceled;
        let mut call = |game: &mut Self, listeners: Listeners<'_>, without: Option<usize>| {
            ids.clear();
            kinds.clear();
            for (id, kind, named) in entries() {
                let alone = named
                    && without.is_some_and(|mod_index| {
                        each.and_then(|each| each.hearing(kind))
                            .is_some_and(|set| set.contains(mod_index))
                    });
                if !alone {
                    ids.push(id);
                    kinds.push(kind);
                }
            }
            if ids.is_empty() {
                return;
            }
            match canceled.as_deref_mut() {
                Some(canceled) => {
                    game.native_entity_tick_pre(game.dim.id(), &ids, &kinds, &mut out, listeners);
                    canceled.extend(out.drain(..).filter(|id| ids.contains(id)));
                }
                None => game.native_entity_tick_post(game.dim.id(), &ids, &kinds, listeners),
            }
        };
        match each {
            None => call(self, Listeners::All, None),
            Some(each) => {
                call(self, Listeners::Except(each.namers()), None);
                for mod_index in each.namers().indices() {
                    call(self, Listeners::One(mod_index), Some(mod_index));
                }
            }
        }
        self.creatures.tick_hooks = TickBuffers::emptied(ids, kinds, out, buffers.names);
    }

    /// `native_entity_tick_pre` for one entity about to tick, told alone: whether it does.
    pub(super) fn native_tick_pre_one(
        &mut self,
        round: &mut TickRound,
        id: u32,
        kind: &str,
    ) -> bool {
        if !round.told.listening {
            return true;
        }
        self.native_tick_pre_alone(round, id, kind, mod_api::__private::Listeners::All)
    }

    /// `native_entity_tick_pre` for an entity of a type a mod named, about to tick, told alone
    /// to the mods that named it (the others heard it with its family): whether it does. One a
    /// mod canceled in its family's Pre isn't told.
    pub(super) fn native_tick_pre_named(
        &mut self,
        round: &mut TickRound,
        id: u32,
        kind: &str,
    ) -> bool {
        if !round.told.listening {
            return true;
        }
        if round.canceled(id) {
            return false;
        }
        let Some(set) = mod_api::__private::entity_tick_each().and_then(|each| each.hearing(kind))
        else {
            return true;
        };
        let ticks =
            self.native_tick_pre_alone(round, id, kind, mod_api::__private::Listeners::Only(set));
        if !ticks {
            // Canceled in its own Pre: the family's Post doesn't name it either.
            round.not_ticked(id);
        }
        ticks
    }

    /// One entity's Pre to `listeners`: whether it ticks.
    fn native_tick_pre_alone(
        &mut self,
        round: &mut TickRound,
        id: u32,
        kind: &str,
        listeners: mod_api::__private::Listeners<'_>,
    ) -> bool {
        self.native_entity_tick_pre(
            self.dim.id(),
            std::slice::from_ref(&id),
            std::slice::from_ref(&kind),
            &mut round.call_canceled,
            listeners,
        );
        let canceled = round.call_canceled.contains(&id);
        round.call_canceled.clear();
        if canceled {
            round.canceled.push(id);
        }
        !canceled
    }

    /// `native_entity_tick_post` for one entity told alone, right after its tick, its move filed
    /// and told.
    pub(super) fn native_tick_post_one(&mut self, round: &TickRound, id: u32, kind: &str) {
        if round.told.listening {
            self.native_entity_tick_post(
                self.dim.id(),
                std::slice::from_ref(&id),
                std::slice::from_ref(&kind),
                mod_api::__private::Listeners::All,
            );
        }
    }

    /// `native_entity_tick_post` for an entity of a type a mod named, right after its tick, to
    /// the mods that named it.
    pub(super) fn native_tick_post_named(&mut self, round: &TickRound, id: u32, kind: &str) {
        if !round.told.listening {
            return;
        }
        if let Some(set) =
            mod_api::__private::entity_tick_each().and_then(|each| each.hearing(kind))
        {
            self.native_entity_tick_post(
                self.dim.id(),
                std::slice::from_ref(&id),
                std::slice::from_ref(&kind),
                mod_api::__private::Listeners::Only(set),
            );
        }
    }
}

impl Game<'_> {
    /// The entity the server keeps unsimulated at `others[i]` (`other_ticks`), of a type a mod
    /// named, at its own place in the join order: its Pre, then its Post, as nothing of it moves
    /// in between. One the Pre canceled or took out of the level hears no Post.
    pub(in crate::game) fn native_other_tick_alone(&mut self, told: Told, i: usize) {
        let mut buffers = std::mem::take(&mut self.creatures.tick_hooks);
        let (id, join) = (self.others[i].id, self.others[i].join);
        buffers.names.push(&self.others[i].kind);
        if let Some(kind) = buffers.names.names().next() {
            let mut round = TickRound {
                told,
                ..TickRound::default()
            };
            if self.native_tick_pre_named(&mut round, id, kind)
                && self.others.locate(join).is_some()
            {
                self.native_tick_post_named(&round, id, kind);
            }
        }
        buffers.names.clear();
        self.creatures.tick_hooks = buffers;
    }

    /// The server's unsimulated entities (`other_ticks`) joined before `born` and in
    /// entity-ticking chunks, once for the family: the Pre and then the Post of those it let
    /// through. One of a named type, told alone to the mods that named it at its place in the
    /// walk (`native_other_tick_alone`), is told here to the others.
    pub(in crate::game) fn native_others_tick_family(&mut self, told: Told, born: JoinSeq) {
        if !told.listening {
            return;
        }
        let ticking = self.entity_ticking_area();
        let mut names = std::mem::take(&mut self.creatures.tick_hooks.names);
        let mut rows = std::mem::take(&mut self.creatures.tick_others);
        for o in self.others.iter() {
            if o.join < born
                && other_ticks(&o.kind)
                && ticking(o.position.x.floor() as i32, o.position.z.floor() as i32)
            {
                rows.push((o.id, told.alone_kind(&o.kind)));
                names.push(&o.kind);
            }
        }
        if !rows.is_empty() {
            let named = rows.iter().any(|&(_, named)| named);
            let entries = || {
                rows.iter()
                    .zip(names.names())
                    .map(|(&(id, named), kind)| (id, kind, named))
            };
            let mut canceled = std::mem::take(&mut self.creatures.tick_canceled);
            self.native_tell_family(entries, named, Some(&mut canceled));
            // Nothing of them moves in between: the Post of those the Pre let through.
            self.native_tell_family(
                || entries().filter(|(id, ..)| !canceled.contains(id)),
                named,
                None,
            );
            canceled.clear();
            self.creatures.tick_canceled = canceled;
        }
        rows.clear();
        self.creatures.tick_others = rows;
        names.clear();
        self.creatures.tick_hooks.names = names;
    }

    /// `native_entity_tick_pre` for the players about to run `ServerPlayer.tick` (the guests the
    /// server ticks): each alone when a mod named `minecraft:player`, else once for the family.
    /// The guests a Pre canceled are kept for the tick ([`Self::player_tick_canceled`]); their
    /// `doTick` body still runs, as vanilla's connection tick does. The round goes on to their
    /// Posts: [`Self::native_player_tick_post_alone`] after each turn, then
    /// `native_tick_post_batch` for the family.
    pub(in crate::game) fn native_players_tick_pre(&mut self) -> TickRound {
        self.creatures.canceled_players.clear();
        let mut round = self.native_tick_round();
        if !round.told.listening {
            return round;
        }
        let Some(host) = self.lan.as_ref() else {
            return round;
        };
        if round.alone_kind(PLAYER) {
            // The mods that didn't name players hear them once for the family, first; then each
            // player its family's Pre let through is told alone to the mods that did.
            for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
                round.add_named(g.entity_id as u32, TickKind::Fixed(PLAYER));
            }
            self.native_tick_pre_batch(&mut round);
            // Taken one by one by index: a Pre's listener may change the guests.
            let guests = self.lan.as_ref().map_or(0, |host| host.hub.guests.len());
            for gi in 0..guests {
                let Some((conn, id)) = self.lan.as_ref().and_then(|host| {
                    let g = host.hub.guests.get(gi).filter(|g| host.hub.here(g))?;
                    Some((g.conn, g.entity_id as u32))
                }) else {
                    continue;
                };
                if !self.native_tick_pre_named(&mut round, id, PLAYER) {
                    self.creatures.canceled_players.push(conn);
                }
            }
        } else {
            for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
                round.add(g.entity_id as u32, TickKind::Fixed(PLAYER));
            }
            self.native_tick_pre_batch(&mut round);
            if let Some(host) = self.lan.as_ref() {
                let canceled = &mut self.creatures.canceled_players;
                canceled.extend(
                    host.hub
                        .guests
                        .iter()
                        .filter(|g| round.canceled(g.entity_id as u32))
                        .map(|g| g.conn),
                );
            }
        }
        round
    }

    /// `native_entity_tick_post` for the player told alone whose turn just ended, if any.
    pub(in crate::game) fn native_player_tick_post_alone(
        &mut self,
        round: &TickRound,
        told: &mut Option<u32>,
    ) {
        if let Some(id) = told.take() {
            self.native_tick_post_named(round, id, PLAYER);
        }
    }

    /// Whether a mod's Pre canceled guest `conn`'s `ServerPlayer.tick` this tick: each of its
    /// parts skips that guest. One branch while no Pre canceled a player.
    pub(in crate::game) fn player_tick_canceled(&self, conn: u32) -> bool {
        let canceled = &self.creatures.canceled_players;
        !canceled.is_empty() && canceled.contains(&conn)
    }
}

impl TickBuffers {
    /// The buffers back, emptied: collecting a vector's own iterator into one of the same
    /// layout keeps its allocation, so the kinds' buffer outlives the borrow it held.
    fn emptied(
        mut ids: Vec<u32>,
        mut kinds: Vec<&str>,
        mut canceled: Vec<u32>,
        mut names: KindNames,
    ) -> Self {
        ids.clear();
        kinds.clear();
        canceled.clear();
        names.clear();
        Self {
            ids,
            kinds: kinds.into_iter().map(|_| "").collect(),
            canceled,
            names,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round() -> TickRound {
        TickRound {
            told: Told {
                listening: true,
                alone: Alone::None,
            },
            ..TickRound::default()
        }
    }

    /// A canceled entity is skipped, and one that went before its tick (an item merged into an
    /// earlier one) hears no Post; a round nobody listens to keeps nothing.
    #[test]
    fn a_round_keeps_the_entities_that_tick() {
        let mut r = round();
        r.ticking
            .push((1, TickKind::Fixed("minecraft:item"), false));
        r.ticking
            .push((2, TickKind::Fixed("minecraft:item"), false));
        r.canceled.push(3);
        assert!(r.canceled(3));
        assert!(!r.canceled(1));
        r.not_ticked(2);
        let left: Vec<_> = r
            .ticking
            .iter()
            .map(|(id, kind, _)| (*id, kind.id()))
            .collect();
        assert_eq!(left, [(1, "minecraft:item")]);
        let mut quiet = TickRound::default();
        quiet.add(4, TickKind::Fixed("minecraft:item"));
        assert_eq!(quiet.let_through(0), None);
        assert!(!quiet.listening());
        quiet.not_ticked(1);
        assert!(quiet.ticking.is_empty() && !quiet.canceled(1));
    }

    /// A family's Pre cancels only entities it was told about, each once: the others keep
    /// their order and are let through, an ID it was not told about or told twice changes
    /// nothing, and the cancels a round already held stay whatever the Pre's listeners did with
    /// their own list.
    #[test]
    fn a_pre_cancels_only_the_entities_it_was_told() {
        let mut r = round();
        r.canceled.push(9);
        for id in [1, 2, 3, 4] {
            r.add(id, TickKind::Fixed("minecraft:item"));
        }
        r.call_canceled.extend([3, 7, 1, 3]);
        r.take_canceled();
        let through: Vec<_> = (0..).map_while(|n| r.let_through(n)).collect();
        assert_eq!(through, [2, 4]);
        assert_eq!(r.canceled, [9, 1, 3]);
        assert!(r.canceled(1) && r.canceled(3) && !r.canceled(7));
        // A later Pre whose listeners pushed nothing, or emptied what they were given, leaves
        // the round's cancels as they were.
        r.take_canceled();
        assert_eq!(r.canceled, [9, 1, 3]);
    }

    /// With no type named, nothing is told alone; with every type named, everything is, and a
    /// round nobody listens to tells nothing alone whatever was named.
    #[test]
    fn a_round_tells_alone_only_what_was_named() {
        let mut r = round();
        assert!(!r.alone_kind("minecraft:item"));
        r.told.alone = Alone::All;
        assert!(r.alone_kind("minecraft:item"));
        r.told.listening = false;
        assert!(!r.alone_kind("minecraft:item"));
    }

    /// An entity is found where it is now, from a hint on, and before the hint when an
    /// earlier one left the list.
    #[test]
    fn place_finds_an_entity_after_the_list_changed() {
        let list = [10, 20, 30, 40];
        assert_eq!(place(&list, 1, |&id| id == 30), Some(2));
        assert_eq!(place(&list, 3, |&id| id == 20), Some(1));
        assert_eq!(place(&list, 9, |&id| id == 40), Some(3));
        assert_eq!(place(&list, 0, |&id| id == 50), None);
    }
}
