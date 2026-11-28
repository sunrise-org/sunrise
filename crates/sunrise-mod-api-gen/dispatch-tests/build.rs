fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    for path in sunrise_mod_api_gen::generate_test_hooks(&root, &out_dir)
        .expect("generating dispatch test fixture")
    {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
