use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_rustls::server::TlsStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{accept_async_with_config, tungstenite::protocol::Message as WsMessage};
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::RelayConfig;
use crate::pb::bridge_v1 as pb;

const PROTOCOL_VERSION: u32 = 1;
const SERVER_NEGOTIABLE_CAPABILITIES: [i32; 3] = [
    pb::Capability::CapTelemetryV1 as i32,
    pb::Capability::CapTimesyncV1 as i32,
    pb::Capability::CapHelloAckV1 as i32,
];

#[derive(Debug)]
pub enum OrchestratorAcquireError {
    NotAllowed,
    LimitReached,
}

pub struct RelayHub {
    relay_cfg: RelayConfig,
    telemetry_tx: watch::Sender<Option<pb::TelemetryFrame>>,
    orchestrator_count: AtomicUsize,
    last_relay_mono_ms: AtomicU64,
}

impl RelayHub {
    pub fn new(relay_cfg: RelayConfig) -> Arc<Self> {
        let (telemetry_tx, _rx) = watch::channel(None);
        Arc::new(Self {
            relay_cfg,
            telemetry_tx,
            orchestrator_count: AtomicUsize::new(0),
            last_relay_mono_ms: AtomicU64::new(0),
        })
    }

    pub fn subscribe_telemetry(&self) -> watch::Receiver<Option<pb::TelemetryFrame>> {
        self.telemetry_tx.subscribe()
    }

    pub fn publish_telemetry(&self, telemetry: &pb::TelemetryFrame) {
        if self.relay_cfg.min_relay_interval_ms > 0 {
            let now = mono_ms();
            let last = self.last_relay_mono_ms.load(Ordering::Relaxed);
            if now.saturating_sub(last) < self.relay_cfg.min_relay_interval_ms {
                return;
            }
            self.last_relay_mono_ms.store(now, Ordering::Relaxed);
        }

        self.telemetry_tx.send_replace(Some(telemetry.clone()));
    }

    pub fn acquire_orchestrator_slot(
        self: &Arc<Self>,
    ) -> std::result::Result<OrchestratorSlot, OrchestratorAcquireError> {
        if !self.relay_cfg.allow_orchestrator_subscribe {
            return Err(OrchestratorAcquireError::NotAllowed);
        }

        loop {
            let current = self.orchestrator_count.load(Ordering::Relaxed);
            if current >= self.relay_cfg.max_orchestrator_subscribers {
                return Err(OrchestratorAcquireError::LimitReached);
            }

            if self
                .orchestrator_count
                .compare_exchange(
                    current,
                    current + 1,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Ok(OrchestratorSlot {
                    hub: Arc::clone(self),
                });
            }
        }
    }
}

pub struct OrchestratorSlot {
    hub: Arc<RelayHub>,
}

impl Drop for OrchestratorSlot {
    fn drop(&mut self) {
        self.hub.orchestrator_count.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct SessionState {
    pub session_id: String,
    pub server_seq: u64,
    pub last_peer_seq: u64,
    pub agent_id: Option<String>,
    pub peer_role: pb::PeerRole,
    pub peer_caps: Vec<i32>,
    pub send_timeout_ms: u64,
}

impl SessionState {
    pub fn new(send_timeout_ms: u64) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            server_seq: 0,
            last_peer_seq: 0,
            agent_id: None,
            peer_role: pb::PeerRole::PeerRoleUnspecified,
            peer_caps: Vec::new(),
            send_timeout_ms,
        }
    }

    fn supports_hello_ack(&self) -> bool {
        self.peer_caps
            .iter()
            .any(|cap| *cap == pb::Capability::CapHelloAckV1 as i32)
    }
}

pub async fn run_ws_session(
    tls_stream: TlsStream<TcpStream>,
    max_ws_message_bytes: usize,
    hello_timeout_ms: u64,
    send_timeout_ms: u64,
    relay_hub: Arc<RelayHub>,
) -> Result<()> {
    let ws_cfg = WebSocketConfig {
        max_send_queue: Some(32),
        max_message_size: Some(max_ws_message_bytes),
        max_frame_size: Some(max_ws_message_bytes),
        accept_unmasked_frames: false,
    };

    let mut ws = accept_async_with_config(tls_stream, Some(ws_cfg))
        .await
        .context("websocket accept")?;

    let mut st = SessionState::new(send_timeout_ms);
    info!(session_id = %st.session_id, "ws connected");

    let hello_msg = tokio::time::timeout(std::time::Duration::from_millis(hello_timeout_ms), ws.next())
        .await
        .context("hello timeout")?
        .transpose()
        .context("ws read")?
        .ok_or_else(|| anyhow::anyhow!("ws closed before hello"))?;

    let hello_env = match decode_envelope(hello_msg) {
        Ok(v) => v,
        Err(e) => {
            send_error(
                &mut ws,
                &mut st,
                pb::ErrorCode::ErrorCodeDecodeFailed,
                "invalid hello envelope",
                "hello-decode",
            )
            .await?;
            return Err(e);
        }
    };

    st.last_peer_seq = hello_env.seq;

    if hello_env.protocol_version != PROTOCOL_VERSION {
        send_error(
            &mut ws,
            &mut st,
            pb::ErrorCode::ErrorCodeProtocolViolation,
            "protocol_version mismatch",
            "hello-proto",
        )
        .await?;
        anyhow::bail!("protocol_version mismatch");
    }

    let hello = match hello_env.payload {
        Some(pb::envelope::Payload::Hello(h)) => h,
        _ => {
            send_error(
                &mut ws,
                &mut st,
                pb::ErrorCode::ErrorCodeProtocolViolation,
                "expected hello",
                "hello-shape",
            )
            .await?;
            anyhow::bail!("expected hello");
        }
    };

    st.agent_id = Some(hello.agent_id.clone());
    st.peer_caps = hello.capabilities.clone();
    st.peer_role = pb::PeerRole::from_i32(hello.role).unwrap_or(pb::PeerRole::PeerRoleUnspecified);

    let supports_hello_ack = st.supports_hello_ack();
    if !hello.handshake_id.trim().is_empty() {
        warn!(
            agent_id = %hello.agent_id,
            client_handshake_id = %hello.handshake_id,
            "ignored client handshake_id; bridge enforces server-side handshake_id"
        );
    }
    let handshake_id = st.session_id.clone();

    info!(
        agent_id = %hello.agent_id,
        client_version = %hello.client_version,
        role = ?st.peer_role,
        "hello received"
    );

    let mut orchestrator_slot: Option<OrchestratorSlot> = None;
    let mut telemetry_rx: Option<watch::Receiver<Option<pb::TelemetryFrame>>> = None;

    match st.peer_role {
        pb::PeerRole::PeerRoleGameClient => {
            send_handshake_ok(&mut ws, &mut st, supports_hello_ack, &handshake_id).await?;
        }
        pb::PeerRole::PeerRoleOrchestrator => {
            match relay_hub.acquire_orchestrator_slot() {
                Ok(slot) => {
                    telemetry_rx = Some(relay_hub.subscribe_telemetry());
                    orchestrator_slot = Some(slot);
                    send_handshake_ok(&mut ws, &mut st, supports_hello_ack, &handshake_id).await?;
                }
                Err(OrchestratorAcquireError::NotAllowed) => {
                    send_handshake_reject(
                        &mut ws,
                        &mut st,
                        supports_hello_ack,
                        &handshake_id,
                        "orchestrator subscriptions are disabled",
                    )
                    .await?;
                    anyhow::bail!("orchestrator subscriptions are disabled");
                }
                Err(OrchestratorAcquireError::LimitReached) => {
                    send_handshake_reject(
                        &mut ws,
                        &mut st,
                        supports_hello_ack,
                        &handshake_id,
                        "orchestrator subscription limit reached",
                    )
                    .await?;
                    anyhow::bail!("orchestrator subscription limit reached");
                }
            }
        }
        _ => {
            send_handshake_reject(
                &mut ws,
                &mut st,
                supports_hello_ack,
                &handshake_id,
                "unsupported peer role",
            )
            .await?;
            anyhow::bail!("unsupported peer role");
        }
    }

    if let Some(mut rx) = telemetry_rx {
        run_orchestrator_session_loop(&mut ws, &mut st, &relay_hub, &mut rx).await?;
    } else {
        run_standard_session_loop(&mut ws, &mut st, &relay_hub).await?;
    }

    drop(orchestrator_slot);
    Ok(())
}

async fn run_standard_session_loop(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    relay_hub: &Arc<RelayHub>,
) -> Result<()> {
    while let Some(msg) = ws.next().await {
        let msg = msg.context("ws read")?;
        if !handle_ws_message(ws, st, relay_hub, msg).await? {
            break;
        }
    }
    Ok(())
}

async fn run_orchestrator_session_loop(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    relay_hub: &Arc<RelayHub>,
    telemetry_rx: &mut watch::Receiver<Option<pb::TelemetryFrame>>,
) -> Result<()> {
    loop {
        tokio::select! {
            msg = ws.next() => {
                let Some(msg) = msg else {
                    break;
                };
                let msg = msg.context("ws read")?;
                if !handle_ws_message(ws, st, relay_hub, msg).await? {
                    break;
                }
            }
            relay = wait_for_telemetry(telemetry_rx) => {
                match relay {
                    Some(telemetry) => {
                        send_envelope(ws, st, pb::envelope::Payload::Telemetry(telemetry)).await?;
                    }
                    None => {
                        warn!(session_id = %st.session_id, "relay channel closed");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn wait_for_telemetry(
    telemetry_rx: &mut watch::Receiver<Option<pb::TelemetryFrame>>,
) -> Option<pb::TelemetryFrame> {
    if telemetry_rx.changed().await.is_err() {
        return None;
    }
    telemetry_rx.borrow().clone()
}

async fn handle_ws_message(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    relay_hub: &Arc<RelayHub>,
    msg: WsMessage,
) -> Result<bool> {
    if msg.is_close() {
        info!(session_id = %st.session_id, "ws close");
        return Ok(false);
    }
    if msg.is_ping() {
        return Ok(true);
    }
    if !msg.is_binary() {
        return Ok(true);
    }

    let env = match decode_envelope(msg) {
        Ok(v) => v,
        Err(e) => {
            warn!(session_id = %st.session_id, error = %e, "decode failed");
            send_error(
                ws,
                st,
                pb::ErrorCode::ErrorCodeDecodeFailed,
                "decode failed",
                "msg-decode",
            )
            .await?;
            return Ok(true);
        }
    };

    if env.protocol_version != PROTOCOL_VERSION {
        send_error(
            ws,
            st,
            pb::ErrorCode::ErrorCodeProtocolViolation,
            "protocol_version mismatch",
            "msg-proto",
        )
        .await?;
        return Ok(true);
    }

    st.last_peer_seq = env.seq;

    match env.payload {
        Some(pb::envelope::Payload::Telemetry(t)) => {
            if st.peer_role == pb::PeerRole::PeerRoleGameClient {
                relay_hub.publish_telemetry(&t);
            }
            info!(
                agent_id = %st.agent_id.clone().unwrap_or_default(),
                state_version = t.state_version,
                hp = t.hp,
                hunger = t.hunger,
                ack = st.last_peer_seq,
                "telemetry"
            );
        }
        Some(pb::envelope::Payload::Heartbeat(hb)) => {
            info!(
                agent_id = %st.agent_id.clone().unwrap_or_default(),
                rx = hb.rx_queue_len,
                tx = hb.tx_queue_len,
                drop_count = hb.dropped_frames,
                "heartbeat"
            );
        }
        Some(pb::envelope::Payload::TimeSyncReq(req)) => {
            let now = mono_ms();
            let res = pb::TimeSyncResponse {
                t0_mono_ms: req.t0_mono_ms,
                t1_mono_ms: now,
                t2_mono_ms: now,
            };
            send_envelope(ws, st, pb::envelope::Payload::TimeSyncRes(res)).await?;
        }
        Some(pb::envelope::Payload::Hello(_)) => {
            warn!(session_id = %st.session_id, "unexpected hello");
        }
        Some(pb::envelope::Payload::HelloAck(_)) => {
            warn!(session_id = %st.session_id, "unexpected hello_ack");
        }
        Some(pb::envelope::Payload::Error(err)) => {
            warn!(code = err.code, correlation_id = %err.correlation_id, message = %err.message, "peer error");
        }
        _ => {
            // Keep action payloads ignored in MVP-5.
        }
    }

    Ok(true)
}

fn decode_envelope(msg: WsMessage) -> Result<pb::Envelope> {
    let data = msg.into_data();
    let env = pb::Envelope::decode(data.as_slice()).context("prost decode")?;
    Ok(env)
}

async fn send_handshake_ok(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    use_hello_ack: bool,
    handshake_id: &str,
) -> Result<()> {
    if use_hello_ack {
        let negotiated = negotiated_capabilities(&st.peer_caps);
        let ack = pb::HelloAck {
            handshake_id: handshake_id.to_string(),
            accepted: true,
            reason: "ok".to_string(),
            negotiated_capabilities: negotiated,
            server_version: "miqbot-bridge-server/0.2.0".to_string(),
        };
        send_envelope(ws, st, pb::envelope::Payload::HelloAck(ack)).await
    } else {
        let reply = pb::Hello {
            agent_id: "bridge".to_string(),
            role: pb::PeerRole::PeerRoleBridgeServer as i32,
            capabilities: vec![
                pb::Capability::CapTelemetryV1 as i32,
                pb::Capability::CapTimesyncV1 as i32,
            ],
            client_version: "miqbot-bridge-server/0.2.0".to_string(),
            handshake_id: handshake_id.to_string(),
        };
        send_envelope(ws, st, pb::envelope::Payload::Hello(reply)).await
    }
}

fn negotiated_capabilities(peer_caps: &[i32]) -> Vec<i32> {
    SERVER_NEGOTIABLE_CAPABILITIES
        .iter()
        .copied()
        .filter(|cap| peer_caps.contains(cap))
        .collect()
}

async fn send_handshake_reject(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    use_hello_ack: bool,
    handshake_id: &str,
    reason: &str,
) -> Result<()> {
    if use_hello_ack {
        let ack = pb::HelloAck {
            handshake_id: handshake_id.to_string(),
            accepted: false,
            reason: reason.to_string(),
            negotiated_capabilities: Vec::new(),
            server_version: "miqbot-bridge-server/0.2.0".to_string(),
        };
        send_envelope(ws, st, pb::envelope::Payload::HelloAck(ack)).await
    } else {
        send_error(
            ws,
            st,
            pb::ErrorCode::ErrorCodeUnauthorized,
            reason,
            "hello-reject",
        )
        .await
    }
}

async fn send_error(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    code: pb::ErrorCode,
    message: &str,
    correlation_hint: &str,
) -> Result<()> {
    let err = pb::ErrorFrame {
        code: code as i32,
        message: message.to_string(),
        correlation_id: format!("{}-{}", correlation_hint, Uuid::new_v4()),
    };
    send_envelope(ws, st, pb::envelope::Payload::Error(err)).await
}

async fn send_envelope(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    payload: pb::envelope::Payload,
) -> Result<()> {
    st.server_seq += 1;
    let env = pb::Envelope {
        protocol_version: PROTOCOL_VERSION,
        session_id: st.session_id.clone(),
        seq: st.server_seq,
        ack: st.last_peer_seq,
        mono_ms: mono_ms(),
        wall_unix_ms: wall_unix_ms(),
        payload: Some(payload),
    };

    let mut buf = Vec::with_capacity(env.encoded_len());
    env.encode(&mut buf).context("encode env")?;
    match tokio::time::timeout(
        std::time::Duration::from_millis(st.send_timeout_ms),
        ws.send(WsMessage::Binary(buf)),
    )
    .await
    {
        Ok(send_result) => {
            send_result.context("ws send")?;
        }
        Err(_) => {
            anyhow::bail!("ws send timeout after {}ms", st.send_timeout_ms);
        }
    }
    Ok(())
}

fn mono_ms() -> u64 {
    use std::sync::OnceLock;
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    let t0 = T0.get_or_init(std::time::Instant::now);
    t0.elapsed().as_millis() as u64
}

fn wall_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
