import asyncio
import base64
import hashlib
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import websockets


def load_properties(path: Path) -> dict[str, str]:
    props: dict[str, str] = {}
    if not path.exists():
        path.write_text(
            "obs_ws_url=ws://127.0.0.1:4455\n"
            "obs_ws_password=change_me\n"
            "obs_input_name=MiqBOTSubtitle\n"
            "line_max_chars=13\n"
            "min_seconds_per_char=0.25\n",
            encoding="utf-8",
        )

    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        props[key.strip()] = value.strip()

    return props


def compute_obs_auth(password: str, salt: str, challenge: str) -> str:
    secret = hashlib.sha256((password + salt).encode("utf-8")).digest()
    secret_b64 = base64.b64encode(secret).decode("utf-8")
    auth = hashlib.sha256((secret_b64 + challenge).encode("utf-8")).digest()
    return base64.b64encode(auth).decode("utf-8")


def wrap_fixed(text: str, line_max: int) -> str:
    text = text.replace("\r", "").strip()
    if not text:
        return ""

    lines: list[str] = []
    buf = ""

    for ch in text:
        if ch == "\n":
            if buf:
                lines.append(buf)
                buf = ""
            continue

        if len(buf) >= line_max:
            lines.append(buf)
            buf = ""

        buf += ch

    if buf:
        lines.append(buf)

    return "\n".join(lines)


def visible_char_count(text: str) -> int:
    return sum(1 for ch in text if ch not in ["\n", "\r"])


@dataclass
class ObsClient:
    url: str
    password: str
    rpc_version: int = 1
    ws: websockets.WebSocketClientProtocol | None = None
    req_id: int = 0

    async def connect(self) -> None:
        self.ws = await websockets.connect(self.url)
        hello = await self._recv_json()
        if hello.get("op") != 0:
            raise RuntimeError(f"Expected Hello(op=0), got: {hello}")

        auth_data = hello.get("d", {}).get("authentication")
        identify: dict[str, Any] = {"rpcVersion": self.rpc_version, "eventSubscriptions": 0}
        if auth_data and self.password:
            salt = auth_data.get("salt", "")
            challenge = auth_data.get("challenge", "")
            identify["authentication"] = compute_obs_auth(self.password, salt, challenge)

        await self._send_json({"op": 1, "d": identify})
        identified = await self._recv_json()
        if identified.get("op") != 2:
            raise RuntimeError(f"Expected Identified(op=2), got: {identified}")

    async def set_text_input(self, input_name: str, text: str) -> None:
        self.req_id += 1
        request_id = f"req-{self.req_id}"

        payload = {
            "op": 6,
            "d": {
                "requestType": "SetInputSettings",
                "requestId": request_id,
                "requestData": {
                    "inputName": input_name,
                    "inputSettings": {"text": text},
                    "overlay": True,
                },
            },
        }
        await self._send_json(payload)

        res = await self._recv_json()
        if res.get("op") != 7:
            raise RuntimeError(f"Expected RequestResponse(op=7), got: {res}")
        if res.get("d", {}).get("requestId") != request_id:
            raise RuntimeError(f"Mismatched requestId: {res}")

        status = res.get("d", {}).get("requestStatus", {})
        if not status.get("result", False):
            raise RuntimeError(f"OBS request failed: {status}")

    async def close(self) -> None:
        if self.ws:
            await self.ws.close()

    async def _send_json(self, obj: dict[str, Any]) -> None:
        if self.ws is None:
            raise RuntimeError("WebSocket not connected")
        await self.ws.send(json.dumps(obj))

    async def _recv_json(self) -> dict[str, Any]:
        if self.ws is None:
            raise RuntimeError("WebSocket not connected")
        msg = await self.ws.recv()
        return json.loads(msg)


async def main() -> None:
    props = load_properties(Path("config.properties"))
    url = props["obs_ws_url"]
    password = props.get("obs_ws_password", "")
    input_name = props["obs_input_name"]
    line_max = int(props.get("line_max_chars", "13"))
    min_sec_per_char = float(props.get("min_seconds_per_char", "0.25"))

    obs = ObsClient(url=url, password=password)
    await obs.connect()
    print("[OBS] connected. Type lines to show subtitle. Ctrl+C to exit.")

    try:
        while True:
            text = await asyncio.to_thread(input, "> ")
            text = text.strip()
            if not text:
                continue

            wrapped = wrap_fixed(text, line_max)
            chars = visible_char_count(wrapped)
            show_s = max(0.0, chars * min_sec_per_char)

            await obs.set_text_input(input_name, wrapped)
            await asyncio.sleep(show_s)
            await obs.set_text_input(input_name, "")
    except KeyboardInterrupt:
        pass
    finally:
        await obs.close()


if __name__ == "__main__":
    asyncio.run(main())
