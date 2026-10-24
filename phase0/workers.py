# /// script
# dependencies = ["websockets"]
# ///
"""Evaluates an expression in each dedicated worker of Vesktop's Discord page, through
the page's CDP session (Target.setAutoAttach, flattened)."""
import asyncio, itertools, json, sys, urllib.request, websockets

async def main(expr):
    targets = json.load(urllib.request.urlopen("http://127.0.0.1:9222/json"))
    page = next(t for t in targets if t["type"] == "page" and "discord.com" in t["url"])
    ids = itertools.count(1)
    async with websockets.connect(page["webSocketDebuggerUrl"], max_size=2**30) as ws:
        async def call(method, params=None, session=None):
            i = next(ids)
            msg = {"id": i, "method": method, "params": params or {}}
            if session:
                msg["sessionId"] = session
            await ws.send(json.dumps(msg))
            return i
        await call("Target.setAutoAttach", {"autoAttach": True, "waitForDebuggerOnStart": False, "flatten": True})
        sessions, pending = {}, {}
        loop = asyncio.get_running_loop()
        end = loop.time() + 8
        while loop.time() < end:
            try:
                m = json.loads(await asyncio.wait_for(ws.recv(), end - loop.time()))
            except asyncio.TimeoutError:
                break
            if m.get("method") == "Target.attachedToTarget":
                p = m["params"]
                s = p["sessionId"]
                sessions[s] = p["targetInfo"]
                pending[await call("Runtime.evaluate", {"expression": expr, "returnByValue": True,
                                                        "awaitPromise": True, "timeout": 3000}, s)] = s
            elif m.get("id") in pending:
                s = pending.pop(m["id"])
                r = m.get("result", {}).get("result", {})
                print(sessions[s]["type"], sessions[s]["targetId"][:8], r.get("value", r))
                if not pending:
                    end = min(end, loop.time() + 1)

asyncio.run(main(sys.argv[1]))
