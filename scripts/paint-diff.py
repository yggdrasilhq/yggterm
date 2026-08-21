#!/usr/bin/env python3
"""Diff what the terminal HELD against what the screen SHOWED, in one frame.

⛔⛔ WHY THIS EXISTS, AND WHY IT IS THE FIRST DELIVERABLE OF THE PAINT BUG.

A faithful screenshot costs ~6.8 s. A buffer read costs ~116 ms. On a live agent
row the two can never be sequenced into one frame — so every "the buffer says X
but the screen shows Y" reading ever taken on a busy row compared two moments
seven seconds apart, and some unknown part of the disagreement was TIME rather
than a paint fault. One such pair returned `Nesting... 1m42s` from the buffer
beside `Whirring... 29s` from the pixels: two different turns of the same agent,
presented as a contradiction.

The composite now reads the buffer in the SAME synchronous JS turn as
`toDataURL`, and writes it beside the PNG as `<png>.paint-frame.json`. xterm
parses PTY bytes on its own task queue, which cannot interleave with synchronous
script, so the text in that sidecar is EXACTLY the text those pixels were drawn
from. This script lays the two against each other. The comparison stops being an
argument and becomes a diff.

    server app screenshot /tmp/f.png          # writes f.png + f.png.paint-frame.json
    scripts/paint-diff.py /tmp/f.png

⭐ THE UNIT IS A CELL, NOT A LINE. The buffer side is an ink mask built from the
cells themselves ('#' = the cell holds a printable glyph), never from
`translateToString` — that trims trailing runs and gives a wide glyph one char
across two cells, so a column index taken from the string does not address the
cell the renderer drew. The pixel side asks each cell's own box whether anything
was drawn in it. Both sides are therefore indexed by (row, col) and the answer is
a per-cell disagreement, which is what "each CLI breaks in its own REGION" needs.

⛔ INK IS WITHIN-CELL CONTRAST, NOT DARKNESS. A cell painted a solid non-default
background (selection, a highlighted status bar, a themed footer) is not blank to
a "differs from the terminal background" test, and counting it as ink would
report a GHOST on every styled-but-empty cell — which is most of a TUI chrome
line. A cell holding a glyph has internal contrast; a cell holding a flat colour
has none, whatever that colour is. So ink = (max - min) luminance inside the
cell box, and the test is blind to what colour the cell was painted.

VERDICTS, per row

    ok        the two masks agree within tolerance
    MISSING   the buffer holds glyphs and the pixels hold none      <- unpainted
    PARTIAL   the pixels hold far fewer glyphs than the buffer does <- half-painted
    GHOST     the pixels hold glyphs the buffer does not            <- stale/mixed
    (blank)   both sides empty

⚠ WHAT THIS INSTRUMENT CANNOT SEE, stated so nobody builds a claim on it:
  * It does not read the glyphs. A cell painted with the WRONG character is `ok`
    here. It answers "was something drawn in this cell", not "was it right".
  * A native web surface draws above all DOM and is absent from the frame, so a
    row under one reads MISSING for a reason that is not a paint fault. The
    capture reports this in `capture_faithful`; check it.
  * `capture_faithful: false` means the frame is canvas-blind. Every row will
    read MISSING and none of it means anything.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

try:
    import numpy as np
    from PIL import Image
except ImportError as exc:  # pragma: no cover - environment guard
    print(f"paint-diff needs numpy and pillow: {exc}", file=sys.stderr)
    raise SystemExit(2)

# A cell whose luminance range is below this was painted flat: no glyph, whatever
# colour it was painted. Deliberately low — dim placeholder text in a composer
# ("press up to edit queued messages") is real content and must not read blank.
INK_CONTRAST = 18
# ...but one stray antialiased pixel bleeding in from the neighbouring cell is not
# a glyph, so a cell must carry a few contrasting pixels to count.
INK_MIN_PIXELS = 2
# Below this share of the buffer's glyphs a row is PARTIAL rather than ok. A row
# genuinely differs by a cell or two through antialiasing and cursor overlap.
PARTIAL_SHARE = 0.6
# A row this short is too small for a share to mean anything; use an absolute gap.
SHORT_ROW_CELLS = 5


def load_pair(png_path: Path, sidecar_path: Path | None):
    sidecar = sidecar_path or Path(str(png_path) + ".paint-frame.json")
    if not sidecar.exists():
        raise SystemExit(
            f"no paint-frame sidecar at {sidecar}\n"
            "  The capture writes it only on the faithful path — check\n"
            "  `capture_backend` on the screenshot response. A plain DOM\n"
            "  snapshot has no xterm buffer behind it and nothing to diff."
        )
    record = json.loads(sidecar.read_text())
    image = Image.open(png_path).convert("RGB")
    return record, image, sidecar


def cell_edges(start: float, size: float, count: int, scale: float, limit: int):
    """Integer pixel edges for `count` cells, so cells tile with no gap or overlap.

    Computed from the fractional cell size and rounded ONCE per boundary rather
    than per cell: rounding each cell independently accumulates and the last rows
    of a 65-row grid land a cell high, which reads as a whole-row disagreement
    that is really an off-by-one in this function.
    """
    edges = []
    for i in range(count + 1):
        px = (start + size * i) * scale
        edges.append(min(max(int(round(px)), 0), limit))
    return edges


def ink_grid(image: Image.Image, rows: int, cols: int, x_edges, y_edges):
    """Per-cell (contrast, contrasting-pixel-count) for the whole grid."""
    rgb = np.asarray(image, dtype=np.float32)
    lum = 0.2126 * rgb[:, :, 0] + 0.7152 * rgb[:, :, 1] + 0.0722 * rgb[:, :, 2]
    contrast = np.zeros((rows, cols), dtype=np.float32)
    pixels = np.zeros((rows, cols), dtype=np.int32)
    for r in range(rows):
        y0, y1 = y_edges[r], y_edges[r + 1]
        if y1 <= y0:
            continue
        band = lum[y0:y1, :]
        for c in range(cols):
            x0, x1 = x_edges[c], x_edges[c + 1]
            if x1 <= x0:
                continue
            box = band[:, x0:x1]
            if box.size == 0:
                continue
            lo = float(box.min())
            hi = float(box.max())
            contrast[r, c] = hi - lo
            if hi - lo >= INK_CONTRAST:
                pixels[r, c] = int(np.count_nonzero(box - lo >= INK_CONTRAST))
    return contrast, pixels


def spans(mask: str, char: str) -> str:
    """Compact column ranges where `mask` holds `char` — the REGION, readably."""
    out, start = [], None
    for i, ch in enumerate(mask):
        if ch == char and start is None:
            start = i
        elif ch != char and start is not None:
            out.append((start, i - 1))
            start = None
    if start is not None:
        out.append((start, len(mask) - 1))
    return ",".join(f"{a}-{b}" if b > a else str(a) for a, b in out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("png", type=Path)
    ap.add_argument("--sidecar", type=Path, default=None)
    ap.add_argument("--all-rows", action="store_true", help="print every row, not just the disagreements")
    ap.add_argument("--json", action="store_true", help="emit the verdict table as JSON")
    args = ap.parse_args()

    record, image, sidecar = load_pair(args.png, args.sidecar)
    buf = record.get("buffer") or {}
    masks = buf.get("ink_masks") or []
    lines = buf.get("lines") or []
    rows = int(buf.get("rows") or 0)
    cols = int(buf.get("cols") or 0)
    screen = buf.get("screen_css") or {}
    cell_w = float(buf.get("cell_css_width") or 0)
    cell_h = float(buf.get("cell_css_height") or 0)
    if not (rows and cols and masks and screen and cell_w > 0 and cell_h > 0):
        raise SystemExit(f"sidecar {sidecar} has no usable grid geometry")

    # CSS -> PNG. The two capture backends write different rectangles, and a band
    # computed against the wrong one lands on the wrong row with every verdict
    # after it confidently wrong. `png_space` is recorded for exactly this.
    png_w, png_h = image.size
    space = record.get("png_space")
    if space == "window":
        scale_x = png_w / max(float(record.get("win_w") or 1), 1.0)
        scale_y = png_h / max(float(record.get("win_h") or 1), 1.0)
        origin_x, origin_y = 0.0, 0.0
    elif space == "frame":
        frame = record.get("frame_css") or {}
        scale_x = png_w / max(float(frame.get("width") or 1), 1.0)
        scale_y = png_h / max(float(frame.get("height") or 1), 1.0)
        origin_x = float(frame.get("left") or 0.0)
        origin_y = float(frame.get("top") or 0.0)
    else:
        raise SystemExit(f"unknown png_space {space!r} in {sidecar}")

    x_edges = cell_edges(float(screen["left"]) - origin_x, cell_w, cols, scale_x, png_w)
    y_edges = cell_edges(float(screen["top"]) - origin_y, cell_h, rows, scale_y, png_h)
    contrast, pixels = ink_grid(image, min(rows, len(masks)), cols, x_edges, y_edges)

    # ⛔ `x or -1` reads -1 when x is 0, and column 0 is where a shell prompt's
    # cursor actually sits — so the falsy default silently disabled the cursor
    # handling in the commonest case. Ask whether the key is present, never
    # whether its value is truthy.
    def _coord(key):
        value = buf.get(key)
        return int(value) if isinstance(value, (int, float)) else -1

    cursor_x = _coord("cursor_x")
    cursor_y = _coord("cursor_y")

    verdicts = []
    for r in range(min(rows, len(masks))):
        bmask = masks[r]
        pmask = "".join(
            "#" if pixels[r, c] >= INK_MIN_PIXELS else "." for c in range(cols)
        )
        # The cursor cell is excluded from BOTH sides. A block cursor paints a
        # filled box, which is ink the buffer does not hold, so it manufactures a
        # GHOST on whatever row the cursor happens to sit — a moving false
        # positive that would land somewhere different in every frame. It is
        # reported separately below, where it is a finding rather than noise.
        if r == cursor_y and 0 <= cursor_x < cols:
            bmask = bmask[:cursor_x] + "." + bmask[cursor_x + 1 :]
            pmask = pmask[:cursor_x] + "." + pmask[cursor_x + 1 :]
        bn = bmask.count("#")
        pn = pmask.count("#")
        missing = "".join("#" if b == "#" and p == "." else "." for b, p in zip(bmask, pmask))
        ghost = "".join("#" if p == "#" and b == "." else "." for b, p in zip(bmask, pmask))

        if bn == 0 and pn == 0:
            verdict = "blank"
        elif bn > 0 and pn == 0:
            verdict = "MISSING"
        elif bn == 0 and ghost.count("#") > INK_MIN_PIXELS:
            verdict = "GHOST"
        elif bn >= SHORT_ROW_CELLS and pn < bn * PARTIAL_SHARE:
            verdict = "PARTIAL"
        elif bn < SHORT_ROW_CELLS and missing.count("#") > 2:
            verdict = "PARTIAL"
        else:
            verdict = "ok"
        verdicts.append(
            {
                "row": r,
                "verdict": verdict,
                "buffer_cells": bn,
                "painted_cells": pn,
                "missing_cols": spans(missing, "#"),
                "ghost_cols": spans(ghost, "#"),
                "text": (lines[r] if r < len(lines) else "")[:96],
            }
        )

    bad = [v for v in verdicts if v["verdict"] not in ("ok", "blank")]

    if args.json:
        print(json.dumps({"record": {k: record[k] for k in ("png", "png_space", "capture_backend", "captured_at_ms") if k in record},
                          "session_path": buf.get("session_path"),
                          "rows": rows, "cols": cols,
                          "cursor": {"x": cursor_x, "y": cursor_y, "char": buf.get("cursor_char"),
                                     "style": buf.get("cursor_style")},
                          "verdicts": verdicts, "disagreements": len(bad)}, indent=2))
        return 0

    print(f"png          {record.get('png')}  ({png_w}x{png_h}, space={space})")
    print(f"backend      {record.get('capture_backend')}")
    print(f"session      {buf.get('session_path')}")
    print(f"grid         {rows}x{cols}  cell {cell_w:.2f}x{cell_h:.2f} css  base_y={buf.get('base_y')} viewport_y={buf.get('viewport_y')}")
    renderer = record.get("renderer") or {}
    if renderer:
        hosts = renderer.get("hosts") or []
        shared = renderer.get("atlas_shared")
        print(f"renderer     {renderer.get('host_count')} hosts, "
              f"{renderer.get('distinct_atlases')} distinct atlas"
              f"{'es' if (renderer.get('distinct_atlases') or 0) != 1 else ''}"
              f"{'  <- SHARED across hosts' if shared else ''}")
        for h in hosts:
            mark = "*" if h.get("is_active") else " "
            suppressed = (h.get("recent_frame_like_write_until_ms") or 0) > (renderer.get("now_ms") or 0)
            print(f"  {mark} atlas#{h.get('atlas_index')} pages={h.get('atlas_pages')} "
                  f"forced_refresh={h.get('forced_refresh_count')} "
                  f"atlas_clears={h.get('forced_atlas_clear_count')} "
                  f"repair={h.get('retained_write_paint_repair_count')} "
                  f"{'REFRESH-SUPPRESSED' if suppressed else ''}")
    cur_char = buf.get("cursor_char") or ""
    print(f"cursor       row {cursor_y} col {cursor_x}  style={buf.get('cursor_style')!r}  cell holds {cur_char!r}")
    if cur_char.strip() and cursor_y >= 0 and 0 <= cursor_x < cols:
        drawn = pixels[cursor_y, cursor_x] >= INK_MIN_PIXELS
        print(f"             -> the buffer has a glyph under the cursor; the cell "
              f"{'was' if drawn else 'was NOT'} painted with contrast "
              f"({contrast[cursor_y, cursor_x]:.0f}). A block cursor that ERASES "
              f"the glyph instead of inverting it shows up here.")
    print()
    header = f"{'row':>3}  {'verdict':<8} {'buf':>4} {'px':>4}  {'missing cols':<24} {'ghost cols':<20} text"
    print(header)
    print("-" * len(header))
    shown = verdicts if args.all_rows else (bad or verdicts[:0])
    for v in shown:
        print(f"{v['row']:>3}  {v['verdict']:<8} {v['buffer_cells']:>4} {v['painted_cells']:>4}  "
              f"{v['missing_cols'][:24]:<24} {v['ghost_cols'][:20]:<20} {v['text']}")
    print()
    if not bad:
        print("no disagreement: every row the buffer holds glyphs on was painted, and")
        print("no row was painted glyphs the buffer does not hold.")
    else:
        kinds = {}
        for v in bad:
            kinds[v["verdict"]] = kinds.get(v["verdict"], 0) + 1
        summary = ", ".join(f"{n} {k}" for k, n in sorted(kinds.items()))
        rows_hit = [v["row"] for v in bad]
        print(f"{len(bad)} of {len(verdicts)} rows disagree ({summary}); rows {min(rows_hit)}-{max(rows_hit)}")
        third = max(1, len(verdicts) // 3)
        region = {"top": 0, "middle": 0, "bottom": 0}
        for r in rows_hit:
            region["top" if r < third else ("middle" if r < 2 * third else "bottom")] += 1
        print(f"region: top {region['top']}, middle {region['middle']}, bottom {region['bottom']}")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
