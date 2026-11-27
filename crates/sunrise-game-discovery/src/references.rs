//! What a test target's functions reach, by name resolution over its source: the check that keeps prepared test
//! data behind one boundary (`tools/ci/check_ignored_test_rows.py`) reads this.
//!
//! The check seeds three things: the one required acquisition function (the boundary), a string literal read in
//! full (an environment variable) and a string literal that contains a checkout path. A function that spells one
//! of the literals, or that is the boundary, is a seed. Every function, constant and `macro_rules!` of the target
//! that refers to a seed, directly or through others, reaches it. Nothing is listed by hand: a wrapper is a wrapper
//! because of what it refers to.
//!
//! Approximated within one crate: items of the module (including those of `include!`d
//! fragments and inline modules); `use` items, plain, renamed (`as`), grouped, nested and `self`, in any module,
//! followed through chains of re-exports; the `use` items of a function body bind for that function alone and
//! win over its module's; glob imports of the crate's own modules (`use super::*`); the `crate`, `self`, `super`
//! and `Self` prefixes; paths through modules; `type` aliases of a path, followed to their type; associated
//! functions of `impl` blocks and of trait default bodies (`Type::function`, `Self::function`,
//! `<Type as Trait>::function`); methods (functions with a `self` receiver) on the impl's own type
//! (`self.method()`) or on a struct literal receiver (`Type { ... }.method()`); constants and statics;
//! `macro_rules!` macros by name, with the paths in their bodies and in the tokens of every macro call;
//! functions passed as values.
//!
//! Visibility narrows what a name can mean where nothing resolves it: a private item is visible in its module and
//! below, `pub(super)` in the parent's subtree, `pub(crate)`, `pub` and `pub(in ..)` in the crate; a `use` that
//! re-exports an item widens it to its own visibility; a method is as visible as its type. A reference that
//! does not resolve is compared only with items visible from the module that spells it. Test-only items
//! are excluded from those comparisons and unknown-receiver method lists outside test contexts.
//!
//! Not resolved, and refused when a seed is involved: a path that enters the crate's own names (a module, a trait,
//! a glob that may come from outside) and does not end at a definition, spelled like a visible function that
//! reaches a seed (a function from a glob of another crate, a module that does not hold it, a generated or
//! trait-provided associated function); a path on a concrete type that lacks the function, when a visible trait's
//! default body of that name reaches a seed; and a method call on an unknown receiver whose name is shared by a
//! visible method that reaches a seed and one that does not. Those come out as `unresolved` records, never as
//! silence. A path that leaves the crate by its first segment (`std::`, another crate's name) is not followed: a
//! fixture helper in another crate is that crate's own target. Methods on other receivers are matched by name
//! among the visible methods of the crate's impl blocks: a call reaches a seed only when every one of that name
//! does. Associated functions without a receiver are never reached by a method call.
//!
//! The records, one JSON object per line:
//! * `reaches`: `seed` (`boundary` or `legacy`), `package`, `target`, `path`, `line`, `name` (the module path and
//!   name, as the test inventory writes a test's), `test`, `chain` (the names from this function to the seed).
//! * `unresolved`: the same identity, `seed` the kind of function the reference is spelled like, and `reference`.
//!
//! Not control-flow analysis: whether a call runs is not asked, only whether the source names it.

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use syn::{
    Attribute, Block, Expr, ExprMethodCall, ExprPath, FnArg, ImplItem, Item, ItemUse, LitStr,
    Macro, Meta, PatIdent, Signature, Token, TraitItem, Type, UseTree, Visibility,
    punctuated::Punctuated, visit::Visit,
};

/// What a search starts from.
#[derive(Clone, Default)]
pub struct Seeds {
    /// The file (a path suffix) and name of the required acquisition function.
    pub boundary_file: String,
    pub boundary_name: String,
    /// String literals that are an acquisition when spelled in full (an environment variable).
    pub exact_literals: Vec<String>,
    /// String literals that are an acquisition when they contain this (a path into the checkout).
    pub literal_fragments: Vec<String>,
}

impl Seeds {
    /// Whether `source` spells anything a search starts from. A target none of whose files does reaches nothing.
    pub fn mentions(&self, source: &str) -> bool {
        source.contains(&self.boundary_name)
            || self
                .exact_literals
                .iter()
                .any(|exact| source.contains(exact.as_str()))
            || self
                .literal_fragments
                .iter()
                .any(|part| source.contains(part.as_str()))
    }
}

/// Whether the attributes make a function a test, as the test inventory counts one.
pub fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test"))
}

/// Whether a `cfg` requires `test`, not just mentions it (such as `not(test)` or `any(test, unix)`).
fn is_cfg_test(attrs: &[Attribute]) -> bool {
    fn requires_test(meta: &Meta) -> bool {
        match meta {
            Meta::Path(path) => path.is_ident("test"),
            Meta::List(list) if list.path.is_ident("all") => list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .is_ok_and(|parts| parts.iter().any(requires_test)),
            _ => false,
        }
    }

    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<Meta>()
                .is_ok_and(|meta| requires_test(&meta))
    })
}

const CRATE: usize = 0;
const THIS: usize = 1;
const SUPER: usize = 2;
const SELF_TYPE: usize = 3;
const NONE: usize = usize::MAX;

#[derive(Default)]
struct Interner {
    ids: HashMap<String, usize>,
    names: Vec<String>,
}

impl Interner {
    fn id(&mut self, name: &str) -> usize {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        self.names.push(name.to_owned());
        self.ids.insert(name.to_owned(), self.names.len() - 1);
        self.names.len() - 1
    }

    fn name(&self, id: usize) -> &str {
        &self.names[id]
    }
}

/// A reference as a body spells it.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Raw {
    Path(Vec<String>),
    /// A method call; whether its receiver is `self`.
    Method(String, bool),
    Macro(String),
    /// `<Type as Trait>::member`: the type's path then the member, and the trait's path as written.
    Qualified(Vec<String>, Vec<String>),
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Path(Vec<usize>),
    Method(usize),
    SelfMethod(usize),
    Macro(usize),
    Qualified(Vec<usize>, Vec<usize>),
}

/// What one body refers to, as written.
struct Body {
    refs: HashSet<Raw>,
    locals: HashSet<String>,
    literal: bool,
    /// `use` items inside the body: bound for this body only.
    uses: Vec<UseTree>,
}

struct Scan<'a> {
    seeds: &'a Seeds,
    body: Body,
}

impl<'a> Scan<'a> {
    fn new(seeds: &'a Seeds) -> Self {
        Self {
            seeds,
            body: Body {
                refs: HashSet::new(),
                locals: HashSet::new(),
                literal: false,
                uses: Vec::new(),
            },
        }
    }

    fn literal(&mut self, value: &str) {
        self.body.literal |= self.seeds.exact_literals.iter().any(|exact| exact == value)
            || self
                .seeds
                .literal_fragments
                .iter()
                .any(|part| value.contains(part.as_str()));
    }

    /// The names and literals of a macro's tokens, which no syntax tree holds: every chain of identifiers is a
    /// path (a call, or a value), `name!` a macro call, `.name(` a method call.
    fn tokens(&mut self, stream: TokenStream) {
        let trees: Vec<TokenTree> = stream.into_iter().collect();
        let mut at = 0;
        while at < trees.len() {
            match &trees[at] {
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Literal(literal) => {
                    let text = literal.to_string();
                    let inner = text
                        .strip_prefix('"')
                        .and_then(|rest| rest.strip_suffix('"'))
                        .unwrap_or(&text);
                    self.literal(inner);
                }
                TokenTree::Punct(_) => {}
                TokenTree::Ident(first) => {
                    let after_dot = at > 0
                        && matches!(&trees[at - 1], TokenTree::Punct(p) if p.as_char() == '.');
                    let mut segments = vec![first.to_string()];
                    let mut end = at + 1;
                    while let (
                        Some(TokenTree::Punct(a)),
                        Some(TokenTree::Punct(b)),
                        Some(TokenTree::Ident(next)),
                    ) = (trees.get(end), trees.get(end + 1), trees.get(end + 2))
                    {
                        if a.as_char() != ':' || b.as_char() != ':' {
                            break;
                        }
                        segments.push(next.to_string());
                        end += 3;
                    }
                    let called = matches!(trees.get(end), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis);
                    let bang = matches!(trees.get(end), Some(TokenTree::Punct(p)) if p.as_char() == '!')
                        && matches!(trees.get(end + 1), Some(TokenTree::Group(_)));
                    let field_label = segments.len() == 1
                        && matches!(trees.get(end), Some(TokenTree::Punct(p)) if p.as_char() == ':')
                        && !matches!(trees.get(end + 1), Some(TokenTree::Punct(p)) if p.as_char() == ':');
                    if after_dot {
                        if called && segments.len() == 1 {
                            self.body
                                .refs
                                .insert(Raw::Method(segments.remove(0), false));
                        }
                    } else if bang {
                        if let Some(last) = segments.pop() {
                            self.body.refs.insert(Raw::Macro(last));
                        }
                    } else if !field_label {
                        self.body.refs.insert(Raw::Path(segments));
                    }
                    at = end;
                    continue;
                }
            }
            at += 1;
        }
    }
}

impl<'ast> Visit<'ast> for Scan<'_> {
    // `#[ignore = "..."]` and doc comments are not what a body reads.
    fn visit_attribute(&mut self, _: &'ast Attribute) {}

    fn visit_expr_path(&mut self, node: &'ast ExprPath) {
        let segments: Vec<String> = node
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        match node
            .qself
            .as_ref()
            .map(|qself| (qself.position, &*qself.ty))
        {
            // `<Fixture as Trait>::member` is `Fixture`'s member, or the trait's own default of it.
            Some((position, Type::Path(ty))) => {
                let mut typed: Vec<String> = ty
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();
                typed.extend(segments.last().cloned());
                if position == 0 {
                    self.body.refs.insert(Raw::Path(typed));
                } else {
                    self.body.refs.insert(Raw::Qualified(typed, segments));
                }
            }
            Some((position, _)) => {
                if position > 0 {
                    self.body.refs.insert(Raw::Path(segments));
                }
            }
            None if segments != ["self"] => {
                self.body.refs.insert(Raw::Path(segments));
            }
            None => {}
        }
        syn::visit::visit_expr_path(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        let mut receiver = &*node.receiver;
        loop {
            receiver = match receiver {
                Expr::Paren(paren) => &paren.expr,
                Expr::Group(group) => &group.expr,
                Expr::Reference(reference) => &reference.expr,
                _ => break,
            };
        }
        let method = node.method.to_string();
        if let Expr::Struct(literal) = receiver
            && literal.qself.is_none()
        {
            let mut path: Vec<_> = literal
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            path.push(method);
            self.body.refs.insert(Raw::Path(path));
        } else {
            let on_self =
                matches!(receiver, Expr::Path(p) if p.qself.is_none() && p.path.is_ident("self"));
            self.body.refs.insert(Raw::Method(method, on_self));
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast Macro) {
        if let Some(last) = node.path.segments.last() {
            self.body.refs.insert(Raw::Macro(last.ident.to_string()));
        }
        self.tokens(node.tokens.clone());
    }

    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        self.body.uses.push(node.tree.clone());
    }

    fn visit_pat_ident(&mut self, node: &'ast PatIdent) {
        self.body.locals.insert(node.ident.to_string());
        syn::visit::visit_pat_ident(self, node);
    }

    fn visit_lit_str(&mut self, node: &'ast LitStr) {
        self.literal(&node.value());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Function,
    Constant,
    Macro,
}

/// What `use` items bind: a module's, or those of one function body (which no other function sees).
#[derive(Default)]
struct Scope {
    bindings: Vec<Binding>,
    globs: Vec<Glob>,
}

impl Scope {
    fn is_empty(&self) -> bool {
        self.bindings.is_empty() && self.globs.is_empty()
    }
}

struct Binding {
    name: usize,
    path: Vec<usize>,
    /// The module whose subtree the `use` is visible in (what a re-export widens an item to).
    visible: usize,
}

struct Glob {
    path: Vec<usize>,
    visible: usize,
}

struct Node {
    name: usize,
    module: usize,
    file: usize,
    line: usize,
    kind: Kind,
    test: bool,
    cfg_test: bool,
    imp: Option<usize>,
    /// A function with a `self` receiver, which a method call can reach.
    receiver: bool,
    /// The module whose subtree the item is visible in, before re-exports and its owning type are considered.
    visible: usize,
    refs: Vec<usize>,
    locals: Vec<usize>,
    /// The `use` items of the body itself.
    uses: Scope,
    literal: bool,
}

/// The identity of a node before it is added.
struct Head {
    module: usize,
    file: usize,
    line: usize,
    name: String,
    kind: Kind,
    test: bool,
    cfg_test: bool,
    imp: Option<usize>,
    receiver: bool,
    visible: usize,
}

/// An `impl` block, or a trait with default bodies, whose functions attach to the type it names.
struct Imp {
    module: usize,
    owner: Option<usize>,
    self_path: Option<Vec<usize>>,
    nodes: Vec<usize>,
    /// A trait definition, whose functions are default bodies any implementor may be called through.
    trait_def: bool,
    cfg_test: bool,
}

/// A struct, enum, union, trait or type alias.
#[derive(Default)]
struct TypeData {
    module: usize,
    cfg_test: bool,
    /// The associated functions, by name.
    assoc: HashMap<usize, Vec<usize>>,
    is_trait: bool,
    /// A `type` alias: the module it is written in and the path of its target.
    alias: Option<(usize, Vec<usize>)>,
    visible: usize,
}

#[derive(Default)]
struct Module {
    path: String,
    parent: Option<usize>,
    cfg_test: bool,
    values: HashMap<usize, Vec<usize>>,
    types: HashMap<usize, usize>,
    children: HashMap<usize, usize>,
    scope: Scope,
}

/// What a path that does not end at a definition was looking for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Lookup {
    /// A name among the items of a module.
    Module,
    /// An associated function of a trait, or of a `Self` that is not known: any `impl` may supply it.
    Associated,
    /// An associated function a concrete type does not define: only the default body of a trait can supply it.
    Provided,
}

/// Where a path leads.
#[derive(Clone)]
enum Found {
    Nodes(Vec<usize>),
    Module(usize),
    Type(usize),
    /// Out of the crate by its first segment, or a name nothing in the crate could supply.
    Foreign,
    /// Into the crate's own names and not to a definition: the last segment asked for, and where it was looked up.
    Unknown(usize, Lookup),
    /// Not bound here; whether a glob that may come from outside could have supplied it.
    Missing(bool),
}

#[derive(Clone, Default)]
struct Outcome {
    nodes: Vec<usize>,
    unknown: Option<(usize, Lookup)>,
}

/// (the module, or a function with its own `use` items, the path is read in; what `Self` is; the reference).
type Memo = HashMap<(usize, Option<usize>, usize), Outcome>;

/// The methods of one name a caller can see, by (module, test context, name), each list once.
#[derive(Default)]
struct Lists {
    all: Vec<Vec<usize>>,
    ids: HashMap<(usize, bool, usize), usize>,
}

/// A target's source, added file by file, then searched.
pub struct Analysis {
    seeds: Seeds,
    names: Interner,
    keys: Vec<Key>,
    key_ids: HashMap<Key, usize>,
    modules: Vec<Module>,
    module_ids: HashMap<String, usize>,
    types: Vec<TypeData>,
    nodes: Vec<Node>,
    imps: Vec<Imp>,
    files: Vec<String>,
    macros: HashMap<usize, Vec<usize>>,
    /// The functions with a `self` receiver, by name: what a method call may reach.
    methods: HashMap<usize, Vec<usize>>,
}

struct Edges {
    direct: Vec<Vec<usize>>,
    /// Per node, the lists (of `Lists`) of the methods its calls may reach.
    methods: Vec<Vec<usize>>,
}

/// The names `tree` binds, into `scope`; `visible` is the module whose subtree sees the `use`.
fn collect_uses(
    names: &mut Interner,
    scope: &mut Scope,
    visible: usize,
    tree: &UseTree,
    prefix: &mut Vec<usize>,
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(names.id(&path.ident.to_string()));
            collect_uses(names, scope, visible, &path.tree, prefix);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let ident = name.ident.to_string();
            if ident == "self" {
                if let Some(&last) = prefix.last() {
                    scope.bindings.push(Binding {
                        name: last,
                        path: prefix.clone(),
                        visible,
                    });
                }
            } else {
                let id = names.id(&ident);
                let mut path = prefix.clone();
                path.push(id);
                scope.bindings.push(Binding {
                    name: id,
                    path,
                    visible,
                });
            }
        }
        UseTree::Rename(rename) => {
            let alias = rename.rename.to_string();
            if alias == "_" {
                return;
            }
            let alias = names.id(&alias);
            let mut path = prefix.clone();
            let target = rename.ident.to_string();
            if target != "self" {
                path.push(names.id(&target));
            }
            scope.bindings.push(Binding {
                name: alias,
                path,
                visible,
            });
        }
        UseTree::Glob(_) => scope.globs.push(Glob {
            path: prefix.clone(),
            visible,
        }),
        UseTree::Group(group) => {
            for item in &group.items {
                collect_uses(names, scope, visible, item, prefix);
            }
        }
    }
}

impl Analysis {
    pub fn new(seeds: Seeds) -> Self {
        let mut names = Interner::default();
        for fixed in ["crate", "self", "super", "Self"] {
            names.id(fixed);
        }
        let mut analysis = Self {
            seeds,
            names,
            keys: Vec::new(),
            key_ids: HashMap::new(),
            modules: vec![Module::default()],
            module_ids: HashMap::new(),
            types: Vec::new(),
            nodes: Vec::new(),
            imps: Vec::new(),
            files: Vec::new(),
            macros: HashMap::new(),
            methods: HashMap::new(),
        };
        analysis.module_ids.insert(String::new(), 0);
        analysis
    }

    /// Adds one source file as a part of `module` (`a::b`, empty for the crate root); `file` is how its records
    /// name it.
    pub fn add(&mut self, module: &str, file: &str, source: &str) -> Result<(), String> {
        let parsed = syn::parse_file(source).map_err(|e| format!("{file}: {e}"))?;
        let module = self.module(module);
        self.files.push(file.to_owned());
        self.items(module, self.files.len() - 1, &parsed.items);
        Ok(())
    }

    fn module(&mut self, path: &str) -> usize {
        if let Some(&id) = self.module_ids.get(path) {
            return id;
        }
        let (parent_path, name) = path.rsplit_once("::").unwrap_or(("", path));
        let parent = self.module(parent_path);
        let id = self.modules.len();
        self.modules.push(Module {
            path: path.to_owned(),
            parent: Some(parent),
            ..Module::default()
        });
        self.module_ids.insert(path.to_owned(), id);
        let name = self.names.id(name);
        self.modules[parent].children.insert(name, id);
        id
    }

    fn key(&mut self, raw: Raw) -> usize {
        let key = match raw {
            Raw::Path(segments) => Key::Path(self.ids(&segments)),
            Raw::Qualified(typed, traited) => Key::Qualified(self.ids(&typed), self.ids(&traited)),
            Raw::Method(name, on_self) => {
                let id = self.names.id(&name);
                if on_self {
                    Key::SelfMethod(id)
                } else {
                    Key::Method(id)
                }
            }
            Raw::Macro(name) => Key::Macro(self.names.id(&name)),
        };
        if let Some(&id) = self.key_ids.get(&key) {
            return id;
        }
        self.keys.push(key.clone());
        self.key_ids.insert(key, self.keys.len() - 1);
        self.keys.len() - 1
    }

    fn ids(&mut self, segments: &[String]) -> Vec<usize> {
        segments
            .iter()
            .map(|segment| self.names.id(segment))
            .collect()
    }

    /// The module whose subtree sees an item written in `module` with `visibility`: a private item, its module;
    /// `pub(super)`, the parent's; `pub`, `pub(crate)` and `pub(in ..)`, the crate.
    fn visibility(&self, module: usize, visibility: &Visibility) -> usize {
        match visibility {
            Visibility::Inherited => module,
            Visibility::Restricted(restricted) if restricted.in_token.is_none() => {
                if restricted.path.is_ident("super") {
                    self.modules[module].parent.unwrap_or(0)
                } else if restricted.path.is_ident("self") {
                    module
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn node(&mut self, head: Head, body: Body) -> usize {
        let mut uses = Scope::default();
        for tree in &body.uses {
            collect_uses(
                &mut self.names,
                &mut uses,
                head.module,
                tree,
                &mut Vec::new(),
            );
        }
        let name = self.names.id(&head.name);
        let mut refs = Vec::with_capacity(body.refs.len());
        for raw in body.refs {
            refs.push(self.key(raw));
        }
        refs.sort_unstable();
        let mut locals = Vec::with_capacity(body.locals.len());
        for local in &body.locals {
            locals.push(self.names.id(local));
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            name,
            module: head.module,
            file: head.file,
            line: head.line,
            kind: head.kind,
            test: head.test,
            cfg_test: head.test
                || head.cfg_test
                || head.imp.is_some_and(|imp| self.imps[imp].cfg_test),
            imp: head.imp,
            receiver: head.receiver,
            visible: head.visible,
            refs,
            locals,
            uses,
            literal: body.literal,
        });
        if head.kind == Kind::Macro {
            self.macros.entry(name).or_default().push(id);
        }
        id
    }

    fn function(
        &mut self,
        (module, file, imp): (usize, usize, Option<usize>),
        visible: usize,
        attributes: &[Attribute],
        signature: &Signature,
        block: &Block,
    ) -> usize {
        let mut scan = Scan::new(&self.seeds);
        for input in &signature.inputs {
            if let FnArg::Typed(typed) = input {
                scan.visit_pat(&typed.pat);
            }
        }
        scan.visit_block(block);
        let body = scan.body;
        let head = Head {
            module,
            file,
            line: signature.ident.span().start().line,
            name: signature.ident.to_string(),
            kind: Kind::Function,
            test: is_test(attributes),
            cfg_test: is_cfg_test(attributes),
            imp,
            receiver: signature.receiver().is_some(),
            visible,
        };
        self.node(head, body)
    }

    fn define(
        &mut self,
        module: usize,
        ident: &syn::Ident,
        visible: usize,
        attrs: &[Attribute],
    ) -> usize {
        let name = self.names.id(&ident.to_string());
        let cfg_test = is_cfg_test(attrs);
        if let Some(&id) = self.modules[module].types.get(&name) {
            // A shared name is test-only only if all definitions require tests.
            self.types[id].cfg_test &= cfg_test;
            return id;
        }
        self.types.push(TypeData {
            module,
            cfg_test,
            visible,
            ..TypeData::default()
        });
        let id = self.types.len() - 1;
        self.modules[module].types.insert(name, id);
        id
    }

    fn items(&mut self, module: usize, file: usize, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(function) => {
                    let visible = self.visibility(module, &function.vis);
                    let node = self.function(
                        (module, file, None),
                        visible,
                        &function.attrs,
                        &function.sig,
                        &function.block,
                    );
                    let name = self.nodes[node].name;
                    self.modules[module]
                        .values
                        .entry(name)
                        .or_default()
                        .push(node);
                }
                Item::Const(constant) => {
                    let mut scan = Scan::new(&self.seeds);
                    scan.visit_expr(&constant.expr);
                    let body = scan.body;
                    let visible = self.visibility(module, &constant.vis);
                    self.value(
                        module,
                        file,
                        &constant.ident,
                        (visible, is_cfg_test(&constant.attrs)),
                        body,
                    );
                }
                Item::Static(constant) => {
                    let mut scan = Scan::new(&self.seeds);
                    scan.visit_expr(&constant.expr);
                    let body = scan.body;
                    let visible = self.visibility(module, &constant.vis);
                    self.value(
                        module,
                        file,
                        &constant.ident,
                        (visible, is_cfg_test(&constant.attrs)),
                        body,
                    );
                }
                Item::Macro(defined) if defined.mac.path.is_ident("macro_rules") => {
                    if let Some(ident) = &defined.ident {
                        let mut scan = Scan::new(&self.seeds);
                        scan.tokens(defined.mac.tokens.clone());
                        let body = scan.body;
                        let head = Head {
                            module,
                            file,
                            line: ident.span().start().line,
                            name: ident.to_string(),
                            kind: Kind::Macro,
                            test: false,
                            cfg_test: is_cfg_test(&defined.attrs),
                            imp: None,
                            receiver: false,
                            visible: 0,
                        };
                        self.node(head, body);
                    }
                }
                Item::Use(used) => {
                    let visible = self.visibility(module, &used.vis);
                    collect_uses(
                        &mut self.names,
                        &mut self.modules[module].scope,
                        visible,
                        &used.tree,
                        &mut Vec::new(),
                    );
                }
                Item::Mod(declared) => {
                    let path = if self.modules[module].path.is_empty() {
                        declared.ident.to_string()
                    } else {
                        format!("{}::{}", self.modules[module].path, declared.ident)
                    };
                    let child = self.module(&path);
                    self.modules[child].cfg_test |= is_cfg_test(&declared.attrs);
                    if let Some((_, inner)) = &declared.content {
                        self.items(child, file, inner);
                    }
                }
                Item::Struct(defined) => {
                    let visible = self.visibility(module, &defined.vis);
                    self.define(module, &defined.ident, visible, &defined.attrs);
                }
                Item::Enum(defined) => {
                    let visible = self.visibility(module, &defined.vis);
                    self.define(module, &defined.ident, visible, &defined.attrs);
                }
                Item::Union(defined) => {
                    let visible = self.visibility(module, &defined.vis);
                    self.define(module, &defined.ident, visible, &defined.attrs);
                }
                Item::Type(defined) => {
                    let visible = self.visibility(module, &defined.vis);
                    let id = self.define(module, &defined.ident, visible, &defined.attrs);
                    if let Type::Path(target) = &*defined.ty
                        && target.qself.is_none()
                    {
                        let path = target
                            .path
                            .segments
                            .iter()
                            .map(|segment| self.names.id(&segment.ident.to_string()))
                            .collect::<Vec<_>>();
                        self.types[id].alias = Some((module, path));
                    }
                }
                Item::Trait(defined) => {
                    let visible = self.visibility(module, &defined.vis);
                    let id = self.define(module, &defined.ident, visible, &defined.attrs);
                    self.types[id].is_trait = true;
                    let owner = self.names.id(&defined.ident.to_string());
                    let imp = self.imps.len();
                    self.imps.push(Imp {
                        module,
                        owner: Some(owner),
                        self_path: Some(vec![owner]),
                        nodes: Vec::new(),
                        trait_def: true,
                        cfg_test: is_cfg_test(&defined.attrs),
                    });
                    for entry in &defined.items {
                        if let TraitItem::Fn(function) = entry
                            && let Some(block) = &function.default
                        {
                            // A trait's functions are as visible as the trait (bounded by it, in `finish`).
                            let node = self.function(
                                (module, file, Some(imp)),
                                0,
                                &function.attrs,
                                &function.sig,
                                block,
                            );
                            self.method(imp, node);
                        }
                    }
                }
                Item::Impl(implementation) => {
                    let self_path = match &*implementation.self_ty {
                        Type::Path(path) => Some(
                            path.path
                                .segments
                                .iter()
                                .map(|segment| self.names.id(&segment.ident.to_string()))
                                .collect::<Vec<_>>(),
                        ),
                        _ => None,
                    };
                    let imp = self.imps.len();
                    self.imps.push(Imp {
                        module,
                        owner: self_path.as_ref().and_then(|path| path.last().copied()),
                        self_path,
                        nodes: Vec::new(),
                        trait_def: false,
                        cfg_test: is_cfg_test(&implementation.attrs),
                    });
                    for entry in &implementation.items {
                        if let ImplItem::Fn(function) = entry {
                            // The functions of a trait impl take the trait's visibility, not their own.
                            let visible = if implementation.trait_.is_some() {
                                0
                            } else {
                                self.visibility(module, &function.vis)
                            };
                            let node = self.function(
                                (module, file, Some(imp)),
                                visible,
                                &function.attrs,
                                &function.sig,
                                &function.block,
                            );
                            self.method(imp, node);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn value(
        &mut self,
        module: usize,
        file: usize,
        ident: &syn::Ident,
        (visible, cfg_test): (usize, bool),
        body: Body,
    ) {
        let head = Head {
            module,
            file,
            line: ident.span().start().line,
            name: ident.to_string(),
            kind: Kind::Constant,
            test: false,
            cfg_test,
            imp: None,
            receiver: false,
            visible,
        };
        let node = self.node(head, body);
        let name = self.nodes[node].name;
        self.modules[module]
            .values
            .entry(name)
            .or_default()
            .push(node);
    }

    /// Attaches `node` to its `impl` block; only a function with a receiver can be reached by a method call.
    fn method(&mut self, imp: usize, node: usize) {
        self.imps[imp].nodes.push(node);
        if self.nodes[node].receiver {
            self.methods
                .entry(self.nodes[node].name)
                .or_default()
                .push(node);
        }
    }

    fn lookup(&self, module: usize, name: usize, seen: &mut Vec<(usize, usize)>) -> Found {
        if seen.contains(&(module, name)) {
            return Found::Missing(false);
        }
        seen.push((module, name));
        let found = self.lookup_here(module, name, seen);
        seen.pop();
        found
    }

    fn lookup_here(&self, module: usize, name: usize, seen: &mut Vec<(usize, usize)>) -> Found {
        let data = &self.modules[module];
        if let Some(nodes) = data.values.get(&name) {
            return Found::Nodes(nodes.clone());
        }
        if let Some(&id) = data.types.get(&name) {
            return Found::Type(id);
        }
        if let Some(&id) = data.children.get(&name) {
            return Found::Module(id);
        }
        if let Some(binding) = data.scope.bindings.iter().find(|b| b.name == name) {
            return self.resolve(module, None, None, &binding.path, seen);
        }
        let mut outside = false;
        for glob in &data.scope.globs {
            match self.resolve(module, None, None, &glob.path, seen) {
                Found::Module(target) => match self.lookup(target, name, seen) {
                    Found::Missing(inner) => outside |= inner,
                    found => return found,
                },
                _ => outside = true,
            }
        }
        Found::Missing(outside)
    }

    /// `name` as a function body spells it in `module`: its own `use` items first, then the module's.
    fn lookup_in_body(
        &self,
        uses: Option<&Scope>,
        module: usize,
        name: usize,
        seen: &mut Vec<(usize, usize)>,
    ) -> Found {
        let Some(uses) = uses else {
            return self.lookup(module, name, seen);
        };
        let mut nodes = Vec::new();
        let mut other = None;
        for binding in uses.bindings.iter().filter(|b| b.name == name) {
            match self.resolve(module, None, None, &binding.path, seen) {
                Found::Nodes(found) => nodes.extend(found),
                found => other = other.or(Some(found)),
            }
        }
        if !nodes.is_empty() {
            return Found::Nodes(nodes);
        }
        if let Some(found) = other {
            return found;
        }
        let mut outside = false;
        for glob in &uses.globs {
            match self.resolve(module, None, None, &glob.path, seen) {
                Found::Module(target) => match self.lookup(target, name, seen) {
                    Found::Missing(inner) => outside |= inner,
                    found => return found,
                },
                _ => outside = true,
            }
        }
        match self.lookup(module, name, seen) {
            Found::Missing(inner) => Found::Missing(inner || outside),
            found => found,
        }
    }

    /// A type alias is the type it names (a cycle of aliases leads nowhere).
    fn follow(&self, found: Found, seen: &mut Vec<(usize, usize)>) -> Found {
        let Found::Type(id) = found else {
            return found;
        };
        let Some((module, path)) = &self.types[id].alias else {
            return found;
        };
        let marker = (NONE, id);
        if seen.contains(&marker) {
            return Found::Foreign;
        }
        seen.push(marker);
        let target = self.resolve(*module, None, None, path, seen);
        seen.pop();
        target
    }

    /// `path` as written in `module`, in a body with the `use` items `uses` if it is one; `self_type` is what
    /// `Self` means there.
    fn resolve(
        &self,
        module: usize,
        uses: Option<&Scope>,
        self_type: Option<usize>,
        path: &[usize],
        seen: &mut Vec<(usize, usize)>,
    ) -> Found {
        let Some((&first, rest)) = path.split_first() else {
            return Found::Foreign;
        };
        let last = path[path.len() - 1];
        let mut found = match first {
            CRATE => Found::Module(0),
            THIS => Found::Module(module),
            SUPER => match self.modules[module].parent {
                Some(parent) => Found::Module(parent),
                None => Found::Unknown(last, Lookup::Module),
            },
            SELF_TYPE => match self_type {
                Some(id) => Found::Type(id),
                None => Found::Unknown(last, Lookup::Associated),
            },
            _ => match self.lookup_in_body(uses, module, first, seen) {
                // A glob of another crate may hold a bare name; a path that starts with an unbound name is
                // that crate's own (or the standard library's).
                Found::Missing(true) if rest.is_empty() => {
                    return Found::Unknown(last, Lookup::Module);
                }
                Found::Missing(_) => return Found::Foreign,
                found => found,
            },
        };
        for &segment in rest {
            found = match self.follow(found, seen) {
                Found::Module(inside) => match segment {
                    THIS => Found::Module(inside),
                    SUPER => match self.modules[inside].parent {
                        Some(parent) => Found::Module(parent),
                        None => Found::Unknown(segment, Lookup::Module),
                    },
                    _ => match self.lookup(inside, segment, seen) {
                        Found::Missing(_) => Found::Unknown(segment, Lookup::Module),
                        found => found,
                    },
                },
                Found::Type(id) => match self.types[id].assoc.get(&segment) {
                    Some(nodes) => Found::Nodes(nodes.clone()),
                    None if self.types[id].is_trait => Found::Unknown(segment, Lookup::Associated),
                    None => Found::Unknown(segment, Lookup::Provided),
                },
                Found::Foreign => return Found::Foreign,
                Found::Unknown(name, lookup) => return Found::Unknown(name, lookup),
                Found::Nodes(_) | Found::Missing(_) => {
                    return Found::Unknown(segment, Lookup::Module);
                }
            };
        }
        self.follow(found, seen)
    }

    /// What `node`'s reference `key` leads to, over each of `paths` (a qualified path has two ways to a function).
    /// Read in the module of the node, or with the node's own `use` items when it has any.
    fn outcome<'m>(
        &self,
        memo: &'m mut Memo,
        node: usize,
        on: Option<usize>,
        key: usize,
        paths: &[&[usize]],
    ) -> &'m Outcome {
        let data = &self.nodes[node];
        memo.entry((self.scope_key(node), on, key))
            .or_insert_with(|| {
                let uses = (!data.uses.is_empty()).then_some(&data.uses);
                let mut outcome = Outcome::default();
                let mut unknown = None;
                for path in paths {
                    match self.resolve(data.module, uses, on, path, &mut Vec::new()) {
                        Found::Nodes(found) => outcome.nodes.extend(found),
                        Found::Unknown(name, lookup) => unknown = unknown.or(Some((name, lookup))),
                        _ => {}
                    }
                }
                if outcome.nodes.is_empty() {
                    outcome.unknown = unknown;
                }
                outcome
            })
    }

    /// What a node's paths are memoized by: its module, or itself when its body has `use` items of its own.
    fn scope_key(&self, node: usize) -> usize {
        if self.nodes[node].uses.is_empty() {
            self.nodes[node].module
        } else {
            self.modules.len() + node
        }
    }

    /// The type each impl block is for, once every module is known; then each function attached to its type.
    fn link(&mut self) -> Vec<Option<usize>> {
        let mut types = vec![None; self.imps.len()];
        let mut attach = Vec::new();
        for (index, imp) in self.imps.iter().enumerate() {
            let Some(path) = &imp.self_path else { continue };
            if let Found::Type(id) = self.resolve(imp.module, None, None, path, &mut Vec::new()) {
                types[index] = Some(id);
                for &node in &imp.nodes {
                    attach.push((id, self.nodes[node].name, node));
                }
            }
        }
        for (id, name, node) in attach {
            self.types[id].assoc.entry(name).or_default().push(node);
        }
        types
    }

    /// Whether `module` is `scope` or inside it.
    fn within(&self, mut module: usize, scope: usize) -> bool {
        loop {
            if module == scope {
                return true;
            }
            match self.modules[module].parent {
                Some(parent) => module = parent,
                None => return false,
            }
        }
    }

    /// The subtree of both `a` and `b`: what an item private to one becomes when the other re-exports it.
    fn widen(&self, a: usize, b: usize) -> usize {
        let mut at = a;
        while !self.within(b, at) {
            at = self.modules[at].parent.unwrap_or(0);
        }
        at
    }

    /// The part of `a` and `b` both see: what a method is bounded to by its type.
    fn narrow(&self, a: usize, b: usize) -> usize {
        if self.within(a, b) {
            a
        } else if self.within(b, a) {
            b
        } else {
            a
        }
    }

    /// The module whose subtree sees each function: its own visibility, widened to that of every `use`
    /// that re-exports it, and for a function of an `impl` block bounded by the type's.
    fn visible(&self, impl_types: &[Option<usize>]) -> Vec<usize> {
        let mut types: Vec<usize> = self.types.iter().map(|data| data.visible).collect();
        let mut nodes: Vec<usize> = self.nodes.iter().map(|node| node.visible).collect();
        for (module, data) in self.modules.iter().enumerate() {
            for binding in &data.scope.bindings {
                match self.resolve(module, None, None, &binding.path, &mut Vec::new()) {
                    Found::Nodes(found) => {
                        for callee in found {
                            nodes[callee] = self.widen(nodes[callee], binding.visible);
                        }
                    }
                    Found::Type(id) => types[id] = self.widen(types[id], binding.visible),
                    _ => {}
                }
            }
            for glob in &data.scope.globs {
                if let Found::Module(target) =
                    self.resolve(module, None, None, &glob.path, &mut Vec::new())
                {
                    let inside = &self.modules[target];
                    for &callee in inside.values.values().flatten() {
                        nodes[callee] = self.widen(nodes[callee], glob.visible);
                    }
                    for &id in inside.types.values() {
                        types[id] = self.widen(types[id], glob.visible);
                    }
                }
            }
        }
        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(owner) = node.imp.and_then(|imp| impl_types[imp]) {
                nodes[index] = self.narrow(nodes[index], types[owner]);
            }
        }
        nodes
    }

    fn in_cfg_test(&self, mut module: usize) -> bool {
        loop {
            if self.modules[module].cfg_test {
                return true;
            }
            match self.modules[module].parent {
                Some(parent) => module = parent,
                None => return false,
            }
        }
    }

    /// The methods named `name` that the caller can see, as an index into `lists`.
    fn method_list(
        &self,
        visible: &[usize],
        cfg_test: &[bool],
        lists: &mut Lists,
        key: (usize, bool, usize),
    ) -> usize {
        let (module, caller_cfg_test, name) = key;
        if let Some(&id) = lists.ids.get(&key) {
            return id;
        }
        let found = self.methods[&name]
            .iter()
            .copied()
            .filter(|&callee| {
                (caller_cfg_test || !cfg_test[callee]) && self.within(module, visible[callee])
            })
            .collect();
        lists.all.push(found);
        lists.ids.insert(key, lists.all.len() - 1);
        lists.all.len() - 1
    }

    /// Everything that reaches a seed; a `blocked` node is never reached through (the boundary is not itself an
    /// acquisition a test must be refused for).
    fn reach(
        &self,
        edges: &Edges,
        lists: &[Vec<usize>],
        seeds: Vec<bool>,
        blocked: &[bool],
    ) -> (Vec<bool>, Vec<usize>) {
        let mut hit = seeds;
        let mut via = vec![NONE; self.nodes.len()];
        loop {
            let mut changed = false;
            for node in 0..self.nodes.len() {
                if hit[node] || blocked[node] {
                    continue;
                }
                let step = edges.direct[node]
                    .iter()
                    .copied()
                    .find(|&callee| hit[callee])
                    .or_else(|| {
                        edges.methods[node].iter().find_map(|&list| {
                            let all = &lists[list];
                            all.iter().all(|&callee| hit[callee]).then(|| all[0])
                        })
                    });
                if let Some(callee) = step {
                    hit[node] = true;
                    via[node] = callee;
                    changed = true;
                }
            }
            if !changed {
                return (hit, via);
            }
        }
    }

    fn qualified(&self, node: usize) -> String {
        let node = &self.nodes[node];
        let mut parts = Vec::new();
        if !self.modules[node.module].path.is_empty() {
            parts.push(self.modules[node.module].path.clone());
        }
        if let Some(owner) = node.imp.and_then(|imp| self.imps[imp].owner) {
            parts.push(self.names.name(owner).to_owned());
        }
        parts.push(self.names.name(node.name).to_owned());
        let qualified = parts.join("::");
        if node.kind == Kind::Macro {
            format!("{qualified}!")
        } else {
            qualified
        }
    }

    /// The names from the function `start` calls to the seed, then `tail` if the seed reads a literal.
    fn chain(&self, via: &[usize], start: usize, tail: Option<&str>) -> String {
        let mut parts = Vec::new();
        let mut at = start;
        while via[at] != NONE && parts.len() < 32 {
            at = via[at];
            parts.push(self.names.name(self.nodes[at].name).to_owned());
        }
        parts.extend(tail.map(str::to_owned));
        parts.join(" -> ")
    }

    fn spelled(&self, key: &Key) -> String {
        let join = |path: &[usize]| {
            path.iter()
                .map(|&segment| self.names.name(segment))
                .collect::<Vec<_>>()
                .join("::")
        };
        match key {
            Key::Path(path) | Key::Qualified(path, _) => join(path),
            Key::Method(name) | Key::SelfMethod(name) => format!(".{}()", self.names.name(*name)),
            Key::Macro(name) => format!("{}!", self.names.name(*name)),
        }
    }

    /// Every function, constant and macro that reaches a seed, and every reference that spells one and does not
    /// resolve, for `package`'s `target`.
    pub fn finish(mut self, package: &str, target: &str) -> Vec<Value> {
        let impl_types = self.link();
        let visible = self.visible(&impl_types);
        // Resolve inherited gates now: an out-of-line declaration may have been seen after its file.
        let test_target = target.starts_with("test:");
        let module_cfg_test: Vec<bool> = (0..self.modules.len())
            .map(|module| self.in_cfg_test(module))
            .collect();
        let cfg_test: Vec<bool> = self
            .nodes
            .iter()
            .map(|node| {
                test_target
                    || node.cfg_test
                    || module_cfg_test[node.module]
                    || node
                        .imp
                        .and_then(|imp| impl_types[imp])
                        .is_some_and(|owner| {
                            let ty = &self.types[owner];
                            ty.cfg_test || module_cfg_test[ty.module]
                        })
            })
            .collect();
        let count = self.nodes.len();
        let mut memo = Memo::new();
        let mut lists = Lists::default();
        let mut edges = Edges {
            direct: vec![Vec::new(); count],
            methods: vec![Vec::new(); count],
        };
        for index in 0..count {
            let node = &self.nodes[index];
            let own_type = node.imp.and_then(|imp| impl_types[imp]);
            for &id in &node.refs {
                match &self.keys[id] {
                    Key::Path(path) => {
                        let on = if path[0] == SELF_TYPE { own_type } else { None };
                        let outcome = self.outcome(&mut memo, index, on, id, &[path.as_slice()]);
                        edges.direct[index].extend(outcome.nodes.iter().copied());
                    }
                    Key::Qualified(typed, traited) => {
                        let on = if typed[0] == SELF_TYPE {
                            own_type
                        } else {
                            None
                        };
                        let outcome = self.outcome(
                            &mut memo,
                            index,
                            on,
                            id,
                            &[typed.as_slice(), traited.as_slice()],
                        );
                        edges.direct[index].extend(outcome.nodes.iter().copied());
                    }
                    Key::Method(name) => {
                        if self.methods.contains_key(name) {
                            let list = self.method_list(
                                &visible,
                                &cfg_test,
                                &mut lists,
                                (node.module, cfg_test[index], *name),
                            );
                            if !lists.all[list].is_empty() {
                                edges.methods[index].push(list);
                            }
                        }
                    }
                    Key::SelfMethod(name) => {
                        match own_type.and_then(|id| self.types[id].assoc.get(name)) {
                            Some(nodes) => edges.direct[index].extend(nodes.iter().copied()),
                            None if self.methods.contains_key(name) => {
                                let list = self.method_list(
                                    &visible,
                                    &cfg_test,
                                    &mut lists,
                                    (node.module, cfg_test[index], *name),
                                );
                                if !lists.all[list].is_empty() {
                                    edges.methods[index].push(list);
                                }
                            }
                            None => {}
                        }
                    }
                    Key::Macro(name) => {
                        if let Some(nodes) = self.macros.get(name) {
                            edges.direct[index].extend(nodes.iter().copied());
                        }
                    }
                }
            }
        }
        let boundary: Vec<bool> = self
            .nodes
            .iter()
            .map(|node| {
                node.kind == Kind::Function
                    && node.imp.is_none()
                    && self.names.name(node.name) == self.seeds.boundary_name
                    && self.files[node.file].ends_with(&self.seeds.boundary_file)
            })
            .collect();
        let literal: Vec<bool> = self
            .nodes
            .iter()
            .zip(&boundary)
            .map(|(node, boundary)| node.literal && !boundary)
            .collect();
        let (boundary_hit, boundary_via) =
            self.reach(&edges, &lists.all, boundary.clone(), &vec![false; count]);
        let (legacy_hit, legacy_via) = self.reach(&edges, &lists.all, literal.clone(), &boundary);
        let mut records = Vec::new();
        let put = |records: &mut Vec<Value>, node: usize, mut record: Value| {
            let object = record.as_object_mut().expect("a record is an object");
            object.insert("package".into(), json!(package));
            object.insert("target".into(), json!(target));
            object.insert("path".into(), json!(self.files[self.nodes[node].file]));
            object.insert("line".into(), json!(self.nodes[node].line));
            object.insert("name".into(), json!(self.qualified(node)));
            object.insert("test".into(), json!(self.nodes[node].test));
            records.push(record);
        };
        for node in 0..count {
            if boundary_hit[node] && !boundary[node] {
                let chain = self.chain(&boundary_via, node, None);
                put(
                    &mut records,
                    node,
                    json!({"kind": "reaches", "seed": "boundary", "chain": chain}),
                );
            }
            if legacy_hit[node] {
                let chain = self.chain(&legacy_via, node, Some("reads the prepared root"));
                put(
                    &mut records,
                    node,
                    json!({"kind": "reaches", "seed": "legacy", "chain": chain}),
                );
            }
        }
        // What an unresolved path may have meant: the functions of its name, by where it was looked up. A module
        // path leads to a free function, a trait or a `Self` that is not known to any `impl` function, and a
        // concrete type only to what a trait's default body provides.
        let mut named: HashMap<(usize, Lookup), Vec<usize>> = HashMap::new();
        for (index, data) in self.nodes.iter().enumerate() {
            if data.kind == Kind::Macro {
                continue;
            }
            match data.imp {
                None => named
                    .entry((data.name, Lookup::Module))
                    .or_default()
                    .push(index),
                Some(imp) => {
                    named
                        .entry((data.name, Lookup::Associated))
                        .or_default()
                        .push(index);
                    if self.imps[imp].trait_def {
                        named
                            .entry((data.name, Lookup::Provided))
                            .or_default()
                            .push(index);
                    }
                }
            }
        }
        for node in 0..count {
            let data = &self.nodes[node];
            let own_type = data.imp.and_then(|imp| impl_types[imp]);
            for &id in &data.refs {
                for (seed, hit) in [("boundary", &boundary_hit), ("legacy", &legacy_hit)] {
                    let refused = match &self.keys[id] {
                        Key::Path(path) | Key::Qualified(path, _) => {
                            let on = if path[0] == SELF_TYPE { own_type } else { None };
                            let unknown = memo
                                .get(&(self.scope_key(node), on, id))
                                .and_then(|outcome| outcome.unknown);
                            // A name the body binds itself is not an import.
                            let local = matches!(&self.keys[id], Key::Path(_))
                                && path.len() == 1
                                && data.locals.contains(&path[0]);
                            unknown.is_some_and(|(name, lookup)| {
                                !local
                                    && named.get(&(name, lookup)).is_some_and(|all| {
                                        all.iter().any(|&callee| {
                                            hit[callee]
                                                && (cfg_test[node] || !cfg_test[callee])
                                                && self.within(data.module, visible[callee])
                                        })
                                    })
                            })
                        }
                        Key::Method(name) | Key::SelfMethod(name) => {
                            let exact = matches!(&self.keys[id], Key::SelfMethod(_))
                                && own_type
                                    .is_some_and(|id| self.types[id].assoc.contains_key(name));
                            !exact
                                && lists
                                    .ids
                                    .get(&(data.module, cfg_test[node], *name))
                                    .is_some_and(|&list| {
                                        let all = &lists.all[list];
                                        let reaching =
                                            all.iter().filter(|&&callee| hit[callee]).count();
                                        reaching > 0 && reaching < all.len()
                                    })
                        }
                        Key::Macro(_) => false,
                    };
                    if refused {
                        let reference = self.spelled(&self.keys[id]);
                        put(
                            &mut records,
                            node,
                            json!({"kind": "unresolved", "seed": seed, "reference": reference}),
                        );
                    }
                }
            }
        }
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds() -> Seeds {
        Seeds {
            boundary_file: "fixtures.rs".into(),
            boundary_name: "prepared_root".into(),
            exact_literals: vec!["PREPARED_ROOT".into()],
            literal_fragments: vec![".prepared/checkout".into()],
        }
    }

    /// Records of a crate given as (module, file, source).
    fn run(sources: &[(&str, &str, &str)]) -> Vec<Value> {
        let mut analysis = Analysis::new(seeds());
        for (module, file, source) in sources {
            analysis.add(module, file, source).unwrap();
        }
        analysis.finish("pkg", "lib")
    }

    fn names(records: &[Value], kind: &str, seed: &str) -> Vec<String> {
        let mut found: Vec<String> = records
            .iter()
            .filter(|r| r["kind"] == kind && r["seed"] == seed)
            .map(|r| r["name"].as_str().unwrap().to_owned())
            .collect();
        found.sort();
        found
    }

    fn chain(records: &[Value], name: &str, seed: &str) -> String {
        records
            .iter()
            .find(|r| r["kind"] == "reaches" && r["seed"] == seed && r["name"] == name)
            .unwrap_or_else(|| panic!("{name} does not reach the {seed} seed"))["chain"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    const FIXTURES: &str =
        "pub fn prepared_root() -> std::path::PathBuf { std::path::PathBuf::new() }";

    #[test]
    fn a_test_reaches_the_boundary_through_helpers_of_other_modules() {
        let records = run(&[
            ("", "lib.rs", "mod fixtures; mod helper; mod tests;"),
            ("fixtures", "fixtures.rs", FIXTURES),
            (
                "helper",
                "helper.rs",
                "pub fn with_root<R>(run: impl FnOnce(&std::path::Path) -> R) -> R {
                     run(&crate::fixtures::prepared_root())
                 }",
            ),
            (
                "tests",
                "tests.rs",
                "use crate::helper::with_root;
                 #[test] fn routed() { with_root(|root| drop(root)); }
                 #[test] fn synthetic() { let root = std::path::Path::new(\"prepared_root\"); drop(root); }",
            ),
        ]);
        assert_eq!(
            names(&records, "reaches", "boundary"),
            ["helper::with_root", "tests::routed"]
        );
        assert_eq!(
            chain(&records, "tests::routed", "boundary"),
            "with_root -> prepared_root"
        );
        assert!(names(&records, "reaches", "legacy").is_empty());
        assert!(names(&records, "unresolved", "boundary").is_empty());
    }

    #[test]
    fn an_alias_of_a_legacy_wrapper_across_modules_is_followed() {
        // The shape of `native_menu_places`: a wrapper imported under another name and called by a local
        // wrapper of the same name as the first, beside a decoy of that name that reads nothing.
        let records = run(&[
            ("", "lib.rs", "mod refs; mod menu; mod decoy; mod other;"),
            (
                "refs",
                "refs.rs",
                "pub mod tests {
                     fn fixture_root() -> Option<std::ffi::OsString> { std::env::var_os(\"PREPARED_ROOT\") }
                     pub fn with_game(run: impl FnOnce()) { fixture_root(); run() }
                 }",
            ),
            (
                "menu",
                "menu.rs",
                "use crate::refs::tests::with_game as with_base_game;
                 fn with_game(run: impl FnOnce()) { with_base_game(|| run()) }
                 #[test] fn a_new_menu_test() { with_game(|| ()) }",
            ),
            ("decoy", "decoy.rs", "fn with_game(run: impl FnOnce()) { run() }\n#[test] fn synthetic() { with_game(|| ()) }"),
            ("other", "other.rs", "use crate::decoy::*;\n#[test] fn also_synthetic() { with_game(|| ()) }"),
        ]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            [
                "menu::a_new_menu_test",
                "menu::with_game",
                "refs::tests::fixture_root",
                "refs::tests::with_game"
            ]
        );
        // The alias is the name the source gives; the chain is of the functions it leads to.
        assert_eq!(
            chain(&records, "menu::a_new_menu_test", "legacy"),
            "with_game -> with_game -> fixture_root -> reads the prepared root"
        );
    }

    #[test]
    fn constants_statics_and_macros_carry_what_they_name() {
        let records = run(&[(
            "",
            "lib.rs",
            "const ROOT: &str = \".prepared/checkout/26.3\";
             static ALSO: &str = ROOT;
             macro_rules! rooted { () => { std::path::Path::new(ROOT) }; }
             fn through_const() { let _ = ALSO; }
             fn through_macro() { let _ = rooted!(); }
             fn through_call_tokens() { assert_eq!(rooted!(), rooted!()); }
             fn unrelated() { assert_eq!(1, 1); }
             fn passed_as_a_value() { let wrapper = through_const; wrapper(); }",
        )]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            [
                "ALSO",
                "ROOT",
                "passed_as_a_value",
                "rooted!",
                "through_call_tokens",
                "through_const",
                "through_macro"
            ]
        );
    }

    #[test]
    fn methods_resolve_on_self_exactly_and_elsewhere_by_name() {
        let records = run(&[(
            "",
            "lib.rs",
            "struct Fixture; struct Plain;
             impl Fixture {
                 fn root(&self) -> std::ffi::OsString { std::env::var_os(\"PREPARED_ROOT\").unwrap() }
                 fn open(&self) { self.root(); }
                 fn open_with(&self) { Self::root(self); }
                 fn mixed_receiver(&self) { self.plain(); }
                 fn plain(&self) {}
             }
             impl Plain { fn plain(&self) {} fn root(&self) {} }
             fn through_unknown_receiver(x: &Fixture) { x.open(); }
             fn through_ambiguous_receiver(x: &Plain) { x.root(); }",
        )]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            [
                "Fixture::open",
                "Fixture::open_with",
                "Fixture::root",
                "through_unknown_receiver"
            ]
        );
        // `.root()` could be either `root`: one reaches the prepared root, the other does not.
        assert_eq!(
            names(&records, "unresolved", "legacy"),
            ["through_ambiguous_receiver"]
        );
    }

    #[test]
    fn only_test_contexts_consider_test_only_method_and_path_candidates() {
        let records = run(&[
            (
                "fixtures",
                "test_fixtures.rs",
                r#"pub mod nested {
                    pub struct ModuleFixture;
                    impl ModuleFixture { pub fn run(&self) { std::env::var_os("PREPARED_ROOT"); } }
                    pub fn helper() { std::env::var_os("PREPARED_ROOT"); }
                }"#,
            ),
            (
                "",
                "lib.rs",
                r#"use outside::*;
                #[cfg(all(unix, all(test)))] mod fixtures;
                #[cfg(test)] struct Fixture;
                impl Fixture { fn run(&self) { std::env::var_os("PREPARED_ROOT"); } }
                struct Plain;
                #[cfg(not(test))] impl Plain { fn run(&self) {} }
                fn production(x: &Plain) { x.run(); helper(); }
                #[test] fn ambiguous(x: &Fixture) { x.run(); helper(); }"#,
            ),
        ]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            [
                "Fixture::run",
                "fixtures::nested::ModuleFixture::run",
                "fixtures::nested::helper"
            ]
        );
        assert_eq!(
            names(&records, "unresolved", "legacy"),
            ["ambiguous", "ambiguous"]
        );
    }

    #[test]
    fn a_struct_literal_receiver_selects_its_own_method() {
        let records = run(&[(
            "",
            "lib.rs",
            r#"struct Fixture { value: u32 }
             impl Fixture { fn run(&self) { std::env::var_os("PREPARED_ROOT"); } }
             struct Plain;
             impl Plain { fn run(&self) {} }
             #[test] fn direct() { (&(Fixture { value: 1 })).run(); }"#,
        )]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            ["Fixture::run", "direct"]
        );
        assert!(names(&records, "unresolved", "legacy").is_empty());
    }

    #[test]
    fn a_struct_field_label_in_macro_tokens_is_not_a_path() {
        let records = run(&[(
            "",
            "lib.rs",
            r#"mod reader { pub fn heard() -> bool { std::env::var_os("PREPARED_ROOT").is_some() } }
             use outside::*;
             struct MyStruct { heard: bool }
             #[test] fn label() { let flag = true; let _ = vec![MyStruct { heard: flag }]; }
             #[test] fn value() { let _ = vec![MyStruct { heard: reader::heard() }]; }"#,
        )]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            ["reader::heard", "value"]
        );
        assert!(names(&records, "unresolved", "legacy").is_empty());
    }

    #[test]
    fn indirection_it_cannot_resolve_is_refused_when_a_wrapper_is_spelled() {
        let records = run(&[
            ("", "lib.rs", "mod real; mod gap; mod missing_home;"),
            (
                "real",
                "real.rs",
                "pub fn with_game() { std::env::var_os(\"PREPARED_ROOT\"); }",
            ),
            (
                "gap",
                "gap.rs",
                "use outside::fixtures::*;
                 use crate::missing_home;
                 #[test] fn from_a_glob_of_another_crate() { with_game(); }
                 #[test] fn through_a_module_without_it() { missing_home::with_game(); }
                 #[test] fn a_name_nothing_spells() { unrelated(); }
                 #[test] fn a_local_of_the_same_name() { let with_game = 1; assert_eq!(with_game, 1); }
                 #[test] fn leaves_the_crate() { outside::with_game(); }",
            ),
            ("missing_home", "missing_home.rs", "pub fn other() {}"),
        ]);
        assert_eq!(
            names(&records, "unresolved", "legacy"),
            [
                "gap::from_a_glob_of_another_crate",
                "gap::through_a_module_without_it"
            ]
        );
        assert!(names(&records, "reaches", "legacy").contains(&"real::with_game".to_owned()));
        assert!(
            names(&records, "unresolved", "boundary").is_empty(),
            "no boundary function is spelled"
        );
    }

    #[test]
    fn a_boundary_function_spelled_through_an_unresolved_path_is_refused() {
        let records = run(&[
            ("", "lib.rs", "mod fixtures; mod gap;"),
            ("fixtures", "fixtures.rs", FIXTURES),
            (
                "gap",
                "gap.rs",
                "use outside::*;
                 #[test] fn glob() { prepared_root(); }
                 #[test] fn typed() { Harness::prepared_root(); }",
            ),
        ]);
        assert_eq!(names(&records, "unresolved", "boundary"), ["gap::glob"]);
    }

    #[test]
    fn a_test_that_names_no_fixture_has_no_record() {
        let records = run(&[(
            "",
            "lib.rs",
            "use std::collections::*;
             fn double(x: i32) -> i32 { x * 2 }
             #[test] fn arithmetic() { assert_eq!(double(2), 4); }
             #[test] fn strings() { let root = \"prepared_root and PREPARED_ROOT\"; assert!(root.len() > 3); }
             #[test] fn collections() { let mut m = HashMap::new(); m.insert(1, 2); assert_eq!(m.len(), 1); }",
        )]);
        assert!(records.is_empty(), "{records:?}");
    }

    #[test]
    fn the_boundary_is_not_a_legacy_accessor_and_legacy_does_not_pass_through_it() {
        let records = run(&[
            ("", "lib.rs", "mod fixtures; mod tests;"),
            (
                "fixtures",
                "fixtures.rs",
                "pub fn prepared_root() { std::env::var_os(\"PREPARED_ROOT\"); }",
            ),
            (
                "tests",
                "tests.rs",
                "#[test] fn routed() { crate::fixtures::prepared_root(); }",
            ),
        ]);
        assert_eq!(names(&records, "reaches", "boundary"), ["tests::routed"]);
        assert!(names(&records, "reaches", "legacy").is_empty());
    }

    #[test]
    fn a_function_local_use_binds_for_that_function_alone() {
        let records = run(&[
            ("", "lib.rs", "mod real; mod decoy; mod callers;"),
            (
                "real",
                "real.rs",
                "pub fn with_game() { std::env::var_os(\"PREPARED_ROOT\"); }",
            ),
            ("decoy", "decoy.rs", "pub fn with_game() {}"),
            (
                "callers",
                "callers.rs",
                "fn a() { use crate::decoy::with_game; with_game(); }
                 fn b() { use crate::real::with_game; with_game(); }
                 #[test] fn through_the_decoy() { a(); }
                 #[test] fn through_the_real_one() { b(); }",
            ),
        ]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            [
                "callers::b",
                "callers::through_the_real_one",
                "real::with_game"
            ]
        );
        assert!(names(&records, "unresolved", "legacy").is_empty());
    }

    #[test]
    fn an_associated_constructor_without_a_receiver_is_not_a_method_or_a_name_for_other_types() {
        let records = run(&[
            ("", "lib.rs", "mod seed; mod other;"),
            (
                "seed",
                "seed.rs",
                "struct Edits;
                 impl Edits {
                     fn new() -> Self { std::env::var_os(\"PREPARED_ROOT\"); Edits }
                 }
                 #[test] fn builds() { Edits::new(); }",
            ),
            (
                "other",
                "other.rs",
                "use outside::*;
                 struct Other;
                 struct Bare;
                 impl Other { fn new() -> Self { Other } }
                 #[test] fn own() { Other::new(); }
                 #[test] fn missing() { Bare::new(); }
                 #[test] fn glob() { new(); }
                 #[test] fn receiver(x: &Other) { x.new(); }",
            ),
        ]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            ["seed::Edits::new", "seed::builds"]
        );
        assert!(
            names(&records, "unresolved", "legacy").is_empty(),
            "{records:?}"
        );
    }

    #[test]
    fn a_private_helper_is_not_a_name_in_an_unrelated_module() {
        let records = run(&[
            ("", "lib.rs", "mod a; mod b;"),
            (
                "a",
                "a.rs",
                "fn run() { std::env::var_os(\"PREPARED_ROOT\"); }
                 #[test] fn inside() { run(); }",
            ),
            (
                "b",
                "b.rs",
                "use outside::*;
                 #[test] fn unrelated() { run(); }",
            ),
        ]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            ["a::inside", "a::run"]
        );
        assert!(names(&records, "unresolved", "legacy").is_empty());
    }

    #[test]
    fn a_crate_visible_helper_is_still_refused_where_unresolved() {
        let records = run(&[
            ("", "lib.rs", "mod a; mod b; mod c;"),
            (
                "a",
                "a.rs",
                "pub(crate) fn run() { std::env::var_os(\"PREPARED_ROOT\"); }",
            ),
            (
                "b",
                "b.rs",
                "pub(super) fn load() { std::env::var_os(\"PREPARED_ROOT\"); }",
            ),
            (
                "c",
                "c.rs",
                "use outside::*;
                 #[test] fn run_it() { run(); }
                 #[test] fn load_it() { load(); }",
            ),
        ]);
        assert_eq!(
            names(&records, "unresolved", "legacy"),
            ["c::load_it", "c::run_it"]
        );
    }

    #[test]
    fn a_receiver_method_of_another_module_is_compared_only_when_visible() {
        let records = run(&[
            ("", "lib.rs", "mod a; mod b; mod c;"),
            (
                "a",
                "a.rs",
                "pub struct Fixture;
                 impl Fixture { pub fn open(&self) { std::env::var_os(\"PREPARED_ROOT\"); } }",
            ),
            (
                "b",
                "b.rs",
                "pub struct Plain;\nimpl Plain { pub fn open(&self) {} }",
            ),
            (
                "c",
                "c.rs",
                "struct Hidden;
                 impl Hidden { fn open(&self) {} }
                 #[test] fn ambiguous(x: &Hidden) { x.open(); }",
            ),
        ]);
        // `Fixture::open` reaches and `Plain::open` does not, both visible from `c`.
        assert_eq!(names(&records, "unresolved", "legacy"), ["c::ambiguous"]);
        let alone = run(&[
            ("", "lib.rs", "mod a; mod b; mod c;"),
            (
                "a",
                "a.rs",
                "struct Fixture;
                 impl Fixture { fn open(&self) { std::env::var_os(\"PREPARED_ROOT\"); } }",
            ),
            (
                "b",
                "b.rs",
                "pub struct Plain;\nimpl Plain { pub fn open(&self) {} }",
            ),
            (
                "c",
                "c.rs",
                "#[test] fn plain(x: &crate::b::Plain) { x.open(); }",
            ),
        ]);
        // The private `Fixture::open` is not visible from `c`: nothing visible reaches, nothing is said.
        assert!(names(&alone, "unresolved", "legacy").is_empty());
        assert!(
            names(&alone, "reaches", "legacy")
                .iter()
                .all(|n| n != "c::plain")
        );
    }

    #[test]
    fn a_type_alias_is_followed_to_its_type() {
        let records = run(&[(
            "",
            "lib.rs",
            "struct Fixture;
             impl Fixture { fn open() { std::env::var_os(\"PREPARED_ROOT\"); } }
             type Alias = Fixture;
             type Again = Alias;
             type Loop = Loop2;
             type Loop2 = Loop;
             #[test] fn through_an_alias() { Alias::open(); }
             #[test] fn through_two() { Again::open(); }
             #[test] fn through_a_cycle() { Loop::open(); }",
        )]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            ["Fixture::open", "through_an_alias", "through_two"]
        );
    }

    #[test]
    fn a_qualified_path_follows_its_type_and_trait() {
        let records = run(&[(
            "",
            "lib.rs",
            "struct Fixture;
             trait Open { fn open() {} }
             impl Open for Fixture { fn open() { std::env::var_os(\"PREPARED_ROOT\"); } }
             struct Plain;
             impl Open for Plain { fn open() {} }
             #[test] fn qualified() { <Fixture as Open>::open(); }
             #[test] fn plain() { <Plain as Open>::open(); }",
        )]);
        assert_eq!(
            names(&records, "reaches", "legacy"),
            ["Fixture::open", "qualified"]
        );
    }

    #[test]
    fn a_trait_default_body_that_reaches_a_seed_is_refused_on_a_concrete_type_that_lacks_the_function()
     {
        let records = run(&[(
            "",
            "lib.rs",
            "trait Open { fn open() { std::env::var_os(\"PREPARED_ROOT\"); } }
             struct Plain;
             #[test] fn provided() { Plain::open(); }",
        )]);
        assert_eq!(names(&records, "unresolved", "legacy"), ["provided"]);
    }
}
