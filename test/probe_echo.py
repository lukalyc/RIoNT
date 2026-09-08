"""Probe: does an ntcore server echo a client's own published value back?

Connects a raw NT4 websocket client, subscribes to everything, publishes a
value to an existing topic (exactly the way RIONT does), and logs every
binary value frame received for that topic for ~3 seconds.

Run:  conda run -n nt-tui-test python test/probe_echo.py [server_port]
(the fake robot test/server.py must be running; needs websocket-client
and msgpack in addition to the harness deps — see CONTRIBUTING.md)
"""
import json
import sys
import time

import msgpack
import websocket

PORT = sys.argv[1] if len(sys.argv) > 1 else "5814"
TOPIC = "SmartDashboard/kP"
NEW_VALUE = 0.999

ws = websocket.create_connection(
    f"ws://127.0.0.1:{PORT}/nt/probe-echo",
    subprotocols=["networktables.first.wpi.edu"],
    timeout=0.2,
)

subscribe = json.dumps(
    [
        {
            "method": "subscribe",
            "params": {
                "topics": [""],
                "subuid": 1,
                "options": {"prefix": True, "all": True, "periodic": 0.02},
            },
        }
    ]
)
ws.send(subscribe)
time.sleep(0.5)

# Declare the publisher and send the value frame, RIONT-style.
pubuid = 4242
publish = json.dumps(
    [
        {
            "method": "publish",
            "params": {
                "name": "/" + TOPIC,
                "pubuid": pubuid,
                "type": "double",
                "properties": {},
            },
        }
    ]
)
ws.send(publish)
frame = msgpack.packb([pubuid, int(time.time() * 1e6), 1, NEW_VALUE])
ws.send_binary(frame)
print(f"published {TOPIC} = {NEW_VALUE} (pubuid {pubuid})")

topic_id = None
got_value = None
t0 = time.time()
while time.time() - t0 < 3.0:
    try:
        m = ws.recv()
    except websocket.WebSocketTimeoutException:
        continue
    if isinstance(m, str):
        for item in json.loads(m):
            if item.get("method") == "announce":
                p = item["params"]
                print("announce:", p["name"], "id", p["id"])
                if p["name"] == TOPIC:
                    topic_id = p["id"]
        continue
    # One binary frame may contain several msgpack messages.
    dec = msgpack.Unpacker(raw=False, strict_map_key=False)
    dec.feed(m)
    for arr in dec:
        if isinstance(arr, list) and len(arr) == 4:
            rid, ts, dt, val = arr
            marker = " <-- OUR TOPIC" if topic_id is not None and rid == topic_id else ""
            print(f"value frame: id={rid} ts={ts} dt={dt} val={val}{marker}")
            if topic_id is not None and rid == topic_id:
                got_value = val

print(
    "RESULT: echo received"
    if got_value == NEW_VALUE
    else f"RESULT: no echo (last value seen for topic: {got_value})"
)
