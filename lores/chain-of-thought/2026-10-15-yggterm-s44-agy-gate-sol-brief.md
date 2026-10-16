You are the yggterm campaign's consultant (gpt-6.1-sol HIGH). Verdict shape per
verdict: accept | rebut | defer — correction. The seat executes; judge the
DESIGN, read the staged files, demand evidence.

UNIT: [11.226] Want (2) — the agy (Antigravity CLI) folder-trust startup gate
gets AUTO-ANSWERED. Owner decision 2026-10-15 (settled-calls.md #20, verbatim):
"agy trust gate — AUTO-ANSWER EVERYWHERE (spawn path AND interactive GUI opens;
the owner accepted silent trust of fresh project folders). Unblocks every
agy-minting E2E."

READ (staged in this directory):
- queue-entry.md — the [11.226] queue entry (gate text, DA2 root-cause, the
  measured input laws, today's fresh-start probe FAIL = the owed honesty bar).
- descriptor-excerpt.rs — AgentCliDescriptor gate machinery: the
  startup_gate_screen_phrases field, screen_shows_startup_gate, agy's entry
  (needle "do you trust the contents of this project?"), kimi-code's TWO gates
  (per-directory trust asked EVERY launch + a one-time migration picker — its
  suite answers the trust gate ONLY for its own scratch dirs, deliberately).
- write-guard-excerpt.rs — the daemon TerminalWrite startup-gate guard
  (refuses PROGRAMMATIC sends on gated rows; plain/human writes pass through;
  order vs the draft guard is load-bearing) + STARTUP_GATE_REFUSAL_MESSAGE.
- remote-mint-excerpt.rs — the remote start-agy verb parse + the probe's mint
  call (daemon-side detached spawn; daemon owns the PTY wrapper).
- probe-fail.json — today's live evidence (deployed 9377b5ce): the fresh-start
  scenario mints a fresh-cwd agy row, the row PAINTS the gate (426 chars,
  output seen), the probe fails it labeled "[startup_gate painted — row is at
  a trust gate, not connected]".

MEASURED LAWS IN HAND (do not re-derive):
- agy 1.2.x gates its whole TUI on a DA2 handshake; the daemon protocol filter
  answers DA2 since ~Oct 3, so even UNMOUNTED (headless) rows paint the gate.
- 2026-09-10 measurement (agy 1.2.0): while the first-run modal is up the CLI
  DISCARDS typed input, and a trailing Enter ANSWERED the gate. The picker's
  default selection is ALREADY "> Yes, I trust this folder" — Enter confirms,
  no arrows needed.
- The gate trust decision is "allow this CLI to read/edit/execute files in
  this folder". The OWNER has accepted silent trust for fresh project folders
  — for agy. The ruling names agy ONLY; kimi-code's gates are NOT covered.
- The E2E probe's fresh_start never writes to the row: it mints and WATCHES
  for the composer. So an answer-at-write-time mechanism alone cannot make
  that leg pass — the row must be answered without any programmatic write.

QUESTIONS (verdict each, with corrections):
Q1 PLACEMENT: ONE daemon-level answerer vs write-guard inline answer vs both.
   Constraint set: (a) must cover minted-but-never-written rows (the E2E
   fresh_start leg), (b) must cover interactive GUI opens on the GUI host
   (owner's own viewport — he wants to never see the gate), (c) must work for
   UNMOUNTED rows (daemon reads their screen fine — session_screen_plain_rows),
   (d) the write-guard currently REFUSES programmatic writes on gated rows —
   that refusal is load-bearing protection ([11.107] order law) and stays for
   CLIs WITHOUT the auto-answer policy. Candidate A: a bounded post-spawn
   watcher per minted/resumed agent row (observe → classify → answer once →
   re-classify → bounded retries) + leave the write-guard refusal intact (it
   becomes near-unreachable for agy). Candidate B: answer inline in the
   write-guard (answer, wait for clear, then admit the original write) + a
   spawn watcher for the never-written rows. Candidate C: your shape.
Q2 POLICY HOME: a new per-descriptor field (e.g. startup_gate_auto_answer:
   Option<StartupGateAutoAnswer{ confirm: &str, max_attempts, settle window }>)
   — agy Some(Enter), kimi None, the other nine None (no gate declared).
   Any objection? Should the field carry the CONFIRM BYTES or an enum?
Q3 THE OBSERVATION WINDOW: the gate paints seconds after spawn (measured ~30s
   on a mounted row; DA2 answered instantly now). A single check at spawn+N
   races the paint. What is the honest bound: poll classification every X ms
   for Y seconds post-mint? Only while the row is still classified
   startup_gate? What stops the watcher from answering a LATER re-asked gate
   days after spawn (kimi-style re-asks) — should the watcher arm once per
   row-mint and DISARM after the first cleared classification or a deadline?
Q4 VERIFY AFTER ANSWER: after sending Enter, the watcher must re-read the
   screen and confirm the gate CLEARED (composer/footer painted) before
   declaring success; on persistence after max_attempts it must stop, leave
   the row gated (probes still catch it honestly), and trace the failure.
   Agree? What trace events/fields does the campaign need (the sitter
   convention: named events with structured fields)?
Q5 THE WRITE-GUARD AFTER AUTO-ANSWER: for agy the refusal text says "answer
   it interactively... then resend". With the watcher live, a programmatic
   write racing an unanswered gate still gets refused once, then succeeds on
   resend. Acceptable (one NACK, the client retries) or should the guard
   trigger the answerer inline and admit the write after clear (no NACK at
   all)? Weigh against: the guard's refusal is ALSO the safety net if the
   watcher is dead; silently admitting writes after an inline answer hides
   the answer from the caller.
Q6 GUI-HOST MOUNTED ROWS: on the GUI host the row is mounted and its xterm.js
   viewport answers DA2; the owner's own GUI opens should also auto-answer.
   The daemon-side watcher writing the PTY directly covers mounted rows too
   (the PTY is the same fd the GUI renders). Any GUI-side loop hazard (the
   viewport echoing the Enter, double-delivery)?
Q7 WHAT ELSE BREAKS: name anything in the staged excerpts that this design
   threatens (the [11.107] draft/gate order, the startup-gate refusal tests,
   the E2E assertions that currently EXPECT the honest gate-FAIL — those must
   be updated to expect pass-after-auto-answer for agy while kimi-style rows
   still FAIL labeled; the defmiss scenarios' gate readings).

Budget: ~100k tokens. Be concrete; cite the files. End with a numbered
attack order for the seat.
