#![cfg(any(test, feature = "parity-fixtures"))]
#[cfg(feature = "client")]
#[cfg(any(test, feature = "parity-fixtures"))]
#[path = "gamemode_split_tests_client.rs"]
pub(crate) mod gamemode_split_tests;
