use super::*;
use mod_api::prelude::RecipesClientHost;
use mod_api::{FluidStack, RecipeLayout, RecipePage, RecipeResult, RecipeView, Stack};
use std::cell::RefCell;

/// A window that records each page it was asked to show.
#[derive(Default)]
struct Client {
    shown: RefCell<Vec<(String, Option<RecipePage>)>>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl RecipesClientHost for Client {
    fn panel_recipes(&mut self, mod_id: &str, page: Option<RecipePage>) {
        self.shown.borrow_mut().push((mod_id.to_owned(), page));
    }
}
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(recipes));

fn keys() {
    install_counter();
    // The fixture mod's own requests, and the page's items.
    mod_api::__private::resolve_items([
        "minecraft:stone",
        "minecraft:emerald",
        "minecraft:iron_helmet",
        "minecraft:amethyst_shard",
        "minecraft:oak_log",
        "minecraft:oak_planks",
    ])
    .unwrap();
    mod_api::__private::resolve_fluids(
        &["minecraft:water", "minecraft:lava"],
        &sunrise_world::fluid::FluidKind::ALL.map(|kind| kind.name()),
    )
    .unwrap();
}
fn stack(item: &str, count: u32, components: &str) -> Stack {
    Stack {
        item: mod_api::__private::item_key(item).unwrap(),
        count,
        components: components.into(),
    }
}

/// Every field of a page set, every layout used, a chance and a missing one, both action states.
fn page() -> RecipePage {
    let log = stack("minecraft:oak_log", 1, "");
    let planks = stack("minecraft:oak_planks", 4, "[minecraft:custom_name=\"P\"]");
    let water = FluidStack::new(
        mod_api::__private::fluid_key("minecraft:water").unwrap(),
        1000,
    );
    let lava = FluidStack::new(
        mod_api::__private::fluid_key("minecraft:lava").unwrap(),
        250,
    );
    let layouts = [
        RecipeLayout::Grid,
        RecipeLayout::Cooking,
        RecipeLayout::Single,
        RecipeLayout::Smithing,
        RecipeLayout::Brewing,
    ];
    let mut recipes: Vec<RecipeView> = layouts.into_iter().map(RecipeView::new).collect();
    recipes[0] = RecipeView::grid(2, 3)
        .with_slots(vec![
            vec![log.clone(), planks.clone()],
            Vec::new(),
            vec![log.clone()],
        ])
        .with_results(vec![
            RecipeResult::new(vec![planks.clone()]),
            RecipeResult::new(vec![log.clone()]).with_chance(12.5),
        ])
        .with_fluids(vec![water], vec![lava])
        .with_note("10 s")
        .with_action(false);
    recipes[1] = recipes[1].clone().with_action(true);
    RecipePage::new("Oak Planks", recipes).with_tabs(vec![log, planks], 1)
}

#[test]
fn a_recipe_page_round_trips_on_the_wire() {
    keys();
    let page = page();
    let mut writer = Writer::new();
    writer.put(&page);
    assert_eq!(Reader::decode::<RecipePage>(writer.bytes()).unwrap(), page);
    let mut writer = Writer::new();
    writer.put(&None::<RecipePage>);
    assert_eq!(
        Reader::decode::<Option<RecipePage>>(writer.bytes()).unwrap(),
        None
    );
}

#[test]
fn a_wasm_mods_panel_recipes_reaches_the_client_with_its_arguments() {
    keys();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client::default();
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the client and its frame outlive this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    guest::Guest.panel_recipes("wire_test", Some(page()));
    guest::Guest.panel_recipes("wire_test", None);
    guest::set_transport(None);
    assert_eq!(
        *client.shown.borrow(),
        [
            ("wire_test".to_owned(), Some(page())),
            ("wire_test".to_owned(), None)
        ]
    );
}

/// What the panel tells its owner crosses the wire whole, a drop's stack and point included.
#[test]
fn panel_events_and_a_drop_round_trip() {
    use mod_api::PanelEvent;
    keys();
    let stone = stack("minecraft:stone", 3, "[minecraft:custom_name=\"F\"]");
    for event in [
        PanelEvent::Clicked {
            index: 7,
            right: false,
        },
        PanelEvent::Search("iron".into()),
        PanelEvent::Tab(2),
        PanelEvent::Dropped {
            stack: stone.clone(),
            x: 41.5,
            y: -3.25,
        },
    ] {
        let mut out = Writer::new();
        out.put(&event);
        assert_eq!(Reader::decode::<PanelEvent>(out.bytes()).unwrap(), event);
    }
}
