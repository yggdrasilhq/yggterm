# ynpm - the yggdrasilhq package manager

ynpm is the ONE command that keeps every `@ygghq` binary current on every
fleet host: install, drift-watching, generations with rollback. It ships
with yggterm (deployed by `scripts/deploy-dev.sh` to the same canonical
paths as `yggterm` and `yggterm-headless`) and runs anywhere a shell runs:
no daemon, no GUI, no npm required.

## How delivery works

Fleet binaries are published to the public npm registry under the `@ygghq`
scope, one platform package per machine shape (`@ygghq/ychrome-linux-x64`,
`@ygghq/ychrome-linux-arm64`; linux only, honestly - WebKitGTK does not
exist elsewhere). ynpm resolves the package, downloads the platform tarball,
and installs the binaries it declares:

```sh
ynpm install ychrome          # @ygghq/ychrome, latest
ynpm install ychrome@0.2.1    # pinned
ynpm list                     # installed, with what each binary self-reports
ynpm check                    # disk vs state vs registry latest (the drift instrument)
ynpm sync                     # install the registry latest of everything installed
ynpm rollback ychrome         # the previous generation back into place
```

A bare name means `@ygghq/<name>`; the scope is the registry identity of the
fleet. Destinations: binaries land in `~/.local/bin` (override with
`YNPM_DEST`); generations are kept under `~/.yggterm/ynpm/generations/`;
state lives in `~/.yggterm/ynpm/state.json`.

## The three rules (each one earned by an incident)

1. **A binary must tell the truth about itself.** Before anything is
   swapped, every freshly downloaded binary is run with `--version` and the
   answer must name the package's version. A release whose npm package was
   bumped while its crate was not shipped binaries that answered with the
   PREVIOUS version; every host "updated" and still showed the old number.
   ynpm refuses that install.
2. **Swap by rename.** A running binary cannot be written ("Text file
   busy"), but a rename over its directory entry always works: the running
   process keeps its inode, the next launch picks up the new build.
3. **Every install keeps a generation.** Rollback restores real bytes that
   were verified when they were installed, never a guess.

## The fleet sweep (standing rule)

After any `@ygghq` release, roll EVERY fleet host the same day:

```sh
for h in host-a host-b host-c; do ssh $h 'ynpm check && ynpm sync'; done
```

`ynpm check` is the pre-flight (is anything drifted?), `ynpm sync` is the
sweep. Daemon-backed apps (ychrome and friends) also want their daemon
handed over after a binary change (`<app> daemon restart`) - the swap is
safe under a running process, but the running process serves the old build
until it is restarted.

## Releasing a new version of an @ygghq package

Bump the **crate's** `Cargo.toml` version and nothing else - it is the
single source of truth (it is what the binary prints). The publish workflow
(`ynpm-publish.yml`) derives the npm version from Cargo.toml, stamps
`package.json` and the platform pins from it, and publishes on a `v*` tag.

## Scope

ynpm manages `@ygghq` packages only. It is not a general npm client: no
transitive dependencies, no lockfiles, no lifecycle scripts - the platform
package's `bin` table is the whole contract, and each binary is verified
before it is trusted.
