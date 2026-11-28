//! Cargo-facing generation: the mod API's and the game's generated files.
use super::{Catalog, Inputs, Target, host};
use std::{
    fs,
    path::{Path, PathBuf},
};
mod vocab_boundary;

fn write_vocab_output(
    out_dir: &Path,
    name: &str,
    code: &str,
    inventory: &vocab_boundary::Inventory,
    vocab_root: &Path,
) -> Result<(), String> {
    vocab_boundary::validate_with_modules(name, code, inventory, Some(vocab_root))
        .map_err(|e| format!("{name}: {e}"))?;
    write_if_changed(out_dir.join(name), code).map_err(|e| e.to_string())
}

/// Preserve output timestamps when generation produces identical bytes.
pub fn write_if_changed(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = path.as_ref();
    let bytes = bytes.as_ref();
    match fs::read(path) {
        Ok(previous) if previous == bytes => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::write(path, bytes)
}

/// Generate only this crate's files, returning the watched source paths quietly for the caller's
/// Cargo build script to emit. `root` is supplied at run time by the consuming crate's build script.
pub fn generate(root: &Path, out_dir: &Path, target: Target) -> Result<Vec<PathBuf>, String> {
    let mut inputs = Inputs::default();
    generate_with_vocab(root, out_dir, target, None, &mut inputs)?;
    Ok(inputs.into_paths())
}

/// Generate the gameplay vocabulary files, returning the watched source paths quietly for the
/// caller's Cargo build script to emit.
pub fn generate_gameplay_vocab(root: &Path, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut inputs = Inputs::default();
    let catalog = super::gameplay::Routing::load_tracked(root, &mut inputs)?
        .catalog_tracked(root, &mut inputs)?;
    generate_with_vocab(root, out_dir, Target::Vocab, Some(catalog), &mut inputs)?;
    Ok(inputs.into_paths())
}

fn generate_with_vocab(
    root: &Path,
    out_dir: &Path,
    target: Target,
    catalog: Option<Catalog>,
    inputs: &mut Inputs,
) -> Result<(), String> {
    if let Some(package) = gameplay_package(&target) {
        inputs.record_if_staged(root);
        for (name, code) in super::gameplay::generate_tracked(root, package, inputs)? {
            let code = generated_source(root, &target, name, code)?;
            syn::parse_file(&code).map_err(|e| format!("generated Rust: {name}: {e}"))?;
            write_if_changed(out_dir.join(name), code).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let inventory = vocab_boundary::inventory(root, inputs)?;
    let vocab_root = vocab_boundary::modules::vocab_root(root)?;
    let catalog = match catalog {
        Some(catalog) => catalog,
        None => Catalog::load_tracked(root, inputs)?,
    };
    let areas = host::load_tracked(root, inputs)?;
    let areas_code = host::areas_source_from(root, &areas)?;
    let parts_collected = super::parts::collect_tracked(root, inputs)?;
    let exports = super::exports::sources_tracked(
        root,
        Some(&areas_code),
        Some(&parts_collected.trait_exports),
        inputs,
    )?;
    let host_code = host::generate_tracked(root, &areas, Some(&exports), inputs)?;
    let hooks_code = super::hooks_source(&catalog)?;

    let (keys, parts_source) = {
        let mut expanded_map = exports.clone();
        expanded_map.insert(
            super::parts::OUTPUT.to_owned(),
            parts_collected.prefix.clone(),
        );
        let included = [
            (host::AREAS_OUTPUT, areas_code.as_str()),
            ("mod_api_generated.rs", hooks_code.as_str()),
            ("mod_api_generated_host.rs", host_code.as_str()),
            (super::capability_keys::OUTPUT, ""),
        ];
        let mut index = super::index::Index::new(root, &included, super::index::CRATES)
            .expanding(|_, source| super::exports::expand_with(source, &expanded_map));
        let keys = super::capability_keys::generate_with_index(&mut index)?;
        let parts_source = parts_collected.finish(root, &mut index, inputs)?;
        inputs.extend_paths(index.read.borrow().iter().cloned());
        (keys, parts_source)
    };

    write_vocab_output(
        out_dir,
        super::parts::OUTPUT,
        &parts_source,
        &inventory,
        &vocab_root,
    )?;
    for (name, code) in &exports {
        write_vocab_output(out_dir, name, code, &inventory, &vocab_root)?;
    }
    let outputs = [
        (host::AREAS_OUTPUT, areas_code),
        ("mod_api_generated.rs", hooks_code),
        ("mod_api_generated_host.rs", host_code),
        (super::capability_keys::OUTPUT, keys),
    ];
    for (name, code) in outputs {
        let code = generated_source(root, &target, name, code)?;
        write_vocab_output(out_dir, name, &code, &inventory, &vocab_root)?;
        syn::parse_file(&code).map_err(|e| format!("generated Rust: {name}: {e}"))?;
    }
    Ok(())
}

/// Generate the mod API's Part registration modules, returning watched source paths quietly.
pub fn generate_api_parts(root: &Path, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut inputs = Inputs::default();
    let source = super::parts::registration_source(root, &mut inputs)?;
    write_if_changed(out_dir.join("parts_registration_modules.rs"), source)
        .map_err(|e| e.to_string())?;
    Ok(inputs.into_paths())
}

fn gameplay_package(target: &Target) -> Option<super::GameplayPackage> {
    use super::GameplayPackage::*;
    match target {
        Target::GameplayCommon => Some(Common),
        Target::GameplayServer => Some(Server),
        Target::GameplayClient => Some(Client),
        Target::Vocab => None,
    }
}

fn generated_source(
    root: &Path,
    target: &Target,
    name: &str,
    code: String,
) -> Result<String, String> {
    if gameplay_package(target).is_some()
        && matches!(
            name,
            "mod_api_fire_generated.rs" | "mod_api_goal_view_generated.rs"
        )
        && root.join(".sunrise-source-staged").is_file()
    {
        super::internals::widen_generated(Path::new(name), code)
    } else {
        Ok(code)
    }
}

#[cfg(test)]
mod generated_visibility_tests {
    use super::*;

    #[test]
    fn staged_game_helpers_are_widened_but_source_helpers_are_unchanged() {
        let root =
            std::env::temp_dir().join(format!("fnr-generated-visibility-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let input = "impl Game<'_> { pub(super) fn fire(&mut self) {} }";
        for name in [
            "mod_api_fire_generated.rs",
            "mod_api_goal_view_generated.rs",
        ] {
            assert_eq!(
                generated_source(&root, &Target::GameplayClient, name, input.into()).unwrap(),
                input
            );
        }
        fs::write(root.join(".sunrise-source-staged"), "").unwrap();
        for name in [
            "mod_api_fire_generated.rs",
            "mod_api_goal_view_generated.rs",
        ] {
            assert_eq!(
                generated_source(&root, &Target::GameplayClient, name, input.into()).unwrap(),
                "impl Game<'_> { pub fn fire(&mut self) {} }"
            );
            assert_eq!(
                generated_source(&root, &Target::Vocab, name, input.into()).unwrap(),
                input
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

/// The game's side of the host traits, which `game/host/mod.rs` includes.
pub(crate) const HOST_OUTPUT: &str = "game_host_generated.rs";

/// The generated forwarders from each host area's trait to the inherent `Game` functions.
pub(crate) fn game_host_generated(root: &Path) -> Result<String, String> {
    Ok(format!(
        "mod generated {{\n{}\n}}\n",
        host::game_forwarders(root)?
    ))
}

/// Generate the native-only raw render records, without trait dispatch or subscriptions,
/// returning watched source paths quietly.
pub fn generate_render(root: &Path, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut inputs = Inputs::default();
    let generated = super::dispatch::render(&Catalog::load_tracked(root, &mut inputs)?)?;
    let source = generated.render_dispatch;
    use syn::spanned::Spanned;
    let file = syn::parse_file(&source).map_err(|error| error.to_string())?;
    let lines: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let offset = |point: proc_macro2::LineColumn| lines[point.line - 1] + point.column;
    let mut records = String::from("use super::*;\n");
    for item in file.items {
        let syn::Item::Struct(record) = item else {
            continue;
        };
        let start = record
            .attrs
            .first()
            .map_or_else(|| record.vis.span().start(), |attr| attr.span().start());
        records.push_str(&source[offset(start)..offset(record.span().end())]);
        records.push('\n');
    }
    syn::parse_file(&records).map_err(|error| error.to_string())?;
    write_if_changed(out_dir.join("native_render_generated.rs"), records)
        .and_then(|()| {
            write_if_changed(
                out_dir.join("native_render_fire_generated.rs"),
                generated.render_fire,
            )
        })
        .map_err(|error| error.to_string())?;
    Ok(inputs.into_paths())
}

#[test]
fn render_records_need_no_game_sources() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = workspace.join(format!("target/render-records-only-{}", std::process::id()));
    let declarations = root.join(super::catalog::DECLARATIONS);
    fs::create_dir_all(&declarations).unwrap();
    for entry in fs::read_dir(workspace.join(super::catalog::DECLARATIONS)).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            fs::copy(&path, declarations.join(path.file_name().unwrap())).unwrap();
        }
    }
    assert!(!root.join("crates/sunrise-client").exists());
    let inputs = generate_render(&root, &root).unwrap();
    assert!(inputs.contains(&declarations));
    let generated = fs::read_to_string(root.join("native_render_generated.rs")).unwrap();
    let file = syn::parse_file(&generated).unwrap();
    assert_eq!(
        file.items
            .iter()
            .filter(|item| matches!(item, syn::Item::Struct(_)))
            .count(),
        5
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unchanged_outputs_keep_their_timestamp_but_changed_outputs_replace_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    fs::create_dir_all(&root).unwrap();
    let output = root.join(format!("write-if-changed-test-{}", std::process::id()));
    write_if_changed(&output, b"first").unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    fs::File::options()
        .write(true)
        .open(&output)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    write_if_changed(&output, b"first").unwrap();
    assert_eq!(fs::metadata(&output).unwrap().modified().unwrap(), old);
    write_if_changed(&output, b"second").unwrap();
    assert_eq!(fs::read(&output).unwrap(), b"second");
    fs::remove_file(output).unwrap();
}

#[test]
fn export_module_move_invalidates_tracked_inputs_without_tracking_unrelated_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let src = root.join("crates/sunrise-mod-vocab/src");
    let out = root.join("out");
    fs::create_dir_all(root.join("tools/layering")).unwrap();
    fs::create_dir_all(src.join("exports")).unwrap();
    fs::create_dir_all(src.join("hooks")).unwrap();
    fs::create_dir_all(src.join("host")).unwrap();
    fs::create_dir_all(src.join("parts")).unwrap();
    fs::create_dir_all(src.join("client")).unwrap();
    fs::create_dir_all(&out).unwrap();

    fs::write(root.join("tools/layering/mod_vocab_state.json"), "{}\n").unwrap();
    fs::write(
        src.join("lib.rs"),
        "include!(concat!(env!(\"OUT_DIR\"), \"/mod_api_exports_root.rs\"));\n",
    )
    .unwrap();
    fs::write(
        src.join("hooks/sample.rs"),
        "pub trait Hooks {\n    /// Sample hook.\n    #[hook(dispatch = \"flat\", side = \"server\", contract = \"listen\")]\n    fn sample();\n}\n",
    )
    .unwrap();
    fs::write(
        src.join("host/sample.rs"),
        "pub trait SampleHost {}\npub trait SampleServerHost {}\npub trait SampleClientHost {}\npub trait SampleBackend {}\npub trait SampleBackendMut {}\npub trait SampleClientBackendMut {}\n",
    )
    .unwrap();
    fs::write(
        src.join("parts/unions.rs.in"),
        "pub trait Registrar {} pub trait CapabilityProviders {}\n",
    )
    .unwrap();
    fs::write(
        src.join("exports/attachment_queries.rs"),
        "mod attachment_queries;\npub use attachment_queries::QueryRow;\n",
    )
    .unwrap();
    fs::write(
        src.join("exports/domains.rs"),
        "pub mod client;\npub mod runtime;\npub use client::types::*;\npub use client::camera::CameraState;\n",
    )
    .unwrap();
    fs::write(src.join("attachment_queries.rs"), "pub struct QueryRow;\n").unwrap();
    fs::write(
        src.join("client/mod.rs"),
        "pub mod camera;\npub(crate) mod types;\n",
    )
    .unwrap();
    fs::write(src.join("client/camera.rs"), "pub struct CameraState;\n").unwrap();
    fs::write(src.join("client/types.rs"), "pub struct SampleRef;\n").unwrap();
    fs::write(src.join("runtime.rs"), "pub struct RuntimeState;\n").unwrap();

    let first_inputs = generate(&root, &out, Target::Vocab).unwrap();
    for tracked in &first_inputs {
        assert!(
            tracked.exists(),
            "tracked input must exist on disk so Cargo does not rerun every build: {}",
            tracked.display()
        );
    }
    let file_root = src.join("attachment_queries.rs");
    let dir_root = src.join("attachment_queries/mod.rs");
    let types_file = src.join("client/types.rs");
    let types_dir = src.join("client/types/mod.rs");
    let camera = src.join("client/camera.rs");
    assert!(first_inputs.contains(&file_root), "{first_inputs:?}");
    assert!(first_inputs.contains(&src.join("client/mod.rs")));
    assert!(first_inputs.contains(&types_file), "{first_inputs:?}");
    assert!(!first_inputs.contains(&src.join("runtime/types.rs")));
    assert!(
        !first_inputs
            .iter()
            .any(|tracked| camera == *tracked || (tracked.is_dir() && camera.starts_with(tracked))),
        "unrelated client/camera.rs must not be watched by file or directory: {first_inputs:?}"
    );
    let root_exports = fs::read_to_string(out.join("mod_api_exports_root.rs")).unwrap();
    assert!(
        root_exports.contains(&file_root.to_string_lossy().into_owned()),
        "{root_exports}"
    );

    // Move x.rs -> x/mod.rs for both an exported module and a domain types file without editing exports/.
    fs::create_dir_all(dir_root.parent().unwrap()).unwrap();
    fs::rename(&file_root, &dir_root).unwrap();
    fs::create_dir_all(types_dir.parent().unwrap()).unwrap();
    fs::rename(&types_file, &types_dir).unwrap();
    assert!(
        first_inputs.iter().any(|tracked| !tracked.exists()),
        "moving x.rs -> x/mod.rs must invalidate a previously tracked input"
    );

    let second_inputs = generate(&root, &out, Target::Vocab).unwrap();
    for tracked in &second_inputs {
        assert!(
            tracked.exists(),
            "tracked input after move must exist on disk: {}",
            tracked.display()
        );
    }
    assert!(second_inputs.contains(&dir_root), "{second_inputs:?}");
    assert!(!second_inputs.contains(&file_root), "{second_inputs:?}");
    assert!(second_inputs.contains(&types_dir), "{second_inputs:?}");
    assert!(!second_inputs.contains(&types_file), "{second_inputs:?}");
    let root_exports = fs::read_to_string(out.join("mod_api_exports_root.rs")).unwrap();
    assert!(
        root_exports.contains(&dir_root.to_string_lossy().into_owned()),
        "{root_exports}"
    );

    // Adding the flat twin beside the directory root without removing it is refused on a clean run.
    fs::write(&file_root, "pub struct QueryRow;\n").unwrap();
    let err = generate(&root, &out, Target::Vocab).unwrap_err();
    assert!(
        err.contains(&file_root.to_string_lossy().into_owned())
            && err.contains(&dir_root.to_string_lossy().into_owned()),
        "expected ambiguity refusal naming both candidates, got: {err}"
    );
    fs::remove_file(&file_root).unwrap();

    // Move back x/mod.rs -> x.rs.
    fs::rename(&dir_root, &file_root).unwrap();
    fs::rename(&types_dir, &types_file).unwrap();
    assert!(
        second_inputs.iter().any(|tracked| !tracked.exists()),
        "moving x/mod.rs -> x.rs must invalidate a previously tracked input"
    );
    let third_inputs = generate(&root, &out, Target::Vocab).unwrap();
    assert!(third_inputs.contains(&file_root), "{third_inputs:?}");
    assert!(third_inputs.contains(&types_file), "{third_inputs:?}");
    let root_exports = fs::read_to_string(out.join("mod_api_exports_root.rs")).unwrap();
    assert!(
        root_exports.contains(&file_root.to_string_lossy().into_owned()),
        "{root_exports}"
    );
}
