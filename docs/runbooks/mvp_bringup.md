# MVP Bring-up Runbook (Windows-first)

## Prerequisites
- Windows 11 / PowerShell 7+
- Python 3.11+
- Java 21+
- Rust toolchain (for MVP-1 and MVP-5)
- OpenSSL and keytool in PATH
- Minecraft Fabric 1.21.4 client environment
- OBS Studio with obs-websocket v5 enabled

## Step 1: Start MVP-1 Bridge Server
1. Open PowerShell in repository root.
2. Generate mTLS certificates:
   - `cd mvp1_bridge_server`
   - Optional SAN override:
     - `$env:MIQBOT_TLS_DNS="localhost,streampc.local"`
     - `$env:MIQBOT_TLS_IPS="127.0.0.1,192.168.1.10"`
   - `powershell -ExecutionPolicy Bypass -File scripts/gen_certs.ps1`
3. Start server:
   - `cargo run --release`
4. Expected log:
   - `bridge server listening`

## Step 2: Prepare certs for MVP-2 Java client
Run in `mvp1_bridge_server/certs`:
1. `openssl pkcs12 -export -out client.p12 -inkey client.key.pem -in client.crt.pem -passout pass:changeit`
2. `keytool -importcert -noprompt -alias MiqBOT-ca -file ca.crt.pem -keystore ca.p12 -storetype PKCS12 -storepass changeit`
3. Copy `client.p12` and `ca.p12` to ASCII-only path, for example `C:/MiqBOT/certs/`.

## Step 3: Start MVP-2 Fabric client
1. Build mod:
   - `cd mvp2_fabric_mod`
   - `gradle build`
2. Install built mod jar into Minecraft Fabric mods folder.
3. Start Minecraft once to generate `.minecraft/config/MiqBOT-client.properties`.
4. Edit config:
   - `bridge_url=wss://<BridgeHostOrIP>:40100`
   - `client_p12_path=C:/MiqBOT/certs/client.p12`
   - `trust_p12_path=C:/MiqBOT/certs/ca.p12`
5. Relaunch Minecraft.
6. Expected Bridge logs:
   - `hello received`
   - periodic `telemetry` entries.

## Step 4: Start MVP-3 TTS server
1. `cd mvp3_tts_server`
2. `python -m venv .venv`
3. `.venv\Scripts\activate`
4. `pip install -r requirements.txt`
5. `uvicorn server:app --host 127.0.0.1 --port 40300`
6. Test:
   - `curl -X POST http://127.0.0.1:40300/v1/tts -H "Content-Type: application/json" -d "{\"text\":\"hello\",\"sample_rate_hz\":48000}" --output out.wav`

## Step 5: Start MVP-4 OBS subtitle gateway
1. In OBS, create a text input source named `MiqBOTSubtitle`.
2. Confirm obs-websocket v5 is enabled on port 4455.
3. `cd mvp4_obs_subtitles`
4. `python -m venv .venv`
5. `.venv\Scripts\activate`
6. `pip install -r requirements.txt`
7. Set `config.properties` values for OBS URL/password/input name.
8. `uvicorn obs_gateway:app --host 127.0.0.1 --port 48100`
9. Health check:
   - `curl http://127.0.0.1:48100/healthz`

## Step 6: Start MVP-5 Orchestrator
1. `cd mvp5_orchestrator`
2. Confirm `config/orchestrator.toml` paths point to valid bridge cert/key/ca files.
3. `cargo run --release`
4. Expected behavior:
   - Bridge telemetry is received by orchestrator.
   - Subtitle posts are visible in OBS.
   - TTS requests are generated.
   - Local audio playback is attempted; fallback wav is written if playback command is unavailable.
   - `mvp5_metrics.jsonl` gets appended with pipeline metrics.

## Minimal E2E validation
1. Verify startup order: Bridge -> Fabric -> TTS -> OBS gateway -> Orchestrator.
2. Confirm no prolonged silence:
   - Observe continuous subtitle/audio events with gaps generally below ~1.2s.
3. Confirm metrics fields exist per line:
   - `ttft_ms`
   - `subtitle_show_s`
   - `silence_gap_ms`
   - `pipeline_latency_ms`
4. Confirm bridge relay path:
   - Fabric telemetry arrives at Bridge and is forwarded to Orchestrator.

## Integration handoff points
- Bridge contract: `proto/MiqBOT_bridge_v1.proto`
- TTS endpoint: `POST http://127.0.0.1:40300/v1/tts`
- Subtitle endpoint: `POST http://127.0.0.1:48100/v1/subtitle`
- Health endpoint: `GET http://127.0.0.1:48100/healthz`
- Future flow: `Bridge <-> Orchestrator <-> (TTS + OBS + TikTok)`
