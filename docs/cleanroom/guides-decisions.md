# Built-in mod guides: the implementer's decisions

Answers to the open questions of [guides-spec.md](guides-spec.md) (revision 2) and to the implementer's own questions
(`questions/Q-055.md` on), each with the test that settles it. Every decision was taken from the spec, Minecraft 26.3,
Sunrise's own code and first principles; none from another implementation. Tests named `guides::tests::...` are in
`crates/sunrise-client/src/guides/tests.rs` (`cargo test -p sunrise-client-lib --lib guides::`).

Decisions not listed here are left for later pull requests (see "Not decided yet" at the end).

## Discovery and front matter

- **Q-001** `assets/<ns>/ae2guide/` is guide `<ns>:guide`. Test: `discovery_finds_both_layouts_and_skips_bad_names`.
- **Q-002** A namespace with both layouts makes one guide, `<ns>:guide`. When both give the same page path, the
  `guides/` copy wins: it is the documented location, so an addon or pack writing there means to replace. Test:
  `both_layouts_merge_and_untitled_pages_take_their_file_name`.
- **Q-003** `default_language` is read at the top level of the definition file. Test:
  `discovery_finds_both_layouts_and_skips_bad_names` (its definition file's `custom_colors` are read from the same
  level).
- **Q-004** A guide shows its namespace as its name until the mod list's names reach the viewer (the first pull request
  has no mod display names: every guide shows its namespace, except where a mod name is known). Test:
  `discovery_finds_both_layouts_and_skips_bad_names` (`ironfurnaces`, and AE2's name when it is given).
- **Q-005** In the `guides/` layout a page's id takes the **guide's** namespace (`<gns>:<path>`), whatever namespace
  folder the file is in. When several namespace folders give the same page, the guide's own namespace's copy wins,
  then the others by namespace. Test: `discovery_finds_both_layouts_and_skips_bad_names` (`ironfurnaces:furnaces.md`).
- **Q-006** `icon_nbt` is ignored on 26.3: the old item NBT has no reliable component equivalent. The icon shows the
  plain item.
- **Q-007** A top-level `icon` is not a navigation icon (spec 3.4: a key at the wrong level is unknown). Test:
  `front_matter_absent_unclosed_or_broken`.
- **Q-008** A page with neither `navigation.title` nor a level-1 heading is titled by its file name without `.md`.
  Test: `both_layouts_merge_and_untitled_pages_take_their_file_name`.
- **Q-041** A translation without a default page is ignored. Test: `translations_replace_pages_and_assets_with_fallback`.
- **Q-055** (own question) A guide path in the `guides/` layout is one folder: `guides/<gns>/<gpath>/` with `<gpath>`
  a single name, like the definition files' names. Deeper folders are folders of pages. Test:
  `discovery_finds_both_layouts_and_skips_bad_names`.

## Markdown and tags

- **Q-009** Autolinks `<https://...>` are read (they cost nothing and are CommonMark); task lists and footnotes are
  not. Code: `parse_inlines`.
- **Q-010** Code blocks are drawn in the code colour on a dark band and wrap at the page width rather than scroll.
  Test: `blocks_headings_lists_quotes_code_rules` (parsing); the drawing is `layout.rs`, `Block::Code`.
- **Q-011** `"true"`, `"false"` and numbers written as strings are accepted wherever braces are documented. Test:
  `tags_blocks_inline_attributes_and_nesting` (`scale={2}`, `gap="8"`, a bare `fullWidth`).
- **Q-012** A `<div>` with a `class` is a note box (tinted, with a bar on the left); without one it is a plain block.
- **Q-013** An unknown tag shows its error only, not its children. Test: `unknown_tags_show_an_error_without_children`.
- **Q-014** A lowercase `<name>` that is not `a`, `br` or `div` is text. Test:
  `comments_unknown_lowercase_and_unclosed_tags` (`/ae2 channelmode <mode>` stays text).
- **Q-016** Heading anchors: the heading's plain text (formatting removed) lowercased; letters (any script), digits,
  `-` and `_` kept; whitespace becomes `-`; other characters dropped; the second heading with the same anchor gets
  `-1`, the third `-2`. Test: `heading_anchors_drop_punctuation_and_number_repeats`.
- **Q-022** Category names match exactly (case and spaces). Test: `categories_list_matching_pages_by_title`.
- **Q-023** `CategoryIndex` lists pages by title. Test: `categories_list_matching_pages_by_title`.
- An unclosed tag shows its error and the page goes on after it (spec 10.1). Test:
  `unclosed_tags_dont_break_the_page`.

## Links, items and images

- **Q-015** A link to a page hidden by its required mods is drawn in the error colour, does nothing when clicked, and
  its tooltip names the mod ("Needs the mod ae2"). Test:
  `requires_mods_hides_pages_and_item_lookup_prefers_the_items_guide`.
- **Q-017** Images are drawn at their size in GUI pixels, scaled down to the page width, never enlarged. PNG only in
  this first version (they go into the GUI atlas's dynamic texture pool, up to 1024 pixels a side); a JPEG or an image
  that does not fit shows a grey box. Test: `floating_images_push_text_aside_until_cleared` (size and placement).
- **Q-018** `scale="1"` is 16 GUI pixels for `ItemImage`. Test: `rows_place_children_side_by_side_and_recipes_wrap`
  (scale 2 is 32 pixels).
- **Q-019** Clicking an item image, icon or item link opens its page, as W does. Test:
  `item_links_name_items_and_link_their_pages` (the hotspot's action).
- **Q-021** `BlockImage` shows the tooltip of the block's item when it has one.
- **Q-024** `CommandLink` asks no confirmation; the command goes through the normal path a clicked chat command takes
  (`click_command`), so the server checks it.
- **Q-025** The viewer is dark. The sixteen colour names use Minecraft's text colours, and `custom_colors` use their
  `dark_mode` value. Test: `discovery_finds_both_layouts_and_skips_bad_names` (both values read).
- **Q-047** Several pages claiming one item: the page of the guide whose namespace is the item's, else the first by page
  id. Test: `requires_mods_hides_pages_and_item_lookup_prefers_the_items_guide`.

## Recipes

- **Q-026** `RecipeFor` shows the first recipe making the item in the client's recipe tables' order: the recipe book's
  order, then campfire, stonecutting, smithing and brewing (`Game::made_recipes`).
- **Q-027** A recipe that the tables don't have shows `fallbackText`, or the "No recipe" error. A type the tables hold
  is always drawn with the item panel's layouts (grid, cooking, single, smithing, brewing). Test:
  `rows_place_children_side_by_side_and_recipes_wrap` (`fallbackText`).
- **Q-053** Recipes come from the same tables the item panel's recipe pages use. On a multiplayer server these are the
  recipe book's entries the server sent, plus campfire, stonecutting and smithing; a recipe the client never received
  shows `fallbackText` or the error.

## Scenes

- **Q-028** `background` takes `transparent` (the default) and `#RRGGBB` or `#AARRGGBB`; anything else is transparent.
  Test: `scene_children_run_in_document_order`.
- **Q-029** Structure files follow the language fallback like images. Code: `Library::asset`, used by both.
- **Q-030** Of several `palettes`, the first is shown. Test: `binary_nbt_structures_gzipped`.
- **Q-031** `.snbt` files in the `blocks` + palette index layout are read. Test: `snbt_structures_in_both_layouts`.
- **Q-032** Old `DataVersion`s are not upgraded in this first version: block ids and states are read as written, and a
  state the game doesn't know is left out of the scene (logged).
- **Q-033** Block entity data is kept with each block but not yet drawn: blocks show their block model only. Modded
  block entities and signs need the scene drawing's block-entity input, a follow-up.
- **Q-034** `rotationX` defaults to 0 and is the head's pitch. Entities are parsed (test:
  `scene_children_run_in_document_order`) but not yet drawn in this first version.
- **Q-035** Both `RemoveBlocks` and `RemoveBlock` are accepted. Test: `scene_children_run_in_document_order`.
- **Q-036** A scene is drawn at 8 GUI pixels a block times `zoom`, its box the blocks' projected size plus 8 pixels
  each side (at least 32 high), scaled down to fit the page width. Test: `scenes_are_framed_and_fit_the_page`.
- **Q-037** Interactive scenes: drag with the left button (a degree a pixel; pitch kept within -90 to 90), the wheel
  zooms by 10 % a notch from a quarter to four times the page's zoom, and a reset control in the box's top-right
  corner returns to the page's camera. `IsometricCamera`'s `roll` is read but not drawn (the scene view turns about
  two axes).
- **Q-038** Annotations draw and show their tooltips in every scene, interactive or not; they are drawn over the blocks
  (`alwaysOnTop` is read, and every annotation is on top for now).
- **Q-039** Not measured in this pull request (no AE2 files are read by the implementation team); scenes over 4096
  blocks or 16 entities show the error in their box. Test: `scenes_over_the_limit_or_missing_files_report`.

## Cross-mod content

- **Q-040** "Installed": a loaded mod's id, or a namespace with items in the client's registries; `minecraft` always.
  The page-level key is `requires_mods: [id, "!id"]`. Tests: `optional_sections_follow_installed_mods`,
  `requires_mods_hides_pages_and_item_lookup_prefers_the_items_guide`.

## The viewer

- **Q-042** W over an item without a page does nothing.
- **Q-043** W is the physical W key (its position, so AZERTY's Z key), pressed once, fixed (not a setting) in this
  first version. No tooltip hint line yet.
- **Q-044** Not decided yet: the viewer opens from an item only.
- **Q-045** A page without `position` sorts as `position: 0`. Test:
  `navigation_tree_order_depth_and_cross_guide_parents`.
- **Q-046** History and the open sidebar entries last while the game runs, across worlds.
- **Q-048** Level-1 headings are bold at twice the size; level 2 bold and white with a rule under it, as level 1;
  levels 3 to 6 bold.
- **Q-049** Results: pages matching in their title first, then by the number of matches, then by title. The excerpt is
  up to 90 characters from 30 before the first match. A result opens the page at the heading its first match is under.
  Test: `search_matches_word_prefixes_titles_first`.
- **Q-050** Every query word must match the start of a word; letters of scripts written without spaces (CJK, kana,
  Hangul, Thai) are each a word. No stemming. Test: `search_matches_word_prefixes_titles_first`.
- **Q-051** No client command in this first version.
- **Q-052** Behind the `sunrise-extras` Cargo feature, with no setting: with no guide installed nothing changes, and W
  over an item on a container screen does nothing in vanilla, so the game looks and plays as vanilla.
- **Q-054** W over a page link or plain page text does nothing.
- **Q-056** (own question) The viewer draws over the container screen it was opened from, which stays open (its
  menu too), so the world runs on as under a container screen. The viewer takes every key, click and wheel notch
  while it is open; when the container closes underneath (the server closed it), the viewer closes with it.

## Not decided yet

Q-020 (bad `p:` properties are left at the state's defaults by the block state parser, without an error yet), Q-044,
Q-051, and the narration of pages (spec 9.5.5).
