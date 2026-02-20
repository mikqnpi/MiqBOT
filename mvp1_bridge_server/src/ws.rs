use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_rustls::server::TlsStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{accept_async_with_config, tungstenite::protocol::Message as WsMessage};
use tracing::{info, warn};
use uuid::Uuid;

use crate::pb::bridge_v1 as pb;
use crate::relay::{ActionRelayFrame, OrchestratorAcquireError, RelayHub};

const PROTOCOL_VERSION: u32 = 1;
const SERVER_NEGOTIABLE_CAPABILITIES: [i32; 4] = [
    pb::Capability::CapTelemetryV1 as i32,
    pb::Capability::CapTimesyncV1 as i32,
    pb::Capability::CapActionsV1 as i32,
    pb::Capability::CapHelloAckV1 as i32,
];

pub struct SessionState {
    pub session_id: String,
    pub server_seq: u64,
    pub last_peer_seq: u64,
    pub agent_id: Option<String>,
    pub peer_role: pb::PeerRole,
    pub peer_caps: Vec<i32>,
    pub send_timeout_ms: u64,
    pub is_primary_game: bool,
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
            is_primary_game: false,
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

    let hello_msg = match tokio::time::timeout(
        std::time::Duration::from_millis(hello_timeout_ms),
        ws.next(),
    )
    .await
    {
        Ok(msg) => msg
            .transpose()
            .context("ws read")?
            .ok_or_else(|| anyhow::anyhow!("ws closed before hello"))?,
        Err(_) => {
            send_error(
                &mut ws,
                &mut st,
                pb::ErrorCode::ErrorCodeTimeout,
                "hello timeout",
                "hello-timeout",
            )
            .await?;
            anyhow::bail!("hello timeout");
        }
    };

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

    match st.peer_role {
        pb::PeerRole::PeerRoleGameClient => {
            let mut action_rx: Option<mpsc::Receiver<pb::ActionRequest>> = None;
            if relay_hub.is_primary_game_agent(&hello.agent_id) {
                let (tx, rx) = mpsc::channel(relay_hub.action_queue_size());
                if let Err(e) = relay_hub
                    .attach_primary_game_sender(tx, &hello.agent_id)
                    .await
                    .context("attach primary game sender")
                {
                    send_handshake_reject(
                        &mut ws,
                        &mut st,
                        supports_hello_ack,
                        &handshake_id,
                        "primary game sender is unavailable",
                    )
                    .await?;
                    return Err(e);
                }
                st.is_primary_game = true;
                action_rx = Some(rx);
            } else {
                warn!(
                    agent_id = %hello.agent_id,
                    primary_game_agent_id = %relay_hub.primary_game_agent_id(),
                    "non-primary game client connected; telemetry/action relay disabled for this session"
                );
            }

            send_handshake_ok(&mut ws, &mut st, supports_hello_ack, &handshake_id).await?;
            let run_res = run_game_session_loop(&mut ws, &mut st, &relay_hub, action_rx.as_mut()).await;
            if st.is_primary_game {
                relay_hub.detach_primary_game_sender().await;
            }
            run_res?;
        }
        pb::PeerRole::PeerRoleOrchestrator => {
            let _slot = match relay_hub.acquire_orchestrator_slot() {
                Ok(slot) => slot,
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
            };

            let mut telemetry_rx = relay_hub.subscribe_telemetry();
            let (action_reply_tx, mut action_reply_rx) =
                mpsc::channel::<ActionRelayFrame>(relay_hub.action_queue_size());

            send_handshake_ok(&mut ws, &mut st, supports_hello_ack, &handshake_id).await?;
            run_orchestrator_session_loop(
                &mut ws,
                &mut st,
                &relay_hub,
                &mut telemetry_rx,
                &mut action_reply_rx,
                &action_reply_tx,
            )
            .await?;
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

    Ok(())
}

async fn run_game_session_loop(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    relay_hub: &Arc<RelayHub>,
    action_rx: Option<&mut mpsc::Receiver<pb::ActionRequest>>,
) -> Result<()> {
    if let Some(action_rx) = action_rx {
        loop {
            tokio::select! {
                msg = ws.next() => {
                    let Some(msg) = msg else {
                        break;
                    };
                    let msg = msg.context("ws read")?;
                    if !handle_ws_message(ws, st, relay_hub, msg, None).await? {
                        break;
                    }
                }
                action_req = action_rx.recv() => {
                    let Some(action_req) = action_req else {
                        break;
                    };
                    send_envelope(ws, st, pb::envelope::Payload::ActionReq(action_req)).await?;
                }
            }
        }
        Ok(())
    } else {
        run_standard_session_loop(ws, st, relay_hub).await
    }
}

async fn run_standard_session_loop(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    relay_hub: &Arc<RelayHub>,
) -> Result<()> {
    while let Some(msg) = ws.next().await {
        let msg = msg.context("ws read")?;
        if !handle_ws_message(ws, st, relay_hub, msg, None).await? {
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
    action_reply_rx: &mut mpsc::Receiver<ActionRelayFrame>,
    action_reply_tx: &mpsc::Sender<ActionRelayFrame>,
) -> Result<()> {
    loop {
        tokio::select! {
            msg = ws.next() => {
                let Some(msg) = msg else {
                    break;
                };
                let msg = msg.context("ws read")?;
                if !handle_ws_message(ws, st, relay_hub, msg, Some(action_reply_tx)).await? {
                    break;
                }
            }
            relay = wait_for_telemetry(telemetry_rx) => {
                match relay {
                    Some(telemetry) => {
                        send_envelope(ws, st, pb::envelope::Payload::Telemetry(telemetry)).await?;
                    }
                    None => {
                        warn!(session_id = %st.session_id, "telemetry relay channel closed");
                        break;
                    }
                }
            }
            action_reply = action_reply_rx.recv() => {
                let Some(action_reply) = action_reply else {
                    break;
                };
                match action_reply {
                    ActionRelayFrame::Ack(ack) => {
                        send_envelope(ws, st, pb::envelope::Payload::ActionAck(ack)).await?;
                    }
                    ActionRelayFrame::Result(result) => {
                        send_envelope(ws, st, pb::envelope::Payload::ActionRes(result)).await?;
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
    action_reply_tx: Option<&mpsc::Sender<ActionRelayFrame>>,
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
            if st.peer_role == pb::PeerRole::PeerRoleGameClient && st.is_primary_game {
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
        Some(pb::envelope::Payload::ActionReq(req)) => {
            if st.peer_role != pb::PeerRole::PeerRoleOrchestrator {
                warn!(session_id = %st.session_id, "unexpected action_req from non-orchestrator");
                return Ok(true);
            }
            let Some(action_reply_tx) = action_reply_tx else {
                warn!(session_id = %st.session_id, "missing orchestrator action reply channel");
                return Ok(true);
            };
            if let Err(e) = relay_hub.enqueue_action(req.clone(), action_reply_tx.clone()).await {
                let nack = pb::ActionAck {
                    request_id: req.request_id,
                    accepted: false,
                    reason: e.to_string(),
                };
                send_envelope(ws, st, pb::envelope::Payload::ActionAck(nack)).await?;
            }
        }
        Some(pb::envelope::Payload::ActionAck(ack)) => {
            if st.peer_role == pb::PeerRole::PeerRoleGameClient && st.is_primary_game {
                relay_hub.route_action_ack(&ack).await;
            } else {
                warn!(session_id = %st.session_id, "unexpected action_ack");
            }
        }
        Some(pb::envelope::Payload::ActionRes(result)) => {
            if st.peer_role == pb::PeerRole::PeerRoleGameClient && st.is_primary_game {
                relay_hub.route_action_result(&result).await;
            } else {
                warn!(session_id = %st.session_id, "unexpected action_res");
            }
        }
        Some(pb::envelope::Payload::Hello(_)) => {
            warn!(session_id = %st.session_id, "unexpected hello");
        }
        Some(pb::envelope::Payload::HelloAck(_)) => {
            warn!(session_id = %st.session_id, "unexpected hello_ack");
        }
        Some(pb::envelope::Payload::Error(err)) => {
            warn!(
                code = err.code,
                correlation_id = %err.correlation_id,
                message = %err.message,
                "peer error"
            );
        }
        None => {}
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
            server_version: "miqbot-bridge-server/0.3.0".to_string(),
        };
        send_envelope(ws, st, pb::envelope::Payload::HelloAck(ack)).await
    } else {
        let reply = pb::Hello {
            agent_id: "bridge".to_string(),
            role: pb::PeerRole::PeerRoleBridgeServer as i32,
            capabilities: vec![
                pb::Capability::CapTelemetryV1 as i32,
                pb::Capability::CapTimesyncV1 as i32,
                pb::Capability::CapActionsV1 as i32,
            ],
            client_version: "miqbot-bridge-server/0.3.0".to_string(),
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
            server_version: "miqbot-bridge-server/0.3.0".to_string(),
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
