# DOMAIN CHARTER: Fabric Game Client MVP

## Purpose / Capability
Collect Minecraft client telemetry and send it to the Bridge over secure transport.

## Ubiquitous Language
- Game Client
- Telemetry collector
- Tick loop
- Bridge URL
- mTLS config

## Owned Data + Invariants
- Owns periodic sampling cadence (10Hz for MVP).
- Maintains per-connection sequence and state version counters.
- Uses latest-only telemetry buffering so game tick thread never blocks on network I/O.
- Emits telemetry values normalized to contract ranges.

## Public API
- Client config file: `.minecraft/config/MiqBOT-client.properties`
- Outbound envelope stream to Bridge endpoint.

## Inbound / Outbound
- Inbound: local Minecraft state + bridge configuration.
- Outbound: `Hello` and `TelemetryFrame` envelopes.

## Canonical Contract
- Source of truth: `proto/MiqBOT_bridge_v1.proto`

## Explicitly Out of Scope
- Baritone/path planning
- Action execution loop
- Persistent storage
