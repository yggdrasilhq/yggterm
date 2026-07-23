# Spec: the agent-CLI harness contract — one pathway, N CLIs

**Status:** DESIGN COMPLETE 2026-07-23 · implementation not started (user-gated)
**Directive:** user, 2026-07-23 — *"line up {remote,local}×{cc,codex} code pathways
and optimize them … as the number of CLI harnesses increases they can share a
shared code pathway like codex and claude code."* Planned CLIs 3–5: **kimi**,
**antigravity**, **opencode**.
**Owner surfaces:** yggterm-server (daemon.rs, lib.rs), yggterm-core, the GUI
terminal path in yggterm-shell, the CLI arms in apps/yggterm.

This spec subsumes the behavioral half of three older contracts and does not
repeat them: [[spec-unify-local-remote]] (kind-driven display; path prefix never
drives display), [[spec-cwd-tree-agent-cli-unified]] (one scanner shape, one tree
builder — the pattern this spec generalizes), and
[[spec-agent-cli-wrapper-render-parity]] (wrapper output ≡ manual CLI output —
this spec's acceptance method).

---

## 1. The problem, stated from evidence

There are four live agent pathways today — `local-codex`, `local-cc`,
`remote-codex` (`remote-session://`), `remote-cc` (`remote-cc://`) — and they
have drifted: different launch-command builders, different readiness gates,
different replay sources, different birth sites, different predicate coverage.
Every bug in this family has had to be fixed 2–4 times, and several were fixed
on one arm only. Adding three more CLIs under the current shape means 10
pathways and a combinatorial bug surface. The receipts:

- **`remote-cc-replay-codex-only`** (docs/xterm-bugs.md): the retained-replay
  readiness gate recognized only Codex prompt signatures, so resumed CC
  viewports blanked. A per-CLI heuristic hardcoded into one arm.
- **`persisted_live_session_is_recoverable`** once recognized only the codex
  remote scheme; every remote CC keep-alive row was dropped at daemon restart
  (fixed 2026-06-10, but the shape that allowed it is still the norm).
- **The sanitizer's scheme list is incomplete TODAY**: the transport-noise
  predicate in `batch_terminal_chunks` matches `local://`,
  `remote-session://`, `codex-runtime://` — `cc-runtime://` is absent. Every
  hand-enumerated scheme list is one CLI behind.
- **Born-keep-alive has two birth sites and only one got the rule** (2.12.4
  fixed `insert_live_session_with_launch`; live-caught 2026-07-23 that
  `open_or_focus_session`→`build_session` births agent rows blue —
  docs/pending-bugs.md).
- **The cross-pathway switch blinks** (user, 2026-07-23): local-cc → remote-cc
  constructs twice per reveal, generation labels run backward, and the client
  resizes against a `cc-runtime://` key the daemon doesn't know. Same-pathway
  switches are clean. Drift is user-visible as a symptom, not just as
  architecture debt.
- **Codex cold launch painted raw transcript prose with a duplicated turn and
  no TUI chrome** (jojo, 2026-07-23, right after a daemon swap). The daemon
  screen probed later the same day is a correct cursor-addressed TUI — so the
  corruption is an **attach-window** artifact. The duplicated turn is the
  tell: two writers painted the same content (leading hypothesis: the
  retained-replay/snapshot seed AND the CLI's own history repaint, §5.4).

## 2. The contract in one sentence

**An agent CLI is DATA (a descriptor); the harness is CODE (one pathway); and
locality is a TRANSPORT (one wrapper) — nothing else may fork on CLI or on
local-vs-remote.**

Corollaries, each of which is a reviewable rule:

1. **One descriptor per CLI.** All CLI-specific knowledge lives in one
   `AgentCliDescriptor` value (§3). If a function needs to know "is this
   codex or cc," it must get the answer from the descriptor, not from a
   `match` on `SessionKind` scattered at the call site. `SessionKind` remains
   the enum key that *selects* the descriptor.
2. **One transport seam.** `local` and `remote` differ only inside the
   launch-transport function (spawn vs `assemble_remote_ssh_command` →
   `login_shell_wrap`). Above that seam, code may not branch on locality;
   below it, code may not branch on CLI beyond substituting descriptor
   fields.
3. **One scheme registry.** Every session-path/runtime-key scheme is declared
   once, with its role (row identity vs runtime key vs transport), and every
   predicate that filters by scheme derives from the registry. A predicate
   with a hand-written scheme list is a bug (the sanitizer's missing
   `cc-runtime://` is the standing exhibit).
4. **One birth site.** Exactly one constructor births a live agent row;
   every entry point (GUI click, `server connect`, restore, adoption,
   migration, scan promotion) funnels through it. Birth defaults (keep-alive
   for agent kinds) therefore hold everywhere by construction. Restore stays
   authoritative over birth defaults (2.12.4 rule, unchanged).
5. **One content source per attach.** At any instant of an attach, exactly
   one writer feeds the terminal buffer: the seed (replay/snapshot) OR the
   live PTY — with an explicit, traced cutover. Two writers is the
   duplicated-turn cold-launch bug by definition (§5.4).
6. **Parity is the acceptance test, per CLI × locality.** A session opened
   through yggterm must be pixel-indistinguishable (chrome aside) from the
   manual `ssh -t <host> '<cli> resume <id>'` / local `<cli> resume <id>` in
   a clean shell ([[spec-agent-cli-wrapper-render-parity]]).

## 3. `AgentCliDescriptor` — the per-CLI data

One value per CLI, compile-time constructed, owned by yggterm-core (the same
crate that owns `SessionKind`), so every crate reads the same answers.

```rust
pub struct AgentCliDescriptor {
    /// The enum key this descriptor serves.
    pub kind: SessionKind,
    /// Human name for UI ("Codex", "Claude Code", "Kimi", …).
    pub display_name: &'static str,
    /// Where this CLI persists its own sessions on a host, relative to $HOME
    /// (globs). This is the CLI's OWN store — yggterm never writes into it.
    pub session_store_globs: &'static [&'static str],
    /// Extract (session_id, cwd, modified_epoch_ms, title_material) from one
    /// store file. Feeds the cwd tree scanner AND identity rebinding.
    pub read_store_entry: fn(&Path) -> Option<AgentStoreEntry>,
    /// Build the resume argv for an EXISTING session id in a cwd.
    /// The harness — not the descriptor — wraps this for transport
    /// (login_shell_wrap for remote, direct spawn for local) and appends the
    /// terminal-appearance env. Minimum flags only (CLAUDE.md rule).
    pub resume_argv: fn(session_id: &str, cwd: &Path) -> Vec<String>,
    /// Build the argv for a NEW session in a cwd.
    pub launch_argv: fn(cwd: &Path) -> Vec<String>,
    /// True when this CLI re-derives full content from its own store on
    /// resume (all five planned CLIs do). Drives replay policy §5.4:
    /// re-derivable ⇒ the PTY is disposable and rows ride every persist.
    pub content_rederives_on_resume: bool,
    /// OPTIONAL prompt-signature hints for readiness heuristics. Hints, not
    /// gates: absence must degrade to the generic readiness predicate —
    /// remote-cc-replay-codex-only is what a REQUIRED signature does.
    pub prompt_signature_hints: &'static [&'static str],
    /// How "the agent is working" is read for this CLI (store mtime edge,
    /// process presence, output cadence) — one of a small closed enum, not
    /// free code.
    pub working_signal: WorkingSignalSource,
}
```

Explicitly NOT in the descriptor (harness-owned, CLI-independent): keep-alive
default (ALL agent kinds are born keep-alive — that is what first-class
means), migratability (all agent kinds are migratable — they re-derive),
recoverability (derived from `content_rederives_on_resume` + locality),
sanitizer behavior, snapshot/replay mechanics, title/summary LLM calls
(descriptors supply title *material*; generation is one shared chore),
mount/reveal behavior in the GUI.

**The registry:** `AGENT_CLIS: &[AgentCliDescriptor]` in yggterm-core.
`SessionKind::is_agent()` becomes `AGENT_CLIS.iter().any(|d| d.kind == self)`
— adding a CLI without registering a descriptor is impossible by
construction. Non-agent kinds (`Shell`, `SshShell`, `Document`) are outside
this spec.

## 4. Identity: schemes, rows, runtime keys

**Today** (to be confirmed against the divergence inventory in §7): row
identity uses `local://<uuid>`, `remote-session://<machine>/<uuid>` (codex),
`remote-cc://<machine>/<uuid>` (CC); runtime keys use `codex-runtime://` /
`cc-runtime://`; plain shells use `live::`.

**Target:**

- **Row identity = (kind, locality, machine, cli-session-uuid).** The uuid is
  the CLI's own (from its store) — the same uuid the user would type after
  `codex resume` / `claude -r`. One parser, one formatter, in yggterm-core,
  next to the registry. Existing scheme strings stay valid on the wire and on
  disk (no migration of persisted state), but they become *encodings of the
  4-tuple*, produced and parsed in exactly one module. New CLIs get
  `agent://<cli>/<machine|local>/<uuid>` — they never mint a new bespoke
  scheme. (`remote-session://` ≠ `remote-codex://` only for historical
  reasons; the 4-tuple module hides that.)
- **Runtime keys are derived, not invented:** one function
  `runtime_key(row_key) -> String`, one inverse. The cross-pathway
  `remote_pty_resize_failed { "…not found: cc-runtime://…" }` class of bug is
  a client and daemon disagreeing about a derivation that exists in two
  places today.
- **The scheme registry (§2.3)** lists every scheme with its role. Predicates
  (`session_path_is_remote_agent`, the sanitizer's scheme-qualified match,
  recoverability, migratability) are generated from it. Test lock: a unit
  test iterates the registry × the predicates and fails when a predicate's
  hand-list misses a registered scheme — the class of
  `cc-runtime://`-missing bugs becomes unrepresentable.

## 5. The harness pathway — each stage, one owner

The life of an agent session, with the single owning function per stage. The
inventory of today's per-stage forks is §7; the migration order is §8.

### 5.1 Discover (scan)
`build_local_cwd_tree` already does this right locally: descriptors' scanners
feed `LocalAgentSessionSummary` into one builder
([[spec-cwd-tree-agent-cli-unified]]). Remote discovery must become the SAME
scanner run through the transport seam (the remote helper executes the scan
on the remote host and returns the same summary shape) — not a parallel
implementation. One summary shape in, one tree out, regardless of locality.

### 5.2 Birth (row creation)
One constructor (§2.4). `insert_live_session_with_launch` is the incumbent
owner; `open_or_focus_session`'s `build_session` arm must funnel into it (or
both into a deeper common fn). Birth applies: keep-alive-for-agents, trace
event, row-order placement. Restore/adoption paths SET persisted values over
birth defaults (authoritative-restore, three existing tests).

### 5.3 Launch/resume (the product's core promise)
One builder: `descriptor.resume_argv(id, cwd)` → harness adds
terminal-appearance env → transport seam wraps it (`login_shell_wrap` +
`ssh -t` for remote; direct spawn for local). The `--require-existing`
distinction (resume must not silently create) is a harness flag, not per-CLI
code. **No CLI flags beyond the minimum** (CLAUDE.md): cwd, session id,
terminal-appearance env. The remote resume wrapper (`server remote
resume-cc|resume-codex`) collapses to `server remote resume <kind> <uuid>
<cwd>` internally routed through the same descriptor (old subcommand names
stay as aliases; scripts and Connect strings in the metadata rail keep
working).
**Deadline rule** (from the `resume-cc` deadlock, docs/pending-bugs.md): every
wrapper step that talks to a daemon socket carries a connect/read deadline;
a wrapper may never block forever *before* spawning the CLI. A lease whose
attach never reached ready is reclaimable, not deferred to forever.

### 5.4 Attach (seed → live cutover) — the cold-launch acceptance
One content source at a time (§2.5):

1. Attach opens with the SEED phase: retained replay or daemon-screen
   snapshot bytes — whichever the existing reconcile logic picks — written
   to the client buffer, traced with byte counts.
2. The moment live PTY bytes for this incarnation begin, the seed phase is
   CLOSED (traced cutover). For a **fresh incarnation** (cold launch,
   re-resume after migration/swap): if `content_rederives_on_resume`, the
   CLI will repaint everything it needs — so the seed for a fresh
   incarnation is at most the daemon's CURRENT screen (never a byte more),
   and when the CLI opens with home+clear (both codex and CC do), the seed's
   only job is to prevent a blank flash. Replaying a PRIOR incarnation's
   retained chunks into a fresh incarnation's buffer is forbidden — that is
   the duplicated-turn hypothesis, and even if the hypothesis is wrong for
   this instance, two-writers must die as a class.
3. Readiness gates use descriptor `prompt_signature_hints` as accelerators
   only; the fallback is CLI-agnostic (settle-gates, first-bytes,
   post-resize output — machinery that already exists).

### 5.5 Preserve (daemon swap, migration, GUI death)
Already CLI-agnostic in design; the spec makes the inputs uniform:
migratability = `is_agent` (all descriptors), recoverability = derived,
advertisement/adoption (B4) carries rows independent of CLI. A plain shell
still pins its daemon (lossless fd-handoff remains out of scope). Nothing in
preserve may consult anything but the descriptor-derived flags.

### 5.6 Extract (title, summary, working, notifications)
One chore reads `descriptor.read_store_entry` output for title material; one
LLM pipeline generates titles/summaries under the existing 3-per-tick / stop
on 429 budget; `working_signal` comes from the descriptor's declared source.
No per-CLI chore forks. (Local reads the store directly; remote reads it
through the SAME helper that runs the scan — locality stays under the seam.)

### 5.7 Render (GUI)
Out of scope for per-CLI logic entirely: the GUI terminal path renders a PTY
and may not know which CLI is behind it. Mount/reveal fixes (the
double-construct on cross-pathway switch) are GUI bugs to fix once, not per
pathway. Kind may drive ICON/COLOR only, per [[spec-unify-local-remote]].

## 6. Acceptance cases (all must pass before any new CLI lands)

| # | Case | Method |
|---|------|--------|
| A1 | Cold-launch parity, codex, remote: open a codex session cold through yggterm right after a daemon swap; daemon screen and faithful pixel are indistinguishable from manual `ssh -t <machine> codex resume <uuid>` (no duplicated turn, composer present) | wrapper-vs-manual diff per [[spec-agent-cli-wrapper-render-parity]]; the 2026-07-23 gibberish screenshot is the failure exemplar |
| A2 | Cross-pathway switch: local-cc → remote-cc reveal mounts ONCE (one `constructed` per reveal), no generation regression, no `remote_pty_resize_failed` on runtime-key mismatch | trace assertion during a scripted switch (the 2026-07-23 episode's trace is the failure exemplar) |
| A3 | Same fix, four arms: a readiness/replay/sanitizer change verified on ALL of {local,remote}×{cc,codex} in one pass | the four-arm test matrix (§8 phase 2) runs per PR touching the harness |
| A4 | Born-green everywhere: an agent row born from EVERY entry point (GUI new, GUI click on scanned row, `server connect`, restore, B4 adoption, migration) reads keep-alive=true unless the persisted value says otherwise | unit tests per entry point + live `server app rows` diff after exercising each |
| A5 | Scheme completeness: registry × predicate lock (§4) green | unit test |
| A6 | New-CLI drill: adding a stub descriptor (fake CLI echoing a marker) yields a working end-to-end session on local AND remote with ZERO changes outside the descriptor + the enum + UI glyph dispatches | the kimi pilot (§8 phase 5) proves it for real |

## 7. Divergence inventory (where the four pathways fork today)

**This is the code map for whoever implements — surveyed 2026-07-23 against
main @ 5f304e0.** Line numbers will drift; names won't. `server` =
crates/yggterm-server/src, `shell` = crates/yggterm-shell/src.

### 7.1 Schemes: constructors, parsers, roles

Constructors: `local_live_runtime_key` (`local://`, server/lib.rs:1645),
`remote_scanned_session_path` (`remote-session://`, lib.rs:8112),
`remote_cc_session_path` (`remote-cc://`, lib.rs:13031),
`remote_runtime_codex_session_key` / `remote_runtime_cc_session_key`
(`codex-runtime://` / `cc-runtime://`, lib.rs:8137/8145), `live::` and
`document::` (lib.rs:5910). **Legacy parse-only aliases still live:**
`codex://`, `codex::`, `codex-litellm://`, `codex-litellm::`, `local::`
(lib.rs:1634) — the registry (§2.3) must carry them as aliases so no parser
hand-lists them again.

Roles: `remote-session://`/`remote-cc://` are ROW identity; `*-runtime://`
are RUNTIME keys; **`local://` is BOTH for local rows** (one string, two
roles — the registry must model that, not paper over it). The one existing
per-kind SSOT island to grow: the kind→scheme table at lib.rs:8153–8200
(`remote_runtime_agent_session_key`, `remote_agent_resume_subcommand`, …).

### 7.2 The `cc-runtime://` hole (one bug class, seven+ sites)

Predicates that recognize `codex-runtime://` but NOT `cc-runtime://` (and
usually not `remote-cc://`):

| predicate | site | consequence |
|---|---|---|
| `local_runtime_id_from_key` | lib.rs:1634 | cc-runtime keys unrecognized by recoverable/snapshot predicates + restore normalizers (lib.rs:4505/4541/4558 have codex-runtime branches only) |
| `uses_runtime_owned_terminal_path` | daemon.rs:705 | CC daemon-owned runtimes miss runtime-owned handling |
| `terminal_key_prefers_initial_screen_snapshot` | terminal.rs:2578 | CC attaches don't get the codex initial-snapshot seed policy |
| `launch_command_looks_like_remote_resume_attach` | terminal.rs:2554 | matches `resume-codex`/`start-codex` only — `resume-cc`/`start-cc` invisible |
| `bridge_initial_snapshot_should_use_raw_stream` | lib.rs:15185 | codex bridges delay raw stream, CC bridges take a different path |
| `terminal_line_internal_transport_error_index` | shell.rs:73310 | a real `…not found: cc-runtime://…` transport error is NOT excised |
| `terminal_line_is_internal_transport_error` (SSOT twin) | terminal_observe.rs:3702 | same hole, second copy |
| `is_hot_terminal_sidebar_path` | shell.rs:25820 | includes remote-cc but not cc-runtime |

This table is the registry lock's (§2.3, A5) initial work-list.

### 7.3 Remote-resume readiness is codex-only

`is_remote_resume_agent_session` (shell.rs:62581) checks `remote-session://`
+ LiveSsh **only**. It drives `is_remote_resume_session` (shell.rs:63539)
which flips ~8 readiness signals (`terminal_has_meaningful_output`,
`terminal_overlay_dismissed`, `terminal_live_host_connected`,
`terminal_resume_surface_staged`, `attach_ready`, `connected_for_resume`,
`stalled_remote_resume`, `transport_error_after_attach`, read-poll cadence).
**A `remote-cc://` session gets NONE of the remote-resume overlay/readiness
path.** Same family: `is_remote_scanned_live_session_path` (lib.rs:327),
`is_remote_scanned_sidebar_row` (shell.rs:25879),
`terminal_session_uses_remote_runtime` (shell.rs:12987),
`remote_session_starts_new_codex` cold-launch discriminator (shell.rs:73820
— no `start-cc` twin). Retained-chunk preservation gates on
`remote-session://` only: `initial_remote_attach_should_preserve_retained_chunks`
/ `select_remote_retained_initial_chunks` (terminal.rs:2584/2599).

### 7.4 Launch/resume construction asymmetries

- **Born-identity:** CC launches with `--session-id <uuid>` (row id ==
  transcript id from birth; lib.rs:20822, 21666, 4835 — deliberately no
  rebind poll). Codex launches bare and discovers its id later
  (lib.rs:20806). The descriptor absorbs this as data
  (`launch_argv` + whether `read_store_entry` id is pre-assigned);
  the harness must not care.
- **Resume verb shape:** codex `resume -C "$PWD" <id>` subcommand
  (codex_cli.rs:1579); CC `--resume <id>` flag (codex_cli.rs:1554–1577).
  Descriptor data.
- **CC-only env plumbing:** `YGGTERM_CC_EXTRA_ARGS` / `claude_extra_args`
  threaded end-to-end (codex_cli.rs:1600, lib.rs:9486, 13568, 13629, daemon
  request param) with no codex analog — generalize to a descriptor-supplied
  extra-args var or delete.
- **CC-local resume consults the transcript store for cwd and may re-birth**
  (lib.rs:21776–21786); codex has no equivalent.
- All four remote arms already converge on ONE stdio bridge
  (`bridge_remote_runtime_session_stdio`, lib.rs:14935) and one ssh wrapper
  (`assemble_remote_ssh_command` → `login_shell_wrap`, lib.rs:9439) — the
  transport seam (§2.2) already exists; the forks are above and below it.

### 7.5 Identity rebinding: codex has three mechanisms, CC has one

Codex: local fd-walk rebind
(`refresh_live_codex_runtime_identities_for_persistence`, daemon.rs:3140),
remote cwd-paired identity poll (`match_codex_identities_to_targets`,
daemon.rs:8310; gated on `looks_like_synthesized_uuidv4_session_id`),
per-snapshot overlay (`overlay_codex_runtime_snapshot_session`,
daemon.rs:3098). CC: local fd-walk rebind only (daemon.rs:3181) — the remote
poll is unnecessary BY DESIGN (born with `--session-id`), which is the
correct shape: **descriptor data (`id pre-assigned: yes/no`) decides whether
the poll runs, instead of a per-CLI code fork.** Kimi/antigravity/opencode
get whichever they need by declaring that one bit. (Cleanup: the CC daemon
handler still calls codex-named `adopt_legacy_local_codex_runtime`,
daemon.rs:6543.)

### 7.6 Attach seed: the decision tree that must become §5.4's single-writer

The daemon-side seed for an attach is chosen in `PtySession::read()`
(terminal.rs:1844): retained-chunk merge vs initial-attach chunks vs screen
snapshot vs viewport-reconcile chunk, with codex-specific exclusions
(terminal.rs:1914/1937 — codex resume attaches skip the reconcile chunk
because their vt100 can be staler than the retained tail). The client-side
reveal separately picks `daemon_retained_snapshot` vs
`daemon_screen_snapshot` (shell.rs:65067, 73020), where the authoritative
screen fallback is `codex_like`-gated (terminal_retained_replay_policy.rs:25
— CC only gets it in CollapsedScrollbackRecovery mode; this is the
`remote-cc-replay-codex-only` / snapshot-poison axis). Mid-stream gap
resync is detected but deliberately a no-op (terminal.rs:1855–1864) — the
seam suspected in the live-path corruption entry (docs/pending-bugs.md).
**Two writers exist by construction today: daemon seed + client reveal
replay both write into the same buffer under different policies.** §5.4
collapses this to one traced seed phase with one cutover.
(Confirmed NOT a suspect: nothing parses CLI JSONL into the terminal buffer;
the historical buffer-leak fns are neutered — shell.rs:73750–73793. The
remaining raw-seed risk is the `terminal_lines` overload: the daemon seeds
launch boilerplate into `session.terminal_lines` (lib.rs:21464 + ~8 seed
sites) which `local_terminal_prefill_text` (shell.rs:75173) writes into
xterm, guarded only by `terminal_chunk_is_daemon_launch_seed` — the
seed-connection-state class. Stop overloading `terminal_lines`.)

### 7.7 Sanitizer: content-gated, scheme-incomplete, codex-flavored

`batch_terminal_chunks` (shell.rs:72923) receives chunks WITHOUT a session
key — so per-pathway gating is impossible at the callsite today, and its own
comment (shell.rs:72934) already names the real fix: per-session attach-phase
state (sanitize only while the launch wrapper owns the PTY) — which is
exactly §5.4's seed/live cutover. Additional codex flavor:
`sanitize_terminal_resume_runtime_output` is content-gated on the codex
banner (shell.rs:73390–73391) with no CC twin;
`terminal_chunk_is_transport_error` hardcodes "saved codex session"
(terminal_observe.rs:3655).

### 7.8 Title/summary: three collector forks + two transports

CC titles are CLI-store-authoritative with no LLM (local:
`collect_live_cc_title_syncs`, daemon.rs:7694, gated to `local://` +
`cc-runtime://`; remote: `collect_remote_cc_title_syncs`, daemon.rs:7859,
ssh + `REMOTE_CC_TITLE_SCRIPT`). Codex titles/summaries go through the LLM
chore (`build_background_copy_updates`, daemon.rs:7999) with three candidate
collectors (local stored / live local / remote) and a context-source fork
(`copy_target_context`, daemon.rs:7961). Target shape (§5.6): ONE collector
walking descriptors — `read_store_entry` supplies title material and
declares whether the store is title-authoritative (CC) or the LLM generates
(codex); locality picks the transport, not the collector.

### 7.9 Working signal: uniform mechanism, codex-flavored matcher

`working_flags` (daemon.rs:2803) is already kind-uniform for agents (screen
text via `screen_text_shows_agent_working`). But the SSOT matcher
(core/lib.rs:1251) knows `"esc to interrupt"` (both CLIs) plus Codex-only
extras (`"working ("`, `"/stop to close"`, `"background terminal running"`)
— a codex background task reads as working where the CC equivalent would
not. These phrase lists are descriptor data
(`working_signal`/`prompt_signature_hints`).

### 7.10 Mount/reveal (GUI): the per-pathway locals that A2 kills

Mount computes pathway-flavored locals: `is_remote_resume_session`
(remote-session-only, §7.3), `codex_like_session` (excludes CC,
shell.rs:63585), `remote_starting_codex_session` (shell.rs:63543), and the
placeholder fork local-prefill vs remote-none (shell.rs:63575). The
anti-churn mount-epoch machinery (`resolve_active_open_mount_epoch`,
shell.rs:12926 — reveal-in-place vs cold remount, futile-remount cap) is
where the cross-pathway double-construct lives; the m1/m2 generation labels
come from `terminal_session_host_id` (shell.rs:12981). The
`remote_pty_resize_failed {…cc-runtime://…}` half of the blink is NOT a
scheme bug anymore (`resize_remote_agent_session_pty`, lib.rs:20067, keys
per kind correctly) — it fires because the remote daemon doesn't own the
key yet after a local→remote switch, i.e. an ORDERING bug in the switch
flow, not a naming bug.

### 7.11 Already-unified islands (build on these, don't reinvent)

`remote_runtime_agent_session_key` + the kind tables (lib.rs:8153–8200);
`remote_agent_pty_target_for_path` (lib.rs:3031);
`resize_remote_agent_session_pty` (lib.rs:20067); the single stdio bridge
(lib.rs:14935); the single ssh wrapper + login-shell wrap (lib.rs:9439);
`working_flags`' agent arm (daemon.rs:2809); the D5 dual-caret replay
recognizers (shell.rs:87143/87236); the top-level persistence gates
(`session_kind_is_migratable_agent`, `session_kind_persists_by_default`,
`session_path_is_remote_agent`, `persisted_live_session_is_recoverable`) —
these already cover all four pathways correctly.

## 8. Migration order (each phase shippable alone, oldest debt first)

0. **Scheme registry + predicate locks** (pure addition, no behavior change;
   immediately catches the sanitizer's missing `cc-runtime://`).
1. **Descriptor extraction**: introduce `AgentCliDescriptor` for codex + CC;
   port `resume_argv`/`launch_argv` construction and the store scanners onto
   it; delete the per-arm builders as each caller moves. No wire changes.
2. **Four-arm test matrix**: the A3 harness (jsdom/PTY-level where possible)
   so later phases can't regress one arm silently.
3. **Birth-site collapse** (fixes the standing keep-alive bug as a
   by-product) + **attach single-writer** (A1, A2 close here).
4. **Extraction unification** (title/summary/working via descriptor).
5. **Kimi pilot** (A6): first new CLI lands descriptor-only; then
   antigravity, opencode.

Phase 3 is where the two user-visible pending bugs die; phases 0–2 are
prerequisites that keep it honest. Nothing here blocks unrelated bug work.

## 9. Out of scope

- The web/chat pretty-view of JSONL (separate surface, active development).
- Parsing CLI JSONL into the terminal viewport (explicitly forbidden by
  CLAUDE.md — the CLI paints its own TUI).
- Lossless fd-handoff for plain shells (tracked in the campaign, not an
  agent-CLI concern).
- Adding CLI flags beyond cwd/uuid/appearance env to any resume command.
- Second-class (shell) session behavior, except where a predicate shared
  with agents comes from the same registry.
