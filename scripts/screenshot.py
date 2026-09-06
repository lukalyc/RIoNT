"""Generate README screenshots: real RIONT binary + real ntcore server.

Runs the debug binary headless against test/server.py with a pre-seeded
watchlist (config last_view), captures the pyte screen buffer at chosen
moments, and renders PNGs with Pillow (per-cell fg/bg/bold/reverse).

Usage: conda run -n nt-tui-test python scripts/screenshot.py
Output: docs/screenshot-main.png, docs/screenshot-field.png
"""
import codecs
import json
import os
import subprocess
import sys
import threading
import time

import pyte
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug",
                   "riont.exe" if os.name == "nt" else "riont")
SERVER = os.path.join(ROOT, "test", "server.py")
CFG = os.path.join(ROOT, "test", "screenshot-config.json")
OUT = os.path.join(ROOT, "docs")
COLS, ROWS = 120, 36

# --- terminal palette (gruvbox-dark, matches the app's own constants) -----
BG = (0x1D, 0x20, 0x21)
FG = (0xEB, 0xDB, 0xB2)
NAMED = {
    "black": (0x28, 0x28, 0x28), "red": (0xFB, 0x49, 0x34),
    "green": (0xB8, 0xBB, 0x26), "yellow": (0xFA, 0xBD, 0x2F),
    "brown": (0xFA, 0xBD, 0x2F), "blue": (0x83, 0xA5, 0x98),
    "magenta": (0xD3, 0x86, 0x9B), "cyan": (0x8E, 0xC0, 0x7C),
    "white": (0xEB, 0xDB, 0xB2),
    "brightblack": (0x9A, 0x93, 0x86), "brightred": (0xFB, 0x49, 0x34),
    "brightgreen": (0xB8, 0xBB, 0x26), "brightyellow": (0xFA, 0xBD, 0x2F),
    "brightblue": (0x83, 0xA5, 0x98), "brightmagenta": (0xD3, 0x86, 0x9B),
    "brightcyan": (0x8E, 0xC0, 0x7C), "brightwhite": (0xFF, 0xFF, 0xFF),
}

# crossterm emits xterm-256 codes for the 8 base colors (pyte resolves them
# to the xterm hexes below); translate to the gruvbox palette the app was
# designed against.
XTERM256 = {
    "800000": NAMED["red"], "00cd00": NAMED["green"], "cdcd00": NAMED["yellow"],
    "0000ee": NAMED["blue"], "cd00cd": NAMED["magenta"], "00cdcd": NAMED["cyan"],
    "e5e5e5": NAMED["white"], "7f7f7f": NAMED["brightblack"],
    "ff0000": NAMED["brightred"], "00ff00": NAMED["brightgreen"],
    "ffff00": NAMED["brightyellow"], "5c5cff": NAMED["brightblue"],
    "ff00ff": NAMED["brightmagenta"], "00ffff": NAMED["brightcyan"],
}

FONT_SIZE = 20                      # px; DejaVu Mono advance = 0.6 * size
CELL_W, CELL_H = 12, 24             # exact multiples -> crisp image
SCALE = 2                           # supersample for hidpi crispness
MONO = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
MONO_B = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
BRAILLE = "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf"


def color(spec, is_bg=False):
    """pyte color spec -> RGB tuple. Unset bg = terminal dark, unset
    fg = light text — they must NOT share one default."""
    if spec in (None, "default"):
        return BG if is_bg else FG
    if spec in NAMED:
        return NAMED[spec]
    if isinstance(spec, str) and spec in XTERM256:
        return XTERM256[spec]
    if isinstance(spec, str) and len(spec) in (3, 6) \
            and all(c in "0123456789abcdef" for c in spec):
        n = int(spec, 16)
        if len(spec) == 3:
            n = int("".join(c * 2 for c in spec), 16)
        return ((n >> 16) & 255, (n >> 8) & 255, n & 255)
    return FG


def render_png(screen, path):
    w, h = COLS * CELL_W * SCALE, ROWS * CELL_H * SCALE
    img = Image.new("RGB", (w, h), tuple(c * SCALE and c for c in BG))
    draw = ImageDraw.Draw(img)
    f = ImageFont.truetype(MONO, FONT_SIZE * SCALE)
    fb = ImageFont.truetype(MONO_B, FONT_SIZE * SCALE)
    fbraille = ImageFont.truetype(BRAILLE, int(FONT_SIZE * 1.05 * SCALE))
    for y in range(ROWS):
        for x in range(COLS):
            cell = screen.buffer[y][x]
            ch = cell.data
            if ch in (" ", ""):
                continue
            fg = color(cell.fg) if not cell.reverse else color(cell.bg, True)
            bg = color(cell.bg, True) if not cell.reverse else color(cell.fg)
            if bg != BG:
                draw.rectangle(
                    [x * CELL_W * SCALE, y * CELL_H * SCALE,
                     (x + 1) * CELL_W * SCALE - 1, (y + 1) * CELL_H * SCALE - 1],
                    fill=bg)
            font = fb if cell.bold else f
            if "\u2800" <= ch <= "\u28ff":
                font = fbraille
            # center glyph in cell
            bbox = draw.textbbox((0, 0), ch, font=font)
            cw, chh = bbox[2] - bbox[0], bbox[3] - bbox[1]
            px = x * CELL_W * SCALE + (CELL_W * SCALE - cw) // 2 - bbox[0]
            py = y * CELL_H * SCALE + (CELL_H * SCALE - chh) // 2 - bbox[1]
            draw.text((px, py), ch, font=font, fill=fg)
    img.save(path)
    print("wrote", path, f"({w}x{h})")


def main():
    os.makedirs(OUT, exist_ok=True)
    # Pre-seeded watchlist: what a connected operator sees.
    with open(CFG, "w") as fh:
        json.dump({
            "last_target": "127.0.0.1:5814",
            "last_view": [
                "SmartDashboard/Battery Voltage",
                "SmartDashboard/Shooter RPM",
                "SmartDashboard/botpose_wpiblue",
            ],
            "saved_targets": [
                {"name": "Simulation", "ip": "127.0.0.1:5810"},
                {"name": "USB Tether", "ip": "172.22.11.2"},
            ],
            "presets": {},
            "system": {"ssh_user": "admin",
                       "restart_cmd": "/usr/local/frc/bin/frcRunRobot.sh restart"},
            "field": {"alliance": "blue", "length_m": 16.54, "width_m": 8.21},
        }, fh)

    server = subprocess.Popen([sys.executable, SERVER], cwd=ROOT,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    env = dict(os.environ, RIONT_HEADLESS="1", RIONT_SIZE=f"{COLS}x{ROWS}",
               RIONT_CONFIG=CFG)
    p = subprocess.Popen([EXE, "127.0.0.1:5814"], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                         env=env, cwd=ROOT)

    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.Stream()
    stream.attach(screen)
    decoder = codecs.getincrementaldecoder("utf-8")()

    def reader():
        while True:
            d = p.stdout.read1(65536)
            if not d:
                break
            stream.feed(decoder.decode(d))
    threading.Thread(target=reader, daemon=True).start()

    def wait(pred, timeout=15):
        end = time.time() + timeout
        while time.time() < end:
            if pred():
                return True
            time.sleep(0.05)
        return False

    def send(keys):
        p.stdin.write((keys + "\n").encode())
        p.stdin.flush()

    try:
        # boot + let values stream (CODE RUNNING, Hz samples, trail points)
        wait(lambda: "COMM: ONLINE" in screen.display[0])
        time.sleep(4.0)
        render_png(screen, os.path.join(OUT, "screenshot-main.png"))

        # enlarged field view: watchlist focus, hover the pose card, f
        send("TAB")         # TAB token -> watchlist
        time.sleep(0.4)
        send("j")
        time.sleep(0.25)
        send("j")
        time.sleep(0.4)
        send("f")
        wait(lambda: "FIELD VIEW" in "\n".join(screen.display), timeout=5)
        time.sleep(0.6)
        render_png(screen, os.path.join(OUT, "screenshot-field.png"))
    finally:
        p.kill()
        server.terminate()
        try:
            server.wait(timeout=5)
        except Exception:
            server.kill()
        os.remove(CFG)


if __name__ == "__main__":
    main()
