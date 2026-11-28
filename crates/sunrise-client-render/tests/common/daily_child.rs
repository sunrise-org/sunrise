//! Runs a render pack daily in a fresh child process of its own test binary, shared by the pack
//! test files that `#[path]`-include it. The child runs only the calling test, with throwaway
//! homes; the parent passes only when the child ran exactly that one test and it passed. libtest
//! exits 0 when `--exact` matches no test, so the exit status alone would let a renamed test pass
//! without running anything (#6462).

use std::path::Path;

/// The calling test's path inside its test binary, as `--exact` takes it: the name comes from the
/// function itself, never from a literal typed beside it.
macro_rules! test_name {
    () => {{
        fn here() {}
        let path = std::any::type_name_of_val(&here);
        let path = path.strip_suffix("::here").unwrap();
        // Drop the test crate's own name.
        path.split_once("::").map_or(path, |(_, inner)| inner)
    }};
}
pub(crate) use test_name;

/// Runs `fixture` with the output folder `output_env` names inside the child, or starts that
/// child (with `env` set) from the parent. `label` names the fixture in failures.
pub fn run_daily(
    test: &str,
    output_env: &str,
    env: &[(&str, &str)],
    label: &str,
    fixture: impl FnOnce(&Path),
) {
    if let Some(output) = std::env::var_os(output_env) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        pollster::block_on(instance.request_adapter(&Default::default()))
            .unwrap_or_else(|_| panic!("{label} refused: no Vulkan adapter"));
        fixture(Path::new(&output));
        return;
    }

    // Keep the event loop and every writable home in a fresh process.
    let scratch = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--ignored", "--nocapture"])
        .current_dir(scratch.path())
        .env(output_env, scratch.path().join("output"))
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_TELEMETRY_URL", "off")
        .envs(env.iter().copied());
    for name in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_RUNTIME_DIR",
        "SUNRISE_LAUNCHER_TRASH",
        "SUNRISE_LAUNCHER_CLIPBOARD",
        "SUNRISE_LAUNCHER_DESKTOP",
    ] {
        let home = scratch.path().join(name);
        std::fs::create_dir(&home).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        command.env(name, home);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{label} {test} failed");
    assert_ran_one(&stdout, test, label);
}

/// libtest's own summary of the child: it ran one test, that one, and it passed.
pub fn assert_ran_one(stdout: &str, test: &str, label: &str) {
    assert!(
        stdout.lines().any(|line| line.trim() == "running 1 test"),
        "{label} {test}: the child ran no test of that name"
    );
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("test result: ok. 1 passed;")),
        "{label} {test}: the child's test did not pass"
    );
}

/// A child that matched nothing, or failed, fails its wrapper; one that ran and passed doesn't.
#[test]
fn a_child_that_ran_no_test_fails_its_wrapper() {
    let ran = "\nrunning 1 test\ntest pack ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 1.00s\n";
    assert_ran_one(ran, "pack", "fixture");
    let none = "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 10 filtered out; finished in 0.00s\n";
    for stdout in [none, ""] {
        let refused = std::panic::catch_unwind(|| assert_ran_one(stdout, "renamed", "fixture"));
        assert!(refused.is_err(), "{stdout:?} passed");
    }
}

/// The name a test hands its child is its own path, so a rename can't leave a stale one behind.
#[test]
fn a_tests_name_is_its_own_function() {
    assert_eq!(
        test_name!(),
        "daily_child::a_tests_name_is_its_own_function"
    );
}
