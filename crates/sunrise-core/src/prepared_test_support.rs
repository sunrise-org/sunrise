//! Pure prepared-root input lists and stamp/input diagnostics for tests.
//!
//! Exposed behind `#[cfg(any(test, feature = "test-support"))]` so tests across crates share the
//! input lists and stamp checks without duplicating or suppressing `dead_code`. The
//! `prepared_root` entry point itself stays in `prepared_test_root.rs` so each target that acquires
//! prepared data includes it with `#[path]` for `sunrise-game-discovery`'s single-target check.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use crate::prepared_profile::{MANIFEST, Manifest, Profile};

/// Inputs the render, light and mesh tables of a streaming context read from the prepared root
/// (`Assets::load`, `BlockFacts::load`, `LightTables::load`, `MeshTables::new`, `Biomes::load`).
pub const STREAMING_INPUTS: &[&str] = &[
    "sunrise-resources.json",
    "facts.json",
    "block-tags.json",
    "block-behaviours.json",
    "occlusion.nbt",
    "render-definitions.json",
    "assets",
    "data/minecraft/worldgen/biome",
];

/// Inputs of the block registry and the biome table without the render tables
/// (`Assets::load`, `Biomes::load`) and the world generator over them.
pub const WORLDGEN_INPUTS: &[&str] = &[
    "sunrise-resources.json",
    "facts.json",
    "block-tags.json",
    "assets",
    "items.json",
    "worldgen",
    "data/minecraft/worldgen",
];

/// Inputs of a server's synchronized data (`item_defaults::read`, `Items::catalog`, `ResolvedSources::load`,
/// `SyncData::load`).
pub const SERVER_INPUTS: &[&str] = &[
    "items.json",
    "registries.json",
    "data/minecraft/recipe",
    "data/minecraft/tags",
];

/// Why `root`'s preparation stamp is not the one this build's full profile writes, one sentence each.
pub fn stamp_problems(root: &Path) -> Vec<String> {
    if !root.join(MANIFEST).is_file() {
        return vec![format!("{MANIFEST} is missing")];
    }
    let manifest = match Manifest::read(root) {
        Ok(manifest) => manifest,
        Err(error) => return vec![error],
    };
    let mut problems = Vec::new();
    if manifest.profile() != Profile::Full {
        problems
            .push("prepared for the server profile, tests need full client resources".to_owned());
    }
    problems.extend(manifest.version_problems(root, crate::VANILLA_VERSION));
    problems
}

/// Panics naming every way `root`'s stamp is stale, and the command that prepares it again.
pub fn require_current_stamp(root: &Path) {
    let problems = stamp_problems(root);
    assert!(
        problems.is_empty(),
        "prepared test resources are stale: {}; run {} (resources root: {})",
        problems.join("; "),
        Profile::Full.repair(root),
        root.display()
    );
}

/// `root`, once every input of `needs` is there and readable; otherwise a panic naming each one that is not.
pub fn require_inputs(root: PathBuf, needs: &[&[&str]]) -> PathBuf {
    let unavailable = unavailable_inputs(&root, needs);
    assert!(
        unavailable.is_empty(),
        "prepared test resources are required, but {}; set SUNRISE_TEST_RESOURCES to a prepared {} root or \
         run sunrise-prepare (resources root: {})",
        unavailable.join("; "),
        crate::VANILLA_VERSION,
        root.display()
    );
    root
}

/// What of `needs` under `root` is missing or cannot be read, one sentence each.
pub fn unavailable_inputs(root: &Path, needs: &[&[&str]]) -> Vec<String> {
    let mut found = Vec::new();
    for input in needs.iter().copied().flatten() {
        let path = root.join(input);
        let opened = match fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => fs::read_dir(&path).map(drop),
            Ok(_) => fs::File::open(&path).map(drop),
            Err(error) => Err(error),
        };
        match opened {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                found.push(format!("{input} is missing"));
            }
            Err(error) => found.push(format!("{input} is unreadable ({error})")),
        }
    }
    found
}
