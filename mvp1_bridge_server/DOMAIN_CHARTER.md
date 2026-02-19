# DOMAIN CHARTER: Bridge Server MVP

## Purpose / Capability
Provide secure transport and canonical protocol handling for game telemetry and control envelopes.

## Ubiquitous Language
- Bridge Server
- Session
- Envelope
- Hello handshake
- Telemetry frame
- Ack sequence

## Owned Data + Invariants
- Owns websocket session state (`session_id`, local `seq`, observed peer `seq`).
- Accepts only protocol version `1` envelopes.
- Enforces max websocket frame/message size limit.
- Requires `Hello` frame as first payload within timeout.

## Public API
- WSS endpoint: `wss://<bind_addr>:40100`
- Payload format: `proto/MiqBOT_bridge_v1.proto`

## Inbound / Outbound
- Inbound: WSS binary protobuf envelopes from authenticated clients.
- Outbound: WSS binary protobuf envelopes (`Hello`, `TimeSyncResponse`, `ErrorFrame`).

## Canonical Contract
- Source of truth: `proto/MiqBOT_bridge_v1.proto`

## Explicitly Out of Scope
- Learning logic
- Action planning logic
- TikTok / OBS integration
