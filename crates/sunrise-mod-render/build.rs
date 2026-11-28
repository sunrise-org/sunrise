fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    for path in
        sunrise_mod_api_gen::generate_render(&root, &out).expect("generating raw render records")
    {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
