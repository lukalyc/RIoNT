"""Fake roboRIO running REAL ntcore (pyntcore) — closes the gap between
test/server_rio.py (hand-rolled WS) and actual roboRIO traffic.

Emulates the reported robot: swerve + shooter + Limelight botpose +
StaleCounter, streaming at 20 Hz. NT4 server on port 5815.

Run: conda run -n nt-tui-test python test/server_rio_ntcore.py
"""
import math
import time

import ntcore

inst = ntcore.NetworkTableInstance.getDefault()
inst.startServer("riont-rio-persist.json", "0.0.0.0", 1735, 5815)
print("ntcore roboRIO server on 0.0.0.0:5815", flush=True)

sd = inst.getTable("SmartDashboard")
swerve = inst.getTable("Swerve")
fl = swerve.getSubTable("FrontLeft")
fr = swerve.getSubTable("FrontRight")
fms = inst.getTable("FMSInfo")

shooter_rpm = sd.getDoubleTopic("Shooter RPM").publish()
gyro = sd.getDoubleTopic("Gyro Angle").publish()
stale = sd.getDoubleTopic("StaleCounter").publish()
battery = sd.getDoubleTopic("Battery Voltage").publish()
botpose = sd.getDoubleArrayTopic("botpose_wpiblue").publish()
fl_vel = fl.getDoubleTopic("Velocity").publish()
fl_cur = fl.getDoubleTopic("Current").publish()
fr_vel = fr.getDoubleTopic("Velocity").publish()
fr_cur = fr.getDoubleTopic("Current").publish()
is_red = fms.getBooleanTopic("IsRedAlliance").publish()

# Second client mimicking the Driver Station: over USB tether the DS is
# always connected too, publishing FMSInfo into the robot's server. The
# DS emulation lives in test/ds_emu.py (a separate process — a second
# ntcore instance inside this one proved flaky).

t0 = time.time()
n = 0
import os
quiet = os.environ.get("RIO_QUIET") == "1"
while True:
    t = time.time() - t0
    n += 1
    if quiet and n > 5:
        # Static-telemetry simulation: stop publishing changed values
        # (NT4 pushes only changes). The ntcore server stays alive and
        # keeps answering RTT echoes — CODE must read RUNNING.
        time.sleep(0.05)
        continue
    shooter_rpm.set(4600.0 + 100.0 * math.sin(t * 3))
    gyro.set(math.degrees(math.sin(t) * 0.5))
    battery.set(12.6 - 0.02 * math.sin(t))
    botpose.set([2.7, 3.9, 0.0, 0.0, 0.0, 90.0 + t])
    fl_vel.set(2.7 + 0.1 * math.sin(t * 5))
    fl_cur.set(16.0 + 8.0 * math.sin(t * 2))
    fr_vel.set(2.65 + 0.1 * math.cos(t * 5))
    fr_cur.set(18.0 + 8.0 * math.cos(t * 2))
    if n % 25 == 0:
        stale.set(float(n // 25))
        is_red.set(True)
    time.sleep(0.05)
