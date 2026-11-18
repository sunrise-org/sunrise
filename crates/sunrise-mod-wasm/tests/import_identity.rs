//! A mod built by the build tool names each item of the game it uses; the real fixture loads on
//! the game's table, and is refused, naming the item, when one of those items changed.
#[path = "../src/test_stack.rs"]
mod test_stack;
use sunrise_mod_package::manifest::Package;
use sunrise_mod_wasm::{WasmPackage, load};

fn fixture() -> (WasmPackage, Vec<u8>) {
    let (Ok(fixture), Ok(std)) = (
        std::env::var("SUNRISE_WASM_FIXTURE"),
        std::env::var("SUNRISE_WASM_STD"),
    ) else {
        panic!(
            "SUNRISE_WASM_FIXTURE and SUNRISE_WASM_STD must be set: run with the wasm-mods workflow's environment"
        );
    };
    let path = std::path::Path::new(&fixture);
    let mut package = WasmPackage {
        manifest: Package::read(path).unwrap().manifest,
        module: Package::read_wasm(path).unwrap(),
    };
    package.manifest.inject.clear();
    (package, std::fs::read(std).unwrap())
}

/// The module with the last digit of the first name that starts `prefix` changed: the name keeps
/// its length, so no section moves.
fn changed(module: &[u8], prefix: &[u8]) -> (Vec<u8>, String) {
    let start = module
        .windows(prefix.len())
        .position(|window| window == prefix)
        .expect("the fixture imports such a name");
    let end = start
        + module[start..]
            .iter()
            .position(|&byte| byte == b'#')
            .unwrap()
        + 33;
    let mut out = module.to_vec();
    out[end - 1] = if out[end - 1] == b'0' { b'1' } else { b'0' };
    (out, String::from_utf8(module[start..end].to_vec()).unwrap())
}

#[test]
#[ignore = "needs SUNRISE_WASM_FIXTURE and SUNRISE_WASM_STD, which the wasm-mods workflow builds"]
fn the_fixture_loads_on_the_games_table() {
    test_stack::on_guest_stack(|| {
        mod_api::__private::initialize().unwrap();
        let (package, std) = fixture();
        let loaded = load(&std, vec![package]);
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 1);
    })
}

#[test]
#[ignore = "needs SUNRISE_WASM_FIXTURE and SUNRISE_WASM_STD, which the wasm-mods workflow builds"]
fn a_changed_call_the_fixture_uses_refuses_it_by_name() {
    test_stack::on_guest_stack(|| {
        mod_api::__private::initialize().unwrap();
        for prefix in [&b"call:"[..], b"hook:", b"part:"] {
            let (mut package, std) = fixture();
            let (module, name) = changed(&package.module, prefix);
            package.module = module;
            let loaded = load(&std, vec![package]);
            assert!(loaded.mods.is_empty());
            let reason = loaded.refused[0].1.to_string();
            let item = name.split('#').next().unwrap();
            assert!(
                reason.contains(item.trim_start_matches("call:")) && reason.contains("changed"),
                "{name}: {reason}"
            );
            assert!(
                reason.ends_with("(or a type it carries changed)"),
                "{reason}"
            );
        }
    })
}

/// A mod built before a method was added to the API loads on the game that has it: the table is
/// searched by name, so an entry no import names changes nothing for the mod.
#[test]
#[ignore = "needs SUNRISE_WASM_FIXTURE and SUNRISE_WASM_STD, which the wasm-mods workflow builds"]
fn an_unrelated_host_method_added_to_the_game_refuses_nothing() {
    let (package, _) = fixture();
    let imports_before = sunrise_mod_wasm::fixture::mismatches_with_added_call(
        &package.module,
        "call:Host::a_method_added_after_this_mod_was_built#00112233445566778899aabbccddeeff",
    )
    .unwrap();
    assert!(imports_before.is_empty(), "{imports_before:?}");
    // The same table with a changed method the mod uses is refused, so the check above bites.
    let (module, _) = changed(&package.module, b"call:");
    let refused = sunrise_mod_wasm::fixture::mismatches_with_added_call(
        &module,
        "call:Host::a_method_added_after_this_mod_was_built#00112233445566778899aabbccddeeff",
    )
    .unwrap();
    assert_eq!(refused.len(), 1, "{refused:?}");
}

/// A mod built before this change, by the SDK of the base commit 36839e7ce7d (the one numbered
/// `host` import and the whole-API layout export), is refused once, with the rebuild text.
#[test]
#[ignore = "needs SUNRISE_WASM_OLD_FIXTURE and SUNRISE_WASM_OLD_STD: the fixture and std built by the SDK before the per-item imports"]
fn a_mod_built_before_the_break_is_refused_once_with_the_rebuild_text() {
    test_stack::on_guest_stack(|| {
        mod_api::__private::initialize().unwrap();
        let (Ok(old), Ok(std)) = (
            std::env::var("SUNRISE_WASM_OLD_FIXTURE"),
            std::env::var("SUNRISE_WASM_OLD_STD"),
        ) else {
            panic!("SUNRISE_WASM_OLD_FIXTURE and SUNRISE_WASM_OLD_STD must be set");
        };
        let path = std::path::Path::new(&old);
        let mut package = WasmPackage {
            manifest: Package::read(path).unwrap().manifest,
            module: Package::read_wasm(path).unwrap(),
        };
        package.manifest.inject.clear();
        sunrise_mod_wasm::set_game_build("0123abcd");
        let loaded = load(&std::fs::read(std).unwrap(), vec![package]);
        assert!(loaded.mods.is_empty());
        assert_eq!(loaded.refused.len(), 1, "{:?}", loaded.refused);
        let reason = loaded.refused[0].1.to_string();
        assert!(
            reason.starts_with("built for an older Sunrise, rebuild with SDK sdk-0123abcd: "),
            "{reason}"
        );
    })
}

struct MathHost {
    called: std::sync::atomic::AtomicBool,
}
impl mod_api::prelude::MathHost for MathHost {
    fn math_profile(&self) -> mod_api::MathProfile {
        self.called.store(true, std::sync::atomic::Ordering::SeqCst);
        mod_api::MathProfile::StrictMath
    }
}
impl mod_api::Host for MathHost {}
mod_api::impl_host_areas!(MathHost, except(math));
impl mod_api::ClientHost for MathHost {}
mod_api::impl_client_host_areas!(MathHost);

fn wasm_call_imports(module: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(module) {
        if let Ok(wasmparser::Payload::ImportSection(section)) = payload {
            for import in section.into_imports().flatten() {
                if import.module == "sunrise" && import.name.starts_with("call:") {
                    found.push(import.name.to_owned());
                }
            }
            break;
        }
    }
    found
}

fn minimal_mod() -> (WasmPackage, Vec<u8>) {
    let minimal = std::env::var("SUNRISE_WASM_MINIMAL_MOD").unwrap_or_else(|_| {
        let default_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/minimal_mod/target/sunrise-mod/minimal_mod.smod");
        default_path.to_str().unwrap().to_owned()
    });
    let std = std::env::var("SUNRISE_WASM_STD").unwrap_or_else(|_| {
        let default_std = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/sunrise-wasm-std.wasm");
        default_std.to_str().unwrap().to_owned()
    });
    let path = std::path::Path::new(&minimal);
    if !path.exists() {
        panic!("minimal mod smod does not exist at {minimal}");
    }
    let mut package = WasmPackage {
        manifest: Package::read(path).unwrap().manifest,
        module: Package::read_wasm(path).unwrap(),
    };
    package.manifest.inject.clear();
    (package, std::fs::read(std).unwrap())
}

/// A built mod imports ONLY host calls its code can reach; a one-call mod has exactly one
/// call import, loads on the host table, and executes its hook with observable host result.
#[test]
#[ignore = "needs SUNRISE_WASM_MINIMAL_MOD and SUNRISE_WASM_STD, which the wasm-mods workflow builds"]
fn minimal_mod_imports_only_reached_host_calls_and_executes_with_observable_result() {
    test_stack::on_guest_stack(|| {
        mod_api::__private::initialize().unwrap();
        let (package, std) = minimal_mod();

        // 1. Verify exact call imports in the wasm module: exactly 1 call import.
        let call_imports = wasm_call_imports(&package.module);
        assert_eq!(
            call_imports.len(),
            1,
            "minimal mod must import exactly one host call, found: {call_imports:?}"
        );
        assert!(
            call_imports[0].starts_with("call:Host::math_profile#"),
            "expected math_profile call, found: {}",
            call_imports[0]
        );

        // 2. Verify an unused call being changed/added causes no refusal.
        let mismatches = sunrise_mod_wasm::fixture::mismatches_with_added_call(
            &package.module,
            "call:Host::a_method_never_called#00112233445566778899aabbccddeeff",
        )
        .unwrap();
        assert!(
            mismatches.is_empty(),
            "unused call caused refusal: {mismatches:?}"
        );

        // 3. Verify a used call being changed refuses the mod by name.
        let (mut broken_package, _) = minimal_mod();
        let (broken_module, name) = changed(&broken_package.module, b"call:Host::math_profile#");
        broken_package.module = broken_module;
        let loaded_broken = load(&std, vec![broken_package]);
        assert!(loaded_broken.mods.is_empty());
        let reason = loaded_broken.refused[0].1.to_string();
        let item = name.split('#').next().unwrap();
        assert!(
            reason.contains(item.trim_start_matches("call:")) && reason.contains("changed"),
            "{name}: {reason}"
        );

        // 4. Load minimal mod and execute hook with observable host result.
        let info = mod_api::PackageInfo::new(
            package.manifest.id.clone(),
            package.manifest.version.clone(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![package.manifest.id.clone()],
            vec![],
        );
        let loaded = load(&std, vec![package]);
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 1);
        loaded.initialize(&[info]).expect("guest initialization");

        let _registration =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        let host = MathHost {
            called: std::sync::atomic::AtomicBool::new(false),
        };
        mod_api::testing::with_mods(loaded.mods.into(), || {
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            let mut checked = -2.0;
            mod_api::dispatch_tick_target_millis(&host, &mut checked);
            assert!(
                host.called.load(std::sync::atomic::Ordering::SeqCst),
                "host.math_profile was not called across wire!"
            );
        });
    });
}
