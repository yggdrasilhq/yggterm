# Spec: Unified Cross-Harness Fleet Memory System (`~/.yggterm/memory`)

**Status:** APPROVED 2026-08-15 · **Implementation in progress on `lane/dev/unified-fleet-memory`**  
**Directive:** user, 2026-08-15 — *"make a ~/.yggterm/memory system so that all agents can have a tab on every CLI harness memory ... keep a diff of each other's memory ... cheap token efficient toolcalls."*  
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

### 1.2 The Core Contract
`~/.yggterm/memory` is the host-resident, cross-harness memory synchronization hub that tracks a live diff of every CLI harness's memory state. It enables any agent on any harness to:
1. **Query Diffs with Minimal Tokens:** Ask "What has changed since my last sync?" in a 25-token toolcall.
2. **Selective / Impatient Ingestion:** Ingest only the specific campaign or topic doors needed for the current turn, or perform a full sync.
3. **Advance Watermarks:** Mark specific memory items or the entire state as acknowledged.
4. **Publish New Doors:** Write newly discovered findings or handover notes directly into the unified layer for other harnesses to consume.

### 1.3 Scope & Non-Goals (What This Spec Does NOT Cover)
- **Does NOT replace harness-native session context:** Each CLI harness continues to manage its own active conversation window and internal prompt memory.
- **Does NOT commit memory into the git repository:** Per `feedback-no-session-data-in-repo`, all memory files live exclusively in user-space (`~/.yggterm/memory/` and `~/.claude/...`) and travel between fleet hosts (`***`, `dev`, `oc`) over SSH.
- **Does NOT parse raw LLM transcripts into memory doors:** Distillation of transcripts into memory doors is performed by the working session during its handover ritual, not by a background heuristic parser.

---

## 2. Architecture & Data Layout

```
~/.yggterm/memory/
├── journal.jsonl                        # Global append-only log of memory operations
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
        ├── campaign-*.md                # Campaign ledgers and handover logs
        ├── finding-*.md                 # Verified empirical findings
        ├── feedback-*.md                # User directives and working rules
        ├── spec-*.md                    # Behavior contracts
        └── index-*.md                   # Sub-indexes
```

### 2.1 Door Files & Frontmatter
Memory entries are structured as individual Markdown files ("Doors, not rooms") with YAML frontmatter:
```markdown
---
name: finding-pty-grid-ssot
description: "PTY grid SSOT divergence on Wayland resize — daemon holds grid dimensions, client adapts."
metadata:
  type: finding
  modified: 2026-08-15T10:30:00Z
  origin_harness: claude
  origin_session_id: 09b58856-c5c4-4ca4-a7e8-17b66ddf3d12
---

# Finding: PTY Grid SSOT Divergence
...
```

### 2.2 Append-Only Journal (`journal.jsonl`)
Every memory mutation (creation, update, archive) appends a single JSON record:
```json
{
  "seq": 1042,
  "ts": 1786720100,
  "iso": "2026-08-15T14:15:00Z",
  "ns": "-home-pi-gh-yggterm",
  "file": "finding-pty-grid-ssot.md",
  "kind": "finding",
  "action": "upsert",
  "harness": "claude",
  "summary": "PTY grid SSOT divergence on Wayland resize"
}
```

### 2.3 Per-Harness Watermarks (`watermarks/<harness>.json`)
Tracks the sequence number and file state absorbed by each CLI harness:
```json
{
  "harness": "grok",
  "last_seq": 1039,
  "last_sync_ts": "2026-08-15T12:00:00Z",
  "namespaces": {
    "-home-pi-gh-yggterm": {
      "campaign-yggterm-unified.md": "sha256:4f8a...",
      "finding-pty-grid-ssot.md": null
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
| `status` | Reports whether harness is behind, count of new doors, and updated topics | **~25–40 tokens** |
| `diff` | Outputs compact delta list with one-line descriptions/hooks | **~80–150 tokens** |
| `get` | Fetches full Markdown content of a specific memory door | Variable (file size) |
| `ack` | Advances harness watermark globally (`--all`) or for specific files | **~10–20 tokens** |
| `publish` | Ingests a new/updated memory door into `~/.yggterm/memory` & journal | **~20 tokens** |
| `sync-harness` | Two-way sync between harness-local directory and unified store | Local filesystem |
| `sync-fleet` | Multi-host mesh synchronization across `***`, `dev`, `oc` over SSH | Network |

### 4.2 Example Interaction Workflows

#### Fast Startup Check (Turn 1)
```bash
$ python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py status --harness grok
{"behind": 2, "last_seq": 1039, "latest_seq": 1041, "changed_doors": ["campaign-6.0-orchestrator-handover.md", "finding-pty-grid-ssot.md"]}
```

#### Scoped Diff Inspection
```bash
$ python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py diff --harness grok
[#1040 | campaign] campaign-6.0-orchestrator-handover.md (by claude): Seat 6.0 orchestrator cluster state & dead SHA sweep
[#1041 | finding] finding-pty-grid-ssot.md (by claude): PTY grid SSOT divergence on Wayland resize
```

#### Impatient / Selective Sync & Ack
```bash
# Ingest only the campaign handover
$ python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py get --file campaign-6.0-orchestrator-handover.md
# Acknowledge only that campaign
$ python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py ack --harness grok --files campaign-6.0-orchestrator-handover.md
{"status": "ok", "acked": ["campaign-6.0-orchestrator-handover.md"], "remaining_behind": 1}
```

---

## 5. Multi-Host Fleet Synchronization Mesh

Unified memory syncs across the three fleet hosts (`***`, `dev`, `oc`) following the established fleet protocol (`reference-fleet-memory-sync.md`):

1. **Snapshot First:** Local memory is snapshotted to `~/.yggterm/memory-backups/<timestamp>/` before sync.
2. **Two-Pass Mesh:** Pull from every reachable peer, then push to every reachable peer over SSH.
3. **Newest-Wins per Door File (`rsync -az -u`):** Preserves recent edits without clobbering.
4. **Append-Only Journal Merge:** Merges `journal.jsonl` entries deterministically sorted by timestamp and sequence ID.
5. **No Auto-Delete:** Pruning is an intentional fleet archive operation (`~/.yggterm/memory-archive/`), never a local unilateral deletion.

---

## 6. Verification & Privacy Invariants

1. **Privacy Guard (`scripts/check-privacy.sh`):** Unit tests and documentation must only use synthetic test fixtures (invented UUIDs, invented project paths like `/home/user/proj`). Zero real filings, personal paths, or credentials in fixtures.
2. **Locking & Concurrency:** File locks (`flock`) prevent interleaving when multiple agents read/write concurrently.
3. **Idempotence:** Every operation (`ack`, `publish`, `sync-harness`) is strictly idempotent.
