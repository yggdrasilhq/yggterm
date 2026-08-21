#!/usr/bin/env python3
"""Synthetic proof that paint-diff's geometry and verdicts are right.

Builds a PNG that paints SOME of a grid and a sidecar that claims ALL of it,
with three planted faults in known places, then checks paint-diff finds exactly
those and nothing else.

⛔ THE FLAT-BACKGROUND ROWS ARE THE POINT OF THE FIXTURE, not decoration. A TUI's
chrome — status bars, footers, selected rows — is mostly cells that are STYLED
and EMPTY. An ink test that asked "does this differ from the terminal background"
would call every one of them a ghost, and the instrument would report a screen
full of faults on a perfectly painted terminal. Rows 18-19 are painted a solid
non-default colour with nothing written in them, and they must come back blank.
"""
import json, subprocess, sys, tempfile
from pathlib import Path
from PIL import Image, ImageDraw

OUT = Path(tempfile.mkdtemp(prefix="paint-diff-selftest-"))
ROWS, COLS = 20, 40
CW, CH = 8.0, 16.0          # css cell
LEFT, TOP = 30.0, 24.0      # screen rect inside the window
WIN_W, WIN_H = 800.0, 600.0
DPR = 2.0

png = OUT / "synth.png"
img = Image.new("RGB", (int(WIN_W*DPR), int(WIN_H*DPR)), (20, 20, 24))
d = ImageDraw.Draw(img)

# buffer content: rows 0..14 hold "x" in cols 2..30; rows 15..19 blank
masks, lines = [], []
for r in range(ROWS):
    if r < 15:
        m = "".join("#" if 2 <= c <= 30 else "." for c in range(COLS))
        lines.append(" " * 2 + "x" * 29)
    else:
        m = "." * COLS
        lines.append("")
    masks.append(m)

def cell_box(r, c):
    x0 = (LEFT + CW * c) * DPR
    y0 = (TOP + CH * r) * DPR
    return (round(x0), round(y0), round(x0 + CW*DPR), round(y0 + CH*DPR))

# PAINT: rows 0..11 fully, row 12 only cols 2..10 (PARTIAL), rows 13-14 not at
# all (MISSING x2), and row 17 painted though the buffer is blank (GHOST).
for r in range(ROWS):
    for c in range(COLS):
        paint = False
        if r < 12 and 2 <= c <= 30: paint = True
        elif r == 12 and 2 <= c <= 10: paint = True
        elif r == 17 and 5 <= c <= 12: paint = True
        if not paint: continue
        x0, y0, x1, y1 = cell_box(r, c)
        d.rectangle([x0+2, y0+3, x1-3, y1-4], fill=(220, 220, 220))

# A flat non-default background over rows 18-19 (a themed footer). The buffer is
# blank there and this must NOT read as a GHOST — that is the whole reason ink is
# within-cell contrast rather than difference-from-background.
x0, _, _, _ = cell_box(18, 0)
_, y0, _, _ = cell_box(18, 0)
_, _, x1, y1 = cell_box(19, COLS-1)
d.rectangle([x0, y0, x1, y1], fill=(70, 40, 110))

img.save(png)

sidecar = {
    "schema": "yggterm.paint_frame/1", "png": str(png), "png_space": "window",
    "capture_backend": "xterm_canvas_composite_over_dom", "captured_at_ms": 0,
    "dpr": DPR, "win_w": WIN_W, "win_h": WIN_H,
    "frame_css": {"left": 0, "top": 0, "width": WIN_W, "height": WIN_H},
    "buffer": {
        "session_path": "synthetic://selftest", "rows": ROWS, "cols": COLS,
        "base_y": 0, "viewport_y": 0, "line_count": ROWS,
        "nonblank_line_count": 15, "lines": lines, "ink_masks": masks,
        "cursor_x": 31, "cursor_y": 14, "cursor_char": "",
        "cursor_style": "block",
        "screen_css": {"left": LEFT, "top": TOP, "width": CW*COLS, "height": CH*ROWS},
        "cell_css_width": CW, "cell_css_height": CH,
    },
    "buffer_error": "",
}
Path(str(png) + ".paint-frame.json").write_text(json.dumps(sidecar, indent=2))

r = subprocess.run([sys.executable, "scripts/paint-diff.py", str(png), "--json"],
                   capture_output=True, text=True)
print(r.stderr, file=sys.stderr)
got = json.loads(r.stdout)
by_row = {v["row"]: v["verdict"] for v in got["verdicts"]}
expected = {12: "PARTIAL", 13: "MISSING", 14: "MISSING", 17: "GHOST"}
bad = []
for row, verdict in by_row.items():
    want = expected.get(row, "ok" if row < 15 else "blank")
    if verdict != want:
        bad.append(f"row {row}: got {verdict}, want {want}")
print("verdicts:", {k: v for k, v in by_row.items() if v not in ("ok", "blank")})
if bad:
    print("FAIL"); [print("  " + b) for b in bad]; sys.exit(1)
print("PASS — geometry maps cells to pixels, and all three planted faults were found,")
print("       with the flat-background footer rows correctly NOT read as ghosts.")
