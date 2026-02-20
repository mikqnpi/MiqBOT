use crate::config::TtsMode;
use anyhow::{Context, Result};
use base64::Engine;
use serde::Deserialize;

#[derive(Clone)]
pub struct TtsClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Clone)]
pub struct SynthResult {
    pub wav_bytes: Vec<u8>,
    pub ttft_ms: Option<u64>,
    pub total_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TtsWithMetaResponse {
    ttft_ms: u64,
    total_ms: u64,
    audio_wav_base64: String,
}

impl TtsClient {
    pub fn new(base_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
        }
    }

    pub async fn synthesize(&self, text: &str, mode: TtsMode) -> Result<SynthResult> {
        match mode {
            TtsMode::WavOnly => self.synthesize_wav_only(text).await,
            TtsMode::WithMeta => self.synthesize_with_meta(text).await,
        }
    }

    async fn synthesize_wav_only(&self, text: &str) -> Result<SynthResult> {
        let url = format!("{}/v1/tts", self.base_url);
        let res = self
            .http
            .post(url)
            .json(&serde_json::json!({
                "text": text,
                "sample_rate_hz": 48000,
            }))
            .send()
            .await
            .context("tts request failed")?;

        if !res.status().is_success() {
            anyhow::bail!("tts request returned non-success status: {}", res.status());
        }

        Ok(SynthResult {
            wav_bytes: res.bytes().await.context("read tts bytes")?.to_vec(),
            ttft_ms: None,
            total_ms: None,
        })
    }

    async fn synthesize_with_meta(&self, text: &str) -> Result<SynthResult> {
        let url = format!("{}/v1/tts_with_meta", self.base_url);
        let res = self
            .http
            .post(url)
            .json(&serde_json::json!({
                "text": text,
                "sample_rate_hz": 48000,
            }))
            .send()
            .await
            .context("tts_with_meta request failed")?;

        if !res.status().is_success() {
            anyhow::bail!(
                "tts_with_meta request returned non-success status: {}",
                res.status()
            );
        }

        let body: TtsWithMetaResponse = res.json().await.context("decode tts_with_meta response")?;
        let wav = base64::engine::general_purpose::STANDARD
            .decode(body.audio_wav_base64.as_bytes())
            .context("decode audio_wav_base64")?;

        Ok(SynthResult {
            wav_bytes: wav,
            ttft_ms: Some(body.ttft_ms),
            total_ms: Some(body.total_ms),
        })
    }
}
