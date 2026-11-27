#[allow(
    unused_imports,
    reason = "standalone tests and probes that include this file may only use prepared_root"
)]
pub use sunrise_sim::test_resources::*;

use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use sunrise_core::prepared_profile::{MANIFEST, Manifest, Profile};

/// Inputs the render, light and mesh tables of a streaming context read from the prepared root
/// (`Assets::load`, `BlockFacts::load`, `LightTables::load`, `MeshTables::new`, `Biomes::load`).
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not stream"
)]
pub(crate) const STREAMING_INPUTS: &[&str] = &[
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
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not generate"
)]
pub(crate) const WORLDGEN_INPUTS: &[&str] = &[
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
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not host a server"
)]
pub(crate) const SERVER_INPUTS: &[&str] = &[
    "items.json",
    "registries.json",
    "data/minecraft/recipe",
    "data/minecraft/tags",
];

/// The prepared root of a test: `SUNRISE_TEST_RESOURCES`, else `.local/sunrise-client/<version>` in the
/// checkout. The one way a test gets prepared data. A root that exists must carry the current full-profile
/// stamp (checked once per root per process), and every input of `needs` must be present and readable, or
/// the test fails naming each one that is not; it never returns a value a test could skip on.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not need prepared data"
)]
pub(crate) fn prepared_root(needs: &[&[&str]]) -> PathBuf {
    let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
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

/// Why `root`'s preparation stamp is not the one this build's full profile writes, one sentence each.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not need prepared data"
)]
pub(crate) fn stamp_problems(root: &Path) -> Vec<String> {
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
    problems.extend(manifest.version_problems(root, sunrise_core::VANILLA_VERSION));
    problems
}

/// Panics naming every way `root`'s stamp is stale, and the command that prepares it again.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not need prepared data"
)]
pub(crate) fn require_current_stamp(root: &Path) {
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
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not need prepared data"
)]
pub(crate) fn require_inputs(root: PathBuf, needs: &[&[&str]]) -> PathBuf {
    let unavailable = unavailable_inputs(&root, needs);
    assert!(
        unavailable.is_empty(),
        "prepared test resources are required, but {}; set SUNRISE_TEST_RESOURCES to a prepared {} root or \
         run sunrise-prepare (resources root: {})",
        unavailable.join("; "),
        sunrise_core::VANILLA_VERSION,
        root.display()
    );
    root
}

/// What of `needs` under `root` is missing or cannot be read, one sentence each.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not need prepared data"
)]
pub(crate) fn unavailable_inputs(root: &Path, needs: &[&[&str]]) -> Vec<String> {
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
