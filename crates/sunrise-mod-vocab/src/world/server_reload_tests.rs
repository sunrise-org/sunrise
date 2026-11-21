use super::*;
fn id(value: &str) -> Identifier {
    Identifier::parse(value).unwrap()
}
fn files() -> FileToIdConverter {
    FileToIdConverter::json("machines").unwrap()
}

#[test]
fn registry_views_and_explicit_order_before_vanilla() {
    let mut listeners = ServerListenerRegistry::new();
    let [first, functions, last, mod_first, mod_second] = [
        "minecraft:first",
        "minecraft:function_library",
        "minecraft:last",
        "test_mod:first",
        "test_mod:second",
    ]
    .map(id);
    listeners
        .add_listener(mod_first.clone(), files(), Arc::new(Boundary))
        .unwrap();
    listeners
        .add_listener(mod_second.clone(), files(), Arc::new(Boundary))
        .unwrap();
    assert_eq!(listeners.get_last_vanilla_listener(), &functions);
    assert!(listeners.get_registry().iter().eq([
        &first,
        &functions,
        &last,
        &mod_first,
        &mod_second,
    ]));
    listeners
        .add_dependency(mod_second.clone(), functions.clone())
        .unwrap();
    assert!(listeners.get_graph().iter().eq([
        (&first, &functions),
        (&functions, &last),
        (&first, &mod_first),
        (&mod_first, &last),
        (&first, &mod_second),
        (&mod_second, &last),
        (&mod_second, &functions),
    ]));
    let sorted = listeners.finish().unwrap().listeners;
    assert_eq!(
        sorted.into_iter().map(|entry| entry.id).collect::<Vec<_>>(),
        [first, mod_second, functions, mod_first, last]
    );
}

#[test]
fn failed_registration_is_not_silenced_by_ignoring_the_result() {
    let mut listeners = ServerListenerRegistry::new();
    assert_eq!(
        listeners
            .add_dependency(id("mistyped:listener"), VanillaServerListeners::functions())
            .unwrap_err(),
        ReloadOrderError::UnknownListener {
            id: id("mistyped:listener")
        }
    );
    assert!(matches!(
        listeners.finish(),
        Err(ReloadOrderError::UnknownListener { .. })
    ));
}

#[test]
fn duplicate_objects_cannot_make_the_reverse_lookup_ambiguous() {
    let mut listeners = ServerListenerRegistry::new();
    let listener = Arc::new(Boundary);
    listeners
        .add_listener(id("one:listener"), files(), listener.clone())
        .unwrap();
    assert_eq!(
        listeners.add_listener(id("two:listener"), files(), listener),
        Err(ReloadOrderError::DuplicateObject {
            id: id("two:listener"),
            registered_as: id("one:listener"),
        })
    );
}

#[test]
fn retained_listeners_read_back_typed_from_the_resources() {
    struct OtherListener;
    impl ServerReloadListener for OtherListener {
        fn prepare(
            &self,
            _: &ServerReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            Ok(Box::new(|| {}))
        }
    }
    let mut listeners = ServerListenerRegistry::new();
    let original = Arc::new(Boundary);
    let key = ListenerKey::new(id("test_mod:kept"));
    (&mut listeners as &mut dyn ServerReloadListeners)
        .add_retained_listener(&key, files(), original.clone())
        .unwrap();
    assert_eq!(key.id(), &id("test_mod:kept"));
    let ordered = listeners.finish().unwrap();
    let mut resources = RetainedListeners::default();
    resources.install(ordered.retained);
    let kept = (&resources as &dyn ServerReloadResources)
        .listener(&key)
        .unwrap();
    assert!(Arc::ptr_eq(&original, &kept));
    assert!(
        (&resources as &dyn ServerReloadResources)
            .listener(&ListenerKey::<OtherListener>::new(id("test_mod:kept")))
            .is_none()
    );
    assert!(
        (&resources as &dyn ServerReloadResources)
            .listener(&ListenerKey::<Boundary>::new(id("test_mod:absent")))
            .is_none()
    );
}

#[test]
fn reload_replaces_the_retained_store_whole() {
    let mut listeners = ServerListenerRegistry::new();
    let first = ListenerKey::<Boundary>::new(id("test_mod:first"));
    (&mut listeners as &mut dyn ServerReloadListeners)
        .add_retained_listener(&first, files(), Arc::new(Boundary))
        .unwrap();
    let mut resources = RetainedListeners::default();
    resources.install(listeners.finish().unwrap().retained);
    assert!(
        (&resources as &dyn ServerReloadResources)
            .listener(&first)
            .is_some()
    );

    let later = ServerListenerRegistry::new().finish().unwrap();
    resources.install(later.retained);
    assert!(
        (&resources as &dyn ServerReloadResources)
            .listener(&first)
            .is_none()
    );
}

#[test]
fn retained_registration_follows_the_same_rules_as_plain_listeners() {
    let mut listeners = ServerListenerRegistry::new();
    let listener = Arc::new(Boundary);
    listeners
        .add_listener(id("one:listener"), files(), listener.clone())
        .unwrap();
    assert_eq!(
        (&mut listeners as &mut dyn ServerReloadListeners).add_retained_listener(
            &ListenerKey::new(id("two:listener")),
            files(),
            listener
        ),
        Err(ReloadOrderError::DuplicateObject {
            id: id("two:listener"),
            registered_as: id("one:listener"),
        })
    );
    let key = ListenerKey::<Boundary>::new(id("one:kept"));
    (&mut listeners as &mut dyn ServerReloadListeners)
        .add_retained_listener(&key, files(), Arc::new(Boundary))
        .unwrap();
    assert!(
        (&mut listeners as &mut dyn ServerReloadListeners)
            .add_retained_listener(
                &ListenerKey::<Boundary>::new(id("one:kept")),
                files(),
                Arc::new(Boundary)
            )
            .is_err()
    );
    assert!(matches!(
        listeners.finish(),
        Err(ReloadOrderError::DuplicateObject { .. })
    ));
    drop(key);
}
