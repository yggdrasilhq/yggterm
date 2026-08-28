# ynpm — the yggdrasilhq package manager

**Status:** spec · **Owner:** the yggterm campaign · **Scope:** fleet-wide binary
delivery over the npm registry, for first-party libyggterm binaries and every
future yggdrasilhq tool.

---

## 1. Why

yggterm already runs a package manager for agent CLIs: a provisioner that
installs, generations with atomic publication, a runs-before-publish gate, an
aggressive TTL auto-update sweep, fleet fan-out, and a single-writer install
lock. That machinery exists and is measured. What it lacks is a *face a shell
can call* and a *channel for our own binaries*.

The npm registry is the delivery channel: it is content-addressed, CDN-cached
worldwide, has integrity hashes on every tarball, supports per-platform
optional dependencies (the exact shape the fleet's own CLIs already arrive
in), and runs identically on Linux, macOS, Windows, FreeBSD and Illumos.
Publishing once to npm means every fleet machine pulls from the CDN instead of
each one building, scp'ing, or re-downloading from a vendor URL.

So: **ynpm** — a `ynpm` binary (a workspace bin of yggterm, sharing
`yggterm-server`'s provisioner) that manages two classes of packages:

| | agent CLIs (existing) | **ygghq packages (new)** |
|---|---|---|
| examples | codex, claude, qwen, … | @ygghq/ytop, @ygghq/ychrome, @ygghq/yedit, @ygghq/kasten, @ygghq/ychrome-vault |
| source | third-party vendors | yggdrasilhq repos, built by their release workflow |
| published by | the vendor | `ynpm publish` from a repo checkout, or the repo's GitHub release workflow |
| install location | `~/.yggterm/npm/bin` (managed dir, NOT on PATH; launch prepends it) | **the user PATH** (`~/.local/bin`) — the one delta: these are tools a human and every other program invokes directly |
| consumers | yggterm launches | anything: the user's shell, scripts, yggterm, cron |

Everything else — generations, locks, gates, sweeps, pruning — is shared.

## 2. Package shape (the shape the fleet already finalizes)

A ynpm package is the **claude/opencode shape**, which the direct fetcher has
finalized, health-checked and fleet-proven:

```
@ygghq/ytop                     main package
├── package.json
│   ├── version: 0.1.0                    (mirrors Cargo.toml; the release
│   │                                      workflow derives it from the tag)
│   ├── bin: { "ytop": "bin/ytop" }       the entry shim
│   ├── optionalDependencies:             EXACT pins, one per platform
│   │   { "@ygghq/ytop-linux-x64":  "0.1.0",
│   │     "@ygghq/ytop-linux-arm64":"0.1.0",
│   │     "@ygghq/ytop-darwin-x64": "0.1.0",
│   │     "@ygghq/ytop-darwin-arm64":"0.1.0",
│   │     "@ygghq/ytop-win32-x64":  "0.1.0",
│   │     "@ygghq/ytop-win32-arm64":"0.1.0" }
│   └── scripts: { "postinstall": "node ./finalize.mjs" }
└── finalize.mjs                copies the platform package's native binary
                                over `bin/ytop`, chmod 755 — then exits 0
                                only if `--version` runs

@ygghq/ytop-linux-x64           platform package: the built ELF + bin/ytop
```

Why this shape and not a single fat package: npm has no per-platform file
selection, every host would download every target's binary, and a broken
platform build could not be patched independently. This is what claude,
opencode and grok do; yggterm's fetcher already walks `optionalDependencies`,
runs the finalize step, and refuses to publish a binary that does not run.

### Package kinds

| kind | applies | delivery |
|---|---|---|
| `native` | ytop, ychrome, ychrome-vault, yedit, kasten | the shape above |
| `script` | single-file entry points | the bin IS the package file; no platform deps |
| `git` | yRDP (deliberately a git-checkout tool: "no install step on purpose — `git pull` updates it") | the package ships a launcher that clones/pulls `~/gh`-style checkout under `~/.local/share/ynpm/<name>` and execs its entry — preserving the pull-to-update design while making it ynpm-installable |

## 3. Install location: the user path, with generations underneath

```
~/.local/bin/ytop                     → symlink → generation (below)
~/.yggterm/ynpm/cli/ytop.gen7/        one generation: the unpacked package
~/.yggterm/ynpm/cli/ytop.gen8/        the new generation, verified, then
                                      atomically renamed over the symlink
```

- `~/.local/bin` is on every login PATH (it is where the user's own tools
  already live: claude, cargo, elan…). That is the whole delta from the agent
  CLI flow: the published symlink lands in the user path instead of the
  managed dir.
- Generations are the SAME layout the agent CLI provisioner uses, for the
  SAME reasons: atomic rename (no half-installed window), rollback
  (`ynpm rollback ytop` repoints the symlink one generation back), and the
  **liveness-aware prune** — a generation a running process executes from is
  deferred to a later sweep, never deleted underneath ychrome mid-session
  (measured: a pruned generation breaks every lazy helper spawn of the
  running binary).
- The **runs-before-publish gate** applies at install too: a finalize that
  leaves a shim that cannot `--version` never becomes the live binary.

## 4. ynpm the binary

A workspace bin of yggterm (`crates/yggterm-server/src/bin/ynpm.rs`), sharing
the provisioner crate — one install lock, one TTL sweep, one npm cache, one
fetcher, one gate. Nothing re-implemented.

```
ynpm install [@ygghq/<name>[@version|@tag]]   resolve → fetch → finalize →
                                              gate → publish symlink → prune
ynpm remove <name>                            reverse (user-local only; a
                                              system path is refused by name)
ynpm update [name]                            targeted foreground refresh
ynpm list [--outdated]                        installed, versions, sources
ynpm rollback <name>                          repoint the symlink one generation
ynpm sync-fleet                               one-shot: every connected machine
                                              installs the same versions (the
                                              fleet sweep's ynpm arm)
ynpm publish [repo]                           from a checkout: build matrix →
                                              pack platform packages → publish
                                              main + platform (needs the
                                              publish token, §6)
ynpm doctor                                   lock, PATH, tokens, registry,
                                              per-package health
```

Default invocation with no arguments = `ynpm update` over everything (the
aggressive auto-updater, the CLI sweep's sibling).

## 5. Auto-update and the fleet

- **TTL sweep:** the same scheduled refresh that keeps agent CLIs current
  gains the ynpm set. Installed once, then checked on the sweep cadence with
  per-package jitter; a new version is installed generationally and the
  symlink atomically republished. Running processes keep their old inode
  until they exit (the generation layout's existing property).
- **Fleet one-shot:** `ynpm publish` puts a version on the registry once;
  `ynpm sync-fleet` walks the connected machines and installs it everywhere
  from the CDN. No more per-host build-and-scp for these tools.
- **The fetcher is the fleet's own:** registry tarball fetch with integrity
  (npm), optional-dependency resolution, finalize script, runs-gate. The
  npmjs outage path is the npm cache; no external vendor URLs.

## 6. Publishing, auth, and the org

- **Registry:** npmjs.com, public packages, scope **@ygghq**. Public is what
  makes the fleet work: any machine installs with zero auth; only publishing
  authenticates. (GitHub Packages was rejected: even public packages need an
  authenticated install there, which breaks fleet machines without tokens.)
- **The org** `@ygghq` on npmjs is created once by the owner (browser +
  passkey). Publishing authenticates with a **granular access token** scoped
  to `@ygghq/*`, packages read/write, no account-wide rights.
- **Local publish:** `ynpm publish` reads the token from **ychrome-vault**
  (`www.npmjs.com` item, custom field `publish_token`) and writes it to the
  user `~/.npmrc` for the call — the password/passkey never leave the vault,
  and the token is vault-managed, revocable, and rotatable.
- **CI publish:** each repo's release workflow (tag `v*`) builds the platform
  matrix on GitHub runners, packs the tarballs, and publishes with the
  `NPM_TOKEN` repository secret (the same granular token). Nothing long-lived
  in the repo; rotation is one vault edit + one secret update.
- **Provenance:** `npm publish --provenance` ties the published package to
  the yggdrasilhq repo and the exact workflow run.

## 7. Security posture

- User-local only, never sudo, never system paths (removal refuses them by
  path, exactly like the CLI provisioner's guard).
- The postinstall is **our own first-party finalize** — the same trust level
  as building the repo from source — and it runs under the same boundaries as
  every vendor script: HOME intact, stdin closed, TMPDIR on disk, bounded,
  exit-checked.
- The runs-gate means a tampered or broken artifact cannot become the live
  binary silently.
- Tokens: vault for humans/agents, GitHub secrets for CI, granular scope,
  rotatable, never in a repo file.
- The npm cache is the existing shared, GC'd cache (`npm cache verify`
  retention — measured 1.65 GB reclaimed on the build host).

## 8. Migration (deleting the custom shipping)

Today these binaries reach hosts by hand: each repo's install step copies its
`target/release/<bin>` into `~/.local/bin`, and agents scp them during
deploys. That is the ad-hoc chore ynpm retires:

1. `ynpm publish` lands the current versions under @ygghq (one-time, per repo).
2. `ynpm sync-fleet` installs them on every connected machine.
3. Each repo's install step is replaced by the release workflow (tag →
   publish); the local install instruction becomes `ynpm install @ygghq/<name>`.
4. A sweep reaps the hand-copied binaries it superseded (same inode-age rule
   as staging sweeps: a binary ynpm now owns replaces the hand copy; anything
   still not owned is reported by `ynpm doctor`, never silently deleted).

## 9. What is NOT covered

- Not a general-purpose npm client: ynpm manages the @ygghq set (and reads
  agent-CLI state); `npm` remains the tool for everything else.
- No lockfiles/consumer manifests: consumers are binaries on a PATH, not
  dependency trees.
- No private packages in v1: public delivery is the fleet enabler; private
  GitHub Packages delivery is documented as the fallback if a package must
  ever be closed, accepting the authenticated-install cost.
- Windows CI matrix: packaging supports win32 targets from day one; the
  build runners for them are added per repo as each tool's Windows port
  reaches parity.
- Dev mode and alternative sources are channel features, not trust
  boundaries: a dev binary is built from a checkout the builder already
  trusts, and forgejo/github fetches verify against the release checksums
  exactly as npm fetches verify against the registry integrity hash.

## 10. First milestone

1. Owner: create the @ygghq org on npmjs (browser + passkey) and add a
   granular publish token as the vault item's `publish_token` field.
2. `ynpm` bin: `install/remove/update/list/rollback/sync-fleet` over the
   shared provisioner; `publish` driving the per-repo build matrix.
3. ytop first (smallest surface), end to end: tag → workflow → registry →
   `ynpm install` on three hosts → the sweep keeps it current.
4. Then ychrome, ychrome-vault, yedit, kasten; yRDP as a `git` package.

---

## 11. Dev mode: `ynpm dev`, and the drift law

Releasing through GitHub Actions → npm → `ynpm sync-fleet` is the production
channel: slow by design (CI matrix, review, tag). Development cannot wait on
it, and hand-scp'ing dev binaries is exactly the chore ynpm retired. So dev
is a first-class channel:

```
ynpm dev <repo-checkout>          build release locally → install as a DEV
                                  generation on THIS host (registry untouched)
ynpm dev --fleet <repo-checkout>  build once here, push the built generation
                                  to every connected machine's dev slot (the
                                  same transport yggterm's fleet already has)
ynpm prod <name>                  back to the registry channel (latest prod)
ynpm status                       per package: channel, dev age, prod drift
```

- **Channels are per package, not per host.** A package in dev mode carries a
  `dev` marker generation with: build commit, built-at time, builder host, and
  the prod version it supersedes. `~/.local/bin/<name>` points at whichever
  channel is live; switching is the same atomic symlink rename as any
  generation publish.
- **The drift law (6 hours).** The TTL sweep reads the dev markers. A package
  whose live channel is `dev` for more than 6 hours past its last prod
  publish raises a yggterm notification: *"<name> has been dev for 7h —
  publish it (`ynpm publish`) or pin it (`ynpm dev --pin`)"*. The reminder
  is the law; dev must not silently become the fleet's production.
- **The auto-switch back is conditional, and only forward:** when the drift
  reminder has fired and a prod version ≥ the dev build lands on the
  registry (CI published it), the sweep auto-switches that package to prod
  and drops the dev generation. Dev never auto-publishes — publishing is a
  human/CI decision; switching back is bookkeeping.
- **Fleet drift is symmetric:** a machine on prod while the dev host has
  moved on is normal during development; `ynpm dev --fleet` re-converges
  them. `ynpm status --fleet` shows the per-machine channel map.

## 12. Sources: npm is the default, not the gate

The registry channel needs no auth to install (public packages), but it is
not the only door. Each package resolves through a **source chain** — config
declares the order, per package or globally:

| source | tarballs from | auth | used when |
|---|---|---|---|
| `npm` (default) | registry.npmjs.org | none to install | everything public |
| `forgejo` | the self-hosted Forgejo release assets | forgejo token (vault) | private/pre-release builds, and hosts that must not touch npmjs |
| `github` | GitHub release assets | optional (public repos need none) | consumers who avoid the npm registry |

- **The asset contract is the same shape as the npm platform package** (a
  tarball with `bin/`, `package.json`, `finalize.mjs`), attached to a release
  tagged `v<version>`, named `@ygghq/<name>-<platform>.tgz`. One artifact
  shape, three doors.
- `ynpm install @ygghq/kasten --from forgejo`, or config:
  `{ "kasten": { "sources": ["forgejo", "npm"] } }`. The chain is
  fall-through in order; a source that cannot serve the version is skipped
  and named in the output.
- Checksums: a `SHA256SUMS` asset on the release verifies forgejo/github
  fetches (npm tarballs carry the registry's integrity hash already).
- `ynpm publish --release forgejo` drives the same build matrix locally and
  uploads release assets — the private-channel counterpart of the CI
  publish.

## 13. The registries: one config family, user files last

Two registries answer "what can be installed/launched", and both are
**config files, not code** — a user teaches ynpm and yggterm about new
software by dropping a file, Debian-style:

```
/etc-analogue (shipped, updated by releases):
  ~/.yggterm/ynpm/clis.d/00-base.json        the CLI registry — every agent
                                             CLI ynpm knows (codex, claude,
                                             qwen, kimi, grok, opencode, pi,
                                             muse, antigravity, …)
  ~/.yggterm/ynpm/apps.d/00-base.json        the surfaces registry — every
                                             launchable app surface (the
                                             libyggterm apps + peers)

/user-analogue (owned by the user, never written by ynpm or yggterm):
  ~/.yggterm/ynpm/clis.d/*.json              drop-ins; lexicographic; last wins
  ~/.yggterm/ynpm/apps.d/*.json
```

- **CLI registry entry** (what ynpm needs to install and yggterm needs to
  launch): name, npm package (or source chain), install shape
  (`native`/`script`/`git`), launch command, and — explicitly — an
  `integration` field: `none` for "installable + launchable as a plain
  binary", or the name of a compiled integration (transcript parsing,
  SessionKind, resume) that ships in yggterm itself. **A registry entry can
  make a brand-new CLI installable today; the integration nuances
  (transcript JSONL, resume flags, picker phrases) remain code-side work.**
  The shipped `00-base.json` is generated FROM the compiled `AGENT_CLIS`
  descriptors at yggterm build time — one SSOT, two consumers.
- **Surfaces registry entry** (what yggterm's right-click / startpage needs
  to launch a program): command (PATH-resolved), args, title, icon, and the
  surface it takes over. **Not exclusive to libyggterm apps**: `emacs -nx`,
  `tmux new`, anything on PATH is a legitimate entry. The libyggterm apps
  are entries whose command happens to speak the surface protocol.
- **Conflict rule:** ynpm and yggterm write ONLY the `00-base` files; user
  files are read-merged over them and never touched by tooling. `ynpm
  doctor` reports merge conflicts (same name in two files) and names the
  winning file.
- Documented as a yggui skill (`.agents/skills/`) so any user or agent can
  add an entry: the schema, an example, and the merge rules.

## 14. Bookkeeping: small by policy

npm's bloat is a policy failure, not a law. ynpm inherits the provisioner's
hygiene and adds the ynpm-specific sweep arms:

- generations: liveness-aware prune (a running binary's generation is
  deferred, never deleted underneath a session), N-keep by default;
- dev generations: shorter TTL (dev churn is the highest-volume churn);
- the npm cache: shared, GC'd (`npm cache verify` on the sweep cadence —
  measured 1.65 GB reclaimed);
- `ynpm gc`: the manual arm — prunes generations, GCs the cache, removes
  orphaned shims (a `~/.local/bin` entry whose generation is gone), prints
  the before/after totals per package;
- every prune is a ytrace event (§15) with the reason, so "where did my disk
  go" is answerable.

## 15. ytrace probe points

ynpm is observable like everything else in the fleet — every verb emits
ytrace events (category `ynpm`) at the decision points where bugs live:

| probe | at | carries |
|---|---|---|
| `ynpm/resolve` | version/source chosen | package, source chain order, the source that won, why |
| `ynpm/fetch` | tarball/asset fetched | url, bytes, integrity result, cache hit |
| `ynpm/finalize` | postinstall ran | exit, fast-copy or fallback path |
| `ynpm/gate` | runs-before-publish gate | binary, --version exit, elapsed |
| `ynpm/install` | generation published | package, version, channel, generation id |
| `ynpm/dev_install` | dev generation | commit, built-at, host, fleet targets |
| `ynpm/drift` | the 6h law fires | package, dev age, last prod |
| `ynpm/prune` | anything deleted | what, why, bytes reclaimed |
| `ynpm/publish` | registry/release upload | package, version, source, duration |

The events are the debugging surface: an install that "hangs" is a missing
`ynpm/gate`, a flaky update is a `ynpm/resolve` source-chain surprise, and
the drift law's firings are queryable history.
