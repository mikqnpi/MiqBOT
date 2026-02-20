package io.MiqBOT.mod;

import io.MiqBOT.bridge.v1.ActionRequest;
import io.MiqBOT.bridge.v1.ActionStatus;
import io.MiqBOT.bridge.v1.ActionType;
import net.minecraft.client.MinecraftClient;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

public final class ActionExecutor {
    private static final int DEDUPE_MAX = 512;
    private static final long MIN_GUARD_TIMEOUT_MS = 250L;
    private static final long DEFAULT_GUARD_TIMEOUT_MS = 5000L;

    private final String agentId;
    private final WsBridgeClient bridge;
    private final ScheduledExecutorService scheduler;
    private final AtomicBoolean gotoInFlight = new AtomicBoolean(false);
    private final Map<String, Long> seenRequests = new LinkedHashMap<>(DEDUPE_MAX, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, Long> eldest) {
            return size() > DEDUPE_MAX;
        }
    };
    private volatile ScheduledFuture<?> gotoGuardTask;

    public ActionExecutor(String agentId, WsBridgeClient bridge) {
        this.agentId = agentId;
        this.bridge = bridge;
        this.scheduler = Executors.newSingleThreadScheduledExecutor(new ActionThreadFactory());
    }

    public void shutdown() {
        cancelGotoGuardTask();
        gotoInFlight.set(false);
        scheduler.shutdownNow();
    }

    public void handleAction(ActionRequest req) {
        String requestId = req.getRequestId().trim();
        if (requestId.isEmpty()) {
            return;
        }

        if (isDuplicate(requestId)) {
            bridge.sendActionAck(requestId, false, "duplicate request_id");
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_REJECTED, "duplicate request_id");
            return;
        }

        long nowUnixMs = System.currentTimeMillis();
        if (req.getExpiresAtUnixMs() > 0 && nowUnixMs > req.getExpiresAtUnixMs()) {
            bridge.sendActionAck(requestId, false, "request expired");
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_TIMEOUT, "request expired");
            return;
        }

        if (!req.getTargetAgentId().isEmpty() && !req.getTargetAgentId().equals(agentId)) {
            bridge.sendActionAck(requestId, false, "target_agent mismatch");
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_REJECTED, "target_agent mismatch");
            return;
        }

        ActionType type = req.getType();
        if (!isAllowlisted(type)) {
            bridge.sendActionAck(requestId, false, "action not allowlisted");
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_REJECTED, "action not allowlisted");
            return;
        }

        try {
            switch (type) {
                case ACTION_TYPE_STOP_ALL -> executeStopAll(requestId);
                case ACTION_TYPE_BARITONE_GOTO -> executeBaritoneGoto(requestId, req);
                default -> {
                    bridge.sendActionAck(requestId, false, "unsupported action type");
                    bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_REJECTED, "unsupported action type");
                }
            }
        } catch (RuntimeException ex) {
            gotoInFlight.set(false);
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_FAILED, "executor exception: " + ex);
        }
    }

    private static boolean isAllowlisted(ActionType type) {
        return type == ActionType.ACTION_TYPE_STOP_ALL || type == ActionType.ACTION_TYPE_BARITONE_GOTO;
    }

    private synchronized boolean isDuplicate(String requestId) {
        if (seenRequests.containsKey(requestId)) {
            return true;
        }
        seenRequests.put(requestId, System.currentTimeMillis());
        return false;
    }

    private void executeStopAll(String requestId) {
        MinecraftClient client = MinecraftClient.getInstance();
        if (client == null) {
            bridge.sendActionAck(requestId, false, "minecraft client unavailable");
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_FAILED, "minecraft client unavailable");
            return;
        }

        bridge.sendActionAck(requestId, true, "accepted");
        client.execute(() -> {
            cancelGotoGuardTask();
            gotoInFlight.set(false);
            releaseMovementKeys(client);
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_OK, "stop_all executed");
        });
    }

    private void executeBaritoneGoto(String requestId, ActionRequest req) {
        if (!req.hasBaritoneGoto()) {
            bridge.sendActionAck(requestId, false, "baritone_goto payload missing");
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_REJECTED, "baritone_goto payload missing");
            return;
        }
        if (!gotoInFlight.compareAndSet(false, true)) {
            bridge.sendActionAck(requestId, false, "goto already in progress");
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_REJECTED, "goto already in progress");
            return;
        }

        MinecraftClient client = MinecraftClient.getInstance();
        if (client == null) {
            gotoInFlight.set(false);
            bridge.sendActionAck(requestId, false, "minecraft client unavailable");
            bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_FAILED, "minecraft client unavailable");
            return;
        }

        var gotoReq = req.getBaritoneGoto();
        client.execute(() -> {
            if (client.player == null) {
                gotoInFlight.set(false);
                bridge.sendActionAck(requestId, false, "player is null");
                bridge.sendActionResult(requestId, ActionStatus.ACTION_STATUS_FAILED, "player is null");
                return;
            }

            double dx = gotoReq.getX() - client.player.getX();
            double dy = gotoReq.getY() - client.player.getY();
            double dz = gotoReq.getZ() - client.player.getZ();
            double distance = Math.sqrt(dx * dx + dy * dy + dz * dz);

            int maxDistance = gotoReq.getMaxDistance();
            if (maxDistance > 0 && distance > maxDistance) {
                gotoInFlight.set(false);
                bridge.sendActionAck(requestId, false, "goto max_distance exceeded");
                bridge.sendActionResult(
                        requestId,
                        ActionStatus.ACTION_STATUS_REJECTED,
                        "goto max_distance exceeded"
                );
                return;
            }

            bridge.sendActionAck(requestId, true, "accepted");

            long timeoutMs = normalizeGuardMs(gotoReq.getTimeoutMs());
            long stuckTimeoutMs = normalizeGuardMs(gotoReq.getStuckTimeoutMs());
            long guardMs = Math.min(timeoutMs, stuckTimeoutMs);
            guardMs = Math.max(guardMs, MIN_GUARD_TIMEOUT_MS);

            cancelGotoGuardTask();
            gotoGuardTask = scheduler.schedule(() -> client.execute(() -> {
                releaseMovementKeys(client);
                bridge.sendActionResult(
                        requestId,
                        ActionStatus.ACTION_STATUS_FAILED,
                        "baritone goto guard timeout (stuck/timeout). stop_all enforced"
                );
                gotoInFlight.set(false);
                clearGotoGuardTaskReference();
            }), guardMs, TimeUnit.MILLISECONDS);
        });
    }

    private static long normalizeGuardMs(long v) {
        if (v <= 0L) {
            return DEFAULT_GUARD_TIMEOUT_MS;
        }
        return Math.max(v, MIN_GUARD_TIMEOUT_MS);
    }

    private static void releaseMovementKeys(MinecraftClient client) {
        client.options.forwardKey.setPressed(false);
        client.options.backKey.setPressed(false);
        client.options.leftKey.setPressed(false);
        client.options.rightKey.setPressed(false);
    }

    private synchronized void cancelGotoGuardTask() {
        if (gotoGuardTask != null) {
            gotoGuardTask.cancel(false);
            gotoGuardTask = null;
        }
    }

    private synchronized void clearGotoGuardTaskReference() {
        gotoGuardTask = null;
    }

    private static final class ActionThreadFactory implements ThreadFactory {
        @Override
        public Thread newThread(Runnable r) {
            Thread t = new Thread(r, "MiqBOT-action-executor");
            t.setDaemon(true);
            return t;
        }
    }
}
