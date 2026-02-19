import io
import math
import wave
from dataclasses import dataclass

from fastapi import FastAPI
from fastapi.responses import Response
from pydantic import BaseModel, Field

app = FastAPI(title="MiqBOT TTS Server (MVP-3)")


class TtsRequest(BaseModel):
    text: str = Field(..., min_length=1, max_length=4000)
    sample_rate_hz: int = Field(48000, ge=8000, le=48000)


@dataclass
class TtsEngine:
    """MVP stub engine; replace implementation with Qwen3-TTS later."""

    def synthesize_wav_bytes(self, text: str, sample_rate_hz: int) -> bytes:
        n_chars = max(1, len(text))
        duration_s = min(3.0, 0.15 + 0.03 * n_chars)

        freq_hz = 440.0
        amp = 0.2
        n_samples = int(sample_rate_hz * duration_s)

        buf = io.BytesIO()
        with wave.open(buf, "wb") as wf:
            wf.setnchannels(1)
            wf.setsampwidth(2)
            wf.setframerate(sample_rate_hz)

            frames = bytearray()
            for i in range(n_samples):
                t = i / sample_rate_hz
                s = amp * math.sin(2.0 * math.pi * freq_hz * t)
                v = int(max(-1.0, min(1.0, s)) * 32767)
                frames.extend(v.to_bytes(2, byteorder="little", signed=True))
            wf.writeframes(frames)

        return buf.getvalue()


engine = TtsEngine()


@app.post("/v1/tts")
def tts(req: TtsRequest) -> Response:
    wav_bytes = engine.synthesize_wav_bytes(req.text, req.sample_rate_hz)
    return Response(content=wav_bytes, media_type="audio/wav")
