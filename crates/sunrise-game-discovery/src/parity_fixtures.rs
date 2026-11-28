//! Discover the parity dispatcher entries from the libtest wrappers that drive them.

use mod_source::modules::{cfg_conditions, evaluate_cfg, may_compile, walk_module_files};
use std::{collections::BTreeMap, fmt::Write as _, fs, path::Path};
use syn::{Expr, Item, Stmt, UseTree};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscoveredFixture {
    pub public_name: String,
    pub runner: String,
    pub cfg: Vec<String>,
}

fn may_compile_with_test(conditions: &[String]) -> bool {
    if conditions.is_empty() {
        return true;
    }
    let mut gated = conditions.to_vec();
    gated.push("test".to_owned());
    may_compile(&gated, true)
}

fn non_test_cfgs(conditions: &[String]) -> Vec<String> {
    let mut cfg = Vec::new();
    for condition in conditions {
        if evaluate_cfg(condition, |atom| (atom == "test").then_some(true)) != Some(true)
            && !cfg.contains(condition)
        {
            cfg.push(condition.clone());
        }
    }
    cfg
}

/// A fixture is a `#[test]` whose only statement calls its module's `run_<test>()`.
/// The runner can be defined in the module or imported from another module.
pub fn discover(manifest: &Path) -> Result<Vec<DiscoveredFixture>, String> {
    let parse = |path: &Path, source: &str| {
        syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display()))
    };
    let files = walk_module_files(
        "root".into(),
        manifest.join("src/lib.rs"),
        manifest.join("src"),
        false,
        true,
        parse,
    )?;
    let mut runners = BTreeMap::new();
    let mut wrappers = Vec::new();
    for file in files {
        println!("cargo::rerun-if-changed={}", file.path.display());
        if !may_compile_with_test(&file.cfg) {
            continue;
        }
        let module = file.module.strip_prefix("root::").unwrap_or("");
        let source =
            fs::read_to_string(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
        collect(
            module,
            &parse(&file.path, &source)?.items,
            &file.cfg,
            &mut runners,
            &mut wrappers,
        );
    }

    let mut fixtures = Vec::new();
    for (public_name, runner, wrapper_cfg) in wrappers {
        if !public_name.starts_with("game::") && !public_name.starts_with("cut_layout::game::") {
            continue;
        }
        // A glob may supply the runner. In that case keep the wrapper's gates
        // and let Rust resolve the binding when it compiles the table.
        let empty_cfg = Vec::new();
        let runner_cfgs = runners
            .get(&runner)
            .map_or(std::slice::from_ref(&empty_cfg), Vec::as_slice);
        for runner_cfg in runner_cfgs {
            let mut conditions = wrapper_cfg.clone();
            conditions.extend_from_slice(runner_cfg);
            if may_compile_with_test(&conditions) {
                fixtures.push(DiscoveredFixture {
                    public_name: public_name.clone(),
                    runner: runner.clone(),
                    cfg: non_test_cfgs(&conditions),
                });
            }
        }
    }
    fixtures.sort_unstable();
    fixtures.dedup();
    Ok(fixtures)
}

fn collect(
    module: &str,
    items: &[Item],
    cfg: &[String],
    runners: &mut BTreeMap<String, Vec<Vec<String>>>,
    wrappers: &mut Vec<(String, String, Vec<String>)>,
) {
    let path = |name| {
        if module.is_empty() {
            name
        } else {
            format!("{module}::{name}")
        }
    };
    for item in items {
        match item {
            Item::Mod(child) if let Some((_, items)) = &child.content => {
                let mut conditions = cfg.to_vec();
                conditions.extend(cfg_conditions(&child.attrs));
                if may_compile_with_test(&conditions) {
                    collect(
                        &path(child.ident.to_string()),
                        items,
                        &conditions,
                        runners,
                        wrappers,
                    );
                }
            }
            Item::Fn(function) if function.sig.inputs.is_empty() => {
                let name = function.sig.ident.to_string();
                let mut conditions = cfg.to_vec();
                conditions.extend(cfg_conditions(&function.attrs));
                if crate::references::is_test(&function.attrs) {
                    let runner = format!("run_{name}");
                    if let [Stmt::Expr(Expr::Call(call), _)] = function.block.stmts.as_slice()
                        && call.args.is_empty()
                        && let Expr::Path(callee) = &*call.func
                        && callee.qself.is_none()
                        && callee.path.is_ident(runner.as_str())
                    {
                        wrappers.push((path(name), path(runner), conditions));
                    }
                } else if name.starts_with("run_") {
                    runners.entry(path(name)).or_default().push(conditions);
                }
            }
            Item::Use(import) => {
                let mut conditions = cfg.to_vec();
                conditions.extend(cfg_conditions(&import.attrs));
                visit_import_bindings(&import.tree, &mut |name| {
                    let name = name.to_string();
                    if name.starts_with("run_") {
                        runners
                            .entry(path(name))
                            .or_default()
                            .push(conditions.clone());
                    }
                });
            }
            _ => {}
        }
    }
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

fn write_rows(output: &mut String, fixtures: Vec<DiscoveredFixture>) {
    for DiscoveredFixture {
        public_name,
        runner,
        cfg,
    } in fixtures
    {
        match cfg.as_slice() {
            [] => {}
            [only] => writeln!(output, "    #[cfg({only})]").unwrap(),
            _ => writeln!(output, "    #[cfg(all({}))]", cfg.join(", ")).unwrap(),
        }
        writeln!(output, "    ({public_name:?}, crate::{runner} as fn()),").unwrap();
    }
}

fn write_table(out_dir: &Path, table: String) -> Result<(), String> {
    let path = out_dir.join("parity_fixtures.rs");
    fs::write(&path, table).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_crate_table(manifest: &Path, out_dir: &Path) -> Result<(), String> {
    let mut table = String::from("const PARITY_FIXTURES: &[(&str, fn())] = &[\n");
    write_rows(&mut table, discover(manifest)?);
    table.push_str("];\n");
    write_table(out_dir, table)
}

pub fn write_bin_table(client_manifest: &Path, out_dir: &Path) -> Result<(), String> {
    let mut table = String::from(
        r#"
pub(crate) static FIXTURES: std::sync::LazyLock<Vec<(&'static str, fn())>> = std::sync::LazyLock::new(|| {
    let mut rows: Vec<(&'static str, fn())> = Vec::new();
    rows.extend_from_slice(sunrise_gameplay_common::parity_fixtures());
    rows.extend_from_slice(sunrise_gameplay::parity_fixtures());
    #[cfg(feature = "client-defaults")]
    rows.extend_from_slice(sunrise_client_gameplay::parity_fixtures());
    rows.extend_from_slice(&[
"#,
    );
    write_rows(&mut table, discover(client_manifest)?);
    table.push_str(
        r#"
    ]);
    rows.sort_unstable_by_key(|row| row.0);
    for pair in rows.windows(2) {
        assert_ne!(pair[0].0, pair[1].0, "duplicate parity fixture");
    }
    rows
});
"#,
    );
    write_table(out_dir, table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_sibling_runners_across_includes_and_preserves_build_gates() {
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
            fn disabled() { run_disabled(); }
            #[cfg(any())]
            fn run_disabled() {}
        }
        #[cfg(not(test))]
        mod impossible {
            #[test]
            fn never() { run_never(); }
            fn run_never() {}
        }
    }
    mod outside {
        #[test]
        fn other() { run_other(); }
        fn run_other() {}
    }
}
"#,
        )
        .unwrap();
        fs::write(src.join("runner.rs"), "fn run_split() {}\n").unwrap();

        assert_eq!(
            discover(dir.path()).unwrap(),
            ["inline", "split"].map(|name| DiscoveredFixture {
                public_name: format!("cut_layout::game::tests::{name}"),
                runner: format!("cut_layout::game::tests::run_{name}"),
                cfg: vec!["feature = \"client\"".into()],
            }),
        );
    }

    #[test]
    fn discovers_split_runner_imports_and_glob_bound_runners() {
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
        #[cfg(feature = "client")]
        pub(crate) use super::side::{run_split, run_original as run_renamed};
        pub(crate) use super::glob_side::*;

        #[cfg(all(test, feature = "client"))]
        #[test]
        fn split() { run_split(); }
        #[cfg(all(test, feature = "client"))]
        #[test]
        fn renamed() { run_renamed(); }
        #[test]
        fn glob() { run_glob(); }
    }
}
"#,
        )
        .unwrap();

        assert_eq!(
            discover(dir.path()).unwrap(),
            ["glob", "renamed", "split"].map(|name| DiscoveredFixture {
                public_name: format!("game::tests::{name}"),
                runner: format!("game::tests::run_{name}"),
                cfg: if name == "glob" {
                    vec![]
                } else {
                    vec![
                        "all (test , feature = \"client\")".into(),
                        "feature = \"client\"".into(),
                    ]
                },
            }),
        );
    }
}
