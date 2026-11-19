# Built-in mod guides: specification

> **Revision 2, 2026-11-18.** Spec team: one AI agent session (Claude Opus 5.5). Revision 1 was discarded unread
> (see [guides-provenance-log.md](guides-provenance-log.md), entry 0). Feature request: sunrise-org/sunrise#2664.

This document describes, for an implementer who reads only this spec, Sunrise's own code and Minecraft 26.3:

- the **file format** of guide pages that mods already ship (the format GuideME documents for mod authors, and the
  layout AE2's guide uses), and
- the **behaviour** of Sunrise's built-in guide viewer.

It contains no code from any guide implementation. Examples of page markup below are written by the spec team to
illustrate the format; they are not copied from any guide.

Every statement carries a source tag in square brackets. The tags are defined in section 13. Statements tagged
**[SR-DESIGN]** are Sunrise's own choices taken from the feature request (#2664) or proposed by the spec team; they are
not facts about any existing guide system and may be changed in review. Points the sources leave open are listed in
section 12 as **Q-001** to **Q-054**; the implementer settles each in `decisions.md` (see [README.md](README.md)).

Words: a **guide** is one set of pages. A **page** is one Markdown file. A **guide root** is the folder holding a
guide's pages. A **tag** is an element written like `<Name attr="v">` inside a page. **Namespace** is a resource
namespace (`assets/<namespace>/...`).

---

## 1. Scope

1.1 Sunrise shows every installed mod's guide in one built-in viewer. There is no guide book item, no per-mod key and no
library dependency. Pages are read from assets only (mods' assets and resource packs). [ISSUE] [SR-DESIGN]

1.2 The feature is client-side only. Nothing is sent to or required from a server, except what the client already
receives (recipes, registries) and what Sunrise's Recipes panel already uses. [ISSUE]

1.3 A mod that ships no guide sees no change. [ISSUE]

1.4 Not in scope: GuideME's Java extension points (custom tags, custom recipe renderers, custom indices registered from
code), its website export, its live-preview tooling and its Patchouli converter. The spec team did not read those
pages (section 13.3). A page that uses a tag this spec does not define is handled by 4.12.

---

## 2. Where guides live

### 2.1 GuideME layout (`guides/` folders)

2.1.1 A guide has an id of the form `<namespace>:<path>`, for example `mymod:guide`. [G-DDG §Guide Definition]

2.1.2 The pages of guide `<gns>:<gpath>` are every file ending in `.md` under the folder
`guides/<gns>/<gpath>/` of **any** namespace of **any** loaded resource source (mod assets or resource packs), in that
folder and all its subfolders. Example: guide `mod:guide` reads `assets/<any>/guides/mod/guide/**/*.md`. [G-AUTH §intro]

2.1.3 Because pages are resources, file and folder names must be valid resource paths: lowercase, and only the
characters Minecraft allows in a resource path. A file whose name is not a valid resource path is skipped (with a log
warning). [G-AUTH §intro note] [MC]

2.1.4 Since pages come from all resource sources, a resource pack (or another mod) can add pages to a guide, or replace a
page by shipping a file at the same resource location; the usual resource-pack ordering decides which copy wins.
[G-OV] ("Allow resource packs or addons to add their own content") [MC]

2.1.5 Guide definition file (optional for Sunrise, see 2.3): `assets/<ns>/guideme_guides/<name>.json` declares guide
`<ns>:<name>`. [G-DDG §Guide Definition] Its documented keys:

| Key | Meaning | Sunrise |
|---|---|---|
| `item_settings` | Object describing the guide book item: `display_name` (a text component), `tooltip_lines` (a list of text components), `model` (an item model id). All optional. [G-DDG §Generic Guide Item] | Read and ignored: Sunrise has no guide item [SR-DESIGN] |
| `default_language` | The Minecraft language code the guide's default (untranslated) pages are written in. Used to pick language rules for search. [G-DDG §Default Language] | Used by 8.3 and 10.4 |
| `custom_colors` | Object mapping a colour id (`<ns>:<name>`) to `{ "dark_mode": "#RRGGBB", "light_mode": "#RRGGBB" }`. Pages of this guide may name these colours without the namespace. [G-DDG §Custom Colors] | Used by 5.11 |

Unknown keys are ignored. Where exactly `default_language` sits in the file is not shown by the documentation (Q-003).

### 2.2 AE2 layout (`ae2guide/` folders)

2.2.1 AE2 ships its guide as the folder `assets/ae2/ae2guide/` inside its jar: pages (`*.md`) at the top and in
subfolders (`ae2-mechanics/`, `items-blocks-machines/`, `example-setups/`), and an `assets/` subfolder holding images
(`.png`) and structure files (`.snbt`) that pages reference by relative path. Both the 1.20 build and the 26.3 build
use this layout. [AE2-15] [AE2-26]

2.2.2 Sunrise's ports carry their pages in this same layout under their own namespace: `assets/<ns>/ae2guide/`.
[ISSUE] [SR-DESIGN]

2.2.3 Rule for Sunrise: each folder `assets/<ns>/ae2guide/` is the guide root of the guide of namespace `<ns>`, and every
`.md` file below it is a page of that guide. Its guide id is decided in Q-001. No definition file is needed or read for
this layout. [SR-DESIGN]

2.2.4 How the two layouts combine when a namespace has both (or when `guides/<ns>/...` exists for an `ae2guide` mod) is
Q-002.

### 2.3 Which guides exist

2.3.1 A guide exists when at least one page for it is found. The definition file of 2.1.5 is not required for Sunrise to
show a guide (it only adds colours and the default language). [SR-DESIGN] (GuideME itself registers code-defined guides
from mods; that registration is out of scope, 1.4.)

2.3.2 The guide's display name in the viewer is the mod's display name as Sunrise's mod list shows it, falling back to
the namespace. [SR-DESIGN] (Q-004 for resource-pack-only guides.)

### 2.4 Page ids and paths

2.4.1 A page is identified by `<namespace>:<path>` where `<path>` is the file's path relative to its guide root,
including the `.md` extension, with `/` separators: e.g. `ae2:ae2-mechanics/channels.md`. Navigation parents in
front matter are written this way. [G-AUTH §Adding Pages to the Navigation Bar] [AE2-26]

2.4.2 Which namespace a page id carries in the `guides/` layout when the file lives in a different namespace from the
guide id is not stated (Q-005). For `ae2guide/` it is the folder's `<ns>`.

2.4.3 Translated copies (section 8) are not separate pages; they replace the default page's content.

2.4.4 Files under the guide root that are not `.md` (images, structures) are **guide assets**, addressed by a path
relative to the page that uses them (5.4, 6.2). The documentation also mentions putting images in an `assets` folder of
the guide; AE2 does so (`assets/...` below the guide root). Nothing requires that folder name. [G-AUTH §Using Images]
[AE2-26]

---

## 3. Front matter

3.1 A page may begin with a YAML front matter block: a line `---`, YAML, and a closing `---` line. Everything after
the block is the page body. A page without front matter is valid. [G-AUTH §Frontmatter] [G-MD §Specifications]

3.2 Keys and meanings:

| Key | Type | Meaning | Source |
|---|---|---|---|
| `navigation` | map | Puts the page into the contents tree (section 9.2). Absent: the page is not in the tree but can still be reached by links, item lookups, search and `SubPages`/`CategoryIndex`. | [G-AUTH §Adding Pages…] |
| `navigation.title` | string | The page's title in the contents tree (and in lists of pages). | [G-AUTH] |
| `navigation.icon` | item id | Optional icon shown beside the title. An id without namespace takes the page's namespace. | [G-AUTH] |
| `navigation.icon_components` | map: component id → SNBT value | Optional data components for the icon stack, as the item-stack syntax of commands. Seen in AE2 26.x: a map with one entry whose key is a quoted component id and whose value is an SNBT/id string. | [AE2-26] [W-ARG] |
| `navigation.icon_nbt` | string (SNBT) | 1.20-era form: the icon stack's NBT tag as an SNBT string. Seen in AE2 1.20 only. | [AE2-15], Q-006 |
| `navigation.parent` | page id | Optional parent page in the tree. Same-namespace parents may omit the namespace (a path such as `getting-started.md` or `folder/page.md`, relative to the guide root, not to the page). Otherwise `<ns>:<path>.md`. | [G-AUTH] [AE2-26] |
| `navigation.position` | number | Optional sort key among siblings, ascending; ties broken by title. | [G-AUTH] |
| `navigation.max_depth` | integer | How many levels below this page the tree shows (9.2.3). | [G-AUTH] |
| `item_ids` | list of item ids | The items this page is the main page for (9.4, 5.3). Ids without namespace take the page's namespace. | [G-AUTH §Declaring Pages as ItemLink targets] |
| `categories` | list of strings | Free-form category names; a page may be in any number of categories (5.8). | [G-AUTH §Category Index] |

3.3 Parent paths are relative to the guide root. Evidence: AE2 pages in `ae2-mechanics/` name their parent as
`ae2-mechanics/ae2-mechanics-index.md`. [AE2-26]

3.4 Unknown keys are ignored without error (GuideME lets code add custom keys, 1.4). A key placed at the wrong level is
an unknown key: AE2 has one page with `icon:` at the top level instead of under `navigation`; it has no icon. [G-OV]
[AE2-26] (Q-007)

3.5 The page title shown at the top of a page comes from the page body: pages are expected to begin with a level-1
heading (`# Title`). The front matter title is for the tree. [G-AUTH §intro] Where a page has no `navigation.title`, its
title in search results and lists is its first level-1 heading, else its file name (Q-008).

3.6 Malformed YAML: the page is still shown, without metadata, and the error is reported (4.13). [SR-DESIGN]

---

## 4. Markdown

4.1 Pages are CommonMark Markdown with these GitHub-Flavored extensions: **tables** and **strikethrough**; and YAML
front matter (section 3). Other GFM extensions (task lists, autolink literals, footnotes) are not part of the documented
subset. [G-MD §Specifications] (Q-009)

4.2 Inline formatting: `*italic*` / `_italic_`; `**bold**` / `__bold__`; `~~strike~~` / `~strike~`; `` `inline code` ``;
links `[text](target)`; images `![alt](src)`. [G-MD §Inline Formatting]

4.3 Blocks: ATX headings `#` to `######`; thematic break `---`; block quotes `>`; bullet lists (`-`) and ordered lists
(`1.`); GFM tables with a header row and alignment row; paragraphs. [G-MD] Fenced and indented code blocks are
CommonMark and are shown as monospace preformatted text (Q-010).

4.4 Headings define link anchors (5.2.3). A level-1 heading is the page title (3.5).

4.5 **Tags.** Inside the body, elements in an MDX/JSX-like syntax are allowed, both as blocks (on their own lines) and
inline within a paragraph: `<Name attr="value" />` (self-closing) or `<Name attr="value">children</Name>`. Children may
be Markdown text and other tags. [G-AUTH §Custom Tags] [G-SCN] Tag names are case-sensitive and start with a capital
letter for guide tags; lowercase HTML-like names also occur (4.11). [AE2-26]

4.6 **Attribute values** take these forms [G-AUTH] [G-SCN] [AE2-26]:

- a quoted string: `id="ae2:controller"`, `zoom="6"`, `scale="1.5"`;
- an expression in braces: `{true}`, `{false}`, `{4}`; only boolean and number literals are documented;
- a bare attribute name with no value, meaning `true`: `<ItemLink id="x" optional />`.

Numbers given as strings (`zoom="2.5"`) and as braces (`zoom={2.5}`) mean the same. Booleans: `{true}`, `"true"` and
a bare name are true (Q-011 for `"true"` as a string).

4.7 Attribute names may contain a colon: `p:<property>` sets a block state property (5.6, 6.3). [G-AUTH] [G-SCN]

4.8 **Ids in attributes.** Item, block, page and colour ids without a namespace take the namespace of the page they are
on, not `minecraft:`. A vanilla item on an AE2 page must be written `minecraft:stick`. [G-AUTH §Custom Tags] [AE2-26]
(all non-AE2 ids in AE2's pages carry `minecraft:`)

4.9 **Comments**: `{/* ... */}`, single- or multi-line, are removed and show nothing. [G-AUTH §Comments]

4.10 Line breaks and whitespace follow CommonMark inside paragraphs. Text inside a tag's children is laid out as
Markdown (paragraphs, inline formatting, links). [G-SCN §Annotations] (annotation content mixes text and tags)

4.11 **HTML-like elements seen in real pages** [AE2-15] [AE2-26] [G-AUTH]:

| Element | Meaning |
|---|---|
| `<br clear="all" />` | Ends the wrapping of text around all floating images above it (5.5). [G-AUTH §Using Images] `<br />` alone is a line break. |
| `<a name="x"></a>` | An anchor named `x` at this point of the page, usable as `page.md#x`. [AE2-26] |
| `<a href="page.md">text</a>` | A link, the same as `[text](page.md)`. Seen in AE2 1.20 pages. [AE2-15] |
| `<div class="...">…</div>` | A block container; AE2 uses one with a notification-like class at the top of a page. Show its children as a block; the class may style it as a highlighted note box (Q-012). [AE2-26] |

4.12 **Unknown tags** (not in sections 4.11, 5 and 6): show an error marker in place of the tag (4.13) and do not show its
children (Q-013). Text in a paragraph that merely looks like a tag but is not valid tag syntax (for example `<mode>`
inside a sentence about a command) is a known pitfall: AE2 has such text (`/ae2 channelmode <mode>`). It must not
break the page (Q-014).

4.13 **Errors.** Several sources say "an error will be shown" (unknown item in `ItemLink`, missing page). Sunrise shows an
error as a short red inline text naming the problem (e.g. the id), keeps rendering the rest of the page, and logs it
once per page load. [G-AUTH §Item Links] [SR-DESIGN]

---

## 5. Tags: text, links, items, layout

### 5.1 Links

5.1.1 `[text](target)` targets [G-MD §Inline Formatting] [AE2-26]:

| Target form | Meaning |
|---|---|
| `page.md`, `./page.md`, `folder/page.md`, `../folder/page.md` | Another page of the same guide, path relative to **the linking page's folder**. |
| `<ns>:path/page.md` | A page by full id, possibly in another guide (cross-guide link). Path is relative to that guide's root. |
| any of the above with `#anchor` | That page, scrolled to the anchor (5.2.3). |
| `#anchor` | An anchor on the current page. (Not observed in AE2; CommonMark-valid.) |
| `http://…`, `https://…` | An external web link. |

5.1.2 Clicking a page link navigates (9.3). Hovering a page link shows a tooltip with the target page's title.
[G-OV] ("quick information in link tooltips") [SR-DESIGN for the content]

5.1.3 Clicking an external link opens Minecraft's own "open this link?" confirmation, as a chat link does, honouring the
game's chat-links options. [MC] [SR-DESIGN]

5.1.4 A link to a page that does not exist is shown as normal text in the error colour, with a tooltip saying the page is
missing; it is reported per 4.13. [SR-DESIGN]

5.1.5 A link to a page that exists but is hidden by its required mods (section 7) behaves as a missing page, with a
tooltip naming the missing mod (Q-015). [SR-DESIGN]

### 5.2 Anchors

5.2.1 `page.md#name` scrolls the target page so the anchor is at the top of the view.

5.2.2 Explicit anchors: `<a name="name"></a>` (4.11).

5.2.3 Heading anchors: every heading has an anchor equal to its text lowercased with spaces replaced by `-`. Observed:
`## Dense Cable` is linked as `#dense-cable`, `# Crafting Storage` as `#crafting-storage`. [AE2-26] The handling of
punctuation, inline formatting and duplicate headings is Q-016.

5.2.4 An unknown anchor opens the page at the top, without an error. [SR-DESIGN]

### 5.3 `ItemLink`

`<ItemLink id="item_id" />` [G-AUTH §Item Links]

- Shows the item's translated name (from the stack built from `id` and `components`), as a link.
- Hover: the item's normal tooltip (as in an inventory), built from the same stack.
- Click: opens the page whose `item_ids` contains this item (9.4). If that page is the current one, nothing happens.
- Attributes:

| Attribute | Meaning |
|---|---|
| `id` | Item id; namespace defaults to the page's (4.8). Required. |
| `components` | Data components, same syntax as the component list of the `/give` command's item argument without the brackets: `name=value` entries separated by commas, `!name` to remove a default; values are SNBT. E.g. `components="rarity=epic"`. The tooltip and name use this stack. [W-ARG] |
| `optional` | When the item exists but no page claims it: `false` (the default for items of the page's own namespace) shows an error; `true` (the default for other namespaces) shows the name as plain text with the tooltip, no link and no error. |
| `fallback` | Text to show (no error) when the item id does not exist at all, e.g. an item of a mod that is not installed. Without it, an unknown item is an error (4.13). |

### 5.4 Images

5.4.1 `![alt](path.png)`: an image file (`.png` or `.jpg`) addressed relative to the page's folder (AE2:
`assets/logo.png` from the root page, `../assets/diagrams/x.png` from a subfolder page). Shown at its natural size in
GUI pixels, scaled down to fit the page width if wider (Q-017). The alt text is the tooltip and is what narration reads.
[G-AUTH §Using Images] [G-MD] [AE2-26]

5.4.2 Images are looked up first in the current language's folder, then the default (8.2). [G-TR]

5.4.3 A missing image shows the alt text in the error colour (4.13). [SR-DESIGN]

### 5.5 `FloatingImage`

`<FloatingImage src="path.png" align="left|right" />`: an image that floats to the left or right of the following
text, which wraps around it. `align="left"` puts the image on the left and text to its right. `<br clear="all" />` stops
the wrapping (subsequent content starts below all floating images). `src` is resolved as in 5.4. [G-AUTH §Using Images]

### 5.6 `ItemImage`, `ItemIcon`, `ItemGrid`, `BlockImage`

5.6.1 `<ItemImage id="…" />`: draws an item stack as in an inventory slot (without a slot background). [G-AUTH §Item Images]

| Attribute | Meaning |
|---|---|
| `id` | Item id, namespace defaults to the page's. |
| `components` | As in 5.3. |
| `scale` | Size factor, decimal allowed; `1` is the natural 16-pixel item size (Q-018). AE2 uses 2, 4 and 8. [AE2-26] |
| `float` | `left` or `right`: floats like `FloatingImage`. |

Hover shows the item's tooltip; W over it opens its page (9.4); click opens its page too (Q-019).

5.6.2 `<ItemGrid>` containing `<ItemIcon id="…" components="…" />` children: lays the icons out in a grid of
slot-sized cells, wrapping to the page width. `ItemIcon` takes `id` and `components` as in 5.3. Hover and W as 5.6.1.
[G-AUTH §Item Grids]

5.6.3 `<BlockImage id="…" />`: a 3D rendering of one block, as the block looks in the world (its block model; animated
textures animate). [G-AUTH §Block Images]

| Attribute | Meaning |
|---|---|
| `id` | Block id, namespace defaults to the page's. |
| `scale` | Size factor as for `ItemImage`. |
| `float` | `left` or `right`. |
| `perspective` | `isometric-north-east` (default: the block's north-east corner faces the viewer) or `isometric-north-west`. |
| `p:<name>` | Sets block state property `<name>` to the value, e.g. `p:powered="true"`. Unknown properties or values: the property is left at its default and an error is shown (Q-020). |

Hover shows the block's item tooltip if the block has an item (Q-021).

### 5.7 `Row` and `Column`

`<Row>…</Row>` lays its children out horizontally, wrapping to the next line when the page is too narrow;
`<Column>…</Column>` stacks them vertically. Attributes: `gap` (pixels between children, default 5); `alignItems`
(`start`, `center` or `end`: alignment across the layout axis, i.e. vertical in a `Row`, horizontal in a `Column`);
`fullWidth` (boolean: stretch to the full page width instead of shrinking to the content, which is needed to centre
content on the page). Recipe tags inside a `Row` sit side by side and wrap. [G-AUTH §Column / Row Layout, §Recipes]

### 5.8 `CategoryIndex`

`<CategoryIndex category="name" />`: a list of links to every page whose `categories` contains `name` (exact,
case-sensitive match; Q-022), with their titles. Order: Q-023. [G-AUTH §Category Index]

### 5.9 `SubPages`

`<SubPages />`: a list of links to the current page's children in the contents tree (9.2). Attributes: `id` (a page id:
list that page's children instead); `alphabetical` (boolean: sort by title instead of tree order); `icons` (boolean:
show each child's navigation icon). Children are listed even beyond the tree's depth limit. [G-AUTH §Sub Pages,
§Adding Pages…]

### 5.10 `CommandLink`

`<CommandLink command="/cmd args">text</CommandLink>`: a link that, when clicked, sends `command` from the client as if
the player typed it. It must start with `/`; it is subject to the server's normal permission checks. Attributes: `title`
(extra tooltip text; the tooltip always also shows the command); `close` (boolean: close the viewer when clicked).
[G-AUTH §Command Links] Sunrise asks no confirmation unless Q-024 decides otherwise.

### 5.11 `Color`

`<Color color="#RRGGBB">text</Color>` colours its children with a fixed colour. `<Color id="name">text</Color>` uses a
symbolic colour that has a light-mode and a dark-mode value. [G-AUTH §Colored Text]

Built-in symbolic colours: the sixteen Minecraft text colour names `black`, `dark_blue`, `dark_green`, `dark_aqua`,
`dark_red`, `dark_purple`, `gold`, `gray`, `dark_gray`, `blue`, `green`, `aqua`, `red`, `light_purple`, `yellow`,
`white`. [G-AUTH] Their exact light/dark values are not given in text by the documentation (Q-025). A guide may add
colours with `custom_colors` (2.1.5); ids without namespace take the page's namespace.

### 5.12 `PlayerName`, `KeyBind`

- `<PlayerName />`: the current player's name. [G-AUTH §Player Name]
- `<KeyBind id="key.jump" />`: the current binding of the key mapping with that id, shown as the Controls screen shows
  it (e.g. `Space`). Unknown id: error (4.13). [G-AUTH §Key Bindings] [MC]

### 5.13 `OptionalSection`

`<OptionalSection modId="x">…</OptionalSection>` shows its children only when mod `x` is installed;
`modId="!x"` shows them only when `x` is **not** installed. `modId` is required. [G-AUTH §Optional Sections] See
section 7 for what "installed" means in Sunrise.

### 5.14 Recipe tags

| Tag | Shows | Source |
|---|---|---|
| `<RecipeFor id="item" />` | The first recipe whose result is that item (Q-026 for "first"). | [G-AUTH §Recipes] |
| `<RecipesFor id="item" />` | Every recipe whose result is that item. | [G-AUTH] |
| `<Recipe id="ns:recipe/path" />` | The recipe with that id. | [G-AUTH] |

- Attribute `fallbackText`: text shown when no recipe is found. `fallbackText=""` shows nothing. Without it, no recipe
  is an error (4.13). [G-AUTH]
- Recipes are drawn as **live slots** from the game's recipe data, laid out like the recipe type's layout in Sunrise's
  Recipes panel: crafting (shaped and shapeless), smelting, blasting, smoking, campfire cooking, stonecutting, smithing,
  and any type Sunrise's Recipes panel can show (including ported mods' types it supports). [ISSUE] [SR]
- Each slot behaves as in the Recipes panel: ingredients that accept several items cycle through them; hover shows the
  stack's tooltip; W opens that item's guide page (9.4); clicking a slot opens the item's page as well. [G-AUTH §Declaring
  Pages…] ("slots in recipes that show that item" link to its page) [ISSUE]
- A recipe type Sunrise cannot draw is shown as a fallback box naming the recipe id and its result item (Q-027).

---

## 6. 3D scenes

### 6.1 `GameScene`

`<GameScene …>children</GameScene>` is a box on the page showing a small world of blocks and entities, drawn with
Sunrise's scene drawing (`DrawContext::draw_scene`, living scenes) so blocks keep their own looks, animated textures and
particles (furnace flames, smoke, dripping). [G-SCN] [ISSUE] [SR-LS]

| Attribute | Meaning | Source |
|---|---|---|
| `zoom` | Scale factor of the scene, default 1. Decimals allowed (AE2 uses 2 to 8, also 2.5). | [G-SCN] [AE2-26] |
| `background` | Background colour of the scene box. `transparent` is used by AE2 (no background). Other accepted forms: Q-028. | [G-SCN] [AE2-26] |
| `interactive` | Boolean. When true, the player can turn and zoom the scene (6.6). Not in the attribute table of the documentation but used in its own examples and by AE2 on about a third of its scenes. | [G-SCN §Entity] [AE2-26] |

Scene children are processed **in document order** (this matters for 6.5 and 6.7).

### 6.2 `ImportStructure`

`<ImportStructure src="file.snbt" pos="x y z" />` places a structure from a file in the scene. [G-SCN §ImportStructure]

- `src`: a guide asset path relative to the page's folder (AE2: `../assets/assemblies/x.snbt`), looked up with the
  language fallback of 8.2 (Q-029). Both binary `.nbt` and text `.snbt` structure files are accepted.
- `pos`: integer offset of the structure's origin in scene coordinates, default `0 0 0`.
- The tag may appear any number of times in one scene.

File formats:

- **`.nbt`**: the vanilla structure template format (gzip-compressed NBT): `DataVersion`, `size` (3 ints), `palette` (list
  of `{Name, Properties}`) or `palettes` (list of such lists; use the first, Q-030), `blocks` (each with `state` = palette
  index, `pos` = 3 ints, optional `nbt` = block entity data without x/y/z), `entities` (each with `pos` (3 doubles),
  `blockPos` (3 ints) and `nbt`). Positions are relative to the structure's minimum corner. [W-STRUCT §NBT structure]
- **`.snbt`**: the same structure written as SNBT text, in the form AE2's files use: `DataVersion`, `size`, `data` (a list of
  `{pos: [x, y, z], state: "<block state string>", nbt: {…}}`, `nbt` optional), `entities` (list, same entry form as in
  `.nbt`), and `palette` (a list of block state **strings** such as `"ae2:cable_bus{light_level:9,waterlogged:false}"`,
  i.e. `id{prop:value,…}`). The block of each `data` entry is given by its own `state` string. [AE2-26] Files written in the
  `.nbt` field layout but as SNBT text (`blocks` + index `state`) must also be accepted (Q-031).
- Block state strings in `.snbt` use `{}` around properties with `:` between name and value, unlike the command syntax
  `[prop=value]`. [AE2-26]
- `DataVersion` may be older than 26.3 (AE2 1.20 files use 3337, 3460, 3465). Upgrading old block ids, states and block
  entity data: Q-032.
- Block entity data (`nbt`) is given to the block's block entity for drawing when the block has one, so blocks whose look
  depends on it (signs, banners, AE2's cable bus parts) look right. How far Sunrise's scene drawing supports this is
  Q-033. [W-STRUCT] [AE2-26]
- Entities in a structure file are added to the scene like `Entity` tags (6.4), within the scene entity limit (6.9).

### 6.3 `Block`

`<Block id="…" x="0" y="0" z="0" p:<name>="value" />` sets one block at an integer position (each coordinate default 0),
replacing what was there. `p:<name>` sets state properties; others take their defaults. `id="minecraft:air"` clears a
position, e.g. to hide a block an imported structure contains. Neighbours are not updated (the given state is drawn as
written). [G-SCN §Block]

### 6.4 `Entity`

`<Entity id="minecraft:sheep" x y z rotationY rotationX data />` adds an entity, as `/summon` would describe it.
[G-SCN §Entity]

| Attribute | Meaning |
|---|---|
| `id` | Entity type id. Required. |
| `x`, `z` | Position of the entity's centre, default 0.5 each. |
| `y` | Position of the entity's feet, default 0. |
| `rotationY` | Turn about the vertical axis, degrees, default -45. |
| `rotationX` | Turn about the horizontal axis, degrees (meaning depends on the entity; typically head pitch). Default 0 (Q-034). |
| `data` | Initial entity data in SNBT as `/summon` takes it, e.g. `{Color:2}`. |

Sunrise draws these as `SceneEntity`s (draw-only entities that never join a level or tick, aged by the scene clock).
[SR-LS §Entities]

### 6.5 `RemoveBlocks`

`<RemoveBlocks id="block_id" p:<name>="value" />` removes every block of that type (optionally only those whose state
matches the given properties) present in the scene at that point, without updating neighbours. Typical use: removing
support blocks under redstone or wall torches, or barriers. The documentation names the tag `RemoveBlock` in its prose
and `RemoveBlocks` in its example; accept both names. [G-SCN §Remove Blocks] (Q-035)

### 6.6 Camera, turning and zooming

6.6.1 The scene is drawn with an orthographic (isometric) projection. With no `IsometricCamera` tag the camera is turned
yaw 225° and pitch 30°. [G-SCN §IsometricCamera]

6.6.2 `<IsometricCamera yaw="…" pitch="…" roll="…" />` sets the turn in degrees about the vertical axis (yaw), the
horizontal X axis (pitch) and the Z axis (roll). When the tag is present, a missing attribute is 0 (AE2 has a few scenes
with only `yaw`, hence pitch 0). [G-SCN] [AE2-26]

6.6.3 Framing: the scene's content is centred in the box and fitted so that at `zoom` 1 the content fits (Q-036 for the
exact fit and box size).

6.6.4 Interactive scenes (`interactive` true): dragging with the left mouse button turns the scene (horizontal drag:
yaw; vertical drag: pitch); the mouse wheel over the scene zooms in and out; a reset control returns to the page's
camera and zoom. The wheel over a scene does not scroll the page while the scene is interactive. Non-interactive scenes
neither turn nor zoom, and the wheel scrolls the page. [ISSUE] ("a scene the player can turn and zoom") [SR-DESIGN]
(Q-037 for drag bindings, limits and controls)

6.6.5 Turning uses Sunrise's `SceneView` (centre, yaw, pitch, scale). [SR-LS]

### 6.7 Annotations

Annotations add outlines and markers to a scene. Every annotation may have children (Markdown and tags, including item
images); when the pointer is over the annotation, the children are shown as a tooltip. An annotation without children
draws but has no tooltip. Annotation tooltips work in interactive and non-interactive scenes (Q-038). [G-SCN §Annotations]

| Tag | Attributes | What it draws |
|---|---|---|
| `BlockAnnotation` | `x`, `y`, `z` (integers, default 0); `color` (`#RRGGBB`, default white) | A box outline around that block position. |
| `BoxAnnotation` | `min`, `max` ("x y z", decimals; `max` is exclusive, i.e. the far corner); `color`; `thickness` (line thickness in block units, default 0.5/16 = half a texture pixel); `alwaysOnTop` (boolean, default false: not hidden by blocks) | A box outline. |
| `LineAnnotation` | `from`, `to` ("x y z", decimals); `color`; `thickness`; `alwaysOnTop` | A 3D line. |
| `DiamondAnnotation` | `pos` ("x y z", decimals; `0.5 0.5 0.5` is the centre of block 0,0,0); `color` | A flat diamond icon drawn over the scene, anchored to that point and always facing the viewer. |

[G-SCN §BlockAnnotation, §BoxAnnotation, §LineAnnotation, §DiamondAnnotation]

`<BlockAnnotationTemplate id="block" p:<name>="value">annotation</BlockAnnotationTemplate>` repeats the annotation
inside it for every block of that type (and matching properties) present in the scene **at the point where the template
appears**, with the annotation's coordinates taken relative to each such block. So it must come after the
`ImportStructure` tags it applies to. [G-SCN §Block Annotation Templates]

### 6.8 Scene clock and particles

Scenes run on Sunrise's living-scene clock: one clock per scene on the page, ticking at 20 per second while the page is
shown, so animate-tick particles and entity idle animations play. Leaving the page pauses it. [ISSUE] [SR-LS §The
scene's clock] Each scene on a page gets its own clock id (SR-LS asks one id per scene).

### 6.9 Limits

Sunrise's scene drawing refuses more than 4096 blocks or 16 entities in one scene and animates at most four living
scenes per frame; a page with more scenes in view still draws their blocks. A scene over a limit shows the error of
4.13 in its box instead of a partial picture. [SR-LS §Refusals] (AE2's largest scenes must fit; Q-039.)

---

## 7. Cross-mod content

7.1 A part of a page that needs other mods is wrapped in `OptionalSection` (5.13). [G-AUTH §Optional Sections]

7.2 "Installed" for Sunrise: a mod id is installed when a loaded mod (Sunrise native mod, port, or Java mod through the
JVM bridge) has that id, or when its namespace has registry content (Q-040). `minecraft` is always installed. [SR-DESIGN]

7.3 Whole cross-mod pages ("With AE2", "With Iron Furnaces"): the issue asks for pages shown only when named mods are
installed. The GuideME documentation describes only the section-level tag. Sunrise extension [SR-DESIGN, proposed]:
a front matter key `requires_mods: [modid, ...]` (all must be installed; `!modid` must be absent). A page whose
requirement is not met is treated as absent: not in the contents tree, not in search, not a target for item lookups, and
links to it behave as 5.1.5. The key's name and form are to be fixed in review (part of Q-040); guides written for
GuideME will not use it, and GuideME ignores unknown keys (3.4).

7.4 A cross-mod page may live in either mod's guide. It may name its parent in another guide with a full page id
(3.2 `navigation.parent`), so it appears in that guide's tree. [G-AUTH] [SR-DESIGN]

---

## 8. Translations

8.1 A translated page lives under the guide root in a folder named `_<langcode>` holding the same relative path:
`folder/page.md` translated to German is `_de_de/folder/page.md`. [G-TR]

8.2 The viewer uses the game's current language. For each page it loads the translated file if present, else the
default file. Guide assets (images, structures) follow the same rule: language folder first, then the default.
[G-TR] [ISSUE]

8.3 The default (untranslated) pages are in the guide's `default_language` when a definition file (2.1.5) says so,
otherwise English (`en_us`). [G-DDG §Default Language] [ISSUE]

8.4 Translated files are not pages of their own: they are never listed separately, and the `_<langcode>` folders are
skipped when collecting pages. A translation without a default page is ignored for now (Q-041). [SR-DESIGN] Front matter of a translated page replaces the default page's front matter (Q-002 covers layout merging;
the translated `navigation.title` is the translated title). [SR-DESIGN]

8.5 Item names, tooltips and key names come from the game's language files as usual. [MC]

8.6 Changing the language while the viewer is open reloads the current page in the new language. [SR-DESIGN]

---

## 9. The viewer

### 9.1 Opening

9.1.1 **W over an item** opens that item's page (9.4) in the viewer, anywhere an item stack is shown under the
pointer: container and inventory screens (slots and the carried stack is not used: the hovered slot's stack), the
creative inventory, Sunrise's item panel, Recipes panel recipe pages, and guide pages (item images, item links, recipe
slots, item grids). [ISSUE]

9.1.2 W is matched by its **physical key position** in Sunrise's key handling as a fixed key (it is the forward key's
default), and acts only while a screen is open, the pointer is over an item, and no text field has focus. In the world
with no screen open W remains movement. If the item has no page, nothing opens (Q-042 for feedback). [SR-DESIGN]
(GuideME's own default is a different key and is documented as "hold" from an item tooltip; Sunrise uses W per the
issue. Whether W must be held briefly: Q-043.) [G-KEY] [G-OV] [ISSUE]

9.1.3 The viewer is a screen. Opening it from another screen remembers that screen; closing the viewer (Escape or a
close control) returns to it, with the container still open. [SR-DESIGN]

9.1.4 Opening the viewer without an item (to browse all guides): Q-044.

### 9.2 Contents

9.2.1 The viewer shows a contents sidebar beside the page. Its first level lists the guides that exist (2.3), by display
name (2.3.2); under each guide, the guide's own tree. [ISSUE] [SR-DESIGN]

9.2.2 A guide's tree: pages with `navigation` and no `parent` are the top level; pages naming a parent are its
children. Siblings are sorted by `position` ascending, then by title. A page whose parent does not exist is placed at
the top level and reported (4.13). Cycles are broken and reported. [G-AUTH §Adding Pages…] [SR-DESIGN for the error
cases] Pages without `position`: Q-045.

9.2.3 Depth: by default two levels of a guide's tree are shown. A page's `max_depth` sets how many levels below it are
shown, replacing the inherited value (`0` shows none of its children, `2` shows children and grandchildren); its
descendants inherit the remaining depth unless they set their own. Deeper pages stay reachable by links and `SubPages`.
[G-AUTH] Whether the guide-name level of 9.2.1 counts toward the two: it does not. [SR-DESIGN]

9.2.4 Each entry shows its `icon` (with `icon_components`) when set, and its title. The current page is highlighted;
clicking an entry opens it. Entries with children can be expanded and collapsed. [G-AUTH] [SR-DESIGN]

### 9.3 Navigation history

The viewer keeps a history of visited pages (with scroll positions). **Back** and **forward** controls (and the mouse's
back/forward buttons) move through it; following a link or an item lookup discards the forward part. History is kept
while the game runs and is cleared when the client leaves the world (Q-046). [ISSUE] [SR-DESIGN]

### 9.4 Item lookup

An item's page is the page whose `item_ids` lists the item's id (components are ignored for the lookup). Pages hidden by
section 7 are skipped. If several pages list the same item: Q-047. [G-AUTH §Declaring Pages…] [G-KEY]

### 9.5 Page view

9.5.1 The page is laid out to the page column's width with text reflow; inline elements (item links, item images,
coloured text) sit in the text flow. The page scrolls smoothly with the wheel and a scrollbar. [G-OV] [SR-DESIGN]

9.5.2 Text uses the game font. Headings are larger or bold by level (Q-048 for styles), block quotes indented with a
bar, tables with cell borders, code in a distinct colour.

9.5.3 Tooltips: items (5.3, 5.6, 5.14), page links (5.1.2), annotations (6.7), images (alt text), command links (5.10).

9.5.4 The viewer's light and dark appearance: Q-025.

9.5.5 Narration: page text is narrated as text, headings first, as Sunrise's narrator reads other screens. [SR-DESIGN]

### 9.6 Search

9.6.1 A search field searches the text and titles of all pages of all guides (pages hidden by section 7 excluded), in
the current language (translated text when a translation exists). [G-SRCH] [G-TR] [ISSUE]

9.6.2 Results list page titles with their guide and a short excerpt around the match; clicking a result opens the page
(and its anchor if the match is under a heading; Q-049). Ranking: title matches before text matches, then by number of
matches (Q-049). [SR-DESIGN]

9.6.3 Language rules: GuideME's search uses language-specific word rules for a list of languages and falls back to
English rules for others. [G-SRCH] Sunrise needs at least: case-insensitive matching, word boundaries, and matching of
word prefixes; stemming is optional (Q-050).

9.6.4 The search field matches the Recipes panel's search field look. [SR-PANEL] [SR-DESIGN]

### 9.7 Cross-guide links

Links with a full page id in another guide (5.1.1), parents in another guide (7.4) and item lookups for items of other
mods all open pages of other guides in the same viewer, with the same history. [G-AUTH] [ISSUE]

### 9.8 Reloading

Guides are collected when resources load and again on every resource reload (F3+T, resource pack changes), so a pack
that adds pages takes effect after a reload. Parsing may be lazy (on first open of a page) as long as item lookup,
the contents tree and search see every page. [G-AUTH §intro] [MC] [SR-DESIGN]

### 9.9 Commands

GuideME offers commands to open a guide or a page (`<guide-id> <page-id>#<anchor>`) for oneself or other players.
[G-CMD] Sunrise is not required to provide them; if it adds a client command, it uses the same argument form
(Q-051).

---

## 10. Data limits and robustness

10.1 A page that fails to parse shows what was parsed plus an error (4.13); it never closes the viewer or the game.
[SR-DESIGN]

10.2 Structure files larger than the scene limits (6.9) are refused per scene, not per page. [SR-LS]

10.3 Resource reload during viewing keeps the viewer open on the same page id if it still exists. [SR-DESIGN]

10.4 Files that are not UTF-8 are skipped and reported. [SR-DESIGN]

---

## 11. What is deliberately different from GuideME

| GuideME (as documented) | Sunrise |
|---|---|
| One guide per mod opened with a guide item or a hotkey from a tooltip [G-DDG] [G-KEY] | One viewer for all guides; W over any item; no items [ISSUE] |
| Mods register guides in code [G-OV] | Guides are found from asset folders alone (2.3) |
| Java extension tags and recipe renderers [G-OV] | Not supported (1.4); unknown tags per 4.12; recipe types per the Recipes panel |
| Only section-level mod conditions [G-AUTH] | Also page-level conditions (7.3, proposed) |
| Commands, website export, live preview [G-CMD] | Not required |

---

## 12. Open questions

- **Q-001** Guide id for `assets/<ns>/ae2guide/`: `<ns>:guide` (matches the documented example `ae2:guide`, [G-CMD]) or
  something else. Only matters for commands and cross-guide links by guide id; page ids use `<ns>:path`.
- **Q-002** A namespace with both an `ae2guide/` folder and `guides/<ns>/…` pages: one merged guide, or two guides; and
  which wins when the same page path exists in both.
- **Q-003** Where `default_language` sits in the guide definition file (top level is assumed).
- **Q-004** Display name of a guide that comes only from a resource pack (no mod): definition `display_name`, the
  namespace, or the pack's name.
- **Q-005** Page id namespace in the `guides/` layout for files under a different `assets/<x>/` namespace than the guide id.
- **Q-006** Meaning of `icon_nbt` (1.20 pages) on 26.3: whether to translate the old NBT (e.g. a facade's stored item) into
  the equivalent data component, ignore it, or try a data upgrade.
- **Q-007** Whether top-level `icon` (outside `navigation`) should be accepted as a navigation icon (one AE2 page uses it).
- **Q-008** Title of a page with no `navigation.title` and no level-1 heading.
- **Q-009** Whether autolinks (`<https://…>`), task lists or footnotes appear in real guides and need support.
- **Q-010** Code blocks: style, wrapping or horizontal scrolling of long lines.
- **Q-011** Whether string booleans (`"true"`) and numeric strings are accepted everywhere the docs show braces.
- **Q-012** Styling of `<div class="...">` (the notification-like box at the top of AE2's getting-started page).
- **Q-013** Unknown tags: show the children anyway, or only an error marker.
- **Q-014** How text like `<mode>` inside prose is distinguished from a tag (lowercase name not in 4.11 inside a
  paragraph: treat as text?).
- **Q-015** Exact behaviour of links to pages hidden by missing mods.
- **Q-016** Heading anchor rule for punctuation, formatting, non-ASCII and duplicate headings.
- **Q-017** Image scaling: GUI scale, maximum width, and whether images can be enlarged on click.
- **Q-018** What `scale="1"` means for `ItemImage` and `BlockImage` in pixels.
- **Q-019** Whether clicking an `ItemImage` / `ItemIcon` opens the item's page (W does).
- **Q-020** Invalid `p:` properties: error or silent default.
- **Q-021** Tooltip for `BlockImage`.
- **Q-022** Category name matching: case and whitespace.
- **Q-023** Order of pages in `CategoryIndex` (title, position, or file order).
- **Q-024** Whether `CommandLink` asks for confirmation before sending a command.
- **Q-025** Light/dark mode of the viewer, and the light/dark values of the sixteen symbolic colours (vanilla text
  colours for one mode; the other mode undefined).
- **Q-026** Which recipe is "first" for `RecipeFor` when several produce the item (recipe id order, recipe type order,
  or the Recipes panel's order).
- **Q-027** Fallback drawing of recipe types Sunrise cannot draw.
- **Q-028** Accepted `background` values besides `transparent` and `#RRGGBB`; the default background.
- **Q-029** Whether structure files follow the language fallback of 8.2 like images.
- **Q-030** Structures with `palettes` (several palettes): which one is shown.
- **Q-031** `.snbt` files written in the `blocks`/index layout: confirm support is needed.
- **Q-032** Upgrading structures with an old `DataVersion` (renamed blocks, changed states, block entity data) to 26.3.
- **Q-033** Which block entities Sunrise's scene drawing can show from structure `nbt` (signs, banners, chests, modded
  block entities such as AE2's cable bus), and what is drawn when it cannot.
- **Q-034** Default and exact meaning of `rotationX` for entities.
- **Q-035** `RemoveBlock` versus `RemoveBlocks`: the canonical name.
- **Q-036** Scene box size on the page (fixed height, or from content bounds × zoom, clamped to the page width) and
  framing.
- **Q-037** Interactive scene controls: which button turns, pitch limits, zoom range and step, reset control, and
  keyboard/controller equivalents.
- **Q-038** Whether annotation tooltips and outlines show in non-interactive scenes; whether annotations can be hidden
  by a toggle.
- **Q-039** AE2's largest structure against the 4096-block limit (to be measured by the implementer).
- **Q-040** Definition of "installed mod" for `OptionalSection` (mod id vs namespace; Java mods through the JVM bridge);
  name and form of the page-level requirement key (7.3).
- **Q-041** A translated page with no default page: ignore or treat as a page in that language only.
- **Q-042** Feedback when W is pressed over an item that has no page (nothing, a sound, a short toast).
- **Q-043** Press or hold W; whether W is a fixed key or a rebindable Sunrise setting (and its behaviour on AZERTY layouts); whether the item tooltip shows a hint line ("W: guide") for items that have a page.
- **Q-044** How the viewer is opened without an item (a button on the Recipes panel, the pause menu, a key, or a client
  command), and which page it opens first (last viewed, or the guide list).
- **Q-045** Sort position of pages without `position` relative to those with one.
- **Q-046** Whether history and the last page survive leaving and rejoining a world.
- **Q-047** Several pages claiming the same item: first by guide of the item's namespace, then by page id; or a chooser.
- **Q-048** Heading sizes and styles with the game font.
- **Q-049** Search ranking, excerpt length and jumping to the matching heading.
- **Q-050** Search word rules beyond case-insensitive prefix matching (stemming per language, CJK text without
  spaces).
- **Q-051** Whether Sunrise provides a client command to open a page, and its name.
- **Q-052** Whether the feature sits behind the `sunrise-extras` Cargo feature, and whether it is on by default (the
  extras doc requires every extra to be off by default; the issue leaves this to review).
- **Q-053** Recipes on multiplayer servers: whether the recipe data Sunrise's Recipes panel uses is available on a
  vanilla server, and what recipe tags show when it is not.
- **Q-054** W pressed over a page link or a guide page itself (not an item): no action is assumed.

There are **54** open questions.

---

## 13. Sources

### 13.1 Source tags

| Tag | Source | Section(s) used |
|---|---|---|
| [ISSUE] | sunrise-org/sunrise#2664, issue body, as of 2026-11-18T09:03Z | whole issue |
| [G-OV] | https://guideme.appliedenergistics.org/ (Overview) | feature list (the example page there was read as page format only) |
| [G-AUTH] | https://guideme.appliedenergistics.org/authoring/ (Authoring Pages) | intro, Frontmatter, Adding Pages to the Navigation Bar, Declaring Pages as ItemLink targets, Using Images, Comments, Custom Tags and all its subsections |
| [G-MD] | https://guideme.appliedenergistics.org/authoring/markdown (Supported Markdown) | Specifications, Inline Formatting, Headings, Other Block Elements, Lists, Tables |
| [G-SCN] | https://guideme.appliedenergistics.org/authoring/game-scenes (Game Scenes) | all sections |
| [G-DDG] | https://guideme.appliedenergistics.org/data-driven-guides (Data Driven Guides) | Guide Definition, Generic Guide Item, Default Language, Custom Colors |
| [G-CMD] | https://guideme.appliedenergistics.org/commands | Open Guides, Create Guide Items |
| [G-KEY] | https://guideme.appliedenergistics.org/open-guide-hotkey | whole page |
| [G-SRCH] | https://guideme.appliedenergistics.org/search | language list |
| [G-TR] | https://guideme.appliedenergistics.org/translation | whole page |
| [AE2-26] | Guide page files (`.md`, `.snbt`) in `assets/ae2/ae2guide/` of the published jar `appliedenergistics2-26.3.2-alpha.jar` (Modrinth), as data | layout, front matter keys, tag and attribute usage, SNBT layout, anchors |
| [AE2-15] | The same folder in `appliedenergistics2-forge-15.4.12.jar` (Modrinth, Minecraft 1.20.1), as data | 1.20 layout and differences (`icon_nbt`, `<a href>`) |
| [W-STRUCT] | https://minecraft.wiki/w/Structure_file, section "NBT structure" | `.nbt` format |
| [W-ARG] | https://minecraft.wiki/w/Argument_types, section `minecraft:item_stack` | `components` syntax |
| [MC] | Minecraft 26.3 behaviour (resource locations and packs, chat link confirmation, key mappings, language selection) | as cited |
| [SR-LS] | Sunrise worktree: `docs/modding/src/how-to/client/living-scenes.md`; module comment of `crates/sunrise-client/src/game/modding/native_scene_live.rs` | scenes, clock, limits |
| [SR-PANEL] | Sunrise worktree: `docs/modding/src/how-to/items/panel-style.md` | Recipes panel search field |
| [SR] | Sunrise worktree: `docs/extras.md`, `CONTRIBUTING.md`, `docs/cleanroom/README.md` | process, extras |
| [SR-DESIGN] | Choices of the spec team or the issue | marked where used |

Revisions, dates and hashes are in [guides-provenance-log.md](guides-provenance-log.md).

### 13.2 Section → sources

| Spec section | Sources |
|---|---|
| 1 | ISSUE, G-OV |
| 2 | G-AUTH, G-DDG, G-CMD, AE2-26, AE2-15, ISSUE, MC |
| 3 | G-AUTH, AE2-26, AE2-15, W-ARG |
| 4 | G-MD, G-AUTH, G-SCN, AE2-26, AE2-15 |
| 5 | G-AUTH, G-MD, G-TR, G-OV, W-ARG, AE2-26, MC, SR, ISSUE |
| 6 | G-SCN, W-STRUCT, AE2-26, AE2-15, SR-LS, ISSUE |
| 7 | G-AUTH, ISSUE |
| 8 | G-TR, G-DDG, ISSUE, MC |
| 9 | ISSUE, G-AUTH, G-OV, G-KEY, G-SRCH, G-TR, G-CMD, SR-PANEL, MC |
| 10, 11 | as cited inline |

### 13.3 Not read

GuideME's Integration pages (custom tags, recipe types, symbolic colors, website export), its Javadoc, its changelog,
its downloads and live-preview pages (they contain build-script code), the GitHub "source" link of its overview example,
its test mod, and any source code of GuideME, AE2 or other guide mods. See the provenance log.

---

## Changelog

- **Revision 2 (2026-11-18):** written from scratch by a new spec session. Revision 1 was discarded unread.
