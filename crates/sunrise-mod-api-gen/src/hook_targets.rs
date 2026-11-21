//! Native hook targets come from definitions, not declaration file names.
use crate::catalog::{Catalog, PartKind};
use quote::ToTokens;
use std::{collections::BTreeMap, fs, path::Path};

fn definitions(root: &Path) -> Result<BTreeMap<String, (String, syn::Signature)>, String> {
    let mut targets = BTreeMap::new();
    for path in sunrise_game_discovery::native_hook_files(root)? {
        let area = path.file_stem().unwrap().to_str().unwrap();
        let file = syn::parse_file(&fs::read_to_string(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for item in file.items {
            let syn::Item::Impl(item) = item else {
                continue;
            };
            if item.trait_.is_some() {
                continue;
            }
            let ty = item.self_ty.to_token_stream().to_string().replace(' ', "");

            for member in item.items {
                let syn::ImplItem::Fn(method) = member else {
                    continue;
                };
                let name = method.sig.ident.to_string();
                if !name.starts_with("native_") {
                    continue;
                }
                let target = format!("sunrise_client::game::native_hooks::{area}::{ty}::{name}");
                if targets
                    .insert(format!("{ty}::{name}"), (target, method.sig))
                    .is_some()
                {
                    return Err(format!("duplicate native hook definition: {name}"));
                }
            }
        }
    }
    Ok(targets)
}

pub(crate) const EFFECT_TICK_TARGET: &str = "mod_api::parts::effects::dispatch_mob_effect_tick";

pub(crate) fn resolve(root: &Path, catalog: &mut Catalog) -> Result<(), String> {
    let targets = definitions(root)?;
    let mut checked_effect_tick_target = false;
    for hook in &mut catalog.hooks {
        if hook
            .scope
            .as_ref()
            .is_some_and(|scope| scope.kind == PartKind::Effect)
        {
            if !matches!(
                hook.name.as_str(),
                "should_apply_effect_tick_this_tick" | "apply_effect_tick"
            ) || hook.target != EFFECT_TICK_TARGET
                || hook.call.is_empty()
            {
                return Err(format!(
                    "{}: an effect tick hook must declare its call to {EFFECT_TICK_TARGET}",
                    hook.name
                ));
            }
            if !checked_effect_tick_target {
                let path = root.join("crates/sunrise-mod-vocab/src/parts/effects.rs");
                let source =
                    fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let parsed = syn::parse_file(&source).map_err(|e| e.to_string())?;
                if !parsed.items.iter().any(|item| matches!(item, syn::Item::Fn(f) if f.sig.ident == "dispatch_mob_effect_tick")) {
                    return Err(format!("{}: no definition for {EFFECT_TICK_TARGET}", hook.name));
                }
                checked_effect_tick_target = true;
            }
            continue;
        }
        if hook
            .target
            .starts_with("sunrise_client::game::native_hooks::")
        {
            let tail = hook
                .target
                .strip_prefix("sunrise_client::game::native_hooks::")
                .unwrap();
            let name = tail.split("::").skip(1).collect::<Vec<_>>().join("::");
            let Some((target, signature)) = targets.get(&name) else {
                return Err(format!("{}: no native definition", hook.name));
            };
            if hook.stack_count.is_some() && hook.call.is_empty() {
                let output = signature.inputs.iter().find_map(|input| {
                    let syn::FnArg::Typed(input) = input else {
                        return None;
                    };
                    let syn::Pat::Ident(parameter) = &*input.pat else {
                        return None;
                    };
                    (Some(parameter.ident.to_string()).as_ref() == hook.out.as_ref())
                        .then_some(&*input.ty)
                });
                crate::catalog::validate_stack_count_output(&hook.name, output)
                    .map_err(|e| e.to_string())?;
            }
            hook.target = target.clone();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_signature_still_requires_its_native_definition() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let declared = Catalog::load_declarations(&root).unwrap();
        let mut hook = declared
            .hooks
            .into_iter()
            .find(|hook| !hook.call.is_empty())
            .unwrap();
        hook.target =
            "sunrise_client::game::native_hooks::base::Game::native_missing_fixture".into();
        hook.scope = None;
        let mut catalog = Catalog {
            hooks: vec![hook],
            draw_contexts: Vec::new(),
            general_notes: None,
        };
        assert!(!catalog.hooks[0].call.is_empty());
        assert!(
            resolve(&root, &mut catalog)
                .unwrap_err()
                .contains("no native definition")
        );
    }

    #[test]
    #[ignore = "daily"]
    fn every_catalog_target_resolves_and_missing_function_is_rejected() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut catalog = Catalog::load(&root).unwrap();

        let declared = Catalog::load_declarations(&root).unwrap();
        let wrong = declared
            .hooks
            .iter()
            .zip(&catalog.hooks)
            .filter(|(old, new)| old.target != new.target)
            .map(|(old, _)| old.name.as_str())
            .collect::<Vec<_>>();
        println!("{} corrected targets: {}", wrong.len(), wrong.join(", "));
        // Dispatch resolves named game methods and free render functions through the
        // same source parsers used for generation (including crate aliases).
        crate::sources(&root, crate::FillMode::Require).unwrap();
        crate::dispatch::render(&root, &catalog).unwrap();
        let hook = catalog
            .hooks
            .iter_mut()
            .find(|h| h.name == "anvil_craft_post")
            .unwrap();
        assert!(hook.target.contains("::base::"));
        hook.target.push_str("_nonexistent");
        assert!(
            resolve(&root, &mut catalog)
                .unwrap_err()
                .contains("no native definition")
        );
    }
}

#[cfg(test)]
#[test]
fn stack_count_rejects_inferred_non_stack_outputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut catalog = Catalog::load(&root).unwrap();
    let hook = catalog
        .hooks
        .iter_mut()
        .find(|h| h.name == "can_equip")
        .unwrap();
    hook.stack_count = Some(crate::catalog::StackCount::Trim);
    assert!(
        resolve(&root, &mut catalog)
            .unwrap_err()
            .contains("mutable Vec<ItemStack>")
    );
}
