# Spec: Unified Cross-Harness Fleet Memory System (`~/.yggterm/memory`)

**Status:** APPROVED 2026-08-15 · **Extended with Harness-Scoped Steers**  
**Directive:** user, 2026-08-15 — *"make a ~/.yggterm/memory system so that all agents can have a tab on every CLI harness memory ... keep a diff of each other's memory ... cheap token efficient toolcalls."*  
**Steering Directive:** user, 2026-08-15 — *"while we sync 99% of the memory with each other there are some parts that are meant for ONLY that harness ... upgrade the design for the memory system."*  
**Owner surfaces:** `.agents/skills/yggterm-agent-fleet/ygg-memory.py`, `.agents/skills/yggterm-agent-fleet/SKILL.md`, `~/.yggterm/memory/`, harness memory hooks/adapters.

---

## 1. Executive Summary & Problem Definition

### 1.1 The Multi-Harness Memory Silo
Modern agentic workflows in `yggterm` utilize multiple distinct AI CLI harnesses concurrently:
- **Claude Code** (`~/.claude/projects/<namespace>/memory/`)
- **Antigravity / Gemini CLI** (`~/.gemini/`, `~/.agents/`)
- **Grok CLI** (`~/.grok/`)
- **Codex CLI** (`~/.codex/`)
- **Muse / T3 / Kimi / Qwen** (`~/.muse/`, etc.)

Currently, each CLI harness maintains its own isolated memory store. When one harness (e.g., Claude Code) discovers critical bug classes, refines behavioral specs, or logs a campaign handover, a newly launched session of another harness (e.g., Grok or Antigravity) in the same project/campaign area starts with zero awareness of those recent findings.

### 1.2 The Core Contract & 99% Shared vs 1% Scoped Layering
`~/.yggterm/memory` is the host-resident, cross-harness memory synchronization hub that tracks a live diff of every CLI harness's memory state:
1. **99% Shared Fleet Memory (`target_harness: all`)**: Findings, specs, feedback, and campaign ledgers are synchronized across all harnesses.
2. **1% Harness-Scoped Steering (`target_harness: <harness>`)**: CLI-specific behavioral corrections, tool-calling habits, and model quirks (e.g. `steer-gemini-*.md`, `steer-claude-*.md`) are targeted strictly to that harness, eliminating prompt conflation across different AI engines.
3. **Query Diffs with Minimal Tokens:** Ask "What has changed since my last sync?" in a 25-token toolcall.
4. **Selective / Impatient Ingestion:** Ingest only the specific campaign or topic doors needed for the current turn, or perform a full sync.
5. **Advance Watermarks:** Mark specific memory items or the entire state as acknowledged.
6. **Publish New Doors:** Write newly discovered findings, handover notes, or harness steers directly into the unified layer for relevant harnesses to consume.

### 1.3 Scope & Non-Goals (What This Spec Does NOT Cover)
- **Does NOT replace harness-native session context:** Each CLI harness continues to manage its own active conversation window and internal prompt memory.
- **Does NOT commit memory into the git repository:** Per `feedback-no-session-data-in-repo`, all memory files live exclusively in user-space (`~/.yggterm/memory/` and `~/.claude/...`) and travel between fleet hosts over SSH, the roster read from `~/.config/ygg-fleet/mesh`.
- **Does NOT parse raw LLM transcripts into memory doors:** Distillation of transcripts into memory doors is performed by the working session during its handover ritual, not by a background heuristic parser.

### 1.4 Harness Isolation Law (No Cross-Harness Private Writes)
- ⛔ **Private stores are strictly PRIVATE:** No agent harness (Gemini/Antigravity, Grok, Codex, Kimi, Muse, etc.) is permitted to write directly into another harness's private directory (`~/.claude/`, `~/.gemini/`, `~/.grok/`, `~/.codex/`).
- ✅ **Reading is allowed; writing is forbidden:** An agent may read another harness's historical store if needed for reference, but must NEVER mutate it.
- ⭐ **The Unified Store is the Only Conduit:** Any agent wishing to share findings, rules, or handover ledgers with the rest of the fleet must publish exclusively to the host-resident unified store via `ygg-memory publish` or commit discussions to the canonical project repository (e.g. `docs/discussions/`).
- `sync-harness --harness <name>` is scoped strictly to `<name>`'s own private store and the unified layer.

---

## 2. Architecture & Data Layout

```
~/.yggterm/memory/
├── journal.jsonl                        # Global append-only log of memory operations (with target_harness)
├── watermarks/                          # Per-harness checkpoint vectors
│   ├── claude.json
│   ├── gemini.json
│   ├── grok.json
│   └── codex.json
└── namespaces/
    ├── -home-pi/                        # Fleet-wide memories (rules, hardware topologies)
    │   ├── MEMORY.md                    # Root door index with intelligent steering block
    │   └── ...
    └── -home-pi-gh-yggterm/             # Project-specific namespace
        ├── MEMORY.md                    # Root door index with intelligent steering block
        ├── campaign-*.md                # Campaign ledgers and handover logs (target: all)
        ├── finding-*.md                 # Verified empirical findings (target: all)
        ├── feedback-*.md                # User directives and universal working rules (target: all)
        ├── spec-*.md                    # Behavior contracts (target: all)
        ├── steer-gemini-*.md            # Gemini/Antigravity-scoped steers (target: gemini)
        ├── steer-claude-*.md            # Claude-scoped steers (target: claude)
        └── index-*.md                   # Sub-indexes
```

### 2.1 Door Files & Frontmatter Scoping
Memory entries are structured as individual Markdown files ("Doors, not rooms") with YAML frontmatter:
```markdown
---
name: steer-gemini-ytop-pixel-test
description: "Iterative ytop UI updates must be verified with screenshots until finished."
metadata:
  type: steer
  origin_harness: gemini
  target_harness: gemini               # 'all' | 'gemini' | 'claude' | 'grok' | 'codex'
  modified: 2026-08-15T20:45:00Z
---

# Steer: Antigravity / Gemini ytop Pixel Testing
When updating ytop UI components, take visual screenshots and iterate to completion.
```

### 2.2 Append-Only Journal (`journal.jsonl`)
Every memory mutation appends a single JSON record with `target_harness`:
```json
{
  "seq": 1042,
  "ts": 1786720100,
  "iso": "2026-08-15T14:15:00Z",
  "ns": "-home-pi-gh-yggterm",
  "file": "steer-gemini-ytop-pixel-test.md",
  "kind": "steer",
  "action": "create",
  "harness": "gemini",
  "target_harness": "gemini",
  "summary": "Iterative ytop UI updates must be verified with screenshots until finished."
}
```

### 2.3 Per-Harness Watermarks (`watermarks/<harness>.json`)
Tracks the sequence number and file state absorbed by each CLI harness:
```json
{
  "harness": "gemini",
  "last_seq": 1042,
  "last_sync_ts": "2026-08-15T15:15:00Z",
  "namespaces": {
    "-home-pi-gh-yggterm": {
      "campaign-yggterm-unified.md": "sha256:4f8a...",
      "finding-pty-grid-ssot.md": "sha256:1a2b...",
      "steer-gemini-ytop-pixel-test.md": "sha256:9c8d..."
    }
  }
}
```

---

## 3. Intelligent Ingestion Trigger

To ensure any agent automatically discovers and leverages unified memory without hardcoded model modifications, every root `MEMORY.md` index includes an **Intelligent Ingestion Steering Block** at the very top:

```markdown
> 🌐 **UNIFIED FLEET MEMORY**: Before deep memory recall or after campaign handovers, consult `ygg-memory status --harness <me>` or `ygg-memory diff` to catch updates from Claude, Grok, Codex, or Gemini. Ingest full or partial diffs as needed.
```

When an agent reads `MEMORY.md` at session start:
1. It sees the steering prompt.
2. It executes `ygg-memory status --harness <me>`.
3. If updates exist, it reviews the diff and chooses whether to selectively ingest the active campaign door or perform a full sync.

---

## 4. Tooling Interface: `ygg-memory.py`

The CLI tool lives in `.agents/skills/yggterm-agent-fleet/ygg-memory.py` and is callable directly by any agent or hook.

### 4.1 Subcommands & Token Costs

| Subcommand | Purpose | Typical Token Cost |
|---|---|---|
| `status` | Reports whether harness is behind on shared doors + its own scoped steers | **~25–40 tokens** |
| `diff` | Outputs compact delta list with one-line descriptions/hooks | **~80–150 tokens** |
| `get` | Fetches full Markdown content of a specific memory door | Variable (file size) |
| `ack` | Advances harness watermark globally (`--all`) or for specific files | **~10–20 tokens** |
| `publish` | Ingests a memory door with optional `--target-harness` scope | **~20 tokens** |
| `sync-harness` | Two-way sync between harness-local directory and matching unified doors | Local filesystem |
| `sync-fleet` | Multi-host mesh synchronization across the configured fleet roster over SSH | Network |

### 4.2 Example Interaction Workflows

#### Fast Startup Check (Turn 1)
```bash
$ python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py status --harness gemini
{"behind": 2, "last_seq": 1039, "latest_seq": 1041, "changed_doors": ["campaign-6.0-orchestrator-handover.md", "steer-gemini-ytop-pixel-test.md"]}
```

#### Scoped Diff Inspection
```bash
$ python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py diff --harness gemini
[#1040 | campaign] campaign-6.0-orchestrator-handover.md (by claude): Seat 6.0 orchestrator cluster state & dead SHA sweep
[#1041 | steer -> gemini] steer-gemini-ytop-pixel-test.md (by gemini): Iterative ytop UI updates must be verified with screenshots until finished
```

---

## 5. Multi-Host Fleet Synchronization Mesh

Unified memory syncs across the fleet hosts named by `~/.config/ygg-fleet/mesh`, following the established fleet protocol (`reference-fleet-memory-sync.md`):

1. **Snapshot First:** Local memory is snapshotted to `~/.yggterm/memory-backups/<timestamp>/` before sync.
2. **Two-Pass Mesh:** Pull from every reachable peer, then push to every reachable peer over SSH.
3. **Newest-Wins per Door File (`rsync -az -u`):** Preserves recent edits without clobbering.
4. **Append-Only Journal Merge:** Merges `journal.jsonl` entries deterministically sorted by timestamp and sequence ID.
5. **No Auto-Delete:** Pruning is an intentional fleet archive operation (`~/.yggterm/memory-archive/`), never a local unilateral deletion.

**⛔ The roster is configuration, and it lives outside every checkout.** `sync-fleet`
resolves the peer list in one place — `resolve_fleet_mesh` in `ygg-memory.py` — reading
`--mesh`, then `$YGG_FLEET_MESH`, then `~/.config/ygg-fleet/mesh` (one ssh alias per line,
`#` comments). There is deliberately **no built-in default**: a default would put private
host names back into a public tree, which is exactly where they used to be — hardcoded as
the `--mesh` default here and a second time in `ygg-memory-sync`, so every push carried
them and the two copies could drift. `ygg-memory-sync` now passes no roster at all, which
is what leaves one owner. An unresolved roster raises, naming the three sources it tried;
it never degrades to a silent no-op that reports success.

---

## 6. Verification & Privacy Invariants

1. **Privacy Guard (`scripts/check-privacy.sh`):** Unit tests and documentation must only use synthetic test fixtures (invented UUIDs, invented project paths like `/home/user/proj`). Zero real filings, personal paths, or credentials in fixtures.
2. **Locking & Concurrency:** File locks (`flock`) prevent interleaving when multiple agents read/write concurrently.
3. **Idempotence:** Every operation (`ack`, `publish`, `sync-harness`) is strictly idempotent.
