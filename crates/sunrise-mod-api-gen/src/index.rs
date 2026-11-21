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
    indexed: Vec<WorkspaceCrate>,
    /// Dependency names as Rust spells them, keyed by the indexed crate that declares them.
    dependencies: HashMap<String, HashMap<String, String>>,
}

struct WorkspaceCrate {
    name: String,
    source: String,
    root_file: Option<String>,
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
        let mut indexed = Vec::new();
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
                || member == "crates/sunrise-mod-api-macros"
                || CRATES
                    .iter()
                    .any(|(_, src)| Path::new(src).parent() == Some(Path::new(member)));
            if !seam {
                crates.insert(crate_name.clone());
                let lib_path = package
                    .get("lib")
                    .and_then(|lib| lib.get("path"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or("src/lib.rs");
                let source_file = package_path.join(lib_path);
                if source_file.is_file() {
                    let source = source_file
                        .parent()
                        .and_then(|path| path.strip_prefix(root).ok())
                        .ok_or_else(|| format!("{member}/Cargo.toml: invalid library path"))?
                        .to_string_lossy()
                        .into_owned();
                    let root_file = source_file
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                    indexed.push(WorkspaceCrate {
                        name: crate_name,
                        source,
                        root_file,
                    });
                }
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
            indexed,
            dependencies,
        })
    }

    /// Adds outside workspace crates to the lazy source index so glob imports can be resolved to
    /// actual exported names without treating primitives or prelude types as leaks.
    pub(crate) fn add_to_index(&self, index: &mut Index<'_>) {
        for krate in &self.indexed {
            index.add_crate(&krate.name, &krate.source);
            if let Some(file) = &krate.root_file
                && file != "lib.rs"
            {
                index.set_root_file(&krate.name, file);
            }
        }
    }

    /// Resolves an indexed source path to an outside workspace crate, following Cargo and `use`
    /// aliases. The returned path uses the owning crate's real Rust library name.
    pub(crate) fn external_path(
        &self,
        index: &mut Index<'_>,
        module: usize,
        path: &[String],
    ) -> Option<Vec<String>> {
        self.resolve_external(index, module, path, &mut HashSet::new())
    }

    fn indexed_external_path(
        &self,
        index: &Index<'_>,
        id: crate::index::ItemId,
    ) -> Option<Vec<String>> {
        let module = &index.modules[id.module];
        if !self.crates.contains(&module.krate) {
            return None;
        }
        let name = match index.item(id) {
            Item::Const(item) => item.ident.to_string(),
            Item::Enum(item) => item.ident.to_string(),
            Item::Struct(item) => item.ident.to_string(),
            Item::Trait(item) => item.ident.to_string(),
            Item::Type(item) => item.ident.to_string(),
            Item::Union(item) => item.ident.to_string(),
            _ => return None,
        };
        Some(
            std::iter::once(module.krate.to_owned())
                .chain(module.path.iter().cloned())
                .chain(std::iter::once(name))
                .collect(),
        )
    }

    fn globbed_external_path(
        &self,
        index: &mut Index<'_>,
        module: usize,
        name: &str,
        seen: &mut HashSet<(usize, String)>,
    ) -> Option<Vec<String>> {
        if let Ok(crate::index::Found::Item(id)) = index.resolve(module, name) {
            return self.indexed_external_path(index, id);
        }
        let mut globs = Vec::new();
        for item in &index.modules[module].items {
            if let Item::Use(item) = item {
                uses(&item.tree, &mut Vec::new(), &mut |path, alias| {
                    if alias.is_none() {
                        globs.push(path);
                    }
                });
            }
        }
        for glob in globs {
            let target = if let Some(path) = self.resolve_external(index, module, &glob, seen) {
                let mut path = path;
                path.push(name.to_owned());
                index.resolve_path(module, &path).ok()
            } else {
                Self::indexed_module(index, module, &glob)
                    .and_then(|target| index.resolve(target, name).ok())
            };
            if let Some(crate::index::Found::Item(id)) = target
                && let Some(path) = self.indexed_external_path(index, id)
            {
                return Some(path);
            }
        }
        None
    }

    /// Loads an indexed module named by a local path, including `crate::` and `super::` paths.
    fn indexed_module(index: &mut Index<'_>, module: usize, path: &[String]) -> Option<usize> {
        Self::resolve_indexed_module(index, module, path, &mut HashSet::new())
    }

    fn resolve_indexed_module(
        index: &mut Index<'_>,
        module: usize,
        path: &[String],
        seen: &mut HashSet<(usize, String)>,
    ) -> Option<usize> {
        if let Some((krate, path)) = index.absolute(module, path)
            && let Ok(target) = index.module(krate, &path)
        {
            return Some(target);
        }

        let (first, rest) = path.split_first()?;
        if !seen.insert((module, first.clone())) {
            return None;
        }
        let mut imported = None;
        for item in &index.modules[module].items {
            match item {
                Item::Use(item) => uses(&item.tree, &mut Vec::new(), &mut |path, alias| {
                    if alias.as_deref() == Some(first.as_str()) {
                        imported = Some(path);
                    }
                }),
                Item::ExternCrate(item) => {
                    let alias = item
                        .rename
                        .as_ref()
                        .map_or_else(|| item.ident.to_string(), |(_, name)| name.to_string());
                    if alias == *first {
                        imported = Some(vec![item.ident.to_string()]);
                    }
                }
                _ => {}
            }
        }
        let target = Self::resolve_indexed_module(index, module, &imported?, seen)?;
        if rest.is_empty() {
            Some(target)
        } else {
            Self::resolve_indexed_module(index, target, rest, seen)
        }
    }

    fn resolve_external(
        &self,
        index: &mut Index<'_>,
        module: usize,
        path: &[String],
        seen: &mut HashSet<(usize, String)>,
    ) -> Option<Vec<String>> {
        // Resolve `crate::`, `self::` and `super::` before looking for imports in that module.
        let (first, _) = path.split_first()?;
        let krate = index.modules.get(module)?.krate.clone();
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

        // A local module shadows a crate with the same spelling. Follow it before checking the
        // external workspace crate and import aliases.
        if !rest.is_empty()
            && let Some(target) = Self::indexed_module(index, scope, std::slice::from_ref(first))
        {
            let target_module = &index.modules[target];
            if self.crates.contains(&target_module.krate) {
                return Some(
                    std::iter::once(target_module.krate.to_owned())
                        .chain(target_module.path.iter().cloned())
                        .chain(rest.iter().cloned())
                        .collect(),
                );
            }
            return self.resolve_external(index, target, rest, seen);
        }
        if self.crates.contains(first) {
            return Some(path.to_vec());
        }
        let krate = index.modules.get(scope)?.krate.clone();
        if let Some(real) = self
            .dependencies
            .get(&krate)
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
        let mut named = None;
        for item in &index.modules[scope].items {
            match item {
                Item::Use(item) => uses(&item.tree, &mut Vec::new(), &mut |imported, alias| {
                    if alias.as_deref() == Some(first.as_str()) {
                        named = Some(imported);
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
            if !rest.is_empty()
                && let Some(target) = Self::indexed_module(index, scope, &imported)
            {
                return self.resolve_external(index, target, rest, seen);
            }
            return None;
        }
        self.globbed_external_path(index, scope, first, seen)
    }

    pub(crate) fn error(item: &str, path: &[String]) -> String {
        format!(
            "{item}: `{}` names a workspace crate outside the mod API type index",
            path.join("::")
        )
    }
}
