use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use sunrise_mod_package::Package;

#[test]
#[ignore = "daily"]
fn builds_and_checks_a_native_addon_with_local_libraries() {
    // book-quickstart assembles and signs this run's SDK, like the native run smoke test.
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(target).unwrap();
    let scratch = tempfile::tempdir_in(target).unwrap();
    let root = scratch.path().canonicalize().unwrap();
    let library = root.join("native");
    let addon = root.join("test_addon");
    let tool = |dir: &Path, args: &[&str], registry: Option<&str>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"));
        cmd.current_dir(dir)
            .env("SUNRISE_MOD_CACHE", root.join("cache"))
            .env("SUNRISE_HOME", root.join("home"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .args(args);
        if let Some(registry) = registry {
            cmd.env("SUNRISE_MOD_REGISTRY_TEST_BASE", registry);
        }
        cmd.output().unwrap()
    };
    let passed = |output: Output| {
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    passed(tool(
        &root,
        &["new", "native", "--template", "native"],
        None,
    ));
    passed(tool(
        &root,
        &["new", "test_addon", "--template", "native"],
        None,
    ));
    fs::write(
        library.join("src/lib.rs"),
        r#"
#[derive(Default)]
struct Library;
#[mod_api::sunrise_mod]
impl mod_api::Hooks for Library {}
pub fn magic() -> u32 { 1729 }
"#,
    )
    .unwrap();
    fs::write(
        addon.join("src/lib.rs"),
        r#"
#[derive(Default)]
struct Addon;
#[mod_api::sunrise_mod]
impl mod_api::Hooks for Addon {
    fn register(&self, _: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        assert_eq!(native::magic(), 1729);
        println!("addon registered with server library");
        std::process::exit(0);
    }
}
"#,
    )
    .unwrap();
    // The alias must be the mod ID, not the Cargo package's name.
    let cargo = library.join("Cargo.toml");
    let text = fs::read_to_string(&cargo).unwrap();
    fs::write(&cargo, text.replace("native", "check-library-crate")).unwrap();
    let library_manifest = library.join("sunrise-mod.toml");
    let original = fs::read_to_string(&library_manifest)
        .unwrap()
        .replace("0.1.0", "1.0.0");
    fs::write(&library_manifest, &original).unwrap();
    let addon_manifest = addon.join("sunrise-mod.toml");
    let base_manifest = fs::read_to_string(&addon_manifest).unwrap();
    let dependencies = |rows: &str| {
        fs::write(
            &addon_manifest,
            format!("{base_manifest}\n[dependencies]\n{rows}\n"),
        )
        .unwrap()
    };
    dependencies("native = '^1'");
    let path = library.to_str().unwrap();
    let pin = Package::read(&library)
        .unwrap()
        .manifest
        .native
        .unwrap()
        .sunrise;
    // A missing registry cannot stop a complete local build. Check first: the API route should
    // type-check the linkage without either linking mod libraries or fetching an engine slice.
    let offline = Some("http://127.0.0.1:1");
    passed(tool(&addon, &["check", "--with", path], offline));
    let sdk_cache = root.join("cache/sdk").join(&pin);
    let mut hosts = fs::read_dir(sdk_cache.join("api-slices")).unwrap();
    let host = hosts.next().unwrap().unwrap();
    assert!(hosts.next().is_none());
    let slices = sunrise_native_build::prebuilt::Package::at(&host.path());
    assert!(slices.runtime.join("bin").is_dir());
    assert!(slices.api.join("deps").is_dir());
    assert!(!slices.engine.exists());
    assert!(!sdk_cache.join(host.file_name()).join("engine").exists());
    passed(tool(&addon, &["build", "--with", path], offline));

    let content = root.join("content");
    fs::create_dir(&content).unwrap();
    fs::write(
        content.join("sunrise-mod.toml"),
        "id='content'\nname='Content'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n",
    )
    .unwrap();
    let content_package = root.join("content.smod");
    fs::write(&content_package, Package::pack_directory(&content).unwrap()).unwrap();
    dependencies("native = '^1'\ncontent = '^1'");
    let content_path = content_package.to_str().unwrap();
    for command in ["build", "check"] {
        passed(tool(
            &addon,
            &[command, "--with", path, "--with", content_path],
            offline,
        ));
    }
    dependencies("native = '^1'");
    let source = addon.join("src/lib.rs");
    let code = fs::read_to_string(&source).unwrap();
    fs::write(
        &source,
        format!("{code}\nfn wrong() -> &'static str {{ native::magic() }}"),
    )
    .unwrap();
    let refused = tool(&addon, &["check", "--with", path], None);
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success()
            && error.contains("mismatched types")
            && error.contains(&source.display().to_string()),
        "{error}"
    );
    fs::write(&source, code).unwrap();

    let other_pin = "a".repeat(40);
    fs::write(&library_manifest, original.replace(&pin, &other_pin)).unwrap();
    let refused = tool(&addon, &["check", "--with", path], None);
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success()
            && error.contains("native")
            && error.contains(&other_pin)
            && error.contains("uses list"),
        "{error}"
    );
    fs::write(&library_manifest, original).unwrap();
    let package = root.join("native.smod");
    fs::write(&package, Package::pack_directory(&library).unwrap()).unwrap();
    // A packed library is staged from its archive, as a registry library is after its checks.
    let stdout = passed(tool(
        &addon,
        &["build", "--with", package.to_str().unwrap()],
        offline,
    ));
    assert!(
        stdout.contains("Using library native 1.0.0 from"),
        "{stdout}"
    );
    let output = addon.join("target/sunrise-mod/test_addon.smod");
    assert_eq!(Package::read(&output).unwrap().manifest.id, "test_addon");
    assert_eq!(
        Package::entry(&output, "native/src/lib.rs", 4096).unwrap(),
        None
    );

    // `run` also carries the dependencies beside the SDK's executable. Registration only needs
    // a current full-profile marker and the registries table; it exits before a world opens.
    let resources = root.join("resources");
    fs::create_dir(&resources).unwrap();
    for (name, json) in [
        (
            "sunrise-resources.json",
            r#"{"schema":7,"minecraft":"26.3"}"#,
        ),
        ("sunrise-prepare.json", r#"{"output_version":8}"#),
        ("registries.json", "{}"),
    ] {
        fs::write(resources.join(name), json).unwrap();
    }
    let run = addon.join("target/sunrise-mod/run");
    fs::create_dir_all(run.join("game")).unwrap();
    fs::write(run.join("game/eula.txt"), "eula=true\n").unwrap();
    dependencies("native = { version = '^1', side = 'server' }");
    let stdout = passed(tool(
        &addon,
        &[
            "run",
            "--with",
            package.to_str().unwrap(),
            "--resources",
            resources.to_str().unwrap(),
            "--",
            "--dedicated",
        ],
        offline,
    ));
    assert!(
        stdout.contains("addon registered with server library"),
        "{stdout}"
    );
    let records = fs::read_to_string(run.join("app/native-mods.json")).unwrap();
    assert!(
        records.contains("test_addon") && records.contains("native"),
        "{records}"
    );
}
