import asyncio
import contextlib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

from fastapi import FastAPI
from fastapi.responses import JSONResponse
from pydantic import BaseModel, Field

from obs_common import ObsClient, load_properties, visible_char_count, wrap_fixed


class SubtitleReq(BaseModel):
    text: str = Field(..., min_length=1, max_length=500)


@dataclass
class GatewayState:
    obs: Optional[ObsClient] = None
    input_name: str = ""
    line_max: int = 13
    min_sec_per_char: float = 0.25
    request_seq: int = 0
    clear_task: Optional[asyncio.Task] = None
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)


app = FastAPI(title="MiqBOT OBS Subtitle Gateway (MVP-5)")
state = GatewayState()


@app.on_event("startup")
async def startup() -> None:
    props = load_properties(Path("config.properties"))
    obs = ObsClient(url=props["obs_ws_url"], password=props.get("obs_ws_password", ""))
    await obs.connect()

    state.obs = obs
    state.input_name = props["obs_input_name"]
    state.line_max = int(props.get("line_max_chars", "13"))
    state.min_sec_per_char = float(props.get("min_seconds_per_char", "0.25"))


@app.on_event("shutdown")
async def shutdown() -> None:
    if state.clear_task and not state.clear_task.done():
        state.clear_task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await state.clear_task

    if state.obs:
        await state.obs.close()


@app.get("/healthz")
async def healthz() -> JSONResponse:
    ok = state.obs is not None and state.obs.ws is not None
    return JSONResponse({"ok": ok})


@app.post("/v1/subtitle")
async def post_subtitle(req: SubtitleReq) -> JSONResponse:
    if state.obs is None:
        raise RuntimeError("OBS client is not initialized")

    wrapped = wrap_fixed(req.text, state.line_max)
    chars = visible_char_count(wrapped)
    show_s = max(0.0, chars * state.min_sec_per_char)

    async with state.lock:
        state.request_seq += 1
        request_id = f"sub-{state.request_seq}"

        if state.clear_task and not state.clear_task.done():
            state.clear_task.cancel()

        await state.obs.set_text_input(state.input_name, wrapped)
        state.clear_task = asyncio.create_task(_clear_after(state.obs, state.input_name, show_s, request_id))

    return JSONResponse(
        {
            "ok": True,
            "request_id": request_id,
            "wrapped": wrapped,
            "visible_chars": chars,
            "show_s": show_s,
        }
    )


async def _clear_after(obs: ObsClient, input_name: str, show_s: float, request_id: str) -> None:
    try:
        await asyncio.sleep(show_s)
        await obs.set_text_input(input_name, "")
    except asyncio.CancelledError:
        return
    except (RuntimeError, ValueError, OSError) as exc:
        print(f"[OBS gateway] clear failed request_id={request_id}: {exc}")
