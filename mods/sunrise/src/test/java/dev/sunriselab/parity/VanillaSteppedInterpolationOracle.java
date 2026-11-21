package dev.sunriselab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.world.TickRateManager;
import net.minecraft.world.entity.AbstractInterpolationHandler;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.InterpolationHandler;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.PositionPath;
import net.minecraft.world.entity.PositionStep;
import net.minecraft.world.entity.SteppedInterpolationHandler;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.entity.EntityInLevelCallback;
import net.minecraft.world.phys.Vec3;

/**
 * How a client walks a living entity along the positions its server sends: a real pig in a client
 * level with its real {@code SteppedInterpolationHandler}, moved by {@code Entity.moveOrInterpolateTo}
 * on each move packet and by the handler's {@code interpolate} each tick. Both place it through the
 * entity's own {@code setPos} and {@code setRot}, so {@code setYRot} and {@code setXRot} wrap, clamp
 * and discard non-finite angles. The entity is allocated without its constructor and given only the
 * fields those calls read; nothing else moves it (no riding, no physics), at the normal tick rate.
 *
 * <p>Input (args[0]), one event a line: {@code case STEPS X Y Z YAW PITCH} (a fresh entity there
 * with a handler of that many interpolation steps), {@code move L X Y Z} or {@code move S N
 * (X Y Z TICKS)xN} or {@code move N} (no position), then {@code R YAW PITCH} or {@code -} (no
 * rotation), and {@code tick}. Output (args[1]): the entity after every event.
 */
public final class VanillaSteppedInterpolationOracle {
    /** Set when the handler starts walking to a packet ({@code onInterpolationStart}). */
    static boolean started;

    static final class Mob extends Pig {
        Mob() {
            super(EntityTypes.PIG, null);
        }

        @Override
        public void onInterpolationStart(InterpolationHandler interpolation) {
            started = true;
        }

        // No rider controls it, so this client doesn't simulate its movement.
        @Override
        public LivingEntity getControllingPassenger() {
            return null;
        }
    }

    private static Field field(Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    private static String hex(double v) {
        return Long.toHexString(Double.doubleToRawLongBits(v));
    }

    private static String hex(float v) {
        return Integer.toHexString(Float.floatToRawIntBits(v));
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Class<?> u = Class.forName("sun.misc.Unsafe");
        Object unsafe = field(u, "theUnsafe").get(null);
        var allocate = u.getMethod("allocateInstance", Class.class);
        ClientLevel level = (ClientLevel) allocate.invoke(unsafe, ClientLevel.class);
        field(Level.class, "isClientSide").setBoolean(level, true);
        field(ClientLevel.class, "tickRateManager").set(level, new TickRateManager());
        Constructor<SteppedInterpolationHandler> handler = SteppedInterpolationHandler.class.getDeclaredConstructor(Entity.class);
        handler.setAccessible(true);
        Field interpolationSteps = field(AbstractInterpolationHandler.class, "interpolationSteps");
        Field interpolation = field(Entity.class, "interpolationHandler");
        Field type = field(Entity.class, "type");
        Field entityLevel = field(Entity.class, "level");
        Field position = field(Entity.class, "position");
        Field blockPosition = field(Entity.class, "blockPosition");
        Field chunkPosition = field(Entity.class, "chunkPosition");
        Field delta = field(Entity.class, "deltaMovement");
        Field yRot = field(Entity.class, "yRot");
        Field xRot = field(Entity.class, "xRot");
        Field callback = field(Entity.class, "levelCallback");
        Field dimensions = field(Entity.class, "dimensions");
        Field equipment = field(LivingEntity.class, "equipment");

        Entity entity = null;
        int steps = 0;
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                String[] f = line.trim().split(" ");
                int i = 1;
                String what;
                switch (f[0]) {
                    case "case" -> {
                        EntityType<?> kind = EntityTypes.PIG;
                        entity = (Entity) allocate.invoke(unsafe, Mob.class);
                        Vec3 at = new Vec3(Double.parseDouble(f[2]), Double.parseDouble(f[3]), Double.parseDouble(f[4]));
                        type.set(entity, kind);
                        entityLevel.set(entity, level);
                        position.set(entity, at);
                        blockPosition.set(entity, BlockPos.containing(at));
                        chunkPosition.set(entity, ChunkPos.containing(BlockPos.containing(at)));
                        delta.set(entity, Vec3.ZERO);
                        yRot.setFloat(entity, Float.parseFloat(f[5]));
                        xRot.setFloat(entity, Float.parseFloat(f[6]));
                        callback.set(entity, EntityInLevelCallback.NULL);
                        dimensions.set(entity, kind.getDimensions());
                        equipment.set(entity, new EntityEquipment());
                        steps = Integer.parseInt(f[1]);
                        SteppedInterpolationHandler h = handler.newInstance(entity);
                        interpolationSteps.setInt(h, steps);
                        interpolation.set(entity, h);
                        what = "c";
                    }
                    case "move" -> {
                        PositionPath path = null;
                        switch (f[i++]) {
                            case "L" -> {
                                path = PositionPath.of(new Vec3(Double.parseDouble(f[i]), Double.parseDouble(f[i + 1]), Double.parseDouble(f[i + 2])));
                                i += 3;
                            }
                            case "S" -> {
                                int n = Integer.parseInt(f[i++]);
                                List<PositionStep> list = new ArrayList<>();
                                for (int k = 0; k < n; k++) {
                                    list.add(new PositionStep(new Vec3(Double.parseDouble(f[i]), Double.parseDouble(f[i + 1]), Double.parseDouble(f[i + 2])), Integer.parseInt(f[i + 3])));
                                    i += 4;
                                }
                                path = PositionPath.stepped(List.copyOf(list));
                            }
                            default -> {
                            }
                        }
                        boolean hasRotation = f[i].equals("R");
                        float y = hasRotation ? Float.parseFloat(f[i + 1]) : 0;
                        float x = hasRotation ? Float.parseFloat(f[i + 2]) : 0;
                        started = false;
                        entity.moveOrInterpolateTo(path, y, x, hasRotation);
                        what = steps == 0 ? "s" : started ? "i" : "k";
                    }
                    case "tick" -> {
                        what = entity.isInterpolating() ? "t" : "n";
                        entity.getInterpolation().interpolate();
                    }
                    default -> throw new IllegalArgumentException(line);
                }
                Vec3 pos = entity.position();
                out.println(what + " " + hex(pos.x) + " " + hex(pos.y) + " " + hex(pos.z) + " " + hex(entity.getYRot()) + " " + hex(entity.getXRot()));
            }
        }
    }
}
