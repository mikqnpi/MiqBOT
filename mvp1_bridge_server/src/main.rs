mod config;
mod pb;
mod tls;
mod ws;

use anyhow::Result;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let cfg_path =
        std::env::var("MIQBOT_BRIDGE_CONFIG").unwrap_or_else(|_| "config/bridge.toml".to_string());
    let cfg = config::BridgeConfig::load(&cfg_path)?;

    let tls_cfg = tls::make_server_config(
        &cfg.tls.server_cert_pem,
        &cfg.tls.server_key_pem,
        &cfg.tls.client_ca_cert_pem,
    )?;
    let acceptor = TlsAcceptor::from(tls_cfg);

    let listener = TcpListener::bind(&cfg.bind_addr).await?;
    info!(bind_addr = %cfg.bind_addr, "bridge server listening");
    let relay_hub = ws::RelayHub::new(cfg.relay.clone());

    loop {
        tokio::select! {
            res = listener.accept() => {
                let (tcp, addr) = res?;
                let acceptor = acceptor.clone();
                let limits = cfg.limits.clone();
                let relay_hub = relay_hub.clone();

                tokio::spawn(async move {
                    let tls = match acceptor.accept(tcp).await {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(peer = %addr, error = %e, "tls accept failed");
                            return;
                        }
                    };

                    if let Err(e) = ws::run_ws_session(
                        tls,
                        limits.max_ws_message_bytes,
                        limits.hello_timeout_ms,
                        relay_hub,
                    )
                    .await
                    {
                        warn!(peer = %addr, error = %e, "ws session error");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown");
                break;
            }
        }
    }

    Ok(())
}
