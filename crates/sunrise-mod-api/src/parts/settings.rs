//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[test]
    pub(crate) fn settings_are_owned_and_bad_or_duplicate_ones_are_refused() {
        let mut registrar = super::PartRegistrar::new("first");
        let first_on = registrar
            .register_setting(SettingSpec::toggle("on", "first.on", true))
            .unwrap();
        assert_eq!(first_on.owner(), "first");
        assert_eq!(
            registrar.register_setting(SettingSpec::toggle("on", "first.on", false)),
            Err(SettingError::Duplicate("on".into()))
        );
        assert_eq!(
            registrar.register_setting(SettingSpec::range(
                "speed",
                "first.speed",
                1.0,
                0.0,
                0.1,
                0.5
            )),
            Err(SettingError::EmptyRange)
        );
        registrar.mod_id = "second";
        let second_on = registrar
            .register_setting(SettingSpec::toggle("on", "second.on", false))
            .unwrap();
        assert_eq!(second_on.owner(), "second");
        assert_ne!(first_on, second_on);
        let owners: Vec<_> = registrar
            .parts
            .settings
            .iter()
            .map(|(owner, s)| (*owner, s.key.as_str()))
            .collect();
        assert_eq!(owners, [("first", "on"), ("second", "on")]);
        assert!(
            registrar.refused.is_empty(),
            "a refused setting is the mod's error, not startup's"
        );
    }

    #[test]
    pub(crate) fn categories_are_owned_and_unknown_or_empty_ones_are_refused() {
        let mut registrar = super::PartRegistrar::new("first");
        registrar
            .register_setting(SettingSpec::toggle("on", "first.on", true))
            .unwrap();
        registrar
            .register_setting(SettingSpec::toggle("off", "first.off", false))
            .unwrap();
        assert_eq!(registrar.categorize_setting("on", "first.group"), Ok(()));
        assert_eq!(
            registrar.categorize_setting("on", ""),
            Err(SettingError::EmptyLabel)
        );
        assert_eq!(
            registrar.categorize_setting("missing", "first.group"),
            Err(SettingError::UnknownSetting("missing".into()))
        );
        // A later call replaces the category instead of adding a second entry.
        assert_eq!(registrar.categorize_setting("on", "first.other"), Ok(()));
        assert_eq!(
            registrar.parts.setting_categories,
            [("first", "on".to_owned(), "first.other".to_owned())]
        );
        // Another mod cannot categorize the first mod's setting.
        registrar.mod_id = "second";
        assert_eq!(
            registrar.categorize_setting("on", "second.group"),
            Err(SettingError::UnknownSetting("on".into()))
        );
        assert_eq!(registrar.parts.setting_categories.len(), 1);
    }

    #[test]
    pub(crate) fn setting_rows_can_depend_on_an_earlier_choice() {
        let mut registrar = super::PartRegistrar::new("first");
        registrar
            .register_setting(SettingSpec::choice("mode", "mode", ["spyglass", "key"], 0))
            .unwrap();
        registrar
            .register_setting(SettingSpec::toggle("on", "on", true))
            .unwrap();
        registrar
            .register_setting(SettingSpec::range("key_zoom", "key", 2.0, 20.0, 1.0, 4.0))
            .unwrap();
        assert_eq!(
            registrar.show_setting_when_choice("key_zoom", "mode", 1),
            Ok(())
        );
        assert_eq!(
            registrar.parts.setting_visibility,
            [("first", "key_zoom".to_owned(), "mode".to_owned(), 1)]
        );
        assert_eq!(
            registrar.show_setting_when_choice("key_zoom", "mode", 2),
            Err(SettingError::InvalidVisibility {
                setting: "key_zoom".into(),
                controller: "mode".into(),
            })
        );
        assert_eq!(
            registrar.parts.setting_visibility,
            [("first", "key_zoom".to_owned(), "mode".to_owned(), 1)],
            "an invalid replacement must keep the previous condition"
        );
        assert_eq!(
            registrar.show_setting_when_choice("key_zoom", "missing", 0),
            Err(SettingError::UnknownSetting("missing".into()))
        );
        assert_eq!(
            registrar.show_setting_when_choice("missing", "mode", 0),
            Err(SettingError::UnknownSetting("missing".into()))
        );
        assert_eq!(
            registrar.show_setting_when_choice("key_zoom", "on", 0),
            Err(SettingError::InvalidVisibility {
                setting: "key_zoom".into(),
                controller: "on".into(),
            })
        );
        assert_eq!(
            registrar.show_setting_when_choice("mode", "mode", 0),
            Err(SettingError::InvalidVisibility {
                setting: "mode".into(),
                controller: "mode".into(),
            })
        );
        assert_eq!(
            registrar.show_setting_when_choice("mode", "key_zoom", 0),
            Err(SettingError::InvalidVisibility {
                setting: "mode".into(),
                controller: "key_zoom".into(),
            }),
            "a later setting cannot control an earlier row, so cycles are impossible"
        );
        let mut forward = super::PartRegistrar::new("forward");
        forward
            .register_setting(SettingSpec::range("row", "row", 0.0, 2.0, 1.0, 0.0))
            .unwrap();
        forward
            .register_setting(SettingSpec::choice("mode", "mode", ["a", "b"], 0))
            .unwrap();
        assert_eq!(
            forward.show_setting_when_choice("row", "mode", 0),
            Err(SettingError::InvalidVisibility {
                setting: "row".into(),
                controller: "mode".into(),
            })
        );
        assert!(forward.parts.setting_visibility.is_empty());
        assert_eq!(
            registrar.show_setting_when_choice("key_zoom", "mode", 0),
            Ok(())
        );
        assert_eq!(
            registrar.parts.setting_visibility,
            [("first", "key_zoom".to_owned(), "mode".to_owned(), 0)]
        );
        registrar.mod_id = "second";
        assert_eq!(
            registrar.show_setting_when_choice("key_zoom", "mode", 1),
            Err(SettingError::UnknownSetting("key_zoom".into()))
        );
    }
}

impl SettingsRegistrar for PartRegistrar {
    fn register_setting(
        &mut self,
        setting: crate::SettingSpec,
    ) -> Result<crate::SettingKey, crate::SettingError> {
        setting.check()?;
        if self
            .parts
            .settings
            .iter()
            .any(|(owner, prior)| *owner == self.mod_id && prior.key == setting.key)
        {
            return Err(crate::SettingError::Duplicate(setting.key));
        }
        let slot = self.parts.settings.len();
        self.parts.settings.push((self.mod_id, setting));
        Ok(mod_vocab::__private::setting_key(self.mod_id, slot))
    }

    fn categorize_setting(&mut self, key: &str, category: &str) -> Result<(), crate::SettingError> {
        if category.is_empty() {
            return Err(crate::SettingError::EmptyLabel);
        }
        if !self
            .parts
            .settings
            .iter()
            .any(|(owner, spec)| *owner == self.mod_id && spec.key.as_str() == key)
        {
            return Err(crate::SettingError::UnknownSetting(key.to_owned()));
        }
        if let Some(entry) = self
            .parts
            .setting_categories
            .iter_mut()
            .find(|(owner, k, _)| *owner == self.mod_id && k.as_str() == key)
        {
            entry.2 = category.to_owned();
        } else {
            self.parts
                .setting_categories
                .push((self.mod_id, key.to_owned(), category.to_owned()));
        }
        Ok(())
    }

    fn show_setting_when_choice(
        &mut self,
        key: &str,
        controller: &str,
        choice: usize,
    ) -> Result<(), crate::SettingError> {
        let Some(setting_index) = self
            .parts
            .settings
            .iter()
            .position(|(owner, spec)| *owner == self.mod_id && spec.key == key)
        else {
            return Err(crate::SettingError::UnknownSetting(key.to_owned()));
        };
        let Some((controller_index, (_, controller_spec))) = self
            .parts
            .settings
            .iter()
            .enumerate()
            .find(|(_, (owner, spec))| *owner == self.mod_id && spec.key == controller)
        else {
            return Err(crate::SettingError::UnknownSetting(controller.to_owned()));
        };
        let valid_choice = matches!(
            &controller_spec.kind,
            crate::SettingKind::Choice { options, .. } if choice < options.len()
        );
        // The controller must precede the dependent row. That gives every condition a
        // registration-order edge and makes visibility cycles impossible.
        if controller_index >= setting_index || !valid_choice {
            return Err(crate::SettingError::InvalidVisibility {
                setting: key.to_owned(),
                controller: controller.to_owned(),
            });
        }
        if let Some(entry) = self
            .parts
            .setting_visibility
            .iter_mut()
            .find(|(owner, setting, _, _)| *owner == self.mod_id && setting == key)
        {
            entry.2 = controller.to_owned();
            entry.3 = choice;
        } else {
            self.parts.setting_visibility.push((
                self.mod_id,
                key.to_owned(),
                controller.to_owned(),
                choice,
            ));
        }
        Ok(())
    }

    fn register_config_screen(
        &mut self,
        factory: Arc<dyn crate::ConfigScreenFactory>,
    ) -> Result<(), String> {
        if self
            .parts
            .config_screens
            .iter()
            .any(|prior| prior.owner == self.mod_id)
        {
            return Err(format!(
                "mod {} registers a second config screen",
                self.mod_id
            ));
        }
        self.parts.config_screens.push(ConfigScreenRegistration {
            owner: self.mod_id,
            index: self.mod_index,
            factory,
            panicked: Default::default(),
        });
        Ok(())
    }
}
