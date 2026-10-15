# sol s46 — the creation-churn identity churn (i-r2 survival arm)

- **Date:** 2026-10-15, sitting 46 (zcode, jojo sess_69744283-36c8-4503-94c0-0b18a497e631)
- **Consultant:** gpt-6.1-sol, reasoning effort HIGH, via codex exec on dev (juju), 220,132 tokens.
- **Staged raw:** /tmp/s46-sol-consult/ (BRIEF.md, trace-slice.jsonl from the
  i2c-rig RED run, i2c-rig.sh). Ephemeral.
- **Graph id:** yggterm@s46-churn-identity-sol

## What was asked

The (i-r2) creation-churn survival arm (sol s45 Q4): a row demoted by a
sibling's creation loses its mount loop (exit_hint=pre_select + the
bootstrap_reset/spawn_skipped pair at the SAME epoch). Q1 name the
reconstruction path; Q2 the next measurement; Q3 the fix shape under the
stable-component-identity law; Q4 rig honesty.

## Verdicts (all applied in the lane)

- **Q1 REBUT — correction (c2, high confidence):** the for-loop's extra
  expression block makes each iteration an UNKEYED wrapper
  (dioxus-rsx 0.7.10 ForLoop::to_tokens → TemplateBody::to_tokens →
  BodyNode::key() = None for raw expressions; the key lives in a nested
  fragment). Prepend ([a] → [b,a], seat_new_live_session puts new sessions
  first) → diff_non_keyed_children: growth branch creates the extra tail
  FIRST (fresh a: bootstrap_reset 291 + skip 292), then diffs index 0 and
  destroys old a (drop_scope removes the scope's spawned tasks →
  terminal_mount_task_dropped 296 pre_select + claim release 297).
  Predicts the exact measured ordering fresh-a → fresh-b → old-a-dropped
  with no epoch change and no retention eviction. c1 (sync sweep eviction)
  ranked second but unproved and unnecessary; c3 distant third.
- **Q2 REBUT — correction:** the macro expansion + diff ordering IS the
  discriminator; no instrumentation lane before the fix. The executable
  check is the render-shape itself (the rig, [a] → [b,a]).
- **Q3 ACCEPT — f4:** make the iteration root the keyed div:
  `{list.iter().cloned().map(|session| { locals; rsx!{ div { key: ... } } })}`
  — IntoVNode for Element preserves the keyed VNode as the item. Applied at
  the retained-terminal loop AND the pinned-web-pane loop (same shape).
  f1/f2/f3 rejected for this failure (positional wrapper replacement is not
  prevented by predicate fixes or hysteresis; broad decoupling unneeded).
  Retention policies, the s15 punch, fences, LEG C all untouched.
- **Q4 ACCEPT — corrections applied to the rig:** BAR-D/R discriminate; BAR-T
  non-discriminating as instrumented (remount re-seeds from the daemon
  screen; read_terminal_buffer_for reads the JS xterm registry — the
  ambiguity is how the buffer acquired content). Split markers
  (`printf '%s%s' CHURN LATE_<nonce>` — the typed echo can never satisfy the
  contiguous bar) + assert absent before demotion; BAR-D rejects
  bootstrap_reset too; BAR-R sliced at the re-select boundary; BAR-P
  delivery-only (no latency claim). The honest client-level trickle
  instrument (a post-write receipt with instance id + source=live_stream at
  the term.write callback, terminal_scripts.rs ~12973) is NAMED and queued —
  its own unit, not this fix's bar.

## What was applied

lane/i2/churn-identity: the keyed-iteration-root patch (viewport.rs, both
loops, constraint comments citing the mechanism) + tools/uxspeed/i2c-rig.sh
(tightened). Bars: i2c-rig RED rc6 on deployed main 1296a65b → GREEN ×2 on
the lane (see the queue record for the landed numbers).

## Same-shape residue (dream)

Any other `for { {locals; rsx!{keyed}} }` loop in the tree churns its
children on prepend/reorder — the split-divider loop in the same MainSurface
is one. Codebase-wide sweep owed; posted to the dreams board.
