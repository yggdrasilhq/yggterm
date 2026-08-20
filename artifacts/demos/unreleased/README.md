# Unreleased Proof Bundles

This directory holds proof bundles for work that is not yet cut into a release.

Each bundle should represent one feature or one coherent fix. Keep bundles small,
reproducible, and easy to promote into a release-page pipeline later. A bundle is
a `manifest.json` (what is claimed, which commits, which artefacts) plus a
`summary.md` (the human reading of the same evidence) plus a `trace/` directory of
machine output.

## ⛔ NO FAITHFUL GUI SCREENSHOT MAY BE PUBLISHED HERE

**A faithful frame is a photograph of a live desktop, and this repository is
public.** The screenshot verbs exist so an agent can *prove a fix to itself* — that
is their whole job, and it is a good one. Committing the result is a different act
with a different audience.

Measured 2026-08-20, on six 1920×1200 grabs that had been sitting on `main`: every
one carried the operator's home path, the machine's ssh aliases, the cwd tree —
including private data-store directories — and the sidebar's campaign row titles,
which name projects, people and subject matter that have nothing to do with a
terminal emulator. One frame additionally rendered a session's working directory,
transcript path and resume command in the metadata panel. None of that was the
thing being proven, and none of it was noticed, because a screenshot leaks by
*background* rather than by subject: you look at the pane you are proving and ship
everything around it.

⇒ **Take the screenshot. Read it. Do not commit it.** Record in `summary.md` what
the frame showed and which fields it carried, and commit the machine-readable
trace instead — `trace/*.json` is the reproducible half anyway, and it can be
redacted line by line in a way an image cannot. A bundle whose proof was visual
says so, and names what was withheld:

```json
"captures": [],
"captures_withheld": "why this frame cannot be public"
```

**And name artefacts by ROLE, never by machine** — `trace/app-state-gui-host.json`,
not the host's alias. A filename is scanned by nothing and read by everyone.

## Redaction is normal, not a defect

`{"redacted": true}` in a `trace/` file is a first-class outcome. The claim lives
in `summary.md`; the trace is there to be re-derived on a machine that has the
right to see it. Prefer a redacted stub with a note over an omitted file, so a
later reader can tell the difference between *withheld* and *never captured*.

Before committing a bundle, run `scripts/check-privacy.sh`. It scans text; it
cannot open a PNG, which is exactly why the rule above is a rule and not a check.
