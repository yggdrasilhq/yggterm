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

## Editor Behavior

The theme editor is a compact floating modal with a preview pad and color-stop
controls. It applies changes live so the user can judge the actual shell while
editing. The editor shell itself is an opaque editing surface so the main app
does not bleed through controls or text. The overlay behind it may stay very
low-alpha, but it must not use opacity as a substitute for modal readability.
Closing the editor persists the live draft, and Reset restores the default
Yggui theme.

The regression bar for this surface is `theme_editor_contract` in
`scripts/smoke_xterm_embed_faults.py`: it opens the editor, resets to defaults,
sets brightness through app-control, verifies saved/effective theme truth,
verifies legacy alpha/grain values stay pinned to stable defaults, verifies the
shell CSS variables changed, and resets again.

## Blur And Autohidden Titlebar

Supporting chrome should share the shell's tint and gradient. When the
autohidden titlebar is revealed, it must look like the normal titlebar became
visible, not like a detached overlay. It draws over the workspace and may cover
the top strip of content while hovered, but it must not resize the workspace,
shift the terminal grid, or change xterm rows/columns.

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

## Observability

App-control state must expose:

- `effective_yggui_theme` and `saved_yggui_theme`
- theme editor brightness input rect/value
- shell frame background, gradient CSS variable, chrome tint CSS variable, and
  backdrop filter
- `live_blur_supported=false`, `css_backdrop_filter_enabled=false`,
  `compositor_blur_active=false`
- `material_blur_px=0`
- titlebar background, background image, backdrop filter, and rect while
  auto-hide reveal is active

The 23-smoke release gate must include a theme/chrome pass so X11, Wayland,
macOS, and Windows can prove the behavior in their own compositor realities.
