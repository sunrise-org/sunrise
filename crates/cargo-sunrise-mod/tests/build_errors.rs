use object::{Object, ObjectSymbol};
use std::{path::Path, process::Command};

/// A starter native mod in a folder whose name has an `=`, with `extra` added to its manifest
/// and `edit` applied to its source, and the tool run in it.
struct Mod {
    _scratch: tempfile::TempDir,
    cache: std::path::PathBuf,
    project: std::path::PathBuf,
}

impl Mod {
    fn new(extra: &str) -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let parent = scratch.path().join("mods=one");
        std::fs::create_dir(&parent).unwrap();
        let mut this = Self {
            cache: scratch.path().join("cache"),
            project: parent.join("greeter"),
            _scratch: scratch,
        };
        let made = this.tool(&parent, &["new", "greeter", "--template", "native"]);
        assert!(
            made.status.success(),
            "`new` failed:\n{}{}",
            String::from_utf8_lossy(&made.stdout),
            String::from_utf8_lossy(&made.stderr)
        );
        let manifest = this.project.join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, format!("{text}{extra}")).unwrap();
        this.project = std::fs::canonicalize(&this.project).unwrap();
        this
    }

    fn tool(&self, dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
            .current_dir(dir)
            .env("SUNRISE_MOD_CACHE", &self.cache)
            .env_remove("SUNRISE_NATIVE_CARGO_TIMINGS")
            .env_remove("SUNRISE_NATIVE_BUILD_VERBOSE")
            .args(args)
            .output()
            .unwrap()
    }

    /// The mod's source with `change` applied, then built.
    fn build(&self, flags: &[&str], change: impl Fn(&str) -> String) -> std::process::Output {
        let source = self.project.join("src/lib.rs");
        let template = std::fs::read_to_string(&source).unwrap();
        std::fs::write(&source, change(&template)).unwrap();
        let mut args = vec!["build"];
        args.extend_from_slice(flags);
        self.tool(&self.project, &args)
    }

    fn build_log(built: &std::process::Output) -> String {
        let stdout = String::from_utf8_lossy(&built.stdout);
        let log = stdout
            .lines()
            .find_map(|line| line.strip_prefix("Build log: "))
            .unwrap_or_else(|| panic!("no build log path in {stdout}"));
        std::fs::read_to_string(log).unwrap()
    }

    /// What a failed build shows names the mod's own file, and no location in the cache.
    fn assert_names_the_authors_file(&self, built: &std::process::Output) {
        assert!(!built.status.success(), "{built:?}");
        let shown = String::from_utf8_lossy(&built.stderr);
        assert!(
            shown.contains(&self.project.join("src/lib.rs").display().to_string()),
            "{shown}"
        );
        // The error's locations; the line naming the build log is in the cache, as it should be.
        let locations: Vec<_> = shown.lines().filter(|line| line.contains("-->")).collect();
        assert!(!locations.is_empty(), "{shown}");
        assert!(
            locations
                .iter()
                .all(|line| !line.contains(&self.cache.display().to_string())),
            "{shown}"
        );
    }
}

fn type_error(source: &str) -> String {
    source.replace("format!(", "{ let wrong: u32 = \"text\"; wrong }; format!(")
}

/// A starter native mod with a type error, built by the tool on the direct route: the error
/// names the mod's own file, not the copy the build compiles in its cache.
#[test]
#[ignore = "book-quickstart.yml: the run's own SDK"]
fn a_failed_native_build_names_the_authors_file_and_not_the_cache_copy() {
    let project = Mod::new("");
    let built = project.build(&["-v", "--timings"], |source| {
        format!("{source}\nfn diagnostics_probe() {{}}\n")
    });
    assert!(built.status.success(), "{built:?}");
    let stderr = String::from_utf8_lossy(&built.stderr);
    assert!(stderr.contains("function `diagnostics_probe`"), "{stderr}");
    assert!(
        stderr.contains("this build route did not run Cargo"),
        "{stderr}"
    );
    assert!(Mod::build_log(&built).contains("function `diagnostics_probe`"));
    assert!(!String::from_utf8_lossy(&built.stdout).contains("diagnostics_probe"));

    let built = project.build(&[], type_error);
    project.assert_names_the_authors_file(&built);
}

/// The same on the registry route (a crates.io dependency, built by Cargo): it builds when the
/// source is right, and its error names the mod's own file when it is not.
#[test]
#[ignore = "book-quickstart.yml: the run's own SDK"]
fn a_failed_registry_build_names_the_authors_file_and_a_good_one_still_builds() {
    let project = Mod::new("itoa = \"1\"\n");
    let built = project.build(&["--timings"], |source| {
        source.replace(
            "fn greeting(player: &str) -> String {",
            "fn greeting(player: &str) -> String {\n    let _ = itoa::Buffer::new().format(7u32);",
        )
    });
    assert!(built.status.success(), "{built:?}");
    let stderr = String::from_utf8_lossy(&built.stderr);
    let report = stderr
        .lines()
        .find_map(|line| line.strip_prefix("Cargo timings: "))
        .unwrap_or_else(|| panic!("no timing report path in {stderr}"));
    let report = Path::new(report);
    assert!(
        report.is_file() && report.starts_with(dunce::canonicalize(&project.cache).unwrap()),
        "{report:?}"
    );
    assert_eq!(report.file_name().unwrap(), "cargo-timing.html");
    assert!(!stderr.contains("report saved to "), "{stderr}");
    assert!(Mod::build_log(&built).contains("report saved to "));

    let warm = project.build(&["--verbose", "--timings"], |source| {
        format!("{source}\nfn registry_diagnostics_probe() {{}}\n")
    });
    assert!(warm.status.success(), "{warm:?}");
    let stderr = String::from_utf8_lossy(&warm.stderr);
    let warning_at = stderr
        .find("function `registry_diagnostics_probe`")
        .unwrap_or_else(|| panic!("no live compiler warning on stderr:\n{stderr}"));
    let timings_line = format!("Cargo timings: {}", report.display());
    let timings_at = stderr
        .find(&timings_line)
        .unwrap_or_else(|| panic!("no timings line on stderr:\n{stderr}"));
    assert!(
        warning_at < timings_at,
        "compiler warning was not mirrored before build completion:\n{stderr}"
    );
    assert!(stderr.contains("report saved to "), "{stderr}");
    let warm_log = Mod::build_log(&warm);
    assert!(warm_log.contains("function `registry_diagnostics_probe`"));
    assert!(warm_log.contains("report saved to "));
    let warm_stdout = String::from_utf8_lossy(&warm.stdout);
    assert!(!warm_stdout.contains("registry_diagnostics_probe"));
    assert!(!warm_stdout.contains("Cargo timings:"));
    let output = project.project.join("target/sunrise-mod/greeter.smod");
    assert_eq!(
        sunrise_mod_package::Package::read(&output)
            .unwrap()
            .manifest
            .id,
        "greeter"
    );
    let built = project.build(&["--timings"], type_error);
    project.assert_names_the_authors_file(&built);
    assert!(!String::from_utf8_lossy(&built.stderr).contains("Cargo timings:"));
    assert!(
        !output.exists(),
        "the previous package survived a failed build"
    );
}

/// A native mod whose build script compiles one C file with `cc` and whose Rust calls it, built by
/// the tool: the C compiler must match the target the SDK builds for (on Windows MinGW's GCC for
/// `x86_64-pc-windows-gnu`, where `cc` would look for MSVC on an `-msvc` target). The tool's own
/// build runs the script, on the registry route.
#[test]
#[ignore = "daily"]
fn a_native_mod_compiles_its_c_source_with_cc_from_its_build_script() {
    let project = Mod::new("\n[build-dependencies]\ncc = \"1\"\n");
    std::fs::write(
        project.project.join("build.rs"),
        "fn main() {\n    cc::Build::new().file(\"c/answer.c\").compile(\"answer\");\n}\n",
    )
    .unwrap();
    std::fs::create_dir(project.project.join("c")).unwrap();
    std::fs::write(
        project.project.join("c/answer.c"),
        "unsigned int sunrise_c_answer(void) { return 42; }\n",
    )
    .unwrap();
    let built = project.build(&[], |source| {
        let source = source.replace(
            "fn greeting(player: &str) -> String {",
            "fn greeting(player: &str) -> String {\n    let _ = unsafe { sunrise_c_answer() };",
        );
        format!("{source}\nunsafe extern \"C\" {{\n    fn sunrise_c_answer() -> u32;\n}}\n")
    });
    assert!(
        built.status.success(),
        "{}{}",
        String::from_utf8_lossy(&built.stdout),
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(
        project
            .project
            .join("target/sunrise-mod/greeter.smod")
            .is_file()
    );
    // On Linux a dylib may leave an undefined symbol to be resolved at load, so a green build
    // does not show the C function was linked: the library the build left in its cache must
    // define it.
    let name = format!(
        "{}mod_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let (mut pending, mut libraries) = (vec![project.cache.clone()], Vec::new());
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else if entry.file_name().to_str() == Some(name.as_str()) {
                libraries.push(entry.path());
            }
        }
    }
    assert!(!libraries.is_empty(), "no {name} under the cache");
    let defines_the_c_function = |library: &Path| {
        let bytes = std::fs::read(library).unwrap();
        let file = object::File::parse(&*bytes).unwrap();
        file.symbols().chain(file.dynamic_symbols()).any(|symbol| {
            // Mach-O prefixes C names with an underscore.
            symbol.is_definition()
                && symbol
                    .name()
                    .is_ok_and(|name| name.trim_start_matches('_') == "sunrise_c_answer")
        })
    };
    assert!(
        libraries
            .iter()
            .any(|library| defines_the_c_function(library)),
        "no {name} under the cache defines sunrise_c_answer: {libraries:?}"
    );
}
