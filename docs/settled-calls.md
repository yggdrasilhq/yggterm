# Settled calls — the user's own answers, not to be re-litigated

These were decisions, not defects, and they lived in the bug queue where they
outnumbered the bugs. They are the user's stated intent, quoted; where a clause
is still unbuilt it is filed as a bug in [`pending-bugs.md`](pending-bugs.md)
and referenced from here.

⛔ Do not re-open a call in this file by argument. Re-open it only if the USER
says the answer changed.


## ★★★ THE SWEEP SYSTEM — INVISIBLE BY DEFAULT, AND IT MAY REWRITE A TRANSCRIPT (2026-08-08)

**Settled by the owner, verbatim on the shape:** *"I mostly want the processes
to be automatically taken care of without me being ever informed. But I can
manually request too."*

Design and thresholds: [`spec-sweep-policy.md`](spec-sweep-policy.md). Three
clauses were the owner's own calls and are not to be re-litigated:

1. **Compaction may rewrite an agent CLI's transcript, because yggterm restores
   it before handing off.** Offered as three options (never touch a transcript /
   one-way strip to a placeholder / externalize with rehydration on resume) and
   he chose **rehydration**, accepting that the failure mode lands on the resume
   path in exchange for being lossless. ⛔ Conditional on the spec's §5 law: no
   original is released until a rehydrated copy has been read back and matched
   against its pre-strip digest.
2. **Build budgets:** `incremental/` swept **daily**, `target/debug` capped at
   **40 GB** with oldest-first eviction past the cap. Chosen over the
   conservative arm (incremental only) and the aggressive one (purge any debug
   tree idle 30 days), on the integrator host that regenerates ~13 GB/day.
3. **The `emd-renderer` low-resolution webview cache stays infinite and is never
   swept.** His words: *"I think it should stay this way."* ⚠ The cache does not
   exist yet; the exemption is recorded ahead of it on purpose, so that whoever
   builds it does not find it swept by an engine that shipped first.

**The silence rule is part of the ruling, not a default.** C0/C1/C2 never speak;
C3 writes one trace line; only `degraded` reaches him, because a sweep that
could not verify what it was deleting is the one state where silence is a false
report of health.

## ⛔⛔ yggterm AUTO-INSTALLS AND AUTO-UPDATES **EVERY** CLI, ON EVERY CONNECTED SYSTEM INCLUDING LOCALHOST (2026-08-08)

**Settled by the owner, verbatim:** *"Trying to launch Muse CLI shows
notification to install it. yggterm should auto install, update ALL clis in all
connected systems including localhost."*

**This overrides two standing design decisions, both of which must now change:**

1. **`CliInstall::VendorScript` no longer means "record the URL and refuse".**
   Its doc comment says *"yggterm records it so the provisioner can name what is
   missing, and does NOT run it unattended"* — that clause is superseded. Muse
   (`https://dev.meta.ai/install.sh`), and every other vendor-script CLI,
   is to be installed automatically.
2. **Muse is NO LONGER AN OWNER GATE.** `owner-attention.md` parked it as
   needing his login first; he has now ruled that the INSTALL must happen
   unattended. (A LOGIN may still be his — installing a CLI and authenticating
   it are different acts — but the install must not wait on him.)

**"update" is half the ruling and is the half with no code at all today.** The
provisioner installs on demand at launch; nothing keeps an already-installed CLI
current, on any host. Both verbs are owed, on **all connected systems including
localhost** — localhost named explicitly because the fleet paths and the local
path are different code today.

⚠ **Stated once, not as an objection:** this makes yggterm pipe a vendor's
`install.sh` from the internet, unattended, on every machine it touches. That is
his call to make on his own fleet and it is made; the implementation should still
prefer the CLI's own updater where one exists, keep every install USER-LOCAL (the
existing `⛔ never sudo` rule is untouched), and record what it ran.

Build state: [`pending-bugs.md`](pending-bugs.md) ▸ *AUTO-PROVISIONING…*.


## ★★ `codex-anything` IS NOT A CLI — IT IS A CODEX SESSION'S FLIP SWITCH (2026-08-08)

**Settled by the user, verbatim:** *"we should not have codex-litellm as another
CLI. It is a special codex session flip switch to codex-litellm (which I will
later rename to codex-anything). It is a codex ONLY sessions superpower."*

⛔ **THE NAME IS LOCKED — same day, second directive:** *"Everywhere start
referencing codex-litellm as codex-anything from now on. The name is locked but
the repo and binary is still called codex-litellm."*
⇒ **`codex-anything` is the name in every surface a human reads** — UI labels,
docs, specs, commit messages, this file. **`codex-litellm` survives only as an
identifier**: the repo, the binary at `~/.yggterm/npm/bin/codex-litellm`, the
provider key in `~/.codex/config.toml` (`[model_providers.litellm]`), and any
path or package name. ⛔ Do not rename the binary to chase the label, and do not
let the label leak back into an identifier — that is how one thing becomes two.

⇒ It is **not** a first-class agent CLI, it gets **no** row in the
`Open Session Here ▸` submenu, **no** icon of its own, and **no** row in the
extra-args modal (`spec-agent-cli-extra-args-modal.md`). It is a *mode a codex
session can be flipped into*, and the flip belongs to the codex session's own
surface.

⚠ It is a `--kind` value today (`codex, codex-litellm, claude-code, pi,
opencode, qwen-code, kimi, muse, antigravity`) and has its own provisioned
binary at `~/.yggterm/npm/bin/codex-litellm` on every host. Removing it from the
kind list without removing the capability is the work; it is filed in
[`pending-bugs.md`](pending-bugs.md).

⚖ The flip belongs to the codex session's own surface: a **codex ↔ Anything**
slider in settings (`spec-settings-model-providers.md`), never a separate row in
the session menu.

## ★★★ NO REPO MAY CARRY TWO LICENCE CLAIMS, AND GADGETS GATES GOING PUBLIC (2026-08-07)

**Settled by the user, verbatim:** *"Such duplicity of licenses must be force
push delete cleaned and only then proceeded. In case on any new projects
entering the public space always gadgets should analyze what the license
should be and then we proceed."*

Two rules, and they answer different questions.

1. **Duplicity is cleaned by rewriting history, not by fixing forward.** A repo
   whose `LICENSE` says one thing and whose `README`/`AGENTS.md` says another is
   not "mostly right" — it is two answers to one question, and the permissive
   one is the one a reader relies on. The fix is a history rewrite plus a force
   push, so no commit anywhere in the lineage carries the contradiction.
2. **Nothing of his goes public until gadgets has decided its licence**, and
   the answer is a row in `~/git/gour.top/docs/venture/ip-register.md` (§THE
   LAUNCH GATE, Step 0). Steer phrase: *"continue the gadgets campaign"*.

⚖ **This overrode a verdict an agent had written the same day** — *"fix forward
+ a dated NOTICE erratum; never a rewrite, never a public statement."* That
verdict is still correct about what a rewrite achieves **legally**: relicensing
retracts no grant already made, and a rewrite only shrinks the discoverable
surface. It was answering "can we take the offer back", and the answer is still
no. The user is answering a different question — "may the repo keep contradicting
itself" — and the answer is no. Both hold, and the honest record of each
exposure window lives in the register, which is private.

⛔ **Do not add a public erratum when cleaning duplicity.** An erratum
re-publishes the claim it corrects, which is the opposite of the instruction.
The record goes in the register.

**Executed the same day:** yedit rewritten across all 13 commits and
force-pushed (tip `fbf1540`); cellulose and paper **deleted** at his direction
(*"far far superceded in concept"*, cellulose's name reserved for a fresh repo
later); charts's `app/` carved out of its root Apache grant; the orphaned Apache
`yggdrasil-build-main` tree deleted. Full detail in the register's §THE LICENCE
SWEEP.

## ★★★ A RUNNING SESSION KEEPS ITS START-PAGE CARD (2026-08-06) — REVERSES A 2026-05-26 CALL

**Settled by the user, verbatim:** *"when I launch a session that entry drops
from the startpage. This is buggy behavior. If the session is open it should
switch to that not LIE about sessions present."*

This **reverses** the 2026-05-26 call that live sessions are deliberately absent
from start-page recents. That call is not being re-litigated by argument — the
user changed the answer, which is the only thing that may reopen one.

Worth recording *why* the old call stopped holding, because the reasoning was
sound when it was made. Its stated premise was that live sessions are "already
surfaced in the Live Sessions sidebar group", so the start page did not need to
be a third presence. On 2026-08-06 a daemon-reachability bug emptied that
sidebar group, and the strip turned one broken surface into total invisibility:
the sessions were running, and no surface in the product admitted they existed.
A rule that is only correct while another surface is healthy is a rule with a
hidden dependency.

- **A live session keeps its start-page card**, and it sorts ABOVE stored ones —
  running-now outranks a file mtime, and live rows frequently carry no mtime.
- **Opening that card SWITCHES to the running session**; it never starts a
  second copy.
- **One session, one card.** A live row and its stored transcript row collapse,
  deduped on session id rather than path, because no path normalization relates
  `local://<id>` to `~/.codex/sessions/<id>.jsonl`.
- Unchanged: the 2026-05-25 dual-presence claim (Live Sessions group + cwd tree)
  and start-page recency ordering below the live block.

## ★★★ 3.0.0 IS THE libyggterm SEPARATION RELEASE (2026-08-02)

**Settled by the user, verbatim:** *"I previously reserved v3 for all platform
builds but libyggterm MPL separation is the major change in our repos."*

This **redefines** 3.0.0. It was scoped as the Windows/macOS platform-builds
release; it is now the release that separates `libyggterm` (MPL-2.0) from
yggterm (GPL-3.0-or-later). Do not re-open it by pointing at the old scope.

- **3.0.0 gates on:** the rewire building, suites green, install path verified
  on the release lane. Linux.
- **3.0.0 does NOT gate on:** Windows or macOS building — those become 3.x
  milestones; nor on eMudhra/TM-A, which gates Substack posts naming yggterm
  and never gates a release.
- **Releases are load-bearing** (yggclient pins fetch them): additive only,
  never delete or replace an old release.

The estate side of the same call, so it is not re-derived: the cut is `yggui` +
`yggui-contract` **only**. `emd-renderer` and `yggterm-platform` stay GPL —
only what third-party apps must LINK goes MPL, because everything under MPL can
be combined into proprietary works, so each crate moved out of the GPL fence is
moat spent. Under-cutting is reversible in one commit on sole copyright;
over-cutting is a one-way door.

⚠ **The `emd-renderer` half of that sentence was superseded by the user the
same day — see the next entry.** It is left standing rather than edited,
because this file records what he settled and when; what it must not do is
answer the licence question twice. For `emd-renderer`, the next entry is the
answer. `yggterm-platform` still stands exactly as written.

## ★★★ `emd-renderer` IS A PLATFORM LIB — MPL, IN libyggterm (2026-08-02)

**Settled by the user, later the same day, and it OVERRIDES the entry above
where that entry says `emd-renderer` stays GPL.** The earlier call named the
crate as staying inside the GPL fence; the user then applied the
licence-by-role rule to it directly: emd is a **platform organ of the app
pipeline**, and its intended consumers are every pipeline app — yedit and
ztlkasten's document surfaces, breezed, charts-webapp. A library those apps must
LINK is MPL by the same rule that put `yggui` there. It is not a part of the
terminal that happened to be reused.

Executed 2026-08-02: the crate and its spec moved to
`yggdrasilhq/libyggterm` (MPL-2.0, tag `v0.3.0`) and yggterm consumes it as a
pinned git dependency like the rest of libyggterm. `yggterm-platform` is
untouched by this call and stays GPL.

⛔ Do not read the two entries as a contradiction to be resolved by argument.
The first is the narrow-cut *rule*; this is the user applying that rule to one
crate whose role he restated. Only the user moves another crate.

## ★★★ USER-SETTLED CALLS + FEATURE REQUESTS

**★★★ USER-SETTLED CALLS + FEATURE REQUESTS (2026-07-26, verbatim intent).**
These answer questions an agent asked; do NOT re-litigate them.
1. **PLAIN SHELLS ARE FIRST-CLASS AND MUST SURVIVE A DAEMON BUMP.** Settled by
   the user. The 2.12.15 bump lost `local://b7ccbab4` ("ychrome HTTP Fixture
   Support") because a plain shell cannot migrate — no `SCM_RIGHTS` fd passing
   exists anywhere in the tree, so the only way to move a PTY is
   kill-and-re-resume, and a shell is not re-resumable. **That is now a BUG,
   not a documented limitation.** Two levels of fix, both wanted: (a) the ROW
   must survive even when the PTY cannot, so the user can restart it with a
   click; (b) properly, lossless fd handoff so the PTY survives too.

   **LEVEL (a) IS BUILT — ⚠ NOT LIVE-VERIFIED (the deploy happens after the
   lane that wrote it, so this entry STAYS until a real bump proves it).**
   What the cause chain actually was, and where each half is fixed:
   - The predecessor advertises its ROUTINE persist as `live_terminal_sessions`,
     and a routine persist synthesizes no `restore_reason` — so the successor
     adopted the shell (correctly: the peer still owned its PTY, which is the
     `peer_live_row_is_adoptable` rescue arm) but the adopted row carried no
     record of WHY it was there. Fixed by `peer_live_rows_marked_as_rescued`
     in `adopt_missing_live_session_rows_from_reachable_daemons`: rows the
     peer still owns are stamped with the handover restore reason before
     admission. The ownerless-agent arm is deliberately NOT stamped.
   - Then, the moment the predecessor retired and the PTY died,
     `apply_terminal_runtime_truth_to_snapshot` erased the row: a shell has no
     agent-store arm to fall back on. Fixed by
     `snapshot_session_is_handover_orphaned_row` — a row that crossed a daemon
     handover survives its runtime as a RUNTIME-LESS row
     (`TerminalLaunchPhase::RemoteBootstrap`, the same shape agent rows
     already use), and the click resolves through the ordinary
     `terminal_spec` → shell launch command at the recorded `Cwd`. Scrollback
     is lost at this level; the row, title, cwd and POSITION are not.
   - The discriminator is deliberately narrow: **a shell whose own PTY exited
     (the user typed `exit`) is still a husk and still disappears.** That is
     the same class `peer_live_row_is_adoptable` refuses, and widening the
     arm to "any Shell" would put jojo's three ownerless loopback shells
     (`local://3803a7ed`, `local://5220ce5d`, `local://a689ee28`) back on
     screen. It also does NOT weaken keep-alive: `PrepareClientClose` still
     removes a non-keep-alive shell from the live order outright, so a GUI
     close never reaches this filter.
   - **Known residual, level (a):** the mark is only re-applied on an
     update-restart persist and on a rescue adoption. A daemon killed WITHOUT
     a handoff (`kill -TERM`, a crash) writes only routine persists, so a
     shell row cold-restored from that file has no mark and is still hidden.
     Closing that means letting the routine persist carry the row's existing
     `Runtime Restore Reason` metadata instead of re-deriving it — cheap, but
     it also widens `live_session_restart_protected`, so it was left out
     rather than guessed at.
   - **Second residual:** nothing CLEARS the mark once the successor spawns
     its own PTY for the row, so a rescued shell the user later exits keeps a
     runtime-less row instead of becoming a husk. That reads as correct under
     call #4 ("no runtime is none of our business") and as a husk under
     requirement 3 below — which is exactly the question requirement 3 says to
     ask the user rather than guess. Clearing it at the `ensure_session`
     chokepoint is the obvious fix once that answer exists.

   **LEVEL (b) — LOSSLESS `SCM_RIGHTS` FD HANDOFF: where it would slot in.**
   ⏳ **INCREMENT 1 IS MERGED (2.12.17); INCREMENT 2 IS NOT BUILT.** Nothing
   is wired into the handoff yet, so level (b) changes no behaviour today —
   no PTY has ever moved. The map below is unchanged and is still the sizing
   document.
   - **Increment 1, merged — the child handle learns Owned vs Adopted.**
     `PtyChildHandle` is `Owned(Box<dyn Child>)` vs
     `Adopted { pid, start_time }`, and every call site is taught which it
     holds: `is_running()` replaces `try_wait().is_none()` everywhere, because
     the old shape forced every caller to think in exit statuses, which an
     adopted child can never supply. Three rules are enforced rather than
     described — an Adopted child NEVER reports an exit status (fabricating a
     success would be worse than returning nothing); killing it is explicit,
     since dropping the master only SIGHUPs the foreground group; and identity
     is **(pid, start_time)**, never the pid alone, gating SIGNALLING as well
     as reporting, which is the assertion that actually prevents killing a
     stranger after PID reuse. Found while building, not in the spike: an
     adopted child has a ZOMBIE WINDOW nothing reaps on our behalf, so `/proc`
     state `'Z'` must read as dead or every shutdown path waits out its full
     timeout on an already-dead process. `ReceivedMasterPty` — the master type
     `portable_pty` cannot build (`UnixMasterPty` and `PtyFd` are private and
     `openpty()` always creates a NEW pair) — is in-tree and under test but
     deliberately unused: `F_DUPFD_CLOEXEC` never plain `dup` (a plain `dup`
     leaks the master past exec, the slave's hangup never arrives and the
     shell never sees EOF), `EIO` mapped to EOF exactly as `PtyFd` does, and
     dropping the writer sends newline + the termios `VEOF` byte so the
     trait's documented EOF contract still holds. The adoption machinery is
     Linux-gated **at the variant**, so the module compiles on every target
     rather than only the one it was written on.
   - **Increment 2 — the `HotRestart` `sendmsg` wiring — is integrator-gated
     and NOT built.** Two decisions are already settled and should not be
     re-litigated when it is: the transcript payload travels BEFORE the fd,
     and **`sendmsg` success is the commit point** — after it the fd belongs
     to the successor, so nothing downstream may be recovered by re-sending.
   - **Who owns the fd.** `PtySessionRuntime` in
     `crates/yggterm-server/src/terminal.rs` holds
     `master: Arc<Mutex<Box<dyn MasterPty + Send>>>`, and
     `TerminalManager { sessions: HashMap<String, PtySessionRuntime> }` is the
     map keyed by runtime key. The raw fd is already reachable —
     `master.as_raw_fd()` is what `foreground_process_group_leader` uses for
     `tcgetpgrp` — so the SEND side needs no new plumbing into the pty layer.
   - **Who owns the child.** `PtySessionRuntime.child:
     Arc<Mutex<Box<dyn Child + Send + Sync>>>`. This is the part that does not
     travel: a `Child` handle cannot cross a process boundary, and the shell
     is the predecessor's direct child. After the fd moves, the successor can
     drive the PTY but cannot `waitpid` it; the predecessor must either stay
     alive as a reaper until it exits (defeats the point) or the child must be
     re-parented to init and the successor must fall back to
     `kill(pid, 0)` / `/proc` liveness. **Decide this before writing any
     `sendmsg`** — it is the actual design question, not the ancillary data.
     ✅ **DECIDED and built in increment 1:** re-parent to init, no lingering
     reaper, `/proc` liveness keyed on (pid, start_time).
   - **Who owns the scrollback.** The reader thread plus `chunks`,
     `seq`, `retained_bytes` and `spawn_id` on the same struct. The fd alone
     hands over a live terminal with an empty transcript, so the ring has to
     travel beside it (the existing `terminal_snapshot` payload is the obvious
     carrier) or the user gets a working shell with no history — barely better
     than level (a).
   - **Where `sendmsg` would live.** The wire is one JSON line per request over
     a `UnixStream` (`read_request` / `write_response` in `daemon.rs`), which
     has no room for ancillary data — `SCM_RIGHTS` needs a real `sendmsg`
     on the same socket, so this must be an out-of-band step on the handoff
     connection, not a new `ServerRequest` field. The natural site is the
     `ServerRequest::HotRestart` preserving-handoff branch in `daemon.rs`,
     immediately where it calls `PreservedTerminalOwnerRegistry::write_handoff`
     — that registry (`hot-update-terminal-owners.json`, runtime key → owner
     socket + pid) is already exactly the list of fds that would be sent, and
     `attempt_self_retire_preserving_handoff` is the caller that reaches it on
     a `disk_binary_replaced` retire.
   - **The receive side is the expensive half.** `portable_pty`'s `MasterPty`
     is a trait object with no `from_raw_fd` constructor, so the successor
     cannot rebuild a `PtySessionRuntime` from a received fd without either a
     local Unix master type implementing the trait or a fork of the pty layer.
     Budget the work there, not in the socket call.
   - **What it would retire.** `session_kind_is_migratable_agent` could then
     admit `Shell`, and `progressive_migration_session_released` would stop
     being kill-and-re-resume for every kind — which is also what unpins the
     supernumerary daemons that one idle `bash -i` keeps alive forever.
2. **THE ROW-ORDER LEDGER WAS WRITE-ONLY ON RESTORE. ✅ FIXED AND PROVEN LIVE
   ACROSS THE 2.12.16 DAEMON BUMP — 22 rows before, 22 after, ORDER IDENTICAL,
   and the pre-swap receipt was written.** That was the maiden constitution
   deploy and it is the proof this clause asked for; the mechanism below is
   kept as the record of WHY it holds. Every future bump re-proves it silently
   through the J-battery, so a bump that scrambles the order is a REGRESSION,
   not a fresh discovery.
   The original defect, for the record: verified across the 2.12.15 bump, the
   ledger was byte-identical before and after (143 entries, the user's curated
   order intact) and *nothing read it back*. Restored rows land first, adopted
   live rows are appended after, so the user's two live sessions moved from
   positions 1-2 to 6-7 and they had to re-curate by hand for the third time
   in a day.
   What now exists (`crates/yggterm-server/src/row_order_ledger.rs`):
   - **The restore.** `reconcile_order_with_remembered` is the one owner of
     the rule — rows the ledger knows take the ledger's relative order; rows
     it has never seen keep the slot the anchored import walk
     (`import_peer_live_rows_in_order`) gave them, still under the same
     neighbour. Both handover rebuild passes
     (`run_deferred_preserved_owner_deep_reconcile` and
     `takeover_superseded_daemon_state`) end by applying it, **before** their
     own persist — `persist()` records the live order INTO the ledger, so
     persisting the freshly-imported scramble first would erase the very
     arrangement the restore reads. It reconciles against
     `DaemonRuntime::booted_with_row_order`, the ledger as this daemon booted,
     for the same reason.
   - **It cannot resurrect.** The reconcile is a permutation of the rows the
     daemon already holds, and `replace_live_session_order` separately refuses
     any path that is not already a row — two independent refusals, each
     locked by its own test, so a tombstoned row in the ledger stays out.
   - **The reorder verb exists and is fixed** (it had been added since this
     entry was written; the defect was that it ignored dormant rows and
     reported success anyway — field guide §4.5). It now moves dormant rows
     and answers with `applied` / `skipped` lists.
   - **The pre-swap receipt** lands at
     `~/.yggterm/manual-snapshots/pre-daemon-swap-<unix-secs>-<pid>.json`,
     written by the outgoing daemon on `PrepareUpdateRestart` and by the
     incoming daemon before it imports a row.
   **How it was closed, and how every bump re-checks it:** capture
   `server app rows` before and after the swap, confirm the order is
   unchanged, and confirm a `pre-daemon-swap-*` file appeared. Never on unit
   tests. (What this entry still does NOT cover: a plain shell's row surviving
   a bump — that is level (a) above, and it has not been exercised by a real
   swap yet.)
3. ✅ **RESURRECTION IS FIXED, PROVEN ACROSS A REAL VERSION BUMP.** 8 closed
   rows, 8 tombstones kept, **0 resurrected**, 0 orphaned processes, and the
   daemon self-retired gracefully in 40 s. Keep this result; it is the
   baseline any future change to the import path must not regress.
4. **A ROW WITH NO RUNTIME IS CORRECT AND DESIRABLE.** User's words: *"No
   runtime is none of our business. The user can click to start it."* The
   model is explicitly GTA 5 vs Crysis — an asset that is not rendered but
   looks rendered. Do NOT reap runtime-less rows; freeing the runtime while
   keeping the row IS the feature.
5. **yedit AND ychrome CLOSE ON EVERY RESTART AND MUST NOT.** They should stay
   up and stay on their **libyggterm surface**, not fall back to the terminal
   surface.
   **CAUSE (found, and it is not what the symptom says): the apps never
   closed.** They run in daemon-owned PTYs and survive the GUI fine. What dies
   is the CLIENT's memory of their surfaces — both `web_surfaces` and
   `sidebar_contributions` are built by an OSC 7717 parser that only exists
   while a terminal host is MOUNTED. After a relaunch the tables are empty,
   so every session paints the terminal surface. The OSC heartbeat cannot
   repair it: it reaches only a session whose host is mounted (so never a
   background row), and a two-tier app like yedit declares exactly ONCE and
   exits, so there is no heartbeat to catch at all. Both daemon-replay rebuild
   paths already existed — they were just wired to agent verbs only
   (`right-panel pane:<id>`, `web ensure --session`) and to nothing that runs
   on its own.
   **FIX (GUI-side, no daemon change, no version bump): `restore_app_surfaces_tick`**
   on the 2.5s working-flags poll tick drives those SAME two rebuild paths.
   Endpoint-probed liveness, never declare age (the rail half already gets
   this right — see "liveness is the ENDPOINT" below); a dead endpoint or an
   unanswerable preserved owner degrades to the terminal view with
   `daemon_declare_endpoint_dead` / `daemon_declare_unavailable` /
   `daemon_declare_absent` in the trace, never a blank surface. One ask per
   (session, `terminal_process_id`), so a handover that re-resumes a PTY
   re-arms it and nothing else becomes a per-tick daemon poll; 3 sessions per
   tick; active row first. It never activates a session, moves focus, or opens
   a rail — it restores surface STATE, which the user sees when they visit.
   ⚠ **NOT yet verified live on jojo.** Unit-locked only (4 decision locks +
   a wiring scan). The live proof owed: restart the GUI with a yedit and an
   ychrome session running, then confirm both come back on their own surface
   without a manual reopen, and that `app_surface_restore` appears in the
   trace.
6. **★★★ AGENTS MUST DRIVE SHADOW SURFACES EVEN WHILE THE USER'S GUI IS
   CLOSED.** Felt concretely: two background filing agents each drove a ychrome
   session row and the GUI host burned. This is the same requirement as
   server-side rendering — agent browsing should never have been on the GUI
   host (docs/optimization-pass.md WS2, `ychrome/docs/agent-engine.md`).
   Wanted as a real feature, not a workaround.
7. **DAEMON HANDOVER MUST TELL THE USER AND STOP DRAWING.** On a daemon
   version change the GUI host burns. Spawn a notification ("daemon is
   changing, please wait"), **stop drawing the terminal for the duration**, and
   entertain the user. The render cost during handover is the thing being
   avoided, so the fix is to stop painting, not to paint a spinner harder.

   ⏳ **BUILT, NOT YET LIVE-VERIFIED (GUI-side, no daemon change — it deploys
   without a version bump).** `crates/yggterm-shell/src/handover_gate.rs` is
   the one owner of the predicate, derived from the DAEMON'S OWN report
   (`preserved_terminal_owner_keys` — the keys it serves but a predecessor
   still owns), scoped to the runtime keys this client has mounted, and
   resolved when the successor adopts them. On the ON edge: a coalescing
   "Daemon updating" job notification, a static veil over the viewport, and
   the terminal read/write path stops — no daemon read, no `term.write`, no
   render-health sampling (so no recovery `redrawTerminal`), no visible-paint
   scheduling. Resume is the NORMAL read from the unchanged cursor, never a
   daemon-screen replay. Three fail-safes: the first observation is a baseline
   (a GUI starting beside a lingering preserved owner never opens veiled), an
   unreadable status resumes paint, and a 90 s ceiling ends any suspension and
   latches that handover so it cannot re-arm. Probe it at
   `server app state` → `handover_paint`; trace events
   `handover_paint_suspended` / `handover_paint_resumed` (component
   `daemon_handover`).
   **Still open:** (a) live proof on jojo across a real daemon bump — nobody
   has watched this fire yet; (b) detection latency is bounded by the
   runtime-status poll (10 s busy / 60 s idle), so the notification can land
   several seconds into the handover — a cheaper immediate trigger (e.g. an
   out-of-band status refresh on the read loop's cursor-rewind tell) is the
   obvious follow-up; (c) suspending reads for a long handover can overrun the
   daemon's 512-chunk ring, which lands on the existing `resync_required`
   path (scrollback-preserving screen reconcile), unchanged by this lane.
8. ✅ **AUDIO NOTIFICATIONS NEED A PRE-ROLL — CLOSED 2026-08-02, CONFIRMED BY
   THE USER BY EAR.** The premise the first implementation was built on was
   wrong (the webview never made a sound at all), so the shipped path is native
   Rust and the numbers are the measured ones: **pre-roll 0.70 s**, flush tail
   **1.10 s**, TPDF dither at ~-57 dBFS **spanning the whole render, not just
   the front** — the tune is mostly silence by duration, so a front-only
   pre-roll left every later note exposed to a sink that had gone back to
   sleep, which was the reported ending-clip. The context is long-lived rather
   than opened per chime, the adaptive skip is real
   (`NOTIFICATION_PREROLL_LINK_AWAKE_WINDOW_MS`, 10 s, traced as
   `notification_sound_preroll`), and `yggterm_core::notification_audio` owns
   pre-roll, tail and dither for BOTH players so the native CLI and the webview
   script cannot drift into two different chimes.


## ★★★ USER REQUIREMENTS FOR THE SESSION-ROW LIFECYCLE (stated 2026-07-26, after

**★★★ USER REQUIREMENTS FOR THE SESSION-ROW LIFECYCLE (stated 2026-07-26, after
curating the list by hand TWICE).** The user's words: *"A daemon bump and
restart should not destroy the row order and number of sessions. If destroyed
this order is supposed to be snapshotted properly. And lastly all the rows not
connected should die (gracefully is recommended)."*
1. **A daemon bump must preserve row ORDER and COUNT.** ✅ Verified for a
   GUI-only restart 2026-07-26 (21 rows, byte-identical order across the swap,
   snapshot at `~/.yggterm/manual-snapshots/pre-gui-restart-*`) and ✅ **PROVEN
   ACROSS A REAL DAEMON BUMP on 2.12.16** — the case that actually breaks it,
   where rows are re-imported from peer daemons: **22 rows before, 22 after,
   ORDER IDENTICAL**. The anchored-placement fix
   (`import_peer_live_rows_in_order`) has now been exercised by a real daemon
   swap. Every later bump re-proves this silently through the J-battery, so a
   scramble is a regression to bisect, not a new finding.
   ⚠ **What is still NOT exercised: a plain shell's row surviving a bump**
   (level (a) in standing-traps item 1). The 2.12.16 proof says nothing about
   that half.
2. **If order is destroyed it must be recoverable from a snapshot.**
   ✅ **BUILT AND PROVEN LIVE across the 2.12.16 bump** (order identical, and
   the pre-swap receipt was written).
   `~/.yggterm/row-order-ledger.json` records order+membership and
   `removed-rows.json` records closes; the original defect was that nothing
   RESTORED from them automatically, and an agent had to reconstruct by hand.
   Both halves now exist and both ran — the automatic restore on every handover
   rebuild pass, and the pre-swap receipt at
   `~/.yggterm/manual-snapshots/pre-daemon-swap-<unix-secs>-<pid>.json`.
   See standing-traps item 2 above for the mechanism.
3. ✅ **"All rows not connected should die" — DECIDED by the user
   (2026-07-26, asked directly): "not connected" means rows that were
   explicitly CLOSED — by the user or by an agent.** It does NOT mean
   runtime-less rows (call #4 stands: never reap those). That is exactly the
   tombstone plane: both the GUI close and an agent's session-remove flow
   through the same daemon handler (`tombstone_live_row` before
   `remove_live_session`), so the requirement is implemented and was proven
   across the 2.12.15 bump (8 closed, 8 tombstones kept, 0 resurrected).
   One recorded nuance, deliberate: the `PrepareClientClose` non-keep-alive
   reap does NOT tombstone — that is contract death (second-class shells die
   with their GUI), not an explicit close; the import admission predicate's
   owns-runtime refusal is what keeps those husks from coming back.
