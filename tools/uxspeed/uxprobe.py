#!/usr/bin/env python3
"""uxprobe — the UX-SPEED campaign's synthetic timing + accuracy harness.

Times the campaign's UX actions verb→effect using the server app verb
family (the GUI's own sanctioned automation surface) and ASSERTS accuracy,
on self-created scratch rows only. Blast-radius law: rows not created by
this run are never mutated; teardown removes exactly what the run spawned.

Actions:
  spawn  — server app terminal new → wait the paint ladder in ytrace
         (first_frame | settle | mount_open — first_frame is write-scoped and
         does not fire on a mount that never receives a byte)
  drag   — server app drag begin/hover/drop reorder of two scratch rows
  felt   — the REAL input-plane drag: pointer down on a scratch row, dwell
           under the 6px threshold, cross it, hover across rows, release on
           the target (server app pointer + dom-eval rects); asserts the
           felt-path falsifiers from the trace (no merges pre-begin/in-drag),
           the reorder, and records app-render counts for [11.129]
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
    Object.assign({{ accepted: false, reason }}, extra || {{}}));
if (document.querySelector('[data-delete-confirm-overlay]')) {{
    refuse("delete_overlay_already_open");
    return;
}}
const PATH = '__live_sessions__';
const row = await (async () => {{
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
    refuse("live_sessions_group_row_missing");
    return;
}}
const rect = row.getBoundingClientRect();
if (!(rect.width > 0 && rect.height > 0)) {{
    refuse("live_sessions_group_row_not_visible");
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
let menu = null, closeAll = null;
const openDeadline = Date.now() + 1500;
while (Date.now() < openDeadline) {{
    await settle(40);
    menu = document.querySelector('[data-context-menu="1"]');
    closeAll = menu?.querySelector(
        '[data-context-menu-action="close-all-live-sessions"]') || null;
    if (menu && closeAll) break;
}}
if (!menu || !closeAll) {{
    refuse("menu_or_close_all_item_not_observed", {{ session_path: PATH }});
    return;
}}
const menu_open_ms = Date.now() - t_open;
await settle(120);
const clickInit = {{ bubbles: true, cancelable: true, composed: true,
                     view: window, button: 0, buttons: 1 }};
closeAll.dispatchEvent(new MouseEvent('mousedown', clickInit));
closeAll.dispatchEvent(new MouseEvent('mouseup',
    {{ ...clickInit, buttons: 0 }}));
closeAll.dispatchEvent(new MouseEvent('click', clickInit));
const t_click = Date.now();
const mountDeadline = t_click + 1500;
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
    dialog_title: String(overlay.querySelector(
        '[data-delete-confirm-title]')?.textContent || ''),
    action_label: String(overlay.querySelector(
        '[data-delete-confirm-action]')?.textContent || ''),
    unkept_button_present: !!overlay.querySelector(
        '[data-delete-confirm-unkept-action]'),
    dialog_text: String(dialog?.textContent || '').slice(0, 400),
}});
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
CHORD_LEG_JS = """
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const refuse = (reason, extra) => dioxus.send(
    Object.assign({{ accepted: false, reason }}, extra || {{}}));
const q = (sel) => !!document.querySelector(sel);
if (q('[data-yggterm-menu-open]') || q('[data-delete-confirm-overlay]')) {{
    refuse("menu_or_overlay_already_open");
    return;
}}
const kd = (key, code) => window.dispatchEvent(new KeyboardEvent('keydown',
    {{ key, code, bubbles: true, cancelable: true, composed: true }}));
const ku = (key, code) => window.dispatchEvent(new KeyboardEvent('keyup',
    {{ key, code, bubbles: true, cancelable: true, composed: true }}));
const t_tap = Date.now();
kd('Alt', 'AltLeft');
ku('Alt', 'AltLeft');
let overlaySeen = false;
const ovDeadline = Date.now() + 1500;
while (Date.now() < ovDeadline) {{
    await settle(40);
    if (q('[data-yggterm-keytip-breadcrumb]')) {{ overlaySeen = true; break; }}
}}
if (!overlaySeen) {{
    refuse("alt_overlay_did_not_open", {{ tap_to_overlay_ms: null }});
    return;
}}
const tap_to_overlay_ms = Date.now() - t_tap;
const t_e = Date.now();
kd('e', 'KeyE');
let menuSeen = false;
const mDeadline = Date.now() + 1500;
while (Date.now() < mDeadline) {{
    await settle(40);
    if (q('[data-yggterm-menu-open]')) {{ menuSeen = true; break; }}
}}
const walk_to_menu_ms = menuSeen ? Date.now() - t_e : null;
kd('Escape', 'Escape');
let menuClosed = false;
const gDeadline = Date.now() + 1500;
while (Date.now() < gDeadline) {{
    await settle(40);
    if (!q('[data-yggterm-menu-open]')) {{ menuClosed = true; break; }}
}}
let overlayClosed = !q('[data-yggterm-keytip-breadcrumb]');
if (!overlayClosed) {{
    kd('Escape', 'Escape');
    const d2 = Date.now() + 1000;
    while (Date.now() < d2) {{
        await settle(40);
        if (!q('[data-yggterm-keytip-breadcrumb]')) {{ overlayClosed = true; break; }}
    }}
}}
dioxus.send({{
    accepted: true,
    overlay_seen: true,
    menu_seen: menuSeen,
    tap_to_overlay_ms,
    walk_to_menu_ms,
    escape_closed_menu: menuClosed,
    overlay_closed: overlayClosed,
}});
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

    # ---- instruments -------------------------------------------------

    def verb(self, *argv: str, timeout: float | None = None) -> dict:
        """Run one `yggterm server app …` verb; return parsed reply + wall ms."""
        cmd = [YGGTERM, "server", "app", *argv]
        t0 = time.perf_counter()
        try:
            proc = subprocess.run(
                cmd, capture_output=True, text=True,
                timeout=timeout or self.timeout_s,
            )
        except subprocess.TimeoutExpired:
            return {"ok": False, "error": "verb timeout", "wall_ms": None,
                    "raw": ""}
        wall_ms = int((time.perf_counter() - t0) * 1000)
        reply: dict = {"ok": proc.returncode == 0, "wall_ms": wall_ms}
        try:
            reply["json"] = json.loads(proc.stdout)
        except (json.JSONDecodeError, ValueError):
            reply["json"] = None
            reply["raw"] = (proc.stdout + proc.stderr)[-2000:]
        if proc.returncode != 0:
            reply["error"] = (proc.stderr or proc.stdout)[-500:]
        self.dump_artifact(f"verb-{'-'.join(argv[:3])}-{now_ms()}.json", reply)
        return reply

    def ping_overhead_ms(self, samples: int = 5) -> float | None:
        """CLI round-trip floor: every verb measurement includes this."""
        walls = []
        for _ in range(samples):
            r = self.verb("clients")
            if r["ok"] and r["wall_ms"] is not None:
                walls.append(r["wall_ms"])
            time.sleep(0.05)
        return statistics.median(walls) if walls else None

    def ytrace_events(self, since_ms: int, lines: int = 400) -> list[dict]:
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
        paint = self.wait_paint_milestones(path, since_ms=t0)
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
            if paint.get("paint_marker") is None:
                acc.append("no paint milestone fired within timeout")
            out["iterations"].append({
                "spawn_to_paint_ms": paint.get("spawn_to_paint_ms"),
                "paint_marker": paint.get("paint_marker"),
                "daemon_processed_ms": row.get("daemon_processed_ms"),
                "queued": row.get("queued"),
                "cli_new_ms": row["cli_ms"],
                "milestone_offsets_ms": {k: v["offset_ms"]
                                         for k, v in paint["milestones"].items()},
                "open_to_write_ms": paint.get("open_to_write_ms"),
                "write_to_frame_ms": paint.get("write_to_frame_ms"),
                "blank_frames_before_write": paint.get("blank_frames_before_write"),
                "accuracy_failures": acc,
            })
        return summarize(out, key="spawn_to_paint_ms",
                         rate_key="blank_frames_before_write")

    def action_drag(self, iters: int) -> dict:
        rows_ready = self.ensure_two_scratch_rows()
        if len(rows_ready) < 2:
            return {"error": "could not establish two scratch rows", "iterations": []}
        a_path, b_path = rows_ready[0], rows_ready[1]
        out = {"iterations": []}
        for i in range(iters):
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
            out["iterations"].append({
                "commit_ms": commit_ms,
                "settle_ms": settle_ms,
                "begin_ms": rb["wall_ms"], "hover_ms": rh["wall_ms"],
                "drop_ms": rd["wall_ms"],
                "accuracy_failures": acc,
                "trace_events_in_window": sorted(e for e in events
                                                 if e and "drag" in e.lower())
                                                or None,
            })
            # drag back so iterations stay independent
            t0 = now_ms()
            self.verb("drag", "begin", a_path)
            self.verb("drag", "hover", b_path, "--placement", "after")
            self.verb("drag", "drop")
            self.wait_order(b_path, a_path)
        return summarize(out, key="commit_ms")

    def _row_rects(self, paths: list[str]) -> dict:
        """Sidebar DOM rects for row paths, via one dom-eval. The sidebar
        virtualizes — the caller must `tree select` first so nodes exist."""
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
        r = self.verb("dom-eval", js, timeout=15)
        reply = r.get("json") or {}
        return (reply.get("data") or {}).get("result") or {}

    def _events_between(self, t0: int, t1: int) -> list[dict]:
        return [e for e in self.ytrace_events(t0) if e.get("ts_ms", 0) <= t1]

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
            uy = rb["y"] + rb["h"] * 0.75
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
            flipped, flip_ms = self.wait_order(b_path, a_path)
            it["felt_ms"] = now_ms() - t_down
            it["reorder_settle_ms"] = flip_ms
            evs = self._events_between(t_down, now_ms())
            names = [e.get("name") for e in evs]
            begins = [e["ts_ms"] for e in evs
                      if e.get("name") == "tree_drag_begin"]
            it["drag_events"] = sorted({n for n in names
                                        if n and "drag" in n.lower()})
            it["merge_events_in_drag"] = sum(
                1 for n in names if n == "merge_rows_breakdown")
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
        latency + identity (to == the clicked row), the reveal outcome
        (reveal_ready with its self-timed first_output_ms, or the honest
        incomplete/failed marker — reveal events are joined by time-proximity
        inside the window, the door's known pairing gap), and the ambient
        churn inside the window (ui/block, merge_rows_breakdown) — the felt
        cost line. Walls are overhead-inclusive (release verb wall included);
        the report's cli_overhead_ms is the floor to subtract."""
        rows_ready = self.ensure_two_scratch_rows()
        if len(rows_ready) < 2:
            return {"error": "could not establish two scratch rows",
                    "iterations": []}
        a_path, b_path = rows_ready
        # make both nodes exist under the sidebar's virtualization, as felt does
        self.verb("tree", "select", a_path)
        self.verb("tree", "select", b_path)
        time.sleep(0.2)
        rects = self._row_rects([a_path, b_path])
        if not rects.get(a_path) or not rects.get(b_path):
            return {"error": "row nodes not rendered (virtualized out?)",
                    "iterations": []}
        out = {"iterations": []}
        targets = [a_path, b_path] * iters
        for i, path in enumerate(targets):
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
            reveal = None
            deadline = time.time() + self.timeout_s
            while time.time() < deadline:
                evs = self.ytrace_events(t_release, lines=900)
                if activation is None:
                    for e in evs:
                        p = e.get("payload") or {}
                        if (e.get("name") == "activation"
                                and e.get("category") == "session"
                                and p.get("origin") == "user_gesture"
                                and str(p.get("to") or "").startswith(path)):
                            activation = e
                            break
                if activation is not None and reveal is None:
                    for e in evs:
                        if e.get("category") == "reveal" and e.get(
                                "name") in ("reveal_ready",
                                            "reveal_forced_incomplete",
                                            "reveal_failed"):
                            reveal = e
                            break
                if activation is not None and (reveal is not None
                                               or time.time() > deadline - (
                                                   self.timeout_s - 3)):
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
            if activation is not None and reveal is None:
                acc.append("no reveal_ready/incomplete/failed within the "
                           "window — the reveal leg never reported (the "
                           "door's pairing gap, honest null)")
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
            # give the previous switch's churn room to drain before the next
            time.sleep(0.6)
        ready = [i for i in out["iterations"]
                 if i.get("reveal_name") == "reveal_ready"]
        out["reveal_ready_rate"] = len(ready) / len(out["iterations"])
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
                    default="spawn,drag,group,menu,modal,close,felt,closeall,switch")
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

    try:
        for action in actions:
            log(f"action {action} …")
            if action == "spawn":
                report["actions"]["spawn"] = probe.action_spawn(args.iters)
            elif action == "drag":
                report["actions"]["drag"] = probe.action_drag(args.iters)
            elif action == "felt":
                report["actions"]["felt"] = probe.action_felt(args.iters)
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
        if end_floor and report["cli_overhead_ms"]:
            drift = end_floor - report["cli_overhead_ms"]
            report["harness_load_drift_ms"] = drift
            log(f"overhead drift {drift:+d} ms (the probe's own pollution proxy)")
        with open(args.out, "w") as fh:
            json.dump(report, fh, indent=1, default=str)
        log(f"report → {args.out}")
    return 0


def log(msg: str) -> None:
    print(f"[uxprobe] {msg}", file=sys.stderr, flush=True)


if __name__ == "__main__":
    sys.exit(main())
