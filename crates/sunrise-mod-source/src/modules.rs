//! A crate's source tree as the tools walk it: each file with its module path and the `cfg`s it
//! is compiled under, following `mod` declarations, `#[path]`s, inline modules and literal
//! item-position `include!`s.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::{
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
    /// The parsed items of this file or inline module.
    pub items: Vec<Item>,
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
    Atom(Meta),
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
            _ => Predicate::Atom(meta),
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
            Predicate::Atom(meta) => meta.to_token_stream(),
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

    /// Evaluate the predicate with the given cfg facts. An unknown atom matters only if neither
    /// `all` nor `any` can decide without it.
    pub fn evaluate(&self, mut atom: impl FnMut(&str) -> Option<bool>) -> Option<bool> {
        fn eval(cfg: &Cfg, atom: &mut impl FnMut(&str) -> Option<bool>) -> Option<bool> {
            match &cfg.predicate {
                Predicate::Bool(value) => Some(*value),
                Predicate::Not(arg) => eval(arg, atom).map(|value| !value),
                Predicate::Atom(meta) => atom(&atom_key(meta)),
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
            predicate: Predicate::Atom(syn::parse_quote!(test)),
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
/// A bare `include!` is the standard macro unless `shadowed` ([`IncludeScope`]); one written
/// `std::include!` or `core::include!` always is. Any other invocation is an opaque macro.
pub fn literal_include(item: &Item, shadowed: bool) -> Option<(syn::LitStr, &[syn::Attribute])> {
    let item = standard_include(item, shadowed)?;
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

/// An item-position `include!(concat!(env!("OUT_DIR"), "/<file>"))` of a build script's output,
/// with the file's name and the attributes on the invocation. Only this exact shape of the
/// standard `include!` ([`literal_include`] says which one): any other `include!` is an ordinary
/// one, whatever its path says.
pub fn generated_include(item: &Item, shadowed: bool) -> Option<(String, &[syn::Attribute])> {
    let item = standard_include(item, shadowed)?;
    let Ok(syn::Expr::Macro(concat)) = item.mac.parse_body::<syn::Expr>() else {
        return None;
    };
    if !concat.mac.path.is_ident("concat") {
        return None;
    }
    let args = concat
        .mac
        .parse_body_with(Punctuated::<syn::Expr, Token![,]>::parse_terminated)
        .ok()?;
    let mut args = args.iter();
    let (
        Some(syn::Expr::Macro(env)),
        Some(syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(file),
            ..
        })),
        None,
    ) = (args.next(), args.next(), args.next())
    else {
        return None;
    };
    if !env.mac.path.is_ident("env")
        || env.mac.parse_body::<syn::LitStr>().ok()?.value() != "OUT_DIR"
    {
        return None;
    }
    let file = file.value();
    let name = file.strip_prefix('/')?;
    (!name.is_empty() && !name.contains('/')).then(|| (name.to_owned(), item.attrs.as_slice()))
}

/// `items` as compiled when the invocation that produced them carries `attrs`: its `cfg` gates
/// join each item's own.
/// `cfg_attr` can apply attributes other than `cfg` to the invocation itself. Refuse it
/// rather than forwarding those attributes to the expanded items.
pub fn gate_items(items: Vec<Item>, attrs: &[syn::Attribute]) -> Result<Vec<Item>, String> {
    if attrs.iter().any(|attr| attr.path().is_ident("cfg_attr")) {
        return Err("cannot expand a generated include! with a cfg_attr attribute".into());
    }
    let gates: Vec<_> = attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"))
        .collect();
    if gates.is_empty() {
        return Ok(items);
    }
    Ok(items
        .into_iter()
        .map(|item| -> Item { syn::parse_quote!(#(#gates)* #item) })
        .collect())
}

/// An item-position invocation of the standard `include!`: see [`literal_include`].
pub fn standard_include(item: &Item, shadowed: bool) -> Option<&syn::ItemMacro> {
    let Item::Macro(item) = item else { return None };
    let path = &item.mac.path;
    let standard = if path.leading_colon.is_none() && path.is_ident("include") {
        !shadowed
    } else {
        let segments: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        matches!(segments.as_slice(), [root, name] if (root == "std" || root == "core") && name == "include")
    };
    standard.then_some(item)
}

/// A direct `macro_rules! include` shadows bare `include!` after its definition, including
/// in child modules declared after it. Named imports apply throughout their own module;
/// literal include fragments stay in that module. A glob import may import `include`, but we
/// do not resolve globs here. We do not track macro leakage from `#[macro_use] mod` or
/// `#[macro_use] extern crate`, or shadowing introduced by macro expansion.
#[derive(Clone, Copy, Default)]
pub struct IncludeScope {
    lexical: bool,
    imported: bool,
}

impl IncludeScope {
    pub fn with_imports(mut self, items: &[Item]) -> Self {
        fn binds(tree: &syn::UseTree) -> bool {
            match tree {
                syn::UseTree::Path(path) => binds(&path.tree),
                syn::UseTree::Name(name) => name.ident == "include",
                syn::UseTree::Rename(rename) => rename.rename == "include",
                syn::UseTree::Group(group) => group.items.iter().any(binds),
                syn::UseTree::Glob(_) => false,
            }
        }
        self.imported |= items
            .iter()
            .any(|item| matches!(item, Item::Use(used) if binds(&used.tree)));
        self
    }

    /// Return the scope at this item, advancing the cursor for subsequent items.
    pub fn before(&mut self, item: &Item) -> Self {
        let before = *self;
        self.lexical |= matches!(item, Item::Macro(defined)
            if defined.mac.path.is_ident("macro_rules")
                && defined.ident.as_ref().is_some_and(|ident| ident == "include"));
        before
    }

    pub fn shadowed(self) -> bool {
        self.lexical || self.imported
    }

    /// Enter a child module: textual macro scope is inherited, named imports are not.
    pub fn child(self) -> Self {
        Self {
            imported: false,
            ..self
        }
    }
}

/// Whether some build can hold every predicate in `conditions` at once: an assignment of the
/// atoms they test (a name or `key = "value"`) under which none of them is false. Atoms vary
/// independently; `test` is false unless `include_tests`.
pub fn may_compile(conditions: &[String], include_tests: bool) -> bool {
    let mut atoms: Vec<String> = Vec::new();
    for condition in conditions {
        evaluate_cfg(condition, |atom| {
            if !(atoms.iter().any(|known| known == atom) || (!include_tests && atom == "test")) {
                atoms.push(atom.to_owned());
            }
            None
        });
    }
    fn search(
        conditions: &[String],
        atoms: &[String],
        values: &mut Vec<bool>,
        include_tests: bool,
    ) -> bool {
        let refuted = conditions.iter().any(|condition| {
            evaluate_cfg(condition, |atom| {
                if !include_tests && atom == "test" {
                    return Some(false);
                }
                let at = atoms.iter().position(|known| known == atom)?;
                values.get(at).copied()
            }) == Some(false)
        });
        if refuted {
            return false;
        }
        if values.len() == atoms.len() {
            return true;
        }
        [true, false].into_iter().any(|value| {
            values.push(value);
            let found = search(conditions, atoms, values, include_tests);
            values.pop();
            found
        })
    }
    search(conditions, &atoms, &mut Vec::new(), include_tests)
}

/// The files of `module` read from `path`, with conventional children in `dir`. With `every_build`,
/// a module behind a `cfg` is left out with its descendants. Unless `include_tests`, test-only
/// modules are left out, including files gated by inner attributes.
pub fn walk_module_files(
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
    include_tests: bool,
    parse: impl Fn(&Path, &str) -> Result<syn::File, String>,
) -> Result<Vec<ModuleFile>, String> {
    let mut files = FileCollector(Vec::new());
    ModuleWalk {
        every_build,
        include_tests,
    }
    .walk_tree(module, path, dir, parse, &mut |_| true, &mut files)?;
    files.0.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files.0)
}

/// Options controlling a crate module walk (`every_build` omits `cfg`-gated modules;
/// `include_tests` keeps `#[cfg(test)]` modules).
#[derive(Clone, Copy, Debug)]
pub struct ModuleWalk {
    pub every_build: bool,
    pub include_tests: bool,
}

trait WalkSink {
    fn inline(&mut self, _file: ModuleFile, _items: &[Item]) {}
    fn file(&mut self, file: ModuleFile, parsed: syn::File);
}

struct FileVisitor<F>(F);

impl<F: FnMut(&ModuleFile, syn::File)> WalkSink for FileVisitor<F> {
    fn file(&mut self, file: ModuleFile, parsed: syn::File) {
        (self.0)(&file, parsed);
    }
}

struct FileCollector(Vec<ModuleFile>);

impl WalkSink for FileCollector {
    fn file(&mut self, file: ModuleFile, _parsed: syn::File) {
        self.0.push(file);
    }
}

struct VariantCollector(Vec<ModuleVariant>);

impl WalkSink for VariantCollector {
    fn inline(&mut self, file: ModuleFile, items: &[Item]) {
        self.0.push(ModuleVariant {
            file,
            items: items.to_vec(),
        });
    }

    fn file(&mut self, file: ModuleFile, parsed: syn::File) {
        self.0.push(ModuleVariant {
            file,
            items: parsed.items,
        });
    }
}

impl ModuleWalk {
    /// Walk modules for which `descend(module)` is true, borrowing each parsed file's
    /// [`ModuleFile`] alongside its [`syn::File`] in one pass without collecting variants.
    pub fn visit_files(
        self,
        module: String,
        path: PathBuf,
        dir: PathBuf,
        parse: impl Fn(&Path, &str) -> Result<syn::File, String>,
        mut descend: impl FnMut(&str) -> bool,
        visit: impl FnMut(&ModuleFile, syn::File),
    ) -> Result<(), String> {
        if !descend(&module) {
            return Ok(());
        }
        let mut sink = FileVisitor(visit);
        self.walk_tree(module, path, dir, parse, &mut descend, &mut sink)
    }

    fn walk_tree<D: FnMut(&str) -> bool, S: WalkSink>(
        self,
        module: String,
        path: PathBuf,
        dir: PathBuf,
        parse: impl Fn(&Path, &str) -> Result<syn::File, String>,
        descend: &mut D,
        sink: &mut S,
    ) -> Result<(), String> {
        let mut walker = Walker {
            every_build: self.every_build,
            include_tests: self.include_tests,
            descend,
            sink,
            pending: vec![Pending {
                file: ModuleFile {
                    module,
                    path,
                    cfg: Vec::new(),
                    included: false,
                },
                dir,
                ancestors: Vec::new(),
                scope: IncludeScope::default(),
            }],
        };
        while let Some(Pending {
            mut file,
            dir,
            mut ancestors,
            scope,
        }) = walker.pending.pop()
        {
            // A target with only conditional paths need not have a conventional file. A literal
            // include has no such fallback: a fragment some build compiles must be readable, and
            // one no build can compile is never opened.
            if file.included && !may_compile(&file.cfg, self.include_tests) {
                continue;
            }
            if !file.included && !file.path.is_file() {
                continue;
            }
            let physical = file
                .path
                .canonicalize()
                .map_err(|e| format!("{}: {e}", file.path.display()))?;
            if ancestors.contains(&physical) {
                return Err(format!("{}: source inclusion cycle", file.path.display()));
            }
            ancestors.push(physical);
            let source = fs::read_to_string(&file.path)
                .map_err(|e| format!("{}: {e}", file.path.display()))?;
            let parsed = parse(&file.path, &source)?;
            if !walker.keep(&parsed.attrs) {
                continue;
            }
            file.cfg.extend(walker.conditions(&parsed.attrs));
            let here = file.path.parent().expect("a source file has a folder");
            walker.children(&file, &parsed.items, here, &dir, &ancestors, scope);
            walker.sink.file(file, parsed);
        }
        Ok(())
    }
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
    let mut variants = VariantCollector(Vec::new());
    ModuleWalk {
        every_build,
        include_tests,
    }
    .walk_tree(module, path, dir, parse, &mut |_| true, &mut variants)?;
    variants.0.sort_by(|a, b| a.file.path.cmp(&b.file.path));
    Ok(variants.0)
}

struct Walker<'a, D, S> {
    every_build: bool,
    include_tests: bool,
    descend: &'a mut D,
    sink: &'a mut S,
    pending: Vec<Pending>,
}

/// A file to read, the directory Rust uses for its conventional children, its physical
/// ancestors and the include shadowing scope reaching it.
struct Pending {
    file: ModuleFile,
    dir: PathBuf,
    ancestors: Vec<PathBuf>,
    scope: IncludeScope,
}

impl<D: FnMut(&str) -> bool, S: WalkSink> Walker<'_, D, S> {
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

    /// Look through inline modules and literal `include!`s as well as regular file declarations.
    fn children(
        &mut self,
        file: &ModuleFile,
        items: &[Item],
        here: &Path,
        dir: &Path,
        ancestors: &[PathBuf],
        scope: IncludeScope,
    ) {
        let mut cursor = scope.with_imports(items);
        for item in items {
            let scope = cursor.before(item);
            if let Some((relative, attrs)) = literal_include(item, scope.shadowed())
                && self.keep(attrs)
            {
                let mut included_file = file.clone();
                included_file.path = file
                    .path
                    .parent()
                    .expect("a source file has a folder")
                    .join(relative.value());
                included_file.cfg.extend(self.conditions(attrs));
                included_file.included = true;
                self.pending.push(Pending {
                    file: included_file,
                    dir: dir.to_owned(),
                    ancestors: ancestors.to_vec(),
                    scope,
                });
            }
            let Item::Mod(declared) = item else { continue };
            if !self.keep(&declared.attrs) {
                continue;
            }
            let name = declared.ident.to_string();
            let module = format!("{}::{name}", file.module);
            if !(self.descend)(&module) {
                continue;
            }
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
                    self.children(
                        &child,
                        inner,
                        &inner_dir,
                        &inner_dir,
                        ancestors,
                        scope.child(),
                    );
                    self.sink.inline(child, inner);
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
                        scope: scope.child(),
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

    fn walk(root: &Path, include_tests: bool) -> Result<Vec<String>, String> {
        let files = walk_module_files(
            "lower".into(),
            root.join("lib.rs"),
            root.to_owned(),
            false,
            include_tests,
            |path, text| syn::parse_file(text).map_err(|e| format!("{}: {e}", path.display())),
        )?;
        Ok(files
            .into_iter()
            .map(|file| file.path.strip_prefix(root).unwrap().display().to_string())
            .collect())
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

    #[test]
    #[ignore = "daily"]
    fn include_shadowing_matches_rustc_across_child_modules() {
        use crate::index::Index;

        // A missing file must never be read where the custom macro is in scope. The
        // standard includes in the imported macro's children must supply a real type.
        let lib = r#"
#[macro_export]
macro_rules! dummy { ($($t:tt)*) => {}; }
mod imported {
    use crate::dummy as include;
    include!("never-read.rs");
    pub mod inline { include!("gen.rs"); }
    pub mod file;
}
mod lexical {
    pub mod early { include!("gen.rs"); }
    pub mod early_file;
    macro_rules! include { ($($t:tt)*) => {}; }
    mod inline { include!("never-read.rs"); }
    mod file;
}
pub fn generated() {
    let _ = (
        imported::inline::Generated,
        imported::file::Generated,
        lexical::early::Generated,
        lexical::early_file::Generated,
    );
}
"#;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        for parent in ["imported", "lexical"] {
            fs::create_dir_all(src.join(parent)).unwrap();
        }
        let generated = [("gen.rs", "pub struct Generated;")];
        fs::write(src.join("gen.rs"), generated[0].1).unwrap();
        fs::write(src.join("imported/gen.rs"), generated[0].1).unwrap();
        fs::write(src.join("lexical/gen.rs"), generated[0].1).unwrap();
        fs::write(src.join("lexical/file.rs"), r#"include!("never-read.rs");"#).unwrap();

        for out_dir in [false, true] {
            let write = |path: &str, text: &str| {
                let text = if out_dir {
                    text.replace(r#""gen.rs""#, r#"concat!(env!("OUT_DIR"), "/gen.rs")"#)
                } else {
                    text.to_owned()
                };
                fs::write(src.join(path), text).unwrap();
            };
            write("lib.rs", lib);
            write("imported/file.rs", r#"include!("gen.rs");"#);
            write("lexical/early_file.rs", r#"include!("gen.rs");"#);
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let output = std::process::Command::new(rustc)
                .args(["--crate-type=lib", "--edition=2024", "--emit=metadata"])
                .arg(src.join("lib.rs"))
                .arg("--out-dir")
                .arg(dir.path())
                .env("OUT_DIR", &src)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );

            if out_dir {
                let mut index = Index::new(dir.path(), &generated, &[("k", "src")]);
                for (parent, child, expanded) in [
                    ("imported", "inline", true),
                    ("imported", "file", true),
                    ("lexical", "early", true),
                    ("lexical", "early_file", true),
                    ("lexical", "inline", false),
                    ("lexical", "file", false),
                ] {
                    let module = index.module("k", &[parent.into(), child.into()]).unwrap();
                    assert_eq!(
                        index.resolve(module, "Generated").is_ok(),
                        expanded,
                        "{parent}::{child}"
                    );
                }
            } else {
                let files = walk_module_files(
                    "k".into(),
                    src.join("lib.rs"),
                    src.clone(),
                    false,
                    false,
                    |_, text| syn::parse_file(text).map_err(|err| err.to_string()),
                )
                .unwrap();
                let mut included: Vec<_> = files
                    .iter()
                    .filter(|file| file.included)
                    .map(|file| (file.module.as_str(), file.path.strip_prefix(&src).unwrap()))
                    .collect();
                included.sort_unstable();
                assert_eq!(
                    included,
                    [
                        ("k::imported::file", Path::new("imported/gen.rs")),
                        ("k::imported::inline", Path::new("gen.rs")),
                        ("k::lexical::early", Path::new("gen.rs")),
                        ("k::lexical::early_file", Path::new("lexical/gen.rs")),
                    ]
                );
            }
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

    #[test]
    fn visit_module_files_prunes_unmatched_subtrees_in_one_pass() {
        let root = tempfile::tempdir().unwrap();
        let put = |rel: &str, text: &str| {
            let path = root.path().join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, text).unwrap();
        };
        put(
            "lib.rs",
            "mod wanted;\nmod skipped;\nmod inline_parent { mod nested; mod ignored; }\n",
        );
        put(
            "wanted.rs",
            "include!(\"wanted_part.rs\");\nfn from_wanted() {}\n",
        );
        put("wanted_part.rs", "fn from_include() {}\n");
        put("skipped.rs", "fn (\n");
        put("inline_parent/nested.rs", "fn from_nested() {}\n");
        put("inline_parent/ignored.rs", "fn (\n");

        let allowed = [
            "lower",
            "lower::wanted",
            "lower::inline_parent",
            "lower::inline_parent::nested",
        ];
        let parsed_count = std::cell::Cell::new(0usize);
        let mut visited = Vec::new();
        ModuleWalk {
            every_build: false,
            include_tests: false,
        }
        .visit_files(
            "lower".into(),
            root.path().join("lib.rs"),
            root.path().to_owned(),
            |path, text| {
                parsed_count.set(parsed_count.get() + 1);
                syn::parse_file(text).map_err(|e| format!("{}: {e}", path.display()))
            },
            |module| allowed.contains(&module),
            |file, parsed| {
                let rel = file
                    .path
                    .strip_prefix(root.path())
                    .unwrap()
                    .display()
                    .to_string();
                let fns: Vec<_> = parsed
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::Fn(func) => Some(func.sig.ident.to_string()),
                        _ => None,
                    })
                    .collect();
                visited.push((file.module.clone(), rel, fns));
            },
        )
        .unwrap();
        visited.sort();
        assert_eq!(parsed_count.get(), 4);
        assert_eq!(
            visited,
            [
                ("lower".into(), "lib.rs".into(), vec![]),
                (
                    "lower::inline_parent::nested".into(),
                    "inline_parent/nested.rs".into(),
                    vec!["from_nested".into()],
                ),
                (
                    "lower::wanted".into(),
                    "wanted.rs".into(),
                    vec!["from_wanted".into()],
                ),
                (
                    "lower::wanted".into(),
                    "wanted_part.rs".into(),
                    vec!["from_include".into()],
                ),
            ]
        );
    }

    #[test]
    fn walk_module_variants_returns_file_and_nested_inline_items() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("lib.rs"),
            r#"
fn root_fn() {}
mod outer {
    fn outer_fn() {}
    mod inner {
        fn inner_fn() {}
    }
}
mod file_child;
"#,
        )
        .unwrap();
        fs::write(root.path().join("file_child.rs"), "fn child_fn() {}\n").unwrap();

        let variants = walk_module_variants(
            "lower".into(),
            root.path().join("lib.rs"),
            root.path().to_owned(),
            false,
            false,
            |path, text| syn::parse_file(text).map_err(|e| format!("{}: {e}", path.display())),
        )
        .unwrap();

        let mut by_module: Vec<_> = variants
            .iter()
            .map(|variant| {
                let fns: Vec<_> = variant
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::Fn(func) => Some(func.sig.ident.to_string()),
                        _ => None,
                    })
                    .collect();
                (variant.file.module.clone(), fns)
            })
            .collect();
        by_module.sort();
        assert_eq!(
            by_module,
            [
                ("lower".into(), vec!["root_fn".into()]),
                ("lower::file_child".into(), vec!["child_fn".into()]),
                ("lower::outer".into(), vec!["outer_fn".into()]),
                ("lower::outer::inner".into(), vec!["inner_fn".into()]),
            ]
        );
    }
}
