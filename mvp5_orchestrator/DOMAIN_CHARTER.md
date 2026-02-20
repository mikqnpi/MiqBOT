# DOMAIN CHARTER: Orchestrator MVP

## Purpose / Capability
Consume game telemetry and generate continuous stream-safe speech events that drive TTS and OBS subtitles.

## Ubiquitous Language
- Orchestrator
- Speech policy
- Silence gap
- Telemetry context line
- Pipeline latency

## Owned Data + Invariants
- Owns single-writer StateActor state (`last_spoken_ms`, dedupe, silence control, speech queue).
- Owns pipeline metrics output (`TTFT`, `subtitle_show_s`, `silence_gap_ms`, `pipeline_latency_ms`).
- Maintains minimum speaking cadence to avoid >1.2s silence when running.
- Tracks action inflight ledger and emits emergency `STOP_ALL` on ack/result timeouts.

## Public API
- Runtime config: `mvp5_orchestrator/config/orchestrator.toml`
- Inputs: Bridge telemetry envelopes (WSS+mTLS)
- Outputs: TTS HTTP (`/v1/tts` or `/v1/tts_with_meta`), Subtitle HTTP (`/v1/subtitle`), local audio playback, and allowlisted action requests to Bridge

## Inbound / Outbound
- Inbound: telemetry envelopes from Bridge.
- Outbound: subtitle requests, tts requests, local audio output side effects, metrics JSONL.

## Canonical Contract
- Source of truth: `proto/MiqBOT_bridge_v1.proto`

## Explicitly Out of Scope
- Full autonomous action planning/strategy
- Long-term memory/planning
- Qwen3-TTS model hosting and optimization
