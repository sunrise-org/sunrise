//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_render` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ClientRenderClientHost {
    /// Atomically overwrite retained model copies; use [`InstanceGroupRef::write`].
    #[access(PublishedResources)]
    fn instance_group_write(
        &self,
        group: InstanceGroupRef,
        first: u32,
        instances: &[ModelInstance],
    ) -> Result<(), GroupError>;
    /// Set the drawn length; use [`InstanceGroupRef::set_len`].
    #[access(PublishedResources)]
    fn instance_group_set_len(&self, group: InstanceGroupRef, len: u32) -> Result<(), GroupError>;
    /// Replace the group pose; use [`InstanceGroupRef::set_group_transform`].
    #[access(PublishedResources)]
    fn instance_group_set_transform(
        &self,
        group: InstanceGroupRef,
        transform: [f32; 12],
    ) -> Result<(), GroupError>;
    /// Retire retained model copies; use [`InstanceGroupRef::destroy`].
    #[access(PublishedResources)]
    fn instance_group_destroy(&self, group: InstanceGroupRef) -> Result<(), GroupError>;
    /// Side: client.
    /// `ItemStackRenderState.usesBlockLight` of `stack`'s model resolved for `context`
    /// (`ItemModelResolver.updateForTopItem`, no owner or level): whether the model is lit as a
    /// block is, from the side (`gui_light: side`, vanilla's 3D item models), where a flat item
    /// is lit from the front. It is what the 1.21.3 `BakedModel.isGui3d` became: a mod uses it to
    /// draw a stack as a flat sprite or as a 3D model. False for an
    /// empty or invalid stack, and for a model that draws nothing. Reads the model the client
    /// resolved from its resource packs.
    #[access(ClientSetup)]
    fn item_uses_block_light(&self, stack: &ItemStack, context: ItemDisplayContext) -> bool;
    /// Side: client.
    /// Asks `block_render_visibility` again for every state and, when the answers differ
    /// from those in use, remeshes every loaded section with them. Call it when your answer changes (a toggle key, a scan
    /// finishing) and once when the player joins, not every frame: a rebuild takes the workers a
    /// few seconds. With no part answering, nothing is asked and the mesher keeps its no-mod path.
    #[access(WorldOnly)]
    fn refresh_render_visibility(&mut self);
    /// Side: client.
    /// Asks `biome_colors` again for every biome and, when the colours
    /// differ from those in use, rebuilds every loaded section and the far
    /// terrain with them (`LevelRenderer.allChanged`, as changing Biome Blend
    /// does). Call it when your colours change (a new season, a new day of a
    /// slow fade) and once when the player joins, not every frame: a rebuild
    /// takes the workers a few seconds.
    #[access(WorldOnly)]
    fn refresh_biome_colors(&mut self);
    /// Side: client.
    /// Vanilla's `TextureManager.register` of a `DynamicTexture` under `id`, then its `upload`:
    /// the calling mod's RGBA image `id`, in its own namespace (`mymod:dynamic/panel`), made from
    /// `rgba`, `width * height` pixels of four bytes, row by row from the top left. The image
    /// takes a slot of the HUD atlas's dynamic texture pool, of the smallest class
    /// ([`DYNAMIC_TEXTURE_CLASSES`]) that holds it, and `id` names it as a GUI sprite of
    /// `width` x `height` pixels: [`DrawContext::sprite`] (`src` in the image's own pixels),
    /// [`DrawContext::gui_nine_slice`], [`DrawContext::tiled_sprite`] and
    /// [`gui_sprite`](Self::gui_sprite) draw and find it as they do any GUI sprite, from the next
    /// frame on. It lasts, through resource reloads, until
    /// [`free_dynamic_texture`](Self::free_dynamic_texture) or the game's end.
    ///
    /// Refused by name, with nothing changed and nothing allocated, as [`DynamicTextureError`]
    /// lists: `Unavailable` when the atlas has no pool, `NotOwned` for an id outside the mod's
    /// namespace, `BadSize` for a side of 0 or pixels that are not `width * height * 4` bytes,
    /// `TooLarge` for a side past [`DYNAMIC_TEXTURE_MAX_SIDE`], `NameTaken` for an id that
    /// already names an image or a GUI sprite, `OverBudget` past the mod's
    /// [`DYNAMIC_TEXTURE_MOD_IMAGES`] images or [`DYNAMIC_TEXTURE_MOD_BYTES`] of pixels, and
    /// `NoSlot` when every slot of the image's class is held. The game keeps a copy of the
    /// pixels, counted in the byte budget, to upload them again into a rebuilt atlas. Native
    /// and wasm mods make the same call; the pixels cross as bytes.
    #[access(WorldOnly)]
    fn create_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// Vanilla's `DynamicTexture.upload` of new pixels: the calling mod's image `id` takes
    /// `rgba`, `width * height` pixels of four bytes, written into the atlas slot it already holds
    /// from the next frame on. The size may change within the image's class; the sprite `id`
    /// takes the new size. Each update is one copy into the kept pixels and one upload of the
    /// image, so send one when the image changed, not every frame.
    ///
    /// Refused by name, with nothing changed, as [`DynamicTextureError`] lists: `Unavailable`,
    /// `NotOwned`, `Unknown` for an id that names no image, `BadSize`, `TooLarge`,
    /// `ClassChange` for a size of another class than the image's (free it and create it again),
    /// and `OverBudget` when the new size would pass [`DYNAMIC_TEXTURE_MOD_BYTES`].
    #[access(WorldOnly)]
    fn update_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// Vanilla's `TextureManager.release`: frees the calling mod's image `id`. The GUI sprite
    /// `id` is gone at once, the slot goes back to the pool for any mod, and the image and its
    /// bytes go back to the mod's budget. Refused by name, with nothing changed:
    /// `Unavailable`, `NotOwned` for an id outside the mod's namespace and `Unknown` for one
    /// that names no image.
    #[access(WorldOnly)]
    fn free_dynamic_texture(&mut self, id: &Identifier) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// What the calling mod may still create: images and bytes of pixels left of its
    /// [`DYNAMIC_TEXTURE_MOD_IMAGES`] and [`DYNAMIC_TEXTURE_MOD_BYTES`]. These are limits of the
    /// atlas pool, which vanilla's `TextureManager` does not have. A free slot of a class is not
    /// part of it: the slots are shared by every mod. `Unavailable` when the atlas has no pool,
    /// `NotOwned` when no mod is calling.
    #[access(WorldOnly)]
    fn dynamic_texture_budget(&self) -> Result<DynamicTextureBudget, DynamicTextureError>;
    /// Side: client.
    /// For a client mod drawing a block's faces itself (a copycat drawing its material's, from
    /// `block_sprites`): the 0xRRGGBB colour a face of `state` with tint index `tint_index`
    /// (`FaceSprite::tint_index`) is multiplied by at `pos`, as the chunk mesher tints it there
    /// (`BlockColors.getColor`, biome blend included: grass, foliage and water follow the biome
    /// around `pos`). None for an untinted face (-1, or an index past the state's tints). Reads
    /// this client's copy of the world.
    #[access(WorldOnly)]
    fn block_tint_color(&self, state: BlockState, tint_index: i32, pos: [i32; 3]) -> Option<u32>;
    /// Side: client.
    /// The standalone models (resolved with [`RenderAssets::model`]) drawn as part of the block
    /// at `pos` in this client's level, as NeoForge's `ModelData` lets a block's model choose
    /// parts at run time without a block state for each choice: a cable's per-side channel
    /// counts. The chunk mesher draws them after the block's own models, unturned from the
    /// block's lower north-west corner (with its offset), in the block's own light and tints.
    /// Each quad takes its render layer as a block's would (cut out, translucent), and a face
    /// with a cull direction is culled by the neighbour there as the block's own faces are. On
    /// air they draw nothing.
    ///
    /// `models` replaces the list the position had; an empty list clears it. The section is
    /// meshed again when the list changes. The list is the client's own, never saved or sent:
    /// it is cleared when the block state at `pos` changes, when its column unloads (a column
    /// the server sends again comes without it) and when the player's level changes. A block
    /// entity is not needed. The mesher reads the list on its workers and calls no mod code.
    /// A section's extra models are held to its quad and byte limits, after the blocks' own
    /// geometry: past them, the rest of the section's extra models, in block order and then list
    /// order, are not drawn, and the blocks' own always are.
    ///
    /// A handle a resource reload made stale draws nothing until the list is set again from the
    /// new handles. The call is World-only, so not in `client_resources_reloaded` (Reloading):
    /// resolve the new handles there with [`RenderAssets::model`] and set the lists again from
    /// the next World-phase callback, such as `client_tick_pre`.
    ///
    /// Refused with nothing changed: `NotLoaded` when the client holds no column at `pos` or it
    /// is outside the level's height, `TooMany` past [`BLOCK_EXTRA_MODELS_MAX`] models, `Model`
    /// for a stale or unknown handle, and `Unavailable` outside the World phase (a resource
    /// reload, setup, config loading), on native and wasm alike.
    #[access(WorldOnly)]
    fn set_block_extra_models(
        &mut self,
        pos: [i32; 3],
        models: &[ModelRef],
    ) -> Result<(), BlockExtraModelsError>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: what each resolved block state draws on its six sides, preserving
    /// its properties without reparsing names. Resolve states once at load. Each face's sprite (array and
    /// layer, as `WorldDrawn::block_sprites` cuts them) and its texture by the name `cube` and `quad` take,
    /// where it lies on the side, how its texture runs across it (`FaceSprite::quad_on` draws it on another
    /// block's box, turned as its model turns it), its tint source and index (`block_tint_color`) and
    /// whether it is cut out, translucent or animated; its particle sprite; whether it is a plain
    /// cube. None for a block the game does not know. The model is the state's first random variant. API
    /// 380.
    #[access(PublishedResources)]
    fn block_sprites(&self, blocks: &[BlockState]) -> Vec<Option<BlockSprites>>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: each texture's sprite by name (`minecraft:block/stone`, `block/stone`), none for one no
    /// model uses. API 380.
    #[access(PublishedResources)]
    fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<SpriteRef>>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: where GUI sprite `sprite` lies in the GUI atlas and its size, none for one the atlas lacks
    /// (a server's metrics-only atlas lacks them all). The atlas names vanilla's sprites by path alone
    /// (`gui/sprites/container/slot`) and a mod's with its namespace (`mymod:gui/panel`).
    #[access(PublishedResources)]
    fn gui_sprite(&self, sprite: &Identifier) -> Option<AtlasRegion>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: changes each time the resources load (a resource pack turned on or off can move every
    /// sprite): ask `native_block_sprites` again then. API 380.
    #[access(PublishedResources)]
    fn block_sprites_version(&self) -> u64;
}
