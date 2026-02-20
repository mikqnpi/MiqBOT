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
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;

public final class WsBridgeClient implements WebSocket.Listener {
    private static final long SEND_INTERVAL_MS = 100L; // 10Hz latest-only sender

    private final String agentId;
    private final URI uri;
    private final HttpClient http;
    private final long startNano;

    private final AtomicLong seq = new AtomicLong(0);
    private final AtomicLong lastPeerSeq = new AtomicLong(0);
    private final AtomicLong stateVersion = new AtomicLong(0);
    private final AtomicReference<TelemetrySnapshot> latestSnapshot = new AtomicReference<>(null);
    private final AtomicBoolean senderStarted = new AtomicBoolean(false);
    private final AtomicBoolean sendInFlight = new AtomicBoolean(false);

    private final ScheduledExecutorService senderExecutor;

    private volatile WebSocket ws;
    private volatile String sessionId = UUID.randomUUID().toString();
    private final String handshakeId = UUID.randomUUID().toString();

    private static final class TelemetrySnapshot {
        final TelemetryCollector.Telemetry telemetry;
        final long stateVersion;

        TelemetrySnapshot(TelemetryCollector.Telemetry telemetry, long stateVersion) {
            this.telemetry = telemetry;
            this.stateVersion = stateVersion;
        }
    }

    public WsBridgeClient(String agentId, String bridgeUrl, SSLContext sslContext) {
        this.agentId = agentId;
        this.uri = URI.create(bridgeUrl);
        this.http = HttpClient.newBuilder()
                .sslContext(sslContext)
                .connectTimeout(Duration.ofSeconds(5))
                .build();
        this.startNano = System.nanoTime();
        this.senderExecutor = Executors.newSingleThreadScheduledExecutor(new SenderThreadFactory());
    }

    public void connect() {
        this.http.newWebSocketBuilder()
                .buildAsync(this.uri, this)
                .thenAccept(webSocket -> {
                    this.ws = webSocket;
                    startSenderLoop();
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

    public void updateTelemetry(TelemetryCollector.Telemetry t) {
        long version = stateVersion.incrementAndGet();
        TelemetryCollector.Telemetry copy = copyTelemetry(t);
        latestSnapshot.set(new TelemetrySnapshot(copy, version));
    }

    private void startSenderLoop() {
        if (!senderStarted.compareAndSet(false, true)) {
            return;
        }

        senderExecutor.scheduleAtFixedRate(() -> {
            try {
                sendLatestTelemetry();
            } catch (Exception e) {
                System.out.println("[MiqBOT] sender loop error: " + e);
                e.printStackTrace();
            }
        }, SEND_INTERVAL_MS, SEND_INTERVAL_MS, TimeUnit.MILLISECONDS);
    }

    private void sendLatestTelemetry() {
        WebSocket webSocket = this.ws;
        if (webSocket == null) {
            return;
        }
        if (sendInFlight.get()) {
            return;
        }

        TelemetrySnapshot snapshot = latestSnapshot.getAndSet(null);
        if (snapshot == null) {
            return;
        }
        if (!sendInFlight.compareAndSet(false, true)) {
            latestSnapshot.compareAndSet(null, snapshot);
            return;
        }

        TelemetryCollector.Telemetry t = snapshot.telemetry;
        TelemetryFrame tf = TelemetryFrame.newBuilder()
                .setStateVersion(snapshot.stateVersion)
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

        sendTelemetry(webSocket, env);
    }

    private void sendHello() {
        Hello h = Hello.newBuilder()
                .setAgentId(this.agentId)
                .setRole(PeerRole.PEER_ROLE_GAME_CLIENT)
                .addCapabilities(Capability.CAP_TELEMETRY_V1)
                .addCapabilities(Capability.CAP_TIMESYNC_V1)
                .addCapabilities(Capability.CAP_HELLO_ACK_V1)
                .setClientVersion("MiqBOT-mod/0.2.0")
                .setHandshakeId(this.handshakeId)
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
        WebSocket webSocket = this.ws;
        if (webSocket == null) {
            return;
        }

        try {
            byte[] bytes = env.toByteArray();
            webSocket.sendBinary(ByteBuffer.wrap(bytes), true);
        } catch (Exception e) {
            System.out.println("[MiqBOT] WS send error: " + e);
            e.printStackTrace();
        }
    }

    private void sendTelemetry(WebSocket webSocket, Envelope env) {
        try {
            byte[] bytes = env.toByteArray();
            webSocket.sendBinary(ByteBuffer.wrap(bytes), true)
                    .whenComplete((ignored, error) -> {
                        sendInFlight.set(false);
                        if (error != null) {
                            System.out.println("[MiqBOT] WS telemetry send error: " + error);
                            error.printStackTrace();
                        }
                    });
        } catch (Exception e) {
            sendInFlight.set(false);
            System.out.println("[MiqBOT] WS telemetry send exception: " + e);
            e.printStackTrace();
        }
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

    private static TelemetryCollector.Telemetry copyTelemetry(TelemetryCollector.Telemetry t) {
        TelemetryCollector.Telemetry copy = new TelemetryCollector.Telemetry();
        copy.worldTick = t.worldTick;
        copy.dimension = t.dimension;
        copy.x = t.x;
        copy.y = t.y;
        copy.z = t.z;
        copy.yawDeg = t.yawDeg;
        copy.pitchDeg = t.pitchDeg;
        copy.hp = t.hp;
        copy.hunger = t.hunger;
        copy.air = t.air;
        copy.sprinting = t.sprinting;
        copy.sneaking = t.sneaking;
        copy.onGround = t.onGround;
        return copy;
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

            if (env.hasHelloAck()) {
                var ack = env.getHelloAck();
                System.out.println("[MiqBOT] hello_ack accepted=" + ack.getAccepted() + " reason=" + ack.getReason());
            } else if (env.hasHello()) {
                System.out.println("[MiqBOT] server hello (legacy): " + env.getHello().getClientVersion());
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
        sendInFlight.set(false);
        return WebSocket.Listener.super.onClose(webSocket, statusCode, reason);
    }

    private static final class SenderThreadFactory implements ThreadFactory {
        @Override
        public Thread newThread(Runnable r) {
            Thread t = new Thread(r, "MiqBOT-telemetry-sender");
            t.setDaemon(true);
            return t;
        }
    }
}
