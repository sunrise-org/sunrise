//! Exercise the reminder on a rolling binary, installed where self-update expects it.
use std::{fs, process::Command};
use sunrise_native_build::contract::CONTRACT;

#[test]
#[ignore = "daily"]
fn the_rolling_tool_checks_for_updates_only_after_a_successful_everyday_command() {
    // A normal developer build would skip the reminder even with the old dispatch order.
    assert_eq!(
        env!("SUNRISE_TOOL_CHANNEL"),
        "mod-tool",
        "build and run this test with SUNRISE_TOOL_CHANNEL=mod-tool"
    );
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().join("home");
    let (home_var, bin) = if cfg!(windows) {
        ("USERPROFILE", ".sunrise/bin")
    } else {
        ("HOME", ".local/bin")
    };
    let bin = home.join(bin);
    fs::create_dir_all(&bin).unwrap();
    let exe = bin.join(format!("cargo-sunrise-mod{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_cargo-sunrise-mod"), &exe).unwrap();
    let project = scratch.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("sunrise-mod.toml"),
        "id = 'ruby'\nname = 'Ruby'\nversion = '0.1.0'\nside = 'both'\n",
    )
    .unwrap();
    let cache = scratch.path().join("cache");
    let stamp = cache.join("mod-tool-update-check");
    let command = || {
        let mut command = Command::new(&exe);
        command
            .current_dir(&project)
            .env(home_var, &home)
            .env("SUNRISE_MOD_CACHE", &cache)
            .env("SUNRISE_SDK_BASE", scratch.path().join("no-sdk"))
            .env_remove("CI")
            .env_remove("CARGO_NET_OFFLINE")
            // The proxy layer refuses unsupported schemes before opening a connection. Leave
            // the reminder enabled so its real attempt stamp still records any attempted check.
            .env("HTTPS_PROXY", "file://unused")
            .env("NO_PROXY", "");
        command
    };
    for (args, code, reason) in [
        (&["build", "--typo"][..], 1, "--typo"),
        (&["build", "--update-lock"][..], 1, "--update-lock"),
        (&["check", "--build", "nightly", "--uses"][..], 2, "--uses"),
    ] {
        let output = command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.lines().next().unwrap().contains(reason), "{error}");
        assert!(!cache.exists(), "{args:?} checked for tool updates");
        assert!(
            !project.join("target").exists(),
            "{args:?} left build output"
        );
    }
    let fit = command()
        .args(["check", "--build", "nightly"])
        .output()
        .unwrap();
    assert_eq!(fit.status.code(), Some(2), "{fit:?}");
    assert!(String::from_utf8_lossy(&fit.stdout).contains("is a content mod"));
    assert!(fit.stderr.is_empty(), "{fit:?}");
    assert!(!cache.exists());

    let help = command().args(["build", "--help"]).output().unwrap();
    assert!(help.status.success(), "{help:?}");
    assert!(help.stdout.starts_with(b"usage: cargo sunrise-mod build "));
    assert!(help.stderr.is_empty(), "{help:?}");
    assert!(!cache.exists());

    let plan = scratch.path().join("plan.json");
    fs::write(
        &plan,
        format!(
            "{}\n",
            serde_json::json!({
                "contract": CONTRACT,
                "mods": scratch.path().join("no-mods"),
                "build": "b".repeat(40),
            })
        ),
    )
    .unwrap();
    let sdk = command()
        .arg("native-check")
        .stdin(fs::File::open(plan).unwrap())
        .output()
        .unwrap();
    assert!(sdk.status.success(), "{sdk:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&sdk.stdout).unwrap(),
        serde_json::json!({"contract": CONTRACT, "route": null, "mods": []})
    );
    assert!(sdk.stderr.is_empty(), "{sdk:?}");
    assert!(!cache.exists());

    let normal = command().args(["new", "greeter"]).output().unwrap();
    assert!(normal.status.success(), "{normal:?}");
    assert!(normal.stdout.starts_with(b"Created greeter"), "{normal:?}");
    assert!(
        stamp.is_file(),
        "a successful everyday command did not check for updates"
    );
}
