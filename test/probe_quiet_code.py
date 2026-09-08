"""E2E check for the CODE indicator against STATIC telemetry.

Fake roboRIO with RIO_QUIET=1 + DS emulator with DS_QUIET=1: the robot's
ntcore server is alive (answers RTT pings) but pushes ZERO value frames
(static telemetry — NT4 only sends changes). CODE must read RUNNING
(bug: the old values-freshness heuristic read STOPPED).

Run: conda run -n nt-tui-test python test/probe_quiet_code.py
Exits 2 if CODE wrongly reads STOPPED.
"""
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(ROOT)
sys.argv = ["probe_v3.py", "1"]

src = open(os.path.join(ROOT, "test", "probe_v3.py")).read()
src = src.replace(
    'ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))',
    f'ROOT = "{ROOT}"')
src = src.replace(
    'rio = subprocess.Popen([PY, os.path.join(ROOT, "test", "server_rio_ntcore.py")],',
    'rio = subprocess.Popen([PY, os.path.join(ROOT, "test", "server_rio_ntcore.py")], env=dict(os.environ, RIO_QUIET="1"),')
src = src.replace(
    'ds = subprocess.Popen([PY, os.path.join(ROOT, "test", "ds_emu.py")],',
    'ds = subprocess.Popen([PY, os.path.join(ROOT, "test", "ds_emu.py")], env=dict(os.environ, DS_QUIET="1"),')

# after ONLINE: 4 s of silence, then CODE must still be RUNNING
src = src.replace("""    if online:
        time.sleep(2.0)""", """    if online:
        time.sleep(4.0)
        hud_q = text()[:200]
        print("QUIET HUD:", hud_q[:110], flush=True)
        if "CODE: STOPPED" in hud_q:
            print("FAIL: CODE says STOPPED while server answers pings", flush=True)
            sys.exit(2)
        if "CODE: RUNNING" not in hud_q:
            print("FAIL: CODE neither RUNNING nor STOPPED", flush=True)
            sys.exit(2)""")

exec(src)
