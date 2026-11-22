// Writes what vanilla defines in code about how blocks affect movement and mining: every block
// state's collision shape and outline shape (as boxes, in block units, with each shape's
// edges as `forAllEdges` gives them for drawing outlines), each block's friction,
// speed factor and jump factor where they differ from the defaults, and every block's destroy
// time and whether it drops only when mined with the right tool, and every state's fluid and
// which of its faces are sturdy, for swimming, and whether rain stops on it, and how every
// state affects light: its dampening and emission, and the faces of its occlusion shape as
// the light engine sees them (`LightEngine.getOcclusionShape`) and as faces are culled against
// them (`BlockState.getFaceOcclusionShape`, `Block.shouldRenderFace`). `cargo xtask gen-reports`
// runs it on the unpacked server with `java DumpPhysics.java OUT.json`, which needs a JDK.
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
import net.minecraft.core.Direction;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.IceBlock;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Heightmap;
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
        StringBuilder fluids = new StringBuilder();
        StringBuilder sturdy = new StringBuilder();
        StringBuilder light = new StringBuilder();
        StringBuilder occlusion = new StringBuilder();
        StringBuilder renderOcclusion = new StringBuilder();
        Map<String, Integer> occlusionIds = new HashMap<>();
        List<String> occlusionFaces = new ArrayList<>();
        for (int id = 0; id < Block.BLOCK_STATE_REGISTRY.size(); id++) {
            BlockState state = Block.BLOCK_STATE_REGISTRY.byId(id);
            if (id > 0) {
                states.append(',');
                outlines.append(',');
                fluids.append(',');
                sturdy.append(',');
                light.append(',');
                occlusion.append(',');
                renderOcclusion.append(',');
            }
            VoxelShape collision = state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
            states.append(shapeId(collision, shapeIds, shapes, edges));
            VoxelShape outline = state.getShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
            outlines.append(shapeId(outline, shapeIds, shapes, edges));
            // The fluid (tags aren't loaded here, so by type): 1 water or 2 lava, 4 a source, 8 falling, and the amount times 16.
            FluidState fluid = state.getFluidState();
            int type = fluid.getType().isSame(Fluids.WATER) ? 1 : fluid.getType().isSame(Fluids.LAVA) ? 2 : 0;
            fluids.append(type == 0 ? 0
                : type | (fluid.isSource() ? 4 : 0) | (fluid.getValue(FlowingFluid.FALLING) ? 8 : 0) | fluid.getAmount() << 4);
            // A bit per `Direction` (down, up, north, south, west, east) whose face is sturdy,
            // 64 for ice, which `FlowingFluid.isSolidFace` doesn't count, and 128 for states
            // the `MOTION_BLOCKING` heightmap counts, where rain and snow stop.
            int faces = state.getBlock() instanceof IceBlock ? 64 : 0;
            if (Heightmap.Types.MOTION_BLOCKING.isOpaque().test(state)) faces |= 128;
            for (Direction direction : Direction.values()) {
                if (state.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, direction)) {
                    faces |= 1 << direction.ordinal();
                }
            }
            sturdy.append(faces);
            // `getLightDampening`, `getLightEmission` times 16, and 256 for `useShapeForLightOcclusion`.
            light.append(state.getLightDampening() | state.getLightEmission() << 4
                | (state.useShapeForLightOcclusion() ? 256 : 0));
            // 0 for `LightEngine.isEmptyShape`, else 1 + an index into `occlusion_faces`.
            // `render_occlusion` is the same for every state that `canOcclude`, which is all
            // `getFaceOcclusionShape` gives faces for.
            int renderId = 0;
            if (state.canOcclude()) {
                StringBuilder key = new StringBuilder("[");
                for (Direction direction : Direction.values()) {
                    if (direction.ordinal() > 0) key.append(',');
                    key.append('"').append(faceMask(state.getFaceOcclusionShape(direction), direction.getAxis())).append('"');
                }
                renderId = occlusionIds.computeIfAbsent(key.append(']').toString(), k -> {
                    occlusionFaces.add(k);
                    return occlusionFaces.size();
                });
            }
            occlusion.append(state.useShapeForLightOcclusion() ? renderId : 0);
            renderOcclusion.append(renderId);
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
            out.println("  \"fluids\": [" + fluids + "],");
            out.println("  \"sturdy\": [" + sturdy + "],");
            out.println("  \"light\": [" + light + "],");
            out.println("  \"occlusion\": [" + occlusion + "],");
            out.println("  \"render_occlusion\": [" + renderOcclusion + "],");
            out.println("  \"occlusion_faces\": [\n    " + String.join(",\n    ", occlusionFaces) + "\n  ],");
            out.println("  \"blocks\": {\n    " + blocks + "\n  },");
            out.println("  \"mining\": {\n    " + mining + "\n  }");
            out.println("}");
        }
        System.out.println(shapes.size() + " collision and outline shapes for " + Block.BLOCK_STATE_REGISTRY.size() + " states");
    }

    /**
     * Which sixteenths of a face shape cover the face, as 64 hex digits: bit `v * 16 + u` of
     * four longs, where `u` and `v` are x and z across a y face, x and y across a z face, and
     * y and z across an x face. A face shape spans the whole block along its axis, so its
     * middle is sampled.
     */
    static String faceMask(VoxelShape face, Direction.Axis axis) {
        long[] bits = new long[4];
        List<AABB> boxes = face.toAabbs();
        for (int v = 0; v < 16; v++) {
            for (int u = 0; u < 16; u++) {
                double a = (u + 0.5) / 16, b = (v + 0.5) / 16;
                double x = axis == Direction.Axis.X ? 0.5 : a;
                double y = axis == Direction.Axis.Y ? 0.5 : axis == Direction.Axis.X ? a : b;
                double z = axis == Direction.Axis.Z ? 0.5 : b;
                for (AABB box : boxes) {
                    if (box.minX <= x && x < box.maxX && box.minY <= y && y < box.maxY && box.minZ <= z && z < box.maxZ) {
                        int bit = v * 16 + u;
                        bits[bit / 64] |= 1L << (bit % 64);
                        break;
                    }
                }
            }
        }
        return String.format("%016x%016x%016x%016x", bits[0], bits[1], bits[2], bits[3]);
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
