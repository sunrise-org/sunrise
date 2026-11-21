//! #3123: a native mod's Host call the load phase refuses, through the loader the game uses.
use mod_api::__private::{ModPhase, PhaseOwner};
use mod_api::{Host, ServerHost};
use std::path::Path;
use std::sync::Mutex;

struct Capture(Mutex<Vec<String>>);
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        self.0.lock().unwrap().push(record.args().to_string());
    }
    fn flush(&self) {}
}
static LOG: Capture = Capture(Mutex::new(Vec::new()));

struct NoHost;
impl Host for NoHost {}
mod_api::impl_host_areas!(NoHost, except(tags));
// The one Host method the probe's mod calls that its phase permits: a Metadata read with a known answer.
impl mod_api::prelude::TagsHost for NoHost {
    fn fluid_is_same(&self, fluid: &str, other: &str) -> bool {
        mod_api::__private::check_access("fluid_is_same", mod_api::__private::LoadAccess::Metadata);
        fluid == other
    }
}
impl ServerHost for NoHost {}
mod_api::impl_server_host_areas!(NoHost);

fn refused(folder: &Path, mod_id: &str, why: &str) {
    let Err(error) = sunrise_mod_host::native_libraries::load(folder) else {
        panic!("{mod_id} was loaded, not refused");
    };
    assert!(
        error.contains(&format!("mod {mod_id}:")) && error.contains(why),
        "{mod_id}: {error}"
    );
}

/// `folders`: the good mod, the `-C panic=abort` library, the library that says it is built with
/// `panic=abort`, and the library with its own `#[global_allocator]`.
pub fn run(folders: &[&Path]) {
    let [good, abort, abort_identity, allocator] = folders else {
        panic!("four phase folders");
    };
    refused(abort, "phase_abort", "libstd");
    refused(abort_identity, "phase_abort_identity", "panic=abort");
    refused(allocator, "phase_allocator", "#[global_allocator]");

    let libraries = sunrise_mod_host::native_libraries::load(good).expect("the good mod loads");
    mod_api::__private::initialize().unwrap();
    mod_api::__private::install(
        libraries
            .iter()
            .flat_map(|library| (library.entries)())
            .collect::<Vec<_>>()
            .into(),
    );
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    // The window thread before its first config load: a hook runs, and its mod calls a world method.
    let _owner = PhaseOwner::new(ModPhase::Setup);
    for _ in 0..2 {
        let mut stacks = [None, None, None, None, None];
        let mut cancelled = false;
        mod_api::dispatch_potion_brew_pre(&mut NoHost, [1, 2, 3], &mut stacks, &mut cancelled);
        // The listener's cancel is undone and the game goes on.
        assert!(!cancelled);
    }
    // The mod saw its first call refused, and its second refused by the latch though it caught the first.
    assert!(
        mod_api::content::crash::details()
            .iter()
            .any(|(key, value)| *key == "phase probe"
                && value == "answered true first true latched true")
    );
    let lines = LOG.0.lock().unwrap();
    let named: Vec<_> = lines
        .iter()
        .filter(|line| line.contains("native refusal: phase_ok hook potion_brew_pre"))
        .collect();
    // Named once for the mod and the method, with the phase.
    assert_eq!(named.len(), 1, "{lines:?}");
    assert!(named[0].contains("mod API entity is unavailable during Setup"));
}
