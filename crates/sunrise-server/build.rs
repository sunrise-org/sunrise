#[path = "../../packaging/executable_link.rs"]
mod executable_link;
fn main() {
    println!("cargo:rerun-if-changed=../../packaging/executable_link.rs");
    println!("cargo:rerun-if-changed=../sunrise-guest-stack/src/guest_stack_constants.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    for arg in executable_link::arguments(&os, &env) {
        println!("cargo:rustc-link-arg-bins={arg}");
    }

    fn git(args: &[&str]) -> Option<String> {
        let output = std::process::Command::new("git").args(args).output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    // Rolling tools have the same Cargo version across releases. Identify the checked-out commit and
    // track the ref (not just HEAD's symbolic name) for incremental builds on a branch or worktree.
    if let Some(commit) = git(&["show", "-s", "--format=%H%n%ct", "HEAD"])
        && let Some((sha, timestamp)) = commit.split_once('\n')
    {
        for name in [Some("HEAD".into()), git(&["symbolic-ref", "-q", "HEAD"])]
            .into_iter()
            .flatten()
        {
            if let Some(path) = git(&["rev-parse", "--git-path", &name]) {
                println!("cargo:rerun-if-changed={path}");
            }
        }
        println!("cargo:rustc-env=SUNRISE_SERVER_COMMIT={sha}");
        println!("cargo:rustc-env=SUNRISE_SERVER_COMMIT_TIME={timestamp}");
    }
}
