use crate::{SessionKind, shell_single_quote};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use yggterm_core::{ENV_YGGTERM_HOME, PerfSpan, append_trace_event, resolve_yggterm_home};
use yggui_contract::UiTheme;

const MANAGED_NPM_DIRNAME: &str = "npm";
const MANAGED_NPM_CACHE_DIRNAME: &str = "npm-cache";
const EXPORTED_TERM_PROGRAM: &str = "vscode";
const YGGTERM_TERM_PROGRAM: &str = "yggterm";
const YGGTERM_TERM_PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");
const TERMINAL_IDENTITY_ENV_REMOVALS: &[&str] = &["NO_COLOR"];
const MANAGED_CLI_REFRESH_STATE_FILENAME: &str = "managed-cli-refresh-state.json";
const MANAGED_CLI_REFRESH_TTL_ENV: &str = "YGGTERM_MANAGED_CLI_REFRESH_TTL_MS";
pub const DEFAULT_MANAGED_CLI_REFRESH_TTL_MS: u64 = 6 * 60 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedCliTool {
    Codex,
    CodexLiteLlm,
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
    pub(crate) fn from_session_kind(kind: SessionKind) -> Option<Self> {
        match kind {
            SessionKind::Codex => Some(Self::Codex),
            SessionKind::CodexLiteLlm => Some(Self::CodexLiteLlm),
            SessionKind::Shell | SessionKind::SshShell | SessionKind::Document => None,
        }
    }

    pub(crate) fn binary_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::CodexLiteLlm => "codex-litellm",
        }
    }

    pub(crate) fn package_name(self) -> &'static str {
        match self {
            Self::Codex => "@openai/codex",
            Self::CodexLiteLlm => "@avikalpa/codex-litellm",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::CodexLiteLlm => "Codex-LiteLLM",
        }
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
        let mut exports = terminal_identity_shell_exports();
        exports.extend([
            format!(
                "export NPM_CONFIG_PREFIX={}",
                shell_single_quote(&self.prefix.display().to_string())
            ),
            format!(
                "export npm_config_prefix={}",
                shell_single_quote(&self.prefix.display().to_string())
            ),
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

fn ambient_terminal_appearance() -> String {
    env::var("YGGTERM_APPEARANCE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "light" | "dark"))
        .unwrap_or_else(|| "light".to_string())
}

fn colorfgbg_for_appearance(appearance: &str) -> &'static str {
    match appearance {
        "dark" => "15;0",
        _ => "0;15",
    }
}

fn terminal_identity_env_pairs_with_home(
    include_yggterm_home: bool,
) -> Vec<(&'static str, String)> {
    let appearance = ambient_terminal_appearance();
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
    pairs
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

pub(crate) fn sync_terminal_identity_env(theme: UiTheme) {
    let appearance = match theme {
        UiTheme::ZedLight => "light",
        UiTheme::ZedDark => "dark",
    };
    // The daemon owns terminal launch commands and needs a process-wide identity for child PTYs
    // and remote shell command synthesis. This is updated on startup/theme changes only.
    unsafe {
        for key in terminal_identity_env_removals() {
            env::remove_var(key);
        }
        env::set_var("TERM", "xterm-256color");
        env::set_var("COLORTERM", "truecolor");
        env::set_var("TERM_PROGRAM", EXPORTED_TERM_PROGRAM);
        env::set_var("TERM_PROGRAM_VERSION", YGGTERM_TERM_PROGRAM_VERSION);
        env::set_var("YGGTERM_TERM_PROGRAM", YGGTERM_TERM_PROGRAM);
        env::set_var("YGGTERM_APPEARANCE", appearance);
        env::set_var("COLORFGBG", colorfgbg_for_appearance(appearance));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_identity_shell_exports_unset_no_color() {
        let exports = terminal_identity_shell_exports();
        assert_eq!(exports.first().map(String::as_str), Some("unset NO_COLOR"));
    }

    #[test]
    fn sync_terminal_identity_env_removes_no_color() {
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
    if let Some(system_binary) = resolve_binary_on_path(tool.binary_name()) {
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

fn install_latest(
    paths: &ManagedCliPaths,
    tools: &[ManagedCliTool],
    background: bool,
) -> Result<()> {
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
    for tool in tools {
        command.arg(format!("{}@latest", tool.package_name()));
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

pub(crate) fn managed_cli_shell_command(
    kind: SessionKind,
    cwd: Option<&str>,
    action: ManagedCliAction<'_>,
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
    parts.push(paths.shell_exports(tool));
    let invocation = match action {
        ManagedCliAction::Launch => tool.binary_name().to_string(),
        ManagedCliAction::ResumePicker { persistent } => {
            let prefix = if persistent { "exec " } else { "" };
            format!("{prefix}{} resume", tool.binary_name())
        }
        ManagedCliAction::Resume {
            session_id,
            persistent,
        } => {
            let prefix = if persistent { "exec " } else { "" };
            if matches!(kind, SessionKind::Codex) && has_cwd {
                format!(
                    "{prefix}{} resume -C \"$PWD\" {}",
                    tool.binary_name(),
                    shell_single_quote(session_id)
                )
            } else {
                format!(
                    "{prefix}{} resume {}",
                    tool.binary_name(),
                    shell_single_quote(session_id)
                )
            }
        }
    };
    parts.push(invocation);
    Ok(parts.join(" && "))
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

pub(crate) fn ensure_local_managed_cli(tool: ManagedCliTool) -> Result<ManagedCliToolStatus> {
    let paths = ManagedCliPaths::resolve()?;
    let now_ms = current_time_ms();
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "ensure_begin",
        serde_json::json!({ "tool": tool.binary_name() }),
    );
    let before = probe_tool(&paths, tool);
    if before.available {
        let detail = match before.source {
            Some(ManagedCliBinarySource::Managed) => {
                format!("{} is already managed by Yggterm.", tool.display_name())
            }
            Some(ManagedCliBinarySource::System) => format!(
                "{} is currently coming from the system PATH. Yggterm will keep using it until the managed copy is refreshed in the background.",
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
        format!(
            "Installed a Yggterm-managed {} toolchain under {}.",
            tool.display_name(),
            paths.prefix.display()
        ),
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
    let tools = [ManagedCliTool::Codex, ManagedCliTool::CodexLiteLlm];
    let before = probe_tools(&paths, &tools);
    record_managed_cli_probe_span(
        &paths.home,
        "refresh_managed_codex_probe",
        &before,
        "before",
    );

    let refresh_state = load_managed_cli_refresh_state(&paths.home);
    let npm_available = npm_binary().is_some();
    let mut install_error = None::<String>;
    let mut install_attempted = false;
    let mut install_deferred = false;
    let mut skipped_recently = false;
    let mut ttl_remaining_ms = None::<u64>;
    if background && npm_available {
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
    }
    if npm_available && !skipped_recently && !install_deferred {
        install_attempted = true;
        let install_perf = PerfSpan::start(&paths.home, "cli", "refresh_managed_codex_install");
        if let Err(error) = install_latest(&paths, &tools, background) {
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

    if npm_available && install_error.is_none() && !skipped_recently && !install_deferred {
        if let Err(error) = persist_managed_cli_refresh_state(&paths.home, &after, now_ms) {
            append_trace_event(
                &paths.home,
                "server",
                "managed_cli",
                "refresh_state_write_error",
                serde_json::json!({ "error": error.to_string() }),
            );
        }
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
                let detail = managed_cli_deferred_install_detail(tool, &after_probe);
                tool_status(tool, before_probe, after_probe, "deferred_install", detail)
            } else if let Some(error) = install_error.as_ref() {
                tool_status(
                    tool,
                    before_probe,
                    after_probe,
                    "error",
                    format!("{} refresh failed: {error}", tool.display_name()),
                )
            } else if !npm_available {
                let action = if after_probe.available { "system_fallback" } else { "unavailable" };
                let detail = if after_probe.available {
                    format!(
                        "npm is unavailable on this machine, so Yggterm kept using the existing {} binary from PATH.",
                        tool.display_name()
                    )
                } else {
                    format!(
                        "npm is unavailable on this machine and {} is not currently installed.",
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
        "npm_available": npm_available,
        "install_attempted": install_attempted,
        "install_deferred": install_deferred,
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
        return format!("{scope}: using existing PATH Codex binaries until npm is available");
    }

    format!("{scope}: Codex tools already current")
}
