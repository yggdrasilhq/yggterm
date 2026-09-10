---
name: ynpm
description: Use ynpm for yggterm-aware distribution of agent CLIs, TUIs, libyggterm apps, and other executable packages across the recognized SSH fleet. Covers integrated inventory, atomic generations, dev-first builds, fleet import, offline ynpx, and safe legacy-package cleanup.
---

# ynpm — the executable distribution plane

Use `ynpm` whenever the work distributes an executable across the yggterm
fleet. The recognized fleet is localhost plus the machines in the permanent
yggterm SSH roster. The server owns PTYs and launch composition; ynpm owns
package identity, downloads, lifecycle finalization, generations, cache,
publication, rollback, and fleet import.

## Entry gate

Before touching a fleet package, read `docs/ynpm.md` and establish the real
subject:

```sh
ynpm list
ynpm check
ynpm doctor
```

`ynpm list` is the integrated inventory, not merely a dump of
`~/.yggterm/ynpm/state.json`. It derives all registered agent CLIs from
yggterm-core's `AGENT_CLIS` table and reads the host's
`~/.yggterm/apps/*.json` app manifests. It reports the measured executable
version, source, and path. A `not-installed` row is different from an
unreachable or unverified path.

Do not infer inventory from a single `PATH` in a non-login shell. The launch
path and the package-manager path are separate observations; use the path
reported by `ynpm list` and the descriptor that owns the CLI.

## Production operations

```sh
ynpm install <package>[@<version-or-tag>]
ynpm sync
ynpm sync --integrated
ynpm sync-fleet --hosts machine-a,machine-b --integrated
ynpm export <package> --metadata --archive <disk-backed-path>  # peer protocol
ynpm rollback <package>
ynpm remove <package>
ynpm purge-legacy
```

The generic npm path resolves a tag to an exact version, installs one package
into an isolated staging prefix, enables only that package's declared
lifecycle scripts, runs every declared bin's `--version` gate, and publishes
the resulting generation with a symlink-then-rename. There is no shared
`npm install -g` batch and no `--force`. A failed package cannot unlink its
neighbours.

First-party `@ygghq/<name>` packages retain their platform-package contract
when the main package declares the matching host companion in
`optionalDependencies`: the main package and exact platform package must agree
on version, and every platform bin must pass the gate before publication. A
portable `@ygghq` package with its own `bin` table uses the generic npm path;
the scope alone never invents a native companion.

The OpenCode v2 pin is `@opencode-ai/cli@beta`, binary `opencode2`. The
unscoped `opencode-ai@beta` package is the abandoned v1 line. Never repair an
opencode2 problem by installing the similarly named v1 package.

`sync-fleet --integrated` asks each named peer for its highest verified
production generation before consulting the registry. It reuses that peer
archive when it is ahead, then archives each final local production generation
once and imports it on every named host. This is the download-once path. The
remote host updates its own state and repeats the bin gate. A successful `scp`
or SSH request is not the effect proof; run `ynpm list` and `ynpm check` on the
target. A same-version archive still replaces changed four-product bytes by
atomic rename, allowing release-only polish to reach an existing generation;
the syncing manager is reasserted to both `ynpm` and `ynpx` aliases after the
import. `export` is the machine-readable peer protocol; it refuses dev trees.

Run `ynpm doctor` on a new host before a fleet sync. It resolves
`HOME`/`USERPROFILE`, expands `~`, reports absolute manager/destination/scratch
roots, checks `curl`, `tar`, and `npm`, and identifies whether the host has a
published native target. A generic portable package can still use the npm path
on a host without a native first-party matrix.

The three non-npm integrated sources (uv, vendor installer, and manual/self
updater) stay visible in the inventory with their source. Do not relabel them
as npm packages or delete system-owned files to make the inventory look
uniform.

## Dev-first workflow

For a new CLI/TUI/libyggterm app, use a dev generation first:

```sh
ynpm dev --build '<build command>' \
  --bin tool=dist/tool \
  --watch @scope/tool \
  --fleet machine-a,machine-b \
  /home/user/gh/tool
```

If the checkout has `package.json`, put the recipe in its `ynpm` object:

```json
{
  "ynpm": {
    "build": "bun build --compile src/main.ts --outfile dist/tool",
    "bins": { "tool": "dist/tool" },
    "watch": "@scope/tool"
  }
}
```

Cargo workspaces with multiple bins should pass explicit `--build` and
`--bin` values. Already-built artifacts use:

```sh
ynpm install --dev @scope/tool --watch @scope/tool \
  --dest /home/user/.local/bin --bin tool=dist/tool
```

For an integrated npm CLI, omit `--dest` so the dev generation is published
to `~/.yggterm/ynpm/bin`, the same directory the server launches. Older dev
states that were published to `~/.local/bin` are bridged into that agent
directory by `ynpm sync --integrated` without discarding the original link.

The dev marker records commit, build time, builder host, watched production
package, superseded production version/fingerprint, and the dev fingerprint.
The registry is never changed by a dev install.

`ynpm sync` keeps the dev generation until the watched production version is
strictly newer, or the same version has a different known production
fingerprint. Unknown registry data keeps the local dev build. `ynpm prod`
is the explicit immediate handback. This is the intended DX: one build, one
health check, optional fleet import, and no manual copy/repoint ritual.

Yggterm itself is an ynpm-managed product release. `ynpm self-update --json`
downloads and verifies the four-product release generation (`yggterm`,
`yggterm-headless`, `ynpm`, `ynpx`) and advances the direct-install state. A
newer production release takes over a dev build; a same-version release takes
over only when its verified binary fingerprint differs. The GUI delegates its
update check to this verb and owns only the notification and graceful,
PTY-preserving restart. `ynpm sync` invokes the same self-update path.

The first self-update also repairs old compatibility state. A historical
fleet deploy may have left `fleet-deploy`/`fleet` as transport markers beside
the active binary; ynpm maps those markers to the canonical yggterm GitHub
release and rewrites the active version from the binary's `--version` answer.
Never turn those legacy values into a release URL or hand-copy a replacement.
The canonical direct install state's active executable also wins over an older
production `versions/<version>` directory, so a dev generation remains the
reported and launched product until production handback.

Every installable libyggterm app release must carry the same `package.json`
metadata inside its npm package, GitHub/Forgejo archive, or local checkout.
`package.json.yggterm.schema` is `1`; `yggterm.app.binary` names a bin key;
`context_menu` is explicit, and `row_spawn` controls whether a verb belongs in
a row menu. The package manager projects this into an absolute host manifest;
the app must not write one at startup.

`ynpx` source forms are `@scope/name`, `github:owner/name`,
`forgejo:https://host/owner/name`, `tarball:https://host/app.tgz#sha256=…`,
and `--dev /absolute/checkout`. They all use the same metadata, version gate,
generation, and publication path. Release metadata and binaries are refused
before a published link changes when the package file, checksum, or bin gate
is invalid.

For a new libyggterm app, declare the stable `package.json.yggterm.app` block
documented in `docs/ynpm.md`. `binary` is a package bin key, not a path;
`context_menu` is the explicit opt-in for workspace/session row menus. App
processes must not write `~/.yggterm/apps` on first use, and the daemon must
not delete a registration during a scan. ynpm writes/removes the normalized
absolute-path manifest during install, upgrade, dev publication, import, and
remove.

## ynpx

`ynpx` is the npx-shaped launch surface:

```sh
ynpx @scope/tool --flag value
ynpx github:owner/tool --help
ynpx --dev /home/user/gh/tool -- --profile demo
```

It attempts the newest install/update on every invocation. If the network is
unavailable, it launches the last verified generation; a reachable package
whose install or lifecycle is broken fails loudly rather than being mistaken
for offline mode. Arguments after the package belong to the selected bin;
`--bin` and `--dev` are controls only before the package name and are app flags
after it.
GitHub sources are cloned or fast-forwarded under ynpm's own state root and
then use the same checkout recipe and `--version` gate.

Operations append structured `component: "ynpm"`, `category: "distribution"`
events to `~/.yggterm/event-trace.jsonl`, correlated by `run_id`. Read these
events when a fetch, metadata parse, version gate, generation swap, app
registration, fleet import, offline fallback, or launch fails. Human output is
not the effect proof, and raw application flags or credential query strings do
not belong in trace payloads. `--json` verbs keep stdout to one JSON result;
repair notices and diagnostics go to stderr.

## Server integration and flags

The managed-CLI server path must call ynpm for npm-backed tools and publish
them into `~/.yggterm/ynpm/bin`. It must not create or refresh a second
`~/.yggterm/npm` generation tree. The server prepends the ynpm bin directory
to the CLI launch `PATH`; the process's `/proc/<pid>/exe` and helper paths are
the proof of lineage.

The settings modal and launch builder share the per-CLI descriptor table.
Configured extra args cross SSH as a per-launch request field. For Codex,
`-s danger-full-access` disables the sandbox but still asks for approvals;
`--dangerously-bypass-approvals-and-sandbox` is the actual visible YOLO
posture. The modal labels that exact tier `YOLO: skip checks and prompts`.
The same forwarded setting is used when a daemon rebuilds a remote resume or
picker command, not just for a fresh start. No YOLO warning is therefore not,
by itself, evidence of dropped flags. Check the composed command and the
remote process command line.

## Cleanup safety

Legacy cleanup is allowed only after the new ynpm publication is measured on
the target host. Identify running processes by `/proc/<pid>/exe` and retain
any generation that a live process or helper executes from. Remove only exact
ynpm-owned paths; never purge `/usr`, `/usr/local`, a user-owned unrelated
package tree, or a row the current agent did not create. A residual live
legacy generation is an honest migration remainder and must be reported.

For repository work, use a worktree lane and the fleet's `ygg-ci` build plane.
Do not build a release in a shared worktree with bare `cargo build` when the
result will replace live yggterm binaries.
