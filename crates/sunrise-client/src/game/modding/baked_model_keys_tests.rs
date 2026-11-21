//! #861 part 1 through the game's own hooks: a client mod reads the keys of a block state's baked
//! models and copies of their quads in `model_baking_completed`, and a reload supersedes them.
use super::model_baking_completed_tests::reload;
use super::native_entity_refs::tests::with_client_game;
use mod_api::__private::{ModPhase, PhaseOwner};
use mod_api::{
    BakedModelKey, BakedModelView, BlockStateRecord, ClientHost, Hooks, ModEntry,
    ModelBakingCompletedEvent, ModelError, ModelRef,
};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};

/// What the mod read in each `model_baking_completed`: stone's keys and their copies.
#[derive(Default)]
struct Read {
    keys: Vec<Result<Vec<BakedModelKey>, ModelError>>,
    views: Vec<Vec<Result<BakedModelView, ModelError>>>,
    invalid_keys: Vec<[Result<Vec<BakedModelKey>, ModelError>; 2]>,
    /// An oak log lying along x and standing along y: two states of one block, two bindings.
    log_keys: Vec<[Result<Vec<BakedModelKey>, ModelError>; 2]>,
    /// Stone's drawable model, then lookups for an invalid ordinal and an unknown block.
    models: Vec<[Result<ModelRef, ModelError>; 3]>,
}
struct Reader(Arc<Mutex<Read>>);
#[mod_api::__subscriptions]
impl Hooks for Reader {
    fn model_baking_completed(&self, host: &mut dyn ClientHost, _: &mut ModelBakingCompletedEvent) {
        let mut read = self.0.lock().unwrap();
        let stone = host.block_state_id("minecraft:stone").expect("stone");
        let unknown = mod_api::__private::parts_installation::block_key(u32::MAX);
        let assets = host.render_assets();
        let keys = assets.block_state_models(stone);
        let views = keys
            .iter()
            .flatten()
            .map(|key| assets.baked_model(*key))
            .collect();
        read.invalid_keys.push([
            assets.block_state_models(BlockStateRecord::new(stone.block(), u16::MAX).into()),
            assets.block_state_models(BlockStateRecord::new(unknown, 0).into()),
        ]);
        read.log_keys.push(["x", "y"].map(|axis| {
            let state = host
                .block_state_id(&format!("minecraft:oak_log[axis={axis}]"))
                .expect("an oak log state");
            assets.block_state_models(state)
        }));
        read.models.push([
            assets.block_state_model(stone),
            assets.block_state_model(BlockStateRecord::new(stone.block(), u16::MAX).into()),
            assets.block_state_model(BlockStateRecord::new(unknown, 0).into()),
        ]);
        read.keys.push(keys);
        read.views.push(views);
    }
}

/// Stone's four weighted variants (26.3's blockstate lists four models, two of them turned half a
/// way round), each a full cube of six quads: read on the first load and again after a reload; the
/// first load's key answers `StaleModel` afterwards and the untouched model reads the same.
#[test]
#[ignore = "daily"]
fn a_client_mod_reads_a_block_states_baked_models_once_per_load() {
    let read = Arc::new(Mutex::new(Read::default()));
    let mods = vec![ModEntry::new("reader", Box::new(Reader(Arc::clone(&read))))].into();
    with_mods(mods, || {
        with_client_game(|game| {
            game.publish_initial_client_models().unwrap();
            let first = {
                let read = read.lock().unwrap();
                let Ok(keys) = &read.keys[0] else {
                    panic!("stone's keys: {:?}", read.keys);
                };
                assert_eq!(keys.len(), 4, "stone has four weighted variants");
                let Ok(view) = &read.views[0][0] else {
                    panic!("stone's model: {:?}", read.views);
                };
                assert_eq!(view.quads.len(), 6, "a full cube");
                assert!(
                    view.quads
                        .iter()
                        .all(|q| q.sprite == "minecraft:block/stone")
                );
                assert_eq!(
                    read.invalid_keys,
                    [[Err(ModelError::UnknownModel), Err(ModelError::UnknownModel)]],
                    "unknown ordinals and block keys are refused",
                );
                let [Ok(along_x), Ok(along_y)] = &read.log_keys[0] else {
                    panic!("oak log's keys: {:?}", read.log_keys);
                };
                assert!(!along_x.is_empty() && !along_y.is_empty());
                assert_ne!(along_x, along_y, "each axis state selects its own binding");
                let [stone_model, past_ordinal, unknown_block] = &read.models[0];
                let model = stone_model.as_ref().expect("stone's drawable model");
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                assert_eq!(*model, game.native_block_state_model(stone).unwrap());
                let slot = mod_api::__private::game::Fields::fields(model).slot;
                let faces = game.model_faces(slot).expect("drawable faces");
                assert_eq!(faces.len(), 6);
                assert!(
                    faces
                        .iter()
                        .all(|face| face.quad.material.sprite == "minecraft:block/stone")
                );
                assert_eq!(
                    past_ordinal,
                    &Err(ModelError::UnknownModel),
                    "an ordinal past the block's states is refused"
                );
                assert_eq!(
                    unknown_block,
                    &Err(ModelError::UnknownModel),
                    "an unknown block is refused"
                );
                (keys[0], view.clone())
            };
            reload(game);
            let read = read.lock().unwrap();
            let Ok(keys) = &read.keys[1] else {
                panic!("stone's keys after reload: {:?}", read.keys);
            };
            assert_ne!(keys[0], first.0, "a new generation's key");
            assert_eq!(
                read.views[1][0].as_ref(),
                Ok(&first.1),
                "untouched: the same quads"
            );
            assert_eq!(
                game.pack.render.baked_model(first.0),
                Err(ModelError::StaleModel),
                "the first load's key is superseded"
            );
            assert_eq!(game.pack.render.baked_model(keys[0]).as_ref(), Ok(&first.1));
            drop(read);

            // Keep the live registry but give the fixture an asset store that bakes no models.
            let cache = tempfile::tempdir().unwrap();
            let root = sunrise_assets::resource_source::ResourceRoot::new(
                game.options.resources.clone(),
                cache.path().to_owned(),
                game.pack.source.clone(),
            );
            game.pack.render = Arc::new(game.pack.assets.reload_for_server(&root).unwrap());
            let _phase = PhaseOwner::scope(ModPhase::World);
            let host: &dyn ClientHost = game;
            let stone = host.block_state_id("minecraft:stone").expect("stone");
            assert_eq!(host.render_assets().block_state_models(stone), Ok(vec![]));
        });
    });
}
