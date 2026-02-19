use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{accept_async_with_config, tungstenite::protocol::Message as WsMessage};
use tracing::{info, warn};
use uuid::Uuid;

use crate::pb::bridge_v1 as pb;

const PROTOCOL_VERSION: u32 = 1;

pub struct SessionState {
    pub session_id: String,
    pub server_seq: u64,
    pub last_peer_seq: u64,
    pub agent_id: Option<String>,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            server_seq: 0,
            last_peer_seq: 0,
            agent_id: None,
        }
    }
}

pub async fn run_ws_session(
    tls_stream: TlsStream<TcpStream>,
    max_ws_message_bytes: usize,
    hello_timeout_ms: u64,
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

    let mut st = SessionState::new();
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
            )
            .await?;
            anyhow::bail!("expected hello");
        }
    };

    st.agent_id = Some(hello.agent_id.clone());
    info!(agent_id = %hello.agent_id, client_version = %hello.client_version, "hello received");

    let reply = pb::Hello {
        agent_id: "bridge".to_string(),
        role: pb::PeerRole::PeerRoleBridgeServer as i32,
        capabilities: vec![
            pb::Capability::CapTelemetryV1 as i32,
            pb::Capability::CapTimesyncV1 as i32,
        ],
        client_version: "miqbot-bridge-server/0.1.0".to_string(),
    };
    send_envelope(&mut ws, &mut st, pb::envelope::Payload::Hello(reply)).await?;

    while let Some(msg) = ws.next().await {
        let msg = msg.context("ws read")?;

        if msg.is_close() {
            info!(session_id = %st.session_id, "ws close");
            break;
        }
        if msg.is_ping() {
            continue;
        }
        if !msg.is_binary() {
            continue;
        }

        let env = match decode_envelope(msg) {
            Ok(v) => v,
            Err(e) => {
                warn!(session_id = %st.session_id, error = %e, "decode failed");
                send_error(
                    &mut ws,
                    &mut st,
                    pb::ErrorCode::ErrorCodeDecodeFailed,
                    "decode failed",
                )
                .await?;
                continue;
            }
        };

        if env.protocol_version != PROTOCOL_VERSION {
            send_error(
                &mut ws,
                &mut st,
                pb::ErrorCode::ErrorCodeProtocolViolation,
                "protocol_version mismatch",
            )
            .await?;
            continue;
        }

        st.last_peer_seq = env.seq;

        match env.payload {
            Some(pb::envelope::Payload::Telemetry(t)) => {
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
                send_envelope(&mut ws, &mut st, pb::envelope::Payload::TimeSyncRes(res)).await?;
            }
            Some(pb::envelope::Payload::Hello(_)) => {
                warn!(session_id = %st.session_id, "unexpected hello");
            }
            Some(pb::envelope::Payload::Error(err)) => {
                warn!(code = err.code, message = %err.message, "peer error");
            }
            _ => {
                // Keep action payloads ignored in MVP-1.
            }
        }
    }

    Ok(())
}

fn decode_envelope(msg: WsMessage) -> Result<pb::Envelope> {
    let data = msg.into_data();
    let env = pb::Envelope::decode(data.as_slice()).context("prost decode")?;
    Ok(env)
}

async fn send_error(
    ws: &mut tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>,
    st: &mut SessionState,
    code: pb::ErrorCode,
    message: &str,
) -> Result<()> {
    let err = pb::ErrorFrame {
        code: code as i32,
        message: message.to_string(),
        correlation_id: "".to_string(),
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
    ws.send(WsMessage::Binary(buf)).await.context("ws send")?;
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

