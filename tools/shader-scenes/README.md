# Shader scene suite

`tools/shader_scenes.py` photographs a shader pack at the same fixed views in Sunrise and in Java 26.3, and
compares the pictures. `real_packs` says whether a pack's programs translate; this says whether the pack looks
like it does in Java with Iris. It needs a GPU, prepared resources and a Java 26.3 client, so it is a developer
tool, not a CI job; its own logic is tested by `tests/test_shader_scenes.py`.

## What is here

- `scenes.json`: the scene file. A seed, the picture size, pinned options (`options.txt` lines both games get;
  `graphicsPreset:"custom"` among them, since without it a first start applies a preset's distances instead),
  gamerules, the settle time, and the views. A view is a `name` (`[a-z0-9_]+`, also its picture's file name),
  a `dimension` (`overworld`, `the_nether`, `the_end`), the feet position `pos`, `yaw` and `pitch`, an optional
  `time` (day ticks) and `weather` (`clear`, `rain`, `thunder`), and `setup` commands run in the view's
  dimension once its chunks are loaded (blocks to place, mobs to summon), and an optional `settle_seconds` of its
  own. `python3 tools/shader_scenes.py check` validates it and prints every command each view sends.
- Settling: after a view is loaded each game waits `settle_seconds` before its picture, or `move_settle_seconds`
  when the camera moved from the previous view; a view whose weather changes first thaws the frozen world for
  `weather_settle_seconds`. The values are measured (`time` noon to sunset and `weather` clear to rain at one
  place, and a move to the warped forest; a capture after a short wait against the same view captured again 8 s
  later; Sunrise without a pack and with BSL, vanilla Java; the 64x36 grid difference, and whether the pixels are
  equal). With the world frozen and no pack, a time change gives pixel-identical pictures after 0.5 s on both
  sides; a move leaves 0.060 (Java) and 0.038 (Sunrise) after 1 s and 0.019 and 0.016 after 3 s; with BSL, Sunrise
  stays at the pack's own animation level (about 3.4 against 2.6 for a held view) from 1 s. Hence 1. Rain needs
  the level's ticks: thawed, it reaches its own level of change after 4 to 6 s; hence 6. Packs also smooth colours
  that depend on the place over seconds after a move (with 1 s, Complementary's warped forest fog in Java with Iris
  was still turning from the previous biome's), so a moved view keeps 6 s.
- A thaw must take effect: `tick unfreeze` succeeds and the game reports the level running before the thaw's time
  starts, and before it freezes again the rain and thunder must have reached the view's weather (Java: both
  levels at 0 or 1; Sunrise reports only the raining and thundering flags).
- `mods.lock.json`: the released Sodium, Iris and Fabric API jars the Java side loads, with their Modrinth URLs
  and sha512.
- `mods/sunrise/src/scenes/`: the dev-only Fabric driver mod for the Java side, compiled by the tool against your
  own `client.jar`. It reads the script the tool writes from the scene file, runs every command on the integrated
  server as its console (as `sunrise-ctl command` does in Sunrise), waits for the chunks, and saves each picture
  from the main render target.

## How a run goes

```sh
python3 tools/shader_scenes.py run --out OUT --pack PACKS/BSL_v10.1.8.zip --mods-from MODS
```

1. `world`: a vanilla Java client (Fabric loader and the driver only) creates the world from the seed, visits
   every view so its chunks are generated, goes back to the first view, saves and quits. The save is published to
   `OUT/world/scenes` only when the game exited with status 0, every view and gamerule checked out and the camera
   read back after the return is at the first view; `OUT/world/generation.json` then records a fresh generation id
   and the scene's hash. Both games load copies of this one world, and open it where the player was saved. Two fresh worlds on the same
   seed place some features differently, so comparing fresh worlds would measure the trees, not the pack.
2. `sunrise`: for each pack, a Sunrise game (`sunrise-client --offscreen --control-socket`, driven through
   `sunrise-ctl`) with scratch home and XDG folders inside `OUT/sunrise/<pack>/`, the scene's options and a copy of
   the world. It runs the gamerules, `gamemode spectator @a` and `tick freeze`, hides the HUD (F1), then for each view places the
   camera, waits until it is loaded (below), runs the setup, waits again, settles, reads `state` back and takes a
   `screenshot`.
3. `java`: the same views in Java 26.3. The pack `none` is vanilla (Fabric loader and the driver); a pack runs
   with Sodium and Iris from `mods.lock.json`, the pack in `shaderpacks/` and `config/iris.properties` selecting
   it, on OpenGL. A pack run takes only the checked-in `scenes.json` (D-11, below).
4. `compare`: every picture is box-filtered to a 64x36 grid (16:9 at a twentieth of 1280x720, so animated water,
   clouds and leaves count for little) and the two sides are compared cell by cell. `OUT/report/` gets
   `index.html` (both pictures side by side with a heat map of the difference), `report.md` (packs ranked by mean
   difference, then each pack's views) and `report.json`. `--pairs PACK/VIEW` (or `all`) also writes
   side-by-side PNGs under `report/pairs/`.

Both games freeze the world (`tick freeze`) before the first view: entities (the End's dragon, the summoned
mobs), fluids and tick-driven particles hold still, so they don't differ between the two games or between runs.
A view whose weather changes thaws it for `weather_settle_seconds`, since rain fades in and out only while the
level ticks, and freezes it again. While frozen, leaving the End keeps the dragon fight's boss bar, and its world
fog, in other dimensions (vanilla drops it only when the fight scans for players, which needs the End to tick), so
the scene file refuses a first view in the End and any view after an End view in another dimension: End views go
last. Java also reads its boss bars back, and a view outside the End with one has a problem (Sunrise's control
protocol has no boss-bar read-back). Client-side texture animation (lava, fire, water, portals) and anything a pack
animates by its own clock still run on both sides.

Frozen texture animations stop on whatever frame each game reached, unrelated between the games, so each view takes
`phase_shots` pictures: the first, then one after each `tick step` of the frozen world, 1 tick apart in Sunrise and
`phase_shots` ticks apart in Java. The K x K pairs then cover every offset from 0 to K x K - 1 ticks; with 9 shots,
81 ticks, past vanilla's longest common block animation (still lava, 76 ticks; prismarine's slow 6600-tick cycle is
not covered). A view's difference is its best-matching pair, the one with the two sides in the same phase (each
shot's phase is its index and stepped ticks, recorded in `run.json`; the report names the pair). Its own variation
is the larger of the two sides' mean difference between their own shots: what texture animation and a pack's own
clock (`frameTimeCounter`, which no step lines up) change on one side alone. A difference at or under it is marked
"within the pack's own variation". Choosing the best of 81 pairs leans low for views with nothing to line up;
the own variation is the yardstick for those.

`run` always includes the pack `none`: vanilla Sunrise against vanilla Java is the noise floor (clouds, animation,
the two renderers' own small differences), printed and shown above the packs. The steps also run on their own
(`world`, `sunrise`, `java`, `compare`), with the same `--out`. `--jobs N` (default 2) runs that many games at
once after the world is made: with `run`, a Sunrise and a Java game side by side. Each game has its own folders,
display and socket, and needs a few GB of memory.

Both games run in a headless `gamescope` display of their own at the scene's size (`--display headless`, the
default), so no window opens on your desktop and the picture size doesn't depend on the desktop's compositor;
`--display current` uses the desktop's display instead.

## What is checked

- Loaded: Sunrise's view counts as loaded once the camera is in the view's dimension at its position, the resident
  chunks reached 80% of the view circle, the chunk counts held still for a second and no streaming stage held work
  for half a second (`tools/perf_bench.py`'s `wait_loaded`; `wait-ready` alone can answer right after a teleport,
  before loading starts). That full wait runs when the camera moved and after a pack load; at the place of the
  previous view (only time, weather or setup changed) only steady, drawn chunks are waited for. Java's driver waits for every chunk within the render distance minus one and for no
  section left to compile.
- The pack: Sunrise loads a pack when the world opens and again at each change of dimension, and logs each load
  that finishes (`shader pack <name>: N programs`) or fails (`... turned off: why`). Each load is tied to the change
  that caused it, in order; a load that never comes, comes late or comes twice makes the pack state of every later
  view unknown, never guessed. Java must log `Using shaderpack: <name>` for the requested pack; without it every
  view of the run has a problem.
- Before each picture, both games read the camera back (dimension, position, yaw, pitch, open screen, spectator
  mode, frozen: Java's level tick state, Sunrise's `tick query`), the overworld's time and weather, and the drawing settings they hold (render distance, simulation
  distance, field of view), which must equal the scene's pinned options; anything that differs from the view is written to the run's
  `run.json` and shown beside the picture. The commands that place the camera are judged this way, by what they did:
  vanilla's `time set` fails when the clock already shows that time.
- The gamerules are queried after they are set; a rule the game doesn't hold at the scene's value is named
  (setting a rule to the value it has fails in vanilla, so the sets themselves are not judged).
- A setup command that fails, a view that never finished loading within `load_timeout_seconds`, a pack that never
  loaded in a dimension or was turned off (Sunrise), a missing picture, a picture of another size than the
  scene's, and frames that are black, washed out or flat are named in the report.
- Each run records the world generation, the scene hash and the sha256 of the pack input it staged; a pair from
  different worlds, scene definitions or pack files is not compared as valid. A failed start check (a gamerule,
  the freeze) is a problem of every view of that run. Each picture's size is checked when it is taken. A picture that can't be read (damaged, cut short, another size) is a
  problem of that picture only.
- A view with any problem on either side keeps its pictures, difference and notes in the report but counts in no
  mean and not in the noise floor. `compare` exits non-zero when no view is scored; `sunrise`, `java` and `run`
  exit non-zero when a pack they ran has no view without a problem, and `run` also when a requested pack has no
  scored view. Two packs whose names give the same folder name are refused before anything runs.

## Where it writes

- Everything a run makes goes under `OUT`: the world, each side's folder per pack (game folders, homes, XDG
  folders, logs, pictures, `run.json`), the Java build and world-generation scratch folders (fresh
  `OUT/build/java-*` and `OUT/build/world-run-*` each time) and the report. A side's folder for a pack is never
  overwritten: run again with `--replace`, which removes only folders this tool made (they carry a
  `.shader-scenes-run` mark).
- Outside `OUT`: the Fabric libraries in `.local/runtime/` and the mod jars in `.local/pack-cache/` (downloaded
  there on a cold cache, checked against their locks; `--mods-from` fills the jar cache from a local folder
  instead), and Sunrise's control socket in a fresh `/tmp/ssc-*` folder (a socket path must stay short), removed
  after the run.

## Inputs

- Sunrise: `--sunrise-ctl` (default: the cargo target's `fast/sunrise-ctl`; `CARGO_TARGET_DIR` is honoured),
  `--sunrise-client` (default: beside it) and `--resources` (default `.local/sunrise-client/26.3`). Build with
  `cargo build --profile fast -p sunrise-game --bin sunrise-client -p sunrise-client-lib --bin sunrise-ctl`.
- Java: the prepared vanilla copy in `.local/minecraft/26.3` (`tools/lab.py prepare`), the Fabric loader from
  `tools/benchmark-deps/fabric.lock.json` (`tools/runtime.py`), a Java 25 (`--java-home`, default `JAVA_HOME`),
  and for packs the jars of `mods.lock.json`: from `--mods-from` (a folder holding them, such as a launcher
  instance's `mods/`; matched by sha512) or downloaded from the lock's URL, cached in `.local/pack-cache/`.
- Packs: zips or folders named with `--pack`. They are copied into each game's `shaderpacks/` and loaded; the
  tool never opens them.

Nothing the tool writes is committed: pictures, worlds, jars and packs stay in `OUT` and `.local/`. To share
references, keep `OUT/java/<pack>/` (and `OUT/world/`, which they were taken in) as a local folder or a CI
artifact; `python3 tools/shader_scenes.py java --out OUT --pack P` regenerates them.

## Clean room

[D-11](../../docs/cleanroom/decisions.md#d-11-java-263--iris-as-a-black-box-reference-for-the-scene-suite) allows
one thing: running the pinned Iris and Sodium jars as a black box, only to capture screenshots of the checked-in
`scenes.json`'s views as comparison references. The tool refuses a pack run with another scene file. Their source,
decompiled code and documentation, any inspection of the jars, and the packs' GLSL stay out of bounds; the
`depends` field of each jar's `fabric.mod.json` (metadata, to know which jars load together) is the one thing
looked up. Nothing in Sunrise's code comes from the pictures. [docs/cleanroom/README.md](../../docs/cleanroom/README.md)
says what the shader implementation may read.
