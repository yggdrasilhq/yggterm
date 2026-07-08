# Theme And Chrome Contract

Yggterm's theme engine is a shell-chrome system. It may tint the desktop shell,
side rails, titlebar, settings rail, and floating utility surfaces. It must not
become a terminal renderer, a prompt background repair layer, or a substitute
for xterm.js cell styling.

## Theme Model

The portable theme lives in `~/.yggterm/settings.json` as a `YgguiThemeSpec`:

- `colors`: up to six editor stops, with the renderer using the first bounded
  set for the shell gradient
- `brightness`: global lightness within the clamped product range
<<<<<<< HEAD
- `alpha`: legacy setting retained for compatibility, pinned by the stable
  renderer to a high readable material fill
- `grain`: legacy setting retained for compatibility, pinned to zero by the
  stable renderer

Stable Yggterm exposes only brightness as a scalar editor control. Alpha,
desktop blur, and grain are preserved on the `experimental/alpha-blur` branch,
not in the stable release path. The stable renderer must not honor old saved
low-alpha or grain values from `settings.json`; it clamps them to the readable
product defaults before rendering.

Theme ownership is part of the source-of-truth audit in
`docs/architecture-audit-2026-05-16.md`. Stable theme code must not depend on
compositor timing, focus restore, WebKit repaint order, or branch-only blur
experiments. If a saved setting came from an experimental alpha/blur/grain build,
stable code treats it as compatibility input and clamps it before rendering.
=======
- `alpha`: global material translucency for shell tint and gradient stops
- `grain`: subtle repeated film-grain texture density

The editor exposes those three scalar values as three dial-like controls:
brightness, alpha, and grain. App-control must expose their DOM rects and
values, and must support setting them deterministically for smoke tests.

Alpha is a material control, not plain window opacity. The dial value must map
to the shell material alpha on blur-capable shells: alpha 50 means a 50% shell
material paired with blur, not a second hidden opacity scale. Yggterm computes a
stronger blur budget as alpha drops. When compositor/CSS blur is unavailable,
Yggterm must raise the material fill opacity instead of showing raw readable
windows through an alpha-only shell.
>>>>>>> c162185 (Snapshot alpha blur experiment)

## Editor Behavior

The theme editor is a compact floating modal with a preview pad and color-stop
controls. It applies changes live so the user can judge the actual shell while
editing. The editor shell itself is an opaque editing surface so the main app
does not bleed through controls or text. The overlay behind it may stay very
low-alpha, but it must not use opacity as a substitute for modal readability.
Closing the editor persists the live draft, and Reset restores the default
Yggui theme.

<<<<<<< HEAD
The regression bar for this surface is `theme_editor_contract` in
`scripts/smoke_xterm_embed_faults.py`: it opens the editor, resets to defaults,
sets brightness through app-control, verifies saved/effective theme truth,
verifies legacy alpha/grain values stay pinned to stable defaults, verifies the
shell CSS variables changed, and resets again.
=======
The grain control must change a real repeated background layer on the shell, not
only the serialized theme number. Smoke tests should be able to see the grain
layer in `background-image`, `background-size`, and `background-repeat`.

The regression bar for this surface is `theme_editor_contract` in
`scripts/smoke_xterm_embed_faults.py`: it opens the editor, resets to defaults,
sets brightness/alpha/grain through app-control, verifies saved/effective theme
truth, verifies the shell CSS variables changed, and resets again.
>>>>>>> c162185 (Snapshot alpha blur experiment)

## Blur And Autohidden Titlebar

Supporting chrome should share the shell's tint and gradient. When the
autohidden titlebar is revealed, it must look like the normal titlebar became
visible, not like a detached overlay. It draws over the workspace and may cover
the top strip of content while hovered, but it must not resize the workspace,
shift the terminal grid, or change xterm rows/columns.

<<<<<<< HEAD
The stable shell does not use desktop compositor blur or CSS
`backdrop-filter`. Blur proved too dependent on compositor timing, focus
epochs, and WebKit repaint behavior across KDE Wayland/X11. The stable contract
is deliberately simpler: a high-opacity shell fill, restrained gradient tint,
and no blur-backed alpha. The experimental branch may continue research, but a
stable release must report `live_blur_supported=false`,
`css_backdrop_filter_enabled=false`, `compositor_blur_active=false`, and
`material_blur_px=0`.

The stable shell-side owner for these booleans is
`crates/yggterm-shell/src/theme_contract.rs`. Experimental compositor blur,
alpha, or grain work must not reintroduce parallel stable decisions in
`shell.rs`.

If app-control ever reports blur or alpha behavior in a stable build, that is a
contract violation even when the screenshot looks pleasant. Do not "finish" a
theme task by keeping a visually attractive compositor side effect. Move the
behavior to the experimental branch or remove it from the stable path.
=======
Yggterm separates two blur concepts:

- `css_backdrop_filter_enabled`: the WebView material layer uses
  `backdrop-filter`/`-webkit-backdrop-filter` with a translucent tint,
  saturation, and the shell gradient. This is the same family of web material
  used by modern docs sites and app navbars.
- `live_blur_supported`: the desktop compositor can actually blur pixels from
  windows behind Yggterm. CSS alone cannot prove that on a transparent
  top-level GTK/WebKit window.

The CSS material layer is enabled for contained in-window chrome unless
`YGGTERM_DISABLE_LIVE_BLUR=1` is set. The autohidden titlebar hover reveal,
search lane, side rail, and modal surfaces may use `backdrop-filter` because
WebKit can blur the app content behind those bounded surfaces deterministically.
The full top-level shell frame is stricter: on Linux it must not use
full-window `backdrop-filter` unless `YGGTERM_ENABLE_FULL_WINDOW_CSS_BLUR=1` is
explicitly set for a test, because a whole transparent GTK/WebKit window trying
to blur the desktop is visually misleading and CPU-heavy on KDE Wayland.

On Linux, compositor live blur is reported when Yggterm successfully attaches a
native blur region to the transparent Wayland surface, or when an explicit test
override such as `YGGTERM_ASSUME_COMPOSITOR_BLUR=1` or
`YGGTERM_ENABLE_COMPOSITOR_BLUR=1` is set. Without that native compositor proof,
the shell uses a stronger, readable material tint with no full-window blur
filter so focus changes and background windows cannot shine through as unstable
alpha-only chrome. The smoke contract checks these fields, requires the bounded
titlebar hover material to keep its CSS blur, and requires a higher shell fill
alpha when compositor blur is not available.

Reference check: the Tauri v2 docs shell uses a stable `--sl-color-bg-nav`
navbar color and reserves `backdrop-filter: blur(.25rem)` for contained
backdrop/material surfaces. That is the model Yggterm follows: use DOM material
blur inside the app where WebKit can render it deterministically, and require an
explicit compositor-backed path before claiming the transparent desktop window
can blur unrelated windows behind it.

The compositor-backed Linux path first uses the Wayland
`ext-background-effect-v1` protocol when the compositor advertises the blur
capability, then falls back to KDE/KWin's older
`org_kde_kwin_blur_manager` protocol on Plasma sessions that have not adopted
the standard protocol yet. Yggterm imports GTK's real `wl_surface`, requests a
full-window blur region in surface-local coordinates, reapplies that region on
resize/focus/key epochs, and exposes `shell.compositor_blur_active=true` only
after the region is accepted and flushed. X11/KWin blur-behind remains a future
equivalent path. If neither native Wayland protocol is present, app-control must
report `live_blur_supported=false` and the shell must stay on the high-alpha
material fallback.

Linux full-window WebKit `backdrop-filter` remains opt-in through
`YGGTERM_ENABLE_FULL_WINDOW_CSS_BLUR=1`. The normal Linux path relies on the
native compositor blur region for pixels behind the app and keeps CSS blur to
contained in-window chrome, avoiding the high CPU behavior observed when a
transparent top-level WebKit surface tries to blur the desktop itself.
>>>>>>> c162185 (Snapshot alpha blur experiment)

## Observability

App-control state must expose:

- `effective_yggui_theme` and `saved_yggui_theme`
<<<<<<< HEAD
- theme editor brightness input rect/value
- shell frame background, gradient CSS variable, chrome tint CSS variable, and
  backdrop filter
- `live_blur_supported=false`, `css_backdrop_filter_enabled=false`,
  `compositor_blur_active=false`
- `material_blur_px=0`
=======
- theme editor brightness, alpha, and grain input rects/values
- shell frame background, gradient CSS variable, chrome tint CSS variable, and
  backdrop filter
- `live_blur_supported`, `css_backdrop_filter_enabled`, and
  `compositor_blur_active`
- `material_blur_px`, the computed CSS blur budget for the current alpha value
>>>>>>> c162185 (Snapshot alpha blur experiment)
- titlebar background, background image, backdrop filter, and rect while
  auto-hide reveal is active

The 23-smoke release gate must include a theme/chrome pass so X11, Wayland,
macOS, and Windows can prove the behavior in their own compositor realities.
