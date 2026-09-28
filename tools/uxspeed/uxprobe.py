#!/usr/bin/env python3
"""uxprobe — the UX-SPEED campaign's synthetic timing + accuracy harness.

Times the campaign's UX actions verb→effect using the server app verb
family (the GUI's own sanctioned automation surface) and ASSERTS accuracy,
on self-created scratch rows only. Blast-radius law: rows not created by
this run are never mutated; teardown removes exactly what the run spawned.

Actions:
  spawn  — server app terminal new → the paint answer is SCREEN CONTENT
         (read-buffer nonblank poll — honest verb→content); the ytrace
         milestone ladder (open_attempt/mount, and first_frame | settle |
         mount_open when the build fires them) rides along as diagnostics
  drag   — server app drag begin/hover/drop reorder of two scratch rows
  felt   — the REAL input-plane drag: pointer down on a scratch row, dwell
           under the 6px threshold, cross it, hover across rows, release on
           the target (server app pointer + dom-eval rects); asserts the
           felt-path falsifiers from the trace (no merges pre-begin/in-drag),
           the reorder, and records app-render counts for [11.129]
  shiftdrag — the shift-drag vertical: a RANGE selection (tree select
           paths+anchor = the shift+click ExtendRange twin), then a REAL
           pointer drag of the whole set onto a target outside it; asserts
           tree_drag_begin.drag_paths carries exactly the set, the family
           completes, the set lands contiguous in order, and the [11.129]
           render attribution
  group  — server app row-set --into / --out on two scratch rows
  menu   — server app terminal probe-context-menu on a scratch row
  modal  — the delete-confirm dialog on a scratch row, opened through the
           REAL context-menu path (dispatched right-click → Delete item
           click via server app dom-eval), timed as the in-app
           modal_open_requested → modal/shown pair AND the DOM
           dispatch→mount wall; cancelled through the dialog's own
           Cancel button; the row must survive the cancel
  close  — server app session remove of the scratch rows (the teardown,
           instrumented — every spawned row is closed exactly once)
  closeall — the bulk-close vertical: the Live Sessions group row's REAL
           menu → Close All… → the bulk confirm dialog, timed as
           menu_open / click_to_mount / the in-app modal pair
           (modal_open_requested{bulk:true} → modal/shown), then CANCELLED
           with a survivors assert (cancel accuracy). The chord leg drives
           the REAL alt-tap bridge (synthetic KeyboardEvents on window) for
           tap→overlay→row-menu walls and asserts the ui/chord identity
           events ([11.113] gap 3 instrument). The CONFIRM leg runs ONLY
           when every live session on screen is a probe scratch row —
           close-all closes ALL live sessions, so on any real desktop the
           probe refuses it and says so (blast-radius law).

  switch — the felt switch: real pointer CLICKS on the pair's sidebar rows
           (press+release, no threshold cross), alternating A→B; asserts the
           user_gesture activation (latency + identity to == clicked row),
           joins the reveal outcome by time-proximity (reveal_ready's
           self-timed first_output_ms, or the honest incomplete/failed
           marker), and counts the ambient churn inside each window

Report: schema-keyed JSON, honest nulls for anything not measured.
A p50 without its iteration count is not a measurement.

Usage:
  python3 tools/uxspeed/uxprobe.py --actions spawn,drag,menu --iters 3 \
      --out /tmp/uxspeed-report.json [--artifacts DIR] [--keep] [-v]
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import statistics
import subprocess
import sys
import time

PROBE_TITLE_PREFIX = "uxspeed-probe-"
YGGTERM = "yggterm"
YTRACE = "ytrace"

# The modal action opens the delete-confirm dialog through the row's REAL
# context menu: a dispatched right-click on the row's SIDEBAR tree node
# (data-sidebar-row-path — always rendered, unlike the xterm surface which
# mounts lazily), then a click on the menu's Delete item. That is the DOM
# path a human's right-click takes, and the same menu the ALT+E,X chord
# drives. Everything happens in ONE dom-eval so the open→click→mount
# deltas are in-page walls, unpolluted by CLI round-trips. Every exit
# calls dioxus.send explicitly — the dom-eval bridge delivers the FIRST
# sent message; a bare `return` value reads back null. The in-app pair
# (modal_open_requested → modal/shown) is read from ytrace separately —
# the app's own account of the same open.
MODAL_OPEN_CLICK_JS = """
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const PATH = {session!r};
const refuse = (reason, extra) => dioxus.send(
    Object.assign({{ accepted: false, reason }}, extra || {{}}));
if (document.querySelector('[data-delete-confirm-overlay]')) {{
    refuse("delete_overlay_already_open");
    return;
}}
const dismiss = async () => {{
    try {{
        const t = document.elementFromPoint(3, 3) || document.body;
        const b = {{ bubbles: true, cancelable: true, composed: true,
                     view: window, clientX: 3, clientY: 3, screenX: 3,
                     screenY: 3, button: 0, buttons: 1 }};
        t.dispatchEvent(new MouseEvent('mousedown', b));
        t.dispatchEvent(new MouseEvent('mouseup', {{ ...b, buttons: 0 }}));
        t.dispatchEvent(new MouseEvent('click', {{ ...b, buttons: 0 }}));
        await settle(80);
    }} catch (_e) {{}}
}};
await dismiss();
const row = await (async () => {{
    // the sidebar virtualizes: the node may not exist until the row is
    // selected and scrolled into view (the driver tree-selects first)
    const deadline = Date.now() + 1500;
    while (Date.now() < deadline) {{
        const n = document.querySelector(
            '[data-sidebar-row-path="' + PATH + '"]');
        if (n) return n;
        await settle(50);
    }}
    return null;
}})();
if (!row) {{
    refuse("sidebar_row_missing", {{ session_path: PATH }});
    return;
}}
const rect = row.getBoundingClientRect();
if (!(rect.width > 0 && rect.height > 0)) {{
    refuse("sidebar_row_not_visible", {{ session_path: PATH }});
    return;
}}
const cx = Number((rect.left + rect.width / 2).toFixed(2));
const cy = Number((rect.top + rect.height / 2).toFixed(2));
const init = {{ bubbles: true, cancelable: true, composed: true, view: window,
                clientX: cx, clientY: cy, screenX: cx, screenY: cy,
                button: 2, buttons: 2, detail: 1 }};
const t_open = Date.now();
row.dispatchEvent(new MouseEvent('mousedown', init));
row.dispatchEvent(new MouseEvent('mouseup', {{ ...init, buttons: 0 }}));
row.dispatchEvent(new MouseEvent('auxclick', {{ ...init, buttons: 0 }}));
row.dispatchEvent(new MouseEvent('contextmenu', init));
let menu = null, deleteItem = null;
const openDeadline = Date.now() + 1500;
while (Date.now() < openDeadline) {{
    await settle(40);
    menu = document.querySelector('[data-context-menu="1"]');
    // a LIVE session row's item is delete-session; plain delete is the
    // folder/saved-ssh variant — same confirm dialog either way
    deleteItem = menu?.querySelector(
        '[data-context-menu-action="delete-session"],'
        + ' [data-context-menu-action="delete"]') || null;
    if (menu && deleteItem) break;
}}
if (!menu || !deleteItem) {{
    await dismiss();
    refuse("context_menu_or_delete_item_not_observed",
           {{ session_path: PATH }});
    return;
}}
const menu_open_ms = Date.now() - t_open;
await settle(120);
const clickInit = {{ bubbles: true, cancelable: true, composed: true,
                     view: window, button: 0, buttons: 1 }};
deleteItem.dispatchEvent(new MouseEvent('mousedown', clickInit));
deleteItem.dispatchEvent(new MouseEvent('mouseup',
    {{ ...clickInit, buttons: 0 }}));
deleteItem.dispatchEvent(new MouseEvent('click', clickInit));
const t_click = Date.now();
const mountDeadline = t_click + 1100;
let overlay = null;
while (Date.now() < mountDeadline) {{
    await settle(20);
    overlay = document.querySelector('[data-delete-confirm-overlay]');
    if (overlay) break;
}}
if (!overlay) {{
    refuse("delete_overlay_did_not_mount", {{ menu_open_ms }});
    return;
}}
const t_mounted = Date.now();
const dialog = overlay.querySelector('[data-delete-confirm-dialog]');
dioxus.send({{
    accepted: true,
    menu_open_ms,
    click_to_mount_ms: t_mounted - t_click,
    dispatch_to_mount_ms: t_mounted - t_open,
    overlay_count: document.querySelectorAll(
        '[data-delete-confirm-overlay]').length,
    dialog_text: String(dialog?.textContent || '').slice(0, 400),
}});
"""

MODAL_CANCEL_JS = """
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const t0 = Date.now();
const refuse = (reason) => dioxus.send(
    { accepted: false, reason });
const cancel = document.querySelector('[data-delete-confirm-cancel]');
if (!cancel) { refuse("cancel_button_missing"); return; }
const init = { bubbles: true, cancelable: true, composed: true, view: window,
               button: 0, buttons: 1 };
cancel.dispatchEvent(new MouseEvent('mousedown', init));
cancel.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }));
cancel.dispatchEvent(new MouseEvent('click', init));
const deadline = Date.now() + 1500;
while (Date.now() < deadline) {
    await settle(30);
    if (!document.querySelector('[data-delete-confirm-overlay]')) {
        dioxus.send({ accepted: true,
                      cancel_to_gone_ms: Date.now() - t0 });
        return;
    }
}
refuse("delete_overlay_did_not_close");
"""


# The bulk-close vertical (chord-close-all lane): the Live Sessions group
# row (__live_sessions__) offers exactly one menu action — Close All… — which
# opens the bulk delete-confirm dialog. One dom-eval does right-click → menu
# → item click → overlay mount so the deltas are in-page walls. CANCELLED by
# the caller through MODAL_CANCEL_JS; the confirm leg has its own script.
CLOSEALL_OPEN_JS = """
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const refuse = (reason, extra) => dioxus.send(
    Object.assign({ accepted: false, reason }, extra || {}));
if (document.querySelector('[data-delete-confirm-overlay]')) {
    refuse("delete_overlay_already_open");
    return;
}
const PATH = '__live_sessions__';
const row = await (async () => {
    const deadline = Date.now() + 1500;
    while (Date.now() < deadline) {
        const n = document.querySelector(
            '[data-sidebar-row-path="' + PATH + '"]');
        if (n) return n;
        await settle(50);
    }
    return null;
})();
if (!row) {
    refuse("live_sessions_group_row_missing");
    return;
}
const rect = row.getBoundingClientRect();
if (!(rect.width > 0 && rect.height > 0)) {
    refuse("live_sessions_group_row_not_visible");
    return;
}
const cx = Number((rect.left + rect.width / 2).toFixed(2));
const cy = Number((rect.top + rect.height / 2).toFixed(2));
const init = { bubbles: true, cancelable: true, composed: true, view: window,
                clientX: cx, clientY: cy, screenX: cx, screenY: cy,
                button: 2, buttons: 2, detail: 1 };
const t_open = Date.now();
row.dispatchEvent(new MouseEvent('mousedown', init));
row.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }));
row.dispatchEvent(new MouseEvent('auxclick', { ...init, buttons: 0 }));
row.dispatchEvent(new MouseEvent('contextmenu', init));
let menu = null, closeAll = null;
const openDeadline = Date.now() + 1500;
while (Date.now() < openDeadline) {
    await settle(40);
    menu = document.querySelector('[data-context-menu="1"]');
    closeAll = menu?.querySelector(
        '[data-context-menu-action="close-all-live-sessions"]') || null;
    if (menu && closeAll) break;
}
if (!menu || !closeAll) {
    refuse("menu_or_close_all_item_not_observed", { session_path: PATH });
    return;
}
const menu_open_ms = Date.now() - t_open;
await settle(120);
const clickInit = { bubbles: true, cancelable: true, composed: true,
                     view: window, button: 0, buttons: 1 };
closeAll.dispatchEvent(new MouseEvent('mousedown', clickInit));
closeAll.dispatchEvent(new MouseEvent('mouseup',
    { ...clickInit, buttons: 0 }));
closeAll.dispatchEvent(new MouseEvent('click', clickInit));
const t_click = Date.now();
const mountDeadline = t_click + 1500;
let overlay = null;
while (Date.now() < mountDeadline) {
    await settle(20);
    overlay = document.querySelector('[data-delete-confirm-overlay]');
    if (overlay) break;
}
if (!overlay) {
    refuse("delete_overlay_did_not_mount", { menu_open_ms });
    return;
}
const t_mounted = Date.now();
const dialog = overlay.querySelector('[data-delete-confirm-dialog]');
dioxus.send({
    accepted: true,
    menu_open_ms,
    click_to_mount_ms: t_mounted - t_click,
    dispatch_to_mount_ms: t_mounted - t_open,
    overlay_count: document.querySelectorAll(
        '[data-delete-confirm-overlay]').length,
    dialog_title: String(overlay.querySelector(
        '[data-delete-confirm-title]')?.textContent || ''),
    action_label: String(overlay.querySelector(
        '[data-delete-confirm-action]')?.textContent || ''),
    unkept_button_present: !!overlay.querySelector(
        '[data-delete-confirm-unkept-action]'),
    dialog_text: String(dialog?.textContent || '').slice(0, 400),
});
"""

# The CONFIRM leg of close-all — gated by the caller (only when every live
# session on screen is a probe scratch row). Clicks the dialog's real
# confirm button and waits for the overlay to drop.
CLOSEALL_CONFIRM_JS = """
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const t0 = Date.now();
const refuse = (reason) => dioxus.send(
    { accepted: false, reason });
const confirm = document.querySelector('[data-delete-confirm-action]');
if (!confirm) { refuse("confirm_button_missing"); return; }
const init = { bubbles: true, cancelable: true, composed: true, view: window,
               button: 0, buttons: 1 };
confirm.dispatchEvent(new MouseEvent('mousedown', init));
confirm.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }));
confirm.dispatchEvent(new MouseEvent('click', init));
const deadline = Date.now() + 10000;
while (Date.now() < deadline) {
    await settle(50);
    if (!document.querySelector('[data-delete-confirm-overlay]')) {
        dioxus.send({ accepted: true,
                      confirm_to_gone_ms: Date.now() - t0 });
        return;
    }
}
refuse("delete_overlay_did_not_close");
"""

# The chord leg: drives the REAL alt-tap bridge listeners with synthetic
# KeyboardEvents on window (they are window-level capture listeners, exactly
# what a real key hits) and walks ALT-tap → E → Escape. Asserts the overlay
# and the row menu open through the chord path — the walls the chord
# instrument exists to measure — and leaves every container closed again.
# No menu item is ever fired here.
SPLIT_OPEN_JS = """
// PHASE 1 (menu open + split-item click) — must fit the GUI's 3s
// app-control eval budget ([11.200]: the monolithic script's 2.5s DOM tail
// pushed every send past the budget and dom_eval_timeout discarded the
// whole iteration, menu included). The DOM truth read lives in SPLIT_DOM_JS.
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const PATH_A = {session_a!r};
const AXIS = {axis!r};  // "split-side-by-side" | "split-stacked"
if (document.querySelector('[data-split-group-row="1"]')) {
    dioxus.send({ accepted: false, reason: "split_group_already_on_screen" });
    return;
}
const dismiss = async () => {
    try {
        const t = document.elementFromPoint(3, 3) || document.body;
        const b = { bubbles: true, cancelable: true, composed: true,
                     view: window, clientX: 3, clientY: 3, screenX: 3,
                     screenY: 3, button: 0, buttons: 1 };
        t.dispatchEvent(new MouseEvent('mousedown', b));
        t.dispatchEvent(new MouseEvent('mouseup', { ...b, buttons: 0 }));
        t.dispatchEvent(new MouseEvent('click', { ...b, buttons: 0 }));
        await settle(80);
    } catch (_e) {}
};
await dismiss();
// the driver tree-selects + verifies the rect before this eval, so a short
// node wait is enough — the 1500ms crawl is gone
let row = document.querySelector(
    '[data-sidebar-row-path="' + PATH_A + '"]');
const rowDeadline = Date.now() + 800;
while (!row && Date.now() < rowDeadline) {
    await settle(50);
    row = document.querySelector(
        '[data-sidebar-row-path="' + PATH_A + '"]');
}
if (!row) {
    dioxus.send({ accepted: false, reason: "sidebar_row_missing",
                  session_path: PATH_A });
    return;
}
const rect = row.getBoundingClientRect();
if (!(rect.width > 0 && rect.height > 0)) {
    dioxus.send({ accepted: false, reason: "sidebar_row_not_visible",
                  session_path: PATH_A });
    return;
}
const cx = Number((rect.left + rect.width / 2).toFixed(2));
const cy = Number((rect.top + rect.height / 2).toFixed(2));
const init = { bubbles: true, cancelable: true, composed: true, view: window,
                clientX: cx, clientY: cy, screenX: cx, screenY: cy,
                button: 2, buttons: 2, detail: 1 };
const t_open = Date.now();
row.dispatchEvent(new MouseEvent('mousedown', init));
row.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }));
row.dispatchEvent(new MouseEvent('auxclick', { ...init, buttons: 0 }));
row.dispatchEvent(new MouseEvent('contextmenu', init));
let menu = null, splitItem = null;
const openDeadline = Date.now() + 1400;
while (Date.now() < openDeadline) {
    await settle(40);
    menu = document.querySelector('[data-context-menu="1"]');
    splitItem = menu?.querySelector(
        '[data-context-menu-action="' + AXIS + '"]') || null;
    if (menu && splitItem) break;
}
if (!menu || !splitItem) {
    await dismiss();
    dioxus.send({ accepted: false,
                  reason: "context_menu_or_split_item_not_observed",
                  session_path: PATH_A, axis: AXIS });
    return;
}
const menu_open_ms = Date.now() - t_open;
const item_label = String(splitItem.textContent || '').slice(0, 80);
await settle(100);
const clickInit = { bubbles: true, cancelable: true, composed: true,
                     view: window, button: 0, buttons: 1 };
splitItem.dispatchEvent(new MouseEvent('mousedown', clickInit));
splitItem.dispatchEvent(new MouseEvent('mouseup',
    { ...clickInit, buttons: 0 }));
splitItem.dispatchEvent(new MouseEvent('click', clickInit));
dioxus.send({ accepted: true, menu_open_ms, item_label,
              t_open, t_click: Date.now() });
"""

SPLIT_DOM_JS = """
// PHASE 2 (split DOM truth: compound row + pane rects) — its own eval so
// the 2.5s wait can never eat phase 1's reply ([11.200]). The page clock is
// shared with phase 1, so found_at - t_click is honest click_to_dom.
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const t0 = Date.now();
const deadline = t0 + 2200;
let compound = null, panes = [], found_at = null;
while (Date.now() < deadline) {
    await settle(25);
    compound = document.querySelector('[data-split-group-row="1"]');
    panes = [...document.querySelectorAll('[data-split-session]')];
    if (compound && panes.length >= 2) { found_at = Date.now(); break; }
}
const paneRect = (n) => {
    const r = n.getBoundingClientRect();
    return { x: Math.round(r.left), y: Math.round(r.top),
              w: Math.round(r.width), h: Math.round(r.height) };
};
dioxus.send({
    compound_found: !!compound,
    found_at,
    waited_ms: Date.now() - t0,
    compound_label: compound ? String(compound.textContent || '').slice(0, 80) : null,
    pane_count: panes.length,
    panes: panes.map((n) => ({
        session: n.getAttribute('data-split-session'),
        pane_index: n.getAttribute('data-split-pane-index'),
        rect: paneRect(n),
    })),
});
"""

SPLIT_UNGROUP_JS = """
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const refuse = (reason, extra) => dioxus.send(
    Object.assign({ accepted: false, reason }, extra || {}));
const compound = document.querySelector('[data-split-group-row="1"]');
if (!compound) {
    refuse("compound_row_missing");
    return;
}
const init = { bubbles: true, cancelable: true, composed: true, view: window,
                clientX: 0, clientY: 0, button: 2, buttons: 2, detail: 1 };
const r = compound.getBoundingClientRect();
init.clientX = init.screenX = Math.round(r.left + r.width / 2);
init.clientY = init.screenY = Math.round(r.top + r.height / 2);
const t0 = Date.now();
compound.dispatchEvent(new MouseEvent('mousedown', init));
compound.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }));
compound.dispatchEvent(new MouseEvent('auxclick', { ...init, buttons: 0 }));
compound.dispatchEvent(new MouseEvent('contextmenu', init));
let menu = null, item = null;
const deadline = Date.now() + 1500;
while (Date.now() < deadline) {
    await settle(40);
    menu = document.querySelector('[data-context-menu="1"]');
    item = menu?.querySelector(
        '[data-context-menu-action="ungroup-split"]') || null;
    if (menu && item) break;
}
if (!menu || !item) {
    await dismiss();
    refuse("context_menu_or_ungroup_item_not_observed");
    return;
}
await settle(120);
const clickInit = { bubbles: true, cancelable: true, composed: true,
                     view: window, button: 0, buttons: 1 };
item.dispatchEvent(new MouseEvent('mousedown', clickInit));
item.dispatchEvent(new MouseEvent('mouseup',
    { ...clickInit, buttons: 0 }));
item.dispatchEvent(new MouseEvent('click', clickInit));
const tClick = Date.now();
const deadline2 = tClick + 1200;
while (Date.now() < deadline2) {
    await settle(25);
    if (!document.querySelector('[data-split-group-row="1"]') &&
        document.querySelectorAll('[data-split-session]').length === 0) break;
}
const tGone = Date.now();
const stillCompound = !!document.querySelector('[data-split-group-row="1"]');
dioxus.send({
    accepted: !stillCompound,
    ungroup_to_gone_ms: tGone - tClick,
    menu_open_ms: 0,
    still_compound: stillCompound,
    pane_count_after: document.querySelectorAll(
        '[data-split-session]').length,
});
"""
CHORD_LEG_JS = """
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const refuse = (reason, extra) => dioxus.send(
    Object.assign({ accepted: false, reason }, extra || {}));
const q = (sel) => !!document.querySelector(sel);
if (q('[data-yggterm-menu-open]') || q('[data-delete-confirm-overlay]')) {
    refuse("menu_or_overlay_already_open");
    return;
}
const kd = (key, code) => window.dispatchEvent(new KeyboardEvent('keydown',
    { key, code, bubbles: true, cancelable: true, composed: true }));
const ku = (key, code) => window.dispatchEvent(new KeyboardEvent('keyup',
    { key, code, bubbles: true, cancelable: true, composed: true }));
const t_tap = Date.now();
kd('Alt', 'AltLeft');
ku('Alt', 'AltLeft');
let overlaySeen = false;
const ovDeadline = Date.now() + 1500;
while (Date.now() < ovDeadline) {
    await settle(40);
    if (q('[data-yggterm-keytip-breadcrumb]')) { overlaySeen = true; break; }
}
if (!overlaySeen) {
    refuse("alt_overlay_did_not_open", { tap_to_overlay_ms: null });
    return;
}
const tap_to_overlay_ms = Date.now() - t_tap;
const t_e = Date.now();
kd('e', 'KeyE');
let menuSeen = false;
const mDeadline = Date.now() + 1500;
while (Date.now() < mDeadline) {
    await settle(40);
    if (q('[data-yggterm-menu-open]')) { menuSeen = true; break; }
}
const walk_to_menu_ms = menuSeen ? Date.now() - t_e : null;
kd('Escape', 'Escape');
let menuClosed = false;
const gDeadline = Date.now() + 1500;
while (Date.now() < gDeadline) {
    await settle(40);
    if (!q('[data-yggterm-menu-open]')) { menuClosed = true; break; }
}
let overlayClosed = !q('[data-yggterm-keytip-breadcrumb]');
if (!overlayClosed) {
    kd('Escape', 'Escape');
    const d2 = Date.now() + 1000;
    while (Date.now() < d2) {
        await settle(40);
        if (!q('[data-yggterm-keytip-breadcrumb]')) { overlayClosed = true; break; }
    }
}
dioxus.send({
    accepted: true,
    overlay_seen: true,
    menu_seen: menuSeen,
    tap_to_overlay_ms,
    walk_to_menu_ms,
    escape_closed_menu: menuClosed,
    overlay_closed: overlayClosed,
});
"""


def now_ms() -> int:
    return int(time.time() * 1000)


class Probe:
    def __init__(self, args):
        self.args = args
        self.timeout_s = args.timeout_ms / 1000.0
        self.artifacts = args.artifacts
        if self.artifacts:
            os.makedirs(self.artifacts, exist_ok=True)
        self.spawned_paths: list[str] = []
        self.scratch_titles: list[dict] = []
        self.trace_events_seen: dict[str, set] = {}
        self.last_dom_error: str | None = None

    # ---- instruments -------------------------------------------------

    def verb(self, *argv: str, timeout: float | None = None) -> dict:
        """Run one `yggterm server app …` verb; return parsed reply + wall ms.
        The CLI runs in its own process group; a budget expiry killpg's the
        GROUP and drains bounded — one wedged app-control call costs its
        budget, never minutes ([11.200]: one iteration held 972895 ms)."""
        cmd = [YGGTERM, "server", "app", *argv]
        budget = timeout or self.timeout_s
        t0 = time.perf_counter()
        try:
            proc = subprocess.Popen(
                cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                text=True, start_new_session=True)
        except OSError as exc:
            return {"ok": False, "error": f"spawn failed: {exc}",
                    "wall_ms": int((time.perf_counter() - t0) * 1000),
                    "raw": ""}
        timed_out = False
        try:
            out, err = proc.communicate(timeout=budget)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError, OSError):
                proc.kill()
            try:
                out, err = proc.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                out, err = "", ""
        wall_ms = int((time.perf_counter() - t0) * 1000)
        if timed_out:
            reply: dict = {"ok": False,
                           "error": f"verb timeout (group killed at {budget:g}s)",
                           "wall_ms": wall_ms, "raw": (out or "")[-2000:]}
            self.dump_artifact(f"verb-{'-'.join(argv[:3])}-{now_ms()}.json", reply)
            return reply
        reply = {"ok": proc.returncode == 0, "wall_ms": wall_ms}
        try:
            reply["json"] = json.loads(out)
        except (json.JSONDecodeError, ValueError):
            reply["json"] = None
            reply["raw"] = ((out or "") + (err or ""))[-2000:]
        if proc.returncode != 0:
            reply["error"] = (err or out or "")[-500:]
        self.dump_artifact(f"verb-{'-'.join(argv[:3])}-{now_ms()}.json", reply)
        return reply

    # dom_eval_for_app_control answers {"error": …} in data when the eval
    # never produced a value; this class is the GUI's own retryable set
    # (app_control_eval_error_should_retry) — a FRESH eval is the medicine.
    EVAL_RETRYABLE_MARKERS = ("eval has already ran", "EvalError::Finished")

    def dom_eval(self, js: str, timeout: float = 15,
                 retries: int = 2) -> dict:
        """One app-control dom-eval, honest + retry-bounded. Returns the
        data dict: {"result": …} on success, {"error": …} on failure —
        NEVER silently empty ([11.200]: the battery read only data.result
        and reported every real failure as "split refused: None")."""
        last: dict = {}
        for attempt in range(retries + 1):
            r = self.verb("dom-eval", js, timeout=timeout)
            data = (r.get("json") or {}).get("data")
            if isinstance(data, dict):
                last = data
            elif r.get("ok"):
                last = {"error": "dom-eval returned no data payload"}
            else:
                last = {"error": f"dom-eval verb failed: "
                                 f"{r.get('error') or 'unknown'}"}
            if last.get("error") is None and "result" in last:
                return last
            reason = str(last.get("error"))
            if attempt < retries and any(m in reason
                                         for m in self.EVAL_RETRYABLE_MARKERS):
                time.sleep(0.3)
                continue
            break
        return last

    def ping_overhead_ms(self, samples: int = 5) -> float | None:
        """CLI round-trip floor: every verb measurement includes this."""
        walls = []
        for _ in range(samples):
            r = self.verb("clients")
            if r["ok"] and r["wall_ms"] is not None:
                walls.append(r["wall_ms"])
            time.sleep(0.05)
        return statistics.median(walls) if walls else None

    def gui_overhead_ms(self, samples: int = 3) -> float | None:
        """GUI-plane round-trip floor: one minimal dom-eval through the same
        app-control surface every action verb pays on. The CLI floor is
        daemon-side and stays QUIET while the GUI's UI thread is contended —
        the 2026-09-29 drag-warm-slowdown attribution measured drag verbs
        inflating ~1.2-2x under a concurrent sibling probe + perf capture
        while the CLI floor sat flat at 68 ms. Report both planes; a quiet
        cli_overhead_ms with an inflated gui_overhead_ms names GUI-side
        contention instead of laundering it into the action's numbers."""
        walls = []
        for _ in range(samples):
            t0 = time.perf_counter()
            r = self.dom_eval("dioxus.send(1);", timeout=5, retries=0)
            if r.get("error") is None:
                walls.append(int((time.perf_counter() - t0) * 1000))
            time.sleep(0.05)
        return statistics.median(walls) if walls else None

    def ytrace_events(self, since_ms: int, lines: int = 1200) -> list[dict]:
        try:
            proc = subprocess.run(
                [YTRACE, "tail", "--lines", str(lines), "--json"],
                capture_output=True, text=True, timeout=15)
            events = json.loads(proc.stdout)
        except Exception:
            return []
        return [e for e in events if isinstance(e, dict)
                and e.get("ts_ms", 0) >= since_ms]

    def rows(self) -> list[dict]:
        r = self.verb("rows", "--json")
        if not r["ok"] or not r.get("json"):
            return []
        return (r["json"].get("data") or {}).get("rows") or []

    def find_row(self, path: str) -> dict | None:
        for row in self.rows():
            if row.get("full_path") == path:
                return row
        return None

    def rows_order_indices(self, paths: tuple[str, ...]) -> dict[str, int | None]:
        """One listing fetch serves every lookup. wait_order used to make one
        full `rows --json` call PER PATH per poll — two ~1-4 s calls on a
        424-row sidebar ate its whole 5 s budget and produced false accuracy
        failures ([11.125]'s instrument note)."""
        out: dict[str, int | None] = {path: None for path in paths}
        for i, row in enumerate(self.rows()):
            fp = row.get("full_path")
            if fp in out and out[fp] is None:
                out[fp] = i
        return out

    def dump_artifact(self, name: str, payload) -> None:
        if not self.artifacts:
            return
        safe = "".join(c if c.isalnum() or c in "-._" else "_" for c in name)
        if len(safe) > 120:  # a dom-eval script must not become the filename
            safe = safe[:100] + "…" + safe[-16:]
        try:
            with open(os.path.join(self.artifacts, safe), "w") as fh:
                json.dump(payload, fh, indent=1, default=str)
        except OSError:
            pass

    # ---- scratch-row lifecycle ----------------------------------------

    def clean_stale(self) -> int:
        """Remove scratch rows left by an earlier crashed run (ours by title)."""
        removed = 0
        for row in self.rows():
            label = row.get("label") or ""
            if label.startswith(PROBE_TITLE_PREFIX):
                path = row.get("full_path")
                if path and self.remove_row(path):
                    removed += 1
        return removed

    def remove_row(self, path: str) -> bool:
        """session remove + poll-gone. ONLY legal on paths this run created
        (or stale scratch rows matching the probe title prefix)."""
        r = self.verb("session", "remove", path, timeout=30)
        verified = bool((r.get("json") or {}).get("data", {}).get("verified"))
        deadline = time.time() + 15
        while time.time() < deadline:
            if self.find_row(path) is None:
                if path in self.spawned_paths:
                    self.spawned_paths.remove(path)
                return verified
            time.sleep(0.08)
        return False

    def spawn_scratch_row(self, index: int, activate: bool = False) -> dict:
        """One scratch terminal row; returns the probe row-dict (not a report)."""
        title = f"{PROBE_TITLE_PREFIX}{index}-{now_ms()}"
        argv = ["terminal", "new", "--title", title, "--cwd", "/tmp"]
        if not activate:
            argv.append("--no-activate")
        t0 = now_ms()
        r = self.verb(*argv)
        if not r["ok"]:
            return {"error": f"terminal new failed: {r.get('error')}"}
        path = self.extract_session_path(r, title)
        if not path:
            return {"error": "no session path in reply or rows diff"}
        self.spawned_paths.append(path)
        self.scratch_titles.append({"path": path, "title": title})
        row = {"path": path, "title": title, "cli_ms": r["wall_ms"],
               "t0_ms": t0}
        data = (r.get("json") or {}).get("data") or {}
        row["daemon_processed_ms"] = (
            (r["json"].get("completed_at_ms") - t0)
            if r.get("json", {}).get("completed_at_ms") else None)
        row["queued"] = data.get("queued")
        # Milestones are DIAGNOSTIC since the [11.171] marker rework: on a
        # healthy spawn only open_attempt/mount fire, and waiting the full
        # action timeout for markers that never fire burns 12 s per
        # iteration. Cap the diagnostic window; the content poll owns the
        # honest paint leg.
        paint = self.wait_spawn_truth(path, since_ms=t0,
                                      milestone_timeout_s=min(self.timeout_s, 6.0))
        row["paint"] = paint
        return row

    def extract_session_path(self, reply: dict, title: str) -> str | None:
        """Pull local://<uuid> out of the reply, else fall back to a live
        rows lookup by the probe title."""
        def walk(node):
            if isinstance(node, str) and node.startswith("local://"):
                return node
            if isinstance(node, dict):
                for v in node.values():
                    hit = walk(v)
                    if hit:
                        return hit
            if isinstance(node, list):
                for v in node:
                    hit = walk(v)
                    if hit:
                        return hit
            return None
        hit = walk(reply.get("json"))
        if hit:
            row = self.find_row(hit)
            if row and (row.get("label") or "").startswith(title):
                return hit
        for row in self.rows():
            if (row.get("label") or "").startswith(title):
                return row.get("full_path")
        return None

    def wait_paint_milestones(self, path: str, since_ms: int,
                              timeout_s: float | None = None) -> dict:
        """Milestone ladder for one spawn, keyed by first ts per marker.

        first_frame (first WRITE→frame) does NOT fire for idle shells —
        the paint answer is xterm_paint/settle (or mount_open). Which
        marker answered is part of the result; a null ladder entry that
        never fired is a fact, not an error.
        """
        uuid = path.split("://")[-1]
        markers = {
            "open_attempt_begin": ("terminal_open_attempt", "begin"),
            "mount_begin": ("terminal_mount", "begin"),
            "mount_open": ("xterm_paint", "mount_open"),
            "paint_settle": ("xterm_paint", "settle"),
            "first_frame": ("xterm_paint", "first_frame"),
        }
        found: dict[str, dict] = {}
        deadline = time.time() + (timeout_s or self.timeout_s)
        while time.time() < deadline and len(found) < len(markers):
            for e in self.ytrace_events(since_ms):
                for key, (cat, name) in markers.items():
                    if key in found:
                        continue
                    if e.get("category") != cat or e.get("name") != name:
                        continue
                    if uuid not in json.dumps(e):
                        continue
                    found[key] = {"ts_ms": e["ts_ms"],
                                  "offset_ms": e["ts_ms"] - since_ms,
                                  "payload": e.get("payload")}
            if len(found) < len(markers):
                time.sleep(0.25)
        ff = found.get("first_frame", {})
        paint_marker = ("first_frame" if "first_frame" in found
                        else "paint_settle" if "paint_settle" in found
                        else "mount_open" if "mount_open" in found
                        else None)
        return {"milestones": found,
                "paint_marker": paint_marker,
                "spawn_to_paint_ms": found[paint_marker]["offset_ms"]
                if paint_marker else None,
                "open_to_write_ms": ff.get("payload", {}).get("open_to_write_ms"),
                "write_to_frame_ms": ff.get("payload", {}).get("write_to_frame_ms"),
                "blank_frames_before_write":
                    ff.get("payload", {}).get("blank_frames_before_write")}

    def wait_screen_content(self, path: str, since_ms: int,
                            timeout_s: float | None = None) -> dict:
        """THE spawn paint truth: poll the daemon screen until it holds
        nonblank content. The xterm_paint marker family does not answer
        idle-shell spawns — first_frame is write-scoped and settle has not
        fired on a spawn since the [11.171] marker rework — so a spawned row
        whose prompt VISIBLY painted answers the milestone ladder with
        silence (measured 2026-09-27: open_attempt/mount fire at ~465 ms,
        then no marker ever, while read-buffer shows the prompt).
        read-buffer is content- AND session-addressed (it never touches the
        viewport), so nonblank_line_count >= 1 is the honest verb→content
        leg. A null here on a live row IS the blank-first-screen defect
        class — this assert is what catches a broken-daemon build on
        iteration 1 instead of after five silent timeouts."""
        deadline = time.time() + (timeout_s or self.timeout_s)
        while time.time() < deadline:
            r = self.verb("terminal", "read-buffer", path, "--mode", "screen")
            data = (r.get("json") or {}).get("data") or {}
            n = data.get("nonblank_line_count")
            if r["ok"] and isinstance(n, int) and n > 0:
                return {"content_first_ms": now_ms() - since_ms,
                        "nonblank_line_count": n}
            time.sleep(0.12)
        return {"content_first_ms": None, "nonblank_line_count": None}

    def wait_spawn_truth(self, path: str, since_ms: int,
                         milestone_timeout_s: float = 6.0) -> dict:
        """ONE interleaved watcher for both spawn observation legs.

        The sequential form (milestone wait to its cap, THEN the content
        poll) overstated verb\u2192content by the whole milestone cap while
        content_first_ms kept counting from t0 \u2014 on e9cc152c the trace
        showed first_frame +3.0 s / settle +3.9 s while the probe reported
        content_first +8.3-9.2 s, 5/5 (2026-09-29). This loop polls
        read-buffer and scans the trace in the SAME pass, records every
        read-buffer wall (the poll-cost half of the content leg), and
        returns when both legs answered or their deadlines passed."""
        uuid = path.split("://")[-1]
        markers = {
            "open_attempt_begin": ("terminal_open_attempt", "begin"),
            "mount_begin": ("terminal_mount", "begin"),
            "mount_open": ("xterm_paint", "mount_open"),
            "paint_settle": ("xterm_paint", "settle"),
            "first_frame": ("xterm_paint", "first_frame"),
        }
        found: dict[str, dict] = {}
        content = {"content_first_ms": None, "nonblank_line_count": None,
                   "content_poll_walls_ms": [], "content_polls": 0}
        start = time.time()
        ct_deadline = start + self.timeout_s
        # The content poll runs with NO trace scanning in the loop: a
        # `ytrace tail` inside a hot generation costs seconds (measured
        # 5.6 s/call on jojo 2026-09-29) and that cost was landing inside
        # content_first_ms. Milestone offsets are ts_ms-based, so ONE scan
        # after the content leg answers recovers them exactly.
        while content["content_first_ms"] is None and time.time() < ct_deadline:
            t = time.time()
            r = self.verb("terminal", "read-buffer", path,
                          "--mode", "screen")
            content["content_poll_walls_ms"].append(
                round((time.time() - t) * 1000))
            content["content_polls"] += 1
            data = (r.get("json") or {}).get("data") or {}
            n = data.get("nonblank_line_count")
            if r["ok"] and isinstance(n, int) and n > 0:
                content["content_first_ms"] = now_ms() - since_ms
                content["nonblank_line_count"] = n
            else:
                time.sleep(0.15)
        for e in self.ytrace_events(since_ms, lines=2400):
            for key, (cat, name) in markers.items():
                if key in found:
                    continue
                if e.get("category") != cat or e.get("name") != name:
                    continue
                if uuid not in json.dumps(e):
                    continue
                found[key] = {"ts_ms": e["ts_ms"],
                              "offset_ms": e["ts_ms"] - since_ms,
                              "payload": e.get("payload")}
        ff = found.get("first_frame", {})
        paint_marker = ("first_frame" if "first_frame" in found
                        else "paint_settle" if "paint_settle" in found
                        else "mount_open" if "mount_open" in found
                        else None)
        walls = sorted(content["content_poll_walls_ms"])
        p50 = walls[len(walls) // 2] if walls else None
        return {"milestones": found,
                "paint_marker": paint_marker,
                "spawn_to_paint_ms": found[paint_marker]["offset_ms"]
                if paint_marker else None,
                "open_to_write_ms": ff.get("payload", {}).get("open_to_write_ms"),
                "write_to_frame_ms": ff.get("payload", {}).get("write_to_frame_ms"),
                "blank_frames_before_write":
                    ff.get("payload", {}).get("blank_frames_before_write"),
                "content": {"content_first_ms": content["content_first_ms"],
                            "nonblank_line_count": content["nonblank_line_count"]},
                "content_poll_walls_p50_ms": p50,
                "content_polls": content["content_polls"]}

    # ---- actions -------------------------------------------------------

    def action_spawn(self, iters: int) -> dict:
        out = {"iterations": []}
        for i in range(iters):
            row = self.spawn_scratch_row(i, activate=True)
            if "error" in row:
                out["iterations"].append({"error": row["error"]})
                continue
            paint = row["paint"]
            present = self.find_row(row["path"]) is not None
            acc = []
            if not present:
                acc.append("row absent from server app rows after spawn")
            content = paint.get("content") or {}
            content_ms = content.get("content_first_ms")
            if content_ms is None and paint.get("paint_marker") is None:
                acc.append("no screen content and no paint milestone "
                           "within timeout (blank first screen?)")
            out["iterations"].append({
                "spawn_to_paint_ms": content_ms
                if content_ms is not None else paint.get("spawn_to_paint_ms"),
                "paint_marker": "screen_content" if content_ms is not None
                else paint.get("paint_marker"),
                "content_first_ms": content_ms,
                "nonblank_line_count": content.get("nonblank_line_count"),
                "daemon_processed_ms": row.get("daemon_processed_ms"),
                "queued": row.get("queued"),
                "cli_new_ms": row["cli_ms"],
                "milestone_offsets_ms": {k: v["offset_ms"]
                                         for k, v in paint["milestones"].items()},
                "open_to_write_ms": paint.get("open_to_write_ms"),
                "write_to_frame_ms": paint.get("write_to_frame_ms"),
                "blank_frames_before_write": paint.get("blank_frames_before_write"),
                "trace_paint_marker": paint.get("paint_marker"),
                "trace_paint_ms": paint.get("spawn_to_paint_ms"),
                "content_poll_walls_p50_ms": paint.get("content_poll_walls_p50_ms"),
                "content_polls": paint.get("content_polls"),
                "accuracy_failures": acc,
            })
        return summarize(out, key="spawn_to_paint_ms",
                         rate_key="blank_frames_before_write")

    def _drag_verb_cycle(self, a_path: str, b_path: str) -> dict:
        """One timed begin/hover(before)/drop cycle + order assert + the
        un-timed drag-back that keeps iterations independent. Shared by
        action_drag and action_mergescale — the merge-scale A/B must measure
        the SAME cycle the §BASELINES drag rows did."""
        t0 = now_ms()
        rb = self.verb("drag", "begin", a_path)
        rh = self.verb("drag", "hover", b_path, "--placement", "before")
        rd = self.verb("drag", "drop")
        commit_ms = now_ms() - t0
        settled, settle_ms = self.wait_order(a_path, b_path)
        events = {e.get("name") for e in self.ytrace_events(t0)
                  if e.get("name")}
        acc = []
        if not (rb["ok"] and rh["ok"] and rd["ok"]):
            acc.append("verb failure begin=%s hover=%s drop=%s errors=%s" % (
                rb["ok"], rh["ok"], rd["ok"],
                [v.get("error") for v in (rb, rh, rd) if not v["ok"]]))
        if not settled:
            acc.append("order did not reflect [A before B] within timeout")
        it = {
            "commit_ms": commit_ms,
            "settle_ms": settle_ms,
            "begin_ms": rb["wall_ms"], "hover_ms": rh["wall_ms"],
            "drop_ms": rd["wall_ms"],
            "accuracy_failures": acc,
            "trace_events_in_window": sorted(e for e in events
                                             if e and "drag" in e.lower())
                                            or None,
            "merged_scale": self._merged_sizes_in_window(t0, now_ms()),
        }
        # drag back so iterations stay independent
        tb = now_ms()
        self.verb("drag", "begin", a_path)
        self.verb("drag", "hover", b_path, "--placement", "after")
        self.verb("drag", "drop")
        self.wait_order(b_path, a_path)
        it["dragback_ms"] = now_ms() - tb
        return it

    def action_drag(self, iters: int) -> dict:
        rows_ready = self.ensure_two_scratch_rows()
        if len(rows_ready) < 2:
            return {"error": "could not establish two scratch rows", "iterations": []}
        a_path, b_path = rows_ready[0], rows_ready[1]
        out = {"iterations": []}
        for i in range(iters):
            out["iterations"].append(self._drag_verb_cycle(a_path, b_path))
        return summarize(out, key="commit_ms")

    # ---- drag-merge-scale lane (2026-09-29, ACK-5b36ae4c33) ------------

    def _sidebar_expand_state(self) -> dict | None:
        """Per-group expander state + rendered row count from the sidebar
        DOM. rendered_rows is VIRTUALIZED context only — the authoritative
        merged size is the restore_debug/merge_rows telemetry (the sidebar
        renders a window; the merge walks the whole surface)."""
        js = """const ex = Array.from(document.querySelectorAll("[data-sidebar-group-expander]"));
const st = ex.map(b => {
  const row = b.closest("[data-sidebar-row-path]");
  return { p: row ? row.getAttribute("data-sidebar-row-path") : null,
           e: b.getAttribute("data-sidebar-group-expanded") };
});
dioxus.send({ groups: st, rendered_rows:
  document.querySelectorAll("[data-sidebar-row-path]").length });
"""
        r = self.dom_eval(js, timeout=10)
        if r.get("error") is not None:
            self.last_dom_error = str(r["error"])
            return None
        self.last_dom_error = None
        return r.get("result") or None

    def _set_group_expansion(self, want_expanded: bool,
                             only: list[str] | None = None,
                             max_rounds: int = 8) -> dict:
        """Click group expander buttons until every group (or every group in
        `only`) reaches want_expanded. Buttons are re-queried every round —
        each click's merge + re-render replaces them. Only groups whose
        state differs are clicked, so restoring never disturbs groups that
        were already in the wanted state."""
        t0 = time.perf_counter()
        clicked = 0
        state = None
        for _ in range(max_rounds):
            state = self._sidebar_expand_state()
            if state is None:
                break
            targets = [g["p"] for g in state["groups"]
                       if (g["e"] == "true") != want_expanded
                       and (only is None or g["p"] in only)]
            if not targets:
                break
            js = """const wanted = __WANT__;
const want_state = __STATE__;
let n = 0;
for (const b of document.querySelectorAll("[data-sidebar-group-expander]")) {
  const row = b.closest("[data-sidebar-row-path]");
  const p = row ? row.getAttribute("data-sidebar-row-path") : null;
  if (wanted.includes(p) &&
      b.getAttribute("data-sidebar-group-expanded") !== want_state) {
    b.click(); n += 1;
  }
}
dioxus.send(n);
""".replace("__WANT__", json.dumps(targets)).replace(
                "__STATE__", "true" if want_expanded else "false")
            r = self.dom_eval(js, timeout=10)
            if r.get("error") is not None:
                self.last_dom_error = str(r["error"])
                break
            clicked += r.get("result") or 0
            time.sleep(0.35)  # let the per-click merges + re-render settle
        return {"clicked": clicked,
                "wall_ms": int((time.perf_counter() - t0) * 1000),
                "final_state": state,
                "dom_error": self.last_dom_error}

    def _merged_sizes_in_window(self, t0: int, t1: int) -> dict:
        """merged_row_count + expanded_path_count seen in [t0, t1], from the
        two telemetry families that carry them: restore_debug (ui_telemetry
        on begin_drag/set_drag_hover_target/select_tree_row — payload is
        NESTED) and merge_rows/merge_rows_breakdown (sidebar perf, meta
        sub-payload). Both read levels defensively."""
        sizes: list[int] = []
        exps: list[int] = []
        for e in self._events_between(t0, t1):
            name = e.get("name")
            if name not in ("restore_debug", "merge_rows",
                            "merge_rows_breakdown"):
                continue
            p = e.get("payload", {})
            levels = [p]
            if isinstance(p, dict) and isinstance(p.get("payload"), dict):
                levels.append(p["payload"])
            for lv in levels:
                meta = lv.get("meta", lv) if isinstance(lv, dict) else {}
                if isinstance(meta, dict):
                    if isinstance(meta.get("merged_row_count"), int):
                        sizes.append(meta["merged_row_count"])
                    if isinstance(meta.get("expanded_path_count"), int):
                        exps.append(meta["expanded_path_count"])
                    eff = meta.get("effective_expanded_paths")
                    if isinstance(eff, list):
                        exps.append(len(eff))
        return {"merged_row_count": sizes, "expanded_path_count": exps}

    def action_mergescale(self, iters: int) -> dict:
        """THE drag-merge-scale A/B (lane/uxspeed/drag-merge-scale): the
        drag verb cycle at the AS-FOUND sidebar expansion vs ALL-EXPANDED,
        with merged_row_count read per cycle from the cycle's own trace
        window. The as-found per-group expansion map is RESTORED at the end
        and the restore VERIFIED — the owner's sidebar view-state is the
        restore contract. Not in the default battery (it mutates sidebar
        view-state mid-battery); run via --actions mergescale."""
        out: dict = {"iterations_collapsed": [], "iterations_expanded": []}
        rows_ready = self.ensure_two_scratch_rows()
        if len(rows_ready) < 2:
            return {"error": "could not establish two scratch rows"}
        a_path, b_path = rows_ready[0], rows_ready[1]
        time.sleep(0.4)

        snapshot = self._sidebar_expand_state()
        if not snapshot or not snapshot.get("groups"):
            return {"error": f"sidebar expand state unreadable: "
                             f"{self.last_dom_error}"}
        out["as_found"] = {
            "expanded_groups": [g["p"] for g in snapshot["groups"]
                                if g["e"] == "true"],
            "collapsed_groups": [g["p"] for g in snapshot["groups"]
                                 if g["e"] == "false"],
            "rendered_rows_dom": snapshot.get("rendered_rows"),
        }

        # arm A — as-found expansion (boot state: folders collapsed)
        for _ in range(iters):
            out["iterations_collapsed"].append(
                self._drag_verb_cycle(a_path, b_path))

        # arm B — everything expanded
        out["expand_all"] = self._set_group_expansion(True)
        time.sleep(0.6)  # let the last merge + re-render settle
        expanded_state = self._sidebar_expand_state()
        out["expanded_dom"] = {
            "rendered_rows_dom": (expanded_state or {}).get("rendered_rows"),
            "groups": len((expanded_state or {}).get("groups") or []),
            "state_error": self.last_dom_error,
        }
        for _ in range(iters):
            out["iterations_expanded"].append(
                self._drag_verb_cycle(a_path, b_path))

        # restore the as-found map EXACTLY and verify
        out["restore"] = self._set_group_expansion(
            False, only=out["as_found"]["collapsed_groups"])
        time.sleep(0.4)
        final = self._sidebar_expand_state()
        now_map = {g["p"]: g["e"] for g in ((final or {}).get("groups") or [])}
        was_map = {g["p"]: g["e"] for g in snapshot["groups"]}
        out["restore_verified"] = (now_map == was_map) if final else None
        if out["restore_verified"] is False:
            out["accuracy_failures"] = [
                {"expansion_state_drifted": {"was": was_map, "now": now_map}}]

        for arm, key in (("iterations_collapsed", "collapsed"),
                         ("iterations_expanded", "expanded")):
            its = out[arm]
            for f in ("commit_ms", "begin_ms", "hover_ms", "drop_ms",
                      "settle_ms", "dragback_ms"):
                vals = [it[f] for it in its if isinstance(it.get(f), int)]
                if vals:
                    out[f"{key}_{f}"] = {"p50": statistics.median(vals),
                                         "min": min(vals), "max": max(vals),
                                         "n": len(vals)}
            mvals = sorted(m for it in its
                           for m in it["merged_scale"]["merged_row_count"])
            out[f"{key}_merged_row_count"] = (
                {"min": mvals[0], "max": mvals[-1],
                 "median": statistics.median(mvals), "n": len(mvals)}
                if mvals else None)
        return out

    def _row_rects(self, paths: list[str]) -> dict:
        """Sidebar DOM rects for row paths, via one dom-eval. The sidebar
        virtualizes — the caller must `tree select` first so nodes exist.
        An eval failure returns {} and names itself in last_dom_error, so a
        latched eval plane can never masquerade as "virtualized out"
        ([11.200] misattribution)."""
        js = """const out = {};
for (const p of __PATHS__) {
  let el = null;
  for (const n of document.querySelectorAll('[data-sidebar-row-path]')) {
    if (n.getAttribute('data-sidebar-row-path') === p) { el = n; break; }
  }
  if (!el) { out[p] = null; continue; }
  const r = el.getBoundingClientRect();
  out[p] = { x: r.x, y: r.y, w: r.width, h: r.height };
}
dioxus.send(out);
""".replace("__PATHS__", json.dumps(paths))
        r = self.dom_eval(js, timeout=15)
        if r.get("error") is not None:
            self.last_dom_error = str(r["error"])
            return {}
        self.last_dom_error = None
        return r.get("result") or {}

    def _events_between(self, t0: int, t1: int) -> list[dict]:
        return [e for e in self.ytrace_events(t0) if e.get("ts_ms", 0) <= t1]

    def render_attribution(self, t0: int, t1: int, hover_steps: int) -> dict:
        """[11.129] gesture-scoped render attribution. The falsifier: on the
        fixed build the drag-pointer stream re-renders the ghost LEAF only
        (DragGhost / RowDragGhost read the small drag_ghost_pointer signal);
        the `app` component pays only the gesture's real edges (press,
        release), never per pointer step. Honesty: component_window events
        are AGGREGATES (~2.5-7 s windows) so a gesture's windows also hold
        ambient churn — the report carries both the per-component sums and
        the app-renders-per-hover-step ratio, and the caller contrasts
        against the ambient windows this helper also returns."""
        evs = self.ytrace_events(max(0, t0 - 20000), lines=1200)
        wins = [e for e in evs
                if e.get("name") == "component_window"
                and t0 <= e.get("ts_ms", 0) <= t1 + 12000]
        pre = sorted((e for e in evs
                      if e.get("name") == "component_window"
                      and e.get("ts_ms", 0) < t0),
                     key=lambda e: e.get("ts_ms", 0))[-1:]
        def per_component(windows):
            sums: dict[str, int] = {}
            for w in windows:
                for c in (w.get("payload", {}) or {}).get("components", []):
                    name = c.get("component") or "?"
                    sums[name] = sums.get(name, 0) + int(c.get("renders") or 0)
            return sums
        sums = per_component(wins)
        ambient_wins = sorted((e for e in evs
                               if e.get("name") == "component_window"
                               and e.get("ts_ms", 0) < t0),
                              key=lambda e: e.get("ts_ms", 0))[-3:]
        ambient = per_component(ambient_wins)
        ghost = sums.get("DragGhost", 0) + sums.get("RowDragGhost", 0)
        app = sums.get("app", 0)
        ambient_app = (ambient.get("app", 0) / len(ambient_wins)
                       if ambient_wins else None)
        causes = sorted(
            ((c.get("site"), int(c.get("writes") or 0),
              int(c.get("renders_preceded") or 0))
             for w in wins
             for c in (w.get("payload", {}) or {}).get("causes", [])),
            key=lambda t: -t[2])
        return {
            "component_windows_in_gesture": len(wins),
            "component_window_ms_total": sum(
                int((w.get("payload", {}) or {}).get("window_ms") or 0)
                for w in wins),
            "app_renders_in_gesture_windows": app,
            "ghost_leaf_renders_in_gesture_windows": ghost,
            "ghost_per_hover_step": round(ghost / hover_steps, 2)
                                    if hover_steps else None,
            "app_renders_per_hover_step": round(app / hover_steps, 2)
                                          if hover_steps else None,
            "ambient_app_renders_mean3": (round(ambient_app, 1)
                                          if ambient_app is not None else None),
            "components_seen": sums,
            "top_causes_by_renders": [
                {"site": s, "writes": w, "renders_preceded": r}
                for s, w, r in causes[:8]],
            "max_site_writes": max((w for _, w, _ in causes), default=0),
        }

    def assert_render_attribution(self, ra: dict, hover_steps: int = 0) -> list[str]:
        """[11.129] falsifier reads: the ghost LEAF re-rendered with the
        pointer stream (the drag_ghost_pointer signal's subscribers), and no
        ShellState write site shows a per-move storm (the old whole-shell
        tax paired writes-with-steps on update_drag_pointer; the fix keeps
        only the gesture's real edges — begin, hover-target change, clear —
        in ShellState)."""
        acc: list[str] = []
        if ra["component_windows_in_gesture"] == 0:
            acc.append("no component_window in the gesture window "
                       "(render attribution unmeasurable this iteration)")
            return acc
        if not ra["ghost_leaf_renders_in_gesture_windows"]:
            acc.append("ghost leaf never re-rendered in the gesture windows "
                       "(ghost card missing, or the pointer stream writes "
                       "nothing — [11.129] regression?)")
        # ⚠ REPORT-ONLY: component_window aggregates (~2.5-7 s) on a
        # multi-seat desktop hold other seats' churn, so a bare writes-count
        # alarm false-positives. The [11.129] proof reads the ghost-leaf
        # renders (with the stream) and the ABSENCE of a drag-pointer write
        # site from the causes array, not this counter.
        storm_floor = max(4, 2 * int(hover_steps or 0))
        ra["storm_suspect"] = ra["max_site_writes"] > storm_floor
        return acc

    def action_felt(self, iters: int) -> dict:
        """The felt drag: real pointer events, not the drag verbs. A FRESH
        scratch pair per iteration (no drag-back restore to go wrong): press
        on A, DWELL under the 6px threshold (sub-threshold moves — must merge
        nothing and not begin), cross the threshold (tree_drag_begin fires
        here), hover across row boundaries, release on B's lower half (the
        After band) — then assert the reorder and the trace falsifiers. All
        spawned rows close in the shared teardown."""
        out = {"iterations": []}
        for i in range(iters):
            rows = [self.spawn_scratch_row(i * 2 + k) for k in range(2)]
            paths = [r.get("path") for r in rows if "path" in r]
            if len(paths) < 2:
                out["iterations"].append({"accuracy_failures": [
                    "spawn failed: %s" % [r.get("error") for r in rows
                                          if "error" in r]]})
                continue
            a_path, b_path = paths
            time.sleep(0.4)  # let the spawn-promotion apply settle first
            self.verb("tree", "select", a_path)
            self.verb("tree", "select", b_path)
            time.sleep(0.2)
            rects = self._row_rects([a_path, b_path])
            ra, rb = rects.get(a_path), rects.get(b_path)
            acc = []
            if not ra or not rb:
                out["iterations"].append({"accuracy_failures": [
                    "row node not rendered (virtualized out?) — rects missing"]})
                continue
            ax = ra["x"] + min(ra["w"] / 2, 120.0)
            ay = ra["y"] + ra["h"] / 2
            ux = rb["x"] + min(rb["w"] / 2, 120.0)
            uy = rb["y"] + rb["h"] - 3.0  # the After band (bottom edge)
            it = {"dwell_steps": 0, "hover_steps": 0}
            t_down = now_ms()
            rd = self.verb("pointer", "press", "--x", str(int(ax)),
                           "--y", str(int(ay)))
            it["down_ms"] = rd["wall_ms"]
            dwell_t0 = now_ms()
            for dy in (2, 1, -1):
                self.verb("pointer", "move", "--x", str(int(ax + 2)),
                          "--y", str(int(ay + dy)))
                time.sleep(0.12)
                it["dwell_steps"] += 1
            dwell = {e.get("name") for e in self._events_between(
                dwell_t0, now_ms())}
            it["dwell_merge_events"] = sorted(
                e for e in dwell if e == "merge_rows_breakdown")
            it["dwell_drag_events"] = sorted(
                e for e in dwell if e and "drag" in e.lower())
            cross_t = now_ms()
            self.verb("pointer", "move", "--x", str(int(ax)),
                      "--y", str(int(ay + 40)))
            for s_i in range(1, 5):
                y = ay + 40 + (uy - ay - 40) * s_i / 4
                self.verb("pointer", "move", "--x", str(int(ux)),
                          "--y", str(int(y)))
                time.sleep(0.05)
                it["hover_steps"] += 1
            self.verb("pointer", "release")
            t_release = now_ms()
            flipped, flip_ms = self.wait_order(b_path, a_path)
            it["felt_ms"] = now_ms() - t_down
            it["reorder_settle_ms"] = flip_ms
            it["paths"] = {"a": a_path, "b": b_path}
            evs = self._events_between(t_down, now_ms())
            gesture_evs = [e for e in evs
                           if e.get("ts_ms", 0) <= t_release]
            names = [e.get("name") for e in evs]
            begins = [e["ts_ms"] for e in gesture_evs
                      if e.get("name") == "tree_drag_begin"]
            it["drag_events"] = sorted({n for n in names
                                        if n and "drag" in n.lower()})
            it["merge_events_in_drag"] = sum(
                1 for e in gesture_evs if e.get("name") == "merge_rows_breakdown")
            it["merge_events_post_release"] = sum(
                1 for e in evs if e.get("ts_ms", 0) > t_release
                and e.get("name") == "merge_rows_breakdown")
            it["merges_in_drag_detail"] = [
                {"ts_offset_ms": e.get("ts_ms", 0) - t_down,
                 "rows": (e.get("payload", {}).get("payload",
                          e.get("payload", {})) or {}).get("merged_row_count"),
                 "total_ms": round((e.get("payload", {}).get("payload",
                                    e.get("payload", {})) or {})
                                   .get("total_ms") or 0, 1)}
                for e in evs if e.get("name") == "merge_rows_breakdown"]
            it["begin_within_cross_step"] = bool(begins)
            if not begins:
                acc.append("no tree_drag_begin in the gesture window")
            if it["dwell_merge_events"]:
                acc.append("merges fired during the sub-threshold dwell: %s"
                           % it["dwell_merge_events"])
            if it["dwell_drag_events"]:
                acc.append("drag events fired during the dwell (began early?):"
                           " %s" % it["dwell_drag_events"])
            if not flipped:
                acc.append("drop did not land A after B within timeout")
            else:
                ra_ = self.find_row(a_path) or {}
                rb_ = self.find_row(b_path) or {}
                if ra_ and rb_ and ra_.get("depth") != rb_.get("depth"):
                    acc.append("A landed NESTED under B (into-band drop), "
                               "not reordered: depth %s vs %s"
                               % (ra_.get("depth"), rb_.get("depth")))
            it["render_attribution"] = self.render_attribution(
                t_down, t_release, it["hover_steps"])
            acc.extend(self.assert_render_attribution(
                it["render_attribution"], it["hover_steps"]))
            it["accuracy_failures"] = acc
            out["iterations"].append(it)
        return summarize(out, key="felt_ms")

    def _ui_payload(self, e: dict) -> dict:
        p = e.get("payload", {})
        return p.get("payload", p) if isinstance(p, dict) else {}

    def wait_relative_order(self, expected: list[str],
                            timeout_s: float = 6.0) -> tuple[bool, int | None]:
        """True iff the expected paths sit in exactly this relative order.
        NOT contiguity — on a live desktop other seats' rows legally interleave
        between ours, and demanding contiguity manufactured false failures."""
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < timeout_s:
            idx = self.rows_order_indices(tuple(expected))
            seq = [idx[p] for p in expected]
            if all(s is not None for s in seq) and seq == sorted(seq):
                return True, int((time.perf_counter() - t0) * 1000)
            time.sleep(0.3)
        return False, None

    def _stable_rects(self, paths: list[str], tries: int = 4) -> dict | None:
        """Sidebar rects, fetched twice and required equal — the sidebar
        smooth-scrolls after a tree select, and a rect fetched mid-scroll
        points at the WRONG ROW (the shiftdrag driver's first lesson)."""
        last = None
        for _ in range(tries):
            r = self._row_rects(paths)
            if r and all(r.get(p) for p in paths):
                if last == r:
                    return r
                last = r
            time.sleep(0.25)
        return last if last and all(last.get(p) for p in paths) else None

    def action_shiftdrag(self, iters: int) -> dict:
        """The owner's goal names SHIFT-DRAG. Yggterm semantics (the sidebar
        row's onmousedown guard refuses to start a drag with any modifier
        held): shift+click EXTENDS the range selection
        (TreeSelectionMode::ExtendRange) and a plain drag then carries the
        WHOLE selected set (tree_drag_begin {anchor, drag_paths}). So the
        felt shift-drag = a multi-row range selection, then a REAL pointer
        drag of the set onto a target outside it. Selection setup rides the
        `tree select <paths…> --anchor` verb (SetTreeSelection — setup, not
        the measured action); the measured action is the pointer drag of the
        set. Asserts the begin payload carries exactly the set anchored on
        the pressed row, the family completes, the set lands after the
        target in set order, and the [11.129] render attribution."""
        out = {"iterations": []}
        for i in range(iters):
            rows = [self.spawn_scratch_row(i * 4 + k) for k in range(4)]
            paths = [r.get("path") for r in rows if "path" in r]
            if len(paths) < 4:
                out["iterations"].append({"accuracy_failures": [
                    "spawn failed: %s" % [r.get("error") for r in rows
                                          if "error" in r]]})
                continue
            a_path, b_path, c_path, d_path = paths
            acc = []
            ok0, _ = self.wait_relative_order([a_path, b_path, c_path, d_path])
            if not ok0:
                acc.append("scratch rows did not settle in spawn order")
            rs = self.verb("tree", "select", a_path, b_path, c_path,
                           "--anchor", a_path)
            if not rs["ok"]:
                acc.append("set-selection verb failed: %s" % rs.get("error"))
            time.sleep(0.4)
            it = {"hover_steps": 0, "attempts": 1, "drag_events": [],
                  "commit_persist_events": 0, "felt_ms": None,
                  "reorder_settle_ms": None}
            names: list = []
            begins: list[dict] = []
            dp = None
            landed = False
            tree_drop_ignored = False
            for attempt in (1, 2):
                rects = self._stable_rects([a_path, d_path])
                ra, rd = (rects or {}).get(a_path), (rects or {}).get(d_path)
                if not ra or not rd:
                    acc.append("row node not rendered (virtualized out?) "
                               "— rects missing")
                    break
                ax = ra["x"] + min(ra["w"] / 2, 120.0)
                ay = ra["y"] + ra["h"] / 2
                dx = rd["x"] + min(rd["w"] / 2, 120.0)
                dy = rd["y"] + rd["h"] - 3.0  # the After band (bottom edge)
                it = {"hover_steps": 0, "attempts": attempt}
                t_down = now_ms()
                rpress = self.verb("pointer", "press", "--x", str(int(ax)),
                                   "--y", str(int(ay)))
                it["down_ms"] = rpress["wall_ms"]
                self.verb("pointer", "move", "--x", str(int(ax)),
                          "--y", str(int(ay + 40)))
                for s_i in range(1, 6):
                    y = ay + 40 + (dy - ay - 40) * s_i / 5
                    self.verb("pointer", "move", "--x", str(int(dx)),
                              "--y", str(int(y)))
                    time.sleep(0.05)
                    it["hover_steps"] += 1
                self.verb("pointer", "release")
                t_release = now_ms()
                landed, settle_ms = self.wait_relative_order(
                    [d_path, a_path, b_path, c_path])
                it["felt_ms"] = now_ms() - t_down
                it["reorder_settle_ms"] = settle_ms
                evs = self._events_between(t_down, now_ms())
                gesture_evs = [e for e in evs
                               if e.get("ts_ms", 0) <= t_release]
                names = [e.get("name") for e in evs]
                begins = [self._ui_payload(e) for e in gesture_evs
                          if e.get("name") == "tree_drag_begin"]
                it["drag_events"] = sorted({n for n in names
                                            if n and "drag" in n.lower()})
                it["merge_events_in_drag"] = sum(
                    1 for e in gesture_evs
                    if e.get("name") == "merge_rows_breakdown")
                it["merge_events_post_release"] = sum(
                    1 for e in evs if e.get("ts_ms", 0) > t_release
                    and e.get("name") == "merge_rows_breakdown")
                dp = begins[-1].get("drag_paths") if begins else None
                it["begin_drag_paths"] = dp
                tree_drop_ignored = any(
                    e.get("name") == "tree_drop_ignored" for e in evs)
                it["begin_anchor"] = (begins[-1].get("anchor")
                                      if begins else None)
                it["paths"] = {"a": a_path, "b": b_path, "c": c_path,
                               "d": d_path}
                if begins and begins[-1].get("anchor") not in (None, a_path):
                    # The press landed on a different row than the rect said
                    # (scroll shifted under us) — one honest retry.
                    acc.append("attempt %d pressed %s, expected %s "
                               "(rect staleness)" % (
                                   attempt, begins[-1].get("anchor"), a_path))
                    it["attempts"] = attempt
                    if attempt == 1:
                        time.sleep(0.5)
                        continue
                break
            if not begins:
                acc.append("no tree_drag_begin in the gesture window")
            else:
                if set(dp or []) != {a_path, b_path, c_path}:
                    acc.append("tree_drag_begin.drag_paths != the selected "
                               "set: %s" % dp)
                if begins[-1].get("anchor") != a_path:
                    acc.append("tree_drag_begin.anchor != %s: %s"
                               % (a_path, begins[-1].get("anchor")))
            for need in ("tree_drag_hover", "tree_drag_ended"):
                if need not in it["drag_events"]:
                    acc.append("drag family missing %s" % need)
            it["commit_persist_events"] = sum(
                1 for n in names if n == "live_session_persist_dropped")
            if not it["commit_persist_events"] and not tree_drop_ignored:
                acc.append("drop committed nothing "
                           "(no live_session_persist_dropped in window)")
            if not landed:
                acc.append("drop did not land the set after D in set order "
                           "within timeout")
            it["render_attribution"] = self.render_attribution(
                t_down, t_release, it["hover_steps"])
            acc.extend(self.assert_render_attribution(
                it["render_attribution"], it["hover_steps"]))
            it["accuracy_failures"] = acc
            out["iterations"].append(it)
        return summarize(out, key="felt_ms")

    def action_switch(self, iters: int) -> dict:
        """The felt switch: real pointer CLICKS on sidebar tree rows — press
        and release at the row node, no threshold cross (a click must never
        begin a drag) — so the switch enters through the SAME user-gesture
        path a human's click takes (session/activation origin:user_gesture).
        One scratch pair serves the whole action; clicks alternate A→B so
        every click is a real switch, click 1 marked `cold` (the row's first
        activation this GUI boot). Per iteration: the activation event's
        latency + identity (to == the clicked row), then the [11.171] END
        MARKER — the common-path switch remounts the surface and the paint
        chain reports it (terminal_mount/begin {session_path, host_id} →
        xterm_paint/first_frame {open_to_write_ms, rows_painted} →
        xterm_paint/settle {painted, complete, rows_content_unpainted}),
        paired by host_id else session_path, never by time-proximity alone;
        terminal_mount/reveal_forced_incomplete is the honest incomplete end
        (the ui/reveal family covers only first-reveal-after-boot). Plus the
        ambient churn inside the window (ui/block, merge_rows_breakdown) —
        the felt cost line. Walls are overhead-inclusive (release verb wall
        included); the report's cli_overhead_ms is the floor to subtract.
        paint_end_rate is the falsifier number: >=7/8 clicks must report a
        first_frame end with rows painted."""
        rows_ready = self.ensure_two_scratch_rows()
        if len(rows_ready) < 2:
            return {"error": "could not establish two scratch rows",
                    "iterations": []}
        a_path, b_path = rows_ready
        # make both nodes exist under the sidebar's virtualization, as felt does
        self.verb("tree", "select", a_path)
        self.verb("tree", "select", b_path)
        time.sleep(0.2)
        # the select's scroll-into-view is async: verify the rects with a
        # bounded poll, and name an eval-plane failure AS an eval-plane
        # failure — "virtualized out" is only honest when the eval answered
        # and the nodes truly are not in the DOM ([11.200])
        rects: dict = {}
        for attempt in range(3):
            rects = self._row_rects([a_path, b_path])
            if rects.get(a_path) and rects.get(b_path):
                break
            time.sleep(0.5)
        if not rects.get(a_path) or not rects.get(b_path):
            if self.last_dom_error:
                return {"error": "row rects unavailable — dom-eval plane "
                                 f"failed: {self.last_dom_error}",
                        "iterations": []}
            return {"error": "row nodes not rendered (virtualized out? — "
                             "eval answered, nodes absent from the DOM)",
                    "iterations": []}
        out = {"iterations": []}
        confirmed_active: str | None = None

        def _other(p: str) -> str:
            return b_path if p == a_path else a_path

        for i in range(iters):
            # The blind A→B alternation assumed every click switches; one
            # swallowed click ([11.130] class: events land, the gesture path
            # does not fire) desynced it and every later click could hit the
            # already-active row — the 6/16 no-activation cascade
            # (2026-09-27). Target from the LAST CONFIRMED active row so a
            # measured click is always a real switch; a swallowed click
            # costs one unmeasured re-sync click at the bottom of the loop.
            path = (_other(confirmed_active)
                    if confirmed_active in (a_path, b_path)
                    else (a_path if i % 2 == 0 else b_path))
            rect = rects[path]
            x = rect["x"] + min(rect["w"] / 2, 120.0)
            y = rect["y"] + rect["h"] / 2
            it = {"cold": i == 0, "target": path}
            acc: list[str] = []
            rd = self.verb("pointer", "press", "--x", str(int(x)),
                           "--y", str(int(y)))
            it["down_ms"] = rd["wall_ms"]
            t_release = now_ms()
            rr = self.verb("pointer", "release")
            it["up_ms"] = rr["wall_ms"]
            activation = None
            mount_begin = None
            first_frame = None
            settle = None
            forced = None
            reveal = None
            # [11.172] THE REVEAL RAISE ENDS. A switch served as a raise of a
            # retained host never fires the mount chain: it fires
            # terminal_mount/reveal_served (the Rust raise marker) and
            # xterm_paint/reveal (the stamp script's paint truth — visibility
            # geometry + buffer content, NO xterm re-init). Both are honest
            # ends in their own families; settle cannot follow a raise (the
            # settle recheck belongs to the mount chain), so the raise's break
            # does not wait for one.
            reveal_served = None
            paint_reveal = None
            deadline = time.time() + self.timeout_s
            t_loop = time.time()
            while time.time() < deadline:
                evs = self.ytrace_events(t_release, lines=1200)
                if activation is None:
                    for e in evs:
                        p = e.get("payload") or {}
                        if (e.get("name") == "activation"
                                and e.get("category") == "session"
                                and p.get("origin") == "user_gesture"
                                and str(p.get("to") or "").startswith(path)):
                            activation = e
                            break
                if activation is not None:
                    # [11.171] THE END MARKER. The common-path switch remounts
                    # the surface, and the paint chain reports it:
                    # terminal_mount/begin {session_path, host_id} →
                    # xterm_paint/first_frame {session_path, paint truth} →
                    # xterm_paint/settle {complete, coverage}. Pair by host_id
                    # from THIS window's mount_begin when it is present, else by
                    # session_path — never by time-proximity alone, so the
                    # previous switch's settle recheck cannot impersonate this
                    # one. terminal_mount/reveal_forced_incomplete is the
                    # honest incomplete end (the ui/reveal family only covers
                    # first-reveal-after-boot).
                    host_id = (mount_begin or {}).get("payload", {}).get("host_id")
                    act_ts = activation.get("ts_ms", 0)
                    for e in evs:
                        p = e.get("payload") or {}
                        sp = str(p.get("session_path") or "")
                        ts = e.get("ts_ms", 0)
                        n, c = e.get("name"), e.get("category")
                        identity = sp.startswith(path) or (
                            host_id is not None and p.get("host_id") == host_id)
                        if ts < act_ts or not identity:
                            continue
                        if mount_begin is None and c == "terminal_mount" and n == "begin":
                            mount_begin = e
                            host_id = host_id or p.get("host_id")
                        elif first_frame is None and c == "xterm_paint" and n == "first_frame":
                            first_frame = e
                        elif settle is None and c == "xterm_paint" and n == "settle":
                            settle = e
                        elif forced is None and c == "terminal_mount" and n == "reveal_forced_incomplete":
                            forced = e
                        elif reveal is None and c == "reveal" and n in (
                                "reveal_ready", "reveal_forced_incomplete",
                                "reveal_failed"):
                            reveal = e
                        elif (reveal_served is None and c == "terminal_mount"
                                and n == "reveal_served"):
                            reveal_served = e
                        elif (paint_reveal is None and c == "xterm_paint"
                                and n == "reveal"):
                            paint_reveal = e
                if activation is not None and (
                        first_frame is not None or forced is not None
                        or reveal is not None or paint_reveal is not None) and (
                        settle is not None or paint_reveal is not None
                        or time.time() - t_loop > 4.0):
                    break
                time.sleep(0.05)
            window = evs
            names = [e.get("name") for e in window]
            it["ui_block_in_window"] = sum(1 for n in names if n == "block")
            it["merge_events_in_window"] = sum(
                1 for n in names if n == "merge_rows_breakdown")
            if activation is None:
                acc.append("no user_gesture activation for the clicked row "
                           "within timeout — click did not switch (the "
                           "[11.130] class: synthetic pointer events land "
                           "but the gesture path does not fire)")
            else:
                it["activation_ms"] = activation.get("ts_ms", 0) - t_release
            if activation is not None:
                if mount_begin is not None:
                    it["mount_begin_ms"] = mount_begin.get("ts_ms", 0) - t_release
                if reveal_served is not None:
                    it["reveal_served_ms"] = (reveal_served.get("ts_ms", 0)
                                              - t_release)
                if first_frame is not None:
                    it["first_frame_ms"] = first_frame.get("ts_ms", 0) - t_release
                    fp = first_frame.get("payload") or {}
                    it["open_to_write_ms"] = fp.get("open_to_write_ms")
                    it["write_to_frame_ms"] = fp.get("write_to_frame_ms")
                    it["rows_painted"] = fp.get("rows_painted")
                    if not fp.get("rows_painted"):
                        acc.append("first_frame painted no rows — not paint "
                                   "truth (rows_painted=%r)" % fp.get("rows_painted"))
                elif paint_reveal is not None:
                    # [11.172] the raise's paint truth: the retained canvas
                    # is visible and holds content WITHOUT any re-init.
                    it["paint_reveal_ms"] = (paint_reveal.get("ts_ms", 0)
                                             - t_release)
                    rp = paint_reveal.get("payload") or {}
                    it["reveal_visible"] = rp.get("visible")
                    it["reveal_content_rows"] = rp.get("content_rows")
                    it["reveal_rows"] = rp.get("rows")
                    if rp.get("visible") is not True:
                        acc.append("reveal stamp says the host is not "
                                   "visible — the raise did not paint "
                                   "(visible=%r)" % (rp.get("visible"),))
                    if not rp.get("content_rows"):
                        acc.append("reveal stamp found no content rows — "
                                   "the retained canvas is empty "
                                   "(content_rows=%r)" % (rp.get("content_rows"),))
                else:
                    acc.append("no xterm_paint/first_frame for the clicked row "
                               "within the window — the felt switch has no "
                               "paint end ([11.171] unfixed on this build)")
                if settle is not None:
                    it["settle_ms"] = settle.get("ts_ms", 0) - t_release
                    sp = settle.get("payload") or {}
                    it["settle_painted"] = sp.get("painted")
                    it["settle_complete"] = sp.get("complete")
                    it["open_to_frame_ms"] = sp.get("open_to_frame_ms")
                    it["rows_content_unpainted"] = sp.get("rows_content_unpainted")
                    if not sp.get("painted"):
                        acc.append("settle says painted=false — no frame after "
                                   "bytes reached the canvas")
                    elif sp.get("complete") is False:
                        acc.append("settle incomplete: rows_content_unpainted="
                                   "%r (partial paint, the eye sees it)" %
                                   sp.get("rows_content_unpainted"))
                if forced is not None and first_frame is None:
                    it["forced_incomplete_ms"] = forced.get("ts_ms", 0) - t_release
                    # The forced event's two counts are the verdict: client
                    # rows <3 fired the forced write; the daemon count (new
                    # 2026-09-27, absent on older builds) says whether the
                    # screen was LEGITIMATELY short (benign) or the client
                    # LOST its frame while the daemon held content — the
                    # blank-flash class.
                    fp = (forced.get("payload") or {}).get("payload",
                         forced.get("payload") or {})
                    it["forced_client_rows"] = fp.get("visible_nonblank_rows")
                    it["forced_daemon_rows"] = fp.get("daemon_visible_nonblank_rows")
                    it["forced_defer_chain_ms"] = fp.get("defer_chain_ms")
                    dr = it["forced_daemon_rows"]
                    if dr is not None and dr >= 3:
                        acc.append("forced-incomplete is a LOST CLIENT FRAME "
                                   "(daemon holds %s nonblank rows, client "
                                   "showed %s, wrong frame stood %sms) — the "
                                   "blank-flash class, not a short screen"
                                   % (dr, it["forced_client_rows"],
                                      it["forced_defer_chain_ms"]))
                    else:
                        acc.append("switch ended forced-incomplete (reveal never "
                                   "became meaningful within the deadline)")
                if reveal is not None:
                    it["reveal_name"] = reveal.get("name")
                    it["reveal_ms"] = reveal.get("ts_ms", 0) - t_release
                    rp = (reveal.get("payload") or {}).get("payload",
                         reveal.get("payload") or {})
                    it["reveal_first_output_ms"] = rp.get("first_output_ms")
                    if reveal.get("name") != "reveal_ready":
                        acc.append(f"reveal did not reach clean ready: "
                                   f"{reveal.get('name')}")
            if not (rd["ok"] and rr["ok"]):
                acc.append("pointer verb failure press=%s release=%s"
                           % (rd["ok"], rr["ok"]))
            it["accuracy_failures"] = acc
            out["iterations"].append(it)
            if activation is None:
                # Swallowed click: the active state is unconfirmed, so the
                # next measured click could hit the already-active row.
                # Re-sync with one UNMEASURED click on the other row (the
                # owed [11.130]-class probe artifact) and reset the
                # confirmed state.
                orect = rects[_other(path)]
                self.verb("pointer", "press", "--x",
                          str(int(orect["x"] + min(orect["w"] / 2, 120.0))),
                          "--y", str(int(orect["y"] + orect["h"] / 2)))
                self.verb("pointer", "release")
                confirmed_active = None
                out["swallowed_clicks"] = out.get("swallowed_clicks", 0) + 1
                time.sleep(0.4)
            else:
                confirmed_active = path
            # give the previous switch's churn room to drain before the next
            time.sleep(0.6)
        ready = [i for i in out["iterations"]
                 if i.get("reveal_name") == "reveal_ready"]
        out["reveal_ready_rate"] = len(ready) / len(out["iterations"])
        # [11.171]+[11.172] THE FALSIFIER: >=7/8 clicks report a paint-truth
        # end — first_frame with rows painted (a mount) or xterm_paint/reveal
        # with content (a raise). This rate is the number the door's
        # activation→paint column exists to carry.
        ends = [i for i in out["iterations"]
                if i.get("first_frame_ms") is not None
                or i.get("paint_reveal_ms") is not None]
        out["paint_end_rate"] = len(ends) / len(out["iterations"])
        completes = [i for i in ends if i.get("settle_complete")]
        out["settle_complete_rate"] = (
            len(completes) / len(ends) if ends else None)
        return summarize(out, key="activation_ms")

    def action_group(self, iters: int) -> dict:
        rows_ready = self.ensure_two_scratch_rows()
        if len(rows_ready) < 2:
            return {"error": "could not establish two scratch rows", "iterations": []}
        a_path, b_path = rows_ready[0], rows_ready[1]
        out = {"iterations": []}
        for i in range(iters):
            t0 = now_ms()
            ri = self.verb("row-set", a_path, "--into", b_path)
            nested, settle_ms = self.wait_depth(a_path, expected_child_of=b_path)
            into_ms = now_ms() - t0
            acc = []
            if not ri["ok"]:
                acc.append(f"row-set --into failed: {ri.get('error')}")
            if not nested:
                acc.append("A did not become a child of B within timeout")
            # undo immediately — sticky verbs must not leak state
            ro = self.verb("row-set", a_path, "--out")
            unnested, out_settle_ms = self.wait_depth(a_path, expected_child_of=None)
            if not ro["ok"] or not unnested:
                acc.append("row-set --out did not restore the flat order")
            out["iterations"].append({
                "into_commit_ms": into_ms, "into_settle_ms": settle_ms,
                "out_settle_ms": out_settle_ms,
                "into_cli_ms": ri["wall_ms"],
                "accuracy_failures": acc,
            })
        return summarize(out, key="into_commit_ms")

    def action_menu(self, iters: int) -> dict:
        # The terminal viewport menu lives on the xterm surface, and the
        # surface mounts ONLY on the active view — a --no-activate scratch
        # row has no host in the webview registry at all (measured live
        # 2026-09-26: terminal_host_missing 3/3), so the probe must land on
        # an ACTIVATED row — queue item 7's mounted-surface discipline.
        rows_ready = self.ensure_scratch_rows(1, activate=True)
        if not rows_ready:
            return {"error": "no scratch row to probe", "iterations": []}
        path = rows_ready[0]
        out = {"iterations": []}
        for i in range(iters):
            self.verb("terminal", "focus", path)
            time.sleep(0.2)
            t0 = now_ms()
            r = self.verb("terminal", "probe-context-menu", path)
            menu_ms = now_ms() - t0
            reply = r.get("json") or {}
            data = reply.get("data") or {}
            # The terminal probe's accuracy payload is the menu's own action
            # list (`actions`/`action_names`/`has_terminal_action`) — `items`
            # never existed on this reply, so every "pass" here asserted
            # against an empty set and proved nothing ([11.113] gap 2 lane).
            items = data.get("actions") or []
            action_names = data.get("action_names") or []
            acc = []
            if not r["ok"]:
                acc.append(f"probe-context-menu failed: {r.get('error')}")
            elif not data.get("accepted"):
                acc.append("menu refused: reason=%s host_wait_ms=%s"
                           % (data.get("reason"), data.get("host_wait_ms")))
            else:
                if not data.get("has_terminal_action"):
                    acc.append("terminal action absent from menu: %s"
                               % action_names)
                if not data.get("no_paste_side_effect"):
                    acc.append("right-click leaked a paste side effect")
                if not data.get("menu_rect"):
                    acc.append("menu_rect missing (menu not visibly placed)")
                if data.get("menu_wait_ms") is None:
                    acc.append("menu never observed in DOM (menu_wait_ms null)")
            events = sorted({n for ev in self.ytrace_events(t0)
                             if isinstance((n := ev.get("name")), str)
                             and "menu" in n.lower()})
            out["iterations"].append({
                "menu_open_ms": menu_ms,
                "cli_ms": r["wall_ms"],
                # input→DOM truth from inside the webview ([11.113] gap 2)
                "menu_wait_ms": data.get("menu_wait_ms"),
                "host_wait_ms": data.get("host_wait_ms"),
                "item_count": len(items) if items else None,
                "action_names": action_names or None,
                "menu_rect": data.get("menu_rect"),
                "has_terminal_action": data.get("has_terminal_action"),
                "no_paste_side_effect": data.get("no_paste_side_effect"),
                "accuracy_failures": acc,
                "trace_events_in_window": events or None,
            })
        return summarize(out, key="menu_open_ms")

    def action_modal(self, iters: int) -> dict:
        """Open the delete-confirm dialog on ONE scratch row through the real
        context-menu path, time it, assert it, cancel it. The dialog is
        never confirmed — the Cancel button is part of the action under
        test, and the row must survive it (that IS an accuracy assertion:
        a modal that eats the row on cancel is wrong)."""
        rows_ready = self.ensure_scratch_rows(1, activate=True)
        if not rows_ready:
            return {"error": "no scratch row to probe", "iterations": []}
        path = rows_ready[0]
        title = next((r["title"] for r in self.scratch_titles
                      if r["path"] == path), "")
        out = {"iterations": []}
        for i in range(iters):
            # the sidebar virtualizes; selecting renders + scrolls to the
            # row's node so the right-click has a target
            self.verb("tree", "select", path)
            self.verb("terminal", "focus", path)
            time.sleep(0.2)
            t0 = now_ms()
            script = MODAL_OPEN_CLICK_JS.format(session=path)
            r = self.verb("dom-eval", script, timeout=15)
            open_wall_ms = now_ms() - t0
            reply = r.get("json") or {}
            result = (reply.get("data") or {}).get("result") or {}
            acc = []
            if not r["ok"]:
                acc.append(f"dom-eval failed: {r.get('error')}")
            if result.get("dom_eval_error"):
                acc.append(f"script error: {result['dom_eval_error']}")
            if not result.get("accepted"):
                acc.append(f"modal open refused: {result.get('reason')}")
            if result.get("accepted"):
                if result.get("overlay_count") != 1:
                    acc.append("overlay_count=%s (want exactly 1)"
                               % result.get("overlay_count"))
                if title and title not in (result.get("dialog_text") or ""):
                    acc.append("scratch row title absent from dialog text "
                               "(wrong row in the modal?)")
            # the app's own account: the request→shown pair from ytrace
            pair = self.modal_pair_from_trace(t0)
            if result.get("accepted"):
                if pair.get("pair_ms") is None:
                    acc.append("no modal_open_requested → modal/shown pair "
                               "in ytrace within window")
                else:
                    rows_n = pair.get("requested_rows")
                    if rows_n not in (1, None):
                        acc.append("modal_open_requested rows=%s (want 1)"
                                   % rows_n)
                    if pair.get("shown_kind") not in ("delete", None):
                        acc.append("modal/shown kind=%s (want delete)"
                                   % pair.get("shown_kind"))
            # cancel — and the row must survive it
            cancel = {}
            if result.get("accepted"):
                rc = self.verb("dom-eval", MODAL_CANCEL_JS, timeout=10)
                cancel = ((rc.get("json") or {}).get("data")
                          or {}).get("result") or {}
                if not cancel.get("accepted"):
                    acc.append(f"cancel failed: {cancel.get('reason')}")
                else:
                    time.sleep(0.15)
                    if self.find_row(path) is None:
                        acc.append("SCRATCH ROW GONE AFTER CANCEL — the "
                                   "modal path deleted the row")
            out["iterations"].append({
                "open_wall_ms": open_wall_ms,
                "menu_open_ms": result.get("menu_open_ms"),
                "click_to_mount_ms": result.get("click_to_mount_ms"),
                "dispatch_to_mount_ms": result.get("dispatch_to_mount_ms"),
                "pair_ms": pair.get("pair_ms"),
                "requested_rows": pair.get("requested_rows"),
                "shown_kind": pair.get("shown_kind"),
                "cancel_to_gone_ms": cancel.get("cancel_to_gone_ms"),
                "dialog_text_head": (result.get("dialog_text") or "")[:160],
                "accuracy_failures": acc,
            })
        return summarize(out, key="dispatch_to_mount_ms")

    def modal_pair_from_trace(self, since_ms: int,
                              timeout_s: float = 6.0) -> dict:
        """modal_open_requested (ui_telemetry intent) and modal/shown
        (category modal, name shown, kind delete — the DeleteConfirmOverlay
        mount effect). Both ts_ms are the app's own clock; the pair delta is
        the in-app request→paint latency."""
        requested = shown = None
        deadline = time.time() + timeout_s
        while time.time() < deadline and not (requested and shown):
            for e in self.ytrace_events(since_ms):
                name = e.get("name")
                if name == "modal_open_requested" and requested is None:
                    requested = e
                elif (name == "shown" and e.get("category") == "modal"
                        and shown is None):
                    shown = e
            if not (requested and shown):
                time.sleep(0.25)
        payload = (requested or {}).get("payload") or {}
        # ui_telemetry payloads NEST in ytrace (payload.payload — the
        # closeall pair helper learned this first); without the unwrap the
        # rows-assert reads None and passes vacuously
        if isinstance(payload.get("payload"), dict):
            payload = payload["payload"]
        shown_payload = (shown or {}).get("payload") or {}
        pair_ms = None
        if requested and shown:
            pair_ms = shown["ts_ms"] - requested["ts_ms"]
        return {"requested_ts_ms": (requested or {}).get("ts_ms"),
                "shown_ts_ms": (shown or {}).get("ts_ms"),
                "pair_ms": pair_ms,
                "requested_rows": payload.get("rows"),
                "requested_bulk": payload.get("bulk"),
                "requested_kind": payload.get("kind"),
                "shown_kind": shown_payload.get("kind")}

    def action_close(self) -> dict:
        """The teardown, instrumented — every spawned row closed exactly once."""
        out = {"iterations": []}
        for path in list(self.spawned_paths):
            row = self.find_row(path)
            t0 = now_ms()
            ok = self.remove_row(path)
            out["iterations"].append({
                "close_to_gone_ms": now_ms() - t0,
                "verified": ok,
                "accuracy_failures": [] if ok else ["row did not leave the live order"],
            })
        return summarize(out, key="close_to_gone_ms")

    def chord_events_from_trace(self, since_ms: int,
                                timeout_s: float = 5.0) -> dict:
        """The ui/chord identity events ([11.113] gap 3 instrument): every
        bridge chord message since since_ms with its face + key. On a build
        without the instrument this returns chord_events=False HONESTLY —
        the falsifier rerun happens after the GUI rotates onto the
        instrument build, not before."""
        deadline = time.time() + timeout_s
        events = []
        while time.time() < deadline:
            events = [e for e in self.ytrace_events(since_ms)
                      if e.get("category") == "ui" and e.get("name") == "chord"]
            if events:
                break
            time.sleep(0.25)
        faces = [{"face": (e.get("payload") or {}).get("face"),
                  "key": (e.get("payload") or {}).get("key"),
                  "ts_ms": e.get("ts_ms")}
                 for e in events]
        return {"chord_events": bool(events), "faces": faces}

    def live_session_paths(self) -> list[str]:
        """Sidebar rows of kind Session — the close-all blast radius."""
        return [r.get("full_path") for r in self.rows()
                if r.get("kind") == "Session" and r.get("full_path")]

    def action_closeall(self, iters: int) -> dict:
        """The bulk-close vertical. CANCEL legs measure + assert; the CONFIRM
        leg runs once, ONLY when every live session on screen is a probe
        scratch row (close-all closes ALL live sessions — blast-radius law)."""
        out = {"iterations": []}
        scratch = self.ensure_scratch_rows(2, activate=True)
        if not scratch:
            return {"error": "no scratch rows to protect", "iterations": []}
        scratch_set = set(scratch)
        pre = [p for p in self.live_session_paths() if p not in scratch_set]
        out["pre_existing_live_sessions"] = len(pre)
        confirm_allowed = not pre
        out["confirm_leg"] = ("run" if confirm_allowed else
                              "refused: pre_existing_live_sessions>0 "
                              "(close-all closes ALL live sessions; "
                              "blast-radius law)")
        for i in range(iters):
            acc = []
            self.verb("tree", "select", "__live_sessions__")
            time.sleep(0.15)
            t0 = now_ms()
            r = self.verb("dom-eval", CLOSEALL_OPEN_JS, timeout=20)
            open_wall_ms = now_ms() - t0
            result = ((r.get("json") or {}).get("data") or {}).get("result") or {}
            if not r["ok"]:
                acc.append(f"dom-eval failed: {r.get('error')}")
            if result.get("dom_eval_error"):
                acc.append(f"script error: {result['dom_eval_error']}")
            if not result.get("accepted"):
                acc.append(f"close-all open refused: {result.get('reason')}")
            if result.get("accepted"):
                if result.get("overlay_count") != 1:
                    acc.append("overlay_count=%s (want exactly 1)"
                               % result.get("overlay_count"))
                if result.get("dialog_title") != "Close Live Sessions?":
                    acc.append("dialog_title=%r (want Close Live Sessions?)"
                               % result.get("dialog_title"))
                if result.get("action_label") != "Close All Sessions":
                    acc.append("action_label=%r (want Close All Sessions)"
                               % result.get("action_label"))
            pair = self.modal_pair_from_trace(t0)
            if result.get("accepted"):
                if pair.get("pair_ms") is None:
                    acc.append("no modal_open_requested → modal/shown pair "
                               "in ytrace within window")
                else:
                    if pair.get("requested_kind") not in ("delete", None):
                        acc.append("requested kind=%s (want delete)"
                                   % pair.get("requested_kind"))
                    if pair.get("requested_bulk") is not True:
                        acc.append("requested bulk=%s (want true — this is "
                                   "the bulk close path)"
                                   % pair.get("requested_bulk"))
                    if pair.get("shown_kind") not in ("delete", None):
                        acc.append("modal/shown kind=%s (want delete)"
                                   % pair.get("shown_kind"))
            # CANCEL — and EVERY live session must survive it
            cancel = {}
            if result.get("accepted"):
                rc = self.verb("dom-eval", MODAL_CANCEL_JS, timeout=10)
                cancel = ((rc.get("json") or {}).get("data")
                          or {}).get("result") or {}
                if not cancel.get("accepted"):
                    acc.append(f"cancel failed: {cancel.get('reason')}")
                else:
                    time.sleep(0.2)
                    live = set(self.live_session_paths())
                    lost_pre = [p for p in pre if p not in live]
                    lost_scratch = [p for p in scratch if p not in live]
                    if lost_pre:
                        acc.append("PRE-EXISTING SESSIONS CLOSED BY A "
                                   "CANCELLED CLOSE-ALL: %s" % lost_pre[:5])
                    if lost_scratch:
                        acc.append("scratch rows gone after cancel: %s"
                                   % lost_scratch)
            # chord leg — the instrument's live half (non-destructive)
            chord = {}
            if result.get("accepted"):
                self.verb("tree", "select", scratch[0])
                time.sleep(0.15)
                tc = now_ms()
                rc = self.verb("dom-eval", CHORD_LEG_JS, timeout=15)
                chord = ((rc.get("json") or {}).get("data")
                         or {}).get("result") or {}
                if not r["ok"]:
                    acc.append(f"chord dom-eval failed: {rc.get('error')}")
                if not chord.get("accepted"):
                    acc.append(f"chord leg refused: {chord.get('reason')} "
                               "(walk letters may differ — honest refusal)")
                else:
                    if not chord.get("menu_seen"):
                        acc.append("chord walk did not open the row menu "
                                   "(badge letter drift?)")
                    if not chord.get("overlay_closed"):
                        acc.append("alt overlay left open after Escape")
                cev = self.chord_events_from_trace(tc)
                chord["trace"] = cev
                if not cev.get("chord_events"):
                    acc.append("NO ui/chord events in ytrace — build not "
                               "rotated onto the chord instrument yet")
            out["iterations"].append({
                "open_wall_ms": open_wall_ms,
                "menu_open_ms": result.get("menu_open_ms"),
                "click_to_mount_ms": result.get("click_to_mount_ms"),
                "dispatch_to_mount_ms": result.get("dispatch_to_mount_ms"),
                "pair_ms": pair.get("pair_ms"),
                "requested_rows": pair.get("requested_rows"),
                "requested_bulk": pair.get("requested_bulk"),
                "shown_kind": pair.get("shown_kind"),
                "dialog_title": result.get("dialog_title"),
                "unkept_button_present": result.get("unkept_button_present"),
                "cancel_to_gone_ms": cancel.get("cancel_to_gone_ms"),
                "chord_tap_to_overlay_ms": chord.get("tap_to_overlay_ms"),
                "chord_walk_to_menu_ms": chord.get("walk_to_menu_ms"),
                "chord_faces": [f.get("face") for f in
                                (chord.get("trace") or {}).get("faces", [])],
                "accuracy_failures": acc,
            })
        # THE CONFIRM LEG — only on a screen whose live sessions are all ours
        confirm_result = {"ran": False}
        if confirm_allowed:
            confirm_result = {"ran": True}
            self.verb("tree", "select", "__live_sessions__")
            time.sleep(0.15)
            r = self.verb("dom-eval", CLOSEALL_OPEN_JS, timeout=20)
            result = ((r.get("json") or {}).get("data")
                      or {}).get("result") or {}
            if not result.get("accepted"):
                confirm_result.update({"closed": False,
                                       "error": result.get("reason")})
            else:
                rc = self.verb("dom-eval", CLOSEALL_CONFIRM_JS, timeout=20)
                confirm = ((rc.get("json") or {}).get("data")
                           or {}).get("result") or {}
                confirm_result.update(confirm)
                if confirm.get("accepted"):
                    gone_by = time.time() + 20
                    still = list(scratch)
                    while time.time() < gone_by and still:
                        live = set(self.live_session_paths())
                        still = [p for p in still if p in live]
                        if still:
                            time.sleep(0.15)
                    confirm_result["rows_gone"] = not still
                    confirm_result["survivors"] = still
                    if not still:
                        # verified gone — teardown has nothing to close
                        self.spawned_paths = [p for p in self.spawned_paths
                                              if p not in scratch_set]
                    else:
                        confirm_result["accuracy_failures"] = [
                            "scratch rows survived close-all: %s" % still]
                else:
                    confirm_result["accuracy_failures"] = [
                        f"confirm failed: {confirm.get('reason')}"]
        out["confirm_leg_result"] = confirm_result
        return summarize(out, key="dispatch_to_mount_ms")

    def split_events_from_trace(self, since_ms: int) -> dict:
        """The split pair: context_menu_activate{action:split-*} -> split/create
        (joined by the window, both one-shot events; group_id in the payload is
        the identity assert). Refusal events are surfaced, never swallowed."""
        out = {"activate": None, "create": None, "create_refused": None,
               "ungrouped": None, "ungroup_refused": None}
        for ev in self.ytrace_events(since_ms):
            name = ev.get("name")
            payload = self._ui_payload(ev)
            if name == "context_menu_activate" \
                    and str(payload.get("action", "")).startswith("split-"):
                out["activate"] = {"ts": ev.get("ts_ms"),
                                   "action": payload.get("action")}
            elif name == "split/create":
                out["create"] = {"ts": ev.get("ts_ms"),
                                 "group_id": payload.get("group_id"),
                                 "axis": payload.get("axis"),
                                 "origin": payload.get("origin"),
                                 "members": payload.get("members")}
            elif name == "split/create_refused":
                out["create_refused"] = {"reason": payload.get("reason")}
            elif name == "split/ungrouped":
                out["ungrouped"] = {"ts": ev.get("ts_ms"),
                                    "group_id": payload.get("group_id")}
            elif name == "split/ungroup_refused":
                out["ungroup_refused"] = {"reason": payload.get("reason")}
        if out["activate"] and out["create"]:
            out["pair_ms"] = out["create"]["ts"] - out["activate"]["ts"]
        return out

    def action_split(self, iters: int) -> dict:
        """The split vertical: right-click a scratch row's REAL sidebar menu ->
        Split side by side -> commit + DOM truth. Accuracy = the pane set is
        EXACTLY the two scratch rows, both pane rects on screen, and ungroup
        restores the rows without closing anyone (blast-radius law)."""
        out = {"iterations": []}
        scratch = self.ensure_two_scratch_rows()
        if len(scratch) < 2:
            return {"error": "need two scratch rows", "iterations": []}
        a_path, b_path = scratch[0], scratch[1]
        for i in range(iters):
            acc = []
            # both rows selected: the menu's split candidates are the
            # right-clicked row + the selection (split_candidate_paths_for)
            self.verb("tree", "select", a_path, b_path, "--anchor", a_path)
            time.sleep(0.15)
            t0 = now_ms()
            js = SPLIT_OPEN_JS.replace("{session_a!r}", repr(a_path))
            js = js.replace("{axis!r}", repr("split-side-by-side"))
            data = self.dom_eval(js, timeout=25)
            open_wall_ms = now_ms() - t0
            result = data.get("result") or {}
            if not result and data.get("error") is not None:
                acc.append(f"dom-eval error: {data['error']}")
            if result.get("dom_eval_error"):
                acc.append(f"script error: {result['dom_eval_error']}")
            if result and not result.get("accepted"):
                acc.append(f"split refused: {result.get('reason')}")
            # PHASE 2 — the DOM truth read rides its own eval so the 2.5s
            # wait can never eat phase 1's reply under the GUI's 3s eval
            # budget ([11.200]). committed = the menu actually opened and
            # the split item was clicked.
            committed = bool(result.get("accepted"))
            dom = {}
            if committed:
                t_dom0 = now_ms()
                ddata = self.dom_eval(SPLIT_DOM_JS, timeout=15)
                dom = ddata.get("result") or {}
                if not dom and ddata.get("error") is not None:
                    acc.append(f"dom-truth eval error: {ddata['error']}")
                elif dom.get("compound_found"):
                    result["pane_count"] = dom.get("pane_count")
                    result["panes"] = dom.get("panes") or []
                    result["compound_label"] = dom.get("compound_label")
                    t_click = result.get("t_click")
                    found_at = dom.get("found_at")
                    if t_click and found_at:
                        result["click_to_dom_ms"] = found_at - t_click
                else:
                    acc.append("split_dom_truth_did_not_land "
                               f"(waited {dom.get('waited_ms')} ms)")
            pair = self.split_events_from_trace(t0)
            if committed:
                if pair.get("pair_ms") is None:
                    acc.append("no context_menu_activate -> split/create pair "
                               "in ytrace within window (build not rotated "
                               "onto the split instrument, or refusal)")
                create = pair.get("create") or {}
                if create.get("origin") not in ("menu", None):
                    acc.append("split/create origin=%s (want menu)"
                               % create.get("origin"))
                if create.get("axis") not in ("side by side", None):
                    acc.append("split/create axis=%s (want side by side)"
                               % create.get("axis"))
                members = [m.get("session") for m in
                           (create.get("members") or [])]
                if members and set(members) != {a_path, b_path}:
                    acc.append("split/create members=%s (want exactly the "
                               "two scratch rows)" % members)
                panes = result.get("panes") or []
                pane_sessions = {p.get("session") for p in panes}
                if panes and not {a_path, b_path} <= pane_sessions:
                    acc.append("pane rects missing a scratch session: %s"
                               % pane_sessions)
                rects = [p.get("rect") for p in panes]
                if any(not r2 or r2["w"] <= 0 or r2["h"] <= 0 for r2 in rects):
                    acc.append("a pane rect is empty (split not visible)")
            # UNGROUP — the teardown must restore both rows alive
            ungroup = {}
            if committed and dom.get("compound_found"):
                time.sleep(0.3)
                udata = self.dom_eval(SPLIT_UNGROUP_JS, timeout=15)
                ungroup = udata.get("result") or {}
                if not ungroup.get("accepted"):
                    why = (f"ungroup dom-eval error: {udata['error']}"
                           if udata.get("error") is not None and not ungroup
                           else f"ungroup failed: {ungroup.get('reason')} "
                                f"(still_compound={ungroup.get('still_compound')})")
                    acc.append(why)
                else:
                    pair2 = self.split_events_from_trace(t0)
                    if not pair2.get("ungrouped"):
                        acc.append("no split/ungrouped event in ytrace")
                time.sleep(0.3)
                live = set(self.live_session_paths())
                lost = [p for p in (a_path, b_path) if p not in live]
                if lost:
                    acc.append("UNGROUP CLOSED SCRATCH ROWS: %s" % lost)
            out["iterations"].append({
                "open_wall_ms": open_wall_ms,
                "menu_open_ms": result.get("menu_open_ms"),
                "click_to_dom_ms": result.get("click_to_dom_ms"),
                "dispatch_to_dom_ms": result.get("dispatch_to_dom_ms"),
                "pair_ms": pair.get("pair_ms"),
                "create_origin": (pair.get("create") or {}).get("origin"),
                "create_axis": (pair.get("create") or {}).get("axis"),
                "member_count": len((pair.get("create") or {})
                                    .get("members") or []),
                "pane_count": result.get("pane_count"),
                "compound_label": result.get("compound_label"),
                "ungroup_to_gone_ms": ungroup.get("ungroup_to_gone_ms"),
                "accuracy_failures": acc,
            })
        return summarize(out, key="click_to_dom_ms")

    # ---- waiters --------------------------------------------------------

    def wait_order(self, a_path: str, b_path: str,
                   timeout_s: float = 5.0) -> tuple[bool, int | None]:
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < timeout_s:
            idx = self.rows_order_indices((a_path, b_path))
            ia, ib = idx[a_path], idx[b_path]
            if ia is not None and ib is not None and ib - ia == 1:
                return True, int((time.perf_counter() - t0) * 1000)
            time.sleep(0.3)
        return False, None

    def wait_depth(self, a_path: str, expected_child_of: str | None,
                   timeout_s: float = 5.0) -> tuple[bool, int | None]:
        """Relative-depth wait: scratch rows live INSIDE a group already, so
        nesting is A.depth == B.depth + 1, never an absolute depth."""
        b = self.find_row(expected_child_of) if expected_child_of else None
        b_depth = b.get("depth", 0) if b else 0
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < timeout_s:
            a = self.find_row(a_path)
            if expected_child_of:
                if a and a.get("depth") == b_depth + 1:
                    return True, int((time.perf_counter() - t0) * 1000)
            elif a and a.get("depth") == b_depth:
                return True, int((time.perf_counter() - t0) * 1000)
            time.sleep(0.3)
        return False, None

    def ensure_scratch_rows(self, n: int, activate: bool = False) -> list[str]:
        live = [p for p in self.spawned_paths if self.find_row(p) is not None]
        while len(live) < n:
            row = self.spawn_scratch_row(len(live), activate=activate)
            if "error" in row:
                break
            live = [p for p in self.spawned_paths
                    if self.find_row(p) is not None]
        return live[:n]

    def ensure_two_scratch_rows(self) -> list[str]:
        return self.ensure_scratch_rows(2)


def summarize(out: dict, key: str, rate_key: str | None = None) -> dict:
    iters = out.get("iterations") or []
    vals = [i[key] for i in iters
            if isinstance(i.get(key), (int, float))]
    out["n"] = len(iters)
    out[f"{key}_p50"] = statistics.median(vals) if vals else None
    out[f"{key}_max"] = max(vals) if vals else None
    if rate_key:
        nums = [i.get(rate_key) for i in iters]
        out[f"{rate_key}_positive_rate"] = (
            sum(1 for v in nums if isinstance(v, (int, float)) and v > 0)
            / len(nums)) if nums else None
    fails = [f for i in iters for f in i.get("accuracy_failures", [])]
    out["accuracy_failures"] = fails
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--actions",
                    default="spawn,drag,group,menu,modal,close,felt,shiftdrag,closeall,switch,split")
    ap.add_argument("--iters", type=int, default=3)
    ap.add_argument("--out", default="/tmp/uxspeed-report.json")
    ap.add_argument("--artifacts", default="/tmp/uxspeed-artifacts")
    ap.add_argument("--timeout-ms", type=int, default=12000)
    ap.add_argument("--keep", action="store_true",
                    help="do not close the scratch rows (debugging)")
    ap.add_argument("-v", action="store_true")
    args = ap.parse_args()

    probe = Probe(args)
    actions = [a.strip() for a in args.actions.split(",") if a.strip()]
    report: dict = {"schema": "uxspeed-uxprobe-v1",
                    "started": now_ms(), "plane": "live-gui",
                    "scratch_title_prefix": PROBE_TITLE_PREFIX,
                    "actions": {}}

    stale = probe.clean_stale()
    if stale:
        log(f"removed {stale} stale scratch row(s) from an earlier run")

    report["cli_overhead_ms"] = probe.ping_overhead_ms()
    report["cli_overhead_end_ms"] = None
    log(f"cli overhead floor: {report['cli_overhead_ms']} ms")
    report["gui_overhead_ms"] = probe.gui_overhead_ms()
    report["gui_overhead_end_ms"] = None
    log(f"gui overhead floor: {report['gui_overhead_ms']} ms")

    try:
        for action in actions:
            log(f"action {action} …")
            if action == "spawn":
                report["actions"]["spawn"] = probe.action_spawn(args.iters)
            elif action == "drag":
                report["actions"]["drag"] = probe.action_drag(args.iters)
            elif action == "mergescale":
                report["actions"]["mergescale"] = probe.action_mergescale(args.iters)
            elif action == "felt":
                report["actions"]["felt"] = probe.action_felt(args.iters)
            elif action == "shiftdrag":
                report["actions"]["shiftdrag"] = probe.action_shiftdrag(args.iters)
            elif action == "switch":
                report["actions"]["switch"] = probe.action_switch(args.iters)
            elif action == "group":
                report["actions"]["group"] = probe.action_group(args.iters)
            elif action == "menu":
                report["actions"]["menu"] = probe.action_menu(args.iters)
            elif action == "modal":
                report["actions"]["modal"] = probe.action_modal(args.iters)
            elif action == "close":
                report["actions"]["close"] = probe.action_close()
            elif action == "closeall":
                report["actions"]["closeall"] = probe.action_closeall(args.iters)
            elif action == "split":
                report["actions"]["split"] = probe.action_split(args.iters)
            else:
                report["actions"][action] = {"error": f"unknown action {action}"}
            log(f"  {json.dumps(report['actions'][action], default=str)[:300]}")
    finally:
        if not args.keep:
            log("teardown: closing scratch rows …")
            teardown = probe.action_close()
            if "close" not in report["actions"]:
                report["actions"]["close"] = teardown
        report["ended"] = now_ms()
        report["rows_left_behind"] = len(probe.spawned_paths)
        end_floor = probe.ping_overhead_ms(samples=3)
        report["cli_overhead_end_ms"] = end_floor
        end_gui = probe.gui_overhead_ms()
        report["gui_overhead_end_ms"] = end_gui
        if end_floor and report["cli_overhead_ms"]:
            drift = end_floor - report["cli_overhead_ms"]
            report["harness_load_drift_ms"] = drift
            log(f"overhead drift {drift:+d} ms (the probe's own pollution proxy)")
        if end_gui and report["gui_overhead_ms"]:
            gdrift = end_gui - report["gui_overhead_ms"]
            report["gui_load_drift_ms"] = gdrift
            log(f"gui overhead drift {gdrift:+d} ms (GUI-plane pollution proxy)")
        with open(args.out, "w") as fh:
            json.dump(report, fh, indent=1, default=str)
        log(f"report → {args.out}")
    return 0


def log(msg: str) -> None:
    print(f"[uxprobe] {msg}", file=sys.stderr, flush=True)


if __name__ == "__main__":
    sys.exit(main())
