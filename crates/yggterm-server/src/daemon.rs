use crate::terminal::{TerminalBufferStats, terminal_data_has_scrollback_text};
use crate::{
    GhosttyHostSupport, ManagedSessionView, PersistedDaemonState, RemoteMachineSnapshot,
    RemoteRuntimeRegistry, ServerUiSnapshot, SessionKind, SnapshotSessionView, SshConnectTarget,
    TerminalManager, WorkspaceViewMode, YggtermServer, active_client_instance_records,
    current_millis, fetch_remote_generation_context,
    local_headless_companion_executable_from_current, persist_remote_generated_copy,
    remote_resume_runtime_output_requires_restart, request_remote_codex_session_shutdown,
    spawn_hot_restart_daemon_process, terminate_remote_codex_session,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
const REMOTE_ATTACH_STARTUP_GRACE_MS: u64 = 900;
const REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS: u64 = 18_000;
const CLIENT_CLOSE_FORCE_SHUTDOWN_AFTER_SECS: u64 = 60 * 60;
const ENV_YGGTERM_ENABLE_BACKGROUND_COPY_CHORE: &str = "YGGTERM_ENABLE_BACKGROUND_COPY_CHORE";

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
    let target = proc_exe_target.to_string_lossy();
    if target.contains(" (deleted)") {
        return true;
    }
    !allowed
        .iter()
        .any(|candidate| target == candidate.to_string_lossy())
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
        if ping(&ServerEndpoint::UnixSocket(candidate.clone())).is_ok() {
            continue;
        }
        let _ = fs::remove_file(&candidate);
        let _ = std::os::unix::fs::symlink(current, &candidate);
    }
}

#[cfg(unix)]
fn versioned_socket_alias_is_legacy(
    current_version: (u64, u64, u64),
    candidate_version: (u64, u64, u64),
) -> bool {
    candidate_version < current_version
}

#[cfg(unix)]
fn server_socket_path_lexists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
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
    pub terminal_session_count: usize,
    #[serde(default)]
    pub terminal_session_keys: Vec<String>,
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
    },
    StartRemoteCodexSession {
        target: String,
        prefix: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
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
    },
    EnsureRemoteRuntimeCodexSession {
        session_id: String,
        cwd: Option<String>,
        require_existing: bool,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
    },
    StartRemoteRuntimeCodexSession {
        session_id: String,
        cwd: Option<String>,
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
    SyncExternalWindow,
    RaiseExternalWindow,
    SyncTheme {
        theme: UiTheme,
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
    },
    TerminalSnapshot {
        text: String,
        running: bool,
        runtime_output_seen: bool,
    },
    TerminalRetainedSnapshot {
        text: String,
        running: bool,
        runtime_output_seen: bool,
    },
    Ack {
        message: Option<String>,
    },
    Error {
        message: String,
    },
}

struct DaemonRuntime {
    support: GhosttyHostSupport,
    state_path: PathBuf,
    store: SessionStore,
    server: YggtermServer,
    terminals: TerminalManager,
    remote_machine_refreshes_in_flight: HashSet<String>,
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
        let runtime = Self {
            support,
            state_path,
            store,
            server,
            terminals: TerminalManager::new(),
            remote_machine_refreshes_in_flight: HashSet::new(),
            restored_from_persisted_state,
            restored_stored_sessions,
            restored_live_sessions,
            restored_remote_machines,
        };
        perf.finish(serde_json::json!({
            "prefer_ghostty_backend": settings.prefer_ghostty_backend,
            "theme": format!("{:?}", settings.theme),
        }));
        Ok(runtime)
    }

    fn status(&self) -> ServerRuntimeStatus {
        let terminal_stats = self.terminals.stats();
        let payload_stats = self.server.payload_stats();
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
            terminal_session_count: terminal_stats.session_count,
            terminal_session_keys: self.terminals.session_keys(),
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

    fn snapshot_response(&self, message: Option<String>) -> ServerResponse {
        let mut snapshot = self.server.snapshot();
        let runtime_keys = self.terminals.session_keys().into_iter().collect();
        apply_terminal_runtime_truth_to_snapshot(&self.server, &runtime_keys, &mut snapshot);
        if let Some(active_session) = snapshot.active_session.as_mut() {
            self.overlay_terminal_runtime_snapshot_session(active_session);
        }
        for session in &mut snapshot.live_sessions {
            self.overlay_terminal_runtime_snapshot_session(session);
        }
        ServerResponse::Snapshot { snapshot, message }
    }

    fn terminal_runtime_key_for_path(&self, path: &str) -> String {
        self.server.terminal_runtime_key_for_path(path)
    }

    fn ensure_terminal_for_path(&mut self, path: &str) -> Result<Option<String>> {
        self.ensure_terminal_for_path_with_initial_size(path, None)
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
    ) -> Result<Option<String>> {
        self.ensure_terminal_for_path_with_initial_size_and_seed(path, None, false)
    }

    fn ensure_terminal_for_path_with_initial_size_and_seed(
        &mut self,
        path: &str,
        initial_size: Option<(u16, u16)>,
        seed_remote_snapshot: bool,
    ) -> Result<Option<String>> {
        let runtime_path = self.terminal_runtime_key_for_path(path);
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
        self.server.request_terminal_launch_for_path(path);
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

    fn persist(&self) -> Result<()> {
        write_persisted_state(&self.state_path, &self.server.persisted_state())
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
                write_persisted_state(
                    &self.state_path,
                    &self.server.persisted_state_for_update_restart(),
                )?;
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
                let daemon_executable = canonical_hot_restart_executable(&daemon_executable)?;
                write_persisted_state(
                    &self.state_path,
                    &self.server.persisted_state_for_update_restart(),
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
            } => {
                let key = self.server.start_ssh_session(
                    &target,
                    prefix.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                )?;
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::StartRemoteCodexSession {
                target,
                prefix,
                cwd,
                title_hint,
            } => {
                let key = self.server.start_remote_codex_session(
                    &target,
                    prefix.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                )?;
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
                    if let Err(error) = persist_remote_generated_copy(
                        &machine,
                        &session_id,
                        &cwd,
                        title.as_deref(),
                        precis.as_deref(),
                        summary.as_deref(),
                        "manual",
                    ) {
                        warn!(
                            machine_key=%machine.machine_key,
                            session_id=%session_id,
                            error=%error,
                            "failed to persist remote session copy hints"
                        );
                    }
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
                let updated = self.server.set_live_session_keep_alive(&path, keep_alive)?;
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
            } => {
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
            } => {
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
                initial_cols,
                initial_rows,
            } => {
                let key = self.server.ensure_remote_runtime_codex_session(
                    &session_id,
                    cwd.as_deref(),
                    require_existing,
                )?;
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
                initial_cols,
                initial_rows,
            } => {
                let key = self
                    .server
                    .start_remote_runtime_codex_session(&session_id, cwd.as_deref())?;
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
            ServerRequest::TerminalEnsure { path } => {
                let message = self.ensure_terminal_for_path(&path)?;
                ServerResponse::Ack { message }
            }
            ServerRequest::TerminalRead { path, cursor } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
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
                }
            }
            ServerRequest::TerminalSnapshot { path } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                let text = self
                    .terminals
                    .session_screen_snapshot(&runtime_path)
                    .unwrap_or_default();
                ServerResponse::TerminalSnapshot {
                    text,
                    running: self.terminals.session_is_running(&runtime_path),
                    runtime_output_seen: self.terminals.session_has_runtime_output(&runtime_path),
                }
            }
            ServerRequest::TerminalRetainedSnapshot { path } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                let text = self
                    .terminals
                    .session_snapshot(&runtime_path)
                    .unwrap_or_default();
                ServerResponse::TerminalRetainedSnapshot {
                    text,
                    running: self.terminals.session_is_running(&runtime_path),
                    runtime_output_seen: self.terminals.session_has_runtime_output(&runtime_path),
                }
            }
            ServerRequest::TerminalWrite { path, data } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
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
                self.terminals.resize(&runtime_path, cols, rows)?;
                ServerResponse::Ack { message: None }
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

fn snapshot_session_requires_terminal_runtime(session: &SnapshotSessionView) -> bool {
    matches!(
        session.source,
        crate::SessionSource::LiveLocal | crate::SessionSource::LiveSsh
    ) && session.kind != SessionKind::Document
}

fn apply_terminal_runtime_truth_to_snapshot(
    server: &YggtermServer,
    runtime_keys: &HashSet<String>,
    snapshot: &mut ServerUiSnapshot,
) {
    snapshot.live_sessions.retain(|session| {
        runtime_keys.contains(&server.terminal_runtime_key_for_path(&session.session_path))
    });

    let active_path = snapshot.active_session_path.clone().or_else(|| {
        snapshot
            .active_session
            .as_ref()
            .map(|session| session.session_path.clone())
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
    if !active_requires_runtime || active_runtime_present {
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

fn mark_daemon_activity(last_activity_ms: &AtomicU64) {
    last_activity_ms.store(current_millis_u64(), Ordering::Relaxed);
}

fn daemon_should_idle_shutdown(
    home_dir: &Path,
    endpoint: &ServerEndpoint,
    last_activity_ms: &AtomicU64,
    idle_shutdown_ms: u64,
) -> bool {
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
        ServerRequest::TerminalEnsure { .. } => "terminal_ensure",
        ServerRequest::TerminalRead { .. } => "terminal_read",
        ServerRequest::TerminalSnapshot { .. } => "terminal_snapshot",
        ServerRequest::TerminalRetainedSnapshot { .. } => "terminal_retained_snapshot",
        ServerRequest::TerminalWrite { .. } => "terminal_write",
        ServerRequest::TerminalResize { .. } => "terminal_resize",
        ServerRequest::SyncExternalWindow => "sync_external_window",
        ServerRequest::RaiseExternalWindow => "raise_external_window",
        ServerRequest::SyncTheme { .. } => "sync_theme",
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

    paths
        .into_iter()
        .filter_map(|path| {
            let endpoint = ServerEndpoint::UnixSocket(path);
            let runtime = status(&endpoint).ok()?;
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
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartSshSession {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
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
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartRemoteCodexSession {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
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
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartLocalSession {
            session_kind: kind,
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
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
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartCommandSession {
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            launch_command: launch_command.to_string(),
            source_label: source_label.map(ToOwned::to_owned),
        },
    )?)
}

pub fn ensure_remote_runtime_codex_session(
    endpoint: &ServerEndpoint,
    session_id: &str,
    cwd: Option<&str>,
    require_existing: bool,
    initial_size: Option<(u16, u16)>,
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::EnsureRemoteRuntimeCodexSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            require_existing,
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
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::StartRemoteRuntimeCodexSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
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
) -> Result<(u64, Vec<TerminalStreamChunk>, bool, bool, bool)> {
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
        } => Ok((
            cursor,
            chunks,
            running,
            runtime_output_seen,
            eof_without_output,
        )),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected terminal stream response: {:?}", other),
    }
}

pub fn terminal_snapshot(endpoint: &ServerEndpoint, path: &str) -> Result<(String, bool, bool)> {
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
        } => Ok((text, running, runtime_output_seen)),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected terminal snapshot response: {:?}", other),
    }
}

pub fn terminal_retained_snapshot(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(String, bool, bool)> {
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
        } => Ok((text, running, runtime_output_seen)),
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
    let daemon_executable = daemon_executable
        .canonicalize()
        .unwrap_or_else(|_| daemon_executable.to_path_buf());
    expect_ack(send_request(
        endpoint,
        &ServerRequest::HotRestart {
            daemon_executable: daemon_executable.display().to_string(),
            expected_version: expected_version.map(ToOwned::to_owned),
            expected_build_id,
            reason: reason.map(ToOwned::to_owned),
        },
    )?)
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

fn spawn_active_terminal_prewarm(
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
    home_dir: PathBuf,
) {
    std::thread::spawn(move || {
        let paths = {
            let runtime = lock_daemon_runtime(&runtime, "startup_prewarm_active_path");
            let mut paths = Vec::<String>::new();
            if runtime.server.active_view_mode() == WorkspaceViewMode::Terminal
                && runtime.server.active_session_supports_terminal()
            {
                if let Some(path) = runtime.server.active_session_path().map(ToOwned::to_owned) {
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
            paths
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
            append_trace_event(
                &home_dir,
                "daemon",
                "startup_prewarm",
                "begin",
                serde_json::json!({ "path": path }),
            );
            let outcome = {
                let mut runtime = lock_daemon_runtime(&runtime, "startup_prewarm_ensure");
                runtime.ensure_terminal_for_path_for_startup_prewarm(&path)
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
    #[cfg(target_os = "linux")]
    if let Ok(current_exe) = std::env::current_exe() {
        let _ = cleanup_legacy_linux_daemon_processes(endpoint, &current_exe);
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
        let mut restart_after_exit = None::<PathBuf>;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    let runtime = runtime.clone();
                    let last_activity_ms = last_activity_ms.clone();
                    match handle_unix_stream(stream, runtime, last_activity_ms) {
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
fn linux_home_has_reachable_terminal_runtime(home: &Path) -> bool {
    let Ok(entries) = fs::read_dir(home) else {
        return false;
    };
    for entry in entries.flatten() {
        let socket_path = entry.path();
        if parse_versioned_server_socket_name(&socket_path).is_none() {
            continue;
        }
        let endpoint = ServerEndpoint::UnixSocket(socket_path);
        if status(&endpoint)
            .ok()
            .is_some_and(|runtime| runtime.terminal_session_count > 0)
        {
            return true;
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn linux_home_has_recoverable_runtime_activity(home: &Path) -> bool {
    linux_home_has_live_bridge_process(home) || linux_home_has_reachable_terminal_runtime(home)
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
        let has_gui_clients = daemon_home
            .as_deref()
            .and_then(|home| active_client_instance_records(home, &default_endpoint(home)).ok())
            .is_some_and(|records| !records.is_empty());
        let has_recoverable_runtime_activity = daemon_home
            .as_deref()
            .is_some_and(linux_home_has_recoverable_runtime_activity);
        let has_clients = has_gui_clients || has_recoverable_runtime_activity;
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
                "skipped_cross_home_orphan": skipped_cross_home_orphan,
                "same_home_daemon_count": same_home_daemon_count,
                "oldest_same_home_pid": oldest_same_home_pid,
                "reap_after_ms": reap_after_ms,
                "current_socket_inode": current_socket_inode,
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
    let request = read_request(stream.try_clone().context("cloning unix stream")?)?;
    mark_daemon_activity(last_activity_ms.as_ref());
    let response = daemon_request_response(&runtime, request.clone());
    let outcome = daemon_request_outcome_for_response(&request, &response);
    write_response(&mut stream, &response)?;
    trim_process_heap_if_supported();
    Ok(outcome)
}

fn handle_tcp_stream(
    mut stream: std::net::TcpStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
) -> Result<DaemonRequestOutcome> {
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
        | ServerRequest::StartRemoteRuntimeCodexSession { .. } => DAEMON_LONG_REQUEST_IO_TIMEOUT_MS,
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
        RemoteMachineRefreshQueueStatus, apply_terminal_runtime_truth_to_snapshot,
        daemon_background_copy_chore_enabled_from_env, mark_remote_machine_refresh_queued,
        terminal_sidebar_snapshot_from_screen,
    };
    use std::collections::HashSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::collect_remote_copy_candidates;
    #[cfg(target_os = "linux")]
    use super::legacy_daemon_reap_applies_to_home;
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
        cleanup_legacy_unix_daemons, daemon_binary_is_legacy, default_endpoint,
        parse_versioned_server_socket_name, unix_socket_path_fits_platform,
        versioned_server_socket_alias_candidates, versioned_socket_alias_is_legacy,
    };
    use crate::{
        GhosttyHostSupport, PersistedDaemonState, PersistedLiveSession, RemoteDeployState,
        RemoteMachineHealth, RemoteMachineSnapshot, RemoteScannedSession, ServerUiSnapshot,
        SessionKind, SessionSource, SnapshotPreview, SnapshotSessionView, TerminalBackend,
        TerminalLaunchPhase, WorkspaceViewMode, YggtermServer, remote_scanned_session_path,
    };
    use yggui_contract::UiTheme;

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
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::StartSshSession {
                target: "dev".to_string(),
                prefix: None,
                cwd: None,
                title_hint: None,
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
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
    fn versioned_socket_alias_policy_only_targets_older_versions() {
        let current = (2, 1, 32);
        assert!(versioned_socket_alias_is_legacy(current, (2, 1, 31)));
        assert!(!versioned_socket_alias_is_legacy(current, current));
        assert!(!versioned_socket_alias_is_legacy(current, (2, 1, 33)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn daemon_binary_is_legacy_for_deleted_proc_exe_target() {
        let current = Path::new("/home/pi/.yggterm/bin/yggterm");
        assert!(daemon_binary_is_legacy(
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
    fn startup_prewarm_skips_remote_snapshot_seed_for_latency_budget() {
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
