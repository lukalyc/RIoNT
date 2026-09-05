"""Fake robot: real WPILib ntcore NT4 server on 0.0.0.0:5810.

Publishes realistic FRC telemetry so the TUI can be exercised end to end.
Run:  conda run -n nt-tui-test python test/server.py
"""
import math
import time

import ntcore

inst = ntcore.NetworkTableInstance.getDefault()
inst.startServer("riont-test-persist.json", "0.0.0.0", 1735, 5814)
print("NT4 server on 0.0.0.0:5814", flush=True)

sd = inst.getTable("SmartDashboard")
swerve = inst.getTable("Swerve")
fl = swerve.getSubTable("FrontLeft")
fr = swerve.getSubTable("FrontRight")

t0 = time.time()
stale_counter = 0
SLOW_EVERY = 20  # ticks: 2 Hz
N = 0

while True:
    t = time.time() - t0
    N += 1

    # fast streams (20 Hz)
    sd.putNumber("Battery Voltage", 12.6 - 0.02 * math.sin(t * 0.7) - t * 0.001)
    sd.putNumber("Shooter RPM", 4500 + 120 * math.sin(t * 3.0))
    sd.putNumber("Gyro Angle", (t * 45.0) % 360.0)
    fl.putNumber("Velocity", 3.1 + 0.4 * math.sin(t * 2.0))
    fl.putNumber("Current", 18.0 + 6.0 * math.sin(t * 5.0))
    fr.putNumber("Velocity", 3.0 + 0.4 * math.sin(t * 2.0 + 0.3))
    fr.putNumber("Current", 17.5 + 6.0 * math.sin(t * 5.0 + 0.3))

    # static-ish controls (writable: user edits these)
    if N == 1:
        sd.putNumber("kP", 0.012)
        sd.putNumber("kI", 0.0001)
        sd.putNumber("kD", 0.003)
        sd.putBoolean("Compressor Enabled", True)
        sd.putString("Alliance", "blue")
        sd.putBoolean("Climb Locked", False)
        fl.putNumber("Module Angle", 42.5)
        swerve.putNumberArray(
            "Module Velocities", [3.1, 3.0, 2.9, 3.2]
        )
        swerve.putBooleanArray("Wheel Faults", [False, False, False, True])
        swerve.putStringArray("Module Names", ["FL", "FR", "RL", "RR"])
        sd.putString("Autonomous Routine", "5 Note Left")

    # slow stream (2 Hz) - tests Hz display
    if N % SLOW_EVERY == 0:
        sd.putNumber("Match Time", max(0.0, 135.0 - t))

    # stale stream: stops updating after 8 s - tests staleness flag
    if t < 8.0:
        stale_counter += 1
        sd.putNumber("StaleCounter", stale_counter)

    # pose sources for the field-card checks: an exact Limelight name
    # (auto-classifies) and a lookalike that must NOT auto-classify.
    sd.putNumberArray(
        "botpose_wpiblue",
        [2.0 + 0.8 * math.sin(t * 0.8), 4.105 + 0.4 * math.cos(t * 0.8), 0.0, 0.0, 0.0, 90.0],
    )
    sd.putNumberArray("targetpose", [1.0, 2.0, 0.0, 0.0, 0.0, 45.0])

    time.sleep(0.05)
