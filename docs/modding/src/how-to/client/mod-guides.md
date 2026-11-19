# Ship a guide with your mod

Sunrise shows every mod's guide in one built-in viewer. A player opens it by pressing **W** with the pointer over an
item on a container screen (the inventory, the creative inventory, a container, the item panel or a recipe page), or
over an item on a guide page. There is no guide book item, no key of your own and no library to depend on: you ship
Markdown pages as assets and Sunrise finds them. A mod with no guide changes nothing.

The format is the one GuideME documents for mod authors, so guides written for it work unchanged. The viewer is a
Sunrise extra (`sunrise-extras`, [extras](https://github.com/sunrise-org/sunrise/blob/main/docs/extras.md)); its
behaviour is specified in `docs/cleanroom/guides-spec.md`, and the choices the specification leaves open are in
`docs/cleanroom/guides-decisions.md`.

## Where the pages go

Either layout works; both may be used in one mod.

| Layout | Folder | Guide id | Page ids |
|---|---|---|---|
| GuideME | `assets/<any namespace>/guides/<ns>/<name>/` | `<ns>:<name>` | `<ns>:<path under the folder>` |
| AE2 | `assets/<ns>/ae2guide/` | `<ns>:guide` | `<ns>:<path under the folder>` |

Every `.md` file below the folder, in subfolders too, is a page: `ae2-mechanics/channels.md` is the page
`mymod:ae2-mechanics/channels.md`. Other files (`.png` images, `.snbt` and `.nbt` structures) are the guide's assets,
named by a path relative to the page that uses them. File and folder names must be valid resource paths (lowercase
letters, digits, `_`, `-`, `.`); others are skipped with a warning.

Resource packs and other mods can add pages to your guide, or replace one, by shipping a file at the same place.

An optional `assets/<ns>/guideme_guides/<name>.json` adds `default_language` (the language your untranslated pages are
in, `en_us` by default) and `custom_colors` (`{"hot": {"dark_mode": "#FF8800", "light_mode": "#AA4400"}}`, used as
`<Color id="hot">`). Its other keys are ignored.

## A page

```markdown
---
navigation:
  title: Iron Furnace
  icon: iron_furnace
  parent: index.md
  position: 10
item_ids:
  - iron_furnace
categories:
  - Machines
---

# Iron Furnace

Smelts faster than a <ItemLink id="minecraft:furnace" />. Fuel it like a furnace.

<RecipeFor id="iron_furnace" />
```

- `navigation` puts the page in the contents tree: `title`, an `icon` item (with `icon_components`), the `parent` page
  (a path under the guide's folder, or a full id `othermod:page.md` to sit in another mod's tree), `position` (smaller
  first) and `max_depth` (how many levels of children the tree shows under it; two by default).
- `item_ids` are the items this is the page for: W over one of them opens it.
- `categories` list the page in `<CategoryIndex category="Machines" />`.
- `requires_mods: [ae2]` shows the page only when that mod is installed (`!ae2`: only when it is not). Use it for pages
  about two mods working together. It is a Sunrise addition; other viewers ignore it.

Ids without a namespace are your page's namespace, not `minecraft:`.

## Markdown and tags

Pages are CommonMark with tables and strikethrough. Links take a path relative to the page (`../machines/furnace.md`),
a full id (`othermod:page.md`), an anchor (`page.md#heading-name`) or a web address. `{/* comments */}` show nothing.

| Tag | Shows |
|---|---|
| `<ItemLink id="x" />` | The item's name, linking to its page; `optional`, `fallback="text"`, `components="..."` |
| `<ItemImage id="x" scale="2" float="left" />`, `<ItemGrid><ItemIcon id="x" /></ItemGrid>` | Items as in slots |
| `<BlockImage id="x" scale="4" p:facing="north" />` | One block, in 3D |
| `![alt](image.png)`, `<FloatingImage src="image.png" align="right" />`, `<br clear="all" />` | Images |
| `<Row gap="5">`, `<Column alignItems="center" fullWidth>` | Layout |
| `<RecipeFor id="x" />`, `<RecipesFor id="x" />`, `<Recipe id="ns:path" />` | Recipes as live slots; `fallbackText` |
| `<GameScene zoom="4" interactive={true}>` with `<ImportStructure src="s.snbt" />`, `<Block id="x" x="1" />`, `<RemoveBlocks id="x" />`, `<IsometricCamera yaw="45" pitch="30" />` | A 3D scene the player can turn and zoom |
| `<BlockAnnotation x="0" y="0" z="0" color="#ff0000">tooltip</BlockAnnotation>`, `BoxAnnotation`, `LineAnnotation`, `DiamondAnnotation`, `BlockAnnotationTemplate` | Marks in a scene, with tooltips |
| `<OptionalSection modId="ae2">...</OptionalSection>` | A part shown only with that mod (`!ae2`: without) |
| `<SubPages />`, `<CategoryIndex category="x" />` | Lists of pages |
| `<Color id="red">`, `<KeyBind id="key.jump" />`, `<PlayerName />`, `<CommandLink command="/time set day">` | Text |

A tag Sunrise doesn't know shows a short red error in its place, and the rest of the page still shows.

Scenes are drawn with Sunrise's [living scenes](living-scenes.md): furnaces flicker and campfires smoke. A scene takes
at most 4096 blocks; a bigger one shows an error in its box.

## Translations

A translated page has the same path under a `_<language>` folder of the guide: `_de_de/machines/furnace.md`. The viewer
uses the game's language and falls back to the untranslated page; images and structures follow the same rule.

## Trying it

Put the folder in a resource pack (`assets/<ns>/guides/...`) or in your mod's assets, start a world, open your
inventory and press W over one of the page's `item_ids`. After editing pages, F3+T reloads them.
