use std::path::PathBuf;

fn main() {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    sunrise_mod_api_gen::generate(&root, &out, sunrise_mod_api_gen::Target::GameplayServer)
        .expect("generate gameplay API");
    if std::env::var_os("CARGO_FEATURE_PARITY_FIXTURES").is_some() {
        sunrise_game_discovery::parity_fixtures::write_crate_table(&manifest, &out)
            .expect("generate parity fixtures");
    }
}
