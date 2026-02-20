use crate::audio_player::AudioPlayer;
use crate::config::TtsMode;
use crate::speech_queue::SpeechJob;
use crate::subtitle_client::SubtitleClient;
use crate::tts_client::TtsClient;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;
use tracing::warn;

pub struct PipelineOutcome {
    pub ttft_ms: u64,
    pub tts_total_ms: u64,
    pub subtitle_show_s: f64,
    pub subtitle_request_id: String,
    pub subtitle_visible_chars: u64,
    pub subtitle_wrapped: String,
    pub pipeline_latency_ms: u64,
    pub audio_path: PathBuf,
}

pub async fn run_pipeline(
    job: &SpeechJob,
    subtitle: &SubtitleClient,
    tts: &TtsClient,
    audio: &AudioPlayer,
    tts_mode: TtsMode,
) -> Result<PipelineOutcome> {
    let started = Instant::now();

    let subtitle_res = subtitle.post_subtitle(&job.text).await;
    if let Err(err) = &subtitle_res {
        warn!(error = %err, "subtitle post failed");
    }

    let tts_started = Instant::now();
    let synth = tts.synthesize(&job.text, tts_mode).await?;
    let measured_ttft_ms = tts_started.elapsed().as_millis() as u64;

    let audio_path = audio.play_or_fallback(&synth.wav_bytes)?;
    let pipeline_latency_ms = started.elapsed().as_millis() as u64;

    let (subtitle_show_s, subtitle_req_id, subtitle_wrapped, subtitle_chars) = match subtitle_res {
        Ok(body) => (body.show_s, body.request_id, body.wrapped, body.visible_chars),
        Err(_) => (0.0_f64, String::new(), String::new(), 0_u64),
    };

    Ok(PipelineOutcome {
        ttft_ms: synth.ttft_ms.unwrap_or(measured_ttft_ms),
        tts_total_ms: synth.total_ms.unwrap_or(pipeline_latency_ms),
        subtitle_show_s,
        subtitle_request_id: subtitle_req_id,
        subtitle_visible_chars: subtitle_chars,
        subtitle_wrapped,
        pipeline_latency_ms,
        audio_path,
    })
}
