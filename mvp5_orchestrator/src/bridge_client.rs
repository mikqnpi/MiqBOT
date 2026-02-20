use crate::config::TlsConfig;
use crate::pb::bridge_v1 as pb;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use rustls::{Certificate, ClientConfig, PrivateKey, RootCertStore, ServerName};
use std::fs::File;
use std::io::BufReader;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;

const PROTOCOL_VERSION: u32 = 1;

pub struct BridgeClient {
    ws: WebSocketStream<TlsStream<TcpStream>>,
    session_id: String,
    seq: u64,
    last_peer_seq: u64,
}

impl BridgeClient {
    pub async fn connect(
        bridge_url: &str,
        agent_id: &str,
        client_version: &str,
        tls_cfg: &TlsConfig,
    ) -> Result<Self> {
        let url = Url::parse(bridge_url).context("parse bridge_url")?;
        let host = url.host_str().context("bridge_url host missing")?.to_string();
        let port = url.port_or_known_default().context("bridge_url port missing")?;

        let tls_client_cfg = make_client_config(tls_cfg)?;
        let connector = TlsConnector::from(Arc::new(tls_client_cfg));

        let tcp = TcpStream::connect((host.as_str(), port))
            .await
            .context("tcp connect bridge")?;
        let server_name = server_name_from_host(&host)?;
        let tls_stream = connector
            .connect(server_name, tcp)
            .await
            .context("tls connect bridge")?;

        let (ws, _response) = client_async(bridge_url, tls_stream)
            .await
            .context("ws handshake")?;

        let mut client = Self {
            ws,
            session_id: Uuid::new_v4().to_string(),
            seq: 0,
            last_peer_seq: 0,
        };

        client.send_hello(agent_id, client_version).await?;
        client.wait_for_handshake().await?;

        Ok(client)
    }

    pub async fn next_telemetry(&mut self) -> Result<Option<pb::TelemetryFrame>> {
        while let Some(msg) = self.ws.next().await {
            let msg = msg.context("ws read")?;
            if msg.is_close() {
                return Ok(None);
            }
            if !msg.is_binary() {
                continue;
            }

            let env = decode_envelope(msg)?;
            self.last_peer_seq = env.seq;

            match env.payload {
                Some(pb::envelope::Payload::Telemetry(t)) => return Ok(Some(t)),
                Some(pb::envelope::Payload::Heartbeat(hb)) => {
                    info!(
                        rx = hb.rx_queue_len,
                        tx = hb.tx_queue_len,
                        dropped = hb.dropped_frames,
                        "bridge heartbeat"
                    );
                }
                Some(pb::envelope::Payload::HelloAck(ack)) => {
                    if !ack.accepted {
                        anyhow::bail!("bridge rejected hello: {}", ack.reason);
                    }
                }
                Some(pb::envelope::Payload::Hello(hello)) => {
                    info!(
                        client_version = %hello.client_version,
                        "bridge legacy hello reply"
                    );
                }
                Some(pb::envelope::Payload::Error(err)) => {
                    warn!(
                        code = err.code,
                        correlation_id = %err.correlation_id,
                        message = %err.message,
                        "bridge error"
                    );
                }
                _ => {}
            }
        }

        Ok(None)
    }

    async fn send_hello(&mut self, agent_id: &str, client_version: &str) -> Result<()> {
        let hello = pb::Hello {
            agent_id: agent_id.to_string(),
            role: pb::PeerRole::PeerRoleOrchestrator as i32,
            capabilities: vec![
                pb::Capability::CapTelemetryV1 as i32,
                pb::Capability::CapTimesyncV1 as i32,
                pb::Capability::CapHelloAckV1 as i32,
            ],
            client_version: client_version.to_string(),
            handshake_id: Uuid::new_v4().to_string(),
        };

        self.send_envelope(pb::envelope::Payload::Hello(hello)).await
    }

    async fn wait_for_handshake(&mut self) -> Result<()> {
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(5));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    anyhow::bail!("bridge hello handshake timeout");
                }
                msg = self.ws.next() => {
                    let Some(msg) = msg else {
                        anyhow::bail!("bridge closed before handshake");
                    };
                    let msg = msg.context("ws read during handshake")?;
                    if !msg.is_binary() {
                        continue;
                    }

                    let env = decode_envelope(msg)?;
                    self.last_peer_seq = env.seq;

                    match env.payload {
                        Some(pb::envelope::Payload::HelloAck(ack)) => {
                            if !ack.accepted {
                                anyhow::bail!("bridge rejected handshake: {}", ack.reason);
                            }
                            info!(reason = %ack.reason, "bridge hello_ack accepted");
                            return Ok(());
                        }
                        Some(pb::envelope::Payload::Hello(hello)) => {
                            info!(client_version = %hello.client_version, "bridge legacy hello accepted");
                            return Ok(());
                        }
                        Some(pb::envelope::Payload::Error(err)) => {
                            anyhow::bail!("bridge error during handshake: {} ({})", err.message, err.correlation_id);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn send_envelope(&mut self, payload: pb::envelope::Payload) -> Result<()> {
        self.seq += 1;
        let env = pb::Envelope {
            protocol_version: PROTOCOL_VERSION,
            session_id: self.session_id.clone(),
            seq: self.seq,
            ack: self.last_peer_seq,
            mono_ms: mono_ms(),
            wall_unix_ms: wall_unix_ms(),
            payload: Some(payload),
        };

        let mut buf = Vec::with_capacity(env.encoded_len());
        env.encode(&mut buf).context("encode envelope")?;
        self.ws.send(WsMessage::Binary(buf)).await.context("ws send")?;
        Ok(())
    }
}

fn decode_envelope(msg: WsMessage) -> Result<pb::Envelope> {
    let data = msg.into_data();
    let env = pb::Envelope::decode(data.as_slice()).context("decode envelope")?;
    if env.protocol_version != PROTOCOL_VERSION {
        anyhow::bail!("protocol_version mismatch");
    }
    Ok(env)
}

fn make_client_config(tls_cfg: &TlsConfig) -> Result<ClientConfig> {
    let certs = load_certs(&tls_cfg.client_cert_pem).context("load client cert")?;
    let key = load_private_key(&tls_cfg.client_key_pem).context("load client key")?;

    let mut roots = RootCertStore::empty();
    for cert in load_certs(&tls_cfg.ca_cert_pem).context("load ca cert")? {
        roots.add(&cert).context("add ca cert")?;
    }

    let cfg = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(roots)
        .with_single_cert(certs, key)
        .context("build tls client config")?;

    Ok(cfg)
}

fn load_certs(path: &str) -> Result<Vec<Certificate>> {
    let f = File::open(path).with_context(|| format!("open cert file: {path}"))?;
    let mut r = BufReader::new(f);
    let certs = rustls_pemfile::certs(&mut r).context("read certs")?;
    Ok(certs.into_iter().map(Certificate).collect())
}

fn load_private_key(path: &str) -> Result<PrivateKey> {
    let f = File::open(path).with_context(|| format!("open key file: {path}"))?;
    let mut r = BufReader::new(f);

    let keys = rustls_pemfile::pkcs8_private_keys(&mut r).context("read pkcs8 keys")?;
    if let Some(k) = keys.into_iter().next() {
        return Ok(PrivateKey(k));
    }

    let f = File::open(path).with_context(|| format!("open key file: {path}"))?;
    let mut r = BufReader::new(f);
    let keys = rustls_pemfile::rsa_private_keys(&mut r).context("read rsa keys")?;
    if let Some(k) = keys.into_iter().next() {
        return Ok(PrivateKey(k));
    }

    anyhow::bail!("no private key found in {path}")
}

fn server_name_from_host(host: &str) -> Result<ServerName> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip));
    }

    ServerName::try_from(host.to_string()).context("invalid tls server name")
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
