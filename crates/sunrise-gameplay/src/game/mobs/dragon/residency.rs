//! Dragon-owned identity and attachment metadata.
use super::*;
impl crate::game::ServerGame<'_> {
    /// Install a replacement only after retiring the previous parts' sidecars.
    pub fn replace_resident_dragon(&mut self, dragon: Dragon) {
        self.discard_resident_dragon();
        self.common.dragon.replace(dragon);
    }

    /// Physical retirement must forget sidecar data before IDs can be reused,
    /// not merely unregister the attachment index when the parts are dropped.
    pub fn discard_resident_dragon(&mut self) {
        self.forget_dragon_sidecars_before_discard();
        self.common.dragon.discard();
    }

    #[inline(always)]
    fn forget_dragon_sidecars_before_discard(&mut self) {
        if let Some(id) = self
            .common
            .dragon
            .as_ref()
            .filter(|dragon| dragon.id != 0)
            .map(|dragon| dragon.id)
        {
            for part in 0..8 {
                self.forget_entity(dragon_part_id(id, part));
            }
            self.forget_entity(id);
        }
    }

    /// The dragon the level holds and the index (`getSubEntities` order) of
    /// its part with that entity id, while the dragon is there, dying or
    /// not: a part is an entity for as long as its parent is.
    pub(in crate::game::server_side) fn resident_dragon_part_source(
        &self,
        id: u32,
    ) -> Option<(&Dragon, usize)> {
        use sunrise_sim::join_seq::residency::{Source, table::EntityLookup};
        let EntityLookup::Found {
            location,
            value: Target::DragonPart(_),
        } = self.server_resident_entity(id)
        else {
            return None;
        };
        if location.owner.dimension != self.common.dim.key() {
            return None;
        }
        let Source::DragonPart(offset) = location.source else {
            return None;
        };
        Some((self.common.dragon.as_ref()?, usize::from(offset)))
    }

    pub(in crate::game) fn server_resident_dragon_part_index(&self, id: u32) -> Option<usize> {
        self.resident_dragon_part_source(id).map(|(_, part)| part)
    }

    pub(in crate::game::server_side) fn assign_resident_crystal_ids(&mut self) {
        crate::game::entity_residency::assign_resident_ids!(
            self.common.crystals,
            c,
            &mut self.server.next_entity,
            None
        );
    }
}

#[cfg(test)]
mod departure_order_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_resident_game;

    #[test]
    #[ignore = "daily"]
    fn dragon_discard_forgets_part_sidecars_while_the_dragon_is_present() {
        with_resident_game(|game| {
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 100;
            game.replace_resident_dragon(dragon);
            let ids = (0..8).map(|part| dragon_part_id(100, part)).chain([100]);
            for id in ids.clone() {
                game.common.persist.no_physics.insert(id, true);
                game.common.persist.uuids.insert(id, [0, 0, 0, id as i32]);
            }

            game.discard_resident_dragon();

            assert!(game.common.dragon.as_ref().is_none());
            for id in ids {
                assert!(!game.common.persist.no_physics.contains_key(&id));
                assert!(!game.common.persist.uuids.contains_key(&id));
            }
        });
    }
}
