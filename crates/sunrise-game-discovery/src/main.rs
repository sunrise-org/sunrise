//! Source-only test identities for CI. Input: PACKAGE<TAB>TARGET<TAB>crate-root<TAB>manifest-directory.
//! Output: JSONL records with package, target, path, line, name, ignored and reason.
//! `--files` lists each target's test-build source files; `--declared-files` keeps cfg-gated modules too.
//! `--references --boundary FILE::NAME [--literal TEXT]...
//! [--fragment TEXT]...` writes what each target's functions reach (see `references.rs`) instead, then one
//! `target_files` record (`package`, `target`, `files`) for each target whose source spells a seed. Paths are
//! relative to ROOT with `/` separators.
use mod_source::modules::cfg_conditions;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
};
use sunrise_game_discovery::{
    attrs_may_compile_in_tests, may_compile_in_tests,
    references::{self, Analysis, Seeds},
};
use syn::{Attribute, Expr, Item, ItemFn, Lit, Meta};

const USAGE: &str = "usage: sunrise-game-discovery ROOT [--files | --declared-files | --references --boundary FILE::NAME \
[--literal TEXT]... [--fragment TEXT]...] < targets.tsv";

enum Mode {
    Tests,
    Files,
    DeclaredFiles,
    References(Seeds),
}

fn mode(arguments: impl Iterator<Item = String>) -> Result<Mode, String> {
    let mut arguments = arguments.peekable();
    let mut files = None;
    let mut references = false;
    let mut seeds = Seeds::default();
    let mut boundary = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--files" => files = Some(Mode::Files),
            "--declared-files" => files = Some(Mode::DeclaredFiles),
            "--references" => references = true,
            "--boundary" | "--literal" | "--fragment" => {
                let value = arguments
                    .next()
                    .ok_or(format!("{argument} needs a value"))?;
                match argument.as_str() {
                    "--boundary" => {
                        let (file, name) = value
                            .split_once("::")
                            .ok_or("--boundary takes FILE::NAME")?;
                        seeds.boundary_file = file.to_owned();
                        seeds.boundary_name = name.to_owned();
                        boundary = true;
                    }
                    "--literal" => seeds.exact_literals.push(value),
                    _ => seeds.literal_fragments.push(value),
                }
            }
            _ => return Err(format!("unknown argument {argument}\n{USAGE}")),
        }
    }
    match (files, references, boundary) {
        (None, false, false) => Ok(Mode::Tests),
        (Some(files), false, false) => Ok(files),
        (None, true, true) => Ok(Mode::References(seeds)),
        _ => Err(USAGE.into()),
    }
}

/// `path` as a repository-relative path with `/` separators, on every platform.
fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn literal(attribute: &Attribute) -> Option<String> {
    let Meta::NameValue(value) = &attribute.meta else {
        return None;
    };
    let Expr::Lit(value) = &value.value else {
        return None;
    };
    let Lit::Str(value) = &value.lit else {
        return None;
    };
    Some(value.value())
}

fn parse(
    manifest: &Path,
    exports: Option<&BTreeMap<String, String>>,
    path: &Path,
    source: &str,
    test_build: bool,
) -> Result<syn::File, String> {
    let mut parsed = syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut cfg = if test_build {
        cfg_conditions(&parsed.attrs)
    } else {
        Vec::new()
    };
    if may_compile_in_tests(&cfg) {
        parsed.items = expand_items(manifest, exports, parsed.items, &mut cfg, test_build)?;
    } else {
        // Keep the inner attributes for the walker, but do not visit children of this file.
        parsed.items.clear();
    }
    Ok(parsed)
}

fn expand_items(
    manifest: &Path,
    exports: Option<&BTreeMap<String, String>>,
    original: Vec<Item>,
    cfg: &mut Vec<String>,
    test_build: bool,
) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    for mut item in original {
        if let Item::Mod(child) = &mut item {
            let saved = cfg.len();
            if test_build {
                cfg.extend(cfg_conditions(&child.attrs));
            }
            if !may_compile_in_tests(cfg) {
                cfg.truncate(saved);
                continue;
            }
            if let Some((_, inner)) = &mut child.content {
                *inner = expand_items(manifest, exports, std::mem::take(inner), cfg, test_build)?;
            }
            cfg.truncate(saved);
        }
        if let Item::Macro(included) = &mut item
            && included.mac.path.is_ident("include")
        {
            if test_build && !attrs_may_compile_in_tests(cfg, &included.attrs) {
                continue;
            }
            if let Some(path) = sunrise_game_discovery::exports::env_concat_include(
                included.mac.tokens.clone(),
                "OUT_DIR",
                Path::new(""),
            )? && let Some(file_name) = path.file_name().and_then(|n| n.to_str())
                && file_name.starts_with(sunrise_game_discovery::exports::PREFIX)
            {
                let sources =
                    exports.ok_or("vocabulary exports include without its source fragments")?;
                let source = sources.get(file_name).ok_or_else(|| {
                    format!("unknown vocabulary export include: {}", path.display())
                })?;
                items.extend(syn::parse_file(source).map_err(|e| e.to_string())?.items);
                continue;
            }
            if let Some(path) = sunrise_game_discovery::exports::env_concat_include(
                included.mac.tokens.clone(),
                "CARGO_MANIFEST_DIR",
                manifest,
            )? {
                let path = path.to_str().ok_or("manifest include path is not UTF-8")?;
                let path = syn::LitStr::new(path, included.mac.path.segments[0].ident.span());
                included.mac.tokens = quote::quote!(#path);
            }
        }
        items.push(item);
    }
    Ok(items)
}

fn emit(
    module: &str,
    items: &[Item],
    cfg: &mut Vec<String>,
    write: &mut impl FnMut(&str, &ItemFn) -> Result<(), String>,
) -> Result<(), String> {
    for item in items {
        match item {
            Item::Mod(child) => {
                if let Some((_, items)) = &child.content {
                    let saved = cfg.len();
                    cfg.extend(cfg_conditions(&child.attrs));
                    if may_compile_in_tests(cfg) {
                        let name = if module.is_empty() {
                            child.ident.to_string()
                        } else {
                            format!("{module}::{}", child.ident)
                        };
                        emit(&name, items, cfg, write)?;
                    }
                    cfg.truncate(saved);
                }
            }
            Item::Fn(function)
                if references::is_test(&function.attrs)
                    && attrs_may_compile_in_tests(cfg, &function.attrs) =>
            {
                let name = if module.is_empty() {
                    function.sig.ident.to_string()
                } else {
                    format!("{module}::{}", function.sig.ident)
                };
                write(&name, function)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let root = std::env::args_os().nth(1).ok_or(USAGE)?;
    let mode = mode(std::env::args().skip(2))?;
    let root = PathBuf::from(root)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let export_directory = root.join("crates/sunrise-mod-vocab/src/exports");
    let exports = if export_directory.is_dir() {
        let (scopes, _, _) = sunrise_game_discovery::exports::fragments(
            &root,
            sunrise_game_discovery::module_files(&export_directory)?,
        )?;
        Some(
            scopes
                .into_iter()
                .map(|(scope, source)| {
                    (
                        format!("{}{scope}.rs", sunrise_game_discovery::exports::PREFIX),
                        source,
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        )
    } else {
        None
    };
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let fields: Vec<_> = line.split('\t').collect();
        let [package, target, path, manifest] = fields.as_slice() else {
            return Err("expected PACKAGE<TAB>TARGET<TAB>crate-root<TAB>manifest-directory".into());
        };
        let manifest = root.join(manifest);
        let file = root.join(path);
        let directory = file
            .parent()
            .ok_or("crate root has no parent")?
            .to_path_buf();
        let test_build = !matches!(mode, Mode::DeclaredFiles);
        let files = mod_source::modules::walk_module_files(
            "root".into(),
            file,
            directory,
            false,
            true,
            |path, source| parse(&manifest, exports.as_ref(), path, source, test_build),
        )?;
        let mut analysis = match &mode {
            Mode::References(seeds) => Some(Analysis::new(seeds.clone())),
            _ => None,
        };
        let mut mentioned = false;
        let mut sources = Vec::new();
        for mut file in files {
            if test_build && !may_compile_in_tests(&file.cfg) {
                continue;
            }
            let path = file
                .path
                .canonicalize()
                .map_err(|e| format!("{}: {e}", file.path.display()))?;
            if matches!(mode, Mode::Files | Mode::DeclaredFiles) {
                let relative = path
                    .strip_prefix(&root)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                println!("{}", posix(relative));
                continue;
            }
            let source =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let module = if file.module == "root" {
                ""
            } else {
                file.module
                    .strip_prefix("root::")
                    .ok_or("module walker returned an unexpected root")?
            };
            if let Mode::References(seeds) = &mode {
                mentioned |= seeds.mentions(&source);
                let relative = path
                    .strip_prefix(&root)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                sources.push((module.to_owned(), posix(relative), source));
                continue;
            }
            let parsed =
                syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
            let relative = path
                .strip_prefix(&root)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            emit(
                module,
                &parsed.items,
                &mut file.cfg,
                &mut |name, function| {
                    let attribute = function.attrs.iter().find(|a| a.path().is_ident("ignore"));
                    let reason = match attribute {
                        None => None,
                        Some(attribute) if matches!(attribute.meta, Meta::Path(_)) => None,
                        Some(attribute) => Some(literal(attribute).ok_or_else(|| {
                            format!("{}: {name}: ignore reason must be a string", path.display())
                        })?),
                    };
                    println!(
                        "{}",
                        serde_json::json!({
                            "package": package, "target": target, "path": posix(relative),
                            "line": function.sig.fn_token.span.start().line, "name": name,
                            "ignored": attribute.is_some(), "reason": reason,
                        })
                    );
                    Ok(())
                },
            )?;
        }
        // A target none of whose files spells a seed reaches nothing: its files are not parsed again.
        if let Some(mut analysis) = analysis.take().filter(|_| mentioned) {
            for (module, path, source) in &sources {
                analysis.add(module, path, source)?;
            }
            for record in analysis.finish(package, target) {
                println!("{record}");
            }
            let files: Vec<&str> = sources.iter().map(|(_, path, _)| path.as_str()).collect();
            println!(
                "{}",
                serde_json::json!({
                    "kind": "target_files", "package": package, "target": target, "files": files,
                })
            );
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_include_with_prefix_in_filename_is_not_treated_as_generated() {
        let prefix = sunrise_game_discovery::exports::PREFIX;
        let source = format!("include!(\"{prefix}custom.rs\");");
        let items = parse(Path::new("."), None, Path::new("lib.rs"), &source, true)
            .expect("static include should not require exports")
            .items;
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Item::Macro(_)));
    }

    #[test]
    fn out_dir_concat_include_with_prefix_expands_vocabulary_export() {
        let name = format!("{}sample.rs", sunrise_game_discovery::exports::PREFIX);
        let source = format!("include!(concat!(env!(\"OUT_DIR\"), \"/{name}\"));");
        let mut exports = BTreeMap::new();
        exports.insert(name, "fn exported_stub() {}".to_string());
        let items = parse(
            Path::new("."),
            Some(&exports),
            Path::new("lib.rs"),
            &source,
            true,
        )
        .expect("out_dir concat include should expand")
        .items;
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Item::Fn(_)));
    }

    #[test]
    fn unsupported_out_dir_includes_do_not_require_export_fragments() {
        for source in [
            r#"include!(concat!("/mod_api_exports_root.rs/", env!("OUT_DIR")));"#,
            r#"include!(concat!(env!("OUT_DIR"), env!("OUT_DIR"), "/mod_api_exports_root.rs"));"#,
            r#"include!(concat!(env!("OUT_DIR"), "mod_api_exports_root.rs"));"#,
        ] {
            let items = parse(Path::new("."), None, Path::new("lib.rs"), source, true)
                .expect("unsupported include should not require exports")
                .items;
            assert!(
                matches!(items.as_slice(), [Item::Macro(mac)] if mac.mac.path.is_ident("include"))
            );
        }
    }

    #[test]
    fn unknown_vocabulary_export_include_errors() {
        let name = format!("{}unknown.rs", sunrise_game_discovery::exports::PREFIX);
        let source = format!("include!(concat!(env!(\"OUT_DIR\"), \"/{name}\"));");
        let exports = BTreeMap::new();
        let Err(err) = parse(
            Path::new("."),
            Some(&exports),
            Path::new("lib.rs"),
            &source,
            true,
        ) else {
            panic!("unknown vocabulary export include should error");
        };
        assert!(
            err.contains("unknown vocabulary export include"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn out_dir_fragment_include_without_sources_errors() {
        let name = format!("{}root.rs", sunrise_game_discovery::exports::PREFIX);
        let source = format!("include!(concat!(env!(\"OUT_DIR\"), \"/{name}\"));");
        let Err(err) = parse(Path::new("."), None, Path::new("lib.rs"), &source, true) else {
            panic!("out_dir fragment include without sources should error");
        };
        assert!(
            err.contains("without its source fragments"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn impossible_file_and_include_gates_do_not_expand_includes() {
        let name = format!("{}missing.rs", sunrise_game_discovery::exports::PREFIX);
        for gate in [
            "#![cfg(any())]",
            "#[cfg(any())]",
            "#[cfg_attr(test, cfg(any()))]",
        ] {
            let source = format!("{gate}\ninclude!(concat!(env!(\"OUT_DIR\"), \"/{name}\"));");
            let parsed = parse(Path::new("."), None, Path::new("lib.rs"), &source, true).unwrap();
            assert!(parsed.items.is_empty(), "{gate}");
        }
    }

    #[test]
    fn emit_only_lists_tests_possible_in_a_test_build() {
        let parsed = syn::parse_file(
            r#"
            #[cfg(any())] mod inline { #[test] fn culled() {} }
            #[cfg(any())] #[test] fn never() {}
            #[cfg(false)] #[test] fn explicit_false() {}
            #[cfg(not(all()))] #[test] fn not_true() {}
            #[cfg_attr(all(), cfg(any()))] #[test] fn conditional_always() {}
            #[cfg_attr(test, cfg(any()))] #[test] fn conditional_test() {}
            #[cfg(unix)] mod contradiction {
                #[cfg(not(unix))] #[test] fn neither() {}
            }
            #[cfg(test)] mod tests {
                #[test] fn active() {}
                #[cfg(feature = "client")] #[test] fn client() {}
            }
        "#,
        )
        .unwrap();
        let mut names = Vec::new();
        emit("", &parsed.items, &mut Vec::new(), &mut |name, _| {
            names.push(name.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(names, ["tests::active", "tests::client"]);
    }
}
