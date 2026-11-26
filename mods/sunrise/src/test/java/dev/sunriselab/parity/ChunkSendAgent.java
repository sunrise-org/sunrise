package dev.sunriselab.parity;

import java.lang.classfile.ClassFile;
import java.lang.classfile.MethodModel;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.security.ProtectionDomain;

/**
 * Test-only Java agent for {@link VanillaChunkSendOracle} (tools/test_chunk_send_kit_parity.py builds it). It rewrites
 * one method, PlayerChunkSender.sendChunk, the only one on the send path that needs a real level: it builds the
 * chunk's ClientboundLevelChunkWithLightPacket from the level's light engine and starts the debug tracking. The
 * rewritten body hands the connection and the chunk to {@link VanillaChunkSendOracle#sent} instead, which records
 * the chunk's position. What is sent, in which order and in which batch stays 26.3's own code: everything else in
 * PlayerChunkSender (sendNextChunks, collectChunksToSend, dropChunk, markChunkPendingToSend,
 * onChunkBatchReceivedByClient) runs as the jar has it.
 */
public final class ChunkSendAgent {
    private static final String TARGET = "net/minecraft/server/network/PlayerChunkSender";
    private static final String DESCRIPTOR = "(Lnet/minecraft/server/network/ServerGamePacketListenerImpl;"
        + "Lnet/minecraft/server/level/ServerLevel;Lnet/minecraft/world/level/chunk/LevelChunk;)V";
    private static volatile boolean installed;

    public static void premain(String options, Instrumentation instrumentation) {
        instrumentation.addTransformer(new ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String name, Class<?> redefined, ProtectionDomain domain, byte[] bytes) {
                if (!TARGET.equals(name)) {
                    return null;
                }
                try {
                    return rewrite(bytes);
                } catch (Throwable error) {
                    // The JVM ignores a transformer's exceptions: stop rather than send real chunk packets.
                    error.printStackTrace();
                    Runtime.getRuntime().halt(3);
                    return null;
                }
            }
        });
    }

    private static byte[] rewrite(byte[] bytes) {
        ClassFile files = ClassFile.of();
        boolean[] found = {false};
        ClassDesc connection = ClassDesc.of("net.minecraft.server.network.ServerGamePacketListenerImpl");
        ClassDesc chunk = ClassDesc.of("net.minecraft.world.level.chunk.LevelChunk");
        byte[] rewritten = files.transformClass(files.parse(bytes), (builder, element) -> {
            if (element instanceof MethodModel method && method.methodName().equalsString("sendChunk")
                    && method.methodType().equalsString(DESCRIPTOR)) {
                found[0] = true;
                // static: slot 0 the connection, 1 the level, 2 the chunk.
                builder.withMethodBody(method.methodName(), method.methodType(), method.flags().flagsMask(), code -> code
                    .aload(0)
                    .aload(2)
                    .invokestatic(ClassDesc.of("dev.sunriselab.parity.VanillaChunkSendOracle"), "sent",
                        MethodTypeDesc.of(ConstantDescs.CD_void, connection, chunk))
                    .return_());
            } else {
                builder.with(element);
            }
        });
        if (!found[0]) {
            throw new IllegalStateException("PlayerChunkSender has no sendChunk" + DESCRIPTOR);
        }
        installed = true;
        return rewritten;
    }

    /** Whether PlayerChunkSender was rewritten (the oracle refuses to run without it). */
    public static boolean installed() {
        return installed;
    }
}
