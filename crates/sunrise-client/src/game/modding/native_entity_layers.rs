//! Layers on living entity renderers, added by mods on the client: `native_register_entity_layers`
//! (NeoForge's `EntityRenderersEvent.AddLayers`) is asked on every resource load, after the entity
//! renderers resolve, and the layers it collects are resolved to the slot each draw reads: a mob's
//! type (its shared `MobType`, which the game's types never replace while it runs) or a player's
//! model. A frame then hands an entity to its own target's layers, after its model and the vanilla
//! layers, each through the guard renderers draw through ([`Game::guarded_entity_draw`]). With no
//! layer at all, a frame checks one flag per entity and builds nothing.
use super::{
    Game,
    entity_type_rows::EntityTypeRows,
    native_entity_renderers::RegisteredEntityRenderer,
    native_mob_draw::{entity_render_state, mob_pose},
};
use glam::DVec3;
use mod_api::{
    __private::game::{CallbackOwner, RegistrationContextNew, discard_registration},
    EntityLayers, EntityRenderLayer, FrameModel, HookId, LayerTarget, PlayerModel,
    RegistrationContext,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, LazyLock, Mutex, PoisonError, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use sunrise_client::{
    fast_hash::FxHashMap,
    mesher::Vertex,
    mobs::{Mob, MobType},
};

/// The player's entity type: its renderers are the player models', so its layers go to those.
const PLAYER: &str = "minecraft:player";

/// The player models, each with a renderer of its own.
const PLAYER_MODELS: [PlayerModel; 2] = [PlayerModel::Wide, PlayerModel::Slim];

/// One registered entity layer and its session callback owner (if added by a mod).
pub(in crate::game) struct RegisteredLayer {
    layer: Arc<dyn EntityRenderLayer>,
    owner: Option<CallbackOwner>,
}

impl RegisteredLayer {
    fn is_enabled(&self) -> bool {
        self.owner.as_ref().is_none_or(CallbackOwner::is_enabled)
    }
}

/// One target's layers, in the order they were added.
pub(in crate::game) type Layers = Arc<[RegisteredLayer]>;

struct LayerOwner {
    layer: Weak<dyn EntityRenderLayer>,
    owner: CallbackOwner,
}

type LayerOwners = HashMap<(&'static str, usize), LayerOwner>;

/// Session owners keyed by mod and callback allocation, independent of target and layer order.
/// The weak reference keeps the address from being reused while the entry exists, without
/// keeping the callback alive after a reload replaces it.
static LAYER_OWNERS: LazyLock<Mutex<LayerOwners>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Whether the last load resolved any layer: a frame checks only this when none was added.
static ANY: AtomicBool = AtomicBool::new(false);

/// The table the last resource load resolved; none before the first.
static TABLE: RwLock<Option<LayerTable>> = RwLock::new(None);

type LayerEdit = (Option<&'static str>, Arc<dyn EntityRenderLayer>);

/// What one `register_entity_layers` round collects: the [`EntityLayers`] mods are handed, with
/// what this load's renderers are, and the layers added to each target.
pub(in crate::game) struct LayerRegistration<'a> {
    rows: &'a EntityTypeRows,
    mod_renderers: &'a BTreeMap<String, RegisteredEntityRenderer>,
    /// Whether a living type draws where layers run (as a mob).
    routed: &'a dyn Fn(&str) -> bool,
    entity: BTreeMap<String, Vec<LayerEdit>>,
    wide: Vec<LayerEdit>,
    slim: Vec<LayerEdit>,
}

impl<'a> LayerRegistration<'a> {
    pub(super) fn new(
        rows: &'a EntityTypeRows,
        mod_renderers: &'a BTreeMap<String, RegisteredEntityRenderer>,
        routed: &'a dyn Fn(&str) -> bool,
    ) -> Self {
        Self {
            rows,
            mod_renderers,
            routed,
            entity: BTreeMap::new(),
            wide: Vec::new(),
            slim: Vec::new(),
        }
    }

    pub(in crate::game) fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn EntityLayers) -> bool,
    ) {
        let mut pending = Self::new(self.rows, self.mod_renderers, self.routed);
        if listen(&mut pending) {
            for (id, layers) in pending.entity {
                self.entity.entry(id).or_default().extend(layers);
            }
            self.wide.extend(pending.wide);
            self.slim.extend(pending.slim);
        } else {
            for (owner, layer) in pending
                .entity
                .into_values()
                .flatten()
                .chain(pending.wide)
                .chain(pending.slim)
            {
                discard_registration(
                    (HookId::RegisterEntityLayers, "register_entity_layers"),
                    owner,
                    layer,
                );
            }
        }
    }

    /// Why `id` can't take a layer; none for a living entity type.
    fn refusal(&self, id: &str) -> Option<&'static str> {
        if id == PLAYER {
            Some("a player takes layers through its model (wide or slim)")
        } else if !self.rows.contains(id) {
            Some("it is no entity type")
        } else if !self.rows.is_living(id) {
            Some("its renderer is not a living entity's, and only those take layers")
        } else if !(self.routed)(id) {
            Some("it draws through a route of its own, which does not run layers yet")
        } else {
            None
        }
    }
}

impl EntityLayers for LayerRegistration<'_> {
    fn entity_types(&self) -> &[String] {
        &self.rows.non_player_ids
    }

    fn is_living(&self, entity_type: &str) -> bool {
        entity_type != PLAYER && self.rows.is_living(entity_type)
    }

    fn player_models(&self) -> &[PlayerModel] {
        &PLAYER_MODELS
    }

    fn has_mod_renderer(&self, entity_type: &str) -> bool {
        self.rows.contains(entity_type) && self.mod_renderers.contains_key(entity_type)
    }

    fn add(
        &mut self,
        target: LayerTarget,
        layer: Arc<dyn EntityRenderLayer>,
    ) -> Result<(), String> {
        let layers = match target {
            LayerTarget::Player(PlayerModel::Wide) => &mut self.wide,
            LayerTarget::Player(PlayerModel::Slim) => &mut self.slim,
            LayerTarget::Entity(id) => {
                if let Some(why) = self.refusal(&id) {
                    let why = format!("the entity layer for {id} is refused: {why}");
                    log::warn!("Sunrise: {why}");
                    return Err(why);
                }
                self.entity.entry(id).or_default()
            }
        };
        layers.push((mod_api::content::crash::current_native_mod(), layer));
        Ok(())
    }
}

/// The layers by the slot each draw reads: rebuilt on every resource load.
#[derive(Default)]
pub(super) struct LayerTable {
    /// By the address of the mob type the game shares with every mob of the type.
    mobs: FxHashMap<usize, Layers>,
    wide: Option<Layers>,
    slim: Option<Layers>,
}

impl LayerTable {
    /// Resolve the successfully registered layers against the types the game draws as mobs.
    fn resolve(
        registration: LayerRegistration<'_>,
        mob_types: &HashMap<String, Arc<MobType>>,
        owners: &Mutex<LayerOwners>,
    ) -> Self {
        let resolved = |layers: Vec<LayerEdit>| -> Option<Layers> {
            (!layers.is_empty()).then(|| {
                layers
                    .into_iter()
                    .map(|(mod_id, layer)| {
                        let owner = mod_id.map(|mod_id| {
                            owners
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .entry((mod_id, Arc::as_ptr(&layer).cast::<()>() as usize))
                                .or_insert_with(|| LayerOwner {
                                    layer: Arc::downgrade(&layer),
                                    owner: CallbackOwner::for_mod(mod_id),
                                })
                                .owner
                                .clone()
                        });
                        RegisteredLayer { layer, owner }
                    })
                    .collect()
            })
        };
        let mobs = registration
            .entity
            .into_iter()
            .filter_map(|(id, layers)| {
                let kind = mob_types.get(&id)?;
                Some((Arc::as_ptr(kind) as usize, resolved(layers)?))
            })
            .collect();
        Self {
            mobs,
            wide: resolved(registration.wide),
            slim: resolved(registration.slim),
        }
    }

    fn is_empty(&self) -> bool {
        self.mobs.is_empty() && self.wide.is_none() && self.slim.is_none()
    }

    /// The layers of `mob`: its player model's for a player, else its type's.
    pub(super) fn of(&self, mob: &Mob) -> Option<&Layers> {
        match (&*mob.kind.id == PLAYER, mob.slim) {
            (true, true) => self.slim.as_ref(),
            (true, false) => self.wide.as_ref(),
            (false, _) => self.mobs.get(&(Arc::as_ptr(&mob.kind) as usize)),
        }
    }
}

impl Game<'_> {
    /// The layers the last load resolved for `mob` (a player's model's for a player), looked up
    /// once per mob a frame: the answer both decides whether the draw keeps the mob's final model
    /// and is what `mod_layer_geometry` draws. With no layer added at all it is none after one
    /// flag check, with no lock and no lookup.
    pub(in crate::game) fn mod_layers(&self, mob: &Mob) -> Option<Layers> {
        if !ANY.load(Ordering::Relaxed) {
            return None;
        }
        let table = TABLE.read().unwrap_or_else(PoisonError::into_inner);
        table
            .as_ref()?
            .of(mob)
            .filter(|layers| layers.iter().any(RegisteredLayer::is_enabled))
            .cloned()
    }

    /// Asks the mods for their layers on the renderers this load built (`rows` its entity types,
    /// `mod_renderers` the registered renderers) and resolves them.
    pub(super) fn resolve_entity_layers(
        &self,
        rows: &EntityTypeRows,
        mod_renderers: &BTreeMap<String, RegisteredEntityRenderer>,
    ) {
        let mob_types = self.res.mob_types();
        let routed = |id: &str| mob_types.contains_key(id);
        let mut registration = LayerRegistration::new(rows, mod_renderers, &routed);
        Self::native_register_entity_layers(&RegistrationContext::new(), &mut registration);
        let table = LayerTable::resolve(registration, mob_types, &LAYER_OWNERS);
        let any = !table.is_empty();
        *TABLE.write().unwrap_or_else(PoisonError::into_inner) = Some(table);
        ANY.store(any, Ordering::Relaxed);
        // Drop stale identities after the previous table releases its layers.
        LAYER_OWNERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, entry| entry.layer.strong_count() != 0);
    }

    /// Draws `layers` over `mob`, told as the entity `id`, each from its frame (at the feet,
    /// turned with the body), in the order they were added. Each layer borrows the same final
    /// `model`, if this draw produced a body pose.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn registered_layer_geometry(
        &self,
        mob: &Mob,
        id: u32,
        layers: &Layers,
        model: Option<&dyn FrameModel>,
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let entity = entity_render_state(mob, id, partial);
        let feet = mob.previous.lerp(mob.position, f64::from(partial));
        let feet = DVec3::new(feet.x, feet.y, feet.z);
        let pose = mob_pose(
            feet - camera,
            entity.living.expect("mob living state").body_rot,
        );
        let block = [feet.x, feet.y, feet.z].map(|c| c.floor() as i32);
        for layer in layers.iter() {
            self.guarded_entity_draw(
                layer.owner.as_ref(),
                "entity layer",
                "EntityRenderLayer::submit",
                partial,
                &mob.kind.id,
                block,
                pose,
                light,
                None,
                out,
                |draw| layer.layer.submit(&entity, model, draw),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{
        BlockEntityDrawContext, EntityRenderState, Hooks, ModEntry, RegisterEntityLayersEvent,
    };

    struct NoDraw;
    impl EntityRenderLayer for NoDraw {
        fn submit(
            &self,
            _: &EntityRenderState,
            _: Option<&dyn FrameModel>,
            _: &mut dyn BlockEntityDrawContext,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    struct Registrant {
        layer: Arc<dyn EntityRenderLayer>,
        panic_after: Option<Arc<AtomicBool>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Registrant {
        fn register_entity_layers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterEntityLayersEvent<'_>,
        ) {
            for target in [
                LayerTarget::Entity("test:golem".into()),
                LayerTarget::Player(PlayerModel::Wide),
                LayerTarget::Player(PlayerModel::Slim),
            ] {
                event.layers.add(target, self.layer.clone()).unwrap();
            }
            if let Some(added) = &self.panic_after {
                added.store(true, Ordering::Relaxed);
                panic!("after adding all three layers");
            }
        }
    }

    #[test]
    fn a_panicking_layer_registrar_does_not_publish_its_layers() {
        let added = Arc::new(AtomicBool::new(false));
        let earlier: Arc<dyn EntityRenderLayer> = Arc::new(NoDraw);
        let later: Arc<dyn EntityRenderLayer> = Arc::new(NoDraw);
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "earlier",
                    Box::new(Registrant {
                        layer: earlier.clone(),
                        panic_after: None,
                    }),
                ),
                ModEntry::new(
                    "failed",
                    Box::new(Registrant {
                        layer: Arc::new(NoDraw),
                        panic_after: Some(added.clone()),
                    }),
                ),
                ModEntry::new(
                    "later",
                    Box::new(Registrant {
                        layer: later.clone(),
                        panic_after: None,
                    }),
                ),
            ]
            .into(),
            || {
                let rows = EntityTypeRows::parse(br#"{"test:golem":{"attributes":{}}}"#);
                let renderers = BTreeMap::new();
                let mut registration = LayerRegistration::new(&rows, &renderers, &|_| true);
                Game::native_register_entity_layers(&RegistrationContext::new(), &mut registration);
                assert!(added.load(Ordering::Relaxed));
                let entity = &registration.entity["test:golem"];
                assert_eq!(entity.len(), 2);
                assert!(Arc::ptr_eq(&entity[0].1, &earlier));
                assert!(Arc::ptr_eq(&entity[1].1, &later));
                let owners = Mutex::default();
                let table = LayerTable::resolve(registration, &HashMap::new(), &owners);
                for list in [table.wide.unwrap(), table.slim.unwrap()] {
                    assert_eq!(list.len(), 2);
                    assert!(Arc::ptr_eq(&list[0].layer, &earlier));
                    assert!(Arc::ptr_eq(&list[1].layer, &later));
                }
            },
        );
    }
}
