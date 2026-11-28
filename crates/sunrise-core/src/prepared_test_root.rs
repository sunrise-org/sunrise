//! The prepared-root boundary of the tests: the one way a test gets prepared data. Pure `std` and
//! `sunrise_core`, so each target that reads prepared resources includes this one file with `#[path]`.

#[cfg(test)]
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[cfg(test)]
use sunrise_core::prepared_test_support::{require_current_stamp, require_inputs};

pub(crate) mod configured {
    use std::path::PathBuf;

    /// The explicitly configured prepared root, without a checkout fallback or input checks.
    pub(crate) fn prepared_root() -> Option<PathBuf> {
        std::env::var_os("SUNRISE_TEST_RESOURCES").map(PathBuf::from)
    }
}

/// The prepared root of a test: `SUNRISE_TEST_RESOURCES`, else `.local/sunrise-client/<version>` in the
/// checkout. The one way a test gets prepared data. A root that exists must carry the current full-profile
/// stamp (checked once per root per process), and every input of `needs` must be present and readable, or
/// the test fails naming each one that is not; it never returns a value a test could skip on.
#[cfg(test)]
pub(crate) fn prepared_root(needs: &[&[&str]]) -> PathBuf {
    let root = configured::prepared_root().unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION)
    });
    if root.is_dir() {
        static VERIFIED_ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
        let verified = VERIFIED_ROOTS.get_or_init(Default::default);
        let known = verified.lock().unwrap().contains(&root);
        if !known {
            require_current_stamp(&root);
            verified.lock().unwrap().insert(root.clone());
        }
    }
    require_inputs(root, needs)
}
