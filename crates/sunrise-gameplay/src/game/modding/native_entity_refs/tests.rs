use crate::game::server_side::resource_load_server::load_resources_for_role;
use mod_api::{EntityRef, LevelReadError, LevelRef};
use std::{ops::ControlFlow, path::Path, sync::Arc, time::Instant};
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_game_data::lang;
pub use sunrise_gameplay_common::game::test_fixtures::{
    FRESH_CASE_LIMIT, fixture_block_names, fixture_resident_column, fixture_root, output_within,
    ran_one_test, record_key, resolve_parts, run_fresh_cases, run_fresh_cases_at_once,
    run_fresh_cases_with_env, run_with_game_stack,
};
use sunrise_gameplay_common::game::{
    GameGameExt1 as _, PackResources, Resources,
    execution_role::ExecutionRole,
    mods::LoadRole,
    native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
    player_identity::PlayerIdentityGameExt1 as _,
    resource_load::{Progress, with_skip_facts_memo},
    test_fixtures::{
        acquire_in_process_load_permit, fixture_facts_mut, offline_profiles, tempfile,
    },
    test_resource_cache::{
        CacheKey, get_or_build, hydrate_common_hit, is_plain_directory_resource_root,
    },
};
use sunrise_level::datapack::DataPack;

pub fn collect_all_entities(
    host: &dyn mod_api::Host,
    level: LevelRef,
) -> Result<Vec<EntityRef>, LevelReadError> {
    let mut entities = Vec::new();
    let _ = level.visit_all_entities(host, &mut |entity| {
        entities.push(entity);
        ControlFlow::Continue(())
    })?;
    Ok(entities)
}

pub fn collect_entities_in(
    host: &dyn mod_api::Host,
    level: LevelRef,
    min: [f64; 3],
    max: [f64; 3],
) -> Result<Vec<EntityRef>, LevelReadError> {
    let mut entities = Vec::new();
    let _ = level.visit_entities_in(host, min, max, &mut |entity| {
        entities.push(entity);
        ControlFlow::Continue(())
    })?;
    Ok(entities)
}

pub fn collect_game_all_entities(
    game: &crate::game::ServerGame<'_>,
    level: LevelRef,
) -> Result<Vec<EntityRef>, LevelReadError> {
    let mut entities = Vec::new();
    let _ = game.level_visit_all_entities(level, &mut |entity| {
        entities.push(entity);
        ControlFlow::Continue(())
    })?;
    Ok(entities)
}

pub fn collect_game_entities_in(
    game: &crate::game::ServerGame<'_>,
    level: LevelRef,
    min: [f64; 3],
    max: [f64; 3],
) -> Result<Vec<EntityRef>, LevelReadError> {
    let mut entities = Vec::new();
    let _ = game.level_visit_entities_in(level, min, max, &mut |entity| {
        entities.push(entity);
        ControlFlow::Continue(())
    })?;
    Ok(entities)
}

pub fn prepare_entity_ticking_neighborhood(
    game: &mut crate::game::ServerGame<'_>,
    center: sunrise_core::position::ChunkPos,
) {
    for z in center.z - 2..=center.z + 2 {
        for x in center.x - 2..=center.x + 2 {
            if !game.world.chunks.contains_key(&(x, z)) {
                let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
                // A ticking column is lit (`ChunkMap.prepareTickingChunk`); `forceload` waits for it.
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
                game.block_ticks
                    .add_empty_chunk(sunrise_core::position::ChunkPos::new(x, z))
                    .unwrap();
            }
        }
    }
}

/// Bind the fixture body's canonical identity before configuring its acting context.
pub fn bind_local_player(game: &mut crate::game::ServerGame<'_>, id: u32, uuid: u128) {
    game.server.next_entity = game.server.next_entity.max(id);
    game.player.as_mut().expect("fixture player body").entity_id =
        i32::try_from(id).expect("fixture entity ID fits the protocol");
    game.level.player_uuid = Some(sunrise_sim::lan::playerdata::uuid_ints(uuid));
    game.bind_local_player_residency();
    game.server.lan_acting_entity = Some(id);
    game.server.lan_acting_uuid = Some(uuid);
}

/// Install the server record backing a fixture's already configured acting body.
pub fn bind_acting_guest(game: &mut crate::game::ServerGame<'_>) {
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    let conn = game.server.lan_acting.unwrap();
    let id = game.server.lan_acting_entity.unwrap();
    let uuid = game.server.lan_acting_uuid.unwrap();
    let name = game.server.lan_acting_name.clone().unwrap();
    let at = game.player.as_ref().unwrap().position;
    game.server.next_entity = game.server.next_entity.max(id);
    game.lan_host(
        crate::game::server_side::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::lan(0, String::new(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: game.dim.id().to_owned(),
            allow_flight: true,
            seed: 0,
            max_players: 2,
            integrated: true,
            levels: game.server.level_shapes.clone(),
            level_seeds: game.server.levels.0.runtime.seeds(),
        },
    )
    .unwrap();
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid,
            name,
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: String::new(),
            sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
        },
        id as i32,
        [at.x, at.y, at.z],
        10,
    );
    guest.placed = true;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    game.reset_entity_residency();
}

pub fn with_game(test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    with_game_at(fixture_root(), test);
}

/// Loads the origin column and its ticking neighbours so entity handles at the origin are accessible.
pub fn with_resident_game(test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, sunrise_core::position::ChunkPos::new(0, 0));
        test(game);
    });
}

pub fn with_game_at(root: std::path::PathBuf, test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    with_game_role(root, ExecutionRole::IntegratedServer, test);
}

pub fn with_uncached_game(test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    with_fixture_game(
        fixture_root(),
        ExecutionRole::IntegratedServer,
        false,
        None,
        test,
    );
}

pub fn with_game_role(
    root: std::path::PathBuf,
    role: ExecutionRole,
    test: impl FnOnce(&mut crate::game::ServerGame<'_>),
) {
    with_game_role_and_facts(root, role, None, test);
}

/// Configure the fixture's owned gameplay facts before constructing the game.
pub fn with_game_role_and_facts(
    root: std::path::PathBuf,
    role: ExecutionRole,
    configure: Option<
        fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
    >,
    test: impl FnOnce(&mut crate::game::ServerGame<'_>),
) {
    with_fixture_game(root, role, configure.is_none(), configure, test);
}

fn with_fixture_game(
    root: std::path::PathBuf,
    role: ExecutionRole,
    cached: bool,
    configure: Option<
        fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
    >,
    test: impl FnOnce(&mut crate::game::ServerGame<'_>),
) {
    use mod_api::Authority;
    use sunrise_gameplay_common::game::Options;
    use sunrise_level::sim_settings::SimSettings;

    #[cfg(test)]
    mod_platform::__private::test_platform::install();
    sunrise_gameplay_common::game::test_fixtures::prepare_game_boot(
        &root,
        mod_api::testing::engine_platform::register_game_keys,
    );
    let saves = tempfile::tempdir().unwrap();
    let settings = SimSettings::default();
    let load = if cached {
        load_fixture_server_resources
    } else {
        load_fixture_server_resources_uncached
    };
    let (mut resources, pack) = load(&root, role);
    if let Some(configure) = configure {
        configure(pack.assets.registry, fixture_facts_mut(&mut resources));
    }
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = role;
    options.game_dir = Some(saves.path().to_path_buf());
    let mut game =
        crate::game::ServerGame::new(&resources, pack, options, settings, None, offline_profiles)
            .unwrap();
    game.reset_entity_residency();
    game.native_lifecycle
        .begin_open(Some(Authority::IntegratedServer));

    // A game that opened its world resolves its parts; the harness stops
    // short of the open, so do it here for the by-state part table.
    resolve_parts(&game).unwrap();
    game.install_root_block_transformers();
    // The title's save folder is under the working directory: give each test its own, so a
    // test that saves leaves the checkout clean and parallel tests don't share files.
    game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
        saves.path().to_path_buf(),
        sunrise_sim::save::LevelShape::OVERWORLD,
        Some(Default::default()),
    ));
    test(&mut game);
}

pub fn load_fixture_server_resources(
    root: &Path,
    role: ExecutionRole,
) -> (Arc<Resources>, PackResources) {
    load_server_resources_cached(
        ResourceRoot::directory(root.to_path_buf()),
        Vec::new(),
        root,
        role.into(),
        1,
        Instant::now(),
        false,
        &Progress::new(),
        true,
    )
    .unwrap()
}

pub fn load_fixture_server_resources_uncached(
    root: &Path,
    role: ExecutionRole,
) -> (Arc<Resources>, PackResources) {
    load_server_resources_uncached(
        ResourceRoot::directory(root.to_path_buf()),
        Vec::new(),
        root,
        role.into(),
        1,
        Instant::now(),
        false,
        &Progress::new(),
        true,
    )
    .unwrap()
}

pub fn load_server_resources_cached(
    root: ResourceRoot,
    additional_data: Vec<Arc<DataPack>>,
    vanilla_root: &Path,
    role: LoadRole,
    threads: usize,
    started: Instant,
    skip_lang: bool,
    progress: &Progress,
    validate_entity_part_state: bool,
) -> sunrise_game_data::Result<(Arc<Resources>, PackResources)> {
    #[cfg(test)]
    mod_platform::__private::test_platform::install();

    // Server-role client loads validate entity part state too. Don't let loads that
    // opt out populate a slot shared with a loader that requires validation.
    if validate_entity_part_state
        && !role.execution.is_client()
        && additional_data.is_empty()
        && let Some(raw_root) = is_plain_directory_resource_root(&root)
        && let Some(key) = CacheKey::for_vanilla_root(&raw_root, vanilla_root, role, threads, None)
    {
        let cached = get_or_build(key.clone(), move |key| {
            let (resources, pack) = with_skip_facts_memo(|| {
                load_resources_for_role(
                    ResourceRoot::directory(key.root.clone()),
                    Vec::new(),
                    &key.vanilla_root,
                    role,
                    key.worker_threads,
                    Instant::now(),
                    &Default::default(),
                    &Progress::new(),
                    validate_entity_part_state,
                )
            })
            .map_err(|e| e.to_string())?;
            Ok((resources, pack, None))
        })?;
        if let Some((resources, pack, _)) =
            hydrate_common_hit(&cached, &key, lang::DEFAULT, true, skip_lang)?
        {
            progress.set(9);
            return Ok((resources, pack));
        }
    }
    load_server_resources_uncached(
        root,
        additional_data,
        vanilla_root,
        role,
        threads,
        started,
        skip_lang,
        progress,
        validate_entity_part_state,
    )
}

pub fn load_server_resources_uncached(
    root: ResourceRoot,
    additional_data: Vec<Arc<DataPack>>,
    vanilla_root: &Path,
    role: LoadRole,
    threads: usize,
    started: Instant,
    skip_lang: bool,
    progress: &Progress,
    validate_entity_part_state: bool,
) -> sunrise_game_data::Result<(Arc<Resources>, PackResources)> {
    #[cfg(test)]
    mod_platform::__private::test_platform::install();

    let _permit = acquire_in_process_load_permit();
    let _language =
        skip_lang.then(|| lang::isolate_publication(lang::Language::of(lang::DEFAULT, [])));
    with_skip_facts_memo(|| {
        load_resources_for_role(
            root,
            additional_data,
            vanilla_root,
            role,
            threads,
            started,
            &Default::default(),
            progress,
            validate_entity_part_state,
        )
    })
}

/// A world with block `support` at (5, 64, 5) and a powered lever on its east face.
pub fn lever_on(game: &mut crate::game::ServerGame<'_>, support: &str) {
    use std::sync::Arc;
    use sunrise_level::chunk::Chunk;

    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let registry = game.pack.assets.registry;
    let support = registry.default_state(support).unwrap();
    let lever = registry
        .resolve(&sunrise_world::chunk::BlockState {
            name: "minecraft:lever".into(),
            properties: [("face", "wall"), ("facing", "east"), ("powered", "true")]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        })
        .unwrap();
    game.world.set(5, 64, 5, support).unwrap();
    game.world.set(6, 64, 5, lever).unwrap();
}

/// A plain piston pushing east at (5, `y`, 5) with `block` at (6, `y`, 5) and `above` on top
/// of it, powered from (4, `y`, 5): whether the block above moved with the one below. Moved,
/// its old cell is air once the push started. Each scenario has its own `y`, so none meets
/// the moving blocks of another.
pub fn piston_pulls_above(
    game: &mut crate::game::ServerGame<'_>,
    y: i32,
    block: &str,
    above: &str,
) -> bool {
    use sunrise_core::position::BlockPos;
    let registry = game.pack.assets.registry;
    let state = |name: &str| registry.default_state(name).unwrap();
    let air = state("minecraft:air");
    game.with_level(|b, level, ticks| {
        let piston = |extended: &str| {
            registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:piston".into(),
                    properties: [
                        ("facing".into(), "east".into()),
                        ("extended".into(), extended.into()),
                    ]
                    .into_iter()
                    .collect(),
                })
                .unwrap()
        };
        let mut place = |pos: [i32; 3], s| {
            let flags = sunrise_world::level::flags::UPDATE_ALL;
            b.set_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                s,
                flags,
            );
        };
        place([5, y, 5], piston("false"));
        place([6, y, 5], state(block));
        place([6, y + 1, 5], state(above));
        place([4, y, 5], state("minecraft:redstone_block"));
        b.run_block_events(level, ticks);
        assert_eq!(
            sunrise_world::level::LevelAccess::block_state(level, BlockPos::new(5, y, 5)),
            piston("true"),
            "{block}: the piston extended"
        );
        sunrise_world::level::LevelAccess::block_state(level, BlockPos::new(6, y + 1, 5)) == air
    })
}
