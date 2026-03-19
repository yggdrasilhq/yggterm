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

pub fn run_attach(uuid: &str) -> Result<()> {
    let store = SessionStore::open_or_init()?;
    let session_dir = store.home_dir().join("runtime").join("attach").join(uuid);
    fs::create_dir_all(&session_dir)
        .with_context(|| format!("creating attach dir {}", session_dir.display()))?;

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
        return exec_tmux(uuid);
    }

    exec_shell()
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

fn exec_tmux(uuid: &str) -> Result<()> {
    let session_name = format!("yggterm-{}", short_session_name(uuid));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new("tmux")
            .arg("new-session")
            .arg("-A")
            .arg("-s")
            .arg(session_name)
            .exec();
        Err(anyhow::anyhow!("failed to exec tmux: {error}"))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new("tmux")
            .arg("new-session")
            .arg("-A")
            .arg("-s")
            .arg(session_name)
            .status()
            .context("running tmux attach")?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("tmux exited with status {status}"))
        }
    }
}

fn exec_shell() -> Result<()> {
    let shell = shell_program();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new(&shell).exec();
        Err(anyhow::anyhow!("failed to exec shell {shell}: {error}"))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new(&shell)
            .status()
            .with_context(|| format!("running shell {shell}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("shell exited with status {status}"))
        }
    }
}

fn short_session_name(uuid: &str) -> String {
    uuid.chars().take(16).collect()
}
