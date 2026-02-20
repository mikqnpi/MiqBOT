use anyhow::Result;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

use crate::config::RelayConfig;
use crate::pb::bridge_v1 as pb;

#[derive(Debug)]
pub enum OrchestratorAcquireError {
    NotAllowed,
    LimitReached,
}

#[derive(Clone)]
pub enum ActionRelayFrame {
    Ack(pb::ActionAck),
    Result(pb::ActionResult),
}

pub struct RelayHub {
    relay_cfg: RelayConfig,
    telemetry_tx: watch::Sender<Option<pb::TelemetryFrame>>,
    orchestrator_count: AtomicUsize,
    last_relay_mono_ms: AtomicU64,
    primary_game_sender: Mutex<Option<mpsc::Sender<pb::ActionRequest>>>,
    pending_actions: Mutex<HashMap<String, mpsc::Sender<ActionRelayFrame>>>,
}

impl RelayHub {
    pub fn new(relay_cfg: RelayConfig) -> Arc<Self> {
        let (telemetry_tx, _rx) = watch::channel(None);
        Arc::new(Self {
            relay_cfg,
            telemetry_tx,
            orchestrator_count: AtomicUsize::new(0),
            last_relay_mono_ms: AtomicU64::new(0),
            primary_game_sender: Mutex::new(None),
            pending_actions: Mutex::new(HashMap::new()),
        })
    }

    pub fn subscribe_telemetry(&self) -> watch::Receiver<Option<pb::TelemetryFrame>> {
        self.telemetry_tx.subscribe()
    }

    pub fn action_queue_size(&self) -> usize {
        self.relay_cfg.action_queue_size
    }

    pub fn is_primary_game_agent(&self, agent_id: &str) -> bool {
        agent_id == self.relay_cfg.primary_game_agent_id
    }

    pub fn primary_game_agent_id(&self) -> &str {
        &self.relay_cfg.primary_game_agent_id
    }

    pub fn publish_telemetry(&self, telemetry: &pb::TelemetryFrame) {
        if self.relay_cfg.min_relay_interval_ms > 0 {
            let now = mono_ms();
            let last = self.last_relay_mono_ms.load(Ordering::Relaxed);
            if now.saturating_sub(last) < self.relay_cfg.min_relay_interval_ms {
                return;
            }
            self.last_relay_mono_ms.store(now, Ordering::Relaxed);
        }
        self.telemetry_tx.send_replace(Some(telemetry.clone()));
    }

    pub fn acquire_orchestrator_slot(
        self: &Arc<Self>,
    ) -> std::result::Result<OrchestratorSlot, OrchestratorAcquireError> {
        if !self.relay_cfg.allow_orchestrator_subscribe {
            return Err(OrchestratorAcquireError::NotAllowed);
        }

        loop {
            let current = self.orchestrator_count.load(Ordering::Relaxed);
            if current >= self.relay_cfg.max_orchestrator_subscribers {
                return Err(OrchestratorAcquireError::LimitReached);
            }

            if self
                .orchestrator_count
                .compare_exchange(
                    current,
                    current + 1,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Ok(OrchestratorSlot {
                    hub: Arc::clone(self),
                });
            }
        }
    }

    pub async fn attach_primary_game_sender(
        &self,
        sender: mpsc::Sender<pb::ActionRequest>,
        agent_id: &str,
    ) -> Result<()> {
        if !self.is_primary_game_agent(agent_id) {
            anyhow::bail!("non-primary game agent cannot attach action sender");
        }

        let mut slot = self.primary_game_sender.lock().await;
        if slot.is_some() {
            anyhow::bail!("primary game sender already attached");
        }
        *slot = Some(sender);
        Ok(())
    }

    pub async fn detach_primary_game_sender(&self) {
        let mut slot = self.primary_game_sender.lock().await;
        *slot = None;
        drop(slot);
        self.fail_all_pending("primary game client disconnected").await;
    }

    pub async fn enqueue_action(
        &self,
        req: pb::ActionRequest,
        reply_tx: mpsc::Sender<ActionRelayFrame>,
    ) -> Result<()> {
        if req.request_id.trim().is_empty() {
            anyhow::bail!("request_id must not be empty");
        }

        if !req.target_agent_id.trim().is_empty() && req.target_agent_id != self.relay_cfg.primary_game_agent_id {
            anyhow::bail!(
                "target_agent_id={} does not match primary_game_agent_id={}",
                req.target_agent_id,
                self.relay_cfg.primary_game_agent_id
            );
        }

        let sender_opt = { self.primary_game_sender.lock().await.clone() };
        let Some(primary_sender) = sender_opt else {
            anyhow::bail!("primary game client is not connected");
        };

        let request_id = req.request_id.clone();
        {
            let mut pending = self.pending_actions.lock().await;
            pending.insert(request_id.clone(), reply_tx);
        }

        let send_res = tokio::time::timeout(
            std::time::Duration::from_millis(self.relay_cfg.action_send_timeout_ms),
            primary_sender.send(req),
        )
        .await;

        match send_res {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => {
                self.pending_actions.lock().await.remove(&request_id);
                anyhow::bail!("primary game action queue closed")
            }
            Err(_) => {
                self.pending_actions.lock().await.remove(&request_id);
                anyhow::bail!(
                    "action enqueue timeout after {}ms",
                    self.relay_cfg.action_send_timeout_ms
                )
            }
        }
    }

    pub async fn route_action_ack(&self, ack: &pb::ActionAck) {
        let maybe_reply_tx = {
            let mut pending = self.pending_actions.lock().await;
            if ack.accepted {
                pending.get(&ack.request_id).cloned()
            } else {
                pending.remove(&ack.request_id)
            }
        };

        if let Some(reply_tx) = maybe_reply_tx {
            if reply_tx.send(ActionRelayFrame::Ack(ack.clone())).await.is_err() {
                self.pending_actions.lock().await.remove(&ack.request_id);
            }
        }
    }

    pub async fn route_action_result(&self, result: &pb::ActionResult) {
        let maybe_reply_tx = self.pending_actions.lock().await.remove(&result.request_id);
        if let Some(reply_tx) = maybe_reply_tx {
            let _ = reply_tx.send(ActionRelayFrame::Result(result.clone())).await;
        }
    }

    async fn fail_all_pending(&self, reason: &str) {
        let drained = {
            let mut pending = self.pending_actions.lock().await;
            pending.drain().collect::<Vec<_>>()
        };
        for (request_id, reply_tx) in drained {
            let ack = pb::ActionAck {
                request_id: request_id.clone(),
                accepted: false,
                reason: reason.to_string(),
            };
            let result = pb::ActionResult {
                request_id,
                status: pb::ActionStatus::ActionStatusTimeout as i32,
                detail: reason.to_string(),
                final_state_version: 0,
            };
            let _ = reply_tx.send(ActionRelayFrame::Ack(ack)).await;
            let _ = reply_tx.send(ActionRelayFrame::Result(result)).await;
        }
    }
}

pub struct OrchestratorSlot {
    hub: Arc<RelayHub>,
}

impl Drop for OrchestratorSlot {
    fn drop(&mut self) {
        self.hub.orchestrator_count.fetch_sub(1, Ordering::SeqCst);
    }
}

fn mono_ms() -> u64 {
    use std::sync::OnceLock;
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    let t0 = T0.get_or_init(std::time::Instant::now);
    t0.elapsed().as_millis() as u64
}
