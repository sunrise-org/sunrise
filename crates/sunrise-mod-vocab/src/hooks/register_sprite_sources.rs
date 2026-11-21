//! The `register_sprite_sources` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The atlas sprite source types a mod adds (NeoForge's `RegisterSpriteSourcesEvent`): push
    /// each type id, `<mod id>:<path>`. Asked once at start on the client, before resources load.
    /// An entry of an atlas definition's `sources` whose `type` is a registered id is answered by
    /// `sprite_source` at its place in the list, on every load and reload. An id in vanilla's
    /// namespace, not of that form, or registered already is logged and left out.
    #[notes = r#"## Sprite sources

An atlas definition (`assets/<namespace>/atlases/<path>.json`) lists the sources that give the atlas its
sprites: vanilla's `directory`, `single`, `filter`, `unstitch` and `paletted_permutations`. A mod adds a
source type of its own (NeoForge's `RegisterSpriteSourcesEvent`) in two steps, as an item tint source:
`Hooks::register_sprite_sources` pushes the type's id once at start, before resources load, and
`Hooks::sprite_source` answers each entry of that type when a definition is read, on every load and
reload.

```rust,ignore
fn register_sprite_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterSpriteSourcesEvent<'_>,
) {
    event.sources.push("example:checker".into());
}

fn sprite_source(&self, _context: &RegistrationContext, event: &mut SpriteSourceEvent<'_>) {
    if event.source != "example:checker" {
        return;
    }
    // A JSON number arrives as the narrowest tag that holds it: `8` is a `Tag::Byte`.
    let side = match event.config.get("size") {
        Some(Tag::Byte(size)) => (*size).clamp(1, 64) as u32,
        _ => 16,
    };
    let rgba = (0..side * side)
        .flat_map(|i| if (i / side + i % side) % 2 == 0 { [255, 0, 255, 255] } else { [0, 0, 0, 255] })
        .collect();
    event.output.push(SpriteSourceOutput::Pixels {
        sprite: Identifier::parse("example:checker").unwrap(),
        width: side,
        height: side,
        rgba,
    });
}
```

Use it as an entry of a definition's `sources`, `{"type": "example:checker", "size": 8}`: the whole object
reaches the hook as `event.config`, `event.atlas` names the atlas, and `event.resources` reads the load's
resources. A source pushes `SpriteSourceOutput`s, which apply in order as vanilla's
`SpriteSource.Output` calls do: `Texture` adds a texture of the resources as a sprite, `Pixels` adds pixels
the mod made, and `Remove` drops a sprite by id (vanilla's `removeAll`). A sprite of an id listed
already replaces the earlier one, as a built-in source's does. A source never sees what the entries before
it listed: output is write-only.

`event.resources` is the `ResourceManager` vanilla hands `SpriteSource.run`, read-only: `read(path)` gives
the winning copy of a resource (`assets/<namespace>/textures/<path>.png`), `read_stack(path)` every pack's
copy lowest first, and `list(prefix)` the paths under a prefix. A source can make a sprite from the pixels
of a texture any pack has, a player's retexture included. It is valid for the one callback it is given to: a reader a source keeps reads nothing
afterwards and keeps nothing alive.

Each entry is asked once per atlas each time the client prepares its resources, and the answer is kept
for everything that reads the atlas, so a source that counts or varies its answers is still seen as one
answer. Sprite sources run in the game's blocks, items and GUI atlas definitions (`assets/minecraft/atlases/blocks.json`,
`items.json` and `gui.json`: the sprites block and item models name, and the GUI's own, which `DrawContext::sprite` and
`gui_sprite` find by id: `minecraft:container/slot`, a mod's `mymod:panel`) and in the definition of an atlas a mod
registered (`register_texture_atlases`); an atlas is asked under its own id, so two atlases sharing a
definition each get their own question. Each atlas keeps its own sprites, by its own id: when the blocks
and items atlases both list an id, a model's material is the items atlas's, and `texture_sprites` finds
the blocks atlas's sprite of that id (and every sprite a source made in the blocks atlas, though no model
names it) after `texture_atlas_stitched`. A sprite only the items atlas lists has no block array to be found
in. An entry whose type no mod registered fails its definition, as an unknown type fails it in vanilla, and
the log names the type and the entry's place. A `Texture` the resources lack, and `Pixels` whose length
isn't `width * height * 4` or whose side is over 32768 pixels, are logged and skipped one by one."#]
    #[hook(
        side = "client",
        version = 2,
        contract = "collect",
        out = "sources",
        context = "registration",
        neoforge = "RegisterSpriteSourcesEvent"
    )]
    fn register_sprite_sources();
}
