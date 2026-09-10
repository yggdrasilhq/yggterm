# ynpm — the yggterm-aware app distribution plane

**Status:** shipped in the yggterm workspace; this document is the behaviour
spec. **Owner:** the yggterm distribution campaign.

`ynpm` is the package manager for the yggterm-recognized fleet: localhost plus
the machines the yggterm SSH roster can reach persistently. It is deliberately
generic: an application, agent CLI, TUI, or libyggterm surface is a package
with an executable contract. A package is downloaded and verified once, then
its exact generation can be imported to every fleet host.

The yggterm server remains the session/launch owner. `ynpm` is the package,
generation, cache, and publication owner. No daemon or GUI is required to run
it.

## Commands

```sh
# Production registry packages. Bare names mean @ygghq/<name>.
ynpm install ytop
ynpm install @scope/tool@1.2.3

# The complete local inventory: every integrated CLI plus registered app
# surfaces, whether installed, legacy-managed, system-provided, or absent.
ynpm list

# Check packages already recorded by ynpm against disk and registry.
ynpm check

# Probe path roots and required host tools before a first install or platform
# launch. Generic packages may still work on a host without a native matrix.
ynpm doctor
ynpm sync

# Reconcile every npm-backed integrated CLI into the ynpm agent bin directory.
ynpm sync --integrated

# Make one local, verified generation and import it to named fleet hosts.
ynpm sync-fleet --hosts machine-a,machine-b,machine-c --integrated

# Internal peer-transfer protocol (normally called by sync-fleet).
ynpm export @scope/tool --metadata --archive ~/.yggterm/scratchpad/ynpm/tool.tgz

# Development channel: build recipe + bins come from the checkout.
ynpm dev --fleet machine-a,machine-b /home/user/gh/tool

# Or install already-built artifacts explicitly.
ynpm install --dev @scope/tool --watch @scope/tool \
  --dest /home/user/.local/bin --bin tool=dist/tool

# Leave dev and take the production package now; rollback is generation based.
ynpm prod @scope/tool
ynpm rollback @scope/tool
ynpm remove @scope/tool
ynpm purge-legacy                         # only after integrated sync; live copies are retained

# npx-shaped: update/install when online, otherwise launch the last verified
# generation. Arguments after the package belong to the package binary.
ynpx @avikalpa/codex-session-tui --model gpt-example
ynpx github:owner/tool --help
ynpx --dev /home/user/gh/tool -- --profile demo
```

`sync-fleet` needs `--hosts` (or `YGGTERM_FLEET_HOSTS`) because a bare package
manager process cannot infer a GUI's private SSH roster. The yggterm GUI and
fleet scripts already know that roster and should pass it explicitly.

`sync-fleet` also converges the active production yggterm generation. It
archives the already-verified four-product generation once, imports it on each
named host, and lets the remote `import-yggterm` gate preserve a newer or
same-bytes dev build. A dev yggterm is distributed with `ynpm dev --fleet`;
production never silently downgrades it. If the public version is unchanged
but one or more product hashes differ, the remote generation refreshes only
those products by atomic rename, so release-only polish and daemon fixes do
not disappear behind an already-complete version directory. The manager that
performed the sync is reasserted to both `ynpm` and `ynpx` aliases after the
import, preventing a stale auxiliary copy from downgrading the fleet manager.

## One state and two safe destinations

All ynpm state is under the user's yggterm home:

```text
~/.yggterm/ynpm/state.json
~/.yggterm/ynpm/generations/<package-key>/<version>/
~/.yggterm/ynpm/cache/                         # shared download cache
~/.yggterm/ynpm/bin/<binary>                   # agent-CLI publication
~/.yggterm/scratchpad/ynpm/                    # disk-backed staging only
```

`YNPM_HOME` may name the OS home explicitly for automation; the server sets it
when `YGGTERM_HOME` is customized so package state and launch paths cannot
silently split across two homes. `YNPM_DEST`/`--dest` selects a publication
directory, not a second state root.

First-party apps normally publish their links to `~/.local/bin`, because a
human shell, cron, and another app should find them. Integrated agent CLIs
publish to `~/.yggterm/ynpm/bin`, which yggterm prepends to every CLI row's
launch `PATH`. Both destinations use the same state, cache, generation and
rollback rules; `--dest`/`YNPM_DEST` selects the destination for a transaction.

The link is published by staging a new symlink and renaming it over the old
one. A running process keeps its old inode. Generations are not deleted while
`/proc/<pid>/exe` proves that a live process still executes from them.

### Path and platform preflight

`ynpm` resolves `YNPM_HOME` from `HOME`/`USERPROFILE`, expands `~`, makes the
manager root and publication destination absolute, and keeps staging under
`~/.yggterm/scratchpad/ynpm` rather than the OS temp directory. `--dest` is
resolved relative to the invoking directory; a relative `YNPM_DEST` is resolved
relative to the manager home. Run `ynpm doctor` before introducing a new host
or platform. It reports the native package target, the `curl`/`tar`/`npm`
substrate, all resolved roots, and refuses an unsafe or incomplete preflight.
When old fleet deployments leave more than one yggterm root, `ynpm list` and
update discovery measure every managed executable and select the newest
verified generation, so a stale compatibility state file cannot direct a new
update into an older path.

Native first-party packages are currently published for the targets named by
their platform dependency matrix. A package without a matching native
companion is still allowed when its own npm `bin` is portable; this keeps
Windows/macOS path handling testable now without pretending a Linux-only
native binary is portable.

## Package contract

There are two supported registry shapes. The `@ygghq` scope is a namespace,
not a promise that every package needs a native companion.

### First-party platform packages

The `@ygghq/<name>` package carries the bin table, exact platform optional
dependencies, and the finalize contract. Its platform package carries the
native executable. The main and platform versions must match. Every declared
bin must run `--version` before any destination changes. The platform path is
selected only when the main package explicitly declares the host companion in
`optionalDependencies`; this lets the same manager handle native Rust apps
and portable shell/Python/JS tools.

### Generic npm packages

Any npm package with a `bin` string or map is installable, including the
third-party agent CLIs. ynpm resolves the tag to an exact version, runs npm in
an isolated per-package staging prefix with a disk-backed `TMPDIR`, enables
only that package's declared lifecycle scripts, verifies every resulting bin,
then keeps and publishes the generation. No shared `npm install -g` batch and
no `--force` is allowed: one bad package cannot unlink the fleet's other CLIs.

OpenCode is a deliberate identity test. `opencode-ai@beta` is the abandoned v1
line; the integrated v2 CLI is `@opencode-ai/cli@beta` and its bin is
`opencode2`. ynpm pins the package identity and verifies the resulting binary,
so an `opencode` v1 shim cannot satisfy the opencode2 slot.

Portable `@ygghq` packages and any other npm package with a `bin` string or map
use the generic npm path; no fictional `@ygghq/<name>-<platform>` package is
invented. Non-npm integrated CLIs remain visible in `ynpm list` with their declared
source (`uv`, vendor installer, or manual/self-updater). Their vendor-specific
arrival semantics are not guessed as npm packages. The server's descriptor
adapter remains responsible for those sources until their own atomic ynpm
adapter is added.

## Standard yggterm package metadata

An npm package may declare yggterm integration under the top-level
`package.json.yggterm` key. The declaration is package data, not code that
runs on first use:

```json
{
  "name": "@ygghq/example-app",
  "bin": { "example-app": "bin/example-app" },
  "yggterm": {
    "app": {
      "name": "example-app",
      "label": "Example App",
      "icon": "",
      "binary": "example-app",
      "keytip": "E",
      "verbs": [
        { "id": "open", "label": "Open Example App", "args": [], "row_spawn": true }
      ],
      "context_menu": {
        "enabled": true,
        "contexts": ["workspace", "session"],
        "verbs": ["open"]
      }
    }
  }
}
```

`binary` is a package bin key, never a machine-specific path. `ynpm` resolves
it to the published generation and writes the normalized host manifest with an
absolute path. `label`, `icon`, `keytip`, and the verb schema feed the generic
titlebar/start-page launcher. `context_menu.enabled` is an explicit opt-in;
`contexts` currently names `workspace` (folder/document) and `session`, and
`verbs` limits which `row_spawn` verbs appear. Omitted/invalid context metadata
does not earn a row context-menu slot. This is how a dashboard can remain
launchable from the start page without appearing in every file's context menu.

On install, upgrade, dev publication, peer import, and destination migration,
ynpm refreshes the normalized manifest. On `ynpm remove`, it removes that
package's registration. The daemon only reads the normalized registry and
omits a currently unresolvable binary from its live snapshot; it does not
delete registrations as a side effect of a scan, and apps no longer write
registrations on every run.

## Integrated inventory

`ynpm list` is not a dump of only packages previously written to
`state.json`. It derives the CLI roster from yggterm-core's `AGENT_CLIS`
descriptor registry and reads `~/.yggterm/apps/*.json` for libyggterm and
other registered app surfaces. Each row reports:

- the stable CLI/app identity and package/source;
- the version returned by the executable's own `--version`, when available;
- the source that will actually be used (`ynpm`, legacy yggterm npm,
  user-local, system, or unavailable); and
- the path that was measured.

This makes a missing row a visible integration defect rather than an empty
state file. It also exposes legacy copies before they are purged.

## Fleet distribution

The expensive operation is resolving, downloading, and finalizing a package.
`ynpm sync-fleet --hosts ... --integrated` bootstraps the named peers, asks
each for its verified production generation, and reuses the highest one before
consulting the registry. If no peer is ahead, the local host resolves and
downloads the package once. It then archives each local verified production
generation once and imports that archive on every named host. The remote host
updates its own state and publication links; transfer archives are removed
after the import. A remote host with no ynpm is bootstrapped with the invoking
binary by an atomic replacement.

`export --metadata` is the small peer protocol used for that discovery;
`export --archive` adds a verified production generation to the metadata
response. It refuses dev generations and re-runs every bin's version gate, so
a peer's state file or a successful SSH request is never treated as proof by
itself. If the registry is unreachable after a peer generation was imported,
integrated sync keeps that verified generation and reports the offline
decision.

The remote import repeats the runs-before-publish gate. A transport success is
not the proof: each host must report the imported package, and a later
`ynpm list`/`ynpm check` is the state read-back.

## Development is a first-class channel

The development path is intentionally shorter than release CI:

```text
checkout build → --version gate → dev generation → atomic publication
             → optional fleet import → later production handback
```

`ynpm dev <checkout>` reads the checkout's `package.json` `ynpm` object when
present:

```json
{
  "name": "@scope/tool",
  "version": "1.2.3",
  "ynpm": {
    "build": "bun build --compile src/main.ts --outfile dist/tool",
    "bins": { "tool": "dist/tool" },
    "watch": "@scope/tool"
  }
}
```

For Cargo checkouts, pass `--build` and `--bin name=target/release/name` when
the workspace has more than one binary. A prebuilt artifact can always use
`ynpm install --dev --bin name=path`.

For a package that is one of yggterm-core's integrated npm CLIs, omitting
`--dest` publishes to `~/.yggterm/ynpm/bin`, the directory the server launches
from. That makes `ynpm dev --fleet ...` exercise the exact build the fleet
will run; use `--dest ~/.local/bin` only when the dev binary is intentionally
for the human shell or another app.

The dev marker records build time, commit, builder host, watched production
package, the production version/fingerprint it stood in for, and the dev
fingerprint. The registry never gets a dev upload. `ynpm sync` keeps the dev
generation until the watched production release is strictly newer, or the
same version has a different known production fingerprint (release metadata
or polish). Unknown or malformed registry data keeps the dev build; it never
silently discards local work. `ynpm prod` is the explicit immediate handback.

This is what makes a scratch CLI/TUI/libyggterm app pleasant to develop: one
build, one local health check, one optional fleet push, and no ten-command
copy/repoint ritual. Release polish can arrive at the same package version and
still be recognised by its production fingerprint.

Yggterm itself follows the same generation discipline, although its public
release source is the GitHub release archive rather than an npm package:

```sh
ynpm self-update --json
```

`ynpm self-update` installs and verifies `yggterm`, `yggterm-headless`, `ynpm`,
and `ynpx` as one versioned generation, then atomically advances the direct
install state. A newer production release replaces an older dev build. A
same-version production release replaces a dev build only when its verified
binary fingerprint differs, which admits release-only polish and metadata
without discarding an equivalent dev build. If the dev and production bytes
are identical, the dev generation remains active. The GUI invokes this command
and owns only the user notification plus session-preserving restart; the old
GUI download/install workflow is retired.

The first self-update also repairs legacy compatibility metadata. Historical
fleet deploys may record `fleet-deploy`/`fleet` beside a newer executable; ynpm
normalizes that transport marker to the canonical yggterm GitHub release and
rewrites the active version from the executable's own `--version` answer before
it queries production. Those values must never become a release URL.

`ynpm sync` also invokes this yggterm self-update, and a failed network check is
reported as offline while existing verified generations remain runnable. A
direct yggterm startup that installs a newer generation notifies the user and
restarts through the existing PTY-preserving handoff, so the update converges
on localhost and on each named fleet host without killing sessions.

## `ynpx` behaviour

`ynpx` is a launcher, not a permanent package declaration:

1. resolve the requested npm package/tag and attempt to install the newest
   exact version through ynpm;
2. if the network is unavailable, use the last ynpm-verified generation;
3. if the network works but the package or lifecycle is invalid, fail loudly;
4. launch the package's matching bin with every argument after the package.
   `--bin` and `--dev` are ynpx controls only before a package name; after the
   package name they are passed through like every other application flag.

`github:owner/repo`, `git+https://github.com/owner/repo`, and a local checkout
are dev sources. GitHub sources are cloned/fast-forwarded under ynpm's own
state root and use the same checkout recipe and `--version` gate. `--dev` is
the explicit local-checkout form. No source is treated as trusted merely
because it is reachable: it must produce a verified executable before launch.

## Migration and purge

The old server provisioner used `~/.yggterm/npm/cli/<slug>.genN` and separate
user-local npm trees. The migration order is:

1. deploy ynpm and the server that launches from `~/.yggterm/ynpm/bin`;
2. run `ynpm sync --integrated` on each recognized host, which installs the
   descriptor's exact npm package (including `@opencode-ai/cli@beta`);
3. verify package state, executable versions, and `/proc` lineage;
4. remove only legacy generations and duplicate user-local package trees that
   no live process executes from;
5. leave a generation in place until its running process and all helper
   processes have drained, then reap it in a later sweep.

The purge never removes `/usr`/`/usr/local` package-manager files and never
kills a live agent row merely to make the filesystem tidy. A residual live
legacy generation is an honest, named migration remainder, not a false
"purged" claim.

## Launch flags and proof

The settings modal and the launch builder read the same per-CLI descriptor
table. The configured text is forwarded per launch, including over SSH; the
remote wrapper passes it as a request field rather than reading the remote
host's unrelated settings file. This applies to new launches and to the
daemon's reconstructed resume/picker commands. A requested per-launch model
or permission mode strips the configured spelling it overrides and appends the
CLI's own native spelling.

Codex has two distinct useful postures:

- `-s danger-full-access` disables the sandbox but still asks for approvals;
- `--dangerously-bypass-approvals-and-sandbox` is the actual YOLO posture and
  is the modal's explicitly named `YOLO: skip checks and prompts` tier.

Therefore a Codex row showing no YOLO warning is not evidence that its flags
were dropped: it may be running the first posture. The proof is the composed
launch command and the remote process command line. A selected YOLO preset
must carry the second flag and the launched Codex TUI must paint its warning.

## Release and build rules

- Cargo/package metadata is the version source of truth for first-party builds.
- `ynpm` refuses a binary that does not answer with its package version.
- Dev builds use the yggterm single-build plane for gitcoding repositories;
  `ynpm dev` consumes the resulting artifact and can distribute it.
- Production publishing is separate from dev publication and remains a human
  or CI decision.
- `scripts/check-privacy.sh` applies to repository changes; examples in this
  document are invented.

## Not covered

- ynpm is a binary/app distribution manager, not a transitive dependency
  resolver or replacement for ordinary project-local npm workflows;
- it does not write user-owned app manifests or vendor configuration files;
- it cannot infer a private yggterm GUI roster from a bare shell, so fleet
  targets must be named or supplied through `YGGTERM_FLEET_HOSTS`;
- it does not silently turn uv/vendor/manual CLIs into npm packages;
- it does not kill running processes or delete a generation that `/proc`
  proves a live process still executes from; and
- `ynpx` does not mask a reachable-but-broken package as an offline cache hit;
  it only falls back when the network/registry is unavailable. Flags after the
  package name are passed verbatim to the launched bin.
