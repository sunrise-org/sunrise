//! Two separately built native mods prove custom static registries end-to-end (#5140):
//!
//! 1. `chemlib` (library mod `1.0.0`): defines `pub struct Gas`, `pub const GASES: RegistryKey<Gas>`,
//!    declares `chemlib:gas` with `.default_key("chemlib:empty").sync(true)`, and registers
//!    `chemlib:empty` + `chemlib:oxygen`.
//! 2. `boiler` (dependent mod with `chemlib = { version = "^1", optional = true }`): registers
//!    `boiler:steam` inside `#[cfg(feature = "chemlib")]`, ships `data/boiler/tags/chemlib/gas/hot.json`
//!    listing `["boiler:steam"]`, and in `server_started` verifies:
//!    - `host.registry(chemlib::GASES)` reads both `chemlib:oxygen` and `boiler:steam` by
//!      `EntryKey<Gas>`, by `by_id`, and `id_of`;
//!    - `host.tags().entries(chemlib::GASES, hot_tag)` contains `[steam]` and
//!      `host.tags().is_in_custom_tag(chemlib::GASES, steam, hot_tag)` is `Some(true)`.
//! 3. Run 2 (`boiler` alone, optional `chemlib` absent): starts cleanly with zero errors.
//!
//! The `neoforge:registered` and `neoforge:tag_empty` load conditions on a custom registry are
//! tested with the load conditions that read them (`m2m_load_conditions.rs`).

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use sunrise_mod_package::Package;

const CHEMLIB_SOURCE: &str = r#"
use mod_api::{EntryKey, Hooks, Registrar, RegistrationContext, RegistryKey, RegistryOptions};
use std::sync::OnceLock;

#[derive(Debug, PartialEq, Eq)]
pub struct Gas {
    pub color: u32,
    pub toxic: bool,
}

pub const GASES: RegistryKey<Gas> = RegistryKey::new("chemlib:gas");

static OXYGEN: OnceLock<EntryKey<Gas>> = OnceLock::new();

pub fn oxygen_key() -> EntryKey<Gas> {
    *OXYGEN.get().expect("chemlib registered oxygen")
}

#[derive(Default)]
struct ChemLib;

#[mod_api::sunrise_mod]
impl Hooks for ChemLib {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_registry(
            GASES,
            RegistryOptions::new()
                .default_key("chemlib:empty")
                .sync(true),
        );
        r.register_entry(
            GASES,
            "chemlib:empty",
            Gas {
                color: 0,
                toxic: false,
            },
        );
        let oxygen = r.register_entry(
            GASES,
            "chemlib:oxygen",
            Gas {
                color: 0xAA_DD_FF,
                toxic: false,
            },
        );
        let _ = OXYGEN.set(oxygen);
    }
}
"#;

const BOILER_SOURCE: &str = r#"
use mod_api::{Hooks, HostRegistries, Identifier, Registrar, RegistrationContext, ServerHost, ServerStartedEvent, TagMembers};

#[cfg(feature = "chemlib")]
static STEAM: std::sync::OnceLock<mod_api::EntryKey<chemlib::Gas>> = std::sync::OnceLock::new();

#[derive(Default)]
struct Boiler;

#[mod_api::sunrise_mod]
impl Hooks for Boiler {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        #[cfg(feature = "chemlib")]
        {
            let steam = r.register_entry(
                chemlib::GASES,
                "boiler:steam",
                chemlib::Gas {
                    color: 0xEE_EE_EE,
                    toxic: false,
                },
            );
            let _ = STEAM.set(steam);
        }
        #[cfg(not(feature = "chemlib"))]
        let _ = r;
    }

    fn server_started(&self, host: &mut dyn ServerHost, _: &mut ServerStartedEvent) {
        let mut reg_ok = false;
        let mut tag_ok = false;

        mod_api::if_installed!(host, chemlib, {
            let gases = host.registry(chemlib::GASES).expect("chemlib:gas frozen");
            let oxygen_key = chemlib::oxygen_key();
            let steam_key = *STEAM.get().expect("boiler:steam registered");
            let oxygen_ok = gases.get(oxygen_key).is_some_and(|g| g.color == 0xAA_DD_FF && !g.toxic);
            let steam_ok = gases.get(steam_key).is_some_and(|g| g.color == 0xEE_EE_EE && !g.toxic);
            let by_id_ok = gases.by_id("boiler:steam").is_some_and(|(k, _)| k == steam_key);
            let id_of_ok = gases.id_of(steam_key).is_some_and(|id| id.to_string() == "boiler:steam");
            let default_ok = gases.default_entry().is_some_and(|(_, id, _)| id.to_string() == "chemlib:empty");
            reg_ok = gases.is_synced() && oxygen_ok && steam_ok && by_id_ok && id_of_ok && default_ok;

            let hot_tag = host
                .tags()
                .key(&Identifier::parse("boiler:hot").unwrap())
                .expect("boiler:hot tag key");
            let entries_ok = host.tags().entries(chemlib::GASES, hot_tag)
                == TagMembers::Members(vec![steam_key]);
            let in_tag_ok =
                host.tags().is_in_custom_tag(chemlib::GASES, steam_key, hot_tag) == Some(true)
                    && host.tags().is_in_custom_tag(chemlib::GASES, oxygen_key, hot_tag)
                        == Some(false);
            tag_ok = entries_ok && in_tag_ok;
        });

        println!("m2m_custom_registries reg_ok={reg_ok} tag_ok={tag_ok}");
        std::process::exit(0);
    }
}
"#;

fn write_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn passed(output: Output) -> String {
    assert!(
        output.status.success(),
        "command failed ({:?})\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn run_game(
    root: &Path,
    run_name: &str,
    server_profile: &Path,
    app: &Path,
    program: &str,
) -> String {
    let game_dir = root.join("runs").join(run_name);
    fs::create_dir_all(&game_dir).unwrap();
    fs::write(game_dir.join("eula.txt"), "eula=true\n").unwrap();
    fs::write(
        game_dir.join("server.properties"),
        "online-mode=false\nserver-ip=127.0.0.1\nserver-port=0\nspawn-protection=0\nview-distance=2\nsimulation-distance=2\n",
    )
    .unwrap();
    let mut cmd = Command::new(app.join(program));
    cmd.current_dir(&game_dir)
        .env("LD_LIBRARY_PATH", app)
        .env("SUNRISE_HOME", root.join("home").join(run_name))
        .env(
            "SUNRISE_LAUNCHER_HOME",
            root.join("launcher").join(run_name),
        )
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_NO_AUDIO", "1")
        .env("XDG_DATA_HOME", root.join("data").join(run_name))
        .env("XDG_CONFIG_HOME", root.join("config").join(run_name))
        .args([
            "--game-dir",
            game_dir.to_str().unwrap(),
            "--resources",
            server_profile.to_str().unwrap(),
            "--dedicated",
            "--nogui",
            "--port",
            "0",
        ]);
    passed(cmd.output().unwrap())
}

fn server_profile() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("SUNRISE_TEST_SERVER_PROFILE")
            .expect("set SUNRISE_TEST_SERVER_PROFILE to prepared 26.3 dedicated server resources"),
    );
    assert!(
        path.join("sunrise-resources.json").is_file(),
        "SUNRISE_TEST_SERVER_PROFILE={} has no sunrise-resources.json",
        path.display()
    );
    path
}

#[test]
#[ignore = "daily"]
fn two_separately_built_mods_share_custom_static_registry_and_tags() {
    let profile = server_profile();
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(target).unwrap();
    let scratch = tempfile::tempdir_in(target).unwrap();
    let root = scratch.path().canonicalize().unwrap();

    let stub_resources = root.join("stub-resources");
    fs::create_dir(&stub_resources).unwrap();
    for (name, json) in [
        (
            "sunrise-resources.json",
            r#"{"schema":7,"minecraft":"26.3"}"#,
        ),
        ("sunrise-prepare.json", r#"{"output_version":9}"#),
        ("registries.json", "{}"),
    ] {
        fs::write(stub_resources.join(name), json).unwrap();
    }

    let tool = |dir: &Path, args: &[&str]| {
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
        passed(cmd.output().unwrap())
    };

    // 1. chemlib library mod (version 1.0.0), packaged as .smod.
    tool(&root, &["new", "chemlib", "--template", "native"]);
    let chemlib = root.join("chemlib");
    fs::write(chemlib.join("src/lib.rs"), CHEMLIB_SOURCE).unwrap();
    let chemlib_manifest = fs::read_to_string(chemlib.join("sunrise-mod.toml"))
        .unwrap()
        .replace("0.1.0", "1.0.0");
    fs::write(chemlib.join("sunrise-mod.toml"), chemlib_manifest).unwrap();
    let chemlib_smod = root.join("chemlib.smod");
    fs::write(&chemlib_smod, Package::pack_directory(&chemlib).unwrap()).unwrap();

    // 2. boiler dependent mod with optional dependency on chemlib, shipping a custom-registry
    // tag and a recipe guarded by `neoforge:registered` on `chemlib:gas` = `boiler:steam`.
    tool(&root, &["new", "boiler", "--template", "native"]);
    let boiler = root.join("boiler");
    fs::write(boiler.join("src/lib.rs"), BOILER_SOURCE).unwrap();
    let boiler_manifest = fs::read_to_string(boiler.join("sunrise-mod.toml")).unwrap();
    fs::write(
        boiler.join("sunrise-mod.toml"),
        format!(
            "{boiler_manifest}\n[dependencies]\nchemlib = {{ version = \"^1\", optional = true }}\n"
        ),
    )
    .unwrap();
    write_file(
        &boiler.join("data/boiler/tags/chemlib/gas/hot.json"),
        r#"{"values":["boiler:steam"]}"#,
    );

    let stage = |name: &str, extra_args: &[&str]| -> (PathBuf, String) {
        let run = boiler.join("target/sunrise-mod/run");
        let _ = fs::remove_dir_all(&run);
        fs::create_dir_all(run.join("game")).unwrap();
        fs::write(run.join("game/eula.txt"), "eula=true\n").unwrap();
        let mut args = vec!["run"];
        args.extend_from_slice(extra_args);
        args.extend_from_slice(&[
            "--resources",
            stub_resources.to_str().unwrap(),
            "--",
            "--dedicated",
            "--initSettings",
        ]);
        let stdout = tool(&boiler, &args);
        let executable = stdout
            .lines()
            .find_map(|line| line.strip_prefix("Starting "))
            .and_then(|line| line.split(" with ").next())
            .map(PathBuf::from)
            .expect("run names the program it starts");
        let kept = root.join("builds").join(name);
        let _ = fs::remove_dir_all(&kept);
        // Copy staged app dir so Run 1 and Run 2 stay independent.
        fn copy_dir(src: &Path, dst: &Path) {
            fs::create_dir_all(dst).unwrap();
            for entry in fs::read_dir(src).unwrap() {
                let entry = entry.unwrap();
                let target = dst.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_dir(&entry.path(), &target);
                } else {
                    fs::copy(entry.path(), &target).unwrap();
                }
            }
        }
        copy_dir(&run.join("app"), &kept);
        (
            kept,
            executable
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        )
    };

    // Run 1: boiler built with chemlib present -> both mods load, custom registry + tag verify.
    let (both_app, both_prog) = stage("both", &["--with", chemlib_smod.to_str().unwrap()]);
    let out_both = run_game(&root, "both", &profile, &both_app, &both_prog);
    let line_both = out_both
        .lines()
        .find(|l| l.starts_with("m2m_custom_registries "))
        .unwrap_or_else(|| panic!("missing marker in Run 1:\n{out_both}"));
    assert_eq!(
        line_both, "m2m_custom_registries reg_ok=true tag_ok=true",
        "Run 1 (chemlib + boiler) failed: {line_both}"
    );

    // Run 2: boiler built alone (optional chemlib absent) -> loads cleanly with zero errors.
    let (alone_app, alone_prog) = stage("alone", &[]);
    let out_alone = run_game(&root, "alone", &profile, &alone_app, &alone_prog);
    let line_alone = out_alone
        .lines()
        .find(|l| l.starts_with("m2m_custom_registries "))
        .unwrap_or_else(|| panic!("missing marker in Run 2:\n{out_alone}"));
    assert_eq!(
        line_alone, "m2m_custom_registries reg_ok=false tag_ok=false",
        "Run 2 (boiler alone) failed: {line_alone}"
    );
}
