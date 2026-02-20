mod audio_player;
mod bridge_client;
mod config;
mod pb;
mod speech_policy;
mod subtitle_client;
mod tts_client;

use anyhow::{Context, Result};
use audio_player::AudioPlayer;
use bridge_client::BridgeClient;
use config::OrchestratorConfig;
use speech_policy::SpeechPolicy;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;
use subtitle_client::SubtitleClient;
use tracing::{info, warn};
use tts_client::TtsClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let cfg_path =
        std::env::var("MIQBOT_ORCH_CONFIG").unwrap_or_else(|_| "config/orchestrator.toml".to_string());
    let cfg = OrchestratorConfig::load(&cfg_path)?;

    let mut bridge = BridgeClient::connect(&cfg.bridge_url, &cfg.agent_id, &cfg.client_version, &cfg.tls)
        .await
        .context("connect bridge")?;

    let tts = TtsClient::new(cfg.tts_url.clone());
    let subtitle = SubtitleClient::new(cfg.subtitle_url.clone());
    let audio = AudioPlayer::new(cfg.audio_output_dir.clone(), cfg.fallback_wav_path.clone())?;

    let mut speech_policy = SpeechPolicy::new(cfg.silence_gap_ms, cfg.duplicate_cooldown_ms);
    let t0 = Instant::now();
    let mut last_spoken_ms = 0_u64;

    info!("orchestrator started");

    loop {
        tokio::select! {
            telemetry = bridge.next_telemetry() => {
                match telemetry? {
                    Some(frame) => {
                        let now_ms = mono_ms(&t0);
                        if let Some(text) = speech_policy.line_from_telemetry(now_ms, &frame) {
                            process_speech(
                                &cfg,
                                &tts,
                                &subtitle,
                                &audio,
                                &text,
                                now_ms,
                                last_spoken_ms,
                            ).await?;
                            last_spoken_ms = mono_ms(&t0);
                        }
                    }
                    None => {
                        warn!("bridge connection closed");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(cfg.loop_sleep_ms)) => {
                let now_ms = mono_ms(&t0);
                if let Some(text) = speech_policy.filler_if_needed(now_ms, last_spoken_ms) {
                    process_speech(
                        &cfg,
                        &tts,
                        &subtitle,
                        &audio,
                        &text,
                        now_ms,
                        last_spoken_ms,
                    ).await?;
                    last_spoken_ms = mono_ms(&t0);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown");
                break;
            }
        }
    }

    Ok(())
}

async fn process_speech(
    cfg: &OrchestratorConfig,
    tts: &TtsClient,
    subtitle: &SubtitleClient,
    audio: &AudioPlayer,
    text: &str,
    now_ms: u64,
    last_spoken_ms: u64,
) -> Result<()> {
    let started = Instant::now();

    let subtitle_res = subtitle.post_subtitle(text).await;
    if let Err(err) = &subtitle_res {
        warn!(error = %err, "subtitle post failed");
    }

    let tts_started = Instant::now();
    let wav = tts.synthesize(text).await?;
    let ttft_ms = tts_started.elapsed().as_millis() as u64;

    let output_path = audio.play_or_fallback(&wav)?;
    let pipeline_latency_ms = started.elapsed().as_millis() as u64;
    let silence_gap_ms = now_ms.saturating_sub(last_spoken_ms);

    let (subtitle_show_s, subtitle_req_id, subtitle_wrapped, subtitle_chars) = match subtitle_res {
        Ok(body) => (body.show_s, body.request_id, body.wrapped, body.visible_chars),
        Err(_) => (0.0_f64, String::from(""), String::from(""), 0_u64),
    };

    let metric = serde_json::json!({
        "event": "speech_pipeline",
        "text": text,
        "ttft_ms": ttft_ms,
        "subtitle_show_s": subtitle_show_s,
        "subtitle_request_id": subtitle_req_id,
        "subtitle_visible_chars": subtitle_chars,
        "subtitle_wrapped": subtitle_wrapped,
        "silence_gap_ms": silence_gap_ms,
        "pipeline_latency_ms": pipeline_latency_ms,
        "audio_path": output_path.display().to_string(),
    });

    append_metric_line(&cfg.metrics_jsonl_path, &metric)?;
    info!(text = %text, ttft_ms, pipeline_latency_ms, "speech emitted");
    Ok(())
}

fn append_metric_line(path: &str, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| format!("create metrics dir: {}", parent.display()))?;
        }
    }

    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open metrics file: {path}"))?;
    writeln!(f, "{}", serde_json::to_string(value).context("serialize metrics")?)
        .context("write metrics line")?;
    Ok(())
}

fn mono_ms(t0: &Instant) -> u64 {
    t0.elapsed().as_millis() as u64
}
