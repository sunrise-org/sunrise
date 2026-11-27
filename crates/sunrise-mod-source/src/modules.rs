//! A crate's source tree as the tools walk it: each file with its module path and the `cfg`s it
//! is compiled under, following `mod` declarations, `#[path]`s, inline modules and literal
//! item-position `include!`s.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Item, Meta, Token,
    parse::{Parse, ParseStream, Parser},
    punctuated::Punctuated,
};

/// A source file of a crate, with its module path and the `cfg`s it is compiled under.
#[derive(Clone)]
pub struct ModuleFile {
    pub module: String,
    pub path: PathBuf,
    /// The conditions of the `mod` and `include!` declarations that lead to it, outermost first.
    pub cfg: Vec<String>,
    /// Whether this file supplies items to the containing module via `include!`.
    pub included: bool,
}

/// One contribution to a module's items, from a file or an inline declaration in that file.
pub struct ModuleVariant {
    pub file: ModuleFile,
    /// The item indices from the file's root down to the inline module (empty for a file module).
    pub inline: Vec<usize>,
}

/// A normalized Rust cfg predicate. Both attribute extraction and stored module predicates use
/// this parser, and emitted cfg attributes quote the same tree.
#[derive(Clone)]
pub struct Cfg {
    predicate: Predicate,
    // Guarded rows in the internal catalog are keyed by their written predicates. Preserve
    // their spelling even when the emitter no longer needs the insignificant punctuation.
    written: String,
}

#[derive(Clone)]
enum Predicate {
    Bool(bool),
    All(Vec<Cfg>),
    Any(Vec<Cfg>),
    Not(Box<Cfg>),
    /// An atom and the key a set of cfg facts looks it up by ([`atom_key`]), made once.
    Atom(Meta, String),
}

impl Parse for Cfg {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(syn::LitBool) {
            let value = input.parse::<syn::LitBool>()?.value;
            return Ok(Self {
                predicate: Predicate::Bool(value),
                written: value.to_string(),
            });
        }
        let meta: Meta = input.parse()?;
        let written = meta.to_token_stream().to_string();
        let predicate = match &meta {
            Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
                let args = list.parse_args_with(Punctuated::<Self, Token![,]>::parse_terminated)?;
                if list.path.is_ident("all") {
                    Predicate::All(args.into_iter().collect())
                } else {
                    Predicate::Any(args.into_iter().collect())
                }
            }
            Meta::List(list) if list.path.is_ident("not") => {
                Predicate::Not(Box::new(list.parse_args_with(cfg_arg)?))
            }
            _ => {
                let key = atom_key(&meta);
                Predicate::Atom(meta, key)
            }
        };
        Ok(Self { predicate, written })
    }
}

impl ToTokens for Cfg {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match &self.predicate {
            Predicate::Bool(value) => quote!(#value),
            Predicate::All(args) => quote!(all(#(#args),*)),
            Predicate::Any(args) => quote!(any(#(#args),*)),
            Predicate::Not(arg) => quote!(not(#arg)),
            Predicate::Atom(meta, _) => meta.to_token_stream(),
        });
    }
}

impl Cfg {
    fn all_guards(guards: &[Self]) -> Self {
        let text = guards
            .iter()
            .map(|guard| guard.written.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            written: format!("all({text})"),
            predicate: Predicate::All(guards.to_vec()),
        }
    }

    /// The conjunction of `guards`, true when there are none.
    fn all_of(guards: Vec<Self>) -> Self {
        let guards: Vec<Self> = guards
            .into_iter()
            .filter(|guard| !matches!(&guard.predicate, Predicate::All(args) if args.is_empty()))
            .collect();
        match guards.as_slice() {
            [only] => only.clone(),
            _ => Self::all_guards(&guards),
        }
    }

    /// The disjunction of `guards`, false when there are none.
    fn any_of(guards: Vec<Self>) -> Self {
        if let [only] = guards.as_slice() {
            return only.clone();
        }
        let text = guards
            .iter()
            .map(|guard| guard.written.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            written: format!("any({text})"),
            predicate: Predicate::Any(guards),
        }
    }

    /// Evaluate the predicate with the given cfg facts. An unknown atom matters only if neither
    /// `all` nor `any` can decide without it.
    pub fn evaluate(&self, mut atom: impl FnMut(&str) -> Option<bool>) -> Option<bool> {
        fn eval(cfg: &Cfg, atom: &mut impl FnMut(&str) -> Option<bool>) -> Option<bool> {
            match &cfg.predicate {
                Predicate::Bool(value) => Some(*value),
                Predicate::Not(arg) => eval(arg, atom).map(|value| !value),
                Predicate::Atom(_, key) => atom(key.as_str()),
                Predicate::All(args) | Predicate::Any(args) => {
                    let all = matches!(&cfg.predicate, Predicate::All(_));
                    let mut unknown = false;
                    for arg in args {
                        match eval(arg, atom) {
                            Some(value) if value != all => return Some(value),
                            Some(_) => {}
                            None => unknown = true,
                        }
                    }
                    (!unknown).then_some(all)
                }
            }
        }
        eval(self, &mut atom)
    }

    /// Whether the predicate is false without `test`, regardless of other build options.
    pub fn disabled_without_tests(&self) -> bool {
        self.evaluate(|atom| (atom == "test").then_some(false)) == Some(false)
    }
}

/// Parse one predicate from a cfg attribute or a `not`, with Rust's optional trailing comma.
fn cfg_arg(input: ParseStream<'_>) -> syn::Result<Cfg> {
    let cfg = input.parse()?;
    input.parse::<Option<Token![,]>>()?;
    Ok(cfg)
}

/// The predicates imposed by `attrs`, including cfgs from arbitrarily nested `cfg_attr`s.
/// A conditional attribute applies only if all the guards enclosing it hold.
pub fn cfg_predicates(attrs: &[syn::Attribute]) -> Vec<Cfg> {
    let mut conditions = Vec::new();
    for attr in attrs {
        effective_attrs(&attr.meta, &mut Vec::new(), &mut |attr, guards| {
            let Some(predicate) = attr_cfg(attr) else {
                return;
            };
            conditions.push(if guards.is_empty() {
                predicate
            } else {
                let when = Cfg::all_guards(guards);
                let skip = Cfg {
                    written: format!("not({})", when.written),
                    predicate: Predicate::Not(Box::new(when)),
                };
                Cfg {
                    written: format!("any({}, {})", skip.written, predicate.written),
                    predicate: Predicate::Any(vec![skip, predicate]),
                }
            });
        });
    }
    conditions
}

fn attr_cfg(attr: &Meta) -> Option<Cfg> {
    if let Meta::List(list) = attr
        && list.path.is_ident("cfg")
    {
        list.parse_args_with(cfg_arg).ok()
    } else if attr.path().is_ident("test") || attr.path().is_ident("bench") {
        Some(Cfg {
            predicate: Predicate::Atom(syn::parse_quote!(test), "test".into()),
            written: "test".into(),
        })
    } else {
        None
    }
}

/// The attributes of an item in one resolved build, or `None` when that build does not contain it.
/// Conditional attributes are expanded in their original order; resolved `cfg` gates are gone.
/// Used when moving an item into a crate that does not share the original crate's cfgs.
pub fn configured_attrs(
    attrs: &[syn::Attribute],
    atom: impl Fn(&str) -> bool,
) -> Option<Vec<syn::Attribute>> {
    let mut active = Vec::new();
    for attr in attrs {
        let mut exists = true;
        effective_attrs(&attr.meta, &mut Vec::new(), &mut |meta, guards| {
            if guards
                .iter()
                .all(|guard| guard.evaluate(|name| Some(atom(name))) == Some(true))
            {
                if let Some(predicate) = attr_cfg(meta) {
                    exists &= predicate.evaluate(|name| Some(atom(name))) == Some(true);
                } else if guards.is_empty() {
                    active.push(attr.clone());
                } else {
                    active.push(syn::Attribute {
                        meta: meta.clone(),
                        ..attr.clone()
                    });
                }
            }
        });
        if !exists {
            return None;
        }
    }
    Some(active)
}

fn effective_attrs(meta: &Meta, guards: &mut Vec<Cfg>, f: &mut impl FnMut(&Meta, &[Cfg])) {
    if let Meta::List(list) = meta
        && list.path.is_ident("cfg_attr")
        && let Ok((guard, attrs)) = list.parse_args_with(|input: ParseStream<'_>| {
            let guard = input.parse::<Cfg>()?;
            let attrs = if input.is_empty() {
                Punctuated::new()
            } else {
                input.parse::<Token![,]>()?;
                Punctuated::<Meta, Token![,]>::parse_terminated(input)?
            };
            Ok((guard, attrs))
        })
    {
        guards.push(guard);
        for attr in &attrs {
            effective_attrs(attr, guards, f);
        }
        guards.pop();
    } else {
        f(meta, guards);
    }
}

/// The `cfg` conditions in `attrs`, including conditional `cfg` attributes, except test-only gates.
pub fn conditions(attrs: &[syn::Attribute]) -> Vec<String> {
    cfg_predicates(attrs)
        .into_iter()
        .filter(|predicate| !predicate.disabled_without_tests())
        .map(|predicate| predicate.written)
        .collect()
}

/// The normalized conditions in `attrs` as text, including test-only gates.
pub fn cfg_conditions(attrs: &[syn::Attribute]) -> Vec<String> {
    cfg_predicates(attrs)
        .into_iter()
        .map(|predicate| predicate.written)
        .collect()
}

/// Read a single variable from rustc's printed cfg set in the form predicates look it up.
pub fn cfg_atom(text: &str) -> syn::Result<String> {
    let meta = syn::parse_str::<Meta>(text)?;
    if matches!(meta, Meta::List(_)) {
        return Err(syn::Error::new_spanned(meta, "expected a cfg atom"));
    }
    Ok(atom_key(&meta))
}

fn atom_key(meta: &Meta) -> String {
    if let Meta::NameValue(entry) = meta
        && let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = &entry.value
    {
        let path = &entry.path;
        let value = syn::LitStr::new(&value.value(), value.span());
        quote!(#path = #value).to_string()
    } else {
        meta.to_token_stream().to_string()
    }
}

/// Evaluate a written cfg predicate with the given cfg facts.
pub fn evaluate_cfg(predicate: &str, atom: impl FnMut(&str) -> Option<bool>) -> Option<bool> {
    cfg_arg.parse_str(predicate).ok()?.evaluate(atom)
}

/// Whether a node cannot be compiled with `test` off, including gates applied by `cfg_attr`.
pub fn test_only(attrs: &[syn::Attribute]) -> bool {
    cfg_predicates(attrs)
        .iter()
        .any(Cfg::disabled_without_tests)
}

/// An item-position standard `include!` of one literal path, with the attributes on the
/// invocation. The caller resolves the path relative to the physical file containing the item.
/// A bare `include!` is the standard macro unless `shadowed` ([`include_shadowed`]); one written
/// `std::include!` or `core::include!` always is. Any other invocation is an opaque macro.
pub fn literal_include(item: &Item, shadowed: bool) -> Option<(syn::LitStr, &[syn::Attribute])> {
    let Item::Macro(item) = item else { return None };
    let path = &item.mac.path;
    let standard = if is_bare_include(path) {
        !shadowed
    } else {
        let segments: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        matches!(segments.as_slice(), [root, name] if (root == "std" || root == "core") && name == "include")
    };
    if !standard {
        return None;
    }
    let path = item
        .mac
        .parse_body_with(|input: ParseStream<'_>| {
            let path = input.parse::<syn::LitStr>()?;
            input.parse::<Option<Token![,]>>()?;
            Ok(path)
        })
        .ok()?;
    Some((path, &item.attrs))
}

/// Whether `path` is the bare name `include`, which a macro or an import of that name can shadow.
fn is_bare_include(path: &syn::Path) -> bool {
    path.leading_colon.is_none() && path.is_ident("include")
}

/// Whether `items` define a `macro_rules! include` or import a name `include`. The tools then
/// read a bare `include!` there, and in the modules inside, as that macro. This reads the items
/// as a whole, whichever line and `cfg` each is under: the module walk instead resolves them in
/// declaration order ([`walk_module_files`]).
pub fn include_shadowed(items: &[Item]) -> bool {
    items
        .iter()
        .any(|item| imports_include(item) || is_include_macro(item))
}

/// Whether `item` is a `use` that imports a name `include`. An import binds in its whole module,
/// above its own line too.
fn imports_include(item: &Item) -> bool {
    fn binds(tree: &syn::UseTree) -> bool {
        match tree {
            syn::UseTree::Path(path) => binds(&path.tree),
            syn::UseTree::Name(name) => name.ident == "include",
            syn::UseTree::Rename(rename) => rename.rename == "include",
            syn::UseTree::Group(group) => group.items.iter().any(binds),
            syn::UseTree::Glob(_) => false,
        }
    }
    matches!(item, Item::Use(used) if binds(&used.tree))
}

/// Whether `item` is a `macro_rules! include`.
fn is_include_macro(item: &Item) -> bool {
    let Item::Macro(defined) = item else {
        return false;
    };
    defined.mac.path.is_ident("macro_rules")
        && defined
            .ident
            .as_ref()
            .is_some_and(|ident| ident == "include")
}

/// The condition under which an item with `attrs` exists, including `cfg`s applied by `cfg_attr`.
fn existence(attrs: &[syn::Attribute]) -> Cfg {
    Cfg::all_of(cfg_predicates(attrs))
}

/// The condition under which `attrs` hold `#[macro_use]`, including through `cfg_attr`, or `None`
/// when they never do.
fn macro_use_guard(attrs: &[syn::Attribute]) -> Option<Cfg> {
    let mut alternatives = Vec::new();
    for attr in attrs {
        effective_attrs(&attr.meta, &mut Vec::new(), &mut |meta, guards| {
            if meta.path().is_ident("macro_use") {
                alternatives.push(Cfg::all_of(guards.to_vec()));
            }
        });
    }
    (!alternatives.is_empty()).then(|| Cfg::any_of(alternatives))
}

/// The condition under which `item` imports the name `include`, or `None` if it does not.
fn import_guard(item: &Item) -> Option<Cfg> {
    match item {
        Item::Use(used) if imports_include(item) => Some(existence(&used.attrs)),
        _ => None,
    }
}

/// The condition under which `item` brings a `macro_rules! include` into scope below it: the
/// definition itself, or an inline `#[macro_use]` module that holds one, whose macros leak into
/// the module around it. `None` when it never does. A `macro_rules!` macro is in scope only below
/// its definition, in the items after it and the modules declared after it, not in the ones before.
fn definition_guard(item: &Item) -> Option<Cfg> {
    match item {
        Item::Macro(defined) if is_include_macro(item) => Some(existence(&defined.attrs)),
        Item::Mod(declared) => {
            let (_, inner) = declared.content.as_ref()?;
            let exported = macro_use_guard(&declared.attrs)?;
            let defined: Vec<Cfg> = inner.iter().filter_map(definition_guard).collect();
            (!defined.is_empty()).then(|| {
                Cfg::all_of(vec![
                    existence(&declared.attrs),
                    exported,
                    Cfg::any_of(defined),
                ])
            })
        }
        _ => None,
    }
}

/// The most steps the walk spends on one search for a build ([`may_compile_within`]).
const CFG_SEARCH_LIMIT: usize = 1 << 12;

/// Whether some build can hold every predicate in `conditions` at once: an assignment of the
/// atoms they test (a name or `key = "value"`) under which none of them is false. Atoms vary
/// independently; `test` is false unless `include_tests`. The search tries every combination of
/// the atoms in the worst case: the module walk bounds it.
pub fn may_compile(conditions: &[String], include_tests: bool) -> bool {
    search_builds(conditions, include_tests, usize::MAX).unwrap_or(true)
}

/// [`may_compile`] for the module walk, which spends at most [`CFG_SEARCH_LIMIT`] steps on it and
/// answers `None` if that is not enough to tell. A few predicates can take every combination of
/// their atoms to refute, as `all(any(a, not(a)), any(b, not(b)), ..., z, not(z))` does with its
/// last two.
fn may_compile_within(conditions: &[String], include_tests: bool) -> Option<bool> {
    search_builds(conditions, include_tests, CFG_SEARCH_LIMIT)
}

/// The search of [`may_compile`], over at most `limit` partial assignments of the atoms; `None`
/// when it has not settled by then.
fn search_builds(conditions: &[String], include_tests: bool, limit: usize) -> Option<bool> {
    // Each predicate is read once: the search weighs all of them at every step.
    let predicates: Vec<Cfg> = conditions
        .iter()
        .filter_map(|condition| cfg_arg.parse_str(condition).ok())
        .collect();
    let mut atoms: Vec<String> = Vec::new();
    for predicate in &predicates {
        predicate.evaluate(|atom| {
            if !(atoms.iter().any(|known| known == atom) || (!include_tests && atom == "test")) {
                atoms.push(atom.to_owned());
            }
            None
        });
    }
    struct Search<'a> {
        predicates: &'a [Cfg],
        atoms: &'a [String],
        include_tests: bool,
        limit: usize,
        steps: usize,
    }
    impl Search<'_> {
        /// Whether some values of the atoms after the first `values.len()` of them leave no
        /// predicate false, given those first values; `None` once the search is out of steps.
        fn extend(&mut self, values: &mut Vec<bool>) -> Option<bool> {
            self.steps = self.steps.saturating_add(1);
            if self.steps > self.limit {
                return None;
            }
            let refuted = self.predicates.iter().any(|predicate| {
                predicate.evaluate(|atom| {
                    if !self.include_tests && atom == "test" {
                        return Some(false);
                    }
                    let at = self.atoms.iter().position(|known| known == atom)?;
                    values.get(at).copied()
                }) == Some(false)
            });
            if refuted {
                return Some(false);
            }
            if values.len() == self.atoms.len() {
                return Some(true);
            }
            for value in [true, false] {
                values.push(value);
                let found = self.extend(values);
                values.pop();
                if found != Some(false) {
                    return found;
                }
            }
            Some(false)
        }
    }
    Search {
        predicates: &predicates,
        atoms: &atoms,
        include_tests,
        limit,
        steps: 0,
    }
    .extend(&mut Vec::new())
}

/// The error for a search for a build that ran out of steps ([`CFG_SEARCH_LIMIT`]).
fn search_gave_up(path: &str) -> String {
    format!(
        "{path}: the `cfg`s of a `macro_rules! include` or an import of `include` that a bare \
         `include!` could name have too many combinations to settle (the search stopped after \
         {CFG_SEARCH_LIMIT} steps)"
    )
}

/// The files of `module` read from `path`, with conventional children in `dir`. With `every_build`,
/// a module behind a `cfg` is left out with its descendants. Unless `include_tests`, test-only
/// modules are left out, including files gated by inner attributes.
///
/// A bare `include!` is the standard macro except in a module that imports a name `include` and
/// below a `macro_rules! include` (or an inline `#[macro_use]` module that holds one): there it
/// names that macro and opens nothing. A definition or import behind a `cfg` shadows only in the
/// builds that have it: one no considered build has (a test-only one unless `include_tests`, or
/// one whose own gates exclude each other) shadows nothing, and a conditional one adds the
/// negated condition to the `cfg`s of the files the later standard `include!`s name, which only
/// the builds without it read (with `every_build` those files are left out). A definition in an
/// included fragment or in a module file whose macros reach its declaring module (`#[macro_use]`
/// on the declaration, or an inner `#![macro_use]` in the file, either also through `cfg_attr`)
/// reaches back into items already walked. That is an error only where a bare literal `include!`
/// follows the declaration or the `include!` of that file, in its module, in a module declared
/// after it, or, where the module's own macros are exported in turn, further up (`std::include!`
/// and `core::include!` name the standard macro in any case), and only if some considered build
/// compiles the definition together with the `cfg`s that lead to the file and the ones that
/// export the macro. A literal `include!` of a file that does not exist is an error too, reported
/// after any such refusal: a local macro that shadows the `include!` is what makes the missing
/// file irrelevant. The `cfg`s of a definition are weighed only when a bare `include!` can be
/// affected by it, and a search that needs more than a few thousand steps is an error.
pub fn walk_module_files(
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
    include_tests: bool,
    parse: impl Fn(&Path, &str) -> Result<syn::File, String>,
) -> Result<Vec<ModuleFile>, String> {
    Ok(
        walk_module_variants(module, path, dir, every_build, include_tests, parse)?
            .into_iter()
            .filter(|variant| variant.inline.is_empty())
            .map(|variant| variant.file)
            .collect(),
    )
}

/// Like [`walk_module_files`], but also reports inline modules with their own conditions.
pub fn walk_module_variants(
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
    include_tests: bool,
    parse: impl Fn(&Path, &str) -> Result<syn::File, String>,
) -> Result<Vec<ModuleVariant>, String> {
    let mut walker = Walker {
        every_build,
        include_tests,
        pending: vec![Pending {
            file: ModuleFile {
                module,
                path,
                cfg: Vec::new(),
                included: false,
            },
            dir,
            ancestors: Vec::new(),
            shadowed: Shadow::default(),
            exports_macros: None,
            origin: None,
        }],
        variants: Vec::new(),
        scopes: Vec::new(),
        calls: Vec::new(),
        leaks: Vec::new(),
        settled: HashMap::new(),
        error: None,
    };
    let mut missing: Option<String> = None;
    while let Some(Pending {
        mut file,
        dir,
        mut ancestors,
        shadowed,
        exports_macros,
        origin,
    }) = walker.pending.pop()
    {
        // A target with only conditional paths need not have a conventional file. A literal
        // include has no such fallback: a fragment some build compiles must be readable, and
        // one no build can compile is never opened.
        if file.included && !may_compile(&file.cfg, include_tests) {
            continue;
        }
        if !file.included && !file.path.is_file() {
            continue;
        }
        let physical = match file.path.canonicalize() {
            Ok(physical) => physical,
            // A local `include` macro that shadows the standard one makes the file it is called
            // with irrelevant, so it need not exist. Say it is missing only if the rest of the
            // walk does not refuse that macro, which explains it better.
            Err(e) if file.included => {
                missing.get_or_insert_with(|| format!("{}: {e}", file.path.display()));
                continue;
            }
            Err(e) => return Err(format!("{}: {e}", file.path.display())),
        };
        if ancestors.contains(&physical) {
            return Err(format!("{}: source inclusion cycle", file.path.display()));
        }
        ancestors.push(physical);
        let source =
            fs::read_to_string(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
        let parsed = parse(&file.path, &source)?;
        if !walker.keep(&parsed.attrs) {
            continue;
        }
        file.cfg.extend(walker.conditions(&parsed.attrs));
        // An included fragment's `macro_rules!` macros stay in scope in the file that includes it,
        // and those of a module file in the module that declares it, below that line, when the
        // declaration or an inner `#![macro_use]` of the file says `#[macro_use]`. The walk has
        // already read those items, so it refuses a definition that some build compiles, rather
        // than miss source, once the walk is done and knows whether a bare `include!` follows it
        // (`refuse_reaching_definitions`). That takes one build that satisfies, all at once, the
        // `cfg`s that lead to the file, the ones that export its macros and the definition's own:
        // where they exclude each other nothing is shadowed.
        let exported = if file.included {
            Some(Cfg::all_of(Vec::new()))
        } else {
            exports_macros.map(|declared| {
                let mut alternatives = vec![declared];
                alternatives.extend(macro_use_guard(&parsed.attrs));
                Cfg::any_of(alternatives)
            })
        };
        let scope = walker.scopes.len();
        walker.scopes.push(Scope {
            parent: origin,
            exports: exported.clone(),
        });
        if let Some(exported) = &exported
            && let Some(at) = origin
        {
            let builds: Vec<Vec<String>> = parsed
                .items
                .iter()
                .filter_map(definition_guard)
                .map(|defined| {
                    let mut builds = file.cfg.clone();
                    builds.extend([exported.written.clone(), defined.written]);
                    builds
                })
                .collect();
            if !builds.is_empty() {
                walker.leaks.push(Leak {
                    at,
                    file: file.path.display().to_string(),
                    included: file.included,
                    builds,
                });
            }
        }
        let here = file.path.parent().expect("a source file has a folder");
        walker.children(
            &file,
            &parsed.items,
            here,
            &dir,
            &mut Vec::new(),
            &ancestors,
            &shadowed,
            scope,
        );
        if let Some(error) = walker.error.take() {
            return Err(error);
        }
        walker.variants.push(ModuleVariant {
            file,
            inline: Vec::new(),
        });
    }
    walker.refuse_reaching_definitions()?;
    if let Some(error) = missing {
        return Err(error);
    }
    walker
        .variants
        .sort_by(|a, b| a.file.path.cmp(&b.file.path));
    Ok(walker.variants)
}

struct Walker {
    every_build: bool,
    include_tests: bool,
    pending: Vec<Pending>,
    variants: Vec<ModuleVariant>,
    /// The lists of items the walk has read, in the order it opened them.
    scopes: Vec<Scope>,
    /// The bare literal `include!`s the walk follows, each as its scope and its item in it.
    calls: Vec<(usize, usize)>,
    /// The `macro_rules! include` that module files and fragments define for the items already
    /// walked, until the walk knows whether a bare `include!` can reach them.
    leaks: Vec<Leak>,
    /// Whether some build has what a `cfg` guards, by the condition as written.
    settled: HashMap<String, Option<bool>>,
    /// The first search for a build that ran out of steps, which ends the walk.
    error: Option<String>,
}

/// A list of items the walk reads in order: a file's, or an inline module's.
struct Scope {
    /// The scope and the item in it (a `mod` or an `include!`) that this one is read under; `None`
    /// for the file the walk starts from.
    parent: Option<(usize, usize)>,
    /// The condition under which the macros this scope defines stay in scope below that item
    /// (`#[macro_use]` on the module, or any `include!`), or `None` if they never do.
    exports: Option<Cfg>,
}

/// A `macro_rules! include` in a module file or a fragment whose macros stay in scope below `at`,
/// the item of an earlier scope that declares or includes it.
struct Leak {
    at: (usize, usize),
    file: String,
    included: bool,
    /// For each definition, the conditions that hold all at once in a build that has it: the
    /// `cfg`s that lead to the file, the ones that export its macros and its own.
    builds: Vec<Vec<String>>,
}

/// When a bare `include!` names a macro other than the standard one: in every build the walk
/// considers, or in the builds that satisfy one of `when`. Everywhere else the standard `include!`
/// applies, so a file it names is read there. `undecided` holds the definitions and imports of
/// `include` whose builds the walk has not yet looked up: it does so, in `Walker::settle`, only
/// when a bare `include!` is read with them.
#[derive(Clone, Default)]
struct Shadow {
    always: bool,
    when: Vec<Cfg>,
    undecided: Vec<Cfg>,
}

/// A file to read, the directory Rust uses for its conventional children, its physical
/// ancestors, which builds a bare `include!` reaching it names a shadowing macro in, and, for a
/// module file, the condition under which the `#[macro_use]` on its declaration applies (also
/// through `cfg_attr`; false when there is none), which its own inner `#![macro_use]` adds to
/// when the file is read. `None` for a file no module declares: the root and included fragments.
/// `origin` is the scope and the item in it that declare or include the file, `None` for the root.
struct Pending {
    file: ModuleFile,
    dir: PathBuf,
    ancestors: Vec<PathBuf>,
    shadowed: Shadow,
    exports_macros: Option<Cfg>,
    origin: Option<(usize, usize)>,
}

impl Walker {
    fn keep(&self, attrs: &[syn::Attribute]) -> bool {
        (self.include_tests || !test_only(attrs)) && !(self.every_build && gated(attrs))
    }

    fn conditions(&self, attrs: &[syn::Attribute]) -> Vec<String> {
        if self.include_tests {
            cfg_conditions(attrs)
        } else {
            conditions(attrs)
        }
    }

    /// Whether `guard` holds in the builds this walk considers: `test` is off unless
    /// `include_tests`, and any other atom is open.
    fn holds(&self, guard: &Cfg) -> Option<bool> {
        guard.evaluate(|atom| (!self.include_tests && atom == "test").then_some(false))
    }

    /// Whether something that exists under `guard`, in the file at `path`, exists in some build the
    /// walk considers. The atoms of `guard` are tried together, so `all(a, not(a))` is as
    /// impossible as `false`. `None`, with the error recorded, when the search runs out of steps.
    fn may_exist(&mut self, guard: &Cfg, path: &Path) -> Option<bool> {
        let include_tests = self.include_tests;
        let found = *self
            .settled
            .entry(guard.written.clone())
            .or_insert_with(|| {
                may_compile_within(std::slice::from_ref(&guard.written), include_tests)
            });
        if found.is_none() {
            self.error
                .get_or_insert_with(|| search_gave_up(&path.display().to_string()));
        }
        found
    }

    /// Record in `shadow` the imports and definitions of `include` it has seen but not weighed,
    /// those that exist in some build the walk considers. A bare `include!` in the file at `path`
    /// is about to be read with them.
    fn settle(&mut self, shadow: &mut Shadow, path: &Path) {
        for guard in std::mem::take(&mut shadow.undecided) {
            let exists = match self.holds(&guard) {
                Some(decided) => decided,
                None => self.may_exist(&guard, path).unwrap_or(false),
            };
            if !exists {
                continue;
            }
            // `every_build` keeps what every build compiles, and the build that has the macro
            // does not compile the file the standard `include!` names.
            if self.every_build || self.holds(&guard) == Some(true) {
                shadow.always = true;
            } else {
                shadow.when.push(guard);
            }
        }
    }

    /// Whether the item `call` of a scope is below the item `at` of another, as Rust reads them:
    /// in that scope after it, or in a scope that is read under a later item of it.
    fn is_below(&self, mut call: (usize, usize), at: (usize, usize)) -> bool {
        loop {
            if call.0 == at.0 {
                return call.1 > at.1;
            }
            match self.scopes[call.0].parent {
                Some(parent) => call = parent,
                None => return false,
            }
        }
    }

    /// Refuse a `macro_rules! include` of a module file or fragment that reaches back into items
    /// already walked, when a bare literal `include!` below it could name it and some build has
    /// it. The macros of a scope stay in scope further up, below its own declaration, where it
    /// exports them in turn; the conditions of those exports join the builds that must hold.
    fn refuse_reaching_definitions(&self) -> Result<(), String> {
        for leak in &self.leaks {
            let mut at = leak.at;
            let mut exports: Vec<String> = Vec::new();
            loop {
                if self.calls.iter().any(|&call| self.is_below(call, at)) {
                    for defined in &leak.builds {
                        let builds: Vec<String> = defined.iter().chain(&exports).cloned().collect();
                        match may_compile_within(&builds, self.include_tests) {
                            Some(true) => {
                                return Err(format!(
                                    "{}: a `macro_rules! include` that some build compiles here \
                                     would shadow the `include!` that follows in the file that {} \
                                     it, which the module walk does not follow",
                                    leak.file,
                                    if leak.included {
                                        "includes"
                                    } else {
                                        "declares"
                                    },
                                ));
                            }
                            Some(false) => {}
                            None => return Err(search_gave_up(&leak.file)),
                        }
                    }
                    break;
                }
                let scope = &self.scopes[at.0];
                let (Some(exported), Some(parent)) = (&scope.exports, scope.parent) else {
                    break;
                };
                exports.push(exported.written.clone());
                at = parent;
            }
        }
        Ok(())
    }

    /// Look through inline modules and literal `include!`s as well as regular file declarations.
    fn children(
        &mut self,
        file: &ModuleFile,
        items: &[Item],
        here: &Path,
        dir: &Path,
        inline: &mut Vec<usize>,
        ancestors: &[PathBuf],
        inherited: &Shadow,
        scope: usize,
    ) {
        let mut shadowed = inherited.clone();
        // An import of `include` binds in the whole module; a `macro_rules! include` only below it.
        // Whether some build has either matters only to a bare `include!` read with them.
        shadowed
            .undecided
            .extend(items.iter().filter_map(import_guard));
        for (index, item) in items.iter().enumerate() {
            shadowed.undecided.extend(
                index
                    .checked_sub(1)
                    .and_then(|previous| definition_guard(&items[previous])),
            );
            let bare = matches!(item, Item::Macro(invoked) if is_bare_include(&invoked.mac.path));
            if bare
                && let Some((_, attrs)) = literal_include(item, false)
                && self.keep(attrs)
            {
                self.settle(&mut shadowed, &file.path);
            }
            if let Some((relative, attrs)) = literal_include(item, shadowed.always)
                && self.keep(attrs)
            {
                let mut included_file = file.clone();
                included_file.path = file
                    .path
                    .parent()
                    .expect("a source file has a folder")
                    .join(relative.value());
                included_file.cfg.extend(self.conditions(attrs));
                if bare {
                    // Where a macro that only some builds have shadows `include!`, the standard
                    // one, and so this file, belongs to the builds without it.
                    included_file.cfg.extend(
                        shadowed
                            .when
                            .iter()
                            .map(|guard| format!("not({})", guard.written)),
                    );
                    self.calls.push((scope, index));
                }
                included_file.included = true;
                self.pending.push(Pending {
                    file: included_file,
                    dir: dir.to_owned(),
                    ancestors: ancestors.to_vec(),
                    shadowed: shadowed.clone(),
                    exports_macros: None,
                    origin: Some((scope, index)),
                });
            }
            let Item::Mod(declared) = item else { continue };
            if !self.keep(&declared.attrs) {
                continue;
            }
            let name = declared.ident.to_string();
            let module = format!("{}::{name}", file.module);
            let mut cfg = file.cfg.clone();
            cfg.extend(self.conditions(&declared.attrs));
            let mut paths = Vec::new();
            for attr in &declared.attrs {
                effective_attrs(&attr.meta, &mut Vec::new(), &mut |attr, guards| {
                    if let Meta::NameValue(value) = attr
                        && value.path.is_ident("path")
                        && let syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(path),
                            ..
                        }) = &value.value
                    {
                        let guard = match guards {
                            [guard] => guard.written.clone(),
                            _ => Cfg::all_guards(guards).written,
                        };
                        paths.push((Some(path.value()), guard));
                    }
                });
            }
            if paths.is_empty() {
                paths.push((None, String::new()));
            } else {
                let alternatives: Vec<_> = paths.iter().map(|(_, guard)| guard.as_str()).collect();
                paths.push((None, format!("not(any({}))", alternatives.join(", "))));
            }
            for (explicit, guard) in paths {
                let mut cfg = cfg.clone();
                match evaluate_cfg(&guard, |atom| {
                    (!self.include_tests && atom == "test").then_some(false)
                }) {
                    Some(false) => continue,
                    Some(true) => {}
                    None if !guard.is_empty() => cfg.push(guard.to_owned()),
                    None => {}
                }
                if let Some((_, inner)) = &declared.content {
                    let inner_dir = dir.join(explicit.as_deref().unwrap_or(&name));
                    let child = ModuleFile {
                        module: module.clone(),
                        path: file.path.clone(),
                        cfg,
                        included: file.included,
                    };
                    inline.push(index);
                    let inner_scope = self.scopes.len();
                    self.scopes.push(Scope {
                        parent: Some((scope, index)),
                        exports: macro_use_guard(&declared.attrs),
                    });
                    self.children(
                        &child,
                        inner,
                        &inner_dir,
                        &inner_dir,
                        inline,
                        ancestors,
                        &shadowed,
                        inner_scope,
                    );
                    self.variants.push(ModuleVariant {
                        file: child,
                        inline: inline.clone(),
                    });
                    inline.pop();
                } else {
                    let (path, child_dir) = if let Some(relative) = explicit {
                        let child = here.join(relative);
                        let dir = child
                            .parent()
                            .expect("a module file has a folder")
                            .to_owned();
                        (child, dir)
                    } else if dir.join(format!("{name}.rs")).is_file() {
                        (dir.join(format!("{name}.rs")), dir.join(&name))
                    } else {
                        (dir.join(&name).join("mod.rs"), dir.join(&name))
                    };
                    self.pending.push(Pending {
                        file: ModuleFile {
                            module: module.clone(),
                            path,
                            cfg,
                            included: false,
                        },
                        dir: child_dir,
                        ancestors: ancestors.to_vec(),
                        shadowed: shadowed.clone(),
                        // The file's own `#![macro_use]` is read when it is opened.
                        exports_macros: Some(
                            macro_use_guard(&declared.attrs)
                                .unwrap_or_else(|| Cfg::any_of(Vec::new())),
                        ),
                        origin: Some((scope, index)),
                    });
                }
            }
        }
    }
}

/// Whether `attrs` hold a `cfg` (or `cfg_attr`): the item is compiled only in some builds.
pub fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discover(
        root: &Path,
        every_build: bool,
        include_tests: bool,
    ) -> Result<Vec<ModuleFile>, String> {
        walk_module_files(
            "lower".into(),
            root.join("lib.rs"),
            root.to_owned(),
            every_build,
            include_tests,
            |path, text| syn::parse_file(text).map_err(|e| format!("{}: {e}", path.display())),
        )
    }

    fn names(root: &Path, files: &[ModuleFile]) -> Vec<String> {
        let mut names: Vec<String> = files
            .iter()
            .map(|file| file.path.strip_prefix(root).unwrap().display().to_string())
            .collect();
        names.sort();
        names
    }

    /// The names of the files the walk opens, sorted.
    fn walk(root: &Path, include_tests: bool) -> Result<Vec<String>, String> {
        Ok(names(root, &discover(root, false, include_tests)?))
    }

    /// The layout of #6502. `rustc` expands the standard `include!` in `early` and the local macro
    /// in `late`, whose file does not exist.
    const EARLY_AND_LATE: &str = r#"
mod parent {
    pub mod early { include!("gen.rs"); }
    macro_rules! include { ($name:literal) => {} }
    mod late { include!("never-read.rs"); }
}
fn main() { parent::early::generated(); }
"#;

    /// A definition of `include` that only a test build has, exported to the items below it. A
    /// normal build expands the standard `include!`, which `generated` then comes from.
    const TEST_ONLY_DEFINITION: &str = r#"
#[cfg(test)]
#[macro_use]
mod shadow {
    macro_rules! include { ($name:literal) => {} }
}
include!("gen.rs");
fn main() {
    #[cfg(not(test))]
    generated();
}
"#;

    /// What `include!("gen.rs")` brings in. A build where a macro shadows `include!` never reads
    /// it; a test build that did would stop on the `compile_error!`.
    const GENERATED: &str = r#"
#[cfg(test)]
compile_error!("the standard include! expanded where a macro shadows it");
pub fn generated() {}
"#;

    /// A local `include` macro that expands to nothing: the body of the fixtures' `defs.rs`.
    const NO_OP_INCLUDE: &str = "macro_rules! include { ($name:literal) => {} }\n";

    /// A module file whose `cfg` and the `cfg_attr` that would give it `#[macro_use]` exclude each
    /// other, with `NO_OP_INCLUDE` as `defs.rs`: it never exports its macros, so the standard
    /// `include!` below it expands in every build.
    const DISJOINT_EXPORT: &str = r#"
#[cfg(feature = "fast")]
#[cfg_attr(not(feature = "fast"), macro_use)]
mod defs;
include!("gen.rs");
fn main() { generated(); }
"#;

    /// The same through an included fragment: `defs.rs` is included only with `fast`, and its
    /// macro, behind `cfg(not(feature = "fast"))`, exists only without it.
    const DISJOINT_FRAGMENT: &str = r#"
#[cfg(feature = "fast")]
include!("defs.rs");
include!("gen.rs");
fn main() { generated(); }
"#;

    /// The same through an inline module, whose export the walk reads from the items above the
    /// `include!`.
    const DISJOINT_INLINE: &str = r#"
#[cfg(feature = "fast")]
#[cfg_attr(not(feature = "fast"), macro_use)]
mod defs { macro_rules! include { ($name:literal) => {} } }
include!("gen.rs");
fn main() { generated(); }
"#;

    /// With `#![macro_use]` before `NO_OP_INCLUDE` in `defs.rs`, the local macro handles the call
    /// below `mod defs;`, so `never-read.rs` does not exist.
    const INNER_MACRO_USE: &str = "mod defs;\ninclude!(\"never-read.rs\");\nfn main() {}\n";

    /// A module exports the local macro of `NO_OP_INCLUDE` and nothing invokes it: the `include!`
    /// below is written `std::include!`, so it names the standard macro.
    const QUALIFIED_INCLUDE: &str =
        "#[macro_use]\nmod defs;\nstd::include!(\"gen.rs\");\nfn main() { generated(); }\n";

    /// A predicate that no build satisfies, which takes every combination of its first `atoms` atoms
    /// to show: `all(any(a0, not(a0)), ..., z, not(z))`.
    fn tangled(atoms: usize) -> String {
        let mut parts: Vec<String> = (0..atoms)
            .map(|n| format!("any(a{n}, not(a{n}))"))
            .collect();
        parts.extend(["z".to_owned(), "not(z)".to_owned()]);
        format!("all({})", parts.join(", "))
    }

    /// A fragment no build can compile under its gates is never opened, tests or not; only the
    /// standard `include!` is followed, written bare or as `std::include!`.
    #[test]
    fn the_walk_opens_only_compilable_standard_includes() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        put(
            "lib.rs",
            r#"
#[cfg(unix)]
include!("unix.rs");
#[cfg(false)]
include!("never.rs");
std::include!("qualified.rs");
mod shadow {
    macro_rules! include { ($($t:tt)*) => {}; }
    include!("no such file");
}
"#,
        );
        put("unix.rs", "#[cfg(not(unix))]\ninclude!(\"gone.rs\");\n");
        put("qualified.rs", "fn qualified() {}\n");
        for include_tests in [false, true] {
            let mut files = walk(root.path(), include_tests).unwrap();
            files.sort();
            assert_eq!(files, ["lib.rs", "qualified.rs", "unix.rs"]);
        }
    }

    /// A `macro_rules!` macro shadows `include!` only below its definition, so the standard
    /// `include!` expands in `early` and the local macro in `late` (this layout compiles, with no
    /// `never-read.rs`). A shadowing macro that would reach back into a file the walk has read,
    /// from an included fragment or a `#[macro_use]` module file, is refused where a bare
    /// `include!` follows it.
    #[test]
    fn a_local_include_macro_shadows_only_the_modules_below_it() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        put("lib.rs", EARLY_AND_LATE);
        put("gen.rs", GENERATED);
        assert_eq!(walk(root.path(), false).unwrap(), ["gen.rs", "lib.rs"]);

        let macro_rules = "macro_rules! include { ($name:literal) => {} }\n";
        put("lib.rs", "include!(\"defs.rs\");\ninclude!(\"gen.rs\");\n");
        put("defs.rs", macro_rules);
        let error = walk(root.path(), false).unwrap_err();
        assert!(
            error.contains("defs.rs") && error.contains("includes"),
            "{error}"
        );

        put("lib.rs", "#[macro_use]\nmod defs;\ninclude!(\"gen.rs\");\n");
        let error = walk(root.path(), false).unwrap_err();
        assert!(
            error.contains("defs.rs") && error.contains("declares"),
            "{error}"
        );

        put(
            "lib.rs",
            &format!(
                "include!(\"gen.rs\");\n#[macro_use]\nmod defs {{ {macro_rules} }}\ninclude!(\"never-read.rs\");\n"
            ),
        );
        assert_eq!(walk(root.path(), false).unwrap(), ["gen.rs", "lib.rs"]);
    }

    /// A definition or an import behind a `cfg` the build disables shadows nothing, however it is
    /// spelled and wherever it leaks to: the standard `include!` below it is read, and a fragment
    /// or a `#[macro_use]` module file that holds one is not refused.
    #[test]
    fn a_disabled_include_macro_does_not_shadow_the_standard_include() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        let macro_rules = "macro_rules! include { ($name:literal) => {} }";
        put("gen.rs", GENERATED);
        for lib in [
            TEST_ONLY_DEFINITION.to_owned(),
            format!("#[cfg(test)]\n{macro_rules}\ninclude!(\"gen.rs\");\n"),
            format!(
                "#[cfg_attr(test, macro_use)]\nmod shadow {{ {macro_rules} }}\ninclude!(\"gen.rs\");\n"
            ),
            "#[cfg(test)]\nuse other::include;\ninclude!(\"gen.rs\");\n".to_owned(),
        ] {
            put("lib.rs", &lib);
            assert_eq!(
                walk(root.path(), false).unwrap(),
                ["gen.rs", "lib.rs"],
                "{lib}"
            );
        }

        put("defs.rs", &format!("#[cfg(test)]\n{macro_rules}\n"));
        for lib in [
            "include!(\"defs.rs\");\ninclude!(\"gen.rs\");\n",
            "#[macro_use]\nmod defs;\ninclude!(\"gen.rs\");\n",
        ] {
            put("lib.rs", lib);
            assert_eq!(
                walk(root.path(), false).unwrap(),
                ["defs.rs", "gen.rs", "lib.rs"],
                "{lib}"
            );
        }

        put("defs.rs", macro_rules);
        put(
            "lib.rs",
            "#[cfg_attr(test, macro_use)]\nmod defs;\ninclude!(\"gen.rs\");\n",
        );
        assert_eq!(
            walk(root.path(), false).unwrap(),
            ["defs.rs", "gen.rs", "lib.rs"]
        );
    }

    /// A `#[macro_use]` that `cfg_attr` applies exports a local `include` macro like a bare one:
    /// the standard `include!` below an inline module is shadowed and a module file is refused.
    #[test]
    fn a_macro_use_applied_by_cfg_attr_exports_a_local_include_macro() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        let macro_rules = "macro_rules! include { ($name:literal) => {} }";
        put("gen.rs", GENERATED);
        put(
            "lib.rs",
            &format!(
                "#[cfg_attr(not(test), macro_use)]\nmod shadow {{ {macro_rules} }}\ninclude!(\"never-read.rs\");\n"
            ),
        );
        assert_eq!(walk(root.path(), false).unwrap(), ["lib.rs"]);

        put("defs.rs", macro_rules);
        put(
            "lib.rs",
            "#[cfg_attr(not(test), macro_use)]\nmod defs;\ninclude!(\"gen.rs\");\n",
        );
        let error = walk(root.path(), false).unwrap_err();
        assert!(
            error.contains("defs.rs") && error.contains("declares"),
            "{error}"
        );
    }

    /// The export, the definition and the file's own `cfg` are read together: a definition that
    /// no build both compiles and exports shadows nothing, so the walk neither refuses the module
    /// file or the fragment that holds it nor drops the `include!` below an inline module that
    /// would export it (also in an every-build walk). Gates that can hold together still refuse.
    #[test]
    fn mutually_exclusive_cfgs_do_not_export_an_include_macro() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        put("gen.rs", GENERATED);

        put("lib.rs", DISJOINT_EXPORT);
        put("defs.rs", NO_OP_INCLUDE);
        assert_eq!(
            walk(root.path(), false).unwrap(),
            ["defs.rs", "gen.rs", "lib.rs"]
        );

        put("lib.rs", DISJOINT_FRAGMENT);
        put(
            "defs.rs",
            &format!("#[cfg(not(feature = \"fast\"))]\n{NO_OP_INCLUDE}"),
        );
        assert_eq!(
            walk(root.path(), false).unwrap(),
            ["defs.rs", "gen.rs", "lib.rs"]
        );
        put(
            "defs.rs",
            &format!("#[cfg(feature = \"fast\")]\n{NO_OP_INCLUDE}"),
        );
        let error = walk(root.path(), false).unwrap_err();
        assert!(
            error.contains("defs.rs") && error.contains("includes"),
            "{error}"
        );

        put("lib.rs", DISJOINT_INLINE);
        for every_build in [false, true] {
            let files = discover(root.path(), every_build, false).unwrap();
            assert_eq!(names(root.path(), &files), ["gen.rs", "lib.rs"]);
            let generated = files
                .iter()
                .find(|file| file.path.ends_with("gen.rs"))
                .unwrap();
            assert!(generated.cfg.is_empty(), "{:?}", generated.cfg);
        }
    }

    /// A module file's inner `#![macro_use]` exports its macros like the attribute on its
    /// declaration: the call below `mod defs;` names the local macro and opens nothing (this
    /// layout compiles), so the walk refuses it rather than open `never-read.rs`. A conditional
    /// inner attribute counts in the builds that apply it, together with the `cfg` of the
    /// declaration.
    #[test]
    fn an_inner_macro_use_exports_a_local_include_macro_from_a_module_file() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        put("gen.rs", GENERATED);

        put("lib.rs", INNER_MACRO_USE);
        put("defs.rs", &format!("#![macro_use]\n{NO_OP_INCLUDE}"));
        let error = walk(root.path(), false).unwrap_err();
        assert!(
            error.contains("defs.rs")
                && error.contains("declares")
                && !error.contains("never-read.rs"),
            "{error}"
        );

        put("lib.rs", "mod defs;\ninclude!(\"gen.rs\");\n");
        put(
            "defs.rs",
            &format!("#![cfg_attr(test, macro_use)]\n{NO_OP_INCLUDE}"),
        );
        assert_eq!(
            walk(root.path(), false).unwrap(),
            ["defs.rs", "gen.rs", "lib.rs"]
        );
        let error = walk(root.path(), true).unwrap_err();
        assert!(
            error.contains("defs.rs") && error.contains("declares"),
            "{error}"
        );

        put(
            "lib.rs",
            "#[cfg(feature = \"fast\")]\nmod defs;\ninclude!(\"gen.rs\");\n",
        );
        put(
            "defs.rs",
            &format!("#![cfg_attr(not(feature = \"fast\"), macro_use)]\n{NO_OP_INCLUDE}"),
        );
        assert_eq!(
            walk(root.path(), false).unwrap(),
            ["defs.rs", "gen.rs", "lib.rs"]
        );
        put(
            "defs.rs",
            &format!("#![cfg_attr(feature = \"fast\", macro_use)]\n{NO_OP_INCLUDE}"),
        );
        let error = walk(root.path(), false).unwrap_err();
        assert!(
            error.contains("defs.rs") && error.contains("declares"),
            "{error}"
        );
    }

    /// A definition that only some builds have shadows `include!` in those builds alone: the walk
    /// keeps the file the standard `include!` names under the condition of the builds without the
    /// macro (and, for the files every build compiles, leaves it out). A conditional definition in
    /// a fragment cannot be placed on the items already walked, so it is reported.
    #[test]
    fn a_conditional_include_macro_leaves_the_standard_include_to_the_builds_without_it() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        let macro_rules = "macro_rules! include { ($name:literal) => {} }";
        let read_when = |files: &[ModuleFile], name: &str, atom: &str, on: bool| {
            let file = files
                .iter()
                .find(|file| file.path.ends_with(name))
                .unwrap_or_else(|| panic!("{name} is not walked"));
            file.cfg.iter().all(|condition| {
                evaluate_cfg(condition, |known| known.contains(atom).then_some(on)) == Some(true)
            })
        };
        put("gen.rs", GENERATED);
        put("more.rs", "pub fn more() {}\n");

        put(
            "lib.rs",
            &format!(
                "#[cfg(feature = \"fast\")]\n{macro_rules}\ninclude!(\"gen.rs\");\nmod later {{ include!(\"more.rs\"); }}\n"
            ),
        );
        let files = discover(root.path(), false, false).unwrap();
        assert_eq!(names(root.path(), &files), ["gen.rs", "lib.rs", "more.rs"]);
        for name in ["gen.rs", "more.rs"] {
            assert!(read_when(&files, name, "fast", false), "{name}");
            assert!(!read_when(&files, name, "fast", true), "{name}");
        }
        let every = discover(root.path(), true, false).unwrap();
        assert_eq!(names(root.path(), &every), ["lib.rs"]);

        put("lib.rs", TEST_ONLY_DEFINITION);
        let files = discover(root.path(), false, true).unwrap();
        assert_eq!(names(root.path(), &files), ["gen.rs", "lib.rs"]);
        assert!(read_when(&files, "gen.rs", "test", false));
        assert!(!read_when(&files, "gen.rs", "test", true));
        let every = discover(root.path(), true, false).unwrap();
        assert_eq!(names(root.path(), &every), ["gen.rs", "lib.rs"]);

        put("lib.rs", "include!(\"defs.rs\");\ninclude!(\"gen.rs\");\n");
        for (definition, include_tests) in [
            ("#[cfg(feature = \"fast\")]\n", false),
            ("#[cfg(test)]\n", true),
        ] {
            put("defs.rs", &format!("{definition}{macro_rules}\n"));
            let error = walk(root.path(), include_tests).unwrap_err();
            assert!(
                error.contains("defs.rs") && error.contains("includes"),
                "{error}"
            );
        }
    }

    /// A `macro_rules! include` that a module file or a fragment exports is refused only where a
    /// bare literal `include!` could name it: below its declaration or `include!` in the module,
    /// in a module declared after it, or in one of the modules that export it in turn. The standard
    /// macro written `std::include!` or `core::include!`, an `include!` above the export, a module
    /// that does not export and a later module with no `include!` all compile as they do without
    /// the macro, and the walk opens the same files.
    #[test]
    fn an_exported_include_macro_is_refused_only_where_a_bare_include_can_name_it() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        put("gen.rs", GENERATED);
        put("defs.rs", NO_OP_INCLUDE);
        for lib in [
            "#[macro_use]\nmod defs;\nstd::include!(\"gen.rs\");\n",
            "#[macro_use]\nmod defs;\ncore::include!(\"gen.rs\");\n",
            "include!(\"defs.rs\");\nstd::include!(\"gen.rs\");\n",
            "include!(\"gen.rs\");\n#[macro_use]\nmod defs;\n",
            "include!(\"gen.rs\");\ninclude!(\"defs.rs\");\n",
            "mod early { include!(\"gen.rs\"); }\n#[macro_use]\nmod defs;\n",
            "mod defs;\ninclude!(\"gen.rs\");\n",
        ] {
            put("lib.rs", lib);
            assert_eq!(
                walk(root.path(), false).unwrap(),
                ["defs.rs", "gen.rs", "lib.rs"],
                "{lib}"
            );
        }
        put("later.rs", "fn later() {}\n");
        put("lib.rs", "#[macro_use]\nmod defs;\nmod later;\n");
        assert_eq!(
            walk(root.path(), false).unwrap(),
            ["defs.rs", "later.rs", "lib.rs"]
        );

        let refused = |lib: &str, verb: &str| {
            put("lib.rs", lib);
            let error = walk(root.path(), false).unwrap_err();
            assert!(
                error.contains("defs.rs") && error.contains(verb),
                "{lib}: {error}"
            );
        };
        refused(
            "#[macro_use]\nmod defs;\ninclude!(\"gen.rs\");\n",
            "declares",
        );
        refused(
            "#[macro_use]\nmod defs;\nmod inner { include!(\"gen.rs\"); }\n",
            "declares",
        );
        put("later.rs", "include!(\"gen.rs\");\n");
        refused("#[macro_use]\nmod defs;\nmod later;\n", "declares");
        refused(
            "include!(\"defs.rs\");\nmod inner { include!(\"gen.rs\"); }\n",
            "includes",
        );

        // `outer` exports what `defs` exports to it only if it says `#[macro_use]` itself.
        put(
            "outer.rs",
            "#[macro_use]\n#[path = \"defs.rs\"]\nmod defs;\n",
        );
        refused(
            "#[macro_use]\nmod outer;\ninclude!(\"gen.rs\");\n",
            "declares",
        );
        put("lib.rs", "mod outer;\ninclude!(\"gen.rs\");\n");
        assert_eq!(
            walk(root.path(), false).unwrap(),
            ["defs.rs", "gen.rs", "lib.rs", "outer.rs"]
        );
    }

    /// The `cfg`s of a definition of `include` are weighed only where a bare `include!` can be
    /// affected by it, and within a bound: a predicate with no build that takes every combination
    /// of its atoms to refute does not stall the walk where nothing follows the definition, and is
    /// an error that names the file where something does.
    #[test]
    fn a_tangled_cfg_of_an_include_macro_is_weighed_only_where_a_bare_include_follows() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        let atoms = CFG_SEARCH_LIMIT.ilog2() as usize + 2;
        let definition = format!("#[cfg({})]\n{NO_OP_INCLUDE}", tangled(atoms));
        put("gen.rs", GENERATED);
        put("defs.rs", &definition);
        for lib in [
            "#[macro_use]\nmod defs;\nstd::include!(\"gen.rs\");\n",
            "include!(\"gen.rs\");\ninclude!(\"defs.rs\");\n",
        ] {
            put("lib.rs", lib);
            assert_eq!(
                walk(root.path(), false).unwrap(),
                ["defs.rs", "gen.rs", "lib.rs"],
                "{lib}"
            );
        }
        put(
            "lib.rs",
            &format!("{definition}fn unrelated() {{}}\nstd::include!(\"gen.rs\");\n"),
        );
        assert_eq!(walk(root.path(), false).unwrap(), ["gen.rs", "lib.rs"]);

        let inline = format!("{definition}include!(\"gen.rs\");\n");
        for (lib, file) in [
            (
                "#[macro_use]\nmod defs;\ninclude!(\"gen.rs\");\n",
                "defs.rs",
            ),
            ("include!(\"defs.rs\");\ninclude!(\"gen.rs\");\n", "defs.rs"),
            (inline.as_str(), "lib.rs"),
        ] {
            put("lib.rs", lib);
            let error = walk(root.path(), false).unwrap_err();
            assert!(
                error.contains(file) && error.contains("too many combinations"),
                "{lib}: {error}"
            );
        }
    }

    /// The whole-module predicate, which `sunrise-mod-api-gen` reads, keeps its meaning: a
    /// `macro_rules! include` among the items shadows, but an inline `#[macro_use]` module that
    /// holds one does not, so a standard `include!` above that module is still expanded.
    #[test]
    fn the_whole_module_predicate_does_not_look_into_macro_use_modules() {
        let items = |text: &str| syn::parse_file(text).unwrap().items;
        let macro_rules = "macro_rules! include { ($name:literal) => {} }";

        let items_above = items(&format!(
            "include!(\"gen.rs\");\n#[macro_use]\nmod shadow {{ {macro_rules} }}\n"
        ));
        assert!(!include_shadowed(&items_above));
        assert!(literal_include(&items_above[0], include_shadowed(&items_above)).is_some());

        let own = items(&format!("include!(\"gen.rs\");\n{macro_rules}\n"));
        assert!(include_shadowed(&own));
        assert!(literal_include(&own[0], include_shadowed(&own)).is_none());
    }

    /// The fixtures above, compiled: `rustc` follows `include!` the way the walk does. In the
    /// first, `early` reads `gen.rs` and `late` reads nothing (there is no `never-read.rs`). In
    /// the second, a normal build expands the standard `include!` and a test build, where the
    /// local macro is on, never reads `gen.rs`, which would stop on its `compile_error!`. In the
    /// third, an inner `#![macro_use]` in the module file makes its macro handle the call below
    /// the declaration. In the fourth, a module exports a local macro that nothing invokes, and
    /// the `std::include!` below it reads `gen.rs`. In the last, the macro's `cfg` and the one
    /// that exports it exclude each other, so every build with and without `fast` expands the
    /// standard `include!`.
    #[test]
    #[ignore = "daily"]
    fn rustc_compiles_the_include_order_fixtures_the_walk_follows() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        let compile = |extra: &[&str]| {
            let output = std::process::Command::new("rustc")
                .args(["--edition=2024", "--crate-name", "fixture"])
                .args(["--emit=metadata", "--out-dir"])
                .arg(root.path())
                .args(extra)
                .arg(root.path().join("lib.rs"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        put("gen.rs", GENERATED);

        put("lib.rs", EARLY_AND_LATE);
        compile(&[]);
        assert_eq!(walk(root.path(), false).unwrap(), ["gen.rs", "lib.rs"]);

        put("lib.rs", TEST_ONLY_DEFINITION);
        compile(&[]);
        compile(&["--test"]);
        assert_eq!(walk(root.path(), false).unwrap(), ["gen.rs", "lib.rs"]);

        put("lib.rs", INNER_MACRO_USE);
        put("defs.rs", &format!("#![macro_use]\n{NO_OP_INCLUDE}"));
        compile(&[]);
        assert!(walk(root.path(), false).unwrap_err().contains("defs.rs"));

        put("lib.rs", QUALIFIED_INCLUDE);
        put("defs.rs", NO_OP_INCLUDE);
        compile(&[]);
        assert_eq!(
            walk(root.path(), false).unwrap(),
            ["defs.rs", "gen.rs", "lib.rs"]
        );

        let fast = ["--cfg", "feature=\"fast\""];
        let fragment_defs = format!("#[cfg(not(feature = \"fast\"))]\n{NO_OP_INCLUDE}");
        for (lib, defs) in [
            (DISJOINT_EXPORT, NO_OP_INCLUDE),
            (DISJOINT_FRAGMENT, fragment_defs.as_str()),
            (DISJOINT_INLINE, ""),
        ] {
            put("lib.rs", lib);
            put("defs.rs", defs);
            compile(&[]);
            compile(&fast);
            let walked = walk(root.path(), false).unwrap();
            assert!(walked.iter().any(|name| name.as_str() == "gen.rs"));
        }
    }

    #[test]
    fn contradictory_gates_cannot_compile() {
        let gates = |gates: &[&str]| gates.iter().map(|g| (*g).to_owned()).collect::<Vec<_>>();
        assert!(may_compile(&gates(&["unix", "feature = \"a\""]), false));
        assert!(!may_compile(&gates(&["unix", "not(unix)"]), false));
        assert!(!may_compile(&gates(&["false"]), true));
        assert!(!may_compile(&gates(&["test"]), false));
        assert!(may_compile(&gates(&["test"]), true));
        assert!(!may_compile(
            &gates(&["any(a, b)", "not(a)", "not(b)"]),
            false
        ));
    }
}
