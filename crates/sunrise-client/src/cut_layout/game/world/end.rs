#![cfg(any(test, feature = "parity-fixtures"))]
#[cfg(test)]
#[path = "end/end_gateway_ticking_tests.rs"]
pub(crate) mod ticking_tests;

#[cfg(any(test, feature = "parity-fixtures"))]
#[path = "end/gateway_cooldown_tests.rs"]
pub(crate) mod cooldown_tests;
