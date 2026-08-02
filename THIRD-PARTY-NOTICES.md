# Third-party notices — yggterm

yggterm is **GPL-3.0-or-later** (documentation: CC BY-SA 4.0). This file records
what that obliges, and what upstream obliges us to carry forward.

⚠ **This file was written for Apache-2.0 and was rewritten on 2026-08-01 when the
licence changed.** The audit had to be re-run rather than re-labelled, because the
licence flip inverted the question it answers. Under Apache the question was *"is
there any copyleft in the tree?"* and any GPL dependency would have been a defect.
Under GPL-3.0 the question is *"is anything GPL-INCOMPATIBLE?"* — a strictly
easier bar, and copyleft dependencies are now permitted. The old file recorded a
pass against a test we no longer run.

## What GPL-3.0-or-later obliges us

Nothing at all for **use**. The obligations attach to *conveying* — shipping a
binary or a modified source tree to someone else. If you do that:

- **Pass on the licence.** Recipients get the same GPL-3.0-or-later terms, and a
  copy of `LICENSE` must go with the work.
- **Offer Corresponding Source.** Whoever receives a binary must be able to get
  the complete source it was built from, including build scripts, under the same
  terms. For yggterm that includes everything under `vendor/` and `third_party/`,
  because those are compiled into the product.
- **Mark modified files as changed**, and keep the existing copyright, patent,
  trademark and attribution notices intact.
- **No further restrictions.** You cannot add terms narrowing what recipients may
  do, and you cannot use patents or anti-tamper measures to make the effective
  freedom smaller than the licence says.

Using yggterm inside a company, including commercially and including on modified
copies you keep to yourself, triggers none of this. Distribution does.

## Rust dependencies

Full manifest: **`docs/DEPENDENCY-LICENCES.md`**, generated with
`cargo license --avoid-dev-deps`. **Regenerate it whenever `Cargo.lock` changes**
— a stale manifest is worse than none, because it reads as checked.

### The audit — 2026-08-01, 680 crates

Every crate was classified by its licence *field*, with dual licences resolved to
the arm we take (Apache-2.0 or MIT where offered).

**Verdict: every licence in the dependency tree is GPL-3.0-compatible, and no
GPL-incompatible licence appears anywhere.** Specifically absent: **CDDL, EPL,
SSPL, BUSL, Commons Clause, AGPL, GPL-2.0-only, and anything proprietary.**

The cases that needed a named judgement rather than a glance:

| Licence | Crates | Why it is fine under GPL-3.0 |
|---|---|---|
| Apache-2.0 | 14 direct, plus most dual arms | Compatible with GPL**v3** in one direction: Apache code may go into a GPLv3 work, not the reverse. It is *not* compatible with GPLv2 — one reason the target is v3-or-later. |
| MPL-2.0 | `cssparser`, `cssparser-macros`, `dtoa-short`, `selectors`, `option-ext` | MPL-2.0 §3.3 explicitly permits combining with a "Secondary License", and names the GPL. This was the one caveat under Apache; under GPL it is simply allowed. |
| Apache-2.0 OR LGPL-2.1-or-later OR MIT | `r-efi` (×2) | Permissive arms available anyway, and the "or later" on the LGPL arm reaches LGPL-3, which is GPLv3-compatible. |
| Unicode-3.0 | 18 ICU crates | Permissive with a disclaimer; no copyleft, no advertising clause. |
| BSL-1.0, Zlib, ISC, BSD-2-Clause, BSD-3-Clause, NCSA, 0BSD, CC0-1.0, Unlicense | 30-odd | Plain permissive licences, none with an advertising clause. |
| CDLA-Permissive-2.0 | `webpki-root-certs` | A data licence with no downstream conditions beyond the disclaimer. |
| Apache-2.0 WITH LLVM-exception | `target-lexicon` | The exception only widens permissions. |

**One thing genuinely worth checking, and checked:** `openssl` and `openssl-sys`
are in the tree, via `native-tls`. The crates themselves are Apache-2.0 and MIT —
those are only the Rust bindings. The library they bind to is the system
**OpenSSL 3.x, which is Apache-2.0** and therefore GPLv3-compatible. ⚠ This answer
is version-dependent and was the classic trap: **OpenSSL 1.x used the old SSLeay
licence with an advertising clause and was GPL-INCOMPATIBLE**, which is why so
many projects carried a hand-written "OpenSSL exception". If yggterm is ever built
against a pre-3.0 OpenSSL, that exception becomes necessary again.

### The one finding

**`yggterm-webprobe` declared no licence at all** — it was the single `N/A` row in
the audit. A first-party crate in a public repository with an empty licence field
is unlicensed by default, meaning all rights reserved, which is the exact opposite
of what was intended. Fixed in the same commit as this file.

The licence is now declared **once**, in `[workspace.package]` at the root, and
inherited by every first-party member with `license.workspace = true`. That is
deliberate: twelve independent copies of a licence string are twelve chances for
one to be missed, which is precisely how `yggterm-webprobe` went unnoticed.
Manifests that state their own licence are only the vendored third-party crates
and the crates deliberately detached from the workspace.

## Vendored source

⚠ **Vendored code keeps its own licence and its own headers.** MIT and Apache-2.0
are GPL-3.0-compatible, so these trees combine lawfully into a GPL work — but the
combination does not relicense them, and their notices must travel with any copy.

### `vendor/dioxus-desktop`, `vendor/dioxus-interpreter-js` — MIT OR Apache-2.0

From the Dioxus project. `vendor/dioxus-desktop` is a real workspace member and is
compiled into the product. We take the **Apache-2.0** arm; `LICENSE-APACHE` at the
repository root is that text.

⚠ **`LICENSE-APACHE` is retained for this reason alone.** It is *not* a second
licence for yggterm's own code and must not be read as one — yggterm has not been
Apache-licensed since 2026-08-01.

### `vendor/wry` — Apache-2.0 OR MIT

From the wry project. Upstream ships both licence texts alongside the source, in
`vendor/wry/LICENSE-APACHE` and `vendor/wry/LICENSE-MIT`. Those files must not be
deleted.

### `third_party/t3code-timeline/` — MIT

Source **copied verbatim** from **T3 Code**
(<https://github.com/pingdotgg/t3code>, © 2026 T3 Tools Inc.), **MIT**, at
upstream commit `9e29c9d72895022322da52d8e961b38702bad9cc` (recorded in that
directory's `UPSTREAM_COMMIT`). It landed in `5fb438e`.

MIT permits the reuse and **requires the copyright and permission notice to travel
with the copy** — `third_party/t3code-timeline/LICENSE.t3code` is that notice and
must not be deleted. `NOTICE.md` beside it states the same rule.

⚠ **This entry was missing until 2026-08-01.** The directory carried its own
correct notices, but this file — the one a redistributor reads — did not name it.
A per-directory licence that the top-level notices never mention is the kind of
gap an audit is supposed to catch, so: **any vendored tree under `third_party/`
must be listed here in the same commit that adds it.**

⛔ **Distinguish this from work merely INSPIRED by t3code.** yggterm's own session
timeline (the `Rendered` "Web View" surface) is native Rust + Dioxus, written from
the *ideas* — a heterogeneous entry timeline, foldable tool rows, diff-stat labels.
Ideas are not copyrightable and that work owes nothing. This section covers only
the directory that holds an actual copy.

⚠ **The vendored tree is currently DEAD CODE**: `transcript_view::spawn()` has no
callers and the npm bundle is never built. Dead or not, while the files are in the
repository the obligation above stands. If it is ever removed, remove this section
in the same commit.

## Bundled assets

⚠ **This section was missing until 2026-08-02**, and the omission is the same
shape as the `third_party/t3code-timeline` gap recorded above: source that
carries an obligation, shipped in the binary, named nowhere a redistributor
would look. Vendored *code* was audited; vendored *assets* were not, because
they are not crates and so never appeared in a `cargo` licence sweep. **A
licence obligation does not care whether the file compiles.**

Worse, most of these arrived as build artifacts with their headers already
stripped, so the files cannot speak for themselves. Only `assets/xterm/xterm.css`
still carries its own MIT header; the three JavaScript bundles beside it are
minified and carry nothing. **A minified asset is a notice-erasing operation**,
which is precisely when the top-level notice has to do the work instead.

### `assets/xterm/` — MIT

xterm.js (© 2017 The xterm.js authors; portions © 2012-2013 Christopher Jeffrey),
distributed as prebuilt bundles: `xterm.js`, `addon-fit.js`, `addon-webgl.js`,
and `xterm.css`.

MIT requires the copyright and permission notice to travel with the copy.
`xterm.css` retains its header inline. The three `.js` bundles are minified and
do **not**, so this section is their notice and must not be deleted while those
files are in the repository.

### `assets/terminal-themes/ghostty/` — MIT

463 colour-scheme files in Ghostty's `palette = N=#RRGGBB` format, bundled so the
terminal can offer the same theme names users know from Ghostty
(<https://github.com/ghostty-org/ghostty>, MIT). Ghostty's theme set is itself
largely derived from **iTerm2-Color-Schemes**
(<https://github.com/mbadolato/iTerm2-Color-Schemes>, MIT).

The directory carries no licence file of its own — unlike `vendor/wry` and
`third_party/t3code-timeline`, which ship theirs. It should: see the follow-up
below.

⚠ **Colour values are facts and a palette is thin on originality**, so some of
these files may carry no protectable expression at all. That argument is not
worth relying on when the upstream terms are MIT and the compliance cost is one
paragraph. Attribute, and stop thinking about it.

### `crates/yggterm-shell/assets/symbols-nerd-mono.woff2.b64` — MIT

Symbols Nerd Font Mono, from Nerd Fonts
(<https://github.com/ryanoasis/nerd-fonts>), base64-encoded for inlining. The
symbols-only font is MIT. It sits alone in that directory with no adjacent
notice, so this entry is it.

⚠ **This applies to the symbols-only font specifically.** Nerd Fonts also ships
*patched* versions of other typefaces which inherit their upstream licences —
several are OFL, one is a different licence again. If a patched font is ever
bundled, it needs its own entry here; do not assume this one covers it.

### Follow-up owed

`assets/terminal-themes/ghostty/` and `crates/yggterm-shell/assets/` should each
gain a small `LICENSE`/`NOTICE` file the way `vendor/` and `third_party/` trees
do, so a copy of the directory alone still carries its terms. Listing them here
satisfies the notice obligation for a copy of the *repository*; it does not help
someone who lifts one directory.

## No filter data lives here

⚠ **The reason changed on 2026-08-01; the rule did not.** yggterm once shipped
`assets/web-adblock/rules.json` — hand-written ad-blocking rules derived from
upstream filter lists that are GPL-3.0 or CC BY-SA 3.0. Under Apache-2.0 that was
a licence mismatch, and it is the reason usually given for the deletion.

**That objection has now dissolved.** A GPL-3.0-or-later yggterm could carry
GPL-derived filter data perfectly lawfully. Do not cite this section as a
surviving licence prohibition — it is not one any more.

**The architectural objection stands, and it was always the stronger one.** Filter
lists belong to ychrome, which owns that concept; a ruleset here would be a second
owner of it. yggterm applies a ruleset the app hands it over the wire at runtime,
and must never ship one. If that ever changes, it changes as a design decision
made with ychrome — not because someone noticed the licence no longer forbids it.

## ychrome is a separate work

ychrome is **GPL-3.0-or-later** and lives in its own repository with its own
notices. There is no dependency edge in either direction, and the two communicate
over OSC 7717 and a loopback control endpoint. Process separation across a
documented protocol is not linking, so neither project's licence reaches the other
— a fact that mattered more when the two licences differed, and is still the
reason each repository audits its own tree independently.
