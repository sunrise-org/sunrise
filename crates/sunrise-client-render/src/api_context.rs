//! Borrowed adapters for load and loop hooks shared by client-lib and gameplay.
use std::sync::Arc;
use sunrise_world::native_load_context as engine;

/// Convert the owning load's phase, without reading resources.
pub fn load_origin(origin: engine::DataLoadOrigin) -> mod_api::DataLoadOrigin {
    match origin {
        engine::DataLoadOrigin::ServerDataLoad => mod_api::DataLoadOrigin::ServerDataLoad,
        engine::DataLoadOrigin::ClientPreload => mod_api::DataLoadOrigin::ClientPreload,
        _ => unreachable!("all engine load origins are mapped"),
    }
}

/// Convert the loop's owning side.
pub fn loop_side(side: engine::LoopSide) -> mod_api::LoopSide {
    match side {
        engine::LoopSide::Server => mod_api::LoopSide::Server,
        engine::LoopSide::Client => mod_api::LoopSide::Client,
        _ => unreachable!("all engine loop sides are mapped"),
    }
}

/// Borrows one owning load's frozen metadata, without scanning or copying it.
pub struct LoadReader<'a>(pub &'a engine::LoadContext);
impl LoadReader<'_> {
    /// Build the API view after the hook's subscription guard.
    pub fn context(&self) -> mod_api::OwningLoad<'_> {
        mod_api::OwningLoad::new(load_origin(self.0.origin()), self)
    }
}
impl mod_api::LoadReader for LoadReader<'_> {
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.0.registry(name)
    }
    fn fluid_tags(&self) -> Arc<[(String, Arc<[String]>)]> {
        self.0.fluid_tags().clone()
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.0.setting(mod_id, key)
    }
}

/// Borrows one loop's frozen metadata, without scanning or copying it.
pub struct LoopReader<'a>(pub &'a engine::LoopContext);
impl LoopReader<'_> {
    /// Build the API view after the hook's subscription guard.
    pub fn context(&self) -> mod_api::OwningLoop<'_> {
        mod_api::OwningLoop::new(loop_side(self.0.side()), self)
    }
}
impl mod_api::LoopReader for LoopReader<'_> {
    fn item_is_in_tag(&self, item: mod_api::ItemKey, tag: mod_api::TagKey) -> Option<bool> {
        // A requested token has a name even before its completed item table is published.
        mod_api::__private::item_key("minecraft:air")?;
        let item = mod_api::__private::item_key_id(item)?;
        mod_api::__private::with_tag_name(tag, |name| self.0.item_is_in_tag(item, name?))
    }
    fn item_count(&self, tag: mod_api::TagKey) -> Option<usize> {
        mod_api::__private::item_key("minecraft:air")?;
        mod_api::__private::with_tag_name(tag, |name| Some(self.0.item_tag_members(name?)?.len()))
    }
    fn item_at(&self, tag: mod_api::TagKey, index: usize) -> Option<mod_api::ItemKey> {
        mod_api::__private::with_tag_name(tag, |name| {
            mod_api::__private::item_key(self.0.item_tag_members(name?)?.get(index)?)
        })
    }
    fn mod_recipe_property_contains(&self, set: &str, item: mod_api::ItemKey) -> bool {
        mod_api::__private::item_key_id(item)
            .is_some_and(|name| self.0.mod_recipe_property_contains(set, name))
    }
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.0.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.0.setting(mod_id, key)
    }
}

#[cfg(test)]
mod recipe_properties_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{DataLoadContext, LoopContext};

    #[test]
    fn item_tags_distinguish_unavailable_empty_ordered_and_replaced_generations() {
        struct RequestsItem(Arc<std::sync::OnceLock<mod_api::ItemKey>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for RequestsItem {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                self.0.set(r.item_key("minecraft:emerald")).unwrap();
            }
        }
        let requested = Arc::new(std::sync::OnceLock::new());
        mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
            "test",
            Box::new(RequestsItem(requested.clone())),
        )])
        .unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:grain").unwrap())
            .unwrap();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("test:missing").unwrap())
                .unwrap();
        let load = engine::LoadContext::new(
            engine::DataLoadOrigin::ServerDataLoad,
            Arc::new(engine::LoadInputs::new(
                Default::default(),
                Default::default(),
            )),
            [],
        );
        let mut generation = load.captured_loop_context(engine::LoopSide::Server);
        let reader = LoopReader(&generation);
        let view = reader.context();
        assert!(view.item_count(tag).is_none());
        generation.set_item_tags(Some(std::collections::HashMap::from([(
            "test:grain".into(),
            vec!["minecraft:emerald".into(), "minecraft:diamond".into()],
        )])));
        let reader = LoopReader(&generation);
        let view = reader.context();
        assert!(view.item_count(tag).is_none(), "keys are not bound yet");
        assert_eq!(
            view.is_in_tag(*requested.get().unwrap(), tag),
            None,
            "a registration-time token is not a completed item table"
        );
        mod_api::__private::resolve_items([
            "minecraft:air",
            "minecraft:emerald",
            "minecraft:diamond",
        ])
        .unwrap();
        let emerald = mod_api::__private::item_key("minecraft:emerald").unwrap();
        let diamond = mod_api::__private::item_key("minecraft:diamond").unwrap();
        let frozen = generation.clone();
        generation.set_item_tags(Some(std::collections::HashMap::from([(
            "test:grain".into(),
            vec!["minecraft:diamond".into()],
        )])));
        let old_reader = LoopReader(&frozen);
        let old_view = old_reader.context();
        let old: &dyn LoopContext = &old_view;
        let new_reader = LoopReader(&generation);
        let new_view = new_reader.context();
        let new: &dyn LoopContext = &new_view;
        assert_eq!(old.item_count(tag), Some(2));
        assert_eq!(old.item_at(tag, 0), Some(emerald));
        assert_eq!(old.item_at(tag, 1), Some(diamond));
        assert_eq!(new.item_count(tag), Some(1));
        assert_eq!(new.item_at(tag, 0), Some(diamond));
        assert_eq!(old.is_in_tag(emerald, tag), Some(true));
        assert_eq!(new.is_in_tag(emerald, tag), Some(false));
        assert_eq!(new.is_in_tag(emerald, missing), Some(false));
        assert_eq!(new.item_count(missing), Some(0));
        assert_eq!(new.item_at(tag, usize::MAX), None);
    }

    #[test]
    fn phase_readers_preserve_origins_sides_and_shared_resource_members() {
        let root = std::env::temp_dir().join(format!(
            "phase-reader-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("active")).unwrap();
        std::fs::write(root.join("active/mode"), " captured ").unwrap();
        let inputs = Arc::new(
            engine::LoadInputs::new(
                std::collections::BTreeMap::from([(
                    "item".into(),
                    vec!["test:second".into(), "test:first".into()],
                )]),
                root.clone(),
            )
            .with_fluid_tags(std::collections::HashMap::from([(
                "test:tag".into(),
                vec!["test:flowing".into(), "test:still".into()],
            )])),
        );
        mod_platform::__private::with_test_packages(&["second", "first"], || {
            for (origin, expected) in [
                (
                    engine::DataLoadOrigin::ServerDataLoad,
                    mod_api::DataLoadOrigin::ServerDataLoad,
                ),
                (
                    engine::DataLoadOrigin::ClientPreload,
                    mod_api::DataLoadOrigin::ClientPreload,
                ),
            ] {
                let load = engine::LoadContext::new(origin, inputs.clone(), ["active"]);
                let reader = LoadReader(&load);
                let context = reader.context();
                assert_eq!(context.origin(), expected);
                assert_eq!(
                    context.registry("item").as_deref(),
                    Some(&["test:second".into(), "test:first".into()][..])
                );
                assert!(Arc::ptr_eq(
                    &context.registry("item").unwrap(),
                    &load.registry("item").unwrap()
                ));
                assert!(Arc::ptr_eq(
                    &context.fluid_tags().unwrap(),
                    load.fluid_tags()
                ));
                assert_eq!(
                    context.fluid_tags().unwrap()[0].1.as_ref(),
                    ["test:flowing", "test:still"]
                );
                assert_eq!(context.registry("missing"), None);
                assert_eq!(context.setting("missing", "key"), None);
                assert_eq!(
                    context.setting("active", "mode").as_deref(),
                    Some("captured")
                );
                assert_eq!(
                    context.mods().ids().collect::<Vec<_>>(),
                    ["second", "first"]
                );
                for (side, expected) in [
                    (engine::LoopSide::Client, mod_api::LoopSide::Client),
                    (engine::LoopSide::Server, mod_api::LoopSide::Server),
                ] {
                    let source = load.loop_context(side, ["active"]);
                    let reader = LoopReader(&source);
                    let context = reader.context();
                    assert_eq!(context.side(), expected);
                    assert_eq!(
                        context.registry("item").as_deref(),
                        Some(&["test:second".into(), "test:first".into()][..])
                    );
                    assert_eq!(context.registry("missing"), None);
                    assert_eq!(context.setting("missing", "key"), None);
                    assert_eq!(
                        context.setting("active", "mode").as_deref(),
                        Some("captured")
                    );
                    assert_eq!(
                        context.mods().ids().collect::<Vec<_>>(),
                        ["second", "first"]
                    );
                }
            }
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}
