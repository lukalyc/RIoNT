"""Dump EVERY topic a pyntcore server announces (looking for internal
code-state topics like FRCInfo or $-prefixed diagnostics)."""
import asyncio
import json
import time

import msgpack
import websockets

URI = "ws://127.0.0.1:5814/nt/probe-internal"


async def main():
    async with websockets.connect(URI, subprotocols=["networktables.first.wpi.edu"]) as ws:
        await ws.send(json.dumps([{
            "method": "subscribe",
            "params": {"topics": [""], "subuid": 1,
                        "options": {"prefix": True, "all": True, "periodic": 0.02}},
        }]))
        names = {}
        end = time.time() + 4
        while time.time() < end:
            try:
                m = await asyncio.wait_for(ws.recv(), timeout=1.0)
            except asyncio.TimeoutError:
                continue
            if isinstance(m, str):
                arr = json.loads(m)
                for it in arr if isinstance(arr, list) else [arr]:
                    if isinstance(it, dict) and it.get("method") == "announce":
                        p = it["params"]
                        names[p["name"]] = p.get("type")
        print(f"{len(names)} topics announced")
        special = {k: v for k, v in names.items()
                   if k.startswith("$") or "FRC" in k or "Info" in k}
        print("special topics:", json.dumps(special, indent=1) if special else "NONE")
        print("sample:", sorted(names)[:20])

asyncio.run(main())
