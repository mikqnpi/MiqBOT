package io.MiqBOT.mod;

import net.minecraft.client.MinecraftClient;
import net.minecraft.entity.player.PlayerEntity;
import net.minecraft.registry.RegistryKey;
import net.minecraft.world.World;

public final class TelemetryCollector {
    private TelemetryCollector() {}

    public static class Telemetry {
        public long worldTick;
        public int dimension;
        public double x;
        public double y;
        public double z;
        public double yawDeg;
        public double pitchDeg;
        public int hp;
        public int hunger;
        public int air;
        public boolean sprinting;
        public boolean sneaking;
        public boolean onGround;
    }

    public static Telemetry collect(MinecraftClient client) {
        Telemetry t = new Telemetry();
        if (client.player == null || client.world == null) {
            return t;
        }

        PlayerEntity p = client.player;

        t.worldTick = client.world.getTime();
        t.dimension = mapDimension(client.world.getRegistryKey());

        t.x = p.getX();
        t.y = p.getY();
        t.z = p.getZ();

        t.yawDeg = p.getYaw();
        t.pitchDeg = p.getPitch();

        t.hp = clampInt(Math.round(p.getHealth()), 0, 20);
        t.hunger = clampInt(p.getHungerManager().getFoodLevel(), 0, 20);
        t.air = clampInt(p.getAir(), 0, 300);

        t.sprinting = p.isSprinting();
        t.sneaking = p.isSneaking();
        t.onGround = p.isOnGround();

        return t;
    }

    private static int clampInt(int v, int lo, int hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    private static int mapDimension(RegistryKey<World> k) {
        if (k == World.OVERWORLD) {
            return 1;
        }
        if (k == World.NETHER) {
            return 2;
        }
        if (k == World.END) {
            return 3;
        }
        return 99;
    }
}
