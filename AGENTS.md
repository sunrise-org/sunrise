# Sunrise: a map for agents

Minecraft: Java Edition 26.3 rewritten in Rust (wgpu). Vanilla is the specification wherever it has the
feature: Sunrise behaves and looks bit-exactly like it on the same machine, while its code is free to differ
([CONTRIBUTING.md](CONTRIBUTING.md#vanilla-first)). Read [CONTRIBUTING.md](CONTRIBUTING.md) once;
this page tells you where things are.
Feature → files → harness for the most-changed areas: [docs/architecture.md](docs/architecture.md).

## Rules that bite

- **Never commit Mojang's files**: game jars, assets, decompiled source, prepared resources (`.local/`).
  `tools/check_public_tree.py` rejects them; `git config core.hooksPath .githooks` runs it on commit.
- **Keep your runs away from real player data.** Any game or launcher you start gets fresh `mktemp -d`
  folders in `SUNRISE_HOME` (logs, crash reports, telemetry consent, recordings; not saves), `SUNRISE_LAUNCHER_HOME`, `XDG_DATA_HOME` and
  `XDG_CONFIG_HOME`, plus `SUNRISE_LAUNCHER_NO_SELF_UPDATE=1` and `SUNRISE_NO_AUDIO=1`. Saves, `options.txt` and
  caches go in the game folder, by default the working directory: start the game from a scratch folder with
  `--game-dir` pointing at it, or they land in the repo root, hidden by `.gitignore`. For the
  launcher, `SUNRISE_LAUNCHER_TRASH`, `SUNRISE_LAUNCHER_CLIPBOARD` and `SUNRISE_LAUNCHER_DESKTOP` pointing into the scratch home. Never write to
  `~/.local/share/{applications,sunrise,sunrise-launcher}` or `~/.local/bin`.
- **Mark what an agent posts.** A PR body starts with `> 🤖 Opened by <login>'s AI agent` and a blank line, where
  `<login>` is the GitHub account the PR is opened from (`gh api user --jq .login`). Issues say "Filed by", reviews
  "Review by". Every comment starts with `🤖 Agent:`. That line is the only mark: commits carry no
  `Co-Authored-By:` and PR bodies no "Generated with" line.
- Everything players run is Rust. Python/Java only for dev tools and harness drivers in `tools/`.
- Sunrise-only features follow [docs/design-system.md](docs/design-system.md) and are off by default where
  they would change the vanilla look.
- One concern per PR, at whatever size it needs; no line limit. Labels: `feature` (players get something new), `skip-release` (they see no
  change), none for a fix. Agents never add `release-now`: the maintainer decides it. Merges reach players through the
  nightly, published by the daily run when every leg is green ([docs/release-flow.md](docs/release-flow.md#daily-run)).
- **CI tiers** ([CONTRIBUTING.md](CONTRIBUTING.md#ci-tiers)): every PR push runs `format` and `verify-static`; rollups,
  syncs and main pushes add `lint`, `verify` (unit tests only) and `launcher`; the ignored-test shards,
  `fn-replace-check`, the SDK build and its linkage probe, the machine-sample build and the modding book's
  quickstart run once a day (`daily.yml`). A new test that needs prepared resources, boots the game, starts a
  child process, uses the network or takes 2 s or more is `#[ignore]`d with a row in `tools/ci/ignored-tests/`;
  say in the PR when you could not run it. A daily red becomes an issue listing the suspect PRs, fixed by an
  ordinary PR. `tools/ci/move_to_daily.py` moves a `verify` test that needs resources, a child process or the
  network. Builds: Dev (a commit of main, gate green; published as
  `dev-<date>-<short>`, which no launcher offers), nightly (published by a fully green daily run), release (cut from a nightly).
- **No hardcoded user-facing strings.** Keep UI translation-ready from the start. In the launcher ([sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher)), wrap visible text with `tr("...")` or `trf("... {arg} ...", &[("arg", &val)])` from `crate::i18n` instead of raw string literals or `format!`. In the game, use translation keys (`Text::tr`, `lang::get`), never hardcoded English strings.
- **Claim before you start**: comment `/claim` on the issue (no issue yet: open one, then claim it). An assigned
  issue is taken, and so is one labelled `in progress` (claimed on Discord): pick another. `/unclaim` when you stop;
  a claim with no open PR linking it is released after 5 days. The issues free to claim, without reading threads:
  `gh issue list --search 'is:open no:assignee -label:"in progress" -label:tracker -label:proposal -label:"not claimable" -linked:pr'`
- **Machine translations stay unreviewed.** `tools/lang_review/` records only text a person who speaks the language
  wrote or checked; never `lang_coverage.py --mark-reviewed` machine text ([docs/translations.md](docs/translations.md)).
- **Controllers reach what registers.** Draw controls with a registering helper (`ui::button`, `settings_shell::foot_button`); `graphics_style::button` and `screen_draw::button` only paint, and registering one part of a screen ends its crosshair, so register all of it.
- **Render passes keep HDR's light.** A pass that reads, filters or redraws the world's picture (upscaling,
  anti-aliasing, sharpening, frame generation, post effects, captures) follows [docs/hdr-passes.md](docs/hdr-passes.md).
- **Docs say where and how, not how many.** Point to where a thing lives and the command that lists or measures it.
  Don't write counts, growing lists, timings or current status into a doc: they go stale, and the next agent
  spends a turn fixing them.
- **Contract details go in rustdoc, not concept pages.** A PR that adds or changes a hook, method or edge case updates its `///` doc comment or `#[notes]`, not a concept page in `docs/modding/src/concepts/`; runnable Rust snippets belong in `docs/modding/examples/` via `{{#include}}`.

## Which side code goes on

- **Server** (`sunrise-gameplay`) decides outcomes and owns world writes, ticking, commands, saving, entity tracking, and the LAN and dedicated host.
- **Client** (`sunrise-client-gameplay`) renders, handles screens and input, plays sounds, applies packets, and predicts only what vanilla's client predicts. The server's answer wins. Vanilla uses `ClientLevel`/`MultiPlayerGameMode` and `ServerLevel`/`ServerPlayerGameMode`.
- **Common** (`sunrise-gameplay-common` or a lower crate) holds shared types, pure reads, and block and item logic both sides run.
- **Shell** (`sunrise-client`) owns startup and wiring for both sides and tests that need both. Put gameplay in the crate that owns it.
- **Dependencies point down.** The side crates cannot reach each other, even indirectly, and lower crates cannot reach either side or the shell. The `--no-default-features` server binary omits the client crates. Check `sides` in `tools/layers.json`, `tools/check_layers.py`, and the manifest checks in `tools/ci/check_gameplay_layers.py`.
- When the client needs a server function, extract the shared part into common or a lower crate, or have the server send the result through a packet, level event, or the tracker. Keep client state on the client. Server logic uses its own world, player and connection state and receives player input in packets.
- Use `ServerHost` for server writes, `ClientHost` for client writes, and `Host` for shared reads ([Mod API](#mod-api-the-clean-break)).
- Put server, client and shared tests in their owning crates. Use the shell's `crates/sunrise-client/src/cut_layout/` for tests that need a joined pair. Server-configuration tests must compile with the client feature off.
- `cfg(test)` applies only to the crate under test. Use `cfg(any(test, feature = "test-support"))` for helpers linked by another crate's tests and enable `test-support` in the consuming crate's dev-dependency.
- Carry a platform gate (`#[cfg(target_os = …)]`) with a module's declaration and re-exports.
- Set shared startup inputs in both the window launcher and the dedicated start, including the vanilla resource root used to load the language file.
- Reach each module from the crate root with `mod` and declare spliced fragments with `include!()` (`tools/ci/check_unreached_sources.py`).

Before opening a PR that touches gameplay crates, run both lint commands from `.github/workflows/dyna-ci.yml`:

```sh
python3 tools/ci/check_warnings.py cargo clippy --keep-going --workspace --all-targets
python3 tools/ci/check_warnings.py cargo clippy --keep-going --target-dir target/server-lint \
  -p sunrise-client -p sunrise-game -p sunrise-mod-host --all-targets --no-default-features
```

## Mod API: the clean break

Sunrise (sunrise-org/sunrise) is where the native mod API is built: hooks, the `Host`, handles, Parts,
attachments and mod loading. The mod API is taking **one clean
break**, and every change to it follows these rules. Reviews block a PR that breaks any of them.

- **No compatibility paths.** No aliases, shims, deprecated names, migration or fallback code, version
  gates, or "kept for old mods" branches. Old mods rebuild; the compiler says what moved. Removing an old
  path updates every caller, the catalog, docs, tests and sunrise-machine-sample in the same change.
- **One hook per moment.** A hook with a NeoForge 26.3 counterpart takes its name, side and full contract.
  Two hooks at the same vanilla call site (including `_v1`/`_v2` pairs) merge into that one, and the old
  one is deleted, not kept beside it. A veto before an action and a notice after it are separate moments
  when vanilla runs code between them. A hook whose vanilla call site is gone is removed from the catalog.
- **Every hook gets a `Host`.** Hooks that fire inside client-lib split into decide, fire, apply: client-lib
  decides, the game fires the hook with a `Host`, then applies. The listener's writes and the apply run in
  one transaction, and the apply re-checks the decision's preconditions and refuses (never recomputes) if
  they changed.
- **`Host` shape.** A small shared `Host` (handle constructors, registries, settings, `transaction`), with
  `ServerHost: Host` and `ClientHost: Host` generated from each catalog entry's `side`, so a wrong-side
  call doesn't compile; a hook that fires on both sides gets a typed `HostSide`. Per-object methods live on
  handles (reads take `&dyn Host`, writes `&mut dyn ServerHost`), and the ID-based helper a handle method
  covers is removed when it lands. Drawing goes through a `DrawContext` given only to render and screen
  hooks. Items, fluids and energy go through Parts under transactions. Commands, players, messages and
  scheduling are small service traits reached from `Host`; attachments live on handles. Don't add a
  method to `Host` that has a natural receiver.
- **One dependency model.** Mods declare required and optional dependencies in `[dependencies]` in
  `sunrise-mod.toml`, using semver constraints on manifest versions. The game dispatches in dependency
  order. Native mods link through Cargo; a library mod can publish functions and statics to its
  dependents, which share one copy of the library.
- **Native binaries target the exact game build.** The loader checks the build ID a native mod was built
  against. `.smod` is the one package format for both native and content-only mods.
- **DRY.** One implementation per behaviour: reuse the existing helper, write path or generator rule, and
  let the catalog and generator own repetition instead of hand-written match arms.
- **SOLID.** Each module, trait and handle has one job; traits stay small and split by side and concern;
  new behaviour comes in through the catalog, handles and parts; mods depend on `mod_api`, never on game
  internals.
- **Neat.** Names, layout and docs match the code around them. No dead code, leftover TODO paths, or
  anything a later PR must tidy up.
- **Fast.** No allocation, string building or scan over every mod on hot paths (tick, per-entity and
  per-block hooks). Dispatch only to the mods and parts that subscribe, resolve keys once at load, and a
  PR that touches a hot path states its tick-time effect.

## Don't lose time

Where agent time goes: reading code, compiling `sunrise-client`, waiting in loops, and work someone else had
already done.

- **Check nobody else is on it.** Before you write code, look at the issue's linked pull requests and run
  `gh pr list --state open --search "<issue number, function or file name>"`; then `/claim`.
- **Find code with the map, not by reading.** Start from the tables below and
  `python3 tools/port_ledger.py where Class.method`. Read the function you change and its callers, not whole
  files.
- **Compile cheaply while you work.** `cargo check -p <crate>` after an edit. Run the checks at the end with `pr_ready.py` and use both CI lint commands above for gameplay changes. `cargo clippy -p sunrise-client` and `cargo build -p sunrise-client` are the
  slowest commands here: never after every edit, and build the game only when you need to run it.
- **Don't wait in loops.** Start a long command in the background and carry on, or wait for it once. No
  `sleep` or `until grep` polling of builds, harnesses or CI.
- **One test per changed behaviour.** None for a refactor or a file move, none that reads source text, and no
  harness where vanilla has nothing to compare (Sunrise's own features). Performance work is proven by the area's
  bit-exact harnesses (add one first if none covers the code) and a before/after benchmark.
- **A red check isn't always yours.** When `verify` fails in a test your change can't reach, say so in the pull
  request and move on; don't hunt it.
- **Unsure about a rule? Finish the work.** Take the reading that lets you finish, and say in the pull request
  which one you took. Don't stop, and don't change direction halfway.

## Crates

Game workspace (root `Cargo.toml`); `crates/sunrise-telemetry-server` is a separate workspace, and the launcher is its own repository, [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher).

| Crate | Owns |
|---|---|
| `sunrise-client` | Library shell for window and dedicated startup, the integrated-server wiring, allocator and joined tests. `src/lib.rs` exposes `src/entry.rs`, `src/game_facade.rs` and `src/cut_layout/game.rs`. The `client` feature enables the desktop side and the `sunrise-client-lib` re-exports. |
| `sunrise-client-lib` | Reusable client systems, `sunrise-ctl` and client probes. Its manifest points to `crates/sunrise-client/src/client_lib.rs` and the tool and test sources alongside it. It sits below client gameplay. |
| `sunrise-gameplay` | Logical server (`ServerGame`): world and entity ticks, player turns, world writes, commands, saves, tracking, LAN and dedicated hosting. `src/game/administration/game_tests.rs` and the handlers in `src/game/administration/commands/` implement vanilla GameTest. |
| `sunrise-client-gameplay` | `ClientGame`: the window loop, rendering and screen glue, input and sound, remote packets, client prediction, and client mod hooks. |
| `sunrise-gameplay-common` | `CommonGame`, shared types and behaviour, and the `SideGame` traits used by both sides. |
| `sunrise-sim` | LAN transport and hub, raw payloads and item sync, mob and motion code, entity NBT and saves. |
| `sunrise-level` | World and runtime chunks, lighting and chunk streaming, level block entities, collision queries and portable level data. |
| `sunrise-items` | Blocks and items below gameplay: `blocks.rs` (block facts), `items.rs`, `item_data.rs` (components), `enchant.rs`, `cooldowns.rs`, `bundle.rs`, `item_nbt.rs`. Re-exported as `sunrise_client::items`... |
| `sunrise-command` | Command parsing (Brigadier): the vanilla tree, argument types, selectors, suggestions, the tree's network form. Re-exported as `sunrise_client::command` |
| `sunrise-game-data` | Portable game data below commands and gameplay, used by client and server: `text.rs`, `lang.rs`, `snbt.rs`, `nbt_path.rs`, `component.rs`, `scoreboard.rs`, `bossbar.rs`, `gamerules.rs`, `block_entity_nbt.rs`, registry/particle/sound tables. Re-exported under the old paths (`sunrise_client::text`...) |
| `sunrise-client-input` | Key bindings and key names, toggle keys, mouse options, controller settings and glyphs, the clipboard, window options (`keybinds.rs`, `gamepad.rs`, `mouse.rs`, `window.rs`...). Re-exported under the old paths |
| `sunrise-client-render` | The client's world data, rendering and screens (chunks, meshing, light, streaming, `gfx`, HUD, menus, settings): its modules (`ls crates/sunrise-client-render/src`), which sunrise-client re-exports under the same paths. Nothing in it reaches gameplay |
| `sunrise-stage` | Prebuilt game staging, SDK surface export and comparison, and standalone mod source scans (`surface`, `surface-diff`, `uses`, `fits`) |
| `sunrise-mod-vocab` | Source of truth for the public mod API (engine, portable; imported by engine crates as `mod_api`): host area traits in `src/host/` (named `<Area><Tier>`), hooks in `src/hooks/`, domain types, and `pub const` limits. Re-exported via `OUT_DIR/mod_api_exports_root.rs`. |
| `sunrise-mod-api` | Platform facade re-exporting `sunrise-mod-vocab` (`mod_vocab::*`), plus registration, parts and namespace authority (`src/load_state.rs`, `src/parts/owned_namespaces.rs`). |
| `sunrise-mod-api-gen`, `sunrise-mod-surface`, `sunrise-game-discovery` | Build-time generator for `Host`/`ServerHost`/`ClientHost` forwarders, `fire_<hook>`, `GoalCx`, mixins/loops and `surface.json`; `surface.json` schema/hashing; `game/` module and hook discovery. |
| `sunrise-mod-host` | Platform mod loader (registers the Mods title button via `sunrise-hooks`). |
| `sunrise-mod-runtime` | Engine-independent mod attachment storage, query index and generic registry slot tables; depends only on `sunrise-core` |
| `sunrise-world` | Vanilla world data with no gameplay loop: NBT, regions, chunks, `datafix/`, `worldgen/`, `level/` (server-side block behaviour, ticks, redstone), entity physics, items, effects, food, damage. Depends on `sunrise-mod-vocab`. Runtime `World` and `Chunk` live in `sunrise-level`. |
| `sunrise-border` | Engine crate for world-border math, with only `sunrise-core` as a dependency (not listed in `portable.crates`). |
| `sunrise-core` | Simulation primitives: Java `Random`s, `jmath` (HotSpot-exact `Math`), positions, noise, density, paletted storage, `version.rs` (the pinned version) |
| `sunrise-protocol` | Network protocol 777: codecs, framing, encryption, `packets/{handshake,status,login,configuration,play}.rs`, `entity_data.rs` |
| `sunrise-assets` | Resource decoding: block-state and item models, baking, atlases |
| `sunrise-prepare` | Builds `.local/sunrise-client/26.3` from the player's own Minecraft install (runs the Java exporters) |
| `sunrise-server` | `sunrise-server`: `server.toml`, CLI, supervisor of the headless game ([docs/hosting.md](docs/hosting.md)) |
| `sunrise-server-files` | `server.properties`, eula, ops/whitelist/bans, usercache |
| `sunrise-java-properties` | `java.util.Properties` load/store and the JDK date text, standard library only; used by `sunrise-server-files` |
| `sunrise-friends`, `sunrise-relay` | Play with Friends: invite codes, encrypted tunnel, the relay ([docs/play-with-friends.md](docs/play-with-friends.md)) |
| `sunrise-mod-package`, `cargo-sunrise-mod` | Content mods (`.smod`: manifest, content fence, no code), the registry index and `.spack`, and `cargo sunrise-mod` (`new`, `build`, `run`, `check`, `publish`, `registry`, `native-build`) ([docs/modding/](docs/modding/src/introduction.md)). Code mods are native, built by the launcher ([docs/modding/src/how-to/native-mods.md](docs/modding/src/how-to/native-mods.md)) |
| `sunrise-native-build` | Native mods compiled against a signed prebuilt SDK: the launcher's build, and `cargo sunrise-mod native-build`'s JSON contract that every `sdk-<build id>` ships ([docs/modding/src/reference/sdk/08-native-build.md](docs/modding/src/reference/sdk/08-native-build.md)) |
| `sunrise-port-pack` | Porting Java modpacks ([docs/modpacks.md](docs/modpacks.md)) |
| `sunrise-proxy` | Shared proxy selection and connection pools for CLI downloads |
| `sunrise-connected` | Connected textures whatever file declares them: the neighbour geometry, the format-neutral `Rule` (OptiFine's and mods' both fill it) and its `table` method. Read by `sunrise-client-render`'s `connected.rs` and the mesher, and by `sunrise-optifine`; a mod's files are `assets/<mod>/sunrise/connected/` ([docs/modding/src/how-to/client/connected-textures.md](docs/modding/src/how-to/client/connected-textures.md)) |
| `sunrise-optifine` | OptiFine-format pack files below every feature that reads them: `.properties`, the name/range/block matchers, paths and `optifine/` folders, and custom entity models (`cem/`: `.jem`/`.jpm` parsing and baking) |
| `sunrise-polytone` | Polytone-format pack files (`assets/*/polytone/`), read clean-room from the public wiki: targets, biome and dimension modifiers and their environment attribute layers |
| `sunrise-shaders` | Shader packs (the `shaders/` folder format), rebuilt clean-room from a written spec of the format; no wgpu |
| `sunrise-theme` | Design tokens and fonts for the launcher, the game's own menus and the website |
| `sunrise-perf-format` | The `.hzperf` capture contract ([docs/perf-platform.md](docs/perf-platform.md) §3): `proto/sunrise/perf/v1/capture.proto`, its generated types, the gzip writer and its thread. No Sunrise dependencies; nothing in the game reads a capture |
| `sunrise-telemetry`, `sunrise-telemetry-server`, `sunrise-bench` | Opt-in data collection, its ingest server, the benchmark report format |
| `sunrise-addresses` | Every built-in address of an outside service vanilla doesn't contact (Sunrise layer; [docs/layers.md](docs/layers.md#built-in-addresses)) |
| `sunrise-system` | The machine the game runs on (OS, CPU, memory, drive, power, GPU maker and memory), for the crash report, the renderer's GPU info and telemetry |
| `sunrise-platform` | The platform's public API: what the Sunrise layer may use of the engine ([docs/layers.md](docs/layers.md)); re-exports `sunrise-system` today |
| `sunrise-hooks`, `sunrise-game`, `sunrise-official`, `sunrise-heartbeat` | The layer seam: `sunrise-hooks` is the engine→platform/Sunrise static extension seam (title buttons, settings, HUD, lang, shutdown — **not** the mod API); `sunrise-game` is the thin playable executable (`src/main.rs`, `[[bin]] test = false`); `sunrise-official` is the shipped `sunrise-client` executable registering Sunrise's in-game parts; `sunrise-heartbeat` is the first of them ([docs/layers.md](docs/layers.md#how-a-sunrise-crate-plugs-in)) |
| `sunrise-reconnect` | Auto-reconnect: its switch, its policy and backoff, on the engine's connection hooks; registered by `sunrise-official` ([docs/extras.md](docs/extras.md)) |
| `sunrise-dualsense`, `sunrise-threads`, `sunrise-release-sign`, `sunrise-bots`, `xtask` | DualSense output, thread priorities, signed release checksums, load-test bots, repo tasks |
| [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher) | GPUI launcher, in its own repository (moved there with its history, 2026-11-18): make launcher changes there. `launcher-bridge.yml` signs that repository's releases with this repository's release key and publishes them there and here as `launcher@X.Y.Z`, for launchers installed from here |

Other top-level folders: `tools/` (harnesses and dev scripts), `tests/` (Python unit tests: `python3 -m
unittest discover -s tests`), `mods/sunrise/src/test/java/dev/sunriselab/parity/` (Java oracles),
`docs/`, `.github/workflows/`. The website (playsunrise.net) is `frontend/site` in the private sunrise-org/sunny: its build
reads `README.md`, `docs/api/`, `roadmap/roadmap.json` and `crates/sunrise-theme/tokens.css` from main.

## Write tests with signal

- Test observable behavior, not words in docs, comments or source. Keep checks for machine-consumed contracts.
- Use independent expected values or a vanilla oracle; do not derive the answer from the code under test.
- Test outcomes, not only calls to stubs. Use mocks to isolate behavior, not as the behavior itself.
- Skip tests of trivial constants, derives and getters, or behavior already covered by another test.
- One test per changed behaviour is enough. A refactor or a move that changes no behaviour gets no new test.
- Avoid pinning implementation details; a harmless refactor should not break a behavior test.
- Make slow tests faster without removing cases or weakening their coverage.
- Never delete parity harnesses or tests that pin vanilla behaviour: they are the spec.
- No workarounds: when a harness differs, fix the cause until it matches bit for bit; never add a tolerance, rounding,
  a special case or a compensating patch to pass it. Removing one, from the code or a harness, is welcome while the
  harnesses stay bit-exact: smaller code is a goal.

## Build and test fast

```sh
python3 tools/pr_ready.py                  # self-check; paste its block in the PR
python3 tools/pr_ready.py --quick          # list affected harnesses without running them
cargo test -p <owning-crate> --lib <module>::tests::   # unit tests for one area
cargo test --workspace --lib --bins --tests          # CI's unit-test targets (run --doc separately)
cargo test -p sunrise-sim --lib mod_content::tests:: -- --ignored   # prepared-data content-root tests
cargo build --profile fast -p sunrise-game        # optimized, quick to link: use it to play or run lan_e2e
python3 tools/parity.py --changed origin/main       # the harnesses your branch can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt --list   # which harnesses cover these files, without running them
python3 tools/parity.py mob_ai sculk       # named harnesses
python3 tools/lan_e2e.py --check chest     # two real games over LAN (server-thread host)
python3 tools/agent_control_e2e.py         # sunrise-ctl / MCP end to end
python3 tools/lang_coverage.py --summary   # one line per language: how many keys it carries and what it misses
python3 tools/lang_coverage.py --summary --show-keys   # the same, then the keys behind the counts
```

- GPU tests share `gfx::test_gpu::lock()`: bind the guard before creating GPU objects so it outlives
  every resource, queue, device and instance, including GPU-owning workers (join them before unlocking).
  Helpers returning GPU objects leave the guard with their caller; helpers returning only CPU data may
  own it themselves, without another outer guard. CPU-only tests need no guard. Run renderer coverage
  with `cargo test -p sunrise-client-render --lib -- gfx::`; no global serial-test flag.

- For a player-facing change, drive the game once with `sunrise-ctl` (launch, the action, a `state` or `screenshot`)
  and report the result in the PR's "Tried in game" line; never claim it when it was not run.
- Harnesses and the game need the vanilla data in `.local/` (`minecraft/`, `sunrise-client/26.3`, `datagen/`,
  `tools/`): see [docs/parity-workflow.md](docs/parity-workflow.md). Without it, pr_ready lists the
  harnesses for CI. Fresh git worktrees start without `.local/`; symlink `.local/{minecraft,sunrise-client,datagen}` read-only from an existing prepared checkout instead of re-running `sunrise-prepare`.
- Checking PRs, keep one warm worktree (`.local/` linked as above) and move it between heads with `git checkout --detach <sha>`: Cargo rebuilds only what changed, where a fresh worktree pays a cold build each time (most noticeable on Windows). Never share one `CARGO_TARGET_DIR` between worktrees: Cargo and build-script outputs (such as `sunrise-mod-api-gen`'s) built for one worktree's paths can be reused by another and give wrong results. Don't `git clean` the warm `target/` away (no `git clean -x` there).
- Enable `--features sunrise-client/state-digest` with `-p sunrise-game` for `tools/state_digest_scene.py`
  and `tools/state_digest_compare.py`. The shell's manifest forwards `state-digest` to `sunrise-gameplay`.
- Run `python3 tools/ci/format_scan.py` for Cargo formatting and the per-file checks for included Rust sources.
- Keep the test-profile overrides for `sunrise-client` and `sunrise-client-lib` to limit rustc memory, and
  `lto = "off"` in `[profile.fast]` for incremental builds (`Cargo.toml`). Keep
  `sunrise-client-lib/default` in the `sunrise-game` default features so game and probe builds can reuse the
  same client feature set (`crates/sunrise-game/Cargo.toml`).
- Find a harness with `python3 tools/parity.py --all --list`. A harness pairs a vanilla oracle with a Rust probe
  (`ls crates/*/examples/probe`), a game fixture such as `crates/sunrise-gameplay/src/game/interface/dispense_fixture.rs`,
  or a `#[test]` that calls a single `run_<test>()` helper. Harnesses select those helper fixtures through
  `lab.parity_fixtures()` and the `sunrise-parity-fixtures` executable ([parity workflow](docs/parity-workflow.md#sunrise-client-fixtures)).
- Native mods compile against a pinned prebuilt SDK; `[[native.mixin]]` rows compile into a function-replacement patch library.
  Use `native-build --trust-native-code`; the launcher never rebuilds the game for a mod.
- Probes build into `.local/parity-target/`; harnesses find binaries via `lab.cargo_target()`, never `target/`.
- Harness selection uses Cargo dep-info and reachable-code narrowing (`tools/parity_reach.py`).
  The control tool and client probes use `sunrise-client-lib`, below gameplay. Inspect the actual selection
  with `python3 tools/parity.py --changed-files paths.txt --list`.
- Integration-test files under `crates/sunrise-client/tests/` are registered by the `sunrise-client-lib`
  manifest. Joined gameplay unit tests live under `crates/sunrise-client/src/cut_layout/`.
- Linux desktop builds use `libasound2-dev` and `libudev-dev`. Build the headless game with
  `cargo build -p sunrise-game --no-default-features`.
- Run and drive the game (starts the `sunrise-client` beside `sunrise-ctl`; other arguments go to the game):
  `target/fast/sunrise-ctl --socket $PWD/ctl.sock launch --world W --creative --offscreen --wait --resources $PWD/.local/sunrise-client/26.3`,
  then `sunrise-ctl --socket … state|keys|turn|command|screenshot|quit`
  ([docs/sunrise-ctl-commands.md](docs/sunrise-ctl-commands.md)).

## Checks CI runs that you can run locally

`.github/workflows/dyna-ci.yml` splits static checks across `format`, `verify-static` and `verify`:

```sh
# Source and manifest checks (no Cargo)
python3 tools/ci/check_repository_hygiene.py                 # format / lint
python3 tools/ci/check_gameplay_layers.py                    # verify-static
python3 tools/ci/check_api_boundary.py --source-only         # format / lint (ENGINE set is hardcoded, separate from layers.json)
python3 tools/check_public_tree.py                           # verify-static (CI adds --range BASE..HEAD or --history)
python3 tools/ci/surface_extension_tags.py --check           # verify-static
python3 tools/sync_upstream.py --check                       # verify-static
python3 tools/check_addresses.py --base origin/main          # verify-static local; CI: ${BASE_SHA:+--base "$BASE_SHA" --pull-request-merge}
python3 tools/hotspots/check_guards.py --base origin/main    # verify-static local; CI: ${BASE_SHA:+--base "$BASE_SHA" --pull-request-merge}
python3 tools/hotspots/check_lists.py && python3 tools/hotspots/check_stand_ins.py
python3 tools/check_mod_vocabulary_state.py                  # verify-static
python3 tools/ci/check_port_map_paths.py --base origin/main  # verify-static (--refresh prunes resolved stale rows)
python3 tools/ci/check_game_boot_tests.py && python3 tools/ci/reexec_include_ignored.py --check
python3 -m unittest discover -s tools/ci -p 'test_prepared_*.py' -v   # verify-static
python3 -m unittest tests/test_ci_test_scope.py              # targeted example before the full suite

# Python tests (some compile Rust fixtures)
python3 -m unittest discover -s tests -v --durations=25      # verify-static

# Invoke Cargo, rustfmt, or discovery binaries
python3 tools/ci/format_scan.py                              # format / lint
python3 tools/check_layers.py --base origin/main             # verify-static local (tools/layers.json); CI: ${BASE_SHA:+--base "$BASE_SHA" --pull-request-merge}
bash tools/ci/check_hook_wiring.sh                           # verify-static (all target cfgs without building sunrise-client)
python3 tools/ci/check_ignored_test_rows.py                  # verify-static
python3 tools/ci/check_unreached_sources.py                  # verify-static (sunrise-game-discovery)
python3 tools/ci/check_mod_gpu_access.py                     # verify-static
python3 tools/ci/parity_fixtures_manifest.py --check         # generated manifest
python3 tools/ci/check_process_tests.py && python3 tools/ci/check_nested_locks.py && python3 tools/ci/check_extras_features.py && python3 tools/ci/check_parity_harness_builds.py
```

- **PR merge flag (`--pull-request-merge`):** CI passes `--base "$BASE_SHA" --pull-request-merge` to `check_launcher_mirrors.py`, `check_addresses.py`, `check_guards.py`, `check_port_map_paths.py` and `check_layers.py` when `BASE_SHA` is set from the PR event (otherwise `--base` is omitted). This flag requires a two-parent `HEAD` whose first parent is at or descended from `--base`. For a local topic branch, use `--base origin/main`.
- **Mod API surface check (`verify`):** when heavy `verify` requires Rust checks for a PR or a dispatch with a PR input, it builds `sunrise-stage` and runs `python3 tools/ci/surface_report.py` after the build succeeds (without `--trees`, building the base commit's own `sunrise-stage` and comparing the public mod API and published loops, excluding advisory listed internals); `surface.gate` in `tools/ci/surface.toml` controls whether breaking changes without new change notes fail the job (not whether the report runs). Local form: build the head tool (`cargo build -p sunrise-stage --bin sunrise-stage`), archive the base into a fresh directory (`base_dir=$(mktemp -d) && git archive origin/main | tar -x -C "$base_dir"`), then run `SUNRISE_STAGE_TOOL=$PWD/target/debug/sunrise-stage python3 tools/ci/surface_report.py --trees "$base_dir" "$PWD"` (add `--with-internals` for an internals audit, or `--build-base-tool` when the generator changed). Dump exact `internal:*` IDs with `target/debug/sunrise-stage surface "$PWD" /tmp/out.json --with-internals`.
- **Change notes in `crates/sunrise-mod-vocab/changes/`:** `.toml` files are validated against CI's default non-internals diff and fail as stale if they name `internal:*` IDs; use `.md` for internals-only or narrative notes.
- **Gameplay lint:** use both commands in [Which side code goes on](#which-side-code-goes-on) for the workspace and server configurations.

## Vanilla reference

- Start parity work from the port ledger, not grep: `python3 tools/port_ledger.py issue N` (or `where
  Class.method`, `what file:line`), and `check --base origin/main` before the PR. How:
  [.claude/skills/vanilla-parity/SKILL.md](.claude/skills/vanilla-parity/SKILL.md), [docs/port-ledger.md](docs/port-ledger.md).
- Decompiled sources: `.local/minecraft/26.3/sources/{client,server}/` (`python3 tools/lab.py decompile`).
- Bytecode queries (callers through lambdas, every overload):
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`.
- A harness runs the oracle against the real 26.3 jars and the Rust probe on the same inputs, then compares
  their JSON. Oracle runs are cached by `tools/parity_shim.py`; `--fresh` reruns them.
- Match vanilla's `Math` results with `sunrise_core::jmath`, and vanilla's RNG with `sunrise_core::random`:
  random draws must happen in vanilla's order, in performance work too ([CONTRIBUTING.md](CONTRIBUTING.md#vanilla-first)).

## Gameplay and client files by subsystem

Paths in the table are relative to each crate's `src/`.
**Server** = `sunrise-gameplay`, **Client** = `sunrise-client-gameplay`, **Common** = `sunrise-gameplay-common`,
**Shell** = `sunrise-client` (also the source tree for `sunrise-client-lib`), **Sim** = `sunrise-sim`,
**Level** = `sunrise-level`, **Render** = `sunrise-client-render`, **Input** = `sunrise-client-input`.

| Subsystem | Files |
|---|---|
| Game loop, tick, frame | Shell `cut_layout/game.rs` (`run_with`); Client `game/runtime/window_handler.rs` (`window_event`, `about_to_wait`), `game/game_client.rs` (`render_frame`); Server `game/world/levels.rs` (`tick_server`), `game/world/level_tick.rs` (`tick_world`); Sim `clock.rs`; Level `spans.rs` |
| Rendering | Render `gfx.rs` (`Renderer::render`), `gfx/renderer_construction.rs` (`Renderer::new`), `gfx/`, `particles.rs`, `block_entity_render.rs`, `camera.rs`; Shell `item_render.rs`, `first_person.rs`; Client `game/interface/render_tick.rs`, `game/interface/item_draw.rs`, `game/interface/screen_backgrounds.rs` |
| Chunks, streaming, light | Level `world.rs`, `chunk.rs`, `chunk_map.rs`, `chunk_streamer.rs`, `light.rs`; Render `streaming.rs`, `mesher.rs`, `lod.rs`; Sim `vanillagen.rs`, `ticket_levels.rs`; Server `game/world/world_loading.rs`; Client `game/world/world_update.rs` |
| World generation | `crates/sunrise-world/src/worldgen/` (terrain, features, structures and carvers); Sim `vanillagen.rs` (binds the generator) |
| Block behaviour | `crates/sunrise-world/src/level/` (block updates, redstone, placement, interactions); Server `game/blocks/block_ticks.rs` (`tick_blocks`), `game/blocks/block_entity_ticks.rs`; Level `block_entity_ticks.rs` (ticker state) |
| Mobs | Sim `mobs.rs`, `mobs/` (AI, goals, paths, species and spawn rules); Server `game/mobs/creatures/mod_server.rs`, `game/mobs/natural_spawning.rs`, and the family modules in `game/mobs/`; Shell `mobs/render.rs` |
| Other entities | Sim `entities.rs`, `projectiles.rs`, `entities/arrows.rs`; Server `game/entities/` (boats, stands, paintings, frames, TNT and explosions); Common `game/entities/entity_types.rs`; Client `game/network/remote_objects.rs`, `game/network/remote_projectiles.rs`, `game/network/remote_vehicles.rs` |
| Player actions | Server `game/items/use_hand.rs`, `game/items/using_item.rs`, `game/entities/melee.rs`, `game/blocks/block_breaking_server.rs`; Common `game/items/item_actions.rs`; Client `game/items/hand.rs`, `game/blocks/block_breaking.rs`; Sim `motion.rs`; Level `collisions.rs`; item definitions in `crates/sunrise-items/src/` |
| Inventory and containers | Sim `menu.rs`, `crafting.rs`, `recipe_book.rs`, `stations.rs`, `merchant.rs`; Client `game/interface/menu_clicks_client.rs`, `game/items/recipe_book.rs`, `game/blocks/stations.rs`; Server `game/blocks/machines/mod.rs`, `game/items/trading.rs`; Shell `container_screen.rs`, `creative.rs` |
| Commands | `crates/sunrise-command/src/` parses and encodes the tree; Server `game/administration/commands/`, `game/administration/functions.rs`, `game/administration/command_blocks_server.rs` execute it; Sim `scoreboard_cmd.rs`, `data_cmd.rs`; `crates/sunrise-game-data/src/scoreboard.rs` |
| UI screens | Common `game.rs` (`Screen`); Shell `title.rs`, `multiplayer.rs`, `cut_layout/game/interface/startup_menu.rs`; Render `ui.rs`, `settings_shell.rs`, `hud.rs`, `hud_kit/`, `pad_glyph.rs`; Client `game/runtime/loading_screen.rs`, `game/interface/`, `game/network/lan_screen.rs` ([UI kit](docs/ui-kit.md)) |
| Multiplayer client | Shell `net.rs` (network thread); Client `game/network/remote/mod_client.rs` (`apply_packet`), `game/network/remote/packets_*_client.rs`, `game/network/remote/tick_client.rs` (`tick_remote`) |
| LAN host / server side | Sim `lan/` (connections, hub, chunks, entities, tracking and player data), `raw_payloads/`, `item_sync.rs`; Server `game/network/lan/mod.rs` (`LanHost`), `game/network/lan_actions/`, `game/network/lan_entities.rs` |
| Integrated server | Server `game/world/integrated.rs`, `game/world/levels.rs`, `game/world/server_travel.rs`; Client `game/world/integrated_client.rs`; Shell `cut_layout/game.rs` (wiring), `cut_layout/game/integrated_pair.rs` (test fixture) |
| Dedicated server | Server `game/runtime/loading_server.rs`, `game/interface/dedicated/mod.rs`, `game/interface/dedicated/status.rs`; Shell `entry.rs` and `cut_layout/game.rs`; see `crates/sunrise-server-files/` and `crates/sunrise-server/` for configuration and supervision |
| Save and load | Server `game/world/persist.rs`, `game/world/world_session.rs`; Common `game/world/saving.rs`; Sim `save.rs` and `entity_nbt.rs`; Level `options_file.rs`; `crates/sunrise-game-data/src/block_entity_nbt.rs`, `crates/sunrise-world/src/region.rs`, `crates/sunrise-world/src/datafix/`, `crates/sunrise-core/src/nbt.rs` |
| Input | Client `game/runtime/window_handler.rs`, `game/interface/play_keys.rs`, `game/interface/play_input.rs`, `game/interface/pad/`, `game/interface/pointer_capture.rs`; Input `keybinds.rs`, `keymapping.rs`, `mouse.rs`, `interaction_input.rs`, `gamepad.rs`, `clipboard.rs`; Shell `osk.rs` |
| Audio | Render `audio.rs` (mixer), `sound_engine.rs`; Shell `music.rs`, `ambience.rs`; Client `game/interface/music.rs`, `game/interface/subtitle_hud.rs`; Level `sound_events.rs`, `audio_capture.rs`; Server `game/server_side/server_effects.rs` (sound capture for clients) |
| Resource packs | Render `packs.rs`, `packs/`, `pack_screen.rs`, `atlas_sources.rs`, `model_library.rs`, `font.rs`; Shell `mobs/cem.rs`, `mobs/cem_anim.rs`; Client `game/world/packs/mod.rs`; `crates/sunrise-game-data/src/lang.rs`, `crates/sunrise-assets/`, `crates/sunrise-optifine/` |
| Agent control | Sim `control.rs` (socket); Shell `bin/sunrise-ctl/`; Client `game/runtime/control.rs`, `game/runtime/agent/mod.rs`, `game/runtime/events.rs`, `game/runtime/ui_control.rs` ([control protocol](docs/agent-control.md)) |
| Mods | Server, Client and Common each have `game/host/mod.rs` and `game/native_hooks/fire.rs`; Server `game/modding/mods/`, `game/modding/native_goal_ticks/mod.rs`; Sim `mod_content.rs`; Render `mod_list.rs`. See [Mod API](#mod-api-the-clean-break) and the crate map above. |
| Sunrise extras, settings | Render `extras.rs`, `graphics.rs`, `settings.rs`; Client `game/interface/extras.rs`, `game/interface/extras/`, `game/interface/options.rs` |

## Server game, client game, common state, and shell

- The gameplay crates declare their types in their own `src/game.rs` files. `ServerGame` and `ClientGame`
  each own a `CommonGame` as `self.common` and implement `SideGame` for shared extension traits. Each side's common
  state includes `Resources` (`res`, tables such as blocks and items), `PackResources` (`pack`), and the
  side's world data. Look up an extension method in common as well as on the concrete side.
- Server and Client declare side modules in `game/server_side.rs` and `game/client_side.rs` respectively.
  Look for the concrete tick entry points in the subsystem table, then search the owning crate with
  `rg -n 'fn tick_<thing>|fn <verb>_<thing>'`. The shell calls the side entry points and gives the window a
  server-start callback. `crates/sunrise-game/src/main.rs` is the thin executable and carries the
  `GAME_IDENTITY` marker for launcher imports.
- `ExecutionRole::{Client, IntegratedServer, DedicatedServer}` lives in Common `execution_role.rs`.
  Use `is_logical_server()` or `is_dedicated_server()` for role-specific behaviour. The integrated server
  can also set `options.dedicated`. On the client, `remote` tracks the connection independently of the role.
- Server `game/network/lan_actions/turns_server.rs` runs a guest's packet or tick through `guest_turn`,
  putting that guest's player state in place and restoring the previous state at exit. `with_guest_swap`
  is the command and fixture entry point. `game/world/levels.rs` selects the active dimension and parks
  the other levels. The block log, level events and entity tracker propagate results to each client.

## Single player runs on a server thread

The shell's `crates/sunrise-client/src/cut_layout/game.rs` passes `serve_integrated` to the client startup.
Client `game/world/integrated_client.rs` starts the thread and joins through an in-memory connection when
ready. Server `game/world/integrated.rs` opens the world and runs the server loop, with a level per dimension
in `game/world/levels.rs`. The window ticks its client view and applies packets.

Single player, LAN guests and dedicated players all use server gameplay. For changes to packets, tracking,
guest turns or thread startup, exercise the affected connection with its harness:

```sh
cargo run --profile fast -p sunrise-game -- --resources .local/sunrise-client/26.3 --world T
python3 tools/lan_e2e.py --check <name>
python3 tools/parity.py dedicated_server server_entities server_player_damage
```

## Where a type or symbol really lives

| Looking for | Real home and lookup |
|---|---|
| Mod API host traits, hooks, types and limits | `crates/sunrise-mod-vocab/src/host/` and `crates/sunrise-mod-vocab/src/hooks/`. `#[host_area]` generates handle methods from the hidden backend methods. Search `rg -n 'fn <name>' crates/sunrise-mod-vocab/src/hooks` for hooks or `rg -n 'pub const ' crates/sunrise-mod-vocab/src` for limits. |
| Mod namespace authority and bare IDs | `crates/sunrise-mod-api/src/load_state.rs` and `crates/sunrise-mod-api/src/parts/owned_namespaces.rs` (`declaration_id`). Bare declarations bind to the primary mod ID. |
| Generated host forwarders and `fire_<hook>` | Each gameplay crate's `build.rs` calls `sunrise-mod-api-gen`. See the includes in `game/host/mod.rs` and `game/native_hooks/fire.rs`. Server `game/modding/native_goal_ticks/mod.rs` defines `GoalCx` and includes its generated view. |
| `Dim` and `EXTRA_DIMS` | Common `game.rs`. Server `game/world/levels.rs` owns how the game selects and ticks its dimension state. |
| Level views | Common `game.rs` defines `GameLevel<'w>` and `with_level` for `LevelAccess`; Server `game/mobs/natural_spawning.rs` has a separate `GameLevel<'g, 'a>` for spawning. |
| Command state and chat UI | Common `game/interface/chat_common.rs` (`WorldCommandState`); Client `game/interface/chat.rs` (`ChatUiState`). |
| Block entity render and tick state | Render `block_entity_anim.rs` (`BlockEntityAnims`); Level `block_entity_ticks.rs` (including `gateway_servers` and `conduit_servers`). Look up re-exports in Shell `client_lib.rs`. |
| `HostSide` selection and `ExecutionRole` | Common `game/runtime/execution_role.rs` and `execution_role.rs`; each side implements `host_side` in its side module. |
| Packet enums and `EntityLists` | `packet_enum!` in `crates/sunrise-protocol/src/packets/`; `EntityLists` is declared inside a macro in Common `game/entities/entity_types.rs`. |
| Engine UI and extension hooks | [UI kit](docs/ui-kit.md) for internal UI and `crates/sunrise-hooks/src/` for platform extensions. Mod UI lives in `crates/sunrise-mod-vocab/src/client/` and `crates/sunrise-mod-vocab/src/host/`. |

## Moving or renaming code

- **Module reachability:** update the owning crate's declarations, including `#[path]` and `include!()` edges,
  and preserve the gates on declarations and re-exports. Use `python3 tools/ci/check_unreached_sources.py`.
- **Port map:** update `tools/port_map/*.tsv` to point to the file with the function body. Use
  `python3 tools/ci/check_port_map_paths.py --refresh` to prune resolved entries from
  `tools/ci/port_map_known_stale.txt`.
- **Ignored tests:** update the package, target and full module path in `tools/ci/ignored-tests/` and
  `tools/ci/ignored-test-exceptions.json` when they change. See `SHARDS` in `tools/ci/ignored_shards.py` for
  the packages run by the daily shards. Run `python3 tools/ci/check_ignored_test_rows.py`.
- **Test scope and areas:** check external source reads in `tools/ci/test_scope.py` (`READERS`, `SCANNED`,
  `NOT_READS`), and update `tools/test_areas.toml` for path globs and game-boot tests. Run
  `python3 -m unittest tests/test_ci_test_scope.py tests/test_test_areas.py`.
- **Mod API forwarders, loops and replacements:** keep generated host forwarders resolving to the intended
  side's implementation. Preserve published `#[sunrise_loop]` bindings such as `block_entities.tick`
  (Server `game/blocks/block_entity_ticks.rs`). The staging code in `crates/sunrise-mod-api-gen/src/replace.rs`
  discovers eligible gameplay functions and `#[replaceable]` functions below gameplay. SDK internal
  exports are listed separately in `crates/sunrise-mod-api-gen/internals.txt`. Check the targets and paths in
  `tools/ci/fn-replace-check/` and `crates/sunrise-mod-vocab/behaviours/`, then compare the SDK surface.
- **Parity and source readers:** keep each fixture test as a single call to a `run_<test>()` helper.
  Check the dispatcher in `crates/sunrise-parity-fixtures/build.rs` and regenerate its manifest with
  `python3 tools/ci/parity_fixtures_manifest.py` when the shell manifest changes. Search `tests/`,
  `tools/` and `docs/modding/src/concepts/native-route/` for the old path and symbol before moving them.

## Traps

- **Server sounds are gameplay output.** Server `game/network/lan/mod.rs` starts capture with the hub.
  `game/server_side/server_effects.rs` queues sounds, and `lan_broadcast_effects` in
  `game/network/lan_actions/effects_server.rs` forwards them. Preserve the capture and drain order.
- **Drop order matters.** Keep `player_autosaves` before `session_lock` in Server `game.rs` and
  `game/server_side.rs` so writes finish while the world is locked. Render `streaming.rs` aborts generation
  on `MeshStreamer` drop and leaves its `ChunkStreamer` to drain saves. Pool `Registration` waits for
  running jobs (`crates/sunrise-threads/src/jobs.rs`).
- **A search for a method on the concrete type can miss its definition.** Check Common extension traits,
  macros and included fragments. Search field names without the receiver to catch accesses through
  `self.common`, wrappers and multiline chains.
- **Render dirtiness is consumed.** Render `streaming.rs` drains `World.dirty_sections` during mesh
  processing. Check authoritative block publication through the server's block log and tracker.
- **Mod API contracts live in the vocabulary.** Use rustdoc in `crates/sunrise-mod-vocab/src/` for
  timing and isolation guarantees.

## Docs worth knowing

[docs/parity-workflow.md](docs/parity-workflow.md) (runner, gate, shards, scopes) ·
[docs/native-client.md](docs/native-client.md) (options, controls, sunrise-ctl) ·
[docs/server-levels.md](docs/server-levels.md) (levels per dimension) ·
[docs/status.md](docs/status.md) (what is ported) · [docs/known-issues.md](docs/known-issues.md) ·
[docs/release-flow.md](docs/release-flow.md) · [docs/README.md](docs/README.md) (index).
