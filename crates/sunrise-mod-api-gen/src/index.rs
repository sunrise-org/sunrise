//! The crates the mod API's signatures name, indexed by [`mod_source::index`].
pub(crate) use mod_source::index::{Found, Index, ItemId, uses};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use syn::Item;

/// The crates whose items mod API signatures name: crate name and source directory.
pub(crate) const CRATES: &[(&str, &str)] = &[
    ("mod_api", "crates/sunrise-mod-vocab/src"),
    ("sunrise_core", "crates/sunrise-core/src"),
    ("sunrise_mod_runtime", "crates/sunrise-mod-runtime/src"),
];

/// The mod API's type index, read below `root`.
pub(crate) fn game<'a>(root: &'a Path, included: &'a [(&'a str, &'a str)]) -> Index<'a> {
    Index::new(root, included, CRATES).expanding(crate::exports::expand)
}

/// The workspace libraries that the public API cannot treat as opaque third-party types.
pub(crate) struct OutsideSeam {
    crates: HashSet<String>,
    /// Dependency names as Rust spells them, keyed by the indexed crate that declares them.
    dependencies: HashMap<String, HashMap<String, String>>,
}

impl OutsideSeam {
    pub(crate) fn read(root: &Path) -> Result<Self, String> {
        let workspace_manifest = root.join("Cargo.toml");
        crate::rerun_if_changed(&workspace_manifest.display());
        let manifest = crate::replace::manifest(root)?;
        let workspace_members = manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("members"))
            .and_then(toml::Value::as_array)
            .ok_or("Cargo.toml: no workspace.members array")?;
        let workspace_dependencies = manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("dependencies"))
            .and_then(toml::Value::as_table);
        let mut crates = HashSet::new();
        let mut packages = HashMap::new();
        let mut member_manifests = Vec::new();
        for member in workspace_members {
            let member = member
                .as_str()
                .ok_or("Cargo.toml: workspace member is not a path")?;
            let package_path = root.join(member);
            crate::rerun_if_changed(&package_path.join("Cargo.toml").display());
            let package = crate::replace::manifest(&package_path)?;
            let package_name = package
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("{member}/Cargo.toml: no package name"))?;
            let crate_name = package
                .get("lib")
                .and_then(|lib| lib.get("name"))
                .or_else(|| package.get("package").and_then(|p| p.get("name")))
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("{member}/Cargo.toml: no crate name"))?;
            let crate_name = crate_name.replace('-', "_");
            packages.insert(package_name.to_owned(), crate_name.clone());

            // The facade and seam crates are indexed under names their source writes, which
            // differ from some Cargo library names (`mod_api` vs `mod_vocab`).
            let indexed_name = CRATES
                .iter()
                .find(|(_, src)| Path::new(src).parent() == Some(Path::new(member)))
                .map(|(name, _)| (*name).to_owned())
                .or_else(|| {
                    (member == "crates/sunrise-mod-api").then(|| "sunrise_mod_api".to_owned())
                })
                .unwrap_or_else(|| crate_name.clone());
            let seam = member == "crates/sunrise-mod-api"
                || CRATES
                    .iter()
                    .any(|(_, src)| Path::new(src).parent() == Some(Path::new(member)));
            if !seam {
                crates.insert(crate_name);
            }
            member_manifests.push((indexed_name, package));
        }

        let mut dependencies = HashMap::new();
        for (indexed_name, package) in member_manifests {
            let mut aliases = HashMap::new();
            let mut add = |table: Option<&toml::map::Map<String, toml::Value>>| {
                let Some(table) = table else { return };
                for (alias, spec) in table {
                    let inherited = spec
                        .as_table()
                        .and_then(|table| table.get("workspace"))
                        .and_then(toml::Value::as_bool)
                        == Some(true);
                    let spec = if inherited {
                        workspace_dependencies
                            .and_then(|workspace| workspace.get(alias))
                            .unwrap_or(spec)
                    } else {
                        spec
                    };
                    let package = spec
                        .as_table()
                        .and_then(|table| table.get("package"))
                        .and_then(toml::Value::as_str)
                        .unwrap_or(alias);
                    if let Some(crate_name) = packages.get(package)
                        && crates.contains(crate_name)
                    {
                        aliases.insert(alias.replace('-', "_"), crate_name.clone());
                    }
                }
            };
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                add(package.get(section).and_then(toml::Value::as_table));
            }
            if let Some(targets) = package.get("target").and_then(toml::Value::as_table) {
                for target in targets.values() {
                    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                        add(target.get(section).and_then(toml::Value::as_table));
                    }
                }
            }
            if !aliases.is_empty() {
                dependencies.insert(indexed_name, aliases);
            }
        }
        Ok(Self {
            crates,
            dependencies,
        })
    }

    /// Resolves an indexed source path to an outside workspace crate, following Cargo and `use`
    /// aliases. The returned path uses the owning crate's real Rust library name.
    pub(crate) fn external_path(
        &self,
        index: &Index<'_>,
        module: usize,
        path: &[String],
    ) -> Option<Vec<String>> {
        self.resolve_external(index, module, path, &mut HashSet::new())
    }

    fn resolve_external(
        &self,
        index: &Index<'_>,
        module: usize,
        path: &[String],
        seen: &mut HashSet<(usize, String)>,
    ) -> Option<Vec<String>> {
        let (first, rest) = path.split_first()?;
        if self.crates.contains(first) {
            return Some(path.to_vec());
        }

        let krate = index.modules.get(module)?.krate;
        if let Some(real) = self
            .dependencies
            .get(krate)
            .and_then(|aliases| aliases.get(first))
        {
            return Some(
                std::iter::once(real.clone())
                    .chain(rest.iter().cloned())
                    .collect(),
            );
        }

        // Resolve `crate::`, `self::` and `super::` before looking for imports in that module.
        let (scope, path) = match first.as_str() {
            "crate" | "self" | "super" => {
                let mut scope_path = index.modules[module].path.clone();
                let mut consumed = 1;
                match first.as_str() {
                    "crate" => scope_path.clear(),
                    "self" => {}
                    "super" => {
                        scope_path.pop()?;
                        while path.get(consumed).is_some_and(|segment| segment == "super") {
                            scope_path.pop()?;
                            consumed += 1;
                        }
                    }
                    _ => unreachable!(),
                }
                let scope = index.modules.iter().position(|candidate| {
                    candidate.krate == krate && candidate.path == scope_path
                })?;
                (scope, &path[consumed..])
            }
            _ => (module, path),
        };
        let (first, rest) = path.split_first()?;
        if self.crates.contains(first) {
            return Some(path.to_vec());
        }
        let krate = index.modules.get(scope)?.krate;
        if let Some(real) = self
            .dependencies
            .get(krate)
            .and_then(|aliases| aliases.get(first))
        {
            return Some(
                std::iter::once(real.clone())
                    .chain(rest.iter().cloned())
                    .collect(),
            );
        }

        if !seen.insert((scope, first.clone())) {
            return None;
        }
        let items = &index.modules[scope].items;
        let mut named = None;
        let mut globs = Vec::new();
        for item in items {
            match item {
                Item::Use(item) => uses(&item.tree, &mut Vec::new(), &mut |imported, alias| {
                    if alias.as_deref() == Some(first.as_str()) {
                        named = Some(imported);
                    } else if alias.is_none() {
                        globs.push(imported);
                    }
                }),
                Item::ExternCrate(item) => {
                    let alias = item
                        .rename
                        .as_ref()
                        .map_or_else(|| item.ident.to_string(), |(_, name)| name.to_string());
                    if alias == *first {
                        named = Some(vec![item.ident.to_string()]);
                    }
                }
                _ => {}
            }
        }
        if let Some(imported) = named {
            if let Some(mut resolved) = self.resolve_external(index, scope, &imported, seen) {
                resolved.extend(rest.iter().cloned());
                return Some(resolved);
            }
            return None;
        }
        for glob in globs {
            if let Some(mut resolved) = self.resolve_external(index, scope, &glob, seen) {
                resolved.push(first.clone());
                resolved.extend(rest.iter().cloned());
                return Some(resolved);
            }
        }
        None
    }

    pub(crate) fn error(item: &str, path: &[String]) -> String {
        format!(
            "{item}: `{}` names a workspace crate outside the mod API type index",
            path.join("::")
        )
    }
}
