#!/usr/bin/env bash
# Install the pre-push leak gate into a repo that pushes to public GitHub.
#
# ⛔ WHY THIS SCRIPT EXISTS AT ALL.
# The guard is what stops private data reaching public GitHub, and until
# 2026-08-13 the ENTIRE chain had no source of truth:
#   · the guard itself lived only in ~/.local/bin, in no repository;
#   · `.git/hooks/pre-push` is a two-line shim, and `.git/hooks` is NEVER
#     tracked by git, by design — so the hook was unversioned too;
#   · a fleet sync replicated the binary newest-wins across three hosts.
# ⇒ A weakening edit on any host — deliberate, or a bad merge — would win the
#   race, spread silently, and every subsequent push on all three machines
#   would go out unguarded while still printing its reassuring
#   "✅ no private data found". The failure lands on "you're fine", which is
#   the most expensive shape a check can have.
#
# ⚠ AND ONLY 2 OF 34 GITHUB-REMOTED REPOS HAD THE HOOK — the two with known
#   past leaks. It was installed where it had already burned someone, which is
#   the opposite of what a guard is for. "The agent remembered" is precisely
#   what a guard exists to replace.
#
# ⛔⛔ THE VERB IS `hook`, NOT `pre-push`, AND GETTING IT WRONG REFUSES EVERY
#   PUSH. The guard dispatches on `cmd == "hook"`; anything else falls through
#   to its usage text and a non-zero exit, which git reads as "the hook says
#   no". The first version of this installer wrote `pre-push` — the name of the
#   FILE — and every repo it touched could no longer push at all.
#   ⚠ It failed CLOSED, which is the right direction for a leak gate and is why
#   this was an availability bug rather than an exposure. But an installer
#   cannot be proven by the hook EXISTING: an inert hook and a working one are
#   identical on disk, and the broken one printed a wall of text that looked
#   like the guard running. Prove it with a real push.
#
# ⭐ The wordlist is NOT here and must never be: the private terms live outside
#   every repo in ~/.config/ygg-privacy/private-terms.txt (mode 600). A guard
#   that carries its own wordlist into a public repo publishes the very list
#   that names the private things.
#
# ⛔⛔ THIS IS A SECOND ENCODING AND IT MUST NOT SURVIVE. `ygg-privacy-guard
# install` ALREADY DOES THIS JOB, and predates this script — writing it without
# checking was an SSOT violation, and it diverged on day one: this script wrote
# the hook to invoke `pre-push`, a subcommand the guard does not accept, so the
# guard printed its usage text and exited non-zero, which git reads as "the hook
# says no". ⇒ EVERY REPO THIS SCRIPT TOUCHED COULD NO LONGER PUSH AT ALL.
# ⚠ It failed CLOSED, which is the right direction for a leak gate and is why
#   this was an availability bug and never an exposure — but note the tell:
#   AN INERT HOOK AND A WORKING ONE ARE IDENTICAL ON DISK, and the broken one
#   printed a wall of the guard's own text that looked exactly like the guard
#   running. ⭐ AN INSTALLER IS PROVEN BY A REAL PUSH OR NOT AT ALL.
#
# ⚖ Why this script still exists for now rather than being deleted outright:
# the guard's own `install` uses `<repo>/.git/hooks`, and in a git WORKTREE
# `.git` is a FILE — measured, it raises rather than installing, while this
# script resolves the path with `git rev-parse --git-common-dir` and is correct
# in both. It also skips repos with no github remote. ⇒ Two encodings exist and
# they disagree; that is the defect. COLLAPSE THEM INTO THE GUARD when it gets a
# private repo home — the guard is the owner, and this file is the stopgap.
#
#   scripts/install-privacy-guard.sh [<repo> ...]     # default: this repo
set -uo pipefail
# ⛔ THE GUARD ITSELF IS NOT IN THIS REPO, AND MUST NOT BE.
# It was tried, and the guard REFUSED ITS OWN PUSH — correctly. Its source has to
# know which remotes are private in order to decide when to scan, so that
# knowledge is IN the code; and its comments carry the incidents it was built
# from. ⇒ A leak gate's source is itself private data, and publishing it here
# would leak exactly what it exists to protect. It stays outside; this installer
# is the tracked half.
SELF="${YGG_PRIVACY_GUARD:-$HOME/.local/bin/ygg-privacy-guard}"
[ -x "$SELF" ] || { echo "no guard at $SELF — set YGG_PRIVACY_GUARD" >&2; exit 1; }

install_into() {
  local repo="$1" gitdir
  gitdir=$(git -C "$repo" rev-parse --git-common-dir 2>/dev/null) || {
    echo "  skip $repo — not a git repo"; return; }
  case "$gitdir" in /*) ;; *) gitdir="$repo/$gitdir";; esac
  git -C "$repo" remote get-url origin 2>/dev/null | grep -q 'github\.com' || {
    echo "  skip $(basename "$repo") — no github remote"; return; }
  mkdir -p "$gitdir/hooks"
  cat > "$gitdir/hooks/pre-push" <<SHIM
#!/usr/bin/env bash
# installed by yggterm scripts/install-privacy-guard.sh — do not edit here;
# .git/hooks is untracked, so an edit made here exists on ONE host and nowhere else.
exec "$SELF" hook "\$@"
SHIM
  chmod +x "$gitdir/hooks/pre-push"
  echo "  installed → $(basename "$repo")"
}

if [ $# -gt 0 ]; then for r in "$@"; do install_into "$r"; done
else install_into "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; fi
