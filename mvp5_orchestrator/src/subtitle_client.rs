use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Clone)]
pub struct SubtitleClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubtitleResponse {
    pub ok: bool,
    pub request_id: String,
    pub wrapped: String,
    pub visible_chars: u64,
    pub show_s: f64,
}

impl SubtitleClient {
    pub fn new(base_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
        }
    }

    pub async fn post_subtitle(&self, text: &str) -> Result<SubtitleResponse> {
        let url = format!("{}/v1/subtitle", self.base_url);
        let res = self
            .http
            .post(url)
            .json(&serde_json::json!({ "text": text }))
            .send()
            .await
            .context("subtitle request failed")?;

        if !res.status().is_success() {
            anyhow::bail!("subtitle request returned non-success status: {}", res.status());
        }

        let body: SubtitleResponse = res.json().await.context("subtitle decode failed")?;
        if !body.ok {
            anyhow::bail!("subtitle gateway returned ok=false");
        }
        Ok(body)
    }
}
