use crate::pb::bridge_v1 as pb;
use uuid::Uuid;

pub fn is_allowlisted(action_type: pb::ActionType) -> bool {
    matches!(
        action_type,
        pb::ActionType::ActionTypeStopAll | pb::ActionType::ActionTypeBaritoneGoto
    )
}

pub fn build_stop_all_request(target_agent_id: &str, now_unix_ms: u64, ttl_ms: u64) -> pb::ActionRequest {
    pb::ActionRequest {
        request_id: Uuid::new_v4().to_string(),
        expected_state_version: 0,
        r#type: pb::ActionType::ActionTypeStopAll as i32,
        expires_at_unix_ms: now_unix_ms.saturating_add(ttl_ms),
        idempotency_key: Uuid::new_v4().to_string(),
        target_agent_id: target_agent_id.to_string(),
        r#move: None,
        baritone_goto: None,
    }
}

pub fn build_baritone_goto_request(
    target_agent_id: &str,
    now_unix_ms: u64,
    ttl_ms: u64,
    x: f64,
    y: f64,
    z: f64,
) -> pb::ActionRequest {
    pb::ActionRequest {
        request_id: Uuid::new_v4().to_string(),
        expected_state_version: 0,
        r#type: pb::ActionType::ActionTypeBaritoneGoto as i32,
        expires_at_unix_ms: now_unix_ms.saturating_add(ttl_ms),
        idempotency_key: Uuid::new_v4().to_string(),
        target_agent_id: target_agent_id.to_string(),
        r#move: None,
        baritone_goto: Some(pb::BaritoneGoto {
            x,
            y,
            z,
            max_distance: 200,
            timeout_ms: 15_000,
            stuck_timeout_ms: 5_000,
        }),
    }
}
