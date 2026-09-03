"""Probe v2: join sim's binary value frames against its announce table."""
import asyncio
import json
import time

import msgpack
import websockets

URI = "ws://127.0.0.1:5810/nt/probe2"
SUB = [{
    "method": "subscribe",
    "params": {
        "topics": [""],
        "subuid": 1,
        "options": {"prefix": True, "all": True, "periodic": 0.02},
    },
}]

async def main():
    async with websockets.connect(URI, subprotocols=["networktables.first.wpi.edu"]) as ws:
        await ws.send(json.dumps(SUB))
        names = {}
        samples = []
        t_end = time.time() + 2.5
        n_val = 0
        while time.time() < t_end:
            try:
                m = await asyncio.wait_for(ws.recv(), timeout=0.5)
            except asyncio.TimeoutError:
                continue
            if isinstance(m, str):
                arr = json.loads(m)
                for it in arr if isinstance(arr, list) else [arr]:
                    if isinstance(it, dict) and it.get("method") == "announce":
                        p = it["params"]
                        names[p["id"]] = (p["name"], p.get("type"))
            else:
                u = msgpack.Unpacker(raw=False)
                u.feed(m)
                for val in u:
                    n_val += 1
                    if len(samples) < 6 and isinstance(val, list) and len(val) == 4:
                        tid = val[0]
                        samples.append((tid, names.get(tid, ("?", "?")), val[1], val[2], repr(val[3])[:60]))
        print(f"values received: {n_val}")
        print("id -> (name, type) | ts | typecode | value")
        for s in samples:
            print(" ", s)
        # type codes seen
        codes = {}
        # second pass quick: just report
        print("announced topics:", len(names))
        from collections import Counter
        print("announced types:", Counter(t for _, t in names.values()).most_common())

asyncio.run(main())
