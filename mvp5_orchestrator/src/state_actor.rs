use crate::action_client;
use crate::action_ledger::{ActionLedger, TimeoutKind};
use crate::audio_player::AudioPlayer;
use crate::bridge_client::{BridgeClient, BridgeEvent};
use crate::config::OrchestratorConfig;
use crate::pipeline::run_pipeline;
use crate::speech_queue::{SpeechJob, SpeechPriority, SpeechQueue, SpeechSource};
use crate::subtitle_client::SubtitleClient;
use crate::tts_client::TtsClient;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;
use tracing::{info, warn};
use uuid::Uuid;

pub struct StateActor {
    cfg: OrchestratorConfig,
    bridge: BridgeClient,
    subtitle: SubtitleClient,
    tts: TtsClient,
    audio: AudioPlayer,
    queue: SpeechQueue,
    ledger: ActionLedger,
    t0: Instant,
    last_spoken_ms: u64,
    last_line: Option<String>,
    last_line_ms: u64,
}

impl StateActor {
    pub fn new(
        cfg: OrchestratorConfig,
        bridge: BridgeClient,
        subtitle: SubtitleClient,
        tts: TtsClient,
        audio: AudioPlayer,
    ) -> Self {
        Self {
            queue: SpeechQueue::new(cfg.queue_max_p0, cfg.queue_max_p1, cfg.queue_max_p2),
            ledger: ActionLedger::new(),
            cfg,
            bridge,
            subtitle,
            tts,
            audio,
            t0: Instant::now(),
            last_spoken_ms: 0,
            last_line: None,
            last_line_ms: 0,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(self.cfg.state_tick_ms));
        info!("state actor started");

        loop {
            tokio::select! {
                evt = self.bridge.next_event() => {
                    match evt? {
                        BridgeEvent::Telemetry(frame) => {
                            let now_ms = self.now_ms();
                            let line = self.make_telemetry_line(&frame);
                            self.enqueue_speech(
                                line,
                                SpeechPriority::P2Commentary,
                                SpeechSource::Telemetry,
                                self.cfg.chat_deadline_ms,
                                now_ms,
                            )?;
                        }
                        BridgeEvent::ActionAck(ack) => {
                            self.ledger.on_ack(&ack.request_id, ack.accepted);
                            if !ack.accepted {
                                let now_ms = self.now_ms();
                                let line = format!(
                                    "Action was rejected. reason={}. switching to safe mode.",
                                    ack.reason
                                );
                                self.enqueue_speech(
                                    line,
                                    SpeechPriority::P0Safety,
                                    SpeechSource::ActionSafety,
                                    self.cfg.chat_deadline_ms,
                                    now_ms,
                                )?;
                            }
                        }
                        BridgeEvent::ActionResult(result) => {
                            self.ledger.on_result(&result.request_id);
                            let status = crate::pb::bridge_v1::ActionStatus::from_i32(result.status)
                                .unwrap_or(crate::pb::bridge_v1::ActionStatus::ActionStatusUnspecified);
                            if status != crate::pb::bridge_v1::ActionStatus::ActionStatusOk {
                                let now_ms = self.now_ms();
                                let line = format!(
                                    "Action result status={:?}. prioritizing safe recovery.",
                                    status
                                );
                                self.enqueue_speech(
                                    line,
                                    SpeechPriority::P0Safety,
                                    SpeechSource::ActionSafety,
                                    self.cfg.chat_deadline_ms,
                                    now_ms,
                                )?;
                            }
                        }
                        BridgeEvent::Heartbeat(_hb) => {}
                        BridgeEvent::Closed => {
                            warn!("bridge connection closed");
                            break;
                        }
                    }
                }
                _ = interval.tick() => {
                    self.on_tick().await?;
                }
            }
        }
        Ok(())
    }

    async fn on_tick(&mut self) -> Result<()> {
        let now_ms = self.now_ms();

        for dropped in self.queue.drop_expired(now_ms) {
            self.append_metric_line(&serde_json::json!({
                "event": "speech_dropped",
                "job_id": dropped.job.job_id,
                "text": dropped.job.text,
                "priority": dropped.job.priority.as_str(),
                "source": dropped.job.source.as_str(),
                "dropped_reason": dropped.reason,
            }))?;
        }

        for timeout in self.ledger.poll_timeouts(now_ms) {
            let timeout_label = match timeout.kind {
                TimeoutKind::Ack => "ack_timeout",
                TimeoutKind::Result => "result_timeout",
            };
            let line = format!(
                "Action {} reached {}. sending StopAll.",
                timeout.request_id, timeout_label
            );
            self.enqueue_speech(
                line,
                SpeechPriority::P0Safety,
                SpeechSource::ActionSafety,
                self.cfg.chat_deadline_ms,
                now_ms,
            )?;

            if !action_client::is_allowlisted(crate::pb::bridge_v1::ActionType::ActionTypeStopAll) {
                warn!("stop_all is not allowlisted, skip emergency action send");
                continue;
            }

            let stop_req =
                action_client::build_stop_all_request(&self.cfg.primary_game_agent_id, wall_unix_ms(), 1500);
            let request_id = stop_req.request_id.clone();
            if let Err(err) = self.bridge.send_action_request(stop_req).await {
                warn!(error = %err, request_id = %request_id, "send stop_all failed");
            } else {
                self.ledger.on_sent(
                    request_id,
                    now_ms,
                    self.cfg.action_ack_timeout_ms,
                    self.cfg.action_result_timeout_ms,
                );
            }
        }

        if now_ms.saturating_sub(self.last_spoken_ms) >= self.cfg.silence_gap_ms {
            self.enqueue_speech(
                "Planning the next safe move and checking surroundings.".to_string(),
                SpeechPriority::P2Commentary,
                SpeechSource::Filler,
                self.cfg.filler_deadline_ms,
                now_ms,
            )?;
        }

        let Some(job) = self.queue.pop_next(now_ms) else {
            return Ok(());
        };

        let queue_wait_ms = now_ms.saturating_sub(job.enqueued_ms);
        let silence_gap_ms = now_ms.saturating_sub(self.last_spoken_ms);
        let outcome = run_pipeline(
            &job,
            &self.subtitle,
            &self.tts,
            &self.audio,
            self.cfg.tts_mode(),
        )
        .await?;

        self.last_spoken_ms = self.now_ms();

        self.append_metric_line(&serde_json::json!({
            "event": "speech_pipeline",
            "job_id": job.job_id,
            "text": job.text,
            "priority": job.priority.as_str(),
            "source": job.source.as_str(),
            "ttft_ms": outcome.ttft_ms,
            "tts_total_ms": outcome.tts_total_ms,
            "subtitle_show_s": outcome.subtitle_show_s,
            "subtitle_request_id": outcome.subtitle_request_id,
            "subtitle_visible_chars": outcome.subtitle_visible_chars,
            "subtitle_wrapped": outcome.subtitle_wrapped,
            "silence_gap_ms": silence_gap_ms,
            "queue_wait_ms": queue_wait_ms,
            "pipeline_latency_ms": outcome.pipeline_latency_ms,
            "audio_path": outcome.audio_path.display().to_string(),
        }))?;

        Ok(())
    }

    fn enqueue_speech(
        &mut self,
        text: String,
        priority: SpeechPriority,
        source: SpeechSource,
        deadline_delta_ms: u64,
        now_ms: u64,
    ) -> Result<()> {
        let dedupe_key = normalize_dedupe_key(&text);
        if let Some(last) = &self.last_line {
            if *last == dedupe_key
                && now_ms.saturating_sub(self.last_line_ms) < self.cfg.duplicate_cooldown_ms
            {
                return Ok(());
            }
        }

        self.last_line = Some(dedupe_key.clone());
        self.last_line_ms = now_ms;

        let job = SpeechJob {
            job_id: Uuid::new_v4().to_string(),
            text,
            priority,
            source,
            enqueued_ms: now_ms,
            deadline_ms: now_ms.saturating_add(deadline_delta_ms),
            dedupe_key,
        };

        if let Some(dropped) = self.queue.push(job) {
            self.append_metric_line(&serde_json::json!({
                "event": "speech_dropped",
                "job_id": dropped.job.job_id,
                "text": dropped.job.text,
                "priority": dropped.job.priority.as_str(),
                "source": dropped.job.source.as_str(),
                "dropped_reason": dropped.reason,
            }))?;
        }
        Ok(())
    }

    fn make_telemetry_line(&self, telemetry: &crate::pb::bridge_v1::TelemetryFrame) -> String {
        let dim = match crate::pb::bridge_v1::Dimension::from_i32(telemetry.dimension)
            .unwrap_or(crate::pb::bridge_v1::Dimension::DimensionUnspecified)
        {
            crate::pb::bridge_v1::Dimension::DimensionOverworld => "overworld",
            crate::pb::bridge_v1::Dimension::DimensionNether => "nether",
            crate::pb::bridge_v1::Dimension::DimensionEnd => "end",
            crate::pb::bridge_v1::Dimension::DimensionOther => "other",
            crate::pb::bridge_v1::Dimension::DimensionUnspecified => "unknown",
        };
        format!(
            "Current dimension={}, hp={}, hunger={}. moving with caution.",
            dim, telemetry.hp, telemetry.hunger
        )
    }

    fn append_metric_line(&self, value: &serde_json::Value) -> Result<()> {
        if let Some(parent) = std::path::Path::new(&self.cfg.metrics_jsonl_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create metrics dir: {}", parent.display()))?;
            }
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.cfg.metrics_jsonl_path)
            .with_context(|| format!("open metrics file: {}", self.cfg.metrics_jsonl_path))?;
        writeln!(f, "{}", serde_json::to_string(value).context("serialize metrics")?)
            .context("write metrics line")?;
        Ok(())
    }

    fn now_ms(&self) -> u64 {
        self.t0.elapsed().as_millis() as u64
    }
}

fn normalize_dedupe_key(text: &str) -> String {
    text.split_whitespace().collect::<String>()
}

fn wall_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
