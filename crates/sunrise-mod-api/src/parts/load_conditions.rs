//! Startup authoring, validation, and freeze for mod-declared load condition handlers
//! (`neoforge:conditions` and `fabric:load_conditions`).
use super::*;
use crate::{Identifier, LoadCondition};

/// Built-in `neoforge:*` condition paths (`sunrise_world::conditions::BuiltinCondition`).
const BUILTIN_NEOFORGE_CONDITIONS: &[&str] = &[
    "mod_loaded",
    "always",
    "never",
    "feature_flags_enabled",
    "registered",
    "tag_empty",
    "not",
    "and",
    "or",
];

/// Built-in `fabric:*` condition paths (`sunrise_world::conditions::FABRIC_BUILTINS`).
const BUILTIN_FABRIC_CONDITIONS: &[&str] = &[
    "true",
    "false",
    "not",
    "and",
    "or",
    "all_mods_loaded",
    "any_mods_loaded",
    "tags_populated",
    "features_enabled",
    "registry_contains",
];

fn is_builtin_condition(id: &Identifier) -> bool {
    (id.namespace() == "neoforge" && BUILTIN_NEOFORGE_CONDITIONS.contains(&id.path()))
        || (id.namespace() == "fabric" && BUILTIN_FABRIC_CONDITIONS.contains(&id.path()))
}

impl LoadConditionsRegistrar for PartRegistrar {
    fn register_load_condition_erased(&mut self, id: &str, condition: Box<dyn LoadCondition>) {
        let Some(parsed) = Identifier::parse_namespaced(id) else {
            self.refused.push(format!(
                "mod {} registered load condition {id:?}, which is not a valid namespaced identifier",
                self.mod_id
            ));
            return;
        };
        if is_builtin_condition(&parsed) {
            self.refused.push(format!(
                "mod {} cannot register condition {parsed}: that ID is built in",
                self.mod_id
            ));
            return;
        }
        if let Some(existing) = self
            .load_conditions
            .iter()
            .find(|existing| existing.id == parsed)
        {
            self.refused.push(format!(
                "condition {parsed} registered by both {} and {}",
                existing.owner_mod, self.mod_id
            ));
            return;
        }
        self.load_conditions
            .push(mod_vocab::__private::RegisteredLoadCondition {
                id: parsed,
                owner_mod: self.mod_id,
                condition: Arc::from(condition),
            });
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ConditionError, HookId, Hooks, LoadCondition, ModEntry, Registrar, RegistrationContext,
    };

    struct Always;
    impl LoadCondition for Always {
        fn test(&self, _: &serde_json::Value) -> Result<bool, ConditionError> {
            Ok(true)
        }
    }

    fn install_with(mods: Vec<ModEntry>) -> Result<(), Vec<String>> {
        crate::parts::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
    }

    #[test]
    fn builtin_neoforge_and_fabric_load_conditions_refused_naming_mod() {
        for builtin in ["neoforge:mod_loaded", "fabric:all_mods_loaded"] {
            struct BadMod(&'static str);
            impl Hooks for BadMod {
                fn subscribed(&self) -> &'static [Option<HookId>] {
                    &[]
                }
                fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                    r.register_load_condition(self.0, Always);
                }
            }
            let err = install_with(vec![ModEntry::new("bad_mod", Box::new(BadMod(builtin)))])
                .unwrap_err();
            assert!(
                err.iter()
                    .any(|msg| msg.contains("bad_mod") && msg.contains(builtin)),
                "expected builtin load condition refusal for {builtin}, got: {err:?}"
            );
        }
    }

    #[test]
    fn duplicate_load_condition_refused_naming_both_mods() {
        struct ModA;
        struct ModB;
        impl Hooks for ModA {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_load_condition("shared:flag", Always);
            }
        }
        impl Hooks for ModB {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_load_condition("shared:flag", Always);
            }
        }
        let err = install_with(vec![
            ModEntry::new("mod_a", Box::new(ModA)),
            ModEntry::new("mod_b", Box::new(ModB)),
        ])
        .unwrap_err();
        assert!(
            err.iter().any(|msg| msg.contains("shared:flag")
                && msg.contains("mod_a")
                && msg.contains("mod_b")),
            "expected duplicate load condition refusal, got: {err:?}"
        );
    }
}
