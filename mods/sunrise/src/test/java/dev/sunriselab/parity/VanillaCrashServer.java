package dev.sunriselab.parity;

import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import net.minecraft.server.MinecraftServer;

/**
 * The vanilla dedicated server (`net.minecraft.server.Main`, same arguments), made to crash on request for
 * tools/test_dedicated_crash_save.py: once a file named `crash-now` appears in its folder, a task on the server
 * thread throws a `StackOverflowError`. `BlockableEventLoop.doRunTask` catches only exceptions, so the error leaves
 * the tick loop as a throw in a player's packet would, and `MinecraftServer.runServer` reports the crash and runs
 * `stopServer`. Needs `--add-opens java.base/java.lang=ALL-UNNAMED`: the server is found through the shutdown hook
 * `Main` registers for it.
 */
public final class VanillaCrashServer {
    private VanillaCrashServer() {}

    public static void main(String[] args) throws Exception {
        net.minecraft.server.Main.main(args);
        MinecraftServer server = started();
        Path trigger = Path.of("crash-now");
        Thread watch = new Thread(() -> {
            try {
                while (!Files.exists(trigger)) {
                    Thread.sleep(100);
                }
            } catch (InterruptedException e) {
                return;
            }
            server.execute(() -> {
                throw new StackOverflowError("crash fixture");
            });
        }, "Crash fixture");
        watch.setDaemon(true);
        watch.start();
    }

    /** The server `Main` started: its "Server Shutdown Thread" hook holds it. */
    private static MinecraftServer started() throws Exception {
        Field hooks = Class.forName("java.lang.ApplicationShutdownHooks").getDeclaredField("hooks");
        hooks.setAccessible(true);
        @SuppressWarnings("unchecked")
        Map<Thread, Thread> registered = (Map<Thread, Thread>) hooks.get(null);
        synchronized (Class.forName("java.lang.ApplicationShutdownHooks")) {
            for (Thread hook : registered.keySet()) {
                for (Field field : hook.getClass().getDeclaredFields()) {
                    if (MinecraftServer.class.isAssignableFrom(field.getType())) {
                        field.setAccessible(true);
                        return (MinecraftServer) field.get(hook);
                    }
                }
            }
        }
        throw new IllegalStateException("no server shutdown hook: the server did not start");
    }
}
