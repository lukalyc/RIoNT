"""Quick visual smoke test of the layout (headless)."""
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


def send(keys):
    token_map = {"\r": "RET", "\x1b": "ESC", "\t": "TAB", " ": "SPC"}
    parts = [token_map.get(c, c) for c in keys]
    p.stdin.write(("\x1f".join(parts) + "\n").encode())
    p.stdin.flush()


def pump(s):
    time.sleep(s)


def dump(title):
    print(f"===== {title} =====")
    for ln in screen.display:
        print(ln.rstrip())
    print()


pump(2.5)
dump("initial")
send("l")       # expand SmartDashboard
pump(0.5)
send("jjj")     # cursor to Battery Voltage
pump(0.5)
send(" ")       # pin to watchlist
pump(0.5)
dump("after pin")
send("/")       # search
pump(0.4)
send("swerve")
pump(0.5)
send(" ")       # pin first match
pump(0.4)
send("\r")
pump(0.8)
send("\t")      # focus watchlist
pump(0.5)
dump("watchlist focus")
send(":")       # palette
pump(0.4)
dump("palette")
send("\x1b")
pump(0.3)
send("e")       # edit (cursor on watchlist card)
pump(0.4)
dump("edit prompt")
send("\x1b")
pump(0.2)
p.kill()
srv.terminate()
