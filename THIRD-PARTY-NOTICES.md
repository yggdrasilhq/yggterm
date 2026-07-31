# Third-party notices — yggterm

yggterm is **Apache-2.0**. This file records what that obliges us to carry
forward from upstream.

## Rust dependencies

Full manifest: **`docs/DEPENDENCY-LICENCES.md`**, generated with
`cargo license --avoid-dev-deps`. **Regenerate it whenever `Cargo.lock`
changes** — a stale manifest is worse than none, because it reads as checked.

### The audit, and its one finding

Every crate was classified by its licence *field*, with dual licences resolved
to their permissive arm (we take Apache-2.0 or MIT where offered).

**No GPL, AGPL, SSPL, CDDL or EPL dependency exists in this repository** — which
is the property an Apache-2.0 project has to keep true. The only copyleft in the
tree is **MPL-2.0**, five crates:

| Crate | Reached via |
|---|---|
| `cssparser`, `cssparser-macros`, `dtoa-short`, `selectors` | Servo's CSS stack, through the vendored webview layer |
| `option-ext` | `dirs` → `dirs-sys` |

**MPL-2.0 is file-level ("weak") copyleft and does not conflict with Apache-2.0
distribution.** Its scope is the individual file: our own sources keep their own
licence. Depending on these crates *unmodified* is fine.

**What it obliges:** preserve those crates' licence notices, and make their
source available. Normal crates.io distribution satisfies the second; this file
and the manifest satisfy the first. ⚠ **If anyone vendors and MODIFIES one of
these five, the modified files stay MPL-2.0 and their source must be
published.** That matters more here than in most projects: this repository
already vendors `wry` and `dioxus-desktop` under `vendor/`, so vendoring is a
normal move rather than an exotic one. Vendoring an MPL crate to patch it is the
case to watch.

## Vendored source — T3 Code's timeline renderer

`third_party/t3code-timeline/` contains source **copied verbatim** from
**T3 Code** (<https://github.com/pingdotgg/t3code>, © 2026 T3 Tools Inc.),
**MIT**, at upstream commit `9e29c9d72895022322da52d8e961b38702bad9cc`
(recorded in that directory's `UPSTREAM_COMMIT`). It landed in `5fb438e`.

MIT permits the reuse and **requires the copyright and permission notice to
travel with the copy** — `third_party/t3code-timeline/LICENSE.t3code` is that
notice and must not be deleted. `NOTICE.md` beside it states the same rule.

⚠ **This entry was missing until 2026-08-01.** The directory carried its own
correct notices, but this file — the one a redistributor reads — did not name
it. A per-directory licence that the top-level notices never mention is the
kind of gap an audit is supposed to catch, so: **any vendored tree under
`third_party/` must be listed here in the same commit that adds it.**

⛔ **Distinguish this from work merely INSPIRED by t3code.** yggterm's own
session timeline (the `Rendered` "Web View" surface) is native Rust + Dioxus,
written from the *ideas* — a heterogeneous entry timeline, foldable tool rows,
diff-stat labels. Ideas are not copyrightable and that work owes nothing. This
section covers only the directory that holds an actual copy. The same
distinction is drawn, at length, in ychrome's `THIRD-PARTY-NOTICES.md`
(§"Scriptlets — reimplemented, not copied" and §SponsorBlock).

⚠ **The vendored tree is currently DEAD CODE**: `transcript_view::spawn()` has
no callers and the npm bundle is never built. Dead or not, while the files are
in the repository the obligation above stands. If it is ever removed, remove
this section in the same commit.

## No filter data lives here

⚠ **Read this before adding any.** yggterm once shipped
`assets/web-adblock/rules.json` — 10 KB of hand-written ad-blocking rules
derived from upstream filter lists that are GPL-3.0 or CC BY-SA 3.0. An
Apache-2.0 repository shipping a derivative of GPL data is a licence mismatch,
and it was deleted for that reason (it was also a second owner of a concept
ychrome owns).

**Filter lists and anything compiled from them belong in ychrome, which is
GPL-3.0-or-later precisely so it can carry them.** yggterm applies a ruleset the
app hands it over the wire at runtime; it must never ship one.

## ychrome is a separate work

ychrome is **GPL-3.0-or-later** and lives in its own repository with its own
notices. It does not affect this one: there is no dependency edge in either
direction, and the two communicate over OSC 7717 and a loopback control
endpoint. Process separation across a documented protocol is not linking, and
yggterm's permissive licence is deliberate.
