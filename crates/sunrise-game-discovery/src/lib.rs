//! Source-only helpers shared by the generator and the ignored-test inventory.
#![forbid(unsafe_code)]

use std::{fs, path::Path};

pub mod exports;
pub mod references;

/// Whether a test build can satisfy all `conditions`; feature and platform atoms may vary.
pub fn may_compile_in_tests(conditions: &[String]) -> bool {
    if conditions.is_empty() {
        return true;
    }
    let mut gated = Vec::with_capacity(conditions.len() + 1);
    gated.extend_from_slice(conditions);
    // `include_tests` lets `test` vary; conjoining `test` fixes it to true.
    gated.push("test".to_owned());
    mod_source::modules::may_compile(&gated, true)
}

/// Whether a test build can compile an item carrying `attrs` inside enclosing `conditions`.
pub fn attrs_may_compile_in_tests(conditions: &[String], attrs: &[syn::Attribute]) -> bool {
    let mut local = mod_source::modules::cfg_conditions(attrs);
    if local.is_empty() {
        return may_compile_in_tests(conditions);
    }
    local.extend_from_slice(conditions);
    may_compile_in_tests(&local)
}

/// The `*.rs` file stems of `directory` but `mod`, sorted, each checked to be a Rust identifier.
pub fn module_files(directory: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            if name != "mod" {
                syn::parse_str::<syn::Ident>(&name).map_err(|_| {
                    format!("{}: the file name is not a Rust identifier", path.display())
                })?;
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}
