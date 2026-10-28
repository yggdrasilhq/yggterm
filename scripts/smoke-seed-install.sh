#!/usr/bin/env bash
# Fixture-qualified fresh-install smoke for the ygg4 seed installer.
#
# This exercises the installer's OWN contract end to end in a sandboxed
# HOME against a fixture dist: mandatory checksum, exact-version gate,
# staged activation, previous-generation preservation, command
# publication, and the PATH migration guard. The transaction owner here
# is a fixture that emulates the observable `ynpm seed-activate` contract
# (digest-named staged generation, atomic current flip, --expect-version
# refusal); the REAL native transaction is exercised by the release
# workflow on native runners. Passing here is fixture qualification only —
# it is NOT live-4 proof.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SB="${HOME}/.yggterm/scratchpad/ygg4-install-smoke/run-$$"
VERSION="$(python3 -c "import tomllib;print(tomllib.load(open('$ROOT/Cargo.toml','rb'))['workspace']['package']['version'])")"

pass() { printf 'smoke: PASS  %s\n' "$*"; }
fail_exit() { printf 'smoke: FAIL  %s\n' "$*" >&2; exit 1; }

# --------------------------------------------------------------- fixtures
build_fixture_dist() { # $1 = dist dir, $2 = marker
  python3 - "$1" "$2" "$VERSION" <<'PYEOF'
import hashlib, json, stat, sys, tarfile
from pathlib import Path

dist, marker, version = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
stage = dist / "_stage"
stage.mkdir(parents=True)

# The transaction-owner fixture reads the seed root from the environment so
# the smoke can sandbox it; the installer passes --expect-version, which
# this fixture refuses to ignore (exact-version gate falsifier).
ynpm_fixture = r"""#!/usr/bin/env python3
import hashlib, json, os, shutil, subprocess, sys, tarfile
args, i = sys.argv[1:], 0
opts = {}
while i < len(args):
    a = args[i]
    if a in ("--archive", "--manifest", "--package", "--expect-version", "--seed-root"):
        opts[a] = args[i + 1]; i += 2
    elif a == "--json":
        opts[a] = True; i += 1
    else:
        i += 1
manifest = json.loads(open(opts["--manifest"]).read())
if manifest["release"]["version"] != opts["--expect-version"]:
    sys.exit("fixture: expected %s, manifest says %s" % (opts["--expect-version"], manifest["release"]["version"]))
if opts.get("--package", "ygg-seed") != "ygg-seed":
    sys.exit("fixture: package filter must be ygg-seed")
digest = hashlib.sha256(open(opts["--archive"], "rb").read()).hexdigest()
seed_root = os.environ.get("YGG_SEED_ROOT") or opts.get("--seed-root")
if not seed_root:
    sys.exit("fixture: no seed root")
gen = os.path.join(seed_root, "generations", "%s-%s" % (manifest["release"]["version"], digest[:12]))
tmp = os.path.join(seed_root, "generations", ".staging-%d" % os.getpid())
shutil.rmtree(tmp, ignore_errors=True)
os.makedirs(tmp)
with tarfile.open(opts["--archive"]) as tar:
    tar.extractall(tmp, filter="data")
os.makedirs(os.path.join(tmp, "bin"), exist_ok=True)
for b in ("ygg", "ynpm", "ynpx"):
    os.replace(os.path.join(tmp, b), os.path.join(tmp, "bin", b))
    os.chmod(os.path.join(tmp, "bin", b), 0o755)
shutil.rmtree(gen, ignore_errors=True)
os.replace(tmp, gen)
current = os.path.join(seed_root, "current")
if os.path.islink(current):
    os.unlink(current)
else:
    shutil.rmtree(current, ignore_errors=True)
os.symlink(gen, current)
if opts.get("--json"):
    print(json.dumps({"outcome": "activated", "version": manifest["release"]["version"], "digest12": digest[:12], "current": current}))
"""

(stage / "ygg").write_text("#!/bin/sh\necho 'ygg-fixture-frontend %s %s'\n" % (version, marker))
(stage / "ynpm").write_text(ynpm_fixture)
(stage / "ynpx").write_text(ynpm_fixture)
share = stage / "share/ygg"
(share / "components").mkdir(parents=True)
(share / "kernel/introspection").mkdir(parents=True)
(share / "commands.json").write_text('{"schema": 1, "note": "fixture IR at the payload root"}')
(share / "components/ygg_command_tree.py").write_text("# fixture: command tree renderer\n")
(share / "components/commands.json").write_text('{"schema": 1, "note": "fixture IR beside the command tree"}')
(share / "kernel/introspection/commands.json").write_text('{"schema": 1, "note": "fixture kernel IR"}')
(share / "kernel/probe.py").write_text("# fixture kernel entry\n")
(stage / "seed-manifest.json").write_text(json.dumps(
    {"schema_version": 1, "kind": "ygg-seed", "version": version, "fixture": marker}))
for name in ("ygg", "ynpm", "ynpx"):
    path = stage / name
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

archive = dist / "ygg-seed-linux-x86_64.tar.gz"
with tarfile.open(archive, "w:gz") as tar:
    for path in sorted(stage.rglob("*")):
        if path.is_file():
            tar.add(path, arcname=path.relative_to(stage).as_posix(), recursive=False)
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
(archive.parent / (archive.name + ".sha256")).write_text(digest + "  " + archive.name + "\n")
PYEOF
  # The installer consumes the REAL manifest generator's output.
  python3 "$ROOT/scripts/generate-release-manifest.py" --dist "$1" >/dev/null
}

run_installer() { # $1 = sandbox HOME, $2 = dist, extra installer env inherited
  env -i \
    HOME="$1" \
    PATH="$PATH" \
    YGGTERM_SEED=1 \
    YGGTERM_SEED_DIST="$2" \
    YGGTERM_VERSION="$VERSION" \
    YGGTERM_INSTALL_SCRATCH="$1/.yggterm/scratchpad/ygg4-install" \
    YGG_SEED_ROOT="$1/.yggterm/seed" \
    YGG_MIGRATE_NATIVE="${YGG_MIGRATE_NATIVE:-0}" \
    sh "$ROOT/scripts/install.sh" seed
}

# ===================================================================== run
rm -rf "$SB"
mkdir -p "$SB"
build_fixture_dist "$SB/dist-good" "first" || fail_exit "fixture build (first dist)"
build_fixture_dist "$SB/dist-next" "second" || fail_exit "fixture build (second dist)"

WORK="$SB/home"
mkdir -p "$WORK"

# 1. fresh install
run_installer "$WORK" "$SB/dist-good" >>"$SB/install-1.log" 2>&1 \
  || fail_exit "fresh install failed (see $SB/install-1.log)"
[ "$(readlink "$WORK/.local/bin/ygg")" = "$WORK/.yggterm/seed/current/bin/ygg" ] \
  || fail_exit "ygg link does not route through the seed current generation"
[ -x "$WORK/.local/bin/ynpm" ] && [ -x "$WORK/.local/bin/ynpx" ] \
  || fail_exit "ynpm/ynpx links missing after fresh install"
"$WORK/.local/bin/ygg" --version | grep -q "ygg-fixture-frontend $VERSION first" \
  || fail_exit "installed ygg does not answer with the activated generation's version"
GEN1="$(readlink "$WORK/.yggterm/seed/current")"
[ -f "$GEN1/share/ygg/commands.json" ] \
  || fail_exit "generation is missing share/ygg/commands.json (payload root IR)"
[ -f "$GEN1/share/ygg/components/commands.json" ] && [ -f "$GEN1/share/ygg/components/ygg_command_tree.py" ] \
  || fail_exit "kernel staging must keep commands.json beside ygg_command_tree.py"
[ -f "$GEN1/share/ygg/kernel/introspection/commands.json" ] \
  || fail_exit "generation is missing the kernel IR"
[ -f "$GEN1/bin/ygg" ] && [ ! -e "$GEN1/ygg" ] \
  || fail_exit "seed bins must live in generation/bin, not at the generation root"
case "$(cat "$GEN1/seed-manifest.json")" in *'"first"'*) ;; *) fail_exit "wrong generation activated" ;; esac
pass "fresh install: staged generation, current flip, command publication, layout"

# 2. SSOT containment: no installed file may reference the build repository
if grep -rl "$ROOT" "$GEN1" >/dev/null 2>&1; then
  fail_exit "installed generation references the build repository (SSOT containment violated)"
fi
pass "installed generation is repo-free (SSOT containment)"

# 3. update: second dist activates a NEW generation, previous preserved
run_installer "$WORK" "$SB/dist-next" >>"$SB/install-2.log" 2>&1 \
  || fail_exit "second install failed (see $SB/install-2.log)"
GEN2="$(readlink "$WORK/.yggterm/seed/current")"
[ "$GEN2" != "$GEN1" ] || fail_exit "second install did not produce a new generation"
[ -d "$GEN1" ] || fail_exit "previous generation was deleted (rollback history lost)"
"$WORK/.local/bin/ygg" --version | grep -q "second" \
  || fail_exit "command link did not follow the new generation"
pass "update: new generation activated, previous generation preserved, links follow current"

# 4. missing checksum is a HARD failure, and changes nothing
cp -r "$SB/dist-good" "$SB/dist-broken"
rm "$SB/dist-broken/ygg-seed-linux-x86_64.tar.gz.sha256"
if run_installer "$WORK" "$SB/dist-broken" >>"$SB/install-broken.log" 2>&1; then
  fail_exit "install succeeded without a checksum (verification is mandatory)"
fi
[ "$(readlink "$WORK/.yggterm/seed/current")" = "$GEN2" ] \
  || fail_exit "failed install mutated the active generation"
pass "missing checksum: install refused, active generation untouched"

# 5. checksum drift is a hard failure
printf '%064d  ygg-seed-linux-x86_64.tar.gz\n' 0 > "$SB/dist-broken/ygg-seed-linux-x86_64.tar.gz.sha256"
if run_installer "$WORK" "$SB/dist-broken" >>"$SB/install-drift.log" 2>&1; then
  fail_exit "install succeeded with a mismatched checksum"
fi
[ "$(readlink "$WORK/.yggterm/seed/current")" = "$GEN2" ] \
  || fail_exit "checksum-drift install mutated the active generation"
pass "checksum drift: install refused, active generation untouched"

# 6. PATH migration guard: a non-seed ygg is never silently overwritten
# (write beside, then rename over the link: redirection would follow the
# seed symlink and mutate the generation binary instead).
printf '#!/bin/sh\necho pre-existing-native-ygg\n' > "$WORK/.local/bin/ygg.pre"
chmod 0755 "$WORK/.local/bin/ygg.pre"
mv -f "$WORK/.local/bin/ygg.pre" "$WORK/.local/bin/ygg"
if run_installer "$WORK" "$SB/dist-next" >>"$SB/install-guard.log" 2>&1; then :; fi
[ "$(grep -c YGG_MIGRATE_NATIVE "$SB/install-guard.log" 2>/dev/null || true)" -ge 1 ] \
  || fail_exit "installer did not report the explicit-migration requirement"
[ "$("$WORK/.local/bin/ygg")" = "pre-existing-native-ygg" ] \
  || fail_exit "installer silently overwrote an existing non-seed ygg command"
YGG_MIGRATE_NATIVE=1 run_installer "$WORK" "$SB/dist-next" >>"$SB/install-migrate.log" 2>&1 \
  || fail_exit "explicit migration install failed"
"$WORK/.local/bin/ygg" --version | grep -q "ygg-fixture-frontend" \
  || fail_exit "explicit migration did not publish the seed command"
pass "PATH guard: dispersal/native ygg only taken over with YGG_MIGRATE_NATIVE=1"

# 7. the legacy 3.x flow is still present and syntactically intact
sh -n "$ROOT/scripts/install.sh" || fail_exit "install.sh is not valid POSIX sh"
grep -q "yggterm-direct-launcher-v4" "$ROOT/scripts/install.sh" \
  || fail_exit "legacy direct installer markers lost (3.x compatibility broken)"
grep -q "self-update" "$ROOT/scripts/install.sh" \
  || fail_exit "legacy ynpm self-update routing lost (3.x compatibility broken)"
grep -q "install integrate" "$ROOT/scripts/install.sh" \
  || fail_exit "legacy desktop-integration refresh lost (3.x compatibility broken)"
sh -n "$ROOT/scripts/smoke-seed-install.sh" || fail_exit "smoke script is not valid bash/sh"
python3 -m py_compile "$ROOT/scripts/package-ygg-seed.py" "$ROOT/scripts/generate-release-manifest.py" \
  || fail_exit "python scripts do not compile"
pass "legacy 3.x flow retained; python sources compile; sh sources parse"

printf 'smoke: fixture qualification complete (%s)\n' "$SB"
