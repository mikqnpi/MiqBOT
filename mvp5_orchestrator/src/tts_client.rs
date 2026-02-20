use anyhow::{Context, Result};

#[derive(Clone)]
pub struct TtsClient {
    http: reqwest::Client,
    base_url: String,
}

impl TtsClient {
    pub fn new(base_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
        }
    }

    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
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

        Ok(res.bytes().await.context("read tts bytes")?.to_vec())
    }
}
