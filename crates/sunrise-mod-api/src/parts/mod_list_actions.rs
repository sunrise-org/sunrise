use super::*;
use crate::{ModListActionCallback, ModListActionError, ModListActionsRegistrar};
use mod_vocab::__private::{ModListAction, ModListActionId};

impl ModListActionsRegistrar for PartRegistrar {
    fn register_mod_list_action(
        &mut self,
        key: &'static str,
        label: &'static str,
        activate: ModListActionCallback,
    ) -> Result<(), ModListActionError> {
        if key.is_empty()
            || key.starts_with('.')
            || !key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b))
        {
            return Err(ModListActionError::InvalidKey(key.into()));
        }
        if label.is_empty() || label.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(ModListActionError::InvalidLabel(label.into()));
        }
        let (_, actions) = self
            .parts
            .mod_list_actions
            .entry(self.mod_id)
            .or_insert_with(|| (self.mod_index, Vec::new()));
        if actions.iter().any(|prior| prior.id.key == key) {
            return Err(ModListActionError::Duplicate(key.into()));
        }
        actions.push(ModListAction::new(
            ModListActionId::new(self.mod_id, key),
            label,
            activate,
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModListActionResult::{OpenScreen, Stay};

    #[test]
    fn an_invalid_or_duplicate_declaration_leaves_the_first_action_intact() {
        let mut r = PartRegistrar::new("stats");
        assert_eq!(
            r.register_mod_list_action("show", "stats.show", |_, _| Stay),
            Ok(())
        );
        assert_eq!(
            r.register_mod_list_action("show", "stats.replace", |_, _| OpenScreen),
            Err(ModListActionError::Duplicate("show".into()))
        );
        assert_eq!(
            r.register_mod_list_action("other:show", "stats.show", |_, _| Stay),
            Err(ModListActionError::InvalidKey("other:show".into()))
        );
        assert_eq!(
            r.register_mod_list_action("missing_label", "  ", |_, _| Stay),
            Err(ModListActionError::InvalidLabel("  ".into()))
        );
        let (_, actions) = &r.parts.mod_list_actions["stats"];
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].label, "stats.show");
    }
}
