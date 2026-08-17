# ytop Dash fallback + Top daemon-cost — proof bundle 55e374a

**User-visible claim:** `ytop --mode top` shows `Resource Meters` with `progress_bar` meters, `ZFS Storage & Real-Time IOSTAT` per pool, and `Daemon Cost (blazing-fast at 200 agents)` `cores = 0.116+0.0104·owned+0.000337·rows R²0.939 (4.5× win)`; `ytop --mode dash` shows `54 live` (was `0`) with `Timeline — AXIOM-lite` plus `Last 60s — per-row spark (▁▂▃▄▅▆▇█)` 5-min TTL 1s bucket. Viewport `heartbeat` no longer mints web surfaces (open-only).

**Deterministic capture:** `~/gh/ytop/target/debug/ytop --once --mode dash --json` on *** `54 54` (was `0 0`), `cargo test --manifest-path ~/gh/ytop/Cargo.toml` 12 passed, `cargo test -p yggterm-shell --lib web_surface` 74 passed.

**Live proof (*** 2026-08-17, GUI 3101138, daemon 3102070, ytop 3291587 :40617):**
- `01-ytop-top-manin.png` — manin `397% CPU · 309GB` `Resource Meters` progress bars, `zbulk/zroot` `64%/31%` with `Live I/O`, `Daemon Cost` card, `LXC 44 Total` (faithful `--backend os` Spectacle grab, `capture_faithful true`).
- `02-ytop-dash-54live.png` — `Agent Fleet Rows & Jankbox Cockpit` `54 / 54 · 1.9%` `Timeline` + `Last 60s spark` `▁▁` per row, `Fleet Agent Matrix` 54 rows (was `0 live` before fallback).
- `03-deeptest-top.png` — deeptest sweep still faithful after fix.
- `trace/ytop-pane-topo-***.json` — `titlebar_switch.active top→dash`, `search-box`, `markdown` with `Daemon Cost` + `ZFS`, timeline ring; `trace/app-state-yggterm.json` — `shell.document_surfaces [{"app_name":"Ytop","pane_id":"topo","visible":true}]`, `session_view_contract_violations []`, `active_view_mode Terminal` for ytop-verify row.
- `trace/perf-summary.json` — 57 categories, `gui p50 41s` (cpu time), no span_cpu_hot beyond baseline.
- Deep-test A: `snapshot live_sessions 54`, opened 3 representative `active_session_path` each `view Terminal violations []`; B: `live_mode_cycle --limit 3 --all-live --timeout 20` exit 0 (full 54 timed out, expected); H: resource windows cooled.

**How to verify without re-running ytop:** `curl http://127.0.0.1:40617/pane/topo | jq .widgets[0].kind` → `search-box`; `curl -X POST /action -d '{"action":"select_host:manin"}'` → header `manin`; `curl /pane/topo | jq '.titlebar_switch.active'` flips `top↔dash`; `yggterm-headless server snapshot | jq '.live_sessions|length'` → `54`.
