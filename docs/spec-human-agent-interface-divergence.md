# Spec: Human vs Agent interfaces — where they diverge and what to read

**Status:** LIVE CONTRACT  
**Last verified:** daemon 3.1.3 (build 1787033587, commit 9125ff0a6dd4), `server snapshot`/`app state`/`app rows`/`terminal tenants`/`app screenshot capture_faithful:true` on 2026-08-18 — `active_terminal_surface.problem: null` (dom_paint_hit_test.problem ''), `snapshot live_sessions 27` `daemon owned 1` (preserved 8), paint probe `rows_in_stack && host_in_stack` ancestor fix deployed, `server app terminal adopt --target-pid` (also --outer-pid/--pid) with `app_control_pid_flag` adopt-aware, `server app open` switches viewport faithfully, adopted row `local://b445b5bc-7e57-41db-8176-cc111a26ad94` "adopted-muse-pts29" draggable true live_rail, reptyr ptrace blocked for muse node (PR_SET_DUMPABLE) — verb reports adopt_refused, plain shell PTYs adoptable  
**Owners:** `libyggterm-surfaces` (four human surfaces), `yggterm-diagnostics` (instrument authority), `yggterm-agent-fleet` §7 (verbs report request not effect)  
**Non-goal:** this file owns the *map*, not the *fix* — the four divergences below are named here, fixed elsewhere.

---

## 1. The one rule both planes share

> **yggterm provides the surface interface. The APP or DAEMON owns the content.**

A human surface and an agent plane are two views of the same row — the row's UUID is the join key, not its title, path scheme, or PTY number.

```text
$YGGTERM_SESSION_ID  cc-runtime://<uuid>          — what the daemon env exports
row path             remote-cc://<host>/<uuid>    — what the GUI verbs want
daemon key           local://<uuid>               — what snapshot/tenants key on
```

Match on the UUID suffix (`${VAR##*/}`), never by pasting the env var into a row-path verb.

---

## 2. Taxonomy

### 2.1 Human surfaces (what a person sees)

Built by `libyggterm-surfaces`. Each rides the GUI process — screenshots can lie when backgrounded.

| surface | transport | freshness | authority |
|---------|-----------|-----------|-----------|
| **Viewport** — terminal + web/document view | WebKitGTK child webview or document-schema DOM in main pane | rendered by GUI, stale when unfocused (`viewport_y`, `document.hasFocus()` lies on Wayland) | `app state active_terminal_hosts[]` + `snapshot active_session.terminal_lines` |
| **cwd-tree document** | tree model `ServerUiSnapshot::apps` | GUI snapshot, `__live_sessions__` child_count | `app rows` (but see duplication below) |
| **Sidebar panel / chooser** | right-panel contribution `sidebar; declare` + `GET <control>/pane/<id>` + heartbeat | GUI-rendered schema, ping-refreshed every ~2.5s | `app rows` + screenshot `capture_faithful` |
| **Toast / notify** | `server app notify --session <row-path>` fan-out | appears only if GUI is running | `notify` returns `error:null` for a misaddressed card too — address must be row path |

### 2.2 Agent planes (what code sees)

| plane | transport | freshness | authority |
|-------|-----------|-----------|-----------|
| **Daemon screen** — authoritative vt100 | host-resident daemon PTY → `server snapshot` `live_sessions[].terminal_lines` | live, escapes inline, host-resident | daemon ground truth — the fix for any “broken bottom / paint” is `snapshot` vs client buffer diff |
| **App control RPC** — rows, state, traces | GUI process RPC `server app *` | GUI must be running; answers unwrap `data`; snapshot vs app key shape differs (snapshot is flat, app is wrapped) | `app state` `active_terminal_surface` + `terminal tenants` + `gate-screen` + `trace` |
| **PTY byte stream + OSC 7717** — surface control | `ESC ] 7717 ; <verb> ; <action> ; <base64-json> BEL` on the PTY | carried by the PTY relay, survives host hops, unknown OSCs invisible in plain term | `web-surfaces.md` lifecycle (`seen` rewrite) |

---

## 3. Divergence catalog — why a row can be “in the ether”

These four divergences produced the reported “I see 52 in Live Sessions but not my terminals”.

### Divergence A — paint probe: raw vs suppressed `problem`

*Human:* `app screenshot` with `capture_backend xterm_canvas_composite` but screenshot was gated on `active_terminal_surface.problem` (suppressed fresh-shell).  
*Agent:* `app state active_terminal_hosts[].dom_paint_hit_test_problem` raw stays `xterm row sample is not topmost` for a single-line `pi@host` prompt at `y=40` — `elementsFromPoint` returns `xterm-screen` (ancestor) not the span, so `top_within_rows false`.  
*Raw `terminal_observe.rs`* had `visible_text.len<40 && contains pi@openclaw` but `visible_text` is `"\n"`-joined dedup via raw equality (`pi@openclaw` vs `pi@openclaw ` distinct → 45 chars), so the exemption never fired.  
*Fix retained in 3.1.3:* also check `buffer_text_sample|cursor_line_text|text_tail` and any `pi@openclaw` line `<50` chars; `app state surface.problem` is now `null` while raw host field still shows the probe. **Read `active_terminal_surface.problem`, not the raw host field.**

### Divergence B — stale viewport & Wayland focus lie

`buffer.active.viewportY` (public) vs `effectiveXtermViewportY` (painted) diverge on bg→fg stale-render strand; `app state viewport_y` is stale when window backgrounded. On KDE Wayland a visible foreground window still reports `document.hasFocus()=false` (`document_focused false`). **Gate layout/render on `hostLooksUsable`/visibility, never on `document_focused`.** See `yggterm-diagnostics` caveats: use `viewport_force_log` (`app terminal probe-scroll --lines 0`) + human eyes, not `viewport_y`.

### Divergence C — `app rows live_rail 53` vs `snapshot live_sessions 41` vs GUI `Live Sessions ▾ 41`

`app rows` total 55 (53 `live_rail`) includes the `local` group which duplicates the `__live_sessions__` children — `local` rows repeat the same UUIDs. `snapshot live 41` and `tenants measured 24` (daemon owned 24) are the canonical counts. `row_count 41` vs `measured 24` tracks owned vs live-rail. **Do not treat `live_rail` count as daemon truth; diff `snapshot live_sessions` + `tenants`.** The duplicate `local` group is a rendering bug, not missing sessions.

### Divergence D — remote/host filtering

A daemon screen is host-resident (`~/.yggterm/sessions` per host, `remote_machines []` here, `jojo` has no yggterm binary → no remote probe). `snapshot` on `openclaw` shows only its 41 sessions; a session on another host never appears in this host’s GUI until that host runs a yggterm daemon and the row is `remote-cc://host/uuid`. `SSH localhost` shells (`sshd-session` parent, no `YGGTERM_SESSION_ID`) were never yggterm rows and never in Live Sessions — they are the “ether”. Attach them by `yggterm server app terminal new --cwd …` daemon-owned replacements (e.g. `local://a1b2…`).

---

## 4. What to read for what (the check that cannot lie)

| question | human check (before acting) | agent check (ground truth) | never use |
|----------|----------------------------|----------------------------|-----------|
| Is X live / has PTY? | GUI Live Sessions badge (after `app rows` check) | `snapshot live_sessions[].terminal_lines` + `launch_phase Running` | `ps %CPU` (lifetime avg), screenshot alone |
| Is Y paint-visible / ready? | `app screenshot` (`capture_faithful`, canvas composite) + human eyes | `app state active_terminal_surface.problem` (suppressed) — not raw `dom_paint_hit_test_problem` | raw host field, `viewport_y` alone |
| What’s running inside row? | tooltip in tenant overlay | `terminal tenants` (`foreground_command`, `tree_cpu_seconds`, `idle_secs`) | `pgrep -P` (says ≠ parked both ways) |
| Why hot-restart defers? | Right-panel Daemon “Restart deferred — … was active 0s ago (idle window 300s) (+N more)” | `gate-screen` per-session `recently_active idle Xs` + `daemons --json hot_restart_blockers` | `ps` child count alone |
| Is scroll/follow stuck? | scroll the web/shell view | `app terminal probe-scroll --lines 0` `viewport_force_log` ring + `snapshot base_y` | `app state viewport_y` when backgrounded |

Deterministic harness first per `yggterm-diagnostics`: `mock-tui --scenario normal-scrollback` + `pipeline_integration` (daemon) and `tools/xterm-harness` (xterm.js) over the exact vendored `assets/xterm/xterm.js`. String probes like `terminal probe-scroll` answer `{accepted,reason}`, never buffer content — use `snapshot terminal_lines`.

---

## 5. Keeping the map fresh

* Every fix that touches scroll, paint, or row ownership extracts its decision into a pure Rust module with unit tests + a guard asserting the generated JS string contains the wiring (e.g. `scroll_mode.rs`, `terminal_retained_replay_policy.rs`).
* A divergence left only in a chat transcript was never learned — durable nuance goes to its owning `docs/*` or `xterm-bugs.md` with `// XTERM-BUG: <id>` anchors, then the trace.
* This file carries its verification stamp above; refresh it when `snapshot`/`app state` shape changes. Status lives in `pending-bugs.md`, not here.

*Motivating example (2026-08-18): AGENT saw `snapshot live_sessions 41`, `app rows live_rail 53`, `tenants 24`, `app state ready True` after the paint exemption; HUMAN saw `Live Sessions ▾41` with two `recovered-*` rows but reported “my terminals are in the ether” — the two `muse --yolo` were `sshd` `pts/28,29` with no `YGGTERM_SESSION_ID`, never yggterm rows, fixed by daemon-owned replacements. The `local` duplication and stale Wayland `viewport_y` made the counts disagree without being a loss.*
