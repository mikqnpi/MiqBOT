use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct BridgeConfig {
    pub bind_addr: String,
    pub tls: TlsConfig,
    pub limits: LimitsConfig,
    pub relay: RelayConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub client_ca_cert_pem: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LimitsConfig {
    pub max_ws_message_bytes: usize,
    pub hello_timeout_ms: u64,
    pub send_timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RelayConfig {
    pub allow_orchestrator_subscribe: bool,
    pub max_orchestrator_subscribers: usize,
    pub min_relay_interval_ms: u64,
}

impl BridgeConfig {
    pub fn load(path: &str) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("read config: {path}"))?;
        let cfg: BridgeConfig = toml::from_str(&s).context("parse toml")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.bind_addr.trim().is_empty() {
            bail!("bind_addr must not be empty");
        }
        if self.limits.max_ws_message_bytes < 1024 {
            bail!("max_ws_message_bytes too small");
        }
        if self.limits.hello_timeout_ms == 0 {
            bail!("hello_timeout_ms must be > 0");
        }
        if self.limits.send_timeout_ms == 0 {
            bail!("send_timeout_ms must be > 0");
        }
        if self.relay.max_orchestrator_subscribers == 0 {
            bail!("max_orchestrator_subscribers must be > 0");
        }
        Ok(())
    }
}
