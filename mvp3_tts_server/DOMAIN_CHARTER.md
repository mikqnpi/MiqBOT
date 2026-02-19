# DOMAIN CHARTER: TTS Server MVP

## Purpose / Capability
Expose a stable HTTP text-to-audio interface with fixed sample-rate behavior for pipeline integration.

## Ubiquitous Language
- TTS engine
- Synthesis request
- WAV response
- 48kHz output

## Owned Data + Invariants
- Owns `/v1/tts` request validation.
- Returns WAV (`audio/wav`) with PCM16 and requested sample rate.
- Default sample rate is 48000Hz.

## Public API
- `POST /v1/tts`
  - Request JSON: `text`, `sample_rate_hz`
  - Response: WAV bytes

## Inbound / Outbound
- Inbound: text payload from orchestrator or tooling.
- Outbound: WAV bytes.

## Canonical Contract
- HTTP contract defined in this MVP package.

## Explicitly Out of Scope
- Speaker embeddings
- Multi-model fallback
- Production voice quality guarantees
