# /// script
# dependencies = ["websockets"]
# ///
"""Prints the Discord page's console messages that contain a filter string, with
a timestamp, until interrupted. Vesktop must run with --remote-debugging-port=9222.

uv run console.py ObsDiscordLive"""
import asyncio, json, sys, time, urllib.request, websockets

async def main(needle):
    targets = json.load(urllib.request.urlopen("http://127.0.0.1:9222/json"))
    page = next(t for t in targets if t["type"] == "page" and "discord.com" in t["url"])
    async with websockets.connect(page["webSocketDebuggerUrl"], max_size=2**30) as ws:
        await ws.send(json.dumps({"id": 1, "method": "Runtime.enable"}))
        async for raw in ws:
            m = json.loads(raw)
            if m.get("method") != "Runtime.consoleAPICalled":
                continue
            args = m["params"]["args"]
            text = " ".join(str(a.get("value", a.get("description", ""))) for a in args)
            if needle in text:
                print(time.strftime("%H:%M:%S"), m["params"]["type"], text, flush=True)

asyncio.run(main(sys.argv[1] if len(sys.argv) > 1 else ""))
