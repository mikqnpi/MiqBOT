import asyncio
from pathlib import Path

from obs_common import ObsClient, load_properties, visible_char_count, wrap_fixed


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
