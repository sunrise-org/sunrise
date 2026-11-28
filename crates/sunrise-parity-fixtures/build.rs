use std::path::PathBuf;

fn main() {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let client = manifest.join("../sunrise-client");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    println!("cargo::rustc-cfg=test");
    println!("cargo::rustc-check-cfg=cfg(sunrise_parity_fixtures)");
    println!("cargo::rustc-cfg=sunrise_parity_fixtures");
    if std::env::var_os("CARGO_FEATURE_FIXTURES").is_some() {
        sunrise_game_discovery::parity_fixtures::write_bin_table(&client, &out_dir)
            .expect("generate parity fixtures dispatcher table");
    }
}
