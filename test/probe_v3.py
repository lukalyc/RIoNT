"""Probe v3: user-faithful crash repro.

- Config mirrors the operator's: saved targets [Simulation, USB Tether],
  last_target = Simulation (the USB tether entry retargeted to the local
  fake roboRIO so the scenario runs anywhere).
- Sim session pins watchlist cards (the operator's sim session had a
  populated watchlist).
- Retarget goes through the picker's SAVED-TARGET path (arrows + Enter),
  not typed input.
- Fake roboRIO = real ntcore + a second client streaming FMSInfo like the
  Driver Station does over USB.

Run: conda run -n nt-tui-test python test/probe_v3.py [iterations]
"""
import codecs
import os
import socket
import subprocess
import sys
import threading
import time

import pyte

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "riont")
PY = sys.executable
N = int(sys.argv[1]) if len(sys.argv) > 1 else 10

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

COLS, ROWS = 120, 36
CFG = os.path.join(ROOT, "test", "probe-v3-config.json")

with open(CFG, "w") as fh:
    import json
    json.dump({
        "last_target": "127.0.0.1:5814",
        "last_view": [],
        "saved_targets": [
            {"name": "Simulation", "ip": "127.0.0.1:5814"},
            {"name": "USB Tether", "ip": "127.0.0.1:5815"},
        ],
        "presets": {},
    }, fh)


def wait_port(port, timeout=20.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            socket.create_connection(("127.0.0.1", port), timeout=0.5).close()
            return True
        except OSError:
            time.sleep(0.15)
    return False


def run_once(i):
    subprocess.run(["pkill", "-f", "[t]est/server.py"], check=False)
    subprocess.run(["pkill", "-f", "[s]erver_rio_ntcore"], check=False)
    time.sleep(0.4)

    sim = subprocess.Popen([PY, os.path.join(ROOT, "test", "server.py")], cwd=ROOT,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not wait_port(5814):
        sim.kill()
        return "SIM-FAILED"

    env = dict(os.environ, RIONT_HEADLESS="1", RIONT_SIZE=f"{COLS}x{ROWS}",
               RIONT_DEBUG="1", RIONT_CONFIG=CFG)
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.Stream()
    stream.attach(screen)
    dec = codecs.getincrementaldecoder("utf-8")()

    p = subprocess.Popen([EXE], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         env=env, cwd=ROOT)
    err_chunks = []

    def err_reader():
        while True:
            d = p.stderr.read1(65536)
            if not d:
                break
            err_chunks.append(d)

    threading.Thread(target=err_reader, daemon=True).start()
    threading.Thread(target=lambda: [stream.feed(dec.decode(d))
                                     for d in iter(lambda: p.stdout.read1(65536), b"")],
                     daemon=True).start()

    def send(keys):
        tok = {"\r": "RET", "\x1b": "ESC", "\t": "TAB", " ": "SPC"}
        # Arrow words must go through as WHOLE tokens (headless protocol),
        # like the operator pressing an arrow key.
        if keys in ("DOWN", "UP"):
            parts = [keys]
        else:
            parts = [tok.get(c, c) for c in keys]
        p.stdin.write(("\x1f".join(parts) + "\n").encode())
        p.stdin.flush()

    def text():
        return "\n".join("".join(r) for r in screen.display)

    def wait(sub, timeout=15.0):
        end = time.time() + timeout
        while time.time() < end:
            if p.poll() is not None:
                return False
            if sub in text()[:200]:
                return True
            time.sleep(0.05)
        return False

    if not wait("ONLINE"):
        rc = p.poll()
        p.kill()
        sim.kill()
        return f"NEVER-ONLINE rc={rc}"

    # Pin two cards like an operator mid-session would (pose card too).
    for query, marker in (("shooter", "Shooter RPM"), ("botpose", "botpose")):
        send("/")
        time.sleep(0.3)
        send(query)
        time.sleep(0.3)
        send("\r")
        time.sleep(0.3)
        send(" ")
        time.sleep(0.4)
    pinned = "WATCHLIST (2)" in text()
    time.sleep(1.0)

    sim.terminate()
    if not (wait("DISCONNECT") or wait("RECONNECTING")):
        p.kill()
        return "NO-DISCONNECT"

    rio = subprocess.Popen([PY, os.path.join(ROOT, "test", "server_rio_ntcore.py")],
                           cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ds = subprocess.Popen([PY, os.path.join(ROOT, "test", "ds_emu.py")],
                          cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not wait_port(5815):
        rio.kill()
        ds.kill()
        p.kill()
        return "RIO-FAILED"

    # Picker -> saved-target "USB Tether" (arrow down once from Simulation).
    send("c")
    time.sleep(0.3)
    send("DOWN")  # saved-target list selection, like the operator's click
    time.sleep(0.2)
    send("\r")

    end = time.time() + 8.0
    online = False
    while time.time() < end:
        if p.poll() is not None:
            rc = p.returncode
            err = b"".join(err_chunks).decode(errors="replace")
            print(f"\n!!! iteration {i}: RIONT exited rc={rc} (pinned={pinned})", flush=True)
            print("---- stderr ----")
            print(err[-6000:])
            print("---- screen ----")
            print(text())
            rio.kill()
            ds.kill()
            return f"CRASHED rc={rc}"
        if "ONLINE" in text()[:200]:
            online = True
            break
        time.sleep(0.05)
    if online:
        time.sleep(2.0)
        if p.poll() is not None:
            err = b"".join(err_chunks).decode(errors="replace")
            print(f"\n!!! iteration {i}: exited LATE rc={p.returncode}")
            print(err[-6000:])
            rio.kill()
            ds.kill()
            return f"LATE-CRASH rc={p.returncode}"
    p.kill()
    rio.kill()
    ds.kill()
    return ("OK" if online else "NO-REONLINE") + (f" pinned={pinned}" if not pinned else "")


crashes = 0
for i in range(N):
    r = run_once(i)
    print(f"[{i+1}/{N}] {r}", flush=True)
    if "CRASH" in r:
        crashes += 1
subprocess.run(["pkill", "-f", "[t]est/server.py"], check=False)
subprocess.run(["pkill", "-f", "[s]erver_rio_ntcore"], check=False)
subprocess.run(["pkill", "-f", "[d]s_emu"], check=False)
print(f"\n{crashes} crashes in {N} iterations")

# NOTE: test/probe_quiet_code.py replays this probe with RIO_QUIET=1 /
# DS_QUIET=1 by patching the source strings above — reword the replaced
# lines (ROOT assignment, rio/ds Popen lines, the post-online sleep block)
# and update that patcher in the same commit.
