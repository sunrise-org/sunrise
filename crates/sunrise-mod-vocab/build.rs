fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    for path in sunrise_mod_api_gen::generate_gameplay_vocab(&root, &out_dir)
        .expect("generating the mod API")
    {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
