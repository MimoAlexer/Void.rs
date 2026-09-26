// Run against Mojang's locally obtained 26.2 jar. Writes factual IDs/shape flags only.
// See docs/PROTOCOL.md for the classpath setup; does not start a Minecraft server.
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;

public class GenerateBlockStates {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        try (PrintWriter out = new PrintWriter(args[0], StandardCharsets.UTF_8)) {
            out.println("state_id\tblock\tair\tfull_collision\tcan_occlude\tfluid\tfriction\tspeed_factor\tjump_factor\tboxes");
            for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
                String boxes = state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)
                    .toAabbs().stream().map(b -> String.format(java.util.Locale.ROOT,
                        "[%s, %s, %s, %s, %s, %s]", b.minX, b.minY, b.minZ, b.maxX, b.maxY, b.maxZ))
                    .collect(java.util.stream.Collectors.joining(", "));
                out.printf(java.util.Locale.ROOT, "%d\t%s\t%b\t%b\t%b\t%b\t%s\t%s\t%s\t%s%n", Block.getId(state),
                    BuiltInRegistries.BLOCK.getKey(state.getBlock()), state.isAir(),
                    state.isCollisionShapeFullBlock(EmptyBlockGetter.INSTANCE, BlockPos.ZERO),
                    state.canOcclude(), !state.getFluidState().isEmpty(), state.getBlock().getFriction(),
                    state.getBlock().getSpeedFactor(), state.getBlock().getJumpFactor(), boxes);
            }
        }
    }
}
