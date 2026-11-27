# Shader scene suite

`tools/shader_scenes.py` photographs fixed views with two Sunrise builds, base and head, on the same world, scenes,
pack and options, and compares every picture pixel for pixel at full resolution. A pixel that differs is a
difference between the builds: the same build captured twice writes the same bytes. It catches visual regressions
of a change, with or without a shader pack, and is the scene proof for changes that must keep the picture bit for
bit (a planner off and on: the same build twice, with `--head-env` setting the switch). It needs a GPU and prepared
resources, so it is a developer tool, not a CI job; its own logic is tested by `tests/test_shader_scenes.py`.

## How a run goes

```sh
python3 tools/shader_scenes.py run --out OUT --base-bin-dir BASE --head-bin-dir HEAD --pack PACKS/BSL_v10.1.8.zip
```

`BASE` and `HEAD` are folders holding a build's `sunrise-client` and `sunrise-ctl`
(`cargo build --profile fast -p sunrise-game --bin sunrise-client -p sunrise-client-lib --bin sunrise-ctl`, then
copy both out of the target folder). Each side drives its game with its own `sunrise-ctl`.

1. `world`: the base build creates the world from the scene's seed, visits every view so its chunks exist, goes back
   to the first view and quits, which saves it. The save is published only when every view loaded, the camera
   ended at the first view and the game exited with status 0, as a new generation:
   `OUT/world/generations/<id>/scenes` and its `generation.json` (the generation id, the scene's hash, the build's
   sha256 and a digest of the save: its files' relative names and sha256, `session.lock` left out), written whole,
   then `OUT/world/current.json` is pointed at it by one atomic rename. Until then the previous generation stays the
   published one, so a generation or a publication that fails leaves it as it was; earlier generations are kept.
   Every capture loads a copy of the generation `current.json` names, so both sides draw the same chunks and
   entities; the copy is hashed and must be the save its `generation.json` records, or the capture is refused.
2. `capture --side base` and `--side head`: for each pack (`run` always adds `none`, no pack), a game with scratch
   home and XDG folders inside `OUT/<side>/<pack>/`, the scene's `options.txt` (its `version` line is the
   `data_version` of the prepared resources' `sunrise-resources.json`) and `sunrise-graphics.txt`, a copy of
   the world and the pack copied into its `shaderpacks/`. The game runs with `SUNRISE_STILL=1`, which freezes the
   world before its first tick and makes what the frame shows of time follow the world's ticks
   ([docs/agent-control.md](../../docs/agent-control.md#repeatable-screenshots)). It runs the gamerules,
   `gamemode spectator @a` and `tick freeze`, hides the HUD (F1), then for each view places the camera, waits until
   it is loaded (below), runs the setup (only once the view loaded: a view that never loads skips it), waits again,
   settles, reads the state back and captures it with
   `screenshot-sequence FRAMES STEP_MS`: the pack starts over and draws `frames` frames on a clock `step_ms` apart,
   and the last is the picture. Nothing steps the world: no tick runs while a run captures.
3. `compare`: every view of every pack, base against head, pixel for pixel. `OUT/report/` gets `index.html` (base,
   head and a picture of the difference, the differing pixels in red over a darkened head), `report.md`,
   `report.json` and `diff/<pack>/<view>.png` for each view that differs.

`--jobs N` (default 2) runs that many games at once, each with its own folders, display and socket, so a base and a
head game run side by side; it also sets how many pictures are compared at once. `--base-env KEY=VALUE` and
`--head-env KEY=VALUE` (or `capture --env`) set a variable in one side's game environment; the variables the tool
sets itself (the scratch home, launcher home and XDG folders, `SUNRISE_STILL`, `SUNRISE_SHADER_PACK`, the display,
no audio or self-update, `PERF_BENCH_TRACE`) are refused there, and set after any `--env` value. The steps also run on
their own (`world --bin-dir`, `capture --side --bin-dir`, `compare`), with the same `--out`.

Both games run in a headless `gamescope` display of their own at the scene's size (`--display headless`, the
default), so no window opens on your desktop and the picture size doesn't depend on the desktop's compositor;
`--display current` uses the desktop's display instead.

## The scene file

`scenes.json`: a seed, the picture size, the capture clock (`frames`, default 64, and `step_ms`, default 50),
pinned options (`options`: `options.txt` lines; `graphicsPreset:"custom"` among them, since without it a first start
applies a preset's distances), pinned Sunrise graphics settings (`graphics`: `sunrise-graphics.txt` lines;
`screenshot-sequence` needs far terrain, TAA, frame generation and the shadow cache off, and chunk fade 0 in
`options`), gamerules, the settle times and the views. A view is a `name` (`[a-z0-9_]+`, also its picture's file
name), a `dimension` (`overworld`, `the_nether`, `the_end`), the feet position `pos`, `yaw` and `pitch`, an optional
`time` (day ticks) and `weather` (`clear`, `rain`, `thunder`), `setup` commands run in the view's dimension once its
chunks are loaded (blocks to place, mobs to summon, with their facing given and a variant where it is random),
and an optional `settle_seconds` of its own. `python3 tools/shader_scenes.py check` validates it and prints every
command each view sends.

A view waits `settle_seconds` after it is loaded, or `move_settle_seconds` when the camera moved from the previous
view: the server sends the time once a second, and a capture taken while it lands is refused and taken again. With
the world frozen under `SUNRISE_STILL=1`, rain and thunder show at their full level at once, so a weather change needs
no thaw. Leaving the End while frozen keeps the dragon fight's boss bar, and its world fog, in other dimensions
(vanilla drops it only when the fight scans for players, which needs the End to tick), so the scene file refuses a
first view in the End and any view after an End view in another dimension: End views go last.

## What is checked

- Loaded: a view counts as loaded once the camera is in the view's dimension at its position, the resident chunks
  reached 80% of the view circle, the chunk counts held still for a second and no streaming stage held work for half
  a second (`tools/perf_bench.py`'s `wait_loaded`; `wait-ready` alone can answer right after a teleport, before
  loading starts). At the place of the previous view (only time, weather or setup changed) only steady, drawn chunks
  are waited for. `screenshot-sequence` itself refuses while anything is left to load; the capture then waits and
  tries again until `load_timeout_seconds`.
- Before each capture the game reads back the camera (dimension, position, yaw, pitch, open screen, spectator mode,
  frozen: `tick query`), the time and weather, and the drawing settings (render and simulation distance, field of
  view), which must equal the scene's; the day tick must be the view's `time` exactly (a day later is another
  moon), and it is read again after the capture and must not have moved. The capture's reply must name the scene's
  frame count and step, and its last frame's clock must be the one `screenshot-sequence` gives frame FRAMES: counter
  FRAMES, elapsed (FRAMES - 1) x STEP_MS and delta STEP_MS, in seconds, compared exactly as f32s. The
  gamerules are queried after they are set.
- A view whose setup did not run fully (skipped, or a command failed) leaves the world unlike the other side's:
  every view after it is invalid.
- The pack, for each capture: Sunrise loads a pack when the world opens and again at each change of dimension, and
  logs each load that finishes (`shader pack <name>: N programs`) or fails (`... turned off: why`). The log is read
  in order with each teleport's feedback line (`[CHAT] Teleported ...`), which the client logs after applying the
  move. A view that changes dimension awaits one load logged after its own move line, so a load logged before it
  (a reload of the dimension before, however late) never stands in for it. An awaited load stays awaited across
  views of that dimension, so a load that timed out is still missing at the next capture there; a move to another
  dimension while one is still awaited leaves the loads after it impossible to tell apart, and every capture in
  that dimension is invalid. A load nothing awaits makes the capture before it and the next one invalid, since the
  log can't place it. Before a capture its move line must be in the log, every awaited load must have come and the
  last must be running; anything the log shows between a capture's start and its reply makes that capture invalid
  (for `none` the moves are checked the same way). The capture's reply names the pack that drew
  (its kind and file, taken when the sequence starts; the sequence ends with an error if the pack, the world, the
  settings or the window change while it draws), which must be the staged one, or no pack for `none`.
- A capture's picture is hashed (sha256) as soon as its reply says it is written, and recorded in `run.json` with
  the view's checks. Each run records the world generation and its save's digest, the scene hash, the sha256 of the
  staged pack (hashed from the copy the game loads) and of the `sunrise-client` it ran, a digest of the prepared
  resources (every file's relative name and sha256), the renderer the game picked (GPU, vendor, driver, device type
  and graphics backend, from `sunrise-ctl status`) and the environment its `--env` set.
- `compare` takes a view only from a record that carries the view's checks (its `problems`, a list) with none,
  names the view's own picture (`<view>.png`) and its sha256, whose file is there with that sha256 and is a whole
  PNG of the scene's size (one IHDR, first), and only when both sides were taken in the same world and save, for
  the same scene definition (this scene file's), of the same staged pack, with the same prepared resources and on
  the same renderer; the report names what each side drew on. Anything else makes the view invalid: a missing
  record (a capture that stopped before recording), a malformed `run.json` or a field of the wrong type, a missing
  or changed file, a damaged picture, a pack run on one side only. A picture no record names is listed and never
  compared. `run` compares the packs it ran and only those; `compare` on its own compares every run folder under
  `OUT`.
- When a game stops answering, the run's note names each process and what it was doing before the tool's cleanup
  (the `gamescope` wrapper: running, or exited with its status; the `sunrise-client` in it: running or gone) and
  after it.
- Exit status of `compare` and `run`: 0 when every view of every pack is valid and equal, 1 when the comparison
  completed with every view valid and some differ, 2 when any view is invalid or missing; `capture` exits 2 when a
  pack it ran has a view without a record. Every step exits 3 when it can't be set up or the command line is wrong
  (a missing pack or build, a protected `--env` key, a scene file error, an `--out` that is not a folder, a
  published world that can't be read, a report folder it didn't make, a file it can't read or write); nothing is
  compared then. Two packs whose names give the same folder name are refused before
  anything runs.

## Where it writes

Everything a run makes goes under `OUT`: the world, each side's folder per pack (game folder, home, XDG folders,
logs, pictures, `run.json`), the world's scratch folder (a fresh `OUT/build/world-run-*` each time), the world's
generations and their pointer (`OUT/world/generations/`, `OUT/world/current.json`) and the report. An inherited `PERF_BENCH_TRACE` is
ignored: the readiness wait writes no trace.
A side's folder for a pack, the world and the report are never overwritten unless this tool made them (they carry a
mark), and the first two only with `--replace`. Outside `OUT`: Sunrise's control socket in a fresh `/tmp/ssc-*`
folder (a socket path must stay short), removed after the run.

Packs are zips or folders named with `--pack`. They are copied into each game's `shaderpacks/` and loaded; the tool
never opens them. Nothing the tool writes is committed: pictures, worlds and packs stay in `OUT`.
