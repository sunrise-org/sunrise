//! A `render_arm` mod draws in the first-person hand: its `textured_quad` is refused as a
//! `render_hand` mod's is, since the hand's vertices are drawn from the shared atlas by a pass of
//! their own, while its other draws still join the hand.
use mod_api::{
    ClientHost, Flow, Hooks, Identifier, ModEntry, Mods, RenderArmEvent, RenderHandEvent,
    RenderTypeRef,
};
use std::sync::{Arc, Mutex};
use sunrise_client::gfx::{
    DYNAMIC_TEXTURE_CUTOUT_OVERLAY, DYNAMIC_TEXTURE_SOLID_OVERLAY,
    DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY,
};
use sunrise_client_render::mod_pipelines::{BuiltinRenderType, Pipelines};

/// One quad a block wide, a block in front of the arm's origin.
const MARK: [[f32; 3]; 4] = [
    [0.0, 1.0, -1.0],
    [0.0, 0.0, -1.0],
    [1.0, 0.0, -1.0],
    [1.0, 1.0, -1.0],
];
const UNIT_UV: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];

/// What the listeners draw with, and what each call returned: (`textured_quad`, `quad`), the
/// arm's and the hand's.
#[derive(Default)]
struct State {
    render_type: Mutex<Option<RenderTypeRef>>,
    arm: Mutex<Vec<(bool, bool)>>,
    hand: Mutex<Vec<(bool, bool)>>,
}

/// `textured_quad` then `quad` of `MARK` in `draw`, and what each returned.
fn draw_both(
    draw: &mut dyn mod_api::BlockEntityDrawContext,
    render_type: RenderTypeRef,
) -> (bool, bool) {
    let textured = draw.textured_quad(render_type, MARK, UNIT_UV, 0xFF_FF_FF);
    let plain = draw.quad("block/stone", MARK, [0.0, 0.0, 16.0, 16.0], 0xFF_FF_FF);
    (textured, plain)
}

struct Mod(Arc<State>);

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn render_arm(&self, _: &dyn ClientHost, event: &mut RenderArmEvent<'_>) -> Flow {
        let render_type = self.0.render_type.lock().unwrap().unwrap();
        let drawn = draw_both(event.draw, render_type);
        self.0.arm.lock().unwrap().push(drawn);
        Flow::Continue
    }
    fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
        // Only the hand that holds the sword.
        if event.render.stack.is_some() {
            let render_type = self.0.render_type.lock().unwrap().unwrap();
            let drawn = draw_both(event.draw, render_type);
            self.0.hand.lock().unwrap().push(drawn);
        }
        Flow::Continue
    }
}

/// The empty main hand's arm asks `render_arm`: the listener's `textured_quad` of a live dynamic
/// texture returns false and puts nothing in the hand, while its `quad` returns true and joins
/// the hand ahead of the arm. A hand holding a sword asks `render_hand`, refused the same way.
/// The same render type draws outside the hand, so the refusal is the hand and arm draws'.
#[test]
#[ignore = "daily"]
fn a_render_arm_textured_quad_is_refused_as_a_hand_draw() {
    let state = Arc::new(State::default());
    let mods: Mods = vec![ModEntry::new("arm-texture", Box::new(Mod(state.clone())))].into();
    let mut drawn = None;
    mod_platform::testing::with_mods(mods, || {
        // The prepared data the client game boots from (its block, item and render tables, the
        // player model's assets), checked before the boot reads it.
        let root = crate::prepared_test_root::prepared_root(&[
            sunrise_core::prepared_test_support::STREAMING_INPUTS,
            sunrise_core::prepared_test_support::SERVER_INPUTS,
            &["entity-models.json", "gameplay-facts.json"],
        ]);
        crate::game::native_entity_refs::tests::with_game_role_client(
            root,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |game| {
                let mut render = sunrise_client_render::assets::RenderAssets::unpublished();
                render.mod_pipelines = Pipelines::load(
                    &sunrise_assets::resource_source::ResourceSource::directory(
                        "/nonexistent/sunrise-test-assets".into(),
                    ),
                    7,
                )
                .unwrap();
                game.client.client_pack.render = Arc::new(render);
                let texture = Identifier::parse("arm-texture:dynamic/panel").unwrap();
                game.client.dynamic_textures.bind(1, 64);
                game.client
                    .dynamic_textures
                    .create("arm-texture", &texture, [2, 2], 1, &[0x7f; 16])
                    .unwrap();
                let render_type = game
                    .client
                    .client_pack
                    .render
                    .textured_render_type(BuiltinRenderType::EntitySolid, &texture)
                    .unwrap();
                *state.render_type.lock().unwrap() = Some(render_type);
                // Outside a hand draw the same quad draws, naming its texture.
                let mut outside = crate::game::client_side::native_block_entity_draw::tests::draw();
                assert!(game.native_draw_textured_quad(
                    &mut outside,
                    render_type,
                    MARK,
                    UNIT_UV,
                    0xFF_FF_FF
                ));
                let outside = outside.into_vertices();
                assert_eq!(outside.len(), 4);
                assert!(
                    outside
                        .iter()
                        .all(|v| v.layer == DYNAMIC_TEXTURE_SOLID_OVERLAY)
                );
                crate::game::client_side::item_use_client::tests::stand_in_first_person(game);
                let arm = game.hand_geometry(0.0);
                // A sword in the main hand: `render_hand` is asked, `render_arm` is not.
                let sword = sunrise_items::items::ItemStack::new("minecraft:iron_sword", 1);
                game.common.inventory[game.common.selected] = Some(sword);
                game.client.visible_hands[0].1 = game.inventory[game.selected].clone();
                drawn = Some((arm, game.hand_geometry(0.0)));
            },
        );
    });
    let ((arm, arm_runs), (held, _)) = drawn.unwrap();
    // The arm asked once, the hand once: the textured quad refused, the plain one drawn.
    assert_eq!(*state.arm.lock().unwrap(), [(false, true)]);
    assert_eq!(*state.hand.lock().unwrap(), [(false, true)]);
    let dynamic = |v: &sunrise_client::mesher::Vertex| {
        matches!(
            v.layer & 0xffff,
            DYNAMIC_TEXTURE_SOLID_OVERLAY
                | DYNAMIC_TEXTURE_CUTOUT_OVERLAY
                | DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY
        )
    };
    for hand in [&arm, &held] {
        assert!(hand.len() > 4);
        assert!(!hand.iter().any(dynamic));
    }
    // The arm's run, with no rendered item id: the stone quad first, then the arm.
    assert_eq!(arm_runs, [(0, 0)]);
}
