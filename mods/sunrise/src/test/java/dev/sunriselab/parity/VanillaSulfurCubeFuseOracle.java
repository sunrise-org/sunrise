package dev.sunriselab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.syncher.SynchedEntityData;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.monster.cubemob.SulfurCube;

/**
 * Original oracle: a sulfur cube loaded from NBT on a detached ServerLevel, through Entity.load and
 * SulfurCube.readAdditionalSaveData, for each `fuse` (absent, signed boundaries, every numeric tag
 * type, a nonnumeric one) and each `Invulnerable` (absent, false, true). It prints what the load
 * leaves (fuse, MAX_FUSE, isInvulnerable, isPrimed), the save's `fuse` and `Invulnerable`, the
 * MAX_FUSE a late tracker's snapshot carries (getNonDefaultValues), the fuse after three
 * tickFuse calls and the MAX_FUSE those ticks send to existing trackers (packDirty), and what a
 * later load of `fuse:-1` sends them.
 */
public final class VanillaSulfurCubeFuseOracle {
    static final class Level extends ServerLevel {
        Level() { super(null, null, null, null, null, null, false, 0L, java.util.List.of(), false); }
        net.minecraft.server.MinecraftServer fake;
        @Override public net.minecraft.server.MinecraftServer getServer() { return fake; }
        net.minecraft.core.RegistryAccess registries;
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return registries; }
        @Override public net.minecraft.world.Difficulty getDifficulty() { return net.minecraft.world.Difficulty.NORMAL; }
        @Override public long getGameTime() { return 0L; }
        int next = 1;
        @Override public int getNextEntityId() { return next++; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
    }

    static final String[] FUSES = {null, "-2147483648", "-5", "-1", "0", "1", "40", "40s", "40b",
        "4294967336L", "-1.5f", "1.5d", "2147483647", "\"lit\""};
    static final String[] INVULNERABLE = {null, "0b", "1b"};

    static String maxFuse(List<SynchedEntityData.DataValue<?>> values) {
        if (values == null) return "-";
        for (var value : values) if (value.id() == 19) return String.valueOf(value.value());
        return "-";
    }

    static net.minecraft.nbt.CompoundTag tag(String fuse, String invulnerable) throws Exception {
        var snbt = new StringBuilder("{id:\"minecraft:sulfur_cube\",Pos:[0.5d,64d,0.5d],NoAI:1b");
        if (fuse != null) snbt.append(",fuse:").append(fuse);
        if (invulnerable != null) snbt.append(",Invulnerable:").append(invulnerable);
        return net.minecraft.nbt.TagParser.parseCompoundFully(snbt.append('}').toString());
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        var uf = u.getDeclaredField("theUnsafe");
        uf.setAccessible(true);
        Object unsafe = uf.get(null);
        Method allocate = u.getMethod("allocateInstance", Class.class);
        Level level = (Level) allocate.invoke(unsafe, Level.class);
        Object server = allocate.invoke(unsafe, net.minecraft.server.dedicated.DedicatedServer.class);
        Object subs = allocate.invoke(unsafe, net.minecraft.util.debug.ServerDebugSubscribers.class);
        var sf = net.minecraft.server.MinecraftServer.class.getDeclaredField("debugSubscribers");
        sf.setAccessible(true);
        sf.set(server, subs);
        level.fake = (net.minecraft.server.MinecraftServer) server;
        var repo = net.minecraft.server.packs.repository.ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(java.util.List.of("vanilla"));
        var resources = new net.minecraft.server.packs.resources.MultiPackResourceManager(net.minecraft.server.packs.PackType.SERVER_DATA, repo.openAllSelected());
        var layers = net.minecraft.server.RegistryLayer.createRegistryAccess();
        var staticTags = net.minecraft.tags.TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(net.minecraft.server.RegistryLayer.STATIC));
        var context = net.minecraft.tags.TagLoader.buildUpdatedLookups(layers.getAccessForLoading(net.minecraft.server.RegistryLayer.WORLD), staticTags);
        level.registries = net.minecraft.resources.RegistryDataLoader.load(resources, context, net.minecraft.resources.RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        staticTags.forEach(net.minecraft.core.Registry.PendingTags::apply);
        Method tickFuse = SulfurCube.class.getDeclaredMethod("tickFuse");
        tickFuse.setAccessible(true);
        var maxField = SulfurCube.class.getDeclaredField("MAX_FUSE");
        maxField.setAccessible(true);
        @SuppressWarnings("unchecked")
        var maxFuse = (net.minecraft.network.syncher.EntityDataAccessor<Integer>) maxField.get(null);
        var type = BuiltInRegistries.ENTITY_TYPE.get(net.minecraft.resources.Identifier.parse("minecraft:sulfur_cube")).get().value();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (String fuse : FUSES) for (String invulnerable : INVULNERABLE) {
                var cube = (SulfurCube) type.create(level, EntitySpawnReason.LOAD);
                cube.load(net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess(), tag(fuse, invulnerable)));
                int loadedFuse = cube.getFuse(), loadedMax = cube.getEntityData().get(maxFuse);
                boolean invulnerableLoaded = cube.isInvulnerable(), primed = cube.isPrimed();
                var saved = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess());
                cube.saveWithoutId(saved);
                var tag = saved.buildResult();
                String late = maxFuse(cube.getEntityData().getNonDefaultValues());
                cube.getEntityData().packDirty();
                for (int i = 0; i < 3; i++) tickFuse.invoke(cube);
                String ticked = maxFuse(cube.getEntityData().packDirty());
                int fuseAfter = cube.getFuse();
                cube.load(net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess(), tag("-1", null)));
                String back = maxFuse(cube.getEntityData().packDirty());
                out.printf(Locale.ROOT, "%s %s loaded %d %d %b %b saved %s %s late %s ticked %d %s back %s%n",
                    fuse == null ? "absent" : fuse, invulnerable == null ? "absent" : invulnerable,
                    loadedFuse, loadedMax, invulnerableLoaded, primed,
                    tag.contains("fuse") ? String.valueOf(tag.getIntOr("fuse", 0)) : "-",
                    tag.contains("Invulnerable") ? String.valueOf(tag.getByteOr("Invulnerable", (byte) 0)) : "-",
                    late, fuseAfter, ticked, back);
            }
        }
    }
}
