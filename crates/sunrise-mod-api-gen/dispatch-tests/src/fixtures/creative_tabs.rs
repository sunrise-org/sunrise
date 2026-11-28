//! Creative tab declarations use the same per-mod ownership boundary as blocks.
#[test]
fn creative_tab_dispatch_records_each_owner_with_its_submission() {
    struct Declares;
    #[mod_api::__subscriptions]
    impl Hooks for Declares {
        fn creative_tabs(&self, _: &RegistrationContext, event: &mut CreativeTabsEvent<'_>) {
            event
                .register(mod_api::mod_api::CreativeTab { id: "tools".into() })
                .unwrap();
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Declares)),
            ModEntry::new("second", Box::new(Declares)),
        ]
        .into(),
    );
    DECLARATION_OWNERS.with_borrow_mut(Vec::clear);
    let mut creative_tabs = Vec::new();
    dispatch_creative_tabs(&RegistrationContext("bootstrap"), &mut |listen| {
        assert!(listen(&mut |creative_tab| {
            creative_tabs.push(creative_tab);
            Ok(())
        }));
    });
    assert_eq!(
        creative_tabs
            .iter()
            .map(|creative_tab| (creative_tab.mod_id, creative_tab.answer.id.as_str()))
            .collect::<Vec<_>>(),
        [
            ("first", "first/checked/tools"),
            ("second", "second/checked/tools")
        ]
    );
    DECLARATION_OWNERS.with_borrow(|calls| {
        assert_eq!(
            calls,
            &[
                ("first".into(), "tools".into()),
                ("second".into(), "tools".into())
            ]
        );
    });
}
