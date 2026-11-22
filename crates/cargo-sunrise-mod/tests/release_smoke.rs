use std::{path::Path, process::Command};

#[test]
#[ignore = "daily"]
fn tagged_source_release_checks_and_tests_a_required_library() {
    let scratch = std::env::var_os("QS_SCRATCH").expect("book-quickstart must provide QS_SCRATCH");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = Command::new("python3")
        .current_dir(root)
        .args([
            "tools/ci/mod_release_smoke.py",
            "--tool",
            env!("CARGO_BIN_EXE_cargo-sunrise-mod"),
            "--commit",
            env!("SUNRISE_TOOL_BUILT_FROM"),
        ])
        .arg("--scratch")
        .arg(scratch)
        .status()
        .expect("run the signed SDK release smoke");
    assert!(
        status.success(),
        "signed SDK release smoke failed: {status}"
    );
}
