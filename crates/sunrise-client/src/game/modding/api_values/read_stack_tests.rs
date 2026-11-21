use super::*;

#[test]
fn read_encoding_keeps_nonresident_stack_limits() {
    let items = super::stack_limit_tests::catalog();
    let key = mod_api::__private::item_key("minecraft:iron_sword").unwrap();
    let mut damaged = ApiStack::of(key);
    damaged
        .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(250))].into())
        .unwrap();
    damaged.set_damage(251);
    let mut removed = ApiStack::of(key);
    removed
        .set_components(
            [(
                JavaString::from("!minecraft:max_damage"),
                Tag::Compound(Default::default()),
            )]
            .into(),
        )
        .unwrap();
    removed.set_damage(1);
    let mut excess = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
    excess.set_count(65).unwrap();
    for api in [damaged, removed, excess] {
        assert!(api.try_into_engine(&items).is_err(), "write remains strict");
        let encoded = stack_value(&api).unwrap();
        assert_eq!(encoded.count, api.count());
        assert_eq!(encoded.damage, api.damage());
        assert_eq!(encoded.try_into_api().unwrap(), api);
    }
}

#[test]
#[ignore = "daily"]
fn draw_consumers_render_nonresident_stacks() {
    use crate::game::host::draw::{GameBlockEntityDrawContext, GameDrawContext};
    use mod_api::{BlockEntityDrawContext, DrawContext, ItemDisplayContext};
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let mut excess = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        excess.set_count(65).unwrap();
        let mut damaged =
            ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        damaged.set_damage(251);
        let mut removed = damaged.clone();
        removed
            .set_components(
                [(
                    JavaString::from("!minecraft:max_damage"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        removed.set_damage(1);
        for api in [excess, damaged, removed] {
            assert!(
                api.try_into_engine(game.items()).is_err(),
                "not a resident write"
            );
            let mut gui = crate::hud::HudBatch::scratch([320, 240]);
            {
                let mut context = GameDrawContext::new(game, &mut gui);
                assert!(context.item_stack(&api, [16.0, 32.0], 16.0));
            }
            assert!(!gui.vertices.is_empty(), "item_stack renders geometry");
            let mut decorated = crate::hud::HudBatch::scratch([320, 240]);
            GameDrawContext::new(game, &mut decorated).gui_draw_item_with(&api, [16.0, 32.0], None);
            assert!(
                !decorated.vertices.is_empty(),
                "gui_draw_item_with renders geometry"
            );
            let mut world = crate::game::native_block_entity_draw::BlockEntityDraw::posed(
                0.0,
                0,
                0,
                [0, 64, 0],
                glam::Mat4::IDENTITY,
                (glam::Vec3::X, glam::Vec3::Y),
                Vec::new(),
            );
            assert!(GameBlockEntityDrawContext::new(game, &mut world).item(
                &api,
                ItemDisplayContext::Fixed,
                None,
                0
            ));
            assert!(
                !world.into_vertices().is_empty(),
                "block item renders geometry"
            );
        }
    });
}
