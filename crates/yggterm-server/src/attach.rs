use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use time::OffsetDateTime;
use yggterm_core::SessionStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachMetadata {
    pub uuid: String,
    pub backend: String,
    pub shell: String,
    pub hostname: String,
    pub cwd: String,
    pub attach_count: u64,
    pub started_at: String,
    pub last_attached_at: String,
}

pub fn run_attach(uuid: &str, cwd: Option<&str>) -> Result<()> {
    let store = SessionStore::open_or_init()?;
    let session_dir = store.home_dir().join("runtime").join("attach").join(uuid);
    fs::create_dir_all(&session_dir)
        .with_context(|| format!("creating attach dir {}", session_dir.display()))?;

    if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
        std::env::set_current_dir(cwd).with_context(|| format!("setting attach cwd to {cwd}"))?;
    }

    let metadata_path = session_dir.join("session.json");
    let mut metadata = load_metadata(&metadata_path).unwrap_or_else(|| AttachMetadata {
        uuid: uuid.to_string(),
        backend: if tmux_available() {
            "tmux".to_string()
        } else {
            "shell".to_string()
        },
        shell: shell_program(),
        hostname: host_label(),
        cwd: std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .display()
            .to_string(),
        attach_count: 0,
        started_at: timestamp_now(),
        last_attached_at: timestamp_now(),
    });
    metadata.attach_count += 1;
    metadata.last_attached_at = timestamp_now();
    metadata.shell = shell_program();
    metadata.hostname = host_label();
    metadata.cwd = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .display()
        .to_string();
    metadata.backend = if tmux_available() {
        "tmux".to_string()
    } else {
        "shell".to_string()
    };
    fs::write(&metadata_path, serde_json::to_string_pretty(&metadata)?)
        .with_context(|| format!("writing attach metadata {}", metadata_path.display()))?;

    if tmux_available() {
        return exec_tmux(uuid, cwd);
    }

    exec_shell(cwd)
}

fn load_metadata(path: &PathBuf) -> Option<AttachMetadata> {
    let json = fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

fn timestamp_now() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

fn host_label() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|value| value.trim().to_string())
        })
        .unwrap_or_else(|| "unknown-host".to_string())
}

fn shell_program() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string())
}

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn exec_tmux(uuid: &str, cwd: Option<&str>) -> Result<()> {
    let session_name = format!("yggterm-{}", short_session_name(uuid));
    let attach_command = attach_shell_command(cwd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("tmux");
        command
            .arg("new-session")
            .arg("-A")
            .arg("-s")
            .arg(session_name);
        if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
            command.arg("-c").arg(cwd);
        }
        if let Some(attach_command) = attach_command.as_deref() {
            command.arg(attach_command);
        }
        let error = command.exec();
        Err(anyhow::anyhow!("failed to exec tmux: {error}"))
    }

    #[cfg(not(unix))]
    {
        let mut command = Command::new("tmux");
        command
            .arg("new-session")
            .arg("-A")
            .arg("-s")
            .arg(session_name);
        if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
            command.arg("-c").arg(cwd);
        }
        if let Some(attach_command) = attach_command.as_deref() {
            command.arg(attach_command);
        }
        let status = command.status().context("running tmux attach")?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("tmux exited with status {status}"))
        }
    }
}

fn exec_shell(cwd: Option<&str>) -> Result<()> {
    let shell = shell_program();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(&shell);
        command.arg("-i");
        if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
            if uses_bash_shell(&shell) {
                command.env("YGGTERM_START_CWD", cwd);
                command.env(
                    "PROMPT_COMMAND",
                    r#"cd -- "$YGGTERM_START_CWD"; unset PROMPT_COMMAND"#,
                );
            }
        }
        let error = command.exec();
        Err(anyhow::anyhow!("failed to exec shell {shell}: {error}"))
    }

    #[cfg(not(unix))]
    {
        let mut command = Command::new(&shell);
        command.arg("-i");
        if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
            if uses_bash_shell(&shell) {
                command.env("YGGTERM_START_CWD", cwd);
                command.env(
                    "PROMPT_COMMAND",
                    r#"cd -- "$YGGTERM_START_CWD"; unset PROMPT_COMMAND"#,
                );
            }
        }
        let status = command
            .status()
            .with_context(|| format!("running shell {shell}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("shell exited with status {status}"))
        }
    }
}

fn uses_bash_shell(shell: &str) -> bool {
    PathBuf::from(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("bash"))
        .unwrap_or(false)
}

fn attach_shell_command(cwd: Option<&str>) -> Option<String> {
    let cwd = cwd.map(str::trim).filter(|value| !value.is_empty())?;
    let shell = shell_program();
    if uses_bash_shell(&shell) {
        return Some(format!(
            "env YGGTERM_START_CWD={} PROMPT_COMMAND={} {} -i",
            shell_single_quote(cwd),
            shell_single_quote(r#"cd -- "$YGGTERM_START_CWD"; unset PROMPT_COMMAND"#),
            shell_single_quote(&shell)
        ));
    }
    Some(shell_single_quote(&shell))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn short_session_name(uuid: &str) -> String {
    uuid.chars().take(16).collect()
}
