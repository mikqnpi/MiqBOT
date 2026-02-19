package io.MiqBOT.mod;

import io.MiqBOT.bridge.v1.Capability;
import io.MiqBOT.bridge.v1.Dimension;
import io.MiqBOT.bridge.v1.Envelope;
import io.MiqBOT.bridge.v1.Hello;
import io.MiqBOT.bridge.v1.PeerRole;
import io.MiqBOT.bridge.v1.TelemetryFrame;
import io.MiqBOT.bridge.v1.Vec3d;

import javax.net.ssl.SSLContext;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.WebSocket;
import java.nio.ByteBuffer;
import java.time.Duration;
import java.util.UUID;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.atomic.AtomicLong;

public final class WsBridgeClient implements WebSocket.Listener {
    private final String agentId;
    private final URI uri;
    private final HttpClient http;
    private final long startNano;

    private volatile WebSocket ws;

    private final AtomicLong seq = new AtomicLong(0);
    private final AtomicLong lastPeerSeq = new AtomicLong(0);
    private final AtomicLong stateVersion = new AtomicLong(0);

    private volatile String sessionId = UUID.randomUUID().toString();

    public WsBridgeClient(String agentId, String bridgeUrl, SSLContext sslContext) {
        this.agentId = agentId;
        this.uri = URI.create(bridgeUrl);
        this.http = HttpClient.newBuilder()
                .sslContext(sslContext)
                .connectTimeout(Duration.ofSeconds(5))
                .build();
        this.startNano = System.nanoTime();
    }

    public void connect() {
        this.http.newWebSocketBuilder()
                .buildAsync(this.uri, this)
                .thenAccept(webSocket -> {
                    this.ws = webSocket;
                    sendHello();
                })
                .exceptionally(ex -> {
                    System.out.println("[MiqBOT] WS connect failed: " + ex);
                    ex.printStackTrace();
                    return null;
                });
    }

    public boolean isConnected() {
        return this.ws != null;
    }

    public void sendTelemetry(TelemetryCollector.Telemetry t) {
        if (this.ws == null) {
            return;
        }

        long sv = stateVersion.incrementAndGet();

        TelemetryFrame tf = TelemetryFrame.newBuilder()
                .setStateVersion(sv)
                .setWorldTick(t.worldTick)
                .setDimension(mapDimensionEnum(t.dimension))
                .setPos(Vec3d.newBuilder().setX(t.x).setY(t.y).setZ(t.z).build())
                .setYawDeg(t.yawDeg)
                .setPitchDeg(t.pitchDeg)
                .setHp(t.hp)
                .setHunger(t.hunger)
                .setAir(t.air)
                .setIsSprinting(t.sprinting)
                .setIsSneaking(t.sneaking)
                .setIsOnGround(t.onGround)
                .build();

        Envelope env = baseEnvelopeBuilder()
                .setTelemetry(tf)
                .build();

        send(env);
    }

    private void sendHello() {
        Hello h = Hello.newBuilder()
                .setAgentId(this.agentId)
                .setRole(PeerRole.PEER_ROLE_GAME_CLIENT)
                .addCapabilities(Capability.CAP_TELEMETRY_V1)
                .addCapabilities(Capability.CAP_TIMESYNC_V1)
                .setClientVersion("MiqBOT-mod/0.1.0")
                .build();

        Envelope env = baseEnvelopeBuilder()
                .setHello(h)
                .build();

        send(env);
    }

    private Envelope.Builder baseEnvelopeBuilder() {
        long s = seq.incrementAndGet();
        return Envelope.newBuilder()
                .setProtocolVersion(1)
                .setSessionId(this.sessionId)
                .setSeq(s)
                .setAck(lastPeerSeq.get())
                .setMonoMs(monoMs())
                .setWallUnixMs(System.currentTimeMillis());
    }

    private void send(Envelope env) {
        if (this.ws == null) {
            return;
        }
        byte[] bytes = env.toByteArray();
        this.ws.sendBinary(ByteBuffer.wrap(bytes), true);
    }

    private long monoMs() {
        return (System.nanoTime() - startNano) / 1_000_000L;
    }

    private static Dimension mapDimensionEnum(int dim) {
        if (dim == 1) {
            return Dimension.DIMENSION_OVERWORLD;
        }
        if (dim == 2) {
            return Dimension.DIMENSION_NETHER;
        }
        if (dim == 3) {
            return Dimension.DIMENSION_END;
        }
        if (dim == 99) {
            return Dimension.DIMENSION_OTHER;
        }
        return Dimension.DIMENSION_UNSPECIFIED;
    }

    @Override
    public void onOpen(WebSocket webSocket) {
        WebSocket.Listener.super.onOpen(webSocket);
        webSocket.request(1);
        System.out.println("[MiqBOT] WS open: " + uri);
    }

    @Override
    public CompletionStage<?> onBinary(WebSocket webSocket, ByteBuffer data, boolean last) {
        byte[] bytes = new byte[data.remaining()];
        data.get(bytes);

        try {
            Envelope env = Envelope.parseFrom(bytes);
            lastPeerSeq.set(env.getSeq());

            if (env.hasHello()) {
                System.out.println("[MiqBOT] server hello: " + env.getHello().getClientVersion());
            }
        } catch (Exception e) {
            System.out.println("[MiqBOT] WS decode error: " + e);
            e.printStackTrace();
        }

        webSocket.request(1);
        return null;
    }

    @Override
    public CompletionStage<?> onText(WebSocket webSocket, CharSequence data, boolean last) {
        webSocket.request(1);
        return null;
    }

    @Override
    public void onError(WebSocket webSocket, Throwable error) {
        System.out.println("[MiqBOT] WS error: " + error);
        error.printStackTrace();
    }

    @Override
    public CompletionStage<?> onClose(WebSocket webSocket, int statusCode, String reason) {
        System.out.println("[MiqBOT] WS close: " + statusCode + " reason=" + reason);
        this.ws = null;
        return WebSocket.Listener.super.onClose(webSocket, statusCode, reason);
    }
}
