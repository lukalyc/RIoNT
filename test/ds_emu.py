"""Driver-Station emulator: a second NT4 CLIENT on the fake roboRIO,
publishing FMSInfo like the real DS does over USB tether.

Run: python test/ds_emu.py [port]
"""
import sys
import threading
import time

import ntcore

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 5815

ds = ntcore.NetworkTableInstance.create()
ds.startClient4("DriverStation")
ds.setServer("127.0.0.1", PORT)
end = time.time() + 15
while not ds.isConnected() and time.time() < end:
    time.sleep(0.1)
print("DS connected:", ds.isConnected(), flush=True)
if not ds.isConnected():
    sys.exit(1)

fms = ds.getTable("FMSInfo")
fms.getStringTopic("EventName").publish().set("CHS District")
fms.getBooleanTopic("IsRedAlliance").publish().set(True)
fms.getIntegerTopic("MatchNumber").publish().set(42)
match_time = fms.getDoubleTopic("MatchTime").publish()

import os
quiet = os.environ.get("DS_QUIET") == "1"
n = 0
while True:
    n += 1
    if not quiet:
        match_time.set(max(0.0, 135.0 - n * 0.05))
        time.sleep(0.05)
    else:
        # DS idle: no changing values at all (static telemetry).
        time.sleep(0.05)
