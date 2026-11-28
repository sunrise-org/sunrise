//! Owned authoring inputs generated from the private lookup storage graph.
use crate::index::{Found, Index, ItemId};
use mod_source::index::Binding;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::Path,
};
use syn::{Item, ItemStruct, ItemType, Type, Visibility, visit::Visit, visit_mut::VisitMut};

#[derive(Default)]
struct Schema {
    records: BTreeMap<String, (ItemStruct, syn::Path)>,
    aliases: BTreeMap<String, ItemType>,
    items: BTreeMap<String, ItemId>,
    selected: BTreeSet<String>,
    duplicates: BTreeSet<String>,
}

fn derived(record: &str, field: &str) -> bool {
    matches!(
        (record, field),
        ("Parts", "hooks")
            | ("Attached", "panicked")
            | (
                "Installed",
                "epoch"
                    | "namespace_authority"
                    | "hook_bits"
                    | "tag_hook_bits"
                    | "resolved_menus"
                    | "entity_type_defaults"
            )
    )
}

// The generator has syntax but no trait resolver. Recognize the Copy types this
// storage schema uses, including compositions of those types.
fn is_copy(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => reference.mutability.is_none(),
        Type::Tuple(tuple) => tuple.elems.iter().all(is_copy),
        Type::Path(path) if path.qself.is_none() => {
            let segments = &path.path.segments;
            let last = segments.last().unwrap();
            match segments.len() {
                1 if last.ident == "Option" => {
                    matches!(&last.arguments, syn::PathArguments::AngleBracketed(args)
                        if args.args.len() == 1
                            && matches!(args.args.first(), Some(syn::GenericArgument::Type(inner)) if is_copy(inner)))
                }
                1 => matches!(
                    last.ident.to_string().as_str(),
                    "bool"
                        | "char"
                        | "usize"
                        | "isize"
                        | "u8"
                        | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "f32"
                        | "f64"
                        | "HandlerThread"
                        | "PayloadProtocol"
                        | "PayloadDirection"
                        | "VanillaStages"
                ),
                3 => {
                    segments[0].ident == "std"
                        && segments[1].ident == "any"
                        && last.ident == "TypeId"
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn is_primitive(name: &str) -> bool {
    crate::PRIMITIVE_SCALARS.contains(&name) || name == "str"
}

fn is_external_crate(first: &str) -> bool {
    matches!(
        first,
        "std" | "core" | "alloc" | "sunrise_core" | "sunrise_mod_runtime" | "serde_json"
    )
}

impl Schema {
    fn select_item(&mut self, index: &mut Index<'_>, id: ItemId) -> Result<(), String> {
        if index.modules[id.module].krate != "mod_api" {
            return Ok(());
        }
        let owner_path = index.modules[id.module].path.clone();
        let (name, generic_params, refs) = match index.item(id).clone() {
            Item::Struct(item) => {
                let name = item.ident.to_string();
                // Installed is the root Parts epoch, not another domain's
                // unrelated installation wrapper with the same local name.
                if name == "Installed"
                    && owner_path.as_slice() != ["parts", "shared"]
                    && owner_path.as_slice() != ["parts", "shared", "registration_state"]
                {
                    return Ok(());
                }
                // Only private storage, plus these supported opaque lookup owners,
                // needs a mirror. Public semantic API values pass through unchanged.
                if matches!(item.vis, Visibility::Public(_))
                    && !matches!(
                        name.as_str(),
                        "Attached" | "Parts" | "TooltipAppenderPart" | "Payloads"
                    )
                {
                    return Ok(());
                }
                if let Some(&prev) = self.items.get(&name) {
                    if prev != id {
                        self.duplicates.insert(name);
                    }
                    return Ok(());
                }
                let target_name = if name == "Installed" {
                    "crate::parts::shared::Installed".to_string()
                } else if owner_path.is_empty() {
                    format!("crate::{name}")
                } else {
                    format!("crate::{}::{name}", owner_path.join("::"))
                };
                let target = syn::parse_str(&target_name).map_err(|e| e.to_string())?;
                let generic_params: HashSet<String> = item
                    .generics
                    .type_params()
                    .map(|p| p.ident.to_string())
                    .collect();
                let mut refs = Vec::new();
                for field in &item.fields {
                    if field
                        .ident
                        .as_ref()
                        .is_some_and(|id| derived(&name, &id.to_string()))
                    {
                        continue;
                    }
                    TypeRefs(&mut refs).visit_type(&field.ty);
                }
                self.items.insert(name.clone(), id);
                self.records.insert(name.clone(), (item, target));
                (name, generic_params, refs)
            }
            Item::Type(item) if !matches!(item.vis, Visibility::Public(_)) => {
                let name = item.ident.to_string();
                if let Some(&prev) = self.items.get(&name) {
                    if prev != id {
                        self.duplicates.insert(name);
                    }
                    return Ok(());
                }
                let generic_params: HashSet<String> = item
                    .generics
                    .type_params()
                    .map(|p| p.ident.to_string())
                    .collect();
                let mut refs = Vec::new();
                TypeRefs(&mut refs).visit_type(&item.ty);
                self.items.insert(name.clone(), id);
                self.aliases.insert(name.clone(), item);
                (name, generic_params, refs)
            }
            _ => return Ok(()),
        };
        self.selected.insert(name);
        for (rooted, segments) in refs {
            if !rooted && segments.len() == 1 {
                let single = segments[0].as_str();
                if generic_params.contains(single) || is_primitive(single) {
                    continue;
                }
            }
            if !rooted && segments.len() > 1 && is_external_crate(&segments[0]) {
                continue;
            }
            match index.resolve_type_binding(id.module, &segments, rooted)? {
                Binding::Found(Found::Item(dep_id))
                    if index.modules[dep_id.module].krate == "mod_api" =>
                {
                    self.select_item(index, dep_id)?;
                }
                Binding::Ambiguous(error) => return Err(error),
                _ => {}
            }
        }
        Ok(())
    }

    fn input_type(&self, ty: &Type) -> Type {
        let mut input = ty.clone();
        Inputs(&self.selected).visit_type_mut(&mut input);
        input
    }

    fn changed(&self, ty: &Type) -> bool {
        let mut names = BTreeSet::new();
        Names(&mut names).visit_type(ty);
        names.iter().any(|name| self.selected.contains(name))
    }

    fn convert(&self, value: TokenStream, ty: &Type) -> Result<TokenStream, String> {
        if !self.changed(ty) {
            return Ok(value);
        }
        match ty {
            Type::Path(path) => {
                let segment = path.path.segments.last().unwrap();
                let name = segment.ident.to_string();
                if self.records.contains_key(&name) && self.selected.contains(&name) {
                    return Ok(quote!((#value).into()));
                }
                let args = match &segment.arguments {
                    syn::PathArguments::AngleBracketed(args) => args
                        .args
                        .iter()
                        .filter_map(|arg| {
                            if let syn::GenericArgument::Type(ty) = arg {
                                Some(ty.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                if let Some(alias) = self.aliases.get(&name) {
                    let replacements = alias
                        .generics
                        .type_params()
                        .map(|p| p.ident.to_string())
                        .zip(args)
                        .collect();
                    let mut expanded = (*alias.ty).clone();
                    Substitute(&replacements).visit_type_mut(&mut expanded);
                    return self.convert(value, &expanded);
                }
                match (name.as_str(), args.as_slice()) {
                    ("Vec", [element]) => {
                        let element = self.convert(quote!(element), element)?;
                        Ok(quote!((#value).into_iter().map(|element| #element).collect()))
                    }
                    ("HashMap" | "BTreeMap", [key, element]) => {
                        let key = self.convert(quote!(key), key)?;
                        let element = self.convert(quote!(element), element)?;
                        Ok(
                            quote!((#value).into_iter().map(|(key, element)| (#key, #element)).collect()),
                        )
                    }
                    ("Option", [element]) => {
                        let element = self.convert(quote!(element), element)?;
                        Ok(quote!((#value).map(|element| #element)))
                    }
                    ("Arc", [element]) => {
                        let element =
                            self.convert(quote!(std::sync::Arc::unwrap_or_clone(#value)), element)?;
                        Ok(quote!(std::sync::Arc::new(#element)))
                    }
                    ("Box", [element]) => {
                        let element = self.convert(quote!(*#value), element)?;
                        Ok(quote!(Box::new(#element)))
                    }
                    _ => Err(format!("unsupported owned storage container {name}")),
                }
            }
            Type::Array(array) => {
                let element = self.convert(quote!(element), &array.elem)?;
                Ok(quote!((#value).map(|element| #element)))
            }
            Type::Tuple(tuple) => {
                let converted = tuple
                    .elems
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| {
                        let index = syn::Index::from(index);
                        self.convert(quote!((#value).#index), ty)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!((#(#converted,)*)))
            }
            _ => Err("unsupported private storage field shape".into()),
        }
    }

    fn emit(&self) -> Result<TokenStream, String> {
        let mut output = TokenStream::new();
        for name in &self.selected {
            let input_name = format_ident!("{name}Input");
            if let Some(alias) = self.aliases.get(name) {
                let mut input = alias.clone();
                input.ident = input_name;
                input.vis = syn::parse_quote!(pub);
                *input.ty = self.input_type(&alias.ty);
                input.attrs.clear();
                output.extend(quote!(#[doc(hidden)] #input));
                continue;
            }
            let (record, target) = &self.records[name];
            let generics = &record.generics;
            let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
            let mut fields = Vec::new();
            let mut conversions = Vec::new();
            let mut clones = Vec::new();
            let mut reattached = Vec::new();
            let mut reverse = Vec::new();
            for field in &record.fields {
                let field_name = field
                    .ident
                    .as_ref()
                    .ok_or("lookup records must have named fields")?;
                if derived(name, &field_name.to_string()) {
                    conversions.push(quote!(#field_name: Default::default()));
                    continue;
                }
                let ty = self.input_type(&field.ty);
                let attrs = &field.attrs;
                fields.push(quote!(#(#attrs)* pub #field_name: #ty));
                let converted = self.convert(quote!(input.#field_name), &field.ty)?;
                conversions.push(quote!(#field_name: #converted));
                let copied = is_copy(&field.ty);
                reverse.push(if copied {
                    quote!(#field_name: input.#field_name)
                } else {
                    quote!(#field_name: input.#field_name.clone())
                });
                reattached.push(if field_name == "part" {
                    quote!(part)
                } else if copied {
                    quote!(#field_name: self.#field_name)
                } else {
                    quote!(#field_name: self.#field_name.clone())
                });
                clones.push(if copied {
                    quote!(#field_name: self.#field_name)
                } else {
                    quote!(#field_name: self.#field_name.clone())
                });
            }
            let mut derives = Vec::new();
            for attr in &record.attrs {
                if attr.path().is_ident("derive") {
                    let paths = attr.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
                        .map_err(|e| e.to_string())?;
                    derives.extend(
                        paths
                            .into_iter()
                            .filter(|path| path.is_ident("Clone") || path.is_ident("Default")),
                    );
                }
            }
            let derive = if derives.is_empty() {
                quote!()
            } else {
                quote!(#[derive(#(#derives),*)])
            };
            output.extend(quote! {
                #[doc = "No Wire: owned native registration metadata contains callbacks and mod identities."]
                #[doc(hidden)] #derive
                pub struct #input_name #generics #where_clause { #(#fields,)* }
                impl #impl_generics From<#input_name #ty_generics> for #target #ty_generics #where_clause {
                    fn from(input: #input_name #ty_generics) -> Self { Self { #(#conversions,)* } }
                }
            });
            // Narrow testing read views reuse these leaf conversions without exposing Installed.
            if !matches!(name.as_str(), "Parts" | "TagParts" | "Installed")
                && record.fields.iter().all(|field| !self.changed(&field.ty))
            {
                let mut reverse_generics = generics.clone();
                for field in &record.fields {
                    let ty = &field.ty;
                    reverse_generics
                        .make_where_clause()
                        .predicates
                        .push(syn::parse_quote!(#ty: Clone));
                }
                let (reverse_impl, _, reverse_where) = reverse_generics.split_for_impl();
                output.extend(quote! {
                    impl #reverse_impl From<&#target #ty_generics> for #input_name #ty_generics #reverse_where {
                        fn from(input: &#target #ty_generics) -> Self { Self { #(#reverse,)* } }
                    }
                });
            }
            // Deriving Clone would incorrectly impose P: Clone on Arc<P>.
            if name == "Attached" {
                output.extend(quote! {
                    impl #impl_generics #input_name #ty_generics #where_clause {
                        /// Retain ownership metadata when expanding one Part into another view.
                        pub fn with_part<Q: ?Sized>(&self, part: Arc<Q>) -> AttachedInput<Q> {
                            AttachedInput { #(#reattached,)* }
                        }
                    }
                    impl #impl_generics Clone for #input_name #ty_generics #where_clause {
                        fn clone(&self) -> Self { Self { #(#clones,)* } }
                    }
                });
            }
        }
        Ok(output)
    }
}

fn local_path(path: &syn::TypePath) -> bool {
    path.qself.is_none()
        && path.path.leading_colon.is_none()
        && path
            .path
            .segments
            .first()
            .is_some_and(|first| !is_external_crate(&first.ident.to_string()))
}

struct TypeRefs<'a>(&'a mut Vec<(bool, Vec<String>)>);
impl<'ast> Visit<'ast> for TypeRefs<'_> {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if path.qself.is_none() && !path.path.segments.is_empty() {
            self.0.push((
                path.path.leading_colon.is_some(),
                path.path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect(),
            ));
        }
        syn::visit::visit_type_path(self, path);
    }
}

struct Names<'a>(&'a mut BTreeSet<String>);
impl<'ast> Visit<'ast> for Names<'_> {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if local_path(path) {
            self.0
                .insert(path.path.segments.last().unwrap().ident.to_string());
        }
        syn::visit::visit_type_path(self, path);
    }
}
struct Inputs<'a>(&'a BTreeSet<String>);
impl VisitMut for Inputs<'_> {
    fn visit_type_path_mut(&mut self, path: &mut syn::TypePath) {
        syn::visit_mut::visit_type_path_mut(self, path);
        let segment = path.path.segments.last().unwrap();
        if local_path(path) && self.0.contains(&segment.ident.to_string()) {
            let mut segment = segment.clone();
            segment.ident = format_ident!("{}Input", segment.ident);
            path.path = syn::Path {
                leading_colon: None,
                segments: [segment].into_iter().collect(),
            };
        }
    }
}
struct Substitute<'a>(&'a BTreeMap<String, Type>);
impl VisitMut for Substitute<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if let Type::Path(path) = ty
            && path.path.segments.len() == 1
            && let Some(value) = self.0.get(&path.path.segments[0].ident.to_string())
        {
            *ty = value.clone();
        } else {
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
}

pub(super) fn source_with_index(
    root: &Path,
    index: &mut Index<'_>,
    inputs: &mut crate::Inputs,
) -> Result<String, String> {
    let shared = index.module("mod_api", &["parts".to_owned(), "shared".to_owned()])?;
    let installed_id = match index.resolve_type_binding(shared, &["Installed".to_owned()], false)? {
        Binding::Found(Found::Item(id)) => id,
        Binding::Ambiguous(error) => return Err(error),
        _ => return Ok(String::new()),
    };
    let mut schema = Schema::default();
    schema.select_item(index, installed_id)?;
    if !schema.records.contains_key("Installed") {
        return Ok(String::new());
    }
    if let Some(name) = schema.selected.intersection(&schema.duplicates).next() {
        return Err(format!("ambiguous private storage record {name}"));
    }
    let records = schema.emit()?;
    let directory = root.join("crates/sunrise-mod-vocab/src/parts/installation_tokens");
    let mut factories = TokenStream::new();
    if directory.exists() {
        for name in inputs.module_files(&directory)? {
            let file_path = directory.join(format!("{name}.rs"));
            let path = file_path.to_string_lossy().into_owned();
            let module = format_ident!("token_{name}");
            let source = inputs
                .read_to_string(&file_path)
                .map_err(|e| e.to_string())?;
            let parsed = syn::parse_file(&source).map_err(|e| e.to_string())?;
            factories.extend(quote! { #[path = #path] mod #module; });
            for item in parsed.items {
                if let Item::Fn(function) = item
                    && matches!(function.vis, Visibility::Public(_))
                {
                    let name = function.sig.ident;
                    let conditions = function.attrs.iter().filter(|attr| {
                        attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr")
                    });
                    factories.extend(quote! { #(#conditions)* pub use #module::#name; });
                }
            }
        }
    }
    let aliases = schema
        .selected
        .iter()
        .map(|name| {
            let input = format_ident!("{name}Input");
            let original = format_ident!("{name}");
            quote!(#input as #original)
        })
        .collect::<Vec<_>>();
    let parsed = syn::parse2(quote! {
        /// Implementation-only owned inputs; not a supported mod construction surface.
        #[doc(hidden)]
        pub mod installation {
            use super::*;
            #[allow(unused_imports)] use crate::*;
            pub use super::registration_access::{install_input as install, parts_installed};
            #records
            #factories
        }
        /// Imports for the concrete authoring half, generated from the same storage graph.
        #[doc(hidden)]
        #[macro_export]
        macro_rules! __mod_parts_authoring_imports {
            () => {
                use $crate::*;
                use $crate::__private::parts_installation::{#(#aliases),*};
            };
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(prettyplease::unparse(&parsed))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    #[test]
    fn glob_imported_private_record_is_discovered_and_mirrored() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let src = root.join("crates/sunrise-mod-vocab/src");
        fs::create_dir_all(src.join("parts")).unwrap();
        fs::create_dir_all(src.join("world")).unwrap();
        fs::create_dir_all(src.join("client")).unwrap();
        fs::write(src.join("lib.rs"), "mod parts;\nmod world;\nmod client;\n").unwrap();
        fs::write(src.join("world/mod.rs"), "pub(crate) mod hidden_store;\n").unwrap();
        fs::write(
            src.join("world/hidden_store.rs"),
            "pub(crate) struct SecretRecord { pub(crate) tag: Box<str> }\n",
        )
        .unwrap();
        fs::write(src.join("client/mod.rs"), "pub mod camera;\n").unwrap();
        fs::write(src.join("client/camera.rs"), "pub struct CameraState;\n").unwrap();
        fs::write(
            src.join("parts/mod.rs"),
            "include!(concat!(env!(\"OUT_DIR\"), \"/parts_modules.rs\"));\n",
        )
        .unwrap();
        fs::write(
            src.join("parts/unions.rs.in"),
            "pub trait Registrar {} pub trait CapabilityProviders {}\n",
        )
        .unwrap();
        fs::write(
            src.join("parts/shared.rs"),
            r#"
            use crate::world::hidden_store::*;
            #[derive(Default)]
            pub(crate) struct Installed {
                pub(crate) secrets: Vec<SecretRecord>,
            }
            "#,
        )
        .unwrap();

        let mut inputs = crate::Inputs::default();
        let generated = crate::parts::source_tracked(root, &mut inputs).unwrap();
        let tracked = inputs.into_paths();

        assert!(
            generated.contains("pub struct SecretRecordInput"),
            "{generated}"
        );
        assert!(
            generated
                .contains("From<SecretRecordInput> for crate::world::hidden_store::SecretRecord"),
            "{generated}"
        );
        assert!(
            generated.contains("pub secrets: Vec<SecretRecordInput>"),
            "{generated}"
        );
        assert!(
            generated.contains("SecretRecordInput as SecretRecord"),
            "{generated}"
        );
        assert!(
            tracked.contains(&src.join("world/hidden_store.rs")),
            "glob-imported schema file must be tracked: {tracked:?}"
        );
        assert!(
            !tracked.contains(&src.join("client/camera.rs")),
            "unrelated file must not be opened: {tracked:?}"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_foreign_author_constructs_nested_inputs_without_access_to_lookup_fields() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/parts-owned-inputs-{}",
            std::process::id()
        ));
        let dir = root.join("crates/sunrise-mod-vocab/src");
        fs::create_dir_all(dir.join("parts")).unwrap();
        let storage = r#"
            use std::{collections::HashMap, sync::Arc};
            pub struct Attached<P: ?Sized> { pub(crate) owner: &'static str, pub(crate) part: Arc<P> }
            pub(crate) struct Holder<P> { pub(crate) provider: P }
            pub(crate) type Rows<P> = HashMap<Box<str>, Vec<Holder<P>>>;
            #[derive(Clone)] pub(crate) struct Registered {
                pub(crate) owner: &'static str, pub(crate) owner_index: usize,
                pub(crate) part: Arc<dyn crate::Part>,
            }
            #[derive(Clone, Default)] pub struct Payloads {
                pub(crate) phases: [HashMap<String, Registered>; 2],
            }
            #[derive(Default)] pub struct Parts {
                pub(crate) values: Vec<Attached<dyn crate::Part>>,
                pub(crate) rows: Rows<Arc<dyn crate::Part>>,
                pub(crate) payloads: Option<Arc<Payloads>>,
                pub(crate) hooks: Vec<u8>,
            }
            #[derive(Default)] pub(crate) struct TagParts {
                pub(crate) entries: Vec<(Box<str>, Attached<dyn crate::Part>)>,
            }
            #[derive(Default)] pub(crate) struct Installed {
                pub(crate) keys: Parts, pub(crate) tags: TagParts,
                pub(crate) metadata: Vec<crate::meta::Info>,
                pub(crate) hook_bits: Vec<bool>, pub(crate) tag_hook_bits: Vec<bool>,
            }
        "#;
        let metadata = r#"
            pub(crate) struct Info { pub(crate) id: Box<str>, pub(crate) hooks: Vec<u32> }
        "#;
        fs::write(dir.join("lib.rs"), "mod meta;\nmod parts;\n").unwrap();
        fs::write(
            dir.join("parts/mod.rs"),
            "include!(concat!(env!(\"OUT_DIR\"), \"/parts_modules.rs\"));\n",
        )
        .unwrap();
        fs::write(
            dir.join("parts/unions.rs.in"),
            "pub trait Registrar {} pub trait CapabilityProviders {}\n",
        )
        .unwrap();
        fs::write(dir.join("parts/shared.rs"), storage).unwrap();
        fs::write(dir.join("meta.rs"), metadata).unwrap();
        let generated = crate::parts::source(&root).unwrap();
        let native = format!(
            r#"
            pub trait Part: Send + Sync {{}} impl Part for u32 {{}}
            mod meta {{ {metadata} }}
            mod parts {{
                use std::{{collections::HashMap, sync::Arc}};
                mod registration_access {{
                    pub fn parts_installed() -> bool {{ false }}
                    pub fn install_input(input: super::installation::InstalledInput) {{
                        let _: super::shared::Installed = input.into();
                    }}
                }}
                {generated}
            }}
            pub use parts::installation;
            pub use parts::{{Attached, Payloads}};
            pub fn inspect(input: installation::InstalledInput) -> (usize, usize, String, Vec<u32>, bool) {{
                let expected = input.keys.payloads.as_ref().unwrap().phases[0]["sample"].part.clone();
                let stored: parts::Installed = input.into();
                let phases = &stored.keys.payloads.as_ref().unwrap().phases;
                assert_eq!(phases[0]["sample"].owner, "after");
                assert_eq!(phases[1]["sample"].owner, "owner");
                assert_eq!(phases[0]["sample"].owner_index, 11);
                assert_eq!(phases[1]["sample"].owner_index, 19);
                assert!(std::sync::Arc::ptr_eq(&phases[0]["sample"].part, &expected));
                assert!(std::sync::Arc::ptr_eq(&phases[1]["sample"].part, &expected));
                assert_eq!(stored.keys.values[0].owner, "owner");
                assert_eq!(stored.tags.entries[0].0.as_ref(), "tag");
                let item = &stored.metadata[0];
                (stored.keys.values.len(), stored.keys.rows.len(), item.id.to_string(), item.hooks.clone(),
                 stored.keys.hooks.is_empty() && stored.hook_bits.is_empty() && stored.tag_hook_bits.is_empty())
            }}
        "#
        );
        fs::write(root.join("native.rs"), native).unwrap();
        let output = std::process::Command::new("rustc")
            .args(["--edition=2024", "--crate-type=rlib", "--crate-name=lookup"])
            .arg(root.join("native.rs"))
            .arg("-o")
            .arg(root.join("liblookup.rlib"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(root.join("author.rs"), r#"
            use lookup::installation::*;
            use std::{collections::HashMap, sync::Arc};
            #[test] fn owned_metadata_survives_installation() {
                let part: Arc<dyn lookup::Part> = Arc::new(5u32);
                let attached = AttachedInput { owner: "owner", part: part.clone() };
                let cloned = attached.clone();
                assert!(Arc::ptr_eq(&attached.part, &cloned.part));
                let checkpoint = Arc::new(PayloadsInput {
                    phases: [11, 19].map(|owner_index| HashMap::from([("sample".into(), RegisteredInput {
                        owner: "owner", owner_index, part: part.clone(),
                    })])),
                });
                let mut payloads = checkpoint.clone();
                Arc::make_mut(&mut payloads).phases[0].get_mut("sample").unwrap().owner = "after";
                let retained = payloads.clone();
                let input = InstalledInput {
                    keys: PartsInput {
                        payloads: Some(payloads),
                        values: vec![attached, cloned],
                        rows: HashMap::from([("row".into(), vec![HolderInput { provider: part.clone() }])]),
                    },
                    tags: TagPartsInput { entries: vec![("tag".into(), AttachedInput { owner: "tag", part })] },
                    metadata: vec![InfoInput { id: "metadata".into(), hooks: vec![7, 8] }],
                };
                assert_eq!(lookup::inspect(input), (2, 1, "metadata".to_owned(), vec![7, 8], true));
                assert_eq!(checkpoint.phases[0]["sample"].owner, "owner");
                assert_eq!(retained.phases[0]["sample"].owner, "after");
            }
        "#).unwrap();
        let binary = root.join(format!("author{}", std::env::consts::EXE_SUFFIX));
        let output = std::process::Command::new("rustc")
            .args(["--edition=2024", "--test"])
            .arg(root.join("author.rs"))
            .arg("--extern")
            .arg(format!("lookup={}", root.join("liblookup.rlib").display()))
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = std::process::Command::new(binary).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        for construction in [
            r#"lookup::Attached { owner: "not public", part: std::sync::Arc::new(5u32) }"#,
            "lookup::Payloads { phases: Default::default() }",
        ] {
            fs::write(
                root.join("forbidden.rs"),
                format!("fn main() {{ let _ = {construction}; }}"),
            )
            .unwrap();
            let output = std::process::Command::new("rustc")
                .arg("--edition=2024")
                .arg(root.join("forbidden.rs"))
                .arg("--extern")
                .arg(format!("lookup={}", root.join("liblookup.rlib").display()))
                .arg("-o")
                .arg(root.join("forbidden"))
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "lookup fields became public: {construction}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
