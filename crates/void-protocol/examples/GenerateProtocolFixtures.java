// Independent golden fixtures produced by Mojang's actual 26.2 codecs, not Void.rs.
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.LpVec3;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.Strategy;
import net.minecraft.world.phys.Vec3;
import io.netty.buffer.Unpooled;
import java.nio.file.*;

public class GenerateProtocolFixtures {
    static void write(Path target, FriendlyByteBuf data) throws Exception {
        byte[] bytes = new byte[data.readableBytes()]; data.readBytes(bytes); Files.write(target, bytes); data.release();
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion(); Bootstrap.bootStrap();
        Path output = Path.of(args[0]); Files.createDirectories(output);
        PalettedContainer<BlockState> palette = new PalettedContainer<>(Blocks.AIR.defaultBlockState(), Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY));
        for (int z=0; z<16; z++) for (int x=0; x<16; x++) palette.set(x,0,z,Blocks.STONE.defaultBlockState());
        for (int z=0; z<16; z++) palette.set(3,1,z,Blocks.DIRT.defaultBlockState());
        FriendlyByteBuf section = new FriendlyByteBuf(Unpooled.buffer());
        section.writeShort(272); section.writeShort(0); palette.write(section);
        section.writeByte(0); section.writeVarInt(0); // single-valued biome palette
        FriendlyByteBuf chunk = new FriendlyByteBuf(Unpooled.buffer());
        chunk.writeInt(-4); chunk.writeInt(7); chunk.writeVarInt(0);
        chunk.writeVarInt(section.readableBytes()); chunk.writeBytes(section); section.release();
        chunk.writeVarInt(0); // block entities
        for (int i=0;i<6;i++) chunk.writeVarInt(0); // four bitsets, two light arrays
        write(output.resolve("26.2-chunk.bin"), chunk);
        FriendlyByteBuf vectors = new FriendlyByteBuf(Unpooled.buffer());
        for (Vec3 v : new Vec3[]{Vec3.ZERO,new Vec3(0.3,-0.5,0.7),new Vec3(8.0,-16.0,32.0),new Vec3(0.0,1.0,-1.0)}) LpVec3.write(vectors,v);
        write(output.resolve("26.2-lpvec3.bin"), vectors);
    }
}
