// Writes what vanilla defines in code about how blocks affect movement: every block state's
// collision shape (as boxes, in block units), and each block's friction, speed factor and jump
// factor where they differ from the defaults. `cargo xtask gen-reports` runs it on the
// unpacked server with `java DumpPhysics.java OUT.json`, which needs a JDK.
//
// Shapes are taken at the origin with an empty collision context, as for a block nothing is
// standing on; the few that depend on the entity (scaffolding, powder snow) or the position
// (bamboo's offset) come out as that default.

import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;

public class DumpPhysics {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        Map<String, Integer> shapeIds = new HashMap<>();
        List<String> shapes = new ArrayList<>();
        StringBuilder states = new StringBuilder();
        for (int id = 0; id < Block.BLOCK_STATE_REGISTRY.size(); id++) {
            BlockState state = Block.BLOCK_STATE_REGISTRY.byId(id);
            StringBuilder boxes = new StringBuilder("[");
            for (AABB b : state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO).toAabbs()) {
                if (boxes.length() > 1) boxes.append(',');
                boxes.append('[').append(b.minX).append(',').append(b.minY).append(',').append(b.minZ)
                    .append(',').append(b.maxX).append(',').append(b.maxY).append(',').append(b.maxZ)
                    .append(']');
            }
            String key = boxes.append(']').toString();
            Integer shape = shapeIds.get(key);
            if (shape == null) {
                shape = shapes.size();
                shapeIds.put(key, shape);
                shapes.add(key);
            }
            if (id > 0) states.append(',');
            states.append(shape);
        }

        StringBuilder blocks = new StringBuilder();
        for (Block block : BuiltInRegistries.BLOCK) {
            StringBuilder props = new StringBuilder();
            if (block.getFriction() != 0.6f) props.append(",\"friction\":").append(block.getFriction());
            if (block.getSpeedFactor() != 1.0f) props.append(",\"speed_factor\":").append(block.getSpeedFactor());
            if (block.getJumpFactor() != 1.0f) props.append(",\"jump_factor\":").append(block.getJumpFactor());
            if (props.length() == 0) continue;
            if (blocks.length() > 0) blocks.append(",\n    ");
            blocks.append('"').append(BuiltInRegistries.BLOCK.getKey(block)).append("\":{")
                .append(props.substring(1)).append('}');
        }

        try (PrintWriter out = new PrintWriter(args[0], "UTF-8")) {
            out.println("{");
            out.println("  \"shapes\": [");
            for (int i = 0; i < shapes.size(); i++) {
                out.println("    " + shapes.get(i) + (i + 1 < shapes.size() ? "," : ""));
            }
            out.println("  ],");
            out.println("  \"states\": [" + states + "],");
            out.println("  \"blocks\": {\n    " + blocks + "\n  }");
            out.println("}");
        }
        System.out.println(shapes.size() + " collision shapes for " + Block.BLOCK_STATE_REGISTRY.size() + " states");
    }
}
