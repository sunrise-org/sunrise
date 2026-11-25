/// Install rail step parts: a plain rail laid east-west sends a cart east at its top speed,
/// and a detector rail answers a non-finite speed.
pub fn rail_step_game_thread() {
    struct Booster;
    #[mod_api::sunrise_part]
    impl BlockPart for Booster {
        fn rail_step(&self, _: &dyn LoopContext, event: &mut RailStepEvent<'_>) {
            if event.shape == RailShape::EastWest {
                event.velocity[0] = event.max_speed;
            }
        }
    }
    struct Broken;
    #[mod_api::sunrise_part]
    impl BlockPart for Broken {
        fn rail_step(&self, _: &dyn LoopContext, event: &mut RailStepEvent<'_>) {
            event.velocity[0] = f64::NAN;
        }
    }
    struct RailMod;
    #[mod_api::__subscriptions]
    impl Hooks for RailMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let rail = r.block_key("minecraft:rail");
            r.block_part(rail, Booster);
            let detector = r.block_key("minecraft:detector_rail");
            r.block_part(detector, Broken);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("rail_step_test", Box::new(RailMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
