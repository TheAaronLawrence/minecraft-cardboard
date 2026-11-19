// Writes what vanilla defines in code about how blocks affect movement and mining: every block
// state's collision shape and outline shape (as boxes, in block units, with each shape's
// edges as `forAllEdges` gives them for drawing outlines), each block's friction,
// speed factor and jump factor where they differ from the defaults, and every block's destroy
// time and whether it drops only when mined with the right tool. `cargo xtask gen-reports` runs it on the
// unpacked server with `java DumpPhysics.java OUT.json`, which needs a JDK.
//
// Shapes are taken at the origin with an empty collision context, as for a block nothing is
// standing on; the few that depend on the entity (scaffolding, powder snow) or the position
// (bamboo's offset) come out as that default. Outlines come without the random offset some
// plants get, which the renderer leaves out too.

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
import net.minecraft.world.phys.shapes.VoxelShape;

public class DumpPhysics {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        Map<String, Integer> shapeIds = new HashMap<>();
        List<String> shapes = new ArrayList<>();
        List<String> edges = new ArrayList<>();
        StringBuilder states = new StringBuilder();
        StringBuilder outlines = new StringBuilder();
        for (int id = 0; id < Block.BLOCK_STATE_REGISTRY.size(); id++) {
            BlockState state = Block.BLOCK_STATE_REGISTRY.byId(id);
            if (id > 0) {
                states.append(',');
                outlines.append(',');
            }
            VoxelShape collision = state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
            states.append(shapeId(collision, shapeIds, shapes, edges));
            VoxelShape outline = state.getShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
            outlines.append(shapeId(outline, shapeIds, shapes, edges));
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

        StringBuilder mining = new StringBuilder();
        for (Block block : BuiltInRegistries.BLOCK) {
            BlockState state = block.defaultBlockState();
            if (mining.length() > 0) mining.append(",\n    ");
            mining.append('"').append(BuiltInRegistries.BLOCK.getKey(block)).append("\":[")
                .append(block.defaultDestroyTime()).append(',')
                .append(state.requiresCorrectToolForDrops()).append(']');
        }

        try (PrintWriter out = new PrintWriter(args[0], "UTF-8")) {
            out.println("{");
            out.println("  \"shapes\": [");
            for (int i = 0; i < shapes.size(); i++) {
                out.println("    " + shapes.get(i) + (i + 1 < shapes.size() ? "," : ""));
            }
            out.println("  ],");
            out.println("  \"edges\": [");
            for (int i = 0; i < edges.size(); i++) {
                out.println("    " + edges.get(i) + (i + 1 < edges.size() ? "," : ""));
            }
            out.println("  ],");
            out.println("  \"states\": [" + states + "],");
            out.println("  \"outlines\": [" + outlines + "],");
            out.println("  \"blocks\": {\n    " + blocks + "\n  },");
            out.println("  \"mining\": {\n    " + mining + "\n  }");
            out.println("}");
        }
        System.out.println(shapes.size() + " collision and outline shapes for " + Block.BLOCK_STATE_REGISTRY.size() + " states");
    }

    /** The shape's index in `shapes`, adding it the first time it's seen. */
    static int shapeId(
        VoxelShape shape, Map<String, Integer> shapeIds, List<String> shapes, List<String> edges
    ) {
        StringBuilder boxes = new StringBuilder("[");
        for (AABB b : shape.toAabbs()) {
            if (boxes.length() > 1) boxes.append(',');
            boxes.append('[').append(b.minX).append(',').append(b.minY).append(',').append(b.minZ)
                .append(',').append(b.maxX).append(',').append(b.maxY).append(',').append(b.maxZ)
                .append(']');
        }
        String key = boxes.append(']').toString();
        return shapeIds.computeIfAbsent(key, k -> {
            shapes.add(k);
            StringBuilder lines = new StringBuilder("[");
            shape.forAllEdges((x1, y1, z1, x2, y2, z2) -> {
                if (lines.length() > 1) lines.append(',');
                lines.append('[').append(x1).append(',').append(y1).append(',').append(z1)
                    .append(',').append(x2).append(',').append(y2).append(',').append(z2)
                    .append(']');
            });
            edges.add(lines.append(']').toString());
            return shapes.size() - 1;
        });
    }
}
