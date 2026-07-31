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
