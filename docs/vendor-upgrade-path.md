# The vendored-crate upgrade path

`wry`, `dioxus-desktop` and `dioxus-interpreter-js` are **vendored and forked**.
This document is how we move them forward without archaeology, and it exists
because the fork had no recorded baseline at all until 2026-08-01: the crates
were imported under commits titled *"Release 2.1.14"* and *"Patch Dioxus desktop
JS for older mac WebKit"*, with no upstream ref, no README, and no way to
compute our own patch set. `vendor/dioxus-desktop/Cargo.toml` even says the
upstream test files "stay on disk for upstream diffs" — the intent was there,
the mechanism never was.

> **The framing that matters.** In Rust, `0.x` puts the BREAKING change in the
> **minor** slot. `wry 0.55 → 0.56` is not a patch, it is a breaking release,
> and 55 of them have shipped. Treating "one minor behind" as small is how a
> fork quietly becomes unupgradable. **We are only ever postponing the pain, so
> the pain has to be measured, bounded and repeatable.**

## 1. The rule that decides everything: additive files are free

Our changes fall into exactly two categories, and they cost wildly different
amounts at upgrade time:

| | what it is | upgrade cost |
|---|---|---|
| **Additive** — a file that exists ONLY in our tree | `web_surface.rs`, `web_surface_clipboard_image_paste.rs` | **zero.** It carries forward untouched. Upstream cannot conflict with a file it does not have. |
| **Inline** — an edit inside an upstream file | the 10 modified files below | **the entire cost.** Every one is a merge conflict waiting for the next release. |

**Measured 2026-08-01, against pristine `dioxus-desktop 0.7.9`:**

```
PURELY OURS (free)                    INLINE EDITS (the whole cost)
  web_surface.rs              8304      desktop_context.rs   1318
  web_surface_clipboard...     173      webview.rs            285
  ─────────────────────────────────     mobile.rs             142
  8477 lines, zero merge cost          protocol.rs            124
                                        app.rs                 82
                                        lib.rs                 19
                                        launch.rs               6
                                        edits.rs                2
                                        native_eval.js          2
                                        ipc.rs                  1
                                        ─────────────────────────
                                        1981 lines
```

`wry 0.55.0`: **440 lines across 8 files**, all inline.

**So the true merge surface of the whole fork is ~2,400 lines — and 1,318 of
them (55%) sit in ONE file, `desktop_context.rs`.** 8,477 lines of our most
important subsystem cost nothing, because they live in files upstream has never
heard of.

⭐ **THE DOCTRINE, therefore: drive inline edits toward additive files.** When
you add capability to a vendored crate, put the logic in a NEW module of ours
and leave the smallest possible hook in the upstream file. The camera/microphone
work is the worked example — it added 665 lines and put **607 of them in
`web_surface.rs`**, paying merge cost on only 58. Do that every time.

⛔ Never "tidy" an upstream file, never reformat one, never fix an upstream typo
in place. Every line you touch is a line you re-merge forever.

## 2. Establishing the baseline (do this before ANY upgrade)

Pristine sources come from crates.io and are byte-exact:

```sh
curl -sL https://static.crates.io/crates/wry/wry-0.55.0.crate -o wry.crate
tar xzf wry.crate            # -> wry-0.55.0/
diff -rq wry-0.55.0/src vendor/wry/src          # which files we touched
diff -r  wry-0.55.0/src vendor/wry/src | grep -c '^[<>]'   # how much
```

⚠ **Baselines are NOT committed to the repo** — they are reproducible from the
version number, and vendoring two copies of upstream is how a tree doubles in
size for nothing. `VENDOR.toml` (below) records the version; the tarball is
always one `curl` away.

## 3. `vendor/VENDOR.toml` — the record that was missing

Every vendored crate declares where it came from. Without this, step 2 is a
guess and the upgrade is archaeology.

```toml
[wry]
upstream    = "https://github.com/tauri-apps/wry"
version     = "0.55.0"          # the pristine release we forked FROM
inline_files = ["src/lib.rs", "src/web_context.rs", "..."]
additive_files = []

[dioxus-desktop]
upstream    = "https://github.com/DioxusLabs/dioxus"
version     = "0.7.9"
inline_files = ["src/desktop_context.rs", "src/webview.rs", "..."]
additive_files = ["src/web_surface.rs", "src/web_surface_clipboard_image_paste.rs"]
```

A test asserts `VENDOR.toml`'s `version` matches the `version` in each vendored
`Cargo.toml`, so the record cannot silently drift from the code.

## 4. The upgrade procedure

1. **Measure first.** Run §2 against the CURRENT baseline. If the delta does not
   match `VENDOR.toml`, stop — someone edited an upstream file without recording
   it, and that is the bug to fix before upgrading.
2. **Fetch the new pristine** (`vendor/wry` → `wry-0.56.0`).
3. **Carry the additive files over verbatim.** They cannot conflict.
4. **Re-apply the inline delta, hunk by hunk, reading upstream's new code each
   time.** ⛔ Do NOT blind-apply a patch file: the reason a hunk exists may have
   been fixed upstream, in which case the right move is to DROP our edit. Record
   every dropped hunk — a fork that never shrinks only grows.
5. **Run the invariant locks** (§5). They are the acceptance test.
6. **Update `VENDOR.toml`** in the same commit.
7. **Live-verify on the GUI host.** A vendored webview crate is the layer the
   entire product paints through; unit tests cannot see a black window.

## 5. The invariant locks — what an upgrade must never break

These encode the things that have ALREADY broken once and would be silent:

- ⛔⛔ **WAYLAND-NATIVE.** Vendored `dioxus-desktop/src/app.rs` forces
  `GDK_BACKEND=x11` whenever it is unset, and yggterm escapes it only because
  its own policy sets `wayland` FIRST. A careless re-vendor silently puts the
  GUI back on XWayland. ⚠ `/proc/<pid>/environ` **CANNOT** answer this
  (`set_var` never appears there) — measure with `xwininfo -root -children`: no
  yggterm window ⇒ native. See `finding-yggterm-must-run-wayland-native`.
- **The web-surface plane exists**: `web_surface.rs` present and its exported
  entry points still referenced from `crates/yggterm-shell`.
- **Clipboard image paste** survives (`web_surface_clipboard_image_paste.rs`).
- **Media capture** survives: `enable-media-stream` set and a
  `permission-request` handler installed. ⛔ An upgrade that drops the handler
  does not disable the camera — WebKitGTK's default with no handler is DENY, so
  it fails CLOSED and looks like "camera broken", not like a regression.
- **The userscript world default**: a script with no `==UserScript==` header
  silently becomes `Isolated` and its page patches vanish. See
  `finding-userscript-world-default-killed-adblock`.

## 6. Cadence

Upgrade **one crate at a time, one release at a time**, each with its own commit
and its own live verification. A combined `wry + tao + dioxus` jump gives you a
black window and no bisect. The whole point of §1–§3 is that the next hop costs
hours instead of days — which is only true if the hops stay small.
