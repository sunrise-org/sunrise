package dev.sunriselab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.authlib.GameProfile;
import it.unimi.dsi.fastutil.longs.Long2ObjectLinkedOpenHashMap;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import net.minecraft.SharedConstants;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundChunkBatchFinishedPacket;
import net.minecraft.network.protocol.game.ClientboundChunkBatchStartPacket;
import net.minecraft.network.protocol.game.ClientboundForgetLevelChunkPacket;
import net.minecraft.network.protocol.game.ClientboundSetChunkCacheCenterPacket;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ChunkHolder;
import net.minecraft.server.level.ChunkMap;
import net.minecraft.server.level.ChunkResult;
import net.minecraft.server.level.ChunkTrackingView;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.server.level.PlayerMap;
import net.minecraft.server.level.ServerChunkCache;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.server.network.PlayerChunkSender;
import net.minecraft.server.network.ServerGamePacketListenerImpl;
import net.minecraft.util.debug.LevelDebugSynchronizers;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.LevelChunk;
import sun.misc.Unsafe;

/**
 * 26.3's chunk sending for one player, driven by a script with a controlled readiness schedule (#5719): which chunks
 * a server sends and forgets, in which order and in which batches, when chunks become ready in a given order, the
 * player moves and the client acknowledges.
 *
 * <p>Runs as the jar has it: ChunkMap.onChunkReadyToSend (a chunk becoming ready), ChunkMap.applyChunkTrackingView
 * with ChunkTrackingView.of and difference, ChunkMap.markChunkPendingToSend (both), ChunkMap.dropChunk,
 * ChunkMap.getChunkToSend and getVisibleChunkIfPresent, ChunkHolder.getChunkToSend and getTickingChunk,
 * ServerChunkCache.onChunkReadyToSend, PlayerMap, and all of PlayerChunkSender but sendChunk.
 *
 * <p>Rewritten: PlayerChunkSender.sendChunk ({@link ChunkSendAgent}): it records the chunk instead of building its
 * packet. Stubbed (subclasses, built with Unsafe.allocateInstance, their constructors never run): the connection
 * (send records the packet), the player (isAlive is true: a living player's health would need its entity data) and the
 * level's debug synchronizers (registerChunk and startTrackingChunk do nothing). Built with allocateInstance, with
 * only these fields set: ServerLevel (chunkSource, debugSynchronizers, Entity's level for the player), ServerChunkCache
 * (level, chunkMap, chunkHoldersToBroadcast), ChunkMap (level, visibleChunkMap, playerMap), each ChunkHolder (pos,
 * sendSync done, tickingChunkFuture done with the chunk once ready, the two light section filters empty), each
 * LevelChunk (chunkPos), the player (connection, chunkTrackingView, level, chunkPosition, its entity id through setId) and the connection
 * (chunkSender, player).
 *
 * <p>The script, a JSON file: {"memory": bool, "view_distance": r, "ready": [[x, z]...] (ready before the player
 * comes, no event), "join": [x, z], "ticks": [{"ready": [[x, z]...] (chunks becoming ready between ticks, in order:
 * onChunkReadyToSend), "unready": [[x, z]...] (no longer ready: unloaded), "packets": [{"ack": chunks per tick} |
 * {"move": [x, z]} | {"ready": [x, z]}...] (the tick's packets, in order: onChunkBatchReceivedByClient, a move to that
 * chunk applying the view, or a chunk becoming ready mid-tick, as a synchronous getChunk polls the executor), then the tick's sendNextChunks}...]}. The join is the first tick's first move. Prints, per tick,
 * "tick N", then every packet sent: "center X Z", "start", "chunk X Z", "batch N", "forget X Z".
 */
public final class VanillaChunkSendOracle {
    static final List<String> out = new ArrayList<>();
    static Unsafe unsafe;

    static final class KitConnection extends ServerGamePacketListenerImpl {
        KitConnection() {
            super((MinecraftServer) null, (Connection) null, (ServerPlayer) null, (CommonListenerCookie) null);
        }

        @Override
        public void send(Packet<?> packet) {
            if (packet instanceof ClientboundSetChunkCacheCenterPacket p) {
                out.add("center " + p.getX() + " " + p.getZ());
            } else if (packet == ClientboundChunkBatchStartPacket.INSTANCE) {
                out.add("start");
            } else if (packet instanceof ClientboundChunkBatchFinishedPacket p) {
                out.add("batch " + p.batchSize());
            } else if (packet instanceof ClientboundForgetLevelChunkPacket p) {
                out.add("forget " + p.pos().x() + " " + p.pos().z());
            } else {
                throw new AssertionError("unexpected packet " + packet);
            }
        }
    }

    static final class KitPlayer extends ServerPlayer {
        KitPlayer() {
            super((MinecraftServer) null, (ServerLevel) null, (GameProfile) null, (ClientInformation) null);
        }

        @Override
        public boolean isAlive() {
            return true;
        }
    }

    static final class KitDebug extends LevelDebugSynchronizers {
        KitDebug() {
            super((ServerLevel) null);
        }

        @Override
        public void registerChunk(LevelChunk chunk) {
        }

        @Override
        public void startTrackingChunk(ServerPlayer player, ChunkPos pos) {
        }
    }

    /** The rewritten PlayerChunkSender.sendChunk ({@link ChunkSendAgent}). */
    public static void sent(ServerGamePacketListenerImpl connection, LevelChunk chunk) {
        out.add("chunk " + chunk.getPos().x() + " " + chunk.getPos().z());
    }

    static Field field(Class<?> type, String name) {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
                // the superclass declares it
            }
        }
        throw new AssertionError(type.getName() + " has no field " + name);
    }

    static void set(Object owner, String name, Object value) throws Exception {
        field(owner.getClass(), name).set(owner, value);
    }

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception {
        return (T) unsafe.allocateInstance(type);
    }

    static Method method(Class<?> type, String name, Class<?>... args) throws Exception {
        Method m = type.getDeclaredMethod(name, args);
        m.setAccessible(true);
        return m;
    }

    static ChunkPos pos(JsonElement e) {
        JsonArray a = e.getAsJsonArray();
        return new ChunkPos(a.get(0).getAsInt(), a.get(1).getAsInt());
    }

    final ChunkMap map;
    final KitPlayer player;
    final PlayerChunkSender sender;
    final ServerLevel level;
    final Map<Long, ChunkHolder> holders = new HashMap<>();
    final Method onReady;
    final Method applyView;
    final int radius;

    VanillaChunkSendOracle(boolean memory, int radius) throws Exception {
        this.radius = radius;
        level = make(ServerLevel.class);
        ServerChunkCache cache = make(ServerChunkCache.class);
        map = make(ChunkMap.class);
        set(map, "level", level);
        set(map, "visibleChunkMap", new Long2ObjectLinkedOpenHashMap<ChunkHolder>());
        set(map, "playerMap", new PlayerMap());
        set(cache, "level", level);
        set(cache, "chunkMap", map);
        set(cache, "chunkHoldersToBroadcast", new HashSet<ChunkHolder>());
        set(level, "chunkSource", cache);
        set(level, "debugSynchronizers", make(KitDebug.class));
        player = make(KitPlayer.class);
        // PlayerMap hashes the player by its entity id, which its constructor would have assigned.
        player.setId(1);
        KitConnection connection = make(KitConnection.class);
        sender = new PlayerChunkSender(memory);
        set(connection, "chunkSender", sender);
        set(connection, "player", player);
        player.connection = connection;
        set(player, "chunkTrackingView", ChunkTrackingView.EMPTY);
        set(player, "level", level);
        onReady = method(ChunkMap.class, "onChunkReadyToSend", ChunkHolder.class, LevelChunk.class);
        applyView = method(ChunkMap.class, "applyChunkTrackingView", ServerPlayer.class, ChunkTrackingView.class);
    }

    /** The chunk's holder in the visible map, made the first time; its chunk not ready to send yet. */
    ChunkHolder holder(ChunkPos pos) throws Exception {
        ChunkHolder holder = holders.get(pos.pack());
        if (holder == null) {
            holder = make(ChunkHolder.class);
            set(holder, "pos", pos);
            set(holder, "sendSync", CompletableFuture.completedFuture(null));
            set(holder, "tickingChunkFuture", new CompletableFuture<ChunkResult<LevelChunk>>());
            set(holder, "blockChangedLightSectionFilter", new BitSet());
            set(holder, "skyChangedLightSectionFilter", new BitSet());
            holders.put(pos.pack(), holder);
            @SuppressWarnings("unchecked")
            Long2ObjectLinkedOpenHashMap<ChunkHolder> visible =
                (Long2ObjectLinkedOpenHashMap<ChunkHolder>) field(ChunkMap.class, "visibleChunkMap").get(map);
            visible.put(pos.pack(), holder);
        }
        return holder;
    }

    /** The holder's ticking future done with a chunk at `pos`: ready to send (getChunkToSend). */
    LevelChunk makeReady(ChunkPos pos) throws Exception {
        LevelChunk chunk = make(LevelChunk.class);
        field(ChunkAccess.class, "chunkPos").set(chunk, pos);
        set(holder(pos), "tickingChunkFuture", CompletableFuture.completedFuture(ChunkResult.of(chunk)));
        return chunk;
    }

    void move(ChunkPos to) throws Exception {
        field(Entity.class, "chunkPosition").set(player, to);
        applyView.invoke(map, player, ChunkTrackingView.of(to, radius));
    }

    void tick(JsonObject t) throws Exception {
        for (JsonElement e : t.has("unready") ? t.getAsJsonArray("unready") : new JsonArray()) {
            set(holder(pos(e)), "tickingChunkFuture", new CompletableFuture<ChunkResult<LevelChunk>>());
        }
        for (JsonElement e : t.has("ready") ? t.getAsJsonArray("ready") : new JsonArray()) {
            ChunkPos pos = pos(e);
            LevelChunk chunk = makeReady(pos);
            onReady.invoke(map, holder(pos), chunk);
        }
        for (JsonElement e : t.has("packets") ? t.getAsJsonArray("packets") : new JsonArray()) {
            JsonObject p = e.getAsJsonObject();
            if (p.has("ack")) {
                sender.onChunkBatchReceivedByClient(p.get("ack").getAsFloat());
            } else if (p.has("ready")) {
                // A synchronous getChunk while handling a packet (a /forceload add): the executor runs the
                // chunk's readiness then, against the view in effect.
                ChunkPos pos = pos(p.get("ready"));
                onReady.invoke(map, holder(pos), makeReady(pos));
            } else {
                move(pos(p.get("move")));
            }
        }
        sender.sendNextChunks(player);
    }

    public static void main(String[] args) throws Exception {
        Field f = Unsafe.class.getDeclaredField("theUnsafe");
        f.setAccessible(true);
        unsafe = (Unsafe) f.get(null);
        // Bootstrap wraps System.out in the game's logging stream: the lines go to the stream it had.
        java.io.PrintStream raw = System.out;
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        // The agent rewrites PlayerChunkSender as the JVM loads it: load it before asking.
        Class.forName("net.minecraft.server.network.PlayerChunkSender");
        if (!ChunkSendAgent.installed()) {
            throw new IllegalStateException("PlayerChunkSender.sendChunk was not rewritten (no -javaagent?)");
        }
        JsonObject script = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonObject();
        VanillaChunkSendOracle kit = new VanillaChunkSendOracle(script.get("memory").getAsBoolean(),
            script.get("view_distance").getAsInt());
        for (JsonElement e : script.getAsJsonArray("ready")) {
            kit.makeReady(pos(e));
        }
        ((PlayerMap) field(ChunkMap.class, "playerMap").get(kit.map)).addPlayer(kit.player, false);
        JsonArray ticks = script.getAsJsonArray("ticks");
        for (int i = 0; i < ticks.size(); i++) {
            out.add("tick " + i);
            if (i == 0) {
                kit.move(pos(script.get("join")));
            }
            kit.tick(ticks.get(i).getAsJsonObject());
        }
        out.forEach(raw::println);
    }
}
