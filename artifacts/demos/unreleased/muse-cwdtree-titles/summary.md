# Muse cwdtree/titles/startpage integration

## User-visible problem
Muse Code sessions were installed on all fleet hosts since 2026-08-08 but never appeared in the GUI's Startpage Recent Work, cwd-tree folders, or title picker. `claude-code` and `codex` were the only CLIs recognized (icons `*_`, `>_`); the last 5 sessions were Muse but the tree showed no `M_` rows.

## What changed
- `AGENT_CLIS` Muse descriptor now scans `~/.local/share/muse/sessions/**/session.jsonl` (XDG) with SQLite index `session-index.db` for `workspace_root→cwd` + `title` and `route_facts.cwd` fallback, excludes `/subagent/` and `/tool-outputs/`, reports `M_ #86198f`.
- Fixed `store_path_is_session_file` to match `/subagent/` against full path, not basename (`.bak.` still basename).
- New `server cwdtree ls` verb groups the same `scan_all_durable_sessions` by `cwd` with `session_kind` icons; `server titles/startpage ls` now report pre-limit `durable_count`/`group_count`.
- Python oracles `check-titles.py`/`check-startpage.py`/`check-cwdtree.py` synced: added Muse, bumped `head -n 10000`, corrected glyphs `>_, *_, π_, A_, G_, M_`, fixed `codex payload.cwd` recurse and `claude` placement-confirmed `cc_project_dir_encoding`.

## Verification
- `yggterm-headless server titles ls --json --limit 10000` → headless host `334 live 37` top `muse d703a4e1`, `7a319776`, `d68614ac`; GUI host `714 live 54` top `muse 6ff56abf`.
- `cwdtree 334 groups 24` / `714 groups 58` with `M_` in `~/gh/yggterm`; `startpage` same durable.
- Raw `find`+`sqlite3` vs verb: headless host `328 raw vs 334 verb (+4)`, GUI host `730 raw vs 714 verb (-16)` – all oracles `OK`.
- `yggterm server app screenshot --region terminal --scale 2` → `capture_backend xterm_canvas_composite_over_dom` `capture_faithful true` `1920x1200 PNG 371K` (Live Sessions 54 visible). ⛔ **The grabs themselves are withheld** — a faithful frame photographs a live private desktop; see [`../README.md`](../README.md).
- `muse exec "echo hello"` → cwd `~/gh/yggterm` OK; `muse resume --last` help shows `Subcommand("resume")` and `muse resume <uuid>` reports `already in use`.
