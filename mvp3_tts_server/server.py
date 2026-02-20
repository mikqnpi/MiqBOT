import base64
import io
import math
import os
import time
import wave
from dataclasses import dataclass
from typing import List, Optional

from fastapi import FastAPI
from fastapi.responses import JSONResponse, Response
from pydantic import BaseModel, Field

app = FastAPI(title="MiqBOT TTS Server (MVP-9)")


class TtsRequest(BaseModel):
    text: str = Field(..., min_length=1, max_length=4000)
    sample_rate_hz: int = Field(48000, ge=8000, le=48000)
    engine: Optional[str] = Field(default=None)


@dataclass
class SynthesisResult:
    wav_bytes: bytes
    ttft_ms: int
    total_ms: int
    utterances: List[str]
    chunks: List[str]


class TtsEngine:
    def synthesize(self, text: str, sample_rate_hz: int) -> SynthesisResult:
        raise NotImplementedError


class StubEngine(TtsEngine):
    def __init__(self, freq_hz: float) -> None:
        self.freq_hz = freq_hz

    def synthesize(self, text: str, sample_rate_hz: int) -> SynthesisResult:
        started = time.perf_counter()

        n_chars = max(1, len(text))
        duration_s = min(4.0, 0.15 + 0.03 * n_chars)
        amp = 0.2
        n_samples = int(sample_rate_hz * duration_s)

        # MVP-9 placeholder timing model for TTFT/total metrics.
        ttft_ms = int(min(300, 40 + n_chars * 2))

        buf = io.BytesIO()
        with wave.open(buf, "wb") as wf:
            wf.setnchannels(1)
            wf.setsampwidth(2)
            wf.setframerate(sample_rate_hz)

            frames = bytearray()
            for i in range(n_samples):
                t = i / sample_rate_hz
                s = amp * math.sin(2.0 * math.pi * self.freq_hz * t)
                v = int(max(-1.0, min(1.0, s)) * 32767)
                frames.extend(v.to_bytes(2, byteorder="little", signed=True))
            wf.writeframes(frames)

        total_ms = int((time.perf_counter() - started) * 1000)
        utterances = [seg for seg in split_sentences(text) if seg]
        chunks = [seg[:13] for seg in utterances] if utterances else [text[:13]]

        return SynthesisResult(
            wav_bytes=buf.getvalue(),
            ttft_ms=max(ttft_ms, 1),
            total_ms=max(total_ms, 1),
            utterances=utterances or [text],
            chunks=chunks,
        )


def split_sentences(text: str) -> List[str]:
    out: List[str] = []
    current = []
    seps = set(".!?\n")
    for ch in text:
        current.append(ch)
        if ch in seps:
            seg = "".join(current).strip()
            if seg:
                out.append(seg)
            current = []
    if current:
        seg = "".join(current).strip()
        if seg:
            out.append(seg)
    return out


def build_engine(mode: str) -> TtsEngine:
    mode = mode.strip().lower()
    if mode == "qwen_base":
        # Placeholder until Qwen runtime integration is wired.
        return StubEngine(freq_hz=460.0)
    if mode == "qwen_voice_design":
        return StubEngine(freq_hz=490.0)
    if mode == "qwen_custom_voice":
        return StubEngine(freq_hz=520.0)
    return StubEngine(freq_hz=440.0)


DEFAULT_ENGINE = os.getenv("MIQBOT_TTS_ENGINE", "stub")


def get_engine(req: TtsRequest) -> TtsEngine:
    return build_engine(req.engine or DEFAULT_ENGINE)


@app.post("/v1/tts")
def tts(req: TtsRequest) -> Response:
    res = get_engine(req).synthesize(req.text, req.sample_rate_hz)
    return Response(content=res.wav_bytes, media_type="audio/wav")


@app.post("/v1/tts_with_meta")
def tts_with_meta(req: TtsRequest) -> JSONResponse:
    res = get_engine(req).synthesize(req.text, req.sample_rate_hz)
    return JSONResponse(
        {
            "ttft_ms": res.ttft_ms,
            "total_ms": res.total_ms,
            "utterances": res.utterances,
            "chunks": res.chunks,
            "audio_wav_base64": base64.b64encode(res.wav_bytes).decode("ascii"),
        }
    )
