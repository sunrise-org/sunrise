use super::{
    install_packages, install_parts, install_registration_data, profiler, with_registration_open,
};
use mod_vocab::__private::platform::{Platform, PlatformRefusal, install_platform};
use mod_vocab::testing::engine_platform::{self, EngineTestPlatform};

/// Install the engine test platform before the libtest harness runs.
/// Expand once in a `cfg(test)` crate root, never in a shared library.
#[doc(hidden)]
#[macro_export]
macro_rules! __install_engine_test_platform {
    () => {
        #[used]
        #[cfg_attr(
            any(target_os = "linux", target_os = "android", target_os = "freebsd"),
            unsafe(link_section = ".init_array")
        )]
        #[cfg_attr(
            target_vendor = "apple",
            unsafe(link_section = "__DATA,__mod_init_func")
        )]
        #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
        static INIT_TEST_PLATFORM: extern "C" fn() = {
            extern "C" fn enter() {
                $crate::__private::test_platform::install();
            }
            enter
        };
    };
}

pub fn install() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        engine_platform::install(EngineTestPlatform::new(
            super::part_tests::register_game_keys,
            super::part_tests::install_parts_fixture,
            |mods, run| crate::testing::with_mods(mods, run),
        ));
        let result = install_platform(Platform {
            install_packages,
            install_registration_data,
            install_parts,
            with_registration_open,
            profiler_is_enabled: profiler::is_enabled,
            profiler_start: profiler::start,
            profiler_stop: profiler::stop,
            profiler_summary: profiler::summary,
            profiler_firefox_profile: profiler::firefox_profile,
        });
        assert!(matches!(
            result,
            Ok(()) | Err(PlatformRefusal::AlreadyInstalled)
        ));
    });
}
