//! Provider roles and built-in capability metadata derived from the public keys.
use crate::index::{Found, Index};
use quote::ToTokens;
use std::path::Path;
use syn::Item;

/// A capability key's shape: what its providers are registered by, its handler trait as
/// written (`ResourceHandler<ItemResource>`), and an entity key's lookup context.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct KeyShape {
    pub(crate) by: &'static str,
    pub(crate) handler: String,
    pub(crate) context: String,
}

fn is_const_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase()) && !name.chars().any(|c| c.is_ascii_lowercase())
}

fn const_key_shape(item: &syn::ItemConst) -> Result<Option<KeyShape>, String> {
    let syn::Type::Path(path) = &*item.ty else {
        return Ok(None);
    };
    let Some(last) = path.path.segments.last() else {
        return Ok(None);
    };
    let by = match last.ident.to_string().as_str() {
        "CapabilityKey" => "Block",
        "ItemCapabilityKey" => "Item",
        "EntityCapabilityKey" => "Entity",
        _ => return Ok(None),
    };
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return Ok(None);
    };
    let mut args = args.args.iter();
    let Some(syn::GenericArgument::Type(syn::Type::TraitObject(object))) = args.next() else {
        return Ok(None);
    };
    let context = match args.next() {
        Some(syn::GenericArgument::Type(ty)) => written(ty),
        _ => "()".to_string(),
    };
    Ok(Some(KeyShape {
        by,
        handler: trait_name(object)?,
        context,
    }))
}

/// Every exported capability declaration, filtering root `pub use` candidates to constant names
/// before resolving them through [`Index`] so intermediate globs, renames and cfgs stay exact.
fn key_declarations(index: &mut Index<'_>) -> Result<Vec<(String, KeyShape)>, String> {
    let api = index.module("mod_api", &[])?;
    let names: Vec<String> = index.modules[api]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Use(item)
                if matches!(item.vis, syn::Visibility::Public(_))
                    && index.default_build(api, &item.attrs) =>
            {
                Some(&item.tree)
            }
            _ => None,
        })
        .flat_map(use_names)
        .filter(|name| is_const_name(name))
        .collect();
    let mut declarations = Vec::new();
    for name in names {
        let Ok(Found::Item(id)) = index.resolve(api, &name) else {
            continue;
        };
        let Item::Const(item) = index.item(id) else {
            continue;
        };
        if let Some(shape) = const_key_shape(item)? {
            declarations.push((name, shape));
        }
    }
    Ok(declarations)
}

/// Distinct provider shapes, in the order their keys are exported.
pub(crate) fn key_shapes(index: &mut Index<'_>) -> Result<Vec<KeyShape>, String> {
    let mut shapes = Vec::new();
    for (_, shape) in key_declarations(index)? {
        if !shapes.contains(&shape) {
            shapes.push(shape);
        }
    }
    Ok(shapes)
}

impl KeyShape {
    /// Its provider trait, as an owner (`BlockCapabilityProvider<dyn ResourceHandler<ItemResource>>`).
    pub(crate) fn provider(&self) -> String {
        match self.by {
            "Entity" => format!(
                "EntityCapabilityProvider<dyn {}, {}>",
                self.handler, self.context
            ),
            by => format!("{by}CapabilityProvider<dyn {}>", self.handler),
        }
    }
}

/// Every name a `use` tree brings in.
fn use_names(tree: &syn::UseTree) -> Vec<String> {
    match tree {
        syn::UseTree::Path(path) => use_names(&path.tree),
        syn::UseTree::Name(name) => vec![name.ident.to_string()],
        syn::UseTree::Rename(rename) => vec![rename.rename.to_string()],
        syn::UseTree::Group(group) => group.items.iter().flat_map(use_names).collect(),
        syn::UseTree::Glob(_) => Vec::new(),
    }
}

/// A trait object's trait as the generator names it: its last path segment, with its type
/// arguments as written (`ResourceHandler<ItemResource>`).
fn trait_name(object: &syn::TypeTraitObject) -> Result<String, String> {
    object
        .bounds
        .iter()
        .find_map(|bound| match bound {
            syn::TypeParamBound::Trait(bound) => bound.path.segments.last().map(|s| {
                let args = match &s.arguments {
                    syn::PathArguments::AngleBracketed(args) => format!(
                        "<{}>",
                        args.args.iter().map(written).collect::<Vec<_>>().join(", ")
                    ),
                    _ => String::new(),
                };
                format!("{}{args}", s.ident)
            }),
            _ => None,
        })
        .ok_or_else(|| format!("dyn {}", object.to_token_stream()))
}

/// Tokens as written, without the spaces `quote` puts around punctuation
/// (`Option<Direction>`, `dyn ResourceHandler<ItemResource>`).
fn written(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .replace(" < ", "<")
        .replace("< ", "<")
        .replace(" <", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace("( )", "()")
}

pub(crate) const OUTPUT: &str = "builtin_capability_keys.rs";

fn render_keys(declarations: &[(String, KeyShape)]) -> String {
    let entries = declarations
        .iter()
        .map(|(name, shape)| {
            format!(
                "(ProvidedBy::{}, crate::{name}.name, crate::{name}.binding()),",
                shape.by
            )
        })
        .collect::<String>();
    format!(
        "// @generated by sunrise-mod-api-gen from mod_api's capability keys; do not edit.\n\
         pub(crate) fn builtin_capabilities() -> impl Iterator<Item = (ProvidedBy, &'static str, (TypeId, &'static str))> {{\n\
         [{entries}].into_iter()\n}}\n"
    )
}

/// The built-in capability metadata derived from an already-configured [`Index`].
pub(crate) fn generate_with_index(index: &mut Index<'_>) -> Result<String, String> {
    let declarations = key_declarations(index)?;
    Ok(render_keys(&declarations))
}

const GENERATED_STUBS: &[(&str, &str)] = &[
    (crate::host::AREAS_OUTPUT, ""),
    ("mod_api_generated.rs", ""),
    ("mod_api_generated_host.rs", ""),
    (OUTPUT, ""),
];

/// The built-in capability metadata derived from the public capability keys.
pub(crate) fn generate(root: &Path) -> Result<String, String> {
    let exports = crate::exports::sources(root)?;
    let mut index = Index::new(root, GENERATED_STUBS, crate::index::CRATES)
        .expanding(|_, source| crate::exports::expand_with(source, &exports));
    generate_with_index(&mut index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, fs};

    fn scan_all_capability_consts(dir: &Path, out: &mut BTreeMap<String, KeyShape>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                scan_all_capability_consts(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                let parsed = syn::parse_file(&source).unwrap();
                for item in &parsed.items {
                    if let Item::Const(c) = item
                        && matches!(c.vis, syn::Visibility::Public(_))
                        && let Some(shape) = const_key_shape(c).unwrap()
                    {
                        let prev = out.insert(c.ident.to_string(), shape);
                        assert!(
                            prev.is_none(),
                            "duplicate public capability key const {}",
                            c.ident
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn narrowed_capability_key_pass_matches_exhaustive_vocab_scan() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let exports = crate::exports::sources(&root).unwrap();
        let mut index = Index::new(&root, GENERATED_STUBS, crate::index::CRATES)
            .expanding(|_, source| crate::exports::expand_with(source, &exports));
        let narrowed = key_declarations(&mut index).unwrap();
        let read_paths = index.read.borrow().clone();
        let narrowed_map: BTreeMap<_, _> = narrowed.into_iter().collect();
        let mut exhaustive = BTreeMap::new();
        scan_all_capability_consts(&root.join("crates/sunrise-mod-vocab/src"), &mut exhaustive);
        assert_eq!(
            narrowed_map, exhaustive,
            "narrowed capability_keys pass disagreed with exhaustive vocab scan"
        );
        assert!(!narrowed_map.is_empty());
        let camera = root.join("crates/sunrise-mod-vocab/src/client/camera.rs");
        assert!(
            !read_paths.contains(&camera),
            "non-constant file {} should not be opened by capability_keys",
            camera.display()
        );
    }

    #[test]
    fn named_root_export_through_intermediate_glob_is_resolved() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let src = root.join("crates/sunrise-mod-vocab/src");
        fs::create_dir_all(src.join("exports")).unwrap();
        fs::create_dir_all(src.join("item")).unwrap();
        fs::create_dir_all(src.join("client")).unwrap();
        fs::write(
            src.join("lib.rs"),
            "include!(concat!(env!(\"OUT_DIR\"), \"/mod_api_exports_root.rs\"));\n",
        )
        .unwrap();
        fs::write(
            src.join("exports/domains.rs"),
            "pub mod item;\npub mod client;\npub use client::camera::CameraState;\n",
        )
        .unwrap();
        fs::write(
            src.join("exports/capability.rs"),
            "pub use item::capability::ITEMS;\n",
        )
        .unwrap();
        fs::write(
            src.join("item/mod.rs"),
            "pub mod capability;\npub mod keys;\n",
        )
        .unwrap();
        fs::write(
            src.join("item/capability.rs"),
            "pub use crate::item::keys::*;\n",
        )
        .unwrap();
        fs::write(
            src.join("item/keys.rs"),
            "pub const ITEMS: CapabilityKey<dyn ResourceHandler<ItemResource>> = CapabilityKey::new(\"items\");\n",
        )
        .unwrap();
        fs::write(src.join("client/mod.rs"), "pub mod camera;\n").unwrap();
        fs::write(src.join("client/camera.rs"), "pub struct CameraState;\n").unwrap();

        let exports = crate::exports::sources(root).unwrap();
        let included = [(OUTPUT, "")];
        let mut index = Index::new(root, &included, crate::index::CRATES)
            .expanding(|_, source| crate::exports::expand_with(source, &exports));
        let shapes = key_shapes(&mut index).unwrap();
        let generated = generate_with_index(&mut index).unwrap();
        let read_paths = index.read.borrow().clone();

        assert_eq!(
            shapes,
            vec![KeyShape {
                by: "Block",
                handler: "ResourceHandler<ItemResource>".to_owned(),
                context: "()".to_owned(),
            }]
        );
        assert!(generated.contains("crate::ITEMS.name"), "{generated}");
        assert!(read_paths.contains(&src.join("item/capability.rs")));
        assert!(read_paths.contains(&src.join("item/keys.rs")));
        assert!(
            !read_paths.contains(&src.join("client/camera.rs")),
            "unrelated non-constant file was opened: {read_paths:?}"
        );
    }
}
