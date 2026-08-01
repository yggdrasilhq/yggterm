# shellcheck shell=bash
#
# THE answer to "which binary can run as a yggterm GUI view client", for every
# script that has to launch one. Sourced, never executed.
#
# ⛔ YGGTERM_BIN IS NOT A KNOB FOR THIS — it belongs to the DAEMON.
#
# `crates/yggterm-server/src/terminal.rs` exports `YGGTERM_BIN=<its own exe>`
# into every PTY the daemon owns (docs/web-surfaces.md: "the daemon's own
# executable path"). Inside ANY daemon-owned row — which is every in-session
# agent — that names `yggterm-headless`, a build with no GUI that answers every
# non-server argument with "this yggterm build only supports server
# subcommands". Reading it as "the yggterm binary to launch" broke
# `shadow-client.sh` for every in-session agent (docs/pending-bugs.md, J8a/J8b),
# which pushed agents onto the user's live GUI — the exact thing the shadow
# client, and docs/presentation-policy.md, exist to prevent.
#
# It is not a collision a suffix match fixes, either. The exported value comes
# from `current_exe()`, i.e. `/proc/self/exe`, so once a hot restart swaps the
# binary underneath a running daemon it reads `…/yggterm-headless (deleted)`
# and names no file on disk at all — which is how this reproduces on dev, where
# the old code fell through to a `target/release` build that was not there and
# reported "yggterm binary not found". `underglass-sandbox.sh`'s `*-headless`
# case statement misses that spelling too; both callers share this file so
# there is one answer rather than two half-right ones.
#
# ⚠ This is NOT the `/proc/environ`-shows-the-launch-env trap. `YGGTERM_BIN` is
# put there by the daemon at spawn time, so `/proc/<shell>/environ` is the RIGHT
# instrument for it (unlike a `set_var` presentation flag, which never appears
# there at all — docs/presentation-policy.md §4). The mistake was semantic: one
# name, two owners.
#
# So: the scripts' own override is YGGTERM_GUI_BIN, `YGGTERM_BIN` is only ever a
# candidate, and every candidate must PROVE it is a GUI build before it is exec'd.
#
# ⚠ `--version` CANNOT prove it. The fix note filed with the bug suggested that
# probe and it does not work: both builds print a bare version number and
# nothing else (2.12.18 GUI / 2.12.19 headless, measured side by side). The
# discriminator is the binary's own `--help` — only a GUI build offers the
# `install` command (`print_main_help` in apps/yggterm/src/main.rs; the headless
# printer has never carried that line, checked back to 2.0.1). Both halves are
# locked by scripts/check_architecture_contracts.py.

# `/proc/self/exe` grows a " (deleted)" suffix once the binary is replaced.
yggterm_undeleted_path() { printf '%s' "${1% (deleted)}"; }

# Does this binary expose the GUI build's CLI surface? Asks the BINARY, never
# its filename: `~/.yggterm/bin/yggterm` is a headless build on some hosts, and
# a repo `target/release/yggterm` is a GUI one.
yggterm_is_gui_binary() {
  local bin="$1" help
  [ -n "$bin" ] && [ -x "$bin" ] || return 1
  if command -v timeout >/dev/null 2>&1; then
    help="$(timeout 10 "$bin" --help 2>/dev/null || true)"
  else
    help="$("$bin" --help 2>/dev/null || true)"
  fi
  printf '%s\n' "$help" | grep -qE '^[[:space:]]+[^[:space:]]+ install$'
}

# Print the GUI binary to launch, or fail with a named reason on stderr.
# Argument: the repo root, used for the `target/release` fallback.
yggterm_resolve_gui_binary() {
  local repo_root="${1:-}" candidate
  # An EXPLICIT request is never silently replaced: a caller who names a binary
  # that cannot be a view client is told so, rather than handed a different one.
  if [ -n "${YGGTERM_GUI_BIN:-}" ]; then
    candidate="$(yggterm_undeleted_path "$YGGTERM_GUI_BIN")"
    if yggterm_is_gui_binary "$candidate"; then printf '%s' "$candidate"; return 0; fi
    echo "YGGTERM_GUI_BIN=$YGGTERM_GUI_BIN is not a yggterm GUI build — a headless build has no view client to run" >&2
    return 1
  fi
  for candidate in \
    "$HOME/.local/bin/yggterm" \
    "${repo_root:+$repo_root/target/release/yggterm}" \
    "$(yggterm_undeleted_path "${YGGTERM_BIN:-}")"; do
    [ -n "$candidate" ] || continue
    if yggterm_is_gui_binary "$candidate"; then printf '%s' "$candidate"; return 0; fi
  done
  echo "no yggterm GUI build found. Tried \$YGGTERM_GUI_BIN, ~/.local/bin/yggterm, \
${repo_root:-<repo>}/target/release/yggterm and \$YGGTERM_BIN (the daemon's own exe, \
usually headless). Build one with \`cargo build --release -p yggterm\` or point \
YGGTERM_GUI_BIN at it." >&2
  return 1
}
