fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_PARITY_FIXTURES").is_some() {
        let manifest = std::path::PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
        );
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
        sunrise_game_discovery::parity_fixtures::write_crate_table(&manifest, &out)
            .expect("generate parity fixtures");
    }
}
