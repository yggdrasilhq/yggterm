# AGENTS.md

## Mission & Core Value Proposition

Build **Yggdrasil Terminal**: a Rust-first, cross-platform, remote-first terminal workspace with a Dioxus desktop shell shaped like Zed, a daemon-owned PTY core, and an embedded xterm.js terminal surface.

### The product yggterm replaces
The user's pre-yggterm workflow: VSCode terminal panes → tmux inside them (for persistence) → ssh to N different machines → `codex resume` / `claude -r` on each. When VSCode dies, manually reattach to tmux, re-find sessions, re-orient across machines. **yggterm exists to make this workflow disappear.**

### Core value proposition & handoff rule
When the user clicks an agent session in the cwd tree, yggterm performs the equivalent of:
```bash
ssh <machine> "cd <cwd> && codex resume <UUID>"      # or: claude -r <UUID>
```
…and hands off the terminal. The user just types. **This handoff IS the product.**
> **Wrapper-vs-manual parity rule:** If `app open <agent-session>` renders differently from `ssh -t <machine> codex resume <UUID>` typed into a clean shell, that is a yggterm bug, NOT a CLI bug. Diagnose in a clean shell first; fix yggterm's wrapper, PTY setup, or preservation path without adding CLI flags that change CLI behavior.

### First-class vs second-class sessions
- **First-class:** Codex, Claude Code, and all managed agent CLIs (per [[spec-cwd-tree-agent-cli-unified]]). The agent CLI persists via its JSONL; yggterm invokes `<cli> resume <UUID>`. Organized by `cwd` in the tree.
- **Second-class:** Plain shell terminals (`Shell`, `SshShell`). Survive GUI death IFF marked keep-alive. Listed in `Live Sessions` on the sidebar rail; transient otherwise.

### What yggterm does NOT do
- Does not parse codex/CC JSONL into the terminal viewport (terminal view delegates rendering to the CLI itself; reading JSONL belongs to the web view).
- Does not reinvent the agent CLI's rendering.
- Does not add CLI flags beyond the minimum needed for handoff (`cwd`, UUID, terminal appearance).

## Local repository relationships
- `../ghostty` contains legacy Ghostty integration code in Zig.
- `../zed` is an optional visual/reference checkout for shell design study.
- This repo (`yggterm`) is the integration layer and product surface.
- ⛔ `~/gh/paper` and `~/gh/cellulose` no longer exist (deleted 2026-08-07). Cellulose returns later as a fresh repo under private IP gate; its design target is concept-only across `docs/alt-keytips.md`.

## ⛔ Privacy — Public Repo Mandate
**Run `scripts/check-privacy.sh` before every commit (enforced by `tests/privacy.rs`).**
- **INVENT every example:** Sidebar titles, home paths, project names, portals, filenames (`"3. widgets: refactor"`, `/home/user/proj`, `example.test`).
- **War-story comments cite SYMPTOMS, never cases:** "a batch fill of an async autocomplete drops the second value" — not live filing titles or personal entities.
- **Never commit build artifacts:** No `.pyc`, `.db`, or binary cache leaks that embed absolute build paths.

## ⛔ Presentation Policy
**Read `docs/presentation-policy.md` before touching display code.** Sanctioned per-platform defaults live in `crates/yggterm-core/src/presentation_policy.rs`.
1. **Never set `PRESENTATION_VARS` on the user's running GUI:** Use `scripts/underglass-sandbox.sh` for testing.
2. **Wayland runs Wayland-native:** Forcing X11 produces XWayland and invalidates compositor, input latency, and terminal measurements.
3. **Xvfb lessons do not travel:** Headless sway / Xvfb are X11; do not copy `GDK_BACKEND=x11` to Wayland hosts.
4. **Table is SSOT:** To change a default, update the table in `presentation_policy.rs` with measurements.

## ⛔ Scratch Space — No `/tmp` Writes
**`/tmp` is a `tmpfs` (RAM) on desktop hosts.** Bytes written there charge to RAM and swap.
- **Agent scratch goes in `~/.yggterm/scratchpad/<whatever-you-like>`.** Disk-backed only. Not `/tmp`, not `/dev/shm`, not `$XDG_RUNTIME_DIR`.
- Enforced by `scripts/ygg-scratch-guard.sh` and `scripts/ygg-resource-panic.sh`. Reap stale scratch; do not exhort.

## Decentralized Host-Resident Daemon Architecture

Yggterm is a terminal multiplexer that matches tmux's persistence while exceeding it in modern affordances.

**Nomenclature:**
- **yggterm** — the GUI desktop client.
- **yggterm-headless** — the headless client/control interface for agents.
- **yggterm server** — the host daemon that owns PTYs and keeps sessions alive.

**The Model:**
1. **Server runs on EVERY host:** Local machine AND each SSH host. The server owning a session's PTY runs on the machine where the session lives. Remote sessions are held alive by the remote host's yggterm server.
2. **SSH is transport and auth:** Reaching a remote host's yggterm server over SSH is the sole authorization to attach its sessions.
3. **Decentralized metadata:** Each host's `~/.yggterm` stores its own sessions, scrollback, and titles. The cwd tree is a union composed by querying each host's server.
4. **Many clients, one server:** Multiple GUI or headless clients can attach live sessions concurrently.
5. **No tmux dependency:** The host-resident yggterm server IS the multiplexer across GUI restarts and SSH drops.

**Tmux Parity & Exceed Gates:**
- *Parity (Baseline):* Client-disconnect survival, multi-client reattach, synchronized window resize, and persistent terminal scrollback ring (10k+ rows).
- *Exceed:* 24-bit xterm.js rendering, cross-machine unified session tree, cursor/scroll preservation across GUI restart, transcript stitching & generated titles, first-class observability, and unified multi-CLI cwd tree.

---

## Multi-CLI Integration & Startpage / CwdTree Contract

Per [[spec-cwd-tree-agent-cli-unified]] and `docs/cli-integration.md`:
1. **SSOT Discovery (`scan_all_durable_sessions`):** Every agent CLI session flows through `yggterm_core::startpage::scan_all_durable_sessions(&Path)` over `AGENT_CLIS` descriptors in `agent_cli.rs`. Startpage and CwdTree in GUI (`shell/startpage.rs`) and CLI (`startpage_ls.rs`, `cwdtree_ls.rs`) consume identical core state; only the presentation porcelain differs.
2. **Never Show Plain Shells (`[$ ]`) on Startpage or CwdTree:** Plain shells (`SessionKind::Shell`) belong strictly to `Live Sessions` on the sidebar rail (`presence: "live_rail"`). They must NEVER appear as Startpage cards or CwdTree folder sessions.
3. **Recency Descending Sort:** All durable sessions across all CLIs sort by `modified_epoch_ms` descending (preventing 0-epoch fallback to alphabetical UUID).
4. **Zero Shorthash / Corrupt Titles:** No session may surface an 8-hex shorthash (`a8f6dbd1`), raw path, or generic placeholder (`New session`, `Remote <kind> <hash>`). Titling authority rules in `titles.rs` and `managed_cli/{cli}.rs` are authoritative.
5. **Built-in Interface LLM Title Rescue:** When stored transcripts have zero usable title or heuristic signal, yggterm core (`crates/yggterm-core/src/titles.rs`) automatically schedules background Interface LLM rescue (`gpt-5.6-luna` / `gemini-3.7-flash` via LiteLLM) as a built-in measure of last resort.
6. **Managed CLI Modularization:** CLI-specific flags, launch/resume patterns, and environment quirks live in `crates/yggterm-server/src/managed_cli/{cli}.rs`.
7. **Per-CLI Rendering Quirk Isolation:**
   - *Claude Code:* Refresh latching + 1500ms recovery ceiling to prevent CUF whitespace skips from locking partial paints; symmetrical container padding; absolute viewport re-anchoring on switch.
   - *Codex / Codex-LiteLLM:* SIGWINCH resize nudge to eliminate geometry squish; absolute screen state replay on reveal.
   - *Muse:* Authoritative title extraction from `session-index.db`; scrollback offset preservation during re-attach.
   - *Antigravity:* Row bounds clamping on interactive footer shortcuts (`esc/ctrl/enter/tab`) to prevent top prompt clipping; batched streaming token renders.
   - *Pi / Qwen / Grok / OpenCode / Kimi:* Synchronized PTY resize propagation to prevent line-wrap distortions.

---

## Operating Directives & Engineering Contracts

- **Docs SSOT Audit:** `docs/architecture-audit-2026-05-16.md` is required reading before terminal, session, hot-update, theme, telemetry, app-control, or release-gate changes.
- **⛔ SHIP IT (GUI Restart Policy):** Daemon owns the PTYs, sessions, and scrollback across GUI restart. Build it, install it, restart without asking, and report what was done — a GUI restart needs no permission.
- **Source of Truth Rule:** Before fixing any regression, name the authoritative source of truth and the observers involved.
- **Observer Rule:** Never promote an observer into product truth. App-control, telemetry, screenshots, and logs are witnesses, not drivers of state.
- **No Symptom Patching:** Do not patch a symptom by adding a second source of truth (e.g. no shell text overlays, no prompt repair layers, no PTY-byte trimming).
- **Spec Interpretation Rule:** Every spec MUST enumerate what it does NOT cover. Quote exact text, state the literal claim, and state adjacent claims not made.
- **Session Display = Dual Presence:** Active sessions appear in BOTH "Live Sessions" and their cwd folder. Dedup is per-view, never cross-view.
- **SessionKind Drives Display:** Icon, glyph, and color dispatch must consult `SessionKind`, never path-prefix heuristics. Local and remote session display paths share code.
- **Panic Management & Incident Artifacts:** Treat `yggterm-headless server monitor` as the first-line diagnostic tool. Write incident artifacts to disk-backed scratch:
  ```bash
  mkdir -p ~/.tmp/yggterm && yggterm-headless server monitor --scenario panic-report --expect-path <session-path> --jsonl-out ~/.tmp/yggterm/yggterm-incident.jsonl
  ```
- **Stale Binary Prohibition:** Never execute archived GUI binaries or old `dist/` artifacts against live state. Prove versions from canonical metadata (`Cargo.toml`, git tag, `install-state.json`).
- **Deterministic Diagnostics & Screenshots:** Use `.agents/skills/yggterm-diagnostics/SKILL.md`. For visual bugs, proof requires in-process faithful screenshots (`server app screenshot <out.png>`), not telemetry alone.
- **Contract & Regression Tests:** Every fix must update the harness, smoke test, unit test, or CI gate to fail deterministically on the defect class before applying the runtime patch.

---

## Platform, Skills & Licensing

- **libyggterm Surface Ownership:** yggterm provides the surface interface; embedded libyggterm apps (ychrome, ytop, Paper) own their content. Zero app-specific chrome belongs in `yggterm-shell`. Consult `.agents/skills/libyggterm-surfaces/SKILL.md`.
- **Synchronize Skills:** When changing surface mechanisms, telemetry, or diagnostics, update the matching `.agents/skills/` file in the same commit.
- **Licensing:** Code is licensed under `GPL-3.0-or-later` (see `LICENSE`); markdown docs under `CC BY-SA 4.0`. Copyright owner: Avikalpa Kundu.

