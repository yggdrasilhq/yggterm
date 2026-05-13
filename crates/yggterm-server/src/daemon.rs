use crate::codex_cli::sync_terminal_identity_appearance;
use crate::terminal::{TerminalBufferStats, terminal_data_has_scrollback_text};
use crate::{
    CodexRuntimeProcessIdentity, GhosttyHostSupport, ManagedSessionView, PersistedDaemonState,
    RemoteMachineSnapshot, RemoteRuntimeRegistry, ServerUiSnapshot, SessionKind,
    SnapshotSessionView, SshConnectTarget, TerminalManager, WorkspaceViewMode, YggtermServer,
    active_client_instance_records, active_client_instance_records_for_endpoint_scope,
    codex_runtime_process_identity_from_root_pid, current_millis, fetch_remote_generation_context,
    local_headless_companion_executable_from_current, overlay_codex_runtime_snapshot_identity,
    persist_remote_generated_copy, remote_resume_runtime_output_requires_restart,
    request_remote_codex_session_shutdown, spawn_hot_restart_daemon_process,
    terminate_remote_codex_session,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use time::OffsetDateTime;
use tracing::{info, warn};
use yggterm_core::{
    AppSettings, PerfSpan, SessionNode, SessionNodeKind, SessionStore, append_trace_event,
    looks_like_generated_fallback_title, resolve_yggterm_home,
};
use yggui_contract::UiTheme;

pub const SERVER_PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");
const BACKGROUND_COPY_CHORE_MS: u64 = 12_000;
const BACKGROUND_COPY_MAX_IDLE_CHORE_MS: u64 = 60_000;
const BACKGROUND_COPY_BUDGET_PER_TICK: usize = 23;
const DAEMON_ACCEPT_POLL_MS: u64 = 1000;
const DEFAULT_DAEMON_IDLE_SHUTDOWN_MS: u64 = 90_000;
const DEFAULT_TERMINAL_IDLE_TRIM_AFTER_MS: u64 = 45_000;
#[cfg(target_os = "linux")]
const DEFAULT_ORPHAN_DAEMON_REAP_AFTER_MS: u64 = 180_000;
#[cfg(target_os = "linux")]
const DUPLICATE_SAME_HOME_GRACE_MS: u64 = 2_000;
const DAEMON_REQUEST_IO_TIMEOUT_MS: u64 = 10_000;
const DAEMON_LONG_REQUEST_IO_TIMEOUT_MS: u64 = 60_000;
const DAEMON_CLIENT_REQUEST_READ_TIMEOUT_MS: u64 = 2_000;
const REMOTE_ATTACH_STARTUP_GRACE_MS: u64 = 900;
const REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS: u64 = 18_000;
const CLIENT_CLOSE_FORCE_SHUTDOWN_AFTER_SECS: u64 = 60 * 60;
const ENV_YGGTERM_ENABLE_BACKGROUND_COPY_CHORE: &str = "YGGTERM_ENABLE_BACKGROUND_COPY_CHORE";

fn spawn_remote_generated_copy_persist(
    machine: RemoteMachineSnapshot,
    session_id: String,
    cwd: String,
    title: Option<String>,
    precis: Option<String>,
    summary: Option<String>,
    model: &'static str,
) {
    let thread_name = format!("remote-copy-persist-{}", machine.machine_key);
    if let Err(error) = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            if let Err(error) = persist_remote_generated_copy(
                &machine,
                &session_id,
                &cwd,
                title.as_deref(),
                precis.as_deref(),
                summary.as_deref(),
                model,
            ) {
                warn!(
                    machine_key=%machine.machine_key,
                    session_id=%session_id,
                    error=%error,
                    "failed to persist remote session copy hints"
                );
                if let Ok(home) = resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "daemon",
                        "copy",
                        "remote_generated_copy_persist_failed",
                        serde_json::json!({
                            "machine_key": machine.machine_key,
                            "session_id": session_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        })
    {
        warn!(error=%error, "failed to spawn remote generated copy persistence worker");
    }
}

fn daemon_env_flag_truthy(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn daemon_env_flag_falsey_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn startup_prewarm_enabled() -> bool {
    if daemon_env_flag_truthy("YGGTERM_DISABLE_STARTUP_TERMINAL_PREWARM") {
        return false;
    }
    std::env::var("YGGTERM_ENABLE_STARTUP_TERMINAL_PREWARM")
        .ok()
        .map(|value| !daemon_env_flag_falsey_value(&value))
        .unwrap_or(true)
}

fn startup_prewarm_skip_reason_with_flag(active_path: &str, enabled: bool) -> Option<&'static str> {
    if !enabled {
        return Some("disabled_by_env");
    }
    if active_path.starts_with("live::") {
        return Some("runtime_owned_live_session");
    }
    None
}

fn startup_prewarm_skip_reason(active_path: &str) -> Option<&'static str> {
    startup_prewarm_skip_reason_with_flag(active_path, startup_prewarm_enabled())
}

fn daemon_request_trace_enabled(request_name: &str) -> bool {
    request_name != "terminal_read" || daemon_env_flag_truthy("YGGTERM_TRACE_TERMINAL_READS")
}

#[cfg(target_os = "linux")]
fn daemon_expected_binary_paths(current_exe: &Path) -> Vec<PathBuf> {
    let mut allowed = vec![current_exe.to_path_buf()];
    if let Some(headless) = local_headless_companion_executable_from_current(current_exe) {
        allowed.push(headless);
    }
    allowed
}

#[cfg(target_os = "linux")]
fn daemon_binary_is_legacy(
    current_exe: &Path,
    argv0: &str,
    proc_exe_target: Option<&Path>,
) -> bool {
    let allowed = daemon_expected_binary_paths(current_exe);
    let argv_allowed = allowed
        .iter()
        .any(|candidate| argv0 == candidate.to_string_lossy());
    if !argv_allowed {
        return true;
    }
    let Some(proc_exe_target) = proc_exe_target else {
        return false;
    };
    let proc_exe_target = daemon_proc_exe_target_without_deleted_suffix(proc_exe_target);
    let target = proc_exe_target.to_string_lossy();
    !allowed
        .iter()
        .any(|candidate| target == candidate.to_string_lossy())
}

#[cfg(target_os = "linux")]
fn daemon_proc_exe_target_without_deleted_suffix(proc_exe_target: &Path) -> PathBuf {
    let target = proc_exe_target.to_string_lossy();
    target
        .strip_suffix(" (deleted)")
        .map(PathBuf::from)
        .unwrap_or_else(|| proc_exe_target.to_path_buf())
}

#[cfg(target_os = "linux")]
fn legacy_daemon_reap_applies_to_home(
    current_home: Option<&Path>,
    daemon_home: Option<&Path>,
) -> bool {
    current_home.is_some() && daemon_home == current_home
}

#[cfg(target_os = "linux")]
fn orphan_daemon_reap_applies_to_home(
    current_home: Option<&Path>,
    daemon_home: Option<&Path>,
) -> bool {
    current_home.is_some() && daemon_home == current_home
}

fn terminal_sidebar_snapshot_from_screen(text: &str) -> Option<(String, Vec<String>)> {
    let lines = text
        .replace('\r', "")
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let tail = lines
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let status_line = tail.last().cloned().unwrap_or_default();
    Some((status_line, tail))
}

#[cfg(unix)]
fn parse_versioned_server_socket_name(path: &Path) -> Option<(u64, u64, u64)> {
    let file_name = path.file_name()?.to_str()?;
    let version = file_name.strip_prefix("server-")?.strip_suffix(".sock")?;
    let mut parts = version.split('-');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(unix)]
fn versioned_server_socket_alias_candidates(current: &Path) -> Vec<PathBuf> {
    let Some(parent) = current.parent() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let candidate = entry.path();
            if candidate != current
                && parse_versioned_server_socket_name(&candidate).is_some()
                && seen.insert(candidate.clone())
            {
                candidates.push(candidate);
            }
        }
    }
    let client_instances_dir = parent.join("client-instances");
    if let Ok(entries) = fs::read_dir(client_instances_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(socket_name) = name.rsplit("--").next() else {
                continue;
            };
            let Some(socket_name) = socket_name
                .find("server-")
                .map(|offset| &socket_name[offset..])
            else {
                continue;
            };
            let Some(socket_stem) = socket_name.strip_suffix("-sock") else {
                continue;
            };
            let candidate = parent.join(format!("{socket_stem}.sock"));
            if candidate != current
                && parse_versioned_server_socket_name(&candidate).is_some()
                && seen.insert(candidate.clone())
            {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort();
    candidates
}

#[cfg(unix)]
fn refresh_legacy_server_socket_aliases(current: &Path) {
    let Some(current_version) = parse_versioned_server_socket_name(current) else {
        return;
    };
    for candidate in versioned_server_socket_alias_candidates(current) {
        let Some(candidate_version) = parse_versioned_server_socket_name(&candidate) else {
            continue;
        };
        if !versioned_socket_alias_is_legacy(current_version, candidate_version) {
            continue;
        }
        if versioned_socket_alias_points_to_current(&candidate, current) {
            continue;
        }
        if versioned_socket_candidate_is_symlink(&candidate) {
            let _ = fs::remove_file(&candidate);
            let _ = std::os::unix::fs::symlink(current, &candidate);
            continue;
        }
        if ping(&ServerEndpoint::UnixSocket(candidate.clone())).is_ok() {
            continue;
        }
        let _ = fs::remove_file(&candidate);
        let _ = std::os::unix::fs::symlink(current, &candidate);
    }
}

#[cfg(unix)]
fn versioned_socket_alias_points_to_current(candidate: &Path, current: &Path) -> bool {
    let Ok(candidate_target) = candidate.canonicalize() else {
        return false;
    };
    let Ok(current_target) = current.canonicalize() else {
        return false;
    };
    candidate_target == current_target
}

#[cfg(unix)]
fn versioned_socket_candidate_is_symlink(candidate: &Path) -> bool {
    fs::symlink_metadata(candidate)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(unix)]
fn versioned_socket_alias_is_legacy(
    current_version: (u64, u64, u64),
    candidate_version: (u64, u64, u64),
) -> bool {
    candidate_version < current_version
}

fn parse_protocol_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.trim().split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn hot_update_target_regression(
    existing_expected: Option<&str>,
    requested_expected: Option<&str>,
) -> Option<(String, String)> {
    if std::env::var("YGGTERM_ALLOW_HOT_UPDATE_TARGET_DOWNGRADE")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
    {
        return None;
    }
    let existing = existing_expected?;
    let requested = requested_expected?;
    let existing_version = parse_protocol_version(existing)?;
    let requested_version = parse_protocol_version(requested)?;
    (requested_version < existing_version).then(|| (existing.to_string(), requested.to_string()))
}

#[cfg(unix)]
fn server_socket_path_lexists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(unix)]
fn drain_unix_client_outcomes(
    receiver: &std::sync::mpsc::Receiver<Result<DaemonRequestOutcome>>,
    restart_after_exit: &mut Option<PathBuf>,
) -> bool {
    let mut should_shutdown = false;
    while let Ok(result) = receiver.try_recv() {
        match result {
            Ok(outcome) => {
                if let Some(executable) = outcome.restart_executable {
                    *restart_after_exit = Some(executable);
                }
                if outcome.should_shutdown {
                    should_shutdown = true;
                }
            }
            Err(error) => warn!(error=%error, "daemon request failed"),
        }
    }
    should_shutdown
}

#[cfg(unix)]
fn spawn_unix_client_handler(
    stream: std::os::unix::net::UnixStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
    outcomes: std::sync::mpsc::Sender<Result<DaemonRequestOutcome>>,
) {
    let thread_name = format!("yggterm-daemon-client-{}", current_millis());
    if let Err(error) = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let result = handle_unix_stream(stream, runtime, last_activity_ms);
            let _ = outcomes.send(result);
        })
    {
        warn!(error=%error, "failed to spawn daemon client handler");
    }
}

#[cfg(unix)]
const MAX_UNIX_SOCKET_PATH_BYTES: usize = 100;

#[cfg(unix)]
fn unix_socket_path_fits_platform(path: &Path) -> bool {
    path.as_os_str().as_bytes().len() < MAX_UNIX_SOCKET_PATH_BYTES
}

#[cfg(unix)]
fn stable_socket_home_hash(home_dir: &Path) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in home_dir.as_os_str().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(unix)]
fn runtime_socket_base_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::geteuid() };
            std::env::temp_dir().join(format!("yggterm-{uid}"))
        })
        .join("yggterm")
}

#[cfg(unix)]
fn short_unix_socket_path_for_home(home_dir: &Path, socket_name: &str) -> PathBuf {
    let hash = stable_socket_home_hash(home_dir);
    let scoped = PathBuf::from(format!("h-{hash:016x}")).join(socket_name);
    let runtime_candidate = runtime_socket_base_dir().join(&scoped);
    if unix_socket_path_fits_platform(&runtime_candidate) {
        return runtime_candidate;
    }
    let temp_candidate = std::env::temp_dir()
        .join(format!("yggterm-{}", unsafe { libc::geteuid() }))
        .join(scoped);
    if unix_socket_path_fits_platform(&temp_candidate) {
        return temp_candidate;
    }
    std::env::temp_dir().join(format!(
        "yg-{hash:016x}-{}.sock",
        SERVER_PROTOCOL_VERSION.replace('.', "-")
    ))
}

#[cfg(unix)]
fn unix_socket_path_uses_runtime_fallback(path: &Path) -> bool {
    path.parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("yggterm")
}

#[cfg(unix)]
fn daemon_logical_home_for_endpoint(endpoint: &ServerEndpoint) -> Option<PathBuf> {
    match endpoint {
        ServerEndpoint::UnixSocket(path) if unix_socket_path_uses_runtime_fallback(path) => {
            resolve_yggterm_home()
                .ok()
                .or_else(|| path.parent().map(Path::to_path_buf))
        }
        ServerEndpoint::UnixSocket(path) => path.parent().map(Path::to_path_buf),
        ServerEndpoint::Tcp { .. } => resolve_yggterm_home().ok(),
    }
}

fn uses_runtime_owned_terminal_path(path: &str) -> bool {
    path.starts_with("remote-session://") || path.starts_with("codex-runtime://")
}

fn terminal_launch_command_for_path(
    _path: &str,
    stored_launch_command: String,
    _legacy_direct_attach_launch_command: Option<String>,
) -> String {
    stored_launch_command
}

fn remote_resume_stale_attach(
    has_runtime_output: bool,
    runtime_age_ms: u64,
    runtime_output_requires_restart: bool,
    startup_grace_ms: u64,
) -> bool {
    runtime_output_requires_restart || (!has_runtime_output && runtime_age_ms >= startup_grace_ms)
}

fn valid_initial_terminal_size(cols: Option<u16>, rows: Option<u16>) -> Option<(u16, u16)> {
    match (cols, rows) {
        (Some(cols), Some(rows)) if cols > 0 && rows > 0 => Some((cols, rows)),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEndpoint {
    #[cfg(unix)]
    UnixSocket(PathBuf),
    Tcp {
        host: String,
        port: u16,
    },
}

const HOT_UPDATE_OWNERS_FILE: &str = "hot-update-terminal-owners.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", rename_all = "snake_case")]
enum PreservedOwnerEndpoint {
    #[cfg(unix)]
    Unix {
        path: String,
    },
    Tcp {
        host: String,
        port: u16,
    },
}

impl PreservedOwnerEndpoint {
    fn from_endpoint(endpoint: &ServerEndpoint) -> Self {
        match endpoint {
            #[cfg(unix)]
            ServerEndpoint::UnixSocket(path) => Self::Unix {
                path: path.display().to_string(),
            },
            ServerEndpoint::Tcp { host, port } => Self::Tcp {
                host: host.clone(),
                port: *port,
            },
        }
    }

    fn to_endpoint(&self) -> ServerEndpoint {
        match self {
            #[cfg(unix)]
            Self::Unix { path } => ServerEndpoint::UnixSocket(PathBuf::from(path)),
            Self::Tcp { host, port } => ServerEndpoint::Tcp {
                host: host.clone(),
                port: *port,
            },
        }
    }

    fn label(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix { path } => path.clone(),
            Self::Tcp { host, port } => format!("{host}:{port}"),
        }
    }
}

fn owner_endpoint_label(endpoint: &ServerEndpoint) -> String {
    PreservedOwnerEndpoint::from_endpoint(endpoint).label()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreservedTerminalOwnerEntry {
    runtime_key: String,
    endpoint: PreservedOwnerEndpoint,
    owner_server_version: String,
    owner_server_build_id: u64,
    owner_server_pid: u32,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreservedTerminalOwnerRegistry {
    #[serde(default = "preserved_terminal_owner_schema_version")]
    schema_version: u32,
    #[serde(default)]
    expected_server_version: Option<String>,
    #[serde(default)]
    entries: Vec<PreservedTerminalOwnerEntry>,
}

fn preserved_terminal_owner_schema_version() -> u32 {
    1
}

impl Default for PreservedTerminalOwnerRegistry {
    fn default() -> Self {
        Self {
            schema_version: preserved_terminal_owner_schema_version(),
            expected_server_version: None,
            entries: Vec::new(),
        }
    }
}

impl PreservedTerminalOwnerRegistry {
    fn path(home_dir: &Path) -> PathBuf {
        home_dir.join(HOT_UPDATE_OWNERS_FILE)
    }

    fn load(home_dir: &Path) -> Self {
        let path = Self::path(home_dir);
        let Ok(bytes) = fs::read(&path) else {
            return Self {
                schema_version: preserved_terminal_owner_schema_version(),
                expected_server_version: None,
                entries: Vec::new(),
            };
        };
        serde_json::from_slice::<Self>(&bytes).unwrap_or_default()
    }

    fn save(&self, home_dir: &Path) -> Result<()> {
        let path = Self::path(home_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating hot-update owner dir {}", parent.display()))?;
        }
        let temp_path = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).context("serializing hot-update owners")?;
        fs::write(&temp_path, json)
            .with_context(|| format!("writing hot-update owners temp {}", temp_path.display()))?;
        fs::rename(&temp_path, &path)
            .or_else(|_| {
                fs::copy(&temp_path, &path)?;
                fs::remove_file(&temp_path)?;
                Ok::<(), std::io::Error>(())
            })
            .with_context(|| format!("writing hot-update owners {}", path.display()))
    }

    fn write_handoff(
        home_dir: &Path,
        owner_endpoint: &ServerEndpoint,
        owner_status: &ServerRuntimeStatus,
        expected_server_version: Option<String>,
        runtime_keys: Vec<String>,
        existing_entries: Vec<PreservedTerminalOwnerEntry>,
    ) -> Result<Self> {
        let existing_registry = Self::load(home_dir);
        if !existing_registry.entries.is_empty()
            && let Some((existing_expected, requested_expected)) = hot_update_target_regression(
                existing_registry.expected_server_version.as_deref(),
                expected_server_version.as_deref(),
            )
        {
            append_trace_event(
                home_dir,
                "daemon",
                "hot_update",
                "handoff_target_regression_refused",
                serde_json::json!({
                    "existing_expected_server_version": existing_expected,
                    "requested_expected_server_version": requested_expected,
                    "owner_endpoint": owner_endpoint_label(owner_endpoint),
                    "owner_server_version": &owner_status.server_version,
                    "owner_server_pid": owner_status.server_pid,
                    "runtime_keys": runtime_keys,
                    "reason": "newer_handoff_target_already_registered",
                }),
            );
            bail!(
                "refusing hot-update handoff target regression from {existing_expected} to {requested_expected}; preserving the newer session-survival target"
            );
        }
        let owner_endpoint = PreservedOwnerEndpoint::from_endpoint(owner_endpoint);
        let now_ms = current_millis_u64();
        let local_keys = runtime_keys.into_iter().collect::<HashSet<_>>();
        let mut entries = existing_entries
            .into_iter()
            .filter(|entry| !local_keys.contains(&entry.runtime_key))
            .collect::<Vec<_>>();
        entries.extend(
            local_keys
                .into_iter()
                .map(|runtime_key| PreservedTerminalOwnerEntry {
                    runtime_key,
                    endpoint: owner_endpoint.clone(),
                    owner_server_version: owner_status.server_version.clone(),
                    owner_server_build_id: owner_status.server_build_id,
                    owner_server_pid: owner_status.server_pid,
                    created_at_ms: now_ms,
                }),
        );
        let mut entries = entries;
        entries.sort_by(|left, right| left.runtime_key.cmp(&right.runtime_key));
        entries.dedup_by(|left, right| left.runtime_key == right.runtime_key);
        let registry = Self {
            schema_version: preserved_terminal_owner_schema_version(),
            expected_server_version,
            entries,
        };
        registry.save(home_dir)?;
        Ok(registry)
    }

    fn retarget_expected_server_version(
        &mut self,
        home_dir: &Path,
        expected_server_version: Option<String>,
    ) -> Result<bool> {
        if self.entries.is_empty() || self.expected_server_version == expected_server_version {
            return Ok(false);
        }
        if let Some((existing_expected, requested_expected)) = hot_update_target_regression(
            self.expected_server_version.as_deref(),
            expected_server_version.as_deref(),
        ) {
            append_trace_event(
                home_dir,
                "daemon",
                "hot_update",
                "handoff_target_regression_refused",
                serde_json::json!({
                    "existing_expected_server_version": existing_expected,
                    "requested_expected_server_version": requested_expected,
                    "runtime_keys": self.keys(),
                    "reason": "retarget_would_downgrade_existing_handoff",
                }),
            );
            bail!(
                "refusing hot-update handoff target regression from {existing_expected} to {requested_expected}; preserving the newer session-survival target"
            );
        }
        self.expected_server_version = expected_server_version;
        self.save(home_dir)?;
        Ok(true)
    }

    fn keys(&self) -> Vec<String> {
        let mut keys = self
            .entries
            .iter()
            .map(|entry| entry.runtime_key.clone())
            .collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        keys
    }

    fn endpoint_groups(&self) -> Vec<(ServerEndpoint, Vec<String>)> {
        let mut groups = BTreeMap::<String, (ServerEndpoint, Vec<String>)>::new();
        for entry in &self.entries {
            let label = entry.endpoint.label();
            let endpoint = entry.endpoint.to_endpoint();
            groups
                .entry(label)
                .or_insert_with(|| (endpoint, Vec::new()))
                .1
                .push(entry.runtime_key.clone());
        }
        groups
            .into_values()
            .map(|(endpoint, mut runtime_keys)| {
                runtime_keys.sort();
                runtime_keys.dedup();
                (endpoint, runtime_keys)
            })
            .collect()
    }

    fn retain_represented_keys<F>(&mut self, mut is_represented: F) -> Vec<String>
    where
        F: FnMut(&str) -> bool,
    {
        let mut removed = Vec::new();
        self.entries.retain(|entry| {
            if is_represented(&entry.runtime_key) {
                true
            } else {
                removed.push(entry.runtime_key.clone());
                false
            }
        });
        removed.sort();
        removed.dedup();
        removed
    }

    fn owner_for_key(&self, runtime_key: &str) -> Option<&PreservedTerminalOwnerEntry> {
        self.entries
            .iter()
            .find(|entry| entry.runtime_key == runtime_key)
    }

    fn remove_key(&mut self, runtime_key: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.runtime_key != runtime_key);
        before != self.entries.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRuntimeStatus {
    pub server_version: String,
    #[serde(default)]
    pub server_build_id: u64,
    #[serde(default)]
    pub server_pid: u32,
    pub host_kind: String,
    pub host_detail: String,
    pub embedded_surface_supported: bool,
    pub bridge_enabled: bool,
    #[serde(default)]
    pub restored_from_persisted_state: bool,
    #[serde(default)]
    pub restored_stored_sessions: usize,
    #[serde(default)]
    pub restored_live_sessions: usize,
    #[serde(default)]
    pub restored_remote_machines: usize,
    #[serde(default)]
    pub owned_terminal_session_count: usize,
    #[serde(default)]
    pub owned_terminal_session_keys: Vec<String>,
    #[serde(default)]
    pub terminal_session_count: usize,
    #[serde(default)]
    pub terminal_session_keys: Vec<String>,
    #[serde(default)]
    pub preserved_terminal_owner_count: usize,
    #[serde(default)]
    pub preserved_terminal_owner_keys: Vec<String>,
    #[serde(default)]
    pub terminal_retained_chunks: usize,
    #[serde(default)]
    pub terminal_retained_bytes: usize,
    #[serde(default)]
    pub terminal_session_buffer_limit_bytes: usize,
    #[serde(default)]
    pub terminal_idle_buffer_limit_bytes: usize,
    #[serde(default)]
    pub managed_session_count: usize,
    #[serde(default)]
    pub session_metadata_entries: usize,
    #[serde(default)]
    pub session_metadata_bytes: usize,
    #[serde(default)]
    pub session_preview_block_count: usize,
    #[serde(default)]
    pub session_preview_line_count: usize,
    #[serde(default)]
    pub session_preview_bytes: usize,
    #[serde(default)]
    pub session_rendered_section_count: usize,
    #[serde(default)]
    pub session_rendered_line_count: usize,
    #[serde(default)]
    pub session_rendered_bytes: usize,
    #[serde(default)]
    pub session_terminal_line_count: usize,
    #[serde(default)]
    pub session_terminal_bytes: usize,
    #[serde(default)]
    pub session_payload_total_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalStreamChunk {
    pub seq: u64,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerRequest {
    Ping,
    Status,
    Snapshot,
    PrepareUpdateRestart,
    PrepareClientClose,
    HotRestart {
        daemon_executable: String,
        expected_version: Option<String>,
        expected_build_id: Option<u64>,
        reason: Option<String>,
    },
    OpenStoredSession {
        session_kind: SessionKind,
        path: String,
        session_id: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
        view_mode: Option<WorkspaceViewMode>,
    },
    ConnectSsh {
        target_ix: usize,
    },
    ConnectSshCustom {
        target: String,
        prefix: Option<String>,
    },
    StartSshSession {
        target: String,
        prefix: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
    },
    StartRemoteCodexSession {
        target: String,
        prefix: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
    },
    OpenRemoteSession {
        machine_key: String,
        session_id: String,
        cwd: Option<String>,
        title_hint: Option<String>,
        view_mode: Option<WorkspaceViewMode>,
    },
    RefreshRemoteMachine {
        machine_key: String,
    },
    RefreshManagedCli {
        machine_key: Option<String>,
        background: bool,
    },
    RefreshPreview {
        path: String,
    },
    UpdateSessionCopy {
        path: String,
        title: Option<String>,
        precis: Option<String>,
        summary: Option<String>,
    },
    RemoveSshTarget {
        machine_key: String,
    },
    RemoveSession {
        path: String,
    },
    SetSessionKeepAlive {
        path: String,
        keep_alive: bool,
    },
    StartLocalSession {
        session_kind: SessionKind,
        cwd: Option<String>,
        title_hint: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
    },
    SwitchAgentSessionMode {
        path: String,
        session_kind: SessionKind,
    },
    StartCommandSession {
        cwd: Option<String>,
        title_hint: Option<String>,
        launch_command: String,
        source_label: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
    },
    EnsureRemoteRuntimeCodexSession {
        session_id: String,
        cwd: Option<String>,
        require_existing: bool,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
    },
    StartRemoteRuntimeCodexSession {
        session_id: String,
        cwd: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
    },
    FocusLive {
        key: String,
        view_mode: Option<WorkspaceViewMode>,
    },
    SetViewMode {
        mode: WorkspaceViewMode,
    },
    TogglePreviewBlock {
        block_ix: usize,
    },
    SetAllPreviewBlocksFolded {
        folded: bool,
    },
    RequestTerminalLaunch,
    RequestTerminalLaunchForPath {
        path: String,
    },
    TerminalEnsure {
        path: String,
    },
    TerminalRead {
        path: String,
        cursor: u64,
    },
    TerminalSnapshot {
        path: String,
    },
    TerminalRetainedSnapshot {
        path: String,
    },
    TerminalWrite {
        path: String,
        data: String,
    },
    TerminalResize {
        path: String,
        cols: u16,
        rows: u16,
    },
    TerminalRestart {
        path: String,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        force_remote: bool,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
    },
    SyncExternalWindow,
    RaiseExternalWindow,
    SyncTheme {
        theme: UiTheme,
    },
    SyncTerminalIdentity {
        terminal_appearance: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerResponse {
    Pong,
    Status(ServerRuntimeStatus),
    Snapshot {
        snapshot: ServerUiSnapshot,
        message: Option<String>,
    },
    TerminalStream {
        cursor: u64,
        chunks: Vec<TerminalStreamChunk>,
        running: bool,
        runtime_output_seen: bool,
        eof_without_output: bool,
        #[serde(default)]
        post_resize_output_seen: bool,
        #[serde(default)]
        last_resize_seq: u64,
    },
    TerminalSnapshot {
        text: String,
        running: bool,
        runtime_output_seen: bool,
        #[serde(default)]
        post_resize_output_seen: bool,
        #[serde(default)]
        last_resize_seq: u64,
    },
    TerminalRetainedSnapshot {
        text: String,
        running: bool,
        runtime_output_seen: bool,
        #[serde(default)]
        post_resize_output_seen: bool,
        #[serde(default)]
        last_resize_seq: u64,
    },
    Ack {
        message: Option<String>,
    },
    HotUpdateHandoff {
        message: Option<String>,
        owner_endpoint: String,
        owner_server_version: String,
        owner_server_pid: u32,
        target_server_version: Option<String>,
        runtime_keys: Vec<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotRestartResult {
    Restarting {
        message: Option<String>,
    },
    Handoff {
        message: Option<String>,
        owner_endpoint: String,
        owner_server_version: String,
        owner_server_pid: u32,
        target_server_version: Option<String>,
        runtime_keys: Vec<String>,
    },
}

impl HotRestartResult {
    pub fn message(&self) -> Option<String> {
        match self {
            HotRestartResult::Restarting { message } => message.clone(),
            HotRestartResult::Handoff { message, .. } => message.clone(),
        }
    }
}

struct DaemonRuntime {
    support: GhosttyHostSupport,
    state_path: PathBuf,
    store: SessionStore,
    server: YggtermServer,
    terminals: TerminalManager,
    preserved_terminal_owners: PreservedTerminalOwnerRegistry,
    remote_machine_refreshes_in_flight: HashSet<String>,
    codex_process_identity_cache: Mutex<BTreeMap<u32, CodexRuntimeProcessIdentity>>,
    restored_from_persisted_state: bool,
    restored_stored_sessions: usize,
    restored_live_sessions: usize,
    restored_remote_machines: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteMachineRefreshQueueStatus {
    Spawn,
    AlreadyInFlight,
    Loopback,
}

#[derive(Debug, Clone)]
struct BackgroundCopyCandidate {
    session_path: String,
    session_id: String,
    cwd: String,
    title: String,
    source_updated_at: Option<OffsetDateTime>,
    remote_machine: Option<RemoteMachineSnapshot>,
    generation_context: Option<String>,
    storage_path: Option<String>,
    cached_summary: Option<String>,
}

#[derive(Debug, Clone)]
struct BackgroundCopyUpdate {
    session_path: String,
    title: Option<String>,
    summary: Option<String>,
}

impl DaemonRuntime {
    fn load(support: GhosttyHostSupport) -> Result<Self> {
        let store = SessionStore::open_or_init()?;
        let perf = PerfSpan::start(store.home_dir(), "daemon", "runtime_load");
        let settings = store.load_settings().unwrap_or_default();
        let tree = store
            .load_codex_tree(&settings)
            .or_else(|_| store.load_tree())?;
        let mut server = YggtermServer::new(
            &tree,
            settings.prefer_ghostty_backend,
            support.clone(),
            settings.theme,
        );
        let state_path = store.home_dir().join("server-state.json");
        let mut preserved_terminal_owners = PreservedTerminalOwnerRegistry::load(store.home_dir());
        if preserved_terminal_owners
            .expected_server_version
            .as_deref()
            .is_some_and(|version| version != SERVER_PROTOCOL_VERSION)
        {
            preserved_terminal_owners.entries.clear();
        }
        let mut restored_from_persisted_state = false;
        let mut restored_stored_sessions = 0usize;
        let mut restored_live_sessions = 0usize;
        let mut restored_remote_machines = 0usize;
        if let Some(saved) = load_persisted_state(&state_path)? {
            restored_from_persisted_state = true;
            restored_stored_sessions = saved.stored_sessions.len();
            restored_live_sessions = saved.live_sessions.len();
            restored_remote_machines = saved.remote_machines.len();
            // Do not block daemon boot on active terminal launch. Bind the control socket first,
            // then let explicit terminal-open requests or later recovery drive runtime startup.
            server.restore_persisted_state_with_launch_policy(saved, Some(&store), false);
        }
        let mut runtime = Self {
            support,
            state_path,
            store,
            server,
            terminals: TerminalManager::new(),
            preserved_terminal_owners,
            remote_machine_refreshes_in_flight: HashSet::new(),
            codex_process_identity_cache: Mutex::new(BTreeMap::new()),
            restored_from_persisted_state,
            restored_stored_sessions,
            restored_live_sessions,
            restored_remote_machines,
        };
        runtime.prune_unrepresented_preserved_owners("runtime_load");
        runtime.prune_unrepresented_preserved_owner_runtime_sessions("runtime_load");
        perf.finish(serde_json::json!({
            "prefer_ghostty_backend": settings.prefer_ghostty_backend,
            "theme": format!("{:?}", settings.theme),
        }));
        Ok(runtime)
    }

    fn preserved_terminal_owner_keys(&self) -> Vec<String> {
        self.preserved_terminal_owners
            .keys()
            .into_iter()
            .filter(|key| self.server.represents_terminal_runtime_key(key))
            .filter(|key| !self.terminals.has_session(key))
            .collect()
    }

    fn terminal_runtime_keys_including_preserved(&self) -> Vec<String> {
        let mut keys = self.terminals.session_keys();
        keys.extend(self.preserved_terminal_owner_keys());
        keys.sort();
        keys.dedup();
        keys
    }

    fn status(&self) -> ServerRuntimeStatus {
        let terminal_stats = self.terminals.stats();
        let payload_stats = self.server.payload_stats();
        let preserved_owner_keys = self.preserved_terminal_owner_keys();
        let owned_terminal_session_keys = self.terminals.session_keys();
        let mut terminal_session_keys = owned_terminal_session_keys.clone();
        terminal_session_keys.extend(preserved_owner_keys.iter().cloned());
        terminal_session_keys.sort();
        terminal_session_keys.dedup();
        ServerRuntimeStatus {
            server_version: SERVER_PROTOCOL_VERSION.to_string(),
            server_build_id: current_build_id(),
            server_pid: std::process::id(),
            host_kind: self.support.kind.as_str().to_string(),
            host_detail: self.support.detail.clone(),
            embedded_surface_supported: self.support.embedded_surface_supported,
            bridge_enabled: self.support.bridge_enabled,
            restored_from_persisted_state: self.restored_from_persisted_state,
            restored_stored_sessions: self.restored_stored_sessions,
            restored_live_sessions: self.restored_live_sessions,
            restored_remote_machines: self.restored_remote_machines,
            owned_terminal_session_count: owned_terminal_session_keys.len(),
            owned_terminal_session_keys,
            terminal_session_count: terminal_session_keys.len(),
            terminal_session_keys,
            preserved_terminal_owner_count: preserved_owner_keys.len(),
            preserved_terminal_owner_keys: preserved_owner_keys,
            terminal_retained_chunks: terminal_stats.retained_chunks,
            terminal_retained_bytes: terminal_stats.retained_bytes,
            terminal_session_buffer_limit_bytes: crate::terminal::MAX_BUFFER_BYTES,
            terminal_idle_buffer_limit_bytes: crate::terminal::IDLE_TRIM_MAX_BYTES,
            managed_session_count: payload_stats.session_count,
            session_metadata_entries: payload_stats.metadata_entries,
            session_metadata_bytes: payload_stats.metadata_bytes,
            session_preview_block_count: payload_stats.preview_blocks,
            session_preview_line_count: payload_stats.preview_lines,
            session_preview_bytes: payload_stats.preview_bytes,
            session_rendered_section_count: payload_stats.rendered_sections,
            session_rendered_line_count: payload_stats.rendered_lines,
            session_rendered_bytes: payload_stats.rendered_bytes,
            session_terminal_line_count: payload_stats.terminal_lines,
            session_terminal_bytes: payload_stats.terminal_bytes,
            session_payload_total_bytes: payload_stats.total_bytes,
        }
    }

    fn overlay_terminal_runtime_snapshot_session(&self, session: &mut SnapshotSessionView) {
        let runtime_path = self.terminal_runtime_key_for_path(&session.session_path);
        session.terminal_process_id = self.terminals.session_process_id(&runtime_path);
        session.terminal_foreground_active = self
            .terminals
            .session_foreground_process_active(&runtime_path);
        if matches!(
            session.kind,
            SessionKind::Shell | SessionKind::Codex | SessionKind::CodexLiteLlm
        ) && let Some(screen_text) = self.terminals.session_screen_snapshot(&runtime_path)
            && let Some((status_line, terminal_lines)) =
                terminal_sidebar_snapshot_from_screen(&screen_text)
        {
            session.status_line = status_line;
            session.terminal_lines = terminal_lines;
        }
    }

    fn overlay_codex_runtime_snapshot_session(
        &self,
        session: &mut SnapshotSessionView,
        yggterm_home: Option<&Path>,
    ) {
        if session.kind != SessionKind::Codex {
            return;
        }
        let Some(pid) = session.terminal_process_id else {
            return;
        };
        let identity = self
            .codex_process_identity_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&pid).cloned())
            .or_else(|| {
                let identity = codex_runtime_process_identity_from_root_pid(pid)?;
                if let Ok(mut cache) = self.codex_process_identity_cache.lock() {
                    cache.insert(pid, identity.clone());
                }
                Some(identity)
            });
        if let Some(identity) = identity {
            overlay_codex_runtime_snapshot_identity(session, &identity, yggterm_home);
        }
    }

    fn codex_runtime_identity_for_pid(&self, pid: u32) -> Option<CodexRuntimeProcessIdentity> {
        self.codex_process_identity_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&pid).cloned())
            .or_else(|| {
                let identity = codex_runtime_process_identity_from_root_pid(pid)?;
                if let Ok(mut cache) = self.codex_process_identity_cache.lock() {
                    cache.insert(pid, identity.clone());
                }
                Some(identity)
            })
    }

    fn refresh_live_codex_runtime_identities_for_persistence(&mut self) -> usize {
        let yggterm_home = resolve_yggterm_home().ok();
        let keys = self.server.live_codex_session_keys_for_runtime_identity();
        let mut refreshed = 0usize;
        for key in keys {
            let runtime_path = self.server.terminal_runtime_key_for_path(&key);
            let Some(pid) = self.terminals.session_process_id(&runtime_path) else {
                continue;
            };
            let Some(identity) = self.codex_runtime_identity_for_pid(pid) else {
                continue;
            };
            if self.server.apply_codex_runtime_identity_to_live_session(
                &key,
                &identity,
                yggterm_home.as_deref(),
            ) {
                refreshed += 1;
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "persistence",
                    "codex_runtime_identity_refreshed",
                    serde_json::json!({
                        "session_path": key,
                        "runtime_path": runtime_path,
                        "pid": pid,
                        "codex_session_id": identity.session_id,
                        "storage_path": identity.storage_path.display().to_string(),
                    }),
                );
            }
        }
        refreshed
    }

    fn snapshot_response(&self, message: Option<String>) -> ServerResponse {
        let mut snapshot = self.server.snapshot();
        let runtime_keys = self
            .terminal_runtime_keys_including_preserved()
            .into_iter()
            .collect();
        apply_terminal_runtime_truth_to_snapshot(&self.server, &runtime_keys, &mut snapshot);
        let yggterm_home = resolve_yggterm_home().ok();
        if let Some(active_session) = snapshot.active_session.as_mut() {
            self.overlay_terminal_runtime_snapshot_session(active_session);
            self.overlay_codex_runtime_snapshot_session(active_session, yggterm_home.as_deref());
        }
        for session in &mut snapshot.live_sessions {
            self.overlay_terminal_runtime_snapshot_session(session);
            self.overlay_codex_runtime_snapshot_session(session, yggterm_home.as_deref());
        }
        ServerResponse::Snapshot { snapshot, message }
    }

    fn terminal_runtime_key_for_path(&self, path: &str) -> String {
        self.server.terminal_runtime_key_for_path(path)
    }

    fn preserved_owner_for_runtime_key(&self, runtime_key: &str) -> Option<ServerEndpoint> {
        if self.terminals.has_session(runtime_key) {
            return None;
        }
        if !self.server.represents_terminal_runtime_key(runtime_key) {
            return None;
        }
        let entry = self.preserved_terminal_owners.owner_for_key(runtime_key)?;
        let endpoint = entry.endpoint.to_endpoint();
        if endpoint == default_endpoint(self.store.home_dir()) {
            return None;
        }
        Some(endpoint)
    }

    fn remove_preserved_owner(&mut self, runtime_key: &str, reason: &'static str) {
        if !self.preserved_terminal_owners.remove_key(runtime_key) {
            return;
        }
        let _ = self.preserved_terminal_owners.save(self.store.home_dir());
        append_trace_event(
            self.store.home_dir(),
            "daemon",
            "hot_update",
            "preserved_owner_removed",
            serde_json::json!({
                "runtime_key": runtime_key,
                "reason": reason,
            }),
        );
    }

    fn remove_preserved_owner_runtime(&mut self, runtime_key: &str, reason: &'static str) {
        let current_endpoint = default_endpoint(self.store.home_dir());
        let owner_endpoint = self
            .preserved_terminal_owners
            .owner_for_key(runtime_key)
            .map(|entry| entry.endpoint.to_endpoint())
            .filter(|endpoint| endpoint != &current_endpoint);
        if let Some(owner_endpoint) = owner_endpoint {
            match remove_session(&owner_endpoint, runtime_key) {
                Ok((_snapshot, message)) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_runtime_removed",
                        serde_json::json!({
                            "reason": reason,
                            "runtime_key": runtime_key,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "message": message,
                        }),
                    );
                }
                Err(error) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_runtime_remove_failed",
                        serde_json::json!({
                            "reason": reason,
                            "runtime_key": runtime_key,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
        self.remove_preserved_owner(runtime_key, reason);
    }

    fn prune_unrepresented_preserved_owners(&mut self, reason: &'static str) {
        let removed = self
            .preserved_terminal_owners
            .retain_represented_keys(|key| self.server.represents_terminal_runtime_key(key));
        if removed.is_empty() {
            return;
        }
        let _ = self.preserved_terminal_owners.save(self.store.home_dir());
        append_trace_event(
            self.store.home_dir(),
            "daemon",
            "hot_update",
            "preserved_owner_registry_pruned",
            serde_json::json!({
                "reason": reason,
                "removed_runtime_keys": removed,
                "remaining_runtime_keys": self.preserved_terminal_owners.keys(),
            }),
        );
    }

    fn prune_unrepresented_preserved_owner_runtime_sessions(&self, reason: &'static str) {
        let endpoint_groups = self.preserved_terminal_owners.endpoint_groups();
        if endpoint_groups.is_empty() {
            return;
        }
        let represented_runtime_keys = self
            .preserved_terminal_owners
            .keys()
            .into_iter()
            .collect::<HashSet<_>>();
        let current_endpoint = default_endpoint(self.store.home_dir());
        for (owner_endpoint, owner_registry_keys) in endpoint_groups {
            if owner_endpoint == current_endpoint {
                continue;
            }
            let owner_status = match status(&owner_endpoint) {
                Ok(status) => status,
                Err(error) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_runtime_prune_status_failed",
                        serde_json::json!({
                            "reason": reason,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "error": error.to_string(),
                        }),
                    );
                    continue;
                }
            };
            let mut owner_runtime_keys = if owner_status.owned_terminal_session_keys.is_empty() {
                owner_status.terminal_session_keys.clone()
            } else {
                owner_status.owned_terminal_session_keys.clone()
            };
            owner_runtime_keys.sort();
            owner_runtime_keys.dedup();
            let stale_runtime_keys = unrepresented_preserved_owner_runtime_keys(
                &self.server,
                &represented_runtime_keys,
                owner_runtime_keys,
            );
            if stale_runtime_keys.is_empty() {
                continue;
            }
            let mut removed_runtime_keys = Vec::new();
            let mut errors = Vec::new();
            for runtime_key in stale_runtime_keys {
                match remove_session(&owner_endpoint, &runtime_key) {
                    Ok((_snapshot, _message)) => removed_runtime_keys.push(runtime_key),
                    Err(error) => {
                        errors.push(serde_json::json!({
                            "runtime_key": runtime_key,
                            "error": error.to_string(),
                        }));
                    }
                }
            }
            let registry_keys_are_all_keep_alive = owner_registry_keys
                .iter()
                .all(|key| self.server.live_session_keep_alive(key));
            let mut fallback_prepare_client_close = None::<serde_json::Value>;
            if !errors.is_empty() && registry_keys_are_all_keep_alive {
                fallback_prepare_client_close = Some(match prepare_client_close(&owner_endpoint) {
                    Ok(message) => serde_json::json!({
                        "attempted": true,
                        "ok": true,
                        "message": message,
                    }),
                    Err(error) => serde_json::json!({
                        "attempted": true,
                        "ok": false,
                        "error": error.to_string(),
                    }),
                });
            }
            let remaining_stale_runtime_keys = match status(&owner_endpoint) {
                Ok(status_after) => {
                    let after_keys = if status_after.owned_terminal_session_keys.is_empty() {
                        status_after.terminal_session_keys
                    } else {
                        status_after.owned_terminal_session_keys
                    };
                    unrepresented_preserved_owner_runtime_keys(
                        &self.server,
                        &represented_runtime_keys,
                        after_keys,
                    )
                }
                Err(_) => Vec::new(),
            };
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "hot_update",
                "preserved_owner_unrepresented_runtimes_pruned",
                serde_json::json!({
                    "reason": reason,
                    "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                    "owner_server_version": owner_status.server_version,
                    "owner_server_pid": owner_status.server_pid,
                    "removed_runtime_keys": removed_runtime_keys,
                    "errors": errors,
                    "fallback_prepare_client_close": fallback_prepare_client_close,
                    "remaining_stale_runtime_keys": remaining_stale_runtime_keys,
                }),
            );
        }
    }

    fn preserved_owner_status_for_runtime_key(
        &mut self,
        runtime_key: &str,
    ) -> Option<(ServerEndpoint, ServerRuntimeStatus)> {
        let endpoint = self.preserved_owner_for_runtime_key(runtime_key)?;
        match status(&endpoint) {
            Ok(status)
                if status
                    .terminal_session_keys
                    .iter()
                    .any(|key| key == runtime_key) =>
            {
                Some((endpoint, status))
            }
            Ok(status) => {
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "preserved_owner_missing_runtime",
                    serde_json::json!({
                        "runtime_key": runtime_key,
                        "owner_endpoint": format!("{endpoint:?}"),
                        "owner_version": status.server_version,
                        "owner_pid": status.server_pid,
                    }),
                );
                self.remove_preserved_owner(runtime_key, "owner_missing_runtime");
                None
            }
            Err(error) => {
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "preserved_owner_unreachable",
                    serde_json::json!({
                        "runtime_key": runtime_key,
                        "owner_endpoint": format!("{endpoint:?}"),
                        "error": error.to_string(),
                    }),
                );
                self.remove_preserved_owner(runtime_key, "owner_unreachable");
                None
            }
        }
    }

    fn preserved_owner_endpoint_for_request(&self, runtime_key: &str) -> Option<ServerEndpoint> {
        self.preserved_owner_for_runtime_key(runtime_key)
    }

    fn handle_preserved_owner_request_error(
        &mut self,
        runtime_key: &str,
        owner_endpoint: &ServerEndpoint,
        error: &anyhow::Error,
    ) {
        append_trace_event(
            self.store.home_dir(),
            "daemon",
            "hot_update",
            "preserved_owner_request_failed",
            serde_json::json!({
                "runtime_key": runtime_key,
                "owner_endpoint": format!("{owner_endpoint:?}"),
                "error": error.to_string(),
            }),
        );
        match status(owner_endpoint) {
            Ok(status)
                if status
                    .terminal_session_keys
                    .iter()
                    .any(|key| key == runtime_key) => {}
            Ok(_) => self.remove_preserved_owner(runtime_key, "owner_missing_runtime_after_error"),
            Err(_) => self.remove_preserved_owner(runtime_key, "owner_unreachable_after_error"),
        }
    }

    fn ensure_terminal_for_path(&mut self, path: &str) -> Result<Option<String>> {
        self.ensure_terminal_for_path_with_initial_size(path, None)
    }

    fn adopt_legacy_local_codex_runtime(&mut self, session_id: &str, runtime_key: &str) {
        if !runtime_key.starts_with("codex-runtime://") {
            return;
        }
        let legacy_key = crate::local_live_runtime_key(session_id);
        if legacy_key == runtime_key {
            return;
        }
        let renamed = self.terminals.rename_session(&legacy_key, runtime_key);
        if renamed {
            info!(
                legacy_key = legacy_key.as_str(),
                runtime_key, "adopted legacy local Codex runtime key"
            );
        }
    }

    fn ensure_terminal_for_path_with_initial_size(
        &mut self,
        path: &str,
        initial_size: Option<(u16, u16)>,
    ) -> Result<Option<String>> {
        self.ensure_terminal_for_path_with_initial_size_and_seed(path, initial_size, true)
    }

    fn ensure_terminal_for_path_for_startup_prewarm(
        &mut self,
        path: &str,
        seed_remote_snapshot: bool,
    ) -> Result<Option<String>> {
        self.ensure_terminal_for_path_with_initial_size_and_seed(path, None, seed_remote_snapshot)
    }

    fn ensure_terminal_for_path_with_initial_size_and_seed(
        &mut self,
        path: &str,
        initial_size: Option<(u16, u16)>,
        seed_remote_snapshot: bool,
    ) -> Result<Option<String>> {
        let prepare_message = self.server.ensure_managed_cli_for_session_path(path)?;
        if path.starts_with("remote-session://")
            && let Ok(home) = crate::resolve_yggterm_home()
        {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "remote_saved_session_preflight_elided_runtime_launch",
                serde_json::json!({
                    "path": path,
                    "reason": "resume_command_owns_missing_session_failure",
                }),
            );
        }
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "before_request_terminal_launch",
                serde_json::json!({
                    "path": path,
                    "prepare_message": prepare_message,
                }),
            );
        }
        if seed_remote_snapshot {
            self.server.request_terminal_launch_for_path(path);
        } else {
            self.server
                .request_terminal_launch_for_path_preserving_active(path);
        }
        let runtime_path = self.terminal_runtime_key_for_path(path);
        if self
            .preserved_owner_status_for_runtime_key(&runtime_path)
            .is_some()
        {
            return Ok(Some(format!(
                "using hot-update preserved terminal owner for {runtime_path}"
            )));
        }
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "after_request_terminal_launch",
                serde_json::json!({
                    "path": path,
                }),
            );
        }
        let Some((stored_launch_command, cwd)) = self.server.terminal_spec(path) else {
            bail!("no terminal spec for session: {path}");
        };
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "terminal_spec_resolved",
                serde_json::json!({
                    "path": path,
                    "cwd": cwd,
                    "stored_launch_command": stored_launch_command,
                }),
            );
        }
        // The server-owned session model updates the managed session launch command
        // before terminal ensure runs. Recomputing a legacy direct-attach command here
        // can silently bypass the daemon-owned remote runtime path on startup restore.
        let launch_command = terminal_launch_command_for_path(
            path,
            stored_launch_command,
            self.server
                .remote_direct_attach_launch_command_for_path(path),
        );
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "server",
                "terminal_spec",
                "resolved",
                serde_json::json!({
                    "path": path,
                    "cwd": cwd,
                    "launch_command": launch_command,
                }),
            );
        }
        let seed_prefill =
            if terminal_ensure_should_seed_remote_snapshot(path, seed_remote_snapshot) {
                match self.server.remote_resume_seed_snapshot_for_path(path) {
                    Ok(Some(prefill)) => {
                        if let Ok(home) = crate::resolve_yggterm_home() {
                            append_trace_event(
                                &home,
                                "daemon",
                                "terminal_ensure",
                                "remote_resume_seed_snapshot",
                                serde_json::json!({
                                    "path": path,
                                    "bytes": prefill.len(),
                                }),
                            );
                        }
                        Some(prefill)
                    }
                    Ok(None) => self.server.remote_resume_seed_fallback_for_path(path),
                    Err(error) => {
                        if let Ok(home) = crate::resolve_yggterm_home() {
                            append_trace_event(
                                &home,
                                "daemon",
                                "terminal_ensure",
                                "remote_resume_seed_snapshot_error",
                                serde_json::json!({
                                    "path": path,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        self.server.remote_resume_seed_fallback_for_path(path)
                    }
                }
            } else {
                if path.starts_with("remote-session://")
                    && let Ok(home) = crate::resolve_yggterm_home()
                {
                    append_trace_event(
                        &home,
                        "daemon",
                        "terminal_ensure",
                        "remote_resume_seed_snapshot_skipped",
                        serde_json::json!({
                            "path": path,
                            "reason": "startup_prewarm_latency_budget",
                        }),
                    );
                }
                None
            };
        let fresh_remote_codex_start = self.server.session_starts_new_remote_codex(path);
        let remote_attach_startup_grace_ms = if fresh_remote_codex_start {
            REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS
        } else {
            REMOTE_ATTACH_STARTUP_GRACE_MS
        };
        if self.terminals.has_session(&runtime_path) {
            let still_running = self.terminals.session_is_running(&runtime_path);
            let has_runtime_output = self.terminals.session_has_runtime_output(&runtime_path);
            let runtime_age_ms = self
                .terminals
                .session_runtime_age_ms(&runtime_path)
                .unwrap_or(0);
            let runtime_snapshot = if path.starts_with("remote-session://") && has_runtime_output {
                self.terminals.session_snapshot(&runtime_path)
            } else {
                None
            };
            let runtime_saved_session_mismatch = path
                .strip_prefix("remote-session://")
                .and_then(|_| {
                    runtime_snapshot.as_deref().map(|snapshot| {
                        self.server
                            .remote_resume_runtime_output_mismatches_path(path, snapshot.as_bytes())
                    })
                })
                .unwrap_or(false);
            let remote_runtime_output_requires_restart = has_runtime_output
                && self
                    .terminals
                    .read(&runtime_path, 0)
                    .map(|stream| {
                        let snapshot = stream
                            .chunks
                            .into_iter()
                            .map(|chunk| chunk.data)
                            .collect::<String>();
                        remote_resume_runtime_output_requires_restart(snapshot.as_bytes())
                    })
                    .unwrap_or(false);
            let stale_remote_attach = path.starts_with("remote-session://")
                && remote_resume_stale_attach(
                    has_runtime_output,
                    runtime_age_ms,
                    remote_runtime_output_requires_restart,
                    remote_attach_startup_grace_ms,
                );
            let blank_remote_attach = path.starts_with("remote-session://")
                && !has_runtime_output
                && runtime_age_ms >= remote_attach_startup_grace_ms;
            let spec_matches =
                self.terminals
                    .session_matches_spec(&runtime_path, &launch_command, cwd.as_deref());
            let needs_restart =
                !still_running || stale_remote_attach || blank_remote_attach || !spec_matches;
            if let Ok(home) = crate::resolve_yggterm_home() {
                append_trace_event(
                    &home,
                    "server",
                    "terminal_spec",
                    "reuse_check",
                    serde_json::json!({
                        "path": path,
                        "still_running": still_running,
                        "has_runtime_output": has_runtime_output,
                        "runtime_age_ms": runtime_age_ms,
                        "remote_runtime_output_requires_restart": remote_runtime_output_requires_restart,
                        "stale_remote_attach": stale_remote_attach,
                        "blank_remote_attach": blank_remote_attach,
                        "fresh_remote_codex_start": fresh_remote_codex_start,
                        "remote_attach_startup_grace_ms": remote_attach_startup_grace_ms,
                        "runtime_saved_session_mismatch": runtime_saved_session_mismatch,
                        "spec_matches": spec_matches,
                        "remote_resume_path": path.starts_with("remote-session://"),
                        "needs_restart": needs_restart,
                    }),
                );
            }
            if !needs_restart {
                let remote_reuse_missing_retained_scrollback = path
                    .starts_with("remote-session://")
                    && has_runtime_output
                    && seed_prefill
                        .as_deref()
                        .is_some_and(terminal_data_has_scrollback_text)
                    && !self
                        .terminals
                        .session_initial_read_has_scrollback(&runtime_path);
                if (!has_runtime_output || remote_reuse_missing_retained_scrollback)
                    && let Some(prefill) = seed_prefill.as_deref()
                {
                    if let Ok(home) = crate::resolve_yggterm_home() {
                        append_trace_event(
                            &home,
                            "daemon",
                            "terminal_ensure",
                            "reuse_session_seed_prefill",
                            serde_json::json!({
                                "path": path,
                                "bytes": prefill.len(),
                                "reason": if remote_reuse_missing_retained_scrollback {
                                    "remote_reuse_missing_retained_scrollback"
                                } else {
                                    "no_runtime_output"
                                },
                            }),
                        );
                    }
                    self.terminals.seed_session(&runtime_path, prefill)?;
                }
                return Ok(prepare_message);
            }
            if needs_restart {
                let stop_command = self.server.terminal_stop_command(path);
                if let Ok(home) = crate::resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "daemon",
                        "terminal_ensure",
                        "restart_session_begin",
                        serde_json::json!({
                            "path": path,
                            "cwd": cwd,
                            "launch_command": launch_command,
                            "stop_command": stop_command,
                            "initial_cols": initial_size.map(|(cols, _)| cols),
                            "initial_rows": initial_size.map(|(_, rows)| rows),
                        }),
                    );
                }
                self.terminals.restart_session_with_size(
                    &runtime_path,
                    &launch_command,
                    cwd.as_deref(),
                    stop_command.as_deref(),
                    initial_size,
                )?;
                if let Ok(home) = crate::resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "daemon",
                        "terminal_ensure",
                        "restart_session_end",
                        serde_json::json!({
                            "path": path,
                        }),
                    );
                }
                if let Some(prefill) = seed_prefill.as_deref() {
                    self.terminals.seed_session(&runtime_path, prefill)?;
                }
            }
            return Ok(prepare_message);
        }
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "ensure_session_begin",
                serde_json::json!({
                    "path": path,
                    "cwd": cwd,
                    "launch_command": launch_command,
                    "initial_cols": initial_size.map(|(cols, _)| cols),
                    "initial_rows": initial_size.map(|(_, rows)| rows),
                }),
            );
        }
        self.terminals.ensure_session_with_size(
            &runtime_path,
            &launch_command,
            cwd.as_deref(),
            initial_size,
        )?;
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "ensure_session_end",
                serde_json::json!({
                    "path": path,
                }),
            );
        }
        if let Some(prefill) = seed_prefill.as_deref() {
            self.terminals.seed_session(&runtime_path, prefill)?;
        }
        Ok(prepare_message)
    }

    fn ensure_terminal_for_active(&mut self) -> Result<()> {
        let Some(path) = self.server.active_session_path().map(ToOwned::to_owned) else {
            bail!("no active session");
        };
        let _ = self.ensure_terminal_for_path(&path)?;
        Ok(())
    }

    fn persist(&mut self) -> Result<()> {
        self.refresh_live_codex_runtime_identities_for_persistence();
        write_persisted_state(&self.state_path, &self.server.persisted_state())
    }

    fn persisted_state_for_update_restart(&mut self) -> PersistedDaemonState {
        self.refresh_live_codex_runtime_identities_for_persistence();
        self.server.persisted_state_for_update_restart()
    }

    fn handle_request(&mut self, request: ServerRequest) -> Result<ServerResponse> {
        let request_name = server_request_name(&request);
        let trace_request = daemon_request_trace_enabled(request_name);
        if trace_request {
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "request",
                "begin",
                serde_json::json!({ "request": request_name }),
            );
        }
        let response = match request {
            ServerRequest::Ping => ServerResponse::Pong,
            ServerRequest::Status => ServerResponse::Status(self.status()),
            ServerRequest::Snapshot => self.snapshot_response(None),
            ServerRequest::PrepareUpdateRestart => {
                let state = self.persisted_state_for_update_restart();
                write_persisted_state(&self.state_path, &state)?;
                ServerResponse::Ack {
                    message: Some("update restart prepared".to_string()),
                }
            }
            ServerRequest::PrepareClientClose => {
                let force_after =
                    std::time::Duration::from_secs(CLIENT_CLOSE_FORCE_SHUTDOWN_AFTER_SECS);
                let paths = self.server.non_keep_alive_live_session_paths();
                let mut metadata_removed = 0usize;
                let mut terminal_shutdowns = 0usize;
                let mut remote_shutdowns = 0usize;
                let mut errors = Vec::<String>::new();
                for path in paths {
                    let remote_target = self.server.remote_shutdown_target_for_path(&path);
                    let stop_command = self.server.terminal_stop_command(&path);
                    let runtime_path = self.server.terminal_runtime_key_for_path(&path);
                    if let Some((machine, session_id)) = remote_target {
                        match request_remote_codex_session_shutdown(
                            &machine,
                            &session_id,
                            force_after,
                        ) {
                            Ok(()) => remote_shutdowns += 1,
                            Err(error) => errors.push(format!("{path}: {error}")),
                        }
                    }
                    match self.terminals.remove_session_gracefully_with_force_after(
                        &runtime_path,
                        stop_command.as_deref(),
                        force_after,
                    ) {
                        Ok(true) => terminal_shutdowns += 1,
                        Ok(false) => {}
                        Err(error) => errors.push(format!("{path}: {error}")),
                    }
                    match self.server.remove_live_session(&path) {
                        Ok(true) => metadata_removed += 1,
                        Ok(false) => {}
                        Err(error) => errors.push(format!("{path}: {error}")),
                    }
                    self.remove_preserved_owner_runtime(&runtime_path, "client_close_non_keep");
                }
                self.persist()?;
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "lifecycle",
                    "client_close_prepared",
                    serde_json::json!({
                        "metadata_removed": metadata_removed,
                        "terminal_shutdowns": terminal_shutdowns,
                        "remote_shutdowns": remote_shutdowns,
                        "force_after_seconds": CLIENT_CLOSE_FORCE_SHUTDOWN_AFTER_SECS,
                        "errors": errors,
                    }),
                );
                let message = if metadata_removed == 0 {
                    "no non-keep-alive live sessions to close".to_string()
                } else {
                    format!(
                        "closing {metadata_removed} non-keep-alive live session(s) gracefully; force cleanup is scheduled after 1 hour"
                    )
                };
                ServerResponse::Ack {
                    message: Some(message),
                }
            }
            ServerRequest::HotRestart {
                daemon_executable,
                expected_version,
                expected_build_id,
                reason,
            } => {
                let runtime_status = self.status();
                let owned_terminal_session_keys = self.terminals.session_keys();
                let terminal_session_key_set = runtime_status
                    .terminal_session_keys
                    .iter()
                    .cloned()
                    .collect::<HashSet<_>>();
                if hot_restart_should_defer_for_session_survival(&owned_terminal_session_keys) {
                    if let Some(duplicate_owner) = hot_restart_duplicate_runtime_owner_status(
                        self.store.home_dir(),
                        expected_version.as_deref(),
                        &owned_terminal_session_keys,
                        std::process::id(),
                    ) {
                        let mut registry =
                            PreservedTerminalOwnerRegistry::load(self.store.home_dir());
                        let mut removed_runtime_keys = Vec::new();
                        for runtime_key in &owned_terminal_session_keys {
                            if registry.remove_key(runtime_key) {
                                removed_runtime_keys.push(runtime_key.clone());
                            }
                        }
                        if !removed_runtime_keys.is_empty() {
                            registry.save(self.store.home_dir())?;
                        }
                        append_trace_event(
                            self.store.home_dir(),
                            "daemon",
                            "lifecycle",
                            "hot_restart_duplicate_runtime_owner_retired",
                            serde_json::json!({
                                "daemon_executable": daemon_executable,
                                "expected_version": expected_version,
                                "expected_build_id": expected_build_id,
                                "reason": reason,
                                "server_version": SERVER_PROTOCOL_VERSION,
                                "server_build_id": current_build_id(),
                                "pid": std::process::id(),
                                "owned_terminal_session_count": owned_terminal_session_keys.len(),
                                "owned_terminal_session_keys": &owned_terminal_session_keys,
                                "duplicate_owner_pid": duplicate_owner.server_pid,
                                "duplicate_owner_version": duplicate_owner.server_version,
                                "duplicate_owner_build_id": duplicate_owner.server_build_id,
                                "duplicate_owner_terminal_session_keys": &duplicate_owner.terminal_session_keys,
                                "removed_preserved_owner_keys": removed_runtime_keys,
                                "update_priority": "duplicate_runtime_retire",
                            }),
                        );
                        return Ok(ServerResponse::Ack {
                            message: Some(format!(
                                "retiring duplicate stale daemon; {} runtime(s) are already owned by {} pid {}",
                                owned_terminal_session_keys.len(),
                                duplicate_owner.server_version,
                                duplicate_owner.server_pid,
                            )),
                        });
                    }
                    if expected_version
                        .as_deref()
                        .is_none_or(|version| version == SERVER_PROTOCOL_VERSION)
                    {
                        return Ok(ServerResponse::Error {
                            message: "hot update handoff requires a different target daemon version when live terminal runtimes are present".to_string(),
                        });
                    }
                    let daemon_executable = canonical_hot_restart_executable(&daemon_executable)?;
                    let state = self
                        .server
                        .persisted_state_for_update_restart_with_runtime_keys(
                            &terminal_session_key_set,
                        );
                    write_persisted_state(&self.state_path, &state)?;
                    let owner_endpoint = default_endpoint(self.store.home_dir());
                    let represented_preserved_owner_keys = runtime_status
                        .preserved_terminal_owner_keys
                        .iter()
                        .cloned()
                        .collect::<HashSet<_>>();
                    let existing_entries = self
                        .preserved_terminal_owners
                        .entries
                        .iter()
                        .filter(|entry| {
                            represented_preserved_owner_keys.contains(&entry.runtime_key)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let registry = PreservedTerminalOwnerRegistry::write_handoff(
                        self.store.home_dir(),
                        &owner_endpoint,
                        &runtime_status,
                        expected_version.clone(),
                        owned_terminal_session_keys.clone(),
                        existing_entries,
                    )?;
                    let spawn_result =
                        spawn_hot_restart_daemon_process(&daemon_executable, &owner_endpoint);
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "lifecycle",
                        "hot_update_handoff_prepared",
                        serde_json::json!({
                            "daemon_executable": daemon_executable.display().to_string(),
                            "expected_version": expected_version,
                            "expected_build_id": expected_build_id,
                            "reason": reason,
                            "server_version": SERVER_PROTOCOL_VERSION,
                            "server_build_id": current_build_id(),
                            "pid": std::process::id(),
                            "owner_endpoint": format!("{owner_endpoint:?}"),
                            "owned_terminal_session_count": owned_terminal_session_keys.len(),
                            "owned_terminal_session_keys": &owned_terminal_session_keys,
                            "terminal_session_count": runtime_status.terminal_session_count,
                            "terminal_session_keys": &runtime_status.terminal_session_keys,
                            "restored_live_sessions": runtime_status.restored_live_sessions,
                            "managed_session_count": runtime_status.managed_session_count,
                            "handoff_runtime_keys": registry.keys(),
                            "spawn_ok": spawn_result.is_ok(),
                            "spawn_error": spawn_result.as_ref().err().map(|error| error.to_string()),
                            "update_priority": "handoff_preserve_sessions",
                        }),
                    );
                    spawn_result?;
                    return Ok(ServerResponse::HotUpdateHandoff {
                        message: Some(format!(
                            "hot update handoff started: preserving {} live terminal runtime(s) on {}",
                            registry.entries.len(),
                            owner_endpoint_label(&owner_endpoint),
                        )),
                        owner_endpoint: owner_endpoint_label(&owner_endpoint),
                        owner_server_version: runtime_status.server_version,
                        owner_server_pid: runtime_status.server_pid,
                        target_server_version: expected_version,
                        runtime_keys: registry.keys(),
                    });
                }
                let daemon_executable = canonical_hot_restart_executable(&daemon_executable)?;
                let state = self
                    .server
                    .persisted_state_for_update_restart_with_runtime_keys(
                        &terminal_session_key_set,
                    );
                write_persisted_state(&self.state_path, &state)?;
                let represented_preserved_owner_keys = runtime_status
                    .preserved_terminal_owner_keys
                    .iter()
                    .cloned()
                    .collect::<HashSet<_>>();
                let removed_preserved_owner_keys = self
                    .preserved_terminal_owners
                    .retain_represented_keys(|key| represented_preserved_owner_keys.contains(key));
                if !removed_preserved_owner_keys.is_empty() {
                    let _ = self.preserved_terminal_owners.save(self.store.home_dir());
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_registry_pruned",
                        serde_json::json!({
                            "reason": "hot_restart_prepare",
                            "removed_runtime_keys": removed_preserved_owner_keys,
                            "remaining_runtime_keys": self.preserved_terminal_owners.keys(),
                        }),
                    );
                }
                let preserved_owner_registry_retargeted = self
                    .preserved_terminal_owners
                    .retarget_expected_server_version(
                        self.store.home_dir(),
                        expected_version.clone(),
                    )?;
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "lifecycle",
                    "hot_restart_prepared",
                    serde_json::json!({
                        "daemon_executable": daemon_executable.display().to_string(),
                        "expected_version": expected_version,
                        "expected_build_id": expected_build_id,
                        "reason": reason,
                        "server_version": SERVER_PROTOCOL_VERSION,
                        "server_build_id": current_build_id(),
                        "pid": std::process::id(),
                        "owned_terminal_session_count": runtime_status.owned_terminal_session_count,
                        "owned_terminal_session_keys": &runtime_status.owned_terminal_session_keys,
                        "terminal_session_count": runtime_status.terminal_session_count,
                        "terminal_session_keys": &runtime_status.terminal_session_keys,
                        "preserved_terminal_owner_count": runtime_status.preserved_terminal_owner_count,
                        "preserved_terminal_owner_keys": &runtime_status.preserved_terminal_owner_keys,
                        "preserved_owner_registry_retargeted": preserved_owner_registry_retargeted,
                    }),
                );
                ServerResponse::Ack {
                    message: Some(format!(
                        "hot restart prepared: {}",
                        daemon_executable.display()
                    )),
                }
            }
            ServerRequest::OpenStoredSession {
                session_kind,
                path,
                session_id,
                cwd,
                title_hint,
                view_mode,
            } => {
                let document = if session_kind == SessionKind::Document {
                    self.store.load_document(&path)?
                } else {
                    None
                };
                self.server.open_or_focus_session(
                    session_kind,
                    &path,
                    session_id.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                    document.as_ref(),
                );
                let mut opened_in_terminal = false;
                match view_mode.unwrap_or(WorkspaceViewMode::Rendered) {
                    WorkspaceViewMode::Rendered => {
                        self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    }
                    WorkspaceViewMode::Terminal => {
                        if self.server.active_session_supports_terminal() {
                            self.server.set_view_mode(WorkspaceViewMode::Terminal);
                            self.ensure_terminal_for_active()?;
                            opened_in_terminal = true;
                        } else {
                            self.server.set_view_mode(WorkspaceViewMode::Rendered);
                        }
                    }
                }
                self.persist()?;
                self.snapshot_response(Some(if opened_in_terminal {
                    format!("opened {path}")
                } else if session_kind == SessionKind::Document
                    && view_mode == Some(WorkspaceViewMode::Terminal)
                {
                    format!("opened {path} in preview")
                } else {
                    format!("opened {path}")
                }))
            }
            ServerRequest::ConnectSsh { target_ix } => {
                let (key, reused) = self.server.connect_ssh_target(target_ix);
                if key.is_some() && self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(key.map(|key| {
                    if reused {
                        format!("focused existing {key}")
                    } else {
                        format!("connected {key}")
                    }
                }))
            }
            ServerRequest::ConnectSshCustom { target, prefix } => {
                let (key, reused) = self.server.connect_ssh_custom(&target, prefix.as_deref())?;
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(if reused {
                    format!("focused existing {key}")
                } else {
                    format!("connected {key}")
                }))
            }
            ServerRequest::StartSshSession {
                target,
                prefix,
                cwd,
                title_hint,
                terminal_appearance,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref());
                let key = self.server.start_ssh_session(
                    &target,
                    prefix.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                )?;
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::StartRemoteCodexSession {
                target,
                prefix,
                cwd,
                title_hint,
                terminal_appearance,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref());
                let key = self.server.start_remote_codex_session(
                    &target,
                    prefix.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                )?;
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::OpenRemoteSession {
                machine_key,
                session_id,
                cwd,
                title_hint,
                view_mode,
            } => {
                let key = self.server.open_remote_scanned_session_with_view(
                    &machine_key,
                    &session_id,
                    cwd.as_deref(),
                    title_hint.as_deref(),
                    view_mode,
                )?;
                let mut opened_in_terminal = false;
                if let Some(mode) = view_mode {
                    if mode == WorkspaceViewMode::Terminal
                        && !self.server.active_session_supports_terminal()
                    {
                        self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    } else {
                        self.server.set_view_mode(mode);
                    }
                    if mode == WorkspaceViewMode::Terminal
                        && self.server.active_session_supports_terminal()
                    {
                        self.ensure_terminal_for_active()?;
                        opened_in_terminal = true;
                    }
                }
                self.persist()?;
                self.snapshot_response(Some(
                    if view_mode == Some(WorkspaceViewMode::Terminal) && !opened_in_terminal {
                        format!("opened {key} in preview")
                    } else {
                        format!("opened {key}")
                    },
                ))
            }
            ServerRequest::RefreshRemoteMachine { machine_key } => {
                self.server.refresh_remote_machine_by_key(&machine_key)?;
                self.persist()?;
                self.snapshot_response(Some(format!("refreshed {machine_key}")))
            }
            ServerRequest::RefreshManagedCli {
                machine_key,
                background,
            } => {
                let message = if background {
                    self.server
                        .queue_background_managed_cli_refresh(machine_key.as_deref())?
                } else {
                    self.server
                        .refresh_managed_cli(machine_key.as_deref(), background)?
                };
                ServerResponse::Ack {
                    message: Some(message),
                }
            }
            ServerRequest::RefreshPreview { path } => {
                self.server.refresh_session_preview_from_source(&path)?;
                self.persist()?;
                self.snapshot_response(Some(format!("refreshed preview {path}")))
            }
            ServerRequest::UpdateSessionCopy {
                path,
                title,
                precis,
                summary,
            } => {
                let remote_copy_target = self.server.remote_copy_target_for_session_path(&path);
                if let Some(title) = title.as_deref() {
                    self.server.set_session_title_hint(&path, title);
                }
                if let Some(precis) = precis.as_deref() {
                    self.server.set_session_precis_hint(&path, precis);
                }
                if let Some(summary) = summary.as_deref() {
                    self.server.set_session_summary_hint(&path, summary);
                }
                self.persist()?;
                if let Some((machine, session_id, cwd)) = remote_copy_target {
                    spawn_remote_generated_copy_persist(
                        machine, session_id, cwd, title, precis, summary, "manual",
                    );
                }
                ServerResponse::Ack { message: None }
            }
            ServerRequest::RemoveSshTarget { machine_key } => {
                let removed = self.server.remove_ssh_targets_for_machine(&machine_key);
                self.persist()?;
                self.snapshot_response(Some(if removed == 0 {
                    format!("no saved ssh target for {machine_key}")
                } else if removed == 1 {
                    format!("removed saved ssh target for {machine_key}")
                } else {
                    format!("removed {removed} saved ssh targets for {machine_key}")
                }))
            }
            ServerRequest::RemoveSession { path } => {
                let remote_target = self.server.remote_shutdown_target_for_path(&path);
                if let Some((machine, session_id)) = remote_target.as_ref() {
                    terminate_remote_codex_session(machine, session_id).with_context(|| {
                        format!(
                            "failed to terminate remote session {} on {} before removal",
                            session_id, machine.ssh_target
                        )
                    })?;
                }
                let stop_command = self.server.terminal_stop_command(&path);
                let runtime_path = self.server.terminal_runtime_key_for_path(&path);
                let removed_terminal = self
                    .terminals
                    .remove_session(&runtime_path, stop_command.as_deref())?;
                let removed_session = self.server.remove_live_session(&path)?;
                if removed_session {
                    self.remove_preserved_owner_runtime(&runtime_path, "live_session_removed");
                }
                self.prune_unrepresented_preserved_owners("live_session_removed");
                self.persist()?;
                self.snapshot_response(Some(if removed_session {
                    if removed_terminal {
                        format!("closed terminal runtime for {path}")
                    } else {
                        format!("closed terminal metadata for {path}")
                    }
                } else {
                    format!("no live session for {path}")
                }))
            }
            ServerRequest::SetSessionKeepAlive { path, keep_alive } => {
                if keep_alive {
                    let runtime_path = self.server.terminal_runtime_key_for_path(&path);
                    let runtime_is_available = self.terminals.has_session(&runtime_path)
                        || self
                            .preserved_owner_for_runtime_key(&runtime_path)
                            .is_some();
                    if !runtime_is_available {
                        self.persist()?;
                        return Ok(self.snapshot_response(Some(format!(
                            "cannot keep {path} alive without terminal runtime {runtime_path}"
                        ))));
                    }
                }
                let updated = self.server.set_live_session_keep_alive(&path, keep_alive)?;
                self.prune_unrepresented_preserved_owners("keep_alive_updated");
                self.persist()?;
                self.snapshot_response(Some(if updated {
                    if keep_alive {
                        format!("kept {path} alive")
                    } else {
                        format!("stopped keeping {path} alive")
                    }
                } else {
                    format!("no live session for {path}")
                }))
            }
            ServerRequest::StartLocalSession {
                session_kind,
                cwd,
                title_hint,
                terminal_appearance,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref());
                let key = self.server.start_local_session(
                    session_kind,
                    cwd.as_deref(),
                    title_hint.as_deref(),
                );
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::SwitchAgentSessionMode { path, session_kind } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if self
                    .terminals
                    .recent_activity(&runtime_path, std::time::Duration::from_secs(4))
                {
                    bail!("session is still active; wait for it to settle before switching modes");
                }
                let stop_command = self.server.terminal_stop_command(&path);
                self.server.switch_agent_session_mode(&path, session_kind)?;
                if let Some((launch_command, cwd)) = self.server.terminal_spec(&path) {
                    if self.terminals.has_session(&runtime_path) {
                        self.terminals.restart_session(
                            &runtime_path,
                            &launch_command,
                            cwd.as_deref(),
                            stop_command.as_deref(),
                        )?;
                    }
                }
                self.persist()?;
                self.snapshot_response(Some(format!(
                    "switched to {}",
                    match session_kind {
                        SessionKind::Codex => "codex",
                        SessionKind::CodexLiteLlm => "codex-litellm",
                        SessionKind::Shell => "shell",
                        SessionKind::SshShell => "ssh",
                        SessionKind::Document => "document",
                    }
                )))
            }
            ServerRequest::StartCommandSession {
                cwd,
                title_hint,
                launch_command,
                source_label,
                terminal_appearance,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref());
                let key = self.server.start_command_session(
                    cwd.as_deref(),
                    title_hint.as_deref(),
                    &launch_command,
                    source_label.as_deref(),
                );
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::EnsureRemoteRuntimeCodexSession {
                session_id,
                cwd,
                require_existing,
                terminal_appearance,
                initial_cols,
                initial_rows,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref());
                let key = self.server.ensure_remote_runtime_codex_session(
                    &session_id,
                    cwd.as_deref(),
                    require_existing,
                )?;
                self.adopt_legacy_local_codex_runtime(&session_id, &key);
                let _ = self.ensure_terminal_for_path_with_initial_size(
                    &key,
                    valid_initial_terminal_size(initial_cols, initial_rows),
                )?;
                self.persist()?;
                ServerResponse::Ack { message: Some(key) }
            }
            ServerRequest::StartRemoteRuntimeCodexSession {
                session_id,
                cwd,
                terminal_appearance,
                initial_cols,
                initial_rows,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref());
                let key = self
                    .server
                    .start_remote_runtime_codex_session(&session_id, cwd.as_deref())?;
                self.adopt_legacy_local_codex_runtime(&session_id, &key);
                let _ = self.ensure_terminal_for_path_with_initial_size(
                    &key,
                    valid_initial_terminal_size(initial_cols, initial_rows),
                )?;
                self.persist()?;
                ServerResponse::Ack { message: Some(key) }
            }
            ServerRequest::FocusLive { key, view_mode } => {
                self.server.focus_live_session(&key);
                let mut focused_in_terminal = false;
                if let Some(mode) = view_mode {
                    if mode == WorkspaceViewMode::Terminal
                        && !self.server.active_session_supports_terminal()
                    {
                        self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    } else {
                        self.server.set_view_mode(mode);
                    }
                    if mode == WorkspaceViewMode::Terminal
                        && self.server.active_session_supports_terminal()
                    {
                        self.ensure_terminal_for_active()?;
                        focused_in_terminal = true;
                    }
                }
                self.persist()?;
                self.snapshot_response(Some(
                    if view_mode == Some(WorkspaceViewMode::Terminal) && !focused_in_terminal {
                        format!("focused {key} in preview")
                    } else {
                        format!("focused {key}")
                    },
                ))
            }
            ServerRequest::SetViewMode { mode } => {
                let mut effective_mode = mode;
                if mode == WorkspaceViewMode::Terminal
                    && !self.server.active_session_supports_terminal()
                {
                    self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    effective_mode = WorkspaceViewMode::Rendered;
                } else {
                    self.server.set_view_mode(mode);
                }
                if mode == WorkspaceViewMode::Terminal
                    && self.server.active_session_supports_terminal()
                {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(match effective_mode {
                    WorkspaceViewMode::Rendered => "preview mode".to_string(),
                    WorkspaceViewMode::Terminal => "terminal mode".to_string(),
                }))
            }
            ServerRequest::TogglePreviewBlock { block_ix } => {
                self.server.toggle_preview_block(block_ix);
                self.persist()?;
                self.snapshot_response(Some(format!("preview block {}", block_ix + 1)))
            }
            ServerRequest::SetAllPreviewBlocksFolded { folded } => {
                self.server.set_all_preview_blocks_folded(folded);
                self.persist()?;
                self.snapshot_response(Some(if folded {
                    "collapsed preview".to_string()
                } else {
                    "expanded preview".to_string()
                }))
            }
            ServerRequest::RequestTerminalLaunch => {
                self.ensure_terminal_for_active()?;
                self.server.set_view_mode(WorkspaceViewMode::Terminal);
                self.persist()?;
                self.snapshot_response(Some("requested terminal".to_string()))
            }
            ServerRequest::RequestTerminalLaunchForPath { path } => {
                self.server.request_terminal_launch_for_path(&path);
                self.ensure_terminal_for_active()?;
                self.server.set_view_mode(WorkspaceViewMode::Terminal);
                self.persist()?;
                self.snapshot_response(Some(format!("requested terminal for {path}")))
            }
            ServerRequest::TerminalEnsure { path } => {
                let message = self.ensure_terminal_for_path(&path)?;
                ServerResponse::Ack { message }
            }
            ServerRequest::TerminalRead { path, cursor } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_read(&owner_endpoint, &runtime_path, cursor) {
                        Ok((
                            cursor,
                            chunks,
                            running,
                            runtime_output_seen,
                            eof_without_output,
                            post_resize_output_seen,
                            last_resize_seq,
                        )) => {
                            return Ok(ServerResponse::TerminalStream {
                                cursor,
                                chunks,
                                running,
                                runtime_output_seen,
                                eof_without_output,
                                post_resize_output_seen,
                                last_resize_seq,
                            });
                        }
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            return Err(error);
                        }
                    }
                }
                if uses_runtime_owned_terminal_path(&path)
                    && self.terminals.session_hit_eof_without_output(&runtime_path)
                    && !self.terminals.session_has_runtime_output(&runtime_path)
                    && let Some((stored_launch_command, cwd)) = self.server.terminal_spec(&path)
                {
                    // Runtime-owned terminals must stay on their stored daemon-owned launch path.
                    // Falling back to a legacy direct-attach command here can silently switch
                    // the recovery path back to the old semantics after startup.
                    let launch_command = stored_launch_command;
                    let stop_command = self.server.terminal_stop_command(&path);
                    if let Ok(home) = crate::resolve_yggterm_home() {
                        append_trace_event(
                            &home,
                            "server",
                            "terminal_runtime",
                            "restart_before_read",
                            serde_json::json!({
                                "path": path,
                                "runtime_path": runtime_path,
                                "reason": "eof_without_output",
                            }),
                        );
                    }
                    self.terminals.restart_session(
                        &runtime_path,
                        &launch_command,
                        cwd.as_deref(),
                        stop_command.as_deref(),
                    )?;
                }
                let stream = self.terminals.read(&runtime_path, cursor)?;
                ServerResponse::TerminalStream {
                    cursor: stream.cursor,
                    chunks: stream
                        .chunks
                        .into_iter()
                        .map(|chunk| TerminalStreamChunk {
                            seq: chunk.seq,
                            data: chunk.data,
                        })
                        .collect(),
                    running: stream.running,
                    runtime_output_seen: stream.runtime_output_seen,
                    eof_without_output: stream.eof_without_output,
                    post_resize_output_seen: stream.post_resize_output_seen,
                    last_resize_seq: stream.last_resize_seq,
                }
            }
            ServerRequest::TerminalSnapshot { path } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_snapshot(&owner_endpoint, &runtime_path) {
                        Ok((
                            text,
                            running,
                            runtime_output_seen,
                            post_resize_output_seen,
                            last_resize_seq,
                        )) => {
                            return Ok(ServerResponse::TerminalSnapshot {
                                text,
                                running,
                                runtime_output_seen,
                                post_resize_output_seen,
                                last_resize_seq,
                            });
                        }
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            return Err(error);
                        }
                    }
                }
                let text = self
                    .terminals
                    .session_screen_snapshot(&runtime_path)
                    .unwrap_or_default();
                ServerResponse::TerminalSnapshot {
                    text,
                    running: self.terminals.session_is_running(&runtime_path),
                    runtime_output_seen: self.terminals.session_has_runtime_output(&runtime_path),
                    post_resize_output_seen: self
                        .terminals
                        .session_post_resize_output_seen(&runtime_path),
                    last_resize_seq: self.terminals.session_last_resize_seq(&runtime_path),
                }
            }
            ServerRequest::TerminalRetainedSnapshot { path } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_retained_snapshot(&owner_endpoint, &runtime_path) {
                        Ok((
                            text,
                            running,
                            runtime_output_seen,
                            post_resize_output_seen,
                            last_resize_seq,
                        )) => {
                            return Ok(ServerResponse::TerminalRetainedSnapshot {
                                text,
                                running,
                                runtime_output_seen,
                                post_resize_output_seen,
                                last_resize_seq,
                            });
                        }
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            return Err(error);
                        }
                    }
                }
                let text = self
                    .terminals
                    .session_snapshot(&runtime_path)
                    .unwrap_or_default();
                ServerResponse::TerminalRetainedSnapshot {
                    text,
                    running: self.terminals.session_is_running(&runtime_path),
                    runtime_output_seen: self.terminals.session_has_runtime_output(&runtime_path),
                    post_resize_output_seen: self
                        .terminals
                        .session_post_resize_output_seen(&runtime_path),
                    last_resize_seq: self.terminals.session_last_resize_seq(&runtime_path),
                }
            }
            ServerRequest::TerminalWrite { path, data } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_write(&owner_endpoint, &runtime_path, &data) {
                        Ok(_) => return Ok(ServerResponse::Ack { message: None }),
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            return Err(error);
                        }
                    }
                }
                let local_runtime_running = self.terminals.session_is_running(&runtime_path);
                let write_strategy = terminal_write_strategy_for_path(&path, local_runtime_running);
                if matches!(write_strategy, TerminalWriteStrategy::LocalRuntime) {
                    self.terminals.write(&runtime_path, &data)?;
                    return Ok(ServerResponse::Ack { message: None });
                }
                if matches!(write_strategy, TerminalWriteStrategy::RemoteDirectFallback) {
                    match self.server.remote_terminal_write_for_path(&path, &data) {
                        Ok(true) => {
                            return Ok(ServerResponse::Ack { message: None });
                        }
                        Ok(false) => {}
                        Err(error) => {
                            if let Ok(home) = resolve_yggterm_home() {
                                append_trace_event(
                                    &home,
                                    "daemon",
                                    "terminal_io",
                                    "remote_terminal_write_direct_failed",
                                    serde_json::json!({
                                        "path": path,
                                        "runtime_path": runtime_path,
                                        "error": format!("{error:#}"),
                                        "bytes": data.len(),
                                    }),
                                );
                            }
                            return Err(error);
                        }
                    }
                }
                match self.server.remote_terminal_write_for_path(&path, &data) {
                    Ok(true) => {
                        return Ok(ServerResponse::Ack { message: None });
                    }
                    Ok(false) => {}
                    Err(error) => {
                        if let Ok(home) = resolve_yggterm_home() {
                            append_trace_event(
                                &home,
                                "daemon",
                                "terminal_io",
                                "remote_terminal_write_direct_failed_fallback_local_pty",
                                serde_json::json!({
                                    "path": path,
                                    "runtime_path": runtime_path,
                                    "error": format!("{error:#}"),
                                    "bytes": data.len(),
                                }),
                            );
                        }
                    }
                }
                self.terminals.write(&runtime_path, &data)?;
                ServerResponse::Ack { message: None }
            }
            ServerRequest::TerminalResize { path, cols, rows } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_resize(&owner_endpoint, &runtime_path, cols, rows) {
                        Ok(_) => return Ok(ServerResponse::Ack { message: None }),
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            return Err(error);
                        }
                    }
                }
                self.terminals.resize(&runtime_path, cols, rows)?;
                ServerResponse::Ack { message: None }
            }
            ServerRequest::TerminalRestart {
                path,
                terminal_appearance,
                force_remote,
                initial_cols,
                initial_rows,
            } => {
                if let Some(terminal_appearance) = terminal_appearance.as_deref() {
                    sync_terminal_identity_for_request(Some(terminal_appearance));
                }
                let recovered_remote_scan = self
                    .server
                    .recover_remote_scanned_terminal_for_restart(&path)?;
                let launch_refreshed = self
                    .server
                    .refresh_terminal_identity_launch_command_for_path(&path);
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                let (launch_command, cwd) = self
                    .server
                    .terminal_spec(&path)
                    .with_context(|| format!("terminal session not found: {path}"))?;
                let stop_command = self.server.terminal_stop_command(&path);
                if force_remote {
                    if let Some((machine, session_id)) =
                        self.server.remote_shutdown_target_for_path(&path)
                    {
                        terminate_remote_codex_session(&machine, &session_id).with_context(
                            || {
                                format!(
                                    "terminating remote codex session {session_id} before restart"
                                )
                            },
                        )?;
                    }
                    if self
                        .preserved_owner_endpoint_for_request(&runtime_path)
                        .is_some()
                    {
                        self.remove_preserved_owner_runtime(&runtime_path, "terminal_restart");
                    }
                }
                let initial_size = initial_cols.zip(initial_rows);
                self.terminals.restart_session_with_size(
                    &runtime_path,
                    &launch_command,
                    cwd.as_deref(),
                    stop_command.as_deref(),
                    initial_size,
                )?;
                self.persist()?;
                if force_remote {
                    spawn_force_remote_restart_daemon_cleanup(
                        &self.store,
                        self.preserved_terminal_owners.entries.is_empty(),
                    );
                }
                self.snapshot_response(Some(format!(
                    "restarted {path}; launch_refreshed={launch_refreshed}; recovered_remote_scan={recovered_remote_scan}; force_remote={force_remote}"
                )))
            }
            ServerRequest::SyncExternalWindow => {
                let message = self.server.sync_external_terminal_window_for_active();
                self.persist()?;
                self.snapshot_response(Some(message))
            }
            ServerRequest::RaiseExternalWindow => {
                let message = self.server.raise_external_terminal_window_for_active();
                self.persist()?;
                self.snapshot_response(Some(message))
            }
            ServerRequest::SyncTheme { theme } => {
                self.server.sync_theme(theme);
                self.persist()?;
                self.snapshot_response(Some("theme synced".to_string()))
            }
            ServerRequest::SyncTerminalIdentity {
                terminal_appearance,
            } => {
                sync_terminal_identity_for_request(Some(&terminal_appearance));
                let refreshed = self.server.refresh_terminal_identity_launch_commands();
                if refreshed > 0 {
                    self.persist()?;
                }
                ServerResponse::Ack {
                    message: Some(format!(
                        "terminal identity synced; refreshed {refreshed} launch commands"
                    )),
                }
            }
            ServerRequest::Shutdown => {
                let remote_targets = self.server.remote_shutdown_targets();
                let mut remote_errors = Vec::new();
                let mut remote_stopped = 0usize;
                for (machine, session_id) in remote_targets {
                    match terminate_remote_codex_session(&machine, &session_id) {
                        Ok(()) => remote_stopped += 1,
                        Err(error) => remote_errors
                            .push(format!("{}:{}: {}", machine.machine_key, session_id, error)),
                    }
                }
                let summary = self
                    .terminals
                    .shutdown_all(|path| self.server.terminal_stop_command(path));
                self.persist()?;
                let total_errors = summary.errors.len() + remote_errors.len();
                ServerResponse::Ack {
                    message: Some(if total_errors == 0 {
                        format!(
                            "stopped {} terminal sessions and {} remote persistent sessions",
                            summary.stopped, remote_stopped
                        )
                    } else {
                        format!(
                            "stopped {} terminal sessions and {} remote persistent sessions, {} errors",
                            summary.stopped, remote_stopped, total_errors
                        )
                    }),
                }
            }
        };
        if trace_request {
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "request",
                "end",
                serde_json::json!({ "request": request_name }),
            );
        }
        Ok(response)
    }
}

fn sync_terminal_identity_for_request(terminal_appearance: Option<&str>) {
    if let Some(appearance) = terminal_appearance
        .map(str::trim)
        .filter(|appearance| !appearance.is_empty())
    {
        sync_terminal_identity_appearance(appearance);
    }
}

fn snapshot_session_requires_terminal_runtime(session: &SnapshotSessionView) -> bool {
    matches!(
        session.source,
        crate::SessionSource::LiveLocal | crate::SessionSource::LiveSsh
    ) && session.kind != SessionKind::Document
}

fn snapshot_session_is_pending_runtime_launch(session: &SnapshotSessionView) -> bool {
    snapshot_session_requires_terminal_runtime(session)
        && matches!(
            session.launch_phase,
            crate::TerminalLaunchPhase::Queued
                | crate::TerminalLaunchPhase::BridgePending
                | crate::TerminalLaunchPhase::RemoteBootstrap
        )
}

fn snapshot_session_is_keep_alive_recovery_target(session: &SnapshotSessionView) -> bool {
    snapshot_session_requires_terminal_runtime(session)
        && session
            .metadata
            .iter()
            .any(|entry| entry.label == "Runtime Persistence" && entry.value == "keep-alive")
}

fn apply_terminal_runtime_truth_to_snapshot(
    server: &YggtermServer,
    runtime_keys: &HashSet<String>,
    snapshot: &mut ServerUiSnapshot,
) {
    if let Some(active_session) = snapshot.active_session.as_mut()
        && !runtime_keys
            .contains(&server.terminal_runtime_key_for_path(&active_session.session_path))
        && snapshot_session_is_keep_alive_recovery_target(active_session)
    {
        active_session.launch_phase = crate::TerminalLaunchPhase::RemoteBootstrap;
    }
    for session in &mut snapshot.live_sessions {
        if !runtime_keys.contains(&server.terminal_runtime_key_for_path(&session.session_path))
            && snapshot_session_is_keep_alive_recovery_target(session)
        {
            session.launch_phase = crate::TerminalLaunchPhase::RemoteBootstrap;
        }
    }
    let active_path = snapshot.active_session_path.clone().or_else(|| {
        snapshot
            .active_session
            .as_ref()
            .map(|session| session.session_path.clone())
    });
    snapshot.live_sessions.retain(|session| {
        runtime_keys.contains(&server.terminal_runtime_key_for_path(&session.session_path))
            || (active_path.as_deref() == Some(session.session_path.as_str())
                && snapshot_session_is_pending_runtime_launch(session))
            || snapshot_session_is_keep_alive_recovery_target(session)
    });

    let Some(active_path) = active_path else {
        return;
    };
    let active_runtime_present =
        runtime_keys.contains(&server.terminal_runtime_key_for_path(&active_path));
    let active_requires_runtime = snapshot
        .active_session
        .as_ref()
        .is_some_and(snapshot_session_requires_terminal_runtime)
        || snapshot.active_view_mode == WorkspaceViewMode::Terminal;
    let active_pending_runtime_launch = snapshot
        .active_session
        .as_ref()
        .is_some_and(snapshot_session_is_pending_runtime_launch);
    let active_keep_alive_recovery_target = snapshot
        .active_session
        .as_ref()
        .is_some_and(snapshot_session_is_keep_alive_recovery_target);
    if !active_requires_runtime
        || active_runtime_present
        || active_pending_runtime_launch
        || active_keep_alive_recovery_target
    {
        return;
    }

    if let Some(preview) = server.remote_preview_snapshot_session_for_path(&active_path) {
        snapshot.active_session_path = Some(preview.session_path.clone());
        snapshot.active_session = Some(preview);
    } else {
        snapshot.active_session_path = None;
        snapshot.active_session = None;
    }
    snapshot.active_view_mode = WorkspaceViewMode::Rendered;
}

fn unrepresented_preserved_owner_runtime_keys(
    server: &YggtermServer,
    represented_runtime_keys: &HashSet<String>,
    owner_runtime_keys: Vec<String>,
) -> Vec<String> {
    let mut stale_runtime_keys = owner_runtime_keys
        .into_iter()
        .filter(|runtime_key| !represented_runtime_keys.contains(runtime_key))
        .filter(|runtime_key| !server.represents_terminal_runtime_key(runtime_key))
        .collect::<Vec<_>>();
    stale_runtime_keys.sort();
    stale_runtime_keys.dedup();
    stale_runtime_keys
}

fn trim_terminal_buffers(
    runtime: &Arc<Mutex<DaemonRuntime>>,
    idle_trim_after_ms: u64,
) -> Result<TerminalTrimTick> {
    let runtime = lock_daemon_runtime(runtime, "trim_terminal_buffers");
    let before = runtime.terminals.stats();
    let trim = runtime
        .terminals
        .trim_idle_buffers(std::time::Duration::from_millis(idle_trim_after_ms));
    let after = runtime.terminals.stats();
    Ok(TerminalTrimTick {
        before,
        after,
        trimmed_sessions: trim.trimmed_sessions,
        reclaimed_bytes: trim.reclaimed_bytes,
    })
}

#[derive(Debug, Clone, Default)]
struct TerminalTrimTick {
    before: TerminalBufferStats,
    after: TerminalBufferStats,
    trimmed_sessions: usize,
    reclaimed_bytes: usize,
}

fn source_updated_at_for_path(path: &Path) -> Option<OffsetDateTime> {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|ts| ts.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|dur| OffsetDateTime::from_unix_timestamp(dur.as_secs() as i64).ok())
}

fn source_updated_at_for_remote_epoch(epoch: i64) -> Option<OffsetDateTime> {
    (epoch > 0)
        .then(|| OffsetDateTime::from_unix_timestamp(epoch).ok())
        .flatten()
}

fn background_machine_key(raw_machine_key: &str) -> String {
    match raw_machine_key.trim().to_ascii_lowercase().as_str() {
        "juju" | "jujo" => "jojo".to_string(),
        value => value
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string(),
    }
}

fn collect_local_copy_candidates(node: &SessionNode, out: &mut Vec<BackgroundCopyCandidate>) {
    if node.kind == SessionNodeKind::CodexSession {
        if let (Some(session_id), Some(cwd)) = (node.session_id.as_ref(), node.cwd.as_ref()) {
            out.push(BackgroundCopyCandidate {
                session_path: node.path.to_string_lossy().to_string(),
                session_id: session_id.clone(),
                cwd: cwd.clone(),
                title: node.title.clone().unwrap_or_else(|| node.name.clone()),
                source_updated_at: source_updated_at_for_path(&node.path),
                remote_machine: None,
                generation_context: None,
                storage_path: None,
                cached_summary: None,
            });
        }
    }
    for child in &node.children {
        collect_local_copy_candidates(child, out);
    }
}

fn session_cwd_for_background_copy(session: &ManagedSessionView) -> String {
    session
        .metadata
        .iter()
        .find(|entry| entry.label == "Cwd")
        .map(|entry| entry.value.clone())
        .unwrap_or_default()
}

fn collect_live_copy_candidates(
    store: &SessionStore,
    live_sessions: &[ManagedSessionView],
    out: &mut Vec<BackgroundCopyCandidate>,
) {
    for session in live_sessions {
        if session.session_path.starts_with("remote-session://") {
            continue;
        }
        let generation_context = if session.kind == SessionKind::Document {
            store
                .load_document(&session.session_path)
                .ok()
                .flatten()
                .and_then(|document| {
                    let trimmed = document.body.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                })
        } else {
            None
        };
        if generation_context.is_none()
            && !(Path::new(&session.session_path).exists()
                && session.session_path.ends_with(".jsonl"))
        {
            continue;
        }
        out.push(BackgroundCopyCandidate {
            session_path: session.session_path.clone(),
            session_id: session.id.clone(),
            cwd: session_cwd_for_background_copy(session),
            title: session.title.clone(),
            source_updated_at: None,
            remote_machine: None,
            generation_context,
            storage_path: None,
            cached_summary: None,
        });
    }
}

fn collect_remote_copy_candidates(
    remote_machines: &[RemoteMachineSnapshot],
) -> Vec<BackgroundCopyCandidate> {
    let mut out = Vec::new();
    for machine in remote_machines {
        let machine_ref = RemoteMachineSnapshot {
            machine_key: machine.machine_key.clone(),
            label: machine.label.clone(),
            ssh_target: machine.ssh_target.clone(),
            prefix: machine.prefix.clone(),
            remote_binary_expr: machine.remote_binary_expr.clone(),
            remote_deploy_state: machine.remote_deploy_state,
            health: machine.health,
            sessions: Vec::new(),
        };
        for session in &machine.sessions {
            out.push(BackgroundCopyCandidate {
                session_path: session.session_path.clone(),
                session_id: session.session_id.clone(),
                cwd: session.cwd.clone(),
                title: session.title_hint.clone(),
                source_updated_at: source_updated_at_for_remote_epoch(session.modified_epoch),
                remote_machine: Some(machine_ref.clone()),
                generation_context: (!session.recent_context.trim().is_empty())
                    .then(|| session.recent_context.clone()),
                storage_path: (!session.storage_path.trim().is_empty())
                    .then(|| session.storage_path.clone()),
                cached_summary: session.cached_summary.clone(),
            });
        }
    }
    out
}

fn copy_target_context(
    candidate: &BackgroundCopyCandidate,
    ssh_targets: &[SshConnectTarget],
) -> Result<Option<String>> {
    if let Some(context) = candidate.generation_context.as_ref()
        && !context.trim().is_empty()
    {
        return Ok(Some(context.clone()));
    }
    let (Some(machine), Some(storage_path)) = (
        candidate.remote_machine.as_ref(),
        candidate.storage_path.as_deref(),
    ) else {
        return Ok(None);
    };
    let Some(target) = ssh_targets
        .iter()
        .find(|target| background_machine_key(&target.label) == machine.machine_key)
    else {
        return Ok(None);
    };
    fetch_remote_generation_context(target, storage_path).map(Some)
}

fn build_background_copy_updates(
    store: &SessionStore,
    settings: &AppSettings,
    local_root: &SessionNode,
    live_sessions: &[ManagedSessionView],
    remote_machines: &[RemoteMachineSnapshot],
    ssh_targets: &[SshConnectTarget],
) -> Result<Vec<BackgroundCopyUpdate>> {
    let mut candidates = Vec::new();
    collect_local_copy_candidates(local_root, &mut candidates);
    collect_live_copy_candidates(store, live_sessions, &mut candidates);
    candidates.extend(collect_remote_copy_candidates(remote_machines));

    let mut updates = Vec::new();
    let mut seen_candidates = HashSet::new();
    for candidate in candidates
        .into_iter()
        .filter(|candidate| seen_candidates.insert(candidate.session_path.clone()))
        .take(BACKGROUND_COPY_BUDGET_PER_TICK)
    {
        let stored_title = store
            .resolve_title_for_session_id(&candidate.session_id)
            .ok()
            .flatten();
        let title_missing = looks_like_generated_fallback_title(&candidate.title)
            && stored_title
                .as_deref()
                .is_none_or(looks_like_generated_fallback_title);
        let stored_summary = store
            .resolve_summary_for_session_id(&candidate.session_id)
            .ok()
            .flatten();
        let summary_needs_refresh = candidate
            .source_updated_at
            .and_then(|updated_at| {
                store
                    .summary_needs_refresh_for_session_id(&candidate.session_id, updated_at)
                    .ok()
            })
            .unwrap_or(stored_summary.is_none());
        let summary_missing = summary_needs_refresh
            && candidate
                .cached_summary
                .as_deref()
                .is_none_or(|summary| summary.trim().is_empty());
        if !title_missing && !summary_missing {
            continue;
        }

        let maybe_context = copy_target_context(&candidate, ssh_targets)?;
        let (title, summary) = if let Some(context) = maybe_context {
            let title = if title_missing {
                store.generate_title_for_context(
                    settings,
                    &candidate.session_id,
                    &candidate.cwd,
                    &context,
                    false,
                )?
            } else {
                None
            };
            let summary = if summary_missing {
                store.generate_summary_for_context(
                    settings,
                    &candidate.session_id,
                    &candidate.cwd,
                    &context,
                    false,
                )?
            } else {
                None
            };
            if let Some(machine) = candidate.remote_machine.as_ref() {
                if title.is_some() || summary.is_some() {
                    persist_remote_generated_copy(
                        machine,
                        &candidate.session_id,
                        &candidate.cwd,
                        title.as_deref(),
                        None,
                        summary.as_deref(),
                        &settings.interface_llm_model,
                    )?;
                }
            }
            (title, summary)
        } else {
            let title = if title_missing {
                store.generate_title_for_session_path(settings, &candidate.session_path, false)?
            } else {
                None
            };
            let summary = if summary_missing {
                store.generate_summary_for_session_path(settings, &candidate.session_path, false)?
            } else {
                None
            };
            (title, summary)
        };

        if title.is_some() || summary.is_some() {
            updates.push(BackgroundCopyUpdate {
                session_path: candidate.session_path,
                title,
                summary,
            });
        }
    }
    Ok(updates)
}

fn run_background_copy_chore(runtime: &Arc<Mutex<DaemonRuntime>>) -> Result<usize> {
    let (store, settings, local_root, live_sessions, remote_machines, ssh_targets, perf_home) = {
        let runtime = lock_daemon_runtime(runtime, "run_background_copy_chore_read");
        let settings = runtime.store.load_settings().unwrap_or_default();
        let local_root = runtime.store.load_codex_tree(&settings)?;
        (
            runtime.store.clone(),
            settings,
            local_root,
            runtime.server.live_sessions().to_vec(),
            runtime.server.remote_machines().to_vec(),
            runtime.server.ssh_targets().to_vec(),
            runtime.store.home_dir().to_path_buf(),
        )
    };
    let perf = PerfSpan::start(&perf_home, "daemon", "background_copy_chore");
    let updates = build_background_copy_updates(
        &store,
        &settings,
        &local_root,
        &live_sessions,
        &remote_machines,
        &ssh_targets,
    )?;
    perf.finish(serde_json::json!({
        "updates": updates.len(),
        "live_sessions": live_sessions.len(),
        "remote_machines": remote_machines.len(),
    }));

    if updates.is_empty() {
        return Ok(0);
    }

    let mut runtime = lock_daemon_runtime(runtime, "run_background_copy_chore_apply");
    for update in &updates {
        if let Some(title) = update.title.as_deref() {
            runtime
                .server
                .set_session_title_hint(&update.session_path, title);
        }
        if let Some(summary) = update.summary.as_deref() {
            runtime
                .server
                .set_session_summary_hint(&update.session_path, summary);
        }
    }
    runtime.persist()?;
    append_trace_event(
        runtime.store.home_dir(),
        "daemon",
        "background_copy",
        "tick",
        serde_json::json!({ "updates": updates.len() }),
    );
    Ok(updates.len())
}

fn daemon_background_copy_chore_enabled_from_env(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn daemon_background_copy_chore_enabled() -> bool {
    daemon_background_copy_chore_enabled_from_env(
        std::env::var(ENV_YGGTERM_ENABLE_BACKGROUND_COPY_CHORE)
            .ok()
            .as_deref(),
    )
}

fn daemon_idle_shutdown_ms() -> u64 {
    std::env::var("YGGTERM_DAEMON_IDLE_SHUTDOWN_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_DAEMON_IDLE_SHUTDOWN_MS)
}

fn daemon_terminal_idle_trim_after_ms() -> u64 {
    std::env::var("YGGTERM_DAEMON_TERMINAL_IDLE_TRIM_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TERMINAL_IDLE_TRIM_AFTER_MS)
}

fn current_millis_u64() -> u64 {
    u64::try_from(current_millis()).unwrap_or(u64::MAX)
}

fn terminal_ensure_should_seed_remote_snapshot(path: &str, seed_remote_snapshot: bool) -> bool {
    path.starts_with("remote-session://") && seed_remote_snapshot
}

fn startup_prewarm_should_seed_remote_snapshot(
    path: &str,
    active_terminal_path: Option<&str>,
) -> bool {
    path.starts_with("remote-session://") && active_terminal_path == Some(path)
}

fn mark_daemon_activity(last_activity_ms: &AtomicU64) {
    last_activity_ms.store(current_millis_u64(), Ordering::Relaxed);
}

fn daemon_should_idle_shutdown(
    home_dir: &Path,
    endpoint: &ServerEndpoint,
    last_activity_ms: &AtomicU64,
    idle_shutdown_ms: u64,
    terminal_session_count: usize,
) -> bool {
    if terminal_session_count > 0 {
        return false;
    }
    let idle_for_ms = current_millis_u64().saturating_sub(last_activity_ms.load(Ordering::Relaxed));
    if idle_for_ms < idle_shutdown_ms {
        return false;
    }
    match active_client_instance_records(home_dir, endpoint) {
        Ok(records) => records.is_empty(),
        Err(error) => {
            warn!(error=%error, "failed to read active client instances during daemon idle check");
            false
        }
    }
}

#[cfg(unix)]
fn wait_for_listener_ready(fd: i32, timeout_ms: u64) -> Result<bool> {
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout_ms = i32::try_from(timeout_ms).unwrap_or(i32::MAX);
    loop {
        // SAFETY: `pollfd` points to valid memory for one file descriptor descriptor.
        let rc = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if rc > 0 {
            return Ok(true);
        }
        if rc == 0 {
            return Ok(false);
        }
        let err = std::io::Error::last_os_error();
        if err.kind() == ErrorKind::Interrupted {
            continue;
        }
        return Err(err).context("polling daemon listener");
    }
}

fn server_request_name(request: &ServerRequest) -> &'static str {
    match request {
        ServerRequest::Ping => "ping",
        ServerRequest::Status => "status",
        ServerRequest::Snapshot => "snapshot",
        ServerRequest::PrepareUpdateRestart => "prepare_update_restart",
        ServerRequest::PrepareClientClose => "prepare_client_close",
        ServerRequest::HotRestart { .. } => "hot_restart",
        ServerRequest::OpenStoredSession { .. } => "open_stored_session",
        ServerRequest::ConnectSsh { .. } => "connect_ssh",
        ServerRequest::ConnectSshCustom { .. } => "connect_ssh_custom",
        ServerRequest::StartSshSession { .. } => "start_ssh_session",
        ServerRequest::StartRemoteCodexSession { .. } => "start_remote_codex_session",
        ServerRequest::OpenRemoteSession { .. } => "open_remote_session",
        ServerRequest::RefreshRemoteMachine { .. } => "refresh_remote_machine",
        ServerRequest::RefreshManagedCli { .. } => "refresh_managed_cli",
        ServerRequest::RefreshPreview { .. } => "refresh_preview",
        ServerRequest::UpdateSessionCopy { .. } => "update_session_copy",
        ServerRequest::RemoveSshTarget { .. } => "remove_ssh_target",
        ServerRequest::RemoveSession { .. } => "remove_session",
        ServerRequest::SetSessionKeepAlive { .. } => "set_session_keep_alive",
        ServerRequest::StartLocalSession { .. } => "start_local_session",
        ServerRequest::SwitchAgentSessionMode { .. } => "switch_agent_session_mode",
        ServerRequest::StartCommandSession { .. } => "start_command_session",
        ServerRequest::EnsureRemoteRuntimeCodexSession { .. } => {
            "ensure_remote_runtime_codex_session"
        }
        ServerRequest::StartRemoteRuntimeCodexSession { .. } => {
            "start_remote_runtime_codex_session"
        }
        ServerRequest::FocusLive { .. } => "focus_live",
        ServerRequest::SetViewMode { .. } => "set_view_mode",
        ServerRequest::TogglePreviewBlock { .. } => "toggle_preview_block",
        ServerRequest::SetAllPreviewBlocksFolded { .. } => "set_all_preview_blocks_folded",
        ServerRequest::RequestTerminalLaunch => "request_terminal_launch",
        ServerRequest::RequestTerminalLaunchForPath { .. } => "request_terminal_launch_for_path",
        ServerRequest::TerminalEnsure { .. } => "terminal_ensure",
        ServerRequest::TerminalRead { .. } => "terminal_read",
        ServerRequest::TerminalSnapshot { .. } => "terminal_snapshot",
        ServerRequest::TerminalRetainedSnapshot { .. } => "terminal_retained_snapshot",
        ServerRequest::TerminalWrite { .. } => "terminal_write",
        ServerRequest::TerminalResize { .. } => "terminal_resize",
        ServerRequest::TerminalRestart { .. } => "terminal_restart",
        ServerRequest::SyncExternalWindow => "sync_external_window",
        ServerRequest::RaiseExternalWindow => "raise_external_window",
        ServerRequest::SyncTheme { .. } => "sync_theme",
        ServerRequest::SyncTerminalIdentity { .. } => "sync_terminal_identity",
        ServerRequest::Shutdown => "shutdown",
    }
}

fn current_build_id() -> u64 {
    std::env::current_exe()
        .ok()
        .and_then(|path| fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok())
        .and_then(|ts| ts.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs())
        .unwrap_or_default()
}

#[cfg(unix)]
struct DaemonSocketLock {
    file: fs::File,
}

#[cfg(unix)]
impl Drop for DaemonSocketLock {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(unix)]
fn daemon_socket_lock_path(socket_path: &Path) -> PathBuf {
    let file_name = socket_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("server.sock");
    socket_path.with_file_name(format!("{file_name}.lock"))
}

#[cfg(unix)]
fn try_acquire_daemon_socket_lock(
    socket_path: &Path,
    home_dir: &Path,
) -> Result<Option<DaemonSocketLock>> {
    let lock_path = daemon_socket_lock_path(socket_path);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating daemon lock dir {}", parent.display()))?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening daemon socket lock {}", lock_path.display()))?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(Some(DaemonSocketLock { file }));
    }
    let error = std::io::Error::last_os_error();
    let raw_os_error = error.raw_os_error();
    if raw_os_error == Some(libc::EWOULDBLOCK) || raw_os_error == Some(libc::EAGAIN) {
        append_trace_event(
            home_dir,
            "daemon",
            "lifecycle",
            "bind_lock_busy",
            serde_json::json!({
                "socket": socket_path.display().to_string(),
                "lock": lock_path.display().to_string(),
            }),
        );
        return Ok(None);
    }
    Err(error).with_context(|| format!("locking daemon socket {}", lock_path.display()))
}

pub fn default_endpoint(home_dir: &Path) -> ServerEndpoint {
    #[cfg(unix)]
    {
        let socket_name = format!("server-{}.sock", SERVER_PROTOCOL_VERSION.replace('.', "-"));
        let home_socket = home_dir.join(&socket_name);
        let socket_path = if unix_socket_path_fits_platform(&home_socket) {
            home_socket
        } else {
            short_unix_socket_path_for_home(home_dir, &socket_name)
        };
        ServerEndpoint::UnixSocket(socket_path)
    }

    #[cfg(not(unix))]
    {
        let _ = home_dir;
        ServerEndpoint::Tcp {
            host: "127.0.0.1".to_string(),
            port: versioned_tcp_port(),
        }
    }
}

#[cfg(not(unix))]
fn versioned_tcp_port() -> u16 {
    let mut parts = SERVER_PROTOCOL_VERSION.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(2);
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let patch = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0)
        .min(9);
    58000 + major.saturating_mul(100) + minor.saturating_mul(10) + patch
}

pub fn ping(endpoint: &ServerEndpoint) -> Result<()> {
    match send_request(endpoint, &ServerRequest::Ping)? {
        ServerResponse::Pong => Ok(()),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected ping response: {:?}", other),
    }
}

pub fn status(endpoint: &ServerEndpoint) -> Result<ServerRuntimeStatus> {
    match send_request(endpoint, &ServerRequest::Status)? {
        ServerResponse::Status(status) => Ok(status),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected status response: {:?}", other),
    }
}

#[cfg(unix)]
fn versioned_server_status_probe_paths(home_dir: &Path) -> Vec<PathBuf> {
    let mut seen = HashSet::<PathBuf>::new();
    let mut seen_socket_identities = HashSet::<PathBuf>::new();
    let mut paths = Vec::<PathBuf>::new();
    let mut push_path = |path: PathBuf| {
        if !seen.insert(path.clone()) {
            return;
        }
        let socket_identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen_socket_identities.insert(socket_identity) {
            return;
        }
        paths.push(path);
    };

    let current_endpoint = default_endpoint(home_dir);
    if let ServerEndpoint::UnixSocket(current_path) = current_endpoint {
        push_path(current_path.clone());
        for candidate in versioned_server_socket_alias_candidates(&current_path) {
            push_path(candidate);
        }
    }

    if let Ok(entries) = fs::read_dir(home_dir) {
        let mut home_paths = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| parse_versioned_server_socket_name(path).is_some())
            .collect::<Vec<_>>();
        home_paths.sort_by(|a, b| {
            parse_versioned_server_socket_name(b)
                .cmp(&parse_versioned_server_socket_name(a))
                .then_with(|| a.cmp(b))
        });
        for path in home_paths {
            push_path(path);
        }
    }

    paths
}

#[cfg(unix)]
pub fn reachable_versioned_daemon_statuses(
    home_dir: &Path,
) -> Vec<(ServerEndpoint, ServerRuntimeStatus)> {
    let paths = versioned_server_status_probe_paths(home_dir);
    let mut seen_pids = HashSet::<u32>::new();

    paths
        .into_iter()
        .filter_map(|path| {
            let endpoint = ServerEndpoint::UnixSocket(path);
            let runtime = status(&endpoint).ok()?;
            if !seen_pids.insert(runtime.server_pid) {
                return None;
            }
            Some((endpoint, runtime))
        })
        .collect()
}

#[cfg(not(unix))]
pub fn reachable_versioned_daemon_statuses(
    home_dir: &Path,
) -> Vec<(ServerEndpoint, ServerRuntimeStatus)> {
    let endpoint = default_endpoint(home_dir);
    status(&endpoint)
        .ok()
        .map(|runtime| vec![(endpoint, runtime)])
        .unwrap_or_default()
}

pub fn snapshot(endpoint: &ServerEndpoint) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::Snapshot)?)
}

pub fn open_stored_session(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    path: &str,
    session_id: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    open_stored_session_with_view(endpoint, kind, path, session_id, cwd, title_hint, None)
}

pub fn open_stored_session_with_view(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    path: &str,
    session_id: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    view_mode: Option<WorkspaceViewMode>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::OpenStoredSession {
            session_kind: kind,
            path: path.to_string(),
            session_id: session_id.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            view_mode,
        },
    )?)
}

pub fn connect_ssh(
    endpoint: &ServerEndpoint,
    target_ix: usize,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::ConnectSsh { target_ix },
    )?)
}

pub fn connect_ssh_custom(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::ConnectSshCustom {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
        },
    )?)
}

pub fn open_remote_session(
    endpoint: &ServerEndpoint,
    machine_key: &str,
    session_id: &str,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    open_remote_session_with_view(endpoint, machine_key, session_id, cwd, title_hint, None)
}

pub fn open_remote_session_with_view(
    endpoint: &ServerEndpoint,
    machine_key: &str,
    session_id: &str,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    view_mode: Option<WorkspaceViewMode>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::OpenRemoteSession {
            machine_key: machine_key.to_string(),
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            view_mode,
        },
    )?)
}

pub fn start_ssh_session_at(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_ssh_session_at_with_terminal_appearance(endpoint, target, prefix, cwd, title_hint, None)
}

pub fn start_ssh_session_at_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartSshSession {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
        },
    )?)
}

pub fn start_remote_codex_session_at(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_remote_codex_session_at_with_terminal_appearance(
        endpoint, target, prefix, cwd, title_hint, None,
    )
}

pub fn start_remote_codex_session_at_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartRemoteCodexSession {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
        },
    )?)
}

pub fn refresh_remote_machine(
    endpoint: &ServerEndpoint,
    machine_key: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RefreshRemoteMachine {
            machine_key: machine_key.to_string(),
        },
    )?)
}

pub fn refresh_managed_cli(
    endpoint: &ServerEndpoint,
    machine_key: Option<&str>,
    background: bool,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::RefreshManagedCli {
            machine_key: machine_key.map(ToOwned::to_owned),
            background,
        },
    )?)
}

pub fn refresh_preview(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RefreshPreview {
            path: path.to_string(),
        },
    )?)
}

pub fn update_session_copy(
    endpoint: &ServerEndpoint,
    path: &str,
    title: Option<&str>,
    precis: Option<&str>,
    summary: Option<&str>,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::UpdateSessionCopy {
            path: path.to_string(),
            title: title.map(ToOwned::to_owned),
            precis: precis.map(ToOwned::to_owned),
            summary: summary.map(ToOwned::to_owned),
        },
    )?)
}

pub fn remove_ssh_target(
    endpoint: &ServerEndpoint,
    machine_key: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RemoveSshTarget {
            machine_key: machine_key.to_string(),
        },
    )?)
}

pub fn remove_session(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RemoveSession {
            path: path.to_string(),
        },
    )?)
}

pub fn set_session_keep_alive(
    endpoint: &ServerEndpoint,
    path: &str,
    keep_alive: bool,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SetSessionKeepAlive {
            path: path.to_string(),
            keep_alive,
        },
    )?)
}

pub fn start_local_session(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_local_session_at(endpoint, kind, None, None)
}

pub fn start_local_session_at(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_local_session_at_with_terminal_appearance(endpoint, kind, cwd, title_hint, None)
}

pub fn start_local_session_at_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartLocalSession {
            session_kind: kind,
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
        },
    )?)
}

pub fn start_command_session(
    endpoint: &ServerEndpoint,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    launch_command: &str,
    source_label: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_command_session_with_terminal_appearance(
        endpoint,
        cwd,
        title_hint,
        launch_command,
        source_label,
        None,
    )
}

pub fn start_command_session_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    launch_command: &str,
    source_label: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartCommandSession {
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            launch_command: launch_command.to_string(),
            source_label: source_label.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
        },
    )?)
}

pub fn ensure_remote_runtime_codex_session(
    endpoint: &ServerEndpoint,
    session_id: &str,
    cwd: Option<&str>,
    require_existing: bool,
    initial_size: Option<(u16, u16)>,
    terminal_appearance: Option<&str>,
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::EnsureRemoteRuntimeCodexSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            require_existing,
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
        },
    )?)?
    .with_context(|| format!("missing runtime session key for {session_id}"))
}

pub fn start_remote_runtime_codex_session(
    endpoint: &ServerEndpoint,
    session_id: &str,
    cwd: Option<&str>,
    initial_size: Option<(u16, u16)>,
    terminal_appearance: Option<&str>,
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::StartRemoteRuntimeCodexSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
        },
    )?)?
    .with_context(|| format!("missing runtime session key for {session_id}"))
}

pub fn focus_live(
    endpoint: &ServerEndpoint,
    key: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    focus_live_with_view(endpoint, key, None)
}

pub fn focus_live_with_view(
    endpoint: &ServerEndpoint,
    key: &str,
    view_mode: Option<WorkspaceViewMode>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::FocusLive {
            key: key.to_string(),
            view_mode,
        },
    )?)
}

pub fn switch_agent_session_mode(
    endpoint: &ServerEndpoint,
    path: &str,
    kind: SessionKind,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SwitchAgentSessionMode {
            path: path.to_string(),
            session_kind: kind,
        },
    )?)
}

pub fn set_view_mode(
    endpoint: &ServerEndpoint,
    mode: WorkspaceViewMode,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SetViewMode { mode },
    )?)
}

pub fn toggle_preview_block(
    endpoint: &ServerEndpoint,
    block_ix: usize,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::TogglePreviewBlock { block_ix },
    )?)
}

pub fn set_all_preview_blocks_folded(
    endpoint: &ServerEndpoint,
    folded: bool,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SetAllPreviewBlocksFolded { folded },
    )?)
}

pub fn request_terminal_launch(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RequestTerminalLaunch,
    )?)
}

pub fn request_terminal_launch_for_path(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RequestTerminalLaunchForPath {
            path: path.to_string(),
        },
    )?)
}

pub fn terminal_ensure(endpoint: &ServerEndpoint, path: &str) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalEnsure {
            path: path.to_string(),
        },
    )?)
}

pub fn terminal_read(
    endpoint: &ServerEndpoint,
    path: &str,
    cursor: u64,
) -> Result<(u64, Vec<TerminalStreamChunk>, bool, bool, bool, bool, u64)> {
    match send_request(
        endpoint,
        &ServerRequest::TerminalRead {
            path: path.to_string(),
            cursor,
        },
    )? {
        ServerResponse::TerminalStream {
            cursor,
            chunks,
            running,
            runtime_output_seen,
            eof_without_output,
            post_resize_output_seen,
            last_resize_seq,
        } => Ok((
            cursor,
            chunks,
            running,
            runtime_output_seen,
            eof_without_output,
            post_resize_output_seen,
            last_resize_seq,
        )),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected terminal stream response: {:?}", other),
    }
}

pub fn terminal_snapshot(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(String, bool, bool, bool, u64)> {
    match send_request(
        endpoint,
        &ServerRequest::TerminalSnapshot {
            path: path.to_string(),
        },
    )? {
        ServerResponse::TerminalSnapshot {
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
        } => Ok((
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
        )),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected terminal snapshot response: {:?}", other),
    }
}

pub fn terminal_retained_snapshot(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(String, bool, bool, bool, u64)> {
    match send_request(
        endpoint,
        &ServerRequest::TerminalRetainedSnapshot {
            path: path.to_string(),
        },
    )? {
        ServerResponse::TerminalRetainedSnapshot {
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
        } => Ok((
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
        )),
        ServerResponse::Error { message } => bail!(message),
        other => bail!(
            "unexpected terminal retained snapshot response: {:?}",
            other
        ),
    }
}

pub fn terminal_write(endpoint: &ServerEndpoint, path: &str, data: &str) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalWrite {
            path: path.to_string(),
            data: data.to_string(),
        },
    )?)
}

pub fn terminal_resize(
    endpoint: &ServerEndpoint,
    path: &str,
    cols: u16,
    rows: u16,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalResize {
            path: path.to_string(),
            cols,
            rows,
        },
    )?)
}

pub fn terminal_restart(
    endpoint: &ServerEndpoint,
    path: &str,
    terminal_appearance: Option<&str>,
    force_remote: bool,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    terminal_restart_with_size(endpoint, path, terminal_appearance, force_remote, None)
}

pub fn terminal_restart_with_size(
    endpoint: &ServerEndpoint,
    path: &str,
    terminal_appearance: Option<&str>,
    force_remote: bool,
    initial_size: Option<(u16, u16)>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::TerminalRestart {
            path: path.to_string(),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            force_remote,
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
        },
    )?)
}

pub fn sync_external_window(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::SyncExternalWindow)?)
}

pub fn raise_external_window(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::RaiseExternalWindow)?)
}

pub fn sync_theme(
    endpoint: &ServerEndpoint,
    theme: UiTheme,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::SyncTheme { theme })?)
}

pub fn sync_terminal_identity(
    endpoint: &ServerEndpoint,
    terminal_appearance: &str,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::SyncTerminalIdentity {
            terminal_appearance: terminal_appearance.to_string(),
        },
    )?)
}

pub fn prepare_update_restart(endpoint: &ServerEndpoint) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::PrepareUpdateRestart,
    )?)
}

pub fn prepare_client_close(endpoint: &ServerEndpoint) -> Result<Option<String>> {
    expect_ack(send_request(endpoint, &ServerRequest::PrepareClientClose)?)
}

pub fn hot_restart(
    endpoint: &ServerEndpoint,
    daemon_executable: &Path,
    expected_version: Option<&str>,
    expected_build_id: Option<u64>,
    reason: Option<&str>,
) -> Result<Option<String>> {
    Ok(hot_restart_detailed(
        endpoint,
        daemon_executable,
        expected_version,
        expected_build_id,
        reason,
    )?
    .message())
}

pub fn hot_restart_detailed(
    endpoint: &ServerEndpoint,
    daemon_executable: &Path,
    expected_version: Option<&str>,
    expected_build_id: Option<u64>,
    reason: Option<&str>,
) -> Result<HotRestartResult> {
    let daemon_executable = daemon_executable
        .canonicalize()
        .unwrap_or_else(|_| daemon_executable.to_path_buf());
    match send_request(
        endpoint,
        &ServerRequest::HotRestart {
            daemon_executable: daemon_executable.display().to_string(),
            expected_version: expected_version.map(ToOwned::to_owned),
            expected_build_id,
            reason: reason.map(ToOwned::to_owned),
        },
    )? {
        ServerResponse::Ack { message } => Ok(HotRestartResult::Restarting { message }),
        ServerResponse::HotUpdateHandoff {
            message,
            owner_endpoint,
            owner_server_version,
            owner_server_pid,
            target_server_version,
            runtime_keys,
        } => Ok(HotRestartResult::Handoff {
            message,
            owner_endpoint,
            owner_server_version,
            owner_server_pid,
            target_server_version,
            runtime_keys,
        }),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected hot restart response: {:?}", other),
    }
}

pub fn shutdown(endpoint: &ServerEndpoint) -> Result<Option<String>> {
    expect_ack(send_request(endpoint, &ServerRequest::Shutdown)?)
}

pub fn cleanup_legacy_daemons(endpoint: &ServerEndpoint, current_exe: &Path) -> Result<()> {
    #[cfg(unix)]
    cleanup_legacy_unix_daemons(endpoint)?;
    #[cfg(target_os = "linux")]
    cleanup_legacy_linux_daemon_processes(endpoint, current_exe)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn spawn_legacy_linux_daemon_cleanup(
    endpoint: ServerEndpoint,
    current_exe: PathBuf,
    home_dir: PathBuf,
) {
    if let Err(error) = std::thread::Builder::new()
        .name("yggterm-legacy-daemon-cleanup".to_string())
        .spawn(move || {
            append_trace_event(
                &home_dir,
                "daemon",
                "lifecycle",
                "legacy_cleanup_begin",
                serde_json::json!({
                    "endpoint": format!("{endpoint:?}"),
                    "current_exe": current_exe.display().to_string(),
                }),
            );
            match cleanup_legacy_linux_daemon_processes(&endpoint, &current_exe) {
                Ok(()) => append_trace_event(
                    &home_dir,
                    "daemon",
                    "lifecycle",
                    "legacy_cleanup_end",
                    serde_json::json!({}),
                ),
                Err(error) => {
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "lifecycle",
                        "legacy_cleanup_error",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                    warn!(error=%error, "legacy daemon cleanup failed");
                }
            }
        })
    {
        warn!(error=%error, "failed to spawn legacy daemon cleanup");
    }
}

#[cfg(target_os = "linux")]
fn spawn_force_remote_restart_daemon_cleanup(store: &SessionStore, owner_registry_empty: bool) {
    if !owner_registry_empty {
        return;
    }
    let Ok(current_exe) = std::env::current_exe() else {
        return;
    };
    spawn_legacy_linux_daemon_cleanup(
        default_endpoint(store.home_dir()),
        current_exe,
        store.home_dir().to_path_buf(),
    );
}

#[cfg(not(target_os = "linux"))]
fn spawn_force_remote_restart_daemon_cleanup(_store: &SessionStore, _owner_registry_empty: bool) {}

fn spawn_active_terminal_prewarm(
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
    home_dir: PathBuf,
) {
    std::thread::spawn(move || {
        let (active_terminal_path, paths) = {
            let runtime = lock_daemon_runtime(&runtime, "startup_prewarm_active_path");
            let mut paths = Vec::<String>::new();
            let mut active_terminal_path = None::<String>;
            if runtime.server.active_view_mode() == WorkspaceViewMode::Terminal
                && runtime.server.active_session_supports_terminal()
            {
                if let Some(path) = runtime.server.active_session_path().map(ToOwned::to_owned) {
                    active_terminal_path = Some(path.clone());
                    paths.push(path);
                }
            }
            for session in runtime.server.live_sessions() {
                if runtime
                    .server
                    .session_supports_terminal(&session.session_path)
                    && !paths.iter().any(|path| path == &session.session_path)
                {
                    paths.push(session.session_path);
                }
            }
            (active_terminal_path, paths)
        };
        if paths.is_empty() {
            return;
        }
        append_trace_event(
            &home_dir,
            "daemon",
            "startup_prewarm",
            "plan",
            serde_json::json!({
                "active_terminal_path": active_terminal_path.as_deref(),
                "paths": paths.clone(),
            }),
        );
        for path in paths {
            if let Some(reason) = startup_prewarm_skip_reason(&path) {
                append_trace_event(
                    &home_dir,
                    "daemon",
                    "startup_prewarm",
                    "skip",
                    serde_json::json!({
                        "path": path,
                        "reason": reason,
                    }),
                );
                continue;
            }
            let seed_remote_snapshot =
                startup_prewarm_should_seed_remote_snapshot(&path, active_terminal_path.as_deref());
            append_trace_event(
                &home_dir,
                "daemon",
                "startup_prewarm",
                "begin",
                serde_json::json!({
                    "path": path,
                    "seed_remote_snapshot": seed_remote_snapshot,
                }),
            );
            let outcome = {
                let mut runtime = lock_daemon_runtime(&runtime, "startup_prewarm_ensure");
                runtime.ensure_terminal_for_path_for_startup_prewarm(&path, seed_remote_snapshot)
            };
            match outcome {
                Ok(_) => {
                    mark_daemon_activity(last_activity_ms.as_ref());
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "startup_prewarm",
                        "end",
                        serde_json::json!({ "path": path }),
                    );
                }
                Err(error) => {
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "startup_prewarm",
                        "error",
                        serde_json::json!({
                            "path": path,
                            "error": error.to_string(),
                        }),
                    );
                    warn!(path=%path, error=%error, "daemon startup terminal prewarm failed");
                }
            }
        }
    });
}

pub fn run_daemon(endpoint: &ServerEndpoint, runtime: GhosttyHostSupport) -> Result<()> {
    let runtime = Arc::new(Mutex::new(DaemonRuntime::load(runtime)?));
    let last_activity_ms = Arc::new(AtomicU64::new(current_millis_u64()));
    let idle_shutdown_ms = daemon_idle_shutdown_ms();
    let terminal_idle_trim_after_ms = daemon_terminal_idle_trim_after_ms();
    let home_dir = {
        let runtime = lock_daemon_runtime(&runtime, "run_daemon_home_dir");
        runtime.store.home_dir().to_path_buf()
    };
    append_trace_event(
        &home_dir,
        "daemon",
        "lifecycle",
        "run_begin",
        serde_json::json!({
            "endpoint": format!("{endpoint:?}"),
        }),
    );
    match RemoteRuntimeRegistry::open(&home_dir) {
        Ok(registry) => append_trace_event(
            &home_dir,
            "daemon",
            "remote_runtime",
            "bootstrap",
            serde_json::json!({
                "db_path": registry.paths().db_path.display().to_string(),
                "sessions_dir": registry.paths().sessions_dir.display().to_string(),
            }),
        ),
        Err(error) => warn!(error=%error, "failed to initialize remote runtime registry"),
    }
    if daemon_background_copy_chore_enabled() {
        let runtime = runtime.clone();
        let last_activity_ms = last_activity_ms.clone();
        std::thread::spawn(move || {
            let mut sleep_ms = BACKGROUND_COPY_CHORE_MS;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
                match run_background_copy_chore(&runtime) {
                    Ok(update_count) => {
                        if update_count > 0 {
                            mark_daemon_activity(&last_activity_ms);
                            sleep_ms = BACKGROUND_COPY_CHORE_MS;
                        } else {
                            sleep_ms =
                                (sleep_ms.saturating_mul(2)).min(BACKGROUND_COPY_MAX_IDLE_CHORE_MS);
                        }
                    }
                    Err(error) => {
                        sleep_ms =
                            (sleep_ms.saturating_mul(2)).min(BACKGROUND_COPY_MAX_IDLE_CHORE_MS);
                        warn!(error=%error, "daemon background copy chore failed");
                    }
                }
            }
        });
    } else {
        append_trace_event(
            &home_dir,
            "daemon",
            "background_copy",
            "disabled",
            serde_json::json!({
                "env": ENV_YGGTERM_ENABLE_BACKGROUND_COPY_CHORE,
            }),
        );
    }
    {
        let runtime = runtime.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(BACKGROUND_COPY_CHORE_MS));
                match trim_terminal_buffers(&runtime, terminal_idle_trim_after_ms) {
                    Ok(summary) if summary.trimmed_sessions > 0 => {
                        let home_dir = {
                            let runtime =
                                lock_daemon_runtime(&runtime, "trim_terminal_buffers_home_dir");
                            runtime.store.home_dir().to_path_buf()
                        };
                        append_trace_event(
                            &home_dir,
                            "daemon",
                            "terminal_buffers",
                            "idle_trim",
                            serde_json::json!({
                                "trimmed_sessions": summary.trimmed_sessions,
                                "reclaimed_bytes": summary.reclaimed_bytes,
                                "before_sessions": summary.before.session_count,
                                "before_chunks": summary.before.retained_chunks,
                                "before_bytes": summary.before.retained_bytes,
                                "after_sessions": summary.after.session_count,
                                "after_chunks": summary.after.retained_chunks,
                                "after_bytes": summary.after.retained_bytes,
                                "idle_trim_after_ms": terminal_idle_trim_after_ms,
                            }),
                        );
                    }
                    Ok(_) => {}
                    Err(error) => warn!(error=%error, "daemon terminal buffer trim chore failed"),
                }
            }
        });
    }

    #[cfg(unix)]
    if let ServerEndpoint::UnixSocket(path) = endpoint {
        let Some(_daemon_socket_lock) = try_acquire_daemon_socket_lock(path, &home_dir)? else {
            info!(
                path = %path.display(),
                "another yggterm daemon owns the socket bind lock"
            );
            return Ok(());
        };
        if server_socket_path_lexists(path)
            && ping(&ServerEndpoint::UnixSocket(path.clone())).is_ok()
        {
            append_trace_event(
                &home_dir,
                "daemon",
                "lifecycle",
                "existing_socket_owner_reused",
                serde_json::json!({
                    "endpoint": path.display().to_string(),
                    "pid": std::process::id(),
                }),
            );
            info!(
                path = %path.display(),
                "existing yggterm daemon answered before bind"
            );
            return Ok(());
        }
        if server_socket_path_lexists(path) {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) => {
                    warn!(path=%path.display(), error=%error, "failed to remove stale server socket")
                }
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating server socket dir {}", parent.display()))?;
        }
        let listener = std::os::unix::net::UnixListener::bind(path)
            .with_context(|| format!("binding server socket {}", path.display()))?;
        refresh_legacy_server_socket_aliases(path);
        listener
            .set_nonblocking(true)
            .context("setting daemon unix listener nonblocking")?;
        let host = {
            let runtime = lock_daemon_runtime(&runtime, "run_daemon_unix_host");
            runtime.support.kind.as_str().to_string()
        };
        info!(path=%path.display(), host=%host, "yggterm server daemon listening");
        spawn_active_terminal_prewarm(runtime.clone(), last_activity_ms.clone(), home_dir.clone());
        #[cfg(target_os = "linux")]
        if let Ok(current_exe) = std::env::current_exe() {
            spawn_legacy_linux_daemon_cleanup(endpoint.clone(), current_exe, home_dir.clone());
        }
        let (client_outcome_tx, client_outcome_rx) =
            std::sync::mpsc::channel::<Result<DaemonRequestOutcome>>();
        let mut restart_after_exit = None::<PathBuf>;
        loop {
            if drain_unix_client_outcomes(&client_outcome_rx, &mut restart_after_exit) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    spawn_unix_client_handler(
                        stream,
                        runtime.clone(),
                        last_activity_ms.clone(),
                        client_outcome_tx.clone(),
                    );
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if daemon_should_idle_shutdown(
                        &home_dir,
                        endpoint,
                        last_activity_ms.as_ref(),
                        idle_shutdown_ms,
                        lock_daemon_runtime(&runtime, "daemon_idle_terminal_count")
                            .terminals
                            .stats()
                            .session_count,
                    ) {
                        append_trace_event(
                            &home_dir,
                            "daemon",
                            "lifecycle",
                            "idle_shutdown",
                            serde_json::json!({ "idle_shutdown_ms": idle_shutdown_ms }),
                        );
                        info!(path=%path.display(), idle_shutdown_ms, "yggterm daemon idle shutdown");
                        break;
                    }
                    let _ = wait_for_listener_ready(listener.as_raw_fd(), DAEMON_ACCEPT_POLL_MS)?;
                }
                Err(error) => return Err(error).context("accepting daemon client"),
            }
        }
        let restart_executable = restart_after_exit.take();
        append_trace_event(
            &home_dir,
            "daemon",
            "lifecycle",
            "run_end",
            serde_json::json!({
                "endpoint": format!("{endpoint:?}"),
                "transport": "unix",
                "hot_restart": restart_executable.is_some(),
            }),
        );
        drop(listener);
        if let Some(executable) = restart_executable {
            if let Err(error) = fs::remove_file(path)
                && error.kind() != ErrorKind::NotFound
            {
                warn!(
                    path = %path.display(),
                    error = %error,
                    "failed to remove unix socket before hot restart"
                );
            }
            if let Err(error) = spawn_hot_restart_daemon_process(&executable, endpoint) {
                append_trace_event(
                    &home_dir,
                    "daemon",
                    "lifecycle",
                    "hot_restart_spawn_failed",
                    serde_json::json!({
                        "daemon_executable": executable.display().to_string(),
                        "error": error.to_string(),
                    }),
                );
                warn!(
                    daemon_executable = %executable.display(),
                    error = %error,
                    "failed to spawn hot restart daemon"
                );
            }
        }
        return Ok(());
    }

    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => {
            bail!(
                "unix sockets are unsupported on this platform: {}",
                path.display()
            )
        }
        ServerEndpoint::Tcp { host, port } => {
            let listener = std::net::TcpListener::bind((host.as_str(), *port))
                .with_context(|| format!("binding server tcp endpoint {}:{}", host, port))?;
            listener
                .set_nonblocking(true)
                .context("setting daemon tcp listener nonblocking")?;
            let host_kind = {
                let runtime = lock_daemon_runtime(&runtime, "run_daemon_tcp_host");
                runtime.support.kind.as_str().to_string()
            };
            info!(host=%host, port, host_kind=%host_kind, "yggterm server daemon listening");
            #[cfg(target_os = "linux")]
            if let Ok(current_exe) = std::env::current_exe() {
                spawn_legacy_linux_daemon_cleanup(endpoint.clone(), current_exe, home_dir.clone());
            }
            spawn_active_terminal_prewarm(
                runtime.clone(),
                last_activity_ms.clone(),
                home_dir.clone(),
            );
            let mut restart_after_exit = None::<PathBuf>;
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let runtime = runtime.clone();
                        let last_activity_ms = last_activity_ms.clone();
                        match handle_tcp_stream(stream, runtime, last_activity_ms) {
                            Ok(outcome) => {
                                if let Some(executable) = outcome.restart_executable {
                                    restart_after_exit = Some(executable);
                                }
                                if outcome.should_shutdown {
                                    break;
                                }
                            }
                            Err(error) => warn!(error=%error, "daemon request failed"),
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if daemon_should_idle_shutdown(
                            &home_dir,
                            endpoint,
                            last_activity_ms.as_ref(),
                            idle_shutdown_ms,
                            lock_daemon_runtime(&runtime, "daemon_idle_terminal_count")
                                .terminals
                                .stats()
                                .session_count,
                        ) {
                            append_trace_event(
                                &home_dir,
                                "daemon",
                                "lifecycle",
                                "idle_shutdown",
                                serde_json::json!({ "idle_shutdown_ms": idle_shutdown_ms }),
                            );
                            info!(host=%host, port, idle_shutdown_ms, "yggterm daemon idle shutdown");
                            break;
                        }
                        #[cfg(unix)]
                        {
                            let _ = wait_for_listener_ready(
                                listener.as_raw_fd(),
                                DAEMON_ACCEPT_POLL_MS,
                            )?;
                        }
                        #[cfg(not(unix))]
                        {
                            std::thread::sleep(std::time::Duration::from_millis(
                                DAEMON_ACCEPT_POLL_MS,
                            ));
                        }
                    }
                    Err(error) => return Err(error).context("accepting daemon client"),
                }
            }
            let restart_executable = restart_after_exit.take();
            append_trace_event(
                &home_dir,
                "daemon",
                "lifecycle",
                "run_end",
                serde_json::json!({
                    "endpoint": format!("{endpoint:?}"),
                    "transport": "tcp",
                    "hot_restart": restart_executable.is_some(),
                }),
            );
            drop(listener);
            if let Some(executable) = restart_executable {
                if let Err(error) = spawn_hot_restart_daemon_process(&executable, endpoint) {
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "lifecycle",
                        "hot_restart_spawn_failed",
                        serde_json::json!({
                            "daemon_executable": executable.display().to_string(),
                            "error": error.to_string(),
                        }),
                    );
                    warn!(
                        daemon_executable = %executable.display(),
                        error = %error,
                        "failed to spawn hot restart daemon"
                    );
                }
            }
            Ok(())
        }
    }
}

fn lock_daemon_runtime<'a>(
    runtime: &'a Arc<Mutex<DaemonRuntime>>,
    label: &'static str,
) -> MutexGuard<'a, DaemonRuntime> {
    match runtime.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            append_trace_event(
                guard.store.home_dir(),
                "daemon",
                "lifecycle",
                "runtime_lock_poisoned",
                serde_json::json!({
                    "label": label,
                }),
            );
            warn!(label, "daemon runtime lock poisoned; recovering");
            guard
        }
    }
}

fn mark_remote_machine_refresh_queued(
    in_flight: &mut HashSet<String>,
    machine_key: &str,
    should_spawn: bool,
) -> RemoteMachineRefreshQueueStatus {
    if !should_spawn {
        return RemoteMachineRefreshQueueStatus::Loopback;
    }
    if in_flight.insert(machine_key.to_string()) {
        RemoteMachineRefreshQueueStatus::Spawn
    } else {
        RemoteMachineRefreshQueueStatus::AlreadyInFlight
    }
}

fn daemon_queue_remote_machine_refresh(
    runtime: &Arc<Mutex<DaemonRuntime>>,
    machine_key: String,
    request_name: &'static str,
) -> ServerResponse {
    let normalized_machine_key = machine_key.trim().to_string();
    let (target, home_dir, response, should_spawn) = {
        let mut runtime = lock_daemon_runtime(runtime, "queue_remote_machine_refresh");
        if daemon_request_trace_enabled(request_name) {
            append_trace_event(
                runtime.store.home_dir(),
                "daemon",
                "request",
                "begin",
                serde_json::json!({ "request": request_name, "mode": "queued" }),
            );
        }
        let target = match runtime
            .server
            .remote_target_for_machine_key(&normalized_machine_key)
        {
            Ok(target) => target,
            Err(error) => {
                if daemon_request_trace_enabled(request_name) {
                    append_trace_event(
                        runtime.store.home_dir(),
                        "daemon",
                        "request",
                        "end",
                        serde_json::json!({
                            "request": request_name,
                            "mode": "queued",
                            "ok": false,
                            "error": error.to_string(),
                        }),
                    );
                }
                return ServerResponse::Error {
                    message: error.to_string(),
                };
            }
        };
        let should_spawn = !YggtermServer::is_loopback_remote_target(&target);
        let queue_status = mark_remote_machine_refresh_queued(
            &mut runtime.remote_machine_refreshes_in_flight,
            &normalized_machine_key,
            should_spawn,
        );
        let response = runtime.snapshot_response(Some(match queue_status {
            RemoteMachineRefreshQueueStatus::Spawn => {
                format!("queued refresh {normalized_machine_key}")
            }
            RemoteMachineRefreshQueueStatus::AlreadyInFlight => {
                format!("refresh already in progress {normalized_machine_key}")
            }
            RemoteMachineRefreshQueueStatus::Loopback => {
                format!("skipped loopback refresh {normalized_machine_key}")
            }
        }));
        if daemon_request_trace_enabled(request_name) {
            append_trace_event(
                runtime.store.home_dir(),
                "daemon",
                "request",
                "end",
                serde_json::json!({
                    "request": request_name,
                    "mode": "queued",
                    "ok": true,
                    "queue_status": format!("{queue_status:?}"),
                }),
            );
        }
        (
            target,
            runtime.store.home_dir().to_path_buf(),
            response,
            queue_status == RemoteMachineRefreshQueueStatus::Spawn,
        )
    };
    if should_spawn {
        let runtime = Arc::clone(runtime);
        let queued_machine_key = normalized_machine_key.clone();
        std::thread::spawn(move || {
            append_trace_event(
                &home_dir,
                "daemon",
                "remote_machine",
                "background_refresh_begin",
                serde_json::json!({
                    "machine_key": queued_machine_key.clone(),
                    "ssh_target": target.ssh_target.clone(),
                    "prefix": target.prefix.clone(),
                }),
            );
            let scan = YggtermServer::scan_remote_machine_refresh(&target);
            let mut runtime = lock_daemon_runtime(&runtime, "background_remote_machine_refresh");
            let outcome = runtime
                .server
                .apply_remote_machine_refresh_scan(&target, scan);
            if let Err(persist_error) = runtime.persist() {
                warn!(
                    machine_key = %queued_machine_key,
                    error = %persist_error,
                    "failed to persist background remote machine refresh"
                );
            }
            runtime
                .remote_machine_refreshes_in_flight
                .remove(&queued_machine_key);
            match outcome {
                Ok(()) => append_trace_event(
                    runtime.store.home_dir(),
                    "daemon",
                    "remote_machine",
                    "background_refresh_end",
                    serde_json::json!({
                        "machine_key": queued_machine_key.clone(),
                        "ok": true,
                    }),
                ),
                Err(error) => {
                    append_trace_event(
                        runtime.store.home_dir(),
                        "daemon",
                        "remote_machine",
                        "background_refresh_end",
                        serde_json::json!({
                            "machine_key": queued_machine_key.clone(),
                            "ok": false,
                            "error": error.to_string(),
                        }),
                    );
                    warn!(
                        machine_key = %queued_machine_key,
                        error = %error,
                        "background remote machine refresh failed"
                    );
                }
            }
        });
    }
    response
}

fn daemon_request_response(
    runtime: &Arc<Mutex<DaemonRuntime>>,
    request: ServerRequest,
) -> ServerResponse {
    let request_name = server_request_name(&request);
    if let ServerRequest::RefreshRemoteMachine { machine_key } = request {
        return daemon_queue_remote_machine_refresh(runtime, machine_key, request_name);
    }
    let mut runtime = lock_daemon_runtime(runtime, "handle_request");
    let home_dir = runtime.store.home_dir().to_path_buf();
    match panic::catch_unwind(AssertUnwindSafe(|| runtime.handle_request(request))) {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => ServerResponse::Error {
            message: error.to_string(),
        },
        Err(payload) => {
            let panic_message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|value| (*value).to_string())
                })
                .unwrap_or_else(|| "unknown panic payload".to_string());
            append_trace_event(
                &home_dir,
                "daemon",
                "request",
                "panic",
                serde_json::json!({
                    "request": request_name,
                    "panic": panic_message,
                }),
            );
            warn!(request = request_name, panic = %panic_message, "daemon request panicked");
            ServerResponse::Error {
                message: format!("daemon request {request_name} panicked: {panic_message}"),
            }
        }
    }
}

#[cfg(unix)]
fn cleanup_legacy_unix_daemons(endpoint: &ServerEndpoint) -> Result<()> {
    let ServerEndpoint::UnixSocket(current_path) = endpoint else {
        return Ok(());
    };
    let Some(home_dir) = current_path.parent() else {
        return Ok(());
    };
    let entries = match fs::read_dir(home_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading daemon socket dir {}", home_dir.display()));
        }
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("server") || !name.ends_with(".sock") || path == *current_path {
            continue;
        }

        match std::os::unix::net::UnixStream::connect(&path) {
            Ok(_) => {
                // A reachable versioned daemon may own live terminal runtimes that the
                // freshly started app has not reclaimed yet. Leave it running; Linux
                // process cleanup applies the stricter ownership checks below.
            }
            Err(_) => {
                let _ = fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn daemon_orphan_reap_after_ms() -> u64 {
    std::env::var("YGGTERM_DAEMON_ORPHAN_REAP_AFTER_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_ORPHAN_DAEMON_REAP_AFTER_MS)
}

#[cfg(target_os = "linux")]
fn linux_proc_pid_age_ms(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.split_once(") ")?.1;
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let start_ticks = fields.get(19)?.parse::<u64>().ok()?;
    let uptime = fs::read_to_string("/proc/uptime").ok()?;
    let uptime_secs = uptime.split_whitespace().next()?.parse::<f64>().ok()?;
    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if clk_tck <= 0 {
        return None;
    }
    let uptime_ticks = (uptime_secs * clk_tck as f64).floor() as u64;
    Some(
        uptime_ticks
            .saturating_sub(start_ticks)
            .saturating_mul(1_000)
            / (clk_tck as u64),
    )
}

#[cfg(target_os = "linux")]
fn linux_yggterm_home_from_environ_bytes(environ: &[u8]) -> Option<PathBuf> {
    let mut home = None::<PathBuf>;
    for part in environ
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
    {
        let item = String::from_utf8_lossy(part);
        if let Some(value) = item.strip_prefix("YGGTERM_HOME=") {
            return Some(PathBuf::from(value));
        }
        if let Some(value) = item.strip_prefix("HOME=") {
            home = Some(PathBuf::from(value));
        }
    }
    home.map(|path| path.join(".yggterm"))
}

#[cfg(target_os = "linux")]
fn linux_proc_yggterm_home(proc_path: &Path) -> Option<PathBuf> {
    let environ = fs::read(proc_path.join("environ")).ok()?;
    linux_yggterm_home_from_environ_bytes(&environ)
}

#[cfg(target_os = "linux")]
fn linux_yggterm_server_process_is_live_bridge(parts: &[String]) -> bool {
    parts.first().is_some_and(|argv0| argv0.contains("yggterm"))
        && parts.windows(3).any(|window| {
            window[0] == "server"
                && window[1] == "remote"
                && (window[2] == "resume-codex" || window[2] == "start-codex")
        })
}

#[cfg(target_os = "linux")]
fn linux_home_has_live_bridge_process(home: &Path) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else {
            continue;
        };
        if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let proc_path = entry.path();
        if linux_proc_state(&proc_path) == Some('Z') {
            continue;
        }
        if linux_proc_yggterm_home(&proc_path).as_deref() != Some(home) {
            continue;
        }
        let Ok(bytes) = fs::read(proc_path.join("cmdline")) else {
            continue;
        };
        let parts = bytes
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).to_string())
            .collect::<Vec<_>>();
        if linux_yggterm_server_process_is_live_bridge(&parts) {
            return true;
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn linux_preserved_owner_process_ids_for_home(home: Option<&Path>) -> HashSet<u32> {
    let mut owner_pids = HashSet::new();
    let Some(home) = home else {
        return owner_pids;
    };
    let registry = PreservedTerminalOwnerRegistry::load(home);
    let mut probed_endpoints = HashSet::<String>::new();
    for entry in registry.entries {
        if entry.owner_server_pid > 0 {
            owner_pids.insert(entry.owner_server_pid);
        }
        if !probed_endpoints.insert(entry.endpoint.label()) {
            continue;
        }
        if let Ok(status) = status(&entry.endpoint.to_endpoint()) {
            if status.server_pid > 0 {
                owner_pids.insert(status.server_pid);
            }
        }
    }
    owner_pids
}

#[cfg(target_os = "linux")]
fn linux_preserved_owner_runtime_keys_for_home(home: Option<&Path>) -> HashSet<String> {
    let Some(home) = home else {
        return HashSet::new();
    };
    PreservedTerminalOwnerRegistry::load(home)
        .entries
        .into_iter()
        .map(|entry| entry.runtime_key)
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_daemon_runtime_activity_protected_for_cleanup(
    pid: u32,
    runtime_status: Option<&ServerRuntimeStatus>,
    preserved_owner_pids: &HashSet<u32>,
    preserved_owner_runtime_keys: &HashSet<String>,
    owner_registry_guard_active: bool,
    clean_preserved_owner_available: bool,
    current_version_terminal_keys: &HashSet<String>,
    home_has_live_bridge_process: bool,
) -> bool {
    let duplicate_current_runtime = runtime_status.is_some_and(|status| {
        linux_runtime_status_duplicates_current_version(status, current_version_terminal_keys)
    });
    if owner_registry_guard_active {
        if duplicate_current_runtime {
            return false;
        }
        if preserved_owner_pids.contains(&pid) {
            return true;
        }
        if clean_preserved_owner_available {
            return false;
        }
        return runtime_status.is_some_and(|status| {
            if !status.owned_terminal_session_keys.is_empty() {
                status
                    .owned_terminal_session_keys
                    .iter()
                    .any(|key| preserved_owner_runtime_keys.contains(key))
            } else {
                status.owned_terminal_session_count > 0
            }
        });
    }
    if let Some(status) = runtime_status {
        let has_terminal_runtime = status.terminal_session_count > 0
            || !status.terminal_session_keys.is_empty()
            || status.owned_terminal_session_count > 0
            || !status.owned_terminal_session_keys.is_empty();
        if has_terminal_runtime {
            return !duplicate_current_runtime;
        }
    }
    home_has_live_bridge_process
}

#[cfg(target_os = "linux")]
fn linux_runtime_status_duplicates_current_version(
    status: &ServerRuntimeStatus,
    current_version_terminal_keys: &HashSet<String>,
) -> bool {
    status.server_version != SERVER_PROTOCOL_VERSION
        && !status.owned_terminal_session_keys.is_empty()
        && status
            .owned_terminal_session_keys
            .iter()
            .all(|key| current_version_terminal_keys.contains(key))
}

#[cfg(target_os = "linux")]
fn linux_current_version_terminal_keys_for_cleanup(
    runtime_status_by_pid: &HashMap<u32, ServerRuntimeStatus>,
) -> HashSet<String> {
    runtime_status_by_pid
        .values()
        .filter(|status| status.server_version == SERVER_PROTOCOL_VERSION)
        .flat_map(|status| status.owned_terminal_session_keys.iter().cloned())
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_cleanup_has_clean_preserved_owner(
    runtime_status_by_pid: &HashMap<u32, ServerRuntimeStatus>,
    preserved_owner_pids: &HashSet<u32>,
    preserved_owner_runtime_keys: &HashSet<String>,
) -> bool {
    if preserved_owner_runtime_keys.is_empty() {
        return false;
    }
    preserved_owner_pids.iter().any(|pid| {
        let Some(status) = runtime_status_by_pid.get(pid) else {
            return false;
        };
        if status.terminal_session_keys.is_empty() {
            return false;
        }
        let terminal_keys = status
            .terminal_session_keys
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        terminal_keys.len() == preserved_owner_runtime_keys.len()
            && preserved_owner_runtime_keys
                .iter()
                .all(|key| terminal_keys.contains(key))
            && status
                .owned_terminal_session_keys
                .iter()
                .all(|key| preserved_owner_runtime_keys.contains(key))
            && status
                .preserved_terminal_owner_keys
                .iter()
                .all(|key| preserved_owner_runtime_keys.contains(key))
    })
}

#[cfg(target_os = "linux")]
fn linux_cleanup_startup_bridge_sidecar_pid(
    runtime_status_by_pid: &HashMap<u32, ServerRuntimeStatus>,
    preserved_owner_pids: &HashSet<u32>,
    preserved_owner_runtime_keys: &HashSet<String>,
    current_pid: u32,
) -> Option<u32> {
    if preserved_owner_runtime_keys.is_empty() {
        return None;
    }
    if linux_cleanup_has_clean_preserved_owner(
        runtime_status_by_pid,
        preserved_owner_pids,
        preserved_owner_runtime_keys,
    ) {
        return None;
    }
    runtime_status_by_pid
        .iter()
        .filter(|(pid, status)| {
            **pid != current_pid
                && !preserved_owner_pids.contains(pid)
                && status.server_version != SERVER_PROTOCOL_VERSION
                && status.owned_terminal_session_count == 0
                && status.owned_terminal_session_keys.is_empty()
                && !status.terminal_session_keys.is_empty()
                && status
                    .terminal_session_keys
                    .iter()
                    .all(|key| preserved_owner_runtime_keys.contains(key))
                && status
                    .preserved_terminal_owner_keys
                    .iter()
                    .all(|key| preserved_owner_runtime_keys.contains(key))
        })
        .max_by_key(|(pid, status)| {
            (
                daemon_status_version_triplet_for_cleanup(&status.server_version),
                status.server_build_id,
                **pid,
            )
        })
        .map(|(pid, _)| *pid)
}

#[cfg(target_os = "linux")]
fn daemon_status_version_triplet_for_cleanup(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let patch = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    (major, minor, patch)
}

#[cfg(target_os = "linux")]
fn linux_proc_state(proc_path: &Path) -> Option<char> {
    let stat = fs::read_to_string(proc_path.join("stat")).ok()?;
    stat.split_once(") ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .and_then(|state| state.chars().next())
}

#[cfg(target_os = "linux")]
fn terminate_linux_process(pid: u32) {
    unsafe {
        let _ = libc::kill(pid as i32, libc::SIGTERM);
    }
    std::thread::sleep(std::time::Duration::from_millis(120));
    if Path::new(&format!("/proc/{pid}")).exists() {
        unsafe {
            let _ = libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_socket_inode(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.ino())
}

#[cfg(target_os = "linux")]
fn cleanup_legacy_linux_daemon_processes(
    endpoint: &ServerEndpoint,
    current_exe: &Path,
) -> Result<()> {
    let current_home = daemon_logical_home_for_endpoint(endpoint);
    let reap_after_ms = daemon_orphan_reap_after_ms();
    let preserved_owner_pids = linux_preserved_owner_process_ids_for_home(current_home.as_deref());
    let preserved_owner_runtime_keys =
        linux_preserved_owner_runtime_keys_for_home(current_home.as_deref());
    let owner_registry_guard_active =
        !preserved_owner_pids.is_empty() || !preserved_owner_runtime_keys.is_empty();
    let mut runtime_status_by_pid = HashMap::<u32, ServerRuntimeStatus>::new();
    let mut endpoint_by_pid = HashMap::<u32, ServerEndpoint>::new();
    if let Some(home) = current_home.as_deref() {
        for (status_endpoint, status) in reachable_versioned_daemon_statuses(home) {
            if status.server_pid == 0 {
                continue;
            }
            endpoint_by_pid.insert(status.server_pid, status_endpoint);
            runtime_status_by_pid.insert(status.server_pid, status);
        }
    }
    let startup_bridge_sidecar_pid = linux_cleanup_startup_bridge_sidecar_pid(
        &runtime_status_by_pid,
        &preserved_owner_pids,
        &preserved_owner_runtime_keys,
        std::process::id(),
    );
    let clean_preserved_owner_available = linux_cleanup_has_clean_preserved_owner(
        &runtime_status_by_pid,
        &preserved_owner_pids,
        &preserved_owner_runtime_keys,
    );
    let current_version_terminal_keys =
        linux_current_version_terminal_keys_for_cleanup(&runtime_status_by_pid);
    let current_socket_inode = match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => linux_socket_inode(path),
        ServerEndpoint::Tcp { .. } => None,
    };
    let (same_home_daemon_count, oldest_same_home_pid) = if let Some(home) = current_home.as_deref()
    {
        let mut count = 0usize;
        let mut oldest_pid = None::<(u64, u32)>;
        let entries =
            fs::read_dir("/proc").context("reading /proc for same-home daemon counting")?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid_str) = name.to_str() else {
                continue;
            };
            if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }
            let proc_path = entry.path();
            if linux_proc_state(&proc_path) == Some('Z') {
                continue;
            }
            let Ok(bytes) = fs::read(proc_path.join("cmdline")) else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            let parts = bytes
                .split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part).to_string())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                continue;
            }
            let is_daemon = parts[0].contains("yggterm")
                && parts.iter().any(|part| part == "server")
                && parts.iter().any(|part| part == "daemon");
            if !is_daemon {
                continue;
            }
            if linux_proc_yggterm_home(&proc_path).as_deref() == Some(home) {
                count += 1;
                let age_ms =
                    linux_proc_pid_age_ms(pid_str.parse::<u32>().unwrap_or(0)).unwrap_or_default();
                let pid = pid_str.parse::<u32>().unwrap_or(0);
                if pid > 0
                    && oldest_pid.is_none_or(|(oldest_age_ms, oldest_pid_value)| {
                        age_ms > oldest_age_ms
                            || (age_ms == oldest_age_ms && pid < oldest_pid_value)
                    })
                {
                    oldest_pid = Some((age_ms, pid));
                }
            }
        }
        (count, oldest_pid.map(|(_, pid)| pid))
    } else {
        (0, None)
    };
    let mut daemon_candidates = 0usize;
    let mut killed_legacy = 0usize;
    let mut killed_orphan = 0usize;
    let mut killed_duplicate_same_home = 0usize;
    let mut skipped_active_client_legacy = 0usize;
    let mut skipped_runtime_activity_legacy = 0usize;
    let mut skipped_startup_bridge_sidecar_legacy = 0usize;
    let mut skipped_cross_home_orphan = 0usize;
    let proc_entries = fs::read_dir("/proc").context("reading /proc for stale daemon cleanup")?;
    for entry in proc_entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else {
            continue;
        };
        if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let pid = pid_str.parse::<u32>().unwrap_or(0);
        if pid == std::process::id() {
            continue;
        }
        let proc_path = entry.path();
        if linux_proc_state(&proc_path) == Some('Z') {
            continue;
        }
        let proc_exe_target = fs::read_link(proc_path.join("exe")).ok();
        let cmdline_path = entry.path().join("cmdline");
        let Ok(bytes) = fs::read(&cmdline_path) else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let parts = bytes
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).to_string())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        let argv0 = &parts[0];
        let is_daemon = argv0.contains("yggterm")
            && parts.iter().any(|part| part == "server")
            && parts.iter().any(|part| part == "daemon");
        if !is_daemon {
            continue;
        }
        daemon_candidates += 1;
        let daemon_home = linux_proc_yggterm_home(&proc_path);
        let candidate_endpoint = endpoint_by_pid
            .get(&pid)
            .cloned()
            .or_else(|| daemon_home.as_deref().map(default_endpoint));
        let has_gui_clients = daemon_home
            .as_deref()
            .zip(candidate_endpoint.as_ref())
            .and_then(|(home, endpoint)| {
                active_client_instance_records_for_endpoint_scope(home, endpoint).ok()
            })
            .is_some_and(|records| !records.is_empty());
        let home_has_live_bridge_process = daemon_home
            .as_deref()
            .is_some_and(linux_home_has_live_bridge_process);
        let has_recoverable_runtime_activity = linux_daemon_runtime_activity_protected_for_cleanup(
            pid,
            runtime_status_by_pid.get(&pid),
            &preserved_owner_pids,
            &preserved_owner_runtime_keys,
            owner_registry_guard_active,
            clean_preserved_owner_available,
            &current_version_terminal_keys,
            home_has_live_bridge_process,
        );
        let is_startup_bridge_sidecar = startup_bridge_sidecar_pid == Some(pid);
        let has_clients =
            has_gui_clients || has_recoverable_runtime_activity || is_startup_bridge_sidecar;
        let age_ms = linux_proc_pid_age_ms(pid).unwrap_or_default();
        let is_legacy_binary =
            legacy_daemon_reap_applies_to_home(current_home.as_deref(), daemon_home.as_deref())
                && daemon_binary_is_legacy(current_exe, argv0, proc_exe_target.as_deref());
        let is_legacy_reapable = is_legacy_binary && !has_clients;
        if is_legacy_binary && has_gui_clients {
            skipped_active_client_legacy += 1;
        }
        if is_legacy_binary && !has_gui_clients && has_recoverable_runtime_activity {
            skipped_runtime_activity_legacy += 1;
        }
        if is_legacy_binary && is_startup_bridge_sidecar {
            skipped_startup_bridge_sidecar_legacy += 1;
        }
        let is_cross_home_orphan_candidate = !has_clients
            && age_ms >= reap_after_ms
            && daemon_home.as_deref() != current_home.as_deref();
        if is_cross_home_orphan_candidate {
            skipped_cross_home_orphan += 1;
        }
        let is_orphan_clientless = !has_clients
            && age_ms >= reap_after_ms
            && orphan_daemon_reap_applies_to_home(current_home.as_deref(), daemon_home.as_deref());
        let is_duplicate_same_home = same_home_daemon_count > 1
            && daemon_home.as_deref() == current_home.as_deref()
            && Some(pid) != oldest_same_home_pid
            && age_ms >= DUPLICATE_SAME_HOME_GRACE_MS
            && !has_clients;
        if is_legacy_reapable || is_orphan_clientless || is_duplicate_same_home {
            terminate_linux_process(pid);
            if is_legacy_reapable {
                killed_legacy += 1;
            } else if is_duplicate_same_home {
                killed_duplicate_same_home += 1;
            } else {
                killed_orphan += 1;
            }
        }
    }
    if let Some(home) = current_home.as_deref() {
        append_trace_event(
            home,
            "daemon",
            "cleanup",
            "linux_daemon_sweep",
            serde_json::json!({
                "candidates": daemon_candidates,
                "killed_legacy": killed_legacy,
                "killed_orphan": killed_orphan,
                "killed_duplicate_same_home": killed_duplicate_same_home,
                "skipped_active_client_legacy": skipped_active_client_legacy,
                "skipped_runtime_activity_legacy": skipped_runtime_activity_legacy,
                "skipped_startup_bridge_sidecar_legacy": skipped_startup_bridge_sidecar_legacy,
                "skipped_cross_home_orphan": skipped_cross_home_orphan,
                "same_home_daemon_count": same_home_daemon_count,
                "oldest_same_home_pid": oldest_same_home_pid,
                "reap_after_ms": reap_after_ms,
                "current_socket_inode": current_socket_inode,
                "preserved_owner_guard_active": owner_registry_guard_active,
                "clean_preserved_owner_available": clean_preserved_owner_available,
                "preserved_owner_pid_count": preserved_owner_pids.len(),
                "preserved_owner_runtime_key_count": preserved_owner_runtime_keys.len(),
                "startup_bridge_sidecar_pid": startup_bridge_sidecar_pid,
            }),
        );
    }
    Ok(())
}

fn expect_snapshot(response: ServerResponse) -> Result<(ServerUiSnapshot, Option<String>)> {
    match response {
        ServerResponse::Snapshot { snapshot, message } => Ok((snapshot, message)),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected snapshot response: {:?}", other),
    }
}

fn expect_ack(response: ServerResponse) -> Result<Option<String>> {
    match response {
        ServerResponse::Ack { message } => Ok(message),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected ack response: {:?}", other),
    }
}

fn load_persisted_state(path: &Path) -> Result<Option<PersistedDaemonState>> {
    if !path.exists() {
        return Ok(None);
    }
    let json = fs::read_to_string(path)
        .with_context(|| format!("reading daemon state {}", path.display()))?;
    let state = serde_json::from_str(&json)
        .with_context(|| format!("parsing daemon state {}", path.display()))?;
    Ok(Some(state))
}

fn write_persisted_state(path: &Path, state: &PersistedDaemonState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating daemon state dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(state).context("serializing daemon state")?;
    if path.exists() {
        let backup_path = path.with_file_name(format!(
            "{}.previous.json",
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("server-state")
        ));
        if let Err(error) = fs::copy(path, &backup_path) {
            tracing::warn!(
                path = %path.display(),
                backup_path = %backup_path.display(),
                error = %error,
                "failed to back up daemon state before overwrite"
            );
        }
    }
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, json)
        .with_context(|| format!("writing daemon state temp file {}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .or_else(|_| {
            fs::copy(&temp_path, path)?;
            fs::remove_file(&temp_path)?;
            Ok::<(), std::io::Error>(())
        })
        .with_context(|| format!("writing daemon state {}", path.display()))?;
    Ok(())
}

fn canonical_hot_restart_executable(raw_path: &str) -> Result<PathBuf> {
    if raw_path.trim().is_empty() {
        bail!("hot restart daemon executable path is empty");
    }
    let path = PathBuf::from(raw_path);
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving hot restart executable {}", path.display()))?;
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("reading hot restart executable {}", canonical.display()))?;
    if !metadata.is_file() {
        bail!(
            "hot restart executable is not a regular file: {}",
            canonical.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            bail!(
                "hot restart executable is not marked executable: {}",
                canonical.display()
            );
        }
    }
    Ok(canonical)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonRequestOutcome {
    should_shutdown: bool,
    restart_executable: Option<PathBuf>,
}

fn hot_restart_should_defer_for_session_survival(owned_terminal_session_keys: &[String]) -> bool {
    // The daemon only has to stay alive during a hot update when it owns PTY
    // file descriptors. Preserved-owner entries already point at another
    // daemon, and restored metadata alone is not a runtime that this process
    // can keep alive.
    !owned_terminal_session_keys.is_empty()
}

fn runtime_status_owns_all_keys(status: &ServerRuntimeStatus, runtime_keys: &[String]) -> bool {
    if runtime_keys.is_empty() {
        return false;
    }
    let terminal_keys = status
        .owned_terminal_session_keys
        .iter()
        .collect::<HashSet<_>>();
    runtime_keys
        .iter()
        .all(|runtime_key| terminal_keys.contains(runtime_key))
}

fn hot_restart_duplicate_runtime_owner_status_from_statuses<I>(
    expected_version: Option<&str>,
    owned_terminal_session_keys: &[String],
    current_pid: u32,
    statuses: I,
) -> Option<ServerRuntimeStatus>
where
    I: IntoIterator<Item = ServerRuntimeStatus>,
{
    let expected_version = expected_version?;
    statuses
        .into_iter()
        .filter(|status| {
            status.server_pid != current_pid
                && status.server_version == expected_version
                && runtime_status_owns_all_keys(status, owned_terminal_session_keys)
        })
        .max_by_key(|status| (status.server_build_id, status.server_pid))
}

fn hot_restart_duplicate_runtime_owner_status(
    home: &Path,
    expected_version: Option<&str>,
    owned_terminal_session_keys: &[String],
    current_pid: u32,
) -> Option<ServerRuntimeStatus> {
    hot_restart_duplicate_runtime_owner_status_from_statuses(
        expected_version,
        owned_terminal_session_keys,
        current_pid,
        reachable_versioned_daemon_statuses(home)
            .into_iter()
            .map(|(_, status)| status),
    )
}

fn daemon_request_outcome_for_response(
    request: &ServerRequest,
    response: &ServerResponse,
) -> DaemonRequestOutcome {
    if matches!(response, ServerResponse::Error { .. }) {
        return DaemonRequestOutcome {
            should_shutdown: false,
            restart_executable: None,
        };
    }
    if matches!(response, ServerResponse::HotUpdateHandoff { .. }) {
        return DaemonRequestOutcome {
            should_shutdown: false,
            restart_executable: None,
        };
    }

    match request {
        ServerRequest::Shutdown => DaemonRequestOutcome {
            should_shutdown: true,
            restart_executable: None,
        },
        ServerRequest::HotRestart {
            daemon_executable, ..
        } => DaemonRequestOutcome {
            should_shutdown: true,
            restart_executable: Some(
                canonical_hot_restart_executable(daemon_executable)
                    .unwrap_or_else(|_| PathBuf::from(daemon_executable)),
            ),
        },
        _ => DaemonRequestOutcome {
            should_shutdown: false,
            restart_executable: None,
        },
    }
}

fn write_response<W: Write>(writer: &mut W, response: &ServerResponse) -> Result<()> {
    serde_json::to_writer(&mut *writer, response).context("serializing daemon response")?;
    writer
        .write_all(b"\n")
        .context("writing daemon response terminator")?;
    writer.flush().context("flushing daemon response")?;
    Ok(())
}

fn read_request<R: std::io::Read>(reader: R) -> Result<ServerRequest> {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .context("reading daemon request")?;
    if bytes == 0 {
        bail!("daemon client closed connection before sending a request");
    }
    serde_json::from_str(line.trim_end()).context("parsing daemon request")
}

#[cfg(unix)]
fn handle_unix_stream(
    mut stream: std::os::unix::net::UnixStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
) -> Result<DaemonRequestOutcome> {
    let request = read_unix_request_with_timeout(
        &stream,
        std::time::Duration::from_millis(DAEMON_CLIENT_REQUEST_READ_TIMEOUT_MS),
    )?;
    mark_daemon_activity(last_activity_ms.as_ref());
    let response = daemon_request_response(&runtime, request.clone());
    let outcome = daemon_request_outcome_for_response(&request, &response);
    write_response(&mut stream, &response)?;
    trim_process_heap_if_supported();
    Ok(outcome)
}

#[cfg(unix)]
fn read_unix_request_with_timeout(
    stream: &std::os::unix::net::UnixStream,
    timeout: std::time::Duration,
) -> Result<ServerRequest> {
    wait_for_unix_daemon_client_request(stream, timeout)?;
    let read_stream = stream.try_clone().context("cloning unix stream")?;
    configure_unix_daemon_client_read_timeout(&read_stream, timeout)?;
    read_request(read_stream)
}

#[cfg(unix)]
fn configure_unix_daemon_client_read_timeout(
    stream: &std::os::unix::net::UnixStream,
    timeout: std::time::Duration,
) -> Result<()> {
    stream
        .set_read_timeout(Some(timeout))
        .context("setting daemon unix client read timeout")
}

#[cfg(unix)]
fn wait_for_unix_daemon_client_request(
    stream: &std::os::unix::net::UnixStream,
    timeout: std::time::Duration,
) -> Result<()> {
    let mut pollfd = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
        revents: 0,
    };
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    loop {
        // SAFETY: `pollfd` points to valid memory for one connected Unix stream descriptor.
        let rc = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if rc > 0 {
            if pollfd.revents & libc::POLLERR != 0 {
                bail!("daemon client socket reported an error before sending a request");
            }
            if pollfd.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
                return Ok(());
            }
            continue;
        }
        if rc == 0 {
            bail!("timed out waiting for daemon client request bytes");
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == ErrorKind::Interrupted {
            continue;
        }
        return Err(error).context("polling daemon client request");
    }
}

fn handle_tcp_stream(
    mut stream: std::net::TcpStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
) -> Result<DaemonRequestOutcome> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(
            DAEMON_CLIENT_REQUEST_READ_TIMEOUT_MS,
        )))
        .context("setting daemon tcp client read timeout")?;
    let request = read_request(stream.try_clone().context("cloning tcp stream")?)?;
    mark_daemon_activity(last_activity_ms.as_ref());
    let response = daemon_request_response(&runtime, request.clone());
    let outcome = daemon_request_outcome_for_response(&request, &response);
    write_response(&mut stream, &response)?;
    trim_process_heap_if_supported();
    Ok(outcome)
}

#[cfg(target_os = "linux")]
fn trim_process_heap_if_supported() {
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(not(target_os = "linux"))]
fn trim_process_heap_if_supported() {}

fn send_request(endpoint: &ServerEndpoint, request: &ServerRequest) -> Result<ServerResponse> {
    let mut request_bytes =
        serde_json::to_vec(request).context("serializing daemon request payload")?;
    request_bytes.push(b'\n');
    let io_timeout = Some(std::time::Duration::from_millis(
        daemon_request_io_timeout_ms(request),
    ));
    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => {
            let mut stream = std::os::unix::net::UnixStream::connect(path)
                .with_context(|| format!("connecting to {}", path.display()))?;
            stream
                .set_read_timeout(io_timeout)
                .context("setting daemon request read timeout")?;
            stream
                .set_write_timeout(io_timeout)
                .context("setting daemon request write timeout")?;
            stream
                .write_all(&request_bytes)
                .context("writing daemon request")?;
            stream.flush().context("flushing daemon request")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .context("reading daemon response")?;
            serde_json::from_str(line.trim_end()).with_context(|| {
                let trimmed = line.trim_end();
                let snippet = if trimmed.len() > 240 {
                    format!("{}...", &trimmed[..240])
                } else {
                    trimmed.to_string()
                };
                format!("parsing daemon response: {:?}", snippet)
            })
        }
        ServerEndpoint::Tcp { host, port } => {
            let mut stream = std::net::TcpStream::connect((host.as_str(), *port))
                .with_context(|| format!("connecting to {}:{}", host, port))?;
            stream
                .set_read_timeout(io_timeout)
                .context("setting daemon request read timeout")?;
            stream
                .set_write_timeout(io_timeout)
                .context("setting daemon request write timeout")?;
            stream
                .write_all(&request_bytes)
                .context("writing daemon request")?;
            stream.flush().context("flushing daemon request")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .context("reading daemon response")?;
            serde_json::from_str(line.trim_end()).with_context(|| {
                let trimmed = line.trim_end();
                let snippet = if trimmed.len() > 240 {
                    format!("{}...", &trimmed[..240])
                } else {
                    trimmed.to_string()
                };
                format!("parsing daemon response: {:?}", snippet)
            })
        }
    }
}

fn daemon_request_io_timeout_ms(request: &ServerRequest) -> u64 {
    match request {
        ServerRequest::StartSshSession { .. }
        | ServerRequest::StartRemoteCodexSession { .. }
        | ServerRequest::OpenRemoteSession { .. }
        | ServerRequest::EnsureRemoteRuntimeCodexSession { .. }
        | ServerRequest::StartRemoteRuntimeCodexSession { .. }
        | ServerRequest::TerminalRestart {
            force_remote: true, ..
        } => DAEMON_LONG_REQUEST_IO_TIMEOUT_MS,
        _ => DAEMON_REQUEST_IO_TIMEOUT_MS,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalWriteStrategy {
    LocalRuntime,
    RemoteDirectFallback,
    LocalRuntimeFallback,
}

fn terminal_write_strategy_for_path(
    path: &str,
    local_runtime_running: bool,
) -> TerminalWriteStrategy {
    if local_runtime_running {
        TerminalWriteStrategy::LocalRuntime
    } else if path.trim_start().starts_with("remote-session://") {
        TerminalWriteStrategy::RemoteDirectFallback
    } else {
        TerminalWriteStrategy::LocalRuntimeFallback
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RemoteMachineRefreshQueueStatus, SERVER_PROTOCOL_VERSION,
        apply_terminal_runtime_truth_to_snapshot, daemon_background_copy_chore_enabled_from_env,
        mark_remote_machine_refresh_queued, terminal_sidebar_snapshot_from_screen,
    };
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::collect_remote_copy_candidates;
    #[cfg(target_os = "linux")]
    use super::legacy_daemon_reap_applies_to_home;
    #[cfg(target_os = "linux")]
    use super::linux_cleanup_startup_bridge_sidecar_pid;
    #[cfg(target_os = "linux")]
    use super::linux_daemon_runtime_activity_protected_for_cleanup;
    #[cfg(target_os = "linux")]
    use super::linux_yggterm_home_from_environ_bytes;
    use super::load_persisted_state;
    #[cfg(target_os = "linux")]
    use super::orphan_daemon_reap_applies_to_home;
    use super::terminal_launch_command_for_path;
    use super::write_persisted_state;
    use super::{
        REMOTE_ATTACH_STARTUP_GRACE_MS, REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS,
        remote_resume_stale_attach,
    };
    #[cfg(unix)]
    use super::{
        cleanup_legacy_unix_daemons, configure_unix_daemon_client_read_timeout,
        daemon_binary_is_legacy, default_endpoint, parse_versioned_server_socket_name,
        read_request, read_unix_request_with_timeout, unix_socket_path_fits_platform,
        versioned_server_socket_alias_candidates, versioned_socket_alias_is_legacy,
        versioned_socket_alias_points_to_current, versioned_socket_candidate_is_symlink,
        wait_for_unix_daemon_client_request,
    };
    use crate::{
        GhosttyHostSupport, PersistedDaemonState, PersistedLiveSession, RemoteDeployState,
        RemoteMachineHealth, RemoteMachineSnapshot, RemoteScannedSession, ServerRuntimeStatus,
        ServerUiSnapshot, SessionKind, SessionSource, SnapshotPreview, SnapshotSessionView,
        TerminalBackend, TerminalLaunchPhase, WorkspaceViewMode, YggtermServer,
        remote_scanned_session_path,
    };
    use yggui_contract::UiTheme;

    #[test]
    fn terminal_stream_responses_default_resize_fence_fields_for_hot_update() {
        let response: super::ServerResponse = serde_json::from_str(
            r#"{
                "kind": "terminal_stream",
                "cursor": 7,
                "chunks": [],
                "running": true,
                "runtime_output_seen": true,
                "eof_without_output": false
            }"#,
        )
        .expect("new clients must deserialize old daemon terminal_stream responses");
        let super::ServerResponse::TerminalStream {
            post_resize_output_seen,
            last_resize_seq,
            ..
        } = response
        else {
            panic!("expected terminal_stream response");
        };
        assert!(!post_resize_output_seen);
        assert_eq!(last_resize_seq, 0);

        let response: super::ServerResponse = serde_json::from_str(
            r#"{
                "kind": "terminal_snapshot",
                "text": "",
                "running": true,
                "runtime_output_seen": true
            }"#,
        )
        .expect("new clients must deserialize old daemon terminal_snapshot responses");
        let super::ServerResponse::TerminalSnapshot {
            post_resize_output_seen,
            last_resize_seq,
            ..
        } = response
        else {
            panic!("expected terminal_snapshot response");
        };
        assert!(!post_resize_output_seen);
        assert_eq!(last_resize_seq, 0);

        let response: super::ServerResponse = serde_json::from_str(
            r#"{
                "kind": "terminal_retained_snapshot",
                "text": "",
                "running": true,
                "runtime_output_seen": true
            }"#,
        )
        .expect("new clients must deserialize old daemon terminal_retained_snapshot responses");
        let super::ServerResponse::TerminalRetainedSnapshot {
            post_resize_output_seen,
            last_resize_seq,
            ..
        } = response
        else {
            panic!("expected terminal_retained_snapshot response");
        };
        assert!(!post_resize_output_seen);
        assert_eq!(last_resize_seq, 0);
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_client_read_times_out_when_peer_sends_no_request() {
        let (server, _client) = std::os::unix::net::UnixStream::pair().expect("unix stream pair");
        configure_unix_daemon_client_read_timeout(&server, std::time::Duration::from_millis(50))
            .expect("set read timeout");

        let started = std::time::Instant::now();
        let error = read_request(server).expect_err("silent client should time out");

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "silent local clients must not block the daemon accept loop indefinitely"
        );
        let message = format!("{error:#}");
        assert!(
            message.contains("reading daemon request"),
            "timeout should be reported as a daemon request read failure: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_client_poll_times_out_before_request_read() {
        let (server, _client) = std::os::unix::net::UnixStream::pair().expect("unix stream pair");
        let started = std::time::Instant::now();
        let error =
            wait_for_unix_daemon_client_request(&server, std::time::Duration::from_millis(50))
                .expect_err("silent client should time out before read_request");

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "silent local clients must not monopolize the synchronous accept loop"
        );
        let message = format!("{error:#}");
        assert!(
            message.contains("timed out waiting for daemon client request bytes"),
            "poll timeout should name the accept-loop request-byte guard: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_client_poll_accepts_buffered_request_bytes() {
        let (server, mut client) =
            std::os::unix::net::UnixStream::pair().expect("unix stream pair");
        client
            .write_all(b"{\"kind\":\"ping\"}\n")
            .expect("write request bytes");

        wait_for_unix_daemon_client_request(&server, std::time::Duration::from_millis(250))
            .expect("buffered request bytes should be readable");
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_client_partial_request_times_out_after_poll() {
        let (server, mut client) =
            std::os::unix::net::UnixStream::pair().expect("unix stream pair");
        client
            .write_all(b"{\"kind\":\"ping\"")
            .expect("write partial request bytes");

        let started = std::time::Instant::now();
        let error = read_unix_request_with_timeout(&server, std::time::Duration::from_millis(50))
            .expect_err("partial request should time out instead of wedging read_line");

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "partial readable requests must not monopolize the synchronous accept loop"
        );
        let message = format!("{error:#}");
        assert!(
            message.contains("reading daemon request"),
            "partial request timeout should be reported as a daemon request read failure: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_accept_loop_dispatches_clients_off_accept_thread() {
        let source = include_str!("daemon.rs");
        let unix_loop = source
            .split("if let ServerEndpoint::UnixSocket(path) = endpoint {")
            .nth(1)
            .and_then(|suffix| suffix.split("if let ServerEndpoint::Tcp").next())
            .expect("unix daemon accept loop should be present");

        assert!(
            unix_loop.contains("std::sync::mpsc::channel::<Result<DaemonRequestOutcome>>()"),
            "unix accept loop must collect request outcomes asynchronously"
        );
        assert!(
            unix_loop.contains("drain_unix_client_outcomes("),
            "unix accept loop must drain completed client outcomes without blocking accept"
        );
        assert!(
            unix_loop.contains("spawn_unix_client_handler("),
            "unix accept loop must dispatch each accepted client away from the accept thread"
        );
        assert!(
            !unix_loop.contains("match handle_unix_stream(stream"),
            "unix accept loop must not let one partial client monopolize the daemon"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refresh_legacy_socket_aliases_skips_self_alias_without_ping_deadlock() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-self-alias-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp socket dir");
        let current = root.join("server-2-1-214.sock");
        let legacy_alias = root.join("server-2-1-213.sock");
        let _listener =
            std::os::unix::net::UnixListener::bind(&current).expect("bind current socket");
        std::os::unix::fs::symlink(&current, &legacy_alias).expect("create self alias");

        assert!(versioned_socket_alias_points_to_current(
            &legacy_alias,
            &current
        ));
        let started = std::time::Instant::now();
        super::refresh_legacy_server_socket_aliases(&current);

        assert!(
            started.elapsed() < std::time::Duration::from_millis(250),
            "startup alias refresh must not ping a symlink that resolves to the current listener"
        );
        assert_eq!(
            fs::read_link(&legacy_alias).expect("self alias should remain"),
            current
        );
        fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn refresh_legacy_socket_aliases_retargets_symlink_aliases_without_pinging_target() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-stale-alias-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp socket dir");
        let current = root.join("server-2-1-215.sock");
        let stale = root.join("dead-target.sock");
        let stale_alias = root.join("server-2-1-213.sock");
        let _current_listener =
            std::os::unix::net::UnixListener::bind(&current).expect("bind current socket");
        std::os::unix::fs::symlink(&stale, &stale_alias).expect("create stale alias");

        assert!(versioned_socket_candidate_is_symlink(&stale_alias));
        let started = std::time::Instant::now();
        super::refresh_legacy_server_socket_aliases(&current);

        assert!(
            started.elapsed() < std::time::Duration::from_millis(250),
            "startup alias refresh must retarget symlink aliases without pinging a stale target"
        );
        assert_eq!(
            fs::read_link(&stale_alias).expect("stale alias should be retargeted"),
            current
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn daemon_startup_defers_linux_cleanup_until_after_listener_bind() {
        let source = include_str!("daemon.rs");
        let run_daemon = source
            .split("pub fn run_daemon(endpoint: &ServerEndpoint, runtime: GhosttyHostSupport) -> Result<()> {")
            .nth(1)
            .expect("run_daemon should be present");
        let before_unix_bind = run_daemon
            .split("let listener = std::os::unix::net::UnixListener::bind(path)")
            .next()
            .expect("unix listener bind should be present");

        assert!(
            !before_unix_bind.contains("cleanup_legacy_linux_daemon_processes("),
            "daemon startup must not probe legacy daemon status before binding the current socket"
        );
        assert!(
            run_daemon.contains("spawn_legacy_linux_daemon_cleanup(endpoint.clone()"),
            "legacy daemon cleanup should run only after the daemon has become reachable"
        );

        let headless = include_str!("../../../apps/yggterm/src/bin/yggterm-headless.rs");
        let daemon_entrypoint = headless
            .split("if args.as_slice() == [\"server\", \"daemon\"] {")
            .nth(1)
            .and_then(|suffix| suffix.split("if args.len() >= 3").next())
            .expect("headless server daemon branch should be present");
        assert!(
            !daemon_entrypoint.contains("cleanup_legacy_daemons("),
            "headless daemon entrypoint must not run blocking legacy cleanup before run_daemon"
        );
    }

    #[test]
    fn force_remote_restart_schedules_daemon_cleanup_after_owner_takeover() {
        let source = include_str!("daemon.rs");
        let branch = source
            .split("ServerRequest::TerminalRestart {\n                path,")
            .nth(1)
            .and_then(|suffix| suffix.split("ServerRequest::SyncExternalWindow").next())
            .expect("terminal restart branch should be present");

        assert!(
            branch.contains("self.terminals.restart_session("),
            "terminal restart should still restart the requested runtime before cleanup"
        );
        assert!(
            branch.contains("spawn_force_remote_restart_daemon_cleanup("),
            "force-remote restart must rerun daemon cleanup after taking preserved sessions"
        );
        assert!(
            branch.find("self.terminals.restart_session(")
                < branch.find("spawn_force_remote_restart_daemon_cleanup("),
            "cleanup must run after the current daemon has replaced the runtime owner"
        );
    }

    #[test]
    fn update_session_copy_does_not_block_daemon_on_remote_copy_persist() {
        let source = include_str!("daemon.rs");
        let handler = source
            .split("ServerRequest::UpdateSessionCopy {\n                path,")
            .nth(1)
            .and_then(|suffix| suffix.split("ServerRequest::RemoveSshTarget").next())
            .expect("UpdateSessionCopy handler should be present");

        assert!(source.contains("fn spawn_remote_generated_copy_persist("));
        assert!(
            handler.contains("spawn_remote_generated_copy_persist("),
            "remote generated-copy persistence must be spawned away from the daemon request path"
        );
        assert!(
            !handler.contains("persist_remote_generated_copy("),
            "UpdateSessionCopy must not perform remote SSH copy persistence synchronously"
        );
    }

    fn daemon_test_tree() -> yggterm_core::SessionNode {
        yggterm_core::SessionNode {
            kind: yggterm_core::SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        }
    }

    fn daemon_test_snapshot_session(path: &str, source: SessionSource) -> SnapshotSessionView {
        SnapshotSessionView {
            id: path.rsplit('/').next().unwrap_or(path).to_string(),
            session_path: path.to_string(),
            title: path.rsplit('/').next().unwrap_or(path).to_string(),
            kind: SessionKind::Codex,
            host_label: "dev".to_string(),
            source,
            backend: TerminalBackend::Xterm,
            bridge_available: true,
            launch_phase: TerminalLaunchPhase::Running,
            remote_deploy_state: RemoteDeployState::Ready,
            launch_command: String::new(),
            status_line: String::new(),
            terminal_lines: Vec::new(),
            rendered_sections: Vec::new(),
            preview: SnapshotPreview {
                summary: Vec::new(),
                blocks: Vec::new(),
            },
            metadata: Vec::new(),
            terminal_process_id: None,
            terminal_foreground_active: None,
            terminal_window_id: None,
            terminal_host_token: None,
            terminal_host_mode: crate::GhosttyTerminalHostMode::Unsupported,
            embedded_surface_id: None,
            embedded_surface_detail: None,
            last_launch_error: None,
            last_window_error: None,
            ssh_target: Some("dev".to_string()),
            ssh_prefix: None,
        }
    }

    #[test]
    fn daemon_background_copy_chore_is_explicit_opt_in() {
        assert!(!daemon_background_copy_chore_enabled_from_env(None));
        assert!(!daemon_background_copy_chore_enabled_from_env(Some("")));
        assert!(!daemon_background_copy_chore_enabled_from_env(Some(
            "false"
        )));
        assert!(daemon_background_copy_chore_enabled_from_env(Some("1")));
        assert!(daemon_background_copy_chore_enabled_from_env(Some("true")));
        assert!(daemon_background_copy_chore_enabled_from_env(Some(" YES ")));
    }

    #[test]
    fn remote_machine_refresh_queue_coalesces_in_flight_targets() {
        let mut in_flight = HashSet::new();
        assert_eq!(
            mark_remote_machine_refresh_queued(&mut in_flight, "dev", true),
            RemoteMachineRefreshQueueStatus::Spawn
        );
        assert_eq!(
            mark_remote_machine_refresh_queued(&mut in_flight, "dev", true),
            RemoteMachineRefreshQueueStatus::AlreadyInFlight
        );
        assert_eq!(
            mark_remote_machine_refresh_queued(&mut in_flight, "local", false),
            RemoteMachineRefreshQueueStatus::Loopback
        );
    }

    #[test]
    fn daemon_snapshot_filters_live_rows_to_terminal_runtime_keys() {
        let tree = daemon_test_tree();
        let server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let live_sessions = (0..5)
            .map(|ix| {
                daemon_test_snapshot_session(
                    &remote_scanned_session_path("dev", &format!("runtime-{ix}")),
                    SessionSource::LiveSsh,
                )
            })
            .collect::<Vec<_>>();
        let runtime_keys = live_sessions
            .iter()
            .take(4)
            .map(|session| session.session_path.clone())
            .collect::<HashSet<_>>();
        let mut snapshot = ServerUiSnapshot {
            active_session_path: Some(live_sessions[0].session_path.clone()),
            active_session: Some(live_sessions[0].clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: vec![RemoteMachineSnapshot {
                machine_key: "dev".to_string(),
                label: "dev".to_string(),
                ssh_target: "dev".to_string(),
                prefix: None,
                remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
                remote_deploy_state: RemoteDeployState::Ready,
                health: RemoteMachineHealth::Healthy,
                sessions: (0..5)
                    .map(|ix| RemoteScannedSession {
                        session_path: remote_scanned_session_path("dev", &format!("runtime-{ix}")),
                        session_id: format!("runtime-{ix}"),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-05-04T00:00:00Z".to_string(),
                        modified_epoch: ix,
                        event_count: 1,
                        user_message_count: 1,
                        assistant_message_count: 0,
                        title_hint: format!("candidate {ix}"),
                        recent_context: String::new(),
                        cached_precis: None,
                        cached_summary: None,
                        live_runtime: true,
                        storage_path: format!("/home/pi/.codex/sessions/runtime-{ix}.jsonl"),
                    })
                    .collect(),
            }],
            ssh_targets: Vec::new(),
            live_sessions,
        };

        apply_terminal_runtime_truth_to_snapshot(&server, &runtime_keys, &mut snapshot);

        assert_eq!(snapshot.live_sessions.len(), 4);
        assert!(
            snapshot
                .live_sessions
                .iter()
                .all(|session| runtime_keys.contains(&session.session_path))
        );
    }

    #[test]
    fn daemon_snapshot_downgrades_restored_remote_live_without_runtime_to_preview() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let active_path = remote_scanned_session_path("dev", "missing-runtime");
        let remote_machine = RemoteMachineSnapshot {
            machine_key: "dev".to_string(),
            label: "dev".to_string(),
            ssh_target: "dev".to_string(),
            prefix: None,
            remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: vec![RemoteScannedSession {
                session_path: active_path.clone(),
                session_id: "missing-runtime".to_string(),
                cwd: "/home/pi".to_string(),
                started_at: "2026-05-04T00:00:00Z".to_string(),
                modified_epoch: 1,
                event_count: 1,
                user_message_count: 1,
                assistant_message_count: 0,
                title_hint: "Recovered preview".to_string(),
                recent_context: "USER: htop".to_string(),
                cached_precis: Some("preview only".to_string()),
                cached_summary: None,
                live_runtime: true,
                storage_path: "/home/pi/.codex/sessions/missing-runtime.jsonl".to_string(),
            }],
        };
        server.apply_snapshot(ServerUiSnapshot {
            active_session_path: None,
            active_session: None,
            active_view_mode: WorkspaceViewMode::Rendered,
            remote_machines: vec![remote_machine.clone()],
            ssh_targets: Vec::new(),
            live_sessions: Vec::new(),
        });
        let active = daemon_test_snapshot_session(&active_path, SessionSource::LiveSsh);
        let mut snapshot = ServerUiSnapshot {
            active_session_path: Some(active_path.clone()),
            active_session: Some(active.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: vec![remote_machine],
            ssh_targets: Vec::new(),
            live_sessions: vec![active],
        };

        apply_terminal_runtime_truth_to_snapshot(&server, &HashSet::new(), &mut snapshot);

        assert!(snapshot.live_sessions.is_empty());
        assert_eq!(snapshot.active_view_mode, WorkspaceViewMode::Rendered);
        assert_eq!(
            snapshot.active_session_path.as_deref(),
            Some(active_path.as_str())
        );
        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.source),
            Some(SessionSource::Stored)
        );
    }

    #[test]
    fn daemon_snapshot_preserves_keep_alive_remote_live_without_runtime_for_restore() {
        let tree = daemon_test_tree();
        let server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let active_path = remote_scanned_session_path("dev", "kept-runtime");
        let mut active = daemon_test_snapshot_session(&active_path, SessionSource::LiveSsh);
        active.metadata.push(crate::SnapshotMetadataEntry {
            label: "Runtime Persistence".to_string(),
            value: "keep-alive".to_string(),
        });
        let mut snapshot = ServerUiSnapshot {
            active_session_path: Some(active_path.clone()),
            active_session: Some(active.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: vec![RemoteMachineSnapshot {
                machine_key: "dev".to_string(),
                label: "dev".to_string(),
                ssh_target: "dev".to_string(),
                prefix: None,
                remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
                remote_deploy_state: RemoteDeployState::Ready,
                health: RemoteMachineHealth::Healthy,
                sessions: Vec::new(),
            }],
            ssh_targets: Vec::new(),
            live_sessions: vec![active],
        };

        apply_terminal_runtime_truth_to_snapshot(&server, &HashSet::new(), &mut snapshot);

        assert_eq!(
            snapshot.active_session_path.as_deref(),
            Some(active_path.as_str())
        );
        assert_eq!(snapshot.active_view_mode, WorkspaceViewMode::Terminal);
        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.source),
            Some(SessionSource::LiveSsh)
        );
        assert_eq!(snapshot.live_sessions.len(), 1);
        assert_eq!(snapshot.live_sessions[0].session_path, active_path);
        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.launch_phase),
            Some(TerminalLaunchPhase::RemoteBootstrap)
        );
        assert_eq!(
            snapshot.live_sessions[0].launch_phase,
            TerminalLaunchPhase::RemoteBootstrap
        );
    }

    #[test]
    fn daemon_snapshot_keeps_active_pending_launch_before_runtime_exists() {
        let tree = daemon_test_tree();
        let server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let active_path = "ssh://dev/new-terminal";
        let mut active = daemon_test_snapshot_session(active_path, SessionSource::LiveSsh);
        active.kind = SessionKind::SshShell;
        active.launch_phase = TerminalLaunchPhase::RemoteBootstrap;
        active.remote_deploy_state = RemoteDeployState::Planned;
        let stale_path = remote_scanned_session_path("dev", "old-runtime");
        let mut stale = daemon_test_snapshot_session(&stale_path, SessionSource::LiveSsh);
        stale.kind = SessionKind::Codex;
        stale.launch_phase = TerminalLaunchPhase::Running;
        let mut snapshot = ServerUiSnapshot {
            active_session_path: Some(active_path.to_string()),
            active_session: Some(active.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: Vec::new(),
            ssh_targets: Vec::new(),
            live_sessions: vec![active, stale],
        };

        apply_terminal_runtime_truth_to_snapshot(&server, &HashSet::new(), &mut snapshot);

        assert_eq!(snapshot.active_session_path.as_deref(), Some(active_path));
        assert_eq!(snapshot.active_view_mode, WorkspaceViewMode::Terminal);
        assert_eq!(snapshot.live_sessions.len(), 1);
        assert_eq!(snapshot.live_sessions[0].session_path.as_str(), active_path);
    }

    #[test]
    fn remote_start_requests_get_longer_daemon_response_budget() {
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::Status),
            super::DAEMON_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::StartRemoteCodexSession {
                target: "dev".to_string(),
                prefix: None,
                cwd: Some("/home/pi/gh/yggterm".to_string()),
                title_hint: Some("Debug".to_string()),
                terminal_appearance: None,
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::StartSshSession {
                target: "dev".to_string(),
                prefix: None,
                cwd: None,
                title_hint: None,
                terminal_appearance: None,
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::TerminalRestart {
                path: "remote-session://dev/session".to_string(),
                terminal_appearance: Some("dark".to_string()),
                force_remote: true,
                initial_cols: Some(110),
                initial_rows: Some(50),
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::TerminalRestart {
                path: "remote-session://dev/session".to_string(),
                terminal_appearance: None,
                force_remote: false,
                initial_cols: None,
                initial_rows: None,
            }),
            super::DAEMON_REQUEST_IO_TIMEOUT_MS
        );
    }

    #[test]
    fn hot_restart_detects_duplicate_runtime_owner_before_handoff() {
        fn status_for_hot_restart_test(
            version: &str,
            pid: u32,
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            status_for_hot_restart_test_with_owned(version, pid, terminal_keys, terminal_keys)
        }

        fn status_for_hot_restart_test_with_owned(
            version: &str,
            pid: u32,
            owned_keys: &[&str],
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            serde_json::from_value(serde_json::json!({
                "server_version": version,
                "server_build_id": pid as u64,
                "server_pid": pid,
                "host_kind": "linux-controlled-dock",
                "host_detail": "test",
                "embedded_surface_supported": false,
                "bridge_enabled": false,
                "owned_terminal_session_count": owned_keys.len(),
                "owned_terminal_session_keys": owned_keys,
                "terminal_session_count": terminal_keys.len(),
                "terminal_session_keys": terminal_keys,
                "preserved_terminal_owner_count": 0,
                "preserved_terminal_owner_keys": [],
            }))
            .expect("test status")
        }

        let owned_keys = vec![
            "local://one".to_string(),
            "remote-session://dev/two".to_string(),
        ];
        let duplicate_current = status_for_hot_restart_test(
            "2.2.41",
            410,
            &["local://one", "remote-session://dev/two"],
        );
        let partial_current = status_for_hot_restart_test("2.2.41", 411, &["local://one"]);
        let stale_self = status_for_hot_restart_test(
            "2.2.40",
            400,
            &["local://one", "remote-session://dev/two"],
        );

        let duplicate = super::hot_restart_duplicate_runtime_owner_status_from_statuses(
            Some("2.2.41"),
            &owned_keys,
            400,
            vec![partial_current, duplicate_current.clone(), stale_self],
        )
        .expect("duplicate current owner");

        assert_eq!(duplicate.server_pid, duplicate_current.server_pid);
        assert!(
            super::hot_restart_duplicate_runtime_owner_status_from_statuses(
                Some("2.2.41"),
                &owned_keys,
                400,
                vec![status_for_hot_restart_test("2.2.41", 411, &["local://one"])],
            )
            .is_none(),
            "partial current ownership must not retire the stale owner"
        );
        assert!(
            super::hot_restart_duplicate_runtime_owner_status_from_statuses(
                Some("2.2.41"),
                &owned_keys,
                400,
                vec![status_for_hot_restart_test_with_owned(
                    "2.2.41",
                    412,
                    &["local://one"],
                    &["local://one", "remote-session://dev/two"],
                )],
            )
            .is_none(),
            "preserved/runtime-known keys are not enough; the target daemon must directly own the PTYs"
        );
    }

    #[test]
    fn remote_runtime_requests_carry_terminal_appearance() {
        let request = super::ServerRequest::EnsureRemoteRuntimeCodexSession {
            session_id: "abc123".to_string(),
            cwd: Some("/srv/app".to_string()),
            require_existing: true,
            terminal_appearance: Some("dark".to_string()),
            initial_cols: Some(120),
            initial_rows: Some(36),
        };
        let value = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(value["terminal_appearance"], "dark");

        let decoded: super::ServerRequest =
            serde_json::from_value(value).expect("deserialize request");
        match decoded {
            super::ServerRequest::EnsureRemoteRuntimeCodexSession {
                terminal_appearance,
                ..
            } => assert_eq!(terminal_appearance.as_deref(), Some("dark")),
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn remote_session_terminal_input_uses_hot_local_runtime_before_remote_fallback() {
        assert_eq!(
            super::terminal_write_strategy_for_path(
                "remote-session://dev/8931728b-30a7-428a-8b8e-35bee0480444",
                true
            ),
            super::TerminalWriteStrategy::LocalRuntime
        );
        assert_eq!(
            super::terminal_write_strategy_for_path(
                "  remote-session://dev/d98bc22f-91e4-4332-8696-122cca33c71c",
                false
            ),
            super::TerminalWriteStrategy::RemoteDirectFallback
        );
        assert_eq!(
            super::terminal_write_strategy_for_path("ssh://dev/home/pi/gh/yggterm", false),
            super::TerminalWriteStrategy::LocalRuntimeFallback
        );
        assert_eq!(
            super::terminal_write_strategy_for_path("local://shell", true),
            super::TerminalWriteStrategy::LocalRuntime
        );
    }

    #[test]
    fn daemon_request_outcome_restarts_only_after_successful_hot_restart() {
        let request = super::ServerRequest::HotRestart {
            daemon_executable: "/tmp/yggterm-headless-next".to_string(),
            expected_version: Some("2.1.61".to_string()),
            expected_build_id: None,
            reason: Some("test".to_string()),
        };

        let outcome = super::daemon_request_outcome_for_response(
            &request,
            &super::ServerResponse::Ack {
                message: Some("ok".to_string()),
            },
        );
        assert!(outcome.should_shutdown);
        assert_eq!(
            outcome.restart_executable,
            Some(PathBuf::from("/tmp/yggterm-headless-next"))
        );

        let error_outcome = super::daemon_request_outcome_for_response(
            &request,
            &super::ServerResponse::Error {
                message: "no".to_string(),
            },
        );
        assert!(!error_outcome.should_shutdown);
        assert_eq!(error_outcome.restart_executable, None);

        let shutdown_outcome = super::daemon_request_outcome_for_response(
            &super::ServerRequest::Shutdown,
            &super::ServerResponse::Ack {
                message: Some("bye".to_string()),
            },
        );
        assert!(shutdown_outcome.should_shutdown);
        assert_eq!(shutdown_outcome.restart_executable, None);

        let handoff_outcome = super::daemon_request_outcome_for_response(
            &request,
            &super::ServerResponse::HotUpdateHandoff {
                message: Some("handoff".to_string()),
                owner_endpoint: "/tmp/server-2-1-60.sock".to_string(),
                owner_server_version: "2.1.60".to_string(),
                owner_server_pid: 123,
                target_server_version: Some("2.1.61".to_string()),
                runtime_keys: vec!["local://kept".to_string()],
            },
        );
        assert!(!handoff_outcome.should_shutdown);
        assert_eq!(handoff_outcome.restart_executable, None);
    }

    #[test]
    fn hot_restart_defers_when_daemon_owns_live_terminal_runtime() {
        let owned_keys = vec!["local://kept".to_string()];
        assert!(super::hot_restart_should_defer_for_session_survival(
            &owned_keys
        ));

        let empty_owned_keys = Vec::<String>::new();
        assert!(!super::hot_restart_should_defer_for_session_survival(
            &empty_owned_keys
        ));
    }

    #[test]
    fn hot_restart_does_not_defer_for_preserved_only_terminal_runtime() {
        let preserved_only_owned_keys = Vec::<String>::new();
        let preserved_only_status: ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.1.61",
                "server_build_id": 0,
                "server_pid": 45,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "restored_live_sessions": 5,
                "owned_terminal_session_count": 0,
                "owned_terminal_session_keys": [],
                "terminal_session_count": 2,
                "terminal_session_keys": ["local://kept-1", "local://kept-2"],
                "preserved_terminal_owner_count": 2,
                "preserved_terminal_owner_keys": ["local://kept-1", "local://kept-2"],
                "managed_session_count": 0,
            }))
            .expect("status");

        assert_eq!(preserved_only_status.owned_terminal_session_count, 0);
        assert_eq!(preserved_only_status.preserved_terminal_owner_count, 2);
        assert!(!super::hot_restart_should_defer_for_session_survival(
            &preserved_only_owned_keys
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_keeps_existing_sidecar_entries() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-owners-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-61.sock"));
        let chained_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-60.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.61",
            "server_build_id": 0,
            "server_pid": 61,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "terminal_session_count": 1,
            "terminal_session_keys": ["local://new-owner"],
        }))
        .expect("status");
        let existing = vec![super::PreservedTerminalOwnerEntry {
            runtime_key: "local://old-owner".to_string(),
            endpoint: super::PreservedOwnerEndpoint::from_endpoint(&chained_endpoint),
            owner_server_version: "2.1.60".to_string(),
            owner_server_build_id: 0,
            owner_server_pid: 60,
            created_at_ms: 1,
        }];

        let registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.62".to_string()),
            vec!["local://new-owner".to_string()],
            existing,
        )
        .expect("write registry");

        assert_eq!(
            registry.keys(),
            vec![
                "local://new-owner".to_string(),
                "local://old-owner".to_string()
            ]
        );
        assert_eq!(
            registry
                .owner_for_key("local://old-owner")
                .expect("old owner")
                .owner_server_pid,
            60
        );
        assert_eq!(
            registry
                .owner_for_key("local://new-owner")
                .expect("new owner")
                .owner_server_pid,
            61
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_retargets_preserved_sidecar_entries() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-retarget-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-61.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.61",
            "server_build_id": 0,
            "server_pid": 61,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["local://kept"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["local://kept"],
        }))
        .expect("status");
        let mut registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.62".to_string()),
            vec!["local://kept".to_string()],
            Vec::new(),
        )
        .expect("write registry");

        assert!(
            registry
                .retarget_expected_server_version(&home, Some("2.1.63".to_string()))
                .expect("retarget")
        );

        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(loaded.expected_server_version.as_deref(), Some("2.1.63"));
        assert_eq!(loaded.keys(), vec!["local://kept".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_refuses_older_target_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-target-regression-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-206.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.206",
            "server_build_id": 0,
            "server_pid": 206,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["local://kept"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["local://kept"],
        }))
        .expect("status");
        super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.208".to_string()),
            vec!["local://kept".to_string()],
            Vec::new(),
        )
        .expect("write newer registry");

        let error = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.207".to_string()),
            vec!["local://kept".to_string()],
            Vec::new(),
        )
        .expect_err("older target overwrite must be refused");

        assert!(error.to_string().contains("target regression"));
        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(loaded.expected_server_version.as_deref(), Some("2.1.208"));
        assert_eq!(loaded.keys(), vec!["local://kept".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_refuses_older_retarget() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-retarget-regression-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-206.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.206",
            "server_build_id": 0,
            "server_pid": 206,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["local://kept"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["local://kept"],
        }))
        .expect("status");
        let mut registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.208".to_string()),
            vec!["local://kept".to_string()],
            Vec::new(),
        )
        .expect("write newer registry");

        let error = registry
            .retarget_expected_server_version(&home, Some("2.1.207".to_string()))
            .expect_err("older retarget must be refused");

        assert!(error.to_string().contains("target regression"));
        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(loaded.expected_server_version.as_deref(), Some("2.1.208"));
        assert_eq!(loaded.keys(), vec!["local://kept".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_prunes_unrepresented_runtime_keys() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-prune-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-61.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.61",
            "server_build_id": 0,
            "server_pid": 61,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 2,
            "owned_terminal_session_keys": ["local://kept", "local://closed-yesterday"],
            "terminal_session_count": 2,
            "terminal_session_keys": ["local://kept", "local://closed-yesterday"],
        }))
        .expect("status");
        let mut registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.62".to_string()),
            vec![
                "local://kept".to_string(),
                "local://closed-yesterday".to_string(),
            ],
            Vec::new(),
        )
        .expect("write registry");

        let removed = registry.retain_represented_keys(|key| key == "local://kept");
        registry.save(&home).expect("save pruned registry");

        assert_eq!(removed, vec!["local://closed-yesterday".to_string()]);
        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(loaded.keys(), vec!["local://kept".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_groups_runtime_keys_by_endpoint() {
        let home = std::env::temp_dir().join(format!(
            "yggterm-hot-update-owner-groups-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let first_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-163.sock"));
        let second_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-190.sock"));
        let registry = super::PreservedTerminalOwnerRegistry {
            schema_version: 1,
            expected_server_version: Some("2.1.191".to_string()),
            entries: vec![
                super::PreservedTerminalOwnerEntry {
                    runtime_key: "remote-session://dev/kept-samplenotes".to_string(),
                    endpoint: super::PreservedOwnerEndpoint::from_endpoint(&first_endpoint),
                    owner_server_version: "2.1.163".to_string(),
                    owner_server_build_id: 0,
                    owner_server_pid: 163,
                    created_at_ms: 1,
                },
                super::PreservedTerminalOwnerEntry {
                    runtime_key: "remote-session://dev/kept-erome".to_string(),
                    endpoint: super::PreservedOwnerEndpoint::from_endpoint(&first_endpoint),
                    owner_server_version: "2.1.163".to_string(),
                    owner_server_build_id: 0,
                    owner_server_pid: 163,
                    created_at_ms: 1,
                },
                super::PreservedTerminalOwnerEntry {
                    runtime_key: "local://new-runtime".to_string(),
                    endpoint: super::PreservedOwnerEndpoint::from_endpoint(&second_endpoint),
                    owner_server_version: "2.1.190".to_string(),
                    owner_server_build_id: 0,
                    owner_server_pid: 190,
                    created_at_ms: 1,
                },
            ],
        };

        let groups = registry.endpoint_groups();

        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0],
            (
                first_endpoint,
                vec![
                    "remote-session://dev/kept-erome".to_string(),
                    "remote-session://dev/kept-samplenotes".to_string(),
                ],
            )
        );
        assert_eq!(
            groups[1],
            (second_endpoint, vec!["local://new-runtime".to_string()])
        );
    }

    #[test]
    fn preserved_owner_runtime_prune_candidates_reject_closed_ghost_keys() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let kept_samplenotes = remote_scanned_session_path("dev", "kept-samplenotes");
        server.restore_live_session(PersistedLiveSession {
            key: kept_samplenotes.clone(),
            id: "kept-samplenotes".to_string(),
            title: "samplenotes".to_string(),
            kind: SessionKind::Codex,
            keep_alive: true,
            ssh_target: "dev".to_string(),
            prefix: None,
            cwd: Some("/home/pi/git/samplenotes".to_string()),
            remote_launch_action: None,
            storage_path: None,
            restore_reason: None,
        });
        let unkept_update_runtime = remote_scanned_session_path("dev", "temporary-update");
        server.restore_live_session(PersistedLiveSession {
            key: unkept_update_runtime.clone(),
            id: "temporary-update".to_string(),
            title: "temporary update".to_string(),
            kind: SessionKind::Codex,
            keep_alive: false,
            ssh_target: "dev".to_string(),
            prefix: None,
            cwd: Some("/home/pi/gh/yggterm".to_string()),
            remote_launch_action: None,
            storage_path: None,
            restore_reason: Some("update-restart".to_string()),
        });
        let represented_runtime_keys = HashSet::from([kept_samplenotes.clone()]);

        let stale_runtime_keys = super::unrepresented_preserved_owner_runtime_keys(
            &server,
            &represented_runtime_keys,
            vec![
                kept_samplenotes,
                unkept_update_runtime.clone(),
                "remote-session://dev/closed-may-6".to_string(),
                "remote-session://dev/closed-generic-a".to_string(),
                "remote-session://dev/closed-generic-b".to_string(),
            ],
        );

        assert!(server.live_session_keep_alive(&remote_scanned_session_path("dev", "kept-samplenotes")));
        assert!(!server.live_session_keep_alive(&unkept_update_runtime));
        assert_eq!(
            stale_runtime_keys,
            vec![
                "remote-session://dev/closed-generic-a".to_string(),
                "remote-session://dev/closed-generic-b".to_string(),
                "remote-session://dev/closed-may-6".to_string(),
            ],
            "owner daemons must drop PTYs that are neither in the hot-update registry nor represented by current live-session truth"
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonical_hot_restart_executable_requires_executable_file() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-restart-exe-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        fs::create_dir_all(&root).expect("create temp dir");
        let executable = root.join("yggterm-headless");
        fs::write(&executable, b"#!/bin/sh\n").expect("write executable");
        let mut permissions = fs::metadata(&executable).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("chmod executable");

        let canonical =
            super::canonical_hot_restart_executable(executable.to_str().expect("utf8 temp path"))
                .expect("executable should validate");
        assert_eq!(
            canonical,
            executable.canonicalize().expect("canonical path")
        );

        let non_executable = root.join("not-executable");
        fs::write(&non_executable, b"").expect("write non executable");
        assert!(
            super::canonical_hot_restart_executable(
                non_executable.to_str().expect("utf8 temp path"),
            )
            .is_err()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn parse_versioned_server_socket_name_accepts_semver_socket_names() {
        let parsed = parse_versioned_server_socket_name(Path::new("/tmp/server-2-1-5.sock"));
        assert_eq!(parsed, Some((2, 1, 5)));
        assert_eq!(
            parse_versioned_server_socket_name(Path::new("/tmp/server.sock")),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn default_endpoint_keeps_short_home_socket_in_home() {
        let home = Path::new("/tmp/yggterm-short-home");
        let endpoint = default_endpoint(home);
        let super::ServerEndpoint::UnixSocket(path) = endpoint else {
            panic!("unix builds should use a unix socket endpoint")
        };
        assert!(path.starts_with(home));
        assert!(unix_socket_path_fits_platform(&path));
    }

    #[cfg(unix)]
    #[test]
    fn default_endpoint_uses_runtime_socket_for_long_home() {
        let home = std::env::temp_dir()
            .join("yggterm-long-home")
            .join("nested")
            .join("path")
            .join("that")
            .join("would")
            .join("overflow")
            .join("linux")
            .join("sun-path")
            .join("when")
            .join("the")
            .join("versioned")
            .join("server")
            .join("socket")
            .join("is")
            .join("inside")
            .join("the")
            .join("home");
        let endpoint = default_endpoint(&home);
        let super::ServerEndpoint::UnixSocket(path) = endpoint else {
            panic!("unix builds should use a unix socket endpoint")
        };
        assert!(!path.starts_with(&home));
        assert!(unix_socket_path_fits_platform(&path));
        assert_eq!(
            parse_versioned_server_socket_name(&path),
            parse_versioned_server_socket_name(Path::new(&format!(
                "/tmp/server-{}.sock",
                super::SERVER_PROTOCOL_VERSION.replace('.', "-")
            )))
        );
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_legacy_unix_daemons_ignores_missing_socket_dir() {
        let endpoint = super::ServerEndpoint::UnixSocket(
            std::env::temp_dir()
                .join(format!(
                    "yggterm-missing-socket-dir-{}-{}",
                    std::process::id(),
                    super::current_millis()
                ))
                .join("server-2-1-35.sock"),
        );
        cleanup_legacy_unix_daemons(&endpoint).expect("missing socket dir should be harmless");
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_legacy_unix_daemons_keeps_reachable_legacy_socket() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-reachable-legacy-socket-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let sockets_dir = root.join("home").join(".yggterm");
        fs::create_dir_all(&sockets_dir).expect("create socket dir");
        let current = sockets_dir.join("server-2-1-48.sock");
        let legacy = sockets_dir.join("server-2-1-47.sock");
        let _listener =
            std::os::unix::net::UnixListener::bind(&legacy).expect("bind legacy socket");
        let endpoint = super::ServerEndpoint::UnixSocket(current);

        cleanup_legacy_unix_daemons(&endpoint).expect("cleanup should keep reachable legacy");

        assert!(
            legacy.exists(),
            "reachable legacy socket must not be unlinked"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_daemon_cleanup_protects_owner_but_not_stale_preserved_sidecar() {
        fn status_for_cleanup_test(
            version: &str,
            pid: u32,
            owned_keys: &[&str],
            preserved_keys: &[&str],
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            serde_json::from_value(serde_json::json!({
                "server_version": version,
                "server_build_id": 0,
                "server_pid": pid,
                "host_kind": "linux-controlled-dock",
                "host_detail": "test",
                "embedded_surface_supported": false,
                "bridge_enabled": false,
                "owned_terminal_session_count": owned_keys.len(),
                "owned_terminal_session_keys": owned_keys,
                "terminal_session_count": terminal_keys.len(),
                "terminal_session_keys": terminal_keys,
                "preserved_terminal_owner_count": preserved_keys.len(),
                "preserved_terminal_owner_keys": preserved_keys,
            }))
            .expect("test status")
        }

        let stale_sidecar = status_for_cleanup_test(
            "2.1.170",
            170,
            &[],
            &["local://kept", "local://closed"],
            &["local://kept", "local://closed"],
        );
        let actual_owner = status_for_cleanup_test(
            "2.1.170",
            163,
            &["local://kept", "local://closed"],
            &[],
            &["local://kept", "local://closed"],
        );
        let owned_runtime =
            status_for_cleanup_test("2.1.170", 171, &["local://kept"], &[], &["local://kept"]);
        let ghost_owned_runtime = status_for_cleanup_test(
            "2.1.170",
            177,
            &["local://closed"],
            &[],
            &["local://closed"],
        );
        let current_runtime = status_for_cleanup_test(
            super::SERVER_PROTOCOL_VERSION,
            220,
            &["local://kept"],
            &[],
            &["local://kept"],
        );
        let current_version_terminal_keys =
            HashSet::from(["local://kept".to_string(), "local://closed".to_string()]);
        let preserved_owner_pids = HashSet::from([163]);
        let preserved_owner_runtime_keys = HashSet::from(["local://kept".to_string()]);

        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                163,
                Some(&actual_owner),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                true,
                &HashSet::new(),
                true,
            ),
            "the endpoint in the hot-update owner registry is still a session-survival root"
        );
        assert!(
            !linux_daemon_runtime_activity_protected_for_cleanup(
                163,
                Some(&actual_owner),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                true,
                &current_version_terminal_keys,
                true,
            ),
            "a registry owner whose keys are already owned by the current daemon must retire"
        );
        assert!(
            !linux_daemon_runtime_activity_protected_for_cleanup(
                170,
                Some(&stale_sidecar),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                false,
                &HashSet::new(),
                true,
            ),
            "a preserved-only sidecar must not inherit protection from unrelated home runtime activity"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                171,
                Some(&owned_runtime),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                false,
                &HashSet::new(),
                false,
            ),
            "a daemon that directly owns a PTY is protected even if it is not the registry endpoint"
        );
        assert!(
            !linux_daemon_runtime_activity_protected_for_cleanup(
                171,
                Some(&owned_runtime),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                true,
                &HashSet::new(),
                false,
            ),
            "once the registry endpoint is clean, duplicate non-registry owners for the same keys should retire"
        );
        assert!(
            !linux_daemon_runtime_activity_protected_for_cleanup(
                177,
                Some(&ghost_owned_runtime),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                false,
                &HashSet::new(),
                true,
            ),
            "a daemon that only owns closed-session runtime keys should not stay protected"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                170,
                Some(&stale_sidecar),
                &HashSet::new(),
                &HashSet::new(),
                false,
                false,
                &HashSet::new(),
                true,
            ),
            "without current duplicate ownership evidence, cleanup remains conservative for legacy homes"
        );
        assert!(
            !linux_daemon_runtime_activity_protected_for_cleanup(
                163,
                Some(&actual_owner),
                &HashSet::new(),
                &HashSet::new(),
                false,
                false,
                &current_version_terminal_keys,
                true,
            ),
            "an old daemon whose terminal keys are already owned by the current daemon must not stay protected"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                220,
                Some(&current_runtime),
                &HashSet::new(),
                &HashSet::new(),
                false,
                false,
                &current_version_terminal_keys,
                false,
            ),
            "the current daemon that owns live terminal keys is protected"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_daemon_cleanup_keeps_newest_clean_startup_bridge_sidecar() {
        fn status_for_cleanup_test(
            version: &str,
            pid: u32,
            owned_keys: &[&str],
            preserved_keys: &[&str],
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            serde_json::from_value(serde_json::json!({
                "server_version": version,
                "server_build_id": pid as u64,
                "server_pid": pid,
                "host_kind": "linux-controlled-dock",
                "host_detail": "test",
                "embedded_surface_supported": false,
                "bridge_enabled": false,
                "owned_terminal_session_count": owned_keys.len(),
                "owned_terminal_session_keys": owned_keys,
                "terminal_session_count": terminal_keys.len(),
                "terminal_session_keys": terminal_keys,
                "preserved_terminal_owner_count": preserved_keys.len(),
                "preserved_terminal_owner_keys": preserved_keys,
            }))
            .expect("test status")
        }

        let preserved_owner_pids = HashSet::from([163]);
        let preserved_owner_runtime_keys =
            HashSet::from(["local://kept-a".to_string(), "local://kept-b".to_string()]);
        let statuses = HashMap::from([
            (
                163,
                status_for_cleanup_test(
                    "2.1.163",
                    163,
                    &[],
                    &[],
                    &["local://kept-a", "local://kept-b", "local://closed"],
                ),
            ),
            (
                182,
                status_for_cleanup_test(
                    "2.1.182",
                    182,
                    &[],
                    &["local://kept-a", "local://kept-b"],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
            (
                184,
                status_for_cleanup_test(
                    "2.1.184",
                    184,
                    &[],
                    &["local://kept-a", "local://kept-b"],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
            (
                185,
                status_for_cleanup_test(
                    SERVER_PROTOCOL_VERSION,
                    185,
                    &[],
                    &["local://kept-a", "local://kept-b"],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
        ]);

        assert_eq!(
            linux_cleanup_startup_bridge_sidecar_pid(
                &statuses,
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                185,
            ),
            Some(184),
            "the newest clean preserved-only sidecar must survive initial cleanup so startup can retarget it instead of handoffing from the old owner with ghost keys"
        );

        let clean_owner_statuses = HashMap::from([
            (
                163,
                status_for_cleanup_test(
                    "2.1.163",
                    163,
                    &["local://kept-a", "local://kept-b"],
                    &[],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
            (
                184,
                status_for_cleanup_test(
                    "2.1.184",
                    184,
                    &[],
                    &["local://kept-a", "local://kept-b"],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
        ]);
        assert_eq!(
            linux_cleanup_startup_bridge_sidecar_pid(
                &clean_owner_statuses,
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                185,
            ),
            None,
            "a preserved-only startup bridge sidecar must retire once the registry's real PTY owner is already clean"
        );
    }

    #[cfg(unix)]
    #[test]
    fn versioned_server_socket_alias_candidates_include_client_instance_versions() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-alias-test-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let sockets_dir = root.join("home").join(".yggterm");
        let client_instances = sockets_dir.join("client-instances");
        fs::create_dir_all(&client_instances).expect("create client instances dir");
        let current = sockets_dir.join("server-2-1-5.sock");
        fs::write(&current, b"").expect("write current socket placeholder");
        fs::create_dir_all(client_instances.join("unix--home-pi--yggterm-server-2-1-4-sock"))
            .expect("create old instance dir");

        let candidates = versioned_server_socket_alias_candidates(&current);

        assert!(candidates.contains(&sockets_dir.join("server-2-1-4.sock")));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn versioned_server_status_probe_paths_dedupe_symlink_aliases_before_status() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-status-dedupe-test-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let sockets_dir = root.join("home").join(".yggterm");
        fs::create_dir_all(&sockets_dir).expect("create sockets dir");
        let super::ServerEndpoint::UnixSocket(current) = super::default_endpoint(&sockets_dir)
        else {
            panic!("unix test requires unix socket endpoint");
        };
        fs::write(&current, b"").expect("write current socket placeholder");
        std::os::unix::fs::symlink(&current, sockets_dir.join("server-2-1-102.sock"))
            .expect("create 2.1.102 alias");
        std::os::unix::fs::symlink(&current, sockets_dir.join("server-2-1-101.sock"))
            .expect("create 2.1.101 alias");

        let paths = super::versioned_server_status_probe_paths(&sockets_dir);

        assert_eq!(paths, vec![current]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn daemon_socket_lock_is_exclusive_for_socket_path() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-lock-test-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        fs::create_dir_all(&root).expect("create lock root");
        let socket = root.join("server-2-1-148.sock");

        let first = super::try_acquire_daemon_socket_lock(&socket, &root)
            .expect("first lock result")
            .expect("first lock");
        let second =
            super::try_acquire_daemon_socket_lock(&socket, &root).expect("second lock result");
        assert!(
            second.is_none(),
            "second daemon must not acquire the same socket lock"
        );
        drop(first);
        let third =
            super::try_acquire_daemon_socket_lock(&socket, &root).expect("third lock result");
        assert!(third.is_some(), "lock must release when owner exits");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn versioned_socket_alias_policy_only_targets_older_versions() {
        let current = (2, 1, 32);
        assert!(versioned_socket_alias_is_legacy(current, (2, 1, 31)));
        assert!(!versioned_socket_alias_is_legacy(current, current));
        assert!(!versioned_socket_alias_is_legacy(current, (2, 1, 33)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn daemon_binary_is_legacy_allows_deleted_current_install_path() {
        let current = Path::new("/home/pi/.yggterm/bin/yggterm");
        assert!(!daemon_binary_is_legacy(
            current,
            "/home/pi/.yggterm/bin/yggterm",
            Some(Path::new("/home/pi/.yggterm/bin/yggterm (deleted)")),
        ));
        assert!(!daemon_binary_is_legacy(
            current,
            "/home/pi/.yggterm/bin/yggterm",
            Some(Path::new("/home/pi/.yggterm/bin/yggterm")),
        ));
        assert!(!daemon_binary_is_legacy(
            current,
            "/home/pi/.yggterm/bin/yggterm-headless",
            Some(Path::new("/home/pi/.yggterm/bin/yggterm-headless")),
        ));
        assert!(daemon_binary_is_legacy(
            current,
            "/tmp/old-yggterm",
            Some(Path::new("/tmp/old-yggterm")),
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_bridge_process_detection_matches_remote_resume_stdio_bridges() {
        let resume_parts = vec![
            "/home/pi/.yggterm/bin/yggterm".to_string(),
            "server".to_string(),
            "remote".to_string(),
            "resume-codex".to_string(),
            "019dbddf".to_string(),
            "/home/pi/gh/yggterm".to_string(),
            "--require-existing".to_string(),
        ];
        assert!(super::linux_yggterm_server_process_is_live_bridge(
            &resume_parts
        ));
        let start_parts = vec![
            "/home/pi/.yggterm/bin/yggterm".to_string(),
            "server".to_string(),
            "remote".to_string(),
            "start-codex".to_string(),
            "019dbddf".to_string(),
            "/home/pi/gh/yggterm".to_string(),
        ];
        assert!(super::linux_yggterm_server_process_is_live_bridge(
            &start_parts
        ));
        let short_lived_probe = vec![
            "/home/pi/.yggterm/bin/yggterm".to_string(),
            "server".to_string(),
            "remote".to_string(),
            "saved-codex-session-exists".to_string(),
            "019dbddf".to_string(),
        ];
        assert!(!super::linux_yggterm_server_process_is_live_bridge(
            &short_lived_probe
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn legacy_daemon_reap_only_applies_within_same_home() {
        let current_home = Path::new("/home/pi/.yggterm");
        let other_home = Path::new("/tmp/yggterm-other-home");
        assert!(legacy_daemon_reap_applies_to_home(
            Some(current_home),
            Some(current_home)
        ));
        assert!(!legacy_daemon_reap_applies_to_home(
            Some(current_home),
            Some(other_home)
        ));
        assert!(!legacy_daemon_reap_applies_to_home(
            Some(current_home),
            None
        ));
        assert!(!legacy_daemon_reap_applies_to_home(
            None,
            Some(current_home)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn orphan_daemon_reap_only_applies_within_same_home() {
        let current_home = Path::new("/home/pi/.yggterm");
        let other_home = Path::new("/tmp/yggterm-other-home");
        assert!(orphan_daemon_reap_applies_to_home(
            Some(current_home),
            Some(current_home)
        ));
        assert!(!orphan_daemon_reap_applies_to_home(
            Some(current_home),
            Some(other_home)
        ));
        assert!(!orphan_daemon_reap_applies_to_home(
            Some(current_home),
            None
        ));
        assert!(!orphan_daemon_reap_applies_to_home(
            None,
            Some(current_home)
        ));
    }

    #[test]
    fn write_persisted_state_creates_parent_and_round_trips() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-state-roundtrip-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let state_path = root.join("nested").join("server-state.json");
        let expected = PersistedDaemonState {
            active_session_path: Some("remote-session://jojo/demo".to_string()),
            active_view_mode: super::WorkspaceViewMode::Terminal,
            ssh_targets: Vec::new(),
            remote_machines: Vec::new(),
            stored_sessions: Vec::new(),
            live_sessions: vec![PersistedLiveSession {
                key: "remote-session://jojo/demo".to_string(),
                id: "demo".to_string(),
                title: "Demo".to_string(),
                kind: super::SessionKind::Codex,
                keep_alive: true,
                ssh_target: "jojo".to_string(),
                prefix: None,
                cwd: Some("/home/pi".to_string()),
                remote_launch_action: None,
                storage_path: None,
                restore_reason: None,
            }],
        };

        write_persisted_state(&state_path, &expected).expect("write daemon state");
        let loaded = load_persisted_state(&state_path)
            .expect("load daemon state")
            .expect("persisted daemon state");
        assert_eq!(loaded.active_session_path, expected.active_session_path);
        assert_eq!(loaded.live_sessions, expected.live_sessions);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_persisted_state_keeps_previous_copy_before_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-state-backup-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let state_path = root.join("server-state.json");
        let first = PersistedDaemonState {
            active_session_path: Some("remote-session://dev/first".to_string()),
            active_view_mode: super::WorkspaceViewMode::Terminal,
            ssh_targets: Vec::new(),
            remote_machines: Vec::new(),
            stored_sessions: Vec::new(),
            live_sessions: Vec::new(),
        };
        let second = PersistedDaemonState {
            active_session_path: Some("remote-session://dev/second".to_string()),
            active_view_mode: super::WorkspaceViewMode::Rendered,
            ssh_targets: Vec::new(),
            remote_machines: Vec::new(),
            stored_sessions: Vec::new(),
            live_sessions: Vec::new(),
        };

        write_persisted_state(&state_path, &first).expect("write first daemon state");
        write_persisted_state(&state_path, &second).expect("write second daemon state");

        let backup_path = root.join("server-state.previous.json");
        let backup = load_persisted_state(&backup_path)
            .expect("load backup daemon state")
            .expect("backup daemon state");
        let current = load_persisted_state(&state_path)
            .expect("load current daemon state")
            .expect("current daemon state");
        assert_eq!(backup.active_session_path, first.active_session_path);
        assert_eq!(current.active_session_path, second.active_session_path);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_prewarm_keeps_remote_live_sessions_eligible_for_background_load() {
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("local://shell", false),
            Some("disabled_by_env")
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("codex://session", false),
            Some("disabled_by_env")
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag(
                "live::fc99adfa-d0b1-4e50-a27d-cbb3a1602b4b",
                true,
            ),
            Some("runtime_owned_live_session")
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("local://shell", true),
            None
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("codex://session", true),
            None
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("remote-session://dev/session", true),
            None
        );
    }

    #[test]
    fn startup_prewarm_seeds_active_remote_snapshot_only() {
        assert!(super::startup_prewarm_should_seed_remote_snapshot(
            "remote-session://dev/session",
            Some("remote-session://dev/session")
        ));
        assert!(!super::startup_prewarm_should_seed_remote_snapshot(
            "remote-session://dev/background",
            Some("remote-session://dev/session")
        ));
        assert!(!super::startup_prewarm_should_seed_remote_snapshot(
            "remote-session://dev/background",
            None
        ));
        assert!(!super::startup_prewarm_should_seed_remote_snapshot(
            "local://shell",
            Some("local://shell")
        ));
    }

    #[test]
    fn terminal_ensure_skips_remote_snapshot_seed_for_background_latency_budget() {
        assert!(!super::terminal_ensure_should_seed_remote_snapshot(
            "remote-session://dev/session",
            false
        ));
        assert!(super::terminal_ensure_should_seed_remote_snapshot(
            "remote-session://dev/session",
            true
        ));
        assert!(!super::terminal_ensure_should_seed_remote_snapshot(
            "local://shell",
            true
        ));
    }

    #[test]
    fn terminal_sidebar_snapshot_from_screen_prefers_non_empty_tail() {
        let snapshot = "\npi@dev:~/gh/yggterm$ sleep 3\npi@dev:~/gh/yggterm$ \n";
        let (status_line, terminal_lines) =
            terminal_sidebar_snapshot_from_screen(snapshot).expect("screen snapshot");
        assert_eq!(status_line, "pi@dev:~/gh/yggterm$");
        assert_eq!(
            terminal_lines,
            vec![
                "pi@dev:~/gh/yggterm$ sleep 3".to_string(),
                "pi@dev:~/gh/yggterm$".to_string(),
            ]
        );
    }

    #[test]
    fn terminal_sidebar_snapshot_from_screen_returns_none_for_blank_surface() {
        assert_eq!(terminal_sidebar_snapshot_from_screen("\n  \n\r\n"), None);
    }

    #[test]
    fn collect_remote_copy_candidates_do_not_clone_machine_session_lists() {
        let machines = vec![RemoteMachineSnapshot {
            machine_key: "jojo".to_string(),
            label: "jojo".to_string(),
            ssh_target: "jojo".to_string(),
            prefix: Some("sudo -u pi".to_string()),
            remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: vec![
                RemoteScannedSession {
                    session_path: remote_scanned_session_path("jojo", "one"),
                    session_id: "one".to_string(),
                    cwd: "/home/pi".to_string(),
                    started_at: "2026-04-01T00:00:00Z".to_string(),
                    modified_epoch: 1,
                    event_count: 1,
                    user_message_count: 1,
                    assistant_message_count: 0,
                    title_hint: "One".to_string(),
                    recent_context: "USER: one".to_string(),
                    cached_precis: None,
                    cached_summary: None,
                    live_runtime: false,
                    storage_path: "/home/pi/.codex/sessions/one.jsonl".to_string(),
                },
                RemoteScannedSession {
                    session_path: remote_scanned_session_path("jojo", "two"),
                    session_id: "two".to_string(),
                    cwd: "/srv/app".to_string(),
                    started_at: "2026-04-01T00:01:00Z".to_string(),
                    modified_epoch: 2,
                    event_count: 2,
                    user_message_count: 1,
                    assistant_message_count: 1,
                    title_hint: "Two".to_string(),
                    recent_context: String::new(),
                    cached_precis: Some("precis".to_string()),
                    cached_summary: Some("summary".to_string()),
                    live_runtime: false,
                    storage_path: "/home/pi/.codex/sessions/two.jsonl".to_string(),
                },
            ],
        }];

        let candidates = collect_remote_copy_candidates(&machines);

        assert_eq!(candidates.len(), 2);
        for candidate in candidates {
            let machine = candidate
                .remote_machine
                .expect("candidate should keep machine routing");
            assert_eq!(machine.machine_key, "jojo");
            assert!(machine.sessions.is_empty());
            assert_eq!(machine.prefix.as_deref(), Some("sudo -u pi"));
        }
        assert_eq!(machines[0].sessions.len(), 2);
    }

    #[test]
    fn collect_remote_copy_candidates_preserve_machine_order_across_duplicates() {
        let machines = vec![
            RemoteMachineSnapshot {
                machine_key: "jojo".to_string(),
                label: "jojo".to_string(),
                ssh_target: "jojo".to_string(),
                prefix: None,
                remote_binary_expr: None,
                remote_deploy_state: RemoteDeployState::Ready,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![
                    RemoteScannedSession {
                        session_path: remote_scanned_session_path("jojo", "one"),
                        session_id: "one".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-04-01T00:00:00Z".to_string(),
                        modified_epoch: 1,
                        event_count: 1,
                        user_message_count: 1,
                        assistant_message_count: 0,
                        title_hint: "One".to_string(),
                        recent_context: String::new(),
                        cached_precis: None,
                        cached_summary: None,
                        live_runtime: false,
                        storage_path: "/home/pi/.codex/sessions/one.jsonl".to_string(),
                    },
                    RemoteScannedSession {
                        session_path: remote_scanned_session_path("jojo", "two"),
                        session_id: "two".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-04-01T00:01:00Z".to_string(),
                        modified_epoch: 2,
                        event_count: 2,
                        user_message_count: 1,
                        assistant_message_count: 1,
                        title_hint: "Two".to_string(),
                        recent_context: String::new(),
                        cached_precis: None,
                        cached_summary: None,
                        live_runtime: false,
                        storage_path: "/home/pi/.codex/sessions/two.jsonl".to_string(),
                    },
                ],
            },
            RemoteMachineSnapshot {
                machine_key: "oc".to_string(),
                label: "oc".to_string(),
                ssh_target: "oc".to_string(),
                prefix: None,
                remote_binary_expr: None,
                remote_deploy_state: RemoteDeployState::Ready,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![RemoteScannedSession {
                    session_path: remote_scanned_session_path("oc", "three"),
                    session_id: "three".to_string(),
                    cwd: "/root".to_string(),
                    started_at: "2026-04-01T00:02:00Z".to_string(),
                    modified_epoch: 3,
                    event_count: 3,
                    user_message_count: 2,
                    assistant_message_count: 1,
                    title_hint: "Three".to_string(),
                    recent_context: String::new(),
                    cached_precis: None,
                    cached_summary: None,
                    live_runtime: false,
                    storage_path: "/root/.codex/sessions/three.jsonl".to_string(),
                }],
            },
        ];

        let candidates = collect_remote_copy_candidates(&machines);
        let keys = candidates
            .into_iter()
            .map(|candidate| {
                candidate
                    .remote_machine
                    .expect("remote machine")
                    .machine_key
            })
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec!["jojo".to_string(), "jojo".to_string(), "oc".to_string()]
        );
    }

    #[test]
    fn terminal_launch_command_for_remote_session_keeps_server_owned_resume_command() {
        let stored =
            "ssh oc '$HOME/.yggterm/bin/yggterm' server remote resume-codex 019c --require-existing"
                .to_string();
        let legacy = Some("ssh oc legacy-direct-attach 019c".to_string());

        let selected =
            terminal_launch_command_for_path("remote-session://oc/019c", stored.clone(), legacy);

        assert_eq!(selected, stored);
    }

    #[test]
    fn remote_resume_stale_attach_ignores_semantic_mismatch_without_breakage() {
        assert!(!remote_resume_stale_attach(
            true,
            3_500,
            false,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(remote_resume_stale_attach(
            false,
            901,
            false,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(!remote_resume_stale_attach(
            false,
            3_500,
            false,
            REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(remote_resume_stale_attach(
            true,
            3_500,
            true,
            REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS
        ));
    }

    #[cfg(unix)]
    #[test]
    fn daemon_idle_shutdown_is_blocked_by_owned_terminal_sessions() {
        let home =
            std::env::temp_dir().join(format!("yggterm-idle-terminal-{}", std::process::id()));
        fs::create_dir_all(&home).expect("create temp home");
        let endpoint = super::ServerEndpoint::UnixSocket(home.join("server-test.sock"));
        let last_activity = std::sync::atomic::AtomicU64::new(0);

        assert!(
            !super::daemon_should_idle_shutdown(&home, &endpoint, &last_activity, 1, 1),
            "a daemon that still owns a terminal PTY must not idle-exit just because no GUI/client record is present"
        );

        let _ = fs::remove_dir_all(home);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_yggterm_home_from_environ_prefers_explicit_override() {
        let env = b"HOME=/home/pi\0YGGTERM_HOME=/tmp/yggterm-test\0";
        assert_eq!(
            linux_yggterm_home_from_environ_bytes(env),
            Some(std::path::PathBuf::from("/tmp/yggterm-test"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_yggterm_home_from_environ_falls_back_to_default_home_dir() {
        let env = b"HOME=/home/pi\0DISPLAY=:10\0";
        assert_eq!(
            linux_yggterm_home_from_environ_bytes(env),
            Some(std::path::PathBuf::from("/home/pi/.yggterm"))
        );
    }
}
