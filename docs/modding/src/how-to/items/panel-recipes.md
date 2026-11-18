# Show recipes in the item panel

A recipe viewer shows its pages in the item panel's place with `panel_recipes(mod_id, page)`, a
client `WorldOnly` call beside `panel_items` and `panel_style`. The page is data: the game draws it
with its own slots, arrows, fluid wells and buttons, for native and WebAssembly mods alike. It is
the same recipe view the game's own panel uses, so a mod never draws a second recipe window.

```rust
use mod_api::{RecipeLayout, RecipePage, RecipeResult, RecipeView};

fn show_planks(host: &mut dyn mod_api::ClientHost, log: mod_api::Stack, planks: mod_api::Stack) {
    let craft = RecipeView::grid(1, 1)
        .with_slots(vec![vec![log.clone()]])
        .with_results(vec![RecipeResult::new(vec![planks])]);
    let smelt = RecipeView::new(RecipeLayout::Cooking)
        .with_slots(vec![vec![log]])
        .with_note("10 s")
        .with_action(true);
    let page = RecipePage::new("Oak Planks", vec![craft, smelt]);
    host.panel_recipes("my_viewer", Some(page));
}
```

`None`, or a page with no recipes, shows the mod's items again. Showing the same title and tab
again keeps the page the player turned to.

## What a page holds

- `RecipePage`: a `title` beside the back button, a row of `tabs` (each an icon stack; `tab` is the
  selected one, set with `with_tabs`) and the `recipes`.
- `RecipeView`: a `layout`, the input `slots` (each a list of stacks the slot cycles through, one a
  second), up to four `results`, the fluid wells (`with_fluids`, amounts in millibuckets), a `note`
  line and an optional action button (`with_action(enabled)`; `false` draws it greyed).
- `RecipeLayout`: `Grid` (`width` x `height`, up to 3 x 3; slots past the grid continue in rows under
  it), `Cooking` (one input over a flame), `Single` (the stonecutter), `Smithing` and `Brewing`.
- `RecipeResult`: the stacks a result slot cycles through and an optional chance in percent, drawn
  under it ("25%").

## Limits

The renderer's limits are cut, never refused, and a call never panics:

| Item | At most |
|---|---|
| Recipes on a page | 256 (`RecipePage::MAX_RECIPES`) |
| Tabs | 16 (`RecipePage::MAX_TABS`) |
| Input slots in a recipe | 27 (`RecipeView::MAX_SLOTS`) |
| Results in a recipe | 4 (`RecipeView::MAX_RESULTS`) |
| Fluid wells, each way | 8 (`RecipeView::MAX_FLUIDS`) |
| Stacks a slot or result cycles | 64 (`RecipeView::MAX_STACKS`) |

A grid's `width` and `height` are kept to 1 to 3, and a chance to 0 to 100; a chance that isn't
finite draws none. A stack of an item the game doesn't know is left out of its slot's cycle.

## What the player does

The page's events come back through `panel_event`, as the items' do:

- `PanelEvent::SlotClicked { recipe, slot, stack, right }`: a click on a slot. `recipe` is the
  recipe's index in the page's `recipes`; results are numbered after the slots.
- `PanelEvent::Action(recipe)`: the recipe's action button.
- `PanelEvent::Page(first)`: the player turned the page; `first` is the first recipe now shown.
- `PanelEvent::Back`: the back button. The mod chooses what follows, often `panel_recipes(mod_id,
  None)`.
- `PanelEvent::Tab(index)`: the player picked a tab.

## Dragging an item onto the screen

The player can drag an item off the panel's item list and drop it on the open screen, as JEI's
ghost ingredients do, to set a filter or configuration slot to an item they don't hold. A press on
an item becomes a drag once the pointer moves 4 GUI pixels with the left button held; the item
follows the pointer until the button comes up. A plain click stays a click: a left click's
`PanelEvent::Clicked` comes when the button comes up without having dragged (a right click, or a
left one while the player carries a stack, still comes as the button goes down).

- Let go outside the panel: the owner hears `PanelEvent::Dropped { stack, x, y }`, the item as the
  mod sent it and the point in GUI pixels from the window's top left (`ScreenView::mouse`'s
  frame). No slot of the screen hears that release: nothing is picked up, placed or thrown.
- Let go back on the panel, press Escape, or close the screen: the drag is cancelled and nothing is
  sent. Escape during a drag leaves the screen open.

The game moves nothing for a drop. The viewer finds what lies under `x, y` (a slot of a menu it
knows, or a target another mod published), forwards the drop to the mod that owns it, and that mod
checks the change on its own server; a client never decides a slot's contents.

```rust,ignore
fn panel_event(&self, host: &mut dyn mod_api::ClientHost, event: &mut mod_api::PanelEventEvent<'_>) {
    if event.mod_id != "my_viewer" {
        return;
    }
    if let mod_api::PanelEvent::Dropped { stack, x, y } = event.event {
        // Ask the target under (x, y) to take `stack` as a ghost: the target's mod sends it to
        // its server, which sets the filter slot if the player may.
        let _ = (host, stack, x, y);
    }
}
```

## Marking a recipe's cells

A viewer's "+" asks `transfer_recipe_check(recipe)` first. Its `RecipeTransferCheck` names the
recipe's cells, in the order of the recipe's slots (width by height for a shaped recipe, else its
ingredients in order): `missing_cells`, which nothing fills (drawn red, as JEI does), and
`craftable_cells`, which the open menu lacks but could craft (drawn blue: a storage network's
autocrafting). The game's own grid transfer names missing cells with `Missing` and never craftable
ones; a mod menu's `menu_transfer` part names both, with any answer (see
[Menu slots](../../concepts/capabilities/menu-slots.md#recipe-transfer-into-a-mod-menu)).

Looking recipes up stays the viewer's job: it reads them (`recipes_of_type`) and builds the pages.
A machine mod with its own recipe types supplies its pages through the viewer's published
integration; the engine has no hook between the two.
