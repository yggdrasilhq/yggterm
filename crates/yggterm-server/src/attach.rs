use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use time::OffsetDateTime;
use yggterm_core::SessionStore;

const TERMINAL_ENV_REMOVALS: &[&str] = &["NO_COLOR"];

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

    let resolved_cwd = resolve_attach_cwd(cwd);
    if let Some(cwd) = resolved_cwd.as_deref() {
        std::env::set_current_dir(cwd).with_context(|| format!("setting attach cwd to {cwd}"))?;
    }

    let metadata_path = session_dir.join("session.json");
    let mut metadata = load_metadata(&metadata_path).unwrap_or_else(|| AttachMetadata {
        uuid: uuid.to_string(),
        backend: "daemon-shell".to_string(),
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
    metadata.backend = "daemon-shell".to_string();
    fs::write(&metadata_path, serde_json::to_string_pretty(&metadata)?)
        .with_context(|| format!("writing attach metadata {}", metadata_path.display()))?;

    // The host daemon owns/persists the shell PTY (it IS the multiplexer per
    // [[spec-decentralized-host-daemon]]). Bridge stdio to a daemon-owned,
    // resumable shell session — no external multiplexer (tmux/screen) anywhere.
    // Fall back to a plain login shell only if the daemon is unreachable, so a
    // bare `server attach` on a host without a working daemon still gives a
    // usable (non-persistent) shell instead of failing outright.
    match crate::run_daemon_shell_attach(uuid, resolved_cwd.as_deref()) {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!(
                "yggterm: daemon-owned shell attach unavailable ({error}); falling back to a plain shell"
            );
            exec_shell(resolved_cwd.as_deref())
        }
    }
}

fn resolve_attach_cwd(cwd: Option<&str>) -> Option<String> {
    let requested = cwd.map(str::trim).filter(|value| !value.is_empty())?;
    let requested_path = PathBuf::from(requested);
    if requested_path.is_dir() {
        return Some(requested.to_string());
    }
    if let Some(existing_parent) = requested_path.ancestors().find(|path| path.is_dir()) {
        return Some(existing_parent.display().to_string());
    }
    std::env::var("HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .filter(|value| PathBuf::from(value).is_dir())
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

fn exec_shell(cwd: Option<&str>) -> Result<()> {
    let shell = shell_program();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(&shell);
        for key in TERMINAL_ENV_REMOVALS {
            command.env_remove(key);
        }
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
        for key in TERMINAL_ENV_REMOVALS {
            command.env_remove(key);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolve_attach_cwd_reuses_existing_directory() {
        let cwd = std::env::temp_dir();
        let cwd = cwd.display().to_string();
        assert_eq!(resolve_attach_cwd(Some(&cwd)), Some(cwd));
    }

    #[test]
    fn resolve_attach_cwd_falls_back_to_existing_parent() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-attach-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let existing = root.join("existing");
        fs::create_dir_all(&existing).expect("create existing parent");
        let missing = existing.join("missing").join("child");
        assert_eq!(
            resolve_attach_cwd(Some(&missing.display().to_string())),
            Some(existing.display().to_string())
        );
        let _ = fs::remove_dir_all(&root);
    }
}

