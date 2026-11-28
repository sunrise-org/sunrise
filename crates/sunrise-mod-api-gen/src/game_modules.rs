//! The game's generated host forwarders, written into a source copy for tools that can't read OUT_DIR.
use mod_source::modules::{IncludeScope, gate_items, generated_include};
use std::{fs, path::Path};
use syn::spanned::Spanned;

/// `source` with the host forwarders' include replaced by the generated items, without needing OUT_DIR.
/// Other includes are left alone: source tools must not evaluate arbitrary build-time code.
/// Re-parsing the returned text gives the internals pass spans for editing the generated items.
pub(crate) fn expand(root: &Path, path: &Path, source: &str) -> Result<String, String> {
    // The filename itself may be escaped in a string literal.
    if !source.contains("include") {
        return Ok(source.to_owned());
    }
    let file = syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut scope = IncludeScope::default().with_imports(&file.items);
    let lines: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let mut replacements = Vec::new();
    for item in &file.items {
        if let Some(generated) = host_include(root, path, item, scope.before(item).shadowed())? {
            let span = item.span();
            let start = crate::source::offset(source, &lines, span.start());
            let end = crate::source::offset(source, &lines, span.end());
            replacements.push((start..end, prettyplease::unparse(&generated)));
        }
    }
    let mut expanded = source.to_owned();
    for (range, replacement) in replacements.into_iter().rev() {
        expanded.replace_range(range, &replacement);
    }
    Ok(expanded)
}

/// Write the generated host forwarders into a disposable game source copy before external tools
/// traverse it, without widening visibility or requiring an OUT_DIR. Never pass a downloaded cache.
pub fn materialize_generated_host(root: &Path) -> Result<(), String> {
    materialize_with_layout(root, crate::SourceLayout::read(root)?)
}

fn materialize_with_layout(root: &Path, layout: crate::source::SourceLayout) -> Result<(), String> {
    for file in layout.files(root, false)? {
        let source =
            fs::read_to_string(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
        let expanded = expand(root, &file.path, &source)?;
        if expanded != source {
            fs::write(&file.path, expanded).map_err(|e| format!("{}: {e}", file.path.display()))?;
        }
    }
    Ok(())
}

/// `source` parsed once, with the host forwarders' include replaced by the generated items in
/// the tree. Spans address `source`, so an edit that needs them uses [`expand`] instead.
pub(crate) fn parse(root: &Path, path: &Path, source: &str) -> Result<syn::File, String> {
    let mut file = syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut scope = IncludeScope::default().with_imports(&file.items);
    let mut items = Vec::with_capacity(file.items.len());
    for item in file.items {
        if let Some(generated) = host_include(root, path, &item, scope.before(&item).shadowed())? {
            items.extend(generated.items);
        } else {
            items.push(item);
        }
    }
    file.items = items;
    Ok(file)
}

/// Recognize and gate the host include identically for analysis and source editing.
fn host_include(
    root: &Path,
    path: &Path,
    item: &syn::Item,
    shadowed: bool,
) -> Result<Option<syn::File>, String> {
    let Some((name, attrs)) = generated_include(item, shadowed) else {
        return Ok(None);
    };
    if name != crate::build::HOST_OUTPUT {
        return Ok(None);
    }
    let mut file = generated_host(root, path)?;
    file.items = gate_items(file.items, attrs).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(file))
}

/// The generated forwarders of the game file at `path`, parsed.
fn generated_host(root: &Path, path: &Path) -> Result<syn::File, String> {
    let physical_root = root.canonicalize().ok();
    let package = crate::GameplayPackage::ALL.into_iter().find(|package| {
        let domain = package.domain();
        path.starts_with(domain.package_path(root))
            || physical_root
                .as_ref()
                .is_some_and(|root| path.starts_with(domain.package_path(root)))
    });
    let generated = match package {
        Some(package) => crate::gameplay::host_source(root, package)?,
        None => crate::build::game_host_generated(root)?,
    };
    syn::parse_file(&generated).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "include!(concat!(env!(\"OUT_DIR\"), \"/game_host_generated.rs\"));";

    /// Git for Windows checks sources out with CRLF by default.
    #[test]
    fn the_host_include_becomes_its_generated_items_whatever_ends_its_line() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for end in ["\n", "\r\n"] {
            let source = format!("pub(crate) mod draw;{end}{HOST}{end}");
            let file =
                syn::parse_file(&expand(&root, Path::new("game/host/mod.rs"), &source).unwrap())
                    .unwrap();
            let names: Vec<_> = file
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Mod(module) => Some(module.ident.to_string()),
                    _ => None,
                })
                .collect();
            assert_eq!(names, ["draw", "generated"], "{end:?}");
        }
        let other = "include!(concat!(env!(\"OUT_DIR\"), \"/mod_api_fire_generated.rs\"));\n";
        assert_eq!(
            expand(&root, Path::new("game/native_hooks/fire.rs"), other).unwrap(),
            other
        );
    }

    #[test]
    fn parse_and_expand_keep_shadowed_includes_and_gate_qualified_ones() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let host = Path::new("game/host/mod.rs");
        let source = format!(
            "macro_rules! include {{ ($($t:tt)*) => {{}}; }}\n{HOST}\n#[cfg(test)]\nstd::{HOST} // keep this comment\npub(crate) mod draw;\n"
        );
        let expanded = expand(&root, host, &source).unwrap();
        assert!(expanded.contains("// keep this comment"));
        for file in [
            parse(&root, host, &source).unwrap(),
            syn::parse_file(&expanded).unwrap(),
        ] {
            assert!(
                matches!(&file.items[1], syn::Item::Macro(it) if it.mac.path.is_ident("include"))
            );
            let generated = file
                .items
                .iter()
                .find_map(|item| match item {
                    syn::Item::Mod(module) if module.ident == "generated" => Some(module),
                    _ => None,
                })
                .expect("the qualified include expands");
            assert!(mod_source::modules::test_only(&generated.attrs));
        }
    }

    #[test]
    fn parse_and_expand_use_the_scope_at_each_include() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let host = Path::new("game/host/mod.rs");
        let source = format!("{HOST}\nmacro_rules! include {{ ($($t:tt)*) => {{}}; }}\n{HOST}\n");
        let expanded = expand(&root, host, &source).unwrap();
        for file in [
            parse(&root, host, &source).unwrap(),
            syn::parse_file(&expanded).unwrap(),
        ] {
            assert!(matches!(&file.items[0], syn::Item::Mod(it) if it.ident == "generated"));
            assert!(
                matches!(&file.items[2], syn::Item::Macro(it) if it.mac.path.is_ident("include"))
            );
        }
    }

    #[test]
    fn parse_and_expand_refuse_cfg_attr_at_the_host_include() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let host = Path::new("game/host/mod.rs");
        let source = format!("#[cfg_attr(unix, cfg(test), allow(dead_code))]\n{HOST}\n");
        let parse_error = parse(&root, host, &source)
            .err()
            .expect("cfg_attr is unsupported");
        let expand_error = expand(&root, host, &source).unwrap_err();
        assert!(parse_error.contains("cfg_attr"), "{parse_error}");
        assert_eq!(parse_error, expand_error);
    }
}
