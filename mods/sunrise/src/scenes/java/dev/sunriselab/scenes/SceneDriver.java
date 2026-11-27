package dev.sunriselab.scenes;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.blaze3d.systems.RenderSystem;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicInteger;
import net.fabricmc.api.ClientModInitializer;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.commands.CommandSource;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.phys.Vec3;

/**
 * Dev-only scene driver for the vanilla client (tools/shader_scenes.py): the same
 * steps the tool drives in Sunrise through sunrise-ctl. From the title screen it
 * creates the scene world (script "create": true, the seed's world, saved for both
 * games) or opens the copy in saves/, runs the script's start commands, then for
 * each view runs its commands on the integrated server (console source, as
 * sunrise-ctl's "command" does), waits until the view's chunks are loaded and
 * compiled, runs its setup commands, waits the settle time and saves the main
 * render target as {@code <out>/<view>.png}. Every command's outcome, the camera
 * read back before each picture and the picture's path go to the events file as
 * JSON lines; the tool checks them. After the last view it saves the world,
 * leaves it and quits.
 *
 * Script (system property sunrise.scenes.script), written by the tool:
 * {"world", "seed", "create", "shots", "out", "settle", "timeout", "start": [command...],
 * "views": [{"name", "dimension", "pos": [x,y,z], "commands": [command...], "setup": [command...], "settle"?,
 * "thaw"?}], "phase_shots", "phase_step",
 * "finish"?: [command...], "finish_at"?: {"dimension", "pos"}}: finish runs after the last view; once the player
 * is at finish_at, a "finished" event carries the camera read back, then the world is left (which saves it).
 * Events: sunrise.scenes.events.
 */
public final class SceneDriver implements ClientModInitializer {
    private static SceneDriver driver;

    private enum Phase { TITLE, WORLD, START, VIEW, LOAD, SETUP, RELOAD, UNFREEZE, THAW, REFREEZE, SETTLE, SHOT, STEP, SAVE,
        LEAVE, DONE }

    private final JsonObject script;
    private final Path events;
    private final List<JsonObject> views = new ArrayList<>();
    private Phase phase = Phase.TITLE;
    private int view = -1;
    private long phaseStart;
    private long readyCheck;
    private int lastLoaded = -1;
    private long lastChange;
    /** Commands sent to the server thread whose outcome has not come back yet. */
    private final AtomicInteger pending = new AtomicInteger();
    private final ConcurrentLinkedQueue<JsonObject> outcomes = new ConcurrentLinkedQueue<>();
    private volatile boolean shotWritten;
    /** The current view's phase shot: 0 is its main picture. */
    private int shotIndex;
    private long lastProgress;

    /** Fabric's entrypoint instance; the driver itself is the static one. */
    public SceneDriver() {
        this.script = null;
        this.events = null;
    }

    private SceneDriver(JsonObject script, Path events) {
        this.script = script;
        this.events = events;
        for (var element : script.getAsJsonArray("views")) views.add(element.getAsJsonObject());
    }

    @Override public void onInitializeClient() {
        String scriptPath = System.getProperty("sunrise.scenes.script");
        String eventsPath = System.getProperty("sunrise.scenes.events");
        if (scriptPath == null || eventsPath == null) return;
        try {
            JsonObject json = JsonParser.parseString(Files.readString(Path.of(scriptPath))).getAsJsonObject();
            driver = new SceneDriver(json, Path.of(eventsPath));
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        driver.event("init", new JsonObject());
    }

    /** Called at the start of every frame (Minecraft.runTick). */
    public static void frame(Minecraft mc) {
        if (driver != null) driver.step(mc);
    }

    private void step(Minecraft mc) {
        long now = System.nanoTime();
        drainOutcomes();
        double elapsed = (now - phaseStart) / 1e9;
        double timeout = script.get("timeout").getAsDouble();
        if (now - lastProgress > 10_000_000_000L) {
            // Every 10 s: where the driver is, so a run that hangs says on which screen.
            lastProgress = now;
            JsonObject e = new JsonObject();
            e.addProperty("phase", phase.name());
            e.addProperty("view", view);
            e.addProperty("screen", mc.gui.screen() == null ? "none" : mc.gui.screen().getClass().getName());
            e.addProperty("overlay", mc.gui.overlay() == null ? "none" : mc.gui.overlay().getClass().getName());
            event("progress", e);
        }
        if ((phase == Phase.WORLD || phase == Phase.LEAVE || phase == Phase.SHOT) && elapsed > timeout) {
            JsonObject e = new JsonObject();
            e.addProperty("phase", phase.name());
            event("timeout", e);
            phase = Phase.DONE;
            mc.stop();
            return;
        }
        switch (phase) {
            case TITLE -> {
                if (mc.gui.screen() instanceof TitleScreen && mc.gui.overlay() == null) {
                    event("title", renderer(mc));
                    String world = script.get("world").getAsString();
                    enter(Phase.WORLD, now);
                    if (script.get("create").getAsBoolean()) {
                        LevelSettings settings = new LevelSettings(world, GameType.CREATIVE,
                            LevelSettings.DifficultySettings.DEFAULT, true, WorldDataConfiguration.DEFAULT);
                        WorldOptions options = new WorldOptions(script.get("seed").getAsLong(), true, false);
                        mc.createWorldOpenFlows().createFreshLevel(world, settings, options,
                            WorldPresets::createNormalWorldDimensions, mc.gui.screen());
                    } else {
                        mc.createWorldOpenFlows().openWorld(world, () -> { });
                    }
                }
            }
            case WORLD -> {
                if (mc.level != null && mc.player != null && mc.gui.screen() == null && mc.gui.overlay() == null
                        && mc.getSingleplayerServer() != null) {
                    if (!mc.gui.hud.isHidden()) mc.gui.hud.toggle();
                    event("world", new JsonObject());
                    run(mc, script.getAsJsonArray("start"), "start");
                    enter(Phase.START, now);
                }
            }
            case START -> {
                if (pending.get() == 0) next(mc, now);
            }
            case VIEW -> {
                shotIndex = 0;
                run(mc, views.get(view).getAsJsonArray("commands"), views.get(view).get("name").getAsString());
                enter(Phase.LOAD, now);
            }
            case LOAD -> {
                JsonObject v = views.get(view);
                boolean ready = pending.get() == 0 && ready(mc, v, now);
                if (ready || elapsed > timeout) {
                    JsonObject e = named(v);
                    e.addProperty("ready", ready);
                    e.addProperty("load_ms", (now - phaseStart) / 1e6);
                    if (mc.level != null) e.addProperty("chunks", mc.level.getChunkSource().getLoadedChunksCount());
                    event("loaded", e);
                    run(mc, v.getAsJsonArray("setup"), v.get("name").getAsString());
                    enter(Phase.SETUP, now);
                }
            }
            case SETUP -> {
                if (pending.get() == 0) {
                    // Setup changed blocks: wait until they are drawn again before going on.
                    if (views.get(view).getAsJsonArray("setup").isEmpty()) afterSetup(mc, now);
                    else enter(Phase.RELOAD, now);
                }
            }
            case RELOAD -> {
                boolean ready = ready(mc, views.get(view), now);
                if (ready || elapsed > timeout) {
                    JsonObject e = named(views.get(view));
                    e.addProperty("ready", ready);
                    event("reloaded", e);
                    afterSetup(mc, now);
                }
            }
            case UNFREEZE -> {
                // The thaw's time starts once the client level runs.
                if (pending.get() == 0 && !mc.level.tickRateManager().isFrozen()) {
                    enter(Phase.THAW, now);
                } else if (elapsed > timeout) {
                    thawed(mc, false, false);
                    refreeze(mc, now);
                }
            }
            case THAW -> {
                // Rain and thunder fade in and out with the level's ticks: wait the thaw time, then until both
                // levels reached the view's weather.
                JsonObject v = views.get(view);
                double thaw = v.get("thaw").getAsDouble();
                if (elapsed >= thaw && (reached(mc, v) || elapsed > thaw + timeout)) {
                    thawed(mc, true, reached(mc, v));
                    refreeze(mc, now);
                }
            }
            case REFREEZE -> {
                if (pending.get() == 0) enter(Phase.SETTLE, now);
            }
            case SETTLE -> {
                JsonObject v = views.get(view);
                double settle = (v.has("settle") ? v : script).get("settle").getAsDouble();
                if (elapsed >= settle) shoot(mc, now);
            }
            case SHOT -> {
                // Then the phase shots: `tick step` the frozen world (texture animations advance with it) and take
                // another picture, phase_shots - 1 times.
                if (shotWritten) {
                    if (script.get("shots").getAsBoolean() && shotIndex + 1 < script.get("phase_shots").getAsInt()) {
                        shotIndex++;
                        run(mc, single("tick step " + script.get("phase_step").getAsInt()),
                            views.get(view).get("name").getAsString());
                        enter(Phase.STEP, now);
                    } else {
                        next(mc, now);
                    }
                }
            }
            case STEP -> {
                // The step is done once the server and the client stopped stepping and the step's ticks had time
                // to pass; then a few frames draw the new phase.
                double wait = script.get("phase_step").getAsInt() * 0.05 + 0.25;
                var server = mc.getSingleplayerServer();
                if (pending.get() == 0 && elapsed >= wait && !server.tickRateManager().isSteppingForward()
                        && !mc.level.tickRateManager().isSteppingForward()) {
                    phaseShot(mc, now);
                } else if (elapsed > timeout) {
                    JsonObject e = named(views.get(view));
                    e.addProperty("index", shotIndex);
                    e.addProperty("error", "the tick step never finished");
                    event("phase_shot", e);
                    next(mc, now);
                }
            }
            case SAVE -> {
                // After the finish commands: once the player is back at the script's finish_at view (or the
                // timeout passed), the camera is read back for the tool to check, then the world is left.
                if (pending.get() == 0) {
                    // Back means at the view with the level showing: a dimension change shows a loading screen first.
                    boolean back = (!script.has("finish_at") || at(mc, script.getAsJsonObject("finish_at")))
                        && mc.gui.screen() == null && mc.gui.overlay() == null;
                    if (back || elapsed > timeout) {
                        JsonObject e = new JsonObject();
                        e.addProperty("back", back);
                        if (mc.level != null && mc.player != null) e.add("camera", camera(mc));
                        event("finished", e);
                        enter(Phase.LEAVE, now);
                        mc.disconnectWithSavingScreen();
                    }
                }
            }
            case LEAVE -> {
                // Left once the integrated server has stopped (it saves as it stops); the screen stays on
                // "Saving world" since nothing opens the title screen after it.
                if (mc.level == null && mc.getSingleplayerServer() == null) {
                    phase = Phase.DONE;
                    event("done", new JsonObject());
                    mc.stop();
                }
            }
            case DONE -> { }
        }
    }

    private void next(Minecraft mc, long now) {
        view++;
        if (view < views.size()) {
            enter(Phase.VIEW, now);
            return;
        }
        // Leaving the world saves it (save-all is a dedicated server's command).
        run(mc, script.has("finish") ? script.getAsJsonArray("finish") : new JsonArray(), "finish");
        enter(Phase.SAVE, now);
    }

    /** Writes the camera read back and the picture of the current view (or only the camera without shots). */
    private void shoot(Minecraft mc, long now) {
        JsonObject v = views.get(view);
        JsonObject e = named(v);
        e.add("camera", camera(mc));
        enter(Phase.SHOT, now);
        if (!script.get("shots").getAsBoolean()) {
            event("shot", e);
            shotWritten = true;
            return;
        }
        shotWritten = false;
        Path file = Path.of(script.get("out").getAsString(), v.get("name").getAsString() + ".png");
        Screenshot.takeScreenshot(mc.gameRenderer.mainRenderTarget(), image -> {
            try (image) {
                image.writeToFile(file);
                e.addProperty("file", file.toString());
                e.addProperty("width", image.getWidth());
                e.addProperty("height", image.getHeight());
            } catch (IOException error) {
                e.addProperty("error", error.toString());
            }
            event("shot", e);
            shotWritten = true;
        });
    }

    /** Phase shot `shotIndex` of the current view: `<view>.p<index>.png`, after index * phase_step stepped ticks. */
    private void phaseShot(Minecraft mc, long now) {
        JsonObject v = views.get(view);
        JsonObject e = named(v);
        e.addProperty("index", shotIndex);
        e.addProperty("ticks", shotIndex * script.get("phase_step").getAsInt());
        enter(Phase.SHOT, now);
        shotWritten = false;
        Path file = Path.of(script.get("out").getAsString(), v.get("name").getAsString() + ".p" + shotIndex + ".png");
        Screenshot.takeScreenshot(mc.gameRenderer.mainRenderTarget(), image -> {
            try (image) {
                image.writeToFile(file);
                e.addProperty("file", file.toString());
                e.addProperty("width", image.getWidth());
                e.addProperty("height", image.getHeight());
            } catch (IOException error) {
                e.addProperty("error", error.toString());
            }
            event("phase_shot", e);
            shotWritten = true;
        });
    }

    /** The camera as the tool checks it: dimension, feet position, angles, open screen, game mode, time, weather,
     * render and simulation distance and field of view. */
    private static JsonObject camera(Minecraft mc) {
        JsonObject camera = new JsonObject();
        camera.addProperty("dimension", mc.level.dimension().identifier().toString());
        Vec3 at = mc.player.position();
        JsonArray pos = new JsonArray();
        pos.add(at.x);
        pos.add(at.y);
        pos.add(at.z);
        camera.add("pos", pos);
        camera.addProperty("yaw", mc.player.getYRot());
        camera.addProperty("pitch", mc.player.getXRot());
        camera.addProperty("screen", mc.gui.screen() == null ? "playing" : mc.gui.screen().getClass().getSimpleName());
        camera.addProperty("spectator", mc.player.isSpectator());
        camera.addProperty("day_time", mc.level.getOverworldClockTime());
        camera.addProperty("raining", mc.level.isRaining());
        camera.addProperty("thundering", mc.level.isThundering());
        camera.addProperty("frozen", mc.level.tickRateManager().isFrozen());
        camera.addProperty("boss_bars", bossBars(mc));
        // The settings the frame is drawn with, as the game holds them (a preset can replace options.txt's values).
        JsonObject settings = new JsonObject();
        settings.addProperty("render_distance", mc.options.getEffectiveRenderDistance());
        settings.addProperty("simulation_distance", mc.options.simulationDistance().get());
        settings.addProperty("fov", mc.options.fov().get());
        camera.add("settings", settings);
        return camera;
    }

    /** Runs commands on the integrated server, as its console, recording each one's outcome and output. */
    private void run(Minecraft mc, JsonArray commands, String owner) {
        MinecraftServer server = mc.getSingleplayerServer();
        for (var element : commands) {
            String line = element.getAsString();
            pending.incrementAndGet();
            server.execute(() -> {
                JsonObject e = new JsonObject();
                e.addProperty("owner", owner);
                e.addProperty("command", line);
                JsonArray output = new JsonArray();
                CommandSource capture = new CommandSource() {
                    @Override public void sendSystemMessage(Component message) { output.add(message.getString()); }
                    @Override public boolean acceptsSuccess() { return true; }
                    @Override public boolean acceptsFailure() { return true; }
                    @Override public boolean shouldInformAdmins() { return false; }
                };
                boolean[] result = {false};
                int[] value = {0};
                var source = server.createCommandSourceStack().withSource(capture)
                    .withCallback((success, count) -> { result[0] |= success; value[0] = count; });
                try {
                    server.getCommands().performPrefixedCommand(source, line);
                } catch (RuntimeException error) {
                    output.add(error.toString());
                }
                e.addProperty("success", result[0]);
                e.addProperty("result", value[0]);
                e.add("output", output);
                outcomes.add(e);
                pending.decrementAndGet();
            });
        }
    }

    private void drainOutcomes() {
        for (JsonObject e; (e = outcomes.poll()) != null; ) event("command", e);
    }

    private void enter(Phase next, long now) {
        phase = next;
        phaseStart = now;
        lastLoaded = -1;
        lastChange = now;
    }

    /**
     * The player is in the view's dimension within 2 blocks of its position, no
     * screen is open, every chunk within the render distance minus one is on the
     * client, no section waits to be compiled, and the loaded chunk count has not
     * changed for a second.
     */
    private boolean ready(Minecraft mc, JsonObject v, long now) {
        if (now - readyCheck < 10_000_000L) return false;
        readyCheck = now;
        JsonArray p = v.getAsJsonArray("pos");
        Vec3 at = new Vec3(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble());
        if (mc.level == null || mc.player == null || mc.gui.screen() != null || mc.gui.overlay() != null
                || !mc.level.dimension().identifier().toString().equals(v.get("dimension").getAsString())
                || mc.player.position().distanceToSqr(at) > 4.0) {
            lastChange = now;
            lastLoaded = -1;
            return false;
        }
        int loaded = mc.level.getChunkSource().getLoadedChunksCount();
        if (loaded != lastLoaded || !mc.levelRenderer.hasRenderedAllSections() || !chunksAround(mc)) {
            lastLoaded = loaded;
            lastChange = now;
            return false;
        }
        return now - lastChange >= 1_000_000_000L;
    }

    /** After the setup (and its redraw): a view whose weather changes thaws the frozen world first. */
    private void afterSetup(Minecraft mc, long now) {
        JsonObject v = views.get(view);
        if (v.has("thaw") && v.get("thaw").getAsDouble() > 0) {
            run(mc, single("tick unfreeze"), v.get("name").getAsString());
            enter(Phase.UNFREEZE, now);
        } else {
            enter(Phase.SETTLE, now);
        }
    }

    private void refreeze(Minecraft mc, long now) {
        run(mc, single("tick freeze"), views.get(view).get("name").getAsString());
        enter(Phase.REFREEZE, now);
    }

    /** The rain and thunder levels are at the view's weather: 1 or 0 each (vanilla moves them 0.01 a tick). */
    private static boolean reached(Minecraft mc, JsonObject v) {
        String weather = v.has("weather") && !v.get("weather").isJsonNull() ? v.get("weather").getAsString() : "";
        float rain = weather.equals("rain") || weather.equals("thunder") ? 1f : 0f;
        float thunder = weather.equals("thunder") ? 1f : 0f;
        return Math.abs(mc.level.getRainLevel(1f) - rain) < 1e-3
            && Math.abs(mc.level.getThunderLevel(1f) - thunder) < 1e-3;
    }

    private void thawed(Minecraft mc, boolean thawed, boolean reached) {
        JsonObject e = named(views.get(view));
        e.addProperty("thawed", thawed);
        e.addProperty("reached", reached);
        e.addProperty("rain_level", mc.level.getRainLevel(1f));
        e.addProperty("thunder_level", mc.level.getThunderLevel(1f));
        event("thawed", e);
    }

    /** How many boss bars the client holds (the overlay keeps them in its private "events" map; -1 if unreadable). */
    private static int bossBars(Minecraft mc) {
        try {
            var field = mc.gui.hud.getBossOverlay().getClass().getDeclaredField("events");
            field.setAccessible(true);
            return ((java.util.Map<?, ?>) field.get(mc.gui.hud.getBossOverlay())).size();
        } catch (ReflectiveOperationException | RuntimeException error) {
            return -1;
        }
    }

    private static JsonArray single(String command) {
        JsonArray a = new JsonArray();
        a.add(command);
        return a;
    }

    /** The player is in the view's dimension, within 2 blocks of its position. */
    private static boolean at(Minecraft mc, JsonObject v) {
        JsonArray p = v.getAsJsonArray("pos");
        Vec3 want = new Vec3(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble());
        return mc.level != null && mc.player != null
            && mc.level.dimension().identifier().toString().equals(v.get("dimension").getAsString())
            && mc.player.position().distanceToSqr(want) <= 4.0;
    }

    private static boolean chunksAround(Minecraft mc) {
        int r = Math.max(1, mc.options.getEffectiveRenderDistance() - 1);
        var center = mc.player.chunkPosition();
        var chunks = mc.level.getChunkSource();
        for (int dx = -r; dx <= r; dx++) {
            for (int dz = -r; dz <= r; dz++) {
                if (dx * dx + dz * dz <= r * r && !chunks.hasChunk(center.x() + dx, center.z() + dz)) return false;
            }
        }
        return true;
    }

    private static JsonObject named(JsonObject v) {
        JsonObject e = new JsonObject();
        e.addProperty("view", v.get("name").getAsString());
        return e;
    }

    private static JsonObject renderer(Minecraft mc) {
        JsonObject e = new JsonObject();
        var info = RenderSystem.getDevice().getDeviceInfo();
        e.addProperty("gpu", info.name());
        e.addProperty("backend", info.backendName());
        e.addProperty("driver", info.driverInfo());
        JsonArray size = new JsonArray();
        size.add(mc.getWindow().getWidth());
        size.add(mc.getWindow().getHeight());
        e.add("size", size);
        return e;
    }

    private synchronized void event(String name, JsonObject e) {
        e.addProperty("event", name);
        e.addProperty("epoch_ms", System.currentTimeMillis());
        try {
            Files.writeString(events, e + "\n", StandardCharsets.UTF_8, StandardOpenOption.CREATE,
                StandardOpenOption.APPEND);
        } catch (IOException error) {
            throw new UncheckedIOException(error);
        }
    }
}
