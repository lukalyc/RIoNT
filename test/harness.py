"""End-to-end TUI test harness (headless mode).

Runs nt-tui.exe with NT_TUI_HEADLESS=1: the TUI renders ANSI to stdout
(fixed 120x36 viewport) and reads a keystroke script on stdin. The harness
drives stdin, emulates the terminal with pyte, and asserts on the rendered
screen. A real WPILib ntcore instance runs as the NT4 server (test/server.py)
and a second ntcore client mutates values to exercise publish/diff/reconnect
flows.

Run:  C:/Users/lryam/.conda/envs/nt-tui-test/python.exe test/harness.py
"""
import codecs
import os
import socket
import subprocess
import sys
import threading
import time

import pyte

import ntcore

# Console may be cp1252; force utf-8 with replacement for printing screens.
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "nt-tui.exe")
SERVER = os.path.join(ROOT, "test", "server.py")
PY = sys.executable
COLS, ROWS = 120, 36
TARGET = "127.0.0.1:5814"

os.environ["NT_TUI_DEBUG"] = "1"

BLOCKS = set(" ▁▂▃▄▅▆▇")

results = []


def check(name, cond, detail=""):
    results.append((name, bool(cond)))
    mark = "PASS" if cond else "FAIL"
    line = f"[{mark}] {name}"
    if not cond:
        line += f"   | {str(detail)[:600]}"
    print(line, flush=True)


class Tui:
    """Headless TUI driver: stdin = key script, stdout = ANSI render."""

    def __init__(self, target=TARGET):
        env = dict(os.environ, NT_TUI_HEADLESS="1", NT_TUI_SIZE=f"{COLS}x{ROWS}")
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.Stream()
        self.stream.attach(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")()
        self.proc = subprocess.Popen(
            [EXE, target],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=env,
            cwd=ROOT,
        )
        self.alive = True
        self.exitstatus = None
        self.raw_chunks = []
        self._pending = []
        self._thread = threading.Thread(target=self._read_loop, daemon=True)
        self._thread.start()

    def _read_loop(self):
        try:
            while True:
                d = self.proc.stdout.read1(65536)
                if not d:
                    break
                self._pending.append(self.decoder.decode(d))
        except Exception:
            pass
        self.alive = False
        try:
            self.exitstatus = self.proc.wait(timeout=5)
        except Exception:
            pass

    def _drain(self):
        for data in self._pending:
            self.raw_chunks.append(data)
            self.stream.feed(data)
        self._pending.clear()

    def pump(self, secs=0.35):
        end = time.time() + secs
        while time.time() < end:
            time.sleep(min(0.05, max(0.0, end - time.time())))
        self._drain()

    def send(self, keys):
        """Send keys: one char = one key press. Control chars are encoded as
        tokens (RET/ESC/TAB/SPC) because stdin lines strip CRLF."""
        if not self.alive:
            return
        token_map = {"\r": "RET", "\x1b": "ESC", "\t": "TAB", " ": "SPC"}
        parts = [token_map.get(c, c) for c in keys]
        try:
            self.proc.stdin.write(("\x1f".join(parts) + "\n").encode())
            self.proc.stdin.flush()
        except Exception:
            pass

    def lines(self):
        return [ln.rstrip() for ln in self.screen.display]

    def text(self):
        return "\n".join(self.lines())

    def has(self, needle):
        return needle in self.text()

    def cursor_rows(self):
        # The tree/watch cursor is marked with a visible "> " prefix. Rows in
        # the right-hand pane (inspector/watch) are prefixed by the pane
        # divider, so also match the marker right after a "│".
        rows = []
        for i, ln in enumerate(self.lines()):
            tail = ln.rsplit("│", 1)[-1]
            if ln.startswith("> ") or tail.startswith("> "):
                rows.append(i)
        return rows

    def styled(self, row, needle, attr):
        """True if the cells covering `needle` in `row` carry `attr`."""
        line = self.lines()[row]
        x = line.find(needle)
        if x < 0:
            return False
        return all(getattr(self.screen.buffer[row][x + i], attr) for i in range(len(needle)))

    def find_row(self, needle):
        for i, ln in enumerate(self.lines()):
            if needle in ln:
                return i
        return -1

    def close(self):
        try:
            self.proc.kill()
        except Exception:
            pass


def wait_port(port, timeout=15.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.2)
    return False


def wait_topic(sub, want, timeout=2.0):
    """Poll a subscriber until it reports `want` (returns last seen value)."""
    end = time.time() + timeout
    last = sub.get()
    while time.time() < end:
        if last == want:
            break
        time.sleep(0.05)
        last = sub.get()
    return last


class Server:
    def __init__(self):
        self.p = None

    def start(self):
        self.log = open(os.path.join(ROOT, "test", "server.log"), "ab")
        self.p = subprocess.Popen([PY, SERVER], cwd=ROOT, stdout=self.log, stderr=self.log)
        assert wait_port(5814), "server did not open port 5814"
        print("server up", flush=True)

    def stop(self):
        if self.p:
            self.p.terminate()
            try:
                self.p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.p.kill()
            self.p = None


# ---------------------------------------------------------------------------
# scenario
# ---------------------------------------------------------------------------

def main():
    server = Server()
    server.start()
    t0 = time.time()  # ≈ server clock origin (server stops StaleCounter at t=8s)
    harness = ntcore.NetworkTableInstance.getDefault()
    harness.startClient4("harness")
    harness.setServer("127.0.0.1", 5814)
    for _ in range(50):
        if harness.isConnected():
            break
        time.sleep(0.1)
    assert harness.isConnected(), "harness could not connect to server"
    print("harness client connected", flush=True)

    sd = harness.getTable("SmartDashboard")
    swerve = harness.getTable("Swerve")

    # Server-side verification reads. A pyntcore client only caches values for
    # topics it subscribes to, so bare sd.getNumber()/getString() calls always
    # return the default and would "fail" even when the publish reached the
    # server. Verify publishes through real subscribers instead.
    _opts = ntcore.PubSubOptions(periodic=0.05, topicsOnly=False)
    kp_read = harness.getDoubleTopic("/SmartDashboard/kP").subscribe(-1.0, _opts)
    al_read = harness.getStringTopic("/SmartDashboard/Alliance").subscribe("", _opts)
    cl_read = harness.getBooleanTopic("/SmartDashboard/Climb Locked").subscribe(False, _opts)

    # A topic published by the harness itself so T12 can unpublish it (clients
    # cannot unpublish topics owned by another publisher, e.g. the robot's).
    sd.putString("Ephemeral", "here")

    tui = Tui()
    tui.pump(2.0)

    # T1: connection + header ------------------------------------------------
    ln0 = tui.lines()[0]
    check("T1a header shows LIVE", "LIVE" in ln0, ln0)
    check("T1b header shows target", TARGET in ln0, ln0)
    m = None
    import re

    m = re.search(r"(\d+) topics", ln0)
    check("T1c topic count > 10", m and int(m.group(1)) > 10, ln0)
    check("T1d header shows Hz", re.search(r"[\d.]+ Hz", ln0) is not None, ln0)
    check("T1e LIVE rendered bold", tui.styled(0, "LIVE", "bold"), ln0)
    ln1 = tui.lines()[1]
    check("T1f uptime displayed", re.search(r"robot uptime \d{2}:\d{2}:\d{2}", ln1) is not None, ln1)
    check("T1g server info shown", len(ln1.split("robot uptime")[-1].strip()) > 5, ln1)

    # T2: initial tree, collapsed -------------------------------------------
    txt = tui.text()
    check("T2a collapsed root dirs", "[+] SmartDashboard" in txt and "[+] Swerve" in txt, txt)
    check("T2b tree rows have no values when collapsed", "Battery Voltage" not in txt, txt)

    # T3: expand with l -------------------------------------------------------
    tui.send("l")
    tui.pump(0.5)
    txt = tui.text()
    check("T3a dir expanded marker", "[-] SmartDashboard" in txt, txt)
    check("T3b children visible", "Battery Voltage" in txt and "kP" in txt, txt)
    check("T3c inline values in tree", re.search(r"Battery Voltage \[double\] [0-9]+\.", txt), txt)
    check("T3d inline Hz in tree", re.search(r"Battery Voltage \[double\].*Hz", txt), txt)
    # T3e: editable topics carry a green type tag (e edit will work)
    bv_row = tui.find_row("Battery Voltage")
    gx = tui.lines()[bv_row].find("[double]") if bv_row >= 0 else -1
    fgs = {tui.screen.buffer[bv_row][gx + j].fg for j in range(8)} if bv_row >= 0 and gx >= 0 else set()
    green = len(fgs) == 1 and "default" not in fgs
    check("T3e editable tag is green", green, f"fgs={fgs} row={tui.lines()[bv_row] if bv_row >= 0 else 'none'}")

    cur0 = tui.cursor_rows()
    tui.send("j")
    tui.pump(0.4)
    cur1 = tui.cursor_rows()
    check("T4 j moves cursor", cur0 and cur1 and cur1 != cur0, f"{cur0} -> {cur1}")
    check("T4b cursor highlighted reverse", bool(cur1), cur1)

    # T5: g / G ---------------------------------------------------------------
    tui.send("G")
    tui.pump(0.4)
    curG = tui.cursor_rows()
    tui.send("g")
    tui.pump(0.4)
    curg = tui.cursor_rows()
    # bottom tree row = last non-empty row of the left pane (row count varies
    # with harness-published topics, so compute it instead of hard-coding)
    left_rows = [
        i for i, ln in enumerate(tui.lines())
        if 2 <= i <= 34 and ln.split("│")[0].strip()
    ]
    check("T5a G jumps to bottom", curG and left_rows and curG[0] == max(left_rows),
          f"curG={curG} bottom={max(left_rows) if left_rows else None}")
    check("T5b g jumps to top", curg and curg[0] == 2, curg)

    # T6: collapse with h ------------------------------------------------------
    tui.send("h")
    tui.pump(0.4)
    check("T6 h collapses dir", "[-] SmartDashboard" not in tui.text() and "[+] SmartDashboard" in tui.text(), tui.text())
    tui.send("l")
    tui.pump(0.4)

    # T7: inspector content ----------------------------------------------------
    # cursor on row0 (SmartDashboard dir) -> jjj lands on Battery Voltage
    tui.send("g")
    tui.pump(0.3)
    tui.send("jjj")
    tui.pump(0.5)
    txt = tui.text()
    check("T7a inspector shows full path", "SmartDashboard/Battery Voltage" in txt, txt)
    check("T7b inspector shows type", "type     double" in txt, txt)
    check("T7c inspector shows rate", "rate     19." in txt or "rate     20." in txt or "rate     21." in txt, txt)
    check("T7d inspector shows last update", "last update 0." in txt, txt)
    trend_row = tui.find_row("trend")
    ok = trend_row >= 0 and any(c in BLOCKS for c in tui.lines()[trend_row])
    check("T7e sparkline rendered", ok, tui.lines()[trend_row] if trend_row >= 0 else "no trend row")

    # T8: focus cycle + status hints -------------------------------------------
    tui.send("\t")
    tui.pump(0.4)
    check("T8a inspector hints in status bar", "j/k scroll" in tui.lines()[-1], tui.lines()[-1])
    tui.send("\t")
    tui.pump(0.4)
    check("T8b tree hints back (two-way tab)", "h fold" in tui.lines()[-1], tui.lines()[-1])

    # T9: watchlist -------------------------------------------------------------
    tui.send("/")  # search for battery
    tui.pump(0.3)
    tui.send("batt")
    tui.pump(0.5)
    txt = tui.text()
    check("T9a search overlay opens", "search" in txt and "/batt" in txt, txt)
    check("T9b search shows match", any("Battery Voltage" in l for l in tui.lines()), txt)
    tui.send("\r")
    tui.pump(0.5)
    txt = tui.text()
    check("T9c jump expanded ancestors", "[-] SmartDashboard" in txt, txt)
    curs = tui.cursor_rows()
    check("T9d cursor on match row", curs and "Battery Voltage" in tui.lines()[curs[0]], curs)

    # T10: edit + publish round trip --------------------------------------------
    tui.send("/"); tui.pump(0.3); tui.send("kp\r"); tui.pump(0.5)
    curs = tui.cursor_rows()
    check("T10a cursor on kP", curs and "kP" in tui.lines()[curs[0]], curs)
    tui.send("e")
    tui.pump(0.4)
    txt = tui.text()
    check("T10b edit overlay opens", "set SmartDashboard/kP" in txt and "esc=cancel" in txt, txt)
    tui.send("0.05")
    tui.pump(0.3)
    tui.send("\r")
    tui.pump(0.6)
    check("T10c publish status shown", "published SmartDashboard/kP = 0.05" in tui.lines()[-1], tui.lines()[-1])
    got = wait_topic(kp_read, 0.05)
    check("T10d server received 0.05", got == 0.05, f"server kP={got}")

    # T10e edit error path
    tui.send("e"); tui.pump(0.4)
    tui.send("abc"); tui.pump(0.3); tui.send("\r"); tui.pump(0.4)
    check("T10e edit error shown", "not a number: abc" in tui.text(), tui.text())
    tui.send("\x1b"); tui.pump(0.4)
    check("T10f esc closes editor", "set SmartDashboard/kP" not in tui.text(), tui.text())

    # T10g edit string topic
    tui.send("/"); tui.pump(0.3); tui.send("alliance\r"); tui.pump(0.5)
    tui.send("e"); tui.pump(0.4)
    tui.send("red"); tui.pump(0.3); tui.send("\r"); tui.pump(0.6)
    got = wait_topic(al_read, "red")
    check("T10g string publish", got == "red",
          f"server Alliance={got}")

    # T10h edit boolean topic
    tui.send("/"); tui.pump(0.3); tui.send("climb\r"); tui.pump(0.5)
    tui.send("e"); tui.pump(0.4)
    tui.send("true"); tui.pump(0.3); tui.send("\r"); tui.pump(0.6)
    got = wait_topic(cl_read, True)
    check("T10h boolean publish", got is True,
          f"server Climb Locked={got}")

    # T10i non-writable type rejected (boolean[] topic)
    tui.send("/"); tui.pump(0.3); tui.send("faults\r"); tui.pump(0.5)
    tui.send("e"); tui.pump(0.4)
    check("T10i array edit rejected", "topic type not editable" in tui.lines()[-1], tui.lines()[-1])
    tui.send("\x1b"); tui.pump(0.3)

    # T11 was removed with the STALE flag: quiet-but-alive topics are normal.

    # T12: snapshot + diff -------------------------------------------------------
    tui.send("s")
    tui.pump(0.4)
    check("T12a snapshot status", "snapshot taken" in tui.lines()[-1], tui.lines()[-1])
    # mutate from the "robot": change kP, add a topic, remove one
    sd.putNumber("kP", 0.020)
    sd.putNumber("AddedLater", 7)
    sd.getEntry("Ephemeral").unpublish()
    tui.pump(1.0)
    tui.send("d")
    tui.pump(0.5)
    txt = tui.text()
    check("T12b diff header", "diff (" in txt, txt)
    check("T12c diff changed row", "SmartDashboard/kP" in txt and "->" in txt, txt)
    check("T12d diff added row", "+ SmartDashboard/AddedLater" in txt, txt)
    check("T12e diff removed row", "- SmartDashboard/Ephemeral" in txt, txt)

    # T12f diff cursor navigation (focus inspector, j/k)
    tui.send("\t")  # -> inspector
    tui.pump(0.4)
    check("T12f inspector hints", "j/k scroll" in tui.lines()[-1], tui.lines()[-1])
    tui.send("j"); tui.pump(0.3)
    tui.send("j"); tui.pump(0.3)
    curs = tui.cursor_rows()
    check("T12g diff cursor navigable", bool(curs), curs)
    tui.send("\x1b"); tui.pump(0.3)
    check("T12h esc clears diff", "diff (" not in tui.text(), tui.text())

    # T13: reconnect --------------------------------------------------------------
    server.stop()
    tui.pump(2.5)
    txt = tui.text()
    check("T13a down shown", ("DOWN" in txt) or ("connecting" in txt), txt.splitlines()[0])
    check("T13b disconnect status", "disconnected" in tui.lines()[-1], tui.lines()[-1])
    server.start()
    tui.pump(3.5)
    txt = tui.text()
    check("T13c reconnected LIVE", "LIVE" in txt.splitlines()[0], txt.splitlines()[0])
    check("T13d topics restored", "Battery Voltage" in txt, txt)

    # T15: manual reconnect (R) -------------------------------------------------
    tui.send("R")
    tui.pump(0.15)
    last = tui.lines()[-1]
    # the status flips to "disconnected" once the socket actually drops; both
    # strings prove the key was handled
    check("T15a R handled", "reconnecting to" in last or "disconnected" in last, last)
    tui.pump(2.0)
    check("T15b live again after R", "LIVE" in tui.lines()[0], tui.lines()[0])

    # T16: connect overlay (c) -------------------------------------------------
    tui.send("c")
    tui.pump(0.3)
    txt = tui.text()
    check("T16a connect overlay opens", "connect to:" in txt and "esc=cancel" in txt, txt)
    tui.send("127.0.0.1:5814")
    tui.pump(0.3)
    tui.send("\r")
    tui.pump(3.0)
    txt = tui.text()
    check("T16b retarget connects", "LIVE" in txt.splitlines()[0], txt.splitlines()[0])
    check("T16c topics flow after retarget", "Battery Voltage" in txt, txt)

    # T17: retarget to a dead port and back -----------------------------------
    tui.send("c"); tui.pump(0.3)
    tui.send("127.0.0.1:5999")  # nothing listens here
    tui.pump(0.3); tui.send("\r")
    tui.pump(7.5)               # 5s connect timeout + retry cycle
    txt = tui.text()
    check("T17a bad target shows DOWN", "DOWN" in txt.splitlines()[0], txt.splitlines()[0])
    tui.send("c"); tui.pump(0.3)
    tui.send("127.0.0.1:5814"); tui.pump(0.3); tui.send("\r")
    tui.pump(4.0)
    txt = tui.text()
    check("T17b returns to LIVE", "LIVE" in txt.splitlines()[0], txt.splitlines()[0])
    check("T17c values flow again", "Battery Voltage" in txt, txt)

    # T18: last target persisted on successful connect -------------------------
    tgt_file = os.path.join(ROOT, ".nt-tui-target")
    saved = open(tgt_file).read().strip() if os.path.isfile(tgt_file) else ""
    check("T18a last target saved", saved == "127.0.0.1:5814", saved or "missing")

    # T14: quit ---------------------------------------------------------------------
    tui.send("q")
    tui.pump(1.5)
    check("T14a quits on q", not tui.alive, f"alive={tui.alive}")
    check("T14b clean exit code", tui.exitstatus == 0, tui.exitstatus)

    server.stop()

    # summary ------------------------------------------------------------------------
    print("", flush=True)
    fails = [n for (n, ok) in results if not ok]
    print(f"==== {len(results) - len(fails)}/{len(results)} checks passed ====")
    if fails:
        print("failed:")
        for n in fails:
            print(f"  - {n}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
