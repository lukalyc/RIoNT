"""Quick visual probe: 3-column packing with 13+ cards."""
import codecs, json, os, subprocess, sys, threading, time, socket

import pyte

ROOT = r"C:\Users\lryam\Documents\robonauts\software\nt-tui"
EXE = os.path.join(ROOT, "target", "debug", "riont.exe")
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

with open(os.path.join(ROOT, ".nt-views.json"), "w") as fh:
    json.dump([{"name": "Dense", "topics": ["Swerve/*", "SmartDashboard/*"]}], fh)

srv = subprocess.Popen([sys.executable, os.path.join(ROOT, "test", "server.py")], cwd=ROOT,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
end = time.time() + 15
while time.time() < end:
    try:
        socket.create_connection(("127.0.0.1", 5814), timeout=0.5).close()
        break
    except OSError:
        time.sleep(0.2)

COLS, ROWS = 120, 36
env = dict(os.environ, RIONT_HEADLESS="1", RIONT_SIZE=f"{COLS}x{ROWS}")
screen = pyte.Screen(COLS, ROWS)
stream = pyte.Stream()
stream.attach(screen)
decoder = codecs.getincrementaldecoder("utf-8")()
p = subprocess.Popen([EXE, "127.0.0.1:5814"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, env=env, cwd=ROOT)


def reader():
    while True:
        d = p.stdout.read1(65536)
        if not d:
            break
        stream.feed(decoder.decode(d))


threading.Thread(target=reader, daemon=True).start()

time.sleep(2.5)
p.stdin.write(b"1\n")
p.stdin.flush()
time.sleep(1.0)

print("===== dense watchlist (13+ cards) =====")
for ln in screen.display:
    print(ln.rstrip())
p.kill()
srv.terminate()
try:
    os.remove(os.path.join(ROOT, ".nt-views.json"))
except OSError:
    pass
