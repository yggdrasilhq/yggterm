# managed_cli — per-CLI launch and provision nuances

`codex_cli.rs` was a misnomer: it held the launch, provision, terminal-identity,
and refresh logic for **all** eleven CLIs. This directory is the split.

- `mod.rs` — common: `ManagedCliTool`, `ManagedCliAction`, provision, refresh, identity.
- `codex.rs` / `claude.rs` — Codex / Claude Code specific: `--session-id` birth, sqlite/poll rebind, re-root.
- `pi.rs` — Pi: `--session` birth==resume, no permission gate.
- `opencode.rs` — OpenCode: RPC-minted sessions, `--auto` bypass, SQLite store gap.
- `qwen.rs` — Qwen: hidden `--approval-mode` flags, `custom_title` tail scan.
- `kimi.rs` — Kimi: `-r`/`--resume`, MD5 bucket gap, `content_rederives_on_resume=false`.
- `muse.rs` — Muse: `muse resume <id>` subcommand, `session-index.db`, `local://` vs `muse-runtime://`.
- `antigravity.rs` — Antigravity: `--conversation`, `conversation_summaries.db`, two-gate permission.
- `grok_build.rs` — Grok Build: `--session-id` birth vs `--resume`, percent-encoded bucket.

Each file owns the CLI's **measured** flags (from `binary --help` on fleet) and
its **store** shape (`session_store_globs`, `store_scan_gap`). The registry
(`yggterm-core/src/agent_cli.rs:AGENT_CLIS`) is the SSOT; this directory's
per-CLI modules are the launch-side owners that read it.

Phase 1 is a pure rename (`codex_cli` → `managed_cli` shim); per-CLI extraction
is incremental as restore/PTY/viewport nuances are fixed (e.g. Muse/AGY
keep-alive restore, PTY clamp 120×40, webview Rendered scaffold).
