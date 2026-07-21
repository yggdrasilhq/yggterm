# yggui click grid — agent pointer targeting overlay

Generic labeled-grid overlay for agent-driven clicking/hovering on any yggui
surface. Solves the two failure modes live-traced 2026-07-08: pixel-coordinate
guessing from screenshots (blind aiming) and per-surface event-routing
differences (a window-level synthetic click never reaches a native child
webview).

## Model

The agent works a vision loop:

1. `grid show` — draw the overlay, get the cell map back as JSON.
2. screenshot — read the labels next to the thing to click.
3. `grid click B7` — the GUI resolves the cell to coordinates ITSELF (the
   JSON/screenshot is for choosing; resolution is server-side, immune to
   scale factors and stale rects) and dispatches the pointer sequence.
4. Optional `--refine`: instead of clicking, subdivide B7 into a 3×3 with
   labels 1–9, then `grid click B7.5`. Two hops ≈ 20 px precision with
   always-legible labels; fine pitch is never needed.

## Two grids, one geometry

There are two ways to get a grid, and they differ in **who can see it**:

| | live DOM grid | capture-side grid |
|---|---|---|
| verb | `server app grid show` | `server app screenshot --grid` |
| drawn into | the running page | the returned PNG only |
| the user sees it | **yes** (until TTL/hide) | **never** |
| cell resolution | server-side, `grid click B7` | manifest coords + a click verb |

Both derive every rect from **one owner**, `yggterm_core::click_grid::GridGeometry`
— cell codes, tiling, and the 3x3 refine are defined once and shared, so the two
grids can never disagree about where `B7` is.

Use the capture-side grid by default: it is the agent-presence rung of the
control plane (`docs/agent-control-plane.md` slice 3) and it is co-presence-safe
— an agent can aim while the user keeps working, and a human screenshot taken at
the same instant is grid-free.

### `screenshot --grid`

```
server app screenshot <out.png> [--grid | --grid COLSxROWS] [--grid-refine CELL]
                                [--region …] [--crop x,y,w,h] [--scale N]
```

- `--grid` alone = 12x8, the same default as the DOM grid. `--grid 16x10` sets it.
- `--grid-refine C4` draws C4's nine sub-cells (`C4.1`..`C4.9`) and dims the rest.
- Composes with `--crop`/`--region`/`--scale`: the grid spans the **cropped**
  area, and labels scale with `--scale` so they stay proportional.
- The manifest lands in `data.post_process.grid`:

```jsonc
{
  "cols": 12, "rows": 8, "refine": null,
  "region": { "x": 0, "y": 0, "w": 1920, "h": 1160, "cx": 960, "cy": 580 },
  "click_space": "capture",
  "capture_size": [1920, 1160],
  "cells": [ { "code": "A1",
               "capture": { "x": 0, "y": 0, "w": 160, "h": 145, "cx": 80, "cy": 72.5 },
               "image":   { "x": 0, "y": 0, "w": 160, "h": 145, "cx": 80, "cy": 72.5 } } ]
}
```

**Two coordinate spaces, both reported** — `capture` is the frame before
crop/scale (the space `--crop` and `rows_rect` use); `image` is the PNG actually
written. Click with `capture`; read labels off `image`. `capture_size` is there
so you can check it against `window.inner_size` in `app state`: when they match,
capture pixels *are* CSS pixels and `capture.cx/cy` go straight to a click verb.
It is reported rather than assumed so a HiDPI host cannot silently mis-aim.

## Targets — where the grid draws and where the click lands

- `main` — the main (Dioxus) webview: sidebar, terminal viewport, pickers,
  panes. Click dispatch = the existing app-control pointer path.
- `surface` — INSIDE the active session's native child webview (ychrome
  page, canvas/3D app). Overlay + dispatch are injected into the page via
  the web-surface eval channel, in page coordinates — a window-level click
  can never reach a native child widget, so the grid goes to the events'
  own coordinate space instead.

Default target is what the user sees: `surface` when the active session has
a live web surface, else `main`. Override with `--target`.

## Verbs

```
server app grid show  [--cols N] [--rows M] [--region full|terminal]
                      [--target main|surface] [--ttl-secs S]
server app grid click <CELL> [--button primary|middle|secondary] [--count n]
                      [--refine] [--keep]
server app grid hover <CELL> [--keep]
server app grid hide
```

- Defaults: 12 cols × 8 rows, region `full`, ttl 120 s (auto-hide).
- CELL = row letter + column number (`B7`), refined sub-cell `B7.5` (1–9,
  row-major).
- `show` responds with `{target, region, cols, rows, cells: {"A1": {x,y,w,h,cx,cy}, …}}`.
- `click`/`hover` respond with the resolved point and the hit element
  (`tag`, `id`, first attrs) so the agent can verify what it aimed at.
- A click hides the grid (it has served its purpose) unless `--keep`.

## Trust boundary (slice 2)

All dispatch is synthetic (untrusted) DOM events: listeners fire, but WebKit
withholds native default actions — notably FOCUS on inputs. For focus, pair
with `app dom-eval`/`app web eval` (`el.focus()`), or wait for slice 2:
OS-level input injection (KWin fake input / uinput) behind the same
`grid click` verb, which makes agent clicks fully human-equivalent.

## Overlay rendering contract

- `pointer-events: none`, max z-index, excluded from its own hit-testing.
- Labels: 13 px bold pills, white on translucent black — legible in every
  capture backend (DOM snapshot, canvas composite, `--backend os`;
  surface-target grids appear in `--backend os` and `app web screenshot`).
- Refine mode dims everything outside the chosen cell.
- TTL auto-hide so a forgotten grid never pollutes the user's view or a
  later screenshot.
