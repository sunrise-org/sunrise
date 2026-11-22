//! Thin executable for the shared server CLI and supervisor library.
fn main() -> std::process::ExitCode {
    if let Some(code) = sunrise_native_build::registry_rustc::wrapper() {
        return std::process::ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    sunrise_platform::prepare_main_stack();
    sunrise_server::main()
}
