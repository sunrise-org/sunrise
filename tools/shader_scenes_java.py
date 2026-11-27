"""tools/shader_scenes.py's Java side: Java 26.3 from your own prepared copy, with the Fabric loader and the dev-only
driver mod (mods/sunrise/src/scenes), plus released Sodium and Iris for a pack, run as black boxes to take the
reference pictures. `world` generates the shared world with it."""
import json
import os
import shutil
import subprocess
import tempfile
import uuid
import zipfile
from pathlib import Path

from tools.shader_scenes import (ROOT, WORLD, camera_problems, capture_problems, copy_world, dimension_id,
                                 install_pack, options_text, pack_digest, pack_key, record_view, run_folder,
                                 phase_step, scene_hash, settings_expected, settle_plan, setup_commands, start_commands,
                                 start_game, start_problems, stop_game, view_commands, write_run)
from tools.shader_scenes_report import RUN_MARK

MODS_LOCK = ROOT / "tools/shader-scenes/mods.lock.json"
DRIVER_SOURCE = ROOT / "mods/sunrise/src/scenes"
PLAYER = "SceneBot"


def build_driver(classpath, build, javac):
    """Compiles the dev-only driver mod (mods/sunrise/src/scenes) against your own client.jar, into `build` (a
    fresh folder of this run)."""
    classes = build / "classes"
    classes.mkdir()
    sources = sorted(map(str, (DRIVER_SOURCE / "java").rglob("*.java")))
    subprocess.run([javac, "--release", "25", "-proc:none", "-encoding", "UTF-8", "-d", str(classes),
                    "-classpath", os.pathsep.join(map(str, classpath)), *sources], check=True)
    jar = build / "sunrise-scenes.jar"
    with zipfile.ZipFile(jar, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(classes.rglob("*.class")):
            archive.write(path, path.relative_to(classes).as_posix())
        for path in sorted((DRIVER_SOURCE / "resources").rglob("*")):
            if path.is_file():
                archive.write(path, path.relative_to(DRIVER_SOURCE / "resources").as_posix())
    return jar


def iris_mods(mods_from):
    """Sodium, Iris and Fabric API from mods.lock.json: from the folder --mods-from names when a file there has the
    pinned sha512, else downloaded from the lock's URL (checked by runtime.fetch); kept in .local/pack-cache."""
    from tools import lab, runtime
    lock = json.loads(MODS_LOCK.read_text())
    jars = []
    for project in lock["projects"].values():
        target = ROOT / ".local/pack-cache" / project["filename"]
        if mods_from and not target.exists():
            for candidate in sorted(Path(mods_from).glob("*.jar")):
                if lab.digest(candidate, "sha512") == project["sha512"]:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy(candidate, target)
                    break
        jars.append(runtime.fetch(project["url"], target, project["sha512"], "sha512"))
    return jars


def java_command(args, scenes, build, game_dir, script, events, natives, classpath):
    from tools import lab, perf_bench, runtime
    manifest = perf_bench.vanilla_manifest()
    java = Path(args.java_home) / "bin/java" if args.java_home else "java"
    width, height = scenes["size"]
    command = [str(java), f"-Xmx{args.heap}", *perf_bench.LAUNCHER_JVM,
               *runtime.conditional_client_jvm_options(manifest), *perf_bench.manifest_jvm_flags(manifest),
               "--enable-native-access=ALL-UNNAMED",
               f"-Djava.library.path={natives}", f"-Dorg.lwjgl.system.SharedLibraryExtractPath={natives}",
               f"-Dsunrise.scenes.script={script}", f"-Dsunrise.scenes.events={events}",
               "-Dfabric.log.disableAnsi=true",
               "-cp", os.pathsep.join(map(str, classpath)),
               "net.fabricmc.loader.impl.launch.knot.KnotClient",
               "--username", PLAYER, "--uuid", "00000000000000000000000000000000", "--accessToken", "0",
               "--offlineDeveloperMode", "--version", lab.VERSION, "--gameDir", str(game_dir),
               "--assetsDir", str(build / "assets"), "--assetIndex", manifest["assetIndex"]["id"],
               "--width", str(width), "--height", str(height)]
    return command


def java_setup(args):
    """The driver jar, the natives and the classpath, built into a fresh folder under OUT/build (never one an
    earlier run left). The Fabric libraries come from .local/runtime, downloaded there on first use."""
    from tools import perf_bench, runtime
    (Path(args.out) / "build").mkdir(parents=True, exist_ok=True)
    build = Path(tempfile.mkdtemp(prefix="java-", dir=Path(args.out) / "build"))
    (build / "assets" / "indexes").mkdir(parents=True)
    manifest = perf_bench.vanilla_manifest()
    classpath = perf_bench.vanilla_classpath(manifest) + runtime.fabric_libraries()
    javac = str(Path(args.java_home) / "bin/javac") if args.java_home else "javac"
    jar = build_driver(classpath, build, javac)
    natives = build / "natives"
    perf_bench.extract_natives(classpath, natives)
    return build, jar, natives, classpath


def iris_selected(log_text, name):
    """Black-box evidence that Iris runs the requested pack: the line it logs when it takes a pack,
    `Using shaderpack: <name>`, for exactly this name."""
    return any(line.rstrip().endswith(f"Using shaderpack: {name}") for line in log_text.splitlines())


def run_java_game(args, scenes, folder, pack, create, shots, setup):
    from tools import lab
    build, jar, natives, classpath = setup
    game_dir = folder / "game"
    (game_dir / "mods").mkdir(parents=True)
    shutil.copy(jar, game_dir / "mods")
    # Iris draws through OpenGL; the rest keeps first-start screens (onboarding, tutorial toasts) away.
    extra = {"preferredGraphicsBackend": '"opengl"', "onboardAccessibility": "false", "tutorialStep": "none",
             "skipMultiplayerWarning": "true", "realmsNotifications": "false", "narrator": "0"}
    record = {"side": "java", "pack": Path(pack).name if pack != "none" else "none", "pack_sha256": pack_digest(pack),
              "views": {}, "notes": []}
    name = None
    if pack != "none":
        for mod in iris_mods(args.mods_from):
            shutil.copy(mod, game_dir / "mods")
        name = install_pack(pack, game_dir)
        (game_dir / "config").mkdir(exist_ok=True)
        (game_dir / "config" / "iris.properties").write_text(
            f"enableShaders=true\nshaderPack={name}\ndisableUpdateMessage=true\n")
    if not create:
        record.update(copy_world(args.out, game_dir, scenes))
    (game_dir / "options.txt").write_text(options_text(scenes, lab.version_info()["world_version"], extra))
    views = [{"name": v["name"], "dimension": dimension_id(v["dimension"]), "pos": v["pos"],
              "commands": view_commands(v), "setup": setup_commands(v) if shots else [],
              "thaw": thaw, "weather": v["weather"], "settle": settle}
             for v, (thaw, settle) in zip(scenes["views"], settle_plan(scenes))]
    first = scenes["views"][0]
    script = folder / "script.json"
    script.write_text(json.dumps({
        "world": WORLD, "seed": scenes["seed"], "create": create, "shots": shots, "out": str(folder),
        # The world run settles too: its views' weather is checked, and rain fades in and out over seconds.
        "settle": scenes["settle_seconds"], "timeout": scenes["load_timeout_seconds"],
        "start": start_commands(scenes), "views": views,
        "phase_shots": scenes["phase_shots"] if shots else 1, "phase_step": phase_step(scenes, "java"),
        # The world is saved with the player back at the first view, so both games open it in that dimension.
        **({"finish": view_commands(first),
            "finish_at": {"dimension": dimension_id(first["dimension"]), "pos": first["pos"]}} if create else {})},
        indent=2))
    events = folder / "events.jsonl"
    command = java_command(args, scenes, build, game_dir, script, events, natives, classpath)
    env = dict(os.environ, ALSOFT_DRIVERS="null")
    proc = start_game(args, scenes, command, env, folder / "game.log", game_dir, folder)
    try:
        try:
            proc.wait(timeout=args.java_timeout)
        except subprocess.TimeoutExpired:
            record["notes"].append(f"the game did not quit within {args.java_timeout} s")
            stop_game(proc, 0)
        record["exit_status"] = proc.returncode
        if proc.returncode != 0:
            record["notes"].append(f"the game exited with status {proc.returncode}")
        lines = [json.loads(l) for l in events.read_text().splitlines()] if events.exists() else []
        by_view = {}
        for e in lines:
            by_view.setdefault(e.get("view") or e.get("owner"), []).append(e)
        queried = {e["command"]: e["result"] if e["success"] else None
                   for e in by_view.get("start", []) if e["event"] == "command"}
        record["start_problems"] = start_problems(scenes, queried)
        if not any(e["event"] == "done" for e in lines):
            record["notes"].append(f"the driver did not finish: last event {lines[-1:] or 'none'}")
        for e in lines:
            if e["event"] == "title":
                record["renderer"] = {k: e.get(k) for k in ("gpu", "backend", "driver", "size")}
            if e["event"] == "finished":
                record["finished"] = camera_problems(first, e.get("camera"))
        pack_problems = []
        if name:
            log = (folder / "game.log").read_text(errors="replace")
            record["iris_selected"] = iris_selected(log, name)
            if not record["iris_selected"]:
                pack_problems.append(f"Iris never logged that it uses {name}")
        for view, script_view in zip(scenes["views"], views):
            got = by_view.get(view["name"], [])
            shot = next((e for e in got if e["event"] == "shot"), {})
            loaded = next((e for e in got if e["event"] == "loaded"), {})
            # A view with setup commands is ready only if it was ready again after them.
            reloaded = next((e for e in got if e["event"] == "reloaded"), None)
            ready = loaded.get("ready", False) and (not script_view["setup"] or bool(reloaded and reloaded["ready"]))
            thaw = thaw_problems(script_view, next((e for e in got if e["event"] == "thawed"), None))
            phases = [e for e in got if e["event"] == "phase_shot"]
            setup = set(setup_commands(view))
            failed = [e["command"] for e in got
                      if e["event"] == "command" and not e["success"] and e["command"] in setup]
            extra_problems = pack_problems + thaw
            if shots:
                extra_problems += capture_problems(shot, scenes["size"])
                for e in phases:
                    extra_problems += [f"phase shot {e['index']}: {p}" for p in capture_problems(e, scenes["size"])]
                if len(phases) != scenes["phase_shots"] - 1:
                    extra_problems.append(f"{len(phases)} phase shots of {scenes['phase_shots'] - 1}")
                if "error" not in shot and not shot.get("file"):
                    extra_problems.append("no picture was written")
            record_view(record["views"], view, shot.get("camera"), failed, ready, extra_problems,
                        settings_expected(scenes))
            record["views"][view["name"]]["shots"] = [{"index": 0, "ticks": 0}] + [
                {"index": e["index"], "ticks": e.get("ticks")} for e in phases]
            print(f"java {pack_key(pack)} {view['name']}: {record['views'][view['name']]['problems'] or 'ok'}",
                  flush=True)
    except (ValueError, KeyError, OSError) as error:
        record["notes"].append(f"the run's events could not be read: {error}")
    finally:
        write_run(folder, record)
    return record


def thaw_problems(view, event):
    """What went wrong in a view's thaw (view: its script entry), from the driver's "thawed" event: the level must have
    started running after `tick unfreeze` and reached the weather's rain and thunder levels before it froze again."""
    if not view.get("thaw"):
        return []
    if event is None:
        return ["the thaw never finished"]
    if not event.get("thawed"):
        return ["the world never thawed after tick unfreeze"]
    if not event.get("reached"):
        return [f"the weather never reached its levels while thawed: rain {event.get('rain_level')}, "
                f"thunder {event.get('thunder_level')}"]
    return []


def world_failures(record, scenes):
    """Why a world run's save must not be published (empty: publish it): the game must exit with status 0 after
    the driver finished, every gamerule and view must have checked out, and the camera read back after the finish
    commands must be at the first view."""
    failures = list(record.get("notes", [])) + list(record.get("start_problems", []))
    failures += [f"{name}: {p}" for name, v in record.get("views", {}).items() for p in v.get("problems", [])]
    if "finished" not in record:
        failures.append("the camera was never read back after the return to the first view")
    else:
        failures += [f"return to {scenes['views'][0]['name']}: {p}" for p in record["finished"]]
    return failures


def run_world(args, scenes):
    """Generates the shared world with vanilla Java: creates it from the seed, visits every view so its chunks
    exist, returns to the first view, saves and quits. The save is published to OUT/world (with generation.json:
    a fresh generation id and the scene hash) only when world_failures finds nothing. Both games then load copies
    of it."""
    world = Path(args.out) / "world"
    if world.exists():
        if not args.replace:
            raise SystemExit(f"{world} exists: pass --replace to generate it again")
        if not (world / RUN_MARK).exists():
            raise SystemExit(f"{world} was not made by this tool; not removing it")
    (Path(args.out) / "build").mkdir(parents=True, exist_ok=True)
    folder = Path(tempfile.mkdtemp(prefix="world-run-", dir=Path(args.out) / "build"))
    record = run_java_game(args, scenes, folder, "none", True, False, java_setup(args))
    saved = folder / "game" / "saves" / WORLD
    failures = world_failures(record, scenes)
    if not (saved / "level.dat").exists():
        failures.append("no level.dat was saved")
    if failures:
        raise SystemExit("world generation failed, nothing published:\n  " + "\n  ".join(failures)
                         + f"\nsee {folder / 'game.log'}")
    if world.exists():
        shutil.rmtree(world)
    world.mkdir(parents=True)
    (world / RUN_MARK).write_text("")
    shutil.copytree(saved, world / WORLD, ignore=shutil.ignore_patterns("session.lock"))
    (world / "generation.json").write_text(json.dumps({"world_id": uuid.uuid4().hex,
                                                       "scene_hash": scene_hash(scenes)}) + "\n")
    print(f"world: {world / WORLD}")


def run_java(args, scenes, pack, setup):
    folder = run_folder(args.out, "java", pack, args.replace)
    return run_java_game(args, scenes, folder, pack, False, True, setup)
