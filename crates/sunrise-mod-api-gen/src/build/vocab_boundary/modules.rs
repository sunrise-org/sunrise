//! Out-of-line generated modules may only refer to source scanned by the gate.
use quote::ToTokens;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

pub(in crate::build) fn vocab_root(root: &Path) -> Result<PathBuf, String> {
    root.join("crates/sunrise-mod-vocab/src")
        .canonicalize()
        .map_err(|e| e.to_string())
}

pub(super) fn validate(file: &syn::File, vocab_root: Option<&Path>) -> Result<(), String> {
    struct Modules<'a> {
        vocab_root: Option<&'a Path>,
        errors: Vec<String>,
    }
    impl<'ast> Visit<'ast> for Modules<'_> {
        fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
            if module.attrs.iter().any(|attr| {
                attr.path().is_ident("cfg_attr")
                    && attr
                        .meta
                        .to_token_stream()
                        .to_string()
                        .split_whitespace()
                        .any(|token| token == "path")
            }) {
                self.errors
                    .push("conditional generated module paths are not supported".into());
            }
            if module.content.is_none() {
                let path = module.attrs.iter().find_map(|attr| {
                    if !attr.path().is_ident("path") {
                        return None;
                    }
                    if let syn::Meta::NameValue(value) = &attr.meta
                        && let syn::Expr::Lit(value) = &value.value
                        && let syn::Lit::Str(path) = &value.lit
                    {
                        return Some(path.value());
                    }
                    None
                });
                let accepted = self.vocab_root.is_some_and(|vocab_root| {
                    path.as_ref()
                        .and_then(|path| {
                            Path::new(path)
                                .is_absolute()
                                .then(|| Path::new(path).canonicalize().ok())
                                .flatten()
                        })
                        .is_some_and(|path| {
                            path.starts_with(vocab_root)
                                && path.extension().is_some_and(|ext| ext == "rs")
                                && path.is_file()
                        })
                });
                if !accepted {
                    self.errors.push(format!(
                        "generated module {} targets source outside the scanned vocabulary set",
                        module.ident
                    ));
                }
            }
            syn::visit::visit_item_mod(self, module);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            fn declares_module(tokens: proc_macro2::TokenStream) -> bool {
                tokens.into_iter().any(|token| match token {
                    proc_macro2::TokenTree::Ident(name) => name == "mod",
                    proc_macro2::TokenTree::Group(group) => declares_module(group.stream()),
                    _ => false,
                })
            }
            if declares_module(node.tokens.clone()) {
                self.errors
                    .push("generated macros cannot declare unchecked modules".into());
            }
            syn::visit::visit_macro(self, node);
        }
    }
    let mut modules = Modules {
        vocab_root,
        errors: Vec::new(),
    };
    modules.visit_file(file);
    if modules.errors.is_empty() {
        Ok(())
    } else {
        Err(modules.errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn out_of_set_module_is_refused_but_scanned_source_is_accepted() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let vocab_root = super::vocab_root(&root).unwrap();
        let source = root
            .join("crates/sunrise-mod-vocab/src/dispatch_scope.rs")
            .canonicalize()
            .unwrap();
        let safe = syn::parse_file(&format!("#[path = {source:?}] mod socket;")).unwrap();
        assert!(super::validate(&safe, Some(&vocab_root)).is_ok());
        let outside = root
            .join("crates/sunrise-mod-api/src/lib.rs")
            .canonicalize()
            .unwrap();
        let escaped = syn::parse_file(&format!("#[path = {outside:?}] mod hidden;")).unwrap();
        assert!(super::validate(&escaped, Some(&vocab_root)).is_err());
        for mutant in [
            "#[path = \"/outside/hidden.rs\"] mod hidden;",
            "mod hidden;",
            "macro_rules! hidden { () => { #[path = \"/outside/hidden.rs\"] mod hidden; }; }",
        ] {
            assert!(super::validate(&syn::parse_file(mutant).unwrap(), Some(&vocab_root)).is_err());
        }
    }
    use std::path::Path;
}
