//! The required prepared-root boundary names every input it cannot use. These run in `verify`: they build
//! their own temporary root and read no prepared data.

use sunrise_core::prepared_profile::{Profile, SCHEMA, STAMP};
use sunrise_core::prepared_test_support::{
    require_current_stamp, require_inputs, stamp_problems, unavailable_inputs,
};

fn scratch(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sunrise-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn every_missing_input_is_named() {
    let root = scratch("prepared-missing");
    std::fs::write(root.join("items.json"), "{}").unwrap();
    let found = unavailable_inputs(
        &root,
        &[&["items.json", "facts.json"], &["data/minecraft/recipe"]],
    );
    assert_eq!(
        found,
        ["facts.json is missing", "data/minecraft/recipe is missing"]
    );
    assert!(unavailable_inputs(&root, &[&["items.json"]]).is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
#[should_panic(expected = "facts.json is missing")]
fn the_boundary_panics_naming_the_missing_input() {
    let root = scratch("prepared-required");
    std::fs::write(root.join("items.json"), "{}").unwrap();
    require_inputs(root, &[&["items.json", "facts.json"]]);
}

#[test]
fn the_boundary_returns_a_root_whose_inputs_are_all_there() {
    let root = scratch("prepared-present");
    std::fs::write(root.join("items.json"), "{}").unwrap();
    assert_eq!(require_inputs(root.clone(), &[&["items.json"]]), root);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_root_that_is_not_there_names_each_input() {
    let root = scratch("prepared-absent").join("none");
    assert_eq!(
        unavailable_inputs(&root, &[&["items.json", "worldgen"]]),
        ["items.json is missing", "worldgen is missing"]
    );
}

/// A root stamped by a preparer whose `output_version` is `made_by` (none: no stamp file at all).
fn stamped(name: &str, manifest: serde_json::Value, made_by: Option<u32>) -> std::path::PathBuf {
    let root = scratch(name);
    std::fs::write(root.join("sunrise-resources.json"), manifest.to_string()).unwrap();
    if let Some(version) = made_by {
        let stamp = serde_json::json!({ "output_version": version });
        std::fs::write(root.join(STAMP), stamp.to_string()).unwrap();
    }
    root
}

fn current_manifest() -> serde_json::Value {
    serde_json::json!({ "schema": SCHEMA, "minecraft": sunrise_core::VANILLA_VERSION })
}

#[test]
fn a_current_stamp_passes() {
    let root = stamped(
        "stamp-current",
        current_manifest(),
        Some(Profile::Full.output_version()),
    );
    assert!(stamp_problems(&root).is_empty());
    require_current_stamp(&root);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_stale_output_version_names_both_versions_and_the_repair() {
    let found = Profile::Full.output_version() - 1;
    let root = stamped("stamp-stale", current_manifest(), Some(found));
    assert_eq!(
        stamp_problems(&root),
        [format!(
            "made by output version {found}, this build needs {}",
            Profile::Full.output_version()
        )]
    );
    let panic = std::panic::catch_unwind(|| require_current_stamp(&root)).unwrap_err();
    let message = panic.downcast_ref::<String>().unwrap();
    assert!(
        message.contains(&format!("output version {found}")),
        "{message}"
    );
    assert!(
        message.contains(&format!("needs {}", Profile::Full.output_version())),
        "{message}"
    );
    assert!(message.contains("sunrise-prepare"), "{message}");
    assert!(message.contains(&root.display().to_string()), "{message}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn every_other_stale_stamp_is_diagnosed() {
    let full = Some(Profile::Full.output_version());
    let cases = [
        (
            serde_json::json!({ "schema": SCHEMA - 1, "minecraft": sunrise_core::VANILLA_VERSION }),
            full,
            format!(
                "an older layout (schema {}, this preparer writes {SCHEMA})",
                SCHEMA - 1
            ),
        ),
        (
            serde_json::json!({ "schema": SCHEMA, "minecraft": "1.0" }),
            full,
            format!(
                "prepared for Minecraft 1.0, this build needs {}",
                sunrise_core::VANILLA_VERSION
            ),
        ),
        (
            current_manifest(),
            None,
            format!(
                "made by output version unknown, this build needs {}",
                Profile::Full.output_version()
            ),
        ),
        (
            serde_json::json!({
                "profile": "server",
                "schema": SCHEMA,
                "minecraft": sunrise_core::VANILLA_VERSION,
            }),
            Some(Profile::Server.output_version()),
            "prepared for the server profile, tests need full client resources".to_owned(),
        ),
    ];
    for (index, (manifest, made_by, expected)) in cases.into_iter().enumerate() {
        let root = stamped(&format!("stamp-case-{index}"), manifest, made_by);
        assert_eq!(stamp_problems(&root), [expected]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[test]
fn a_root_without_a_manifest_or_with_a_broken_one_is_refused() {
    let root = scratch("stamp-no-manifest");
    assert_eq!(stamp_problems(&root), ["sunrise-resources.json is missing"]);
    std::fs::write(root.join("sunrise-resources.json"), "{").unwrap();
    let problems = stamp_problems(&root);
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].starts_with("sunrise-resources.json is not valid JSON"),
        "{problems:?}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn float_tokens_are_not_integers_in_the_manifest_or_the_stamp() {
    let root = stamped("stamp-float", current_manifest(), None);
    std::fs::write(
        root.join("sunrise-resources.json"),
        format!(
            r#"{{ "schema": {SCHEMA}.0, "minecraft": "{}" }}"#,
            sunrise_core::VANILLA_VERSION
        ),
    )
    .unwrap();
    let problems = stamp_problems(&root);
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].starts_with("sunrise-resources.json is not valid JSON"),
        "{problems:?}"
    );
    std::fs::write(
        root.join(STAMP),
        format!(
            r#"{{ "output_version": {}.0 }}"#,
            Profile::Full.output_version()
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("sunrise-resources.json"),
        current_manifest().to_string(),
    )
    .unwrap();
    assert_eq!(
        stamp_problems(&root),
        [format!(
            "made by output version unknown, this build needs {}",
            Profile::Full.output_version()
        )]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn an_unreadable_input_is_named_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("prepared-unreadable");
    let file = root.join("facts.json");
    std::fs::write(&file, "{}").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    // A superuser reads it anyway; the check is about the diagnosis, so skip only that case.
    if std::fs::File::open(&file).is_ok() {
        std::fs::remove_dir_all(&root).unwrap();
        return;
    }
    let found = unavailable_inputs(&root, &[&["facts.json"]]);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].starts_with("facts.json is unreadable"),
        "{found:?}"
    );
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}
