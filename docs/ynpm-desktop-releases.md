# ynpm desktop & porcelain GitHub releases — the release-manifest contract

**Status:** implemented in `crates/ynpm` (schema + install transaction);
the release/asset pipeline that GENERATES these manifests is owned by the
parent integration lane. **Owner:** the ynpm distribution lane (4.0 wave).

This document specifies the versioned GitHub-release package contract that
lets `ygg ynpm https://github.com/yggdrasilhq/yggterm` install the optional
desktop GUI (or any other porcelain) after a curl/iex seed. The seed itself
installs no GUI; the GUI is one optional client over the same daemon,
identity and memory contracts, never a lifetime owner of the installation.

The contract is generic: ONE schema serves the CLI seed and every optional
porcelain (desktop, web, mobile), with separate package identity, separate
ownership, and separate install policy per role.

## 1. The release manifest

Every GitHub release that ynpm can install publishes a machine-readable
`release-manifest.json` asset (schema 1). Shape reference:
`crates/ynpm/fixtures/release-manifest.example.json`. Validation lives in
`crates/ynpm/src/release_contract.rs`; the install transaction in
`crates/ynpm/src/github_release.rs`.

```json
{
  "schema": 1,
  "release": { "version": "4.0.0", "tag": "v4.0.0" },
  "packages": [
    {
      "id": "yggterm-gui",
      "role": "desktop",
      "summary": "yggterm desktop client (optional porcelain)",
      "artifacts": [
        {
          "platform": "linux-x64",
          "format": "tar.gz",
          "filename": "yggterm-gui-4.0.0-linux-x86_64.tar.gz",
          "sha256": "<exact 64-hex digest of this artifact>",
          "size": 18453248,
          "bins": [],
          "bundle": "yggterm"
        }
      ],
      "version_probe": { "file": "yggterm/version" },
      "launch": { "entry": "yggterm/yggterm" },
      "app": {
        "comment": "optional yggterm.app block, package.json.yggterm.app shape"
      }
    }
  ]
}
```

### 1.1 Hard rules (all enforced by validation, all refusals loud)

1. `schema` MUST be `1`. A newer schema is refused naming both numbers —
   never guessed at.
2. Every artifact MUST carry an exact lowercase `sha256` (64 hex). An
   artifact without a digest is refused at validation: "add checksums
   later" is how a supply-chain gap ships.
3. `size` (optional, bytes) when declared MUST match the staged archive
   exactly. A mismatch is pipeline drift and refuses the install.
4. Artifact `filename` MUST be a flat safe name (`[A-Za-z0-9._-]`, no
   separators, no whitespace/control): filenames are joined into scratch
   paths verbatim, so `../ynpm` or `a\b` is refused before any path is
   built.
5. `platform` MUST be one of `linux-x64`, `linux-arm64`, `macos-x64`,
   `macos-arm64`, `windows-x64`, `windows-arm64`, `android-arm64`,
   `ios-arm64`. A typo'd platform invalidates the manifest rather than
   silently installing nothing.
6. Package `id` is 1–64 chars of `[a-z0-9._-]`, unique within the release.
7. One artifact per platform per package; `format` is `tar.gz`, `zip`
   (installable), or `apk`/`ipa` (mobile handoff only — NEVER extracted,
   NEVER adb).
8. Content paths (`bundle`, `version_probe.file`, `launch.entry`) MUST be
   relative forward-slash paths with no `..`, no absolute, no backslash.
9. A release tag is a safe git tag; the manifest's `release.tag` MUST
   agree with the tag the release actually resolved to.

### 1.2 Roles and install policy

| role    | who installs it | what ynpm does |
|---------|-----------------|----------------|
| `seed`  | the installer via `ynpm seed-activate` (see §5) | verifies, gates, publishes the generation, flips `current`; alias/PATH wiring stays with the installer |
| `desktop` | `ygg ynpm install <repo-url>` | verifies, gates, publishes generation + destination links |
| `web`   | the yggterm web plane | validated for shape; `install` declines naming the owning plane |
| `mobile`| the official package channel (store / Obtainium) | prints the `handoff` URL and installs NOTHING; adb is never part of any path |

### 1.3 Version verification is mandatory for installed roles

- Every declared `bins` entry must exist in the archive and answer
  `--version` with a string CONTAINING the release version before anything
  is published.
- A `desktop`/`seed` artifact with no bins MUST ship a `version_probe`:
  a file inside the archive whose bytes contain the release version.
  Pure bundles have no bin to interrogate; the probe is the contract the
  release pipeline satisfies (e.g. ship `yggterm/version` inside the
  bundle).
- `web` and `mobile` artifacts are shape-validated only; their
  verification belongs to the plane that delivers them.

## 2. The install transaction

`ygg ynpm install https://github.com/<owner>/<repo>[@tag]` (the
`github:owner/repo` shorthand also works; `--package <id>` selects one
package, `--dest <dir>` overrides the publication destination):

1. Resolve the release: explicit tag, else the `releases/latest` redirect.
2. Fetch and validate `release-manifest.json` from that tag's assets; the
   manifest tag must equal the resolved tag.
3. Select packages: `--package <id>`, else the single seed/desktop
   package carrying this host's platform. Zero candidates → an honest
   refusal listing what the release DOES ship per platform. Multiple → a
   refusal asking for `--package`.
4. Download the selected artifact(s) to disk-backed scratch
   (`~/.yggterm/scratchpad/ynpm`, never /tmp).
5. Verify the exact sha256 (and declared size) BEFORE extraction.
6. Extract behind the guards (§3).
7. Gate every bin's `--version`; check bundle presence, launch-entry
   existence and exec bit, version-probe content.
8. Stage the whole tree into `generations/<key>/.release-…-<pid>/`,
   rename it to `generations/<key>/<version>/`, publish bin and bundle
   links into the destination by staged-symlink rename, then update state.
9. Any failure before the state write leaves the previous generation, its
   links, and the state file exactly as they were; staging is removed.

Upgrade semantics match the registry channel: a newer version publishes a
new generation and repoints links, the old generation stays for
`ynpm rollback <id>`; an implicit "latest" never moves backwards (an
explicit tag is operator intent); same-version reinstall is idempotent.
`ynpm remove <id>` removes only this package's links and generations and
refuses while a live process executes from the generation tree —
neighbouring and shared-seed artifacts are untouched.

Package identity: the release package `id` is the ynpm state identity
(storage key `package_storage_key(id)`, `package_name = id`), so the
generic `list`, `rollback`, `remove` verbs address release packages by
their manifest id. The `channel` records `github-release` and `source`
records `github:<owner>/<repo>@<tag>`.

## 3. Archive safety guards

Refusals happen BEFORE extraction, deterministically, from the listing:

- entry-count cap (100,000) and per-entry size cap (512 MiB) and total
  decompressed cap (1 GiB); compressed archive cap 512 MiB;
- names: no absolute paths, no `..` components, no backslashes, no
  control characters, no Windows drive spelling;
- symlinks: only relative targets that stay lexically inside the
  extraction root (a link is kept or refused as spelled, never resolved
  against the live filesystem);
- hard links and exotic entries: refused outright — release archives
  carry plain files, directories and relative symlinks only;
- an archive whose `tar -tvzf` listing cannot be parsed line-for-line is
  REFUSED (GNU tar fleet-wide; bsdtar line shapes accepted for macOS) —
  a listing we cannot read is a listing we cannot guard;
- zip artifacts: `unzip -l` listing guards, extraction requires `unzip`,
  and any symlink appearing on disk after extraction is a refusal (a zip
  listing cannot name links); empty archives are refused.

This lane enforces the guards against REAL tar fixtures built in-test
(traversal names, absolute paths, oversized entries, hardlinks, wrong
`--version` answers, bad digests). It does NOT claim a fuzzed-archiver
qualification; hostile-archive handling beyond the guards above is a
residual, stated, not assumed-away.

## 4. Platform capability matrix (honest)

| platform | declared | exercised by this lane |
|---|---|---|
| linux-x64 / linux-arm64 tar.gz | yes | YES — full transaction tested with real tar fixtures |
| macos-x64 / macos-arm64 tar.gz (`.app` bundle) | yes | schema + listing parser tested (bsdtar line shapes, bundle checks); LIVE macOS installation is NOT tested from this Linux lane |
| windows-x64 / windows-arm64 zip | yes (schema) | schema validation + `unzip -l` guard tested; live Windows install untested; bundle publication on Windows is explicitly UNSHIPPED (needs symlink support) |
| android-arm64 apk / ios-arm64 ipa | handoff only | never installed; handoff message tested |

macOS/Windows GUI packaging capability is therefore DECLARED and
schema-tested, and explicitly NOT claimed as live-tested from Linux.
Platform-native qualification belongs to the parent release matrix.

## 5. Seed activation — the single transaction owner (draft for parent)

The parent seed proposal asks that shell/PowerShell installers not
duplicate generation policy. `ynpm seed-activate` is the proposed API:

```sh
ynpm seed-activate --archive <seed.tar.gz> --manifest <release-manifest.json> \
    [--seed-root ~/.yggterm/seed] [--expect-version 4.0.0] [--package <id>] [--json]
```

The installer downloads the two assets and hands ONE transaction to ynpm:
validate manifest (schema + seed package), verify the archive's exact
sha256, extract behind the guards, gate the bins (`ygg`, `ynpm`, `ynpx`
must answer the release version), publish
`<seed-root>/generations/<version>-<digest12>/`, then atomically flip
`<seed-root>/current` (staged-symlink rename). `--expect-version` refuses
a manifest that is not the version the installer was told it fetched.
Re-running with the same version+digest is an idempotent `already-active`.
Any failure before the flip leaves the previous generation in charge.
Alias/PATH wiring (`~/.local/bin`) stays with the installer, which
receives the generation path (human or `--json` output).

This matches the parent proposal's
`~/.yggterm/seed/generations/<version>-<digest>` layout with one
difference to confirm: the generation suffix here is `<digest12>` (first
12 hex of the ARCHIVE digest), not a full digest — the full digest lives
in the manifest and the active.json-style trace. Parent decides the final
layout before the installers are written; this verb is the transaction
owner either way.

**Integration request (parent):** confirm the seed-root layout and alias
policy, then call `seed-activate` from both the POSIX and PowerShell
installers instead of reimplementing verification/flip logic there.

## 6. The legacy four-product coupling — audit

Today `ynpm self-update` owns the yggterm direct install: one
`yggterm-<label>.tar.gz` release carries FOUR products (`yggterm`,
`yggterm-headless`, `ynpm`, `ynpx`) into one `versions/<version>`
generation, gated and swapped together
(`install_yggterm_release` in `crates/ynpm/src/main.rs`). Findings:

1. **The GUI binary rides the seed release.** Under the 4.0 boundary the
   seed must not ship a GUI. The legacy path keeps working until the new
   seed pipeline replaces it, but no NEW coupling may be added to it.
2. **Ownership split:** `self-update` remains the owner of the legacy
   four-product direct install (state `~/.yggterm/install-state.json`,
   `ynpm`/`ynpx` aux links). Release-manifest packages are a SEPARATE
   identity space (`channel: github-release`, manifest ids) — they never
   write direct-install state and never touch the legacy generation.
   Removing a release-managed desktop package must not erase the seed.
3. **Migration:** the parent's new seed release should adopt THIS
   manifest schema (`role: "seed"`) so one code path owns generation
   policy, and retire the four-product path once the seed pipeline is
   qualified. Until then both channels coexist; `ygg ynpm install
   <repo>` never invokes `self-update`.
4. **The dev channel is untouched:** `ynpm dev`/`prod`/`sync` handback
   semantics apply to registry packages and the legacy yggterm entry;
   release-manifest packages have no dev channel in this lane (a local
   desktop build is a `dev` concern for its own lane, not this
   transaction).

## 7. What this lane does NOT own

- the release pipeline that builds artifacts and generates/publishes
  `release-manifest.json` (parent integration);
- desktop integration beyond publishing the generation and links
  (.desktop files, icons, launch registration) — daemon/GUI lanes;
- web-plane delivery of `web` packages; mobile distribution beyond the
  handoff URL;
- installer alias/PATH wiring (parent, on top of `seed-activate`);
- the legacy four-product self-update behaviour itself (audited only).

## 8. Not covered by this specification

This document does not claim that a 4.0.0 release exists, that any
GitHub release currently publishes a manifest, or that desktop/mobile
artifacts are buildable today. It specifies the contract and the local
transaction only. Live network resolution (`releases/latest` redirect,
asset fetch) is implemented behind the same `curl` substrate as the
existing channels and awaits the parent's real smoke validation; no
network call is exercised by this lane's tests.
