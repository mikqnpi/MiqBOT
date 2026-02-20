use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct OrchestratorConfig {
    pub bridge_url: String,
    pub agent_id: String,
    pub client_version: String,

    pub tts_url: String,
    pub subtitle_url: String,

    pub silence_gap_ms: u64,
    pub loop_sleep_ms: u64,
    pub duplicate_cooldown_ms: u64,

    pub audio_output_dir: String,
    pub fallback_wav_path: String,
    pub metrics_jsonl_path: String,

    pub tls: TlsConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
    pub client_cert_pem: String,
    pub client_key_pem: String,
    pub ca_cert_pem: String,
}

impl OrchestratorConfig {
    pub fn load(path: &str) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("read config: {path}"))?;
        let cfg: OrchestratorConfig = toml::from_str(&s).context("parse toml")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.bridge_url.trim().is_empty() {
            bail!("bridge_url must not be empty");
        }
        if self.tts_url.trim().is_empty() {
            bail!("tts_url must not be empty");
        }
        if self.subtitle_url.trim().is_empty() {
            bail!("subtitle_url must not be empty");
        }
        if self.silence_gap_ms == 0 {
            bail!("silence_gap_ms must be > 0");
        }
        if self.loop_sleep_ms == 0 {
            bail!("loop_sleep_ms must be > 0");
        }
        Ok(())
    }
}
