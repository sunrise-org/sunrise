//! The body a heal or movement hook lends its listeners, held on the mob side so the damage
//! module lends it without naming a mob's concrete type.
use super::Mob;
use crate::health::{Attributes, HitReads, LivingBody, PlayerBody};
use sunrise_world::{
    player_health::PlayerFloatIds, server_player_damage::PlayerBodyRead,
    synced_floats::SyncedFloats,
};

/// The body a heal or movement hook lends: a mob, or the player in place, whose effects, stacks
/// and UUID come from the reads (as a hit's player victim's do, [`PlayerBody::in_place`]).
#[derive(Clone, Copy)]
pub enum LendBody<'a> {
    Mob(&'a Mob),
    Player {
        id: i32,
        attributes: &'a Attributes,
        data: &'a SyncedFloats,
        ids: PlayerFloatIds,
    },
}
/// A living entity a heal or movement hook lends its listeners for reading
/// ([`LivingView`](mod_api::LivingView)): the body the site holds, and what naming and reading it
/// needs beyond the body, which the game builds only once a listener is subscribed.
#[derive(Clone, Copy)]
pub struct LivingLend<'a> {
    pub body: LendBody<'a>,
    pub reads: &'a dyn Fn() -> Option<HitReads<'a>>,
}
impl<'a> LivingLend<'a> {
    /// A mob's lend.
    pub fn mob(mob: &'a Mob, reads: &'a dyn Fn() -> Option<HitReads<'a>>) -> Self {
        Self {
            body: LendBody::Mob(mob),
            reads,
        }
    }
    /// The player in place's lend: its floats and attributes as the site holds them.
    pub fn player(
        id: i32,
        attributes: &'a Attributes,
        data: &'a SyncedFloats,
        ids: PlayerFloatIds,
        reads: &'a dyn Fn() -> Option<HitReads<'a>>,
    ) -> Self {
        Self {
            body: LendBody::Player {
                id,
                attributes,
                data,
                ids,
            },
            reads,
        }
    }
    /// The entity's id (`Entity.getId`).
    pub fn id(&self) -> u32 {
        match self.body {
            LendBody::Mob(m) => m.id,
            LendBody::Player { id, .. } => id as u32,
        }
    }
    /// The entity's type name.
    pub fn kind(&self) -> &'a str {
        match self.body {
            LendBody::Mob(m) => &m.kind.id,
            LendBody::Player { .. } => "minecraft:player",
        }
    }
    /// The body its reads read, completed from `reads`.
    pub fn living_body(&self, reads: &HitReads<'a>) -> LivingBody<'a> {
        match self.body {
            LendBody::Mob(m) => LivingBody::Mob(m),
            LendBody::Player {
                id,
                attributes,
                data,
                ids,
            } => LivingBody::Player(PlayerBody::in_place(
                id,
                PlayerBodyRead {
                    attributes,
                    data,
                    ids,
                },
                Some(reads),
            )),
        }
    }
}
