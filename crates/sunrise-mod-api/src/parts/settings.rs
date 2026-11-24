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
