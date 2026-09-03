// Per-CLI split: each CLI's nuance lives in its own file under this directory.
// `mod.rs` holds the shared provision/identity/refresh machinery.
pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod grok_build;
pub mod kimi;
pub mod muse;
pub mod opencode;
pub mod pi;
pub mod qwen;

use yggterm_core::cli_plane::CliInvocationShape;
use crate::{SessionKind, shell_single_quote};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Read};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use yggterm_core::agent_cli::{AgentCliDescriptor, CliInstall, CliUpdate, agent_cli_descriptor};
use yggterm_core::{
    AgentLaunchOptions, ENV_YGGTERM_HOME, PerfSpan, append_trace_event,
    resolve_yggterm_home,
};
use yggui_contract::UiTheme;

const MANAGED_NPM_DIRNAME: &str = "npm";
const MANAGED_NPM_CACHE_DIRNAME: &str = "npm-cache";
pub(crate) const EXPORTED_TERM_PROGRAM: &str = "vscode";
pub(crate) const YGGTERM_TERM_PROGRAM: &str = "yggterm";
const YGGTERM_TERM_PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");
const ENV_YGGTERM_TERMINAL_APPEARANCE: &str = "YGGTERM_TERMINAL_APPEARANCE";
pub(crate) const ENV_YGGTERM_CC_EXTRA_ARGS: &str = "YGGTERM_CC_EXTRA_ARGS";
/// One launch's model / permission mode, as JSON, carried over ssh to the
/// remote `start-<slug>` wrapper.
///
/// ⚠ Deliberately NOT a second `*_EXTRA_ARGS` variable. `YGGTERM_CC_EXTRA_ARGS`
/// carries already-composed claude TOKENS — the user's configured extra args
/// plus the launch's flags, flattened — so reading it tells you what to append
/// but not what was asked for. A per-CLI copy of that would need the composing
/// to happen on the LOCAL side against the REMOTE CLI's flag spellings, which
/// is the machine that does not know them. Sending the OPTIONS instead lets the
/// owning machine compose them with its own descriptor, and lets a CLI that
/// cannot express a mode refuse by name there rather than silently drop it.
pub(crate) const ENV_YGGTERM_AGENT_LAUNCH_OPTIONS: &str = "YGGTERM_AGENT_LAUNCH_OPTIONS";
/// The CLIENT's configured launch flags for the CLI being started, carried over
/// ssh so a remote row gets the same flags a local one does.
///
/// **The gap this closes.** A remote host reads its OWN settings store, which is
/// a different machine's and holds nothing the user typed into the GUI. Claude
/// Code alone had a way across the hop (`YGGTERM_CC_EXTRA_ARGS`), so eight
/// first-class CLIs launched remotely with no permission flag at all and stopped
/// on a prompt nobody could answer.
///
/// ⚖ **Configured args, not launch OPTIONS — the two variables answer different
/// questions and both are needed.** `YGGTERM_AGENT_LAUNCH_OPTIONS` carries what
/// ONE launch asked for, abstractly, so the owning machine composes it with its
/// own descriptor and can refuse a mode by name. This one carries a string the
/// USER typed for that CLI, which no descriptor can re-derive.
///
/// ⛔ It is keyed to the launch, not to the machine: it is exported on the ssh
/// line for the session being started, never `set_var` into a daemon. The
/// process-global route is how a daemon's frozen env leaks one session's flags
/// into the next.
pub(crate) const ENV_YGGTERM_AGENT_EXTRA_ARGS: &str = "YGGTERM_AGENT_EXTRA_ARGS";
const ENV_YGGTERM_TERMINAL_COLOR_FOREGROUND: &str = "YGGTERM_TERMINAL_COLOR_FOREGROUND";
const ENV_YGGTERM_TERMINAL_COLOR_BACKGROUND: &str = "YGGTERM_TERMINAL_COLOR_BACKGROUND";
const ENV_YGGTERM_TERMINAL_COLOR_PALETTE: [&str; 16] = [
    "YGGTERM_TERMINAL_COLOR_0",
    "YGGTERM_TERMINAL_COLOR_1",
    "YGGTERM_TERMINAL_COLOR_2",
    "YGGTERM_TERMINAL_COLOR_3",
    "YGGTERM_TERMINAL_COLOR_4",
    "YGGTERM_TERMINAL_COLOR_5",
    "YGGTERM_TERMINAL_COLOR_6",
    "YGGTERM_TERMINAL_COLOR_7",
    "YGGTERM_TERMINAL_COLOR_8",
    "YGGTERM_TERMINAL_COLOR_9",
    "YGGTERM_TERMINAL_COLOR_10",
    "YGGTERM_TERMINAL_COLOR_11",
    "YGGTERM_TERMINAL_COLOR_12",
    "YGGTERM_TERMINAL_COLOR_13",
    "YGGTERM_TERMINAL_COLOR_14",
    "YGGTERM_TERMINAL_COLOR_15",
];
const TERMINAL_IDENTITY_ENV_REMOVALS: &[&str] = &["NO_COLOR"];
const MANAGED_CLI_REFRESH_STATE_FILENAME: &str = "managed-cli-refresh-state.json";
const MANAGED_CLI_REFRESH_TTL_ENV: &str = "YGGTERM_MANAGED_CLI_REFRESH_TTL_MS";
const MANAGED_CLI_BACKGROUND_INSTALL_ENV: &str = "YGGTERM_MANAGED_CLI_BACKGROUND_INSTALL";
/// Where fetched vendor installers land, under `~/.yggterm`. Kept rather than
/// deleted so a failed unattended install can be read afterwards.
const VENDOR_INSTALLER_DIRNAME: &str = "vendor-installers";
/// Ceiling on the FETCH of a vendor installer. The installer's own run is not
/// bounded here — the Muse launcher downloads a multi-hundred-MB payload — but
/// it runs with stdin closed, so it cannot block on a prompt.
const VENDOR_FETCH_TIMEOUT_SECS: u64 = 60;
/// A CLI's `--version` is metadata, never a reason to stop identity, title,
/// or attachment maintenance for the whole daemon. In particular a node
/// process can remain in uninterruptible sleep under memory pressure.
const MANAGED_CLI_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_MANAGED_CLI_REFRESH_TTL_MS: u64 = 2 * 60 * 60_000; // 2h — frequent daily checks, yggterm maintains isolated binaries

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedCliTool {
    Codex,
    CodexLiteLlm,
    ClaudeCode,
    // The 2026-08-08 intake. Every registered CLI needs a row here or
    // `managed_cli_tool_and_descriptor_agree_on_every_binary_name` fails: a CLI
    // with no provisioning key is one yggterm can launch but never install, and
    // "it is not installed" would surface as a launch that dies at the PTY.
    Pi,
    OpenCode,
    QwenCode,
    Kimi,
    Muse,
    Antigravity,
    // The 2026-08-13 intake.
    GrokBuild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedCliBinarySource {
    Managed,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedCliToolStatus {
    pub tool: ManagedCliTool,
    pub package_name: String,
    pub binary_name: String,
    #[serde(default)]
    pub version_before: Option<String>,
    #[serde(default)]
    pub version_after: Option<String>,
    #[serde(default)]
    pub source_before: Option<ManagedCliBinarySource>,
    #[serde(default)]
    pub source_after: Option<ManagedCliBinarySource>,
    pub changed: bool,
    pub available: bool,
    pub action: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedCliRefreshReport {
    pub scope: String,
    pub background: bool,
    pub statuses: Vec<ManagedCliToolStatus>,
    #[serde(default)]
    pub skipped_recently: bool,
    #[serde(default)]
    pub ttl_remaining_ms: Option<u64>,
    #[serde(default)]
    pub install_attempted: bool,
    #[serde(default)]
    pub install_deferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalIdentityColorProfile {
    pub foreground: String,
    pub background: String,
    pub palette: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ManagedCliRefreshState {
    #[serde(default)]
    last_successful_refresh_ms: Option<u64>,
    #[serde(default)]
    managed_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct ManagedCliPaths {
    home: PathBuf,
    prefix: PathBuf,
    bin_dir: PathBuf,
    cache_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct ToolProbe {
    version: Option<String>,
    source: Option<ManagedCliBinarySource>,
    available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedCliAction<'a> {
    Launch,
    ResumePicker {
        persistent: bool,
    },
    Resume {
        session_id: &'a str,
        persistent: bool,
    },
}

impl ManagedCliTool {
    /// The session kind this provisioning key serves.
    ///
    /// The ONE place the two enums are matched up. Everything else a tool knows
    /// about itself — binary, package, display name — is read off the registry
    /// descriptor from here, so the second hand-kept table those used to live in
    /// cannot drift from the one the launcher reads.
    fn session_kind(self) -> SessionKind {
        match self {
            Self::Codex => SessionKind::Codex,
            Self::CodexLiteLlm => SessionKind::CodexLiteLlm,
            Self::ClaudeCode => SessionKind::ClaudeCode,
            Self::Pi => SessionKind::Pi,
            Self::OpenCode => SessionKind::OpenCode,
            Self::QwenCode => SessionKind::QwenCode,
            Self::Kimi => SessionKind::Kimi,
            Self::Muse => SessionKind::Muse,
            Self::Antigravity => SessionKind::Antigravity,
            Self::GrokBuild => SessionKind::GrokBuild,
        }
    }

    fn descriptor(self) -> &'static AgentCliDescriptor {
        agent_cli_descriptor(self.session_kind())
            .expect("every managed CLI tool names a registered agent CLI")
    }

    pub(crate) fn from_session_kind(kind: SessionKind) -> Option<Self> {
        Some(match kind {
            SessionKind::Codex => Self::Codex,
            SessionKind::CodexLiteLlm => Self::CodexLiteLlm,
            // Claude Code ships as @anthropic-ai/claude-code on npm and
            // releases near-daily; an unmanaged binary goes missing/stale and
            // the user pays an interactive `npm up -g` round-trip mid-flow.
            // Managing it gives CC the same self-provisioning + 6h refresh
            // the codex CLIs get ([[spec-cli-binary-auto-provisioning]]).
            SessionKind::ClaudeCode => Self::ClaudeCode,
            SessionKind::Pi => Self::Pi,
            SessionKind::OpenCode => Self::OpenCode,
            SessionKind::QwenCode => Self::QwenCode,
            SessionKind::Kimi => Self::Kimi,
            SessionKind::Muse => Self::Muse,
            SessionKind::Antigravity => Self::Antigravity,
            SessionKind::GrokBuild => Self::GrokBuild,
            SessionKind::Shell | SessionKind::SshShell | SessionKind::Document => return None,
        })
    }

    pub(crate) fn binary_name(self) -> &'static str {
        self.descriptor().binary_name
    }

    /// The npm package `npm i -g` may be handed for this tool, or `None` when
    /// this CLI is not npm-provisionable AT ALL.
    ///
    /// ⛔ **`Uv`, `VendorScript` and `Manual` answer `None`, and the installer
    /// refuses them BY NAME.** The old table answered every tool with a package
    /// string, so a uv/vendor CLI reaching [`install_latest`] would have been
    /// appended to one `npm install -g` line — and npm fails the WHOLE batch on
    /// one unresolvable name, which would have taken codex and claude down with
    /// it rather than skipping the one tool npm cannot serve.
    pub(crate) fn npm_package(self) -> Option<&'static str> {
        // ✅ The `CodexLiteLlm` override that stood here from 2026-08-08 is
        // GONE, having done its job: it recorded that the filesystem said npm
        // while the descriptor said Manual, and the descriptor was corrected to
        // `CliInstall::Npm("@avikalpa/codex-litellm")` in the same day. The
        // measurement outlived the workaround, which is the right order.
        match self.descriptor().install {
            CliInstall::Npm(package) => Some(package),
            CliInstall::Uv(_) | CliInstall::VendorScript(_) | CliInstall::Manual => None,
        }
    }

    /// What a status report NAMES as this tool's provisioning source. Not a
    /// package to install — [`Self::npm_package`] is the only thing allowed to
    /// answer that — but the thing a human reads to know where the binary comes
    /// from, which for a uv/vendor/manual CLI is not an npm package at all.
    pub(crate) fn package_name(self) -> &'static str {
        if let Some(package) = self.npm_package() {
            return package;
        }
        match self.descriptor().install {
            CliInstall::Npm(package) | CliInstall::Uv(package) => package,
            CliInstall::VendorScript(url) => url,
            // ⚠ NOT "yggterm never provisions it" — that was true of the whole
            // CLI when install was the only question asked. yggterm cannot FETCH
            // agy; it updated it from 1.0.5 to 1.1.11 on guihost the same day.
            CliInstall::Manual => "a hand-installed binary (yggterm keeps it updated)",
        }
    }

    fn display_name(self) -> &'static str {
        self.descriptor().display_name
    }
}

impl ManagedCliPaths {
    fn resolve() -> Result<Self> {
        let home = resolve_yggterm_home()?;
        let prefix = home.join(MANAGED_NPM_DIRNAME);
        let bin_dir = prefix.join("bin");
        let cache_dir = home.join(MANAGED_NPM_CACHE_DIRNAME);
        Ok(Self {
            home,
            prefix,
            bin_dir,
            cache_dir,
        })
    }

    /// ⛔ WHERE A PACKAGE'S OWN INSTALLER STAGES ITS DOWNLOAD — AND IT MUST NOT
    ///    BE `/tmp`.
    ///
    /// A published CLI's `preinstall` script stages with `os.tmpdir()`, which is
    /// `$TMPDIR` or `/tmp`. On the desktop host `/tmp` is a **tmpfs**, so a
    /// 78 MB release tarball is downloaded into RAM. That alone would be a bad
    /// trade on a 14 GB laptop; the package also never removes the directory it
    /// made, so every auto-update leaks the whole 78 MB and it never comes back.
    ///
    /// ⇒ Measured on the desktop host 2026-08-14: **51 leaked staging dirs,
    /// 2.85 GB, accumulating since 2026-08-02**, held in a RAM-backed
    /// filesystem while the machine sat at 11 GB of 15 GB swap. The owner's
    /// report was memory pressure, and this was the largest single cause.
    ///
    /// ⚠ **The leak is in a package we do not own, and this does not fix it.**
    /// It moves the damage off RAM and onto disk, where a leaked tarball is
    /// merely untidy instead of an eviction. The package still needs its own
    /// `rmSync(tmpDir)`, and the sweep below still has to run.
    fn staging_dir(&self) -> PathBuf {
        self.home.join("cli-staging")
    }

    /// The root of the PER-CLI prefixes, and the reason there is one per CLI.
    ///
    /// ⛔ MEASURED 2026-08-20, twice, deterministically: batching every npm CLI
    /// into one `npm install -g --force <7 packages>` line opens a multi-second
    /// window in which npm has unlinked ALL SEVEN published binaries and not yet
    /// relinked any of them. A kill 12 s in left `bin/` with **zero** working
    /// CLIs and seven orphaned `.<name>-<random>` staging symlinks — every agent
    /// CLI on the machine gone at once, recoverable only by a full reinstall.
    ///
    /// The 2×2 that pins it: a SINGLE-package install, with or without
    /// `--force`, survived the same interrupt intact; a batch WITHOUT `--force`
    /// survived; only batch-plus-`--force` destroyed the set. `--force` is what
    /// makes the window open on EVERY pass rather than only when something
    /// changed — it rewrites all 164 packages and relinks all 7 bins even when
    /// the tree is already current (`changed 164 packages` on a no-op).
    ///
    /// ⇒ One prefix per CLI means one CLI's install can never touch another's
    /// binary, whatever happens to it.
    fn cli_root(&self) -> PathBuf {
        self.prefix.join("cli")
    }

    /// A CLI's tree for one GENERATION of its install.
    ///
    /// Generations exist so publishing is a single atomic `rename` of a symlink
    /// rather than a mutation of a live tree: generation N keeps serving until
    /// the instant N+1 is proven good. An interrupted install damages only the
    /// unpublished N+1 directory, so the old binary keeps working — which is the
    /// property the batch install could not offer at any point in its run.
    ///
    /// ⭐ It is also the vendor's own pattern rather than an invention here:
    /// the grok npm package installs `~/.grok/bin/grok-<version>` and swaps a
    /// `grok` symlink onto it, for the same reason.
    fn cli_generation_dir(&self, slug: &str, generation: u64) -> PathBuf {
        self.cli_root().join(format!("{slug}.gen{generation}"))
    }

    /// Which generation is PUBLISHED, read from the published symlink itself.
    ///
    /// The symlink is the single source of truth for "what is live": scanning
    /// the directory for the highest `gen` would instead name a half-written
    /// tree an interrupted run left behind.
    #[cfg(unix)]
    fn published_generation(&self, slug: &str, binary: &str) -> Option<u64> {
        let target = fs::read_link(self.bin_dir.join(binary)).ok()?;
        let marker = format!("{slug}.gen");
        target.components().find_map(|component| {
            component
                .as_os_str()
                .to_str()?
                .strip_prefix(&marker)?
                .parse::<u64>()
                .ok()
        })
    }

    /// Point `bin/<binary>` at `generation`, atomically.
    ///
    /// ⛔ Symlink-then-`rename`, never `remove`-then-`symlink`: the second form
    /// has a window in which the binary does not exist, which is precisely the
    /// defect this whole layout exists to close. `rename` onto an existing path
    /// replaces it in one step and is never observable half-done.
    #[cfg(unix)]
    fn publish_cli_binary(&self, slug: &str, binary: &str, generation: u64) -> Result<()> {
        // ⛔ ABSOLUTE, derived from `cli_generation_dir` — the one owner of
        //    where a generation lives. A `../cli/...` relative target silently
        //    assumed `bin_dir` is exactly `prefix/bin`; it is not in every
        //    construction of these paths, and the link then pointed at a
        //    directory that does not exist. Caught by
        //    `an_abandoned_generation_leaves_the_published_binary_untouched`.
        let target = self
            .cli_generation_dir(slug, generation)
            .join("bin")
            .join(binary);
        let link = self.bin_dir.join(binary);
        let staged = self.bin_dir.join(format!(".{binary}.ygg-publish"));
        let _ = fs::remove_file(&staged);
        std::os::unix::fs::symlink(&target, &staged).with_context(|| {
            format!("staging the published symlink for {}", link.display())
        })?;
        // A legacy install left a real FILE here; `rename` replaces it just the
        // same, so the migration off the shared prefix needs no separate step.
        fs::rename(&staged, &link)
            .with_context(|| format!("publishing {}", link.display()))?;
        Ok(())
    }

    /// Drop every generation of `slug` except the one just published, and any
    /// partial tree an interrupted run abandoned — EXCEPT a generation a
    /// running process is still executing from.
    ///
    /// ⛔ MEASURED BROKEN LIVE: a long-running CLI references files INSIDE its
    /// generation tree long after its entry binary is loaded — the codex CLI
    /// spawns `codex-code-mode-host` from
    /// `lib/node_modules/<platform pkg>/vendor/.../bin/` on every shell
    /// command it runs. Pruning that tree under a live session does not touch
    /// the running process (its exe is already mapped), but every SUBSEQUENT
    /// helper spawn dies with "No such file or directory", and the session
    /// reads as "the CLI is broken" when the CLI is fine and the install
    /// system deleted its working files. So before removing a generation, the
    /// running processes' executables are sampled: any generation a live
    /// process executes from is deferred to a later sweep, which will reap it
    /// after the process is gone.
    ///
    /// ⚠ On hosts where liveness cannot be measured (no `/proc`), the
    /// immediately-previous generation is retained unconditionally as a grace
    /// generation — one update's worth of survival for a long-running session,
    /// reaped by the refresh after next.
    fn prune_cli_generations(&self, slug: &str, keep: u64) {
        let keep_name = format!("{slug}.gen{keep}");
        let marker = format!("{slug}.gen");
        let Ok(entries) = fs::read_dir(self.cli_root()) else {
            return;
        };
        let live_exes = running_process_executable_paths();
        #[cfg(not(target_os = "linux"))]
        let grace_name = format!("{slug}.gen{}", keep.saturating_sub(1));
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name == keep_name || !name.starts_with(&marker) {
                continue;
            }
            #[cfg(not(target_os = "linux"))]
            if name == grace_name {
                continue;
            }
            let path = entry.path();
            if generation_is_executed_by_running_process(&path, &live_exes) {
                append_trace_event(
                    &self.home,
                    "managed_cli",
                    "install",
                    "prune_deferred_running",
                    serde_json::json!({
                        "slug": slug,
                        "generation": name,
                        "reason": "a running process executes from this generation tree",
                    }),
                );
                continue;
            }
            let _ = fs::remove_dir_all(entry.path());
        }
    }

    fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.prefix)
            .with_context(|| format!("creating managed npm prefix {}", self.prefix.display()))?;
        fs::create_dir_all(&self.bin_dir)
            .with_context(|| format!("creating managed npm bin {}", self.bin_dir.display()))?;
        fs::create_dir_all(&self.cache_dir)
            .with_context(|| format!("creating managed npm cache {}", self.cache_dir.display()))?;
        let staging = self.staging_dir();
        fs::create_dir_all(&staging)
            .with_context(|| format!("creating CLI staging dir {}", staging.display()))?;
        Ok(())
    }

    /// Reap staging directories a package's installer abandoned.
    ///
    /// Best-effort and deliberately unconditional on success: this runs BEFORE
    /// an install rather than after, because the leak belongs to a script we do
    /// not control and which may fail, be killed, or simply never clean up. A
    /// sweep that only ran on the happy path would miss exactly the cases that
    /// leak most.
    ///
    /// ⛔ Root-owned leaks: the upstream `codex-litellm` `preinstall` creates
    /// `codex-litellm-*` in `os.tmpdir()` and never removes it. When that
    /// `tmpdir` is `/tmp` (tmpfs) the 78 MB tarball is RAM, and when the
    /// installer runs as `root` the directory is `root:root 0700` so a
    /// `pi`-owned sweep sees it as unreadable and `du` counts 0. The
    /// disk-backed `TMPDIR=cli-staging` relocation (above) prevents new
    /// pi-owned leaks, but legacy root-owned dirs in `/tmp` survive it.
    /// This sweep therefore also reaps `/tmp/codex-litellm-*` via `sudo -n`
    /// when available, and falls back to a best-effort `remove_dir_all` as `pi`.
    fn sweep_staging(&self) {
        let staging = self.staging_dir();
        if let Ok(entries) = fs::read_dir(&staging) {
            for entry in entries.flatten() {
                // Anything here is a leftover by construction: the directory exists
                // only as scratch for an installer that has already exited.
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        // Legacy tmpfs leaks: root-owned `codex-litellm-*` dirs left in /tmp
        // before TMPDIR was relocated. Best-effort, never fails the install.
        Self::sweep_legacy_tmp_codex_litellm();
    }

    fn sweep_legacy_tmp_codex_litellm() {
        let tmp = std::path::Path::new("/tmp");
        let Ok(entries) = fs::read_dir(tmp) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with("codex-litellm-") {
                continue;
            }
            // Try as pi first; if permission denied, try passwordless sudo.
            let removed = if path.is_dir() {
                fs::remove_dir_all(&path).is_ok()
            } else {
                fs::remove_file(&path).is_ok()
            };
            if !removed {
                let _ = std::process::Command::new("sudo")
                    .arg("-n")
                    .arg("rm")
                    .arg("-rf")
                    .arg(&path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }
    }

    fn env_path(&self) -> OsString {
        let mut parts = vec![self.bin_dir.clone()];
        parts.extend(env::split_paths(
            &env::var_os("PATH").unwrap_or_else(|| OsString::from("")),
        ));
        env::join_paths(parts).unwrap_or_else(|_| OsString::from(""))
    }

    /// The `PATH` prefix a LAUNCHED session gets, single-quoted and joined —
    /// the managed npm bin dir, then every login-shell dir the daemon's own
    /// `PATH` lacks.
    ///
    /// ⛔ **THE DEFECT THIS CLOSES, reported 2026-08-09: `muse`, `kimi`
    /// and `agy` were "not found" on a machine that HAS all three.** The PTY is
    /// spawned `$SHELL -c` — NOT `-lc` (`terminal.rs` `shell_command`) — so it
    /// inherits the daemon's stripped `PATH`, measured on the live host as
    /// `/usr/local/bin:/usr/bin:/bin:/usr/games`. This export prepended the
    /// managed npm bin dir and nothing else, so the split was exact and
    /// invisible: `pi`/`opencode`/`qwen` arrive via npm INTO that dir and
    /// launched fine, while `kimi` (uv), `muse` (vendor script) and `agy`
    /// (manual) land in `~/.local/bin` — on the login `PATH`, on no `PATH` the
    /// PTY could see. `bash` printed `muse: command not found` and stayed at a
    /// prompt, so the row read `Running` with `Launch Error: none`.
    ///
    /// ⚖ It is a [[project-purpose]] WRAPPER-VS-MANUAL PARITY break, which is
    /// why the fix is here and not in a per-CLI flag: typing `muse` into a
    /// normal shell on that host works, and a session yggterm opens must
    /// resolve binaries the same way the user's own terminal does.
    ///
    /// ⚠ [[finding-a-set-is-not-a-fill]] applies to the pairing, not the value:
    /// the probe that decides `available` (and the launch refusal gate built on
    /// it) resolves against [`login_shell_path_dirs`], so a launch that did NOT
    /// see those dirs could only ever disagree with it. Both sides now read the
    /// same list — the gate can no longer pass a launch that is going to fail.
    fn launch_path_prefix(&self) -> String {
        compose_launch_path_prefix_dirs(
            Some(&self.bin_dir),
            user_local_bin_dir().as_deref(),
            &login_shell_path_dirs(),
        )
        .iter()
        .map(|dir| shell_single_quote(&dir.display().to_string()))
        .collect::<Vec<_>>()
        .join(":")
    }

    fn shell_exports(&self, tool: ManagedCliTool) -> String {
        self.shell_exports_with_terminal_appearance(tool, None)
    }

    fn shell_exports_with_terminal_appearance(
        &self,
        tool: ManagedCliTool,
        terminal_appearance: Option<&str>,
    ) -> String {
        let mut exports = terminal_appearance
            .and_then(normalize_terminal_appearance)
            .map(terminal_identity_shell_exports_for_appearance)
            .unwrap_or_else(terminal_identity_shell_exports);
        exports.extend([
            format!(
                "export NPM_CONFIG_PREFIX={}",
                shell_single_quote(&self.prefix.display().to_string())
            ),
            format!(
                "export npm_config_prefix={}",
                shell_single_quote(&self.prefix.display().to_string())
            ),
            "export NPM_CONFIG_UPDATE_NOTIFIER=false".to_string(),
            "export npm_config_update_notifier=false".to_string(),
            "export npm_config_audit=false".to_string(),
            "export npm_config_fund=false".to_string(),
            format!("export PATH={}:\"$PATH\"", self.launch_path_prefix()),
        ]);
        if tool == ManagedCliTool::CodexLiteLlm {
            let codex_home = dirs::home_dir()
                .map(|path| path.join(".codex-litellm"))
                .unwrap_or_else(|| PathBuf::from("$HOME/.codex-litellm"));
            exports.push(format!(
                "export CODEX_HOME={}",
                shell_single_quote(&codex_home.display().to_string())
            ));
        }
        exports.join(" && ")
    }
}

/// WHY a managed-CLI refresh is happening, which is the only thing that decides
/// whether it may install.
///
/// ⛔ This replaces a bare `background: bool` that was answering two unrelated
/// questions at once — *"may I spend time?"* and *"may I install?"* — and the
/// conflation is precisely why the fleet pipeline the owner asked for twice
/// could not be built out of the engine that already existed. A scheduled sweep
/// wants the TTL (its whole job is cadence) **and** the install (its whole job
/// is keeping the CLIs current); `background: true` gave it the first and
/// forbade the second, `background: false` gave it the second and threw away
/// the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedCliRefreshMode {
    /// A human, or an explicit CLI verb, asked for it. Install now and ignore
    /// the TTL — someone is waiting on the answer and asked for it by name.
    Foreground,
    /// Incidental: a focus, an attach or a launch brushed the provisioning path.
    /// Respect the TTL **and** defer every install behind
    /// `YGGTERM_MANAGED_CLI_BACKGROUND_INSTALL`. ⛔ Do not weaken this arm — it
    /// fires on the owner's hot paths, and an npm/uv run there is the fan and
    /// CPU regression that got background installs opted out of in the first
    /// place.
    Incidental,
    /// The scheduled fleet sweep. Respect the TTL — the chore's own cadence plus
    /// the TTL is the pacing — but DO install, because installing and updating
    /// every CLI on every connected machine is the entire reason the sweep runs.
    Scheduled,
}

impl ManagedCliRefreshMode {
    /// The legacy wire/flag word. `Scheduled` reports itself as background so a
    /// remote binary too old to know the mode still gets the cheaper arm.
    pub fn is_background(self) -> bool {
        !matches!(self, Self::Foreground)
    }

    /// Whether a refresh this recent may be skipped outright.
    fn respects_ttl(self) -> bool {
        !matches!(self, Self::Foreground)
    }

    /// Whether installs are deferred rather than performed. Only the incidental
    /// arm defers; see the variant docs.
    fn defers_installs(self) -> bool {
        matches!(self, Self::Incidental)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Incidental => "background",
            Self::Scheduled => "scheduled",
        }
    }

    /// Parse the word a remote `yggterm server remote refresh-managed-cli`
    /// carries. Unknown words mean foreground, which is what the pre-mode
    /// `args[3] == "background"` comparison already did.
    pub fn from_wire_word(raw: &str) -> Self {
        match raw.trim() {
            "background" => Self::Incidental,
            "scheduled" => Self::Scheduled,
            _ => Self::Foreground,
        }
    }
}

fn managed_cli_tool_jitter_ms(tool: ManagedCliTool, ttl_ms: u64) -> u64 {
    // Deterministic per-tool jitter 0..10% TTL, so 7 CLIs don't all fire on same tick
    // (booter/monitor timing: staggered, not thundering herd). Uses tool slug hash.
    let slug = tool.descriptor().slug;
    let mut h: u64 = 0xcbf29ce484222325;
    for b in slug.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h % (ttl_ms / 10).max(1)
}

pub fn managed_cli_refresh_ttl_ms() -> u64 {
    env::var(MANAGED_CLI_REFRESH_TTL_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MANAGED_CLI_REFRESH_TTL_MS)
}

pub(crate) fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn managed_cli_refresh_state_path(home: &Path) -> PathBuf {
    home.join(MANAGED_CLI_REFRESH_STATE_FILENAME)
}

fn load_managed_cli_refresh_state(home: &Path) -> ManagedCliRefreshState {
    let path = managed_cli_refresh_state_path(home);
    let Ok(raw) = fs::read_to_string(&path) else {
        return ManagedCliRefreshState::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_managed_cli_refresh_state(home: &Path, state: &ManagedCliRefreshState) -> Result<()> {
    let path = managed_cli_refresh_state_path(home);
    let Some(parent) = path.parent() else {
        anyhow::bail!("managed cli refresh state path has no parent");
    };
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "creating managed cli refresh state directory {}",
            parent.display()
        )
    })?;
    let encoded =
        serde_json::to_vec_pretty(state).context("serializing managed cli refresh state")?;
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("managed-cli-refresh-state.json"),
        std::process::id(),
        current_time_ms()
    ));
    fs::write(&temp_path, encoded).with_context(|| {
        format!(
            "writing managed cli refresh temp state {}",
            temp_path.display()
        )
    })?;
    if let Err(error) = fs::rename(&temp_path, &path) {
        if error.kind() == ErrorKind::AlreadyExists {
            let _ = fs::remove_file(&path);
            fs::rename(&temp_path, &path).with_context(|| {
                format!(
                    "replacing managed cli refresh state {} after removing the previous file",
                    path.display()
                )
            })?;
        } else {
            let _ = fs::remove_file(&temp_path);
            return Err(error).with_context(|| {
                format!(
                    "renaming managed cli refresh state {} into place",
                    path.display()
                )
            });
        }
    }
    Ok(())
}

/// Every registered agent CLI, as provisioning keys, in registry order.
///
/// The refresh sweep's roster. Derived so registering a CLI is the ONLY step
/// needed to get it probed and version-reported; the hand-listed array this
/// replaced is where a new CLI silently stayed invisible to provisioning.
fn managed_cli_tools_for_refresh() -> Vec<ManagedCliTool> {
    yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .filter_map(|descriptor| ManagedCliTool::from_session_kind(descriptor.kind))
        .collect()
}

fn probe_tools(
    paths: &ManagedCliPaths,
    tools: &[ManagedCliTool],
) -> Vec<(ManagedCliTool, ToolProbe)> {
    tools
        .iter()
        .copied()
        .map(|tool| (tool, probe_tool(paths, tool)))
        .collect::<Vec<_>>()
}

/// What the LOGIN SHELL resolves for `binary_name`, which is the only thing a
/// session will actually execute.
///
/// Remote launches go through `login_shell_wrap` (`exec bash -lc '<cmd>'`), so
/// this reproduces the resolution a real launch performs, deliberately WITHOUT
/// the managed prefix forced onto PATH.
fn login_shell_resolved_cli(binary_name: &str) -> Option<(String, Option<String>)> {
    let script = format!("command -v {binary_name} || exit 1");
    let output = Command::new("bash")
        .arg("-lc")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let path = lines.next()?.to_string();
    let version = run_version_command(Path::new(&path));
    Some((path, version))
}

/// Pull the `1.2.3` out of a `--version` line. Both CLIs decorate it
/// (`2.1.223 (Claude Code)`, `codex-cli 0.144.6`), so a whole-line comparison
/// against a bare managed version would report a false divergence every time.
fn extract_semver_like_version(line: &str) -> Option<&str> {
    line.split_whitespace().find(|token| {
        let mut parts = token.split('.');
        let ok = parts.clone().count() >= 3;
        ok && parts.all(|part| {
            !part.is_empty() && part.chars().next().is_some_and(|c| c.is_ascii_digit())
        })
    })
}

/// Report when the binary yggterm MAINTAINS is not the binary a session RUNS.
///
/// ⛔ The failure this exists to catch, measured across the fleet 2026-08-06:
/// the refresh had `~/.yggterm/npm/bin/claude` at 2.1.223 on all three machines
/// and recorded a successful refresh — while `~/.yggterm/npm/bin` was not on the
/// login PATH at all on guihost or oc, so every session there ran a SEPARATE
/// `~/.local/bin/claude` stuck at 2.1.220. The refresh was verifying its own
/// copy, so it could not see this, and reported success for a month.
///
/// [[spec-cli-binary-auto-provisioning]] already names login-shell resolution as
/// the SSOT for "what will actually run"; this is that clause enforced, and the
/// owner's ask (2026-08-06) that a machine which cannot keep its CLIs current
/// SAY SO in telemetry rather than go quietly stale.
fn report_managed_cli_effective_version_drift(
    home: &Path,
    probes: &[(ManagedCliTool, ToolProbe)],
) {
    for (tool, probe) in probes {
        let binary_name = tool.binary_name();
        let managed_version = match (&probe.version, probe.source) {
            (Some(version), Some(ManagedCliBinarySource::Managed)) => version.clone(),
            _ => continue,
        };
        let Some((resolved_path, resolved_version)) = login_shell_resolved_cli(binary_name) else {
            append_trace_event(
                home,
                "server",
                "managed_cli",
                "effective_cli_unresolvable",
                serde_json::json!({
                    "tool": binary_name,
                    "managed_version": managed_version,
                    "detail": "the login shell resolves no such binary, so a session \
                               cannot launch this CLI on this machine",
                }),
            );
            continue;
        };
        let drifted = resolved_version
            .as_deref()
            .map(|resolved| resolved != managed_version)
            .unwrap_or(true);
        if !drifted {
            continue;
        }
        append_trace_event(
            home,
            "server",
            "managed_cli",
            "effective_cli_version_drift",
            serde_json::json!({
                "tool": binary_name,
                "managed_version": managed_version,
                "effective_version": resolved_version,
                "effective_path": resolved_path,
                "detail": "the refresh updated the managed copy, but the login shell \
                           resolves a DIFFERENT install — sessions on this machine run \
                           the version reported as effective_version, not managed_version",
            }),
        );
    }
}

fn record_managed_cli_probe_span(
    home: &Path,
    name: &str,
    probes: &[(ManagedCliTool, ToolProbe)],
    phase: &str,
) {
    let perf = PerfSpan::start(home, "cli", name);
    perf.finish(serde_json::json!({
        "phase": phase,
        "tools": probes
            .iter()
            .map(|(tool, probe)| serde_json::json!({
                "tool": tool.binary_name(),
                "available": probe.available,
                "source": probe.source,
                "version": probe.version,
            }))
            .collect::<Vec<_>>(),
    }));
    for (tool, probe) in probes {
        let tool_perf = PerfSpan::start(home, "cli", &format!("refresh_managed_{}_probe", tool.binary_name()));
        tool_perf.finish(serde_json::json!({
            "phase": phase,
            "tool": tool.binary_name(),
            "available": probe.available,
            "source": probe.source,
            "version": probe.version,
        }));
    }
}


fn managed_cli_refresh_skip_remaining_ms(
    before: &[(ManagedCliTool, ToolProbe)],
    state: &ManagedCliRefreshState,
    now_ms: u64,
    ttl_ms: u64,
) -> Option<u64> {
    let refreshed_at_ms = state.last_successful_refresh_ms?;
    let age_ms = now_ms.saturating_sub(refreshed_at_ms);
    if age_ms >= ttl_ms {
        return None;
    }
    for (tool, probe) in before {
        if !probe.available || probe.source != Some(ManagedCliBinarySource::Managed) {
            return None;
        }
        let Some(version) = probe.version.as_ref() else {
            return None;
        };
        if state.managed_versions.get(tool.binary_name()) != Some(version) {
            return None;
        }
    }
    Some(ttl_ms.saturating_sub(age_ms))
}

fn managed_cli_refresh_state_from_probes(
    probes: &[(ManagedCliTool, ToolProbe)],
    refreshed_at_ms: u64,
) -> ManagedCliRefreshState {
    let managed_versions = probes
        .iter()
        .filter_map(|(tool, probe)| {
            (probe.source == Some(ManagedCliBinarySource::Managed))
                .then_some(
                    probe
                        .version
                        .as_ref()
                        .map(|version| (tool.binary_name().to_string(), version.clone())),
                )
                .flatten()
        })
        .collect::<BTreeMap<_, _>>();
    ManagedCliRefreshState {
        last_successful_refresh_ms: Some(refreshed_at_ms),
        managed_versions,
    }
}

fn managed_cli_refresh_skip_detail(
    tool: ManagedCliTool,
    ttl_remaining_ms: u64,
    ttl_ms: u64,
) -> String {
    format!(
        "Skipped {} refresh because Yggterm refreshed the managed toolchain recently. About {}s remain in the {}s refresh window.",
        tool.display_name(),
        ttl_remaining_ms / 1000,
        ttl_ms / 1000,
    )
}

fn managed_cli_has_existing_managed_install(probes: &[(ManagedCliTool, ToolProbe)]) -> bool {
    probes
        .iter()
        .any(|(_, probe)| probe.source == Some(ManagedCliBinarySource::Managed))
}

/// Whether the very FIRST managed install on a machine waits for an explicit
/// launch. Incidental refreshes defer it; a scheduled sweep does not, because a
/// machine the owner has not opened in a week is exactly the one the sweep
/// exists to provision.
fn managed_cli_should_defer_initial_install(
    mode: ManagedCliRefreshMode,
    probes: &[(ManagedCliTool, ToolProbe)],
) -> bool {
    mode.defers_installs() && !managed_cli_has_existing_managed_install(probes)
}

fn managed_cli_background_install_enabled() -> bool {
    env::var(MANAGED_CLI_BACKGROUND_INSTALL_ENV)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn managed_cli_refresh_should_attempt_install(
    mode: ManagedCliRefreshMode,
    provisioner_available: bool,
    skipped_recently: bool,
    install_deferred: bool,
    background_install_enabled: bool,
) -> bool {
    provisioner_available
        && !skipped_recently
        && !install_deferred
        && (!mode.defers_installs() || background_install_enabled)
}

fn managed_cli_deferred_install_detail(tool: ManagedCliTool, probe: &ToolProbe) -> String {
    if probe.source == Some(ManagedCliBinarySource::System) && probe.available {
        format!(
            "{} is currently available from PATH. Yggterm deferred the first managed install until you explicitly launch or resume a local {} session.",
            tool.display_name(),
            tool.display_name(),
        )
    } else {
        format!(
            "Yggterm deferred the first managed {} install until you explicitly launch or resume a local {} session.",
            tool.display_name(),
            tool.display_name(),
        )
    }
}

fn managed_cli_deferred_background_install_detail(
    tool: ManagedCliTool,
    probe: &ToolProbe,
) -> String {
    if probe.available {
        format!(
            "{} is already available. Background refresh only probes by default; set {}=1 for an explicit unattended managed update.",
            tool.display_name(),
            MANAGED_CLI_BACKGROUND_INSTALL_ENV,
        )
    } else {
        format!(
            "{} is not installed. Background refresh will not run npm install unless {}=1 is set.",
            tool.display_name(),
            MANAGED_CLI_BACKGROUND_INSTALL_ENV,
        )
    }
}

fn managed_cli_explicit_refresh_needed(
    tool: ManagedCliTool,
    probe: &ToolProbe,
    refresh_state: &ManagedCliRefreshState,
    now_ms: u64,
    ttl_ms: u64,
) -> bool {
    if probe.source != Some(ManagedCliBinarySource::Managed) {
        return true;
    }
    managed_cli_refresh_skip_remaining_ms(&[(tool, probe.clone())], refresh_state, now_ms, ttl_ms)
        .is_none()
}

fn persist_managed_cli_refresh_state(
    home: &Path,
    probes: &[(ManagedCliTool, ToolProbe)],
    refreshed_at_ms: u64,
) -> Result<()> {
    let state = managed_cli_refresh_state_from_probes(probes, refreshed_at_ms);
    if state.managed_versions.is_empty() {
        return Ok(());
    }
    save_managed_cli_refresh_state(home, &state)
}

fn normalize_terminal_appearance(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "dark" => Some("dark"),
        "light" => Some("light"),
        _ => None,
    }
}

fn ambient_terminal_appearance() -> String {
    env::var(ENV_YGGTERM_TERMINAL_APPEARANCE)
        .ok()
        .and_then(|value| normalize_terminal_appearance(&value).map(str::to_string))
        .or_else(|| {
            env::var("YGGTERM_APPEARANCE")
                .ok()
                .and_then(|value| normalize_terminal_appearance(&value).map(str::to_string))
        })
        .or_else(|| {
            env::var("COLORFGBG").ok().and_then(|value| {
                let mut parts = value.split(';').map(str::trim);
                let foreground = parts.next()?;
                let background = parts.next()?;
                if foreground == "15" && background == "0" {
                    Some("dark".to_string())
                } else if foreground == "0" && background == "15" {
                    Some("light".to_string())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "light".to_string())
}

pub(crate) fn terminal_identity_appearance_from_environment() -> String {
    ambient_terminal_appearance()
}

fn colorfgbg_for_appearance(appearance: &str) -> &'static str {
    match appearance {
        "dark" => "15;0",
        _ => "0;15",
    }
}

fn terminal_identity_env_pairs_for_appearance_with_home(
    appearance: &str,
    include_yggterm_home: bool,
) -> Vec<(&'static str, String)> {
    let appearance = normalize_terminal_appearance(appearance)
        .unwrap_or("light")
        .to_string();
    let mut pairs = vec![
        ("TERM", "xterm-256color".to_string()),
        ("COLORTERM", "truecolor".to_string()),
        // Codex already knows how to style itself well inside VS Code's terminal surface.
        // Keep a Yggterm-specific identity alongside that so our own integrations stay explicit.
        ("TERM_PROGRAM", EXPORTED_TERM_PROGRAM.to_string()),
        (
            "TERM_PROGRAM_VERSION",
            YGGTERM_TERM_PROGRAM_VERSION.to_string(),
        ),
        ("YGGTERM_TERM_PROGRAM", YGGTERM_TERM_PROGRAM.to_string()),
        ("YGGTERM_APPEARANCE", appearance.clone()),
        (ENV_YGGTERM_TERMINAL_APPEARANCE, appearance.clone()),
        (
            "COLORFGBG",
            colorfgbg_for_appearance(&appearance).to_string(),
        ),
    ];
    if include_yggterm_home && let Ok(home) = env::var(ENV_YGGTERM_HOME) {
        if !home.trim().is_empty() {
            pairs.push((ENV_YGGTERM_HOME, home));
        }
    }
    pairs.extend(terminal_identity_color_env_pairs_from_environment());
    pairs
}

fn terminal_identity_color_env_pairs_from_environment() -> Vec<(&'static str, String)> {
    let Some(profile) = terminal_identity_color_profile_from_environment() else {
        return Vec::new();
    };
    terminal_identity_color_env_pairs(&profile)
}

pub(crate) fn terminal_identity_color_env_pairs(
    profile: &TerminalIdentityColorProfile,
) -> Vec<(&'static str, String)> {
    let Some(palette) = normalized_terminal_identity_palette(profile) else {
        return Vec::new();
    };
    let Some(foreground) = normalize_terminal_identity_color(&profile.foreground) else {
        return Vec::new();
    };
    let Some(background) = normalize_terminal_identity_color(&profile.background) else {
        return Vec::new();
    };
    let mut pairs = Vec::with_capacity(18);
    pairs.push((ENV_YGGTERM_TERMINAL_COLOR_FOREGROUND, foreground));
    pairs.push((ENV_YGGTERM_TERMINAL_COLOR_BACKGROUND, background));
    for (key, value) in ENV_YGGTERM_TERMINAL_COLOR_PALETTE
        .iter()
        .copied()
        .zip(palette)
    {
        pairs.push((key, value));
    }
    pairs
}

fn normalized_terminal_identity_palette(
    profile: &TerminalIdentityColorProfile,
) -> Option<Vec<String>> {
    if profile.palette.len() != ENV_YGGTERM_TERMINAL_COLOR_PALETTE.len() {
        return None;
    }
    profile
        .palette
        .iter()
        .map(|value| normalize_terminal_identity_color(value))
        .collect()
}

pub(crate) fn normalize_terminal_identity_color(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if hex.len() != 6 || !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    Some(format!("#{hex}").to_ascii_lowercase())
}

pub(crate) fn terminal_identity_color_profile_from_environment()
-> Option<TerminalIdentityColorProfile> {
    let foreground =
        normalize_terminal_identity_color(&env::var(ENV_YGGTERM_TERMINAL_COLOR_FOREGROUND).ok()?)?;
    let background =
        normalize_terminal_identity_color(&env::var(ENV_YGGTERM_TERMINAL_COLOR_BACKGROUND).ok()?)?;
    let palette = ENV_YGGTERM_TERMINAL_COLOR_PALETTE
        .iter()
        .map(|key| {
            env::var(key)
                .ok()
                .and_then(|value| normalize_terminal_identity_color(&value))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(TerminalIdentityColorProfile {
        foreground,
        background,
        palette,
    })
}

fn terminal_identity_env_pairs_with_home(
    include_yggterm_home: bool,
) -> Vec<(&'static str, String)> {
    terminal_identity_env_pairs_for_appearance_with_home(
        &ambient_terminal_appearance(),
        include_yggterm_home,
    )
}

pub(crate) fn terminal_identity_env_pairs() -> Vec<(&'static str, String)> {
    terminal_identity_env_pairs_with_home(true)
}

pub(crate) fn terminal_identity_env_removals() -> &'static [&'static str] {
    TERMINAL_IDENTITY_ENV_REMOVALS
}

pub(crate) fn terminal_identity_shell_exports() -> Vec<String> {
    terminal_identity_env_removals()
        .iter()
        .map(|key| format!("unset {key}"))
        .chain(
            terminal_identity_env_pairs()
                .into_iter()
                .map(|(key, value)| format!("export {key}={}", shell_single_quote(&value))),
        )
        .collect()
}

pub(crate) fn terminal_identity_shell_exports_for_appearance(appearance: &str) -> Vec<String> {
    terminal_identity_env_removals()
        .iter()
        .map(|key| format!("unset {key}"))
        .chain(
            terminal_identity_env_pairs_for_appearance_with_home(appearance, true)
                .into_iter()
                .map(|(key, value)| format!("export {key}={}", shell_single_quote(&value))),
        )
        .collect()
}

pub(crate) fn terminal_identity_shell_exports_for_remote() -> Vec<String> {
    terminal_identity_env_removals()
        .iter()
        .map(|key| format!("unset {key}"))
        .chain(
            terminal_identity_env_pairs_with_home(false)
                .into_iter()
                .map(|(key, value)| format!("export {key}={}", shell_single_quote(&value))),
        )
        .collect()
}

pub fn sync_terminal_identity_appearance(appearance: &str) {
    sync_terminal_identity_appearance_with_profile(appearance, None);
}

pub fn sync_terminal_identity_appearance_with_profile(
    appearance: &str,
    profile: Option<&TerminalIdentityColorProfile>,
) {
    let appearance = normalize_terminal_appearance(appearance).unwrap_or("light");
    let profile = profile
        .cloned()
        .or_else(terminal_identity_color_profile_from_environment);
    // The daemon owns terminal launch commands and needs a process-wide identity for child PTYs
    // and remote shell command synthesis. This is updated on startup/theme changes only.
    unsafe {
        for key in terminal_identity_env_removals() {
            env::remove_var(key);
        }
        env::remove_var(ENV_YGGTERM_TERMINAL_COLOR_FOREGROUND);
        env::remove_var(ENV_YGGTERM_TERMINAL_COLOR_BACKGROUND);
        for key in ENV_YGGTERM_TERMINAL_COLOR_PALETTE {
            env::remove_var(key);
        }
        env::set_var("TERM", "xterm-256color");
        env::set_var("COLORTERM", "truecolor");
        env::set_var("TERM_PROGRAM", EXPORTED_TERM_PROGRAM);
        env::set_var("TERM_PROGRAM_VERSION", YGGTERM_TERM_PROGRAM_VERSION);
        env::set_var("YGGTERM_TERM_PROGRAM", YGGTERM_TERM_PROGRAM);
        env::set_var("YGGTERM_APPEARANCE", appearance);
        env::set_var(ENV_YGGTERM_TERMINAL_APPEARANCE, appearance);
        env::set_var("COLORFGBG", colorfgbg_for_appearance(appearance));
        if let Some(profile) = profile.as_ref() {
            for (key, value) in terminal_identity_color_env_pairs(profile) {
                env::set_var(key, value);
            }
        }
    }
}

pub(crate) fn sync_terminal_identity_env(theme: UiTheme) {
    let appearance = env::var(ENV_YGGTERM_TERMINAL_APPEARANCE)
        .ok()
        .and_then(|value| normalize_terminal_appearance(&value).map(str::to_string))
        .unwrap_or_else(|| {
            match theme {
                UiTheme::ZedLight => "light",
                UiTheme::ZedDark => "dark",
            }
            .to_string()
        });
    sync_terminal_identity_appearance(&appearance);
}

/// Serializes every test that touches the process-global terminal identity.
///
/// This module OWNS that env (`sync_terminal_identity_appearance_with_profile`
/// writes `TERM*`, `YGGTERM_APPEARANCE`, `COLORFGBG` and the 18 palette keys as
/// process-wide state, because the daemon needs one identity for every child
/// PTY it spawns). cargo runs tests in parallel threads of ONE process, so any
/// test that reads or writes that env must serialize against every other one —
/// including tests in OTHER modules, which is why this guard is `pub(crate)`
/// rather than private to the test module below.
///
/// It did not used to be. `agent_arm_matrix::locality_does_not_fork_the_invocation`
/// compares two commands built from identical arguments, and it read the palette
/// out of this env while these tests were clearing it, so it failed roughly
/// whenever the scheduler interleaved them on a host that HAD a palette set —
/// i.e. inside a yggterm session, which is where agents run. Poison is tolerated
/// (`into_inner`) so one panicking test does not cascade-fail the rest.
#[cfg(test)]
pub(crate) fn env_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ENV_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A SECOND mutex over the same env is the same thing as no mutex at all, and
/// the crate carried one for long enough to make a shipped harness flaky: tests
/// holding `codex_cli`'s lock ran concurrently with tests holding `lib.rs`'s
/// `TERMINAL_IDENTITY_TEST_LOCK`, cleared each other's palette, and
/// `agent_arm_matrix::locality_does_not_fork_the_invocation` reported a locality
/// fork that did not exist.
///
/// Enumerating the guards by hand is what let the second one survive, so this
/// scans the source instead. It is deliberately a scan and not a convention:
/// the same shape guards the helper-textarea focus sites in the shell, for the
/// same reason — a rule nobody can forget beats a rule everybody agrees with.
#[cfg(test)]
#[test]
fn the_terminal_identity_env_has_exactly_one_test_guard() {
    let crate_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders: Vec<String> = Vec::new();

    let mut stack = vec![crate_src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read crate src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source");
            for (index, line) in text.lines().enumerate() {
                // A `static … Mutex<()>` whose name mentions the identity env is
                // a rival guard. `env_test_guard`'s own ENV_TEST_LOCK is the one
                // legitimate declaration, and it is matched by name below.
                let declares_a_lock = line.contains("Mutex<()>") && line.contains("static ");
                let names_the_env = line.contains("TERMINAL_IDENTITY") || line.contains("APPEARANCE");
                if declares_a_lock && names_the_env {
                    offenders.push(format!(
                        "{}:{} — {}",
                        path.display(),
                        index + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the terminal-identity env must have exactly one test guard \
         (codex_cli::env_test_guard); these declare a rival lock, which \
         serializes nothing:\n{}",
        offenders.join("\n"),
    );

    // The scan must be able to FIND something, or it is a lock that can only
    // pass — this crate has shipped one of those before. Prove the traversal
    // reaches real source by requiring the guard's own declaration.
    let this_file = crate_src.join("managed_cli/mod.rs");
    let text = std::fs::read_to_string(&this_file).expect("read managed_cli/mod.rs");
    assert!(
        text.contains("static ENV_TEST_LOCK: std::sync::Mutex<()>"),
        "the scan did not find env_test_guard's own lock, so it is not reading \
         the source it claims to police (looked in managed_cli/mod.rs)",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::env_test_guard;

    #[test]
    fn metadata_subprocess_timeout_never_waits_for_the_child_reaper() {
        let started = Instant::now();
        let outcome = bounded_command_output(
            Command::new("sh").args(["-c", "sleep 5"]),
            Duration::from_millis(75),
        );
        assert_eq!(outcome, BoundedCommandOutput::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the caller must not join the detached child reaper"
        );
    }

    /// ⛔ The regression this pins is DESTRUCTIVE, not merely redundant: two
    /// `npm install -g` runs against one prefix were measured deleting the CLI
    /// they were both installing, leaving `opencode` absent from the managed bin
    /// dir entirely. See [`ManagedCliInstallLock`] for the measurement.
    ///
    /// ⚠ Same-process acquisition is a REAL test of the cross-process contract
    /// here: `flock` is owned by the open file description, and each acquire
    /// opens the lock file afresh, so two guards in one process contend exactly
    /// as two daemons would.
    #[cfg(unix)]
    #[test]
    fn one_writer_at_a_time_owns_this_machines_managed_toolchain() {
        let home = std::env::temp_dir().join(format!(
            "yggterm-managed-cli-lock-test-{}",
            uuid::Uuid::new_v4()
        ));

        let held = acquire_managed_cli_install_lock_waiting(&home, 0)
            .expect("the first writer takes the toolchain lock");

        // A second writer must NOT proceed into the install while the first
        // holds it — the whole defect was both writers proceeding.
        let refused = acquire_managed_cli_install_lock_waiting(&home, 200);
        let message = format!("{:#}", refused.expect_err("a concurrent install must be refused"));
        assert!(
            message.contains("installing managed CLIs"),
            "the refusal must say a concurrent install is why, got: {message}"
        );

        // ...and the lock must be released by the kernel when the guard drops,
        // or one crashed install would wedge provisioning until reboot.
        drop(held);
        acquire_managed_cli_install_lock_waiting(&home, 0)
            .expect("the lock is released when the guard drops");

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn managed_cli_focus_cache_reuses_recent_available_probe() {
        let now = 1_000_000_u64;
        // Available + within TTL → reuse (no re-probe subprocess).
        assert!(managed_cli_focus_cache_entry_is_fresh(true, now, now));
        assert!(managed_cli_focus_cache_entry_is_fresh(
            true,
            now,
            now + MANAGED_CLI_FOCUS_PROBE_TTL_MS - 1
        ));
        // Expired → re-probe.
        assert!(!managed_cli_focus_cache_entry_is_fresh(
            true,
            now,
            now + MANAGED_CLI_FOCUS_PROBE_TTL_MS
        ));
        // Not-available is never reused, so a missing tool keeps re-trying install.
        assert!(!managed_cli_focus_cache_entry_is_fresh(false, now, now));
        // Clock skew (now < cached_at) is treated as fresh, never underflows.
        assert!(managed_cli_focus_cache_entry_is_fresh(true, now, now - 5));
    }

    #[test]
    fn terminal_identity_shell_exports_unset_no_color() {
        let exports = terminal_identity_shell_exports();
        assert_eq!(exports.first().map(String::as_str), Some("unset NO_COLOR"));
    }

    #[test]
    fn sync_terminal_identity_env_removes_no_color() {
        let _env = env_test_guard();
        let previous = env::var_os("NO_COLOR");
        unsafe {
            env::set_var("NO_COLOR", "1");
        }
        sync_terminal_identity_env(UiTheme::ZedLight);
        assert!(env::var_os("NO_COLOR").is_none());
        match previous {
            Some(value) => unsafe { env::set_var("NO_COLOR", value) },
            None => unsafe { env::remove_var("NO_COLOR") },
        }
    }

    #[test]
    fn sync_terminal_identity_env_preserves_explicit_terminal_appearance() {
        let _env = env_test_guard();
        let previous_terminal = env::var_os(ENV_YGGTERM_TERMINAL_APPEARANCE);
        let previous_shell = env::var_os("YGGTERM_APPEARANCE");
        let previous_colorfgbg = env::var_os("COLORFGBG");
        unsafe {
            env::set_var(ENV_YGGTERM_TERMINAL_APPEARANCE, "dark");
            env::set_var("YGGTERM_APPEARANCE", "light");
            env::set_var("COLORFGBG", "0;15");
        }

        sync_terminal_identity_env(UiTheme::ZedLight);

        assert_eq!(
            env::var(ENV_YGGTERM_TERMINAL_APPEARANCE).as_deref(),
            Ok("dark")
        );
        assert_eq!(env::var("COLORFGBG").as_deref(), Ok("15;0"));

        match previous_terminal {
            Some(value) => unsafe { env::set_var(ENV_YGGTERM_TERMINAL_APPEARANCE, value) },
            None => unsafe { env::remove_var(ENV_YGGTERM_TERMINAL_APPEARANCE) },
        }
        match previous_shell {
            Some(value) => unsafe { env::set_var("YGGTERM_APPEARANCE", value) },
            None => unsafe { env::remove_var("YGGTERM_APPEARANCE") },
        }
        match previous_colorfgbg {
            Some(value) => unsafe { env::set_var("COLORFGBG", value) },
            None => unsafe { env::remove_var("COLORFGBG") },
        }
    }

    #[test]
    fn terminal_identity_prefers_terminal_appearance_over_shell_appearance() {
        let _env = env_test_guard();
        let previous_terminal = env::var_os(ENV_YGGTERM_TERMINAL_APPEARANCE);
        let previous_shell = env::var_os("YGGTERM_APPEARANCE");
        unsafe {
            env::set_var(ENV_YGGTERM_TERMINAL_APPEARANCE, "dark");
            env::set_var("YGGTERM_APPEARANCE", "light");
        }

        let pairs = terminal_identity_env_pairs_with_home(false);
        assert_eq!(
            pairs
                .iter()
                .find(|(key, _)| *key == "COLORFGBG")
                .map(|(_, value)| value.as_str()),
            Some("15;0")
        );
        assert_eq!(
            pairs
                .iter()
                .find(|(key, _)| *key == ENV_YGGTERM_TERMINAL_APPEARANCE)
                .map(|(_, value)| value.as_str()),
            Some("dark")
        );

        match previous_terminal {
            Some(value) => unsafe { env::set_var(ENV_YGGTERM_TERMINAL_APPEARANCE, value) },
            None => unsafe { env::remove_var(ENV_YGGTERM_TERMINAL_APPEARANCE) },
        }
        match previous_shell {
            Some(value) => unsafe { env::set_var("YGGTERM_APPEARANCE", value) },
            None => unsafe { env::remove_var("YGGTERM_APPEARANCE") },
        }
    }

    #[test]
    fn terminal_identity_falls_back_to_colorfgbg_when_yggterm_vars_missing() {
        let _env = env_test_guard();
        let previous_terminal = env::var_os(ENV_YGGTERM_TERMINAL_APPEARANCE);
        let previous_shell = env::var_os("YGGTERM_APPEARANCE");
        let previous_colorfgbg = env::var_os("COLORFGBG");
        unsafe {
            env::remove_var(ENV_YGGTERM_TERMINAL_APPEARANCE);
            env::remove_var("YGGTERM_APPEARANCE");
            env::set_var("COLORFGBG", "15;0");
        }

        let pairs = terminal_identity_env_pairs_with_home(false);
        assert_eq!(
            pairs
                .iter()
                .find(|(key, _)| *key == ENV_YGGTERM_TERMINAL_APPEARANCE)
                .map(|(_, value)| value.as_str()),
            Some("dark")
        );
        assert_eq!(
            pairs
                .iter()
                .find(|(key, _)| *key == "COLORFGBG")
                .map(|(_, value)| value.as_str()),
            Some("15;0")
        );

        match previous_terminal {
            Some(value) => unsafe { env::set_var(ENV_YGGTERM_TERMINAL_APPEARANCE, value) },
            None => unsafe { env::remove_var(ENV_YGGTERM_TERMINAL_APPEARANCE) },
        }
        match previous_shell {
            Some(value) => unsafe { env::set_var("YGGTERM_APPEARANCE", value) },
            None => unsafe { env::remove_var("YGGTERM_APPEARANCE") },
        }
        match previous_colorfgbg {
            Some(value) => unsafe { env::set_var("COLORFGBG", value) },
            None => unsafe { env::remove_var("COLORFGBG") },
        }
    }

    #[test]
    fn managed_cli_recent_refresh_skip_requires_fresh_managed_versions() {
        let ttl_ms = managed_cli_refresh_ttl_ms();
        let now_ms = 10_000u64;
        let before = vec![
            (
                ManagedCliTool::Codex,
                ToolProbe {
                    version: Some("1.2.3".to_string()),
                    source: Some(ManagedCliBinarySource::Managed),
                    available: true,
                },
            ),
            (
                ManagedCliTool::CodexLiteLlm,
                ToolProbe {
                    version: Some("4.5.6".to_string()),
                    source: Some(ManagedCliBinarySource::Managed),
                    available: true,
                },
            ),
        ];
        let state = ManagedCliRefreshState {
            last_successful_refresh_ms: Some(now_ms.saturating_sub(1_000)),
            managed_versions: BTreeMap::from([
                ("codex".to_string(), "1.2.3".to_string()),
                ("codex-litellm".to_string(), "4.5.6".to_string()),
            ]),
        };
        let remaining_ms = managed_cli_refresh_skip_remaining_ms(&before, &state, now_ms, ttl_ms);
        assert_eq!(remaining_ms, Some(ttl_ms.saturating_sub(1_000)));
    }

    #[test]
    fn managed_cli_recent_refresh_skip_rejects_system_or_stale_tools() {
        let ttl_ms = managed_cli_refresh_ttl_ms();
        let now_ms = 10_000u64;
        let stale_state = ManagedCliRefreshState {
            last_successful_refresh_ms: Some(now_ms.saturating_sub(1_000)),
            managed_versions: BTreeMap::from([
                ("codex".to_string(), "1.2.2".to_string()),
                ("codex-litellm".to_string(), "4.5.6".to_string()),
            ]),
        };
        let system_before = vec![
            (
                ManagedCliTool::Codex,
                ToolProbe {
                    version: Some("1.2.3".to_string()),
                    source: Some(ManagedCliBinarySource::System),
                    available: true,
                },
            ),
            (
                ManagedCliTool::CodexLiteLlm,
                ToolProbe {
                    version: Some("4.5.6".to_string()),
                    source: Some(ManagedCliBinarySource::Managed),
                    available: true,
                },
            ),
        ];
        let managed_before = vec![
            (
                ManagedCliTool::Codex,
                ToolProbe {
                    version: Some("1.2.3".to_string()),
                    source: Some(ManagedCliBinarySource::Managed),
                    available: true,
                },
            ),
            (
                ManagedCliTool::CodexLiteLlm,
                ToolProbe {
                    version: Some("4.5.6".to_string()),
                    source: Some(ManagedCliBinarySource::Managed),
                    available: true,
                },
            ),
        ];
        assert_eq!(
            managed_cli_refresh_skip_remaining_ms(&system_before, &stale_state, now_ms, ttl_ms),
            None
        );
        assert_eq!(
            managed_cli_refresh_skip_remaining_ms(&managed_before, &stale_state, now_ms, ttl_ms),
            None
        );
    }

    #[test]
    fn managed_cli_initial_background_refresh_defers_until_managed_install_exists() {
        let system_only = vec![
            (
                ManagedCliTool::Codex,
                ToolProbe {
                    version: Some("1.2.3".to_string()),
                    source: Some(ManagedCliBinarySource::System),
                    available: true,
                },
            ),
            (
                ManagedCliTool::CodexLiteLlm,
                ToolProbe {
                    version: None,
                    source: None,
                    available: false,
                },
            ),
        ];
        let managed_present = vec![(
            ManagedCliTool::Codex,
            ToolProbe {
                version: Some("1.2.3".to_string()),
                source: Some(ManagedCliBinarySource::Managed),
                available: true,
            },
        )];
        assert!(managed_cli_should_defer_initial_install(
            ManagedCliRefreshMode::Incidental,
            &system_only
        ));
        assert!(!managed_cli_should_defer_initial_install(
            ManagedCliRefreshMode::Foreground,
            &system_only
        ));
        assert!(!managed_cli_should_defer_initial_install(
            ManagedCliRefreshMode::Incidental,
            &managed_present
        ));
        // The scheduled sweep exists to provision the machine nobody has
        // opened. Deferring its FIRST install would be deferring the whole
        // point of it — a machine with no managed install would never get one.
        assert!(!managed_cli_should_defer_initial_install(
            ManagedCliRefreshMode::Scheduled,
            &system_only
        ));
    }

    #[test]
    fn background_managed_cli_refresh_requires_install_opt_in() {
        use ManagedCliRefreshMode::{Foreground, Incidental};
        assert!(!managed_cli_refresh_should_attempt_install(
            Incidental, true, false, false, false
        ));
        assert!(managed_cli_refresh_should_attempt_install(
            Incidental, true, false, false, true
        ));
        assert!(managed_cli_refresh_should_attempt_install(
            Foreground, true, false, false, false
        ));
        assert!(!managed_cli_refresh_should_attempt_install(
            Incidental, true, true, false, true
        ));
        assert!(!managed_cli_refresh_should_attempt_install(
            Incidental, true, false, true, true
        ));
    }

    /// ⭐ The load-bearing property of the third mode, and the reason it exists
    /// rather than a third `bool`: the scheduled sweep must be TTL-gated (so a
    /// redundant fan-out from a second daemon costs a probe, not an install)
    /// and must still INSTALL (so a machine the owner has not opened in a week
    /// actually gets its CLIs). No setting of the old `background: bool` gives
    /// both — `true` forbids the install, `false` throws away the TTL.
    #[test]
    fn a_scheduled_refresh_keeps_the_ttl_and_still_installs() {
        use ManagedCliRefreshMode::{Foreground, Incidental, Scheduled};

        assert!(Scheduled.respects_ttl(), "the sweep is paced by the TTL");
        assert!(
            !Scheduled.defers_installs(),
            "a sweep that installs nothing is not an install pipeline"
        );
        assert!(Incidental.respects_ttl() && Incidental.defers_installs());
        assert!(!Foreground.respects_ttl() && !Foreground.defers_installs());

        // With no opt-in env — the default on every machine — the scheduled
        // sweep installs and the incidental refresh does not.
        assert!(managed_cli_refresh_should_attempt_install(
            Scheduled, true, false, false, false
        ));
        assert!(!managed_cli_refresh_should_attempt_install(
            Incidental, true, false, false, false
        ));
        // …and a TTL skip still stops the sweep, which is what keeps a
        // redundant fan-out cheap.
        assert!(!managed_cli_refresh_should_attempt_install(
            Scheduled, true, true, false, false
        ));
    }

    /// A remote binary older than this change compares `args[3] == "background"`
    /// and has never heard of `scheduled`. It must not accidentally read as the
    /// deferring arm, and the word must survive its own round trip.
    #[test]
    fn the_refresh_mode_wire_word_round_trips_and_degrades_to_foreground() {
        use ManagedCliRefreshMode as Mode;
        for mode in [Mode::Foreground, Mode::Incidental, Mode::Scheduled] {
            assert_eq!(Mode::from_wire_word(mode.as_str()), mode, "{mode:?}");
        }
        assert_eq!(Mode::from_wire_word("nonsense"), Mode::Foreground);
        assert_eq!(Mode::from_wire_word(""), Mode::Foreground);
        // An old remote binary sees a word that is not "background", so it runs
        // its foreground arm: it installs and ignores its TTL. That is a
        // DEGRADED sweep, never a silent no-op.
        assert_ne!(Mode::Scheduled.as_str(), "background");
    }

    #[test]
    fn explicit_managed_cli_ensure_refreshes_system_or_stale_managed_tools() {
        let now_ms = 10_000u64;
        let ttl_ms = managed_cli_refresh_ttl_ms();
        let state = ManagedCliRefreshState {
            last_successful_refresh_ms: Some(now_ms.saturating_sub(1_000)),
            managed_versions: BTreeMap::from([("codex".to_string(), "1.2.3".to_string())]),
        };
        let system_probe = ToolProbe {
            version: Some("1.2.3".to_string()),
            source: Some(ManagedCliBinarySource::System),
            available: true,
        };
        let fresh_managed_probe = ToolProbe {
            version: Some("1.2.3".to_string()),
            source: Some(ManagedCliBinarySource::Managed),
            available: true,
        };
        let stale_managed_probe = ToolProbe {
            version: Some("1.2.2".to_string()),
            source: Some(ManagedCliBinarySource::Managed),
            available: true,
        };

        assert!(managed_cli_explicit_refresh_needed(
            ManagedCliTool::Codex,
            &system_probe,
            &state,
            now_ms,
            ttl_ms,
        ));
        assert!(!managed_cli_explicit_refresh_needed(
            ManagedCliTool::Codex,
            &fresh_managed_probe,
            &state,
            now_ms,
            ttl_ms,
        ));
        assert!(managed_cli_explicit_refresh_needed(
            ManagedCliTool::Codex,
            &stale_managed_probe,
            &state,
            now_ms,
            ttl_ms,
        ));
    }

    #[test]
    fn managed_cli_probe_report_uses_system_binary_without_refresh_action() {
        let status = managed_cli_launch_status_from_probe(
            ManagedCliTool::Codex,
            ToolProbe {
                version: Some("1.2.3".to_string()),
                source: Some(ManagedCliBinarySource::System),
                available: true,
            },
        );

        assert_eq!(status.action, "system_fallback");
        assert!(!status.changed);
        assert!(status.available);
        assert_eq!(
            summarize_managed_cli_report(
                "local",
                &ManagedCliRefreshReport {
                    scope: "local".to_string(),
                    background: true,
                    statuses: vec![status],
                    skipped_recently: false,
                    ttl_remaining_ms: None,
                    install_attempted: false,
                    install_deferred: false,
                },
            ),
            "local: using existing PATH Codex binaries until explicit managed refresh"
        );
    }

    // The focus/attach path's cheap probe must recognize a present managed binary
    /// The drift check compares a bare managed version against a decorated
    /// `--version` line, so the extractor is the whole correctness of it: too
    /// greedy and every machine reports permanent false drift, too strict and
    /// the real drift (2.1.223 managed vs 2.1.220 effective, measured on guihost
    /// and oc 2026-08-06) reads as agreement.
    #[test]
    fn effective_version_is_extracted_from_each_clis_decorated_version_line() {
        assert_eq!(
            extract_semver_like_version("2.1.223 (Claude Code)"),
            Some("2.1.223")
        );
        assert_eq!(
            extract_semver_like_version("codex-cli 0.144.6"),
            Some("0.144.6")
        );
        // The package name must not be mistaken for a version just because it
        // contains dots.
        assert_eq!(
            extract_semver_like_version("@anthropic-ai/claude-code 2.1.223"),
            Some("2.1.223")
        );
        assert_eq!(extract_semver_like_version("no version here"), None);
        assert_eq!(extract_semver_like_version(""), None);
        // Two-component versions are not semver-like enough to compare against.
        assert_eq!(extract_semver_like_version("tool 1.2"), None);
    }

    /// ⛔⛔ THE MOCK-REGISTRY HARNESS: every install shape the direct fetcher
    /// must handle, proven against a local mock npm registry — install AND
    /// update, and every way a vendor package says "my binary is not ready".
    ///
    /// This exists because the fleet ran the real registry's packages through
    /// a fetcher that extracted tarballs and ran NOTHING, and three of ten
    /// CLIs shipped as vendor error shims: present on PATH,
    /// launch-parity-resolvable, and dead on first use ("native binary not
    /// installed", "postinstall was not run", "compiled binary not found").
    /// The shapes below are those vendors' shapes, minimized:
    ///
    /// - `mock-plain`: bin works as shipped (pi/qwen shape).
    /// - `mock-finalize`: platform optional dependency carries the native
    ///   binary; the vendor's postinstall copies it over the error shim
    ///   (claude/opencode shape).
    /// - `mock-preinstall`: the vendor's preinstall materializes the binary
    ///   from an in-package payload (codex-litellm shape).
    /// - `mock-broken`: the postinstall fails — the install must FAIL, never
    ///   publish a shim.
    /// - `mock-missing-dep`: the platform optional dependency 404s — fatal,
    ///   because OUR platform's package is the one this machine needs.
    ///
    /// The update leg publishes a second version and proves the fetcher
    /// resolves, installs and verifies it — the "update is flawless" half of
    /// the owner's requirement.
    struct MockRegistry {
        child: std::process::Child,
        base: String,
        root: std::path::PathBuf,
    }
    impl MockRegistry {
        fn spawn(root: &Path) -> MockRegistry {
            use std::net::TcpListener;
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
            let port = listener.local_addr().expect("addr").port();
            drop(listener);
            let child = std::process::Command::new("python3")
                .arg(env!("CARGO_MANIFEST_DIR").to_string() + "/../../scripts/mock-npm-registry/server.py")
                .arg(root)
                .arg(port.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .expect("spawn mock registry");
            let base = format!("http://127.0.0.1:{port}");
            // Wait for readiness.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if std::time::Instant::now() > deadline {
                    panic!("mock registry did not become ready on port {port}");
                }
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            MockRegistry { child, base, root: root.to_path_buf() }
        }

        /// Lay down one version of one fixture package.
        fn publish(&self, name: &str, version: &str, files: &[(&str, &[u8], u32)], manifest: serde_json::Value) {
            let version_dir = self.root.join("packages").join(name).join(version);
            std::fs::create_dir_all(version_dir.join("files")).expect("version dir");
            for (rel, content, mode) in files {
                let path = version_dir.join("files").join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("file dir");
                }
                std::fs::write(&path, content).expect("write file");
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = std::fs::metadata(&path).expect("stat").permissions();
                permissions.set_mode(*mode);
                std::fs::set_permissions(&path, permissions).expect("chmod");
            }
            std::fs::write(
                version_dir.join("package.json"),
                serde_json::to_vec(&manifest).expect("manifest"),
            )
            .expect("write manifest");
            // The tarball must carry package.json TOO — it is the EXTRACTED
            // copy the install reads for bin/scripts/optionalDependencies.
            std::fs::write(
                version_dir.join("files").join("package.json"),
                serde_json::to_vec(&manifest).expect("manifest"),
            )
            .expect("write in-tar manifest");
        }

        fn url(&self) -> String {
            self.base.clone()
        }
    }
    impl Drop for MockRegistry {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn mock_manifest(bin: &str, bin_path: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut manifest = serde_json::json!({
            "bin": { bin: bin_path },
        });
        if let (Some(obj), Some(extra)) = (manifest.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                obj.insert(key.clone(), value.clone());
            }
        }
        manifest
    }

    const MOCK_SHIM: &[u8] = b"echo \"Error: mock vendor binary not installed.\" >&2\nexit 1\n";
    // A REAL executable for fixture purposes: a shebang script answers
    // --version with exit 0, which is all the publish gate requires.
    const MOCK_NATIVE_1: &[u8] = b"#!/bin/sh\necho \"mock-native 1.0.0\"\n";
    const MOCK_NATIVE_2: &[u8] = b"#!/bin/sh\necho \"mock-native 1.0.1\"\n";

    #[test]
    fn the_mock_registry_proves_install_and_update_for_every_shape() {
        let root = std::env::temp_dir().join(format!(
            "ygg-mock-registry-{}-{}",
            std::process::id(),
            current_time_ms()
        ));
        std::fs::create_dir_all(&root).expect("fixture root");
        let registry = MockRegistry::spawn(&root);

        // mock-plain: the binary works as shipped.
        registry.publish(
            "mock-plain",
            "3.1.0",
            &[("bin/mock", b"#!/bin/sh\necho \"mock-plain 3.1.0\"\n", 0o755)],
            mock_manifest("mock", "bin/mock", serde_json::json!({})),
        );
        // mock-finalize: shim + platform optional dep + postinstall copy.
        registry.publish(
            "mock-finalize",
            "1.0.0",
            &[
                ("bin/mock", MOCK_SHIM, 0o755),
                ("finalize.sh", b"#!/bin/sh\ncp -f ../mock-finalize-linux-x64/native bin/mock && chmod 755 bin/mock\n", 0o755),
            ],
            mock_manifest(
                "mock",
                "bin/mock",
                serde_json::json!({
                    "optionalDependencies": { "mock-finalize-linux-x64": "1.0.0" },
                    "scripts": { "postinstall": "bash ./finalize.sh" },
                }),
            ),
        );
        registry.publish(
            "mock-finalize-linux-x64",
            "1.0.0",
            &[("native", MOCK_NATIVE_1, 0o755)],
            mock_manifest("mock-native", "native", serde_json::json!({})),
        );
        // mock-preinstall: preinstall materializes the binary from a payload.
        registry.publish(
            "mock-preinstall",
            "2.0.0",
            &[
                ("payload/mock", b"#!/bin/sh\necho \"mock-pre 2.0.0\"\n", 0o755),
                ("install.sh", b"#!/bin/sh\nmkdir -p bin && cp payload/mock bin/mock && chmod 755 bin/mock\n", 0o755),
            ],
            mock_manifest(
                "mock",
                "bin/mock",
                serde_json::json!({ "scripts": { "preinstall": "bash ./install.sh" } }),
            ),
        );
        // mock-broken: the postinstall fails and the shim stays a shim.
        registry.publish(
            "mock-broken",
            "9.9.9",
            &[
                ("bin/mock", MOCK_SHIM, 0o755),
                ("broken.sh", b"#!/bin/sh\nexit 3\n", 0o755),
            ],
            mock_manifest(
                "mock",
                "bin/mock",
                serde_json::json!({ "scripts": { "postinstall": "bash ./broken.sh" } }),
            ),
        );
        // mock-missing-dep: the platform dependency 404s.
        registry.publish(
            "mock-missing-dep",
            "4.0.0",
            &[("bin/mock", MOCK_SHIM, 0o755)],
            mock_manifest(
                "mock",
                "bin/mock",
                serde_json::json!({
                    "optionalDependencies": { "mock-missing-dep-linux-x64": "1.0.0" },
                }),
            ),
        );
        let paths = provision_test_paths("mock-registry");
        // The fetcher must talk to the MOCK, never the real registry.
        // SAFETY: test process, single-threaded for this env key — the
        // registry base is read only by this test's installs.
        unsafe {
            std::env::set_var("YGGTERM_NPM_REGISTRY_BASE", registry.url());
        }

        let install_and_version = |prefix: &Path, package: &str, tag: &str| -> String {
            run_direct_install(&paths, prefix, package, tag).expect("install succeeds");
            let bin = prefix.join("bin").join("mock");
            let mut version_command = std::process::Command::new(&bin);
            version_command.arg("--version");
            match bounded_command_output(
                &mut version_command,
                MANAGED_CLI_VERSION_PROBE_TIMEOUT,
            ) {
                BoundedCommandOutput::Completed { stdout, success: true, .. } => {
                    String::from_utf8_lossy(&stdout).trim().to_string()
                }
                other => panic!("installed {package} binary does not answer --version: {other:?}"),
            }
        };

        // INSTALL, shape by shape.
        let plain_prefix = paths.prefix.join("gen-plain");
        assert_eq!(
            install_and_version(&plain_prefix, "mock-plain", "latest"),
            "mock-plain 3.1.0",
            "plain shape: the shipped binary must be published as-is"
        );
        let finalize_prefix = paths.prefix.join("gen-finalize");
        assert_eq!(
            install_and_version(&finalize_prefix, "mock-finalize", "latest"),
            "mock-native 1.0.0",
            "finalize shape: postinstall must swap the shim for the platform native"
        );
        let pre_prefix = paths.prefix.join("gen-pre");
        assert_eq!(
            install_and_version(&pre_prefix, "mock-preinstall", "latest"),
            "mock-pre 2.0.0",
            "preinstall shape: the vendor script must materialize the binary"
        );

        // UPDATE: a second version is PUBLISHED (as a vendor release is), and
        // the next install resolves, fetches, and verifies it.
        registry.publish(
            "mock-plain",
            "3.2.0",
            &[("bin/mock", b"#!/bin/sh\necho \"mock-plain 3.2.0\"\n", 0o755)],
            mock_manifest("mock", "bin/mock", serde_json::json!({})),
        );
        let plain_update_prefix = paths.prefix.join("gen-plain-update");
        assert_eq!(
            install_and_version(&plain_update_prefix, "mock-plain", "latest"),
            "mock-plain 3.2.0",
            "update: the fetcher must resolve and install the newer version"
        );

        // FAILURE shapes never install.
        let broken_prefix = paths.prefix.join("gen-broken");
        let error = run_direct_install(&paths, &broken_prefix, "mock-broken", "latest")
            .expect_err("a vendor script that leaves a dead binary must fail the install");
        assert!(
            error.to_string().contains("does not run"),
            "the failure must come from the publish gate, naming the dead binary: {error}"
        );
        let missing_prefix = paths.prefix.join("gen-missing");
        let error = run_direct_install(&paths, &missing_prefix, "mock-missing-dep", "latest")
            .expect_err("a missing platform dependency must fail the install");
        assert!(
            error.to_string().contains("mock-missing-dep-linux-x64"),
            "the failure must name the dependency it could not fetch: {error}"
        );

        // THE PUBLISH GATE: a shim that exists but does not run is refused.
        let shim_staged = paths.prefix.join("gen-gate");
        std::fs::create_dir_all(shim_staged.join("bin")).expect("staged bin");
        let gate_chmod = |content: &[u8]| {
            let path = shim_staged.join("bin").join("mock");
            std::fs::write(&path, content).expect("write gate fixture");
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path).expect("stat").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).expect("chmod gate fixture");
        };
        gate_chmod(MOCK_SHIM);
        assert!(
            staged_binary_runs(&shim_staged, "mock").is_err(),
            "an error shim must never pass the publish gate"
        );
        gate_chmod(MOCK_NATIVE_1);
        assert!(
            staged_binary_runs(&shim_staged, "mock").is_ok(),
            "a binary that answers --version must pass the publish gate"
        );

        // SAFETY: see the set_var note above.
        unsafe {
            std::env::remove_var("YGGTERM_NPM_REGISTRY_BASE");
        }
        let _ = std::fs::remove_dir_all(&paths.home);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The opencode shim health check: the vendor's error-shim text must NOT
    /// pass, and a real ELF binary must. Without this, the direct fetcher's
    /// fast path saw "bin exists, version matches" and kept a broken install
    /// forever.
    #[test]
    fn the_opencode_error_shim_is_not_a_healthy_binary() {
        let paths = provision_test_paths("opencode-shim");
        let prefix = &paths.prefix;
        let shim_dir = prefix
            .join("lib")
            .join("node_modules")
            .join("opencode-ai")
            .join("bin");
        std::fs::create_dir_all(&shim_dir).expect("create shim dir");
        let shim = shim_dir.join("opencode.exe");

        // The broken state: a text file, exactly what the vendor ships as the
        // placeholder shim.
        std::fs::write(&shim, "echo \"Error: opencode-ai's postinstall script was not run.\"\n")
            .expect("write shim");
        assert!(
            !direct_install_shim_is_healthy(prefix, "opencode-ai"),
            "the error shim must not satisfy the fast path"
        );
        // A real ELF (any ELF header will do for the check).
        std::fs::write(&shim, b"\x7fELF\x02\x01\x01\x00rest-of-binary").expect("write elf");
        assert!(
            direct_install_shim_is_healthy(prefix, "opencode-ai"),
            "a real binary must satisfy the fast path"
        );
        // Other packages are never second-guessed.
        assert!(direct_install_shim_is_healthy(prefix, "@openai/codex"));
        let _ = std::fs::remove_dir_all(&paths.home);
    }

    /// The v2 preview package (`@opencode-ai/cli`) carries the same
    /// entry-bin trap under a scoped path and a new bin name — the health
    /// check must follow the package the descriptor names, or a broken 2.0
    /// install satisfies the fast path forever.
    #[test]
    fn the_opencode2_preview_shim_is_checked_under_its_scoped_path() {
        let paths = provision_test_paths("opencode2-shim");
        let prefix = &paths.prefix;
        let shim_dir = prefix
            .join("lib")
            .join("node_modules")
            .join("@opencode-ai")
            .join("cli")
            .join("bin");
        std::fs::create_dir_all(&shim_dir).expect("create shim dir");
        let shim = shim_dir.join("opencode2.exe");

        std::fs::write(&shim, "echo \"Error: postinstall script was not run.\"\n")
            .expect("write shim");
        assert!(
            !direct_install_shim_is_healthy(prefix, "@opencode-ai/cli"),
            "the scoped package's error shim must not satisfy the fast path"
        );
        std::fs::write(&shim, b"\x7fELF\x02\x01\x01\x00rest-of-binary").expect("write elf");
        assert!(
            direct_install_shim_is_healthy(prefix, "@opencode-ai/cli"),
            "a real binary must satisfy the fast path"
        );
        let _ = std::fs::remove_dir_all(&paths.home);
    }

    fn provision_test_paths(tag: &str) -> ManagedCliPaths {
        let tmp = std::env::temp_dir().join(format!(
            "ygg-provision-{tag}-{}-{}",
            std::process::id(),
            current_time_ms()
        ));
        let bin_dir = tmp.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create temp managed bin dir");
        ManagedCliPaths {
            home: tmp.clone(),
            prefix: tmp.join("managed-npm"),
            bin_dir,
            cache_dir: tmp.join("cache"),
        }
    }

    /// Removal of an npm-managed CLI takes the WHOLE managed tree with it: the
    /// published symlink AND every generation directory. The regression this
    /// locks: a removal that only unlinked the symlink would leave the
    /// multi-hundred-MB generation trees orphaned on disk forever while
    /// reporting "removed".
    #[test]
    fn removing_an_npm_managed_cli_takes_the_generations_with_it() {
        let paths = provision_test_paths("remove-npm");
        let tool = ManagedCliTool::QwenCode;
        let binary = tool.binary_name();
        let generation_bin = paths
            .cli_root()
            .join(format!("{}.gen1", tool.descriptor().slug))
            .join("bin");
        std::fs::create_dir_all(&generation_bin).expect("create generation tree");
        std::fs::write(generation_bin.join(binary), b"#!/bin/sh\nexit 0\n")
            .expect("write fake binary");
        let link = paths.bin_dir.join(binary);
        std::os::unix::fs::symlink(generation_bin.join(binary), &link).expect("publish symlink");

        let status = remove_local_managed_cli_with_paths(&paths, tool).expect("remove");
        assert_eq!(status.action, "removed", "detail: {}", status.detail);
        assert!(
            paths.bin_dir.join(binary).symlink_metadata().is_err(),
            "the published symlink must be gone"
        );
        assert!(
            !paths
                .cli_root()
                .join(format!("{}.gen1", tool.descriptor().slug))
                .exists(),
            "the generation tree must be gone"
        );
        let _ = std::fs::remove_dir_all(&paths.home);
    }

    /// ⛔ THE PRUNE MUST NOT DELETE A GENERATION A RUNNING PROCESS EXECUTES
    /// FROM. Measured broken live: a long-running CLI spawns helper binaries
    /// from inside its own generation tree, and a refresh that pruned the tree
    /// under it broke every subsequent command the session tried to run. The
    /// prune defers such a generation to a later sweep instead.
    #[test]
    fn a_running_process_defers_the_prune_of_its_generation() {
        let paths = provision_test_paths("prune-live");
        let slug = "prune-live-cli";
        let live_gen = paths.cli_root().join(format!("{slug}.gen1"));
        let dead_gen = paths.cli_root().join(format!("{slug}.gen2"));
        for dir in [&live_gen, &dead_gen] {
            std::fs::create_dir_all(dir).expect("create generation");
        }
        // A REAL executable copied into the generation tree: while it runs,
        // /proc/<pid>/exe points inside the tree, exactly as a CLI's native
        // helper does.
        let executable = live_gen.join("sleeper");
        std::fs::copy("/bin/sleep", &executable).expect("copy sleep");
        let mut child = std::process::Command::new(&executable)
            .arg("30")
            .spawn()
            .expect("spawn sleeper");

        paths.prune_cli_generations(slug, 3);
        assert!(
            live_gen.exists(),
            "a generation a running process executes from must survive the prune"
        );
        assert!(
            !dead_gen.exists(),
            "a generation nothing is executing must still be pruned"
        );

        // Once the process is gone, the next sweep reaps the deferred tree.
        let _ = child.kill();
        let _ = child.wait();
        let live_exes = running_process_executable_paths();
        assert!(
            !generation_is_executed_by_running_process(&live_gen, &live_exes),
            "the killed process must no longer hold the generation"
        );
        paths.prune_cli_generations(slug, 3);
        assert!(
            !live_gen.exists(),
            "the deferred generation must be reaped once its process is gone"
        );
        let _ = std::fs::remove_dir_all(&paths.home);
    }

    /// Removing a CLI a process is currently executing from is REFUSED BY PATH:
    /// the running exe would survive mapped, but every helper it spawns from
    /// its generation tree afterwards would die with "No such file or
    /// directory" — the exact live failure that motivated liveness-aware
    /// pruning.
    #[test]
    fn removing_a_cli_a_process_is_running_from_is_refused() {
        let paths = provision_test_paths("remove-running");
        let tool = ManagedCliTool::QwenCode;
        let tool_marker = format!("{}.gen", tool.descriptor().slug);
        let tool_gen_bin = paths.cli_root().join(format!("{tool_marker}1")).join("bin");
        std::fs::create_dir_all(&tool_gen_bin).expect("create tool generation");
        let executable = tool_gen_bin.join(tool.binary_name());
        std::fs::copy("/bin/sleep", &executable).expect("copy sleep");
        std::os::unix::fs::symlink(&executable, paths.bin_dir.join(tool.binary_name()))
            .expect("publish symlink");
        let mut child = std::process::Command::new(&executable)
            .arg("30")
            .spawn()
            .expect("spawn runner");

        let error = remove_local_managed_cli_with_paths(&paths, tool)
            .expect_err("removal must refuse while a process runs from the tree");
        assert!(
            error.to_string().contains("running"),
            "the refusal must say the CLI is running: {error}"
        );
        assert!(
            paths.cli_root().join(format!("{tool_marker}1")).exists(),
            "the running generation must be untouched"
        );
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&paths.home);
    }

    /// A CLI that is not present reports "not installed" and removes nothing:
    /// a removal click on a missing CLI must not be able to create or damage
    /// a managed tree it did not find.
    #[test]
    fn removing_an_absent_cli_is_a_reported_no_op() {
        let paths = provision_test_paths("remove-absent");
        let tool = ManagedCliTool::QwenCode;
        let status = remove_local_managed_cli_with_paths(&paths, tool).expect("remove");
        assert_eq!(status.action, "not installed");
        assert!(!paths.cli_root().exists(), "nothing may be created");
        let _ = std::fs::remove_dir_all(&paths.home);
    }

    /// A vendor-installed CLI whose binary is a REAL file in the managed bin
    /// dir is removed by deleting that file.
    #[test]
    fn removing_a_vendor_cli_deletes_its_user_local_binary() {
        let paths = provision_test_paths("remove-vendor");
        let tool = ManagedCliTool::Muse;
        let binary = tool.binary_name();
        std::fs::write(paths.bin_dir.join(binary), b"#!/bin/sh\nexit 0\n")
            .expect("write fake vendor binary");

        let status = remove_local_managed_cli_with_paths(&paths, tool).expect("remove");
        assert_eq!(status.action, "removed", "detail: {}", status.detail);
        assert!(
            !paths.bin_dir.join(binary).exists(),
            "the vendor binary must be gone"
        );
        let _ = std::fs::remove_dir_all(&paths.home);
    }

    /// ⛔ A PACKAGE'S INSTALLER MUST NOT STAGE ITS DOWNLOAD IN RAM.
    ///
    /// `os.tmpdir()` in a `preinstall` script resolves to `$TMPDIR`, and on the
    /// desktop host `/tmp` is a **tmpfs**. A published CLI stages a 78 MB
    /// tarball there and never removes the directory, so every auto-update
    /// leaked 78 MB of RAM permanently: 51 dirs, 2.85 GB, measured on a machine
    /// sitting at 11 GB of 15 GB swap. The owner reported it as memory pressure.
    ///
    /// Two properties, because either alone is insufficient — staging on disk
    /// without sweeping merely relocates an unbounded leak, and sweeping a
    /// tmpfs still spikes RAM by the download size on every update.
    #[test]
    fn cli_staging_is_on_disk_and_swept() {
        let paths = provision_test_paths("staging");
        let staging = paths.staging_dir();

        // 1. It is under the managed home, NOT the system temp dir, so a
        //    tmpfs-backed /tmp is never where a 78 MB download lands.
        assert!(
            staging.starts_with(&paths.home),
            "staging {} must live under the managed home {}",
            staging.display(),
            paths.home.display()
        );
        assert_ne!(
            staging.parent(),
            Some(std::env::temp_dir().as_path()),
            "staging must not be a child of the system temp dir"
        );

        // 2. A leftover from an installer that never cleaned up is reaped.
        //    The leak belongs to a script we do not own, so the sweep must not
        //    depend on that script behaving.
        paths.ensure_dirs().expect("ensure dirs");
        let abandoned = staging.join("codex-litellm-AbCdEf");
        std::fs::create_dir_all(&abandoned).expect("stage a leftover");
        std::fs::write(abandoned.join("payload.tar.gz"), b"x").expect("write payload");
        assert!(abandoned.exists(), "control: the leftover was staged");

        paths.sweep_staging();

        assert!(
            !abandoned.exists(),
            "an abandoned staging dir must be reaped before the next install"
        );
        assert!(
            staging.exists(),
            "the staging root itself must survive the sweep"
        );
        let _ = std::fs::remove_dir_all(&paths.home);
    }

    /// The owner's 2026-08-08 ruling, asserted as a property of the registry:
    /// *"yggterm should auto install, update ALL clis in all connected systems
    /// including localhost."*
    ///
    /// ⛔ The failure this guards, which is what the ruling was about: the
    /// provisioner asked ONE question — "does this CLI have an npm package?" —
    /// and three of the nine registered CLIs answered no. `kimi` (uv), `muse`
    /// (vendor script) and `agy` (unfetchable, but self-updating) were therefore
    /// never installed and never updated, silently, while the refresh reported
    /// success for the six it did cover.
    #[test]
    fn every_registered_cli_has_something_yggterm_can_run_for_it() {
        for descriptor in yggterm_core::agent_cli::AGENT_CLIS {
            ManagedCliTool::from_session_kind(descriptor.kind)
                .expect("every registered CLI is a managed tool");
            // A binary yggterm can FETCH needs no help from the machine.
            if descriptor.install.provisions_unattended() {
                assert!(
                    provision_step_for(descriptor, false).is_some(),
                    "{}: yggterm declares it provisions this CLI but has no step for it",
                    descriptor.display_name
                );
                continue;
            }
            // One it cannot fetch must at least be updatable once present.
            assert!(
                matches!(
                    provision_step_for(descriptor, true),
                    Some(ProvisionStep::SelfUpdate(argv)) if !argv.is_empty()
                ),
                "{}: yggterm can neither fetch nor update this CLI",
                descriptor.display_name
            );
        }
    }

    /// ⭐ A CLI's own updater outranks re-running its install method — but only
    /// once the binary exists, because an updater cannot install what is absent.
    ///
    /// Antigravity is the case that forced the two axes apart: yggterm cannot
    /// fetch `agy` (166 MB, served behind a sign-in) yet `agy --help` advertises
    /// `update`. Asking only "can we install it" wrote the CLI off entirely.
    #[test]
    fn a_self_updating_cli_updates_itself_only_once_it_is_present() {
        let agy = ManagedCliTool::Antigravity.descriptor();
        // Absent: nothing yggterm can do, and it must say so rather than run an
        // updater against a binary that is not there.
        assert_eq!(
            provision_step_for(agy, false),
            None,
            "an absent unfetchable CLI has no step"
        );
        assert_eq!(
            provision_step_for(agy, true),
            Some(ProvisionStep::SelfUpdate(&["update"])),
            "a present self-updating CLI runs its own updater"
        );
        // And an npm CLI does NOT acquire a self-updater by being present.
        let codex = ManagedCliTool::Codex.descriptor();
        assert_eq!(provision_step_for(codex, true), Some(ProvisionStep::Npm));
        assert_eq!(provision_step_for(codex, false), Some(ProvisionStep::Npm));
    }

    /// ⛔ A SELF-UPDATER THAT IS npm IN DISGUISE MUST NOT BE PREFERRED.
    ///
    /// grok ships `grok update`, and the general rule prefers a CLI's own
    /// updater over re-running the install method. Measured 2026-08-20, its own
    /// `update --check --json` reports `"installer":"npm"` — the updater
    /// delegates back to npm for an npm-provisioned copy. Preferring it would
    /// move the npm call inside a process where the staged prefix, the binary
    /// verification and the atomic publish do not apply, and where an inherited
    /// `npm_config_prefix` would write the SHARED prefix and overwrite the
    /// published per-CLI symlink.
    ///
    /// ⚠ Locks the DECISION, not the mechanism: Antigravity must keep its
    /// self-updater, because for an unfetchable CLI it is the only step there is.
    #[test]
    fn a_self_updater_that_delegates_to_npm_is_not_preferred_over_our_own() {
        let grok = ManagedCliTool::GrokBuild.descriptor();
        assert_eq!(
            provision_step_for(grok, true),
            Some(ProvisionStep::Npm),
            "grok's updater re-enters npm, so yggterm runs npm itself and keeps \
             the generation, verification and atomic-publish guarantees"
        );
        // The rule it declines is still the rule for a CLI that really does
        // update itself.
        let agy = ManagedCliTool::Antigravity.descriptor();
        assert_eq!(
            provision_step_for(agy, true),
            Some(ProvisionStep::SelfUpdate(&["update"])),
            "an unfetchable CLI's own updater is the only step there is"
        );
    }

    /// The non-npm methods are RUN, not recorded.
    ///
    /// ⛔ Guards the exact regression the ruling reversed: `npm_package()`
    /// answering `None` for kimi and muse used to be the gate that skipped them.
    /// It still answers `None` — that is correct, they are not npm packages —
    /// so the gate had to move to a different question, and this proves it did.
    #[test]
    fn a_uv_or_vendor_cli_is_provisioned_rather_than_refused() {
        assert_eq!(ManagedCliTool::Kimi.npm_package(), None);
        assert_eq!(ManagedCliTool::Muse.npm_package(), None);
        for present in [false, true] {
            assert_eq!(
                provision_step_for(ManagedCliTool::Kimi.descriptor(), present),
                Some(ProvisionStep::Uv("kimi-cli"))
            );
            assert_eq!(
                provision_step_for(ManagedCliTool::Muse.descriptor(), present),
                Some(ProvisionStep::VendorScript("https://dev.meta.ai/install.sh"))
            );
        }
    }

    /// ⛔ npm fails a WHOLE `install -g` batch on one unresolvable name. A uv
    /// package handed to that line would not install the wrong thing — it would
    /// take codex and claude's refresh down with it. Now that every tool is
    /// passed to `install_latest`, that separation is load-bearing.
    #[test]
    fn only_npm_packages_reach_the_npm_batch() {
        let paths = provision_test_paths("batch");
        for tool in managed_cli_tools_for_refresh() {
            if provision_step_for(tool.descriptor(), false) == Some(ProvisionStep::Npm) {
                assert!(
                    tool.npm_package().is_some(),
                    "{} would be appended to `npm install -g` with no package",
                    tool.display_name()
                );
            }
        }
        // An empty batch is a no-op, not an "npm is required" error — the case a
        // machine with only uv/vendor CLIs to refresh hits every time.
        assert!(install_npm_isolated(&paths, &[], true).is_ok());
    }

    /// ⛔ AN INTERRUPTED UPDATE MUST LEAVE THE OLD BINARY WORKING.
    ///
    /// The defect this locks was measured twice, deterministically, on
    /// 2026-08-20: one batched `npm install -g --force <7 packages>` spends
    /// several seconds with every published binary unlinked, and a kill inside
    /// that window left `bin/` holding **zero** CLIs and seven orphaned
    /// `.<name>-<random>` staging symlinks.
    ///
    /// The generation layout makes that shape unreachable rather than unlikely:
    /// an install writes an UNPUBLISHED directory, so a run that dies at any
    /// point before the swap cannot be observed at all. Simulated here by
    /// building generation 2 and abandoning it.
    #[cfg(unix)]
    #[test]
    fn an_abandoned_generation_leaves_the_published_binary_untouched() {
        let paths = provision_test_paths("interrupt");
        std::fs::create_dir_all(paths.cli_root()).expect("cli root");

        // Generation 1, published — the "already working" install.
        let live = paths.cli_generation_dir("grok-build", 1);
        std::fs::create_dir_all(live.join("bin")).expect("gen1 bin");
        std::fs::write(live.join("bin").join("grok"), b"#!/bin/sh\necho 1.0.5\n")
            .expect("gen1 binary");
        paths
            .publish_cli_binary("grok-build", "grok", 1)
            .expect("publish gen1");
        assert_eq!(paths.published_generation("grok-build", "grok"), Some(1));

        // Generation 2 starts and dies half-written, exactly as a kill would
        // leave it: a directory, no binary, nothing published.
        let staged = paths.cli_generation_dir("grok-build", 2);
        std::fs::create_dir_all(staged.join("lib")).expect("gen2 partial");

        // The published binary is still generation 1 and still resolves.
        assert_eq!(
            paths.published_generation("grok-build", "grok"),
            Some(1),
            "an unpublished generation must not become live by existing"
        );
        let published = paths.bin_dir.join("grok");
        assert!(
            std::fs::read_to_string(&published)
                .expect("the published symlink still resolves to a real file")
                .contains("1.0.5"),
            "the old binary must survive an interrupted update"
        );
    }

    /// Publishing is a REPLACE, so a legacy install that left a real file at
    /// `bin/<binary>` migrates without a separate step — and without a window
    /// in which the binary is missing.
    #[cfg(unix)]
    #[test]
    fn publishing_replaces_a_legacy_real_file_in_one_step() {
        let paths = provision_test_paths("migrate");
        std::fs::create_dir_all(paths.cli_root()).expect("cli root");
        std::fs::write(paths.bin_dir.join("grok"), b"legacy shared-prefix install")
            .expect("legacy binary");

        let generation = paths.cli_generation_dir("grok-build", 1);
        std::fs::create_dir_all(generation.join("bin")).expect("generation bin");
        std::fs::write(generation.join("bin").join("grok"), b"generation install")
            .expect("generation binary");
        paths
            .publish_cli_binary("grok-build", "grok", 1)
            .expect("publish over the legacy file");

        assert_eq!(paths.published_generation("grok-build", "grok"), Some(1));
        assert_eq!(
            std::fs::read_to_string(paths.bin_dir.join("grok")).expect("read published"),
            "generation install"
        );
    }


    /// ⭐ THE LIVE FALSIFIER, run against real npm and the real registry.
    ///
    /// `#[ignore]`d because it downloads: run it deliberately with
    /// `cargo test -p yggterm-server --lib -- --ignored provisions_grok`.
    /// It is kept in the tree because the property it checks — that a second
    /// pass is a cheap no-op that still republishes cleanly — is exactly what
    /// the old batched installer could not do, and a claim about provisioning
    /// that was never run against the registry is not a measurement.
    #[cfg(unix)]
    #[test]
    #[ignore = "downloads from the npm registry"]
    fn provisions_grok_twice_in_a_row_without_a_partial_tree() {
        let paths = provision_test_paths("e2e-grok");
        let tool = ManagedCliTool::GrokBuild;
        let binary = tool.binary_name();

        for pass in 1..=2 {
            install_npm_isolated(&paths, &[tool], false)
                .unwrap_or_else(|error| panic!("pass {pass} failed: {error}"));

            let generation = paths
                .published_generation("grok-build", binary)
                .unwrap_or_else(|| panic!("pass {pass} published nothing"));
            assert_eq!(generation, pass, "each pass publishes the next generation");

            // The published path must RESOLVE — a dangling symlink is the exact
            // shape of the failure this layout exists to make impossible.
            let published = paths.bin_dir.join(binary);
            assert!(
                published.canonicalize().is_ok(),
                "pass {pass}: the published {binary} does not resolve"
            );

            // Exactly one generation survives: the published one.
            let generations: Vec<String> = std::fs::read_dir(paths.cli_root())
                .expect("cli root")
                .flatten()
                .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
                .filter(|name| name.starts_with("grok-build.gen"))
                .collect();
            assert_eq!(
                generations,
                vec![format!("grok-build.gen{generation}")],
                "pass {pass}: superseded generations must be reaped"
            );
        }

        let _ = std::fs::remove_dir_all(&paths.home);
    }

    /// Superseded generations are reaped, so the per-CLI layout cannot become
    /// the next unbounded store. ⚠ The one just published is never a candidate.
    #[cfg(unix)]
    #[test]
    fn pruning_keeps_the_published_generation_and_drops_the_rest() {
        let paths = provision_test_paths("prune");
        std::fs::create_dir_all(paths.cli_root()).expect("cli root");
        for generation in 1..=3 {
            std::fs::create_dir_all(paths.cli_generation_dir("grok-build", generation))
                .expect("generation");
        }
        // A different CLI's tree must be untouched by another's prune.
        std::fs::create_dir_all(paths.cli_generation_dir("claude-code", 7)).expect("other cli");

        paths.prune_cli_generations("grok-build", 3);

        assert!(paths.cli_generation_dir("grok-build", 3).exists());
        assert!(!paths.cli_generation_dir("grok-build", 1).exists());
        assert!(!paths.cli_generation_dir("grok-build", 2).exists());
        assert!(
            paths.cli_generation_dir("claude-code", 7).exists(),
            "pruning one CLI must never reach another's generations"
        );
    }


    /// ⛔ NO PROVISIONING STEP MAY STAGE IN `/tmp` — CHECKED FOR ALL FOUR
    /// METHODS, NOT JUST THE ONE THAT WAS CAUGHT.
    ///
    /// The npm path was given a disk-backed `TMPDIR` on 2026-08-14 and the other
    /// three were not, so `uv tool install`, the vendor `curl … | sh` and a
    /// CLI's own updater all still staged into a tmpfs for six more days. That
    /// is the shape this asserts against: a fix applied per-callsite, in a file
    /// where callsites keep being added.
    ///
    /// ⚠ Reads the SHIPPED source rather than a comment, because the failure
    /// mode is a new `Command::new` that simply forgets.
    #[test]
    fn every_provisioning_command_stages_on_disk() {
        let source = include_str!("mod.rs");
        // Each provisioning method, and the marker that proves it routes temp.
        // ⛔ NEWLINE-ANCHORED. Without the leading newline each needle matches
        //    THIS TEST'S OWN array literal first — the test module sits above
        //    the functions it reads — and the check then asserts against a slice
        //    of itself and fails for a reason that has nothing to do with the
        //    code. A string-keyed search finds the FIRST match, not the intended
        //    one; caught while writing this.
        for (method, marker) in [
            ("\nfn install_via_uv(", "apply_provision_env(&mut command, paths)"),
            ("\nfn install_via_vendor_script(", "apply_provision_env(&mut run, paths)"),
            ("\nfn update_via_self_command(", "apply_provision_env(&mut command, paths)"),
            ("\nfn run_npm_install(", "\"TMPDIR\""),
        ] {
            let body = source
                .split_once(method)
                .unwrap_or_else(|| panic!("{method} is the owner of one provisioning method"))
                .1;
            let body = body.split_once("\nfn ").map(|(head, _)| head).unwrap_or(body);
            assert!(
                body.contains(marker),
                "{method} does not route its temp directory to disk; on the \
                 desktop host /tmp is a tmpfs, so its staging is RAM"
            );
        }

        // And the helper must actually set TMPDIR, or every check above is
        // asserting the presence of a no-op.
        let helper = source
            .split_once("\nfn apply_provision_env")
            .expect("the shared provisioning environment")
            .1;
        let helper = helper.split_once("\nfn ").map(|(head, _)| head).unwrap_or(helper);
        assert!(
            helper.contains(".env(\"TMPDIR\", paths.staging_dir())"),
            "apply_provision_env must be what puts staging on disk"
        );
    }

    /// ⛔ THE npm INVOCATION MUST NOT CARRY `--force`, AND THE REASON IS THE
    /// WHOLE POINT OF THIS MODULE'S REWRITE.
    ///
    /// `--force` rewrote all 164 packages and relinked every binary on every
    /// pass, including passes where nothing had changed — which is what turned
    /// a routine refresh into the destructive window. A fresh generation
    /// directory is empty, so there is nothing left for it to force.
    ///
    /// ⚠ Asserted against the SHIPPED argument list rather than a comment,
    /// because a comment cannot fail.
    #[test]
    fn the_managed_npm_install_never_forces() {
        let source = include_str!("mod.rs");
        let install = source
            .split_once("fn run_npm_install(")
            .expect("run_npm_install is the one owner of the npm argument list")
            .1;
        let body = install
            .split_once("\nfn ")
            .map(|(body, _)| body)
            .unwrap_or(install);
        assert!(
            !body.contains(".arg(\"--force\")"),
            "`--force` is back in the managed npm install; it reopens the \
             all-binaries-unlinked window this layout exists to close"
        );
        assert!(
            !body.contains(".env(\"npm_config_tmp\""),
            "npm 11 answers `Unknown env config \"tmp\"` and ignores it; \
             TMPDIR is what actually moves staging off a tmpfs"
        );
    }

    /// The status line must name the method that ACTUALLY ran.
    ///
    /// ⛔ MEASURED WRONG, live on guihost 2026-08-08, on all three new lanes at
    /// once: the uv install of `kimi` (landed in `~/.local/bin`), the vendor
    /// install of `muse` (landed in `~/.local/bin`) and the `agy update` that
    /// installed nothing anywhere ALL reported *"a Yggterm-managed <CLI>
    /// toolchain under ~/.yggterm/npm"*. One sentence, true of the npm
    /// lane only. A user who goes looking in the named directory for the binary
    /// we just installed finds nothing and concludes the install failed.
    #[test]
    fn the_install_detail_names_the_method_that_ran() {
        let paths = provision_test_paths("detail");
        // uv: the package and the uv verb, never the npm prefix.
        let kimi = provision_detail(&paths, ManagedCliTool::Kimi);
        assert!(kimi.contains("kimi-cli"), "{kimi}");
        assert!(kimi.contains("uv tool install --upgrade"), "{kimi}");
        assert!(!kimi.contains("npm"), "a uv install must not name the npm prefix: {kimi}");

        // vendor: the URL that was executed.
        let muse = provision_detail(&paths, ManagedCliTool::Muse);
        assert!(muse.contains("https://dev.meta.ai/install.sh"), "{muse}");
        assert!(!muse.contains("npm"), "a vendor install must not name the npm prefix: {muse}");

        // npm: still names the prefix, because for npm that IS where it went.
        let codex = provision_detail(&paths, ManagedCliTool::Codex);
        assert!(codex.contains(&paths.prefix.display().to_string()), "{codex}");

        // ⚠ `package_name` is the other half of the same lie: it called an
        // unfetchable CLI one "yggterm never provisions", which stopped being
        // true the moment yggterm started updating it.
        let manual = ManagedCliTool::Antigravity.package_name();
        assert!(
            !manual.contains("never provisions"),
            "yggterm updates this CLI: {manual}"
        );
    }

    /// Each method's prerequisite is its OWN, and the report must name it.
    ///
    /// ⚠ The single global `npm_available` this replaced was wrong twice over on
    /// a uv CLI: npm's absence is not why `kimi` is missing, and npm's presence
    /// would not have fixed it.
    #[test]
    fn a_missing_provisioner_is_named_per_method() {
        assert_eq!(ManagedCliTool::Kimi.package_name(), "kimi-cli");
        assert_eq!(
            ManagedCliTool::Muse.package_name(),
            "https://dev.meta.ai/install.sh"
        );
        // The vendor installer is fetched to a stable, collision-free path.
        let stem = vendor_script_stem("https://dev.meta.ai/install.sh");
        assert_eq!(stem, "https---dev-meta-ai-install-sh");
        assert!(!stem.contains('/'), "a URL stem must not create directories");
        assert_ne!(
            vendor_script_stem("https://dev.meta.ai/install.sh"),
            vendor_script_stem("https://astral.sh/uv/install.sh"),
            "two vendors must not share one installer path"
        );
    }

    // WITHOUT running a `<cli> --version` subprocess — that subprocess (claude up to
    // 910ms) plus the npm install it can trigger is the cold-switch latency we removed.
    // version==None is the proof that no subprocess ran.
    #[test]
    fn probe_tool_existence_only_skips_version_subprocess_for_present_managed_binary() {
        let tmp = std::env::temp_dir().join(format!(
            "ygg-existence-probe-{}-{}",
            std::process::id(),
            current_time_ms()
        ));
        let bin_dir = tmp.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create temp managed bin dir");
        // A file that exists but is NOT a runnable binary — if the probe tried to run
        // `--version` it would garble/fail; version must stay None because we never run it.
        std::fs::write(bin_dir.join("codex"), b"#!not-a-real-binary").expect("write fake bin");
        let paths = ManagedCliPaths {
            home: tmp.clone(),
            prefix: tmp.join("managed-npm"),
            bin_dir,
            cache_dir: tmp.join("cache"),
        };
        let probe = probe_tool_existence_only(&paths, ManagedCliTool::Codex);
        assert!(probe.available, "present managed binary should be available");
        assert_eq!(probe.source, Some(ManagedCliBinarySource::Managed));
        assert_eq!(
            probe.version, None,
            "the attach probe must not run a --version subprocess"
        );
        // And the status it produces is never an install action (no blocking on attach).
        let status = managed_cli_launch_status_from_probe(ManagedCliTool::Codex, probe);
        assert_eq!(status.action, "ready");
        assert_ne!(status.action, "installed");
        std::fs::remove_dir_all(&tmp).ok();
    }

    // Claude Code rides the managed-CLI lane (self-provision + 6h refresh)
    // with claude's own conventions: `--resume <id>` flag, not the codex
    // `resume` subcommand. A wrong shape here silently breaks every CC
    // launch/resume once the managed lane takes over from the legacy builder.
    // Two tables now name the same executable: `ManagedCliTool` (provisioning
    // owns install/version) and the harness descriptor (invocation shape). They
    // must agree, or a CLI is provisioned under one name and invoked under
    // another — silent until a session refuses to launch.
    #[test]
    fn managed_cli_tool_and_descriptor_agree_on_every_binary_name() {
        for descriptor in yggterm_core::agent_cli::AGENT_CLIS {
            let tool = ManagedCliTool::from_session_kind(descriptor.kind).unwrap_or_else(|| {
                panic!("{:?} has a descriptor but no managed tool", descriptor.kind)
            });
            assert_eq!(
                tool.binary_name(),
                descriptor.binary_name,
                "{:?}: provisioning and invocation must name one binary",
                descriptor.kind
            );
        }
    }

    /// The refusal a user reads when the CLI they clicked is not on the machine.
    ///
    /// ⛔ It must name THREE things, because each replaces a thing the old
    /// silent-shell failure made the user work out for themselves: which CLI
    /// (the row said `healthy`), which executable was looked for (`command not
    /// found` was buried in scrollback), and what to do about it (nothing said
    /// anything). reported 2026-08-08.
    #[test]
    fn a_missing_cli_is_refused_by_name_with_its_install_method() {
        for descriptor in yggterm_core::agent_cli::AGENT_CLIS {
            let message = missing_binary_refusal_message(descriptor);
            assert!(
                message.contains(descriptor.display_name),
                "{:?}: the refusal must name the CLI: {message:?}",
                descriptor.kind
            );
            assert!(
                message.contains(descriptor.binary_name),
                "{:?}: the refusal must name the executable it looked for: {message:?}",
                descriptor.kind
            );
            assert!(
                message.contains(&descriptor.install_instruction()),
                "{:?}: the refusal must carry the descriptor's install method: {message:?}",
                descriptor.kind
            );
            // ⛔ "not installed" is the whole claim. A refusal that hedged would
            // read as a transient glitch and send the user back to clicking.
            assert!(
                message.contains("is not installed on this machine"),
                "{:?}: {message:?}",
                descriptor.kind
            );
        }
    }

    // Byte-for-byte lock on the invocations the descriptor now builds. These
    // strings are what actually reaches the PTY, and phase 1 is a REFACTOR —
    // any change here is a behavior change wearing a refactor's clothes.
    #[test]
    fn descriptor_built_invocations_match_the_shipped_strings() {
        let quoted = shell_single_quote("019d-abc");
        let codex = yggterm_core::agent_cli::agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert_eq!(
            format!(
                "codex{}",
                join_invocation_tokens(&codex.resume_tokens(&quoted, true))
            ),
            "codex resume -C \"$PWD\" '019d-abc'"
        );
        assert_eq!(
            format!(
                "codex{}",
                join_invocation_tokens(&codex.resume_tokens(&quoted, false))
            ),
            "codex resume '019d-abc'"
        );
        let claude =
            yggterm_core::agent_cli::agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert_eq!(
            format!(
                "claude{}",
                join_invocation_tokens(&claude.resume_tokens(&quoted, true))
            ),
            "claude --resume '019d-abc'"
        );
        assert_eq!(
            format!(
                "claude{}",
                join_invocation_tokens(&claude.resume_picker_tokens())
            ),
            "claude --resume"
        );
        // A plain launch carries no tokens at all — byte-identical to the
        // pre-descriptor `format!("{bin}{extra}")`.
        assert_eq!(join_invocation_tokens(&[]), "");
    }

    #[test]
    fn managed_cli_supports_claude_code() {
        assert_eq!(
            ManagedCliTool::from_session_kind(SessionKind::ClaudeCode),
            Some(ManagedCliTool::ClaudeCode)
        );
        assert_eq!(ManagedCliTool::ClaudeCode.binary_name(), "claude");
        assert_eq!(
            ManagedCliTool::ClaudeCode.package_name(),
            "@anthropic-ai/claude-code"
        );
        // ⛔ THE SES_ GUARD: an un-rebounded opencode row carries yggterm's
        // birth uuid; opencode2's service rejects anything not starting with
        // `ses` (the owner's viewport error). The composer degrades to a
        // FRESH LAUNCH instead of composing a doomed resume.
        let phantom = managed_cli_shell_command(
            SessionKind::OpenCode,
            Some("/tmp"),
            ManagedCliAction::Resume {
                session_id: "d4090efe-4e12-42d9-938d-66f61801d2e7",
                persistent: false,
            },
        )
        .expect("opencode phantom resume command");
        assert!(
            !phantom.contains("--session"),
            "a phantom id must not be composed as an opencode resume: {phantom}"
        );
        let real = managed_cli_shell_command(
            SessionKind::OpenCode,
            Some("/tmp"),
            ManagedCliAction::Resume {
                session_id: "ses_real",
                persistent: false,
            },
        )
        .expect("opencode real resume command");
        assert!(real.contains("--session 'ses_real'"), "{real}");

        let resume = managed_cli_shell_command(
            SessionKind::ClaudeCode,
            Some("/tmp"),
            ManagedCliAction::Resume {
                session_id: "abc-123",
                persistent: false,
            },
        )
        .expect("claude resume command");
        assert!(resume.contains("claude"), "{resume}");
        assert!(resume.contains("--resume 'abc-123'"), "{resume}");
        assert!(!resume.contains(" resume 'abc-123'"), "{resume}");
        let launch = managed_cli_shell_command(
            SessionKind::ClaudeCode,
            Some("/tmp"),
            ManagedCliAction::Launch,
        )
        .expect("claude launch command");
        assert!(!launch.contains("--resume"), "{launch}");
    }

    #[test]
    fn managed_cli_shell_exports_prefer_managed_bin_and_suppress_npm_noise() {
        let paths = ManagedCliPaths {
            home: PathBuf::from("/tmp/yggterm-home"),
            prefix: PathBuf::from("/tmp/yggterm-home/npm"),
            bin_dir: PathBuf::from("/tmp/yggterm-home/npm/bin"),
            cache_dir: PathBuf::from("/tmp/yggterm-home/npm-cache"),
        };
        let exports = paths.shell_exports(ManagedCliTool::Codex);
        assert!(exports.contains("export NPM_CONFIG_UPDATE_NOTIFIER=false"));
        assert!(exports.contains("export npm_config_update_notifier=false"));
        assert!(exports.contains("export npm_config_audit=false"));
        assert!(exports.contains("export npm_config_fund=false"));
        // ⚠ Shape, not the whole string: the prefix now also carries the
        // login-shell dirs the daemon's `PATH` lacks, which differ per machine.
        // Asserting the full literal would make this test read the tester's
        // environment — the composition itself is locked deterministically by
        // `a_launch_path_prefix_carries_the_dirs_uv_and_vendor_installs_land_in`.
        assert!(
            exports.contains("export PATH='/tmp/yggterm-home/npm/bin':"),
            "the managed bin dir must stay FIRST on the launch PATH: {exports}"
        );
        assert!(exports.contains(":\"$PATH\""), "{exports}");
    }

    /// ⛔ reported 2026-08-09: `muse`, `kimi` and `agy` were "not found"
    /// on a host carrying all three. The PTY is spawned `$SHELL -c`, not
    /// `-lc`, so it sees the daemon's stripped `PATH`; this prefix was the
    /// managed npm bin dir ALONE, so the three CLIs that do not arrive by npm
    /// — uv, vendor script, manual, all landing in `~/.local/bin` — were
    /// unreachable while the npm three worked.
    #[test]
    fn a_launch_path_prefix_carries_the_dirs_uv_and_vendor_installs_land_in() {
        let managed = PathBuf::from("/tmp/yggterm-home/npm/bin");
        // A real login `PATH` shape: a user dir first, the system dirs in the
        // middle, and a dir the user deliberately put LAST.
        let login = vec![
            PathBuf::from("/home/user/.local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/home/user/.local/bin"),
            PathBuf::from("/opt/plan9/bin"),
        ];

        let user_local = PathBuf::from("/home/user/.local/bin");

        let dirs = compose_launch_path_prefix_dirs(Some(&managed), Some(&user_local), &login);

        assert_eq!(
            dirs.first(),
            Some(&managed),
            "a yggterm-MANAGED binary must still outrank a system copy"
        );
        assert_eq!(
            dirs,
            vec![
                managed.clone(),
                user_local.clone(),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/plan9/bin"),
            ],
            "the login shell's ORDER is reproduced verbatim (repeats collapsed \
             to their first position). Dropping a dir because the daemon's own \
             PATH also has it promotes everything after it — that is how \
             /opt/plan9/bin got in front of /usr/bin and every session was \
             handed Plan 9's `date`"
        );

        // ⛔ THE HALF THAT MUST NOT DEPEND ON A SUBPROCESS. When the login-shell
        // probe returns nothing — it did, live, on a daemon under swap pressure
        // — the uv/vendor install dir must STILL be there, or the fix silently
        // reverts to the bug it was written for.
        assert_eq!(
            compose_launch_path_prefix_dirs(Some(&managed), Some(&user_local), &[]),
            vec![managed.clone(), user_local.clone()],
            "with no login-shell answer at all, yggterm's OWN install dirs are \
             still on the launch PATH"
        );

        // No managed dir: the probe's search order, which must be the launch's
        // minus that one entry — or the probe reports a binary the launch will
        // not exec.
        assert_eq!(
            compose_launch_path_prefix_dirs(None, Some(&user_local), &login),
            dirs[1..].to_vec()
        );
    }

    #[test]
    fn summarize_managed_cli_report_mentions_deferred_initial_install() {
        let report = ManagedCliRefreshReport {
            scope: "local".to_string(),
            background: true,
            statuses: vec![ManagedCliToolStatus {
                tool: ManagedCliTool::Codex,
                package_name: "@openai/codex".to_string(),
                binary_name: "codex".to_string(),
                version_before: Some("1.2.3".to_string()),
                version_after: Some("1.2.3".to_string()),
                source_before: Some(ManagedCliBinarySource::System),
                source_after: Some(ManagedCliBinarySource::System),
                changed: false,
                available: true,
                action: "deferred_install".to_string(),
                detail: "deferred".to_string(),
            }],
            skipped_recently: false,
            ttl_remaining_ms: None,
            install_attempted: false,
            install_deferred: true,
        };
        assert_eq!(
            summarize_managed_cli_report("local", &report),
            "local: deferred initial managed Codex install until first use"
        );
    }

    #[test]
    fn summarize_managed_cli_report_mentions_deferred_background_install() {
        let report = ManagedCliRefreshReport {
            scope: "local".to_string(),
            background: true,
            statuses: vec![ManagedCliToolStatus {
                tool: ManagedCliTool::Codex,
                package_name: "@openai/codex".to_string(),
                binary_name: "codex".to_string(),
                version_before: Some("1.2.3".to_string()),
                version_after: Some("1.2.3".to_string()),
                source_before: Some(ManagedCliBinarySource::Managed),
                source_after: Some(ManagedCliBinarySource::Managed),
                changed: false,
                available: true,
                action: "deferred_background_install".to_string(),
                detail: "deferred".to_string(),
            }],
            skipped_recently: false,
            ttl_remaining_ms: None,
            install_attempted: false,
            install_deferred: true,
        };
        assert_eq!(
            summarize_managed_cli_report("local", &report),
            "local: deferred background managed Codex install"
        );
    }

    #[test]
    fn shell_join_extra_args_quotes_configured_codex_flags() {
        assert_eq!(shell_join_extra_args(""), "");
        assert_eq!(
            shell_join_extra_args("-s danger-full-access --profile \"field test\""),
            " '-s' 'danger-full-access' '--profile' 'field test'"
        );
        assert_eq!(
            shell_join_extra_args("--message \"Avikalpa's laptop\""),
            " '--message' 'Avikalpa'\\''s laptop'"
        );
    }

    // The delegate-launch composition, at the layer that builds the command.
    // `composed_cli_extra_args` reads the user's SETTINGS, so these drive the
    // pure half — strip + append + quote — through the same code path with the
    // configured side supplied explicitly.
    #[test]
    fn a_per_launch_option_appends_after_stripping_what_it_overrides() {
        let configured = split_extra_args("--model claude-fable-5 --verbose");
        let launch = AgentLaunchOptions {
            model: Some("claude-opus-5".to_string()),
            permission_mode: Some(yggterm_core::AgentPermissionMode::Bypass),
        };
        let mut tokens = launch.strip_overridden(SessionKind::ClaudeCode, &configured);
        tokens.extend(launch.launch_tokens(SessionKind::ClaudeCode).unwrap());
        assert_eq!(
            shell_join_tokens(&tokens),
            " '--verbose' '--model' 'claude-opus-5' '--dangerously-skip-permissions'",
            "the configured model must be GONE, not merely outranked"
        );
    }

    // An empty launch must leave the command byte-identical to the pre-flag
    // path, or every human door (titlebar +, KeyTips, start page) changes shape
    // for a feature none of them asked for.
    #[test]
    fn an_empty_launch_composes_to_the_configured_args_verbatim() {
        let configured = split_extra_args("-s danger-full-access --profile 'field test'");
        let launch = AgentLaunchOptions::default();
        assert_eq!(
            shell_join_tokens(&launch.strip_overridden(SessionKind::Codex, &configured)),
            shell_join_extra_args("-s danger-full-access --profile 'field test'")
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BoundedCommandOutput {
    Completed {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        /// The child's exit status. The version probe never cared (it reads
        /// output); the publish gate DOES — a vendor error shim prints its
        /// paragraph on stderr and exits non-zero, and "printed something"
        /// must never read as "ran".
        success: bool,
    },
    TimedOut,
    Failed,
}

/// Spawn a metadata probe with a hard wall-clock ceiling. On timeout the child
/// is killed and handed to a detached reaper: `wait` itself may block forever
/// for a task in Linux D state, so the daemon chore must never wait there.
fn bounded_command_output(command: &mut Command, timeout: Duration) -> BoundedCommandOutput {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let Ok(mut child) = command.spawn() else {
        return BoundedCommandOutput::Failed;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if child
                    .stdout
                    .take()
                    .is_some_and(|mut pipe| pipe.read_to_end(&mut stdout).is_err())
                    || child
                        .stderr
                        .take()
                        .is_some_and(|mut pipe| pipe.read_to_end(&mut stderr).is_err())
                {
                    return BoundedCommandOutput::Failed;
                }
                return BoundedCommandOutput::Completed {
                    stdout,
                    stderr,
                    success: status.success(),
                };
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = std::thread::Builder::new()
                    .name("yggterm-version-probe-reaper".to_string())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                return BoundedCommandOutput::TimedOut;
            }
            Err(_) => return BoundedCommandOutput::Failed,
        }
    }
}

fn run_version_command(binary_path: &Path) -> Option<String> {
    let started = Instant::now();
    let outcome = bounded_command_output(
        Command::new(binary_path).arg("--version"),
        MANAGED_CLI_VERSION_PROBE_TIMEOUT,
    );
    let (outcome_name, output) = match outcome {
        BoundedCommandOutput::Completed { stdout, stderr, .. } => {
            let output = if stdout.is_empty() { stderr } else { stdout };
            ("completed", Some(output))
        }
        BoundedCommandOutput::TimedOut => ("timed_out", None),
        BoundedCommandOutput::Failed => ("failed", None),
    };
    if let Ok(home) = resolve_yggterm_home() {
        append_trace_event(
            &home,
            "server",
            "cli",
            "version_probe",
            serde_json::json!({
                "binary": binary_path.file_name().and_then(|name| name.to_str()),
                "outcome": outcome_name,
                "elapsed_ms": started.elapsed().as_millis(),
                "timeout_ms": MANAGED_CLI_VERSION_PROBE_TIMEOUT.as_millis(),
            }),
        );
    }
    output.and_then(|bytes| extract_version_token(&String::from_utf8_lossy(&bytes)))
}

fn extract_version_token(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let trimmed = token
            .trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | '(' | ')' | '[' | ']'))
            .trim_start_matches('v');
        if trimmed.is_empty() {
            return None;
        }
        let mut saw_digit = false;
        let mut saw_dot = false;
        for ch in trimmed.chars() {
            if ch.is_ascii_digit() {
                saw_digit = true;
                continue;
            }
            if ch == '.' {
                saw_dot = true;
                continue;
            }
            if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-') {
                continue;
            }
            return None;
        }
        if saw_digit && saw_dot {
            Some(trimmed.to_string())
        } else {
            None
        }
    })
}

fn resolve_binary_on_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|base| base.join(binary_name))
        .find(|candidate| candidate.is_file())
}

fn probe_tool(paths: &ManagedCliPaths, tool: ManagedCliTool) -> ToolProbe {
    let managed_binary = paths.bin_dir.join(tool.binary_name());
    if managed_binary.is_file() {
        return ToolProbe {
            version: run_version_command(&managed_binary),
            source: Some(ManagedCliBinarySource::Managed),
            available: true,
        };
    }
    // ⛔ LAUNCH PARITY, not the daemon's own `PATH`. An npm install lands in
    // `paths.bin_dir` and is found by the check above, so this asymmetry never
    // mattered while npm was the only method. A uv or vendor install lands in
    // `~/.local/bin`, which the daemon's `PATH` routinely omits — so probing
    // the daemon `PATH` alone would report a CLI we JUST installed as still
    // absent, and `ensure_local_managed_cli` would bail with "did not become
    // available after the managed install finished" on a successful install.
    // The paths-aware resolver form keeps a test's temp bin dir authoritative
    // for its own probe instead of leaking the real machine's installs in.
    if let Some(system_binary) =
        resolve_binary_for_launch_parity_with(Some(&paths.bin_dir), tool.binary_name())
    {
        return ToolProbe {
            version: run_version_command(&system_binary),
            source: Some(ManagedCliBinarySource::System),
            available: true,
        };
    }
    ToolProbe {
        version: None,
        source: None,
        available: false,
    }
}

/// Every file a running process is currently executing, best effort.
///
/// Linux reads `/proc/<pid>/exe` for each numeric pid. A deleted executable
/// still reports through the link with a ` (deleted)` suffix, and the prefix
/// match in [`generation_is_executed_by_running_process`] is on DIRECTORY
/// components, so the suffix never blocks a match. Any read failure costs one
/// pid's answer, never the sweep.
#[cfg(target_os = "linux")]
fn running_process_executable_paths() -> Vec<PathBuf> {
    let mut executables = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return executables;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        if let Ok(target) = fs::read_link(format!("/proc/{name}/exe")) {
            executables.push(target);
        }
    }
    executables
}

#[cfg(not(target_os = "linux"))]
fn running_process_executable_paths() -> Vec<PathBuf> {
    Vec::new()
}

/// Whether any running process executes from inside `generation_dir`.
fn generation_is_executed_by_running_process(
    generation_dir: &Path,
    live_executables: &[PathBuf],
) -> bool {
    live_executables
        .iter()
        .any(|executable| executable.starts_with(generation_dir))
}

fn npm_binary() -> Option<PathBuf> {
    resolve_binary_on_path("npm")
}

/// `uv` is installed into `~/.local/bin`, which the daemon's own `PATH`
/// routinely omits — so this must resolve with LAUNCH PARITY, exactly like the
/// CLIs themselves. Resolving it off the daemon `PATH` alone reported "uv is
/// unavailable" on guihost, where `~/.local/bin/uv` has been present since May.
fn uv_binary() -> Option<PathBuf> {
    resolve_binary_for_launch_parity("uv")
}

fn curl_binary() -> Option<PathBuf> {
    resolve_binary_for_launch_parity("curl")
}

/// The ONE thing yggterm will actually RUN to make a tool present and current.
///
/// Derived from the two registry axes — [`CliInstall`] (how it arrives) and
/// [`CliUpdate`] (how it stays current) — so that "what do we run for this CLI"
/// has one answer instead of one per call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProvisionStep {
    /// Batched into a single `npm install -g` line with every other npm tool.
    Npm,
    Uv(&'static str),
    VendorScript(&'static str),
    /// The CLI's own updater, e.g. `agy update`. Only reachable when the binary
    /// is already present — an updater cannot install what does not exist.
    SelfUpdate(&'static [&'static str]),
}

/// What to run for `tool`, or `None` when yggterm can do nothing for it.
///
/// ⭐ The self-updater WINS over the install method when the binary is already
/// there. That ordering is the ruling read literally: it was requested for
/// auto-install *and* auto-update, and a CLI that ships its own updater knows
/// where its payload lives in a way an install method re-run does not. It is
/// also the only thing that makes Antigravity — which yggterm cannot fetch at
/// all — a CLI yggterm keeps current rather than one it writes off.
fn provision_step(paths: &ManagedCliPaths, tool: ManagedCliTool) -> Option<ProvisionStep> {
    provision_step_for(
        tool.descriptor(),
        probe_tool_existence_only(paths, tool).available,
    )
}

/// The RULE, with the machine taken out of it.
///
/// ⚠ Split from [`provision_step`] because the rule could not otherwise be
/// tested: `agy` happens to be installed on guihost, so a test that asked for the
/// absent-CLI branch silently got the present one and passed for the wrong
/// reason. A decision that reads the filesystem cannot be locked by a test that
/// also reads the filesystem.
fn provision_step_for(descriptor: &AgentCliDescriptor, present: bool) -> Option<ProvisionStep> {
    if let CliUpdate::SelfCommand(argv) = descriptor.update
        && !argv.is_empty()
        && present
    {
        return Some(ProvisionStep::SelfUpdate(argv));
    }
    match descriptor.install {
        CliInstall::Npm(_) => Some(ProvisionStep::Npm),
        CliInstall::Uv(package) => Some(ProvisionStep::Uv(package)),
        CliInstall::VendorScript(url) => Some(ProvisionStep::VendorScript(url)),
        CliInstall::Manual => None,
    }
}

/// Whether yggterm has a way to make this tool present-and-current on THIS
/// machine right now: a step exists AND the thing that runs it is installed.
///
/// ⚠ Each method has its OWN prerequisite, and they are not interchangeable —
/// a machine with npm but no uv can refresh claude and not kimi. Answering this
/// with one global "is npm here" is how a uv CLI came to be reported
/// `system_fallback` on a machine that could have installed it.
fn provision_step_is_runnable(paths: &ManagedCliPaths, tool: ManagedCliTool) -> bool {
    match provision_step(paths, tool) {
        Some(ProvisionStep::Npm) => npm_binary().is_some(),
        Some(ProvisionStep::Uv(_)) => uv_binary().is_some(),
        Some(ProvisionStep::VendorScript(_)) => curl_binary().is_some(),
        Some(ProvisionStep::SelfUpdate(_)) => true,
        None => false,
    }
}

/// What actually happened, in the words of the method that did it.
///
/// ⛔ MEASURED WRONG, live on guihost 2026-08-08: every install reported *"a
/// Yggterm-managed <CLI> toolchain under ~/.yggterm/npm"* — including the
/// uv install that landed in `~/.local/bin`, the vendor install that landed in
/// `~/.local/bin`, and the `agy update` that installed nothing at all. Three
/// methods, one sentence, and it was true of only one of them. A status line
/// that names the wrong directory is how a user looking for the binary we just
/// installed concludes we did not install it.
fn provision_detail(paths: &ManagedCliPaths, tool: ManagedCliTool) -> String {
    let name = tool.display_name();
    match provision_step(paths, tool) {
        Some(ProvisionStep::Npm) => format!(
            "Installed or refreshed a Yggterm-managed {name} toolchain under {}.",
            paths.prefix.display()
        ),
        Some(ProvisionStep::Uv(package)) => {
            format!("Installed or upgraded {name} with `uv tool install --upgrade {package}`.")
        }
        Some(ProvisionStep::VendorScript(url)) => {
            format!("Installed or upgraded {name} by running the vendor installer at {url}.")
        }
        Some(ProvisionStep::SelfUpdate(argv)) => format!(
            "Updated {name} with its own updater, `{} {}`.",
            tool.binary_name(),
            argv.join(" ")
        ),
        None => format!("Yggterm cannot install or update {name} on this machine."),
    }
}

/// `uv tool install --upgrade <package>` — one command that both installs a
/// missing tool and upgrades a present one, so the install path and the update
/// path cannot drift apart.
///
/// ⛔ No prefix override: uv's default tool bin dir is `~/.local/bin`, which is
/// user-local and already on the login PATH. Forcing it under `~/.yggterm/npm`
/// would put a Python CLI inside the npm prefix and hide it from `uv tool list`.
fn install_via_uv(paths: &ManagedCliPaths, package: &str) -> Result<()> {
    let uv = uv_binary().context(
        "uv is required to install this CLI and is not on the login PATH — \
         install uv (https://astral.sh/uv) and the next refresh will pick it up",
    )?;
    let mut command = Command::new(uv);
    command.arg("tool").arg("install").arg("--upgrade").arg(package);
    apply_provision_env(&mut command, paths);
    run_provision_command(command, &format!("uv tool install {package}"))
}

/// Fetch a vendor installer and run it, unattended and user-local.
///
/// ⚠ Superseded doctrine, stated here so it is not silently reversed: until the
/// owner's 2026-08-08 ruling yggterm recorded this URL and refused to execute
/// it. It now executes it. What did NOT change is the boundary — `HOME` intact,
/// no privilege escalation, stdin closed so an installer that wants to ask a
/// question fails fast instead of hanging a background thread forever.
fn install_via_vendor_script(paths: &ManagedCliPaths, url: &str) -> Result<()> {
    let curl = curl_binary().context("curl is required to fetch a vendor CLI installer")?;
    let dir = paths.home.join(VENDOR_INSTALLER_DIRNAME);
    fs::create_dir_all(&dir)
        .with_context(|| format!("creating vendor installer dir {}", dir.display()))?;
    // Named for the URL, not for a counter or a clock: two refreshes of the same
    // CLI reuse one path, and two different CLIs never collide.
    let script = dir.join(format!("{}.sh", vendor_script_stem(url)));
    let mut fetch = Command::new(curl);
    fetch
        .arg("--fail")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--max-redirs")
        .arg("3")
        .arg("--proto")
        .arg("=https")
        .arg("--proto-redir")
        .arg("=https")
        .arg("--tlsv1.2")
        .arg("--max-time")
        .arg(VENDOR_FETCH_TIMEOUT_SECS.to_string())
        .arg("--output")
        .arg(&script)
        .arg(url);
    apply_provision_env(&mut fetch, paths);
    run_provision_command(fetch, &format!("fetching vendor installer {url}"))?;

    let mut run = Command::new("bash");
    run.arg(&script);
    // ⛔ THE ONE THAT MATTERS MOST. A vendor installer does its own `mktemp -d`
    //    and fetches a ~157 MB tarball into it, and does not remove it on every
    //    path. Without a disk-backed TMPDIR that lands in RAM and stays there.
    apply_provision_env(&mut run, paths);
    run_provision_command(run, &format!("running vendor installer {url}"))
}

/// Run a CLI's own updater, e.g. `agy update`.
fn update_via_self_command(
    paths: &ManagedCliPaths,
    tool: ManagedCliTool,
    argv: &[&str],
) -> Result<()> {
    let binary = resolve_binary_for_launch_parity(tool.binary_name()).with_context(|| {
        format!(
            "{} advertises its own updater but is not on the login PATH",
            tool.display_name()
        )
    })?;
    let mut command = Command::new(binary);
    command.args(argv);
    apply_provision_env(&mut command, paths);
    run_provision_command(
        command,
        &format!("{} {}", tool.binary_name(), argv.join(" ")),
    )
}

/// The `PATH` every provisioning subprocess runs with: the daemon's own, plus
/// the login-shell dirs. An installer that shells out to `curl`, `tar` or
/// `python` must see what a human's shell sees, or it fails on the daemon's
/// stripped `PATH` in ways no user can reproduce.
/// ⛔ EVERY PROVISIONING STEP STAGES ON DISK, NOT IN RAM — AND THIS IS THE ONE
/// PLACE THAT SAYS SO.
///
/// `/tmp` on the desktop host is a **tmpfs**, so a payload staged there is RAM
/// that the kernel can never drop: tmpfs pages can be swapped but not reclaimed,
/// so stale staging becomes permanent swap occupancy. Measured 2026-08-14 at
/// 2.85 GB of leaked npm staging plus 630 MB from a vendor installer, on a
/// 15 GB laptop already 11 GB into swap.
///
/// ⚠ THE npm PATH WAS FIXED AND THE OTHER TWO WERE NOT, which is the whole
/// reason this is a function. Found 2026-08-20: `install_via_uv` and
/// `install_via_vendor_script` both ran with the inherited `TMPDIR`, so a
/// `uv tool install` and — worse — a vendor `curl … | sh` that does its own
/// `mktemp -d` and fetches a ~157 MB tarball both staged straight into RAM. A
/// per-callsite fix is what let that survive; a shared helper is what stops the
/// next provisioning method being added without it.
fn apply_provision_env<'a>(command: &'a mut Command, paths: &ManagedCliPaths) -> &'a mut Command {
    command
        .env("PATH", provision_env_path())
        .env("TMPDIR", paths.staging_dir())
}

fn provision_env_path() -> OsString {
    let mut parts: Vec<PathBuf> = inherited_path_dirs();
    for dir in login_shell_path_dirs() {
        if !parts.contains(&dir) {
            parts.push(dir);
        }
    }
    env::join_paths(parts).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
}

/// The `PATH` this PROCESS carries — the daemon's own, which a launched PTY
/// inherits verbatim because the shell is spawned `-c` and not `-lc`.
fn inherited_path_dirs() -> Vec<PathBuf> {
    env::var_os("PATH")
        .map(|current| env::split_paths(&current).collect())
        .unwrap_or_default()
}

/// The directories a launched session's `PATH` is prefixed with, in search
/// order: the managed npm bin dir first (a yggterm-managed binary must outrank
/// a system copy — that ordering predates this function and is preserved), then
/// every login-shell dir the inherited `PATH` does not already carry.
///
/// Split out from [`ManagedCliPaths::launch_path_prefix`] — which explains WHY
/// this exists — so the composition is testable without a machine that happens
/// to be missing a directory.
/// ⛔ **THE LOGIN SHELL'S OWN ORDER IS THE AUTHORITY — do not "optimise" the
/// dirs the inherited `PATH` already carries out of this list.** Dropping them
/// looks like harmless dedup and is not: it promotes every REMAINING login dir
/// above the ones it removed, so a dir the user deliberately put LAST outranks
/// `/usr/bin`. Measured live on 3.0.68 within minutes of the first attempt —
/// `/opt/plan9/bin` sits last on this host's login `PATH`, the filtered prefix
/// hoisted it above `/usr/bin`, and every launched session got Plan 9's `date`,
/// which ignores `+%s` and prints `Thu Jan  1 ...`. A vendor launcher doing
/// `now="$(date +%s)"` under `set -u` then died with `Thu: unbound variable`.
/// Duplicates in a `PATH` cost nothing; a REORDERED `PATH` is a different
/// machine.
fn compose_launch_path_prefix_dirs(
    managed_bin_dir: Option<&Path>,
    user_local_bin_dir: Option<&Path>,
    login_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = managed_bin_dir.map(Path::to_path_buf).into_iter().collect();
    for dir in user_local_bin_dir.into_iter().map(Path::to_path_buf) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    for dir in login_dirs {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    dirs
}

/// Where yggterm's own NON-npm installs land: `uv tool install`'s default tool
/// bin dir and the target every vendor installer writes to — `CliInstall::Uv`
/// and `CliInstall::VendorScript` each name it, and [`install_via_uv`]
/// deliberately declines to override the prefix.
///
/// ⛔ It is on the launch `PATH` BY CONSTRUCTION, never because a login shell
/// was asked. [`login_shell_path_dirs`] is a parity EXTENSION — it makes a
/// session resolve binaries the way the user's own terminal does — but it
/// spawns a subprocess, and a subprocess can fail. A fix for "yggterm installed
/// this CLI and then could not run it" must not itself depend on something that
/// can fail; when the probe came back empty on 3.0.69, the whole fix silently
/// evaporated and `kimi: command not found` came back.
fn user_local_bin_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".local").join("bin"))
}

/// Every directory a launched session will search for a binary, in the order
/// it searches them — the launch prefix, then the inherited `PATH` the export
/// appends with `:"$PATH"`. The managed bin dir is NOT included: its callers
/// check it first themselves, which is the position it holds in the launch.
///
/// ⚠ The ORDER is load-bearing, not decoration. A probe that searched the
/// inherited `PATH` first would report the version and source of a DIFFERENT
/// copy than the launch execs whenever a binary sits in two dirs
/// ([[finding-a-build-identity-is-not-what-version-says]]).
fn launch_search_dirs() -> Vec<PathBuf> {
    let mut dirs = compose_launch_path_prefix_dirs(
        None,
        user_local_bin_dir().as_deref(),
        &login_shell_path_dirs(),
    );
    dirs.extend(inherited_path_dirs());
    dirs
}

/// A filesystem-safe stem for a vendor installer URL.
fn vendor_script_stem(url: &str) -> String {
    let stem: String = url
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    stem.trim_matches('-').to_string()
}

/// Run one provisioning subprocess with stdin CLOSED and stderr captured.
///
/// ⛔ `Stdio::null()` on stdin is load-bearing, not tidiness: this runs on a
/// background thread with no terminal, and an installer that stops to ask a
/// question would otherwise block that thread for the daemon's lifetime.
fn run_provision_command(mut command: Command, what: &str) -> Result<()> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("running {what}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        anyhow::bail!("{what} exited with status {}", output.status);
    }
    anyhow::bail!("{what} exited with status {}: {stderr}", output.status);
}

/// How long an install waits for another process to finish writing this
/// machine's managed toolchain. Generous because the thing being waited on is a
/// real `npm install -g` of up to nine packages, bounded because a wedged holder
/// must not pin a daemon thread for the life of the process.
const MANAGED_CLI_INSTALL_LOCK_WAIT_MS: u64 = 5 * 60_000;

/// Exclusive, CROSS-PROCESS lock over this machine's managed CLI toolchain,
/// held for the whole of one [`install_latest`].
///
/// ⛔ **THE DEFECT THIS CLOSES, measured 2026-08-09: two installs running at
/// once DELETE the CLI they are both installing.** Reproduced on `dev` by
/// running two `npm install -g opencode-ai@latest` against one
/// `NPM_CONFIG_PREFIX` while sampling the package directory every 50 ms: the
/// directory was ABSENT for 3 of 153 samples, both installs failed (`exit=1`
/// and `exit=239 EEXIST`), and `opencode` was left **entirely missing** from the
/// managed bin dir. A single hand-run afterwards restored it — which is exactly
/// the shape the bug was filed under, *"the provisioner fails where a hand-run
/// `npm install` succeeds"*. The hand-run does not succeed because a human typed
/// it; it succeeds because it is the only writer.
///
/// ⚠ **The filed mechanism was wrong, and it was wrong in a way worth
/// remembering.** The symptom is npm reporting `enoent spawn sh ENOENT`, which
/// reads as *"`sh` is not on `PATH`"* — so the entry blamed the daemon's frozen
/// environment. It is not that: `/bin/sh` is on every daemon `PATH` on all three
/// fleet hosts, and [`ManagedCliPaths::env_path`] PREPENDS to the inherited
/// `PATH` rather than replacing it. Node reports a spawn whose **`cwd` does not
/// exist** as `ENOENT` attributed to the COMMAND, so the missing thing was the
/// package directory the lifecycle script was told to run in — deleted by the
/// other install mid-flight. ⇒ [[finding-enoent-blames-the-command-for-a-missing-cwd]]
///
/// ⛔ It must be a FILE lock, not a `Mutex`, and that is forced by the
/// constitution rather than chosen: version-coexisting daemons are a guarantee
/// this project makes, a per-tool ensure arrives as its OWN short-lived process
/// over ssh (`remote_cli.rs` → `run_remote_ensure_managed_cli`), and the
/// scheduled fleet sweep is a third writer. An in-process lock cannot see any of
/// them. The three contenders are all real today: the remote ensure de-dupes on
/// `(machine_key, tool)`, so provisioning two DIFFERENT tools on one machine is
/// concurrent by construction.
#[derive(Debug)]
struct ManagedCliInstallLock {
    #[cfg(unix)]
    file: fs::File,
    #[cfg(unix)]
    home: PathBuf,
    #[cfg(unix)]
    path: PathBuf,
}

impl Drop for ManagedCliInstallLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            unsafe {
                let _ = libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
            append_trace_event(
                &self.home,
                "managed_cli",
                "install",
                "lock_released",
                serde_json::json!({
                    "path": self.path.display().to_string(),
                    "pid": std::process::id(),
                }),
            );
        }
    }
}

#[cfg(unix)]
fn managed_cli_install_lock_is_busy(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
}

/// Take the toolchain lock, WAITING for a holder rather than skipping past one.
///
/// ⭐ Waiting is the whole point and the obvious alternative is a bug: a caller
/// that gave up and returned would report "not installed" for a tool that is
/// being installed right now, and the launch gate above it would refuse a row
/// that was about to become valid. Waiting also makes the wait FREE in the
/// common case — by the time the lock is ours the other writer has usually
/// installed the very tool we wanted, and the probe that follows finds it.
///
/// ⚠ **WHO CAN BE MADE TO WAIT, stated plainly because it is a launch path.**
/// Every direct caller of `install_latest` is a `run_remote_*` verb — a
/// short-lived process spawned over ssh — and the only in-daemon caller is
/// `spawn_background_managed_cli_refresh`, which is on its own thread. So this
/// wait can NEVER be held while the daemon holds `&mut self`, and it cannot
/// stall other rows. It CAN delay one launch, when a sweep is mid-install and
/// that row's TTL says a refresh is due. **That delay is protective, not a
/// regression:** the alternative is exec'ing a binary another process is part
/// way through replacing. The TTL gate means most launches never reach here.
fn acquire_managed_cli_install_lock(home: &Path) -> Result<ManagedCliInstallLock> {
    acquire_managed_cli_install_lock_waiting(home, MANAGED_CLI_INSTALL_LOCK_WAIT_MS)
}

/// The body of [`acquire_managed_cli_install_lock`], with the deadline passed in
/// so a test can prove the contention behaviour without waiting out the real
/// five-minute budget.
fn acquire_managed_cli_install_lock_waiting(
    home: &Path,
    wait_ms: u64,
) -> Result<ManagedCliInstallLock> {
    #[cfg(unix)]
    {
        fs::create_dir_all(home)
            .with_context(|| format!("creating yggterm home {}", home.display()))?;
        let path = home.join("managed-cli-install.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("opening managed cli install lock {}", path.display()))?;
        let deadline = std::time::Instant::now() + Duration::from_millis(wait_ms);
        let mut waited = false;
        loop {
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                if waited {
                    append_trace_event(
                        home,
                        "managed_cli",
                        "install",
                        "lock_acquired_after_wait",
                        serde_json::json!({
                            "path": path.display().to_string(),
                            "pid": std::process::id(),
                        }),
                    );
                }
                return Ok(ManagedCliInstallLock {
                    file,
                    home: home.to_path_buf(),
                    path,
                });
            }
            let error = std::io::Error::last_os_error();
            if !managed_cli_install_lock_is_busy(&error) {
                return Err(anyhow!(error))
                    .with_context(|| format!("locking managed cli install {}", path.display()));
            }
            if std::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "another yggterm process has been installing managed CLIs for over {}ms \
                     (lock {}); refusing to write the toolchain concurrently",
                    wait_ms,
                    path.display()
                );
            }
            if !waited {
                waited = true;
                append_trace_event(
                    home,
                    "managed_cli",
                    "install",
                    "lock_busy",
                    serde_json::json!({
                        "path": path.display().to_string(),
                        "pid": std::process::id(),
                    }),
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (home, wait_ms);
        Ok(ManagedCliInstallLock {})
    }
}

fn install_latest(
    paths: &ManagedCliPaths,
    tools: &[ManagedCliTool],
    background: bool,
) -> Result<()> {
    // ⛔ ONE WRITER PER MACHINE, across processes. Held for the WHOLE function
    // rather than around the npm batch alone: uv and the vendor scripts install
    // into `~/.local/bin`, which every other lane also reads and writes, so the
    // resource being serialised is "this machine's managed toolchain", not "the
    // npm prefix". See [`ManagedCliInstallLock`] for the measurement.
    let _install_guard = acquire_managed_cli_install_lock(&paths.home)?;

    // ⛔ Each method runs SEPARATELY, and only the npm ones are batched. npm
    // fails a whole `install -g` batch on one unresolvable name, so a uv or
    // vendor CLI appended to that line would not install the wrong package — it
    // would take every OTHER tool's refresh down with it and report the failure
    // against all of them. A tool yggterm can neither install nor update is
    // SKIPPED here (the probe that follows reports it `unavailable` by name);
    // the by-name refusal a user reads lives at the launch site.
    let mut npm_tools: Vec<ManagedCliTool> = Vec::new();
    let mut per_tool: Vec<(ManagedCliTool, ProvisionStep)> = Vec::new();
    for tool in tools.iter().copied() {
        match provision_step(paths, tool) {
            Some(ProvisionStep::Npm) => npm_tools.push(tool),
            Some(step) => per_tool.push((tool, step)),
            None => {}
        }
    }

    // ⛔ Collected, never short-circuited: one CLI's vendor installer failing
    // must not stop the next CLI's update. The old single-method installer had
    // no such case, so `?` was safe there and is not safe here.
    let mut failures: Vec<String> = Vec::new();
    for (tool, step) in per_tool {
        let outcome = match step {
            ProvisionStep::Npm => unreachable!("npm tools are batched above"),
            ProvisionStep::Uv(package) => install_via_uv(paths, package),
            ProvisionStep::VendorScript(url) => install_via_vendor_script(paths, url),
            ProvisionStep::SelfUpdate(argv) => update_via_self_command(paths, tool, argv),
        };
        if let Err(error) = outcome {
            failures.push(format!("{}: {error}", tool.display_name()));
        }
    }

    if let Err(error) = install_npm_isolated(paths, &npm_tools, background) {
        failures.push(error.to_string());
    }

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", failures.join("; "))
    }
}

/// Install or refresh every npm-provisioned CLI — each in its OWN prefix, each
/// published only after it is proven to run.
///
/// ⛔ THIS REPLACED A SINGLE BATCHED `npm install -g --force <every package>`,
/// and the reason is measured, not stylistic. See [`ManagedCliPaths::cli_root`]
/// for the 2×2: the batch form spends several seconds of every pass with ALL
/// published binaries unlinked, so an interrupt anywhere in that window — a
/// reboot, an OOM kill, a daemon restart, a closed laptop — leaves the machine
/// with **no agent CLIs at all**. Reproduced twice, deterministically, 7 → 0.
///
/// Three properties this form has and that one could not:
///
/// - **Isolation.** One prefix per CLI, so a failure can only ever cost the CLI
///   it belongs to. The batch line failed all seven together by construction,
///   because npm fails a whole `install -g` on one bad name.
/// - **Verify-then-publish.** The staged tree must produce the binary before
///   anything is swapped; a install that "succeeds" without one is a failure
///   here, not a silently broken CLI discovered at launch.
/// - **Idempotence.** No `--force`, so an already-current tree is a cheap no-op
///   instead of a full rewrite of every package and a relink of every binary.
fn install_npm_isolated(
    paths: &ManagedCliPaths,
    npm_tools: &[ManagedCliTool],
    background: bool,
) -> Result<()> {
    if npm_tools.is_empty() {
        return Ok(());
    }
    let npm = npm_binary().context("npm is required to manage agent CLI toolchains")?;
    paths.ensure_dirs()?;
    fs::create_dir_all(paths.cli_root())
        .with_context(|| format!("creating per-CLI prefix root {}", paths.cli_root().display()))?;
    // ⛔ Reap what the last install abandoned before making more. See
    //    `staging_dir` for why this is not the package's job to be trusted with.
    paths.sweep_staging();

    // ⛔ Collected, never short-circuited — the same rule the per-tool loop
    //    above follows. One CLI's registry flake must not stop the next CLI's
    //    refresh, which is exactly what the shared batch line could not promise.
    // ⛔ Staggered like booter/monitor: each CLI sleeps 1s + jitter so 7x `npm install`
    //    or direct fetches don't spike 7 cores at once. Fleet sweep already has
    //    6h interval + 5m startup grace + superseded check; this adds per-CLI stagger.
    let mut failures: Vec<String> = Vec::new();
    for (idx, tool) in npm_tools.iter().copied().enumerate() {
        if idx > 0 {
            let stagger_ms = 1000 + managed_cli_tool_jitter_ms(tool, 30_000) % 2000;
            std::thread::sleep(std::time::Duration::from_millis(stagger_ms));
        }
        // Aggressive but lightweight: check registry HEAD first, install only if new
        // (direct fetcher does this inside run_direct_install via /latest; npm path
        // will be skipped here if direct). TTL still gates the whole sweep.
        if let Err(error) = install_one_npm_cli(paths, &npm, tool, background) {
            append_trace_event(
                &paths.home,
                "managed_cli",
                "install",
                "npm_cli_install_failed",
                serde_json::json!({
                    "tool": tool.descriptor().slug,
                    "package": tool.npm_package(),
                    "error": error.to_string(),
                }),
            );
            failures.push(format!("{}: {error}", tool.display_name()));
        }
    }

    // ⛔ Bounded, not emptied — see `gc_npm_cache_if_due`. Runs after the
    //    installs so a due collection never delays the binaries themselves.
    gc_npm_cache_if_due(paths, &npm);

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", failures.join("; "))
    }
}


/// How often the shared npm cache is garbage-collected.
///
/// ⛔ THE CACHE IS THE LARGEST SINGLE CONSUMER OF `~/.yggterm` ON EVERY FLEET
/// HOST and nothing had ever removed anything from it: measured 2026-08-14 at
/// 7.6 GB of a 9.5 GB tree on one host and 5.7 GB of 7.8 GB on another, with
/// content dating back five months.
///
/// ⭐ The fix is a RETENTION RULE, NOT A DELETE. The cache exists so that
/// provisioning is not a fresh download every time; emptying it on a timer
/// trades disk for network on the one path that has to be fast. `npm cache
/// verify` is npm's own garbage collector — it keeps everything the index
/// references and drops orphaned content — so it bounds the store without
/// costing a re-download of anything still in use.
///
/// ⚠ MEASURED 2026-08-20 on the build host: 9,322 MB → 7,669 MB, **1.65 GB
/// reclaimed in 61 s**. And npm's own report cannot be quoted for that number —
/// it said *"Content garbage-collected: 1306 (9,172,360,910 bytes)"*, i.e. 9.17 GB,
/// which over-states the disk actually returned by **5.5×**, because `_cacache`
/// deduplicates by content hash and the figure counts index entries rather than
/// unique bytes. Quote `du`, never npm's summary line.
const NPM_CACHE_GC_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Garbage-collect the shared npm cache, at most once per
/// [`NPM_CACHE_GC_INTERVAL`].
///
/// The interval is carried by the mtime of a marker file inside the cache
/// rather than by a new field in the refresh state: the question "when was this
/// cache last collected" belongs to the cache, and a second store that could
/// disagree with it is the duplicate this project's SSOT law forbids.
///
/// Best-effort throughout — a cache that cannot be collected is untidy, never a
/// failed provisioning pass.
fn gc_npm_cache_if_due(paths: &ManagedCliPaths, npm: &Path) {
    let marker = paths.cache_dir.join(".ygg-last-gc");
    let due = match fs::metadata(&marker).and_then(|meta| meta.modified()) {
        Ok(last) => last
            .elapsed()
            .map(|since| since >= NPM_CACHE_GC_INTERVAL)
            .unwrap_or(true),
        // No marker yet: this host has never collected, so it is due.
        Err(_) => true,
    };
    if !due {
        return;
    }
    // Size-triggered GC: if cache exceeds 500 MiB, collect immediately
    // regardless of interval — weekly alone left 7.3G on tmpfs (18G total
    // in /run/user/3001/yggterm-uglass). Measured verify reclaimed 5.7G.
    let cache_size_bytes = fs::read_dir(&paths.cache_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum::<u64>()
        })
        .unwrap_or(0);
    // Walk _cacache for more accurate size if small dir check undercounts
    let du_size = if cache_size_bytes < 100 * 1024 * 1024 {
        // Quick du via metadata walk depth 3 for _cacache
        fn du(path: &std::path::Path, depth: u32) -> u64 {
            if depth > 4 {
                return 0;
            }
            let Ok(entries) = std::fs::read_dir(path) else {
                return 0;
            };
            let mut total = 0;
            for e in entries.flatten() {
                if let Ok(m) = e.metadata() {
                    if m.is_file() {
                        total += m.len();
                    } else if m.is_dir() {
                        total += du(&e.path(), depth + 1);
                    }
                }
            }
            total
        }
        du(&paths.cache_dir, 0)
    } else {
        cache_size_bytes
    };
    let size_triggered = du_size > 100 * 1024 * 1024;
    if size_triggered {
        // Size trigger bypasses interval — tmpfs leak must be bounded promptly
    } else if !due {
        return;
    }
    // Written BEFORE the run, not after. The collection is slow (61 s on a 9 GB
    // cache) and may be killed; a marker written only on success would make
    // every interrupted pass retry it immediately, which is how a weekly chore
    // becomes a hot loop.
    let _ = fs::write(&marker, b"");

    let started = std::time::Instant::now();
    // When size-triggered, use `clean --force` to reclaim fully (verify left 2G
    // per uglass home, still above 128M threshold — 11G total). Periodic 1d
    // verify keeps hot entries; size trigger does full clean for tmpfs bound.
    let use_clean_force = size_triggered;
    let output = if use_clean_force {
        Command::new(npm)
            .env("npm_config_cache", &paths.cache_dir)
            .env("npm_config_update_notifier", "false")
            .arg("cache")
            .arg("clean")
            .arg("--force")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
    } else {
        Command::new(npm)
            .env("npm_config_cache", &paths.cache_dir)
            .env("npm_config_update_notifier", "false")
            .arg("cache")
            .arg("verify")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
    };
    append_trace_event(
        &paths.home,
        "managed_cli",
        "install",
        "npm_cache_gc",
        serde_json::json!({
            "cache": paths.cache_dir.display().to_string(),
            "ok": output.map(|out| out.status.success()).unwrap_or(false),
            "elapsed_ms": started.elapsed().as_millis() as u64,
        }),
    );
}

/// One CLI: install into an unpublished generation, prove it, then swap.
///
/// ⚠ On a platform without symlinks the publish step cannot be atomic, so the
/// generation layout is not used there and the install writes the shared prefix
/// directly, as it always did. Windows does not currently build, and widening
/// this is that platform's work, not a silent half-measure here.
fn install_one_npm_cli(
    paths: &ManagedCliPaths,
    npm: &Path,
    tool: ManagedCliTool,
    background: bool,
) -> Result<()> {
    let package = tool
        .npm_package()
        .expect("partitioned by the caller: only npm-provisionable tools reach here");

    #[cfg(unix)]
    {
        let slug = tool.descriptor().slug;
        let binary = tool.binary_name();
        let published = paths.published_generation(slug, binary);
        let generation = published.map(|current| current + 1).unwrap_or(1);
        let staged = paths.cli_generation_dir(slug, generation);

        // The only thing that can be here is a tree an interrupted run left.
        let _ = fs::remove_dir_all(&staged);
        fs::create_dir_all(&staged)
            .with_context(|| format!("creating staged prefix {}", staged.display()))?;

        // ⛔ REAP THE HALF-WRITTEN GENERATION ON FAILURE TOO, not only when the
        //    install "succeeded" without producing a binary. `?` here would
        //    return leaving the partial tree on disk; the next pass reaps it
        //    (it recomputes the SAME generation number, because nothing was
        //    published), so this is tidiness rather than correctness — but the
        //    size of what is left is set by WHERE the install died, and a
        //    network drop mid-download leaves far more than the 1 MB a registry
        //    resolution error does.
        let install_result = if managed_cli_fetcher_is_direct() {
            run_direct_install(
                paths,
                &staged,
                package,
                yggterm_core::agent_cli::npm_dist_tag(tool.descriptor().kind).unwrap_or("latest"),
            )
        } else {
            run_npm_install(paths, npm, &staged, package, background)
        };
        if let Err(error) = install_result {
            let _ = fs::remove_dir_all(&staged);
            return Err(error);
        }

        // ⛔ VERIFY BEFORE PUBLISHING — the binary must RUN, not merely exist.
        //    The exists-only check published claude, opencode and codex-litellm
        //    as vendor error shims: a text file on PATH that exits non-zero
        //    with an instruction paragraph on first use. A binary that cannot
        //    answer `--version` never reaches the publish symlink, and the
        //    install fails loudly with the vendor's own first error line.
        if let Err(why) = staged_binary_runs(&staged, binary) {
            let _ = fs::remove_dir_all(&staged);
            anyhow::bail!(
                "npm installed {package} but {binary} does not run ({why}) in {}",
                staged.display()
            );
        }

        paths.publish_cli_binary(slug, binary, generation)?;
        paths.prune_cli_generations(slug, generation);
        append_trace_event(
            &paths.home,
            "managed_cli",
            "install",
            "npm_cli_published",
            serde_json::json!({
                "tool": slug,
                "package": package,
                "generation": generation,
                "replaced": published,
            }),
        );
        Ok(())
    }

    #[cfg(not(unix))]
    {
        let prefix = paths.prefix.clone();
        if managed_cli_fetcher_is_direct() {
            run_direct_install(
                paths,
                &prefix,
                package,
                yggterm_core::agent_cli::npm_dist_tag(tool.descriptor().kind).unwrap_or("latest"),
            )
        } else {
            run_npm_install(paths, npm, &prefix, package, background)
        }
    }
}

/// The npm invocation itself, with the environment every managed install shares.
fn managed_cli_fetcher_is_direct() -> bool {
    // Custom direct tarball fetcher: aggressively checks registry via booter
    // (fleet sweep, TTL 6h) but never via per-row Incidental path — that is
    // what spiked CPU. Direct fetch has no npm cache/prefix on tmpfs (was
    // 1.5G prefix + 2G cache per uglass home, 11G tmpfs). It also recreates
    // the npm env boilerplate (npm_config_prefix, update_notifier, audit,
    // fund, PATH) so CLIs that inspect the env keep working.
    std::env::var("YGGTERM_MANAGED_CLI_FETCHER")
        .map(|v| v.eq_ignore_ascii_case("direct") || v == "1")
        .unwrap_or(true)
}

fn direct_platform_suffix() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("macos", "aarch64") => "darwin-arm64",
        ("windows", "x86_64") => "win32-x64",
        ("windows", "aarch64") => "win32-arm64",
        _ => "linux-x64",
    }
}

fn optional_native_package_for(package: &str) -> Option<String> {
    // Packages that distribute a native binary via optionalDependencies.
    // The JS shim `require`s the platform package at runtime; missing it is
    // the exact `Missing optional dependency @openai/codex-linux-x64` / claude
    // `native binary not installed` error from the screenshots.
    // Note: codex uses alias `npm:@openai/codex@<version>-<platform>` (same
    // package, version-suffixed), while claude uses separate package
    // `@anthropic-ai/claude-code-<platform>`. Both are handled below.
    let platform = direct_platform_suffix();
    match package {
        "@openai/codex" => Some(format!("@openai/codex-{platform}")),
        "@anthropic-ai/claude-code" => Some(format!("@anthropic-ai/claude-code-{platform}")),
        "@xai-official/grok" => Some(format!("@xai-official/grok-{platform}")),
        _ => None,
    }
}

/// The npm registry the direct fetcher talks to. Overridable ONLY for the
/// mock-registry harness (`scripts/mock-npm-registry/`), which proves the
/// install shapes against a local server; production always resolves to the
/// real registry.
fn npm_registry_base() -> String {
    std::env::var("YGGTERM_NPM_REGISTRY_BASE")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "https://registry.npmjs.org".to_string())
}

/// Resolve an optionalDependency entry to the (package, exact version) to
/// fetch. Vendors pin their platform packages to the main package's exact
/// version (`"opencode-linux-x64": "1.18.23"`); npm alias ranges
/// (`"npm:@scope/pkg@1.2.3"`) are unwrapped. Anything else (caret, tilde,
/// `*`, workspace ranges) cannot be resolved from a registry URL and is
/// SKIPPED rather than guessed at — a skipped optional dep is the
/// pre-existing failure mode, never a wrong-binary install.
fn exact_optional_dependency(name: &str, range: &str) -> Option<(String, String)> {
    if let Some(rest) = range.trim().strip_prefix("npm:") {
        let (package, version) = rest.rsplit_once('@')?;
        return Some((package.to_string(), version.to_string()));
    }
    if range.is_empty() || range.contains(|ch: char| "~^><=*| ,".contains(ch)) {
        return None;
    }
    let version = range.trim();
    let first = version.chars().next()?;
    if !(first.is_ascii_digit() || first == 'v') {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

/// Fetch every PLATFORM optional dependency the freshly-extracted package
/// declares, at its pinned version, into the same node_modules — what npm's
/// optional-dependency resolution would have left on disk. This is the
/// GENERAL form of what per-CLI special cases used to do: claude's native
/// binary, opencode's platform binary and grok's all arrive as optional
/// platform packages, and a main tarball without them is a CLI that prints
/// "native binary not installed" the moment a session starts it.
///
/// ⛔ A platform package that fails to fetch or extract is FATAL to the
/// install: the vendor declared it optional only because OTHER platforms
/// don't need it — OUR platform's package is exactly the one this machine
/// does need, and a published CLI without it is the "reports success, dies
/// on use" defect.
fn fetch_platform_optional_dependencies(
    curl: &Path,
    staging: &Path,
    prefix: &Path,
    package: &str,
    package_dir: &Path,
    skip: &[&str],
) -> Result<()> {
    let manifest_path = package_dir.join("package.json");
    let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
        return Ok(());
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };
    let Some(optional) = manifest
        .get("optionalDependencies")
        .and_then(|value| value.as_object())
    else {
        return Ok(());
    };
    let platform = direct_platform_suffix();
    for (name, range) in optional {
        let Some(range) = range.as_str() else { continue };
        if name.contains(platform) && !skip.contains(&name.as_str()) {
            let Some((dep_package, dep_version)) = exact_optional_dependency(name, range) else {
                anyhow::bail!(
                    "{package} declares platform dependency {name} at unresolvable range \
                     {range:?} — cannot fetch the native binary this machine needs"
                );
            };
            let dep_base = npm_registry_base();
            let manifest_url = format!("{dep_base}/{dep_package}/{}", dep_version);
            let meta_output = std::process::Command::new(curl)
                .arg("-fsSL")
                .arg(&manifest_url)
                .output()
                .with_context(|| format!("fetching manifest for {dep_package}"))?;
            if !meta_output.status.success() {
                anyhow::bail!(
                    "platform dependency {dep_package}@{dep_version} fetch failed: {}",
                    String::from_utf8_lossy(&meta_output.stderr)
                );
            }
            let dep_meta: serde_json::Value = serde_json::from_slice(&meta_output.stdout)
                .with_context(|| format!("parsing manifest for {dep_package}"))?;
            let dep_tarball = dep_meta
                .get("dist")
                .and_then(|dist| dist.get("tarball"))
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("no dist.tarball for {dep_package}"))?
                .to_string();
            fetch_and_extract_package(
                curl,
                staging,
                prefix,
                &dep_package,
                &dep_tarball,
                &dep_version,
            )?;
        }
    }
    Ok(())
}

/// Run a package's OWN install scripts — preinstall, install, postinstall, in
/// npm's order — from the package directory we just extracted and verified.
///
/// ⛔ THE DOCTRINE SHIFT, STATED: the direct fetcher was built to run no
/// lifecycle scripts, and that posture is what left claude, opencode and
/// codex-litellm shipping error-shim binaries fleet-wide — their native
/// binary ARRIVES as data (an optional package we already fetch) but is put
/// in PLACE by the vendor's script (`install.cjs`, `postinstall.mjs`,
/// `scripts/install.js`). Running the vendor's own script from the verified
/// tarball is the same trust decision npm makes and the provisioner already
/// makes for Muse's installer; NOT running it is a decision that every
/// script-finalized CLI is broken forever. The boundary that remains: the
/// scripts run from the package we extracted (never re-fetched), HOME intact,
/// stdin closed, TMPDIR on disk, bounded wall clock. Scripts are BEST-EFFORT
/// — the publish gate, not the script's exit code, decides whether an
/// install lands (see the gate and the measured grok case below).
fn run_vendor_install_scripts(
    paths: &ManagedCliPaths,
    package_dir: &Path,
    package: &str,
) -> Result<()> {
    const INSTALL_SCRIPT_ORDER: [&str; 3] = ["preinstall", "install", "postinstall"];
    let manifest_path = package_dir.join("package.json");
    let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
        return Ok(());
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };
    let Some(scripts) = manifest
        .get("scripts")
        .and_then(|value| value.as_object())
    else {
        return Ok(());
    };
    for step in INSTALL_SCRIPT_ORDER {
        let Some(script) = scripts.get(step).and_then(|value| value.as_str()) else {
            continue;
        };
        if script.trim().is_empty() {
            continue;
        }
        let mut command = Command::new("bash");
        command.arg("-c").arg(script);
        command.current_dir(package_dir);
        apply_provision_env(&mut command, paths);
        match bounded_command_output(&mut command, Duration::from_secs(300)) {
            BoundedCommandOutput::Completed { success, stderr, .. } => {
                // ⛔ BEST-EFFORT, AND WHY THE GATE OWNS THE VERDICT: a vendor
                // script can fail for reasons that do not matter to the
                // binary — measured live, grok's postinstall requires
                // `@iarna/toml`, a regular dependency the direct fetcher
                // (main tarball only) does not install, and grok itself runs
                // perfectly without it. Failing the install on every script
                // error would have kept a working CLI out of the fleet. The
                // publish gate (`staged_binary_runs`) is the contract: script
                // succeeded + binary runs -> publish; script failed + binary
                // still runs -> publish WITH the trace event below; binary
                // does not run -> the install fails with the shim's own
                // first error line.
                append_trace_event(
                    &paths.home,
                    "managed_cli",
                    "install",
                    if success {
                        "vendor_script_completed"
                    } else {
                        "vendor_script_failed_nonfatal"
                    },
                    serde_json::json!({
                        "package": package,
                        "script": step,
                        "success": success,
                        "first_error": String::from_utf8_lossy(&stderr)
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string(),
                    }),
                );
            }
            BoundedCommandOutput::TimedOut => {
                append_trace_event(
                    &paths.home,
                    "managed_cli",
                    "install",
                    "vendor_script_failed_nonfatal",
                    serde_json::json!({
                        "package": package,
                        "script": step,
                        "success": false,
                        "first_error": "timed out after 300s",
                    }),
                );
            }
            BoundedCommandOutput::Failed => {
                append_trace_event(
                    &paths.home,
                    "managed_cli",
                    "install",
                    "vendor_script_failed_nonfatal",
                    serde_json::json!({
                        "package": package,
                        "script": step,
                        "success": false,
                        "first_error": "could not start",
                    }),
                );
            }
        }
    }
    Ok(())
}

/// The publish gate: the freshly installed entry binary must EXIST and must
/// RUN (`--version` exits 0 within the probe budget). The exists-only check
/// this replaces published claude, opencode and codex-litellm as text shims
/// that print vendor error paragraphs and exit non-zero — present on PATH,
/// launch-parity-resolvable, and dead on first use.
fn staged_binary_runs(staged: &Path, binary: &str) -> std::result::Result<(), String> {
    let bin = staged.join("bin").join(binary);
    if !bin.is_file() {
        return Err(format!("no bin/{binary} was produced"));
    }
    let mut command = Command::new(&bin);
    command.arg("--version");
    match bounded_command_output(&mut command, MANAGED_CLI_VERSION_PROBE_TIMEOUT) {
        BoundedCommandOutput::Completed { stderr, success, .. } => {
            if success {
                Ok(())
            } else {
                let text = String::from_utf8_lossy(&stderr);
                Err(format!(
                    "--version exited non-zero: {}",
                    text.lines().next().unwrap_or("").trim()
                ))
            }
        }
        BoundedCommandOutput::TimedOut => Err("--version timed out".to_string()),
        BoundedCommandOutput::Failed => Err("--version could not be executed".to_string()),
    }
}

/// Whether the installed entry binary for `package` is a REAL executable
/// rather than a vendor error shim. ⛔ The fast-path version check reads only
/// `package.json`, and opencode's entry bins EXIST even when the install is
/// broken — a text file that prints "postinstall script was not run" and
/// exits. Without this health check, a broken opencode install would satisfy
/// the fast path forever and never self-heal. Both generations share the trap:
/// the abandoned v1 line (`opencode-ai`, bin `opencode.exe`) and the v2
/// preview (`@opencode-ai/cli`, bin `opencode2.exe`) — kept separate so a
/// host still carrying v1 keeps healing too.
fn direct_install_shim_is_healthy(prefix: &Path, package: &str) -> bool {
    let entry_bin = match package {
        "opencode-ai" => ("opencode-ai", "opencode.exe"),
        "@opencode-ai/cli" => ("@opencode-ai/cli", "opencode2.exe"),
        _ => return true,
    };
    let shim = prefix
        .join("lib")
        .join("node_modules")
        .join(entry_bin.0)
        .join("bin")
        .join(entry_bin.1);
    match fs::read(&shim) {
        Ok(bytes) => bytes.starts_with(&[0x7f, b'E', b'L', b'F']),
        Err(_) => false,
    }
}

fn native_tarball_url_for_codex(version: &str, platform: &str) -> String {
    // codex native is same package at version <base>-<platform>, e.g.
    // @openai/codex@0.149.1-linux-x64
    format!("https://registry.npmjs.org/@openai/codex/-/codex-{}-{}.tgz", version, platform)
}

fn fetch_and_extract_package(
    curl: &Path,
    staging: &Path,
    prefix: &Path,
    package: &str,
    tarball: &str,
    version: &str,
) -> Result<()> {
    let tmp_tgz = staging.join(format!("{}-{}.tgz", package.replace('/', "_"), version));
    let fetch = std::process::Command::new(curl)
        .arg("-fsSL")
        .arg("-o")
        .arg(&tmp_tgz)
        .arg(tarball)
        .output()
        .context("fetching tarball")?;
    if !fetch.status.success() {
        anyhow::bail!("tarball fetch failed for {package}: {}", String::from_utf8_lossy(&fetch.stderr));
    }
    let extract_dir = staging.join(format!("extract-{}-{}", package.replace('/', "_"), version));
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir).context("creating extract dir")?;
    let output = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&tmp_tgz)
        .arg("-C")
        .arg(&extract_dir)
        .output()
        .context("extracting tarball")?;
    if !output.status.success() {
        anyhow::bail!("tar extract failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let package_dir = extract_dir.join("package");
    let dest = prefix.join("lib").join("node_modules").join(package);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(dest.parent().unwrap()).context("creating node_modules")?;
    std::fs::rename(&package_dir, &dest).or_else(|_| {
        let status = std::process::Command::new("cp")
            .arg("-a")
            .arg(&package_dir)
            .arg(&dest)
            .status();
        status.and_then(|s| if s.success() { Ok(()) } else { Err(std::io::Error::new(std::io::ErrorKind::Other, "cp failed")) })
    }).context("moving package")?;
    let pkg_json = dest.join("package.json");
    if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(bin) = json.get("bin") {
                let bin_dir = prefix.join("bin");
                std::fs::create_dir_all(&bin_dir).context("creating bin")?;
                let bins: Vec<(String, String)> = if let Some(map) = bin.as_object() {
                    map.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect()
                } else if let Some(s) = bin.as_str() {
                    vec![(package.split('/').last().unwrap_or(package).to_string(), s.to_string())]
                } else {
                    vec![]
                };
                for (name, rel) in bins {
                    let src = dest.join(&rel);
                    let dst = bin_dir.join(&name);
                    let _ = std::fs::remove_file(&dst);
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
                    #[cfg(not(unix))]
                    std::fs::copy(&src, &dst).map(|_| ())?;
                    let _ = std::process::Command::new("chmod").arg("+x").arg(&dst).status();
                }
            }
        }
    }
    let _ = std::fs::remove_file(&tmp_tgz);
    let _ = std::fs::remove_dir_all(&extract_dir);
    Ok(())
}

fn run_direct_install(
    paths: &ManagedCliPaths,
    prefix: &Path,
    package: &str,
    dist_tag: &str,
) -> Result<()> {
    // Direct registry fetch — no npm, no cache, no tmpfs leak. Isolated from
    // system binaries: every CLI lands in its own generation under
    // `~/.yggterm/npm/cli/<slug>.gen<N>` and is published via atomic
    // `rename` of `bin/<binary>`; system `/usr/local/bin` is never touched.
    // GBs saved vs `npm install -g`: no `_cacache` duplication, no full
    // prefix rewrite per CLI, staging on disk (`TMPDIR=cli-staging`) not tmpfs.
    // Frequent everyday checks: fleet sweep + per-CLI TTL = 2h, with per-CLI
    // jitter (1-3s stagger) so 7 CLIs don't spike 7 cores, and download-once
    // via ygg daemon pre-warm.
    // Self-update safe: generation is unpublished until proven, so a running
    // `claude`/`grok` keeps its old inode through the swap; new launches see
    // the new symlink. No in-place overwrite of a live binary.
    let curl = curl_binary().context("curl is required for direct fetch")?;
    let registry_base = npm_registry_base();
    let registry_url = format!(
        "{registry_base}/{}/{}",
        package.replace('/', "%2F"),
        dist_tag
    );
    let meta_output = std::process::Command::new(&curl)
        .arg("-fsSL")
        .arg(&registry_url)
        .output()
        .context("fetching latest manifest")?;
    if !meta_output.status.success() {
        anyhow::bail!("registry fetch failed for {package}: {}", String::from_utf8_lossy(&meta_output.stderr));
    }
    let meta: serde_json::Value = serde_json::from_slice(&meta_output.stdout).context("parsing manifest")?;
    let tarball: String = meta
        .get("dist")
        .and_then(|d: &serde_json::Value| d.get("tarball"))
        .and_then(|v: &serde_json::Value| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("no dist.tarball for {package}"))?
        .to_string();
    let version: String = meta
        .get("version")
        .and_then(|v: &serde_json::Value| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let staging = paths.staging_dir();
    let _ = std::fs::create_dir_all(&staging);
    // Fast-path: already at latest version in this generation's dest — skip
    // download. The outer `install_one_npm_cli` already gates on TTL, but a
    // no-op download still costs 78MB + tar; version check avoids it.
    let dest_check = prefix.join("lib").join("node_modules").join(package).join("package.json");
    if let Ok(raw) = std::fs::read_to_string(&dest_check) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
            if json.get("version").and_then(|v| v.as_str()) == Some(version.as_str()) {
                // Also verify bin exists before skipping — a prior partial
                // install could have correct version but missing bin.
                let bin_ok = json.get("bin").map(|bin| {
                    let bins: Vec<String> = if let Some(map) = bin.as_object() {
                        map.keys().cloned().collect()
                    } else if let Some(s) = bin.as_str() {
                        vec![package.split('/').last().unwrap_or(package).to_string()]
                    } else { vec![] };
                    bins.iter().all(|name| prefix.join("bin").join(name).exists())
                }).unwrap_or(true);
                if bin_ok && direct_install_shim_is_healthy(prefix, package) {
                    return Ok(());
                }
            }
        }
    }
    let tmp_tgz = staging.join(format!("{}-{}.tgz", package.replace('/', "_"), version));
    fetch_and_extract_package(&curl, &staging, prefix, package, &tarball, &version)?;
    let package_dir = prefix
        .join("lib")
        .join("node_modules")
        .join(package);
    // Fetch the platform optional dependencies the package declares (claude's
    // native binary, opencode's platform binary, grok's) at their PINNED
    // versions — the general form of what per-CLI special cases used to do.
    // codex keeps its special case: its native is a version-SUFFIXED tarball
    // of the main package itself, which the optional-dependency walk cannot
    // express.
    let codex_native: Option<String> = if package == "@openai/codex" {
        optional_native_package_for(package).map(|value| value.to_string())
    } else {
        None
    };
    let skip: Vec<&str> = match codex_native.as_deref() {
        Some(special) => vec![special],
        None => Vec::new(),
    };
    if let Some(native_pkg) = codex_native.as_deref() {
        let platform = direct_platform_suffix();
        let native_tarball = native_tarball_url_for_codex(&version, platform);
        let native_version = format!("{}-{}", version, platform);
        // Try fetch; 404 gracefully skipped — not all versions publish every platform
        let _ = fetch_and_extract_package(&curl, &staging, prefix, native_pkg, &native_tarball, &native_version);
    }
    fetch_platform_optional_dependencies(&curl, &staging, prefix, package, &package_dir, &skip)?;
    // Run the vendor's own install scripts. Without them the finalize step
    // never happens: claude's install.cjs, opencode's postinstall.mjs and
    // codex-litellm's scripts/install.js each put the native binary in place,
    // and a skipped script left all three as error-shim text on PATH.
    run_vendor_install_scripts(paths, &package_dir, package)?;
    // ⛔ THE PUBLISH GATE, AT THE CHOKE POINT. Every bin the package declares
    // must RUN (`--version` exits 0) before this install reports success —
    // the caller's symlink publication hangs off it. The exists-only check
    // this generalizes published claude, opencode and codex-litellm as
    // vendor error shims: text on PATH, launch-parity-resolvable, dead on
    // first use. A vendor script that fails (best-effort by design) or a
    // package whose binary simply will not run fails HERE, loudly, with the
    // binary's own first error line.
    if let Ok(raw) = std::fs::read_to_string(package_dir.join("package.json"))
        && let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&raw)
        && let Some(bins) = manifest.get("bin").and_then(|value| value.as_object())
    {
        for name in bins.keys() {
            if let Err(why) = staged_binary_runs(prefix, name) {
                anyhow::bail!("{package} installed but {name} does not run ({why})");
            }
        }
    }
    // Recreate npm env boilerplate marker so `process.env.npm_config_prefix` checks pass
    // (CLIs inspected to complain when binary is copied without npm env).
    // We already export npm_config_prefix etc. in shell_exports, but also ensure
    // the prefix layout matches what `npm install -g` would have left.
    Ok(())
}

fn run_npm_install(
    paths: &ManagedCliPaths,
    npm: &Path,
    prefix: &Path,
    package: &str,
    background: bool,
) -> Result<()> {
    let mut command = Command::new(npm);
    command
        .env("NPM_CONFIG_PREFIX", prefix)
        .env("npm_config_prefix", prefix)
        .env("npm_config_cache", &paths.cache_dir)
        // ⛔ TMPDIR, not /tmp. A package's `preinstall` stages a 78 MB tarball
        //    via `os.tmpdir()` and never removes it; on the desktop host /tmp is
        //    a tmpfs, so that leak lands in RAM.
        //
        // ⚠ `npm_config_tmp` USED TO BE SET HERE AND IS GONE, because npm now
        //    rejects it outright: npm 11.16 answers `Unknown env config "tmp"`
        //    and ignores it. It bought nothing and taught the next reader that
        //    the knob still exists. `TMPDIR` is what actually moves the staging
        //    off RAM, and it is honoured by the package scripts that do the
        //    leaking, which are plain Node calling `os.tmpdir()`.
        .env("TMPDIR", paths.staging_dir())
        .env("npm_config_update_notifier", "false")
        .env("npm_config_audit", "false")
        .env("npm_config_fund", "false")
        .env("PATH", paths.env_path())
        .arg("install")
        .arg("-g");
    if background {
        command.arg("--silent");
    }
    // ⛔ NO `--force`. It was here to make a re-install of an already-current
    //    tree proceed, and what it actually bought was a full rewrite of every
    //    package plus a relink of every binary on every pass — the thing that
    //    turned a routine no-op refresh into the destructive window. A fresh
    //    generation directory is empty, so there is nothing left for it to force.
    command.arg(format!("{package}@latest"));
    let output = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("running npm install for {package}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        anyhow::bail!("npm install {package} exited with status {}", output.status);
    }
    anyhow::bail!(
        "npm install {package} exited with status {}: {}",
        output.status,
        stderr
    )
}

fn tool_status(
    tool: ManagedCliTool,
    before: ToolProbe,
    after: ToolProbe,
    action: &str,
    detail: String,
) -> ManagedCliToolStatus {
    let changed = before.version != after.version || before.source != after.source;
    ManagedCliToolStatus {
        tool,
        package_name: tool.package_name().to_string(),
        binary_name: tool.binary_name().to_string(),
        version_before: before.version,
        version_after: after.version,
        source_before: before.source,
        source_after: after.source,
        changed,
        available: after.available,
        action: action.to_string(),
        detail,
    }
}

/// Build a managed-CLI status from a CHEAP existence probe (no `--version`
/// subprocess, no npm install). This is what the focus/attach path returns:
/// the terminal switch must never block on a probe subprocess or a network
/// install. A present binary (managed or system) yields `ready`/`system_fallback`;
/// a truly-absent binary yields `unavailable` and the caller kicks a BACKGROUND
/// provision — never blocking the user's switch.
fn managed_cli_launch_status_from_probe(
    tool: ManagedCliTool,
    probe: ToolProbe,
) -> ManagedCliToolStatus {
    let (action, detail) = match (probe.available, probe.source) {
        (true, Some(ManagedCliBinarySource::Managed)) => (
            "ready",
            format!("{} is already managed by Yggterm.", tool.display_name()),
        ),
        (true, Some(ManagedCliBinarySource::System)) => (
            "system_fallback",
            format!(
                "{} is available from the system PATH; Yggterm will keep using it until an explicit managed refresh is requested.",
                tool.display_name()
            ),
        ),
        (true, None) => ("ready", format!("{} is available.", tool.display_name())),
        (false, _) => (
            "unavailable",
            format!(
                "{} is not available yet; Yggterm will not block terminal launch on npm install.",
                tool.display_name()
            ),
        ),
    };
    tool_status(tool, probe.clone(), probe, action, detail)
}

pub(crate) fn managed_cli_shell_command(
    kind: SessionKind,
    cwd: Option<&str>,
    action: ManagedCliAction<'_>,
) -> Result<String> {
    managed_cli_shell_command_with_terminal_appearance(kind, cwd, action, None)
}

pub(crate) fn managed_cli_shell_command_with_terminal_appearance(
    kind: SessionKind,
    cwd: Option<&str>,
    action: ManagedCliAction<'_>,
    terminal_appearance: Option<&str>,
) -> Result<String> {
    managed_cli_shell_command_full(
        kind,
        cwd,
        action,
        terminal_appearance,
        &AgentLaunchOptions::default(),
    )
}

pub(crate) fn managed_cli_shell_command_full(
    kind: SessionKind,
    cwd: Option<&str>,
    action: ManagedCliAction<'_>,
    terminal_appearance: Option<&str>,
    launch: &AgentLaunchOptions,
) -> Result<String> {
    managed_cli_shell_command_configured(kind, cwd, action, terminal_appearance, launch, None)
}

/// As above, with the CLIENT's configured flags supplied EXPLICITLY.
///
/// ⚖ A parameter, not a process env var. The remote lane's flags arrive on the
/// ssh line into the WRAPPER, and the daemon that composes the PTY command is a
/// different, long-lived process — measured 2026-08-13: the wrapper had
/// `YGGTERM_AGENT_EXTRA_ARGS` and the composing daemon did not, so the flag
/// crossed the hop and died at the daemon boundary. Claude Code's lane solves
/// that with `set_var` inside the daemon; doing the same for nine CLIs would
/// multiply a process-global write whose failure mode is one session's flags
/// leaking into the next. `None` ⇒ read the local settings store as before.
pub(crate) fn managed_cli_shell_command_configured(
    kind: SessionKind,
    cwd: Option<&str>,
    action: ManagedCliAction<'_>,
    terminal_appearance: Option<&str>,
    launch: &AgentLaunchOptions,
    configured_override: Option<&str>,
) -> Result<String> {
    let Some(tool) = ManagedCliTool::from_session_kind(kind) else {
        anyhow::bail!("session kind does not use a managed Codex CLI");
    };
    let paths = ManagedCliPaths::resolve()?;
    let has_cwd = cwd.filter(|value| !value.trim().is_empty()).is_some();
    let mut parts = Vec::new();
    if let Some(preamble) = best_effort_cwd_shell_prefix(cwd) {
        parts.push(preamble);
    }
    parts.push(paths.shell_exports_with_terminal_appearance(tool, terminal_appearance));
    let extra_args = composed_cli_extra_args_with(kind, launch, configured_override)?;
    // Invocation SHAPE is descriptor data now (harness spec §3, phase 1): which
    // CLI takes `--resume <id>` vs `resume <id>`, and which re-roots with
    // `-C "$PWD"`, is answered once in `yggterm_core::agent_cli` instead of by
    // an `is_claude` branch here — the fork class §7 inventories. The binary
    // name still comes from `ManagedCliTool` because provisioning owns it; the
    // descriptor and the tool are locked to the same string by
    // `managed_cli_tool_and_descriptor_agree_on_every_binary_name`.
    //
    // Per [[project-purpose]] wrapper-vs-manual parity rule the tokens carry
    // ONLY what the manual `ssh -t <machine> codex resume <UUID>` path uses:
    // the manual case produces correct scrollback without `--no-alt-screen`,
    // and adding it here was a wrong-headed guess. The real wrapper-vs-manual
    // divergence lives in yggterm's PTY/preservation path, not in CLI flags.
    let descriptor = yggterm_core::agent_cli::agent_cli_descriptor(kind);
    let (invocation, shape) = match action {
        ManagedCliAction::Launch => (
            format!("{}{}", tool.binary_name(), extra_args),
            CliInvocationShape {
                action: "launch",
                selector: "",
                carries_id: false,
                re_roots_with_cwd: false,
                extra_arg_tokens: 0,
                persistent: false,
            },
        ),
        ManagedCliAction::ResumePicker { persistent } => {
            let prefix = if persistent { "exec " } else { "" };
            let tokens = descriptor
                .map(|descriptor| descriptor.resume_picker_tokens())
                .unwrap_or_default();
            (
                format!(
                    "{prefix}{}{}{}",
                    tool.binary_name(),
                    extra_args,
                    join_invocation_tokens(&tokens)
                ),
                CliInvocationShape {
                    action: "resume_picker",
                    selector: descriptor
                        .map(|descriptor| descriptor.resume_selector_token())
                        .unwrap_or_default(),
                    carries_id: false,
                    re_roots_with_cwd: false,
                    extra_arg_tokens: 0,
                    persistent,
                },
            )
        }
        ManagedCliAction::Resume {
            session_id,
            persistent,
        } => {
            // ⛔ THE SES_ GUARD (2026-09-02, owner screenshot): opencode2's
            // service rejects any session id that does not start with `ses`
            // — and an UN-REBOUND row carries yggterm's birth uuid, which the
            // CLI then renders as a viewport error
            // (`Expected a string starting with "ses" at ["sessionID"]`).
            // A non-ses id on an opencode resume is a PHANTOM: compose a
            // fresh launch instead (the owner's `/sessions` flow re-binds the
            // real session), and say the phantom was dropped.
            if descriptor.is_some_and(|d| d.kind == crate::SessionKind::OpenCode)
                && !session_id.starts_with("ses_")
            {
                let prefix = if persistent { "exec " } else { "" };
                let shape = CliInvocationShape {
                    action: "launch",
                    selector: "",
                    carries_id: false,
                    re_roots_with_cwd: descriptor
                        .is_some_and(|descriptor| descriptor.resume_re_roots_with_cwd)
                        && has_cwd,
                    extra_arg_tokens: split_extra_args(&extra_args).len(),
                    persistent,
                };
                // Issue 31 probe: the composition refused what the descriptor
                // declares (a `--session <id>` resume) and degraded to a
                // fresh launch. Expected vs actual, at the moment of
                // composition — the launch event below would otherwise show
                // only the degraded shape with no word that a degrade
                // happened.
                #[cfg(not(test))]
                yggterm_core::cli_plane::emit_launch_contract(
                    "daemon",
                    kind,
                    descriptor
                        .map(|descriptor| descriptor.resume_selector_token())
                        .unwrap_or_default(),
                    shape,
                    yggterm_core::cli_plane::CliLaunchContractBreach::SesGuardDegrade,
                );
                (
                    format!(
                        "{prefix}{}{}",
                        tool.binary_name(),
                        extra_args
                    ),
                    shape,
                )
            } else {
            let prefix = if persistent { "exec " } else { "" };
            let quoted = shell_single_quote(session_id);
            let tokens = descriptor
                .map(|descriptor| descriptor.resume_tokens(&quoted, has_cwd))
                .unwrap_or_else(|| vec![quoted.clone()]);
            (
                format!(
                    "{prefix}{}{}{}",
                    tool.binary_name(),
                    extra_args,
                    join_invocation_tokens(&tokens)
                ),
                CliInvocationShape {
                    action: "resume",
                    selector: descriptor
                        .map(|descriptor| descriptor.resume_selector_token())
                        .unwrap_or_default(),
                    carries_id: !session_id.trim().is_empty(),
                    re_roots_with_cwd: descriptor
                        .is_some_and(|descriptor| descriptor.resume_re_roots_with_cwd)
                        && has_cwd,
                    extra_arg_tokens: 0,
                    persistent,
                },
            )
            }
        }
    };
    // ⭐ THE ONE COMPOSER, SO NO PER-CLI ARM CAN COMPOSE UNSEEN. This is where
    // "codex got `resume <id>`, Claude Code got `--resume <id>`, this one got
    // no selector at all" becomes a readable fact instead of something you
    // recover by reading `managed_cli`.
    //
    // ⛔ The SHAPE, never the composed line. `parts` carries the user's cwd,
    // their configured flags and an exported environment; putting it on the
    // trace plane would turn a diagnostic surface into a disclosure one.
    yggterm_core::cli_plane::emit_launch(
        "daemon",
        kind,
        CliInvocationShape {
            extra_arg_tokens: split_extra_args(&extra_args).len(),
            ..shape
        },
    );
    parts.push(invocation);
    Ok(parts.join(" && "))
}

/// Join descriptor invocation tokens onto a command, each space-prefixed.
///
/// Empty tokens ⇒ empty string, so a plain launch stays byte-identical to the
/// pre-descriptor `format!("{bin}{extra}")`.
fn join_invocation_tokens(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| format!(" {token}"))
        .collect::<String>()
}

/// The CLI's extra args for ONE launch: the user's configured args, with the
/// flags this launch overrides removed, plus the launch's own tokens.
///
/// **Per-launch wins, and "wins" means the configured flag is gone.** Leaving
/// both on the command line would make the CLI's own precedence rule the source
/// of truth for which model runs — a second encoding of a question yggterm
/// already answers.
///
/// ⛔ The launch options are NEVER written back to settings. A delegate asking
/// for bypass must not mutate `claude_code_extra_args`, which the user owns;
/// that requirement is what made the pre-flag workaround not acceptable.
#[allow(dead_code)]
pub(crate) fn composed_cli_extra_args(
    kind: SessionKind,
    launch: &AgentLaunchOptions,
) -> Result<String> {
    composed_cli_extra_args_with(kind, launch, None)
}

pub(crate) fn composed_cli_extra_args_with(
    kind: SessionKind,
    launch: &AgentLaunchOptions,
    configured_override: Option<&str>,
) -> Result<String> {
    let configured = match configured_override {
        Some(raw) => split_extra_args(raw),
        None => configured_cli_extra_arg_tokens(kind),
    };
    if launch.is_empty() {
        // Byte-identical to the pre-flag path for every human door.
        return Ok(shell_join_tokens(&configured));
    }
    let mut tokens = launch.strip_overridden(kind, &configured);
    tokens.extend(launch.launch_tokens(kind).map_err(|message| anyhow!(message))?);
    Ok(shell_join_tokens(&tokens))
}

#[allow(dead_code)]
fn configured_cli_extra_args(kind: SessionKind) -> String {
    shell_join_tokens(&configured_cli_extra_arg_tokens(kind))
}

/// The user's configured extra args for `kind`, as TOKENS.
///
/// Split out from the joined form because "per-launch wins" is a token-level
/// operation: you cannot remove `--model opus` from a shell-quoted string
/// without re-parsing it, and re-parsing it in a second place is how the two
/// spellings drift.
fn configured_cli_extra_arg_tokens(kind: SessionKind) -> Vec<String> {
    // Remote CC daemon-runtime lane: the CLIENT machine's configured claude
    // extra args (e.g. --dangerously-skip-permissions) travel to this host
    // via YGGTERM_CC_EXTRA_ARGS (ssh export → wrapper → daemon request env),
    // because the remote machine's own settings store does not have them.
    // The per-launch options of a REMOTE delegate ride this same variable,
    // already composed by the client (see `claude_extra_args_remote_exports`),
    // which is why this arm returns early and never re-reads local settings.
    if kind == SessionKind::ClaudeCode
        && let Ok(forwarded) = env::var(ENV_YGGTERM_CC_EXTRA_ARGS)
        && !forwarded.trim().is_empty()
    {
        return split_extra_args(&forwarded);
    }
    let raw = crate::configured_extra_args_for_kind(kind);
    split_extra_args(&raw)
}

/// Parse a configured extra-args STRING and shell-quote it back onto a command.
/// Kept as the one-shot spelling for callers that never override anything.
fn shell_join_extra_args(raw: &str) -> String {
    shell_join_tokens(&split_extra_args(raw))
}

pub(crate) fn split_extra_args(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    shlex::split(trimmed).unwrap_or_else(|| {
        trimmed
            .split_whitespace()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
}

/// Shell-quote tokens onto a command, space-prefixed. Empty ⇒ empty string, so
/// a launch with no extra args stays byte-identical to the pre-flag command.
pub(crate) fn shell_join_tokens(tokens: &[String]) -> String {
    if tokens.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            tokens
                .iter()
                .map(|arg| shell_single_quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

/// The marker the fallback notice opens with. The daemon greps for it, so it
/// is a constant rather than a phrase repeated in two places — a notice whose
/// wording only the shell knew would be a second encoding of the same event,
/// and the reader would go quiet the first time anyone reworded it.
pub const CWD_FALLBACK_NOTICE_MARKER: &str = "yggterm: requested directory not found here";

pub(crate) fn best_effort_cwd_shell_prefix(cwd: Option<&str>) -> Option<String> {
    let requested = cwd.map(str::trim).filter(|value| !value.is_empty())?;
    Some(format!(
        "__yggterm_requested={requested}; \
         __yggterm_cwd_ok=0; \
         __yggterm_cwd=\"$__yggterm_requested\"; \
         while [ -n \"$__yggterm_cwd\" ]; do \
           if cd \"$__yggterm_cwd\" 2>/dev/null; then \
             if [ \"$__yggterm_cwd\" = \"/\" ] && [ \"$__yggterm_requested\" != \"/\" ] && [ -n \"$HOME\" ]; then \
               cd \"$HOME\" 2>/dev/null || true; \
             fi; \
             __yggterm_cwd_ok=1; \
             break; \
           fi; \
           if [ \"$__yggterm_cwd\" = \"/\" ]; then break; fi; \
           __yggterm_next=$(dirname -- \"$__yggterm_cwd\"); \
           if [ \"$__yggterm_next\" = \"$__yggterm_cwd\" ]; then break; fi; \
           __yggterm_cwd=\"$__yggterm_next\"; \
         done; \
         if [ \"$__yggterm_cwd_ok\" != 1 ] && [ -n \"$HOME\" ]; then cd \"$HOME\" 2>/dev/null || true; fi; \
         if [ \"$__yggterm_cwd\" != \"$__yggterm_requested\" ]; then \
           printf '%s\\n' \"{marker}: $__yggterm_requested\" \
             \"yggterm: starting in $PWD instead — work done here is NOT in the directory this row names.\" >&2 \
             || true; \
         fi",
        requested = shell_single_quote(requested),
        marker = CWD_FALLBACK_NOTICE_MARKER,
    ))
}

/// How long a focus-path managed-CLI ensure result is reused before re-probing.
/// `ensure_local_managed_cli` runs a `<cli> --version` subprocess (`probe_tool` →
/// `run_version_command`) UNCONDITIONALLY at its top, before any TTL gate — and for
/// the node-based `claude` CLI that spawn-and-wait costs ~100-400ms. It sits on the
/// terminal-attach critical path (`ensure_managed_cli_for_session_path` →
/// `ensure_terminal_for_path` → the OpenStoredSession reply), so every local agent
/// session focus/switch paid it. That made cold switching feel slow ("the server can
/// attach in a non-blocking IO manner" — user, 2026-06-13). The focus path does not
/// need a fresh probe on every click; it only needs to know the tool is available so
/// the PTY launch works. We memoize the ensure result for a short window so a burst of
/// switches pays the probe at most once. First-run install still works (cache miss →
/// full ensure → install if needed), and the window is short enough that a genuine
/// uninstall self-heals within it.
const MANAGED_CLI_FOCUS_PROBE_TTL_MS: u64 = 60_000;

#[allow(clippy::type_complexity)]
fn managed_cli_focus_cache()
-> &'static std::sync::Mutex<BTreeMap<&'static str, (ManagedCliToolStatus, u64)>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<BTreeMap<&'static str, (ManagedCliToolStatus, u64)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

/// Pure freshness gate for the focus cache: a cached entry is reusable only when the
/// last ensure found the tool available AND it is within the TTL window. A
/// not-available cached entry is never reused, so the next focus re-runs the full
/// ensure and keeps trying to provision.
fn managed_cli_focus_cache_entry_is_fresh(available: bool, cached_at_ms: u64, now_ms: u64) -> bool {
    available && now_ms.saturating_sub(cached_at_ms) < MANAGED_CLI_FOCUS_PROBE_TTL_MS
}

/// The PATH the LAUNCHED session will actually use, resolved once from the user's
/// login shell and cached for the daemon's lifetime. The daemon process runs with a
/// non-login environment whose `PATH` frequently omits `~/.local/bin` (where the
/// fleet installs codex/claude), while the PTY launch command runs under
/// `bash -lc` and DOES see it ([[spec-cli-binary-auto-provisioning]] login_shell_wrap).
/// Without this, the daemon's existence probe wrongly concludes a present CLI is
/// absent and fires a pointless `npm install` on every cold focus (the 5.5s stall
/// measured on guihost, 2026-06-14). Resolving via the login shell closes that gap so the
/// probe matches launch parity. One subprocess per daemon lifetime; never on the hot path.
///
/// ⛔ **A FAILED PROBE IS NOT CACHED, and that is the whole point of the mutex.**
/// This was a `OnceLock`, so the FIRST answer stood for the process's whole life
/// — including an empty one. Measured live on 3.0.69: a daemon that had just
/// restored 40 sessions under swap pressure got nothing back from its one
/// `bash -lc`, froze `[]`, and every session it launched afterwards was composed
/// with the managed npm dir alone. The symptom was the ORIGINAL bug returning at
/// random (`kimi: command not found` on a host with kimi installed), which is
/// worse than the bug: it is the bug, intermittently, with a fix in the tree.
/// A miss now costs one more subprocess on the next call and nothing else.
fn login_shell_path_dirs() -> Vec<PathBuf> {
    static DIRS: std::sync::Mutex<Option<Vec<PathBuf>>> = std::sync::Mutex::new(None);
    if let Ok(cached) = DIRS.lock()
        && let Some(dirs) = cached.as_ref()
    {
        return dirs.clone();
    }
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let output = Command::new(&shell)
        .arg("-lc")
        .arg("printf %s \"$PATH\"")
        .output();
    let dirs: Vec<PathBuf> = match output {
        Ok(output) if output.status.success() => {
            let path = String::from_utf8_lossy(&output.stdout);
            env::split_paths(path.trim()).collect()
        }
        _ => Vec::new(),
    };
    // Only a USEFUL answer is remembered. An empty one means the probe failed
    // (or the login shell genuinely has no PATH, which is the same thing for
    // our purposes) and must be retried rather than enshrined.
    if !dirs.is_empty()
        && let Ok(mut cached) = DIRS.lock()
    {
        *cached = Some(dirs.clone());
    }
    dirs
}

/// Resolve a binary the way the launched session will: daemon `PATH` first (cheap,
/// already in-process), then the cached login-shell `PATH`. Existence check only —
/// no `--version` subprocess.
///
/// Managed npm bin dir (`~/.yggterm/npm/bin` where `grok` lives) is checked
/// first so a CLI present via yggterm's own `npm install` is found even when
/// no login-shell `PATH` carries it — otherwise `grok` was reported absent and
/// required a `~/.local/bin/grok` symlink that then broke `grok update`'s
/// `npm i -g` (EEXIST).
pub(crate) fn resolve_binary_for_launch_parity(binary_name: &str) -> Option<PathBuf> {
    resolve_binary_for_launch_parity_with(
        ManagedCliPaths::resolve().ok().map(|paths| paths.bin_dir).as_deref(),
        binary_name,
    )
}

/// The body of [`resolve_binary_for_launch_parity`], with the managed bin dir
/// taken in so a caller holding explicit paths (and a test) answers for THAT
/// configuration instead of whatever the real home resolves to.
pub(crate) fn resolve_binary_for_launch_parity_with(
    managed_bin_dir: Option<&Path>,
    binary_name: &str,
) -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = managed_bin_dir {
        if !dirs.contains(&dir.to_path_buf()) {
            dirs.push(dir.to_path_buf());
        }
    }
    dirs.extend(launch_search_dirs());
    dirs.into_iter()
        .map(|base| base.join(binary_name))
        .find(|candidate| candidate.is_file())
}

/// Cheap, subprocess-free existence probe for the focus/attach path. Mirrors
/// `probe_tool`'s source resolution (managed bin dir, else launch-parity PATH) but
/// SKIPS `run_version_command` — the attach must never block on a `<cli> --version`
/// subprocess. `version` is left `None`; the background ensure fills it in.
fn probe_tool_existence_only(paths: &ManagedCliPaths, tool: ManagedCliTool) -> ToolProbe {
    let managed_binary = paths.bin_dir.join(tool.binary_name());
    if managed_binary.is_file() {
        return ToolProbe {
            version: None,
            source: Some(ManagedCliBinarySource::Managed),
            available: true,
        };
    }
    if resolve_binary_for_launch_parity(tool.binary_name()).is_some() {
        return ToolProbe {
            version: None,
            source: Some(ManagedCliBinarySource::System),
            available: true,
        };
    }
    ToolProbe {
        version: None,
        source: None,
        available: false,
    }
}

/// Why a LOCAL agent-CLI PTY must be REFUSED instead of spawned — or `None`
/// when the binary the launch will exec genuinely resolves.
///
/// ⛔ The defect this closes, reported 2026-08-08: a missing binary was
/// not a launch failure ANYWHERE in the product. The launch command is
/// `bash -lc '<exports> && muse'`; with no `muse` on the machine, bash printed
/// `muse: command not found`, **exited that one command, and stayed alive at a
/// prompt**. The row went `healthy`, `launch_phase:Running`, `last_launch_error:
/// none` — and the only instrument that could answer "did my CLI start?" was
/// reading the screen text. Nine CLIs are first-class and several are absent on
/// any given host, so that is the common case, not an edge.
///
/// ⚠ [[finding-a-set-is-not-a-fill]]: this is a READBACK, never the descriptor's
/// own hope. It reuses [`probe_tool_existence_only`] — the same launch-parity
/// resolution (managed bin dir, else the login-shell PATH the PTY will run
/// under) the focus path already trusts to decide `available` — so the gate and
/// the provisioner can never disagree about whether a binary is there.
/// ⚠ [[finding-a-build-identity-is-not-what-version-says]]: deliberately NO
/// `--version`/`--help` probe. Those are pure builtins exempt from the exec
/// handoff, so neither can prove a binary exists at the far end of a launch —
/// and a subprocess here would sit on the terminal-attach critical path.
///
/// A machine whose managed layout cannot be resolved at all answers `None`: this
/// gate exists to refuse a launch that is KNOWN to fail, and "I could not look"
/// is not that.
pub(crate) fn local_agent_cli_missing_binary_refusal(tool: ManagedCliTool) -> Option<String> {
    let paths = ManagedCliPaths::resolve().ok()?;
    if probe_tool_existence_only(&paths, tool).available {
        return None;
    }
    Some(missing_binary_refusal_message(tool.descriptor()))
}

/// The words a refused launch shows. Split from the probe so the message is
/// testable without a machine that happens to lack a CLI — the probe is a
/// filesystem fact, this is a contract.
fn missing_binary_refusal_message(descriptor: &AgentCliDescriptor) -> String {
    format!(
        "{} is not installed on this machine — `{}` is not on the launch PATH, so yggterm cannot start this session. {}",
        descriptor.display_name,
        descriptor.binary_name,
        descriptor.install_instruction(),
    )
}

/// Per-tool in-flight guard so at most one background provision/refresh runs per tool
/// at a time (no thread pile-up under rapid switching). The 60s focus cache rate-limits
/// the spawn cadence to <=1/min while actively switching.
fn managed_cli_background_inflight()
-> &'static std::sync::Mutex<std::collections::BTreeSet<&'static str>> {
    static INFLIGHT: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeSet<&'static str>>> =
        std::sync::OnceLock::new();
    INFLIGHT.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeSet::new()))
}

/// Run the full `ensure_local_managed_cli` (probe + TTL-gated install) on a detached
/// thread so it NEVER blocks the terminal-attach reply. Used for two cases off the hot
/// path: (1) genuine first-run provisioning when the binary is absent everywhere
/// (including the login-shell PATH), and (2) the 6h auto-update of a Yggterm-MANAGED
/// binary. Deduped per tool. On success it primes the focus cache so the next switch is
/// instant. `ensure_local_managed_cli` touches only `ManagedCliPaths`/filesystem/npm —
/// never the daemon mutex — so this is safe to spawn from under it.
fn spawn_background_managed_cli_refresh(tool: ManagedCliTool) {
    let name = tool.binary_name();
    {
        let Ok(mut inflight) = managed_cli_background_inflight().lock() else {
            return;
        };
        if !inflight.insert(name) {
            return;
        }
    }
    std::thread::spawn(move || {
        let result = ensure_local_managed_cli(tool);
        if let Ok(status) = result
            && status.available
            && let Ok(mut cache) = managed_cli_focus_cache().lock()
        {
            cache.insert(tool.binary_name(), (status, current_time_ms()));
        }
        if let Ok(mut inflight) = managed_cli_background_inflight().lock() {
            inflight.remove(name);
        }
    });
}

/// Focus/attach-path entry point for the managed-CLI ensure. The terminal switch must
/// be BLAZING FAST: it never runs a `<cli> --version` subprocess or a blocking npm
/// install. It returns the status of a CHEAP existence probe (managed bin dir, else the
/// login-shell PATH the launch will actually use) and defers all subprocess/network work
/// to a background thread:
///   - binary present (managed or system) -> return immediately + cache (fast repeat
///     focuses). For a Yggterm-MANAGED binary, also kick a background ensure so the 6h
///     auto-update still happens, off the switch path.
///   - binary absent everywhere -> return `unavailable` immediately (the session launch,
///     which runs under a login shell, resolves the CLI itself) and kick a background
///     provision so a genuinely fresh machine still installs — never freezing the switch.
/// The 60s in-process cache rate-limits the cheap probe + background spawn to <=1/min.
/// Explicit refresh paths (`refresh_local_managed_cli`) still call
/// `ensure_local_managed_cli`/`install_latest` directly so a user/chore refresh re-probes.
pub(crate) fn ensure_local_managed_cli_for_focus(
    tool: ManagedCliTool,
) -> Result<ManagedCliToolStatus> {
    let now_ms = current_time_ms();
    if let Ok(cache) = managed_cli_focus_cache().lock()
        && let Some((status, cached_at_ms)) = cache.get(tool.binary_name())
        && managed_cli_focus_cache_entry_is_fresh(status.available, *cached_at_ms, now_ms)
    {
        return Ok(status.clone());
    }
    let paths = ManagedCliPaths::resolve()?;
    let probe = probe_tool_existence_only(&paths, tool);
    let status = managed_cli_launch_status_from_probe(tool, probe.clone());
    if probe.available {
        if let Ok(mut cache) = managed_cli_focus_cache().lock() {
            cache.insert(tool.binary_name(), (status.clone(), now_ms));
        }
        // A managed binary still needs its periodic 6h refresh — in the background.
        // A system binary is the user's to update; don't churn npm trying to shadow it.
        if probe.source == Some(ManagedCliBinarySource::Managed) {
            spawn_background_managed_cli_refresh(tool);
        }
    } else {
        spawn_background_managed_cli_refresh(tool);
    }
    Ok(status)
}

pub(crate) fn ensure_local_managed_cli(tool: ManagedCliTool) -> Result<ManagedCliToolStatus> {
    let paths = ManagedCliPaths::resolve()?;
    let now_ms = current_time_ms();
    let ttl_ms = managed_cli_refresh_ttl_ms();
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "ensure_begin",
        serde_json::json!({ "tool": tool.binary_name() }),
    );
    let before = probe_tool(&paths, tool);
    let refresh_state = load_managed_cli_refresh_state(&paths.home);
    // A CLI yggterm does not install is one it may only DETECT. Gating the
    // refresh on it here (rather than letting `install_latest` bail) keeps the
    // ready/system_fallback answer for a uv- or vendor-installed CLI that is
    // sitting on PATH — refusing to install must not read as "unavailable".
    // ⛔ Was `npm_binary().is_some() && tool.npm_package().is_some()`, which
    // answered `false` for every uv, vendor-script and self-updating CLI and so
    // silently exempted three of the nine registered CLIs from the refresh the
    // owner ruled must cover all of them (2026-08-08).
    let provisioner_available = provision_step_is_runnable(&paths, tool);
    if provisioner_available
        && managed_cli_explicit_refresh_needed(tool, &before, &refresh_state, now_ms, ttl_ms)
    {
        install_latest(&paths, &[tool], false)?;
        let after = probe_tool(&paths, tool);
        if !after.available {
            anyhow::bail!(
                "{} did not become available after the managed install finished",
                tool.display_name()
            );
        }
        let status = tool_status(
            tool,
            before,
            after,
            "installed",
            provision_detail(&paths, tool),
        );
        if let Err(error) = persist_managed_cli_refresh_state(
            &paths.home,
            &[(tool, probe_tool(&paths, tool))],
            now_ms,
        ) {
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "ensure_state_write_error",
                serde_json::json!({
                    "tool": tool.binary_name(),
                    "error": error.to_string(),
                }),
            );
        }
        append_trace_event(
            &paths.home,
            "server",
            "managed_cli",
            "ensure_end",
            serde_json::json!({
                "tool": tool.binary_name(),
                "action": status.action.clone(),
                "available": status.available,
                "changed": status.changed,
            }),
        );
        return Ok(status);
    }
    if before.available {
        let detail = match before.source {
            Some(ManagedCliBinarySource::Managed) => {
                format!("{} is already managed by Yggterm.", tool.display_name())
            }
            Some(ManagedCliBinarySource::System) => format!(
                "{} is currently coming from the system PATH. Yggterm will keep using it until an explicit managed refresh is requested.",
                tool.display_name()
            ),
            None => format!("{} is available.", tool.display_name()),
        };
        let status = tool_status(tool, before.clone(), before, "ready", detail);
        if status.source_after == Some(ManagedCliBinarySource::Managed)
            && let Err(error) = persist_managed_cli_refresh_state(
                &paths.home,
                &[(tool, probe_tool(&paths, tool))],
                now_ms,
            )
        {
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "ensure_state_write_error",
                serde_json::json!({
                    "tool": tool.binary_name(),
                    "error": error.to_string(),
                }),
            );
        }
        append_trace_event(
            &paths.home,
            "server",
            "managed_cli",
            "ensure_end",
            serde_json::json!({
                "tool": tool.binary_name(),
                "action": status.action.clone(),
                "available": status.available,
                "changed": status.changed,
            }),
        );
        return Ok(status);
    }

    // Absent AND not ours to install. Refused BY NAME, with the source the
    // descriptor declares, because "yggterm will fix this for you" is a promise
    // it cannot keep for a uv/vendor/manual CLI and a silent npm attempt would
    // install nothing under a name that looks right.
    if !tool.descriptor().install.provisions_unattended() {
        anyhow::bail!(
            "{} is not installed and yggterm cannot fetch it — install it yourself from {}",
            tool.display_name(),
            tool.package_name()
        );
    }
    install_latest(&paths, &[tool], false)?;
    let after = probe_tool(&paths, tool);
    if !after.available {
        anyhow::bail!(
            "{} did not become available after the managed install finished",
            tool.display_name()
        );
    }
    let status = tool_status(
        tool,
        before,
        after,
        "installed",
        provision_detail(&paths, tool),
    );
    if let Err(error) =
        persist_managed_cli_refresh_state(&paths.home, &[(tool, probe_tool(&paths, tool))], now_ms)
    {
        append_trace_event(
            &paths.home,
            "server",
            "managed_cli",
            "ensure_state_write_error",
            serde_json::json!({
                "tool": tool.binary_name(),
                "error": error.to_string(),
            }),
        );
    }
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "ensure_end",
        serde_json::json!({
            "tool": tool.binary_name(),
            "action": status.action.clone(),
            "available": status.available,
            "changed": status.changed,
        }),
    );
    Ok(status)
}

/// The provisioning key serving a descriptor slug — the row key the
/// CLI-installation modal carries — or `None` for a slug the registry does
/// not know.
pub fn managed_cli_tool_for_slug(slug: &str) -> Option<ManagedCliTool> {
    managed_cli_tools_for_refresh()
        .into_iter()
        .find(|tool| tool.descriptor().slug == slug)
}

/// Remove a CLI THIS machine no longer wants, by provisioning key.
///
/// ⛔ **Only user-space installs are removable.** A binary resolving under a
/// system prefix (`/usr/local`, `/usr`, `/opt`) is either root-owned or
/// package-manager-owned, and deleting it is how a "remove Qwen" click becomes
/// a broken machine for every other user of the host. The refusal is BY PATH,
/// so the user can remove it by hand knowing exactly which file it is. This is
/// the removal-side twin of the no-`sudo` rule in the auto-provisioning spec.
///
/// What IS removed, by install method:
/// - npm-managed: the published symlink in the managed bin dir plus every
///   `<slug>.gen*` generation under the managed CLI root — the whole tree
///   `install_npm_isolated` created, nothing else.
/// - uv: `uv tool uninstall <package>`, which removes the tool's bin shims and
///   its tool directory together, so no dangling entry survives in
///   `uv tool list`.
/// - vendor-script / self-updating: the binary file itself, ONLY when it lives
///   in a user-local bin dir (the managed bin dir or `~/.local/bin`).
///
/// The install lock is held for the whole body: removal mutates the same
/// managed tree the install funnel writes, and the lock — not goodwill — is
/// what keeps a concurrent refresh from re-installing the tool being removed
/// ([[ManagedCliInstallLock]]).
pub(crate) fn remove_local_managed_cli(tool: ManagedCliTool) -> Result<ManagedCliToolStatus> {
    remove_local_managed_cli_with_paths(&ManagedCliPaths::resolve()?, tool)
}

/// The body of [`remove_local_managed_cli`], with the machine's paths taken in
/// so a test can prove the removal semantics without touching the real home.
pub(crate) fn remove_local_managed_cli_with_paths(
    paths: &ManagedCliPaths,
    tool: ManagedCliTool,
) -> Result<ManagedCliToolStatus> {
    let _install_guard = acquire_managed_cli_install_lock(&paths.home)?;
    let before = probe_tool(paths, tool);
    let binary = tool.binary_name();
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "remove_begin",
        serde_json::json!({
            "tool": binary,
            "available": before.available,
            "source": source_word(before.source),
        }),
    );

    if !before.available {
        let status = tool_status(
            tool,
            before.clone(),
            before,
            "not installed",
            format!("{} is not installed on this machine.", tool.display_name()),
        );
        append_trace_event(
            &paths.home,
            "server",
            "managed_cli",
            "remove_end",
            serde_json::json!({ "tool": binary, "action": "not installed" }),
        );
        return Ok(status);
    }

    // ⛔ THE GUARD, before any deletion. Where does the binary a launch would
    // run actually live? Under a user-local bin dir the removal may proceed;
    // anywhere else it is a system install yggterm did not put there, and the
    // refusal names the path so a hand removal knows exactly which file.
    let managed_hit = paths.bin_dir.join(binary);
    let user_local_hit = user_local_bin_dir().map(|dir| dir.join(binary));
    let resolved = resolve_binary_for_launch_parity_with(Some(&paths.bin_dir), binary);
    let resolves_user_local = resolved.as_deref() == Some(managed_hit.as_path())
        || user_local_hit
            .as_deref()
            .is_some_and(|candidate| resolved.as_deref() == Some(candidate));
    if !resolves_user_local {
        let refusal = format!(
            "{} resolves to {} which is a system install — yggterm only removes binaries \
             under its own managed dir or your user-local bin dir; remove it by hand if you \
             want it gone",
            tool.display_name(),
            resolved
                .as_deref()
                .map(Path::display)
                .map(|path| path.to_string())
                .unwrap_or_else(|| "a system path".to_string())
        );
        append_trace_event(
            &paths.home,
            "server",
            "managed_cli",
            "remove_refused_system_path",
            serde_json::json!({
                "tool": binary,
                "resolved": resolved.as_deref().map(Path::display).map(|path| path.to_string()),
            }),
        );
        anyhow::bail!("{refusal}");
    }

    // ⛔ A CLI a process is RUNNING cannot be removed cleanly: the running exe
    // survives (it is already mapped) but every helper it spawns from its
    // generation tree dies with "No such file or directory" — the exact
    // failure mode generation pruning learned to avoid. Refuse BY PATH so the
    // user knows what to close first.
    {
        let live_executables = running_process_executable_paths();
        let slug_marker = format!("{}.gen", tool.descriptor().slug);
        let running_from: Option<PathBuf> = fs::read_dir(paths.cli_root())
            .into_iter()
            .flatten()
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(&slug_marker))
                    && generation_is_executed_by_running_process(&entry.path(), &live_executables)
            })
            .map(|entry| entry.path())
            .or_else(|| {
                live_executables
                    .iter()
                    .find(|exe| **exe == paths.bin_dir.join(binary))
                    .cloned()
            });
        if let Some(tree) = running_from {
            anyhow::bail!(
                "{} is running from its managed install ({}) right now — close its sessions \
                 first, then remove it",
                tool.display_name(),
                tree.display()
            );
        }
    }

    let detail = match tool.descriptor().install {
        CliInstall::Npm(_) => remove_managed_npm_install(paths, tool)?,
        CliInstall::Uv(package) => {
            let uv = uv_binary()
                .context("uv is required to remove this CLI and is not on the login PATH")?;
            let mut command = Command::new(uv);
            command.arg("tool").arg("uninstall").arg(&package);
            apply_provision_env(&mut command, paths);
            run_provision_command(command, &format!("uv tool uninstall {package}"))?;
            format!("Uninstalled {package} with `uv tool uninstall`.")
        }
        CliInstall::VendorScript(_) | CliInstall::Manual => {
            remove_user_local_binary(paths, tool)?
        }
    };

    let mut after = probe_tool(paths, tool);
    let mut detail = detail;
    // A legacy npm install could leave a REAL file where the managed layout
    // publishes a symlink (the shared-prefix era). The method-specific removal
    // above does not cover it, so one user-local fallback runs before giving
    // up — guarded by the same user-local boundary as everything else here.
    if after.available {
        if let Ok(extra) = remove_user_local_binary(paths, tool) {
            detail = format!("{detail} {extra}");
            after = probe_tool(paths, tool);
        }
    }
    if after.available {
        append_trace_event(
            &paths.home,
            "server",
            "managed_cli",
            "remove_still_present",
            serde_json::json!({
                "tool": binary,
                "source": source_word(after.source),
            }),
        );
        anyhow::bail!(
            "{} still resolves after removal ({}); refusing to report it gone",
            tool.display_name(),
            after
                .source
                .map(|source| source_word(Some(source)))
                .unwrap_or("unknown source")
        );
    }
    let status = tool_status(tool, before, after, "removed", detail);
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "remove_end",
        serde_json::json!({
            "tool": binary,
            "action": "removed",
        }),
    );
    Ok(status)
}

/// Delete the npm-managed tree for `tool`: the published symlink in the
/// managed bin dir plus every generation directory. Returns the detail line.
fn remove_managed_npm_install(paths: &ManagedCliPaths, tool: ManagedCliTool) -> Result<String> {
    let binary = tool.binary_name();
    let link = paths.bin_dir.join(binary);
    match fs::symlink_metadata(&link) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || metadata.is_file() {
                fs::remove_file(&link)
                    .with_context(|| format!("removing published link {}", link.display()))?;
            } else {
                anyhow::bail!(
                    "{} is a directory where the managed install publishes a binary; refusing to delete it",
                    link.display()
                );
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(anyhow!(error))
                .with_context(|| format!("statting published link {}", link.display()));
        }
    }
    let marker = format!("{}.gen", tool.descriptor().slug);
    if let Ok(entries) = fs::read_dir(paths.cli_root()) {
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.starts_with(&marker) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(format!(
        "Removed the Yggterm-managed {binary} install under {}.",
        paths.cli_root().display()
    ))
}

/// Delete a vendor- or self-installed binary, but ONLY out of a user-local bin
/// dir. A hit anywhere else is a system install yggterm did not put there.
fn remove_user_local_binary(paths: &ManagedCliPaths, tool: ManagedCliTool) -> Result<String> {
    let binary = tool.binary_name();
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(paths.bin_dir.join(binary));
    if let Some(user_bin) = user_local_bin_dir() {
        candidates.push(user_bin.join(binary));
    }
    for candidate in &candidates {
        if let Ok(metadata) = fs::symlink_metadata(candidate) {
            if !(metadata.is_file() || metadata.file_type().is_symlink()) {
                anyhow::bail!(
                    "{} is a directory where a binary was expected; refusing to delete it",
                    candidate.display()
                );
            }
            fs::remove_file(candidate)
                .with_context(|| format!("removing {}", candidate.display()))?;
            return Ok(format!("Removed {}.", candidate.display()));
        }
    }
    let resolved = resolve_binary_for_launch_parity_with(Some(&paths.bin_dir), binary);
    anyhow::bail!(
        "{} resolves to {} which is a system install — yggterm only removes binaries \
         under its own managed dir or your user-local bin dir; remove it by hand if you want it gone",
        tool.display_name(),
        resolved
            .as_deref()
            .map(Path::display)
            .map(|path| path.to_string())
            .unwrap_or_else(|| "a system path".to_string())
    )
}

fn source_word(source: Option<ManagedCliBinarySource>) -> &'static str {
    match source {
        Some(ManagedCliBinarySource::Managed) => "managed",
        Some(ManagedCliBinarySource::System) => "system",
        None => "unavailable",
    }
}

pub(crate) fn refresh_local_managed_cli(
    mode: ManagedCliRefreshMode,
) -> Result<ManagedCliRefreshReport> {
    let background = mode.is_background();
    let paths = ManagedCliPaths::resolve()?;
    let now_ms = current_time_ms();
    let ttl_ms = managed_cli_refresh_ttl_ms();
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "refresh_begin",
        serde_json::json!({
            "mode": mode.as_str(),
            "background": background,
            "ttl_ms": ttl_ms,
        }),
    );
    let perf = PerfSpan::start(&paths.home, "cli", "refresh_managed_codex");
    // DERIVED from the CLI registry, not hand-listed: the three-name array this
    // replaced is the shape where a newly registered CLI is launchable but
    // never provisioned or version-reported, which surfaces to the user as a
    // session that dies at the PTY with no telemetry saying why.
    let tools = managed_cli_tools_for_refresh();
    let before = probe_tools(&paths, &tools);
    record_managed_cli_probe_span(
        &paths.home,
        "refresh_managed_codex_probe",
        &before,
        "before",
    );

    let refresh_state = load_managed_cli_refresh_state(&paths.home);
    // ⛔ Was `npm_binary().is_some()` — one global answer for a question that is
    // now per-tool. It gated the WHOLE refresh, so a machine without npm skipped
    // the uv and vendor CLIs it was perfectly able to install. True when ANY
    // registered CLI has a runnable step; the per-tool truth is re-asked below.
    let provisioner_available = tools
        .iter()
        .copied()
        .any(|tool| provision_step_is_runnable(&paths, tool));
    let background_install_enabled = managed_cli_background_install_enabled();
    let mut install_error = None::<String>;
    let mut install_attempted = false;
    let mut install_deferred = false;
    let mut background_install_deferred = false;
    let mut skipped_recently = false;
    let mut ttl_remaining_ms = None::<u64>;
    if mode.respects_ttl() && provisioner_available {
        ttl_remaining_ms =
            managed_cli_refresh_skip_remaining_ms(&before, &refresh_state, now_ms, ttl_ms);
        skipped_recently = ttl_remaining_ms.is_some();
        if let Some(remaining_ms) = ttl_remaining_ms {
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "refresh_skip_recent",
                serde_json::json!({
                    "ttl_ms": ttl_ms,
                    "ttl_remaining_ms": remaining_ms,
                    "last_successful_refresh_ms": refresh_state.last_successful_refresh_ms,
                }),
            );
        }
        install_deferred =
            !skipped_recently && managed_cli_should_defer_initial_install(mode, &before);
        if install_deferred {
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "refresh_defer_initial_install",
                serde_json::json!({
                    "mode": mode.as_str(),
                    "background": background,
                    "reason": "missing_managed_install",
                }),
            );
        }
        if !skipped_recently
            && !install_deferred
            && mode.defers_installs()
            && !background_install_enabled
        {
            install_deferred = true;
            background_install_deferred = true;
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "refresh_defer_background_install",
                serde_json::json!({
                    "mode": mode.as_str(),
                    "background": background,
                    "reason": "background_install_opt_in_required",
                    "env": MANAGED_CLI_BACKGROUND_INSTALL_ENV,
                }),
            );
        }
    }
    if managed_cli_refresh_should_attempt_install(
        mode,
        provisioner_available,
        skipped_recently,
        install_deferred,
        background_install_enabled,
    ) {
        install_attempted = true;
        let install_perf = PerfSpan::start(&paths.home, "cli", "refresh_managed_codex_install");
        let install_all_perf = PerfSpan::start(&paths.home, "cli", "refresh_managed_all_install");
        // ⭐ EVERY tool goes in. `install_latest` partitions by method — only
        // the npm ones share a batch — so a uv or vendor CLI can no longer poison
        // codex and claude's refresh, and no longer has to be filtered out to
        // protect them. The filter this replaces is exactly what made "yggterm
        // updates all CLIs" false for three of the nine.
        let installable = tools
            .iter()
            .copied()
            .filter(|tool| provision_step_is_runnable(&paths, *tool))
            .collect::<Vec<_>>();
        for tool in &installable {
            let tool_perf = PerfSpan::start(&paths.home, "cli", &format!("refresh_managed_{}_install", tool.binary_name()));
            tool_perf.finish(serde_json::json!({
                "background": background,
                "tool": tool.binary_name(),
            }));
        }
        if let Err(error) = install_latest(&paths, &installable, background) {
            install_error = Some(error.to_string());
        }
        let install_payload = serde_json::json!({
            "background": background,
            "success": install_error.is_none(),
            "tool_count": tools.len(),
        });
        install_perf.finish(install_payload.clone());
        install_all_perf.finish(install_payload);
    }
    let after = if skipped_recently || install_deferred {
        before.clone()
    } else {
        probe_tools(&paths, &tools)
    };
    record_managed_cli_probe_span(
        &paths.home,
        "refresh_managed_codex_post_probe",
        &after,
        "after",
    );

    if provisioner_available && install_error.is_none() && !skipped_recently && !install_deferred {
        if let Err(error) = persist_managed_cli_refresh_state(&paths.home, &after, now_ms) {
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "refresh_state_write_error",
                serde_json::json!({ "error": error.to_string() }),
            );
        }
        // A refresh is only as true as the binary a session will actually run.
        // Checked HERE, after a refresh we believe succeeded, because that is
        // precisely when the silent form of this failure looks like success.
        report_managed_cli_effective_version_drift(&paths.home, &after);
    }

    let statuses = before
        .into_iter()
        .zip(after)
        .map(|((tool, before_probe), (_, after_probe))| {
            if let Some(remaining_ms) = ttl_remaining_ms {
                tool_status(
                    tool,
                    before_probe.clone(),
                    after_probe,
                    "skipped_recent",
                    managed_cli_refresh_skip_detail(tool, remaining_ms, ttl_ms),
                )
            } else if install_deferred {
                if background_install_deferred {
                    let detail = managed_cli_deferred_background_install_detail(tool, &after_probe);
                    tool_status(
                        tool,
                        before_probe,
                        after_probe,
                        "deferred_background_install",
                        detail,
                    )
                } else {
                    let detail = managed_cli_deferred_install_detail(tool, &after_probe);
                    tool_status(tool, before_probe, after_probe, "deferred_install", detail)
                }
            } else if let Some(error) = install_error.as_ref() {
                tool_status(
                    tool,
                    before_probe,
                    after_probe,
                    "failed",
                    format!("Managed refresh failed: {error}"),
                )
            } else if !provision_step_is_runnable(&paths, tool) {
                let source = tool.package_name();
                let action = if after_probe.available { "system_fallback" } else { "unavailable" };
                let detail = if after_probe.available {
                    format!(
                        "Yggterm cannot provision {} on this machine (needs {source}), so it kept using the existing binary from PATH.",
                        tool.display_name()
                    )
                } else {
                    format!(
                        "Yggterm cannot provision {} on this machine (needs {source}) and it is not currently installed.",
                        tool.display_name()
                    )
                };
                tool_status(tool, before_probe, after_probe, action, detail)
            } else if after_probe.source == Some(ManagedCliBinarySource::Managed)
                && before_probe.source != Some(ManagedCliBinarySource::Managed)
            {
                tool_status(
                    tool,
                    before_probe,
                    after_probe,
                    "adopted_managed",
                    format!(
                        "{} is now running from Yggterm's managed toolchain.",
                        tool.display_name()
                    ),
                )
            } else if before_probe.version != after_probe.version {
                tool_status(
                    tool,
                    before_probe,
                    after_probe,
                    "updated",
                    format!("Updated {} to the latest available version.", tool.display_name()),
                )
            } else {
                tool_status(
                    tool,
                    before_probe,
                    after_probe,
                    "checked",
                    format!("{} was already current.", tool.display_name()),
                )
            }
        })
        .collect::<Vec<_>>();

    for status in &statuses {
        let tool_perf = PerfSpan::start(&paths.home, "cli", &format!("refresh_managed_{}", status.binary_name));
        tool_perf.finish(serde_json::json!({
            "action": status.action.clone(),
            "available": status.available,
            "version_before": status.version_before.clone(),
            "version_after": status.version_after.clone(),
        }));
    }


    perf.finish(serde_json::json!({
        "background": background,
        "ttl_ms": ttl_ms,
        "provisioner_available": provisioner_available,
        "install_attempted": install_attempted,
        "install_deferred": install_deferred,
        "background_install_deferred": background_install_deferred,
        "background_install_enabled": background_install_enabled,
        "skipped_recently": skipped_recently,
        "ttl_remaining_ms": ttl_remaining_ms,
        "statuses": statuses.iter().map(|status| serde_json::json!({
            "tool": status.binary_name.clone(),
            "changed": status.changed,
            "available": status.available,
            "action": status.action.clone(),
        })).collect::<Vec<_>>(),
    }));

    let report = ManagedCliRefreshReport {
        scope: "local".to_string(),
        background,
        statuses,
        skipped_recently,
        ttl_remaining_ms,
        install_attempted,
        install_deferred,
    };
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "refresh_end",
        serde_json::json!({
            "background": background,
            "ttl_ms": ttl_ms,
            "install_attempted": install_attempted,
            "install_deferred": install_deferred,
            "background_install_deferred": background_install_deferred,
            "background_install_enabled": background_install_enabled,
            "skipped_recently": skipped_recently,
            "ttl_remaining_ms": ttl_remaining_ms,
            "statuses": report.statuses.iter().map(|status| serde_json::json!({
                "tool": status.binary_name.clone(),
                "action": status.action.clone(),
                "available": status.available,
                "changed": status.changed,
            })).collect::<Vec<_>>(),
        }),
    );
    Ok(report)
}

pub(crate) fn summarize_managed_cli_report(
    scope: &str,
    report: &ManagedCliRefreshReport,
) -> String {
    if report.skipped_recently {
        return format!("{scope}: managed Codex refresh still fresh");
    }

    if report.install_deferred {
        if report
            .statuses
            .iter()
            .any(|status| status.action == "deferred_background_install")
        {
            return format!("{scope}: deferred background managed Codex install");
        }
        return format!("{scope}: deferred initial managed Codex install until first use");
    }

    let changed = report
        .statuses
        .iter()
        .filter(|status| status.changed)
        .map(|status| status.binary_name.clone())
        .collect::<Vec<_>>();
    if !changed.is_empty() {
        return format!("{scope}: updated {}", changed.join(" and "));
    }

    let issues = report
        .statuses
        .iter()
        .filter(|status| status.action == "error" || status.action == "unavailable")
        .map(|status| status.detail.clone())
        .collect::<Vec<_>>();
    if !issues.is_empty() {
        return format!("{scope}: {}", issues.join(" "));
    }

    let fallback = report
        .statuses
        .iter()
        .any(|status| status.action == "system_fallback");
    if fallback {
        return format!(
            "{scope}: using existing PATH Codex binaries until explicit managed refresh"
        );
    }

    format!("{scope}: Codex tools already current")
}
