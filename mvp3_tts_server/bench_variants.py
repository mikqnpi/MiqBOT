import json
import os
import sys
import time
import traceback
import urllib.request
from pathlib import Path
from typing import Any

CASES = [
    "Collect wood and set up a starter base.",
    "Health is low, back off and recover now.",
    "Check gear before entering the nether.",
    "Throw an eye and decide the route.",
]

VARIANTS = ["stub", "qwen_base", "qwen_voice_design", "qwen_custom_voice"]


def post_json(url: str, payload: dict[str, Any]) -> dict[str, Any]:
    req = urllib.request.Request(
        url=url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode("utf-8"))


def current_rss_kb() -> int | None:
    try:
        import resource
    except ImportError:
        return None

    usage = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if sys.platform == "darwin":
        return int(usage / 1024)
    return int(usage)


def append_jsonl(path: Path, record: dict[str, Any]) -> None:
    with path.open("a", encoding="utf-8") as f:
        f.write(json.dumps(record, ensure_ascii=True) + "\n")


def main() -> None:
    out_path = Path(os.getenv("MIQBOT_TTS_BENCH_OUT", "bench_results.jsonl"))
    base_url = os.getenv("MIQBOT_TTS_BENCH_URL", "http://127.0.0.1:40300")

    total = 0
    failed = 0
    started_unix_ms = int(time.time() * 1000)

    for variant in VARIANTS:
        for text in CASES:
            total += 1
            request_started = time.perf_counter()
            record: dict[str, Any] = {
                "event": "tts_bench_case",
                "variant": variant,
                "text": text,
                "sample_rate_hz": 48000,
                "started_unix_ms": int(time.time() * 1000),
                "success": False,
                "ttft_ms": None,
                "total_ms": None,
                "request_elapsed_ms": None,
                "failure": None,
                "rss_kb": None,
                "chunks": 0,
                "utterances": 0,
                "audio_bytes_b64_len": 0,
            }

            try:
                body = post_json(
                    f"{base_url}/v1/tts_with_meta",
                    {"text": text, "sample_rate_hz": 48000, "engine": variant},
                )
                record["success"] = True
                record["ttft_ms"] = body.get("ttft_ms")
                record["total_ms"] = body.get("total_ms")
                record["chunks"] = len(body.get("chunks", []))
                record["utterances"] = len(body.get("utterances", []))
                record["audio_bytes_b64_len"] = len(body.get("audio_wav_base64", ""))
            except (OSError, ValueError, TimeoutError) as exc:
                failed += 1
                record["failure"] = f"{type(exc).__name__}: {exc}"
                record["traceback"] = traceback.format_exc(limit=2)
            finally:
                record["request_elapsed_ms"] = int((time.perf_counter() - request_started) * 1000)
                record["rss_kb"] = current_rss_kb()
                append_jsonl(out_path, record)
                print(record)

    summary = {
        "event": "tts_bench_summary",
        "started_unix_ms": started_unix_ms,
        "finished_unix_ms": int(time.time() * 1000),
        "total_cases": total,
        "failed_cases": failed,
        "failure_rate": (failed / total) if total else 0.0,
    }
    append_jsonl(out_path, summary)
    print(summary)


if __name__ == "__main__":
    main()
