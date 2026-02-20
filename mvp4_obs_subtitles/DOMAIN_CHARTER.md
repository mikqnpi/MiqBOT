# DOMAIN CHARTER: OBS Subtitles MVP

## Purpose / Capability
Render sentence-based subtitles into OBS text input using obs-websocket v5.

## Ubiquitous Language
- Subtitle sentence
- Wrap width
- Visible character count
- Minimum display duration
- OBS request

## Owned Data + Invariants
- Applies line wrap limit (default 13 chars/line).
- Applies minimum visibility duration (`chars * 0.25 sec`).
- Clears subtitle text after display interval.

## Public API
- CLI process consuming stdin sentences (`obs_subtitles.py`).
- HTTP gateway:
  - `POST /v1/subtitle`
  - `GET /healthz`
- Config file: `mvp4_obs_subtitles/config.properties`.

## Inbound / Outbound
- Inbound: sentence text from operator/process.
- Outbound: OBS websocket `SetInputSettings` requests.

## Canonical Contract
- obs-websocket v5 protocol operations (Hello/Identify/Request).

## Explicitly Out of Scope
- Speech recognition
- Subtitle translation
- Scene/layout management
