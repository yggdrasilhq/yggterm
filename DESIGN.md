<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# DESIGN.md — yggterm

This page routes yggterm's design system; the decisions live in the doors.
The layout is the design/ 1.0.0 contract (`yggdrasilhq/ydesign`,
`docs/design-layout.md`). This file is the prose constitution of the shell:
its **exhibition** — the components and canonical patterns rendered live, as
screenshot-able notebooks — is the app `ydesign` (`yggdrasilhq/ydesign`, run
`ydesign` inside yggterm). Any design work in this repo or any libyggterm
app consults the notebooks; where this tree and a ydesign notebook disagree,
fix the drifted layer and record the correction in both.

## Doors

- [design/notebooks](design/notebooks/) — the chrome books, owned HERE: the worked examples (real schemas as specimens) and the design catalogue (the fleet pixels and the choices behind them).
- [design/00-core-system](design/00-core-system.md) — the reusable rules that transfer across projects: brand intent, visual structure, control language, status vocabulary, motion.
- [design/10-overlay-interface](design/10-overlay-interface.md) — the generic overlay template: keep the core system, replace or trim the overlay; do not bury project-only nouns in reusable sections.
- [design/11-yggterm-overlay](design/11-yggterm-overlay.md) — yggterm's own overlay: product vocabulary, workflows, UI emphasis.
- [design/Inheritance](design/Inheritance.md) — the chain: Dioxus → yggui (libyggterm) → yggterm shell chrome.
- [design/assets](design/assets/) — design assets by kind (icons/, fonts/, components/, img/). Build-bundled packaging icons stay in the repo's own `assets/` — that is build input, not design material.
- [docs/spec-yggterm-fs](docs/spec-yggterm-fs.md) — the `~/.yggterm` filesystem organization spec: what lives where, what is ephemeral, who owns each subtree.
- [ydesign skill](~/.yggterm/skills/ydesign/SKILL.md) — navigation for the design system; installed copy at `~/.yggterm/skills/ydesign/`.
