use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechPriority {
    P0Safety,
    P1ChatReply,
    P2Commentary,
}

impl SpeechPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            SpeechPriority::P0Safety => "p0_safety",
            SpeechPriority::P1ChatReply => "p1_chat_reply",
            SpeechPriority::P2Commentary => "p2_commentary",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechSource {
    Telemetry,
    Filler,
    ActionSafety,
}

impl SpeechSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SpeechSource::Telemetry => "telemetry",
            SpeechSource::Filler => "filler",
            SpeechSource::ActionSafety => "action_safety",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpeechJob {
    pub job_id: String,
    pub text: String,
    pub priority: SpeechPriority,
    pub source: SpeechSource,
    pub enqueued_ms: u64,
    pub deadline_ms: u64,
    pub dedupe_key: String,
}

#[derive(Clone, Debug)]
pub struct DroppedSpeechJob {
    pub job: SpeechJob,
    pub reason: &'static str,
}

pub struct SpeechQueue {
    p0: VecDeque<SpeechJob>,
    p1: VecDeque<SpeechJob>,
    p2: VecDeque<SpeechJob>,
    max_p0: usize,
    max_p1: usize,
    max_p2: usize,
}

impl SpeechQueue {
    pub fn new(max_p0: usize, max_p1: usize, max_p2: usize) -> Self {
        Self {
            p0: VecDeque::new(),
            p1: VecDeque::new(),
            p2: VecDeque::new(),
            max_p0,
            max_p1,
            max_p2,
        }
    }

    pub fn push(&mut self, job: SpeechJob) -> Option<DroppedSpeechJob> {
        let (q, cap) = self.queue_and_cap_mut(job.priority);
        let dropped = if q.len() >= cap {
            q.pop_front().map(|old| DroppedSpeechJob {
                job: old,
                reason: "queue_overflow",
            })
        } else {
            None
        };
        q.push_back(job);
        dropped
    }

    pub fn drop_expired(&mut self, now_ms: u64) -> Vec<DroppedSpeechJob> {
        let mut out = Vec::new();
        Self::drain_expired(&mut self.p0, now_ms, &mut out);
        Self::drain_expired(&mut self.p1, now_ms, &mut out);
        Self::drain_expired(&mut self.p2, now_ms, &mut out);
        out
    }

    pub fn pop_next(&mut self, now_ms: u64) -> Option<SpeechJob> {
        if let Some(job) = Self::pop_next_from_queue(&mut self.p0, now_ms, true) {
            return Some(job);
        }
        if let Some(job) = Self::pop_next_from_queue(&mut self.p1, now_ms, false) {
            return Some(job);
        }
        Self::pop_next_from_queue(&mut self.p2, now_ms, false)
    }

    fn pop_next_from_queue(
        queue: &mut VecDeque<SpeechJob>,
        now_ms: u64,
        never_expire: bool,
    ) -> Option<SpeechJob> {
        while let Some(job) = queue.pop_front() {
            if never_expire || job.deadline_ms >= now_ms {
                return Some(job);
            }
        }
        None
    }

    fn queue_and_cap_mut(&mut self, p: SpeechPriority) -> (&mut VecDeque<SpeechJob>, usize) {
        match p {
            SpeechPriority::P0Safety => (&mut self.p0, self.max_p0),
            SpeechPriority::P1ChatReply => (&mut self.p1, self.max_p1),
            SpeechPriority::P2Commentary => (&mut self.p2, self.max_p2),
        }
    }

    fn drain_expired(
        queue: &mut VecDeque<SpeechJob>,
        now_ms: u64,
        out: &mut Vec<DroppedSpeechJob>,
    ) {
        let mut keep = VecDeque::new();
        while let Some(job) = queue.pop_front() {
            let never_expire = job.priority == SpeechPriority::P0Safety;
            if !never_expire && job.deadline_ms < now_ms {
                out.push(DroppedSpeechJob {
                    job,
                    reason: "deadline_expired",
                });
            } else {
                keep.push_back(job);
            }
        }
        *queue = keep;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, p: SpeechPriority, deadline_ms: u64) -> SpeechJob {
        SpeechJob {
            job_id: id.to_string(),
            text: id.to_string(),
            priority: p,
            source: SpeechSource::Telemetry,
            enqueued_ms: 0,
            deadline_ms,
            dedupe_key: id.to_string(),
        }
    }

    #[test]
    fn keeps_priority_order() {
        let mut q = SpeechQueue::new(8, 8, 8);
        q.push(job("p2", SpeechPriority::P2Commentary, 10));
        q.push(job("p0", SpeechPriority::P0Safety, 10));
        q.push(job("p1", SpeechPriority::P1ChatReply, 10));

        assert_eq!(q.pop_next(0).unwrap().job_id, "p0");
        assert_eq!(q.pop_next(0).unwrap().job_id, "p1");
        assert_eq!(q.pop_next(0).unwrap().job_id, "p2");
    }

    #[test]
    fn drops_expired_non_p0() {
        let mut q = SpeechQueue::new(8, 8, 8);
        q.push(job("p2", SpeechPriority::P2Commentary, 1));
        q.push(job("p0", SpeechPriority::P0Safety, 1));
        let dropped = q.drop_expired(2);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].job.job_id, "p2");
        assert_eq!(q.pop_next(2).unwrap().job_id, "p0");
    }
}
