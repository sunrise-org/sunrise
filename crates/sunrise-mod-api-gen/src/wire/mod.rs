//! The WebAssembly interface of the mod API: every method of the traits a mod calls (the hosts,
//! the backends, the drawing contexts) and every method it implements (`Hooks`, the parts, the
//! capability handlers), through one shared-memory boundary and the glue on both sides of it.
mod append;
mod borrowed;
mod calls;
mod descriptors;
mod host;
mod prepared;
#[cfg(test)]
mod prepared_tests;
#[cfg(test)]
mod protocol_pin;
pub use host::{generate_coverage, generate_host};
mod trait_roles;
mod ty;

use crate::capability_providers::key_shapes;
use crate::index::{Found, Index};
use std::path::Path;
use syn::{FnArg, Item, Pat, ReturnType, TraitItem};
use ty::{Ty, Types};

/// Imported traits a call never hands over, each reached through another's method instead.
pub(super) const REACHED: &[(&str, &str, &str)] = &[
    ("ConfigFiles", "", ""),
    ("Commands", "server_host_mut", "Ok(h.commands())"),
    ("Levels", "server_host", "Ok(h.levels())"),
    ("Messages", "server_host", "Ok(h.messages())"),
    ("Players", "server_host", "Ok(h.players())"),
    ("RuntimeLevels", "server_host_mut", "Ok(h.runtime_levels())"),
    ("ClientSession", "client_host", "Ok(h.client_session())"),
    ("RenderAssets", "client_host", "Ok(h.render_assets())"),
    ("InstanceGroups", "client_host", "Ok(h.instance_groups())"),
    // Type metadata belongs to load, so retained snapshots need no live world host.
    (
        "AttachmentDeclarations",
        "",
        "Ok(crate::__private::attachment_declarations())",
    ),
    // A level hook's host first: inside a level write it is the one the call hands over.
    (
        "AttachmentStore",
        "",
        "level_random_host_mut().map(|h| h.attachments()).or_else(|_| host_mut().map(|h| h.attachments()))",
    ),
    (
        "CapabilityProviders",
        "host",
        "Ok(h.capability_providers())",
    ),
    (
        "ServerReloadResources",
        "server_host",
        "Ok(h.reload_resources())",
    ),
    (
        "Backend",
        "host",
        "crate::backend_of(h).ok_or_else(|| missing(\"game backend\"))",
    ),
    (
        "BackendMut",
        "server_host_mut",
        "crate::backend_mut_of(h).ok_or_else(|| missing(\"game backend\"))",
    ),
    (
        "ClientBackendMut",
        "client_host_mut",
        "crate::client_backend_mut_of(h).ok_or_else(|| missing(\"client game backend\"))",
    ),
];

/// Bounded file-capability selectors, shared by the guest proxy and host dispatch.
pub(super) const FILE_ROOTS: &[(u8, &str, &str)] = &[
    (0, "Host", "config_files"),
    (1, "Host", "data_files"),
    (2, "ServerHost", "world_data_files"),
];

/// Every exported trait: the `.export` and `.held` role files, then the handler traits `mod_api`'s
/// capability keys name (`ResourceHandler<ItemResource>`), each instance of a generic one its own
/// export. Set once, by `surface`.
pub(super) fn exports() -> &'static [&'static str] {
    trait_roles::exports()
}

/// Every imported trait: the `.import` role files.
pub(super) fn imports() -> &'static [&'static str] {
    trait_roles::imports()
}

/// Whether the game also hands a mod objects of exported trait `owner` (a capability's handler,
/// another mod's provider): the mod calls one through the `held-<interface>` imports, by the
/// index the game keeps it under.
pub(super) fn held(owner: &str) -> bool {
    owner == "PlaceProvider" || trait_roles::held().contains(&owner)
}

/// The contexts of the load phase: the catalog's registration and data-load contexts and a
/// resource reload's. A call given one reads and parses data, so it runs within the load limit
/// (`Budgets::load`), not a hook call's.
const LOAD_CONTEXTS: &[&str] = &[
    "RegistrationContext",
    "DataLoadContext",
    "ServerReloadContext",
    "ClientReloadContext",
];

/// The traits the game calls on its worker threads: a mod's structure types and their pieces on
/// the worldgen workers, its block tint sources, colour resolvers and section geometry on the
/// meshing workers. They keep no mutable state and get no `Host`.
const WORKERS: &[&str] = &[
    "BlockEntityTransform",
    "StructureType",
    "StructurePiece",
    "BlockTintSource",
    "BiomeColorResolver",
    "SectionGeometry",
];

/// What a call of `method` is to the runtime, for its limits and its wait. A method given the
/// worldgen context (`alter_ground`, a worldgen Part method) runs on the worldgen workers too.
pub(super) fn call_class(types: &ty::Types<'_>, method: &Method) -> &'static str {
    if WORKERS.contains(&method.owner) || takes(types, method, &["WorldgenHost", "ChunkDiskHost"]) {
        "super::Call::Worker"
    } else if takes(types, method, LOAD_CONTEXTS) {
        "super::Call::Load"
    } else {
        "super::Call::Hook"
    }
}

/// Whether one of `method`'s arguments is one of `contexts`.
fn takes(types: &ty::Types<'_>, method: &Method, contexts: &[&str]) -> bool {
    method.params.iter().any(|(_, ty)| {
        let ty = match ty {
            ty::Ty::Ref(inner, _) => &**inner,
            ty => ty,
        };
        match ty {
            ty::Ty::Dyn(name, _) => contexts.contains(&name.as_str()),
            ty::Ty::Named(def) => contexts
                .iter()
                .any(|name| types.defs[*def].rust.rsplit("::").next() == Some(*name)),
            _ => false,
        }
    })
}

/// A trait object that crosses without methods: a provider of a mod's own handler type,
/// erased (`Arc<dyn Any + Send + Sync>`), which only mods of its group take back.
pub(super) const ERASED: &str = "Any";

/// An owner as written (`ResourceHandler<ItemResource>`): the trait's path, and its type arguments.
fn owner_parts(owner: &str) -> Result<(Vec<String>, Vec<syn::Type>), String> {
    let path: syn::Path = syn::parse_str(owner).map_err(|e| format!("{owner}: {e}"))?;
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    let args = match &path.segments.last().unwrap().arguments {
        syn::PathArguments::AngleBracketed(args) => args
            .args
            .iter()
            .filter_map(|arg| match arg {
                syn::GenericArgument::Type(ty) => Some(ty.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok((segments, args))
}

/// The exported traits the mod itself answers, with no handler index, and the guest's
/// expression for its implementation of each; every other export is a registered handler's.
pub(super) const MOD_EXPORTS: &[(&str, &str)] = &[("Hooks", "super::hooks()")];

/// The guest's receiver of `owner`'s exports, if the mod itself answers them.
pub(super) fn mod_receiver(owner: &str) -> Option<&'static str> {
    MOD_EXPORTS
        .iter()
        .find(|(o, _)| *o == owner)
        .map(|(_, r)| *r)
}

/// The exported traits that list the hooks they subscribe to, and the method listing them.
pub(super) const SUBSCRIBED: &[(&str, &str)] = &[
    ("Hooks", "subscribed"),
    ("BlockPart", "block_subscribed"),
    ("BlockEntityPart", "block_entity_subscribed"),
    ("ItemPart", "item_subscribed"),
    ("MenuPart", "menu_subscribed"),
    ("FluidPart", "fluid_subscribed"),
    ("EntityPart", "entity_subscribed"),
];

/// One trait method as it crosses.
#[derive(Debug)]
pub(super) struct Method {
    /// The interface the method belongs to: the union trait a host's area traits are parts of.
    pub(super) owner: &'static str,
    /// The area trait that declares it, when it is a part of `owner` (`host::items::ItemsHost`).
    pub(super) via: Option<String>,
    pub(super) name: String,
    pub(super) access: Option<String>,
    /// `Some(mutable)` for a `self` receiver.
    pub(super) receiver: Option<bool>,
    pub(super) params: Vec<(String, Ty)>,
    pub(super) ret: Ty,
    /// The types as written, for the shape of the call (`names`).
    pub(super) syntax: Option<Syntax>,
}

/// The ids below this are the transport's own (`mod_api::wire::RESERVED_IDS`).
const RESERVED_IDS: u64 = 0x1_0000;

/// A method's types as the source writes them.
#[derive(Debug)]
pub(super) struct Syntax {
    pub(super) module: usize,
    /// Token text of each parameter's type, receiver left out.
    pub(super) params: Vec<String>,
    pub(super) ret: Option<String>,
    pub(super) trait_params: Vec<String>,
    pub(super) owner_args: Vec<String>,
}

/// A method that can't cross, and why.
#[derive(Debug)]
pub(super) struct Unbound {
    pub(super) owner: &'static str,
    pub(super) name: String,
    pub(super) why: String,
}

pub(super) struct Surface<'a> {
    /// The methods that mention a type that does not cross: the traits still need them, and
    /// each answers that it is not available.
    pub(super) dropped: Vec<Method>,
    /// The variants of the generated `HookId`, each with its hook's name.
    pub(super) hooks: Vec<(String, String)>,
    pub(super) types: Types<'a>,
    pub(super) methods: Vec<Method>,
    pub(super) unbound: Vec<Unbound>,
    /// The methods whose doc says `Not on wasm: <why>.`
    pub(super) excused: Vec<(&'static str, String)>,
    /// Each imported trait with the area traits that are parts of it.
    pub(super) parts: Vec<(&'static str, String)>,
}

/// The line on a method's doc that keeps it off wasm, and says why.
const NOT_ON_WASM: &str = "Not on wasm: ";

/// Whether `attrs`' doc has a [`NOT_ON_WASM`] line with a reason.
fn not_on_wasm(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::NameValue(doc) if doc.path.is_ident("doc") => match &doc.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(line),
                ..
            }) => line
                .value()
                .trim()
                .strip_prefix(NOT_ON_WASM)
                .is_some_and(|why| why.len() > 1),
            _ => false,
        },
        _ => false,
    })
}

/// The methods of the bound traits, with their types resolved. `hooks` and `host` are the
/// generated sources `mod_api` includes.
pub(super) fn surface<'a>(
    root: &'a Path,
    included: &'a [(&'a str, &'a str)],
) -> Result<Surface<'a>, String> {
    surface_reading(root, included, None)
}

/// [`surface`]; with `missing`, a listed boundary trait the tree does not have is pushed there
/// instead of refusing the tree (the surface export reads older trees whose API predates a
/// listed trait; the build never does).
pub(super) fn surface_reading<'a>(
    root: &'a Path,
    included: &'a [(&'a str, &'a str)],
    mut missing: Option<&mut Vec<String>>,
) -> Result<Surface<'a>, String> {
    let mut types = Types::new(crate::index::game(root, included))?;
    let api = types.index.module("mod_api", &[])?;
    let mut declared = trait_roles::declared(root)?;
    types
        .index
        .read
        .borrow_mut()
        .extend(std::mem::take(&mut declared.paths));
    for shape in key_shapes(&mut types.index)? {
        for owner in [shape.handler.clone(), shape.provider()] {
            if !declared.exports.contains(&owner) {
                declared.held.push(owner.clone());
                declared.exports.push(owner);
            }
        }
    }
    trait_roles::select(&declared);
    let mut methods = Vec::new();
    let mut unbound = Vec::new();
    let mut excused = Vec::new();
    let mut parts = Vec::new();
    let hooks = hook_ids(&mut types)?;
    for owner in imports().iter().chain(exports()) {
        let (path, args) = owner_parts(owner)?;
        let id = match (types.index.resolve_path(api, &path), missing.as_deref_mut()) {
            (Ok(Found::Item(id)), _) => id,
            (_, Some(missing)) => {
                missing.push((*owner).to_owned());
                continue;
            }
            (Err(error), None) => return Err(error),
            (Ok(_), None) => return Err(format!("{owner} is not a mod_api trait")),
        };
        let Item::Trait(item) = types.index.item(id).clone() else {
            return Err(format!("{owner} is not a trait"));
        };
        // A generic trait's instance: its parameters as the owner's arguments.
        let mut subst = Vec::new();
        for (param, arg) in item.generics.type_params().zip(&args) {
            subst.push((param.ident.to_string(), types.ty(api, arg, &[])?));
        }
        // The trait's own methods, then those of the area traits it is a union of.
        let mut members: Vec<(usize, Option<String>, syn::TraitItemFn)> = Vec::new();
        for item in &item.items {
            if let TraitItem::Fn(function) = item {
                members.push((id.module, None, function.clone()));
            }
        }
        for bound in &item.supertraits {
            let syn::TypeParamBound::Trait(bound) = bound else {
                continue;
            };
            let segments: Vec<String> = bound
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            let area_union = (segments.len() == 4 && segments[..2] == ["crate", "host"])
                || (segments.len() == 3 && segments[..2] == ["crate", "parts"]);
            if !area_union {
                continue;
            }
            let via = segments[1..].join("::");
            let Found::Item(part) = types.index.resolve_path(api, &segments[1..])? else {
                return Err(format!("{via} is not a mod_api trait"));
            };
            let Item::Trait(part_item) = types.index.item(part).clone() else {
                return Err(format!("{via} is not a trait"));
            };
            parts.push((*owner, via.clone()));
            for item in part_item.items {
                if let TraitItem::Fn(function) = item {
                    members.push((part.module, Some(via.clone()), function));
                }
            }
        }
        for (module, via, function) in &members {
            let name = function.sig.ident.to_string();
            let subscription = SUBSCRIBED.iter().any(|(o, n)| o == owner && *n == name);
            if hidden(&function.attrs) && !subscription {
                continue;
            }
            if computed_in_mod(&function.attrs) {
                if function.default.is_none() {
                    unbound.push(Unbound {
                        owner,
                        name,
                        why: "is computed in the mod but has no default body".into(),
                    });
                }
                continue;
            }
            if not_on_wasm(&function.attrs) {
                excused.push((*owner, name.clone()));
            }
            match method(&mut types, *module, owner, function, &subst) {
                Ok(mut method) => {
                    method.via = via.clone();
                    method.syntax = Some(Syntax {
                        module: *module,
                        params: function
                            .sig
                            .inputs
                            .iter()
                            .filter_map(|input| match input {
                                FnArg::Typed(arg) => {
                                    Some(quote::ToTokens::to_token_stream(&arg.ty).to_string())
                                }
                                FnArg::Receiver(_) => None,
                            })
                            .collect(),
                        ret: match &function.sig.output {
                            ReturnType::Default => None,
                            ReturnType::Type(_, ty) => {
                                Some(quote::ToTokens::to_token_stream(ty).to_string())
                            }
                        },
                        trait_params: item
                            .generics
                            .type_params()
                            .map(|p| p.ident.to_string())
                            .collect(),
                        owner_args: args
                            .iter()
                            .map(|ty| quote::ToTokens::to_token_stream(ty).to_string())
                            .collect(),
                    });
                    methods.push(method);
                }
                Err(why) => unbound.push(Unbound { owner, name, why }),
            }
        }
    }
    // A method the API keeps off wasm (`Not on wasm:`) is gated out of the trait on that target:
    // no table holds it, and a native build of the guest answers that it is unavailable.
    let (excused_methods, methods): (Vec<Method>, Vec<Method>) = methods
        .into_iter()
        .partition(|m| excused.iter().any(|(o, n)| *o == m.owner && *n == m.name));
    let target = root.join("target");
    std::fs::create_dir_all(&target).map_err(|e| format!("creating wire report directory: {e}"))?;
    let dropped = excused_methods;
    // Track the source of checked codecs as a build-script dependency. Module
    // lookup loads it into Index even though call generation never parses its fields.
    types
        .index
        .module("mod_api", &["wire".into(), "wire_types".into()])?;
    for owner in exports() {
        let (path, _) = owner_parts(owner)?;
        // Missing only where `missing` let the first pass skip it.
        let Ok(Found::Item(id)) = types.index.resolve_path(api, &path) else {
            continue;
        };
        let Item::Trait(item) = types.index.item(id) else {
            continue;
        };
        let mut list = vec![owner.to_string()];
        for bound in &item.supertraits {
            if let syn::TypeParamBound::Trait(t) = bound {
                let name = t.path.segments.last().unwrap().ident.to_string();
                if exports().contains(&name.as_str()) {
                    list.push(name);
                }
            }
        }
        types.views.push((owner.to_string(), list));
    }
    Ok(Surface {
        dropped,
        hooks,
        types,
        methods,
        unbound,
        excused,
        parts,
    })
}

/// A frame resource already reborrowed through `REACHED`: acquiring it on the
/// guest needs no wire call. Its trait methods are the actual imports.
pub(super) fn frame_accessor(method: &Method) -> bool {
    method.params.is_empty()
        && imports().contains(&method.owner)
        && matches!(&method.ret, Ty::Dyn(name, _) if imports().iter().any(|owner| owner.rsplit("::").next() == Some(name.as_str())))
}

/// A fallible borrowed frame getter sends only its availability result across
/// Wire. The actual borrowed host remains in the synchronous native call frame.
pub(super) fn fallible_frame_accessor(method: &Method) -> Option<(&str, bool, &Ty)> {
    if !method.params.is_empty() || !imports().contains(&method.owner) {
        return None;
    }
    let Ty::Result(ok, error) = &method.ret else {
        return None;
    };
    let Ty::Dyn(name, mutable) = &**ok else {
        return None;
    };
    imports()
        .iter()
        .any(|owner| owner.rsplit("::").next() == Some(name.as_str()))
        .then_some((name.as_str(), *mutable, &**error))
}

/// The name of every method that crosses, by (owner, method): what the guest imports or
/// exports it as, and what the game's table lists it as.
pub(super) struct Names {
    by_method: std::collections::HashMap<(String, String), mod_surface::call::Name>,
    /// The calls a mod makes on a game object of an exported trait it holds (`held-` imports).
    held: std::collections::HashMap<(String, String), mod_surface::call::Name>,
    /// The three named entries of each prepared hook, by (hook, entry).
    prepared: std::collections::HashMap<(String, String), mod_surface::call::Name>,
}

impl Names {
    pub(super) fn get(&self, owner: &str, method: &str) -> &mod_surface::call::Name {
        self.by_method
            .get(&(owner.to_owned(), method.to_owned()))
            .unwrap_or_else(|| panic!("no name for {owner}::{method}"))
    }

    pub(super) fn held(&self, owner: &str, method: &str) -> &mod_surface::call::Name {
        self.held
            .get(&(owner.to_owned(), method.to_owned()))
            .unwrap_or_else(|| panic!("no held name for {owner}::{method}"))
    }

    pub(super) fn prepared(&self, hook: &str, entry: &str) -> &mod_surface::call::Name {
        self.prepared
            .get(&(hook.to_owned(), entry.to_owned()))
            .unwrap_or_else(|| panic!("no name for prepared {hook} entry {entry}"))
    }

    /// Every export a mod may implement, with the owner and method it is written as in
    /// `__sunrise_mark!`, sorted: the methods of the exported traits, then the prepared entries.
    pub(super) fn marked(&self) -> Vec<(&str, &str, &mod_surface::call::Name)> {
        let mut out: Vec<_> = self
            .by_method
            .iter()
            .filter(|((owner, _), _)| exports().contains(&owner.as_str()))
            .map(|((owner, method), name)| (owner.as_str(), method.as_str(), name))
            .chain(
                self.prepared
                    .iter()
                    .map(|((_, entry), name)| ("ItemPart", entry.as_str(), name)),
            )
            .collect();
        out.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        out
    }

    /// Every export's id, which must be distinct.
    pub(super) fn check_ids(&self) -> Result<(), String> {
        let mut seen = std::collections::HashMap::new();
        for name in self
            .by_method
            .iter()
            .filter(|((owner, _), _)| exports().contains(&owner.as_str()))
            .map(|(_, name)| name)
            .chain(self.prepared.values())
        {
            if name.export_id() < RESERVED_IDS {
                return Err(format!(
                    "{} has an id in the transport's block",
                    name.text()
                ));
            }
            if let Some(other) = seen.insert(name.export_id(), name.text()) {
                return Err(format!(
                    "{} and {} have one export id: widen the hash or rename one",
                    other,
                    name.text()
                ));
            }
        }
        Ok(())
    }
}

/// The hook's side as its name, for the hash.
fn side_word(side: &crate::catalog::Side) -> &'static str {
    match side {
        crate::catalog::Side::Server => "server",
        crate::catalog::Side::Client => "client",
        crate::catalog::Side::Both => "both",
    }
}

/// The name text of every call the surface names, by (owner as the wire writes it, method): what
/// `surface.json` lists as each record's `call`.
pub(crate) fn exported_calls(
    surface: &mut Surface<'_>,
    hooks: &[crate::catalog::Hook],
) -> Result<std::collections::BTreeMap<(String, String), String>, String> {
    let prepared: Vec<(String, String)> = hooks
        .iter()
        .filter_map(|hook| {
            hook.prepared
                .clone()
                .map(|field| (hook.name.clone(), field))
        })
        .collect();
    let names = call_names(surface, hooks, &prepared)?;
    Ok(names
        .by_method
        .iter()
        .map(|(key, name)| (key.clone(), name.text()))
        .collect())
}

/// Names every method of the surface that crosses (see `surface::calls`), the calls a mod makes
/// on objects it holds, and the entries of the prepared hooks.
pub(super) fn call_names(
    surface: &mut Surface<'_>,
    hooks: &[crate::catalog::Hook],
    prepared: &[(String, String)],
) -> Result<Names, String> {
    use crate::surface::calls::Call;
    use mod_surface::call::Kind;
    let parse = |text: &String| syn::parse_str::<syn::Type>(text).map_err(|e| e.to_string());
    let mut calls = Vec::new();
    // (table, owner, method) of each call, in order.
    let mut keys: Vec<(u8, String, String)> = Vec::new();
    for m in &surface.methods {
        let syntax = m
            .syntax
            .as_ref()
            .ok_or_else(|| format!("{}::{} has no source types", m.owner, m.name))?;
        let hook = (m.owner == "Hooks")
            .then(|| hooks.iter().find(|h| h.name == m.name))
            .flatten();
        let kind = if imports().contains(&m.owner) {
            Kind::Call
        } else if m.owner == "Hooks" {
            Kind::Hook
        } else {
            Kind::Part
        };
        let private = {
            let tys: Vec<&Ty> = m
                .params
                .iter()
                .map(|(_, ty)| ty)
                .chain(std::iter::once(&m.ret))
                .collect();
            mod_surface::shape::digest(&descriptors::text(&surface.types, &tys))
        };
        let call =
            |kind, owner: String, hook: Option<&crate::catalog::Hook>| -> Result<Call, String> {
                Ok(Call {
                    kind,
                    owner,
                    method: m.name.clone(),
                    receiver: m.receiver,
                    module: syntax.module,
                    params: syntax.params.iter().map(parse).collect::<Result<_, _>>()?,
                    ret: syntax.ret.as_ref().map(parse).transpose()?,
                    trait_params: syntax.trait_params.clone(),
                    owner_args: syntax
                        .owner_args
                        .iter()
                        .map(parse)
                        .collect::<Result<_, _>>()?,
                    extra: format!("{};{private}", hook.map_or("", |h| side_word(&h.side))),
                    version: hook.and_then(|h| u32::try_from(h.version).ok()),
                })
            };
        calls.push(call(kind, m.owner.to_owned(), hook)?);
        keys.push((0, m.owner.to_owned(), m.name.clone()));
        if held(m.owner) {
            calls.push(call(Kind::Call, format!("held::{}", m.owner), None)?);
            keys.push((1, m.owner.to_owned(), m.name.clone()));
        }
    }
    let api = surface.types.index.module("mod_api", &[])?;
    for abi in prepared::describe(&surface.methods, prepared)? {
        for entry in &abi.entries {
            let types = |parts: &[&str]| -> Result<Vec<syn::Type>, String> {
                parts
                    .iter()
                    .map(|part| {
                        let (_, ty) = part.split_once(':').ok_or("prepared argument has a role")?;
                        syn::parse_str(ty).map_err(|e: syn::Error| e.to_string())
                    })
                    .collect()
            };
            let output = types(&entry.output)?;
            calls.push(Call {
                kind: Kind::Part,
                owner: "ItemPart".to_owned(),
                method: entry.name.clone(),
                receiver: None,
                module: api,
                params: types(&entry.input)?,
                ret: Some(syn::Type::Tuple(syn::TypeTuple {
                    paren_token: Default::default(),
                    elems: output.into_iter().collect(),
                })),
                trait_params: Vec::new(),
                owner_args: Vec::new(),
                extra: abi.hook.clone(),
                version: None,
            });
            keys.push((2, abi.hook.clone(), entry.name.clone()));
        }
    }
    let names = crate::surface::calls::names(&mut surface.types.index, &calls)?;
    let mut table = Names {
        by_method: Default::default(),
        held: Default::default(),
        prepared: Default::default(),
    };
    for ((which, owner, method), name) in keys.into_iter().zip(names) {
        match which {
            0 => &mut table.by_method,
            1 => &mut table.held,
            _ => &mut table.prepared,
        }
        .insert((owner, method), name);
    }
    table.check_ids()?;
    Ok(table)
}

/// The coverage check: every method of the bound traits crosses to wasm, or its doc says why
/// not (`/// Not on wasm: <why>.`), and no method that crosses says it doesn't. The game's and
/// the mods' builds both run it, so the wasm side can't fall behind the API.
fn coverage(surface: &Surface<'_>) -> Result<(), String> {
    let excused = |owner: &str, name: &str| {
        surface
            .excused
            .iter()
            .any(|(o, n)| *o == owner && n == name)
    };
    let mut problems: Vec<String> = surface
        .unbound
        .iter()
        .filter(|u| !excused(u.owner, &u.name))
        .map(|u| {
            format!(
                "{}::{} doesn't cross to wasm ({}): fix its types, or add `/// {NOT_ON_WASM}<why>.` to its doc",
                u.owner, u.name, u.why
            )
        })
        .collect();
    problems.extend(
        surface
            .methods
            .iter()
            .filter(|m| excused(m.owner, &m.name) && !frame_accessor(m))
            .map(|m| {
                format!(
                    "{}::{} crosses to wasm: remove `{NOT_ON_WASM}` from its doc",
                    m.owner, m.name
                )
            }),
    );
    match problems.is_empty() {
        true => Ok(()),
        false => Err(problems.join("\n")),
    }
}

/// The variants of the generated `HookId`, each with its hook's name.
fn hook_ids(types: &mut Types<'_>) -> Result<Vec<(String, String)>, String> {
    let api = types.index.module("mod_api", &[])?;
    let Found::Item(id) = types.index.resolve_path(api, &["HookId".to_string()])? else {
        return Err("HookId is not a mod_api enum".into());
    };
    let Item::Enum(item) = types.index.item(id) else {
        return Err("HookId is not an enum".into());
    };
    Ok(item
        .variants
        .iter()
        .map(|v| {
            (
                v.ident.to_string(),
                heck::ToSnakeCase::to_snake_case(v.ident.to_string().as_str()),
            )
        })
        .collect())
}

fn method(
    types: &mut Types<'_>,
    module: usize,
    owner: &'static str,
    function: &syn::TraitItemFn,
    subst: &[(String, Ty)],
) -> Result<Method, String> {
    let sig = &function.sig;
    if sig.generics.type_params().next().is_some() {
        return Err("generic".into());
    }
    let mut receiver = None;
    let mut params = Vec::new();
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(r) => receiver = Some(r.mutability.is_some()),
            FnArg::Typed(arg) => {
                let name = match &*arg.pat {
                    Pat::Ident(ident) => ident.ident.to_string(),
                    _ => format!("arg{}", params.len()),
                };
                let name = name.trim_start_matches('_').to_string();
                params.push((name, types.ty(module, &arg.ty, subst)?));
            }
        }
    }
    let ret = match &sig.output {
        ReturnType::Default => Ty::Unit,
        ReturnType::Type(_, ty) => types.ty(module, ty, subst)?,
    };
    Ok(Method {
        owner,
        via: None,
        name: sig.ident.to_string(),
        access: function
            .attrs
            .iter()
            .find(|a| a.path().is_ident("access"))
            .map(|a| a.parse_args::<syn::Ident>().map(|i| i.to_string()))
            .transpose()
            .map_err(|e| e.to_string())?,
        receiver,
        params,
        ret,
        syntax: None,
    })
}

/// A provided method marked `#[computed_in_mod]`: it gets no call of its own, and its default body
/// runs wherever the trait object lives, calling the methods that do cross.
fn computed_in_mod(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "computed_in_mod")
    })
}

fn hidden(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        matches!(&attr.meta, syn::Meta::List(list)
            if list.path.is_ident("doc") && list.tokens.to_string() == "hidden")
    })
}

/// What the boundary generates from the API's sources and its generated files: the `Wire`
/// impls and layout hash of `mod_api`, and the mod's side of every call.
pub struct Generated {
    pub wire: String,
    pub guest: String,
    /// Every source file read to derive them, for the build script to track.
    pub sources: Vec<std::path::PathBuf>,
}

pub fn generate(root: &Path, included: &[(&str, &str)]) -> Result<Generated, String> {
    let providers = crate::capability_providers::generate(root, included)?.0;
    let mut with_providers = included.to_vec();
    with_providers.push(("capability_providers_generated.rs", providers.as_str()));
    let mut surface = surface(root, &with_providers)?;
    coverage(&surface)?;
    let catalog = crate::Catalog::load(root)?;
    let prepared = catalog
        .hooks
        .iter()
        .filter_map(|hook| {
            hook.prepared
                .clone()
                .map(|field| (hook.name.clone(), field))
        })
        .collect::<Vec<_>>();
    let names = call_names(&mut surface, &catalog.hooks, &prepared)?;
    let mut wire = borrowed::calls(&surface.types);
    let mut guest = calls::guest(
        &surface.types,
        &surface.methods,
        &surface.dropped,
        &surface.excused,
        &surface.parts,
        &names,
    );
    guest.push_str(&calls::hook_names(&surface.hooks));
    guest.push_str(&calls::dispatch(&surface.types, &surface.methods, &prepared, &names).0);
    guest.push_str(&calls::protocol_module());
    wire.push_str(&calls::mark_macro(&names));
    let sources = surface.types.index.read.take();
    Ok(Generated {
        wire,
        guest,
        sources,
    })
}

/// Public owned API values available to point records, without inspecting their fields.
/// The generated record's Wire bounds check its members; Clone permits copying a borrowed
/// argument, while a live capability such as Transaction remains non-copyable.
pub(crate) fn owned_types(
    root: &Path,
    included: &[(&str, &str)],
) -> Result<std::collections::BTreeSet<String>, String> {
    let providers = crate::capability_providers::generate(root, included)?.0;
    let mut with_providers = included.to_vec();
    with_providers.push(("capability_providers_generated.rs", providers.as_str()));
    let mut surface = surface(root, &with_providers)?;
    // Every owned signature def appears in generated WIRE_LAYOUT with a Wire bound.
    // That compile-time guarantee covers macro-generated slot keys/handle codecs too;
    // no owned field traversal or second Wire-implementation discovery is needed.
    let mut clones = std::collections::HashSet::new();
    let implementations = surface
        .types
        .index
        .modules
        .iter()
        .enumerate()
        .flat_map(|(module, source)| {
            source.items.iter().filter_map(move |item| match item {
                Item::Impl(item)
                    if item.trait_.as_ref().is_some_and(|(_, path, _)| {
                        path.segments.last().is_some_and(|s| s.ident == "Clone")
                    }) =>
                {
                    Some((module, item.self_ty.clone()))
                }
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    for (module, ty) in implementations {
        let syn::Type::Path(path) = *ty else {
            continue;
        };
        let path = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>();
        if let Ok(Found::Item(id)) = surface.types.index.resolve_path(module, &path) {
            clones.insert(id);
        }
    }
    let cloned =
        |index: &Index<'_>, id| ty::derives(index.item(id), "Clone") || clones.contains(&id);
    let mut allowed = std::collections::BTreeSet::new();
    for (def, value) in surface
        .types
        .defs
        .iter()
        .enumerate()
        .filter(|(_, value)| value.lifetimes == 0)
    {
        let item = surface
            .types
            .by_item
            .iter()
            .find_map(|(item, d)| (*d == def).then_some(*item));
        if item.map_or(value.rust.starts_with("serde_json::"), |id| {
            cloned(&surface.types.index, id)
        }) {
            allowed.insert(value.rust.replace(' ', ""));
        }
    }
    // Index both the public boundary reexports and checked codec declarations.
    // These lookups load source files; they do not classify any owned fields.
    surface.types.index.module("mod_api", &["wire".into()])?;
    let codec = surface
        .types
        .index
        .module("mod_api", &["wire".into(), "wire_types".into()])?;
    let mut codecs = std::collections::HashSet::new();
    for item in surface.types.index.modules[codec].items.clone() {
        let Item::Impl(item) = item else {
            continue;
        };
        // wire_types imports precisely crate::wire::Wire. Its direct impl targets
        // include foreign reexports such as ConnectionKind, which cannot derive here.
        if !item
            .trait_
            .as_ref()
            .is_some_and(|(_, p, _)| p.segments.last().is_some_and(|s| s.ident == "Wire"))
        {
            continue;
        }
        let syn::Type::Path(path) = &*item.self_ty else {
            continue;
        };
        let path = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>();
        if let Ok(Found::Item(id)) = surface.types.index.resolve_path(codec, &path) {
            codecs.insert(id);
        }
    }
    // Reuse the parsed module inventory. Extra declarations need an explicit
    // derive or checked codec; Clone alone (or a no-Wire reason) is never proof.
    let modules = surface
        .types
        .index
        .modules
        .iter()
        .enumerate()
        .filter(|(_, m)| m.krate == "mod_api")
        .map(|(module, m)| (module, m.path.clone(), m.items.clone()))
        .collect::<Vec<_>>();
    for (module, prefix, items) in modules {
        for item in items {
            let names = match item {
                Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    crate::capability_providers::use_names(&item.tree)
                }
                Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    vec![item.ident.to_string()]
                }
                Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    vec![item.ident.to_string()]
                }
                _ => continue,
            };
            for name in names {
                let Ok(Found::Item(id)) = surface.types.index.resolve(module, &name) else {
                    continue;
                };
                let declaration = surface.types.index.item(id);
                let generics = match declaration {
                    Item::Struct(item) => &item.generics,
                    Item::Enum(item) => &item.generics,
                    _ => continue,
                };
                if generics.params.is_empty()
                    && (ty::derives(declaration, "Wire") || codecs.contains(&id))
                    && cloned(&surface.types.index, id)
                {
                    let path = std::iter::once("mod_api")
                        .chain(prefix.iter().map(String::as_str))
                        .chain(std::iter::once(name.as_str()))
                        .collect::<Vec<_>>()
                        .join("::");
                    if public_path(&surface.types.index, &path) {
                        allowed.insert(path);
                    }
                }
            }
        }
    }
    Ok(allowed)
}

/// A defining path is usable by generated records only through public modules.
fn public_path(index: &Index<'_>, path: &str) -> bool {
    let segments = path.split("::").skip(1).collect::<Vec<_>>();
    (0..segments.len().saturating_sub(1)).all(|depth| {
        index.modules.iter().find(|m| m.krate == "mod_api" && m.path.iter().map(String::as_str).eq(segments[..depth].iter().copied())).is_some_and(|m| m.items.iter().any(|item| matches!(item, Item::Mod(child) if child.ident == segments[depth] && matches!(child.vis, syn::Visibility::Public(_)))))
    })
}

#[cfg(test)]
mod tests {
    /// Every method that crosses gets a name, and no two share one: the table the guest imports
    /// from and the game serves is the real API's, computed from its source.
    #[test]
    #[ignore = "daily"]
    fn every_crossing_method_has_one_distinct_call_name() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let providers = crate::capability_providers::generate(&root, &included)
                    .unwrap()
                    .0;
                let mut with_providers = included.to_vec();
                with_providers.push(("capability_providers_generated.rs", providers.as_str()));
                let mut surface = super::surface(&root, &with_providers).unwrap();
                let hooks = crate::Catalog::load(&root).unwrap().hooks;
                let names = super::call_names(&mut surface, &hooks, &[]).unwrap();
                assert_eq!(names.by_method.len(), surface.methods.len());
                let texts: std::collections::BTreeSet<String> =
                    names.by_method.values().map(|n| n.text()).collect();
                assert_eq!(texts.len(), surface.methods.len());
                for text in &texts {
                    assert!(mod_surface::call::Name::parse(text).is_ok(), "{text}");
                }
                eprintln!(
                    "{} names, longest {}",
                    texts.len(),
                    texts.iter().map(String::len).max().unwrap()
                );
                // A method added to (here: removed from) an owner moves no other call's name,
                // not even its neighbours in the same trait.
                let mut count: std::collections::HashMap<&str, usize> = Default::default();
                for method in &surface.methods {
                    *count.entry(method.owner).or_default() += 1;
                }
                let owner = count.iter().max_by_key(|(_, n)| **n).unwrap().0.to_owned();
                let at = surface
                    .methods
                    .iter()
                    .position(|m| m.owner == owner)
                    .unwrap();
                let gone = surface.methods.remove(at);
                let fewer = super::call_names(&mut surface, &hooks, &[]).unwrap();
                assert_eq!(fewer.by_method.len() + 1, names.by_method.len());
                for (key, name) in &fewer.by_method {
                    assert_eq!(
                        name.text(),
                        names.by_method[key].text(),
                        "removing {}::{} moved {}::{}",
                        gone.owner,
                        gone.name,
                        key.0,
                        key.1
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The generated impls depend on the hand-written `Wire` impls the generator skips, so an
    /// edit to that file must rerun the build script.
    #[test]
    #[ignore = "daily"]
    fn enchantment_damage_registrar_and_callback_cross_to_wire() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let providers = crate::capability_providers::generate(&root, &included)
                    .unwrap()
                    .0;
                let mut surface_included = included.to_vec();
                surface_included.push(("capability_providers_generated.rs", providers.as_str()));
                let surface = super::surface(&root, &surface_included).unwrap();
                for (owner, name) in [
                    ("EnchantmentDamageEffects", "register"),
                    ("EnchantmentDamageEffect", "apply"),
                    ("Hooks", "register_enchantment_damage_effects"),
                ] {
                    assert!(
                        surface
                            .methods
                            .iter()
                            .any(|method| method.owner == owner && method.name == name)
                    );
                }
                super::generate(&root, &included).unwrap();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The marker is the attribute, under any path; a doc line that only says so is not.
    #[test]
    fn a_method_is_computed_in_the_mod_only_by_its_attribute() {
        let item: syn::ItemTrait = syn::parse_quote! {
            trait Draws {
                fn primitive(&mut self) -> i32;
                #[sunrise_mod_api_macros::computed_in_mod]
                fn qualified(&mut self) -> i32 { self.primitive() }
                #[computed_in_mod]
                fn bare(&mut self) -> i32 { self.primitive() }
                /// Computed in the mod: but not marked.
                fn prose(&mut self) -> i32 { self.primitive() }
            }
        };
        let marked: Vec<String> = item
            .items
            .iter()
            .filter_map(|item| match item {
                syn::TraitItem::Fn(function) if super::computed_in_mod(&function.attrs) => {
                    Some(function.sig.ident.to_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(marked, ["qualified", "bare"]);
    }

    /// `RandomSource`'s range helpers are computed in the mod: only the primitive draws cross.
    #[test]
    #[ignore = "daily"]
    fn only_the_primitive_draws_of_a_random_cross() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let providers = crate::capability_providers::generate(&root, &included)
                    .unwrap()
                    .0;
                let mut surface_included = included.to_vec();
                surface_included.push(("capability_providers_generated.rs", providers.as_str()));
                let surface = super::surface(&root, &surface_included).unwrap();
                let mut crossing: Vec<&str> = surface
                    .methods
                    .iter()
                    .filter(|method| method.owner == "RandomSource")
                    .map(|method| method.name.as_str())
                    .collect();
                crossing.sort_unstable();
                assert_eq!(
                    crossing,
                    [
                        "next_boolean",
                        "next_double",
                        "next_float",
                        "next_gaussian",
                        "next_int",
                        "next_int_bound",
                        "next_long",
                    ]
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_worldgen_part_method_crosses_and_waits_for_its_group_as_a_worker_call() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let providers = crate::capability_providers::generate(&root, &included)
                    .unwrap()
                    .0;
                let mut surface_included = included.to_vec();
                surface_included.push(("capability_providers_generated.rs", providers.as_str()));
                let surface = super::surface(&root, &surface_included).unwrap();
                let class = |name: &str| {
                    let method = surface
                        .methods
                        .iter()
                        .find(|method| method.owner == "BlockPart" && method.name == name)
                        .unwrap_or_else(|| panic!("BlockPart::{name} does not cross"));
                    super::call_class(&surface.types, method)
                };
                assert_eq!(class("on_tree_grow"), "super::Call::Worker");
                assert_eq!(class("is_ladder"), "super::Call::Hook");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn the_hand_written_wire_impls_are_tracked_sources() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let sources = super::generate(&root, &included).unwrap().sources;
                assert!(
                    sources
                        .iter()
                        .any(|p| p.ends_with("crates/sunrise-mod-vocab/src/wire/wire_types.rs"))
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
    /// The rules the boundary applies to a borrowed type, which no declaration shows, are in the
    /// text every call that carries it hashes.
    #[test]
    #[ignore = "daily"]
    fn private_borrowed_rules_are_part_of_a_calls_description() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let providers = crate::capability_providers::generate(&root, &[]).unwrap().0;
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                    ("capability_providers_generated.rs", providers.as_str()),
                ];
                let mut surface = super::surface(&root, &included).unwrap();
                let describe = |surface: &super::Surface<'_>, def: usize| {
                    super::descriptors::text(&surface.types, &[&super::ty::Ty::Named(def)])
                };
                let append = surface
                    .types
                    .defs
                    .iter()
                    .position(|d| d.append.is_some())
                    .unwrap();
                let projection = surface
                    .types
                    .defs
                    .iter()
                    .position(|d| d.projection.is_some())
                    .unwrap();
                let borrowed = surface
                    .types
                    .defs
                    .iter()
                    .position(|d| {
                        d.lifetimes != 0
                            && matches!(d.borrowed, super::ty::BorrowedShape::Record(_))
                    })
                    .unwrap();
                for def in [append, projection, borrowed] {
                    let before = describe(&surface, def);
                    assert!(!before.is_empty());
                    match def {
                        d if d == append => {
                            surface.types.defs[d].append.as_mut().unwrap().field += 1
                        }
                        d if d == projection => {
                            surface.types.defs[d].projection.as_mut().unwrap()[0].ty =
                                super::ty::Ty::Unit
                        }
                        d => {
                            if let super::ty::BorrowedShape::Record(fields) =
                                &mut surface.types.defs[d].borrowed
                            {
                                fields[0].ty = super::ty::Ty::Unit;
                            }
                        }
                    }
                    assert_ne!(before, describe(&surface, def));
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
    /// The book renders the coverage page from the generator at build time; what the page
    /// promises is checked here, against the generator itself.
    #[test]
    #[ignore = "daily"]
    fn coverage_page_is_well_formed() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let page = super::generate_coverage(&root).unwrap();
                let mut lines = page.lines();
                assert_eq!(lines.next(), Some("# Wire boundary coverage"));
                let header = "| Method | Status | Reason |";
                let rows: Vec<Vec<&str>> = page
                    .lines()
                    .skip_while(|line| *line != header)
                    .skip(2)
                    .take_while(|line| line.starts_with('|'))
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| line.trim_matches('|').split(" | ").map(str::trim).collect())
                    .collect();
                assert!(
                    rows.len() > 100,
                    "the table lists the surface: {} rows",
                    rows.len()
                );
                let mut methods = std::collections::BTreeSet::new();
                for row in &rows {
                    assert_eq!(row.len(), 3, "three cells: {row:?}");
                    assert!(
                        row.iter().all(|cell| !cell.is_empty()),
                        "no empty cell: {row:?}"
                    );
                    assert!(matches!(row[1], "crosses" | "gap"), "a status: {row:?}");
                    assert!(methods.insert(row[0]), "each method once: {}", row[0]);
                }
                for owner in ["Host", "ServerHost", "ClientHost", "Registrar"] {
                    assert!(
                        methods
                            .iter()
                            .any(|m| m.starts_with(&format!("`{owner}::"))),
                        "the page lists {owner}'s methods"
                    );
                }
                assert!(
                    page.contains("Required gap total: 0."),
                    "required boundary methods must not silently lose coverage"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
    /// #792 stage 4b: an entity part crosses to wasm with its tick alone — the
    /// five native-only methods are excused off the surface, and the registrar
    /// call that attaches a wasm mod's part crosses.
    #[test]
    #[ignore = "daily"]
    fn entity_part_crosses_to_wasm_with_its_tick_alone() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let providers = crate::capability_providers::generate(&root, &included)
                    .unwrap()
                    .0;
                let mut surface_included = included.to_vec();
                surface_included.push(("capability_providers_generated.rs", providers.as_str()));
                let surface = super::surface(&root, &surface_included).unwrap();
                let crosses = |owner: &str, name: &str| {
                    surface
                        .methods
                        .iter()
                        .any(|m| m.owner == owner && m.name == name)
                };
                assert!(
                    crosses("EntityPart", "general_entity_tick"),
                    "the general entity tick crosses"
                );
                assert!(
                    crosses("Registrar", "attach_entity_part"),
                    "a wasm mod's part registers"
                );
                for name in [
                    "register_goals",
                    "register_behaviors",
                    "spawn_cluster_size",
                    "is_shearable",
                    "on_sheared",
                ] {
                    assert!(
                        surface
                            .dropped
                            .iter()
                            .any(|m| m.owner == "EntityPart" && m.name == name),
                        "EntityPart::{name} stays off wasm"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
