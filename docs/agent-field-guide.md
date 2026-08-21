# Agent field guide

How to measure, deploy, and verify yggterm without fooling yourself. This is the
durable half of what agent sessions keep re-learning; the volatile half (current
queue, this week's findings) lives in the agent's own notes, not here.

**Scope note.** This file is public. Describe hosts by role — "the live desktop
host", `$LIVE_HOST` (resolved by `scripts/ygg-live-host.sh`), "a remote machine" —
never by address, and never paste session ids, transcripts, credentials, or
anything that resolves on the public internet. See `SECURITY.md`.

## 1. The instruments lie — know which, and how

Every entry below cost a session at least once.

| Instrument | Lies when | Use instead |
|---|---|---|
| ⛔ `ygg-privacy-guard`'s **path-name** scan, read as *"does this push add a private name?"* | **Whenever a leakily-named file is being DELETED.** The scan listed every path a commit TOUCHED, so removing a privately-named file refused the push that removed it — the deletion names the path one last time, by construction, and git offers no way to delete a file without mentioning it. Measured 2026-08-20 while six privately-named screenshots were being taken off a public repo: the guard blocked the fix and left the leak in place, which is the worst direction a leak gate can fail in, and it fails there while looking perfectly correct. **Fixed** — the path haystack is now `--diff-filter=AR`; only an added or renamed-to path can introduce a name, and a modified path was already public. | If a refusal names a path you are REMOVING, read the diff filter before you read the term. ⛔ Never `YGG_PRIVACY_ALLOW=1` — it suppresses the whole scan, including the added LINES you have not read. Prove the gate still bites with a three-commit fixture: add a named path (refuse), delete it (pass), add a named line (refuse) |
| ⛔⛔ `app screenshot` **as proof that a SETTINGS MODAL is or is not up** | **Whenever one of the Settings-raised overlays (launch flags, CLI install) is open.** The returned PNG shows the sidebar and the right rail correctly and the entire centre of the window as flat background — the overlay is simply not composited into the frame. Measured 2026-08-20: a modal that a DOM hit-test proved was mounted, and whose own text the hit-test read back, photographed as an empty dark rectangle. ⚠ The trap is that the frame looks *plausible* rather than broken — nothing is torn or black-screened, the chrome around the hole is perfect — so it reads as "the click did not work" and sends you to debug a handler that is fine | `server app grid show` then `grid click <cell>`, which returns the `text`/`tag` of the element actually under that point. That reply is a DOM read, not a picture, and it names what is mounted. ⭐ The rail's own summary line is the cheaper proof for anything the modal merely *reports* — it is drawn in the chrome, which the screenshot does capture |
| ⛔ `server app state` → `alt_overlay_modal_scope` read as **"is a modal open"** | **Whenever the modal was opened with the mouse and no ALT chord was pressed.** It reports the scope the ALT/KeyTips layer is bound to, not the presence of a dialog, so it stays `null` over a fully open, fully interactive modal. Measured 2026-08-20 across four consecutive opens; it read `null` every time while the overlay was on screen and answering hit-tests. Reading it as a modal flag produced a false "the button does nothing", and then a false "even the known-good button does nothing" — which nearly got a working feature reverted | A DOM hit-test (`grid click`) inside the modal's own area, which returns its text. `data-yggterm-modal-root="<kind>"` is the marker the overlay actually stamps |
| `server app grid click <cell>` | **The grid EXPIRES.** A click issued after the TTL answers `accepted:false` with `no active grid; run 'server app grid show' first` — loudly and correctly, but a loop that shows the grid once and then clicks several cells silently degrades into one real click and N refusals. ⚠ And on a modal surface, **any cell outside the modal's own panel is its BACKDROP, so the click closes the dialog** — a scan across cells looks like the modal "randomly" disappearing halfway through | Re-issue `grid show` immediately before **every** click, and keep a scan inside the panel's own columns. Read `accepted` on each reply rather than assuming the click landed |
| `app screenshot` (default backend) | A native child webview is on screen — the composite pastes canvas over a DOM snapshot and a GTK widget is in neither layer | `--backend os` |
| `app screenshot` after any GL/compositing change | `toDataURL` returns the canvas backing buffer even when nothing composites to screen; reports `capture_faithful:true` over a black screen | `--backend os`, or the user's eyes |
| `server app row-set --into` on a `depth1` leaf already at top | **A leaf already at top is not detached** — moving it under a header whoops the flat top into a group. `accepted:true` hides it; `rows --json` shows `head child` growing and the leaf at `depth2` | Only a session with `live_runtime true` and `presence row` (detached, not `live_rail`) is a candidate to `row-set --into` a header; leave `live_rail depth1 child1` leaves where they are and verify `depth1` is unchanged |
| `app screenshot` on a client that has switched sessions **(fixed 2026-07-25, `e0dc6c1` — keep the cross-check habit)** | It composited every `isVisible` host ordered by `mountedAt`, and a session you switch BACK to is REVEALED not re-mounted — so its host is the OLDEST while the host it replaced is the newest and stays visible for a while. The stale host was drawn ON TOP: a near-blank frame with `capture_faithful: true` while the terminal was painted fine. Nine rapid switches reproduced it every time | `shadow-client.sh capture` (grim = the compositor's own pixels). The payload now reports `active_session_path` beside the path it drew — if they differ, the frame is not your session |
| ⛔⛔ `pgrep -x yggterm-headless` — **OUR OWN DAEMON, AND IT MATCHES NOTHING, EVER** | **Every single time.** `comm` is truncated by the kernel to **15 characters** (`TASK_COMM_LEN`) and `yggterm-headless` is **16**, so exact-match against a name that can never appear returns **0** with no error. Measured 2026-08-14 on the integrator host: `pgrep -cx yggterm-headless` = **0**, `pgrep -cx yggterm-headles` = **20**, `pgrep -cf yggterm-headless` = 23. It hid a population of **19 legacy daemons burning 3.6 cores** for as long as it took to notice the empty output. ⚠ **This defeats the identify-by-exact-name advice that is correct everywhere else in this table** — `pgrep -x yggterm` works only because that name happens to be 7 characters, so the habit succeeds on one of our two binaries and silently fails on the other | `pgrep -x yggterm-headles` (the truncated `comm`, which is what actually exists) or `pgrep -f yggterm-headless` plus the self-match guards below. ⭐ **Read the truncation off the machine instead of counting characters:** `cat /proc/<a-known-pid>/comm` prints the name the kernel will actually compare against |
| `pgrep <pattern>` **without `-f`, where the pattern is 16+ characters** (the general case) | **Always, and it reports ZERO rather than failing.** Same truncation as the row above, for any long name. Measured the same day: `pgrep -c drkonqi-coredump` (16 chars) returned **0** while `pgrep -cf drkonqi-coredump-launcher` returned **18**, and that 0 was one keystroke away from being published as "that crash counter is gone, delete the claim". ⚠ This is the INVERSE of every other `pgrep` row here: those match too much and are caught by looking, this matches nothing and looks like a clean negative | `pgrep -f <pattern>` (matches the full command line, no length limit) — then apply the self-match guards in the rows below. To sanity-check any zero, re-ask with a pattern under 15 chars: if the short one finds processes and the long one finds none, the length is the whole story. ⛔ **A zero from an instrument you have not proven can return non-zero is BLIND, not empty** |
| `pgrep -x yggterm` / `pgrep -cx yggterm` **as "how many GUIs are running"** | **Whenever any agent on the fleet runs a `yggterm` verb, which on this fleet is continuously.** The CLI and the GUI are the SAME BINARY, so a process-name count cannot tell a window from a one-second `server app rows`. Measured 2026-08-14: baseline **2**, **3** during a single deliberate `yggterm --version`, **1** three seconds later. This produced a false level-1 FAIL in the usability check within two hours of that check being written, and a level-1 alarm that cries wolf is how a REAL duplicate GUI gets waved through | `server app clients` — the product's own registry of live GUI clients, which a CLI invocation never enters (verified: answers 1 while a concurrent CLI call is in flight). Count `client_role == active`; shadows are agent probes, not windows. ⚠ Treat an unreadable registry as BLIND, never as zero — "I could not ask" and "there are none" demand opposite reactions |
| `server status` | It pins to its own version's socket and can answer from — or spawn — an empty orphan daemon | `server app …` (PID-routed) |
| `server status` → `terminal_session_count` **as "how many sessions are on this host"** | **Always during a handover, which is exactly when you are watching.** It counts what the ONE daemon you reached OWNS, and a handover changes which daemon that is: a fresh successor answers with a handful while the rest are still owned by, or preserved on, its predecessors. Measured 2026-08-14 — a watch on this field alerted `53 → 29` on a host that had just gained sessions (host-wide **57**, 261 rows, nothing lost). It fires the loudest false alarm precisely on the event it is meant to police | Sum the `OWNED` column of `server daemons` across every daemon, or read `ROWS` from the newest — both are host-wide. Never compare one daemon's count taken before a swap with another's taken after |
| `server status --endpoint <pid>` → `terminal_session_keys` **as "which sessions this daemon OWNS"** | **Always, and it looks right because it is a superset.** It is `owned + preserved`, and on an old daemon the preserved half dominates: a 3.0.62 daemon answered 28 keys while owning **1** and preserving 27. Attributing a session to a daemon on this field credits the PTY to whichever daemon merely holds a bridge record — so "which daemon owned the row that died" comes out wrong, in the direction of blaming the oldest daemon present. Checked against `server daemons` on 14 daemons: `terminal_session_keys` == `OWNED + PRESV`, 14/14 | `owned_terminal_session_keys` / `owned_terminal_session_count`, which are separate fields in the same payload. `preserved_terminal_owner_keys` is the other half if you want it |
| A **self-control** — a process counting its OWN children to calibrate a child-count probe | **Always, and it returns 100 % by construction.** The sampling loop runs in a shell whose literal parent is the measuring process, so the control finds the counter doing the counting, on every read, forever. Demonstrated directly: `pgrep -P <own claude pid>` returned exactly the pid of the shell running it. A row that "corroborated" a child probe this way got a clean 6-of-6 separation that means nothing. ⛔⛔ **Same family as `pgrep -f` matching the searching shell — but wearing a CONTROL's clothes, so it is trusted MORE rather than less** | Have a **different session** take the reading. ⭐ This instrument **requires a second observer by construction**: no row can produce a valid child-count control about itself, which is a real limit on self-diagnosis |
| `pgrep -P <pid>` (or any child/handle COUNT) used to decide whether a process is **finished** | **Always, on a single read.** A child count is a point sample of a value that is legitimately zero much of the time — an agent between tool calls has no children, by construction. Measured on one pair: the suspect returned **zero children on 5 of 6 reads** at 7.8 % of a core, and the row's own **demonstrably live** process returned zero on **4 of 6**. The signature is identical on a known-live process, so a single read is not weak evidence, it is none. ⛔ It fails towards killing a working agent | Sample **repeatedly across at least a CPU window**, and pair it with a rate (20 s `%CPU`). ⭐ For "did it survive", nothing beats **transcript growth after the fact** — no pre-action sample can establish it |
| A **parent's** CPU rate used to decide whether an agent is working | The work is in a **child**. An agent running `cargo test` sits near the idle rate while its child burns the core — a clean 5× parent-rate separation was obtained on two rows and would have inverted on a third | `pgrep -P` alongside it. ⚠ Neither instrument alone: this pair has now caught two different sessions, once each, in **opposite** directions |
| `ygg-privacy-guard scan --rev-range <R>` → `✅ no private data found` | **The range yielded no added lines.** The report is honest — it prints `0 added lines` in the same breath — but the ✅ is what gets read, and an empty range passes for the same reason a clean one does. Measured 2026-08-14 scanning a reflog-only range (`--all --reflog --not --all`, 3,094 commits) : green, over nothing. This is the third time a scan of an empty range has been quoted as a clean bill of health | **Read the `N added lines` figure before the verdict**; zero means the scan is vacuous, not clean. Give it a range that produces a diff (`<parent>..<commit>`), and confirm the count is non-zero — the tool's own §2 lesson is that *an absence must describe its own boundary*, so use the boundary it prints |
| `ygg-booter list` → `boots=0` **read as "this row has never been booted"** | **Whenever the row is healthy** — which is most of the time. It is a counter of *unanswered* boots, not a lifetime total: `ygg-booter.py` sets `s["boots"] = 0` on `WORKING`/`JUST_ENDED` (*"progress clears the stall counter"*) and again when the transcript grew (*"it worked since last tick"*). ⇒ A row woken repeatedly all evening, responding every time, reads `boots=0` forever — and the zero was quoted as evidence that an arming had never fired | Read it as *"no stall currently unanswered"*. For whether waking works at all, look at whether the row RESUMED work after a wake, not at this counter |
| **Any liveness signal whose positive is produced by a person being present** — draft-presence, recent PTY bytes, focus, scroll position | **Structurally, and in the direction that hurts.** The signal is `True` for exactly as long as the human is there, so its positives are **biased towards the rows where acting is most dangerous**, and a single sample cannot tell "present" from "abandoned mid-way". Found on draft-presence; the same inversion applies to every member of the family | ⛔⛔ **A SIGNAL THAT MEANS "SOMEONE IS HERE" MUST NEVER BE WIRED TO A REMEDY THAT ACTS AS THEM.** Require two samples separated by time with no progress between them, and exempt attended rows from the ADVICE as well as from the automation |
| `server rows drafts` → `drafts_present: <row>` **read as "that row is stalled with litter in it"** | **Most of the time.** The flag is `True` for exactly as long as a human is composing, so a single sample cannot tell a transient draft from an abandoned one — and the rows that flag most often are the rows a person is actively talking to. ⛔ **The positives are biased towards the rows where acting is most dangerous**, and the obvious remedy ("clear the composer") is *typing over a human* on an attended row — the `never-arm.tsv` hazard performed by hand, with the guard file never consulted because the actor is not the booter | Require **two sweeps minutes apart with no transcript growth between them** before reading it as stalled. ⛔ Exempt `never-arm.tsv` rows from the ADVICE as well as from the arming |
| `server rows drafts` → `drafts_present: 0` **read as "nobody is mid-sentence, the bump is safe"** | **Any daemon on the host predates the field.** The sweep asks each daemon for `pending_input_drafts`; one that does not carry it answers nothing, and nothing is not none. Zero drafts over a host where zero daemons could answer is a report that **it did not look** — on its first live run that was 15 daemons and ~60 sessions, every one unasked. The whole verb exists because the previous instrument (`--refuse-if-draft`) required performing the write a draft forbids in order to learn whether a draft forbade it | Read **`verdict`**, never the count. `clear` is the only safe answer; `blind` means some daemon could not be asked and is NOT `clear`; `drafts_present` outranks both. Cross-check `can_answer` per daemon — all-false means the reading is vacuous. ⭐⭐ **AND SWEEP EVERY HOST, BECAUSE THE UNION IS COVERAGE, NOT DUPLICATION:** a row whose PTY host is too old to answer is still answered by its **GUI host**, which reconstructs the draft from the bytes the client typed. Measured: a single-host sweep reported one armed row as unknowable; both hosts together reported **7 of 7 covered, zero blind** |
| `strings <binary> \| grep <a literal your change adds>` **used to prove your build is in** | The literal you picked is a `serde_json::json!` **KEY**. Measured 2026-08-14: a new `"target_source"` key read **0 occurrences** by raw byte search in a binary that unambiguously contained the line — while `"queue_file"` and `"process_memory"`, the two VALUES on that same line and unique to it in the whole tree, were both present and adjacent to the neighbouring key in rodata. Two forced rebuilds and an rlib hunt were spent on a false negative that nearly binned a correct build | Pick a literal that is unique to your change **and appears as a VALUE**, then confirm with `nm -C <binary> \| grep <your new fn>`. ⚠ The presence direction still works — this entry is about the ABSENCE direction being unsound |
| `grep <event> ~/.yggterm/event-trace.*.jsonl` **as "what the live daemons did"** | **You have run `cargo test` on this host.** The test binaries write into the SAME trace directory as the daemons, so a corpus-wide count silently mixes fixtures with production. Measured 2026-08-14 while harvesting a proof: `live_session_persist_dropped` returned exactly 2 events over 2 distinct keys — *precisely the fixed behaviour* — and every one came from a test pid, with sibling events carrying fixture paths (`wedge-signal-probe`, `remote-session://dev/sized-restart`). Live daemons had emitted **none**, so the real reading was *vacuous*, not *fixed* | Filter by pid and confirm each pid is a daemon (`server daemons`), or bound the window to after the last test run. ⚠ A count that matches your hypothesis exactly is the case to check hardest |
| A `MutationObserver` / DOM-mutation count | Something animates via CSS. Animations mutate nothing; a page can present frames forever at 0 mutations | Count presented frames (below) |
| `terminal_host_count` / `active_terminal_host_count` | Detached-but-alive xterm entries exist. It counts hosts in the DOM; `window.__yggtermXtermHosts` can hold more | Enumerate the JS host map |
| A `requestAnimationFrame` probe you installed | Always. rAF self-sustains at refresh rate, so it measures itself | An external frame counter |
| `eglinfo` / `glxinfo` over SSH | Always — no seat session means the driver falls back to software | Whether seat-session processes hold `/dev/dri/render*` fds |
| `/proc/<pid>/environ` | The process called `std::env::set_var` at runtime (yggterm does this for GL and arming decisions) | The app's own reported state |
| The daemon's `terminal_lines` | You are chasing a CLIENT paint bug. That is the daemon's vt100 screen — comparing it to itself proves nothing about what the client painted | A faithful pixel, or the client buffer |
| A verb's own `accepted` / `is_trusted` | Always treat as an assumption, not an observation | Read back the page-side *effect* |
| **Every field in `server app state`, once `webview_edit_faults` is non-zero** | A webview that threw while applying an edit batch is acknowledged as having applied it, so the host diffs against a model in which those mutations landed and NEVER re-sends them. One subtree is then frozen at whatever it held, while every state field keeps reporting the mode it should be showing. **The state is not lying about the state — it has stopped describing the screen.** The rail that drew no body while `right_panel_mode` tracked every command was this, twice, and it cost a retracted bisect across 36 releases | Read `webview_edit_faults` FIRST. Non-zero invalidates the screenshot, not the state, and only a GUI restart clears it. ⛔ Do NOT reach for detached-node counts or interpreter stack depth — a clean instance measures 41 % detached and depth 1, so neither discriminates |
| `dom-eval` returning `{"result": null}` | Your script had no `return`. The body is spliced into an async function, so an *expression* yields `undefined` → `null` — identical to a field that does not exist | Include a `sanity: 1+1` term in every probe |
| `getComputedStyle` (verifying an ANIMATION actually paints) | **Always, for a paint bug.** Reading a computed value **forces the style recalculation the paint path never performs**, so the instrument supplies the very invalidation whose absence is the defect. A blink read back as a perfect 1100 ms square wave was, in the same minute, pixel-identical and ABSENT across ten faithful screenshots — WebKitGTK advances a custom-property animation in the style system without marking its `var()` consumers dirty for paint. A July verification built on this probe could only ever answer yes | A faithful screenshot **burst**, and diff frame-to-frame. Confirm other regions change between captures, or a frozen dot is indistinguishable from a cached frame |
| A performance win that lands exactly on the do-nothing floor | **Always suspect a deletion.** Presented frames falling 9.4/s → 2.1/s was celebrated as a rendering win; 2.1/s *was* the idle baseline, and the real change was that the blink had stopped drawing at all. **A perf number that reaches the floor is a feature that stopped happening until proven otherwise** | Check that the thing being measured still HAPPENS. Pair every cost measurement with a behaviour assertion, or you cannot tell optimisation from removal |
| `server app row-set <row> --into <head>` without checking `child_count` | **When the row is a head** it owns a subtree — `--into` nests the entire subtree. Help says `NESTS … depth+1, NOT peer reorder` but `accepted:true` still looks like success | `server app rows --json` → `child_count` before `--into`; leaf `child 1` attaches, head `child>1` is refused unless `--with-children`. For peers on top use `server app sessions reorder` — no membership change |
| `shell_mut_hist` | **Always, for raw writes.** It counts only `safe_shell_mut`; a bare `state.with_mut` is INVISIBLE to it. A no-op raw write therefore reads as *"unattributed render, empty histogram"* — which says "nothing I can see wrote", not "nothing wrote". Three render-storm autopsies in a row reported exactly that and all three died undiagnosed | Grep for raw `with_mut` callsites directly, and treat an empty histogram beside a real render as an ATTRIBUTION GAP, never as an absence of writes |
| `web_surface_contexts` | A surface has no profile dir. It counts only the KEYED map, and `WebContext::new_ephemeral()` is never inserted — so `0 contexts, 41 surfaces live` is indistinguishable from 41 UNSHARED contexts, the exact failure its own doc says it catches. Reached in ordinary operation: the shell passes `profile_dir: None` whenever another client holds the profile write-lock | Count surfaces and contexts together and compare; a zero count beside live surfaces is a BLIND instrument, not a clean result |
| `app_render_rate` | Never — and that is the point. It is **always on** (no env gate) and had already recorded 739 samples over 12.3 h showing the rate FLAT at ~2/s while CPU climbed 3.6×. Nobody read it, and a cluster was briefed to chase the re-render as the growth | Read the always-on probes BEFORE forming a hypothesis. A constant-rate loop cannot be what grows |
| Any statistic over `perf-telemetry.jsonl` `daemon_request` durations — mean, median, or a fit — for `status`, `ping`, `terminal_read`, `terminal_write`, `terminal_snapshot` or `working_flags` | **Always.** Those six are `perf_span_is_high_frequency_noise`'s list, so a span is written only when it ran **≥ 8 ms** *or* wins a **1-in-50** sample. One live stream held 5,604 sub-floor records (each standing for ~50 replies) against 2,769 tail records (each standing for one): a recorded tail share of **33% against a true 0.98% — 34× enrichment.** ⛔ And it does **not** cancel between processes, which is the assumption that makes a cross-daemon comparison look safe: the 243–246-row daemons put **13.5–16.8%** of their records above the floor where the 70–101-row daemons put **3.5–7.6%**, because more rows push more replies past 8 ms. A per-row slope read straight off the recorded set came out **10.2 µs/row**; the same daemons with each record inverse-probability weighted gave **4.7 µs/row** | Weight each record by its sampling probability (50 below the floor, 1 at or above it) before computing anything, or read a counter that samples nothing. ⚠ And these durations are **wall**, not CPU — an upper bound on the CPU a handler burned, and `status` has been seen waiting 9.89 s for the runtime lock |
| `yggterm --version` | You need the **protocol** version. It reports the `yggterm` package; the daemon uses `yggterm-server`'s, which a version-only bump may not recompile | The daemon's own socket name, `server-<v>.sock` |
| A `FAILED` / `test result:` grep over `cargo test --workspace` | **The workspace does not COMPILE.** A build error yields no failures *and no results*, so every "is it green?" grep reads clean — an empty set wearing a pass. Reached in ordinary operation: a struct gained a field and three test fixtures were not updated, and the suite was unbuildable until someone tried to run one specific test. Every workspace run between that commit and the fix was reading a non-result as a pass | Assert the run HAPPENED before reading its silence: require a non-zero `test result: ok. N passed` with **N > 0**, and check the exit status. A grep that can only ever find bad news cannot distinguish "no bad news" from "no news" |
| *"It passes individually, so it is flaky in parallel"* — as a reason to stop looking | **Always, in this workspace.** The phrase has been standing in for a diagnosis, and the mechanism turns out to be a real defect: `sync_claude_extra_args_for_request` (`daemon.rs:10007`) does `unsafe { std::env::set_var(…) }` **process-wide** and the launch builder reads it back, so in a test binary — one process, every test — launches race. Measured: **4 parallel runs → 2 green, 2 red on DIFFERENT tests; serial ×2 → 1096/1096 both times.** Passing serially proves the failure is not in the test's own subject; it does **not** prove nothing is wrong | Re-run with `--test-threads=1` and treat a parallel red that does not reproduce as **evidence about that `set_var`**, not as noise. ⚠ Same shape as the launch-flag bug beside it: a per-launch value carried in process-global state, outliving its launch — so the tell is *"a red that moves between tests between runs"* |
| `cargo test … <name> -- --exact`, to confirm one test individually | **The test lives in a module and you passed its bare name.** The filter matches nothing and prints `test result: ok. 0 passed; 0 failed; 1112 filtered out` — a **pass line for a run that did not happen**, and it reads as green at a glance. Three tests known to be failing all printed exactly that; the finding would have been inverted. The module path is required: `tests::…`, `terminal::tests::…` | **Read the count, not the verdict.** Require `1 passed` (or N>0) before believing any individual-test result, exactly as the workspace-build entry above requires it before believing a silence. Same family, different costume: not an unbuildable workspace, but a filter that silently matches zero |
| A row-plane verb failing with *"connecting to `~/.yggterm/server-<v>.sock`: No such file or directory"* | **You will read it as an addressing failure.** It is a CONNECTION failure — the CLI is pinned to a version-stamped socket the daemons never bound. Three rows on one campaign each hit this, each concluded *"that row's uuid is not addressable from here / the peer list is incomplete"*, each filed it as **their own limitation**, and each routed around it by guessing a reachable row — so cross-row messages went to the wrong recipient and were relayed by hand. ⚠ It is **invisible** to anyone using the remote form (`ssh <guihost> '…yggterm-headless …'` reaches the GUI process and never touches the socket) and **total** for anyone using the local one, which is why it presents as a patchy peer list rather than a missing file | Read the error's own noun. **The error names the socket; the symptom names the peer list.** Check `ls ~/.yggterm/server-*.sock` before believing anything about reachability, and note an alias may mask it: a `server-<old>.sock` symlink pointing at a NEWER socket is the design; pointing at an older one is a proxy wearing the design's clothes. ⛔ Enumerate an alias set from the versions that EXIST, never from the files that happen to REMAIN — 73 stale socket files on one host, **757** on another |
| `git reset --mixed origin/main` in a SHARED checkout, to drop a commit you already landed elsewhere | **Upstream has moved since you landed it.** The reset resurrects your commit's content as an *uncommitted* diff against the newer upstream — so your now-stale copy of a shared file silently **reverts every entry that landed after yours**. Measured: `git diff --stat origin/main -- docs/pending-bugs.md` → **64 deletions**, all another lane's, staged and ready for the next `git add -A`. ⚠ The file was NOT dirty before the reset; the reset is what dirtied it | Confirm patch-equivalence first (`git cherry origin/main HEAD` → a leading `-`), then **`git checkout origin/main -- <path>`** for every shared file the commit touched, and re-read `git status` — the lane's own dirty files must be the only thing left. ⛔ Never `git add -A` in a shared checkout; stage by explicit path |
| A column index into another tool's **human** output (`awk '{print $3}'`, `split($3,a,":")`) | **The column means something else.** A repair tool read `$3` of `ss` output as an address; in that output `$3` is **Send-Q**, so it extracted nothing — and fell through to the *reassuring* branch, printing `GUI <pid> has no edit socket; not the flush-gate freeze` about a GUI that was holding a listening edit socket the whole time. ⇒ A parse failure that lands on "you're fine" is worse than a crash: it exonerates the thing it was asked to diagnose, in a specific and plausible sentence | Match by **shape**, not position (pull the address out of the LISTENING line by pattern), and prefer a machine-readable mode where one exists. Make the failure branch say *"could not parse"* — never *"nothing found"* |
| `pgrep -f <pattern>` | **Always, when the pattern is in your own argv.** It matches the asking shell, and `pgrep -P` from there walks to an unrelated child and names it with confidence. ⛔ The obvious guards all leak, each disproved by its own control: *exclude my ancestors* misses the **forked subshell** (a fork inherits its parent's `cmdline` verbatim, so every command substitution is another copy below you), and *…and my descendants* misses the **sibling** pipeline member, which is neither above nor below. A hand-written `case "$c" in *pgrep*\|*bash*)` is a list of the shells someone thought of, and fails for `sh`, `zsh`, `python3 -c`, `xargs`, an ssh command string | **Lineage is the wrong axis.** What every false positive shares is that its command line **IS one of ours** — compare the bytes. Needs no list and cannot go stale. `ygg-procfind.sh` is the shared helper; identify a process, never count it |
| A shadow client (`--client <name>`) that worked earlier in the session | **A deploy restarted the GUI and stranded it.** This is the most dangerous entry in the table, because it fails in the opposite direction to the rest: a dead shadow does not return an error, it returns a **plausible picture**. A rail stuck on `Loading…` reads first as a slow fetch and then as a regression in whatever you last changed — and the CONTROL runs through the same dead instrument, so two measurements agree completely and both are worthless. It manufactures a false accusation against someone else's code rather than a false absence | Check the instrument's OWN health in the same run as the measurement (`--client <name>` answers *"no live client by that name"* the moment anything asks). ⛔ A shadow that worked an hour ago is not evidence it works now — the GUI moved **eight versions in one session** (3.0.132 → 3.0.140); never assume the build you measured on is the build you are on |
| Ordering inside `main()` when a guard's contract is "first statement" | **Anything is inserted above it.** `adopt_or_refuse_session_bus()` guards against orphaned session buses, and *being first* is its entire guarantee — GLib caches the D-Bus address on first use, so one earlier call permanently disarms it. A later commit added a build-identity declaration above it and the lock went quietly red in both binaries; nothing about the new line looked dangerous | Keep an executable assertion that the guard is first (the test exists — `every_entry_point_refuses_autolaunch_before_it_can_happen`), and treat "this line is harmless, it only records something" as the exact shape that breaks a first-statement contract |
| **`mtime` on an agent session store** | **Always, on any store a fleet copy has touched.** Measured 2026-08-08: **575 of 618** codex rollouts on the GUI host share a single mtime *minute*. `rsync`/`cp` stamps the copy, so "least recently touched" reads as "all touched at once", which is false for every file. It will also report a decade-old session as fresh | **Birth** = the store's own path/filename convention (`.../<yyyy>/<mm>/<dd>/rollout-<iso>-<uuid>.jsonl`). **Last touch** = the `timestamp` field on the file's **final record** (top-level, ISO-8601, on every record). Both survive a copy; `mtime` does not |
| `du` vs `find -printf %s` on a compressed dataset | Always, and they will not agree. The fleet's pools run `compression=zstd` at `compressratio=1.54x`, so `du` reports **compressed** bytes and `find`/`stat` report **logical** ones. 3.2 GB of swept files returned ~2.0 GB of disk. Nearly filed as a bug | Say which you mean. Report reclaim in **disk** bytes (`du`, or logical ÷ `zfs get compressratio`); size a budget in the same units it will be measured in |
| A **byte** comparison as proof that two transcripts are the same conversation | The CLI has migrated its own format. Codex re-serialised its whole store on 2026-03-14 with a different JSON **key order**, so a `.bak.` and its live rollout hold identical records in different bytes. A byte-prefix proof refused **624 of 753** genuinely-redundant copies | Canonical JSON per record (`json.dumps(..., sort_keys=True)`), compared as a sequence, streamed in lockstep — these files reach 5.5 GB. Policy: [`spec-sweep-policy.md`](spec-sweep-policy.md) §9.6 |
| A file's **size** as a proxy for its importance | You are ranking conversations. Measured on the same store: the largest rollout puts **96.4% of its bytes in 388 of 36,288 lines** (pasted `data:image` blobs). Size tracks attachments, not reasoning, so a byte-weighted rank calls an image dump important and a long argument trivial | Count **user turns**, and count **distinct days appended to** for how often it was returned to. Policy: [`spec-sweep-policy.md`](spec-sweep-policy.md) §4 |
| A session **path scheme** as a statement of WHERE a row runs | **Always.** `live::<uuid>` reads like "local" and is what a REMOTE ssh shell gets; a genuinely local shell is `local://<uuid>`. Measured 2026-08-13: a report concluded `--kind shell --machine-key <remote>` was placing rows on the wrong machine, from the path prefix alone. The row was on the named machine — `kind: ssh_shell`, `host_label: <remote>`, and `exec ssh -tt … <remote>` in its own launch command. The entry was retracted; the flag had never been ignored | `host_label` / `machine_key`, **and** the launch command's ssh hop. The scheme says how a row is ADDRESSED, not where it runs. And a flag that reaches the GUI can be shown to: pass a nonsense key and watch it be refused BY NAME (`unknown remote machine key: …`) — a dropped flag cannot produce that refusal |
| `server reorder`'s response | The rows have no live runtime. It reports `"requested": N` and echoes your list even when it reordered nothing | Re-read `server app rows` |
| `--help` for any `server app` verb | Often — the help text goes stale while the parser gains verbs | The match arm in `apps/yggterm/src/main.rs` |
| A `#[serde(default)]` telemetry field reading `0` | The peer predates the field entirely — absent and zero are the same wire value | Ask whether the KEY exists, not its value |
| A third-party call you *believe* has an effect (`term.open(host)`, a `.focus()`, a `.dispose()`) | The library early-returns on a state you are already in. It does not throw, so the whole repair is a silent no-op and the code around it looks right forever | Assert the EFFECT (`host.childElementCount`, `document.activeElement`), or pin the behaviour in `tools/xterm-harness` |
| `grep <session-id>` over raw `server snapshot` output | **Always, and it convicts YOU.** The snapshot embeds `terminal_lines` for every row, including the row you are typing in — so a grep for a UUID matches your own scrollback echoing the command you just ran. A session that does not exist anywhere reads as "present in the daemon" (this shipped a whole wrong bug hypothesis on 2026-07-27) | Walk the snapshot **structurally** with `terminal_lines`/`scrollback`/`lines`/`screen` excluded, and cross-check `server status`'s `owned_`/`terminal_`/`stored_`/`preserved_terminal_owner_keys` |
| `Animation.playState` on a page you expect to be throttled | The surface is genuinely hidden and frozen. It still reads `"running"` — it reports the animation's *declared* state, not whether the clock advances. Predicting `"paused"` makes a correctly-fixed build look like a FAIL | Sample `Math.round(anim.currentTime)` twice, 60 s apart; frozen means it does not move |
| **`app terminal input-check`** on a row the user says they cannot type into | **Exactly when it matters.** It echoes a probe through the **daemon→PTY** leg and answers `wedged:false, consuming_input:true` — which is TRUE and beside the point, because the leg that refuses the user is **client→daemon**, in the GUI's own input policy. It reported a healthy row while the owner could not type into it (2026-08-11) | The `ui/input_policy` trace on the GUI host — `allow_input` plus its five gate fields is the decision itself, reporting why. `remote_resume_input_ready:false` there is the answer `input-check` structurally cannot see |
| `live_session_birth {activate:true}` and `restore_debug`'s `active_session_path` | You are testing `--no-activate`. Neither field means the GUI activated — the birth record and the restore log both show the new row as active on a create that demonstrably never activates | Poll `server app state`'s `active_session_path` / `active_view_mode` across the create (sub-second), which is the authoritative view state |
| The daemon's `request`/`begin` trace, read as *"a client asked"* | **Whenever the daemon is contended — which is exactly when you are investigating it.** `begin` is emitted INSIDE `handle_request`, i.e. **after** the one global `DaemonRuntime` mutex is acquired, so a request parked on that mutex writes nothing at all. `begin` means *"a client asked AND the daemon was free"*; a starved daemon and an idle one are byte-identical. On 2026-08-10 one daemon went 34.4 s without a single record while three `server terminal resize` processes sat blocked on the lock — the silence read as "nobody asked", and it drove four wrong root causes | The `request`/`lock_wait_begin` + `lock_wait_end{waited_ms}` pair and the `daemon_lock_wait/<request>` row in `server perf-summary` (3.0.103+). Cross-check by counting client PROCESSES that started in the window (`gui`/`startup`/`main_enter`) against `begin` records — a process that started and never logged a `begin` is a blocked request, not an absent one |
| `session remove`'s `verified:true` on a `remote-cc://` row | Always, today. It reaps the LOCAL ssh client and reports success; the remote agent keeps running under the remote host's own daemon | `ps` on the REMOTE host before trusting it (see pending-bugs) |
| ⛔⛔ `server app terminal adopt` / `server app terminal new` `accepted:true` + `capture_faithful:true` + `problem None` as “the ether row is attached and its history is here” | **Always, when the outer PTY was not transplanted.** `adopt` creates a **plain host Shell** (`Shell` `LiveLocal` `exec '/bin/bash' -i`, `105×48`) and `reptyr -T <pid>` runs *inside* it; `accepted:true` means the shell was created, `capture_faithful:true` means the shell's own prompt paints faithfully, `problem None` means the shell's xterm is topmost — none proves the outer PTY's scrollback/history was stolen. Measured 2026-08-18 on a headless host and the GUI host: two `adopted-plain…` shells were created, reordered to top `Live Sessions 54`, screenshotted faithful, reported `adopted-muse-pts29` name — yet `server snapshot` `terminal_lines` / `app terminal read-buffer` `text` for both was only `pi@…:~$` (1 line, 48 rows, `Terminal=false` history) vs the ether yggterm rows' `cc-runtime://…` / `codex-runtime://…` (`claude_code`/`codex` with `Queue live…` / `Resume daemon-owned…` + prior turns). The same shape as the one-shape law: the verb answered “a row exists” while the ask was “the row *contains* the ether’s history”. This manufactured two plain rows at top that the human rightly rejected | Verify the **history**, not the shell: `server snapshot` `live_sessions[]` `terminal_lines` (or `server app terminal read-buffer --mode screen --session <path>` `text`) must contain the ether’s prior output (not just `pi@…:$`), and `kind`/`title` must match the ether row’s (`claude_code`/`codex`, not `shell` `adopted-plain…`). A newly-adopted row with `line_count 48` `nonblank 1` + `text ~ ^pi@.*:\~\$ $` is a host shell that never transplanted — report `adopt_refused` with `reptyr` stderr/pty reason instead of claiming attach. Never use `problem`/`faithful` alone as evidence of transplanted history |
| `pkill -f <pattern>` / `pgrep -f <pattern>` | **Always — and `pkill` is the lethal form.** The pattern matches the `bash -c` running it, so `pkill -f my_fixture.py` kills your own shell mid-teardown (exit 144, seen 2026-07-27). `pgrep` merely lies; `pkill` takes the session with it | `ps -eo pid,args --no-headers \| awk '/[m]y_fixture\.py/ {print $1}'` then `kill` — bracket the first character so the pattern cannot match itself |
| `terminal read-buffer` | It is **two instruments wearing one name.** On a MOUNTED row the GUI CLIENT answers (wrapped in `request_id`/`handled_by_pid`/`data`); on an UNMOUNTED row the DAEMON answers (**top level, no `data` wrapper**, `source:"daemon_screen"`, `client_host:"missing"`). Different freshness, different shape, same verb — and the client's copy can trail the daemon's screen | Read `source` and check whether the reply is wrapped before believing a "missing" line; cross-check against a faithful screenshot |
| `element.hasAttribute('data-web-tab-active')` (or any `data-*` flag) | Whenever the attribute is rendered with the literal value `"false"` — presence is not truth, so every tab reads "active" | `getAttribute(...)` and compare the VALUE |
| `grep` over the working tree for shipped behaviour | Whenever your lane branch is behind `main` — you read source the running binary was never built from, and conclude a shipped knob does not exist | `git show origin/main:<path>`, and check `git log --oneline origin/main` first |
| **A contributed rail pane you are looking at** | The app's daemon predates the installed binary. The pane's schema is served by the DAEMON's resident code, so a shipped, committed, deployed fix renders as the pre-fix pane and every instrument agrees with it. The documented ychrome landmine says a stale daemon REFUSES a new invocation; it has a quieter mode where it ACCEPTS one and serves the old schema (measured 2026-08-02: daemon started 11:58, binary installed 21:11, Edit tab drew the pre-fix wording verbatim) | Compare the daemon process's start time against the binary's mtime BEFORE reading anything into the UI — `ps -eo pid,lstart,args \| grep '[y]chrome --daemon'` vs `ls -la ~/.local/bin/ychrome` |
| `/proc/<daemon-pid>/exe` as "where the installed binary is" | Whenever a deploy RENAMED rather than replaced in place. The link follows a rename, so after `mv yggterm-headless yggterm-headless.old.$$ ; cp new yggterm-headless ; rm -f *.old.*` it reads `…/yggterm-headless.old.4121874 (deleted)` — the grave of the OLD binary, not the install sitting beside it. It names where this process's file WAS. Reading it as the install is what skipped the PTY handoff on `dev` 2026-08-09 and cold-killed **55 live terminals** | The canonical name in that directory (`…/yggterm-headless`), with the exe link as one candidate among several — never as the answer. `disk_replace_handoff_candidates` is the one place that decides this |
| **`observations` on a terminal open attempt** | **Almost always, and it reads as a fault.** Nothing in the GUI produces one: the only caller of `observe_terminal_open_attempt_from_viewport` is the app-control `DescribeState` handler, so the counter measures **how often an agent asked the GUI to describe itself**, not the surface's health. Measured on the desktop host 2026-08-11: **24 of 33 attempts had `observations: 0` and 15 of those reached Ready anyway.** Reading `observations: 0` as "this attempt never got a surface reading" points an investigation at an ingest path that is working exactly as built | The attempt's own `state` and `ready_at_ms`, and the `terminal_open_attempt/ready` event's `extra` — which names the latch (`reason: …` for a `mark_ready`, or the `settled_kind`/`interactive` fields when the viewport observation did it). ⚠ And note what that distinction exposes: an attempt latched by the viewport path became usable **because an agent was watching**, so polling `describe-state` while investigating this area changes the outcome you are measuring |
| The **unsuffixed** `~/.yggterm/event-trace.jsonl` as "the current GUI's trace" | Whenever a short-lived CLI has run — it writes there too, and the GUI is usually on a generation-suffixed file. Following the name instead of the process sends you through a dead pid's history (twice now, ~10 minutes each) | Identify the writer: the file whose fd is held by the `yggterm` GUI process. `for fd in /proc/$(pgrep -x yggterm)/fd/*; do readlink -f $fd; done \| grep event-trace` |
| `pgrep -f "<the GUI's home path>"` as "find the GUI process" | **Always, and it is a different failure from `pgrep` matching your own shell.** A launch command exports the home directory into the environment it hands down, so that string is present in the command line of **every PTY child the GUI ever spawned**. The pattern does not fail to match — it matches a crowd, and the first hit is whichever descendant happened to sort first. Found 2026-08-14 while checking whether the running window was stale, which is precisely the question a wrong answer here settles backwards | **Identify by BINARY, not by any string the process inherited.** `pgrep -x yggterm`, then confirm with `/proc/<pid>/exe` and its md5 against the installed file. Anything a child could have inherited is not an identity |
| A directory listing of `~/.yggterm/server-*.sock` | Always, if you read it as litter. guihost holds **633** of them going back to 2.1.x and **every one accepts a connection**: all but the current version are SYMLINKS the daemon retargets to its live socket at startup (`refresh_legacy_server_socket_aliases`) so an older client can still find it. Sweeping them deletes the cross-version compatibility plane | `ls -l` (they are symlinks, not sockets), or connect and read `server_version` |
| **`server app update restart`** used to make the DAEMON current | **Whenever the GUI is already the newest build — i.e. after every successful deploy.** It is a GUI verb: `RestartPendingUpdate` → `restart_into_pending_update`, which **returns silently** when no newer GUI is staged, and the reply is still `error: null` with a full state dump. So on a host whose GUI is current and whose daemon is three builds behind, the correct-looking call changes nothing, reports success, and leaves the same pid. Two clusters read that as "the update gate is stuck" and went looking for the gate; the gate was never consulted | The daemon swap is a different mechanism entirely — the metadata rail's hot-restart (`ServerRequest::HotRestart`, no idle gate, preserves every runtime). To see whether a swap is owed, read `server daemons` (3.0.124+ prints the queued swap and what it waits for); to see the two versions, `daemon_update_state.current_gui_version` vs `.active_daemon_version` |
| `hot_update_handoff_prepared {spawn_ok: true}` | **Always, as evidence a successor exists.** It records that the *spawn syscall* worked. A child that re-execs to the wrong version, finds the socket bound and exits 0 produces exactly this line, and the daemon then lingers as a preserved owner waiting for an adopter that was never created | The next `spawned_daemon_exit`, and — the fact that actually answers it — whether a daemon at or above the target version is now live (`server daemons`) |
| `git rev-list --all \| wc -l` **used to prove a history rewrite's old lineage is gone** | **Always after a rewrite, which is the only time you ask.** `--all` means all **refs**, not all objects you are keeping alive, and the reflog is precisely what still holds the lineage you just stopped naming. Measured 2026-08-14 across three clones of one repo: one clone read **278** — exactly matching the server, which is why two independent sweeps called it clean — while `--all --reflog` read **728**. It was storing **450** dead commits. A second clone read 343 against 840. 947 reaped in total. ⚠ The matching number is the trap: a correct count of refs coinciding with an unrelated correct count of commits reads as confirmation | `git rev-list --all --reflog \| wc -l`, then reap with `reflog expire --expire=now --all && gc --prune=now` — but only behind refusals on a dirty tree, unpushed commits, or a non-fast-forward. ⛔ A `refs/stash` based on the dead lineage anchors it against every gc and is someone's uncommitted work: save `git stash show -p` as a patch and resolve the owner, never just drop it |
| `git filter-repo` **as an operation scoped to what it CHANGES** | **Whenever any commit in the walked range is GPG-signed.** It re-creates every commit object it walks and does not carry the `gpgsig` header across, so a full-history run **de-signs commits it had no reason to touch** and reports success. Measured 2026-08-14: a five-commit purge changed **15** commits — the 5 intended plus **10 with byte-identical trees**, de-signed for nothing. The tell is a changed SHA whose `^{tree}` is unchanged; `diff <(git cat-file commit <old>) <(git cat-file commit <new>)` shows the signature block as the entire delta | Scope the run: `--refs <parent-sha>..refs/heads/main`. Re-sign the ones genuinely rewritten with `git commit-tree -S`, replaying oldest-first and carrying `GIT_AUTHOR_*`/`GIT_COMMITTER_*` across — ⛔ `git rebase` is not the substitute, it re-dates the committer silently. ⭐ General shape: **a rewrite tool's blast radius is what it WALKS, not what it CHANGES**, and anything living in a commit header rather than the tree is destroyed by being walked |
| A hand-rolled pattern set **used to decide whether a history lineage carries private data** | **Always, and it fails silently in the reassuring direction.** Three plausible patterns returned **0, 0, 0** on a lineage that the privacy guard, run over the same range, refused outright. A guessed pattern set is a guess about the boundary: it reports CLEAN for a term it never had for exactly the same reason it reports CLEAN for a term that is genuinely absent | The scanner that owns the wordlist and prints what it checked: `ygg-privacy-guard scan --rev-range <range>`. ⚠ Its default range is unpushed-only, so on a clean tree it reports **`0 added lines`** and a green tick — **read the added-line count, not the tick** |

**The rule underneath all of them:** if the symptom is visual, the proof is a
faithful pixel. Telemetry that says "healthy" while the user sees a broken screen
means the telemetry is wrong, not the user.

**The generalisation worth carrying to any codebase:** the dangerous instrument
is not one that fails loudly — it is one whose *failure value is indistinguishable
from a legitimate negative result*. `null`, `0`, `[]`, and "unchanged" are all
answers a broken probe gives just as readily as a healthy system does. Whenever a
probe can return one of those, make it carry a term that proves the probe itself
ran (a `sanity` value, a key-existence check, a known-nonzero control). A probe
that cannot fail loudly must be made to succeed loudly.

**★ The same rule applies to REPAIRS, not just probes (learned 2026-07-22, at the
cost of several sessions).** A repair built on a primitive that silently does
nothing is indistinguishable from a repair that ran and failed to help — so the
investigation goes looking for a *second* bug that does not exist. `term.open()`
was the case: it early-returns once `term.element` exists, so `host.innerHTML=""`
+ `term.open(host)` looked like a rebuild and was pure loss. Three sessions of
husk investigation widened the *guards* around a repair that could never have
worked. **Before deciding a fix didn't fix it, verify the primitive underneath it
does what you assume** — and when the primitive belongs to a vendored library,
prove it in `tools/xterm-harness/` (jsdom + the EXACT shipped bundle, minutes to
write) instead of arguing from a live symptom. The harness turns
"upstream probably does X" into a test that fails when a version bump changes X.

**⛔⛔ A READER THAT FINDS NOTHING LOOKS EXACTLY LIKE A THING THAT HAS NOTHING (2026-08-13).** An
empty result and a broken reader are the same picture: nothing errors, every count is plausible, and
the failure hides *precisely because* the system survives emptiness gracefully. Four independent
instances landed in one evening — a collection reader keyed on the wrong filename reading two
populated collections as empty; an editor CLI that accepts a path, reports success and drops it, so
`exit 0` plus an empty editor reads as a paint bug; a surface declaring thousands of characters and
painting none; and a subject-comparison whose normalizer returned empty, so every subject "missed"
and it reported sixteen losses that did not exist.

⇒ **THE ACTIONABLE HALF, which is what makes it a rule rather than an epigram: if a reader's silence
looks the same as success on empty input, give it a way to say *"I looked and found nothing"* apart
from *"there was nothing."*** ⭐ And when you add any reader, ask what its silence would look like
**before** you trust its first clean run. Pairs with the both-controls rule below: a positive control
alone cannot detect an instrument that has collapsed to a constant answer.

*The open defects in this family live in `docs/pending-bugs.md`; this entry owns the durable rule and
does not duplicate them.*

**⛔⛔ `launch_phase: RemoteBootstrap` IS NOT A FAULT STATE, AND COUNTING IT PRODUCED A FALSE OUTAGE
(2026-08-13).** During a real socket outage, two sessions independently read `RemoteBootstrap 41 /
Running 10` as *"41 rows are stranded with no PTY"*. It is the ordinary resting state of a row.
**The counterexample is direct, not inferred:** an orchestrator sampled the field while mid-turn and
found **its own row** in `RemoteBootstrap` — along with three other rows that had each sent a
message minutes earlier.

⇒ **Do not infer health, or its absence, from a phase census.** Two samples 45 s apart were
byte-identical, so it was not even a settling system; the field simply does not answer the question
its name suggests. ⭐ **Through the whole incident the only instrument that was right in either
direction was WHETHER ROWS RESPOND** — a delivered message, an accepted submit, a commit appearing.

⚠ **The trap has a specific shape worth naming: during a genuine incident, any alarming-looking
field acquires a causal story for free.** Both readings above were published — one as *"the outage
is over"*, one as *"not resolved"* — and the disputed field was load-bearing in neither. **Before a
count becomes evidence, find one entity you already know the answer for and check the field against
it.**

**⛔⛔ A PRE-PUSH PRIVACY REFUSAL WITH HUNDREDS OF HITS IS USUALLY A STALE BRANCH, NOT A LEAK — AND
THE ONLY LEVER IT OFFERS IS THE ONE THAT SHIPS ONE (2026-08-13).** The guard scans
`<tip> --not --remotes=origin`. That range is the right one — but it is enormous whenever the local
branch is on a lineage `origin` no longer has, which is the normal state of every checkout after a
history rewrite, a force-push, or a long-lived lane that was reset upstream. Measured on one lane,
same repo, same command, before and after reconciling, with nothing about the commits changed:

```
  diverged branch    1,211,081 added lines scanned →  259 hits → REFUSED
  reconciled branch      1,559 added lines scanned →    0 hits → clean
```

Every one of those 259 was mined out of history that had been public for weeks. ⛔ **And the
refusal text names `YGG_PRIVACY_ALLOW=1` as the way forward, which suppresses the WHOLE scan** — so
a session that knows its own commit is clean is steered straight at the override that would ship a
real leak past the guard.

⇒ **Run this BEFORE believing any guard refusal:**

```sh
git rev-list --left-right --count origin/main...HEAD    # left = behind, right = ahead
```

**If the left number is in the thousands, the branch is the problem and the hits are not yours.**
Reconcile (**replay onto the new lineage — never a plain `rebase origin/main`, which duplicates the
whole history, and never a merge, which reinstates the lineage a scrub removed**) and re-run the
scan. It collapses to your own commits. ⛔ Reach for the override only when the scan is already
scoped to what you actually wrote and you have read every hit.

⭐ **The guard now says this itself.** When a refusal arrives on a branch that is behind its
upstream, it reports the ahead/behind counts, names `git fetch && git rebase <upstream>` as the
next step, and **omits the override line entirely** — replacing it with a warning that the
override suppresses the whole scan. The obvious implementation of that check does not work: keying
it on *"the scanned range is much larger than what I am ahead by"* stays silent in exactly the case
it is for, because after a force-push `ahead` inflates in lockstep with the range and the ratio
stays about 1. **Being BEHIND is the signal; the ratio measures nothing.**

⚠ **The guard's wordlist holds whole tokens, and identity does not live only in tokens.** An
enumeration that names several private things by their short stems and supplies the shared suffix
*once* matches no term and passes clean, while a human reassembles it without effort. The guard now
carries a narrow structural check for that one shape — a suffix shared by two or more known terms,
appearing on a line with two or more of their distinct stems as whole words — derived from the
wordlist itself. Deliberately narrow: a general natural-language leak detector is a hole with no
bottom, and a check that cries wolf manufactures the override habit that defeats the tool.
Measured across three repositories' entire published histories: zero hits.

### ⛔ AN EXIT CODE AFTER A PIPELINE ANSWERS A DIFFERENT QUESTION THAN YOU ASKED

```sh
git show <ref>:<file> | grep -c '<pattern>' ; echo $?     # ⛔ this is grep's status ONLY by luck
```

`$?` is the **last** command's status, so a check written this way asks *"did the final stage
succeed"* rather than *"did the match succeed"* — and the two diverge exactly when an earlier stage
fails, which is when you most need the answer. Same family as `cargo test … | tail`, which reports
`tail`'s status and turns a red suite green. ⇒ use `${PIPESTATUS[0]}`, or drop the pipe.

⭐ **And the discipline that catches it: run both controls in the same command.** The instance that
produced this line was only recognised because a negative control returned 0 and a positive control
returned 168 side by side — a single reading of either would have looked like an answer.

### ⛔⛔ `terminal new` RETURNS `active_session_path` — WHICH IS NOT THE ROW IT JUST CREATED

**This one delivered a 200-line brief into a stranger's live session.** The response to
`server app terminal new … --no-activate` carries a field named `active_session_path`. With
`--no-activate` the newly created row is deliberately NOT activated, so that field still names
**whatever was active before** — another campaign's row, mid-work. Read it as "the row I just made"
and every following step is aimed at the wrong session: the readiness probe types into it, and the
submit lands a whole brief in its composer.

⇒ **Resolve the new row by its TITLE, never from that field:**

```sh
server app rows | python3 -c 'import sys,json
for r in json.load(sys.stdin)["data"]["rows"]:
    if "<the --title you passed>" in (r.get("label") or ""): print(r["full_path"])'
```

⭐ **And the check that catches it even if you get the path wrong:** the four-step spawn ends by
**grepping the SUCCESSOR'S OWN TRANSCRIPT for a token from your brief** — and a transcript lives
under `~/.claude/projects/<cwd-slug>/<uuid>.jsonl`, so **the slug itself proves the cwd**. A token
found under the wrong project slug, or a transcript that is megabytes old when a fresh row should be
kilobytes, is the misdelivery announcing itself. Both tells were present and both are cheap.

⚠ Same family as the `input-check` spelling: the verb is **`server app terminal input-check`**, not
`server app input-check`, which answers `unsupported app control command` — easy to misread as "this
build lacks the verb" rather than "the parent verb is missing".

### ⛔ `server app rows` DOES NOT CARRY THE WEDGE FIELDS — `server snapshot` DOES

A brief handed on the claim that `server app rows` emits `input_unanswered_ms` and
`wedge_suspected`, which would make it the instrument for measuring a deaf row. **It does not.**
Measured: 384 rows, and the union of every field name across all of them contains neither.

```sh
server app rows | python3 -c 'import sys,json; rs=json.load(sys.stdin)["data"]["rows"]; \
  print(sorted({k for r in rs for k in r}))'      # ⇒ no input_unanswered_ms, no wedge_suspected
```

`input_unanswered_ms` lives on `SnapshotSessionView` (`daemon.rs`), so **`server snapshot`** is the
owner; `wedge_suspected` is not a field at all but a derived predicate —
`input_unanswered_suggests_wedge()` against `INPUT_UNANSWERED_WEDGE_SUSPECT_MS` in `yggterm-core`,
which is the single owner of the threshold that gate, row payload and sidebar all read.

⭐ **The lesson is the shape, not the fields:** the claim arrived in a handover as an established
fact and was one command from being falsified. A relayed measurement is a CLAIM until you have run
it yourself — and building a measurement on top of an instrument that does not exist is how a whole
lane's numbers turn out to be about nothing.

### ⛔⛔ `git log -S` SKIPS MERGES, SO A DELETION THAT ARRIVED VIA A MERGE READS AS "NEVER EXISTED"

**This is the correction to a law this campaign already carried, and the missing
half is what resurrected a queue entry on `main`.**

The standing rule was: *a conflict whose upstream side is EMPTY is textually
identical to a deliberate deletion — `git log -S '<phrase>' -- <file>` settles
it.* True, **and only with `--full-history -m`.** Plain `git log -S` does not
walk merge commits, so a deletion that reached `main` *through* a merge is
invisible to it and the command answers **"this was never here"** — which is
exactly the answer that tells a lane to keep its own side.

⇒ Measured 2026-08-14: a lane hit three empty-upstream conflicts, ran the bare
form, got zero commits for each, correctly concluded "never had it" for two —
and **wrong for the third**, which `main` had held and deleted as fixed. The
entry came back. The instrument was right, the invocation was not, and the
failure direction is always toward resurrection.

```sh
git log --oneline --full-history -m -S '<distinctive phrase>' -- <file>
```

⭐ **The general form: a history query that silently excludes a class of commit
answers a narrower question than the one you asked** — and here the excluded
class is *precisely* the one that performs integrations, which is where
deletions cross branches.

### ⭐⭐ THE HEADING DIFF — catch a resurrection WHILE THE CONFLICT IS OPEN, not after

`check-queue-resurrection.sh --strict` catches a resurrection **after** it is
committed. This catches it while you can still fix it for free, and it is immune
to the merge hole that `git log -S` has, because it compares **whole files**
rather than querying history:

```sh
git show origin/main:docs/pending-bugs.md | grep "^## " | sort -u > /tmp/main-h.txt
grep "^## " docs/pending-bugs.md                | sort -u > /tmp/mine-h.txt
comm -23 /tmp/main-h.txt /tmp/mine-h.txt   # on main, gone from mine → MY deliberate deletions, and nothing else
comm -13 /tmp/main-h.txt /tmp/mine-h.txt   # mine, not on main   → MY new entries,      and nothing else
```

⭐ **Both lists must be exactly what you intended**, and the **first** is the one
that matters: anything in it you did not mean to delete is a deletion you are
about to perform, and anything *missing* from it is an upstream deletion you are
about to undo. A correct queue merge usually shows one line each way.

⇒ **The instance that earned it, within an hour of the gate being fixed:** a lane
resolving its own conflict took *ours* on the hunk it was editing, which would
have resurrected a different lane's entry that upstream had deleted as fixed.
**The conflict gave no sign** — the resurrection was a side effect of a correct
resolution to the hunk actually in dispute. Only the whole-file audit saw it.

### ⛔⛔ A GATE THAT REPORTS A VIOLATION AND EXITS 0 IS DECORATION

`check-queue-resurrection.sh` defaults to **report-and-pass**; `--strict` is what
makes it exit 1. That default is right for a human reading output and wrong for
every automated caller — and the campaign's own standing instruction (*"run this
after any queue merge"*) never mentioned the flag.

⇒ **A resurrection therefore sat on `main` through several push loops**, each of
which ran the check, saw exit 0, and reported the push clean. Three separate
sessions "ran the gate" and none of them gated.

- ⛔ **In any script or push loop: `--strict`.** Without it the check is a
  comment.
- ⚠ It now prints `EXITING 0 ANYWAY — findings above are REAL` on the default
  path, because the previous output was indistinguishable from a pass to
  everything except a careful human.
- ⭐ **The pattern to look for elsewhere:** any checker whose *default* is
  advisory. Its findings are invisible to exactly the callers that cannot read.

### ⛔⛔⛔ A "FAITHFUL" FRAME OF A DOCUMENT SURFACE SHOWED THE TERMINAL, NOT THE DOCUMENT

**Found 2026-08-14 while root-causing the document surface that painted garbage.
This is the instrument an agent is instructed to trust for any visual claim, and
on this surface it was compositing the wrong pane.**

Two independent things made it lie, and each alone was enough:

1. **The screenshot compositor painted the xterm canvas over a document surface.**
   The row's own state reads `active_view_mode: Terminal`, so the compositing
   path drew shell output across the document it was asked to photograph.
2. **`terminal_host_visibility_style` returns `opacity:1; visibility:visible;`
   unconditionally**, so a terminal *covered* by another surface keeps painting.

⇒ ⭐ **The consequence is the part to remember: when the document body failed to
mount, the terminal WAS the document** — and the owner's "corrupted glyph
clusters" were simply shell output showing through an empty pane. **The reported
symptom was the instrument, not the subject.** Two frames 24 s apart, same row,
`Document` selected in both, settle it: the "clean" frame shows the identical
lines the "garbled" one renders as mojibake.

### ⛔⛔ A RAIL THAT RENDERS DOES NOT EXONERATE THE VIEWPORT — DIFFERENT PANE, DIFFERENT BATCH

**An app declares its rail and its document body as two SEPARATE panes, fetched
separately and applied in separate webview edit batches.** So "the same app
renders the identical schema completely in the rail, in the same minute" proves
only that the rail's batch applied. It says nothing about the viewport's.

⇒ **That reading closed the wrong door twice.** A published entry used it to rule
OUT the lost-edit-batch class ("a frozen subtree cannot render the same schema
completely elsewhere") and to conclude the viewport had a render-path defect of
its own. It was the lost-batch class, and the rail was the one instrument
structurally incapable of noticing. Proven by an A/B on one probe: pre-fix binary
→ zero markdown nodes and a blank body; fixed binary → the body renders.

⛔ **And "it reproduces on demand" does not rule the class out either.** If the
throwing mutation is emitted on every render of that subtree, the fault re-fires
deterministically — reproducibility is what this class looks like, not evidence
against it.

⭐ **The instrument that does discriminate is `webview_edit_faults`**, and it is
monotonic: read it in `server app state` at the moment you reproduce.

⚠ **Same entry, second inverted conclusion:** it reported the surfaces prose
wrong for saying `list-row` renders at document scale, and told the next reader
to correct it. The host partitions `Markdown | TextInput{multiline} | ListRow`
into the body — **the prose was right and the doc comment beside the code was
merely incomplete.** Correcting the docs on that advice would have made them
wrong. ⇒ **When prose and code disagree, read the code that RUNS, not the comment
above it.**

⛔ **So `capture_faithful: true` answers "was the xterm canvas composited", NOT
"is this what the user sees".** On a terminal view those coincide. On a document
surface they came apart completely, and nothing in the reply said so.

✅ **BOTH HALVES FIXED 2026-08-14 — but read what was fixed, because one of them
was not fixed where you would look for it.** The compositor now refuses to
composite the xterm canvas when a document surface owns the viewport.
`terminal_host_visibility_style` **still returns `opacity:1; visibility:visible;`
unconditionally** and is *correct* to: the standdown is a CSS rule keyed on
`[data-document-surface-owns-viewport="true"]`, beside the `pointer-events` rule
that was already there. ⚠ So a reader who greps that function will conclude the
second half is still open. It is not — grep `DOCUMENT_SURFACE_STANDDOWN_CSS`.

- ⚠ **The fix is in the BINARY, so the question is which binary is running.** On
  a GUI older than this landing, both halves are still live and a
  document-surface screenshot still lies. Settle it by identity
  (`docs/deploy-spec.md` §1), never by version.
- ⭐ **`webview_edit_faults` remains the field to read** whenever a surface
  renders nothing while its state reports healthy — it was the only one that
  moved throughout (4 in the repro, 0 after) while
  `has_schema: true, error: null, visible: true` all reported fine. That is not
  specific to this defect: it is the general tell that a subtree stopped
  tracking its state, and it is monotonic, so a non-zero reading means damage
  has happened even if the screen currently looks right.
- ⭐ **The general rule this belongs to:** *an instrument that composites more
  than one source must say which source it drew.* A boolean that means "the
  compositor ran" reads as "the picture is true", and those are different claims.

### ⛔⛔ `cmd | tail -1 && echo OK` REPORTS THE PIPE'S SUCCESS, NOT THE COMMAND'S

**Caught 2026-08-14 in a push-retry loop that had been used all session.** A bash
pipeline's exit status is its **last** stage, so:

```sh
git push origin HEAD:main 2>&1 | tail -1 && echo PUSHED     # ⛔ prints PUSHED on a REJECTED push
```

`tail` succeeded, therefore the `&&` fired, therefore the loop announced success
and broke out — on a push the remote had refused. The retry loop that existed
precisely to survive a busy `main` was the thing that stopped retrying.

- ⭐ **The failure is silent and it reads as diligence**, because the loop *looks*
  like careful engineering and its log line says the right word.
- ⇒ **Capture the status, do not pipe it:**
  `if out=$(git push … 2>&1); then …; else …; fi` — or `set -o pipefail`.
- ⛔⛔ **AND THEN VERIFY THE EFFECT, NOT THE VERB.** Even a correct exit code is
  the command's opinion. The check that cannot lie is
  `git merge-base --is-ancestor HEAD origin/main` after a fetch. Same rule as
  every row verb in this fleet: **read the state back**.
- ⚠ The saving grace here was that the *earlier* pushes were verified by ancestry
  when the discrepancy surfaced, which is how it was established that only the
  last one had been lost rather than an afternoon of them.

### ⛔⛔ THE ONE-WAY DOOR — a classification whose evidence stops updating once you classify

**Two campaigns hit this in the same week, with the arrow pointing opposite ways, and
neither instrument ever reported an error.** The generalisation is worth more than
either bug:

> **A classification derived from a signal that stops updating once the class is
> entered is a one-way door.**

- **Evidence FREEZES at classification.** A watchdog decided a session was rate-limited
  by reading the tail of its transcript. A rate-limited session stops writing, so the
  tail says the same thing forever — the class could be entered and never left, and
  every re-read looked like fresh confirmation.
- **Evidence ERODES as the work succeeds.** A repair tool selected its worklist with a
  *what is still broken* query and then judged ordering from that same set. Rows left
  the set as they were fixed and took their anchors with them, so **the tool grew more
  confident the more of its own work had succeeded.**

⭐ **Both get more confident the longer they are wrong, and neither can self-correct.**
That is why both presented as *"the instrument reports healthy"* rather than as a
failure — which is the hardest shape to find, because nothing is complaining.

⇒ **THE QUESTION THAT CATCHES BOTH, cheap enough to ask of any state machine:**
**_what writes the signal I classify on, and does it keep writing once I have
classified?_** If the answer is *"the thing I just parked"*, the class has no exit.

⚠ Companion, from the same pair of incidents: **any check whose input is a TODO query
is suspect — ask what leaves the set when the work succeeds.**

### ⛔ A WORKAROUND THAT ALWAYS WORKS HIDES THE THING IT WORKS AROUND

Host discovery in the row-claiming script was broken outright — not flaky, **broken on
every host, every time**. It went unnoticed for a long time because every brief and
every pre-spawn checklist told sessions to pass the host explicitly, and that path
always worked. **The workaround was load-bearing and nobody knew**, because a
reliable workaround produces no failures to investigate.

⇒ When a fix removes the need for a workaround, **go and correct the documents that
mandated it**, recording that it is no longer load-bearing rather than quietly
dropping the flag. Otherwise the next reader cannot tell a live requirement from a
fossil, and the flag propagates forever.

### ⛔⛔ THE TWO PRIVACY GATES DISAGREED, AND THE WEAKER ONE GATED PUSHES

**Fixed 2026-08-14. Recorded because the shape outlives the instance.**

A relay brief carrying an absolute personal home path reached a public lane
branch, and the pre-push guard returned a green tick. The repo-local gate
(`scripts/check-privacy.sh`) refuses that class as its **first** rule. So a class
one checker treats as rule one, the other did not implement **at all** — not a
weak version, none — and the one that runs on every push, in every repo, on every
host was the blind one.

- ⛔ **When two checkers disagree, the question is not which is right — it is
  which one GATES.** A strict checker nobody's push runs through is a document.
- ⭐ **The fix has to go in the WRITER, not the repo that noticed.** A rule added
  to one repo's checker protects one repo; the same rule in the shared pre-push
  guard protects every repo on every host, which is where a backstop belongs.
- ⚠ **A rule that fires on its own prescribed remedy is worse than no rule.** The
  cure for this finding is to write `/home/user`, so the pattern needs a
  placeholder allowlist and a left anchor — without the anchor a URL ending
  `/gp/w/home/activity` reads as a personal home path. **A check that cannot see
  its own remedy becomes noise, and the noise is what teaches the override.**
- ⭐ **Falsify a guard in both directions before trusting it:** that it now
  catches the exact commit it previously passed, *and* that it stays silent
  across a large body of already-public history. Verified here at 3,788 added
  lines over 40 commits of `main`.

⇒ And the standing consequence: **`scripts/check-privacy.sh` is the stricter gate
and it is repo-local.** Run it before pushing docs. A green pre-push guard is a
statement about the classes that guard implements, not about your file.

### ⛔⛔ A CLEAN PRIVACY-GUARD RUN IS NOT EVIDENCE THE FILE IS CLEAN

`ygg-privacy-guard` scans **the commits a push is carrying**, by design — that is
what makes it fast and what makes it fair to the author who did nothing wrong.
The consequence is not obvious and it bites:

⇒ **A private term already on `main` is permanently invisible to it.** It will
never be in anyone's push range again, so every future push over that file
returns a clean result while the term sits in the public tree.

Found 2026-08-14: three identifiers — a `username@hostname` shell prompt, a
private wildcard domain, and a personal video URL — were live in
`docs/pending-bugs.md` on `origin/main`, through a full-object history sweep the
day before **and** through several clean guard runs.

- ⭐ **Scan the WHOLE FILE, not your diff.** That is how these were found, and it
  costs one `grep`.
- ⛔ **And "pushed" is not the finish line for a removal — "on the public default
  branch" is.** A lane push is correct practice for lane work and *wrong* for a
  redaction: what a stranger reads is `HEAD` of `main`. **Verify by ancestry**
  (`git merge-base --is-ancestor <sha> origin/main`), never by your own push
  reporting success. Caught the same day: a removal was reported as landed while
  it sat on a lane branch, and the exposure stayed live in between.
- ⚠ **Both halves are the same shape**: an operation reported success about a
  different question than the one being asked.
- ⛔⛔ **AND THE SUBTLER FORM, which is the one that will catch a careful agent:
  A PASSING CHECK DOES NOT TELL YOU WHICH ACTION CAUSED THE STATE IT OBSERVED.**
  The removal above was verified with the *right* command, which returned the
  *right* answer — but it was run after a `fetch`, by which point another
  session's merge had already produced that state. The check was sound; the
  **attribution** was false, and the report credited the wrong action. ⇒ When two
  events could each explain a state and you sampled only after both, you have
  measured the state, not the cause. **Sample between them, or name the
  ambiguity.** Same shape as comparing an API read against a screenshot taken at
  a different moment, which this project has already paid for once. The guard answered "is this push
  clean", not "is this file clean"; the push answered "did the write succeed",
  not "is it visible on main".

### ⛔ A DOM READ IS `server app dom-eval` — `chrome type --assert` TYPES FIRST

`server app chrome` offers only `type`. Its `--assert <selector>@<attribute>` is a real DOM read but
it writes a keystroke before reading, so it can never be pointed at a GUI a human is using.
**`server app dom-eval "<js>"` runs JS in the GUI webview and returns the serialized result without
typing** — pass the script POSITIONALLY, flags after it, or a leading flag makes it evaluate the
flag string. Base64 the script through `ssh` to keep quoting honest.

⛔ It answers only on the host where the GUI runs, and only to a registered client — a headless host
has a daemon and sessions but no client. `server app clients` answers "is the GUI here" directly.
⭐ A GUI with its own `YGGTERM_HOME` (a sandbox, another lane's rig) is reachable by pointing that
variable at it, which is how a second live instance can be READ for comparison. Never DRIVE one:
a mode change or relaunch corrupts whatever it is measuring.

### 1.99 ⛔⛔⛔ THE INSTRUMENT THAT INCLUDES ITSELF IN WHAT IT MEASURES

**No table row above predicts this one, because it is not a property of any
instrument — it is a property of where you pointed it.** Six sightings on
2026-08-14, across two seats, in one day. Every one was GREEN or QUIET and
WRONG, and the green looked exactly like success:

| the check | what it read that it should not have |
|---|---|
| `until ! pgrep -f "cargo test --workspace"` | the WAITING SHELL's own command line — the pattern is in it ⇒ waited forever for itself. **No suite ran for an hour** while progress was read from a log file that did not exist |
| `pgrep -f -- "--session-id"` + a `case` on a peer's uuid | the SEARCHING SHELL, because the uuid was typed into the pattern ⇒ a report addressed to the orchestrator **was delivered to its author** |
| `pkill -f "ygg-booter.py watch"` | its own command line ⇒ **killed the shell that ran it** |
| a source lock scanning a binary WHOLE | that binary's own TEST assertion, which is written against the very string being sought |
| the same lock after stripping tests | ⭐ **its own COMMENT, which named the needle while explaining the defect.** Introduced *in the act of documenting the class* |
| a stall detector reading a row's screen | a THINKING row, reported as a corpse |

⇒ **The failure is always the same shape and never looks the same twice.** A
process pattern that matches the searcher; a source scan that matches its own
prose; a probe that supplies the state it is testing for (see `getComputedStyle`
in the table above — same class, discovered separately).

#### ⭐⭐ THE ONLY DEFENCE THAT WORKS: MAKE IT FAIL ON PURPOSE

**A check never observed failing has not been tested — it has been run.** The
reachability lock in `app_control_cli.rs` passed its whole life, passed on
`main`, and passed under a mutant that darkened every real dispatch. Three
greens, all meaningless, until one decoy.

⇒ For any check whose input includes the file or the process it lives in:
**mutate the thing it watches and REQUIRE it to go red**, then restore and
require green. Both directions, same run. If it stays green under the mutant, it
is reading something other than the code.

#### ⛔ AND THE CONTROL IS THE NEWER, LESS-EXAMINED HALF — SUSPECT IT FIRST

The corollary, paid for the same day by the seat verifying the fix above. Their
mutant renamed the call sites and got GREEN, and they nearly reported the fix as
broken. The lock searches for `server_cli::`, and each binary carries **nine** of
those — eight call sites plus **an import that was not renamed**. The module was
still genuinely reached. **The lock was right both times; the control was the
untested instrument.**

⇒ When a control disagrees with a fix, check the control before drafting the
accusation. And an incomplete mutant fails in the *reassuring* direction, which
is why it is not self-announcing.

⚠ **A green suite is not evidence a fix worked.** Two lanes fixed one red lock
independently and identically that day; one version stripped tests and comments
and one did not. Both were green. Only the decoy separated the fix from the
placebo.

### ⛔⛔ A DAEMON'S BINARY PATH CANNOT TELL A SANDBOX INSTANCE FROM THE REAL ONE — `HOME` CAN

**Measured 2026-08-20, during an input lockout the owner had to report through a temporary
session.** The diagnosis in flight was "two daemons are fighting over one socket path", naming the
real installed daemon and a build out of a lane's worktree, and the remedy on the table was to kill
the second one. Both halves were wrong about that process, and killing it would have destroyed a
lane's fixture without touching the lockout.

`pgrep -af yggterm-headless` shows the binary path, and that is the one fact which does NOT settle
it: a sandbox daemon, a lane's test daemon and the real one are all `…/yggterm-headless server
daemon`. What separates them is the HOME they were started with.

```sh
for p in $(pgrep -x yggterm-headless); do
  printf '%s %s\n  %s\n  %s\n' "$p" "$(readlink /proc/$p/exe)" \
    "$(tr '\0' '\n' < /proc/$p/environ | grep -E '^(HOME|YGGTERM_HOME)=' | tr '\n' ' ')" \
    "$(ss -xlp 2>/dev/null | grep "pid=$p" | awk '{print $5}' | tr '\n' ' ')"
done
```

Three daemons on one host that day, and the environ read them apart in one pass: the real one on
`$HOME/.yggterm/server-<ver>.sock`; one under the under-glass sandbox's throwaway home
(`/run/user/<uid>/yggterm-uglass/<arm>/home`), listening on a socket INSIDE that home; one under a
lane's scratchpad `fakehome`. **Three homes, three socket paths, no contention** — the
`another yggterm daemon owns the socket bind lock` line in `daemon.log` is a losing bind attempt
from a start-up race, and the repeated `writing daemon response: Broken pipe` beside it is CLIENTS
hanging up before reading their reply, which is a client-timeout signature and not a server fight.

⇒ **Before naming a daemon as a suspect, ask three questions in this order, and none of them is
"which binary is it":**
1. **Which HOME?** `/proc/<pid>/environ` — a fake home cannot affect the user's rows at all.
2. **What does it LISTEN on?** `ss -xlp` — a daemon on a different socket path is not competing.
3. **Does it own a PTY?** `ls -l /proc/<pid>/fd | grep /dev/pts` — zero means killing it destroys
   no session, and also means it was never the thing holding a keystroke.

⚠ And the shape worth carrying past this incident: **an agent's own sandboxes and fixtures look
exactly like production to a process listing.** Any instrument that identifies a component by its
image path will accuse them, most loudly during an incident, when the pressure to act is highest.

### ⛔⛔ A `settings.json` EDIT UNDER A RUNNING GUI DOES NOT HOLD — THE GUI WRITES ITS OWN COPY BACK

**Measured 2026-08-20.** The interface LLM's plan quota ran out, so the title chore was spending
nine seconds per call to be refused. `interface_llm_model` was edited in `~/.yggterm/settings.json`
on the desktop host, the write was read back and confirmed, and a probe against the new model
returned 200. **Thirty-four minutes later the file said the old value again.**

Nothing failed and nothing warned. The GUI holds settings in memory and persists the whole
structure when anything in it changes; that write is a snapshot of what the GUI believes, and it
does not merge. So an outside edit survives exactly until the next time the user or the app touches
any setting at all.

⇒ **Rules:**
- **Never configure the app by editing its settings file underneath it.** A read-back proves the
  write landed, not that it will still be there — and the interval before it is overwritten is
  unbounded, so a check a minute later is not evidence either.
- **A maintenance verb takes the value as a FLAG.** `server titles sweep --model <id>` names the
  model for one run and touches no state, which is the shape any agent-run sweep should have.
- **If a default genuinely must change, it is the owner's to change, through the app.** The file is
  the app's output, not its input.

⚠ The general form, and it is the reason this sits in the field guide rather than in a bug entry:
**a long-lived process that owns a file will restore it from memory, so the file cannot be used as
a channel INTO that process.** Same shape as reading `/proc/<pid>/environ` for flags applied with
`set_var` after exec — the artefact on disk and the state in the process are two different things,
and only one of them is running.

### ⛔⛔ A CHECKER THAT DIES WHEN IT PASSES — the success path is the least-exercised one

**The tell:** a status hook that has run for weeks suddenly ends in a shell error
instead of a verdict, and the fleet looks broken at exactly the moment it became
healthy.

The fleet daemon audit crashed with `SPLIT: unbound variable` **the first time
every host agreed**. Cause: `${#ARRAY[@]}` on a *declared but never assigned*
associative array is an unbound variable under `set -u`. A host had been split
on every previous run, so the array was always populated and the clean branch had
never executed. ⇒ **The path that reports "all good" is the path least likely to
have been run**, and its failure is the one most likely to be misread as a real
problem with the thing being checked.

⚠ **And the obvious hardening is wrong, which is the second half of the lesson.**
`${!ARRAY[*]-}` looks like "keys, with a safe default". It is not: combining `!`
with a default operator turns *key* expansion into *indirect* expansion, so bash
takes the array's VALUES as a variable name and dies with
`1 1: invalid variable name` — on precisely the populated case the branch exists
to handle. The first fix therefore moved the crash from the empty case to the
real case, and passed a test that only exercised the empty one.

⭐ **`${!ARRAY[@]}` (keys) is safe on the same empty array where `${#ARRAY[@]}`
(count) is not.** Measured, not assumed — and the way to know is to test BOTH
branches, including the one you did not change.

### ⛔⛔ `strings <binary>` PROVES A DEPLOY REACHED THE DISK, NEVER THAT IT IS RUNNING

**The instrument:** `strings -a ~/.local/bin/yggterm | grep -c "<marker>"`, the
wave rule's own deploy check — chosen precisely because `--version` and md5 cannot
catch a peer overwrite.

**What it cannot see:** a deploy writes binaries; a PROCESS only adopts a new one
when it restarts. The daemon swaps itself. **The GUI does not** — it cannot
hot-swap its own image in place. So the marker is on disk, the check is green, and
the fix is not running.

⇒ **Ask the live pid instead:**

```bash
grep -c "<marker>" /proc/<pid>/exe      # the image actually executing
readlink /proc/<pid>/exe                # a "(deleted)" suffix = replaced underneath it
ps -o lstart= -p <pid>                  # started BEFORE the deploy? then it is the old image
```

**The instance (2026-08-20).** A ledger line read "marker-verified" for a GUI
marker. On disk: present. In the running GUI — started an hour before the deploy,
exe showing `(deleted)` — `grep -c` returned **0**. Three GUI-side fixes were
reported live and were not, including the one the owner was waiting on. The daemon
half of the same deploy *was* live, which is what makes the half-state so easy to
miss: the deploy genuinely worked, for half the product.

⚠ **The follow-on symptom looks like a broken probe.** A new probe added by a
GUI-side fix reads ZERO after its "successful" deploy — and the natural reading is
"the probe is misconfigured". It is not; the code that emits it is not running.
Same shape as the `input/keystroke` blindness, one layer out.

⚠ **And the state does not converge by itself.** The old GUI queues a hot swap
every ~70s; the daemon answers, correctly, *"the replacement binary is not ahead of
this daemon, so no swap can be owed."* Both are right, nothing resolves, and it
repeats until the GUI PROCESS is restarted. Do not read the swap machinery's
presence as the problem being handled.

### ⚠ `ytrace tail --lines N` CAN HAND YOU A COMPLETE-LOOKING OLD WINDOW

`ytrace` reads across ROTATED generation files (`event-trace.g<gen>.jsonl`). Ask
for a large `--lines` and it can return exactly N records **whose newest predates
the live file's own tail** — a full-looking result from a window that ended hours
ago.

**The tell:** the result is exactly the number you asked for, and its newest
timestamp is old. It made a before/after read conclude a host had gone silent
since a deploy, while the live `ytrace.jsonl` was current to the second.

⇒ For a before/after window, read the live file directly (`tail -N
~/.yggterm/ytrace.jsonl`) or keep `--lines` modest and **check the newest
timestamp against `date` before trusting the window**. (Separately: `--since`
without an explicit `--lines` silently caps at 20 records — the opposite failure,
same consequence.)

### ⛔ `ytrace query` AND `ytrace tail` READ DIFFERENT DATA — never publish a query COUNT

Measured 2026-08-21 on the laptop, same host, same home, same window: `ytrace
query --category input --since 90m` returned **zero rows** while `tail --category
input --since 2h --lines 25000` returned **18,336 events**; and the heartbeat's
`ui_blocks_per_min` (computed through the same `ytrace::query::summarize`)
reported **242.6/min** where the incident files hold ~1/min.

**The inflated direction is ROOT-CAUSED AND FIXED (`29fe1440`, same day):**
`summarize`'s `since_ms` is an ABSOLUTE epoch cutoff and the `host_panic`
caller handed it a window LENGTH (300 000) — "everything after 1970-01-01
00:05" — so the density was the lifetime count over five minutes. Both values
`u128`, both honestly milliseconds; only the values could catch it (the field
repeated IDENTICALLY across heartbeats, which a rate cannot do).

**Still open, and it belongs to the ytrace crate, not this repo:** the
zero-rows direction (the CLI's own query path finding nothing where `tail`
finds thousands), and the sibling defect the fix commit files — the memory arm
alarms on swap USED, a LEVEL nothing is obliged to reclaim, so 7.5 GiB of
residue keeps `host_panic_memory` firing forever on a healed host. Until those
land: **events come from `tail --lines N` (plus the newest-timestamp check
above) or the notebooks; a `query`/`summarize` count or rate is not
evidence.**

### ⛔⛔ A WEBVIEW FLUSH-GATE TIMEOUT IS NOT EVIDENCE THE UI IS STALE

`webview_edit_flush_timeouts` counts times the VirtualDom was frozen waiting for
the webview to acknowledge an edit batch, and the log line it used to leave said
"the UI may be one frame stale". **That reads as DOM divergence and it usually
is not.** The batch is delivered over the websocket; the interpreter applies it
*before* acknowledging; and the acknowledgement has its own 1 s `setTimeout`
backstop on the JS side for exactly the occluded-window case the gate's comment
blames. So the common shape is a webview that was SLOW — the edits landed, the
ack was late, nothing is stale.

**The discriminator is `webview_edit_acks_late`, and you must read it first.** A
timeout followed by a late ack means the surface is alive and behind; a streak of
timeouts with NO late ack means the acknowledgement plane is dead, which is a
different fault with a different remedy (the ladder reloads the page, which is a
full remount). ⚠ `webview_edit_faults` is the one that really does mean
divergence — the webview reporting it could not apply a batch — and it is
restart-only. Three counters, three meanings; the middle one is the one that
looks like the worst and usually is not.

⇒ Read all four together in `DescribeState`
(`webview_edit_{faults,flush_timeouts,acks_late,gate_bypasses,resync_requests}`)
or in the `webview_edit_stall` incident payload, never the timeout alone.

### ⛔ A HAND-ROLLED vt100 SCORES A PAINTED BANNER AS BLANK, AND INVENTS A CUT-OFF TOP

**The instrument:** any quick parser written to answer "how much of the grid did
this TUI paint?" — including a careful one.

**What it says:** that the top rows of a CLI's banner are missing, consistently,
at every window size. Which reads as a real, reproducible rendering bug.

**Why it is wrong, twice over:**

1. **Gradient banners and block art are routinely drawn as SPACES carrying a
   BACKGROUND COLOUR.** A cell can be fully painted and hold no text at all, so a
   "is the text blank" test scores the painted half of a header as empty. This is
   not an edge case — it is how most colour banners are drawn.
2. **Alt-screen (`\x1b[?1049h`) and scroll regions (DECSTBM) are easy to skip**
   and change what "row 1" even means. Miss them and the model scrolls where the
   real terminal does not.

⇒ **Use `scripts/cli-viewport-probe`**, which feeds a real PTY to the **same
`vt100` crate `yggterm-server::terminal` parses with** — so a coverage number is
measured by the daemon's own eyes rather than by a second implementation that can
disagree with it. It reports `bg_only_cells` precisely so the background-painted
case is visible instead of silent.

⭐ **The general form, which is the reusable part:** when you write an instrument
to check a renderer, you have written a SECOND renderer, and now you have two
things that can be wrong. Prefer the one the product already trusts. Caught
2026-08-20 — the hand-rolled probe "confirmed" a qwen cut-off top that the
daemon's own parser then also confirmed, but only after the blank test was
corrected; had the banner been drawn the common way, the first answer would have
been a fabricated bug report against an innocent CLI.

### ⛔⛔ A REMOTE BRANCH THAT SKIPS THE GUARD BLOCK INVERTS THE SAFETY GRADIENT

*Measured 2026-08-20, `server app terminal adopt`.* The verb returned
success-shaped — `started live::…`, `purpose: "adopt outer PTY … via reptyr -T"`
— and produced a bare remote shell with **no adoption at all**.

Every check (pid exists · `reptyr` present · the non-dumpable refusal · PTY
leader) sat inside `if !is_remote_adopt`. The reasoning above it was **right**:
this host's `/proc` cannot answer for another host's pid. The remedy was wrong —
it SKIPPED the guard instead of asking the right machine.

⚠ **The path with LESS local knowledge got LESS checking**, which is backwards,
and it reads as caution — the local check genuinely *would* have been wrong —
which is exactly why it survives review. ⇒ **Whenever a local verb grows an
over-ssh twin, ask where the guard belongs: on the machine that owns the thing,
never on the machine that happens to be running the verb.**

⭐ **The tell that it ran on the right machine** is that the refusal names the
REMOTE artefact: `adopt_refused: pid … exe /…/@anthropic-ai/claude-code/bin/claude.exe
is non-dumpable …`. A guard that cannot name what it inspected did not inspect it.
And an unreachable machine must yield NO refusal rather than a false one — being
unable to check is not evidence of a fault.

⛔ **Claude Code and Muse can never be `reptyr`-adopted**, at any `ptrace_scope`:
node binaries with `PR_SET_DUMPABLE 0` + seccomp. So **adoption is not a recovery
for a frozen agent row.** When an agent's PTY master dies with its daemon the
terminal is gone for good; the CONVERSATION is not, because it is in the CLI's
transcript. Recovery is a relay **handover** while the agent still works, or —
once it is idle — killing the orphan and resuming that session id in a fresh row.
⭐ The double-resume guard is CORRECT and unblocks by itself once the orphan is
gone: the guard was never the bug, the missing recovery path was.

## 2. Profiling recipes that work

No `perf` on a typical desktop host (`perf_event_paranoid=3`), but these do:

### ⛔⛔⛔ THE UNIT LAW — when every instrument agrees and the answer is still missing, suspect the UNIT

**Four lanes spent a day refining precision inside units that could not express
the answer.** Every instrument was honest, every control passed, and the quantity
was not in frame. That is not a precision problem and no amount of care inside
the unit finds it.

**Three faces, all from one day, all invisible to every instrument because every
instrument shared the unit:**

| face | the unit | what it cost |
|---|---|---|
| **STORAGE** | an identifier held at two lengths (8 vs 36) | a `succeed` silently skipped rows and reported a clean count; one seat escalated into a void for hours while the board rendered it healthy |
| **ACCOUNT** | one daemon vs the **POPULATION** | every arm measured a daemon and none measured how MANY — `population ≈ N_reachable × ~0.2 cores`. Per daemon the old and current builds are indistinguishable; the pile is expensive because it is **numerous**, and no amount of precision on one daemon expresses that |
| **SCOPE** | a sandbox daemon vs a **loaded** one | a sandbox daemon has **no sessions and no peers**, so it reads ~0 and can never exhibit the floor. `0.00017 cores` was reproducible, honest, and blind by construction |
| **REGIME** | saturated vs idle | flooding a pty with `yes(1)` measured the reader path's **ceiling**; carried to a fleet whose sessions are idle it inverted the whole ledger |

⭐ **A fourth face is the observer's own:** a unit that silently *includes the
measurer* — a counter charging your load to the subject, a probe reading back its
own write, an "after" window taken in the wake of your own generator.

⇒ **The tell is agreement without resolution.** When independent instruments
converge and the number still will not appear, stop improving the instruments and
ask **what quantity they all decline to express.** Change the unit — count the
population, not the instance; load the subject before pricing it; measure the
regime it actually occupies, not the one that saturates it; compare identifiers by
prefix, not equality.

⛔⛔ **AND A RETRACTION THIS TABLE CARRIES AGAINST ITSELF.** The ACCOUNT row above
originally read *process vs process SUBTREE*, on the reasoning that a sandbox
daemon spawns no children and so has *no subtree to carry the floor*. **That was
withdrawn on 2026-08-14** by a read-only two-sample `/proc` walk over 16 daemons
and 172 descendants: **eleven of sixteen carry 0.13–0.25 cores in the daemon
process itself with essentially empty subtrees.** The subtree only balloons on
daemons hosting working agents, where it is a compile and a live turn — somebody's
work, not daemon overhead. The two framings never disagreed; the distinction was
invented. ⭐ **The rule its author drew from it is the one to keep: a mechanism is
a CANDIDATE until a measurement it could have failed has been run.** Two earlier
mechanisms from the same lane died as cheap hypotheses; this one reached a
document and two lanes' hands first, and that is the only difference.
⚠ Corrected here by the orchestrator because the lane that filed it had already
stood down and the lane that retracted it does not own this file — **an orphaned
withdrawn claim outlives both of them.**

⚠ **And a harness that cannot express the term should say so in its own
documentation**, or it goes on looking general-purpose while being blind by
construction.

### ⛔⛔ an EXTERNAL estimator is unusable when the baseline IS what you subtract

**Read this before pointing any `/proc`-based profiler at a busy process.** It
cost the resource campaign four measurements, and they failed as one class rather
than as four accidents.

The recipes below all work by **differencing a process-level counter**: sample
`/proc/<pid>/stat` twice, attribute the delta to whatever you were doing in
between. That is sound **only while nothing else in the process is running**. The
moment the process has its own concurrent work — background chores, per-session
reader threads, another client's traffic — the counter charges that work to your
window too, and you divide it by *your* denominator.

**The arithmetic that kills it.** To price ~2 ms of per-request work at 100
requests you are looking for ~0.2 core-seconds. A daemon carrying live sessions
drifts by ~0.7 cores over the same 10 s — **~35x larger than the signal**. The
quantity is below the noise floor of the instrument.

⭐ **THE TELL IS A CONTROL THAT REFUSES, AND IT IS WORTH BUILDING FOR:** bracket
the measured window with a baseline **before and after**. When those two
disagree — one live daemon read 0.4240 then 1.1437 cores — the arm is void, and
you learn it instead of publishing a number. One bracket produced an impossible
**negative** per-request cost, which is the cheapest kind of refutation to read.

**Four failures, one cause** (all in `idle-cost-model.md`):

| attempt | how it failed |
|---|---|
| two-point dose slope | slope between two noisy points; did not replicate (8.5 → 33.3 ms, same subject) |
| version/RSS sub-group split | n=2 per group off that slope; died on widening 4 → 8 |
| baseline-subtracted live arm | brackets disagreed 2.7x; negative result; VOID |
| a per-connection cost "outside the handler" | **1.7 ms that does not exist** — background charged to whatever connection happened to be open. Measured directly from inside, the whole floor is **150–230 µs** |

⇒ **What to do instead.** Measure the *operation*, not the *process*: an
in-process span using `CLOCK_THREAD_CPUTIME_ID` around the work itself is immune
to what the rest of the program is doing, and it is the only thing that can price
a per-request quantity on a busy daemon. If you cannot get inside the process,
**say the quantity is unmeasurable rather than publishing the residual.**

⛔ **A RETRACTION APPENDED TO A CLAIM ROTS INDEPENDENTLY OF IT.** Correcting by
appending a note leaves the oldest sentence first, so a reader meets the withdrawn
version before the correction — and when the correction is itself superseded you
get three layers, of which the middle one reads as current. One block in the cost
model accreted `~50 µs` → `38 ms` → `~25 ms` → `150–230 µs` this way.
⇒ **State the current position FIRST and keep the trail beneath it.**
⭐ **And sweep by TERM, not by document:** a docs retraction sweeps docs, while
the dangerous copy is the one in a **different file type** — a source comment
reads as authoritative precisely because it sits next to the code, and nobody
greps comments when retracting a doc.

⛔ **AND THE OBSERVER IS PART OF THE BASELINE.** After driving load at a subject,
its counters stay elevated. A comparison whose "after" window follows your own
arm measures **you**. This produced a published claim that a term swung 25x
between adjacent windows; re-measured quiet, the same daemons moved 1.03–1.12x.
⇒ **Let the subject settle, and never take the second half of a comparison in the
wake of your own generator.**

⚠ **Two per-thread traps in the same family**, since the first recipe below is
exactly where they bite:
- **`/proc/<tid>/stat` utime/stime are 10 ms CLK_TCK units, truncated
  INDEPENDENTLY.** Below ~10 ms the smaller component is annihilated and a
  user/kernel *share* is driven to 100% for the larger. Against
  `getrusage(RUSAGE_THREAD)` on a known 4 ms/20 ms mix: true 83.3%, ticks say
  **100.0%**. Short-lived threads read **zero** — 622 consecutive handler threads
  read 0 ticks while each spent ~1.4 ms. ⇒ **never read a per-thread share from
  tick fields**; use `getrusage(RUSAGE_THREAD)`.
- **A control must live in the SUBJECT'S regime.** A positive control burning
  250 ms per thread says nothing about subjects burning 1.4 ms. Sweep it: the
  process-level *sum* survives (1.000 → 1.058 from 250 ms down to 1.5 ms), the
  per-thread split does not. **A sum of floors is not the floor of a sum.**


- **Per-thread CPU** — read `utime+stime` from `/proc/<pid>/task/*/stat` twice N
  seconds apart. Thread names tell you the subsystem immediately. Include the
  daemon and the WebKit child, not just the GUI.
- **Poor-man's profiler** — `eu-stack -p <pid>` in a loop (~12 samples). One
  busy sample among idle `ppoll`s is still a real attribution.
- **Syscall shape** — `strace -c -p <pid>` for 5s. A hot loop shows up instantly
  as a `clock_gettime` count; repeated `openat`/`mkdir`/`statx` means something
  is re-opening a store on a hot path.
- **Presented frames** — count `memfd_create` on the GUI process. Each new
  buffer is a presented frame. This is the honest "is the app repainting?"
  number, and it is invisible to every DOM-side probe.
- **In-page timing** — wrap the function under suspicion from `app dom-eval`,
  accumulate into a `window.__probe` object, read it back in a later call.
  Instrument *all* candidates, not the one you suspect; the answer is often that
  your suspect costs nothing.

- ⛔ **A syscall count is meaningless until you price the syscall on THAT host.**
  `clock_gettime` is normally a ~27 ns vDSO read and free to ignore. On a host
  whose clocksource has fallen back to `hpet` it is a real syscall reading a
  14.3 MHz MMIO counter at **1222.5 ns — 45.8×** — and the same code costs 1.3%
  of a core on one machine and 58.8% on another. **Check
  `/sys/devices/system/clocksource/clocksource0/current_clocksource` before
  interpreting any profile**, and price the call with a 20-line
  `clock_gettime` loop rather than assuming.
- ⭐ **`utime` vs `stime` splits the search in half before you profile.** Kernel-
  dominant means syscalls (find which with `strace -c`); user-dominant means
  compute (find where with `eu-stack`). Reading the split costs one
  `/proc/<pid>/stat` sample and rules out half the hypotheses.
- ⭐ **A ratio survives the instrument; a rate does not.** `strace` slows the
  target ~13×, so its absolute rates are fiction — but *`clock_gettime` per
  `ppoll`* is unaffected and is what proved the main loop was spinning rather
  than blocking.
- ⭐ **Measure a fresh process against an aged one to learn the defect's KIND.**
  Identical build, identical host: if the cost falls, it is accumulation; if it
  holds, it is a constant loop. This is the one experiment that distinguishes
  "leak" from "hot loop", and a restart destroys the aged sample — so take the
  aged measurement FIRST.

**Hold the workload fixed.** The single most common measurement error here is
comparing two conditions under different load — a CPU/thermal A/B is evidence
only if the same session is doing the same thing in both windows. When the agent
itself drives a live session, run the whole A/B inside ONE script so the agent
emits nothing during the sampling windows.

## 3. Rendering cost model (software-GL hosts)

⛔ **"A desktop host may deliberately run software GL" is no longer standing
guidance, and on the live host it was never true.** The GUI used to hard-code
`LIBGL_ALWAYS_SOFTWARE=1` behind an opt-out nobody set, on a premise (one EACCES
on `card0` under a GBM probe) that was measured false on the very host it named.
The binary now PROBES (`yggterm_core::gl_probe`) and publishes its answer as
`YGGTERM_WEBKIT_GL_POLICY`; read that before assuming anything about a host.

The cost model below still holds **wherever the probe genuinely lands on
software** (a headless server host, a VM with no render node, a host with
`YGGTERM_FORCE_SOFTWARE_GL=1`), and the frame-count findings stand on their own
merits on any host. Consequences that drive real bugs:

- Every repaint costs a full-window CPU blit (`cairo_paint` / `pixman_blt`) on
  the GUI main thread. **Cost tracks the number of presented frames, not the
  number of pixels that changed.**
- Therefore: N independently-phased animations cost N times one animation.
  Paint containment (`contain:paint`, `will-change`) and removing
  `backdrop-filter` do **not** help — measured, twice. Cut frames instead.
- The app owns exactly ONE blink animation, on `:root`, published as an
  inherited custom property (`--yggterm-status-dot-blink`). Any new indicator
  reads that phase; none declares its own animation. See DESIGN.md, "One clock
  for every blink."
- A CSS animation's phase is anchored to when its element was created. You
  cannot phase-lock per-element animations with a computed `animation-delay`:
  changing the delay does not restart the animation, so re-rendered rows drift.
- **Is the GPU actually rasterizing?** `drm-engine-gfx` in
  `/proc/<webproc>/fdinfo/*`, nonzero and RISING across two reads. The repo owns
  that gauge now — `render_top` prints a `gpu_ms` column per role, and a `-`
  there means the counter was unreadable, which is not the same as a zero.

## 4. Deploy protocol

### 4.0 First decide which KIND of deploy this is — it changes everything

| Change lives in | Version | What restarts | Cost to the user |
|---|---|---|---|
| CLI path only (arg parsing, screenshot post-processing, manifest building) | any | nothing | **zero** — run the new binary as a client from `/tmp` |
| GUI only (`shell.rs`) | **KEEP THE CURRENT VERSION** | GUI only | small — one blank re-attach |
| Daemon (`daemon.rs`, `lib.rs`, protocol) | bump | GUI **and** daemon together | real — re-attach symptom class on every live session |

⛔ **A GUI-only patch must NOT bump the version.** A newer GUI classifies the
older daemon as stale and spawns a successor — which is exactly the daemon
handoff (and its frame corruption) you were avoiding. Same version = the daemon
is untouched, its PID does not change, and PTYs never move.

⛔ **Never leave the GUI and daemon on different versions.** An *older GUI*
fights a *newer daemon*: it classifies it as stale and tries to displace it
forever (measured: ~24,000 events at ~2,500/min for 27 minutes, user saw frozen
frames and "daemon connection lost"). The `version_mismatch` warning only fires
when the GUI is newer, which is why the dangerous direction looks safe.

### 4.1 The version-stamp landmine — VERIFY, never trust `--version`

`SERVER_PROTOCOL_VERSION` is `env!("CARGO_PKG_VERSION")` of the **yggterm-server**
crate. A version-only bump in `Cargo.toml` does **not** always force that crate to
recompile, so a release build can ship a binary whose `--version` reads 2.12.2
(the `yggterm` package) while its baked protocol constant is still 2.11.0. The
deployed GUI then reads its own protocol as older than the live daemon, silently
refuses to swap, and you get a mixed-version wedge: a retry spin, a session that
cannot reconnect, broken typing, and a ~50 Hz garbled blink. **This cost hours.**

```bash
cargo clean -p yggterm-server          # before ANY release build after a bump
cargo build --release --bin yggterm --bin yggterm-headless
```

**Then prove the stamp** — `--version` cannot prove it, because it reads a
different crate's version. The socket name is derived from the protocol constant
(`format!("server-{}.sock", SERVER_PROTOCOL_VERSION.replace('.', "-"))`), so a
throwaway daemon in an isolated home spells it out:

```bash
SB=$(mktemp -d)
YGGTERM_HOME="$SB" ./target/release/yggterm-headless server daemon > "$SB/d.log" 2>&1 &
sleep 3; grep -o 'server-[0-9-]*\.sock' "$SB/d.log"   # -> server-2-12-3.sock
kill %1
```

⚠ The socket does **not** live under `YGGTERM_HOME` when that path is long — it
falls back to `/run/user/<uid>/yggterm/h-<hash>/`. Read the path out of the
daemon's own log line, don't `find` the home dir. Clean up that runtime dir too.

### 4.2 Deploy to all FOUR paths

The live host runs the daemon from `~/.local/bin/`, but remote wrappers invoke
`~/.yggterm/bin/`. Miss one and you get a split-version fleet:

```
~/.local/bin/yggterm    ~/.local/bin/yggterm-headless
~/.yggterm/bin/yggterm  ~/.yggterm/bin/yggterm-headless
```

`cp -a` each to `*.rollback` first, then **`mv` the new binary in — never `cp`**
(cp over a running binary is `ETXTBSY`).

⛔ **And never "fix" that `ETXTBSY` by moving the OLD binary aside first.** The
improvisation it invites — `mv yggterm-headless yggterm-headless.old.$$ ; cp new
yggterm-headless ; rm -f *.old.*` — succeeds, prints the new `--version`, and
looks identical to the recipe above. What it actually does is drag every running
daemon's `/proc/self/exe` onto the backup's name and then delete it, which used
to leave the daemon unable to find its successor: it skipped the PTY handoff and
cold-exited, killing **55 live terminals** on `dev` on 2026-08-09. The daemon now
looks in both places, so this no longer destroys sessions — but `mv` in place is
still the one form where nothing has to be recovered from.

⚠ **Cap the rollbacks — they are ~48 MB each and pile up fast.** A single
`*.rollback` per dir would be overwritten each swap, but a dated/named rollback
(`.rollback-<fix>`) accumulates: a day of swaps left 26 of them (~1.2 GB) on a
host already thrashing swap. After staging the new binary, prune to the two
newest per dir: `ls -t "$dir"/yggterm.rollback* | tail -n +3 | xargs -r rm -f`.
The junk that makes the fan angry is often our own deploy residue.

### 4.3 The recipe that works (cross-version, no fight)

```bash
# 0. CAPTURE THE GROUND TRUTH FIRST — this list is what makes recovery exact
ssh $H '~/.local/bin/yggterm-headless server app rows' > rows-before.json
# 1. scp -> rollbacks -> mv into all four paths (above)
# 2. GUI and daemon TOGETHER, in one window:
ssh $H 'kill -TERM <gui-pid>'                       # wait for actual exit
ssh $H 'yggterm-headless server app launch --wait-visible'
```

The new GUI spawns the new-version daemon, which adopts from the old daemon that
is *still alive holding PTY fds* — doing both together skips the version-fight
window entirely. The old daemon staying alive is correct and deliberate:
`hot_restart_should_defer_for_session_survival` returns true while it owns PTY
fds, so sessions are parented by the **daemon**, never the GUI.

⚠ The hot-restart is **blocked while any owned session is "working"** — and the
agent's own session counts, so `hot_restart_block_reason` will name *you*. The
tool that forces it safely is `yggterm-headless server update-daemons --force`
(progressive, preserves PTYs, ungated handoff) — run it from a **correctly
stamped** binary or it will refuse for the wrong reason.

### 4.4 After deploying — the checks, in order

1. **Row count before vs after.** Expect a drop; see §4.5. Nothing is lost, but
   *invisible is lost from where the user sits*, and it was the user who noticed
   last time.
2. `server status` → `server_version`, `server_pid`, `role_enforcement`.
3. **A faithful pixel.** `server app screenshot` and then **Read the PNG**. Check
   `capture_faithful: true`; a `linux_webkit_snapshot` fallback frame is
   canvas-blind and lies about the terminal.
4. **Exercise the fix and quote the evidence.** If you cannot, say so plainly —
   "code is on disk, the running daemon predates the fix" — never "shipped".

**Deploying re-introduces transient symptoms.** A daemon swap re-resumes agent
CLIs on fresh PTYs, and that window looks exactly like the squish/broken-bottom
bug class. Never measure a symptom the deploy itself causes, and never declare a
post-deploy surface healthy without looking at it.

### 4.5 Expect to lose Live Sessions rows — and know the recovery

Every daemon swap drops the rows that no daemon actively owns (root cause and the
designed fix: `docs/pending-bugs.md`, "B4 ROOT CAUSE"). Measured on 2.12.2 →
2.12.3: **25 rows → 12**. Exactly the predecessor's owned keys survive.

```bash
# after the deploy, diff against the ground truth captured in §4.3
comm -23 paths-before.txt paths-after.txt > missing.txt
while read -r p; do
  ssh -n $H "~/.local/bin/yggterm server connect '$p'"
done < missing.txt
```

Three traps in those four lines, all of which have bitten:

- ⚠ **`connect` is on the `yggterm` binary, NOT `yggterm-headless`** — headless
  answers "unsupported server command".
- ⚠ **`ssh -n` or the loop silently reconnects only the FIRST row.** Plain `ssh`
  reads stdin, so it swallows the rest of `missing.txt`. The loop *looks* like it
  worked because the one row it did process succeeded.
- ✅ **`yggterm server reorder` now moves dormant rows too, and answers
  honestly** (fixed in-tree 2026-07-26, NOT yet live-verified on guihost).
  `replace_live_session_order` used to filter on
  `managed_session_is_live_runtime_session`, so rows without a live runtime were
  ignored while the call still reported `"requested": 19` and echoed your list
  back — reading exactly like success. It now admits any row in
  `live_session_order` and returns `applied` / `skipped` lists that the CLI
  prints verbatim; a non-empty `skipped` also makes the command exit non-zero.
  A path that is not a row is refused rather than added, so the verb can never
  create a row. **Until a real daemon on guihost runs a build with this fix, keep
  verifying order by re-reading `server app rows`** — an older daemon cannot
  report `applied`, and the CLI then prints `applied_unreported_by_daemon`.

Rows reappear 5–10 s later. Re-check the count **again once the predecessor has
actually exited** — the drop can be delayed: the predecessor holds dormant rows
until its own disk-binary poll retires it, which can be ~20 minutes later.

⭐ **AND DO NOT DIAGNOSE A MISSING ROW FROM ITS ABSENCE — ASK WHAT HAPPENED TO IT**
(added 2026-08-14):

```bash
yggterm-headless server rows departed --limit 50   # read-only, safe on a live fleet
```

Every row that leaves the live set now records which row, its title, when, and
**why**: `explicit-close` (somebody named it and closed it — a user close, an
agent `session remove`, or the ephemeral reaper) or `gui-close-disposable` (the
GUI window closed and the row was not keep-alive, so it went for what it WAS
rather than because anyone asked). ⚠ **The three-way answer is the point:**

| what you see | what it means |
|---|---|
| an entry saying `explicit-close` | somebody closed it. Not a bug. |
| an entry saying `gui-close-disposable` | it was disposable and a GUI close took it. Working as designed — and if the user made that row on purpose, `docs/spec-app-row-survival.md` says it should not have been disposable. |
| an entry saying `persist-dropped` | ⛔ **it was never closed — it was left OUT of the state file** at a persist, so the SUCCESSOR daemon never learned it existed. The `detail` field names which of the three gates took it. **This is what took the owner's app row group on 2026-08-13**, and it is the reason a lost row could leave `removed-rows.json` empty AND still be listed by the daemon that dropped it. |
| **no entry at all** | ⛔ **it did not leave through any path that knows it left.** That is its own defect, and it is the state the whole ledger exists to make visible — an absence used to be the ONLY evidence, and an absence reads identically to "never existed". |

⚠ It is machine-wide (`~/.yggterm/removed-rows.json`, shared by every daemon on
the host), so it answers for rows a peer daemon owned too — which is what makes
it usable when two daemons' state files disagree about whether a row exists.

## 5. Destructive operations — know before you type

- Any `reconcile` / daemon-screen replay is a full reset + re-seed to the current
  screen. On a healthy session it collapses scrollback and can blank the
  viewport. Run it only on a surface already confirmed broken.
- Never type into a live agent prompt to "test" it.
- ⛔⛔ **BEFORE ANY IRREVERSIBLE ACT, TAKE ONE MORE MEASUREMENT OF A DIFFERENT
  KIND THAN THE ONE THAT CONVINCED YOU.** Not a second reading — a different
  *kind*. If a rate convinced you, check a state; if a state convinced you, check
  a rate. ⭐ **Four vantages have each caught someone here in one day: a different
  INSTRUMENT, a different HOST, a different OBSERVER (a self-control cannot
  fail), and — the one with a person on the other end — a different MOMENT.**
  A single-instant reading is what makes a live human look like a stalled row.
- ⚖ **WHEN TWO SAFE-LOOKING OPTIONS REMAIN, PREFER THE DEFECT THAT SHOUTS TO THE
  ONE THAT IS MUTE.** A supervision row was found armed while a person sometimes
  types into it. Adding it to `never-arm.tsv` looks like the cautious choice and
  is the silent one: **nothing would ever wake that seat again**, and the rows
  escalating to it would go on escalating into something that no longer stirs,
  with no alarm anywhere. Leaving it armed keeps a hazard that **announces itself
  on every listing**. ⇒ Caution is not the same as safety; ask which failure has
  a witness. ⇒ **The general failure this project keeps meeting is a POINT SAMPLE
  STANDING IN FOR A RATE** (and its mirror, a rate read off the wrong subject).
  Two sessions hit it within an hour, in opposite directions, on the same
  question: one read a parent's CPU rate when the work was in a child, the other
  read a child COUNT when the question was whether work was still arriving. Each
  corrected the other and then committed the same shape one level away. **The
  survivor in both cases was not better judgement, it was one extra probe of a
  different kind, taken before acting.**
- ⛔ **A GUI RELAUNCH IS NOT A DAEMON SWAP, AND ONLY ONE OF THEM COSTS ANYTHING.**
  Text typed into a ROW is not in the GUI at all — it lives in a PTY the daemon
  owns. Measured: type an unsent line into a row, kill the GUI process outright,
  and the daemon still holds it with no GUI running; relaunch against the same
  home and daemon and the text and the row are both back. **What re-resumes
  sessions is a DAEMON SWAP**, and a swap taken alongside a relaunch is what
  gets blamed on the relaunch. ⚠ The exception is a **yggterm-side** input — a
  search box, an SSH field, a document buffer — which IS in the page and does not
  survive. ⇒ Five owner-gated items once queued for days behind the untested
  claim that a relaunch destroys a draft; it does not.
  ⭐ Ask which is pending before either: **`server rows drafts`** (§1) answers
  read-only, and its `blind` verdict is not `clear`.
- ⛔ **`git reflog expire --expire=now --all` EMPTIES `git stash list`.** The stash
  is reflog-backed, so a cleanup that only meant to drop unreachable history takes
  the stash entries with it — and it is not announced. Measured 2026-08-14: a
  guarded reap (refusing on a dirty tree, on unpushed commits, on a
  non-fast-forward) ran on a repo whose stash held another session's WIP; `stash
  list` read empty afterwards while the commit itself survived, unlisted, one
  `gc --prune=now` from gone. ⇒ **Before any `reflog expire --all`, record
  `git rev-parse refs/stash` and `git stash list`**, and treat a stash as work to
  be preserved rather than as history to be pruned. Recovery, while the object
  lives: `git stash store -m '<why>' <sha>`.
- Restore the user's active session after any probe that had to switch away.
- ⛔ **`yggterm <unknown-verb>` LAUNCHES THE APP. It does not print help.** Running
  `~/.yggterm/bin/yggterm update --help` to read a usage string instead started a
  second client instance, whose singleton path took the *running* GUI down with
  it and left a `SIGABRT` coredump — 36.9 h of accumulated state gone mid-measurement
  (2026-08-13). The GUI binary parses only what it recognises and otherwise falls
  through to "be the app". ⇒ **Every control verb goes to `yggterm-headless`**,
  which is the client made for agents; reach for the GUI binary only to launch a
  GUI on purpose. This is the same family as the standing rule against running
  archived/versioned GUI binaries "just to see the version".

## 7. Gotchas that cost this project real time (2026-07-26)

Each of these produced a CONFIDENT WRONG ANSWER, which is worse than an error.
They are ordered by how much time they burned.

### 7.0 Ask where a USER loses their work — a suite of green tests will not

**A test asks whether the good path works, and data loss is never on the good
path.** Two data-loss defects in a journal's capture surface survived a 49-test
suite that passed, because every test exercised a successful capture:

- **The write was read-modify-write** — read today's entry, append, write the
  whole file back. That puts *everything already captured today* inside the blast
  radius of one interrupted write, and loses a note outright when two captures
  land at once: each writer writes back what it read, so the slower one erases the
  faster one's words **and reports success.** Fixed to an append that syncs before
  reporting success, so the worst case is the tail it was adding — the item whose
  loss the user is present to notice — and never a byte that was already safe.
- **The surface discarded the input on failure.** The box cleared and looked
  exactly as it does on success while the reason went to stderr, in a terminal
  nobody was looking at — so from where the user sits the words simply vanished
  and nothing said anything. A failed capture now re-declares the box **holding
  their text**, with the reason above it.

⇒ **The question that finds these is not "does it work" but "where does a writer
lose an entry".** It generalises past journals: this project's whole product is
session persistence, so *"where does a user lose a session"* is the same question
pointed at yggterm, and neither failure above would appear in any suite that only
asserts success.

⭐ **And the concurrency fix was verified the only way that means anything:**
reverted to read-modify-write, watched the test fail with *the first thought was
erased*, restored, watched it pass. **A green test that cannot be made red proves
nothing** — the same law as the dead-instrument row in §1, applied to your own
test rather than to someone else's code.

### 7.1 A lock that mutates the new helper is not a lock

**Five could-only-pass locks shipped in two rounds.** Every one tested a
freshly-added helper with hand-built arguments instead of the wiring that was
the actual defect:

- the "20 sequential verbs deliver" test passed a **literal zero** seat-input
  count, synthesizing away the exact bug it existed for;
- a GL lock asserted `f(x) == Clear iff x`, restating a function that *is*
  `if x { Clear } else { Apply }`;
- a memo-key lock built struct literals and never called the production key
  builder — a tautology over `derive(PartialEq)`;
- a source scan anchored on a string occurring **zero times** in the file, so
  `unwrap_or(len)` silently widened its "scope" to 75% of the file including
  the test module;
- the web-surface reclaim locks: reverting **all four production call sites**
  left the suite green at the identical pass count the report cited as proof.

**The rule: mutate the PRODUCTION CALL SITE, run it, see red, restore, and quote
the red output.** If the loop is not directly testable, extract the decision
into a pure function the loop CALLS, so reverting the loop's wiring changes the
pure function's observed input. A test that calls the helper directly is
structurally incapable of observing what the loop passes it.

**All five are now closed** (`41b7b1b` GL, `4a2c836` memo key, ROUND-15 scan
coverage floor, the batch locks in `web_do_verb_tests`, and
`shell::web_surface_reclaim_locks` for the reclaim family). The reclaim one is
the worked example of the rule, because it is the case where the wiring lived in
an `async` loop holding a live `DesktopContext` — nothing a test can call:

1. **The loop keeps no policy and no bookkeeping.** It reads `/proc`, reads the
   configured hold, names its backgrounded surfaces, and calls
   `web_surface_reclaim_background_pass`. Every decision — which pressure
   reading, which surface's audio, whether the reap is recorded, whether a soft
   stash demotes or detaches — moved into that pass.
2. **The world is a trait.** `WebSurfaceBackgroundHost` carries destroy / stash /
   demote / throttle / clear-loading / trace, so a test asserts on what the pass
   DID, not on what a helper returned. `LiveWebSurfaceBackgroundHost` is the only
   un-mockable code left and is four one-line methods.
3. **The residual seam is locked structurally.** The loop's own argument list
   still cannot be reached, so
   `the_reclaim_pass_call_site_is_wired_to_the_live_machine` scans it — over
   `yggterm_core::agent_cli::product_lines`, the workspace's ONE test-module skip
   rule, so the lock's own text cannot satisfy the needles it looks for.

**Twenty-one mutations were run against it, one per production call site, each
proven red and restored.** If you touch this family, re-run them: the script
shape is in the round-25 report, and a lock nobody re-proves decays into the
thing this section is about.

**Two traps in the structural half itself**, both found the hard way on the
leased-surface-row lane:

- **A source scan judges the FILE, not the binary under test.** It reads
  `shell.rs` from `CARGO_MANIFEST_DIR` at *runtime*, so a mutation reddens it
  with no compile at all, and a rewording that behaves identically reddens it
  too. That is fine for its actual job — "the call site moved, come look" — and
  worthless as evidence about behaviour. Keep such a lock to ONE needle, say in
  its doc comment that it is not behavioural, and make every claim about what
  the code DOES with a test that runs the code. A five-needle version of one of
  these was read as a behavioural lock by its own report; the arm it "locked"
  could be neutered by shadowing a local, with all five needles still present.
- **`product_lines` recognizes the literal `#[cfg(test)]` and nothing else.**
  Write `#[cfg(all(test, unix))]` on a test module and the scan stops skipping
  it, so the module's own assertions satisfy the needles it is looking for. Use
  `#[cfg(test)] mod foo { #![cfg(unix)] … }` instead. The self-check that
  catches this (`the scan is reading this test module`) belongs in every lock
  that greps.

### 7.2 Never run several workflow lanes on `main` in one checkout

Three fix lanes were pointed at `main` in the same working tree. Two edited
`shell.rs` simultaneously; 1,222 uncommitted insertions interleaved, the tree
stopped compiling mid-edit, and stopping them left half-finished refactors
(renamed functions with un-updated test call sites, a test written for a
function that was never written). **Use `isolation: 'worktree'` for every
parallel lane, always** — even when the file sets look disjoint, because agents
run the whole suite and will "fix" each other's in-flight edits.

### 7.3 A measurement must be able to refuse

Three separate A/B arms each produced a confident wrong number, all from the
same failure — the load never reached the renderer and nothing said so:
comparing an evening of real use against an overnight window with **23x less
terminal activity**; piping the paint load to `/dev/null`; and restarting the
GUI between arms so it came back displaying a **different session**. That last
arm read 0.27 cores and looked like a 5x win.

`scripts/gl_ab_measure.sh` now refuses to print a number unless the window is
focused, the session under test is the one actually displayed, the active
session did not change mid-arm, and the arm cost more than a floor.

**Corollary — the confound that invalidated both a "win" and a "regression":**
render rows carry `window_focused`, and unfocused→focused alone moves
`render/gui` p50 by **7x**. Before comparing any two render windows, bucket them
by `window_focused` and by `daemon_request/terminal_read` rate. `gpu_ms` zero in
523 of 532 ticks means nothing painted, not that the GPU got faster.

### 7.4 Instruments that lie about their own subject

- **⛔ `pgrep -f "<pattern>"` MATCHES YOUR OWN WRAPPER.** A shell running
  `until ! pgrep -f "cargo build --release"; do sleep 10; done` has that exact
  string in its OWN `/proc/<pid>/cmdline`, so the check matches itself and the
  loop never exits — reporting "still building" forever after the build
  finished. Cost an hour of a session on 2026-08-06, waiting on a build that
  had completed 90 minutes earlier. Use `pgrep -x cargo` (exact NAME, not the
  command line), check the artifact's mtime, or exclude yourself with
  `pgrep -f pat | grep -v $$`. ⚠ Same shape as a structural test whose own
  assertion string satisfies the search it performs — **if a probe's text can
  appear in the thing it probes, it is measuring itself.**
- **⛔ `/proc/<pid>/fd` CANNOT link a `claude` process to its transcript.**
  Claude Code APPENDS AND CLOSES, so the JSONL is not held open and the fd
  table shows nothing — an empty result is the NORMAL case, not evidence that
  the process has no transcript. This is the opposite of Codex, whose open
  transcript fd is exactly how `AGENTS.md` says to recover its session identity,
  which is why the technique gets retried on CC and silently returns nothing.
  Link by EVIDENCE instead: the session id in argv (`--session-id <uuid>`),
  else a known path matched inside the transcript body. Measured 2026-08-06.
- **A row leaving the table is NOT its runtime dying.** `session remove` answers
  with `verified` and, on failure, `verified_refusal` (e.g.
  `remote_runtime_survived`). The ROW LIST updates first, so reading it instead
  of `verified` reports a clean reap over a live orphan — it has already
  produced one. ⚠ `live_processes: []` has been observed in the SAME reply as
  `remote_runtime_survived`; when they disagree, believe the refusal.
- **`set_var`/`remove_var` do NOT change `/proc/<pid>/environ`.** glibc
  reallocates the environ array on the heap; the kernel keeps exposing the
  exec-time stack page. Anything the process set AFTER exec is invisible there,
  and after a hot restart the child's `/proc` shows the PREDECESSOR's values.
  Publish a runtime decision from the process's OWN view.
- **`drm-engine-*` is per DRM CLIENT, not per fd.** Duplicated fds share one
  `struct file` and each repeats the same cumulative counter, so a per-fd sum
  over-counts by the fd count — measured **5.00x on Xorg, 4.00x on a
  compositor**. Dedup on `drm-client-id`.
- **Zero GPU engine time in a window means IDLE, not software.** Read the
  DRM-fd count first: llvmpipe never opens a DRM node, so that is the
  structural answer; engine time is a workload answer.
- **The DAEMON is a lying witness for the GUI on anything OSC 7717.** One
  declare is parsed TWICE: the daemon reads the raw PTY bytes in Rust and
  stores the payload verbatim, while the GUI parses it in xterm.js and
  **hand-builds** the event object field by field. A field present in
  `server terminal app-declares` therefore proves NOTHING about what the GUI
  holds. `control_token` was added to the Rust wire type and missed in the JS
  for three days; ychrome's panes 403'd while every daemon-side probe said the
  token was fine, and four wrong attributions (stale token, pre-gate client,
  old daemon binary, lossy replay) were burned on that asymmetry. Locked by
  `the_js_forwarder_copies_every_sidebar_declare_field`. **Symptom shape: the
  rail draws correctly but its GUI-only routes refuse.** The one-step
  falsification is to replay the daemon's own token by hand — a 200 means the
  app is fine and the GUI is not sending it.
- **`grep -c` on a binary counts LINES.** Use `strings | grep -c`, and pick a
  string the fix definitely contains — a format string, not a code identifier
  that may be inlined away.
- **`document.visibilityState` is a LYING INSTRUMENT for anything WebKit gates
  on page presentation** — capture permission above all. MEASURED on guihost
  (WebKitGTK 2.52.5, 2026-08-02) in a bare GTK+WebKit harness with no yggterm in
  the process:

  | arm | `visibilityState` | GTK `mapped` | `permission-request` | `getUserMedia` |
  |---|---|---|---|---|
  | shown toplevel | `visible` | true | raised at 145 ms | resolved, 215 ms |
  | window never shown | `hidden` | false | **never** | **never settles** |
  | `GtkOffscreenWindow` | `visible` | **true** | **never** | **never settles** |
  | shown late (t=4 s) | — | — | raised 11 ms after the reveal | resolved |

  The offscreen row is the one that matters: the page says `visible`, GTK says
  `mapped`, and WebKit defers anyway. **Ask GTK whether the webview is mapped in
  a mapped toplevel; never ask the page.** A repro that qualifies a surface as
  "provably visible" by reading `visibilityState` is measuring something else,
  and this was already written into a bug entry as the discriminator to use.
- **A version difference can be a visibility difference wearing a version's
  clothes.** The same bug read as "worked on GUI 2.12.24, fails on 3.0.0" — one
  working observation on a surface the user had open, every failing one on
  agent-made surfaces nobody revealed. The bisect was queued and would have
  found nothing. **The harness that removes the product from the process is the
  cheap discriminator**: 60 lines of PyGObject against `gir1.2-webkit2-4.1`
  reproduced it with no yggterm code at all, which killed the version
  hypothesis outright in one run. Reach for that before bisecting a build.

### 7.5 The reaper can be the cause

Reclaim that destroys a page which is immediately re-created reclaims nothing
and pays for a fresh web process every cycle. Measured live: **166
`background_hold_expired` closes against 166 re-opens in fifteen minutes**, one
churned process ballooning to 3.9 GB, on a host whose swap was 100% full —
the reaper reacting to pressure it was substantially creating, while the user
could not hold keyboard focus long enough to type a prompt. Any pressure-driven
reclaim needs hysteresis: a target that keeps coming back is not a target.

### 7.6 A working GPU is not consent to arm a compositing mode

Hardware GL armed Phase F under-glass by default, and the user's entire window
became a background agent's page. Under glass the shell webview is TRANSPARENT
by construction, so anything that stops the shell painting shows whatever
surface is behind it, full screen — and it fired exactly when memory was
exhausted, which is when the shell is least likely to paint. Under the legacy
opaque stack the same starvation is invisible. Keep the GL decision and the
compositing decision separate; `shm_force_for_arming` already refuses SHM on a
hardware host, so hardware GL keeps DMABuf either way.

### 7.7 `kill -TERM` on the daemon is not the graceful drain

The graceful path is the binary-replacement self-retire, which drains in idle
order. But it defers while ANY owned session was active in the last 300 s, so
under agent load it never converges. Killing instead cost **~7 agent PTYs**;
rows and transcripts survived and each resumes on a click, but in-flight work
was interrupted. Decide deliberately, and tell the user the cost BEFORE doing
it.

### 7.8 The agent outlives the binary

`ychrome-vault` and `yedit` both serve OLD code from a running process after
the binary on disk is replaced — the vault agent had been serving pre-fix code
for 42 hours with its exe deleted, while the fixed binary sat installed. A fix
that is "deployed" is not running until the process that serves it restarts.
Check `/proc/<pid>/exe` for ` (deleted)` and compare process start time against
the fix's commit time.

⚠ **But neither `(deleted)` nor the start-time comparison PROVES the process is
stale, and on 2026-07-31 believing them nearly cost a live browsing session.** A
running `ychrome` client showed `(deleted)` and a start time 37 minutes BEFORE
the merge commit, which read as a textbook pre-deploy client, missing the round-29
control token and due to be cycled. Both signals were misleading: the binary had
simply been re-copied over its own inode by a later sync (new inode, same
lineage), and the process had been started from a pre-merge TEST build of the
same lane that already carried the fix. Reading the deleted image directly
settled it — `strings /proc/<pid>/exe` found the feature markers
(`X-Ychrome-Control`, `gui_not_routing_capable`) present, and the journal showed
ONE refusal, on a page-originated read, which is the gate working as designed.

**The rule: `/proc/<pid>/exe` is still READABLE after the file is unlinked, so
interrogate the image instead of inferring from its metadata.** `sha256sum
/proc/<pid>/exe` against the installed file tells you whether it differs at all,
and `strings` on the same path tells you whether the specific fix is present. A
hash difference alone only means "some other build" — it does NOT license
restarting a process that is serving a user, and a build that differs while
still carrying the fix is not a reason to destroy their page.

### 7.9 Source of truth for a tool's own source

The deployed `yedit` binary's features existed **only** as untracked files on
the build host — no git repo, no remote, no copy anywhere. Before editing any
fleet tool, confirm its source is in version control and pushed.

### 7.9a An indentation-keyed patch DOUBLE-INSERTS at every deeper site

A shallower indent is a **substring of a deeper one**: `"<28sp>foo();"` occurs
inside `"<32sp>foo();"`, starting four characters in. So a two-pass
search-and-replace keyed on indentation — one pass per indent level, to keep the
inserted line aligned — matches the deeper sites **twice**: once in its own pass,
and again in the shallower pass, including lines the first pass just created.

Measured instance: five stamp sites, nine insertions, every duplicate silently
mis-indented above its correct twin. Nothing errored, and the code compiled and
passed — a duplicated idempotent store changes no behaviour, so tests cannot see
it. In a public repo it ships as visible sloppiness.

⭐ **The check that caught it: count the patched sites against the anchor you
expected.** Five `reader_activity` stamps must yield five new lines, not nine. A
patch that "worked" can still have fired twice, and the count is the only cheap
witness. Prefer anchoring on a unique enclosing line over indentation; if you
must key on indent, verify each insertion sits immediately after its anchor.

### 7.10 The row ledger is the authoritative record of what existed — consult it FIRST

`~/.yggterm/row-order-ledger.json` records which live-session rows existed and
in what order, and `~/.yggterm/removed-rows.json` records closes. It exists
precisely because ghost rows are a recurring problem here.

**It was not consulted, and the user lost seven rows.** Asked to remove ghost
sessions, I went to `server-state.json` plus agent-transcript mtimes and built a
"untouched for ≥3 days" heuristic — while the ledger sat there holding every
row's identity and position. Two of the seven removed were real work
("Commit and continue v3 parity", "Fileables campaign"), and the user had to ask
for them back.

**The trap that made a bad method look verified.** The user said the list should
be "20-21". The ≥3.0-day cut left exactly 21, and that coincidence read as
confirmation. It was not evidence — it was a threshold fitted to a target the
user had supplied, then quoted back as if it had been derived. **A heuristic that
agrees with the number you were given is not corroborated; it is circular.**

Same failure at the other end: after a daemon restart the sidebar showed 28 rows
and that count was reported as a sign the restart went well, without ever
comparing it against the ledger's recorded set — which is the only thing that
can say whether those 28 are the RIGHT 28. The user had to point out twice that
the number itself was the bug.

**The ledger now reads back (in-tree 2026-07-26, not yet live-proven).** Each
handover rebuild pass ends by reconciling the assembled row list against the
ledger *as the daemon booted with it* — remembered rows take the ledger's order,
rows the ledger never saw keep the slot the anchored import walk gave them, and
the result is a permutation, so nothing can be resurrected through it. A daemon
bump also drops a receipt at
`~/.yggterm/manual-snapshots/pre-daemon-swap-<unix-secs>-<pid>.json` (live order
plus the whole ledger), written by the outgoing daemon on `PrepareUpdateRestart`
and by the incoming daemon before it imports a single row. Newest 32 kept;
hand-made `pre-gui-restart-*` snapshots share the directory and are never swept.

**Rules:**
- **Report the number the USER can see: the LIVE SESSIONS count.** `server app
  rows` returns a TREE, and a naive walk over every node carrying a `path`
  also counts the `__live_sessions__` group header, each
  `__remote_machine__/<name>` node, and the `local` node — six extras on the
  live host as of 2026-07-26. That is how a single removal got reported as
  "24 → 23" while the user's sidebar plainly said "18 → 17" (both true, same
  event, different denominators; the user's is the product truth). Filter to
  session paths, and if a count is going into a user-facing sentence, say
  what it is a count OF.
- Before adding, removing, or judging any row, read the ledger. It is the record;
  `server-state.json` is the current belief, and the whole ghost class is the two
  disagreeing.
- Restoring a removed row means clearing its tombstone in `removed-rows.json`
  first, then re-opening the session — otherwise the veto silently wins.
- Never delete a user's session on a heuristic. Show the list, name the evidence
  per row, and let the user choose. Removal is recoverable (the CLI transcript
  survives), but their attention is not.

## 6. Where the deep material lives

- `docs/pending-bugs.md` — open, user-confirmed bugs. The work queue.
- `docs/xterm-bugs.md` — the terminal bug registry, by class.
- `docs/agent-control-plane.md` — the engine verb layer and shadow model, with
  the slice execution order.
- `docs/web-under-glass.md` — Phase F: under-glass web compositing, phases and
  acceptance gates.
- `docs/protocol.md`, `docs/sessions.md`, `docs/daemon-handoff.md` — session
  identity, persistence, handoff.
- `docs/split-view.md`, `docs/alt-keytips.md`, `docs/web-surfaces.md` — feature
  specs.
- `DESIGN.md` — colors, typography, spacing, interaction vocabulary. Consult it
  before styling anything; add durable decisions there rather than in comments.
- `.agents/skills/yggui-app-control/SKILL.md` — the agent's hands and eyes on
  the live desktop.

## 7.9 Transplanted from the bug queue (2026-08-02)

These are traps, not bugs: they describe instruments that lie and rules that
must be read before acting. They lived in `docs/pending-bugs.md`, which owns
open DEFECTS, so they were moved here, which owns what wastes an agent's time.


### ⛔⛔ PRE-REWRITE GIT LINEAGE

**⛔⛔ PRE-REWRITE GIT LINEAGE — read before merging ANY old branch/worktree
(2026-08-01, updated after the SECOND churn).** All public yggdrasilhq
histories were rewritten in place so GPL-3.0-or-later holds from commit
zero, and force-pushed. yggterm's hashes then changed a SECOND time the same
night (user-ordered root-commit-message fix: the root no longer claims
apache; root `c92495da`, tip `53995093` at the time of writing). Anything on
an older lineage silently REINTRODUCES the old history if merged or pushed —
`git merge-base --is-ancestor c92495da HEAD` failing is the tell for
yggterm. Fleet state after cleanup: dev main + 9 lane worktrees + guihost + oc
all on the final lineage (merged lanes reset; tree-identical proof used, so
nothing was lost). STILL ON OLD LINEAGE, rebase before ANY use:
`lane/dev/chord-focus` (dirty WIP worktree), `lane/dev/agent-liveness`
(1 unique commit, replay with cherry-pick onto new main), oc's yggterm
spare edits if any reappear, and dev's `~/gh/yggdocs` (4 dirty files) +
`~/gh/yggdrasil` (13 dirty files) — both repointed from the deleted Forgejo
mirrors to GitHub origins but NOT yet reset. Pre-rewrite bundles:
`/home/user/repo-bundles-pre-gpl0-20260801/`. Releases: yggterm's 185 +
yggsync's 4 + yggcli's 6 Apache-era releases deleted (user-ordered); live
lanes start at yggterm v2.12.23 and yggsync v0.3.2 (yggclient fetch pins
bumped to v0.3.2 in `bc59617` — yggsync releases are LOAD-BEARING for phone
provisioning, never delete without a replacement release first).


### ★★ THE FOURTH FOCUS PATH

**★★ THE FOURTH FOCUS PATH — FOUND AND FIXED 2026-07-24 (2.12.9). Read this
before ever "fixing" a focus steal again.** The user could not type in yedit;
three previous fixes all missed, because every one of them hardened something
NAMED like a focus path (the reclaim script, the input-policy script, the
`uiOwnsFocus` allowlist, the covered-host `pointer-events:none`). The actual
thief is the shell root's **`onclick` handler** in `fn app()`: it fires for
every click anywhere in the window and `document::eval`s a script that
refocuses the active terminal's helper textarea. It bailed out for a live WEB
surface — the same bug was found and fixed there once ("click the new-profile
field and it loses focus immediately") — but nobody taught it about the
DOCUMENT surface, which did not exist yet when that bail was written.
**How it was finally caught** (the method matters more than the fix): patch
`HTMLElement.prototype.focus` on the live GUI to log any call landing on an
`.xterm-helper-textarea`, AND wrap the registry's `focusTerminal` /
`setInputEnabled` / `term.focus` so a hit says WHICH closure ran; then drive a
REAL `server app pointer click` into the editor. The log read: click lands in
the editor, ~93 ms later `helper.focus()` fires with an EMPTY marks list and a
`global code@dioxus://index.html` stack — i.e. a freshly-eval'd script, not
any registry closure. That empty marks list is what convicted the click
handler. ⚠ A JS `el.focus()` probe passes while the bug is live; only a real
pointer click reproduces it, because the thief is a DOM click handler.
**Fixes:** the Rust bail now includes `document_surface_visible_for`, the
script is extracted as `root_click_terminal_focus_script` carrying the shared
`UI_FOCUS_OWNER_SELECTORS` guard (so it also stops yanking focus out of the
sidebar, the theme editor and settings fields), and
`every_helper_textarea_focus_site_is_guarded_or_a_recorded_probe` scans the
source so a FIFTH script cannot hide the same way — enumerating these by hand
is exactly what let this one survive three rounds.


### THE STALE-DAEMON TRAP

**THE STALE-DAEMON TRAP — read before diagnosing ANY "the fix didn't work".**
A deploy that lands new binaries does NOT mean the new code is running. The
daemon's idle gate defers its own retirement while any owned session is
actively working — and on a campaign machine an agent session is ~always
working, so the daemon can stay pinned indefinitely. On guihost 2026-07-11 the
daemon ran **2.10.3 for 19h44m while 2.10.13 sat on disk**: the CR-faithful
sanitizer fix and the CC re-birth fix from campaign run 1 were compiled,
deployed, and never executed. Both bugs were still live for the user, and run 1
had recorded them as "fixed on branch, live-verify pending" — the gap was
invisible.
**Always check `yggterm-headless server status → server_version` against the
on-disk binary BEFORE concluding anything about a fix.** As of 2.10.14 the
metadata sidebar's Daemon section surfaces version, uptime, a
newer-build-on-disk flag, and the daemon's own deferral reason, plus a manual
hot-restart button — so this is visible in the product rather than only to an
agent who thinks to look.


### Diagnostics available

Diagnostics available

- `~/.yggterm/event-trace*.jsonl` — up to 3 days of trace generations (2.10.2).
- `~/.yggterm/agent-incidents.jsonl` — durable agent resume-error incidents.
- `scripts/render_fail_patterns.py` — groups render fail patterns.


### `killed=0` in the agy store filters nothing, and the columns that look built for this are empty

`scan_antigravity_sessions` selected `WHERE killed=0` and read like a guard.
Measured 2026-08-20 across a 999-row `conversation_summaries.db`: **every row had
`killed=0`**, so the clause decided nothing. The same store had `source`,
`status`, `agent_name`, `nesting_depth`, `parent_conversation_id`, `battle_id`,
`not_fully_idle` and `last_user_input_step_index` uniformly empty or default —
eight more columns that each look like the discriminator and are not one.

⇒ **Before filtering on a store column, print its distribution.** A column whose
name promises a distinction can hold one value for the whole table, and a filter
on it is indistinguishable from no filter until you look. Only `step_count` and
`workspace_uris` carried signal; the rule built on them is
`antigravity_row_is_durable` (`docs/cli-integration.md` §The agy durable rule).

### A count computed before a dedup, shipped beside the rows it disagrees with

`server startpage ls` reported `durable_count: 2394` in the same JSON object as
1742 rows. The count was `rows.len() + remote_total`, taken **before** the
`all_rows_map` merge that produced the rows — so every session present both
locally and on a peer was counted twice.

⇒ **A total and the list it describes must be derived from the same value.** The
failure is invisible from either side alone: the rows were right, the arithmetic
was right, and only holding them together shows it. Both oracles now assert
`durable_count == len(rows)` whenever the reply is not truncated.

### `--json` capped at 200 while the human format did not, and said `group_count: 527` beside 8 groups

The default `--limit 200` truncated the payload with nothing in it saying so, so
a machine reader saw a full-looking reply with a header count 65× its own
contents. The replies now carry `limit` and `truncated`.

⚠ Related, same verbs: `server cwdtree ls --help` **ran the verb** — a full store
walk that printed sessions, the one output nobody reads as help. `--help` is now
answered before the scan in all three `ls` verbs.

### A warning that fires on every run stops being read

All three `ls` verbs printed `OpenCode has no store globs and no declared gap —
sessions will be invisible` on every invocation, while `scan_opencode_sessions`
read that store perfectly well. The predicate asked only about globs; a store
that is one SQLite file cannot have one. It had been printing long enough that
`docs/cli-integration.md` documented both CLIs as *"Gap — by design"*, which was
never true of the shipped code.

⇒ Owner: `kind_has_dedicated_scanner`, shared by the scanner dispatch, the three
warnings and the registry test.

### `prompt_count = 0` does not mean the file is empty — and must never drive a delete

Four muse sessions on one host carry `prompt_count: 0` and `title: "New session"`
in `session-index.db` with **12 KB of real records** behind them: metadata,
route facts, a clean `session_end`, ~12 minutes of uptime. They are correctly
SKIPPED by the scan (nobody typed, so there is nothing to resume) — but the index
row is not evidence that the file is empty.

⛔ `is_noise_session_file` classifies; it has exactly two callers and both
`continue`. **There is no delete path in the scan**, and a claim that one exists
has circulated — do not build on it. `scanning_never_removes_a_session_file` pins
this, because the cost of getting it wrong is somebody else's transcripts.

### An oracle can be blind to an entire CLI and report it as the verb's fault

`check-startpage/cwdtree/titles` walked FILES only. Antigravity keeps its index
in SQLite, so the walk found none of it and reported all 999 agy rows as
`verb has ids not in manual walk (extra)` — the checker's own blind spot,
attributed to the code under test.

Two more of the same shape in those scripts: the manual walk compared with
`abs(verb - manual) > 10` and `> 2` slack, which cannot see ten missing sessions
or two missing folders; and rows the daemon scanned from **peer machines** can
never appear in a local walk, so they showed as permanent drift. All three are
fixed; the rules are shared in `scripts/ygg_scan_truth.py`.

⭐ **`YGGTERM_CHECK_BIN=./target/release/yggterm-headless`** points an oracle at
your own build. Without it the scripts run `~/.local/bin/yggterm-headless` — the
INSTALLED binary — so a lane can "verify" a fix it never actually tested.

### A label test that consulted the live title store, so the same code passed on one host and failed on another

`shell::tests::remote_scanned_session_label_falls_back_to_short_id_not_generic_codex_session`
was reported failing on clean `main` from one fleet host and passed on another at
the same commit. Neither "a lane broke it" nor "the test encodes a stale
contract" was true.

`remote_scanned_session_label` opened the real `~/.yggterm/session-titles.db`
inline. If that store happened to hold a row for the fixture's session id, the
saved title was returned and the fallback assertion failed — so the test's
verdict was a property of the developer's machine.

**Reproduced with a control 2026-08-20:** plant a row for the fixture id in the
live store and the old code fails with `left: "<planted>", right: "00000000"`,
the exact shape reported; the split version passes with the same row present.

⇒ The decision logic is now `remote_scanned_session_label_with_saved_title`,
which takes the store's answer as an ARGUMENT. The wrapper does the file read and
nothing else. **A test that consults a live store measures the machine** — and
this is the recurring shape here, not a one-off; see
`[[finding-a-unit-test-that-reads-the-users-settings-store]]`.

## Corner instruments are blind to the pixel (measured 2026-08-20)

- `dom.shell_root_border_radius` reports the CSS the page ASKED for, not the pixel drawn: it
  read 10px on a window whose corners rendered square. Never cite it as proof of rounding.
- `server app screenshot` returns RGB — the WebKit snapshot flattens the alpha channel that
  distinguishes a rounded corner from a square one. Only a compositor grab can tell them
  apart; `scripts/corner-contract.sh` is the instrument that answers in pixels.
- Sampling a compositor grab at (0,0) reads the compositor's own titlebar and reports square
  on a perfectly rounded window — sample inside the window's own geometry.

## OS-level cursor synthesis does not reach a webview surface (measured 2026-08-20)

`swaymsg seat - cursor set/press/release` moves a real compositor pointer and is the right
instrument for GTK widgets, and it drives NOTHING in the page. A synthesized double-click
aimed at the theme editor's pad added no stop and raised no error — the events simply are not
delivered, so the probe reads exactly like a feature that does not work.

- Use `server app pointer <move|down|up|click|drag|scroll>` for anything inside the webview.
  It was built for Wayland/KWin precisely because OS-level tools cannot be trusted there.
- ⛔ `pointer down` / `pointer move` / `pointer up` as SEPARATE calls do not compose into a
  drag — the press state is not carried between invocations, so the stop never moves and
  nothing reports a problem. Use the composite `pointer drag --start-x … --end-x …`, which is
  the only form that produces a gesture the page sees.
- Aim at the CURRENT geometry, read fresh. Coordinates from an earlier probe go stale the
  moment anything moves, and a drag aimed at where a handle used to be lands on bare
  background and looks like a dead feature.

## `element_coordinates()` is relative to the event TARGET, not to your handler's element

Dioxus's `element_coordinates()` is `offsetX/offsetY`. On a bubbled event that is measured
against whatever was hit, so a handler on a large surface silently receives coordinates in a
small child's space whenever the pointer is over one. Measured mid-drag on the theme pad, one
continuous gesture reported x = 60, 73, then -2, 11 (over a 22px handle), then 110, 123.

- The tell is a value that is plausible but far too SMALL, and only sometimes.
- Gating the children with `pointer-events:none` while a gesture is live fixes every frame
  except the first, because the mousedown that starts the gesture lands on the child and CSS
  cannot apply until the next render. If the first frame matters, make the children inert
  permanently and hit-test in the parent's own coordinates.

## `terminal submit` → `submitted:false` is not answered by `rows.busy`

`server app rows` → `row.busy` read `False` immediately before three separate submits that all
came back `submitted:false`. `busy` is not the gate the submit consults.

⭐ **Read the submit's own `reason` field** — it says which of several unrelated conditions
applied. The one that cost two silent failures here: *"no agent composer row appeared within
the timeout — the row is mid-output, in a menu, or is not an agent CLI, so input readiness is
unanswerable rather than false."* That is a THIRD state, neither ready nor busy, and treating
it as "busy, try later" or as "unreachable" are both wrong. Deliver by file instead of
retrying; the standing rule against retrying `submitted:false` still holds.

## `run_app_control_focus_window` answers "was the request delivered", not "did a window take focus"

It returns `Ok(())` whenever the round trip completes, and never reads
`response.error`. So a reply whose own text is *"app-control focus request did
not produce native window focus"* is an `Ok`.

⛔ **`focus(...).is_ok()` therefore means the message arrived.** The GUI's
startup handoff read it as "a window is up", exited on the strength of it, and
on 2026-08-20 a deploy that retires the incumbent first left the desktop with no
window at all for about twelve minutes.

⇒ Use `app_control_focus_window_took_focus`, which reads the response. The same
shape is worth suspecting in every other `run_app_control_*` verb: they all
`write_stdout_payload(&response)` and return `Ok(())`, so **none of them fails
when the operation fails** — they fail when the *transport* fails. A caller that
needs the outcome has to read the payload.

⚠ And the companion trap in the same incident: the handoff chose its target by
executable path and recency, while a **shadow view runs the identical
executable**. The `client_role` rule that forbids exactly this already existed
and was already enforced for app-control routing; the question "is this the
user's window" had simply been encoded twice. `ClientInstanceRecord::is_active_gui`
is now the one owner. **When an instrument and a rule disagree, look for the
second copy of the rule before doubting the instrument.**

## A per-probe RATE measured off the trace file is wrong during and after a deploy

Several GUI generations write into one trace home, and a retiring GUI keeps
writing until it actually exits. So a census over the live file mixes processes
that are running **different builds** — which is exactly the population you must
not pool when the question is "did my change reduce this probe's volume".

⚠ It cost two wrong conclusions in ten minutes on 2026-08-20: a rationing fix
was measured as "did not work" (2091 flush spans still present) and then
diagnosed at length, when **2044 of them had been written by an already-dead GUI
running the previous build**. The process that mattered had written 4.

⇒ **Filter on `pid`, always, before quoting a rate or a share.** Resolve the
current GUI's pid from `server app clients` and count only its records. The
`pid` field exists for precisely this and is easy to skip because the file reads
as one stream:

```sh
# the pid the question is about — never "the most recent record"
yggterm-headless server app clients | python3 -c 'import sys,json
print([c["pid"] for c in json.load(sys.stdin)["clients"] if c["client_role"]=="active"])'
```

⭐ The same caution covers the opposite error: a probe that looks *absent* may
simply belong to a pid whose records rotated out. Check `event-trace.g*.jsonl`
generations before concluding a probe never fired.

## Two readers named `working` disagree on the same row at the same instant (measured 2026-08-21)

`server snapshot` → a session's **`working`**, and `server gate-screen` →
**`screen_text_shows_agent_working`**, sound like the same question. They are
not, and on a Claude Code row holding an owner question they answer **opposite**:
ten consecutive simultaneous sample pairs read `working: false` beside
`screen_text_shows_agent_working: true`, with `awaiting_user_choice: true` and
the picker confirmed on screen in every one.

**Why, and which to trust.** The daemon sets `session.working` from **THIS CLI's**
descriptor, deliberately — the daemon's own comment says the kind-agnostic matcher
"can mistake one CLI's completion trace for another's work signal". `gate-screen`'s
field is that agnostic union. ⇒ **For a row whose CLI you know, the snapshot's
`working` is the answer; `gate-screen`'s is a union and reads broader.**

⚠ **The trap is which one you meet first.** `gate-screen` prints its field in the
output you are already reading, so it is the one a verifier quotes — and it is the
less specific of the two.

⛔ **A corollary that inverts a widely-repeated premise.** The comments around the
picker state say a row holding a question is mid-turn, *so `working` reads true and
the misread is "busy working"*. Measured, `working` reads **false**: the CLI's
working phrase leaves the screen when the picker takes it. So the misread the
`awaiting_user_choice` state actually prevents is **"this row is IDLE / finished"**
on a row that is stopped and eating typed sentences — the more dangerous reading,
not the milder one.

## Prove a daemon gate in a sandbox home, never on a live daemon (measured 2026-08-21)

A gate that decides whether to roll a daemon can only be trusted once it has been
watched refusing AND accepting. Both halves need a real daemon, and neither is worth
spending someone's live sessions on.

`YGGTERM_HOME` gives a complete, isolated plane: its own socket directory, its own
bind lock, its own state. A daemon started under it cannot see or disturb the real one
— and if the variable is not exported before the daemon starts, the daemon resolves
the REAL home, finds the real bind lock and refuses. That refusal is the guard
working; it is not a reason to force anything.

```sh
SB=$(mktemp -d); mkdir -p "$SB/bin"
cp ~/.local/bin/yggterm-headless "$SB/bin/"
export YGGTERM_HOME="$SB"                 # ⛔ BEFORE the daemon starts, not after
"$SB/bin/yggterm-headless" server daemon & sleep 6

# The SAME-VERSION rebuild, without building anything: the build id is the binary's
# mtime, so `touch` reproduces a deploy's only observable while md5 proves the
# content — and therefore the version — did not change.
md5sum "$SB/bin/yggterm-headless"; sleep 2
touch "$SB/bin/yggterm-headless"
md5sum "$SB/bin/yggterm-headless"         # identical ⇒ provably the same version
"$SB/bin/yggterm-headless" server status | grep -E 'build_id|hot_restart_pending'
"$SB/bin/yggterm-headless" server monitor --scenario hot-restart
```

⛔ **RUN THE CONTROL FIRST.** A gate that refuses everything passes the falsifier and
is still broken. Query it BEFORE re-stamping the binary: a daemon whose running build
is the one on disk must still answer `already_ready`.

⚠ **AND WATCH WHICH BINARY YOU COPY.** The first run of this recipe silently tested a
VERSION BUMP instead of a same-version rebuild, because the fleet binary sync had
updated the installed binary between the two copies — the successor bound a
`server-3-1-20.sock` while the test believed it was still on 3.1.19. Copy ONCE into
the sandbox and re-stamp THAT file; never re-copy from the install mid-test.

⚠ The monitor streams several JSON objects and the last one is not newline-delimited
from the rest. Decode with `json.JSONDecoder().raw_decode` in a loop; `json.load` on
the whole stream throws, and reading only the first line gets `accepted`, never the
result.

Cleanup: kill by EXE, not by pattern — `pgrep -f <path>` matches the shell that holds
the path in its own command line, so it reports a stray that is your own probe.

## Counting incidents: three verbs, three different answers (measured 2026-08-21)

Asking "how often does this fire" has a right instrument and two wrong ones, and the
wrong ones do not announce themselves.

- ⛔ **`ytrace tail` is silently capped** (20 records). Deriving a RATE from it divides
  a real count by the window those 20 records happen to span, which is short — so the
  rate comes out high and confident. A tail that returns exactly 19-20 records is
  showing you the cap, not the population. Caught giving ~67/min for an event whose
  true rate was ~2/min.
- ⚠ **`ytrace query` counts SPANS**, so it answers only for things that are timed. An
  EVENT (`daemon_declare_absent`) has no duration and returns no row at all — which
  reads as "it never happened" rather than "wrong instrument".
- ✅ **`ytrace incidents --since <window> --json` is the one that answers the
  question**, because it filters on `payload.incident=true`.

⭐ **And when two instruments describe the same duration, check whether they are two
findings or one event.** A `webview/edit_stall` reporting *"VirtualDom frozen ~2s"* and
a `ui/block` with p95 1,969ms in the same window are one freeze seen twice. Reporting
them as two problems doubles the apparent defect count and halves the chance either is
diagnosed.

⛔ **Never take `last_activity` on a single incident as its cause.** It records what
happened most recently before the block, so the most FREQUENT event wins by accident.
Across a sample it was `None` for half the blocks, with four different named
activities splitting the rest — one payload read alone would have named a culprit that
is not one.

## Prove an alarm STOPPED by pinning it to the handover, with the trigger still armed (2026-08-21)

An alarm going quiet proves nothing on its own — the condition may simply have
passed. Two checks turn silence into evidence, and both are cheap.

**1. Pin the silence to the build boundary, by emitter version.** Incidents carry
`app_version`, and a fleet mid-roll has several versions emitting at once. Count by
version, not by wall clock:

```sh
ytrace incidents --since 1h --json | python3 -c '...group by app_version...'
```

⚠ A record timestamped AFTER the new daemon bound may still come from the OLD one —
daemons coexist by design during a handover, and the retiring process keeps emitting
for seconds. Read the record's own `app_version` before calling it a regression. A
count that stopped at the handover, with zero from the new version, is the shape you
want; "one incident after the start timestamp" was the old daemon's last word.

**2. Prove the TRIGGER IS STILL ARMED.** This is the half that gets skipped, and
without it the whole reading is worthless:

```sh
# the alarm's own predicate, evaluated against the live machine
mem_used_fraction = (MemTotal - MemAvailable) / MemTotal   # vs the panic threshold
swap_used_gib     = SwapTotal - SwapFree                   # vs the OLD trigger
```

If the old predicate would fire RIGHT NOW and the fixed build is silent, the silence
is the fix. If the condition has lapsed, you have measured the weather.

⇒ Applied to the swap-residency alarm: swap 7.15 GiB against a 4.0 GiB old trigger —
armed — with zero incidents from the fixed version across ten minutes, against a prior
0.82/min. That is a falsifier answered; "no incidents lately" would not have been.
