"""End-to-end contract harness — RIONT dashboard.

Design (read this before editing — the rules keep agent time low):

1. CONTRACT, NOT COPY. This harness verifies what crosses the process
   boundary: values the fake robot receives (ntcore subscriber round-trips),
   values the server published that appear on screen, HUD state
   transitions, watchlist counts, and config-file side effects. It NEVER
   asserts on exact UI wording, hint lines, geometry constants, or colors —
   that is the job of the in-process Rust tests (`cargo test`, see
   src/tests_tui.rs). If a wording change breaks this harness, the harness
   is wrong, not the code.

2. WAIT, DON'T SLEEP. Every interaction is `tap(keys, expect)` /
   `wait_until(cond)` — poll for the expected state with a generous
   timeout. There are no fixed sleeps between action and assertion. A slow
   machine changes nothing; a real regression times out once and fails
   fast.

3. FAIL FAST, SHOW EVERYTHING. The first failed check dumps the complete
   screen, the recent keystrokes, and aborts. Downstream checks of a
   stateful scenario are meaningless after an upstream failure; making an
   agent "fix" 40 cascade failures wastes time and teaches it to patch
   assertions. Set RIONT_HARNESS_CONTINUE=1 to run everything anyway
   (summary mode for final verification).

4. HERMETIC. The TUI under test runs with RIONT_CONFIG pointed at a scratch
   file, so runs never touch the developer's ~/.config/riont. A zombie NT4
   server from a crashed run is killed before the port is claimed.

Run:  conda run -n nt-tui-test python test/harness.py
"""
import codecs
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import threading
import time

import pyte

import ntcore

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
IS_WIN = os.name == "nt"
EXE = os.path.join(ROOT, "target", "debug", "riont.exe" if IS_WIN else "riont")
SERVER = os.path.join(ROOT, "test", "server.py")
PY = sys.executable
COLS, ROWS = 120, 36
TARGET = "127.0.0.1:5814"
PORT = 5814

# Hermetic config: the TUI writes here instead of ~/.config/riont.
CFG_PATH = os.path.join(ROOT, "test", "riont-harness-config.json")

os.environ["RIONT_DEBUG"] = "1"


def cargo_version():
    """Parse `version = "x.y.z"` from Cargo.toml — single source of truth."""
    with open(os.path.join(ROOT, "Cargo.toml")) as fh:
        m = re.search(r'^version\s*=\s*"([^"]+)"', fh.read(), re.M)
    return m.group(1) if m else None


def ensure_binary():
    """Build the debug binary if missing, so agents can't test a stale tree
    by accident and never have to remember this step."""
    if os.path.exists(EXE):
        return
    print("debug binary missing — running cargo build ...", flush=True)
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)
    assert os.path.exists(EXE), f"cargo build did not produce {EXE}"


# ---------------------------------------------------------------------------
# checking: fail fast, show everything
# ---------------------------------------------------------------------------

class HarnessFailure(Exception):
    pass


CURRENT = {"screen": lambda: "", "keys": []}


def full_screen():
    return CURRENT["screen"]()


def dump_context(reason):
    keys = CURRENT["keys"][-14:]
    print("\n" + "=" * 78, flush=True)
    print(f"CHECK FAILED: {reason}", flush=True)
    print("=" * 78)
    print(f"recent keystrokes: {keys}", flush=True)
    print("---- full screen at failure -------------------------------------")
    for ln in full_screen().splitlines():
        print(f"|{ln}|", flush=True)
    print("=" * 78 + "\n", flush=True)


def check(name, cond, detail=""):
    """Fail-fast assertion: dump the whole screen on failure and abort."""
    if cond:
        print(f"[PASS] {name}", flush=True)
        return
    dump_context(f"{name}   | {str(detail)[:300]}")
    if os.environ.get("RIONT_HARNESS_CONTINUE") == "1":
        print(f"[FAIL] {name}", flush=True)
        FAILED.append(name)
    else:
        raise HarnessFailure(name)


FAILED = []


def wait_until(cond, timeout=6.0, desc="condition", poll=0.05):
    """Poll `cond` until true. The ONLY synchronization primitive — no
    fixed sleeps between a keystroke and the assertion that follows it."""
    end = time.time() + timeout
    while True:
        try:
            if cond():
                return True
        except Exception:
            pass
        if time.time() >= end:
            return False
        time.sleep(poll)


# ---------------------------------------------------------------------------
# TUI driver
# ---------------------------------------------------------------------------

class Tui:
    """Headless TUI driver: stdin = key script, stdout = ANSI render."""

    def __init__(self, target=TARGET):
        env = dict(
            os.environ,
            RIONT_HEADLESS="1",
            RIONT_SIZE=f"{COLS}x{ROWS}",
            RIONT_CONFIG=CFG_PATH,
        )
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

    WORD_TOKENS = ("UP", "DOWN", "LEFT", "RIGHT")

    def send(self, keys):
        """Send keys: one char = one key press. Control chars are encoded as
        tokens (RET/ESC/TAB/SPC); arrows as whole words ("DOWN")."""
        if not self.alive:
            return
        if keys in self.WORD_TOKENS:
            parts = [keys]
        elif keys == " ":
            parts = ["SPC"]
        elif " " in keys:
            parts = [p for p in keys.split(" ") if p in self.WORD_TOKENS]
        else:
            token_map = {"\r": "RET", "\x1b": "ESC", "\t": "TAB", " ": "SPC"}
            parts = [token_map.get(c, c) for c in keys]
        CURRENT["keys"].append(keys)
        try:
            self.proc.stdin.write(("\x1f".join(parts) + "\n").encode())
            self.proc.stdin.flush()
        except Exception:
            pass

    def tap(self, keys, expect, timeout=6.0, desc=None):
        """Send keys, then wait for the expected state. `expect` is a
        predicate over the Tui. This replaces send()+pump()+assert()."""
        self.send(keys)
        ok = wait_until(expect, timeout=timeout, desc=desc or repr(keys))
        if not ok and not os.environ.get("RIONT_HARNESS_CONTINUE") == "1":
            dump_context(f"tap({keys!r}) did not reach {desc or 'expected state'} within {timeout}s")
            raise HarnessFailure(f"tap {keys!r}")
        return ok

    # -- viewport accessors ------------------------------------------------

    def lines(self):
        return [ln.rstrip() for ln in self.screen.display]

    def text(self):
        return "\n".join(self.lines())

    def header(self):
        return self.lines()[0]

    def watchlist_count(self):
        m = re.search(r"WATCHLIST \((\d+)\)", self.text())
        return int(m.group(1)) if m else -1

    def tree_cursor_rows(self):
        return [i for i, ln in enumerate(self.lines()) if ln[1:3] == "> "]

    def canvas_seg(self, i):
        parts = self.lines()[i].split("│")
        return parts[-2] if len(parts) >= 5 else ""

    def braille_chars(self):
        return sum(
            1 for ch in self.text() if "\u2800" <= ch <= "\u28ff"
        )

    def close(self):
        try:
            self.proc.kill()
        except Exception:
            pass


# ---------------------------------------------------------------------------
# scenario grounding helpers (each scenario starts from a known state)
# ---------------------------------------------------------------------------

def wait_online(tui, timeout=15.0):
    return wait_until(lambda: "COMM: ONLINE" in tui.header(), timeout=timeout,
                      desc="COMM: ONLINE")


def esc_to_tree(tui):
    """Esc always lands Normal/Tree focus (overlays close, watchlist
    returns focus). Wait for the tree hint line to prove it."""
    tui.send("\x1b")
    wait_until(lambda: tui.tree_cursor_rows(), timeout=4, desc="tree cursor visible")


def goto_topic(tui, query):
    """Search-jump onto a topic: deterministic cursor positioning. Waits
    for the overlay to CLOSE — the proof the jump actually ran."""
    tui.tap("/", lambda: "search —" in tui.text(), desc="search overlay")
    tui.send(query)
    tui.tap("\r", lambda: "search —" not in tui.text(), desc="jump landed")


def clear_watchlist(tui):
    """Reset to an empty watchlist via the palette (state-independent)."""
    tui.send(":")
    wait_until(lambda: "commands" in tui.text(), timeout=4, desc="palette")
    tui.send("clear")
    tui.send("\r")
    wait_until(lambda: tui.watchlist_count() == 0, timeout=5, desc="WATCHLIST (0)")
    esc_to_tree(tui)


# ---------------------------------------------------------------------------
# fake robot (real ntcore NT4 server) + harness subscriber
# ---------------------------------------------------------------------------

class Server:
    def __init__(self):
        self.p = None

    def start(self):
        _kill_port(PORT)
        self.log = open(os.path.join(ROOT, "test", "server.log"), "ab")
        self.p = subprocess.Popen([PY, SERVER], cwd=ROOT, stdout=self.log,
                                  stderr=self.log)
        assert wait_port(PORT), "server did not open port 5814"

    def stop(self):
        if self.p:
            self.p.terminate()
            try:
                self.p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.p.kill()
            self.p = None


def _kill_port(port):
    """Best-effort cleanup of a zombie server holding the port (crashed
    previous run). Cross-platform, tolerates missing tools."""
    if IS_WIN:
        out = subprocess.run(["netstat", "-ano"], capture_output=True, text=True).stdout
        for ln in out.splitlines():
            if str(port) in ln and "LISTENING" in ln:
                subprocess.run(["taskkill", "/F", "/PID", ln.split()[-1]],
                               capture_output=True)
    else:
        subprocess.run(["pkill", "-f", "test/server.py"], capture_output=True)
        if shutil.which("fuser"):
            subprocess.run(["fuser", "-k", f"{port}/tcp"], capture_output=True)


def wait_port(port, timeout=15.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.2)
    return False


def wait_topic(sub, want, timeout=4.0):
    """Poll a subscriber until it reports `want` — the write CONTRACT."""
    end = time.time() + timeout
    last = sub.get()
    while time.time() < end:
        if last == want:
            return True
        time.sleep(0.05)
        last = sub.get()
    return False


# ---------------------------------------------------------------------------
# scenarios
# ---------------------------------------------------------------------------

def scenario_boot(server, harness, tui):
    # Hermetic: fresh scratch config so runs are independent.
    if os.path.exists(CFG_PATH):
        os.remove(CFG_PATH)
    with open(os.path.join(ROOT, ".nt-views.json"), "w") as fh:
        fh.write(json.dumps([
            {"name": "Test", "topics": ["SmartDashboard/Shooter RPM", "Swerve/*"]},
            {"name": "Dense", "topics": ["Swerve/*", "SmartDashboard/*"]},
        ]))
    check("boot TUI reaches COMM ONLINE", wait_online(tui), tui.header())
    # Values stream right after connect: wait for the value-derived HUD
    # (uptime + CODE RUNNING) before any value assertions downstream.
    check("robot frames stream (CODE RUNNING)", wait_until(
        lambda: "CODE: RUNNING" in tui.header() and "--:--:--" not in tui.header(),
        timeout=10), tui.header())


def scenario_hud(tui):
    h = tui.header()
    check("HUD shows ONLINE + target ip", "ONLINE" in h and "127.0.0.1" in h, h)
    check("HUD shows CODE RUNNING", "CODE: RUNNING" in h, h)
    check("HUD shows UPTIME clock", "UPTIME: " in h and ":" in h.split("UPTIME: ")[1], h)
    m = re.search(r"(\d+) topics", h)
    check("HUD topic count > 10", m and int(m.group(1)) > 10, h)
    check("HUD no global Hz / rtt metric", " Hz" not in h and "rtt" not in h, h)
    v = cargo_version()
    check("HUD version == Cargo.toml version (dynamic)",
          v and f"RIONT v{v}" in h, f"cargo={v} hud={h}")

def scenario_tree(tui):
    txt = tui.text()
    check("tree starts collapsed", "[+] SmartDashboard" in txt, txt)
    goto_topic(tui, "batt")
    txt = tui.text()
    check("search-jump expands ancestors", "[-] SmartDashboard" in txt, txt)
    row = next((l for l in tui.lines() if "Battery Voltage" in l), "")
    check("expanded row shows type", "double" in row, row)
    check("expanded row shows server value", "12." in row, row)
    check("expanded row shows Hz", "Hz" in row, row)

    cur0 = tui.tree_cursor_rows()
    tui.send("j")
    wait_until(lambda: tui.tree_cursor_rows() != cur0, desc="cursor moves")
    check("j moves cursor", tui.tree_cursor_rows() != cur0, cur0)

    cur_after_j = tui.tree_cursor_rows()
    tui.send("G")
    wait_until(lambda: tui.tree_cursor_rows() != cur_after_j, desc="G jumps")
    bottom = tui.tree_cursor_rows()
    tui.send("g")
    wait_until(lambda: tui.tree_cursor_rows() != bottom, desc="g jumps")
    top = tui.tree_cursor_rows()
    check("G/g jump to ends", bool(bottom and top and bottom != top), f"{top}/{bottom}")

    # Re-ground on a SmartDashboard topic (G/g left the cursor anywhere),
    # then h folds its parent dir.
    goto_topic(tui, "batt")
    tui.send("h")
    check("h folds dir", wait_until(
        lambda: "[-] SmartDashboard" not in tui.text()
        and "[+] SmartDashboard" in tui.text()), tui.text())
    esc_to_tree(tui)


def scenario_dock(tui):
    goto_topic(tui, "batt")
    txt = tui.text()
    check("dock mirrors full path", "SmartDashboard/Battery Voltage" in txt, txt)
    check("dock shows type", "double" in txt, txt)
    check("dock shows rate", re.search(r"\d+\.\d Hz", txt), txt)
    check("dock shows delta", "Δ " in txt, txt)
    check("dock shows value", re.search(r"12\.\d", txt), txt)


def scenario_pin(tui):
    clear_watchlist(tui)
    goto_topic(tui, "batt")
    tui.tap(" ", lambda: tui.watchlist_count() == 1, desc="card pinned")
    txt = tui.text()
    check("pin toast success + path",
          "[SUCCESS]" in txt and "Battery Voltage" in txt, txt[-400:])
    check("watchlist card appeared", "┌ SmartDashboard/Battery Voltage" in txt, txt[-800:])
    star = next((i for i, ln in enumerate(tui.lines())
                 if ln.startswith("│") and "* Battery Voltage" in ln.split("│")[1]), -1)
    check("pinned star in tree", star >= 0, tui.text()[:900])
    clear_watchlist(tui)


def scenario_edit_publish(tui, kp_read, al_read, cl_read):
    clear_watchlist(tui)
    # double round-trip: THE write contract
    goto_topic(tui, "kp")
    tui.tap("e", lambda: "enter=publish" in tui.text(), desc="edit prompt")
    tui.send("0.05")
    tui.tap("\r", lambda: "[SUCCESS]" in tui.text() and "kP" in tui.text(),
            desc="publish toast")
    check("server received 0.05", wait_topic(kp_read, 0.05), kp_read.get())
    txt = tui.text()
    check("publish toast names topic", "kP" in txt and "[SUCCESS]" in txt, txt[-300:])

    # invalid input: editor stays open, nothing published
    goto_topic(tui, "kp")
    tui.send("e")
    tui.send("abc")
    tui.send("\r")
    check("invalid input keeps editor open", wait_until(
        lambda: "Set SmartDashboard/kP:" in tui.text()), tui.text()[-300:])
    tui.tap("\x1b", lambda: "Set SmartDashboard/kP:" not in tui.text(),
            desc="esc closes editor")

    # string round-trip
    goto_topic(tui, "alliance")
    tui.send("e")
    tui.send("red")
    tui.send("\r")
    check("string publish reaches server", wait_topic(al_read, "red"), al_read.get())

    # boolean round-trip
    goto_topic(tui, "climb")
    tui.send("e")
    tui.send("true")
    tui.send("\r")
    check("boolean publish reaches server", wait_topic(cl_read, True),
          cl_read.get())

    # non-writable type rejected
    goto_topic(tui, "faults")
    tui.send("e")
    check("array edit rejected with [ERROR]", wait_until(
        lambda: "[ERROR]" in tui.text(), timeout=3), tui.lines()[-6:])
    esc_to_tree(tui)


def scenario_stacking(tui):
    clear_watchlist(tui)
    for i, term in enumerate(("gyro", "match", "shooter", "compressor")):
        goto_topic(tui, term)
        tui.tap(" ", lambda i=i: tui.watchlist_count() == i + 1,
                desc=f"card {i + 1} pinned")
        esc_to_tree(tui)
    check("4 scalar cards pinned", tui.watchlist_count() == 4,
          tui.watchlist_count())
    two_col = any(tui.canvas_seg(i).count("┌") >= 2 for i in range(len(tui.lines())))
    check("height-first: single column while cards fit", not two_col, tui.text()[:1500])

    # removal contract
    tui.tap("\t", lambda: True, desc="focus watchlist")
    n = tui.watchlist_count()
    tui.tap("x", lambda: tui.watchlist_count() == n - 1, desc="card removed")
    check("unpin surfaces [INFO]", "[INFO]" in tui.text(), tui.text()[-400:])

    # palette clear contract
    tui.send(":")
    tui.send("clear")
    tui.send("\r")
    check("palette clear empties watchlist", wait_until(
        lambda: tui.watchlist_count() == 0), tui.text())
    esc_to_tree(tui)


def scenario_reconnect(tui, server):
    server.stop()
    check("HUD leaves ONLINE when server dies", wait_until(
        lambda: "RECONNECTING" in tui.header() or "DISCONNECTED" in tui.header(),
        timeout=15), tui.header())
    check("disconnect surfaces [ERROR]", wait_until(
        lambda: "[ERROR]" in tui.text(), timeout=6), tui.text()[-400:])
    server.start()
    check("HUD returns ONLINE after server restart", wait_online(tui), tui.header())
    check("values flow again after reconnect",
          wait_until(lambda: "Battery Voltage" in tui.text(), timeout=6),
          tui.text())

    # palette-driven reconnect
    tui.send(":")
    tui.send("recon")
    tui.send("\r")
    check("palette reconnect returns ONLINE", wait_online(tui, timeout=8),
          tui.header())


def scenario_retarget(tui):
    # picker: type a dead target -> DISCONNECTED; then back -> ONLINE.
    tui.tap("c", lambda: "[CONNECT TARGET]" in tui.text(), desc="picker opens")
    tui.send("127.0.0.1:5999")
    tui.send("\r")
    check("dead target leaves ONLINE", wait_until(
        lambda: "COMM: ONLINE" not in tui.header(), timeout=15), tui.header())
    tui.tap("c", lambda: "[CONNECT TARGET]" in tui.text(), desc="picker again")
    tui.send("127.0.0.1:5814")
    tui.send("\r")
    check("retarget back to live server", wait_online(tui), tui.header())
    check("values flow after retarget",
          wait_until(lambda: "Battery Voltage" in tui.text(), timeout=6), tui.text())

    with open(CFG_PATH) as fh:
        saved = json.load(fh).get("last_target", "")
    check("last target persisted to RIONT_CONFIG file",
          saved == "127.0.0.1:5814", saved or "missing")


def scenario_presets(tui):
    clear_watchlist(tui)
    tui.tap("1", lambda: tui.watchlist_count() > 0, desc="preset 1 loads")
    txt = tui.text()
    check("preset 1 pins Shooter RPM + Swerve",
          "Shooter RPM" in txt and "Velocity" in txt, txt[:400])
    tui.tap("2", lambda: tui.watchlist_count() > 12, desc="dense preset")
    multi = any(tui.canvas_seg(i).count("┌") >= 2 for i in range(len(tui.lines())))
    check("dense preset overflows into columns", multi, tui.text()[:1200])
    clear_watchlist(tui)


def scenario_settings(tui):
    tui.send(":")
    tui.send("add")
    tui.send("\r")
    check("add-target prompt opens", wait_until(
        lambda: "[ADD ROBOT TARGET]" in tui.text(), timeout=4), tui.text()[-400:])
    tui.send("118")
    tui.send("\r")
    check("target added with [SUCCESS]", wait_until(
        lambda: "[SUCCESS]" in tui.text() and "118" in tui.text(),
        timeout=4), tui.text()[-300:])
    tui.tap("c", lambda: "[CONNECT TARGET]" in tui.text(), desc="picker")
    txt = tui.text()
    check("picker lists new target with resolved IP",
          "Team 118" in txt and "10.1.18.2" in txt, txt)
    esc_to_tree(tui)

    tui.send(":")
    tui.send("remove")
    tui.send("\r")
    tui.send("j")
    tui.send("j")
    tui.send("\r")
    check("target removed with [SUCCESS]", wait_until(
        lambda: "[SUCCESS]" in tui.text() and "Removed" in tui.text(),
        timeout=4), tui.text()[-300:])


def scenario_save_preset(tui):
    clear_watchlist(tui)
    goto_topic(tui, "batt")
    tui.tap(" ", lambda: True, desc="pin battery")
    esc_to_tree(tui)
    tui.send(":")
    tui.send("save")
    tui.send("\r")
    tui.send("bench")
    tui.send("\r")
    check("preset saved with [SUCCESS]", wait_until(
        lambda: "[SUCCESS]" in tui.text() and "bench" in tui.text(),
        timeout=4), tui.text()[-300:])
    with open(CFG_PATH) as fh:
        cfg = json.load(fh)
    check("preset persisted to config file", "bench" in cfg.get("presets", {}),
          cfg.get("presets"))
    clear_watchlist(tui)


def scenario_ssh_restart(tui):
    tui.send(":")
    tui.send("restart")
    tui.send("\r")
    check("ssh dispatch surfaced [ERROR] (no sshd here)", wait_until(
        lambda: "[ERROR]" in tui.text(), timeout=10), tui.text()[-400:])


def scenario_field(tui):
    clear_watchlist(tui)
    # exact pose name -> field card with braille
    goto_topic(tui, "botpose_wpiblue")
    tui.tap(" ", lambda: True, desc="pin botpose")
    check("field card draws braille", wait_until(
        lambda: tui.braille_chars() > 20, timeout=4), tui.text()[-500:])

    # lookalike stays a value card
    goto_topic(tui, "targetpose")
    tui.tap(" ", lambda: True, desc="pin targetpose")
    esc_to_tree(tui)
    txt = tui.text()
    check("lookalike targetpose stays a value card",
          "[1.000" in txt and "targetpose" in txt, txt[-600:])

    # palette forces the lookalike into a field card (opt-in contract)
    tui.send("\t")  # watchlist focus, cursor on card 0
    tui.send("j")
    tui.send("j")   # down to targetpose card
    tui.send(":")
    tui.send("toggle")
    tui.send("pose")
    tui.send("\r")
    check("forced lookalike becomes a field card", wait_until(
        lambda: tui.braille_chars() > 20, timeout=4), tui.text()[-500:])
    # undo the force + unpin (restore state)
    tui.send(":")
    tui.send("toggle")
    tui.send("pose")
    tui.send("\r")
    clear_watchlist(tui)

    # empty estimate keeps the field card (sticky pose contract)
    goto_topic(tui, "botpose_wpiblue")
    tui.tap(" ", lambda: True, desc="pin botpose")
    esc_to_tree(tui)
    ntcore.NetworkTableInstance.getDefault().getTable("SmartDashboard") \
        .putBoolean("EmitEmptyPose", True)
    try:
        check("empty estimate keeps field card", wait_until(
            lambda: tui.braille_chars() > 20 and "botpose_wpiblue" in tui.text(),
            timeout=5), tui.text()[-500:])
    finally:
        ntcore.NetworkTableInstance.getDefault().getTable("SmartDashboard") \
            .putBoolean("EmitEmptyPose", False)
    clear_watchlist(tui)


def scenario_quit(tui, server):
    tui.send("q")
    check("quits on q", wait_until(lambda: not tui.alive, timeout=10),
          f"alive={tui.alive}")
    check("clean exit code", tui.exitstatus in (0, None), tui.exitstatus)


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main():
    ensure_binary()
    server = Server()
    server.start()
    print("server up", flush=True)

    harness = ntcore.NetworkTableInstance.getDefault()
    harness.startClient4("harness")
    harness.setServer("127.0.0.1", PORT)
    for _ in range(50):
        if harness.isConnected():
            break
        time.sleep(0.1)
    assert harness.isConnected(), "harness could not connect to server"

    opts = ntcore.PubSubOptions(periodic=0.05, topicsOnly=False)
    kp_read = harness.getDoubleTopic("/SmartDashboard/kP").subscribe(-1.0, opts)
    al_read = harness.getStringTopic("/SmartDashboard/Alliance").subscribe("", opts)
    cl_read = harness.getBooleanTopic("/SmartDashboard/Climb Locked").subscribe(False, opts)

    tui = Tui()
    CURRENT["screen"] = tui.text

    steps = [
        ("boot", lambda: scenario_boot(server, harness, tui)),
        ("hud", lambda: scenario_hud(tui)),
        ("tree", lambda: scenario_tree(tui)),
        ("dock", lambda: scenario_dock(tui)),
        ("pin", lambda: scenario_pin(tui)),
        ("edit+publish", lambda: scenario_edit_publish(tui, kp_read, al_read, cl_read)),
        ("stacking", lambda: scenario_stacking(tui)),
        ("reconnect", lambda: scenario_reconnect(tui, server)),
        ("retarget", lambda: scenario_retarget(tui)),
        ("presets", lambda: scenario_presets(tui)),
        ("settings", lambda: scenario_settings(tui)),
        ("save-preset", lambda: scenario_save_preset(tui)),
        ("ssh-restart", lambda: scenario_ssh_restart(tui)),
        ("field", lambda: scenario_field(tui)),
        ("quit", lambda: scenario_quit(tui, server)),
    ]

    status = 0
    try:
        for name, fn in steps:
            print(f"\n--- scenario: {name} ---", flush=True)
            try:
                fn()
            except HarnessFailure:
                status = 1
                break
    finally:
        tui.close()
        server.stop()
        try:
            os.remove(os.path.join(ROOT, ".nt-views.json"))
        except OSError:
            pass

    print("", flush=True)
    if FAILED:
        print(f"==== {len(FAILED)} check(s) failed (continue mode) ====")
        for n in FAILED:
            print(f"  - {n}")
        return 1
    if status:
        print("==== harness aborted at first failure (fail-fast mode) ====")
        print("    all checks before it passed; fix that one, re-run.")
        return 1
    print("==== all checks passed ====")
    return 0


if __name__ == "__main__":
    sys.exit(main())
