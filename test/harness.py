"""End-to-end TUI test harness (headless mode) — RIONT v0.2.0 dashboard.

Runs riont.exe with RIONT_HEADLESS=1: the TUI renders ANSI to stdout
(fixed 120x36 viewport) and reads a keystroke script on stdin. The harness
drives stdin, emulates the terminal with pyte, and asserts on the rendered
screen. A real WPILib ntcore instance runs as the NT4 server (test/server.py)
and a second ntcore client mutates values to exercise publish/reconnect
flows.

Layout under test: DS-style HUD, 35% left control column (Topic Tree +
passive Inspector Dock), 65% full-height Watchlist Canvas, command palette,
toasts.

Run:  C:/Users/lryam/.conda/envs/nt-tui-test/python.exe test/harness.py
"""
import codecs
import json
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
EXE = os.path.join(ROOT, "target", "debug", "riont.exe")
SERVER = os.path.join(ROOT, "test", "server.py")
PY = sys.executable
COLS, ROWS = 120, 36
TARGET = "127.0.0.1:5814"

os.environ["RIONT_DEBUG"] = "1"

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
        env = dict(os.environ, RIONT_HEADLESS="1", RIONT_SIZE=f"{COLS}x{ROWS}")
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
        self._thread = threading.Thread(target=self._read_loop, daemon=True)
        self._thread.start()

    def _read_loop(self):
        try:
            while True:
                d = self.proc.stdout.read1(65536)
                if not d:
                    break
                self.stream.feed(self.decoder.decode(d))
        except Exception:
            pass
        self.alive = False
        try:
            self.exitstatus = self.proc.wait(timeout=5)
        except Exception:
            pass

    # Whole-key word tokens the driver understands (arrows): overlays
    # navigate by arrows now that j/k are plain input there.
    WORD_TOKENS = ("UP", "DOWN", "LEFT", "RIGHT")

    def send(self, keys):
        """Send keys: one char = one key press. Control chars are encoded as
        tokens (RET/ESC/TAB/SPC) because stdin lines strip CRLF. Arrow keys
        are sent as whole words ("DOWN"), optionally space-separated."""
        if not self.alive:
            return
        if keys in self.WORD_TOKENS:
            parts = [keys]
        elif keys == " ":
            parts = ["SPC"]
        elif " " in keys:
            # Space-separated: keep only whole-word tokens.
            parts = [p for p in keys.split(" ") if p in self.WORD_TOKENS]
        else:
            token_map = {"\r": "RET", "\x1b": "ESC", "\t": "TAB", " ": "SPC"}
            parts = [token_map.get(c, c) for c in keys]
        try:
            self.proc.stdin.write(("\x1f".join(parts) + "\n").encode())
            self.proc.stdin.flush()
        except Exception:
            pass

    def pump(self, secs=0.35):
        end = time.time() + secs
        while time.time() < end:
            time.sleep(min(0.05, max(0.0, end - time.time())))

    def lines(self):
        return [ln.rstrip() for ln in self.screen.display]

    def text(self):
        return "\n".join(self.lines())

    def has(self, needle):
        return needle in self.text()

    def tree_cursor_rows(self):
        """Rows carrying the tree cursor: '> ' right after the pane border."""
        return [i for i, ln in enumerate(self.lines()) if ln[1:3] == "> "]

    def tree_rows(self):
        """Row indices with content inside the TOPIC TREE block (left pane only)."""
        insp = self.find_row("INSPECTOR")
        top = insp if insp > 0 else len(self.lines())
        out = []
        for i in range(2, top):
            ln = self.lines()[i]
            seg = ln.split("│")[1] if ln.startswith("│") else ""
            if seg.strip():
                out.append(i)
        return out

    def canvas_seg(self, i):
        """Text inside the watchlist canvas on row i (between its borders)."""
        parts = self.lines()[i].split("│")
        return parts[-2] if len(parts) >= 5 else ""

    def card_rows(self):
        """Rows whose canvas segment starts with a card's top border."""
        return [i for i in range(len(self.lines())) if self.canvas_seg(i).startswith("┌ ")]

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
        # Hermetic runs: kill any zombie server still holding 5814 (from a
        # previously crashed harness run), then verify the port is free.
        out = subprocess.run(["netstat", "-ano"], capture_output=True, text=True).stdout
        for ln in out.splitlines():
            if "5814" in ln and "LISTENING" in ln:
                pid = ln.split()[-1]
                subprocess.run(["taskkill", "/F", "/PID", pid], capture_output=True)
        end = time.time() + 5
        while time.time() < end:
            if "5814" not in subprocess.run(["netstat", "-ano"],
                                            capture_output=True, text=True).stdout:
                break
            time.sleep(0.2)
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

    _opts = ntcore.PubSubOptions(periodic=0.05, topicsOnly=False)
    kp_read = harness.getDoubleTopic("/SmartDashboard/kP").subscribe(-1.0, _opts)
    al_read = harness.getStringTopic("/SmartDashboard/Alliance").subscribe("", _opts)
    cl_read = harness.getBooleanTopic("/SmartDashboard/Climb Locked").subscribe(False, _opts)

    sd.putString("Ephemeral", "here")

    # Fresh config so runs are independent (T21 preset indices, T18 MRU).
    cfg_path = os.path.join(os.path.expanduser("~"), ".config", "riont", "config.json")
    try:
        os.remove(cfg_path)
    except OSError:
        pass

    # Workspace presets for the T12/T21 preset tests (legacy .nt-views.json).
    with open(os.path.join(ROOT, ".nt-views.json"), "w") as fh:
        fh.write(json.dumps([
            {"name": "Test", "topics": ["SmartDashboard/Shooter RPM", "Swerve/*"]},
            {"name": "Dense", "topics": ["Swerve/*", "SmartDashboard/*"]},
        ]))

    tui = Tui()
    tui.pump(2.5)

    # T1: HUD — driver-station diagnostics ------------------------------------
    ln0 = tui.lines()[0]
    check("T1a header shows COMM ONLINE", "COMM: ONLINE" in ln0, ln0)
    check("T1b header shows target IP", "127.0.0.1" in ln0, ln0)
    check("T1c header shows CODE RUNNING", "CODE: RUNNING" in ln0, ln0)
    check("T1d header shows UPTIME", "UPTIME: " in ln0 and ":" in ln0.split("UPTIME: ")[1], ln0)
    import re

    m = re.search(r"(\d+) topics", ln0)
    check("T1e topic count > 10", m and int(m.group(1)) > 10, ln0)
    check("T1f no global Hz metric", " Hz" not in ln0 and "rtt" not in ln0, ln0)
    check("T1g version 0.2.0", "RIONT v0.2.0" in ln0, ln0)
    check("T1g ONLINE rendered bold+green", tui.styled(0, "ONLINE", "bold"), ln0)

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
    green = "green" in fgs or "00cd00" in fgs
    check("T3e editable tag is green", green, f"fgs={fgs} row={tui.lines()[bv_row] if bv_row >= 0 else 'none'}")

    cur0 = tui.tree_cursor_rows()
    tui.send("j")
    tui.pump(0.4)
    cur1 = tui.tree_cursor_rows()
    check("T4 j moves cursor", cur0 and cur1 and cur1 != cur0, f"{cur0} -> {cur1}")
    check("T4b cursor highlighted reverse", bool(cur1), cur1)

    # T5: g / G ---------------------------------------------------------------
    tui.send("G")
    tui.pump(0.4)
    curG = tui.tree_cursor_rows()
    tui.send("g")
    tui.pump(0.4)
    curg = tui.tree_cursor_rows()
    left_rows = tui.tree_rows()
    check("T5a G jumps to bottom", curG and left_rows and curG[0] == max(left_rows),
          f"curG={curG} bottom={max(left_rows) if left_rows else None}")
    check("T5b g jumps to top", curg and curg[0] == 2, curg)

    # T6: collapse with h ------------------------------------------------------
    tui.send("h")
    tui.pump(0.4)
    check("T6 h collapses dir", "[-] SmartDashboard" not in tui.text() and "[+] SmartDashboard" in tui.text(), tui.text())
    tui.send("l")
    tui.pump(0.4)

    # T7: passive inspector dock ------------------------------------------------
    # cursor on row0 (SmartDashboard dir) -> jjj lands on Battery Voltage
    tui.send("g")
    tui.pump(0.3)
    tui.send("jjj")
    tui.pump(0.5)
    txt = tui.text()
    check("T7a dock shows full path", "SmartDashboard/Battery Voltage" in txt, txt)
    check("T7b dock shows type", "Type:  double" in txt, txt)
    check("T7c dock shows rate", re.search(r"Rate:  \d+\.\d Hz", txt), txt)
    check("T7d dock shows delta", "Δ " in txt, txt)
    check("T7e dock shows value", re.search(r"Value: [0-9]", txt), txt)

    # T8: focus cycle + status hints -------------------------------------------
    tui.send("\t")
    tui.pump(0.4)
    # Empty canvas: the hint line drops the dead card keys (x/e/spc) and
    # keeps only what works — the reviewed fix for advertising dead keys.
    last_line = tui.lines()[-1]
    check("T8a watchlist hints after tab",
          "tab tree" in last_line and "1-9 presets" in last_line and "x remove" not in last_line,
          last_line)
    tui.send("\t")
    tui.pump(0.4)
    check("T8b tree hints back (two-way tab)", "h fold" in tui.lines()[-1], tui.lines()[-1])

    # T9: search jumps, then space pins from the tree ---------------------------
    tui.send("/")  # search for battery
    tui.pump(0.3)
    tui.send("batt")
    tui.pump(0.5)
    txt = tui.text()
    check("T9a search overlay opens", "search" in txt and "/batt" in txt, txt)
    check("T9b search shows match", any("Battery Voltage" in l for l in tui.lines()), txt)
    tui.send("\r")  # jump to the match in the tree
    tui.pump(0.5)
    txt = tui.text()
    check("T9c jump expanded ancestors", "[-] SmartDashboard" in txt, txt)
    curs = tui.tree_cursor_rows()
    check("T9d cursor on match row", curs and "Battery Voltage" in tui.lines()[curs[0]], curs)
    tui.send(" ")  # pin to watchlist from the tree
    tui.pump(0.5)
    txt = tui.text()
    check("T9e pinned toast", "[SUCCESS] pinned SmartDashboard/Battery Voltage" in txt, txt[-400:])
    check("T9f watchlist card appeared", re.search(r"┌ SmartDashboard/Battery Voltage", txt), txt[-800:])
    # pinned star in the tree (left pane only — the card title also matches)
    star_row = next((i for i, ln in enumerate(tui.lines())
                     if ln.startswith("│") and "* Battery Voltage" in ln.split("│")[1]), -1)
    check("T9g pinned star in tree", star_row >= 0, tui.text()[:900])

    # T10: edit + publish round trip (inline bottom prompt) ----------------------
    tui.send("/"); tui.pump(0.3); tui.send("kp\r"); tui.pump(0.5)
    curs = tui.tree_cursor_rows()
    check("T10a cursor on kP", curs and "kP" in tui.lines()[curs[0]], curs)
    tui.send("e")
    tui.pump(0.4)
    txt = tui.text()
    check("T10b inline edit prompt opens", "Set SmartDashboard/kP:" in txt and "enter=publish" in txt, txt[-500:])
    tui.send("0.05")
    tui.pump(0.3)
    tui.send("\r")
    tui.pump(0.6)
    check("T10c publish toast shown", "[SUCCESS] published SmartDashboard/kP = 0.05" in tui.text(), tui.lines()[-6:])
    got = wait_topic(kp_read, 0.05)
    check("T10d server received 0.05", got == 0.05, f"server kP={got}")

    # T10e edit error path
    tui.send("e"); tui.pump(0.4)
    tui.send("abc"); tui.pump(0.3); tui.send("\r"); tui.pump(0.4)
    check("T10e edit error shown", "not a number: abc" in tui.text(), tui.text()[-400:])
    tui.send("\x1b"); tui.pump(0.4)
    check("T10f esc closes editor", "Set SmartDashboard/kP:" not in tui.text(), tui.text()[-400:])

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
    tui.send("e"); tui.pump(0.5)
    check("T10i array edit rejected", "[ERROR] topic type not editable" in tui.text(), tui.lines()[-6:])
    tui.send("\x1b"); tui.pump(0.3)

    # T12: watchlist height-first stacking -----------------------------------
    # 5 scalar cards fit vertically in one column at 120x36 — column 1 must
    # fill 100% of the height before column 2 instantiates.
    for term in ("gyro", "match", "shooter", "compressor"):
        tui.send("/"); tui.pump(0.3); tui.send(term); tui.pump(0.4)
        tui.send(" "); tui.pump(0.4)   # pin first match
        tui.send("\x1b"); tui.pump(0.2)
    txt = tui.text()
    m = re.search(r"WATCHLIST \((\d+)\)", txt)
    n = int(m.group(1)) if m else -1
    check("T12a watchlist count 5-12", 5 <= n <= 12, txt.splitlines()[1] if m else txt)
    # Height-first: all 5 cards stack in ONE column — no row carries two
    # card top-borders inside the canvas.
    two_col = any(tui.canvas_seg(i).count("┌") >= 2 for i in range(len(tui.lines())))
    check("T12b height-first single column at 5 cards", not two_col, txt[:1500])
    # Vertical stacking evidence: consecutive card top-borders exactly one
    # card-height (4 lines) apart, directly below each other.
    card_rows = tui.card_rows()
    check("T12c cards stack vertically", len(card_rows) >= 5 and card_rows[1] - card_rows[0] == 4,
          f"card_rows={card_rows}")

    # T12d: direct removal with x on the watchlist
    tui.send("\t")  # focus watchlist
    tui.pump(0.4)
    before = re.search(r"WATCHLIST \((\d+)\)", tui.text())
    n_before = int(before.group(1)) if before else -1
    tui.send("x")
    tui.pump(0.5)
    after = re.search(r"WATCHLIST \((\d+)\)", tui.text())
    n_after = int(after.group(1)) if after else -1
    check("T12d x removes active card", n_before > 0 and n_after == n_before - 1,
          f"{n_before} -> {n_after}")
    check("T12e unpinned toast", "[INFO] unpinned" in tui.text(), tui.text()[-600:])

    # T12f: command palette clears the watchlist
    tui.send(":"); tui.pump(0.4)
    check("T12f palette opens", "commands" in tui.text() and "Settings: Open Configuration" in tui.text(), tui.text()[-900:])
    tui.send("clear"); tui.pump(0.4)
    tui.send("\r"); tui.pump(0.5)
    txt = tui.text()
    check("T12g clear watchlist command", "WATCHLIST (0)" in txt and "[SUCCESS] watchlist cleared" in txt, txt[-800:])

    # T13: reconnect (auto) -------------------------------------------------------
    server.stop()
    down = False
    end = time.time() + 10
    while time.time() < end:
        ln0 = tui.lines()[0]
        if "RECONNECTING" in ln0 or "DISCONNECTED" in ln0:
            down = True
            break
        time.sleep(0.2)
    check("T13a HUD shows DISCONNECTED or RECONNECTING", down, tui.lines()[0])
    check("T13b disconnect toast", "[ERROR] disconnected" in tui.text(), tui.text()[-600:])
    server.start()
    up = False
    end = time.time() + 6
    while time.time() < end:
        if "COMM: ONLINE" in tui.lines()[0]:
            up = True
            break
        time.sleep(0.2)
    txt = tui.text()
    check("T13c reconnected ONLINE", up, txt.splitlines()[0])
    check("T13d topics restored", "Battery Voltage" in txt, txt)

    # T15: reconnect via command palette ------------------------------------------
    tui.send(":"); tui.pump(0.4)
    tui.send("recon"); tui.pump(0.4)
    tui.send("\r")
    tui.pump(0.5)
    txt = tui.text()
    check("T15a reconnect toast", "reconnecting to" in txt, txt[-700:])
    tui.pump(2.0)
    check("T15b live again after palette reconnect", "COMM: ONLINE" in tui.lines()[0], tui.lines()[0])

    # T16: connection picker (c) — select and connect only -----------------------
    tui.send("c")
    tui.pump(0.3)
    txt = tui.text()
    check("T16a picker opens", "[CONNECT TARGET]" in txt and "connect to:" in txt, txt)
    # No [n] index prefixes (digits are input — see the picker render):
    # rows are plain name + address.
    check("T16b saved targets listed", "Simulation" in txt and "USB Tether" in txt and "[1]" not in txt, txt)
    tui.send("127.0.0.1:5814")
    tui.pump(0.3)
    tui.send("\r")
    up = False
    end = time.time() + 6
    while time.time() < end:
        if "COMM: ONLINE" in tui.lines()[0]:
            up = True
            break
        time.sleep(0.2)
    txt = tui.text()
    check("T16c typed retarget connects", up, txt.splitlines()[0])
    check("T16d topics flow after retarget", "Battery Voltage" in txt, txt)

    # T17: retarget to a dead port and back -----------------------------------
    tui.send("c"); tui.pump(0.3)
    tui.send("127.0.0.1:5999")  # nothing listens here (digits must type!)
    tui.pump(0.3); tui.send("\r")
    tui.pump(7.5)               # 5s connect timeout + retry cycle
    txt = tui.text()
    check("T17a bad target shows DISCONNECTED", "DISCONNECTED" in txt.splitlines()[0] or "RECONNECTING" in txt.splitlines()[0], txt.splitlines()[0])
    tui.send("c"); tui.pump(0.3)
    tui.send("127.0.0.1:5814"); tui.pump(0.3); tui.send("\r")
    up = False
    end = time.time() + 6
    while time.time() < end:
        if "COMM: ONLINE" in tui.lines()[0]:
            up = True
            break
        time.sleep(0.2)
    txt = tui.text()
    check("T17b returns to ONLINE", up, txt.splitlines()[0])
    check("T17c values flow again", "Battery Voltage" in txt, txt)

    # T18: last target persisted in config.json --------------------------------
    cfg_path = os.path.join(os.path.expanduser("~"), ".config", "riont", "config.json")
    saved = ""
    if os.path.isfile(cfg_path):
        with open(cfg_path) as fh:
            saved = json.load(fh).get("last_target", "") or ""
    check("T18a last target saved in config", saved == "127.0.0.1:5814", saved or f"missing ({cfg_path})")

    # T20: W pins a whole subtree ------------------------------------------
    tui.send("/"); tui.pump(0.3); tui.send("modang\r"); tui.pump(0.5)
    tui.send("W"); tui.pump(0.8)
    txt = tui.text()
    check("T20a wildcard cards", "Module Angle" in txt and "Velocity" in txt and "Current" in txt, txt[:400])

    # T21: workspace preset (1) + dense preset (2) -----------------------------
    tui.send("1"); tui.pump(0.8)
    txt = tui.text()
    check("T21a preset loads watchlist", "Shooter RPM" in txt and "Velocity" in txt, txt[:400])

    # T21b: dense preset (20 cards) overflows one column -> multi-column.
    tui.send("2"); tui.pump(0.8)
    txt = tui.text()
    m = re.search(r"WATCHLIST \((\d+)\)", txt)
    n = int(m.group(1)) if m else -1
    check("T21b dense preset count > 12", n > 12, txt.splitlines()[1] if m else txt)
    multi = any(tui.canvas_seg(i).count("┌") >= 2 for i in range(len(tui.lines())))
    check("T21c multi-column when column full", multi, txt[:1200])

    # T22: edit a card straight from the watchlist ---------------------------
    # T21b's dense preset still has the cursor on card 0 (FrontLeft/Current,
    # a writable double).
    tui.send("e"); tui.pump(0.4)
    txt = tui.text()
    check("T22a inline edit opens from card", "Set Swerve/FrontLeft/Current:" in txt, txt[-500:])
    tui.send("\x1b"); tui.pump(0.3)
    tui.send(":"); tui.pump(0.3); tui.send("clear"); tui.pump(0.3); tui.send("\r"); tui.pump(0.4)

    # T23: settings workflow ---------------------------------------------------
    # Add Robot Target via the focused prompt
    tui.send(":"); tui.pump(0.3); tui.send("add"); tui.pump(0.4)
    tui.send("\r"); tui.pump(0.4)
    txt = tui.text()
    check("T23a add-target prompt opens", "[ADD ROBOT TARGET]" in txt and "IP/Team" in txt, txt[-600:])
    tui.send("118"); tui.pump(0.3)
    tui.send("\r"); tui.pump(0.5)
    check("T23b target added toast", "[SUCCESS] Added Team 118" in tui.text(), tui.text()[-600:])
    # It shows up in the connection picker
    tui.send("c"); tui.pump(0.3)
    txt = tui.text()
    check("T23c picker shows added target", "Team 118" in txt and "10.1.18.2" in txt, txt)
    tui.send("\x1b"); tui.pump(0.3)
    # View Settings
    tui.send(":"); tui.pump(0.3); tui.send("view"); tui.pump(0.4)
    tui.send("\r"); tui.pump(0.4)
    txt = tui.text()
    check("T23d settings view overlay", "SETTINGS" in txt and "ssh_user=admin" in txt, txt[-900:])
    tui.send("\x1b"); tui.pump(0.3)
    # T23f: palette Enter must run the HIGHLIGHTED entry, not the top match
    # (regression: every command opened the config editor). j/k are typed
    # input in the palette now ("SparkMax"-style queries), so move with a
    # real arrow key.
    tui.send(":"); tui.pump(0.3)
    tui.send("DOWN"); tui.pump(0.2)  # cursor -> 'Settings: View Settings'
    tui.send("\r"); tui.pump(0.4)
    txt = tui.text()
    check("T23f palette runs highlighted entry", "SETTINGS" in txt, txt[-900:])
    tui.send("\x1b"); tui.pump(0.3)
    # Remove Robot Target
    tui.send(":"); tui.pump(0.3); tui.send("remove"); tui.pump(0.4)
    tui.send("\r"); tui.pump(0.4)
    tui.send("jj"); tui.pump(0.3)  # move to Team 118 (third entry)
    tui.send("\r"); tui.pump(0.5)
    check("T23e target removed toast", "[SUCCESS] Removed Team 118" in tui.text(), tui.text()[-600:])

    # T24: save active watchlist as preset ---------------------------------------
    tui.send("/"); tui.pump(0.3); tui.send("batt"); tui.pump(0.4)
    tui.send(" "); tui.pump(0.4)   # pin battery
    tui.send("\x1b"); tui.pump(0.2)
    tui.send(":"); tui.pump(0.3); tui.send("save"); tui.pump(0.4)
    tui.send("\r"); tui.pump(0.4)
    tui.send("bench"); tui.pump(0.3)
    tui.send("\r"); tui.pump(0.5)
    check("T24a preset saved toast", "[SUCCESS] Preset 'bench' saved" in tui.text(), tui.text()[-600:])
    if os.path.isfile(cfg_path):
        with open(cfg_path) as fh:
            cfg_disk = json.load(fh)
        check("T24b preset persisted", "bench" in cfg_disk.get("presets", {}), cfg_disk.get("presets"))
    else:
        check("T24b preset persisted", False, cfg_path)

    # T25: System: Restart Robot Code dispatches SSH (fails here — no sshd —
    # but must surface an [ERROR] toast, never an NT topic publish)
    tui.send(":"); tui.pump(0.3); tui.send("restart"); tui.pump(0.4)
    txt = tui.text()
    check("T25a palette lists SSH restart", "System: Restart Robot Code" in txt, txt[-900:])
    tui.send("\r"); tui.pump(0.5)
    check("T25b dispatch toast", "restart: ssh admin@127.0.0.1" in tui.text(), tui.text()[-600:])
    # Poll for the failure toast (toast TTL is 3.5s, so poll instead of
    # sleeping past it).
    found = False
    end = time.time() + 8
    while time.time() < end:
        if "[ERROR] restart" in tui.text():
            found = True
            break
        time.sleep(0.2)
    check("T25c ssh failure surfaced", found, tui.text()[-600:])

    # T26: field visualization (pose cards, conservative detection, manual
    # alliance flip) -----------------------------------------------------------
    braille = lambda s: any("\u2800" <= ch <= "\u28ff" for ch in s)

    # T26a: pin the exact-name pose topic -> braille field card appears.
    tui.send("/"); tui.pump(0.2)
    tui.send("botpose_wpiblue"); tui.pump(0.2)
    tui.send(" "); tui.pump(0.1)
    tui.send("\x1b"); tui.pump(0.5)
    txt = tui.text()
    check("T26a field card renders for botpose_wpiblue",
          "botpose_wpiblue" in txt and braille(txt), txt[-800:])

    # T26b: pin the lookalike -> stays a normal array card, no field render.
    tui.send("/"); tui.pump(0.2)
    tui.send("targetpose"); tui.pump(0.2)
    tui.send(" "); tui.pump(0.1)
    tui.send("\x1b"); tui.pump(0.5)
    txt = tui.text()
    check("T26b lookalike targetpose stays a value card", "[1.000," in txt, txt[-800:])

    # T26c: manual opt-in flips the lookalike to a field card (palette).
    # Focus is state-dependent here: ESC forces Tree (no-op from Tree),
    # TAB then guarantees Watchlist. Cards: [Battery, botpose, targetpose];
    # cursor 0, so j twice lands on targetpose.
    tui.send("\x1b"); tui.pump(0.2)  # force Tree focus
    tui.send("\t"); tui.pump(0.2)   # -> Watchlist, cursor 0
    tui.send("j"); tui.pump(0.2)
    tui.send("j"); tui.pump(0.2)    # down to the targetpose card
    tui.send(":"); tui.pump(0.2)
    tui.send("toggle"); tui.pump(0.2)   # send words separately: multi-word
    tui.send("pose"); tui.pump(0.3)     # strings are filtered by send()
    tui.send("\r"); tui.pump(0.5)
    txt = tui.text()
    check("T26c palette toggles pose view on active card",
          "[SUCCESS] field card: SmartDashboard/targetpose" in txt, txt[-800:])
    check("T26d forced lookalike now renders as field card",
          "[1.000," not in txt and braille(txt), txt[-800:])

    # T26e: alliance flip is USER-ONLY, via palette, and mirrored rendering
    # (toast proves the command ran; the mirror itself is a pure function of
    # config.field.alliance exercised in src/field.rs + render path).
    tui.send(":"); tui.pump(0.2)
    tui.send("set"); tui.pump(0.2)
    tui.send("alliance"); tui.pump(0.2)
    tui.send("red"); tui.pump(0.3)
    tui.send("\r"); tui.pump(0.5)
    check("T26e alliance red command confirms (user-driven only)",
          "field: red origin" in tui.text(), tui.text()[-600:])

    # restore state: un-force the lookalike, set alliance back to blue.
    tui.send(":"); tui.pump(0.2)
    tui.send("toggle"); tui.pump(0.2)
    tui.send("pose"); tui.pump(0.3)
    tui.send("\r"); tui.pump(0.4)
    tui.send(":"); tui.pump(0.2)
    tui.send("set"); tui.pump(0.2)
    tui.send("alliance"); tui.pump(0.2)
    tui.send("blue"); tui.pump(0.3)
    tui.send("\r"); tui.pump(0.4)
    # T26f/g: map cycling (palette) — 2025 -> 2026 -> 2024 -> 2025 restore.
    tui.send(":"); tui.pump(0.2)
    tui.send("cycle"); tui.pump(0.2)
    tui.send("map"); tui.pump(0.3)
    tui.send("\r"); tui.pump(0.5)
    check("T26f cycle map command confirms",
          "field map: 2026-tba" in tui.text(), tui.text()[-600:])
    tui.send(":"); tui.pump(0.2)
    tui.send("cycle"); tui.pump(0.2)
    tui.send("map"); tui.pump(0.3)
    tui.send("\r"); tui.pump(0.5)
    check("T26g cycle wraps to 2024 and back to 2025",
          "field map: 2024-crescendo" in tui.text(), tui.text()[-600:])
    tui.send(":"); tui.pump(0.2)
    tui.send("cycle"); tui.pump(0.2)
    tui.send("map"); tui.pump(0.3)
    tui.send("\r"); tui.pump(0.4)  # back to 2025-reefscape (config restored)

    # dismiss the two pose cards; cursor sits on targetpose. x removes it
    # (cursor clamps to botpose), a second x removes botpose. Battery stays.
    tui.send("x"); tui.pump(0.2)
    tui.send("x"); tui.pump(0.3)

    # T14: quit ---------------------------------------------------------------------
    tui.send("q")
    # Poll for exit: a just-spawned ssh child can hold the stdout pipe for a
    # few seconds after the parent exits.
    end = time.time() + 10
    while time.time() < end and tui.alive:
        time.sleep(0.3)
    check("T14a quits on q", not tui.alive, f"alive={tui.alive}")
    check("T14b clean exit code", tui.exitstatus in (0, None) and not tui.alive, tui.exitstatus)

    server.stop()

    # summary ------------------------------------------------------------------------
    try:
        os.remove(os.path.join(ROOT, ".nt-views.json"))
    except OSError:
        pass
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
