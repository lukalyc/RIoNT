"""Probe 2: can RIONT read back a published value to verify a write?

Connection A publishes kP = 0.999. Then:
  1. connection B (separate NT4 client) reports what kP it sees — did the
     write land on the server?
  2. connection A sends a FRESH subscribe for the topic — does the server
     send the current value back on the same connection (despite not
     echoing the change)?

Run:  conda run -n nt-tui-test python test/probe_echo2.py [server_port]
(test/server.py must be running)
"""
import json
import sys
import time

import msgpack
import websocket

PORT = sys.argv[1] if len(sys.argv) > 1 else "5814"
TOPIC = "SmartDashboard/kP"
NEW_VALUE = 0.777

def connect(name):
    ws = websocket.create_connection(
        f"ws://127.0.0.1:{PORT}/nt/{name}",
        subprotocols=["networktables.first.wpi.edu"],
        timeout=0.05,
    )
    ws.send(json.dumps([{
        "method": "subscribe",
        "params": {"topics": [""], "subuid": 1,
                   "options": {"prefix": True, "all": True, "periodic": 0.02}},
    }]))
    return ws

def drain(ws, topic_id_box, seen):
    """Read pending frames; record values for the tracked topic."""
    while True:
        try:
            m = ws.recv()
        except websocket.WebSocketTimeoutException:
            return
        if isinstance(m, str):
            for item in json.loads(m):
                if item.get("method") == "announce":
                    p = item["params"]
                    if p["name"].lstrip("/") == TOPIC:
                        topic_id_box[0] = p["id"]
            continue
        dec = msgpack.Unpacker(raw=False, strict_map_key=False)
        dec.feed(m)
        for arr in dec:
            if isinstance(arr, list) and len(arr) == 4:
                rid, ts, dt, val = arr
                if topic_id_box[0] is not None and rid == topic_id_box[0]:
                    seen.append(val)

a = connect("probe-writer")
time.sleep(0.3)
b = connect("probe-reader")
time.sleep(0.5)

tid_a, tid_b = [None], [None]
a_frames, b_frames = [], []
drain(a, tid_a, a_frames)
drain(b, tid_b, b_frames)
print(f"pre-publish: A sees kP={a_frames[-1] if a_frames else None}, "
      f"B sees kP={b_frames[-1] if b_frames else None}")

# A publishes 0.999
pubuid = 777
a.send(json.dumps([{
    "method": "publish",
    "params": {"name": "/" + TOPIC, "pubuid": pubuid, "type": "double",
               "properties": {}},
}]))
a.send_binary(msgpack.packb([pubuid, int(time.time() * 1e6), 1, NEW_VALUE]))
time.sleep(0.5)

a_frames.clear()
b_frames.clear()
drain(a, tid_a, a_frames)
drain(b, tid_b, b_frames)
print(f"after publish: A received kP frames={a_frames}, "
      f"B sees kP={b_frames[-1] if b_frames else None}")

# A asks again: fresh subscribe for just this topic on the SAME connection.
a.send(json.dumps([{
    "method": "subscribe",
    "params": {"topics": ["/" + TOPIC], "subuid": 2,
               "options": {"prefix": False, "all": True, "periodic": 0.02}},
}]))
time.sleep(1.0)
a_frames.clear()
drain(a, tid_a, a_frames)
print(f"after fresh subscribe (same conn): A received kP frames={a_frames}")

# And B once more, as ground truth of what the server holds.
b_frames.clear()
drain(b, tid_b, b_frames)
print(f"ground truth: B sees kP={b_frames[-1] if b_frames else None}")
