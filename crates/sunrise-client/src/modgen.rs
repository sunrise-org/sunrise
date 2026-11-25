//! A registered chunk generator that runs no vanilla terrain stages: just the resolved biome
//! source and the blocks its `build_terrain` writes.
use crate::{
    chunk::{Chunk, LevelShape},
    streaming::ChunkGenerator,
};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};
use sunrise_core::{
    density::{ConstantSampler, DensitySampler, SamplerContext},
    position::ChunkPos,
};
use sunrise_world::{
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        biome_source::ResolvedBiomeSource,
        climate::ClimateSampler,
        mod_generators::{ChunkAccess, ModChunkGenerator},
        structure::legacy::block_state,
    },
};

/// `ChunkMap` uses `NoiseRouterData.none()` for a non-noise generator. Its climate and erosion
/// fields are all zero; a biome source does not opt the terrain into using a noise router.
const ZERO: ConstantSampler = ConstantSampler(0.0);
static CLIMATE: LazyLock<ClimateSampler> = LazyLock::new(|| {
    let zero: Arc<dyn DensitySampler> = Arc::new(ZERO);
    ClimateSampler::new(std::array::from_fn(|_| zero.clone()))
});

/// A mod's generator over one level.
pub struct ModTerrain {
    generator: Arc<dyn ModChunkGenerator>,
    registry: &'static BlockRegistry,
    shape: LevelShape,
    source: Arc<ResolvedBiomeSource>,
    biome_ids: HashMap<String, u16>,
    level: Box<str>,
    seed: i64,
}

impl ModTerrain {
    /// A generator paired with a source already bound by `BiomeSourceData::resolve_mod_biome_source`.
    /// Translate its possible holders to the client biome table once, before any chunk fills.
    pub fn new(
        level: &str,
        generator: Arc<dyn ModChunkGenerator>,
        source: Arc<ResolvedBiomeSource>,
        registry: &'static BlockRegistry,
        biome_id: impl Fn(&str) -> Option<u16>,
        shape: LevelShape,
        seed: i64,
    ) -> Result<Self, String> {
        let biome_ids = source
            .possible_biomes()
            .into_iter()
            .map(|name| {
                biome_id(name)
                    .map(|id| (name.to_owned(), id))
                    .ok_or_else(|| format!("{level}: unknown biome {name}"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            generator,
            registry,
            shape,
            source,
            biome_ids,
            level: level.into(),
            seed,
        })
    }
}

/// The proto chunk as a generator writes it.
struct Filling<'a> {
    chunk: &'a mut Chunk,
    terrain: &'a ModTerrain,
    pos: ChunkPos,
}

impl ChunkAccess for Filling<'_> {
    fn registry(&self) -> &'static BlockRegistry {
        self.terrain.registry
    }
    fn pos(&self) -> ChunkPos {
        self.pos
    }
    fn level(&self) -> &str {
        &self.terrain.level
    }
    fn seed(&self) -> i64 {
        self.terrain.seed
    }
    fn min_y(&self) -> i32 {
        self.terrain.shape.min_y
    }
    fn height(&self) -> i32 {
        self.terrain.shape.height
    }
    fn state(&self, name: &str) -> BlockStateId {
        block_state(self.terrain.registry, name, &[])
    }
    fn state_with(&self, name: &str, properties: &[(&str, &str)]) -> BlockStateId {
        block_state(self.terrain.registry, name, properties)
    }
    fn set_block(&mut self, [x, y, z]: [i32; 3], state: BlockStateId) -> bool {
        let inside = x >> 4 == self.pos.x
            && z >> 4 == self.pos.z
            && (self.min_y()..self.min_y() + self.height()).contains(&y);
        if inside {
            self.chunk.set(x & 15, y, z & 15, state.0);
        }
        inside
    }
}

impl ChunkGenerator for ModTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(
        &self,
        _: &mut (),
        cx: i32,
        cz: i32,
    ) -> std::result::Result<crate::streaming::Proto, String> {
        let mut chunk = Chunk::with_layout(cx, cz, self.shape.column());
        if let ResolvedBiomeSource::Fixed(source) = self.source.as_ref() {
            let biome = self.biome_ids[&source.biome];
            for s in &mut chunk.sections {
                Arc::make_mut(s).biomes = sunrise_core::paletted::PalettedContainer::new(
                    crate::chunk::BIOME_STRATEGY,
                    u32::from(biome),
                );
            }
        } else {
            // ChunkGenerator.doCreateBiomes: one resolver fills all quart cells of the chunk.
            let mut resolver = self.source.resolver(&CLIMATE, &ZERO);
            let mut context = SamplerContext::default();
            let bottom = self.shape.min_y >> 2;
            let (west, north) = (cx.wrapping_mul(4), cz.wrapping_mul(4));
            for qy in 0..chunk.sections.len() as i32 * 4 {
                for qz in 0..4 {
                    for qx in 0..4 {
                        // The bound source has holders to sample and the non-noise router is constant.
                        // A registered source's failure (its code panicked, or its mod is off)
                        // fails the column: nothing of it is published.
                        let name = resolver
                            .sample(&mut context, [west + qx, bottom + qy, north + qz])
                            .map_err(|error| error.to_string())?;
                        chunk.set_biome_cell(qx, qy, qz, self.biome_ids[name]);
                    }
                }
            }
        }
        self.generator.build_terrain(&mut Filling {
            chunk: &mut chunk,
            terrain: self,
            pos: ChunkPos::new(cx, cz),
        });
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        Ok(Box::new(chunk))
    }
    fn light_view(&self, proto: &crate::streaming::Proto) -> Option<Chunk> {
        proto.downcast_ref::<Chunk>().cloned()
    }
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        *proto.downcast::<Chunk>().expect("mod terrain proto")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};
    use sunrise_world::worldgen::{
        biome_source::BiomeSourceCatalog,
        mod_biome_sources::{LevelBiomeSource, ModBiomeSource, ModBiomeSources},
        mod_generators::ModChunkGenerators,
        overrides::ModDimension,
    };

    const STONE: BlockStateId = BlockStateId(1);
    const BEDROCK: BlockStateId = BlockStateId(2);

    fn registry() -> &'static BlockRegistry {
        Box::leak(Box::new(
            BlockRegistry::from_facts_json(
                &br#"{"states":[["minecraft:air",true,false,"","minecraft:air"],["minecraft:stone",true,false,"","minecraft:stone"],["minecraft:bedrock",true,false,"","minecraft:bedrock"]],"defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:bedrock":2}}"#[..],
            )
            .unwrap(),
        ))
    }

    /// Stone over the chunk, `seed` blocks above the floor, and bedrock in
    /// the top block of one column; writes past the chunk are skipped.
    struct Slab;
    impl ModChunkGenerator for Slab {
        fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
            let (pos, floor) = (chunk.pos(), chunk.min_y() + chunk.seed() as i32);
            let top = chunk.min_y() + chunk.height() - 1;
            let (stone, bedrock) = (chunk.state("stone"), chunk.state("minecraft:bedrock"));
            for x in pos.x * 16..pos.x * 16 + 16 {
                for z in pos.z * 16..pos.z * 16 + 16 {
                    assert!(chunk.set_block([x, floor, z], stone));
                }
            }
            let corner = [pos.x * 16, top, pos.z * 16];
            assert!(chunk.set_block(corner, bedrock));
            assert!(!chunk.set_block([corner[0], top + 1, corner[2]], stone));
            assert!(!chunk.set_block([corner[0] - 1, floor, corner[2]], stone));
            assert!(!chunk.set_block([corner[0], floor, corner[2] + 16], stone));
        }
    }

    fn generators() -> ModChunkGenerators {
        ModChunkGenerators::from([(
            "test:slab".to_owned(),
            Arc::new(Slab) as Arc<dyn ModChunkGenerator>,
        )])
    }

    fn stem(generator: &str) -> serde_json::Value {
        json!({
            "type": "test:pocket",
            "generator": {
                "type": generator,
                "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:lush_caves"},
            },
        })
    }

    /// Desert at the one quart cell `[13, 5 + seed, -7]`, plains everywhere else.
    struct Oasis(Vec<String>);
    impl ModBiomeSource for Oasis {
        fn possible_biomes(&self) -> &[String] {
            &self.0
        }
        fn noise_biome(&self, quart: [i32; 3], seed: i64) -> Result<u16, String> {
            Ok(u16::from(quart == [13, 5 + seed as i32, -7]))
        }
    }

    fn sources(biomes: &[&str]) -> ModBiomeSources {
        let oasis = Oasis(biomes.iter().map(|b| (*b).to_owned()).collect());
        ModBiomeSources::from([("test:oasis".to_owned(), Arc::new(oasis) as _)])
    }

    /// The test world's biome ids: plains 0, desert 3, lush caves 7.
    fn biome_id(name: &str) -> Option<u16> {
        match name {
            "minecraft:plains" => Some(0),
            "minecraft:desert" => Some(3),
            "minecraft:lush_caves" => Some(7),
            "minecraft:the_end" => Some(8),
            "minecraft:end_highlands" => Some(9),
            "minecraft:end_midlands" => Some(10),
            "minecraft:small_end_islands" => Some(11),
            "minecraft:end_barrens" => Some(12),
            _ => None,
        }
    }

    fn for_stem(
        level: &str,
        stem: &serde_json::Value,
        generators: &ModChunkGenerators,
        sources: &ModBiomeSources,
        registry: &'static BlockRegistry,
        biome_id: impl Fn(&str) -> Option<u16>,
        shape: LevelShape,
        seed: i64,
    ) -> Option<Result<ModTerrain, String>> {
        let generator = generators.get(stem["generator"]["type"].as_str()?)?;
        if generator.vanilla_stages().any() {
            return None;
        }
        let source = &stem["generator"]["biome_source"];
        let resolved = if let Some(name) = source["type"]
            .as_str()
            .filter(|name| name.starts_with("test:"))
        {
            // Only these fixture sources use a local registry. Built-in sources go through the
            // production codec; the registered-source codec has its own integration test.
            LevelBiomeSource::resolve(name, sources, seed, |name| {
                biome_id(name).map(|_| name.to_owned())
            })
            .map(ResolvedBiomeSource::Registered)
            .map_err(|e| e.to_string())
        } else {
            let biomes: BTreeSet<_> = [
                "plains",
                "desert",
                "lush_caves",
                "the_end",
                "end_highlands",
                "end_midlands",
                "small_end_islands",
                "end_barrens",
            ]
            .map(|name| format!("minecraft:{name}"))
            .into();
            let catalog =
                BiomeSourceCatalog::new(&biomes, &BTreeMap::new(), &BTreeSet::new()).unwrap();
            catalog
                .bind_custom_source(&source.to_string(), &BTreeMap::new(), seed)
                .map_err(|e| e.to_string())
        };
        Some(resolved.and_then(|source| {
            ModTerrain::new(
                level,
                generator.clone(),
                Arc::new(source),
                registry,
                biome_id,
                shape,
                seed,
            )
        }))
    }

    fn terrain(stem: &serde_json::Value, shape: LevelShape, seed: i64) -> Option<ModTerrain> {
        let sources = sources(&["minecraft:plains", "minecraft:desert"]);
        for_stem(
            "test:pocket",
            stem,
            &generators(),
            &sources,
            registry(),
            biome_id,
            shape,
            seed,
        )
        .map(Result::unwrap)
    }

    fn oasis_stem() -> serde_json::Value {
        json!({"type": "test:pocket", "generator": {"type": "test:slab", "biome_source": {"type": "test:oasis"}}})
    }

    fn chunk(terrain: &ModTerrain, x: i32, z: i32) -> Chunk {
        terrain.finish(terrain.proto(&mut (), x, z).unwrap())
    }

    /// A registered source whose code fails (as a panic caught under its owner reports).
    struct Broken(Vec<String>);
    impl ModBiomeSource for Broken {
        fn possible_biomes(&self) -> &[String] {
            &self.0
        }
        fn noise_biome(&self, _: [i32; 3], _: i64) -> Result<u16, String> {
            Err("mod broken_pack: panicked".into())
        }
    }

    #[test]
    fn a_failing_biome_source_fails_its_column_while_a_healthy_one_publishes() {
        let mut sources = sources(&["minecraft:plains", "minecraft:desert"]);
        sources.insert(
            "test:broken".to_owned(),
            Arc::new(Broken(vec!["minecraft:plains".to_owned()])) as _,
        );
        let build = |source: &str| {
            let stem = json!({"type": "test:pocket", "generator": {"type": "test:slab", "biome_source": {"type": source}}});
            for_stem(
                "test:pocket",
                &stem,
                &generators(),
                &sources,
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                2,
            )
            .unwrap()
            .unwrap()
        };
        let broken = build("test:broken");
        let error = broken.proto(&mut (), 0, 0).expect_err("the column fails");
        assert!(error.contains("panicked"), "{error}");
        let healthy = build("test:oasis");
        assert!(healthy.proto(&mut (), 0, 0).is_ok());
    }

    #[test]
    fn a_generators_blocks_appear_in_its_levels_chunk() {
        let terrain = terrain(&stem("test:slab"), LevelShape::OVERWORLD, 5).unwrap();
        let chunk = chunk(&terrain, 3, -2);
        assert_eq!(chunk.block(0, -59, 0), STONE);
        assert_eq!(chunk.block(15, -59, 15), STONE);
        assert_eq!(chunk.block(7, -60, 7), BlockStateId(0));
        assert_eq!(chunk.block(7, -58, 7), BlockStateId(0));
        assert_eq!(chunk.block(0, 319, 0), BEDROCK);
        assert_eq!(chunk.block(1, 319, 0), BlockStateId(0));
        assert_eq!(chunk.sections[0].biomes.get(0), 7);
    }

    /// One stateless generator serves two levels and tells them apart by `level()`.
    #[test]
    fn a_generator_is_told_the_level_it_fills() {
        struct ByLevel;
        impl ModChunkGenerator for ByLevel {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                let y = match chunk.level() {
                    "test:alpha" => -59,
                    "test:beta" => -58,
                    other => panic!("unexpected level {other}"),
                };
                let stone = chunk.state("stone");
                assert!(chunk.set_block([0, y, 0], stone));
            }
        }
        let generators = ModChunkGenerators::from([(
            "test:by_level".to_owned(),
            Arc::new(ByLevel) as Arc<dyn ModChunkGenerator>,
        )]);
        let open = |level: &str| {
            let stem = stem("test:by_level");
            for_stem(
                level,
                &stem,
                &generators,
                &sources(&[]),
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                5,
            )
            .unwrap()
            .unwrap()
        };
        let (alpha, beta) = (
            chunk(&open("test:alpha"), 0, 0),
            chunk(&open("test:beta"), 0, 0),
        );
        assert_eq!(alpha.block(0, -59, 0), STONE);
        assert_eq!(alpha.block(0, -58, 0), BlockStateId(0));
        assert_eq!(beta.block(0, -58, 0), STONE);
        assert_eq!(beta.block(0, -59, 0), BlockStateId(0));
    }

    #[test]
    fn a_levels_height_comes_from_its_dimension_type() {
        let pocket = ModDimension {
            id: "test:pocket".into(),
            stem: stem("test:slab").to_string(),
            dimension_type:
                r#"{"min_y":-128,"height":448,"logical_height":448,"has_skylight":true,"has_ceiling":false}"#
                    .into(),
            sea_level: None,
        };
        let (overrides, _) = crate::datapack::worldgen_overrides_over(
            &[pocket],
            &crate::datapack::DataPacks::default(),
            |_| true,
        );
        let height = overrides.level_height("test:pocket").unwrap();
        let shape = LevelShape::new(
            height.min_y,
            height.height,
            height.logical_height,
            height.has_skylight,
            height.has_ceiling,
        )
        .unwrap();
        let terrain = terrain(&stem("test:slab"), shape, 0).unwrap();
        let chunk = chunk(&terrain, 0, 0);
        assert_eq!(chunk.sections.len(), 28);
        assert_eq!(chunk.block(4, -128, 4), STONE);
        assert_eq!(chunk.block(0, 319, 0), BEDROCK);
    }

    /// The oasis cell of chunk 3 -2 at seed 2 is blocks 52..56, 28..32 and
    /// -28..-24: chunk columns 4..8, and plains around it.
    #[test]
    fn a_registered_sources_biome_is_what_the_chunk_gets_at_its_quart_cell() {
        let terrain = terrain(&oasis_stem(), LevelShape::OVERWORLD, 2).unwrap();
        let chunk = chunk(&terrain, 3, -2);
        for [x, y, z] in [[4, 28, 4], [7, 31, 7], [5, 30, 6]] {
            assert_eq!(chunk.biome(x, y, z), 3, "{x} {y} {z}");
        }
        for [x, y, z] in [
            [3, 28, 4],
            [8, 28, 4],
            [4, 27, 4],
            [4, 32, 4],
            [4, 28, 8],
            [0, -64, 0],
        ] {
            assert_eq!(chunk.biome(x, y, z), 0, "{x} {y} {z}");
        }
    }

    /// A source naming a biome the world lacks, and a source nobody registered
    /// (its mod removed), refuse the level by name.
    #[test]
    fn a_missing_biome_or_source_refuses_the_level() {
        let refused = |stem: serde_json::Value, biomes: &[&str]| {
            for_stem(
                "test:pocket",
                &stem,
                &generators(),
                &sources(biomes),
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                0,
            )
            .unwrap()
            .err()
        };
        assert_eq!(
            refused(oasis_stem(), &["minecraft:plains", "test:mirage"]).as_deref(),
            Some("biome source test:oasis names the unknown biome test:mirage")
        );
        let mut gone = oasis_stem();
        gone["generator"]["biome_source"]["type"] = json!("test:gone");
        assert_eq!(
            refused(gone, &["minecraft:plains"]).as_deref(),
            Some("unknown biome source test:gone")
        );
    }

    /// A chunk saved and read back keeps the biomes the source placed: the
    /// region file stores them by name, through the world's biome table.
    #[test]
    #[ignore = "daily"]
    fn a_registered_sources_biomes_survive_saving_and_loading() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let assets = crate::assets::Assets::load(&root).unwrap();
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root),
        )
        .unwrap();
        let sources = sources(&["minecraft:plains", "minecraft:desert"]);
        let known = |name: &str| biomes.contains(name).then(|| biomes.id(name));
        let shape = LevelShape::OVERWORLD;
        let terrain = for_stem(
            "test:pocket",
            &oasis_stem(),
            &generators(),
            &sources,
            assets.registry,
            known,
            shape,
            2,
        )
        .unwrap()
        .unwrap();
        let chunk = chunk(&terrain, 3, -2);
        let tables = crate::save::SaveTables::new(&assets);
        let saved = crate::save::encode_with(
            &chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            shape,
            &Default::default(),
        );
        let back = crate::save::decode_with(
            &saved,
            assets.registry,
            &biomes,
            Some(&tables),
            shape.column(),
        )
        .unwrap();
        let name = |x, y, z| biomes.name(back.biome(x, y, z)).to_owned();
        assert_eq!(name(4, 28, 4), "minecraft:desert");
        assert_eq!(name(7, 31, 7), "minecraft:desert");
        assert_eq!(name(8, 28, 4), "minecraft:plains");
        assert_eq!(name(4, 32, 4), "minecraft:plains");
        assert_eq!(name(0, -64, 0), "minecraft:plains");
    }

    /// A generator whose stages are all off makes the chunk it makes today,
    /// its own blocks and nothing else; one that asks for any vanilla stage is
    /// left to the vanilla pipeline.
    #[test]
    fn only_a_generator_without_vanilla_stages_makes_the_mods_terrain_alone() {
        use sunrise_world::worldgen::mod_generators::VanillaStages;
        struct Staged(VanillaStages);
        impl ModChunkGenerator for Staged {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                Slab.build_terrain(chunk);
            }
            fn vanilla_stages(&self) -> VanillaStages {
                self.0
            }
        }
        let for_stem = |stages| {
            let generators = ModChunkGenerators::from([(
                "test:slab".to_owned(),
                Arc::new(Staged(stages)) as Arc<dyn ModChunkGenerator>,
            )]);
            let stem = stem("test:slab");
            for_stem(
                "test:pocket",
                &stem,
                &generators,
                &sources(&["minecraft:plains"]),
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                5,
            )
        };
        let off = for_stem(VanillaStages::default()).unwrap().unwrap();
        let chunk = chunk(&off, 3, -2);
        assert_eq!(chunk.block(9, -60, 4), BlockStateId(0));
        assert_eq!(chunk.block(9, -59, 4), STONE);
        assert_eq!(chunk.block(9, -58, 4), BlockStateId(0));
        assert_eq!(chunk.block(9, 63, 4), BlockStateId(0));
        assert_eq!(chunk.block(0, 319, 0), BEDROCK);
        for stages in [
            VanillaStages {
                surface: true,
                ..Default::default()
            },
            VanillaStages {
                carvers: true,
                ..Default::default()
            },
            VanillaStages {
                structures: true,
                ..Default::default()
            },
            VanillaStages {
                features: true,
                ..Default::default()
            },
        ] {
            assert!(for_stem(stages).is_none(), "{stages:?}");
        }
    }

    /// Both stay on the game's own generator: noise as it always was, and a
    /// generator nobody registered to its error.
    #[test]
    fn only_a_registered_generator_makes_mod_terrain() {
        for generator in ["minecraft:noise", "test:gone"] {
            assert!(terrain(&stem(generator), LevelShape::OVERWORLD, 0).is_none());
        }
    }

    /// A custom chunk generator paired with a direct `minecraft:checkerboard` biome source fills
    /// each quart cell from the checkerboard squares directly in `ModTerrain`.
    #[test]
    fn mod_generator_with_direct_checkerboard_fills_biomes() {
        let stem = json!({
            "type": "test:pocket",
            "generator": {
                "type": "test:slab",
                "biome_source": {
                    "type": "minecraft:checkerboard",
                    "biomes": ["plains", "desert"],
                    "scale": 0,
                },
            },
        });
        let terrain = terrain(&stem, LevelShape::OVERWORLD, 5).unwrap();
        let c0 = chunk(&terrain, 0, 0);
        // scale 0 => square width 2^(0 + 2) = 4 quart cells = 1 chunk (16 blocks).
        assert_eq!(c0.biome(0, 0, 0), 0);
        assert_eq!(c0.biome(15, 64, 15), 0);
        let c1 = chunk(&terrain, 1, 0);
        assert_eq!(c1.biome(0, 0, 0), 3);
        let c2 = chunk(&terrain, 1, 1);
        assert_eq!(c2.biome(0, 0, 0), 0);
    }

    /// Refuse an absent or malformed holder instead of substituting plains.
    #[test]
    fn mod_terrain_refuses_invalid_checkerboard_or_fixed_biomes() {
        for bad in [
            json!({"generator": {"type": "test:slab"}}),
            json!({"generator": {"type": "test:slab", "biome_source": {"type": "fixed"}}}),
            json!({"generator": {"type": "test:slab", "biome_source": {"type": "fixed", "biome": 0}}}),
            json!({
                "type": "test:pocket",
                "generator": {
                    "type": "test:slab",
                    "biome_source": {"type": "minecraft:fixed", "biome": "test:missing"},
                },
            }),
            json!({
                "type": "test:pocket",
                "generator": {
                    "type": "test:slab",
                    "biome_source": {"type": "minecraft:checkerboard", "biomes": [], "scale": 2},
                },
            }),
            json!({
                "type": "test:pocket",
                "generator": {
                    "type": "test:slab",
                    "biome_source": {
                        "type": "minecraft:checkerboard",
                        "biomes": ["minecraft:plains"],
                        "scale": 63,
                    },
                },
            }),
            json!({
                "type": "test:pocket",
                "generator": {
                    "type": "test:slab",
                    "biome_source": {
                        "type": "minecraft:checkerboard",
                        "biomes": ["test:missing"],
                        "scale": 2,
                    },
                },
            }),
        ] {
            let result = for_stem(
                "test:pocket",
                &bad,
                &generators(),
                &sources(&["minecraft:plains"]),
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                0,
            );
            assert!(matches!(result, Some(Err(_))), "{bad}");
        }
    }

    #[test]
    fn non_noise_generation_samples_zero_climate_and_erosion() {
        let point = |biome, temperature| {
            json!({
                "biome": biome,
                "parameters": {"temperature": temperature, "humidity": 0.0,
                    "continentalness": 0.0, "erosion": 0.0, "depth": 0.0,
                    "weirdness": 0.0, "offset": 0.0},
            })
        };
        let mut stem = stem("test:slab");
        // No noise settings in either stem. At zero climate, only plains is an exact match.
        stem["generator"]["biome_source"] = json!({"type": "multi_noise", "biomes": [
            point("desert", 1.0), point("plains", 0.0),
        ]});
        let points = terrain(&stem, LevelShape::OVERWORLD, 3).unwrap();
        for (cx, cz) in [(0, 0), (78, -98)] {
            let chunk = chunk(&points, cx, cz);
            assert_eq!(chunk.biome(0, -64, 0), 0);
            assert_eq!(chunk.biome(15, 319, 15), 0);
        }
        stem["generator"]["biome_source"] = json!({"type": "the_end"});
        let end = terrain(&stem, LevelShape::OVERWORLD, 3).unwrap();
        assert_eq!(chunk(&end, 64, 0).biome(0, 0, 0), 8); // central End
        assert_eq!(chunk(&end, 65, 0).biome(0, 0, 0), 10); // midlands at erosion zero
    }
}
