# /// script
# dependencies = ["websockets"]
# ///
"""Sends one obs-websocket v5 request and prints the response data as JSON.

uv run obsws.py REQUEST [JSON]
uv run obsws.py shot SOURCE OUT.png    # GetSourceScreenshot at the source's own size

OBS_WS_URL picks the OBS (default ws://127.0.0.1:<port from the local config>),
OBS_WS_PASSWORD_FILE a file holding its password (default: the local config's)."""
import asyncio, base64, hashlib, json, os, pathlib, sys, websockets

CONF = pathlib.Path.home() / ".config/obs-studio/plugin_config/obs-websocket/config.json"

def b64sha(s: str) -> str:
    return base64.b64encode(hashlib.sha256(s.encode()).digest()).decode()

def target():
    if "OBS_WS_URL" in os.environ:
        pw = pathlib.Path(os.environ["OBS_WS_PASSWORD_FILE"]).read_text().strip() if "OBS_WS_PASSWORD_FILE" in os.environ else ""
        return os.environ["OBS_WS_URL"], pw
    conf = json.loads(CONF.read_text())
    return f"ws://127.0.0.1:{conf['server_port']}", conf["server_password"]

async def request(kind, data):
    url, pw = target()
    async with websockets.connect(url, max_size=2**28) as ws:
        hello = json.loads(await ws.recv())["d"]
        ident = {"rpcVersion": 1}
        if "authentication" in hello:
            a = hello["authentication"]
            ident["authentication"] = b64sha(b64sha(pw + a["salt"]) + a["challenge"])
        await ws.send(json.dumps({"op": 1, "d": ident}))
        await ws.recv()
        await ws.send(json.dumps({"op": 6, "d": {"requestType": kind, "requestId": "1", "requestData": data}}))
        while True:
            m = json.loads(await ws.recv())
            if m["op"] == 7:
                break
        r = m["d"]
        if not r["requestStatus"]["result"]:
            sys.exit(json.dumps(r["requestStatus"]))
        return r.get("responseData", {})

def main():
    if sys.argv[1] == "shot":
        r = asyncio.run(request("GetSourceScreenshot", {"sourceName": sys.argv[2], "imageFormat": "png"}))
        pathlib.Path(sys.argv[3]).write_bytes(base64.b64decode(r["imageData"].split(",", 1)[1]))
        print("wrote", sys.argv[3])
    else:
        data = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
        print(json.dumps(asyncio.run(request(sys.argv[1], data)), indent=1))

main()
