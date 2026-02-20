use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct OrchestratorConfig {
    pub bridge_url: String,
    pub agent_id: String,
    pub client_version: String,
    pub primary_game_agent_id: String,

    pub tts_url: String,
    pub subtitle_url: String,
    pub tts_mode: String,

    pub silence_gap_ms: u64,
    pub state_tick_ms: u64,
    pub duplicate_cooldown_ms: u64,

    pub queue_max_p0: usize,
    pub queue_max_p1: usize,
    pub queue_max_p2: usize,
    pub chat_deadline_ms: u64,
    pub filler_deadline_ms: u64,

    pub action_ack_timeout_ms: u64,
    pub action_result_timeout_ms: u64,

    pub audio_output_dir: String,
    pub fallback_wav_path: String,
    pub metrics_jsonl_path: String,

    pub tls: TlsConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TtsMode {
    WavOnly,
    WithMeta,
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

    pub fn tts_mode(&self) -> TtsMode {
        match self.tts_mode.as_str() {
            "with_meta" => TtsMode::WithMeta,
            _ => TtsMode::WavOnly,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.bridge_url.trim().is_empty() {
            bail!("bridge_url must not be empty");
        }
        if self.agent_id.trim().is_empty() {
            bail!("agent_id must not be empty");
        }
        if self.client_version.trim().is_empty() {
            bail!("client_version must not be empty");
        }
        if self.primary_game_agent_id.trim().is_empty() {
            bail!("primary_game_agent_id must not be empty");
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
        if self.state_tick_ms == 0 {
            bail!("state_tick_ms must be > 0");
        }
        if self.queue_max_p0 == 0 || self.queue_max_p1 == 0 || self.queue_max_p2 == 0 {
            bail!("queue_max_p0/p1/p2 must be > 0");
        }
        if self.chat_deadline_ms == 0 || self.filler_deadline_ms == 0 {
            bail!("chat_deadline_ms and filler_deadline_ms must be > 0");
        }
        if self.action_ack_timeout_ms == 0 || self.action_result_timeout_ms == 0 {
            bail!("action_ack_timeout_ms and action_result_timeout_ms must be > 0");
        }
        if self.audio_output_dir.trim().is_empty() {
            bail!("audio_output_dir must not be empty");
        }
        if self.fallback_wav_path.trim().is_empty() {
            bail!("fallback_wav_path must not be empty");
        }
        if self.metrics_jsonl_path.trim().is_empty() {
            bail!("metrics_jsonl_path must not be empty");
        }
        Ok(())
    }
}
