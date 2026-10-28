# yggterm 4 — installation (ygg seed)

The ygg4 **seed** is the GUI-free native package: the `ygg` frontend, the
`ynpm` manager (`ynpx` is the same product in launch mode), and the dispersed
payload under `share/ygg/` — `commands.json` at the payload root, fleet
components under `components/` (with `commands.json` kept beside
`ygg_command_tree.py`), and the kernel under `kernel/`. Everything an
installed generation needs is inside the generation; it never reaches back
into a repository.

## Bootstrap a fresh host

Unix (Linux, macOS) — the seed is the DEFAULT entry; the 3.x channel is
explicit:

```sh
curl -fsSL https://raw.githubusercontent.com/yggdrasilhq/yggterm/main/scripts/install.sh -o install.sh
sh install.sh                            # seed install (the default)
YGGTERM_VERSION=vX.Y.Z sh install.sh     # pin an exact release
YGGTERM_LEGACY=1 sh install.sh           # explicit 3.x direct/GUI channel
                                         #   (same as: sh install.sh legacy)
```

Windows (PowerShell, no Python and no GUI required) — same default:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/yggdrasilhq/yggterm/main/scripts/install.ps1 -OutFile install.ps1
./install.ps1                            # seed install (the default; -Seed also selects it)
$env:YGGTERM_VERSION = "vX.Y.Z"; ./install.ps1   # pin an exact release
./install.ps1 -Legacy                    # explicit 3.x direct/GUI channel
```

Hosts that already run `ygg` skip the bootstrap script and use the manager
directly (this is the README's primary flow — the GitHub URL is the TARGET):

```sh
ygg ynpm install https://github.com/yggdrasilhq/yggterm
```

## What the installer guarantees

- **`ynpm seed-activate` is the single transaction owner.** The installer
  only fetches bytes and publishes command links; selection, digest
  verification, extraction, staging, publication and rollback stay inside
  one `ynpm seed-activate` call.
- **Checksums are mandatory.** A missing or mismatched `.sha256` fails the
  install; nothing is activated unverified.
- **Exact versions.** The installer passes `--expect-version`; a manifest
  that disagrees with the requested release refuses the transaction.
- **Staged before activation.** A generation is built under
  `~/.yggterm/seed/generations/<version>-<digest12>` and only then does the
  `current` symlink flip atomically. A failure leaves the previous
  generation in charge.
- **One launch pointer.** Native ynpm is the only writer of
  `~/.yggterm/seed/active.json` (schema 1: generation name plus exact bin
  sha256 digests), published whole by temp+rename. The bootstrap launchers
  (`ygg --seed-launch`, mirrors under `launchers/`) read that pointer,
  enforce the hash gate and refuse self-recursion, and resolve BEFORE any
  installed command IR — an older launcher forwards to a newer generation
  it has never seen. See `crates/ygg-cli/src/seed_launcher.rs`.
- **Rollback.** Previous generations are never deleted; re-activate the
  previous seed archive (or `ynpm rollback`) to go back.
- **Windows running binaries.** Publication copies are staged and swapped;
  a locked, running `ygg.exe` fails the copy loudly and the activated
  generation is never touched — close the process and rerun.
- **GUI and worker safety.** The seed never touches a 3.x direct/GUI
  install root, never runs `install integrate`, never starts a daemon, and
  never alters protected workers or native config under `~/.yggterm`.

## PATH ownership

The seed owns exactly three links: `~/.local/bin/{ygg,ynpm,ynpx}` on Unix
(`%LOCALAPPDATA%\ygg\bin` on Windows, added to the user PATH). If a
non-seed `ygg` command already exists — for example a dispersed native
tree — the installer leaves it in place and says so; takeover requires the
explicit `YGG_MIGRATE_NATIVE=1` (Unix) rerun. Dispersal tooling must not
overwrite the seed's links either: one owner per link.

## Remote operator vs local worker readiness

A seed install makes a host operable **as a ygg system over ssh**: an
operator (human or agent seat) can reach it and run `ygg` verbs remotely.
That capability is distinct from **local worker readiness** — the swarm
worker configuration, agent CLIs and harness state a fully enrolled worker
needs are NOT installed or implied by the seed, and the seed manifest says
so (`worker_readiness: not implied by seed installation`).

## Release artifacts

- `ygg-seed-<label>.tar.gz` / `.zip` — the seed archive (bins at the
  archive root: `ygg`, `ynpm`, `ynpx`; payload under `share/ygg/`).
- `ygg-seed-<label>.tar.gz.sha256` — mandatory digest sidecar.
- `release-manifest.json` — the schema-1 manifest `ynpm seed-activate`
  trusts; the `ygg-seed` package (`role: "seed"`) is kept separate from the
  `desktop` packages.

## Qualification status (honest)

- Advertised seed targets today: **linux-x64** and **windows-x64** — the
  release workflow builds them natively, runs the native version gates, and
  completes a REAL fresh-install smoke (the packaged `ynpm` executes
  `seed-activate` on the runner) before the publish predicate lets
  `release-manifest.json` attach.
- arm64 and macOS seed targets are **not advertised**: a cross-compiled
  binary cannot pass its own version gate or a fresh-install smoke on the
  CI runner, and GUI/deb legs are never accepted as a proxy for either.
- This document describes the pipeline as built; the 4.0 seed release
  itself is not yet published and no 4.0 tag has been cut. Until the
  release legs run green on the real artifacts, treat every fixture-based
  local pass as qualification evidence, not live proof.
