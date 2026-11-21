//! Column reads for server mods, as helpers on the level being ticked: many columns' heights, and the blocks round
//! them, in one call.
use super::Game;
use super::block_edits::MAX_COLUMNS;

impl Game<'_> {
    /// For a client mod: each column's height by `heightmap` in the world this window shows,
    /// none for a column whose chunk this client does not hold. `Err` says why nothing was read:
    /// no client world, or more than 65,536 columns (`MAX_COLUMNS`).
    pub(super) fn client_surface_heights(
        &self,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
    ) -> Result<Vec<Option<i32>>, String> {
        if !self.remote.as_ref().is_some_and(|r| r.logged_in) {
            return Err("no client world is open".to_string());
        }
        if columns.len() > MAX_COLUMNS {
            return Err(format!(
                "{} columns, more than {MAX_COLUMNS} a call",
                columns.len()
            ));
        }
        Ok(self.loaded_column_heights(columns, heightmap))
    }
    /// For a server mod: each column's height by `heightmap` in the authoritative level `dimension` (the one being
    /// ticked), none for a column not loaded. `Err` says why nothing was read: another dimension or a client of a
    /// server, or more than 65,536 columns (`MAX_COLUMNS`).
    pub(super) fn surface_heights(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
    ) -> Result<Vec<Option<i32>>, String> {
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        if columns.len() > MAX_COLUMNS {
            return Err(format!(
                "{} columns, more than {MAX_COLUMNS} a call",
                columns.len()
            ));
        }
        let started = std::time::Instant::now();
        let out = self.loaded_column_heights(columns, heightmap);
        self.native_read_logged(started, "height query", columns.len());
        Ok(out)
    }
    /// For a server mod: `below` blocks under and `above` blocks from each column's height by `heightmap`, as a
    /// palette and indexes, with the biome's grass, foliage and water tints (air outside the column and in columns
    /// not loaded). The tints are the biome's at the column's top, not blended over the biome blend radius as the
    /// game draws them: near a biome border they can differ from what players see. `Err` says why nothing was
    /// read: another dimension or a client of a server, `below + above` of 0 or more than the level's height, or
    /// a read of more than 1,048,576 blocks (`MAX_READ`; it runs on the server tick, so read big areas in pieces).
    pub(super) fn read_columns(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
        range: (u32, u32),
    ) -> Result<mod_api::ColumnBlocks, String> {
        let (below, above) = range;
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        let started = std::time::Instant::now();
        let out = self.read_loaded_block_columns(columns, heightmap, below, above)?;
        self.native_read_logged(started, "column read", columns.len());
        Ok(out)
    }
    pub(super) fn terrain_surface(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<mod_api::TerrainColumn>>, String> {
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        if columns.len() > mod_api::MAX_TERRAIN_COLUMNS {
            return Err(format!(
                "{} columns, more than {} a call",
                columns.len(),
                mod_api::MAX_TERRAIN_COLUMNS
            ));
        }
        let started = std::time::Instant::now();
        let out = match self.end.locator() {
            None => vec![None; columns.len()],
            Some(generator) => {
                let index = super::native_worldgen::prepare(self.pack.assets.registry)?;
                generator
                    .terrain_surfaces(columns)
                    .into_iter()
                    .map(|surface| {
                        surface
                            .map(|surface| {
                                let state = index
                                    .to_api(surface.state)
                                    .map_err(|error| format!("terrain surface block: {error:?}"))?;
                                Ok(mod_api::__private::built::TerrainColumn {
                                    height: surface.height,
                                    block: state.block(),
                                    water: surface.water.map(|(height, _)| height),
                                    biome: world_biomes!(self).name(surface.biome).to_owned(),
                                }
                                .into())
                            })
                            .transpose()
                    })
                    .collect::<Result<Vec<_>, String>>()?
            }
        };
        self.native_read_logged(started, "terrain surface query", columns.len());
        Ok(out)
    }
    /// A read's time logged with `SUNRISE_SPIKES`, as an edit's is.
    fn native_read_logged(&self, started: std::time::Instant, what: &str, columns: usize) {
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self.options.spikes.is_some_and(|threshold| ms > threshold) {
            log::info!("Sunrise: slow mod {what} {ms:.1} ms ({columns} columns)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use std::sync::Arc;
    use sunrise_client::vanillagen::{TerrainResources, VanillaTerrain};
    use sunrise_world::worldgen::generator::FlatSettings;

    #[test]
    #[ignore = "daily"]
    fn terrain_surface_reads_flat_layers_without_chunks() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let directory = game.options.resources.join("worldgen");
            let terrain = VanillaTerrain::for_flat(
                TerrainResources {
                    data: Some(&game.pack.data_changes),
                    dir: &directory,
                    registry: game.pack.assets.registry,
                    biomes: world_biomes!(game),
                    item_data: &game.res.core.item_data,
                },
                &Default::default(),
                12345,
                &FlatSettings {
                    layers: vec![
                        ("minecraft:bedrock".into(), 1),
                        ("minecraft:dirt".into(), 2),
                        ("minecraft:grass_block".into(), 1),
                    ],
                    biome: "minecraft:plains".into(),
                    structure_overrides: Some(Vec::new()),
                    features: false,
                    lakes: false,
                },
                false,
            )
            .unwrap();
            game.end.set_locator(Some(Arc::new(terrain)));
            let dimension = game.dim.id();
            let before = (game.world.chunks.len(), game.streaming.pending());
            let columns = [(100000, -100000), (0, 0), (-1, 31), (100000, -100000)];
            let answer = game.terrain_surface(dimension, &columns).unwrap();
            assert_eq!(answer.len(), columns.len());
            // Four layers from the overworld minimum -64, independent of the sampler.
            for column in &answer {
                let column = column.as_ref().unwrap();
                assert_eq!(column.height, -60);
                assert_eq!(
                    column.block,
                    mod_api::__private::block_key("minecraft:grass_block").unwrap()
                );
                assert_eq!(column.water, None);
                assert_eq!(column.biome, "minecraft:plains");
            }
            assert_eq!(game.terrain_surface(dimension, &columns).unwrap(), answer);
            assert_eq!(before, (game.world.chunks.len(), game.streaming.pending()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn terrain_surface_refuses_before_sampling() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let dimension = game.dim.id();
            game.end.set_locator(None);
            assert!(
                game.terrain_surface("test:unknown", &[])
                    .unwrap_err()
                    .contains("level being ticked")
            );
            let columns = vec![(0, 0); mod_api::MAX_TERRAIN_COLUMNS];
            assert_eq!(
                game.terrain_surface(dimension, &columns).unwrap(),
                vec![None; columns.len()]
            );
            let excessive = vec![(0, 0); mod_api::MAX_TERRAIN_COLUMNS + 1];
            assert!(
                game.terrain_surface(dimension, &excessive)
                    .unwrap_err()
                    .contains("4096")
            );
            assert!(game.terrain_surface(dimension, &[]).unwrap().is_empty());
            game.options.dedicated = None;
            assert!(game.terrain_surface(dimension, &[]).is_err());
        });
    }
}
