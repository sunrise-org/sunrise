# Architecture: feature → files → checks

Use the table to find the code and verification for a feature. The
[agent guide](../AGENTS.md#which-side-code-goes-on) covers where to put gameplay
changes and how to lint both configurations.

The shell (`sunrise-client`) wires startup, the integrated server and the window.
`sunrise-gameplay` owns the authoritative simulation (`ServerGame`), and
`sunrise-client-gameplay` owns the client view (`ClientGame`). Each uses
`CommonGame` and the `SideGame` traits from `sunrise-gameplay-common`. The side
crates depend on common and lower crates. `sunrise-sim` provides networking,
entity and save code, and `sunrise-level` provides levels, chunks, lighting and
chunk streaming. The headless binary builds with `cargo build -p sunrise-game --no-default-features`.

Paths are relative to the `src/` of the named crate. **Server** = `sunrise-gameplay`,
**Client** = `sunrise-client-gameplay`, **Common** = `sunrise-gameplay-common`,
**Shell** = `sunrise-client` (also the source tree for `sunrise-client-lib`),
**Sim** = `sunrise-sim`, **Level** = `sunrise-level`, **Render** = `sunrise-client-render`.

A bare check name runs with `python3 tools/parity.py <name>`. For a row that names
`lan_e2e`, run `python3 tools/lan_e2e.py` and pass `--check <name>` for each selected check. To list the harnesses a change can
affect, write the changed paths one per line to a file and run
`python3 tools/parity.py --changed-files <file> --list`. Client probes are built
with `-p sunrise-client-lib` and the game binary with `-p sunrise-game`.

Item component values live in `crates/sunrise-world/src/component_value.rs`.
`crates/sunrise-protocol/src/item/persistent.rs` reconstructs stream values, and
`crates/sunrise-protocol/src/item/hash/ops.rs` hashes those values after saved-codec validation. Templates retain their own patches and
dense container holes. The last entry for a component wins, and saved projections
and nested template hashes omit transients. Values outside the saved codec's
contract stay intact when projection refuses them. Clicks check typed values
without projecting a new saved tree; a failed component hash refuses the click.
Equality of directly constructed `Value::Map`s allocates nothing but is quadratic
in the number of entries. Binary NBT read limits live in `crates/sunrise-core/src/nbt.rs`;
they do not apply to materialized component trees.
Check this seam with `cargo test -p sunrise-world --lib component_value::` and
`cargo test -p sunrise-protocol --lib item::`.

| Area | Files | Checks |
|---|---|---|
| Game loop, tick order, frame pacing | Server `game/world/levels.rs`, `game/world/level_tick.rs`; Client `game/game_client.rs`, `game/runtime/window_handler.rs`; Sim `clock.rs`; `crates/sunrise-client-input/src/window.rs` | game_loop, frame_limiter, interaction_input, tick |
| Mobs in the game loop, spawning | Server `game/mobs/creatures/mod_server.rs`, `game/mobs/natural_spawning.rs`, `game/mobs/spawner_server.rs`, `game/mobs/trial_spawner.rs`; Sim `mobs/spawning.rs`, `mobs/spawn_rules.rs` | natural_spawn, spawn_rules, finalize_spawn, spawner, trial_spawner, mob_ai |
| World save and load | Server `game/world/persist.rs`, `game/world/world_session.rs`; Common `game/world/saving.rs`; Sim `save.rs`, `entity_nbt.rs`; `crates/sunrise-world/src/region.rs` | save_roundtrip, world_reopen, world, datafix |
| World generation | `crates/sunrise-world/src/worldgen/`; Sim `vanillagen.rs` | chunk_generation, worldgen, server_world_gen, world_spawn, feature_graph, feature_placement, decoration |
| Multiplayer client, packets in | Client `game/network/remote/mod_client.rs`, `game/network/remote/packets_*_client.rs`, `game/network/remote/tick_client.rs`; Shell `net.rs` | dedicated_server, server_entities, `lan_e2e` |
| Agent control, sunrise-ctl | Sim `control.rs`; Shell `bin/sunrise-ctl/`; Client `game/runtime/control.rs`, `game/runtime/agent/mod.rs`, `game/runtime/events.rs`, `game/runtime/ui_control.rs` | `python3 tools/agent_control_e2e.py`; `cargo test -p sunrise-client-lib --bin sunrise-ctl` |
| CI and release | `.github/workflows/dyna-ci.yml`, `.github/workflows/daily.yml`, `.github/workflows/parity.yml`, `.github/workflows/release.yml`, `tools/ci/`, `tools/pr_ready.py` | `python3 -m unittest discover -s tests`; actionlint (`.github/actionlint.yaml`) |
| LAN host: connections, play state, tracking | Sim `lan/conn.rs`, `lan/hub.rs`, `lan/tracking.rs`; Server `game/network/lan/mod.rs`, `game/network/lan_entities.rs` | lan, `lan_e2e`, `cargo test -p sunrise-client-lib --test lan_bot` |
| Renderer core | Render `gfx.rs`, `gfx/renderer_construction.rs`, `gfx/` | `cargo test -p sunrise-client-render --lib -- gfx::`; translucency; `sunrise-client --benchmark standard` |
| Launcher | [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher) | `cargo test` in that repository |
| Mob types and species | Sim `mobs.rs`, `mobs/species.rs`, `mobs/data.rs`, `mobs/living.rs`, `mobs/drops.rs`, `mobs/gear.rs` | mob_attributes, entity_attribute, entity_death_loot, mob_ai |
| Guests' actions on a server | Server `game/network/lan_actions/turns_server.rs`, `game/network/lan_actions/tick_server.rs` and the action modules in `game/network/lan_actions/` | server_player_combat, server_player_damage, server_player_targets, `lan_e2e` |
| Modpack porting, mod content | `crates/sunrise-port-pack/`; Sim `mod_content.rs`; Shell `mod_content_client.rs` | `cargo test -p sunrise-port-pack`; `python3 tools/mod_port_harness.py` |
| Mob AI | Sim `mobs/ai.rs`, `mobs/goals.rs`, `mobs/brain.rs`, `mobs/nav.rs`, `mobs/path.rs`, `mobs/control.rs` | mob_ai |
| Items and item use | `crates/sunrise-items/src/`; Common `game/items/item_actions.rs`; Server `game/items/use_hand.rs`, `game/items/using_item.rs`, `game/items/consume.rs`, `game/items/potions.rs`; Client `game/items/hand.rs` | use_item_on, item_use, item_use_lifecycle, consumable, consume_effect, status_effect |
| Options and settings screens | Render `options_ui.rs`, `settings.rs`, `graphics_ui.rs`; Client `game/interface/options.rs`; Level `options_file.rs` | options, toggle_keys |
| HUD, chat, tab list | Render `hud.rs`, `hud_layout.rs`, `hud_overlays.rs`, `chat.rs`, `tab_list.rs`, `hud_kit/`; Client `game/interface/chat.rs`, `game/interface/locator_bar.rs`, `game/interface/hud_room.rs` | hud, tab_list, locator_bar, chat_screen, secure_chat |
| Command execution | Server `game/administration/commands/`, `game/administration/functions.rs`; Sim `scoreboard_cmd.rs`, `data_cmd.rs`; `crates/sunrise-game-data/src/scoreboard.rs` | command_exec (corpus `tools/command_exec_corpus.py`), scoreboard, server_ops_feedback, lan_e2e commands |
| Chunk streaming, light, tickets | Level `world.rs`, `chunk.rs`, `chunk_map.rs`, `light.rs`, `chunk_streamer.rs`; Render `streaming.rs`; Sim `ticket_levels.rs`, `tick_chunk_membership.rs`; Server `game/world/world_loading.rs` | light, chunk_status, random_tick_sections; `cargo test -p sunrise-client-lib --test streaming_meshes` |
| Mob models and animation | Shell `mobs/render.rs`; Sim `mobs/anim.rs`, `mobs/body_anim.rs`, `mobs/keyframes.rs`; Render `entity.wgsl` | entity_model |
| Block behaviour, server side | `crates/sunrise-world/src/level/`; Server `game/blocks/block_entity_ticks.rs`, `game/blocks/piston_push_server.rs`; Common `game/blocks/moving_pistons.rs`; Level `block_entity_ticks.rs` | block_update, redstone, random_tick, simple_block, piston_movement, block_entity_tickers |
| Player movement and collision | Sim `motion.rs`, `motion/`; Level `collisions.rs`; `crates/sunrise-world/src/entity_move.rs`, `crates/sunrise-world/src/player_travel.rs`, `crates/sunrise-world/src/collision.rs` | collision, entity_move, player_travel, living_motion, climbing, swimming; `python3 tools/test_native_scene_motion.py` |
| Resource packs and models | Render `packs.rs`, `packs/`, `pack_screen.rs`, `atlas_sources.rs`, `model_library.rs`; Client `game/world/packs/`; Shell `mobs/cem.rs`, `mobs/cem_anim.rs`; `crates/sunrise-assets/`, `crates/sunrise-optifine/` | resource_pack, pack_selection, pack_screen, model_resolution, item_model, mipmap |
| Container menus and inventory | Sim `menu.rs`; Level `containers.rs`; Shell `container_screen.rs`, `creative.rs`; Client `game/interface/menu_clicks_client.rs`, `game/interface/inventory_slots_client.rs` | menu_click, container_screen, creative, lan_e2e containers clicks |
| Workstations | Sim `stations.rs`; Client `game/blocks/stations_client.rs`; Level `brewing.rs`; `crates/sunrise-items/src/enchant.rs` | enchanting, screen_layout, lan_e2e brewing loom stonecutter grindstone anvil smithing |
| Crafting and the recipe book | Sim `crafting.rs`, `recipe_book.rs`; Shell `recipe_book_screen/`; Client `game/items/recipe_book.rs`; Server `game/blocks/machines/` | recipe, recipe_book, recipe_book_view, place_recipe, ghost_recipe, crafter |
| Vehicles and mounts | Server `game/entities/boats/`, `game/entities/minecarts_server.rs`, `game/mobs/mounts.rs`; Client `game/interface/mount_menu.rs`, `game/network/remote_vehicles.rs` | server_entities, lan_e2e boats minecarts riding mounts |
| Villagers, trading, raids | Sim `mobs/villager.rs`, `merchant.rs`, `raid.rs`, `gossip.rs`; Server `game/items/trading.rs`, `game/mobs/villagers.rs`, `game/mobs/raids/` | trading, raid, mob_ai |
| Title, worlds, startup | Shell `title.rs`, `cut_layout/game/interface/startup_menu.rs`; Client `game/interface/startup_menu.rs`, `game/runtime/loading_window.rs`, `game/runtime/resource_load_client.rs` | world_options; `cargo test -p sunrise-client-lib --lib title` |
| Dedicated server, sunrise-server | Server `game/interface/dedicated/`; Shell `entry.rs`; `crates/sunrise-server/`, `crates/sunrise-server-files/` | dedicated_server, server_files, user_cache, online_mode, server_ops_feedback |
| Particles and client effects | Render `particles.rs`; Client `game/interface/render_tick.rs`, `game/interface/level_particles_client.rs`; Shell `animate_tick.rs` | particle_providers, block_particles, entity_particles, emitter_particles, marker_particles |
| Bosses and the End | Server `game/mobs/dragon.rs`, `game/mobs/wither.rs`, `game/world/end.rs` | server_dragon, forced_end, end_podium, end_gateway |
| Protocol and packets | `crates/sunrise-protocol/src/packets/`, `crates/sunrise-protocol/src/entity_data.rs`, `crates/sunrise-protocol/src/connection.rs` | dedicated_server, online_mode, secure_chat; `cargo test -p sunrise-protocol` |
| Multiplayer and LAN screens, friends | Shell `multiplayer.rs`; Client `game/network/lan_screen.rs`, `game/network/friends.rs`, `game/network/friends/`; `crates/sunrise-friends/`, `crates/sunrise-relay/` | lan, world_options; `python3 tools/test_friends.py` |
| Dispensers | Server `game/blocks/dispenser.rs`, `game/interface/dispense_fixture.rs` | dispenser |
| Audio and music | Render `audio.rs`, `sound_engine.rs`; Level `sound_events.rs`; Shell `music.rs`, `ambience.rs`; Client `game/interface/music.rs`, `game/interface/subtitle_hud.rs` | sound_engine, sound_event, music, situational_music, ambient_sound, subtitle |
| Integrated server and levels | Server `game/world/integrated.rs`, `game/world/levels.rs`, `game/world/server_travel.rs`; Client `game/world/integrated_client.rs`; Shell `cut_layout/game/integrated_pair.rs` | server_dimension_players, server_dimension_item, lan_e2e portals levels distance lava |
| Command parsing | `crates/sunrise-command/src/` | command_parse (corpus `tools/command_parse_corpus.py`) |
| Benchmarks | Shell `benchmark.rs`; `tools/perf_bench.py`, `crates/sunrise-system/`, `crates/sunrise-bench/` | `python3 -m unittest tests.test_perf_bench`; [benchmark.md](benchmark.md) |
| Explosions and TNT | Server `game/entities/explosion.rs`, `game/entities/tnt.rs`, `game/blocks/nether_portal.rs`, `game/entities/vehicle_teleport.rs` | sulfur_portal_blast (calculator oracle; actual portal, `/tp` and mod cross-level arrivals, immediate item merges, transient TNT portal protection and disk reset); lan_e2e blast |

NBT compounds (`crates/sunrise-core/src/nbt.rs` and `crates/sunrise-core/src/java_hash_map.rs`) retain Java map
traversal, including collision and mutation history. Rust `Clone` preserves that history;
use `nbt::copy_tag` for Java `CompoundTag.copy` semantics. Reads must not reconstruct maps.
Block-entity writers construct superclass/subclass fields before adding metadata, rather
than repairing a completed compound and changing its traversal.

Portal arrival regressions run with `python3 tools/test_sulfur_portal_blast_parity.py`
and prepared 26.3 resources in `SUNRISE_TEST_RESOURCES`. The harness supplies private
game/XDG/launcher homes and serializes the gameplay tests. To use the shell's
lib test binary in an equivalently isolated directory, select
`portal_arrival_effects_ --include-ignored --test-threads 1`.
`ItemEntity.teleport` merges once immediately after recreation on both arrival
paths, using the existing item-cell index and preserving the surviving entity's
identity and age. `PrimedTnt.teleport` sets a live-only flag: its explosion ignores
Nether portal resistance and does not destroy those blocks. Fresh TNT and TNT
recreated from a chunk save use the ordinary calculator; `/data` updates of the
same live TNT do not clear the flag, as in 26.3.
Construction/join callback regressions also cover nested spawns and logically
removed incoming or neighbouring items, before the native removal sweep.

Controller glyphs (`crates/sunrise-client-render/src/pad_glyph.rs`, drawn by the gameplay guide, notices and declared
buttons) are part of the same kit: [ui-kit.md](ui-kit.md) says how to ask for one and how to test it.
