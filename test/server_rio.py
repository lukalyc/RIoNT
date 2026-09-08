"""Fake roboRIO NT4 server: raw websocket server mimicking what a real
robot emits, to reproduce the USB-tether crash RIONT sees against real
hardware (the ntcore-based test/server.py uses epoch timestamps and tame
data; a roboRIO emits FPGA us-since-boot timestamps, struct payloads and
hostile values).

Run:  conda run -n nt-tui-test python test/server_rio.py [port]
"""
import asyncio
import json
import struct
import sys
import time

import msgpack
import websockets

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 5815
T0 = time.time()

def fpga_us():
    # roboRIO FPGA us since boot: SMALL numbers, not epoch.
    return int((time.time() - T0) * 1e6) + 3_600_000_000  # 1h up

TOPICS = [
    # (name, type_str, wire_type_code, value)
    ("/FMSInfo/FRC Network Communications Error", "string", 4, ""),
    ("/FMSInfo/IsRedAlliance", "boolean", 0, False),
    ("/FMSInfo/MatchNumber", "int", 2, -9223372036854775808),
    ("/FMSInfo/MatchTime", "double", 1, float("nan")),
    ("/FMSInfo/GameSpecificMessage", "string", 4, "L3"),
    ("/LiveWindow/Ungrouped/DigitalInput[0]/Value", "boolean", 0, True),
    ("/LiveWindow/Ungrouped/Sensor[3]/Value", "double", 1, float("inf")),
    ("/PathPlanner/activePath", "struct:Pose2d", 5, b""),
    ("/PathPlanner/targetPose", "struct:Pose2d", 5, struct.pack("<3d", 1.5, -2.5, 0.25)),
    ("/PathPlanner/event", "string", 4, "line1\nline2\x07\x1b[31m"),
    ("/ROS2/tf", "msgpack", 5, bytes(range(256))),
    ("/ROS2/joy", "double[]", 17, [0.0] * 64),
    ("/Shuffleboard/Drivetrain/Front Left", "double", 1, -1.0),
    ("/SmartDashboard/Battery Voltage", "double", 1, 12.4),
    ("/SmartDashboard/robot pose", "struct:Pose2d", 5, struct.pack("<3d", 2.0, 4.0, 1.57)),
    ("/SmartDashboard/SwerveStates", "double[]", 17, [1e308, -1e308, 0.0]),
    ("/Tuning/Elevator/KA", "double", 1, 0.45),
    ("/Tuning/Elevator/KP", "double", 1, 3.0),
    ("/Tuning/big string", "string", 4, "x" * 5000),
]

PROPERTIES = {
    "/SmartDashboard/robot pose": {"structSchema": "Pose2d{Translation2d{x:double, y:double}, Rotation2d{radians:double}}"},
    "/PathPlanner/targetPose": {"structSchema": "Pose2d{Translation2d{x:double, y:double}, Rotation2d{radians:double}}"},
    "/PathPlanner/activePath": {"structSchema": "Pose2d{Translation2d{x:double, y:double}, Rotation2d{radians:double}}"},
    "/SmartDashboard/Battery Voltage": {"persistent": True},
}

NAMES = [t[0] for t in TOPICS]

async def stream_values(ws, topic_id):
    """Stream FPGA-timestamped value frames at 20 Hz, like robot code."""
    n = 0
    try:
        while True:
            await asyncio.sleep(0.05)
            n += 1
            ts = fpga_us()
            frames = []
            frames.append(msgpack.packb([topic_id["/SmartDashboard/Battery Voltage"], ts, 1, 12.4 - 0.001 * n]))
            frames.append(msgpack.packb([topic_id["/FMSInfo/MatchTime"], ts, 1, max(0.0, 135.0 - n * 0.05)]))
            frames.append(msgpack.packb([topic_id["/Shuffleboard/Drivetrain/Front Left"], ts, 1, -1.0 + 0.01 * n]))
            frames.append(msgpack.packb([topic_id["/PathPlanner/targetPose"], ts, 5, struct.pack("<3d", 1.5, -2.5, 0.25 + 0.01 * n)]))
            await ws.send(b"".join(frames))
    except Exception:
        pass

async def handle(ws):
    peer = ws.remote_address
    print(f"client connected: {peer}", flush=True)
    topic_id = {name: i + 1 for i, name in enumerate(NAMES)}
    announced = set()
    try:
        async for raw in ws:
            if isinstance(raw, bytes):
                # client binary: value frames / rtt echo — drain silently,
                # but ANSWER the rtt echo like ntcore does.
                data = raw
                while data:
                    try:
                        val, data = msgpack.unpackb(data, raw=False, strict_map_key=False, use_list=True), b""
                    except Exception:
                        break
                    # unpackb consumed all; decode one message
                    break
                dec = msgpack.Unpacker(raw=False, strict_map_key=False)
                dec.feed(raw)
                for arr in dec:
                    if isinstance(arr, list) and len(arr) == 4 and arr[0] == -1:
                        reply = msgpack.packb([-1, fpga_us(), 2, arr[3]])
                        await ws.send(reply)
            else:
                for msg in json.loads(raw):
                    method = msg.get("method")
                    params = msg.get("params", {})
                    if method == "subscribe":
                        # announce everything (real ntcore sends announces
                        # in batches), then a snapshot of values.
                        anns = []
                        for name in NAMES:
                            anns.append(json.dumps({"method": "announce", "params": {
                                "name": name, "id": topic_id[name],
                                "type": dict((t[0], t[1]) for t in TOPICS)[name],
                                "properties": PROPERTIES.get(name, {"persistent": False, "retained": False}),
                            }}))
                        await ws.send("[" + ",".join(anns) + "]")
                        announced = set(NAMES)
                        asyncio.get_event_loop().create_task(stream_values(ws, topic_id))
                    elif method == "publish":
                        pass  # accept silently
    except Exception as e:
        print(f"client gone: {e!r}", flush=True)

async def main():
    async with websockets.serve(handle, "0.0.0.0", PORT,
                                subprotocols=["networktables.first.wpi.edu"]):
        print(f"fake roboRIO NT4 server on 0.0.0.0:{PORT}", flush=True)
        await asyncio.Future()

asyncio.run(main())
