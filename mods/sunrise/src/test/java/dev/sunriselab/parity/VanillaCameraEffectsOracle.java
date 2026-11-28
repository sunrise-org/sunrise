package dev.sunriselab.parity;

import com.google.common.collect.ImmutableList;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import io.netty.buffer.Unpooled;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.SharedConstants;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.core.Holder;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.PacketProcessor;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.game.ClientboundRemoveMobEffectPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateMobEffectPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.TickRateManager;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.entity.monster.zombie.Zombie;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.entity.EntityTickList;

/**
 * Vanilla 26.3's view of a spectated entity's effects, frame by frame (#4126). Each case gives a
 * fresh living entity on a client level and a list of steps: an effect packet decoded from its
 * wire bytes by the packet's own codec and handled by ClientPacketListener's own handler; a tick
 * in a running or frozen level, which runs the real ClientLevel.tickEntities over the level's
 * EntityTickList, so vanilla's own dispatch (removed, passenger, TickRateManager.isEntityFrozen)
 * decides whether the entity ticks; or the local player mounting or leaving it. The level's
 * tickNonPassenger is overridden only to run the entity's real LivingEntity.tickEffects (the
 * client tick's effect countdown and fade) in place of the whole entity tick.
 *
 * <p>After each step three values of the spectated entity are read at each partial tick, as
 * float bits: blindness, a direct hasEffect(BLINDNESS) query (1 or 0); night vision,
 * hasEffect(NIGHT_VISION) ? GameRenderer.nightVisionScale(entity, partial) : 0, the night vision
 * effect branch of LightmapRenderStateExtractor (which reads the local player and, without night
 * vision, takes the player's getWaterVision while it has CONDUIT_POWER; that fallback is not
 * applied here); and darkness,
 * getEffectBlendFactor(DARKNESS, partial) * the Darkness Effect option's floatValue, the product
 * LightmapRenderStateExtractor takes for the local player, applied here to the spectated entity.
 *
 * <p>Usage: VanillaCameraEffectsOracle input.json witness.json
 */
public final class VanillaCameraEffectsOracle {
    static Object unsafe;
    static Method allocate;

    /**
     * A client level that holds one entity under its id and in its EntityTickList; the rest of it
     * is never read here. tickEntities is vanilla's; only the entity's tick itself is replaced.
     */
    static final class OneEntityLevel extends ClientLevel {
        Entity entity;
        Method tickEffects;
        int ticks;

        OneEntityLevel() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        @Override
        public Entity getEntity(int id) {
            return entity != null && entity.getId() == id ? entity : null;
        }

        @Override
        public void tickNonPassenger(Entity ticked) {
            if (ticked != entity) throw new AssertionError("ticked an entity the level does not hold");
            ticks++;
            try {
                tickEffects.invoke(ticked);
            } catch (ReflectiveOperationException e) {
                throw new AssertionError(e);
            }
        }
    }

    static Field field(Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static <T> T make(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }

    static Holder<MobEffect> effect(String name) {
        return BuiltInRegistries.MOB_EFFECT.get(Identifier.parse(name)).orElseThrow();
    }

    static long bits(float value) {
        return Integer.toUnsignedLong(Float.floatToRawIntBits(value));
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        unsafe = field(unsafeClass, "theUnsafe").get(null);
        allocate = unsafeClass.getMethod("allocateInstance", Class.class);
        Method tickEffects = LivingEntity.class.getDeclaredMethod("tickEffects");
        tickEffects.setAccessible(true);
        Field passengers = field(Entity.class, "passengers");
        RegistryAccess registries = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);

        JsonObject input = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonObject();
        JsonArray cases = new JsonArray();
        for (var value : input.getAsJsonArray("cases")) {
            JsonObject c = value.getAsJsonObject().deepCopy();
            OneEntityLevel level = make(OneEntityLevel.class);
            field(Level.class, "isClientSide").setBoolean(level, true);
            Minecraft minecraft = make(Minecraft.class);
            field(Minecraft.class, "packetProcessor").set(minecraft, new PacketProcessor(Thread.currentThread()));
            ClientPacketListener listener = make(ClientPacketListener.class);
            field(ClientCommonPacketListenerImpl.class, "minecraft").set(listener, minecraft);
            field(ClientPacketListener.class, "level").set(listener, level);
            LivingEntity entity = switch (c.get("entity").getAsString()) {
                case "zombie" -> new Zombie(EntityTypes.ZOMBIE, level);
                case "armor_stand" -> new ArmorStand(EntityTypes.ARMOR_STAND, level);
                default -> throw new IllegalArgumentException(c.get("entity").getAsString());
            };
            entity.setId(c.get("id").getAsInt());
            level.entity = entity;
            level.tickEffects = tickEffects;
            EntityTickList ticking = new EntityTickList();
            ticking.add(entity);
            field(ClientLevel.class, "tickingEntities").set(level, ticking);
            // The local player, a passenger only while it rides the camera entity.
            LocalPlayer player = make(LocalPlayer.class);
            passengers.set(player, ImmutableList.of());
            TickRateManager rate = new TickRateManager();
            field(ClientLevel.class, "tickRateManager").set(level, rate);
            JsonArray ticked = new JsonArray();
            float scale = (float) c.get("darkness_scale").getAsDouble();
            JsonArray partials = c.getAsJsonArray("partials");
            JsonArray reads = new JsonArray();
            for (var stepValue : c.getAsJsonArray("steps")) {
                JsonObject step = stepValue.getAsJsonObject();
                switch (step.get("op").getAsString()) {
                    case "update" -> {
                        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), registries);
                        buf.writeVarInt(entity.getId());
                        MobEffect.STREAM_CODEC.encode(buf, effect(step.get("effect").getAsString()));
                        buf.writeVarInt(step.get("amplifier").getAsInt());
                        buf.writeVarInt(step.get("duration").getAsInt());
                        buf.writeByte(step.get("flags").getAsInt());
                        var packet = ClientboundUpdateMobEffectPacket.STREAM_CODEC.decode(buf);
                        if (buf.isReadable()) throw new AssertionError("bytes left after the packet");
                        listener.handleUpdateMobEffect(packet);
                    }
                    case "remove" -> {
                        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), registries);
                        buf.writeVarInt(entity.getId());
                        MobEffect.STREAM_CODEC.encode(buf, effect(step.get("effect").getAsString()));
                        var packet = ClientboundRemoveMobEffectPacket.STREAM_CODEC.decode(buf);
                        if (buf.isReadable()) throw new AssertionError("bytes left after the packet");
                        listener.handleRemoveMobEffect(packet);
                    }
                    case "tick" -> {
                        rate.setFrozen(step.get("frozen").getAsBoolean());
                        rate.tick();
                        if (rate.runsNormally() == step.get("frozen").getAsBoolean()) throw new AssertionError("tick rate state");
                        int before = level.ticks;
                        level.tickEntities();
                        ticked.add(level.ticks - before);
                    }
                    case "carry" -> {
                        boolean on = step.get("on").getAsBoolean();
                        passengers.set(entity, on ? ImmutableList.of(player) : ImmutableList.of());
                    }
                    default -> throw new IllegalArgumentException(step.toString());
                }
                JsonArray frames = new JsonArray();
                for (var partialValue : partials) {
                    float partial = Float.intBitsToFloat((int) partialValue.getAsLong());
                    JsonArray frame = new JsonArray();
                    frame.add(bits(entity.hasEffect(MobEffects.BLINDNESS) ? 1.0f : 0.0f));
                    frame.add(bits(entity.hasEffect(MobEffects.NIGHT_VISION) ? GameRenderer.nightVisionScale(entity, partial) : 0.0f));
                    frame.add(bits(entity.getEffectBlendFactor(MobEffects.DARKNESS, partial) * scale));
                    frames.add(frame);
                }
                reads.add(frames);
            }
            c.add("reads", reads);
            // How many times tickEntities ticked the entity at each tick step (evidence only).
            c.add("ticked", ticked);
            cases.add(c);
        }
        JsonObject witness = new JsonObject();
        witness.addProperty("minecraft", SharedConstants.getCurrentVersion().name());
        witness.addProperty("oracle", "dev.sunriselab.parity.VanillaCameraEffectsOracle");
        witness.add("cases", cases);
        Gson gson = new GsonBuilder().disableHtmlEscaping().create();
        Files.writeString(Path.of(args[1]), gson.toJson(witness), StandardCharsets.UTF_8);
    }
}
