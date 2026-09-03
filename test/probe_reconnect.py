"""Probe: retarget to a dead port, then back — does reconnect recover?"""
import codecs, os, subprocess, sys, threading, time, socket

import pyte

ROOT = r"C:\Users\lryam\Documents\robonauts\software\nt-tui"
EXE = os.path.join(ROOT, "target", "debug", "riont.exe")
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

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
env = dict(os.environ, RIONT_HEADLESS="1", RIONT_SIZE=f"{COLS}x{ROWS}", RIONT_DEBUG="1", RIONT_TRACE="1")
screen = pyte.Screen(COLS, ROWS)
stream = pyte.Stream(); stream.attach(screen)
decoder = codecs.getincrementaldecoder("utf-8")()
p = subprocess.Popen([EXE, "127.0.0.1:5814"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, env=env, cwd=ROOT)

def reader():
    while True:
        d = p.stdout.read1(65536)
        if not d: break
        stream.feed(decoder.decode(d))

threading.Thread(target=reader, daemon=True).start()

def send(keys):
    token_map = {"\r": "RET", "\x1b": "ESC", "\t": "TAB", " ": "SPC"}
    parts = [token_map.get(c, c) for c in keys]
    p.stdin.write(("\x1f".join(parts) + "\n").encode()); p.stdin.flush()

time.sleep(2.5)
print("t0:", screen.display[0].strip())
send("c"); time.sleep(0.3); send("127.0.0.1:5999"); time.sleep(0.3); send("\r")
time.sleep(8)
print("after dead:", screen.display[0].strip())
send("c"); time.sleep(0.3); send("127.0.0.1:5814"); time.sleep(0.3); send("\r")
for i in range(12):
    time.sleep(1)
    print(f"back+{i+1}s:", screen.display[0].strip())
    if "ONLINE" in screen.display[0]:
        break
print("--- toasts/hints ---")
print(screen.display[34].rstrip())
p.kill(); srv.terminate()
