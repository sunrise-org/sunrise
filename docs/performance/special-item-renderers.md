# What a special item renderer costs a draw

A mod's special model renderer (`register_special_model_renderer`, #876) adds two costs to
drawing an item. The first time a model node sees a stack, the renderer is asked: the stack is
converted for the API, the answer is checked, its models are baked or found, and the prepared
answer is kept. For every later frame that draws the same unchanged stack, the frame hashes
the stack's view, does one map lookup and clones an `Arc`. Built-in items pay nothing more
while no renderer is registered, or while one is registered but no drawn model names its type.
Matrix row 13's test pins that (`cost_tests::built_in_scenes_cost_nothing_more_without_a_listener`).

## Measurements

Each figure is the time to resolve and submit one stone cube through the game's own route
(`Game::resolve_item`, then `submit_item_unlit`), drawn three ways:

- `built_in_model`: stone's own `minecraft:model`.
- `special_kept`: a registered `minecraft:special` whose renderer answers stone's cube. Its answer
  is already kept, and each sample starts a frame first (`begin_frame_draws`).
- `special_first_extraction`: the same special with its answers dropped before every draw, so
  the renderer is asked again each time.

The test is `game::native_special_renderers::cost_tests::special_item_draw_cost`: 10,000 samples
after 1,000 to warm up, three runs. Each special draw draws exactly as many vertices as the
built-in one.

| Draw | Median ns | p95 ns | p99 ns |
| --- | --- | --- | --- |
| `built_in_model` | 581 to 591 | 612 to 621 | 631 to 632 |
| `special_kept` | 591 to 601 | 621 to 631 | 641 to 642 |
| `special_first_extraction` | 1,183 to 1,203 | 1,242 to 1,262 | 1,292 to 1,293 |

An unchanged stack costs about 10 ns more per resolve and submit than the built-in model. Frame
starts and answer-cache eviction run before the timer and are not included. A stack's first draw, and its next draw after a frame that didn't draw it, costs about
twice a built-in draw. The renderer here does almost nothing, so a real one adds its own work to
that first draw only.

AMD Ryzen 5 7500F, Linux 6.8, rustc 1.99.1, the `fast` profile (`release` without LTO, 16 codegen
units). Branch `special-renderers-876-stage4`, whose runtime code is main's as of `0dbf78e35f`;
`SUNRISE_TEST_RESOURCES` was the prepared 26.3 resources. To repeat it:

```sh
SUNRISE_TEST_RESOURCES=$PWD/.local/sunrise-client/26.3 cargo test --profile fast -p sunrise-client --lib -- --ignored --nocapture --exact game::native_special_renderers::cost_tests::special_item_draw_cost
```
