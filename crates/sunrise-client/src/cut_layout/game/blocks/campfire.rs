#![cfg(any(test, feature = "parity-fixtures"))]
#[cfg(test)]
#[path = "tests/campfire_tests.rs"]
pub(crate) mod campfire_tests;

#[cfg(any(test, feature = "parity-fixtures"))]
#[path = "tests/block_entity_items_tests.rs"]
pub(crate) mod block_entity_items_tests;
