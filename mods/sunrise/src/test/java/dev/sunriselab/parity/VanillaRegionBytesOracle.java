package dev.sunriselab.parity;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtIo;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.chunk.storage.RegionFileStorage;
import net.minecraft.world.level.chunk.storage.RegionFileVersion;
import net.minecraft.world.level.chunk.storage.RegionStorageInfo;

/**
 * Anvil region files through RegionFileStorage, as a level's chunk storage opens them: each listed slot read
 * (absent, loaded with its NBT as NbtIo.write writes it, or the error), then every loaded chunk written back in
 * place and the storage closed, so tools/test_region_bytes_parity.py can compare the files byte for byte.
 *
 * Input lines: {@code <case directory> <region-file-compression> <slot>...}; arguments: root, inputs, output.
 */
public final class VanillaRegionBytesOracle {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Path root = Path.of(args[0]);
        var info = new RegionStorageInfo("parity", ResourceKey.create(Registries.DIMENSION, Identifier.withDefaultNamespace("overworld")), "chunk");
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2])))) {
            out.println("version " + SharedConstants.getCurrentVersion().name());
            for (String line : Files.readAllLines(Path.of(args[1]))) {
                String[] words = line.split(" ");
                out.println("== " + words[0]);
                RegionFileVersion.configure(words[1]);
                var storage = new RegionFileStorage(info, root.resolve(words[0]), false);
                List<ChunkPos> loaded = new ArrayList<>();
                List<CompoundTag> tags = new ArrayList<>();
                for (int i = 2; i < words.length; i++) {
                    int slot = Integer.parseInt(words[i]);
                    var pos = new ChunkPos(slot % 32, slot / 32);
                    CompoundTag tag;
                    try {
                        tag = storage.read(pos);
                    } catch (Exception e) {
                        // The class family is what a caller can tell apart; the text is logged, not compared.
                        System.err.println(words[0] + " slot " + slot + ": " + e);
                        out.println("slot " + slot + " error " + (e instanceof IOException ? "io" : e.getClass().getName()));
                        continue;
                    }
                    if (tag == null) {
                        out.println("slot " + slot + " absent");
                        continue;
                    }
                    var bytes = new ByteArrayOutputStream();
                    NbtIo.write(tag, new DataOutputStream(bytes));
                    out.println("slot " + slot + " loaded " + HexFormat.of().formatHex(bytes.toByteArray()));
                    loaded.add(pos);
                    tags.add(tag);
                }
                for (int i = 0; i < loaded.size(); i++) {
                    storage.write(loaded.get(i), tags.get(i));
                }
                storage.close();
            }
        }
    }
}
