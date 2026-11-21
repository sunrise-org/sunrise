//! Embeds Sunrise's Windows resources in the `sunrise-official` executable.
#[path = "../../packaging/windows/exe.rs"]
mod windows_exe;

fn main() {
    windows_exe::resources();
}
