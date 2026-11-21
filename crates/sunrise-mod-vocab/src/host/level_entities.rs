//! The `level_entities` area: entities and players in a level, and what a level spawns as items and experience. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_entities` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelEntitiesBackend {
    /// Side: server.
    /// Every available member of the server level's entity lookup
    /// (`ServerLevel.getAllEntities`), including spectators and entities outside
    /// any spatial query. Dragon parts are not lookup members.
    /// Returns None on a remote client, for a stale handle or a parked level.
    /// Returns None while any lookup member is temporarily extracted, rather
    /// than returning an incomplete list.
    /// Order is lookup admission order, preserved across temporary extraction;
    /// removal and re-admission moves an entity to the end. An entity spawned
    /// during play is admitted as it spawns, even when its id comes later. A
    /// chunk's entities join in the order it stored them: section by section,
    /// each in the order its entities entered it, as vanilla stores them. A
    /// server section crossing is recorded as the entity moves
    /// (`Callback.onMove`), whether or not a mod listens: a move out and back is
    /// two crossings, and a passenger enters after its vehicle. Minecarts,
    /// boats, projectiles, items, primed TNT, falling blocks, fireworks, orbs and
    /// the other server entities that report after their own update count
    /// several moves within one tick as one, from where it was to where it
    /// ended; a block piston's push is recorded only while a mod listens to
    /// `entity_entering_section`. Entities kept outside the residency families
    /// (lingering potion clouds, lightning bolts, eyes of ender) are not
    /// members.
    /// Allocates a result buffer, not one allocation per entity; sorting is
    /// O(n log n), followed by one authoritative handle lookup per candidate.
    #[access(WorldOnly)]
    fn level_all_entities(&self, level: LevelRef) -> Option<Vec<EntityRef>>;
    /// Side: both.
    /// Entities whose bounding boxes intersect this box (`Level.getEntities`).
    /// Face-only contact does not intersect; spectator players are excluded.
    /// Returns None for a stale handle or a parked level: its entity handles
    /// cannot resolve until the level is in place, and for a bound that is not
    /// finite (NaN or infinite). A minimum above its maximum is the same box
    /// with the two swapped, as vanilla's `AABB`. On a client (`ClientLevel`),
    /// the entities the server tracks to it, other players among them, and the
    /// window's own player, with the same walk.
    /// None there before login and for a level it does not show. The walk
    /// is in entity-list order, then item frames and paintings, followed by players. Every entity list is walked
    /// once per call: O(entities) regardless of the box. Each overlapping mob
    /// resolves through a mob-list scan: O(K*N) for K overlapping mobs and N mobs. Tridents do not
    /// resolve. On the server block displays, item displays, text displays, markers and
    /// interactions are included using physical boxes (zero-sized for displays and markers),
    /// not display culling bounds. Eyes of ender and leash knots are not yet returned.
    /// Other special entities
    /// are not on the entity-list walk. The dragon follows ordinary entities;
    /// its eight intersecting parts follow players in vanilla hash-table order (`Level.getEntities`).
    /// During `hears_projectile_impacts`, the projectile list is taken out of
    /// the game and is not visible; it is restored for `projectile_impact`.
    #[access(WorldOnly)]
    fn level_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Option<Vec<EntityRef>>;
    /// Side: server.
    /// `ServerLevel.players`: the online players in this level, parked or in
    /// place, each once under their own UUID, also during a guest's turn. Fake
    /// players are not listed, as vanilla's list leaves out connectionless
    /// players; a player displaced by a nested guest turn is not listed either,
    /// since its handle cannot resolve until that turn ends. None on a client
    /// and for a stale handle.
    #[access(WorldOnly)]
    fn level_online_players(&self, level: LevelRef) -> Option<Vec<PlayerRef>>;
    /// Side: server.
    /// Every entity the segment from `from` to `to` passes through, nearest first: the test
    /// `ProjectileUtil.getEntityHitResult` makes on each candidate, each entity's box grown by
    /// `margin` on every side and clipped against the segment. A ray that starts inside a box
    /// does not hit that entity, as in vanilla. `exclude` (the shooter) is skipped. Vanilla's
    /// filter (`canHitEntity`, `isPickable`) is the caller's: take the first hit it accepts.
    /// Candidates are those `entities_in` finds, so spectators are not hit and the same kinds
    /// are left out. None for a negative or non-finite margin, a non-finite point, and where
    /// `entities_in` is None (a stale handle, a parked level).
    #[access(WorldOnly)]
    fn level_ray_trace_entities(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        margin: f64,
        exclude: Option<EntityRef>,
    ) -> Option<Vec<EntityRayHit>>;
}
/// The handles' writes of the `level_entities` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelEntitiesBackendMut {
    /// Side: server.
    /// Drops `stack` as a broken block drops it (`Block.popResource`): an
    /// item entity at a random point within a quarter block of the middle of
    /// `pos` (three draws from the level random, made for an empty stack and
    /// with `block_drops` off too), with its own random motion and the
    /// default pickup delay of 10 ticks. None, with nothing spawned, for an
    /// empty stack or an unknown item, with the `block_drops` rule off, on a
    /// client, for a stale handle, a level not in place, once the entity ids run out, or an
    /// item a mod kept out (`entity_join_level`).
    /// Inside a transaction its join waits for the root commit, so a mod keeping the item out is not
    /// known at the call: the handle is returned and reads stale from the commit on.
    #[access(WorldOnly)]
    fn level_pop_resource(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: ItemStack,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Spawns `stack` as an item entity at `x y z` (`new ItemEntity(level, x,
    /// y, z, stack)` and `addFreshEntity`): its own random motion (up 0.2, up
    /// to 0.1 either way across), with `pickup_delay` ticks before it can be
    /// picked up, as `ItemEntity.setPickupDelay` at construction (vanilla
    /// passes 10 for a block's drop, 40 for a player's throw and 32767 for
    /// never). None, with nothing
    /// spawned, for an empty stack or an unknown item, a value that isn't
    /// finite, on a client, for a stale handle, a level not in place, or an
    /// item a mod kept out (`entity_join_level`).
    /// Inside a transaction its join waits for the root commit, so a mod keeping the item out is not
    /// known at the call: the handle is returned and reads stale from the commit on.
    #[access(WorldOnly)]
    fn level_spawn_item(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        pickup_delay: u32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// `spawn_item` with the item entity's motion set to
    /// `motion` (blocks a tick), as `new ItemEntity(level, x, y, z, stack,
    /// deltaX, deltaY, deltaZ)`, and the same `pickup_delay` ticks before it
    /// can be picked up (`ItemEntity.setPickupDelay`). None in the same
    /// cases, and for a motion that isn't finite.
    #[access(WorldOnly)]
    fn level_spawn_item_with_motion(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        motion: [f64; 3],
        pickup_delay: u32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Awards `amount` experience at `x y z` as vanilla's `ExperienceOrb.award`
    /// does: orbs of the largest sizes first (2477, 1237, ... 3, 1), each
    /// drawing the id `tryMergeToExisting` compares from the level random and
    /// merging into an orb of the same value whose box meets the 1 block box
    /// around `x y z`, or else a new orb with its own random motion. The
    /// `block_drops` rule doesn't apply (vanilla's blocks check it before
    /// they award). True once the orbs are awarded, even if a mod keeps a new
    /// one out (`entity_join_level`). False, with no orbs, for 0, a
    /// value that isn't finite, on a client, for a stale handle or a level
    /// not in place.
    #[access(WorldOnly)]
    fn level_award_experience(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        amount: u32,
    ) -> bool;
}
