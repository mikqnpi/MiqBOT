package io.MiqBOT.mod;

import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.MinecraftClient;

import javax.net.ssl.SSLContext;
import java.nio.file.Path;

public final class MiqBOTClientMod implements ClientModInitializer {
    private WsBridgeClient bridge;

    @Override
    public void onInitializeClient() {
        try {
            Path configDir = FabricLoader.getInstance().getConfigDir();
            Config cfg = Config.load(configDir);

            SSLContext ssl = TlsUtil.buildMtlsContext(
                    cfg.clientP12Path,
                    cfg.clientP12Password,
                    cfg.trustP12Path,
                    cfg.trustP12Password
            );

            this.bridge = new WsBridgeClient(cfg.agentId, cfg.bridgeUrl, ssl);
            this.bridge.connect();

            ClientTickEvents.END_CLIENT_TICK.register(this::onEndClientTick);
            System.out.println("[MiqBOT] client mod initialized");
        } catch (Exception e) {
            System.out.println("[MiqBOT] initialization failed: " + e);
            e.printStackTrace();
        }
    }

    private void onEndClientTick(MinecraftClient client) {
        if (bridge == null || !bridge.isConnected()) {
            return;
        }

        TelemetryCollector.Telemetry t = TelemetryCollector.collect(client);
        bridge.updateTelemetry(t);
    }
}
