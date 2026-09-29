#!/usr/bin/env python3
"""split-create-appear lane instrument (2026-09-29, ACK-aa075a6276).

One dom-eval does the WHOLE create gesture — right-click → menu → split-item
click → poll DOM milestones on the page clock — so the felt appear carries
NO cross-eval transport (the probe's two-phase click_to_dom includes one
full eval round trip + a 25 ms poll; the ungroup direction never paid that,
which is exactly the asymmetry this lane is attributing). Immediately after,
the ytrace tail names the legs inside [t_click-150, ...].

Run on the GUI host (jojo), probe module shipped beside it:
    python3 splitcreate_attr.py /tmp/uxprobe-splitcreate.py
"""
import importlib.util
import json
import sys
import time

ATTR_JS = """
// ONE eval = the whole create gesture + the felt-appear timeline.
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const PATH_A = {session_a!r};
const AXIS = "split-side-by-side";
if (document.querySelector('[data-split-group-row="1"]')) {
    dioxus.send({ accepted: false, reason: "split_group_already_on_screen" });
    return;
}
const t_eval0 = Date.now();
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
let row = document.querySelector(
    '[data-sidebar-row-path="' + PATH_A + '"]');
const rowDeadline = Date.now() + 300;
while (!row && Date.now() < rowDeadline) {
    await settle(30);
    row = document.querySelector('[data-sidebar-row-path="' + PATH_A + '"]');
}
if (!row) {
    dioxus.send({ accepted: false, reason: "sidebar_row_missing" });
    return;
}
const rect = row.getBoundingClientRect();
if (!(rect.width > 0 && rect.height > 0)) {
    dioxus.send({ accepted: false, reason: "sidebar_row_not_visible" });
    return;
}
const cx = Number((rect.left + rect.width / 2).toFixed(2));
const cy = Number((rect.top + rect.height / 2).toFixed(2));
const init = { bubbles: true, cancelable: true, composed: true, view: window,
                clientX: cx, clientY: cy, screenX: cx, screenY: cy,
                button: 2, buttons: 2, detail: 1 };
row.dispatchEvent(new MouseEvent('mousedown', init));
row.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }));
row.dispatchEvent(new MouseEvent('auxclick', { ...init, buttons: 0 }));
row.dispatchEvent(new MouseEvent('contextmenu', init));
let menu = null, splitItem = null;
const openDeadline = Date.now() + 1000;
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
                  reason: "context_menu_or_split_item_not_observed" });
    return;
}
const menu_open_ms = Date.now() - t_eval0;
await settle(100);
const clickInit = { bubbles: true, cancelable: true, composed: true,
                     view: window, button: 0, buttons: 1 };
const t_click = Date.now();
splitItem.dispatchEvent(new MouseEvent('mousedown', clickInit));
splitItem.dispatchEvent(new MouseEvent('mouseup',
    { ...clickInit, buttons: 0 }));
splitItem.dispatchEvent(new MouseEvent('click', clickInit));
// felt-appear milestones, 8 ms cadence, page clock only
const milestones = {};
const paneRect = (n) => n.getBoundingClientRect();
let panes = [];
while (Date.now() - t_click < 1500) {
    await settle(8);
    const now = Date.now() - t_click;
    if (!milestones.compound_row
        && document.querySelector('[data-split-group-row="1"]'))
        milestones.compound_row = now;
    panes = [...document.querySelectorAll(
        '[data-split-session][data-split-pane-index]')];
    if (!milestones.pane_1 && panes.length >= 1) milestones.pane_1 = now;
    if (!milestones.pane_2 && panes.length >= 2) milestones.pane_2 = now;
    if (panes.length >= 2) {
        const rects = panes.map(paneRect);
        if (rects.every((r) => r.width > 0 && r.height > 0)) {
            milestones.rects_nonempty = now;
            break;
        }
    }
}
milestones.eval_tail_ms = Date.now() - t_click;
// paint proxy: the first animation frame AFTER the DOM landed
const t_dom = Date.now();
await new Promise((r) => requestAnimationFrame(r));
await new Promise((r) => requestAnimationFrame(r));
milestones.first_raf_after_dom = Date.now() - t_click;
milestones.raf_lag_ms = Date.now() - t_dom;
dioxus.send({
    accepted: true, menu_open_ms, t_click, milestones,
    pane_count: panes.length,
    pane_sessions: panes.map((n) => n.getAttribute('data-split-session')),
});
"""


def load_probe(path: str):
    spec = importlib.util.spec_from_file_location("uxprobe", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    probe_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/uxprobe-splitcreate.py"
    reps = int(sys.argv[2]) if len(sys.argv) > 2 else 3
    ux = load_probe(probe_path)

    class A:
        actions = "none"; iters = 1; out = "/dev/null"
        artifacts = "/tmp/uxspeed-artifacts"; timeout_ms = 12000
        keep = False; v = False
    p = ux.Probe(A)
    p.clean_stale()
    rows = p.ensure_two_scratch_rows()
    if len(rows) < 2:
        print("FATAL: could not establish two scratch rows")
        return 1
    a_path, b_path = rows[0], rows[1]
    try:
        for rep in range(reps):
            p.verb("tree", "select", a_path, b_path, "--anchor", a_path)
            time.sleep(0.2)
            js = ATTR_JS.replace("{session_a!r}", repr(a_path))
            r = p.dom_eval(js, timeout=25)
            res = r.get("result") or {}
            if not res or not res.get("accepted"):
                print(f"rep {rep}: refused: {res}")
                continue
            t_click = res["t_click"]
            print(f"rep {rep}: menu_open {res['menu_open_ms']} ms, "
                  f"milestones(+ms after click): {res['milestones']} "
                  f"panes={res.get('pane_sessions')}")
            events = [e for e in p.ytrace_events(t_click - 150, lines=2400)
                      if isinstance(e, dict)]
            events.sort(key=lambda e: e.get("ts_ms", 0))
            print(f"  --- ytrace ladder (+ms after t_click) ---")
            for e in events[:150]:
                off = e.get("ts_ms", 0) - t_click
                if off > 1200:
                    continue
                pay = json.dumps(e.get("payload") or {}, default=str)[:160]
                print(f"  {off:+5d} {e.get('category')}/{e.get('name')} {pay}")
            # restore between reps
            ug = p.dom_eval(ux.SPLIT_UNGROUP_JS, timeout=15)
            acc = (ug.get("result") or {}).get("accepted")
            time.sleep(0.35)
            live = set(p.live_session_paths())
            lost = [x for x in (a_path, b_path) if x not in live]
            print(f"  ungroup accepted={acc} rows_lost={lost}")
            time.sleep(0.4)
    finally:
        for row in p.scratch_titles:
            p.remove_row(row["path"])
        print(f"teardown: {len(p.scratch_titles)} scratch rows removed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
