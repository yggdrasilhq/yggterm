#!/usr/bin/env python3
"""Decode tear-probe frames and emit a NUMBER for screen tearing.

The other half of this instrument is `tools/tear-probe/page.html`, which paints
every content band with a colour that encodes the band index plus a checksum:

    band i  ->  r = (i >> 8) & 0xFF,  g = i & 0xFF,  b = 0xA5 ^ ((i * 7) & 0xFF)

So a captured pixel decodes to a CONTENT-SPACE row, and a pixel that was blended
(subpixel scroll edge, scaling, a compositor cross-fade, chrome) fails the
checksum and is discarded instead of mis-decoded. The analyzer can refuse; that
is deliberate, per docs/agent-field-guide.md §7.3.

For one captured frame, define for each decoded screen row y

    D(y) = BAND * i(y) - y

If the whole frame came from ONE scroll/animation position S, then
D(y) = S - ((y + S) mod BAND), i.e. D is confined to a window of width BAND.
A frame containing content from TWO positions -- a tear -- has two plateaus of
D, and the step between them IS the tear magnitude in pixels.

Metrics per frame:
  seams       adjacent coded-row pairs whose |dD| >= BAND (a horizontal seam)
  torn_rows   coded rows whose |D - median(D)| >= BAND
  max_dev_px  the largest such deviation: how far apart the two contents were
  split_rows  rows where the decoded index is NOT uniform ACROSS the row
              (a vertical seam: left half old content, right half new)
  blank_rows  rows inside the page extent that decode to nothing, of which
  unpainted_rows  are NOT a band-boundary blend. A subpixel scroll offset blends
              exactly one row per band and the checksum refuses it -- expected
              and harmless. An unpainted row is the real artefact: a white or
              checkerboarded flash where the page had nothing to show.

A frame is counted TORN when seams >= 1 and torn_rows >= 3. The 3-row floor
rejects single-row decode noise; the seam requirement rejects a frame that is
merely noisy without a contiguous discontinuity.

Usage:
    analyze.py <frame.png|dir> [--band N] [--json] [--per-frame]
"""

from __future__ import annotations

import argparse
import json
import os
import sys

import numpy as np
from PIL import Image

CHECK_XOR = 0xA5
CHECK_MUL = 7


def decode(img: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Return (index, valid) arrays for an HxWx3 uint8 image."""
    r = img[:, :, 0].astype(np.int32)
    g = img[:, :, 1].astype(np.int32)
    b = img[:, :, 2].astype(np.int32)
    idx = (r << 8) | g
    expect = np.bitwise_xor(CHECK_XOR, (idx * CHECK_MUL) & 0xFF)
    valid = b == expect
    return idx, valid


def row_modes(idx: np.ndarray, valid: np.ndarray, min_pixels: int):
    """Per row: (modal index, modal count, valid count). -1 where unusable."""
    height, width = idx.shape
    mode_idx = np.full(height, -1, dtype=np.int64)
    mode_cnt = np.zeros(height, dtype=np.int64)
    valid_cnt = valid.sum(axis=1)
    for y in range(height):
        if valid_cnt[y] < min_pixels:
            continue
        row = idx[y][valid[y]]
        # Rows are solid bands, so bincount over the row's own range is cheap.
        vals, counts = np.unique(row, return_counts=True)
        k = int(np.argmax(counts))
        mode_idx[y] = int(vals[k])
        mode_cnt[y] = int(counts[k])
    return mode_idx, mode_cnt, valid_cnt


def longest_run(mask: np.ndarray) -> tuple[int, int]:
    """Longest contiguous True run as (start, end_exclusive); (0,0) if none."""
    best = (0, 0)
    start = None
    for i, v in enumerate(mask):
        if v and start is None:
            start = i
        elif not v and start is not None:
            if i - start > best[1] - best[0]:
                best = (start, i)
            start = None
    if start is not None and len(mask) - start > best[1] - best[0]:
        best = (start, len(mask))
    return best


def page_extent(coded: np.ndarray, max_gap: int) -> tuple[int, int]:
    """[first, last+1] of the largest cluster of coded rows, gaps <= max_gap."""
    runs = []
    start = None
    for i, v in enumerate(coded):
        if v and start is None:
            start = i
        elif not v and start is not None:
            runs.append((start, i))
            start = None
    if start is not None:
        runs.append((start, len(coded)))
    if not runs:
        return 0, 0
    clusters = [[runs[0]]]
    for run in runs[1:]:
        if run[0] - clusters[-1][-1][1] <= max_gap:
            clusters[-1].append(run)
        else:
            clusters.append([run])
    best = max(clusters, key=lambda c: sum(e - s for s, e in c))
    return best[0][0], best[-1][1]


def analyze_frame(path: str, band: int) -> dict:
    img = np.asarray(Image.open(path).convert("RGB"))
    height, width = img.shape[0], img.shape[1]
    idx, valid = decode(img)

    # A random (non-probe) pixel passes the checksum with p = 1/256, so a
    # 1920-wide chrome row yields ~7 scattered "valid" pixels with unrelated
    # indices. Require both an absolute floor and a dominant mode.
    min_pixels = max(24, width // 20)
    mode_idx, mode_cnt, valid_cnt = row_modes(idx, valid, min_pixels)
    # 0.35, not 0.6: a row that is HALF one content and half another (a vertical
    # seam) must still be decoded and counted, not silently dropped for being
    # non-uniform. The absolute `min_pixels` floor is what keeps chrome out --
    # a 1920-wide chrome row yields ~7 checksum-passing pixels, far below it.
    coded = (
        (mode_idx >= 0)
        & (mode_cnt >= 0.35 * np.maximum(valid_cnt, 1))
        & (mode_cnt >= min_pixels)
    )

    # The page extent must tolerate GAPS: unpainted rows mid-scroll are exactly
    # the artefact we want to count, and a longest-contiguous-run extent would
    # instead shrink around them and report zero blank rows -- hiding the very
    # thing it was asked to measure. Nothing outside the page ever paints a
    # checksum-passing band, so merging across a wide gap cannot pull in chrome.
    y0, y1 = page_extent(coded, max_gap=600)
    page_rows = int(coded[y0:y1].sum())
    out = {
        "frame": os.path.basename(path),
        "width": int(width),
        "height": int(height),
        "page_y0": int(y0),
        "page_y1": int(y1),
        "page_x0": 0,
        "page_x1": 0,
        "page_rows": page_rows,
        "seams": 0,
        "torn_rows": 0,
        "max_dev_px": 0,
        "split_rows": 0,
        "blank_rows": 0,
        "unpainted_rows": 0,
        "d_median": None,
        "torn": False,
        "usable": False,
    }
    if page_rows < 64:
        return out
    out["usable"] = True

    # Horizontal extent, so a driver can aim the seat pointer at the page.
    col_hits = valid[y0:y1].sum(axis=0)
    xs = np.where(col_hits > 0.4 * page_rows)[0]
    if xs.size:
        out["page_x0"], out["page_x1"] = int(xs.min()), int(xs.max()) + 1

    ys = np.arange(y0, y1)
    sub_coded = coded[y0:y1]
    d = band * mode_idx[y0:y1] - ys
    d_valid = d[sub_coded]
    median = int(np.median(d_valid))
    out["d_median"] = median

    dev = np.abs(d_valid - median)
    out["max_dev_px"] = int(dev.max())
    out["torn_rows"] = int((dev >= band).sum())

    dd = np.abs(np.diff(d_valid))
    out["seams"] = int((dd >= band).sum())

    # A vertical seam: the row itself is not one colour.
    split = sub_coded & (mode_cnt[y0:y1] < 0.95 * np.maximum(valid_cnt[y0:y1], 1))
    out["split_rows"] = int(split.sum())

    # A non-decoding row is one of two very different things and they must not
    # be conflated. Under a SUBPIXEL scroll offset the one row at each band
    # boundary is a blend of two adjacent band colours -- expected, harmless,
    # and correctly refused by the checksum. A row that is neither decodable nor
    # a boundary blend is UNPAINTED: a white/checkerboard flash mid-scroll,
    # which is a real user-visible artefact. Boundary blends sit between two
    # coded rows whose indices differ by exactly one.
    blank = 0
    unpainted = 0
    n = y1 - y0
    for k in range(n):
        if sub_coded[k]:
            continue
        blank += 1
        if (
            0 < k < n - 1
            and sub_coded[k - 1]
            and sub_coded[k + 1]
            and mode_idx[y0 + k + 1] - mode_idx[y0 + k - 1] == 1
        ):
            continue
        unpainted += 1
    out["blank_rows"] = blank
    out["unpainted_rows"] = unpainted
    out["torn"] = out["seams"] >= 1 and out["torn_rows"] >= 3
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("target")
    ap.add_argument("--band", type=int, default=4)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--per-frame", action="store_true")
    ap.add_argument("--label", default="")
    args = ap.parse_args()

    if os.path.isdir(args.target):
        frames = sorted(
            os.path.join(args.target, f)
            for f in os.listdir(args.target)
            if f.lower().endswith(".png")
        )
    else:
        frames = [args.target]
    if not frames:
        print("no frames", file=sys.stderr)
        return 2

    rows = [analyze_frame(f, args.band) for f in frames]
    usable = [r for r in rows if r["usable"]]
    torn = [r for r in usable if r["torn"]]
    summary = {
        "label": args.label,
        "band": args.band,
        "frames": len(rows),
        "usable_frames": len(usable),
        "torn_frames": len(torn),
        "tear_rate": round(len(torn) / len(usable), 4) if usable else None,
        "max_dev_px": max((r["max_dev_px"] for r in usable), default=0),
        "total_seams": sum(r["seams"] for r in usable),
        "split_rows_total": sum(r["split_rows"] for r in usable),
        "blank_rows_total": sum(r["blank_rows"] for r in usable),
        "unpainted_rows_total": sum(r["unpainted_rows"] for r in usable),
        "distinct_positions": len({r["d_median"] for r in usable}),
    }
    if args.json:
        payload = {"summary": summary}
        if args.per_frame:
            payload["frames"] = rows
        print(json.dumps(payload, indent=2))
    else:
        if args.per_frame:
            for r in rows:
                print(
                    f"{r['frame']:>28}  usable={int(r['usable'])} rows={r['page_rows']:>5} "
                    f"y={r['page_y0']}-{r['page_y1']} D={r['d_median']} seams={r['seams']} "
                    f"torn_rows={r['torn_rows']} max_dev={r['max_dev_px']} "
                    f"split={r['split_rows']} blend={r['blank_rows'] - r['unpainted_rows']} "
                    f"unpainted={r['unpainted_rows']} "
                    f"{'TORN' if r['torn'] else ''}"
                )
        label = f"[{args.label}] " if args.label else ""
        print(
            f"{label}frames={summary['frames']} usable={summary['usable_frames']} "
            f"torn={summary['torn_frames']} tear_rate={summary['tear_rate']} "
            f"max_dev_px={summary['max_dev_px']} seams={summary['total_seams']} "
            f"split_rows={summary['split_rows_total']} unpainted_rows={summary['unpainted_rows_total']} "
            f"distinct_positions={summary['distinct_positions']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
