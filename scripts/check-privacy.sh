#!/usr/bin/env bash
# Refuse to let private material into this PUBLIC repo.
#
# Why this exists: on 2026-08-07 an audit found the owner's private decision
# structure sitting in tracked test fixtures and war-story comments — his
# sidebar row taxonomy (numbered campaign lanes), the names of private data
# stores, live legal-filing portals, and personal home paths. None of it was a
# credential. All of it was a map of a private life, published.
#
# ⛔ THE RULE THIS FILE ENFORCES, and the thing that makes it different from a
# secret scanner: the leak vector here is NEVER a secret. It is an agent
# writing a REAL example into a test fixture or a comment because a real
# example was in front of it. Use invented examples. Always.
#
# ⛔ This checker must not itself become the leak. It matches SHAPES where it
# can (a numbered row title looks the same whatever it is called), and where it
# must name something it holds the term base64-encoded, so the word is not
# greppable in a public tree. Never add a plaintext private term below.
#
# Exit non-zero with the offending lines; silence means clean.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2
fail=0
note() { echo "privacy: $*" >&2; fail=1; }

# Tracked files AND untracked-but-not-ignored ones, minus vendored/third-party
# trees and binary-ish assets.
#
# ⛔ WHY UNTRACKED FILES ARE IN SCOPE, learned 2026-08-09: this checker is meant
# to run BEFORE a commit, and before a commit a newly written doc is exactly
# `??` — untracked. Scanning `git ls-files` alone therefore reported "ok" on a
# file containing a real home path, a private store name and a host name,
# because the file had not been added yet. The one moment the lock exists to
# cover was the one moment it could not see. Verified by writing a deliberately
# leaky file and watching the checker pass.
#
# `--exclude-standard` keeps .gitignore'd build output out, so this stays fast
# and does not flood on target/ or node_modules.
files=$(git ls-files --cached --others --exclude-standard \
  | grep -vE '^(vendor|third_party|node_modules)/' \
  | grep -vE '^assets/' \
  | grep -vE '(Cargo\.lock|\.b64|\.woff2?|\.png|\.jpg|\.ico|\.gz|\.zip)$' \
  | grep -vE '^docs/archive/' \
  | grep -vE '^scripts/check-privacy\.sh$')
[ -n "$files" ] || exit 0

hits() { echo "$files" | xargs grep -nIE "$1" 2>/dev/null; }

# 1. Personal home paths. A public repo must not know whose machine it was on.
#    ⚠ Invented placeholders are the DESIRED form, so they are allowlisted here.
#    A checker that flags the correct answer gets switched off, and then it
#    protects nothing — so keep this list generous and the failure rare.
#    ⛔ THE DETECTOR MUST NOT REQUIRE A TRAILING SLASH. It used to read
#    `/home/[a-z][a-z0-9_-]*/`, so a bare `/home/<name>` at a word boundary —
#    in prose, in a heading, at the end of a sentence — went straight through.
#    Measured 2026-08-13: five such occurrences sat on the public `main` while
#    this checker reported "ok". The allowlist below already handles the
#    unslashed form via `(/|\b)`, so only the detector was wrong.
#    `runner`/`ubuntu`/`ci` are CI home dirs and are placeholders by nature.
PLACEHOLDER='/home/(user|u|x|y|z|operator|gui-host|example|someone|test|alice|bob|dev|dev-host|build|runner|ubuntu|ci)(/|\b)'
#    ⚠ ANCHORED so `/home/` must begin a path, not sit mid-URL: dropping the
#    trailing slash made `https://host/gp/w/home/activity` read as a home path.
#    ⛔⛔ AND THE ALLOWLIST IS APPLIED PER MATCH, NOT PER LINE. `grep -v` drops
#    the whole line, so a line carrying BOTH a real path and a placeholder —
#    `(\`/home/<real>\` → \`/home/user\` in ...)`, i.e. every line documenting a
#    scrub — was laundered clean by the placeholder sitting next to it. That is
#    a guard discarding the payload to save the wrapper: the one line most
#    likely to quote a real path was the one line guaranteed to pass. So blank
#    the placeholders out FIRST, then ask whether any home path is still there.
HOMEPATH='(^|[^a-zA-Z0-9_.-])/home/[a-z][a-z0-9_-]*'
h=$(hits "$HOMEPATH" | sed -E "s#$PLACEHOLDER#<placeholder>#g" | grep -E "$HOMEPATH")
[ -n "$h" ] && { note "absolute personal home paths — use /home/user or an invented placeholder:"; echo "$h" | head -12 >&2; }

# 2. RFC1918 addresses. Real topology is a signpost to live attack surface;
#    RFC 5737 (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24) exists for docs.
h=$(hits '\b(192\.168\.[0-9]+\.[0-9]+|10\.[0-9]+\.[0-9]+\.[0-9]+|172\.(1[6-9]|2[0-9]|3[01])\.[0-9]+\.[0-9]+)\b')
[ -n "$h" ] && { note "private LAN addresses — use RFC 5737 ranges in examples:"; echo "$h" | head -12 >&2; }

# 3. The owner's sidebar row taxonomy. A fixture shaped `"3. word: phrase"` or
#    `"5.1 word: phrase"` is how his campaign lanes are named, and publishing the
#    set publishes what he is working on.
#    ⚠ The SHAPE is also a real feature (outline numbering) and its tests cannot
#    exist without it — so this flags the shape only when the LABEL is not an
#    obviously invented one. Testing outline parsing is fine; naming his actual
#    lanes is not. Add new synthetic labels here rather than weakening the rule.
#    ⚠ Keep this list AHEAD of the fixtures. On 2026-08-13 a full-history sweep
#    returned 22 row-taxonomy hits on the public branch and every single one was
#    an invented label this list simply did not name yet — the checker was
#    flagging exactly the behaviour it exists to encourage. That is the failure
#    the paragraph above predicts, so: when you invent a label, add it here in
#    the same commit.
SYNTHETIC='"[0-9]+(\.[0-9]+)? (widgets|gadgets|sprockets|cogs|levers|spindles|yggterm|demo|sample|project|alpha|beta|gamma|thing|probe|foo|bar|atlasstore|topic[a-z]*|records|word)(:|\b)'
h=$(hits '"[0-9]+(\.[0-9]+)? [a-z][a-z0-9_-]{2,}: ' | grep -vE "$SYNTHETIC")
[ -n "$h" ] && { note "numbered row-taxonomy fixture names a real lane — use an invented label:"; echo "$h" | head -12 >&2; }

# 4. Named private stores / portals / personal projects, held encoded so this
#    file does not republish them. Add new terms with:
#      printf '%s' 'theterm' | base64
for enc in \
  ZG9zc2llcmdyYXBo Y2FsbGdyYXBo ZmluZ3JhcGg= bWVkZ3JhcGg= dGF4Z3JhcGg= \
  aGluZ2U= Z21hdA== amFncml0aQ== dHJ1ZWNhbGxlcg== L3J1bi9zbWI0aw== c21iZnM= \
  YXZpa2FscGFfb3Bj
do
  term=$(printf '%s' "$enc" | base64 -d 2>/dev/null) || continue
  [ -n "$term" ] || continue
  h=$(echo "$files" | xargs grep -nIiF -- "$term" 2>/dev/null)
  [ -n "$h" ] && { note "a private store/portal/project name is present (term withheld) — use an invented name:"; echo "$h" | head -6 >&2; }
done

# 4b. THE SHARED LIST, if this machine has one. `ygg-privacy-guard` — the thing
#     that actually stands at the push boundary — reads its terms from
#     ~/.config/ygg-privacy/private-terms.txt. On 2026-08-11 the two lists were
#     found to have ZERO OVERLAP: this file enforced 12 project names the guard
#     did not, the guard enforced 19 this file did not, and a leak only had to
#     beat whichever checker happened to run. One private campaign name reached
#     a PUBLIC remote through exactly that gap.
#     ⇒ Read the shared list too, so a term added in either place is enforced in
#     both. The encoded list above stays as the FLOOR — it is what protects a CI
#     runner or a fresh clone where this file does not exist, and it must never
#     be thinned on the assumption that the shared list covers it.
#     ⛔ The file is private (mode 600) and its terms are never echoed; only the
#     offending repo lines are shown, as above.
shared_terms="$HOME/.config/ygg-privacy/private-terms.txt"
if [ -r "$shared_terms" ]; then
  while IFS= read -r term; do
    case "$term" in ''|'#'*) continue ;; esac
    term=$(printf '%s' "$term" | tr -d '\r' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
    [ ${#term} -ge 4 ] || continue   # too short to match without false positives
    h=$(echo "$files" | xargs grep -nIiF -- "$term" 2>/dev/null)
    [ -n "$h" ] && { note "a private name from the shared guard list is present (term withheld) — use an invented name:"; echo "$h" | head -6 >&2; }
  done < "$shared_terms"
fi

# 5. Licence-history and estate-intent narrative (added 2026-08-29). A
#    a licence-lineage commit message sat on public main for four weeks with
#    its rationale, its agreement machinery and the owner's commercial intent
#    in the body. The fact of a licence CHANGE, the agreement machinery,
#    and the ownership vocabulary are private; the CURRENT terms are public and
#    live in LICENSE/NOTICE/README — state them, never narrate them.
#    ⚠ Compliance files are allowlisted per-rule below (LICENSE-APACHE is the
#    vendored trees' own text; debian/copyright and THIRD-PARTY-NOTICES.md
#    legitimately name licences). Narrow on purpose — see the guard's note.
LICENCE_EXCL_B64='KF58LylMSUNFTltDU11FfChefC8pTk9USUNFfFRISVJELVBBUlRZLU5PVElDRVN8ZGViaWFuL2NvcHlyaWdodHxERVBFTkRFTkNZLUxJQ0VOQ0VTfHRlc3RzL3ByaXZhY3lcLnJzfChefC8pQ0xBKFwubWR8LXNpZ25hdHVyZXNcLm1kKT8kfChefC8pQ09OVFJJQlVUSU5HXC5tZCQ='
# decoded at run time; the class vocabulary never sits in the clear
LICENCE_EXCL=$(printf '%s' "$LICENCE_EXCL_B64" | base64 -d 2>/dev/null)
LICENCE_FILES=$(echo "$files" | grep -vE -- "$LICENCE_EXCL")
# ⛔ THE ENTIRE PATTERN LIST IS HELD base64-encoded, not plaintext — including
#    this comment's neighbours: a detector that carries its answer key in the
#    clear is itself the leak (rule 4's law), and measured 2026-08-29 a push
#    that TOUCHED this file was refused by the push guard over exactly that.
# ⭐ AND THE DETECTOR MUST PROVE ITSELF BEFORE THE ALL-CLEAR. The first cut of
#    this rule shipped broken — an unset variable in the pattern plumbing —
#    and still printed the all-clear, because "nothing found" and "cannot
#    look" are indistinguishable without a self-check. So the decoded pattern
#    is fired at a canary that MUST match and a canary that MUST NOT; if
#    either disagrees, this rule reports itself broken and fails the run. A
#    detector that cannot detect must never print ok. (The canaries are also
#    encoded: they are the phrases themselves.)
LICENCE_PATTERNS_B64='cmUtP2xpY2VuW3NjXWV8XGJDTEFcYlxcbU1QTCgtMlwuMCk/XGJ8ZHVhbCBsaWNlbltzY11lfGxpY2VuW3NjXWUgKGNoYW5nZXxkZWNpc2lvbnxjbGFpbXxmbGlwKXxlc3RhdGUgZGVjaXNpb258cGFpZCBlZGl0aW9ufHRoZSBlc3RhdGUucyBvd24gcnVsZQ=='
LICENCE_HIT_B64='dGhlIGVzdGF0ZSBkZWNpc2lvbiwgYSBwYWlkIGVkaXRpb24sIHRoZSBDTEEgYW5kIE1QTC0yLjAgY2xhdXNlcywgZHVhbCBsaWNlbmNlIGNsYWltcywgYSBsaWNlbmNlIGZsaXAsIGFuZCByZWxpY2Vuc2luZyBub3Rlcw=='
LICENCE_CLEAN_B64='YW4gb3JkaW5hcnkgbGluZSBhYm91dCB0aGUgY3VycmVudCB0ZXJtcyBhbmQgbm90aGluZyBlbHNl'
LICENCE_PAT=$(printf '%s' "$LICENCE_PATTERNS_B64" | base64 -d 2>/dev/null)
CANARY_HIT=$(printf '%s' "$LICENCE_HIT_B64" | base64 -d 2>/dev/null)
CANARY_CLEAN=$(printf '%s' "$LICENCE_CLEAN_B64" | base64 -d 2>/dev/null)
probe_hit=$(printf '%s' "$CANARY_HIT" | grep -Eic -- "$LICENCE_PAT" 2>/dev/null)
probe_clean=$(printf '%s' "$CANARY_CLEAN" | grep -Eic -- "$LICENCE_PAT" 2>/dev/null)
if [ -z "$LICENCE_PAT" ] || [ "${probe_hit:-0}" -lt 1 ] || [ "${probe_clean:-0}" -ne 0 ]; then
  note "the licence-narrative detector is BROKEN (pattern empty, or canaries disagree: hit=$probe_hit clean=$probe_clean) — the licence check did not run; fix the encoded pattern blob:"
  echo "$LICENCE_FILES" | head -6 >&2
  fail=1
else
  lh=$(echo "$LICENCE_FILES" | xargs grep -nIEi -- "$LICENCE_PAT" 2>/dev/null)
  [ -n "$lh" ] && { echo "$lh" | grep -vqiE 'first-class' && { note "licence-history / estate-intent narrative — state the current terms, never the change:"; echo "$lh" | head -8 >&2; fail=1; }; }
fi

if [ "$fail" -eq 0 ]; then
  echo "privacy: ok — no personal paths, LAN addresses, row taxonomy, licence-history narrative, or private names"
fi
exit $fail
