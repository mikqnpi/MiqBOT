use std::collections::HashMap;

#[derive(Clone, Debug)]
struct InflightAction {
    ack_deadline_ms: u64,
    result_deadline_ms: u64,
    acked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeoutKind {
    Ack,
    Result,
}

#[derive(Clone, Debug)]
pub struct TimeoutEvent {
    pub request_id: String,
    pub kind: TimeoutKind,
}

pub struct ActionLedger {
    inflight: HashMap<String, InflightAction>,
}

impl ActionLedger {
    pub fn new() -> Self {
        Self {
            inflight: HashMap::new(),
        }
    }

    pub fn on_sent(
        &mut self,
        request_id: String,
        now_ms: u64,
        ack_timeout_ms: u64,
        result_timeout_ms: u64,
    ) {
        self.inflight.insert(
            request_id,
            InflightAction {
                ack_deadline_ms: now_ms.saturating_add(ack_timeout_ms),
                result_deadline_ms: now_ms.saturating_add(result_timeout_ms),
                acked: false,
            },
        );
    }

    pub fn on_ack(&mut self, request_id: &str, accepted: bool) {
        if !accepted {
            self.inflight.remove(request_id);
            return;
        }
        if let Some(v) = self.inflight.get_mut(request_id) {
            v.acked = true;
        }
    }

    pub fn on_result(&mut self, request_id: &str) {
        self.inflight.remove(request_id);
    }

    pub fn poll_timeouts(&mut self, now_ms: u64) -> Vec<TimeoutEvent> {
        let mut timed_out = Vec::new();
        let mut remove_keys = Vec::new();
        for (request_id, inflight) in &self.inflight {
            if !inflight.acked && now_ms >= inflight.ack_deadline_ms {
                timed_out.push(TimeoutEvent {
                    request_id: request_id.clone(),
                    kind: TimeoutKind::Ack,
                });
                remove_keys.push(request_id.clone());
                continue;
            }
            if now_ms >= inflight.result_deadline_ms {
                timed_out.push(TimeoutEvent {
                    request_id: request_id.clone(),
                    kind: TimeoutKind::Result,
                });
                remove_keys.push(request_id.clone());
            }
        }
        for key in remove_keys {
            self.inflight.remove(&key);
        }
        timed_out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_timeout_is_reported_and_removed() {
        let mut ledger = ActionLedger::new();
        ledger.on_sent("req-1".to_string(), 100, 50, 500);

        let events = ledger.poll_timeouts(151);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, "req-1");
        assert_eq!(events[0].kind, TimeoutKind::Ack);

        let events_after = ledger.poll_timeouts(9999);
        assert!(events_after.is_empty());
    }

    #[test]
    fn result_timeout_requires_ack_or_elapsed_deadline() {
        let mut ledger = ActionLedger::new();
        ledger.on_sent("req-2".to_string(), 0, 50, 100);
        ledger.on_ack("req-2", true);

        assert!(ledger.poll_timeouts(99).is_empty());

        let events = ledger.poll_timeouts(101);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, "req-2");
        assert_eq!(events[0].kind, TimeoutKind::Result);
    }
}
