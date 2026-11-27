use crate::{SourceLayout, catalog::BindingKind, gameplay::Routing};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};
use syn::{ImplItem, Item, TraitItem, Type};

#[test]
fn native_hooks_live_in_their_catalog_area() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = Routing::load(&root).unwrap().catalog(&root).unwrap();
    let mut bindings = Vec::new();
    let mut targets = HashSet::new();
    let mut ancestors = HashSet::new();
    for hook in catalog.hooks {
        let Some(binding) = hook.gameplay_binding().unwrap() else {
            continue;
        };
        let module = format!("{}::{}", binding.package.domain().key, binding.module);
        let mut prefix = module.as_str();
        loop {
            if !ancestors.insert(prefix.to_owned()) {
                break;
            }
            let Some((parent, _)) = prefix.rsplit_once("::") else {
                break;
            };
            prefix = parent;
        }
        targets.insert(module.clone());
        bindings.push((hook.target, binding, module));
    }
    let mut modules = BTreeMap::<String, Vec<Item>>::new();
    SourceLayout::read(&root)
        .unwrap()
        .visit_files(
            &root,
            false,
            |module| ancestors.contains(module),
            |file, parsed| {
                if targets.contains(&file.module) {
                    if let Some(items) = modules.get_mut(&file.module) {
                        items.extend(parsed.items);
                    } else {
                        modules.insert(file.module.clone(), parsed.items);
                    }
                }
            },
        )
        .unwrap();
    assert!(!bindings.is_empty());
    for (target, binding, module) in bindings {
        let items = modules
            .get(&module)
            .unwrap_or_else(|| panic!("missing module {module}"));
        assert!(
            items.iter().any(|item| match (item, &binding.kind) {
                (Item::Fn(function), BindingKind::Free | BindingKind::Borrowed(_)) => {
                    function.sig.ident == binding.function
                }
                (Item::Trait(item), BindingKind::Shared) => {
                    item.ident == "GameHooks" && item.items.iter().any(|member| {
                        matches!(member, TraitItem::Fn(method) if method.sig.ident == binding.function && method.default.is_some())
                    })
                }
                (Item::Impl(item), BindingKind::Nominal | BindingKind::Scoped(_)) => {
                    let owner = match &binding.kind {
                        BindingKind::Scoped(owner) => owner.ident.to_string(),
                        _ => binding.package.game_type().to_owned(),
                    };
                    item.trait_.is_none()
                        && matches!(&*item.self_ty, Type::Path(ty) if ty.path.segments.last().is_some_and(|segment| segment.ident == owner))
                        && item.items.iter().any(|member| {
                            matches!(member, ImplItem::Fn(method) if method.sig.ident == binding.function)
                        })
                }
                _ => false,
            }),
            "missing catalog target {target}",
        );
    }
}
