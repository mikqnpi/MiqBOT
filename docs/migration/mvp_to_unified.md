# MVP to Unified Migration Plan

## Scope
This document defines the expiry plan for the temporary MVP split layout:
- `mvp1_bridge_server`
- `mvp2_fabric_mod`
- `mvp3_tts_server`
- `mvp4_obs_subtitles`

## Why the split exists
The split allows independent bring-up and failure isolation while canonical interfaces are stabilized.

## Expiry (hard deadline)
- Expiry date: 2026-06-30
- Owner: MiqBOT core maintainers
- Tracking task id: MIQBOT-UNIFY-001

## Removal conditions
The MVP split must be removed when all conditions below are true:
1. Orchestrator can run end-to-end with Bridge, TTS, and OBS integrations.
2. Action request path (`ActionRequest` -> execution -> `ActionResult`) is validated in integration tests.
3. Unified repository structure is in place with explicit layer boundaries and CI dependency gates.

## Unified target structure
- `domains/bridge/*`
- `domains/game_client/*`
- `domains/voice/*`
- `domains/streaming/*`
- `contracts/proto/MiqBOT_bridge_v1.proto`

## Migration steps
1. Move source files from `mvp*` folders into target domains.
2. Preserve public interfaces while adapting internal package/module names.
3. Update CI and runbooks to reference unified paths.
4. Remove `mvp*` directories after parity checks pass.

## Verification required before removal
- All CI jobs green (Python, Rust, Java, security gates).
- End-to-end smoke run completed on Windows.
- No duplicate contract definitions remain.

## Notes
Any extension to this temporary layout requires explicit review and update of this document.
