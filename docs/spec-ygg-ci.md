# Spec: ygg-ci — the fleet single-build plane & compile optimization

**Status:** COMPILE-CACHE LANDED 2026-09-18 (all four build hosts wired + measured); §4 cache telemetry SPEC'D, implementation open
**Directive:** owner, 2026-09-18 — *"write these optimization techniques in docs/ ygg-ci spec in yggterm repo; the ygg onboarding skill should steer the agent to create optimization techniques based on the ci spec of their own; wire dev first, then jojo, oc, practice, jyas-webapp"*
**Owner surfaces:** `.agents/skills/yggterm-agent-fleet/ygg-ci.py` + SKILL.md §3c (how to DRIVE it — verbs, subscribe, conflicts), `~/.yggterm/relay/ci/ci.json` (the recipes), this file (the build-plane BEHAVIOR contract + the optimization techniques).

Per the docs-ssot law, the neighbors:

| question | owner |
|---|---|
| how to drive ygg-ci (subscribe, tune, conflicts, deploys) | SKILL.md §3c — link, never copy |
| what build/defect work is open | `docs/pending-bugs.md` — link |
| how the build plane must BEHAVE + how to make projects build fast | **this file** |

---

## 1. Recipe schema (`~/.yggterm/relay/ci/ci.json`)

One entry per project; picked up on the next tick (≤300s) without restarting the watcher.

| field | meaning |
|---|---|
| `repo` | absolute path to the clean MAIN checkout on the build host |
| `remote` / `main_branch` / `integration_branch` | where lanes merge before building |
| `build` | the shell command; runs in the main checkout with the watcher's env |
| `gates` | space-separated scripts that must exit 0 before main is pushed |
| `deploy` | optional deploy command (empty = build-and-gate only) |
| `host` | which host builds it (all current recipes: `dev`) |
| `interval` | seconds between ticks |

## 2. §Compile cache — the optimization techniques ⭐

**The law: compile caching is wired PER HOST, once, at the toolchain layer — never per project, never per worktree, never in a recipe.** Every cargo project that builds on that host (current and future) inherits it with zero configuration.

### 2.1 The deployed fleet recipe (2026-09-18 — dev, oc, jojo, practice)

sccache v0.18.0, upstream static musl binary (checksum-verified at install; **prefer this over apt** — distro packages lag rustc flag support and silently stop caching):

1. **Binary location:** `/usr/local/bin/sccache` on dev/oc/practice (sudo hosts; `/usr/local/bin` is on even the barest PATH there). jojo has no sudo from agent seats → `~/.local/bin/sccache`.
2. **The wire is `~/.cargo/config.toml`, NOT an environment variable:**
   ```toml
   [build]
   rustc-wrapper = "/usr/local/bin/sccache"   # absolute path — see below
   ```
   Cargo reads this file itself on every invocation, so interactive shells, non-interactive ssh, systemd services (`cargo run`), dx and cargo-ndk are all covered by one per-user file. **The path must be ABSOLUTE**: `practice`'s non-interactive PATH is `/usr/local/bin:/usr/bin:/bin` and any other host may differ; a bare `sccache` that fails to resolve is a hard build error, not a fallback.
3. **`SCCACHE_BASEDIR="/home/pi"` in `/etc/environment`** — guards cache keys against worktree absolute-path leakage. sshd applies `/etc/environment` via pam_env to non-interactive command sessions too (verified: `ssh dev "env | grep SCCACHE"` shows it). Keep `RUSTC_WRAPPER` OUT of `/etc/environment` — the cargo-config wire above makes it redundant, and env vars leak into every process. On jojo (no sudo): `SCCACHE_BASEDIR` exported in `~/.bashrc` + `~/.profile` instead — known gap: non-interactive ssh INTO jojo misses it; acceptable, jojo builds run in seat shells.
4. **Cache size via `~/.config/sccache/config`** (v0.18 removed the `--cache-size` CLI flag):
   ```toml
   [cache.disk]
   dir = "/home/pi/.cache/sccache"
   size = 53687091200        # 50 GiB — dev, oc, jojo
   ```
   practice gets `107374182400` (100 GiB) — it builds four target matrices (host debug/release, wasm32, aarch64-android) and a smaller LRU would thrash during active waves. (Optionally also `SCCACHE_CACHE_SIZE` in `/etc/environment`; the config file alone is sufficient and env-free.)
5. **`sccache --start-server` once at provisioning** — avoids a daemon-spawn stampede when N parallel cold builds race, and pre-answers the service-cgroup case: a daemon spawned under `practice-rs-api.service` dies with that unit's cgroup on restart (graceful — clients fall back — but pre-starting keeps the server warm).

### 2.2 Why this shape (measured, practice-rs on dev, 2026-09-18)

- Cargo compiles dependencies NON-incrementally and workspace crates WITH incremental. Therefore: **all dependency compiles are cacheable; workspace compiles are not (by design)** — sccache skips incremental crates automatically, so the warm edit loop is untouched.
- Fresh worktree, cold cache: **59.6s** wall, 277 misses (217 rust + 32 C/C++ + 28 assembler — `cc`-crate builds cache too).
- Fresh worktree, warm cache: **23.6s** wall, **100.00% hit rate (277/277)**. The steady-state warm edit loop stays at ~1.9s — it was never the problem.
- The events that dominated seat time were always cache-cold: new lane worktrees (the worktree law gives every unit a fresh `target/`), path-dep churn (libyggterm moved 17 commits/14 days; each pull invalidated every active worktree), and deploy-host builds. sccache makes all of them one-time-per-(host, target, source-version).

### 2.3 Topology decisions (do not relitigate without new evidence)

- **Per-host local disk ONLY.** dev builds host/test targets; practice builds wasm/android/release — disjoint target mixes mean a shared backend (redis, NFS on the fleet ZFS pool) buys almost nothing and adds lock hazards. No sccache-dist compile farm at this fleet size.
- **CI stays incremental.** Integration builds merge fresh content; workspace crates would get ~zero cache hits, so `CARGO_INCREMENTAL=0` would only slow main's warm loop.
- **Deps stay at `opt-level=2` in dev profiles** (e.g. practice-rs): runtime-relevant (argon2, serde), and sccache makes their compile cost one-time.
- **sccache does NOT cover:** wasm-opt (dx pipeline — separate knob, currently SIGABRT-falls-back per web deploy on practice), NDK C dependencies (would need a CC wrapper — skipped, small loss), incremental workspace crates (by design).

### 2.4 Deriving a project's own optimization plan (what onboarding points at)

An agent onboarding ANY project derives its plan from this section, in order:

1. **Inventory the build surfaces**: which hosts build it, which targets/profiles, which CI recipe, which deploy path. (One row per surface.)
2. **Cargo projects:** verify every build host carries §2.1's wire (`cat ~/.cargo/config.toml`, `sccache --show-stats`); if a host is missing it, wire it per §2.1 — do not invent a per-project cache.
3. **Measure one cold event before/after** (fresh worktree or wiped `target/`, `sccache --zero-stats` between) and record the hit rate — a cache that cannot show its hit rate is a cache you cannot trust (§4).
4. **Non-cargo ecosystems** map to the same principle — cache at the compiler/dependency layer, per host, outside the recipe: gradle → persistent build cache + `--build-cache`; JS bundlers → persistent vite/webpack cache + pnpm store; sbcl → warm core image. Name the layer, wire it once per host, document the equivalent of §2.1 in the project's own docs, and point back here.
5. **Record the plan** in the project repo (its `docs/` or CLAUDE.md) with a pointer to this spec — the derivation is per-project, the recipe is not.

## 3. Build-plane behavior contract

- One integration build per project, on one host, in the clean main checkout — never per-worktree builds of deployable binaries (SKILL.md §3c owns the why).
- Gates must pass before main moves; a gate's verdict is recorded in `builds/*.json` with the merge sha.
- Lane conflicts are explicit (`conflicts` in the build record + ci.log); the conflicted lane is excluded for that build only and retries after rebase.
- Builds inherit the watcher's environment; anything the build NEEDS must therefore live in files the build reads (cargo config, recipes, scripts) — not in some seat's shell profile. This is why §2 wires through `~/.cargo/config.toml`.

## 4. Cache telemetry (SPEC'D 2026-09-18 — implementation open, take it as a lane)

sccache fails silently into un-cached builds (daemon death, unparseable flags) — without instrumentation a dead cache is invisible. ygg-ci therefore owes its recipes:

1. Each `builds/<project>--<ts>--<sha>.json` record gains a `sccache` block (hit rate, requests executed, cache size) captured around the build command.
2. An assertion that `(hits + misses)` grows across a build that compiles anything — growth stops ⇒ loudly record `cache: STALE` in the build record.
3. A `ygg-ci.py cache` verb: per-host cache health (hit rate over last N builds, size, `--show-stats` digest) for the fleet.
4. `tune`-time provisioning check: a project tuned onto a host without a working sccache wire gets a warning naming §2.1 as the fix.
