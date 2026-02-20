mod action_client;
mod action_ledger;
mod audio_player;
mod bridge_client;
mod config;
mod pb;
mod pipeline;
mod speech_queue;
mod state_actor;
mod subtitle_client;
mod tts_client;

use anyhow::{Context, Result};
use audio_player::AudioPlayer;
use bridge_client::BridgeClient;
use config::OrchestratorConfig;
use state_actor::StateActor;
use subtitle_client::SubtitleClient;
use tracing::info;
use tts_client::TtsClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let cfg_path =
        std::env::var("MIQBOT_ORCH_CONFIG").unwrap_or_else(|_| "config/orchestrator.toml".to_string());
    let cfg = OrchestratorConfig::load(&cfg_path)?;

    let bridge = BridgeClient::connect(&cfg.bridge_url, &cfg.agent_id, &cfg.client_version, &cfg.tls)
        .await
        .context("connect bridge")?;
    let subtitle = SubtitleClient::new(cfg.subtitle_url.clone());
    let tts = TtsClient::new(cfg.tts_url.clone());
    let audio = AudioPlayer::new(cfg.audio_output_dir.clone(), cfg.fallback_wav_path.clone())?;

    let mut actor = StateActor::new(cfg, bridge, subtitle, tts, audio);

    tokio::select! {
        res = actor.run() => {
            res?;
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown");
        }
    }

    Ok(())
}
