//! Derive the parity dispatcher table from the libtest wrappers that drive it.

use mod_source::modules::{cfg_conditions, evaluate_cfg, may_compile, walk_module_variants};
use proc_macro2::{Group, TokenStream, TokenTree};
use quote::quote;
use std::{collections::BTreeMap, fmt::Write as _, fs, path::Path};
use syn::{Expr, Item, Stmt, UseTree};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Fixture {
    name: String,
    runner: String,
    cfg: Vec<String>,
}

/// Discover from the test build, but keep the other build gates on the emitted entries.
fn libtest_gates(conditions: &[String]) -> Vec<String> {
    fn replace_test(tokens: TokenStream) -> TokenStream {
        tokens
            .into_iter()
            .flat_map(|token| match token {
                TokenTree::Ident(ref ident) if ident == "test" => quote!(all()),
                TokenTree::Group(group) => TokenStream::from(TokenTree::Group(Group::new(
                    group.delimiter(),
                    replace_test(group.stream()),
                ))),
                _ => TokenStream::from(token),
            })
            .collect()
    }
    conditions
        .iter()
        .filter(|gate| evaluate_cfg(gate, |atom| (atom == "test").then_some(true)) != Some(true))
        .map(|gate| replace_test(gate.parse().expect("normalized cfg tokens")).to_string())
        .collect()
}

fn discover(manifest: &Path) -> Result<Vec<Fixture>, String> {
    let src = manifest.join("src");
    let variants = walk_module_variants(
        "root".into(),
        src.join("lib.rs"),
        src,
        false,
        true,
        |path, source| syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display())),
    )?;
    let mut runners = BTreeMap::<String, Vec<Vec<String>>>::new();
    let mut wrappers = Vec::new();
    for variant in variants {
        let file = variant.file;
        println!("cargo::rerun-if-changed={}", file.path.display());
        let module = file.module.strip_prefix("root::").unwrap_or("");
        let path = |name: &str| {
            if module.is_empty() {
                name.to_owned()
            } else {
                format!("{module}::{name}")
            }
        };
        let gates = |attrs: &[syn::Attribute]| {
            let mut cfg = file.cfg.clone();
            cfg.extend(cfg_conditions(attrs));
            libtest_gates(&cfg)
        };
        for item in variant.items {
            match item {
                Item::Fn(function) if function.sig.inputs.is_empty() => {
                    let name = function.sig.ident.to_string();
                    if crate::references::is_test(&function.attrs) {
                        let runner = format!("run_{name}");
                        if let [Stmt::Expr(Expr::Call(call), _)] = function.block.stmts.as_slice()
                            && call.args.is_empty()
                            && let Expr::Path(callee) = &*call.func
                            && callee.qself.is_none()
                            && callee.path.is_ident(runner.as_str())
                        {
                            wrappers.push(Fixture {
                                name: path(&name),
                                runner: path(&runner),
                                cfg: gates(&function.attrs),
                            });
                        }
                    } else if name.starts_with("run_") {
                        runners
                            .entry(path(&name))
                            .or_default()
                            .push(gates(&function.attrs));
                    }
                }
                Item::Use(used) => {
                    visit_import_bindings(&used.tree, &mut |name| {
                        let name = name.to_string();
                        if name.starts_with("run_") {
                            runners
                                .entry(path(&name))
                                .or_default()
                                .push(gates(&used.attrs));
                        }
                    });
                }
                _ => {}
            }
        }
    }
    let mut fixtures = Vec::new();
    for mut wrapper in wrappers {
        if !may_compile(&wrapper.cfg, false) {
            continue;
        }
        if let Some(bindings) = runners.get(&wrapper.runner) {
            let alternatives: Vec<_> = bindings
                .iter()
                .filter_map(|binding| {
                    let extra: Vec<_> = binding
                        .iter()
                        .filter(|gate| !wrapper.cfg.contains(gate))
                        .collect();
                    let mut cfg = wrapper.cfg.clone();
                    cfg.extend(extra.iter().map(|gate| (*gate).clone()));
                    may_compile(&cfg, false).then(|| {
                        extra
                            .into_iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                })
                .collect();
            if !alternatives.is_empty() && !alternatives.iter().any(String::is_empty) {
                let mut alternatives = alternatives
                    .into_iter()
                    .map(|cfg| format!("all({cfg})"))
                    .collect::<Vec<_>>();
                alternatives.sort_unstable();
                alternatives.dedup();
                wrapper
                    .cfg
                    .push(format!("any({})", alternatives.join(", ")));
            }
        }
        // Without a named binding (e.g. a glob), use the wrapper's cfg alone. Rust resolves
        // the name and reports a broken runner rather than losing its fixture from the table.
        wrapper.cfg.sort_unstable();
        wrapper.cfg.dedup();
        fixtures.push(wrapper);
    }
    fixtures.sort_unstable();
    fixtures.dedup();
    Ok(fixtures)
}

fn visit_import_bindings(tree: &UseTree, visit: &mut impl FnMut(&syn::Ident)) {
    match tree {
        UseTree::Path(path) => visit_import_bindings(&path.tree, visit),
        UseTree::Name(name) => visit(&name.ident),
        UseTree::Rename(rename) => visit(&rename.rename),
        UseTree::Group(group) => {
            for tree in &group.items {
                visit_import_bindings(tree, visit);
            }
        }
        UseTree::Glob(_) => {}
    }
}

/// Write a crate's table with full libtest names, independently of the dispatcher's feature set.
pub fn write_crate_table(manifest: &Path, out: &Path) -> Result<(), String> {
    let mut table = String::from("pub(crate) const PARITY_FIXTURES: &[(&str, fn())] = &[\n");
    for Fixture { name, runner, cfg } in discover(manifest)? {
        if !cfg.is_empty() {
            writeln!(table, "    #[cfg(all({}))]", cfg.join(", ")).unwrap();
        }
        writeln!(table, "    ({name:?}, crate::{runner} as fn()),").unwrap();
    }
    table.push_str("];\n");
    let path = out.join("parity_fixtures.rs");
    fs::write(&path, table).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_full_libtest_paths_and_gates_across_includes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(
            src.join("lib.rs"),
            r#"
mod cut_layout {
    #[cfg(any(test, feature = "parity-fixtures"))]
    mod game {
        #[cfg(feature = "client")]
        mod tests {
            include!("runner.rs");
            #[test]
            fn split() { run_split(); }
            #[test]
            fn inline() { run_inline() }
            fn run_inline() {}
            #[test]
            fn missing() { run_missing(); }
        }
        #[cfg(not(test))]
        mod impossible {
            #[test]
            fn never() { run_never(); }
        }
    }
}
"#,
        )
        .unwrap();
        fs::write(src.join("runner.rs"), "fn run_split() {}\n").unwrap();
        assert_eq!(
            discover(dir.path()).unwrap(),
            ["inline", "missing", "split"].map(|name| Fixture {
                name: format!("cut_layout::game::tests::{name}"),
                runner: format!("cut_layout::game::tests::run_{name}"),
                cfg: vec!["feature = \"client\"".into()],
            })
        );
    }

    #[test]
    fn discovers_grouped_reexports_renames_and_globs() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(
            src.join("lib.rs"),
            r#"
mod game {
    mod side {
        pub(crate) fn run_split() {}
        pub(crate) fn run_original() {}
    }
    mod glob_side {
        pub(crate) fn run_glob() {}
    }
    mod tests {
        #[cfg(all(test, feature = "client"))]
        pub(in crate::game) use super::side::{run_split, run_original as run_renamed};
        pub(crate) use super::glob_side::*;

        #[test]
        fn split() { run_split(); }
        #[test]
        fn renamed() { run_renamed(); }
        #[test]
        fn glob() { run_glob(); }
    }
}
"#,
        )
        .unwrap();
        let fixtures = discover(dir.path()).unwrap();
        assert_eq!(
            fixtures
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            [
                "game::tests::glob",
                "game::tests::renamed",
                "game::tests::split"
            ]
        );
        assert!(fixtures[0].cfg.is_empty());
        for fixture in &fixtures[1..] {
            let [gate] = fixture.cfg.as_slice() else {
                panic!("expected client gate")
            };
            assert_eq!(
                evaluate_cfg(gate, |atom| (atom == "feature = \"client\"")
                    .then_some(true)),
                Some(true)
            );
            assert_eq!(
                evaluate_cfg(gate, |atom| (atom == "feature = \"client\"")
                    .then_some(false)),
                Some(false)
            );
        }
    }
}
