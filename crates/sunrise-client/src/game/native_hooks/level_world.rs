use super::*;

/// Server-level helpers for native mods.
impl GameLevel<'_> {
    /// This level's own random (vanilla's `Level.random`): the stream
    /// `native_level_random(dimension)` hands a server hook, for a world hook
    /// that draws as a vanilla block does (`isBonemealSuccess`), so the mod's
    /// draws shift vanilla's as a Java mod's would. `None` where this level
    /// isn't the server's copy.
    pub(in crate::game) fn native_level_random(
        &mut self,
    ) -> Option<&mut sunrise_core::random::LegacyRandom> {
        self.native_authoritative.then_some(&mut self.world.random)
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl Game<'_> {
    /// The random of the server level `level` names: vanilla's `Level.random`
    /// (`level.getRandom()`), the one its explosions, block drops and other
    /// level draws use, so a mod's draws move vanilla's later ones as a Java
    /// mod's do. A parked level (the Nether while the server ticks the
    /// Overworld) answers with its own stream. This is a copy of it as it
    /// stands: draw with `LevelRef::with_random`, which stores the stream
    /// back, so the draws move the level's own. `None` on a client of a
    /// server, for a stale handle and for a custom level not open: the
    /// built-in levels (Overworld, Nether, End) open at server startup and
    /// stay open until shutdown, while custom levels close after 300 ticks
    /// with nobody in them (`close_idle_levels`).
    pub(in crate::game) fn level_random(
        &self,
        level: LevelRef,
    ) -> Option<mod_api::LegacyRandomSource> {
        self.native_level_world(level)?;
        if level.dimension() == self.dim.key() {
            Some(crate::game::api_values::random_to_api(&self.world.random))
        } else {
            self.levels
                .parked_world_of(level.dimension_id())
                .map(|world| crate::game::api_values::random_to_api(&world.random))
        }
    }
    /// Replace the random of the server level `level` names with `random`,
    /// a stream `native_level_random` gave and the mod drew from. False where
    /// that gives `None`.
    pub(in crate::game) fn level_set_random(
        &mut self,
        level: LevelRef,
        random: mod_api::LegacyRandomSource,
    ) -> bool {
        if self.native_level_world(level).is_none() {
            return false;
        }
        let target = if level.dimension() == self.dim.key() {
            Some(&mut self.world.random)
        } else {
            self.levels
                .parked_world_mut_of(level.dimension_id())
                .map(|world| &mut world.random)
        };
        target
            .map(|target| **target = crate::game::api_values::random_to_engine(&random))
            .is_some()
    }
}
