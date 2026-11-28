//! Vanilla's logical side: the gameplay the server jar holds, common code included.
#![forbid(unsafe_code)]

#[cfg(test)]
mod_platform::__install_engine_test_platform!();

#[macro_use]
extern crate sunrise_gameplay_common;
pub mod allays;
pub mod block_entity_ticks;
pub use sunrise_gameplay_common::{
    brushing, cauldron_contents, class_tags, container_openers, crash_save, entity_uuids,
    execution_role, game_modes, game_tests, hit, hoppers, interaction_shapes, leash_knot, lectern,
    minecart_cells, mount_rules, particle_options, projectile_spawned_validation, smithing_icons,
    structure_blocks, test_blocks, test_entities, test_environments, test_instances, world_clock,
    worn_boots,
};
pub mod cat_spawner;
pub mod custom_spawner;
pub mod dedicated;
pub mod dispense_rules;
pub mod dragon_contacts;
pub mod foxes;
pub mod initial_spawn;
pub mod lan;
pub mod mob_level;
pub mod mob_looting;
pub mod rng_count;
pub mod teleport_transition;
pub mod tick_profile;
pub mod trident;
pub mod village_siege;
pub mod wandering_trader;

extern crate self as sunrise_gameplay;
pub mod game;
#[cfg(test)]
#[path = "../../sunrise-core/src/prepared_test_root.rs"]
pub(crate) mod prepared_test_root;
#[cfg(test)]
mod test_game_dir;
