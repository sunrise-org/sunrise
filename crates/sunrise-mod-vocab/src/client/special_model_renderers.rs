//! Special item model renderers, as a client mod registers them in
//! `register_special_model_renderer` (NeoForge's `RegisterSpecialModelRendererEvent` and
//! vanilla's `SpecialModelRenderer`).
//!
//! At client startup, register a [`SpecialModelFactory`] for each namespaced type. Each resource
//! load decodes and bakes every `minecraft:special` model node that names that type. On each draw,
//! the baked renderer extracts an argument from the selected stack and submits geometry or nested
//! stacks through a [`SpecialModelCollector`]. The game owns the surrounding draw state.
use crate::{ClientHost, Component, ItemDisplayContext, ItemKey, ItemStack};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::{any::Any, collections::BTreeMap, num::NonZeroU32, sync::Arc};
use sunrise_core::nbt::Tag;

// no wire: a borrowed view of one resource definition, used only during decode.
/// The inline fields of the special model's `model` object, excluding the dispatch key `type`.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct SpecialModelConfig<'a>(&'a Map<String, Value>);

impl<'a> SpecialModelConfig<'a> {
    // hidden: only the engine supplies the decoder-validated object.
    #[doc(hidden)]
    /// Borrow the object validated by the engine's item definition decoder.
    pub fn new(fields: &'a Map<String, Value>) -> Self {
        Self(fields)
    }

    /// Decode these fields into the factory's owned configuration. Your type's serde rules
    /// determine required fields, defaults and handling of unknown fields.
    pub fn decode<T: DeserializeOwned>(self) -> Result<T, String> {
        let fields = self
            .0
            .iter()
            .filter(|(name, _)| name.as_str() != "type")
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        serde_json::from_value(Value::Object(fields)).map_err(|error| error.to_string())
    }
}

// no wire: process-local decoder and bake callbacks.
/// A decoder paired with a bake function for one special model type.
#[derive(Clone)]
#[non_exhaustive]
pub struct SpecialModelFactory(Arc<BakeSpecialModel>);

type BakeSpecialModel = dyn for<'a> Fn(
        SpecialModelConfig<'a>,
        &dyn SpecialModelBakeContext,
    ) -> Result<BakedSpecialModelRenderer, String>
    + Send
    + Sync;

impl SpecialModelFactory {
    /// Pair a decoder with the bake function that consumes its typed output. For each model node
    /// naming this type, the game decodes and then bakes once per client resource load. The
    /// resulting renderer owns any configuration or geometry it needs during drawing.
    ///
    /// A decode or bake error or panic refuses the enclosing item definition, not the whole
    /// resource load. The game does not try a lower pack's definition.
    pub fn new<C: 'static, R: SpecialModelRenderer>(
        decode: impl for<'a> Fn(SpecialModelConfig<'a>) -> Result<C, String> + Send + Sync + 'static,
        bake: impl Fn(C, &dyn SpecialModelBakeContext) -> Result<R, String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(move |config, context| {
            let config = decode(config).map_err(|error| format!("decode: {error}"))?;
            let renderer = bake(config, context).map_err(|error| format!("bake: {error}"))?;
            Ok(BakedSpecialModelRenderer(Box::new(
                move |host, stack, context, collector| {
                    let argument = renderer.extract_argument(host, stack);
                    renderer.submit(&argument, context, collector)
                },
            )))
        }))
    }

    // hidden: the resource loader owns factory invocation.
    #[doc(hidden)]
    /// Run the factory at the engine's item definition load boundary.
    pub fn bake(
        &self,
        config: SpecialModelConfig<'_>,
        context: &dyn SpecialModelBakeContext,
    ) -> Result<BakedSpecialModelRenderer, String> {
        (self.0)(config, context)
    }
}

// no wire: a scoped view of the selected client resources, used only during bake.
/// The resources and item quad baker available to a special model factory during baking.
pub trait SpecialModelBakeContext {
    /// Bake a model from the selected packs into a lease, or report that it could not be baked.
    /// Only its geometry is used; the special node's `base` supplies display transforms, particle
    /// and GUI light. Bake new leases on reload rather than reusing an old one.
    fn bake_model(&self, model: &str) -> Result<SpecialModelGeometry, String>;

    /// Bake custom quads against the selected packs' item and block atlas sprites. A missing
    /// sprite, non-finite coordinate, invalid facing, tint index or light level rejects the bake.
    fn bake_geometry(&self, quads: &[SpecialModelQuad]) -> Result<SpecialModelGeometry, String>;
}

// no wire: an input to the in-process geometry baker, not a draw recipe.
/// One textured quad to bake for a special item model.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct SpecialModelQuad {
    /// Corners in block units, relative to the model's corner. The first three determine facing
    /// by their winding and must not be collinear.
    pub positions: [[f32; 3]; 4],
    /// Sprite-local UVs for the same four corners, in `[0, 1]`.
    pub uv: [[f32; 2]; 4],
    /// The sprite's resource ID in an item or block atlas.
    pub sprite: String,
    /// Index into the submission's indexed palette, if any.
    pub tint_index: Option<u32>,
    /// Block light floor, from 0 to 15.
    pub light_emission: u8,
    /// Draw on the translucent item sheet even when the sprite is opaque.
    pub force_translucent: bool,
}

impl SpecialModelQuad {
    /// An untinted, non-emissive quad whose render sheet follows its sprite.
    pub fn new(positions: [[f32; 3]; 4], uv: [[f32; 2]; 4], sprite: impl Into<String>) -> Self {
        Self {
            positions,
            uv,
            sprite: sprite.into(),
            tint_index: None,
            light_emission: 0,
            force_translucent: false,
        }
    }
}

// no wire: a process-local lease on baked quads and extents for one client resource load.
/// A lease on baked item geometry. Cloning retains the geometry without rebaking it. Submit this
/// lease only in the resource load in which it was baked; already resolved item states keep
/// their geometry across reloads.
#[derive(Clone)]
#[non_exhaustive]
pub struct SpecialModelGeometry(Arc<dyn Any + Send + Sync>);

impl SpecialModelGeometry {
    // hidden: only the engine can wrap its validated baked geometry.
    #[doc(hidden)]
    /// Store validated, engine-owned geometry produced by a bake context.
    pub fn new(geometry: impl Any + Send + Sync) -> Self {
        Self(Arc::new(geometry))
    }

    // hidden: the engine owns the concrete baked geometry type.
    #[doc(hidden)]
    /// Borrow the baked payload in the engine that produced it.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

// no wire: a borrowed view of the stack being drawn and its effective component reader.
/// The stack that selected the layer, read without copying its component patch. For a layer of
/// a bundle's selected item, this is the selected stack, not the bundle.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct SpecialModelStack<'a> {
    item: ItemKey,
    count: NonZeroU32,
    component: &'a dyn Fn(&str) -> Option<Tag>,
}

impl<'a> SpecialModelStack<'a> {
    // hidden: the engine lends the live component lookup during submit.
    #[doc(hidden)]
    /// Borrow the engine's view of the effective stack components.
    pub fn new(
        item: ItemKey,
        count: NonZeroU32,
        component: &'a dyn Fn(&str) -> Option<Tag>,
    ) -> Self {
        Self {
            item,
            count,
            component,
        }
    }

    /// The stack's item key.
    pub fn item(&self) -> ItemKey {
        self.item
    }

    /// The number of items in the drawn stack.
    pub fn count(&self) -> u32 {
        self.count.get()
    }

    /// Read one effective component with its typed descriptor. The patch overrides the item's
    /// defaults, including explicit removals. Returns `None` if absent or not decodable.
    pub fn get<C: Component>(&self, component: &C) -> Option<C::Value> {
        component.decode(&(self.component)(component.id())?)
    }
}

// no wire: a process-local palette for submitting baked geometry.
/// Colors to apply to submitted geometry.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum SpecialModelTints {
    /// No tint; all quads draw white before lighting.
    #[default]
    Untinted,
    /// Apply the same ARGB to every quad.
    Uniform(u32),
    /// ARGB by quad tint index. A missing or out-of-range index draws white before lighting.
    /// Keep a shared palette on the renderer or its argument to reuse it without copying.
    Indexed(Arc<[u32]>),
}

/// The most nested stacks one item draw may expand, counting all special layers and their
/// descendants. A cycle or an exhausted budget refuses the enclosing special layer.
pub const MAX_NESTED_STACK_PARTS: usize = 64;

// no wire: timing and display state for a single local draw callback.
/// The actual display context and frame time for this draw, even when the stack has not changed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SpecialModelContext {
    /// Where the item is drawn (a hand, a screen, the ground, a frame, the head).
    pub display: ItemDisplayContext,
    /// `getGameTime` of the level the item is drawn in; `None` outside a world.
    pub game_time: Option<i64>,
    /// How far the world is between its last tick and the next, in `[0, 1)`: the fraction the
    /// frame draws the world at.
    pub partial_tick: f32,
}

impl SpecialModelContext {
    /// The context of one drawn item; only the game builds one, mods read it.
    // hidden: the engine assembles the draw state from its current item render.
    #[doc(hidden)]
    pub const fn new(
        display: ItemDisplayContext,
        game_time: Option<i64>,
        partial_tick: f32,
    ) -> Self {
        Self {
            display,
            game_time,
            partial_tick,
        }
    }
}

// no wire: a scoped borrow of the current item layer's submission destination.
/// Submits a special model layer in order. A refusal discards everything submitted through this
/// collector, including geometry submitted by nested stacks.
#[non_exhaustive]
pub struct SpecialModelCollector<'a>(&'a mut dyn SpecialModelSubmission);

impl<'a> SpecialModelCollector<'a> {
    // hidden: the engine binds its frame-local geometry and nested item submission queue.
    #[doc(hidden)]
    /// Borrow an engine submission destination for this scope.
    pub fn new(sink: &'a mut dyn SpecialModelSubmission) -> Self {
        Self(sink)
    }

    /// Submit within a child pose, without changing the outer pose. `transform` is a row-major
    /// 3×4 affine in block units; the current pose multiplies it on the left. The model space
    /// starts at the model's corner (element coordinates divided by 16). A non-finite composed
    /// pose or an error from the child callback refuses the whole layer.
    pub fn with_pose(
        &mut self,
        transform: [f32; 12],
        draw: impl FnOnce(&mut SpecialModelCollector<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut draw = Some(draw);
        self.0.with_pose(transform, &mut |collector| {
            draw.take().ok_or("pose callback already used")?(collector)
        })
    }

    /// Submit a geometry lease from this resource load under the current pose and tints.
    /// The `base` model supplies display transforms, particle and GUI light; the item's layer
    /// supplies its foil. The engine applies the draw's packed light, overlay and outline as
    /// it does for ordinary item geometry.
    pub fn submit_geometry(
        &mut self,
        geometry: &SpecialModelGeometry,
        tints: SpecialModelTints,
    ) -> Result<(), String> {
        self.0.submit_geometry(geometry, tints)
    }

    /// Draw another stack under the current pose, which places its centre. The nested item
    /// uses its own display transform, tints and foil. A nested refusal (including unreadable
    /// input, cycles and the shared [`MAX_NESTED_STACK_PARTS`] budget) discards the whole layer
    /// even if the returned error is ignored.
    pub fn submit_stack(
        &mut self,
        stack: &ItemStack,
        display: ItemDisplayContext,
    ) -> Result<(), String> {
        self.0.submit_stack(stack, display)
    }
}

// no wire: engine-side submission operations for the current layer and pose.
/// Engine backend for a scoped [`SpecialModelCollector`].
pub trait SpecialModelSubmission {
    /// Compose a pose and invoke the callback in that scope exactly once if accepted.
    fn with_pose(
        &mut self,
        transform: [f32; 12],
        draw: &mut dyn FnMut(&mut SpecialModelCollector<'_>) -> Result<(), String>,
    ) -> Result<(), String>;

    /// Append baked item geometry.
    fn submit_geometry(
        &mut self,
        geometry: &SpecialModelGeometry,
        tints: SpecialModelTints,
    ) -> Result<(), String>;

    /// Expand another item stack in the same draw.
    fn submit_stack(
        &mut self,
        stack: &ItemStack,
        display: ItemDisplayContext,
    ) -> Result<(), String>;
}

// no wire: an erased process-local renderer callback, with its argument kept typed inside it.
/// Engine draw entry for a renderer baked by [`SpecialModelFactory`].
#[non_exhaustive]
pub struct BakedSpecialModelRenderer(Box<SubmitBakedModel>);

type SubmitBakedModel = dyn Fn(
        &dyn ClientHost,
        &SpecialModelStack<'_>,
        &SpecialModelContext,
        &mut SpecialModelCollector<'_>,
    ) -> Result<(), String>
    + Send
    + Sync;

impl BakedSpecialModelRenderer {
    /// Extract the typed draw argument and use it to submit the layer.
    pub fn submit(
        &self,
        host: &dyn ClientHost,
        stack: &SpecialModelStack<'_>,
        context: &SpecialModelContext,
        collector: &mut SpecialModelCollector<'_>,
    ) -> Result<(), String> {
        (self.0)(host, stack, context, collector)
    }
}

// no wire: baked in-process renderer; the draw argument is not a serialized recipe.
/// A mod's baked special model renderer: vanilla's `SpecialModelRenderer`. One instance is
/// baked per model node. Both callbacks run for each draw on the client, without a recipe cache.
pub trait SpecialModelRenderer: Send + Sync + 'static {
    /// The typed value passed from extraction to submission during the same draw.
    type Argument;

    /// Read the stack and host state needed by [`Self::submit`]. Read only the components you
    /// need; the game does not build an owned stack or clone the component patch for this call.
    fn extract_argument(
        &self,
        host: &dyn ClientHost,
        stack: &SpecialModelStack<'_>,
    ) -> Self::Argument;

    /// Submit the geometry and stacks for this draw in order. A submission fault, including a
    /// callback error, panic in either callback or invalid geometry or pose, discards the layer
    /// and disables the offending type until resources reload. Unreadable input or a nested
    /// refusal discards the enclosing layer without disabling it. Use `?` to propagate the
    /// collector's errors. Only the outermost refused layer draws a missing model; unrelated
    /// layers are left intact.
    fn submit(
        &self,
        argument: &Self::Argument,
        context: &SpecialModelContext,
        collector: &mut SpecialModelCollector<'_>,
    ) -> Result<(), String>;
}

/// The special model renderer registry `register_special_model_renderer` hands out: NeoForge's
/// `RegisterSpecialModelRendererEvent`.
pub trait SpecialModelRenderers {
    /// The counterpart of NeoForge's `register(location, codec)`: `factory` decodes and bakes
    /// each special model node of type `kind` (`mymod:skillet`). The last registration wins,
    /// including replacements for builtins; the game logs both owners. An id that is not
    /// `<namespace>:<path>` is logged and skipped.
    fn register(&mut self, kind: String, factory: SpecialModelFactory);
}

// no wire: a report the game logs at registration; it holds `&'static` mod ids.
/// A registration that replaced an earlier one of the same type.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReplacedSpecialModelRenderer {
    /// The type.
    pub kind: String,
    /// The mod that registered it first, when a mod's dispatch did.
    pub earlier: Option<&'static str>,
    /// The mod whose registration stays.
    pub later: Option<&'static str>,
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`SpecialModelRenderers`]: the registrations by type.
#[derive(Default)]
// exhaustive: its fields are private; the game builds it
pub struct SpecialModelRendererRegistry {
    factories: BTreeMap<String, (Option<&'static str>, SpecialModelFactory)>,
    replaced: Vec<ReplacedSpecialModelRenderer>,
    initial: Vec<(String, Option<&'static str>)>,
}

impl SpecialModelRenderers for SpecialModelRendererRegistry {
    fn register(&mut self, kind: String, factory: SpecialModelFactory) {
        let later = crate::content::crash::current_native_mod();
        if let Some((earlier, _)) = self.factories.insert(kind.clone(), (later, factory)) {
            self.replaced.push(ReplacedSpecialModelRenderer {
                kind,
                earlier,
                later,
            });
        } else {
            self.initial.push((kind, later));
        }
    }
}

impl SpecialModelRendererRegistry {
    /// The first mod registration of each type. The engine uses its own builtin decoder
    /// to distinguish replacement of a game type from a new type, irrespective of namespace.
    pub fn initial_owners(&self) -> impl Iterator<Item = (&str, Option<&'static str>)> {
        self.initial
            .iter()
            .map(|(kind, owner)| (kind.as_str(), *owner))
    }

    /// Consume the registry into its winning factories, keyed by type, and the list of
    /// registrations that replaced an earlier one.
    pub fn into_renderers(
        self,
    ) -> (
        BTreeMap<String, SpecialModelFactory>,
        Vec<ReplacedSpecialModelRenderer>,
    ) {
        let factories = self
            .factories
            .into_iter()
            .map(|(kind, (_, factory))| (kind, factory))
            .collect();
        (factories, self.replaced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nothing;
    impl SpecialModelRenderer for Nothing {
        type Argument = ();

        fn extract_argument(&self, _: &dyn ClientHost, _: &SpecialModelStack<'_>) {}

        fn submit(
            &self,
            _: &(),
            _: &SpecialModelContext,
            _: &mut SpecialModelCollector<'_>,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    /// The last registration of a type wins, vanilla's types included, and each replacement is
    /// reported.
    #[test]
    fn the_last_registration_of_a_type_wins() {
        let factory = || SpecialModelFactory::new(|_| Ok(()), |(), _| Ok(Nothing));
        let last = factory();
        let mut registry = SpecialModelRendererRegistry::default();
        registry.register("a:pan".into(), factory());
        registry.register("a:pan".into(), last.clone());
        registry.register("minecraft:shield".into(), factory());
        assert_eq!(
            registry.initial_owners().collect::<Vec<_>>(),
            vec![("a:pan", None), ("minecraft:shield", None)]
        );
        let (renderers, replaced) = registry.into_renderers();
        assert_eq!(renderers.len(), 2);
        assert!(Arc::ptr_eq(&renderers["a:pan"].0, &last.0));
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].kind, "a:pan");
    }
}
