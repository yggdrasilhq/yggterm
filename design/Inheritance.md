<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Inheritance

Layer: **yggterm shell chrome (L2 app language)** — the terminal's own
chrome design language: rows, rails, tabs, omnibox, modals, toasts, the
document surface.

Parent: yggui (L1, libyggterm) — semantic theme tokens, typography, row
anatomy, focus, rail and feedback patterns. Transitive: Dioxus (L0).

The exhibition of the parent layer (and of this layer's canonical patterns)
is the ydesign app and its base notebooks (`yggdrasilhq/ydesign`). Where a
notebook and this repo's chrome disagree, fix the drifted layer and record
the correction in both places in the same commit.

Undefined decisions inherit from yggui; explicit decisions in
[design/11-yggterm-overlay.md](11-yggterm-overlay.md) override named rules,
with rationale. Structural accessibility, state ownership and truthful
feedback remain requirements at every layer.
