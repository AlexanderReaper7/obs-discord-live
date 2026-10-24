# /// script
# dependencies = ["websockets"]
# ///
"""Phase 0 CDP driver: `inject` runs probe.js in Vesktop's Discord page, `dump` writes
window.__p0 to a JSON file. Vesktop must run with --remote-debugging-port=9222."""
import asyncio, json, pathlib, sys, urllib.request, websockets

async def evaluate(expr):
    targets = json.load(urllib.request.urlopen("http://127.0.0.1:9222/json"))
    page = next(t for t in targets if t["type"] == "page" and "discord.com" in t["url"])
    async with websockets.connect(page["webSocketDebuggerUrl"], max_size=2**30) as ws:
        await ws.send(json.dumps({"id": 1, "method": "Runtime.evaluate",
                                  "params": {"expression": expr, "returnByValue": True, "awaitPromise": True}}))
        while True:
            m = json.loads(await ws.recv())
            if m.get("id") == 1:
                return m["result"]

cmd = sys.argv[1]
if cmd == "inject":
    print(asyncio.run(evaluate((pathlib.Path(__file__).parent / "probe.js").read_text())))
elif cmd == "dump":
    r = asyncio.run(evaluate("JSON.stringify(window.__p0 ?? null)"))
    pathlib.Path(sys.argv[2]).write_text(r["result"]["value"])
    print("wrote", sys.argv[2])
