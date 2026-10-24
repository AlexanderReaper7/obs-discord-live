# /// script
# dependencies = ["websockets"]
# ///
"""Writes the scripts of Discord's encryption worker to out/worker-src/, via the page's
CDP session and Debugger.getScriptSource."""
import asyncio, itertools, json, pathlib, urllib.request, websockets

OUT = pathlib.Path(__file__).parent / "out" / "worker-src"

async def main():
    OUT.mkdir(parents=True, exist_ok=True)
    targets = json.load(urllib.request.urlopen("http://127.0.0.1:9222/json"))
    page = next(t for t in targets if t["type"] == "page" and "discord.com" in t["url"])
    ids = itertools.count(1)
    async with websockets.connect(page["webSocketDebuggerUrl"], max_size=2**30) as ws:
        async def call(method, params=None, session=None):
            i = next(ids)
            m = {"id": i, "method": method, "params": params or {}}
            if session:
                m["sessionId"] = session
            await ws.send(json.dumps(m))
            return i
        await call("Target.setAutoAttach", {"autoAttach": True, "waitForDebuggerOnStart": False, "flatten": True})
        names, scripts, pending = {}, {}, {}
        loop = asyncio.get_running_loop()
        end = loop.time() + 10
        while loop.time() < end:
            try:
                m = json.loads(await asyncio.wait_for(ws.recv(), end - loop.time()))
            except asyncio.TimeoutError:
                break
            s = m.get("sessionId")
            if m.get("method") == "Target.attachedToTarget" and m["params"]["targetInfo"]["type"] == "worker":
                ss = m["params"]["sessionId"]
                pending[await call("Runtime.evaluate", {"expression": "self.name", "returnByValue": True}, ss)] = ("name", ss)
            elif m.get("id") in pending:
                kind, ss = pending.pop(m["id"])
                if kind == "name":
                    names[ss] = m.get("result", {}).get("result", {}).get("value")
                    if names[ss] == "encryption-worker" and "done" not in names:
                        names["done"] = ss
                        await call("Debugger.enable", {}, ss)
                elif kind == "src":
                    sid, url = scripts[m["id"]]
                    src = m.get("result", {}).get("scriptSource", "")
                    f = OUT / f"{sid}.js"
                    f.write_text(src)
                    print(f.name, len(src), url)
            elif m.get("method") == "Debugger.scriptParsed" and s == names.get("done"):
                p = m["params"]
                i = await call("Debugger.getScriptSource", {"scriptId": p["scriptId"]}, s)
                pending[i] = ("src", s)
                scripts[i] = (p["scriptId"], p.get("url", ""))

asyncio.run(main())
