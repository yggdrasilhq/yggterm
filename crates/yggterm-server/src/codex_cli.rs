use crate::{SessionKind, shell_single_quote};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use yggterm_core::agent_cli::{AgentCliDescriptor, CliInstall, CliUpdate, agent_cli_descriptor};
use yggterm_core::{
    AgentLaunchOptions, ENV_YGGTERM_HOME, PerfSpan, SessionStore, append_trace_event,
    resolve_yggterm_home,
};
use yggui_contract::UiTheme;

const MANAGED_NPM_DIRNAME: &str = "npm";
const MANAGED_NPM_CACHE_DIRNAME: &str = "npm-cache";
const EXPORTED_TERM_PROGRAM: &str = "vscode";
const YGGTERM_TERM_PROGRAM: &str = "yggterm";
const YGGTERM_TERM_PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");
const ENV_YGGTERM_TERMINAL_APPEARANCE: &str = "YGGTERM_TERMINAL_APPEARANCE";
pub(crate) const ENV_YGGTERM_CC_EXTRA_ARGS: &str = "YGGTERM_CC_EXTRA_ARGS";
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
pub const DEFAULT_MANAGED_CLI_REFRESH_TTL_MS: u64 = 6 * 60 * 60_000;

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
            // agy; it updated it from 1.0.5 to 1.1.11 on jojo the same day.
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

    fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.prefix)
            .with_context(|| format!("creating managed npm prefix {}", self.prefix.display()))?;
        fs::create_dir_all(&self.bin_dir)
            .with_context(|| format!("creating managed npm bin {}", self.bin_dir.display()))?;
        fs::create_dir_all(&self.cache_dir)
            .with_context(|| format!("creating managed npm cache {}", self.cache_dir.display()))?;
        Ok(())
    }

    fn env_path(&self) -> OsString {
        let mut parts = vec![self.bin_dir.clone()];
        parts.extend(env::split_paths(
            &env::var_os("PATH").unwrap_or_else(|| OsString::from("")),
        ));
        env::join_paths(parts).unwrap_or_else(|_| OsString::from(""))
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
            format!(
                "export PATH={}:\"$PATH\"",
                shell_single_quote(&self.bin_dir.display().to_string())
            ),
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

pub fn managed_cli_refresh_ttl_ms() -> u64 {
    env::var(MANAGED_CLI_REFRESH_TTL_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MANAGED_CLI_REFRESH_TTL_MS)
}

fn current_time_ms() -> u64 {
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
    let script =
        format!("command -v {binary_name} || exit 1; {binary_name} --version 2>/dev/null | head -1");
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
    let version = lines
        .next()
        .and_then(extract_semver_like_version)
        .map(|version| version.to_string());
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
/// login PATH at all on jojo or oc, so every session there ran a SEPARATE
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

fn managed_cli_should_defer_initial_install(
    background: bool,
    probes: &[(ManagedCliTool, ToolProbe)],
) -> bool {
    background && !managed_cli_has_existing_managed_install(probes)
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
    background: bool,
    provisioner_available: bool,
    skipped_recently: bool,
    install_deferred: bool,
    background_install_enabled: bool,
) -> bool {
    provisioner_available
        && !skipped_recently
        && !install_deferred
        && (!background || background_install_enabled)
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
    let this_file = crate_src.join("codex_cli.rs");
    let text = std::fs::read_to_string(&this_file).expect("read codex_cli.rs");
    assert!(
        text.contains("static ENV_TEST_LOCK: std::sync::Mutex<()>"),
        "the scan did not find env_test_guard's own lock, so it is not reading \
         the source it claims to police",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::env_test_guard;

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
        assert!(managed_cli_should_defer_initial_install(true, &system_only));
        assert!(!managed_cli_should_defer_initial_install(
            false,
            &system_only
        ));
        assert!(!managed_cli_should_defer_initial_install(
            true,
            &managed_present
        ));
    }

    #[test]
    fn background_managed_cli_refresh_requires_install_opt_in() {
        assert!(!managed_cli_refresh_should_attempt_install(
            true, true, false, false, false
        ));
        assert!(managed_cli_refresh_should_attempt_install(
            true, true, false, false, true
        ));
        assert!(managed_cli_refresh_should_attempt_install(
            false, true, false, false, false
        ));
        assert!(!managed_cli_refresh_should_attempt_install(
            true, true, true, false, true
        ));
        assert!(!managed_cli_refresh_should_attempt_install(
            true, true, false, true, true
        ));
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
    /// the real drift (2.1.223 managed vs 2.1.220 effective, measured on jojo
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
        assert!(install_npm_batch(&paths, &[], true).is_ok());
    }

    /// The status line must name the method that ACTUALLY ran.
    ///
    /// ⛔ MEASURED WRONG, live on jojo 2026-08-08, on all three new lanes at
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
    /// anything). Owner-reported 2026-08-08.
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
        assert!(exports.contains("export PATH='/tmp/yggterm-home/npm/bin':\"$PATH\""));
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

fn run_version_command(binary_path: &Path) -> Option<String> {
    let output = Command::new(binary_path).arg("--version").output().ok()?;
    let combined = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    extract_version_token(&combined)
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
    if let Some(system_binary) = resolve_binary_for_launch_parity(tool.binary_name()) {
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

fn npm_binary() -> Option<PathBuf> {
    resolve_binary_on_path("npm")
}

/// `uv` is installed into `~/.local/bin`, which the daemon's own `PATH`
/// routinely omits — so this must resolve with LAUNCH PARITY, exactly like the
/// CLIs themselves. Resolving it off the daemon `PATH` alone reported "uv is
/// unavailable" on jojo, where `~/.local/bin/uv` has been present since May.
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
/// there. That ordering is the owner's ruling read literally: he asked for
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
/// tested: `agy` happens to be installed on jojo, so a test that asked for the
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
/// ⛔ MEASURED WRONG, live on jojo 2026-08-08: every install reported *"a
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
fn install_via_uv(package: &str) -> Result<()> {
    let uv = uv_binary().context(
        "uv is required to install this CLI and is not on the login PATH — \
         install uv (https://astral.sh/uv) and the next refresh will pick it up",
    )?;
    let mut command = Command::new(uv);
    command
        .arg("tool")
        .arg("install")
        .arg("--upgrade")
        .arg(package)
        .env("PATH", provision_env_path());
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
        .arg(url)
        .env("PATH", provision_env_path());
    run_provision_command(fetch, &format!("fetching vendor installer {url}"))?;

    let mut run = Command::new("bash");
    run.arg(&script).env("PATH", provision_env_path());
    run_provision_command(run, &format!("running vendor installer {url}"))
}

/// Run a CLI's own updater, e.g. `agy update`.
fn update_via_self_command(tool: ManagedCliTool, argv: &[&str]) -> Result<()> {
    let binary = resolve_binary_for_launch_parity(tool.binary_name()).with_context(|| {
        format!(
            "{} advertises its own updater but is not on the login PATH",
            tool.display_name()
        )
    })?;
    let mut command = Command::new(binary);
    command.args(argv).env("PATH", provision_env_path());
    run_provision_command(
        command,
        &format!("{} {}", tool.binary_name(), argv.join(" ")),
    )
}

/// The `PATH` every provisioning subprocess runs with: the daemon's own, plus
/// the login-shell dirs. An installer that shells out to `curl`, `tar` or
/// `python` must see what a human's shell sees, or it fails on the daemon's
/// stripped `PATH` in ways no user can reproduce.
fn provision_env_path() -> OsString {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(current) = env::var_os("PATH") {
        parts.extend(env::split_paths(&current));
    }
    for dir in login_shell_path_dirs() {
        if !parts.contains(dir) {
            parts.push(dir.clone());
        }
    }
    env::join_paths(parts).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
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

fn install_latest(
    paths: &ManagedCliPaths,
    tools: &[ManagedCliTool],
    background: bool,
) -> Result<()> {
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
            ProvisionStep::Uv(package) => install_via_uv(package),
            ProvisionStep::VendorScript(url) => install_via_vendor_script(paths, url),
            ProvisionStep::SelfUpdate(argv) => update_via_self_command(tool, argv),
        };
        if let Err(error) = outcome {
            failures.push(format!("{}: {error}", tool.display_name()));
        }
    }

    if let Err(error) = install_npm_batch(paths, &npm_tools, background) {
        failures.push(error.to_string());
    }

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", failures.join("; "))
    }
}

fn install_npm_batch(
    paths: &ManagedCliPaths,
    npm_tools: &[ManagedCliTool],
    background: bool,
) -> Result<()> {
    if npm_tools.is_empty() {
        return Ok(());
    }
    let npm = npm_binary().context("npm is required to manage Codex tools")?;
    paths.ensure_dirs()?;
    let mut command = Command::new(npm);
    command
        .env("NPM_CONFIG_PREFIX", &paths.prefix)
        .env("npm_config_prefix", &paths.prefix)
        .env("npm_config_cache", &paths.cache_dir)
        .env("npm_config_update_notifier", "false")
        .env("npm_config_audit", "false")
        .env("npm_config_fund", "false")
        .env("PATH", paths.env_path())
        .arg("install")
        .arg("-g");
    if background {
        command.arg("--silent");
    }
    for tool in npm_tools {
        let package = tool
            .npm_package()
            .expect("partitioned above: only npm-provisionable tools reach here");
        command.arg(format!("{package}@latest"));
    }
    let output = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("running npm install for managed Codex tools")?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            anyhow::bail!(
                "managed Codex npm install exited with status {}",
                output.status
            );
        }
        anyhow::bail!(
            "managed Codex npm install exited with status {}: {}",
            output.status,
            stderr
        );
    }
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
    let extra_args = composed_cli_extra_args(kind, launch)?;
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
    let invocation = match action {
        ManagedCliAction::Launch => format!("{}{}", tool.binary_name(), extra_args),
        ManagedCliAction::ResumePicker { persistent } => {
            let prefix = if persistent { "exec " } else { "" };
            let tokens = descriptor
                .map(|descriptor| descriptor.resume_picker_tokens())
                .unwrap_or_default();
            format!(
                "{prefix}{}{}{}",
                tool.binary_name(),
                extra_args,
                join_invocation_tokens(&tokens)
            )
        }
        ManagedCliAction::Resume {
            session_id,
            persistent,
        } => {
            let prefix = if persistent { "exec " } else { "" };
            let quoted = shell_single_quote(session_id);
            let tokens = descriptor
                .map(|descriptor| descriptor.resume_tokens(&quoted, has_cwd))
                .unwrap_or_else(|| vec![quoted.clone()]);
            format!(
                "{prefix}{}{}{}",
                tool.binary_name(),
                extra_args,
                join_invocation_tokens(&tokens)
            )
        }
    };
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
/// that requirement is what made the pre-flag workaround unacceptable.
pub(crate) fn composed_cli_extra_args(
    kind: SessionKind,
    launch: &AgentLaunchOptions,
) -> Result<String> {
    let configured = configured_cli_extra_arg_tokens(kind);
    if launch.is_empty() {
        // Byte-identical to the pre-flag path for every human door.
        return Ok(shell_join_tokens(&configured));
    }
    let mut tokens = launch.strip_overridden(kind, &configured);
    tokens.extend(launch.launch_tokens(kind).map_err(|message| anyhow!(message))?);
    Ok(shell_join_tokens(&tokens))
}

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
    let settings = SessionStore::open_or_init()
        .and_then(|store| store.load_settings())
        .ok();
    // ⚖ NOT descriptor-derivable, and the arms are spelled out rather than left
    // to a `_` so that stays visible: the settings store has exactly TWO
    // extra-args fields (`codex_extra_args`, `claude_code_extra_args`), and a
    // CLI with no field of its own has no configured args — not codex's.
    // Giving a new CLI codex's flags is how `--sandbox workspace-write` would
    // reach a binary that has never heard of it and refuse to start.
    let raw = match kind {
        SessionKind::Codex | SessionKind::CodexLiteLlm => {
            settings.map(|settings| settings.codex_extra_args)
        }
        SessionKind::ClaudeCode => settings.map(|settings| settings.claude_code_extra_args),
        // The 2026-08-08 intake owns no settings field yet. A DECLARED gap:
        // per-launch `--model` / `--permission-mode` still work for them
        // (those ride `AgentLaunchOptions`, not this).
        SessionKind::Pi
        | SessionKind::OpenCode
        | SessionKind::QwenCode
        | SessionKind::Kimi
        | SessionKind::Muse
        | SessionKind::Antigravity => None,
        SessionKind::Shell | SessionKind::SshShell | SessionKind::Document => None,
    }
    .unwrap_or_default();
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
         if [ \"$__yggterm_cwd_ok\" != 1 ] && [ -n \"$HOME\" ]; then cd \"$HOME\" 2>/dev/null || true; fi",
        requested = shell_single_quote(requested)
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
/// measured on jojo, 2026-06-14). Resolving via the login shell closes that gap so the
/// probe matches launch parity. One subprocess per daemon lifetime; never on the hot path.
fn login_shell_path_dirs() -> &'static [PathBuf] {
    static DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DIRS.get_or_init(|| {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let output = Command::new(&shell)
            .arg("-lc")
            .arg("printf %s \"$PATH\"")
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout);
                env::split_paths(path.trim()).collect()
            }
            _ => Vec::new(),
        }
    })
}

/// Resolve a binary the way the launched session will: daemon `PATH` first (cheap,
/// already in-process), then the cached login-shell `PATH`. Existence check only —
/// no `--version` subprocess.
fn resolve_binary_for_launch_parity(binary_name: &str) -> Option<PathBuf> {
    if let Some(path) = resolve_binary_on_path(binary_name) {
        return Some(path);
    }
    login_shell_path_dirs()
        .iter()
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
/// ⛔ The defect this closes, owner-reported 2026-08-08: a missing binary was
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

pub(crate) fn refresh_local_managed_cli(background: bool) -> Result<ManagedCliRefreshReport> {
    let paths = ManagedCliPaths::resolve()?;
    let now_ms = current_time_ms();
    let ttl_ms = managed_cli_refresh_ttl_ms();
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "refresh_begin",
        serde_json::json!({
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
    if background && provisioner_available {
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
            !skipped_recently && managed_cli_should_defer_initial_install(background, &before);
        if install_deferred {
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "refresh_defer_initial_install",
                serde_json::json!({
                    "background": background,
                    "reason": "missing_managed_install",
                }),
            );
        }
        if !skipped_recently && !install_deferred && !background_install_enabled {
            install_deferred = true;
            background_install_deferred = true;
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "refresh_defer_background_install",
                serde_json::json!({
                    "background": background,
                    "reason": "background_install_opt_in_required",
                    "env": MANAGED_CLI_BACKGROUND_INSTALL_ENV,
                }),
            );
        }
    }
    if managed_cli_refresh_should_attempt_install(
        background,
        provisioner_available,
        skipped_recently,
        install_deferred,
        background_install_enabled,
    ) {
        install_attempted = true;
        let install_perf = PerfSpan::start(&paths.home, "cli", "refresh_managed_codex_install");
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
        if let Err(error) = install_latest(&paths, &installable, background) {
            install_error = Some(error.to_string());
        }
        install_perf.finish(serde_json::json!({
            "background": background,
            "success": install_error.is_none(),
            "tool_count": tools.len(),
        }));
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
                    "error",
                    format!("{} refresh failed: {error}", tool.display_name()),
                )
            } else if !provision_step_is_runnable(&paths, tool) {
                // ⚠ Per-tool, and it NAMES the missing provisioner. The single
                // "npm is unavailable" sentence this replaces was wrong twice
                // over on a uv CLI: npm's absence is not why kimi is missing,
                // and npm's presence would not have fixed it.
                let action = if after_probe.available { "system_fallback" } else { "unavailable" };
                let source = tool.package_name();
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
