//! Two separately built native mods prove data maps (one `DataMap` trait for built-in keys
//! and custom-registry `EntryKey<T>`, mergers, partial removers, and `Registrar` load
//! conditions) end-to-end (#1097, #1199):
//!
//! 1. `chemlib` (library mod `1.0.0`):
//!    - defines `pub struct Gas`, `pub const GASES: RegistryKey<Gas>`, and
//!      `impl CustomRegistryEntry for Gas`;
//!    - defines and registers two maps through the same `DataMap` trait and
//!      `r.register_data_map(...)`:
//!      - `FuelMap` (`type Entry = ItemKey`, id `"chemlib:fuel"`, `DataMapSync::Required`,
//!        custom `merge` + `remove_partial`);
//!      - `GasHeatMap` (`type Entry = EntryKey<Gas>`, id `"chemlib:gas_heat"`,
//!        `DataMapSync::Optional`);
//!    - registers custom load condition `chemlib:has_gas` via `r.register_load_condition`;
//!    - ships base `data/chemlib/data_maps/item/fuel.json`.
//! 2. `boiler` (dependent mod):
//!    - registers `boiler:steam` in `chemlib::GASES`;
//!    - ships `data/chemlib/data_maps/item/fuel.json` (merging into and partially removing
//!      `chemlib`'s entries) and `data/chemlib/data_maps/chemlib/gas/gas_heat.json`
//!      (attaching heat data to `boiler:steam`, filtered by `chemlib:has_gas`);
//!    - verifies `coal_key.data(host, &chemlib::FuelMap)`, `stick_key.data(host, &chemlib::FuelMap)`,
//!      `steam_key.data(host, &chemlib::GasHeatMap)`, `oxygen_key.data(host, &chemlib::GasHeatMap)`,
//!      and `ReloadContext::data_map(&steam_key, &chemlib::GasHeatMap)` in `data_maps_updated`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use sunrise_mod_package::Package;

#[path = "support/own_sdk.rs"]
mod own_sdk;

use own_sdk::{passed, run_game, server_profile, write_file};

const CHEMLIB_SOURCE: &str = r#"
use mod_api::{
    ConditionError, CustomRegistryEntry, DataMap, DataMapSync, EntryKey, Hooks, Identifier,
    ItemKey, LoadCondition, Registrar, RegistrationContext, RegistryKey, RegistryOptions, Tag,
};
use std::sync::OnceLock;

#[derive(Debug, PartialEq, Eq)]
pub struct Gas {
    pub color: u32,
    pub toxic: bool,
}

pub const GASES: RegistryKey<Gas> = RegistryKey::new("chemlib:gas");

impl CustomRegistryEntry for Gas {
    const REGISTRY: RegistryKey<Self> = GASES;
}

static OXYGEN: OnceLock<EntryKey<Gas>> = OnceLock::new();

pub fn oxygen_key() -> EntryKey<Gas> {
    *OXYGEN.get().expect("chemlib registered oxygen")
}

pub struct FuelMap;

impl DataMap for FuelMap {
    type Entry = ItemKey;
    type Value = Vec<i32>;

    fn id(&self) -> &str {
        "chemlib:fuel"
    }

    fn decode(&self, value: &Tag) -> Option<Vec<i32>> {
        let Tag::List(items) = value else {
            return None;
        };
        items
            .iter()
            .map(|item| match item {
                Tag::Int(n) => Some(*n),
                _ => None,
            })
            .collect()
    }

    fn sync(&self) -> DataMapSync {
        DataMapSync::Required
    }

    fn merge(&self, _entry: &Identifier, existing: &Tag, incoming: &Tag) -> Result<Tag, String> {
        match (existing, incoming) {
            (Tag::List(a), Tag::List(b)) => {
                let mut merged = a.clone();
                merged.extend(b.iter().cloned());
                Ok(Tag::List(merged))
            }
            _ => Err("expected list values for chemlib:fuel".into()),
        }
    }

    fn remove_partial(
        &self,
        _entry: &Identifier,
        existing: &Tag,
        remover: &Tag,
    ) -> Result<Option<Tag>, String> {
        match (existing, remover) {
            (Tag::List(items), Tag::Int(drop_val)) => {
                let kept: Vec<Tag> = items
                    .iter()
                    .filter(|item| !matches!(item, Tag::Int(v) if v == drop_val))
                    .cloned()
                    .collect();
                if kept.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(Tag::List(kept)))
                }
            }
            _ => Err("expected list value and int remover for chemlib:fuel".into()),
        }
    }
}

pub struct GasHeatMap;

impl DataMap for GasHeatMap {
    type Entry = EntryKey<Gas>;
    type Value = i32;

    fn id(&self) -> &str {
        "chemlib:gas_heat"
    }

    fn decode(&self, value: &Tag) -> Option<i32> {
        match value {
            Tag::Int(n) => Some(*n),
            _ => None,
        }
    }

    fn sync(&self) -> DataMapSync {
        DataMapSync::Optional
    }
}

struct HasGas;

impl LoadCondition for HasGas {
    fn test(&self, args: &mod_api::serde_json::Value) -> Result<bool, ConditionError> {
        let gas = args
            .get("gas")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConditionError::new("expected string field `gas`"))?;
        Ok(gas == "boiler:steam")
    }
}

#[derive(Default)]
struct ChemLib;

#[mod_api::sunrise_mod]
impl Hooks for ChemLib {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_registry(GASES, RegistryOptions::new().sync(true));
        let oxygen = r.register_entry(
            GASES,
            "chemlib:oxygen",
            Gas {
                color: 0xAA_DD_FF,
                toxic: false,
            },
        );
        let _ = OXYGEN.set(oxygen);
        r.register_data_map(FuelMap);
        r.register_data_map(GasHeatMap);
        r.register_load_condition("chemlib:has_gas", HasGas);
    }
}
"#;

const BOILER_SOURCE: &str = r#"
use mod_api::{
    DataMapsUpdateCause, DataMapsUpdatedEvent, Hooks, Identifier, Registrar, RegistrationContext,
    ReloadContext, ServerHost, ServerStartedEvent,
};
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

static STEAM: OnceLock<mod_api::EntryKey<chemlib::Gas>> = OnceLock::new();
static RELOAD_GAS_OK: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Boiler;

#[mod_api::sunrise_mod]
impl Hooks for Boiler {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
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

    fn data_maps_updated(
        &self,
        context: &dyn ReloadContext,
        event: &mut DataMapsUpdatedEvent<'_>,
    ) {
        if event.registry == "chemlib:gas" && event.cause == DataMapsUpdateCause::ServerReload {
            let steam_key = *STEAM.get().expect("boiler:steam registered");
            if context.data_map(&steam_key, &chemlib::GasHeatMap) == Ok(Some(373)) {
                RELOAD_GAS_OK.store(true, Ordering::SeqCst);
            }
        }
    }

    fn server_started(&self, host: &mut dyn ServerHost, _: &mut ServerStartedEvent) {
        let coal_key = host
            .item_key(&Identifier::parse("minecraft:coal").unwrap())
            .expect("minecraft:coal key");
        let stick_key = host
            .item_key(&Identifier::parse("minecraft:stick").unwrap())
            .expect("minecraft:stick key");
        let steam_key = *STEAM.get().expect("boiler:steam registered");
        let oxygen_key = chemlib::oxygen_key();

        let item_ok = coal_key.data(host, &chemlib::FuelMap) == Ok(Some(vec![200, 300]))
            && stick_key.data(host, &chemlib::FuelMap) == Ok(None);
        let gas_ok = steam_key.data(host, &chemlib::GasHeatMap) == Ok(Some(373));
        let cond_ok = oxygen_key.data(host, &chemlib::GasHeatMap) == Ok(None);
        let reload_ok = RELOAD_GAS_OK.load(Ordering::SeqCst);

        println!("m2m_data_maps item_ok={item_ok} gas_ok={gas_ok} reload_ok={reload_ok} cond_ok={cond_ok}");
        std::process::exit(0);
    }
}
"#;

#[test]
#[ignore = "daily"]
fn two_separately_built_mods_share_data_maps_mergers_and_load_conditions() {
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

    // 1. chemlib library mod (version 1.0.0) with base data_maps/item/fuel.json.
    tool(&root, &["new", "chemlib", "--template", "native"]);
    let chemlib = root.join("chemlib");
    fs::write(chemlib.join("src/lib.rs"), CHEMLIB_SOURCE).unwrap();
    let chemlib_manifest = fs::read_to_string(chemlib.join("sunrise-mod.toml"))
        .unwrap()
        .replace("0.1.0", "1.0.0");
    fs::write(chemlib.join("sunrise-mod.toml"), chemlib_manifest).unwrap();
    write_file(
        &chemlib.join("data/chemlib/data_maps/item/fuel.json"),
        r#"{"values":{"minecraft:coal":[100,200],"minecraft:stick":[50]}}"#,
    );
    let chemlib_smod = root.join("chemlib.smod");
    fs::write(&chemlib_smod, Package::pack_directory(&chemlib).unwrap()).unwrap();

    // 2. boiler dependent mod merging & partially removing fuel.json and attaching gas_heat.json.
    tool(&root, &["new", "boiler", "--template", "native"]);
    let boiler = root.join("boiler");
    fs::write(boiler.join("src/lib.rs"), BOILER_SOURCE).unwrap();
    let boiler_manifest = fs::read_to_string(boiler.join("sunrise-mod.toml")).unwrap();
    fs::write(
        boiler.join("sunrise-mod.toml"),
        format!("{boiler_manifest}\n[dependencies]\nchemlib = \"^1\"\n"),
    )
    .unwrap();
    write_file(
        &boiler.join("data/chemlib/data_maps/item/fuel.json"),
        r#"{"values":{"minecraft:coal":[300]},"remove":{"minecraft:coal":100,"minecraft:stick":50}}"#,
    );
    write_file(
        &boiler.join("data/chemlib/data_maps/chemlib/gas/gas_heat.json"),
        r#"{"values":{"boiler:steam":{"neoforge:conditions":[{"type":"chemlib:has_gas","gas":"boiler:steam"}],"value":373},"chemlib:oxygen":{"neoforge:conditions":[{"type":"chemlib:has_gas","gas":"other:absent"}],"value":90}}}"#,
    );

    let run = boiler.join("target/sunrise-mod/run");
    let _ = fs::remove_dir_all(&run);
    fs::create_dir_all(run.join("game")).unwrap();
    fs::write(run.join("game/eula.txt"), "eula=true\n").unwrap();
    let stdout = tool(
        &boiler,
        &[
            "run",
            "--with",
            chemlib_smod.to_str().unwrap(),
            "--resources",
            stub_resources.to_str().unwrap(),
            "--",
            "--dedicated",
            "--initSettings",
        ],
    );
    let executable = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Starting "))
        .and_then(|line| line.split(" with ").next())
        .map(PathBuf::from)
        .expect("run names the program it starts");
    let prog = executable
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let out = run_game(&root, "both", &profile, &run.join("app"), &prog);
    let line = out
        .lines()
        .find(|l| l.starts_with("m2m_data_maps "))
        .unwrap_or_else(|| panic!("missing marker in output:\n{out}"));
    assert_eq!(
        line, "m2m_data_maps item_ok=true gas_ok=true reload_ok=true cond_ok=true",
        "two-mod data maps test failed: {line}"
    );
}
