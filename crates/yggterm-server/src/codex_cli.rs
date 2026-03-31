use crate::{SessionKind, shell_single_quote};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use yggterm_core::{PerfSpan, UiTheme, append_trace_event, resolve_yggterm_home};

const MANAGED_NPM_DIRNAME: &str = "npm";
const MANAGED_NPM_CACHE_DIRNAME: &str = "npm-cache";
const EXPORTED_TERM_PROGRAM: &str = "vscode";
const YGGTERM_TERM_PROGRAM: &str = "yggterm";
const YGGTERM_TERM_PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");

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

pub(crate) fn terminal_identity_env_pairs() -> Vec<(&'static str, String)> {
    let appearance = ambient_terminal_appearance();
    vec![
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
    ]
}

pub(crate) fn terminal_identity_shell_exports() -> Vec<String> {
    terminal_identity_env_pairs()
        .into_iter()
        .map(|(key, value)| format!("export {key}={}", shell_single_quote(&value)))
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
        env::set_var("TERM", "xterm-256color");
        env::set_var("COLORTERM", "truecolor");
        env::set_var("TERM_PROGRAM", EXPORTED_TERM_PROGRAM);
        env::set_var("TERM_PROGRAM_VERSION", YGGTERM_TERM_PROGRAM_VERSION);
        env::set_var("YGGTERM_TERM_PROGRAM", YGGTERM_TERM_PROGRAM);
        env::set_var("YGGTERM_APPEARANCE", appearance);
        env::set_var("COLORFGBG", colorfgbg_for_appearance(appearance));
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
    let status = command
        .status()
        .context("running npm install for managed Codex tools")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("managed Codex npm install exited with status {status}");
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
    let mut parts = Vec::new();
    if let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) {
        parts.push(format!("cd {}", shell_single_quote(cwd)));
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
            format!(
                "{prefix}{} resume {}",
                tool.binary_name(),
                shell_single_quote(session_id)
            )
        }
    };
    parts.push(invocation);
    Ok(parts.join(" && "))
}

pub(crate) fn ensure_local_managed_cli(tool: ManagedCliTool) -> Result<ManagedCliToolStatus> {
    let paths = ManagedCliPaths::resolve()?;
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
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "refresh_begin",
        serde_json::json!({ "background": background }),
    );
    let perf = PerfSpan::start(&paths.home, "cli", "refresh_managed_codex");
    let tools = [ManagedCliTool::Codex, ManagedCliTool::CodexLiteLlm];
    let before = tools
        .into_iter()
        .map(|tool| (tool, probe_tool(&paths, tool)))
        .collect::<Vec<_>>();

    let npm_available = npm_binary().is_some();
    let mut install_error = None::<String>;
    if npm_available {
        if let Err(error) = install_latest(&paths, &tools, background) {
            install_error = Some(error.to_string());
        }
    }

    let statuses = before
        .into_iter()
        .map(|(tool, before_probe)| {
            let after_probe = probe_tool(&paths, tool);
            if let Some(error) = install_error.as_ref() {
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
        "npm_available": npm_available,
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
    };
    append_trace_event(
        &paths.home,
        "server",
        "managed_cli",
        "refresh_end",
        serde_json::json!({
            "background": background,
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
