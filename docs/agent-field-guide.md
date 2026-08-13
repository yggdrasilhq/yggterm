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
| `app screenshot` (default backend) | A native child webview is on screen — the composite pastes canvas over a DOM snapshot and a GTK widget is in neither layer | `--backend os` |
| `app screenshot` after any GL/compositing change | `toDataURL` returns the canvas backing buffer even when nothing composites to screen; reports `capture_faithful:true` over a black screen | `--backend os`, or the user's eyes |
| `app screenshot` on a client that has switched sessions **(fixed 2026-07-25, `e0dc6c1` — keep the cross-check habit)** | It composited every `isVisible` host ordered by `mountedAt`, and a session you switch BACK to is REVEALED not re-mounted — so its host is the OLDEST while the host it replaced is the newest and stays visible for a while. The stale host was drawn ON TOP: a near-blank frame with `capture_faithful: true` while the terminal was painted fine. Nine rapid switches reproduced it every time | `shadow-client.sh capture` (grim = the compositor's own pixels). The payload now reports `active_session_path` beside the path it drew — if they differ, the frame is not your session |
| `server status` | It pins to its own version's socket and can answer from — or spawn — an empty orphan daemon | `server app …` (PID-routed) |
| `server status` → `terminal_session_count` **as "how many sessions are on this host"** | **Always during a handover, which is exactly when you are watching.** It counts what the ONE daemon you reached OWNS, and a handover changes which daemon that is: a fresh successor answers with a handful while the rest are still owned by, or preserved on, its predecessors. Measured 2026-08-14 — a watch on this field alerted `53 → 29` on a host that had just gained sessions (host-wide **57**, 261 rows, nothing lost). It fires the loudest false alarm precisely on the event it is meant to police | Sum the `OWNED` column of `server daemons` across every daemon, or read `ROWS` from the newest — both are host-wide. Never compare one daemon's count taken before a swap with another's taken after |
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
| `shell_mut_hist` | **Always, for raw writes.** It counts only `safe_shell_mut`; a bare `state.with_mut` is INVISIBLE to it. A no-op raw write therefore reads as *"unattributed render, empty histogram"* — which says "nothing I can see wrote", not "nothing wrote". Three render-storm autopsies in a row reported exactly that and all three died undiagnosed | Grep for raw `with_mut` callsites directly, and treat an empty histogram beside a real render as an ATTRIBUTION GAP, never as an absence of writes |
| `web_surface_contexts` | A surface has no profile dir. It counts only the KEYED map, and `WebContext::new_ephemeral()` is never inserted — so `0 contexts, 41 surfaces live` is indistinguishable from 41 UNSHARED contexts, the exact failure its own doc says it catches. Reached in ordinary operation: the shell passes `profile_dir: None` whenever another client holds the profile write-lock | Count surfaces and contexts together and compare; a zero count beside live surfaces is a BLIND instrument, not a clean result |
| `app_render_rate` | Never — and that is the point. It is **always on** (no env gate) and had already recorded 739 samples over 12.3 h showing the rate FLAT at ~2/s while CPU climbed 3.6×. Nobody read it, and a cluster was briefed to chase the re-render as the growth | Read the always-on probes BEFORE forming a hypothesis. A constant-rate loop cannot be what grows |
| `yggterm --version` | You need the **protocol** version. It reports the `yggterm` package; the daemon uses `yggterm-server`'s, which a version-only bump may not recompile | The daemon's own socket name, `server-<v>.sock` |
| A `FAILED` / `test result:` grep over `cargo test --workspace` | **The workspace does not COMPILE.** A build error yields no failures *and no results*, so every "is it green?" grep reads clean — an empty set wearing a pass. Reached in ordinary operation: a struct gained a field and three test fixtures were not updated, and the suite was unbuildable until someone tried to run one specific test. Every workspace run between that commit and the fix was reading a non-result as a pass | Assert the run HAPPENED before reading its silence: require a non-zero `test result: ok. N passed` with **N > 0**, and check the exit status. A grep that can only ever find bad news cannot distinguish "no bad news" from "no news" |
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
| `pkill -f <pattern>` / `pgrep -f <pattern>` | **Always — and `pkill` is the lethal form.** The pattern matches the `bash -c` running it, so `pkill -f my_fixture.py` kills your own shell mid-teardown (exit 144, seen 2026-07-27). `pgrep` merely lies; `pkill` takes the session with it | `ps -eo pid,args --no-headers \| awk '/[m]y_fixture\.py/ {print $1}'` then `kill` — bracket the first character so the pattern cannot match itself |
| `terminal read-buffer` | It is **two instruments wearing one name.** On a MOUNTED row the GUI CLIENT answers (wrapped in `request_id`/`handled_by_pid`/`data`); on an UNMOUNTED row the DAEMON answers (**top level, no `data` wrapper**, `source:"daemon_screen"`, `client_host:"missing"`). Different freshness, different shape, same verb — and the client's copy can trail the daemon's screen | Read `source` and check whether the reply is wrapped before believing a "missing" line; cross-check against a faithful screenshot |
| `element.hasAttribute('data-web-tab-active')` (or any `data-*` flag) | Whenever the attribute is rendered with the literal value `"false"` — presence is not truth, so every tab reads "active" | `getAttribute(...)` and compare the VALUE |
| `grep` over the working tree for shipped behaviour | Whenever your lane branch is behind `main` — you read source the running binary was never built from, and conclude a shipped knob does not exist | `git show origin/main:<path>`, and check `git log --oneline origin/main` first |
| **A contributed rail pane you are looking at** | The app's daemon predates the installed binary. The pane's schema is served by the DAEMON's resident code, so a shipped, committed, deployed fix renders as the pre-fix pane and every instrument agrees with it. The documented ychrome landmine says a stale daemon REFUSES a new invocation; it has a quieter mode where it ACCEPTS one and serves the old schema (measured 2026-08-02: daemon started 11:58, binary installed 21:11, Edit tab drew the pre-fix wording verbatim) | Compare the daemon process's start time against the binary's mtime BEFORE reading anything into the UI — `ps -eo pid,lstart,args \| grep '[y]chrome --daemon'` vs `ls -la ~/.local/bin/ychrome` |
| `/proc/<daemon-pid>/exe` as "where the installed binary is" | Whenever a deploy RENAMED rather than replaced in place. The link follows a rename, so after `mv yggterm-headless yggterm-headless.old.$$ ; cp new yggterm-headless ; rm -f *.old.*` it reads `…/yggterm-headless.old.4121874 (deleted)` — the grave of the OLD binary, not the install sitting beside it. It names where this process's file WAS. Reading it as the install is what skipped the PTY handoff on `dev` 2026-08-09 and cold-killed **55 live terminals** | The canonical name in that directory (`…/yggterm-headless`), with the exe link as one candidate among several — never as the answer. `disk_replace_handoff_candidates` is the one place that decides this |
| **`observations` on a terminal open attempt** | **Almost always, and it reads as a fault.** Nothing in the GUI produces one: the only caller of `observe_terminal_open_attempt_from_viewport` is the app-control `DescribeState` handler, so the counter measures **how often an agent asked the GUI to describe itself**, not the surface's health. Measured on the desktop host 2026-08-11: **24 of 33 attempts had `observations: 0` and 15 of those reached Ready anyway.** Reading `observations: 0` as "this attempt never got a surface reading" points an investigation at an ingest path that is working exactly as built | The attempt's own `state` and `ready_at_ms`, and the `terminal_open_attempt/ready` event's `extra` — which names the latch (`reason: …` for a `mark_ready`, or the `settled_kind`/`interactive` fields when the viewport observation did it). ⚠ And note what that distinction exposes: an attempt latched by the viewport path became usable **because an agent was watching**, so polling `describe-state` while investigating this area changes the outcome you are measuring |
| The **unsuffixed** `~/.yggterm/event-trace.jsonl` as "the current GUI's trace" | Whenever a short-lived CLI has run — it writes there too, and the GUI is usually on a generation-suffixed file. Following the name instead of the process sends you through a dead pid's history (twice now, ~10 minutes each) | Identify the writer: the file whose fd is held by the `yggterm` GUI process. `for fd in /proc/$(pgrep -x yggterm)/fd/*; do readlink -f $fd; done \| grep event-trace` |
| A directory listing of `~/.yggterm/server-*.sock` | Always, if you read it as litter. guihost holds **633** of them going back to 2.1.x and **every one accepts a connection**: all but the current version are SYMLINKS the daemon retargets to its live socket at startup (`refresh_legacy_server_socket_aliases`) so an older client can still find it. Sweeping them deletes the cross-version compatibility plane | `ls -l` (they are symlinks, not sockets), or connect and read `server_version` |
| **`server app update restart`** used to make the DAEMON current | **Whenever the GUI is already the newest build — i.e. after every successful deploy.** It is a GUI verb: `RestartPendingUpdate` → `restart_into_pending_update`, which **returns silently** when no newer GUI is staged, and the reply is still `error: null` with a full state dump. So on a host whose GUI is current and whose daemon is three builds behind, the correct-looking call changes nothing, reports success, and leaves the same pid. Two clusters read that as "the update gate is stuck" and went looking for the gate; the gate was never consulted | The daemon swap is a different mechanism entirely — the metadata rail's hot-restart (`ServerRequest::HotRestart`, no idle gate, preserves every runtime). To see whether a swap is owed, read `server daemons` (3.0.124+ prints the queued swap and what it waits for); to see the two versions, `daemon_update_state.current_gui_version` vs `.active_daemon_version` |
| `hot_update_handoff_prepared {spawn_ok: true}` | **Always, as evidence a successor exists.** It records that the *spawn syscall* worked. A child that re-execs to the wrong version, finds the socket bound and exits 0 produces exactly this line, and the daemon then lingers as a preserved owner waiting for an adopter that was never created | The next `spawned_daemon_exit`, and — the fact that actually answers it — whether a daemon at or above the target version is now live (`server daemons`) |

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

## 2. Profiling recipes that work

No `perf` on a typical desktop host (`perf_event_paranoid=3`), but these do:

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

## 5. Destructive operations — know before you type

- Any `reconcile` / daemon-screen replay is a full reset + re-seed to the current
  screen. On a healthy session it collapses scrollback and can blank the
  viewport. Run it only on a surface already confirmed broken.
- Never type into a live agent prompt to "test" it.
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

