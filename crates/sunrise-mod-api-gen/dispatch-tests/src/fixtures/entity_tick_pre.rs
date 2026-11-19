#[test]
fn entity_tick_pre_collects_each_mods_cancels_for_the_family_in_mod_order() {
    struct Cancel(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Cancel {
        fn entity_tick_pre(&self, _: HostSide<'_>, event: &mut EntityTickPreEvent<'_>) {
            for (&entity, &kind) in event.entities.iter().zip(event.kinds) {
                if kind == self.0 {
                    event.canceled.push(entity);
                }
            }
        }
    }
    let entities = [1, 2, 3, 4];
    let kinds = [
        "minecraft:zombie",
        "minecraft:item",
        "minecraft:cow",
        "minecraft:item",
    ];
    let mut canceled = vec![9];
    install(Vec::<ModEntry>::new().into());
    dispatch_entity_tick_pre(
        HostSide::Server(&mut TestHost),
        "minecraft:overworld",
        &entities,
        &kinds,
        &mut canceled,
        Listeners::All,
    );
    assert_eq!(canceled, [9]);
    install(
        vec![
            ModEntry::new("items", Box::new(Cancel("minecraft:item"))),
            ModEntry::new("zombies", Box::new(Cancel("minecraft:zombie"))),
        ]
        .into(),
    );
    dispatch_entity_tick_pre(
        HostSide::Server(&mut TestHost),
        "minecraft:overworld",
        &entities,
        &kinds,
        &mut canceled,
        Listeners::All,
    );
    assert_eq!(canceled, [9, 2, 4, 1]);
}

/// A selected call goes only to the listeners the game chose: those of a set, those outside it,
/// or one mod, each in dispatch order.
#[test]
fn entity_tick_pre_goes_to_the_listeners_the_game_chose() {
    struct Cancel(u32);
    #[mod_api::__subscriptions]
    impl Hooks for Cancel {
        fn entity_tick_pre(&self, _: HostSide<'_>, event: &mut EntityTickPreEvent<'_>) {
            event.canceled.push(self.0);
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Cancel(1))),
            ModEntry::new("second", Box::new(Cancel(2))),
            ModEntry::new("third", Box::new(Cancel(3))),
        ]
        .into(),
    );
    let told = |listeners: Listeners<'_>| {
        let mut canceled = Vec::new();
        dispatch_entity_tick_pre(
            HostSide::Server(&mut TestHost),
            "minecraft:overworld",
            &[1, 2, 3],
            &["minecraft:pig"; 3],
            &mut canceled,
            listeners,
        );
        canceled
    };
    let set = ModSet::of([0, 2]);
    assert_eq!(told(Listeners::All), [1, 2, 3]);
    assert_eq!(told(Listeners::Only(&set)), [1, 3]);
    assert_eq!(told(Listeners::Except(&set)), [2]);
    assert_eq!(told(Listeners::One(1)), [2]);
}
