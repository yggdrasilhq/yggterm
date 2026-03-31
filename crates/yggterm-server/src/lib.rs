mod app_control;
mod attach;
mod codex_cli;
mod daemon;
mod host;
mod protocol;
mod terminal;

pub use app_control::{
    AppControlCommand, AppControlDragCommand, AppControlDragPlacement, AppControlRequest,
    AppControlResponse, AppControlViewMode, ScreenshotTarget, app_control_captures_dir,
    app_control_requests_dir, app_control_requests_pending, app_control_responses_dir,
    complete_app_control_request, current_millis, default_recording_output_path,
    default_screenshot_output_path, enqueue_app_control_request, enqueue_screen_recording_request,
    enqueue_screenshot_request, take_next_app_control_request, wait_for_app_control_response,
};
pub use attach::{AttachMetadata, run_attach};
pub use codex_cli::{ManagedCliTool, ManagedCliToolStatus};
pub use daemon::{
    ServerEndpoint, ServerRequest, ServerResponse, ServerRuntimeStatus, TerminalStreamChunk,
    cleanup_legacy_daemons, connect_ssh, connect_ssh_custom, default_endpoint, focus_live,
    focus_live_with_view, open_remote_session, open_remote_session_with_view, open_stored_session,
    open_stored_session_with_view, ping, raise_external_window, refresh_managed_cli,
    refresh_preview,
    refresh_remote_machine, remove_session, remove_ssh_target, request_terminal_launch, run_daemon,
    set_all_preview_blocks_folded, set_view_mode, shutdown, snapshot, start_command_session,
    start_local_session, start_local_session_at, start_ssh_session_at, status,
    switch_agent_session_mode, sync_external_window, sync_theme, terminal_ensure, terminal_read,
    terminal_resize, terminal_write, toggle_preview_block,
};
pub use host::{GhosttyHostKind, GhosttyHostSupport, GhosttyTerminalHostMode, detect_ghostty_host};
pub use protocol::{
    YGG_LOADING_NOTIFICATION_AFTER_MS, YGG_PROTOCOL_SCHEMA_VERSION, YggCachePolicy,
    YggEventEnvelope, YggEventKind, YggOperationPriority, YggProgress, YggRequestMeta, YggSurface,
    YggTarget,
};
pub use terminal::{TerminalChunk, TerminalManager, TerminalReadResult};

use anyhow::Context;
use codex_cli::{
    ManagedCliAction, ManagedCliRefreshReport, ensure_local_managed_cli, managed_cli_shell_command,
    refresh_local_managed_cli, summarize_managed_cli_report, sync_terminal_identity_env,
    terminal_identity_shell_exports,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;
use time::{OffsetDateTime, UtcOffset, macros::format_description};
use tracing::warn;
use uuid::Uuid;
use yggterm_core::{
    PerfSpan, SessionNode, SessionNodeKind, SessionStore, SessionTitleStore, TranscriptRole,
    UiTheme, WorkspaceDocument, WorkspaceDocumentKind, append_trace_event, event_trace_path,
    follow_trace_lines, generation_context_from_messages, looks_like_generated_fallback_title,
    read_codex_session_identity_fields, read_codex_transcript_messages,
    read_codex_transcript_messages_limited, read_trace_tail, resolve_yggterm_home,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceViewMode {
    Terminal,
    Rendered,
}

const REMOTE_COMMAND_CACHE_VERIFY_TTL_MS: u64 = 60_000;

#[derive(Debug, Clone)]
struct RemoteCommandCacheEntry {
    binary_expr: String,
    verified_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteProtocolDescriptor {
    version: String,
    build_id: u64,
}

static REMOTE_YGGTERM_COMMAND_CACHE: OnceLock<
    Mutex<std::collections::HashMap<String, RemoteCommandCacheEntry>>,
> = OnceLock::new();
static REMOTE_YGGTERM_COMMAND_RESOLVE_LOCKS: OnceLock<
    Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>>,
> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalBackend {
    Xterm,
    Ghostty,
    Mock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadataEntry {
    pub label: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRenderedSection {
    pub title: &'static str,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewTone {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPreviewBlock {
    pub role: &'static str,
    pub timestamp: String,
    pub tone: PreviewTone,
    pub folded: bool,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPreview {
    pub summary: Vec<SessionMetadataEntry>,
    pub blocks: Vec<SessionPreviewBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionSource {
    Stored,
    LiveSsh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Codex,
    CodexLiteLlm,
    Shell,
    SshShell,
    Document,
}

impl SessionKind {
    pub fn is_agent(self) -> bool {
        matches!(self, SessionKind::Codex | SessionKind::CodexLiteLlm)
    }
}

fn live_session_default_title(kind: SessionKind, cwd: Option<&str>, fallback: &str) -> String {
    match kind {
        SessionKind::Shell | SessionKind::SshShell => cwd
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| fallback.to_string()),
        _ => fallback.to_string(),
    }
}

fn live_session_default_summary(kind: SessionKind, target: &SshConnectTarget) -> String {
    let cwd = target
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    match kind {
        SessionKind::SshShell => match cwd {
            Some(cwd) => format!("SSH terminal on {} rooted at {cwd}.", target.ssh_target),
            None => format!("SSH terminal on {}.", target.ssh_target),
        },
        SessionKind::Shell => match cwd {
            Some(cwd) => format!("Local shell rooted at {cwd}."),
            None => "Local shell terminal.".to_string(),
        },
        SessionKind::Codex => match cwd {
            Some(cwd) => format!("Local Codex terminal rooted at {cwd}."),
            None => "Local Codex terminal.".to_string(),
        },
        SessionKind::CodexLiteLlm => match cwd {
            Some(cwd) => format!("Local Codex LiteLLM terminal rooted at {cwd}."),
            None => "Local Codex LiteLLM terminal.".to_string(),
        },
        SessionKind::Document => "Document preview.".to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalLaunchPhase {
    Queued,
    BridgePending,
    RemoteBootstrap,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteDeployState {
    NotRequired,
    Planned,
    CopyingBinary,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnectTarget {
    pub label: String,
    pub kind: SessionKind,
    pub ssh_target: String,
    pub prefix: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteMachineHealth {
    Healthy,
    Cached,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteScannedSession {
    pub session_path: String,
    pub session_id: String,
    pub cwd: String,
    pub started_at: String,
    pub modified_epoch: i64,
    pub event_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    pub title_hint: String,
    #[serde(default)]
    pub recent_context: String,
    #[serde(default)]
    pub cached_precis: Option<String>,
    #[serde(default)]
    pub cached_summary: Option<String>,
    pub storage_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMachineSnapshot {
    pub machine_key: String,
    pub label: String,
    pub ssh_target: String,
    pub prefix: Option<String>,
    #[serde(default)]
    pub remote_binary_expr: Option<String>,
    #[serde(default = "default_remote_machine_deploy_state")]
    pub remote_deploy_state: RemoteDeployState,
    pub health: RemoteMachineHealth,
    pub sessions: Vec<RemoteScannedSession>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMetadataEntry {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRenderedSection {
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPreviewBlock {
    pub role: String,
    pub timestamp: String,
    pub tone: PreviewTone,
    pub folded: bool,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPreview {
    pub summary: Vec<SnapshotMetadataEntry>,
    pub blocks: Vec<SnapshotPreviewBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSessionView {
    pub id: String,
    pub session_path: String,
    pub title: String,
    pub kind: SessionKind,
    pub host_label: String,
    pub source: SessionSource,
    pub backend: TerminalBackend,
    pub bridge_available: bool,
    pub launch_phase: TerminalLaunchPhase,
    pub remote_deploy_state: RemoteDeployState,
    pub launch_command: String,
    pub status_line: String,
    pub terminal_lines: Vec<String>,
    pub rendered_sections: Vec<SnapshotRenderedSection>,
    pub preview: SnapshotPreview,
    pub metadata: Vec<SnapshotMetadataEntry>,
    pub terminal_process_id: Option<u32>,
    pub terminal_window_id: Option<String>,
    pub terminal_host_token: Option<String>,
    pub terminal_host_mode: GhosttyTerminalHostMode,
    pub embedded_surface_id: Option<String>,
    pub embedded_surface_detail: Option<String>,
    pub last_launch_error: Option<String>,
    pub last_window_error: Option<String>,
    pub ssh_target: Option<String>,
    pub ssh_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerUiSnapshot {
    #[serde(default)]
    pub active_session_path: Option<String>,
    #[serde(default)]
    pub active_session: Option<SnapshotSessionView>,
    #[serde(default = "default_workspace_view_mode")]
    pub active_view_mode: WorkspaceViewMode,
    #[serde(default)]
    pub remote_machines: Vec<RemoteMachineSnapshot>,
    #[serde(default)]
    pub ssh_targets: Vec<SshConnectTarget>,
    #[serde(default)]
    pub live_sessions: Vec<SnapshotSessionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedStoredSession {
    pub path: String,
    #[serde(default = "default_persisted_stored_kind")]
    pub kind: SessionKind,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub title_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedLiveSession {
    pub key: String,
    pub id: String,
    pub title: String,
    #[serde(default = "default_persisted_live_kind")]
    pub kind: SessionKind,
    pub ssh_target: String,
    pub prefix: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDaemonState {
    #[serde(default)]
    pub active_session_path: Option<String>,
    #[serde(default = "default_workspace_view_mode")]
    pub active_view_mode: WorkspaceViewMode,
    #[serde(default)]
    pub ssh_targets: Vec<SshConnectTarget>,
    #[serde(default)]
    pub remote_machines: Vec<RemoteMachineSnapshot>,
    pub stored_sessions: Vec<PersistedStoredSession>,
    pub live_sessions: Vec<PersistedLiveSession>,
}

fn default_workspace_view_mode() -> WorkspaceViewMode {
    WorkspaceViewMode::Rendered
}

fn default_remote_machine_deploy_state() -> RemoteDeployState {
    RemoteDeployState::Planned
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedSessionView {
    pub id: String,
    pub session_path: String,
    pub title: String,
    pub kind: SessionKind,
    pub host_label: String,
    pub source: SessionSource,
    pub backend: TerminalBackend,
    pub bridge_available: bool,
    pub launch_phase: TerminalLaunchPhase,
    pub remote_deploy_state: RemoteDeployState,
    pub launch_command: String,
    pub status_line: String,
    pub terminal_lines: Vec<String>,
    pub rendered_sections: Vec<SessionRenderedSection>,
    pub preview: SessionPreview,
    pub metadata: Vec<SessionMetadataEntry>,
    pub terminal_process_id: Option<u32>,
    pub terminal_window_id: Option<String>,
    pub terminal_host_token: Option<String>,
    pub terminal_host_mode: GhosttyTerminalHostMode,
    pub embedded_surface_id: Option<String>,
    pub embedded_surface_detail: Option<String>,
    pub last_launch_error: Option<String>,
    pub last_window_error: Option<String>,
    pub ssh_target: Option<String>,
    pub ssh_prefix: Option<String>,
    pub stored_preview_hydrated: bool,
}

#[derive(Debug, Clone)]
pub struct YggtermServer {
    sessions: BTreeMap<String, ManagedSessionView>,
    active_session_path: Option<String>,
    active_view_mode: WorkspaceViewMode,
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_host: GhosttyHostSupport,
    ssh_targets: Vec<SshConnectTarget>,
    remote_machines: Vec<RemoteMachineSnapshot>,
    live_session_order: Vec<String>,
}

impl YggtermServer {
    pub fn new(
        tree: &SessionNode,
        prefer_ghostty_backend: bool,
        ghostty_host: GhosttyHostSupport,
        theme: UiTheme,
    ) -> Self {
        let _ = prefer_ghostty_backend;
        let backend = TerminalBackend::Xterm;
        sync_terminal_identity_env(theme);

        let mut this = Self {
            sessions: BTreeMap::new(),
            active_session_path: None,
            active_view_mode: WorkspaceViewMode::Rendered,
            backend,
            theme,
            ghostty_host,
            ssh_targets: Vec::new(),
            remote_machines: Vec::new(),
            live_session_order: Vec::new(),
        };

        if let Some(first_session) = first_session_leaf(tree) {
            this.open_or_focus_session(
                match first_session.kind {
                    SessionNodeKind::CodexSession => SessionKind::Codex,
                    SessionNodeKind::Document => SessionKind::Document,
                    SessionNodeKind::Group => SessionKind::Codex,
                },
                &first_session.path,
                Some(&first_session.session_id),
                Some(&first_session.cwd),
                Some(&first_session.title),
                None,
            );
        }

        this
    }

    pub fn backend(&self) -> TerminalBackend {
        self.backend
    }

    pub fn active_view_mode(&self) -> WorkspaceViewMode {
        self.active_view_mode
    }

    pub fn set_view_mode(&mut self, mode: WorkspaceViewMode) {
        self.active_view_mode = mode;
    }

    pub fn refresh_active_session_preview_from_source(&mut self) -> anyhow::Result<()> {
        let Some(path) = self.active_session_path.clone() else {
            return Ok(());
        };
        self.refresh_session_preview_from_source(&path)
    }

    pub fn refresh_session_preview_from_source(&mut self, path: &str) -> anyhow::Result<()> {
        if let Some((raw_machine_key, session_id)) = parse_remote_scanned_session_path(&path) {
            let machine_key = normalize_machine_key(raw_machine_key);
            self.refresh_remote_scanned_session_preview(&machine_key, session_id)?;
            return Ok(());
        }
        let Some(session) = self.sessions.get(path).cloned() else {
            return Ok(());
        };
        match session.source {
            SessionSource::Stored => self.refresh_stored_session_preview(path, &session)?,
            SessionSource::LiveSsh => {
                if let Some((raw_machine_key, session_id)) =
                    parse_remote_scanned_session_path(path)
                {
                    let machine_key = normalize_machine_key(raw_machine_key);
                    self.refresh_remote_scanned_session_preview(&machine_key, session_id)?;
                }
            }
        }
        Ok(())
    }

    pub fn sync_theme(&mut self, theme: UiTheme) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        sync_terminal_identity_env(theme);
        for session in self.sessions.values_mut() {
            let appearance = match theme {
                UiTheme::ZedDark => "dark",
                UiTheme::ZedLight => "light",
            };
            let launch_status = describe_launch_phase(
                session.source,
                session.launch_phase,
                session.remote_deploy_state,
                session.bridge_available,
            );
            session.status_line = format!(
                "{} · {} scheme requested · {}",
                match session.backend {
                    TerminalBackend::Xterm => "xterm.js",
                    TerminalBackend::Ghostty => "Ghostty",
                    TerminalBackend::Mock => "Mock",
                },
                appearance,
                launch_status,
            );
        }
    }

    pub fn open_or_focus_session(
        &mut self,
        kind: SessionKind,
        path: &str,
        session_id: Option<&str>,
        cwd: Option<&str>,
        title_hint: Option<&str>,
        document: Option<&WorkspaceDocument>,
    ) {
        let was_missing = !self.sessions.contains_key(path);
        let entry = self.sessions.entry(path.to_string()).or_insert_with(|| {
            build_session(
                kind,
                path,
                session_id,
                cwd,
                title_hint,
                document,
                self.backend,
                self.theme,
                self.ghostty_host.bridge_enabled,
                StoredPreviewHydrationMode::Eager,
            )
        });
        entry.kind = kind;
        entry.backend = self.backend;
        if let Some(session_id) = session_id {
            entry.id = session_id.to_string();
        }
        if let Some(cwd) = cwd {
            entry.metadata.retain(|entry| entry.label != "Cwd");
            entry.metadata.push(SessionMetadataEntry {
                label: "Cwd",
                value: cwd.to_string(),
            });
        }
        if let Some(title_hint) = title_hint {
            entry.title = title_hint.to_string();
        }
        if let Some(document) = document {
            hydrate_document_session(entry, document);
        }
        let should_refresh_stored_preview = !was_missing
            && entry.source == SessionSource::Stored
            && kind != SessionKind::Document
            && !entry.stored_preview_hydrated;
        self.active_session_path = Some(path.to_string());
        if should_refresh_stored_preview {
            let existing = self.sessions.get(path).cloned();
            if let Some(existing) = existing.as_ref() {
                let _ = self.refresh_stored_session_preview(path, existing);
            }
        }
    }

    pub fn active_session(&self) -> Option<&ManagedSessionView> {
        self.active_session_path
            .as_ref()
            .and_then(|path| self.sessions.get(path))
    }

    pub fn active_session_path(&self) -> Option<&str> {
        self.active_session_path.as_deref()
    }

    pub fn switch_agent_session_mode(
        &mut self,
        path: &str,
        target_kind: SessionKind,
    ) -> anyhow::Result<()> {
        if !target_kind.is_agent() {
            anyhow::bail!("target mode is not an agent session")
        }
        let existing = self
            .sessions
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("session not found: {path}"))?;
        if !existing.kind.is_agent() {
            anyhow::bail!("only codex sessions can switch mode")
        }
        if existing.kind == target_kind {
            return Ok(());
        }

        let replacement = if existing.source == SessionSource::Stored {
            let cwd = metadata_value(&existing, "Cwd");
            build_session(
                target_kind,
                path,
                Some(&existing.id),
                (!cwd.is_empty()).then_some(cwd.as_str()),
                Some(&existing.title),
                None,
                self.backend,
                self.theme,
                self.ghostty_host.bridge_enabled,
                StoredPreviewHydrationMode::Eager,
            )
        } else {
            let cwd = metadata_value(&existing, "Cwd");
            let target =
                local_session_target(target_kind, (!cwd.is_empty()).then_some(cwd.as_str()));
            let mut session = build_live_session(
                &existing.id,
                target_kind,
                &target,
                self.backend,
                self.theme,
                self.ghostty_host.bridge_enabled,
            );
            session.title = existing.title.clone();
            session
        };

        self.sessions.insert(path.to_string(), replacement);
        self.active_session_path = Some(path.to_string());
        self.active_view_mode = WorkspaceViewMode::Terminal;
        self.request_terminal_launch_for_path(path);
        Ok(())
    }

    pub fn request_terminal_launch_for_path(&mut self, path: &str) {
        if let Ok(home) = resolve_yggterm_home() {
            append_trace_event(
                &home,
                "server",
                "session",
                "request_terminal_launch",
                serde_json::json!({ "path": path }),
            );
        }
        self.active_session_path = Some(path.to_string());
        self.request_terminal_launch_for_active();
    }

    fn resolve_session_storage_key<'a>(&'a self, path: &'a str) -> Option<&'a str> {
        if self.sessions.contains_key(path) {
            return Some(path);
        }
        self.sessions
            .iter()
            .find(|(_, session)| session.session_path == path)
            .map(|(key, _)| key.as_str())
    }

    pub fn terminal_spec(&self, path: &str) -> Option<(String, Option<String>)> {
        let key = self.resolve_session_storage_key(path)?;
        self.sessions.get(key).and_then(|session| {
            if session.kind == SessionKind::Document {
                return recipe_terminal_spec(session);
            }
            let cwd = session
                .metadata
                .iter()
                .find(|entry| entry.label == "Cwd")
                .map(|entry| entry.value.clone());
            Some((session.launch_command.clone(), cwd))
        })
    }

    pub fn terminal_stop_command(&self, path: &str) -> Option<String> {
        let key = self.resolve_session_storage_key(path)?;
        let session = self.sessions.get(key)?;
        if session.kind == SessionKind::Document {
            return recipe_terminal_spec(session).map(|_| "exit\r".to_string());
        }
        if session.session_path.starts_with("remote-session://") {
            return None;
        }
        match session.kind {
            SessionKind::Codex | SessionKind::CodexLiteLlm => Some("/quit\r".to_string()),
            SessionKind::Shell | SessionKind::SshShell => Some("exit\r".to_string()),
            SessionKind::Document => None,
        }
    }

    pub fn remote_shutdown_targets(&self) -> Vec<(RemoteMachineSnapshot, String)> {
        let mut targets = Vec::new();
        let mut seen = HashSet::<(String, String)>::new();
        for path in self.sessions.keys() {
            let Some((raw_machine_key, session_id)) = parse_remote_scanned_session_path(path)
            else {
                continue;
            };
            let machine_key = normalize_machine_key(raw_machine_key);
            let dedupe_key = (machine_key.clone(), session_id.to_string());
            if !seen.insert(dedupe_key) {
                continue;
            }
            if let Some(machine) = self
                .remote_machines
                .iter()
                .find(|machine| machine.machine_key == machine_key)
                .cloned()
            {
                targets.push((machine, session_id.to_string()));
            }
        }
        targets
    }

    pub fn ssh_targets(&self) -> &[SshConnectTarget] {
        &self.ssh_targets
    }

    pub fn remove_ssh_targets_for_machine(&mut self, machine_key: &str) -> usize {
        let machine_key = normalize_machine_key(machine_key);
        let before = self.ssh_targets.len();
        self.ssh_targets
            .retain(|target| machine_key_from_ssh_target(&target.ssh_target) != machine_key);
        let removed = before.saturating_sub(self.ssh_targets.len());
        if removed == 0 {
            return 0;
        }

        let removed_ssh_targets = self
            .sessions
            .iter()
            .filter_map(|(path, session)| {
                session
                    .ssh_target
                    .as_ref()
                    .filter(|ssh_target| machine_key_from_ssh_target(ssh_target) == machine_key)
                    .map(|_| path.clone())
            })
            .collect::<Vec<_>>();
        let removed_remote_sessions = self
            .sessions
            .keys()
            .filter(|path| path.starts_with(&format!("remote-session://{machine_key}/")))
            .cloned()
            .collect::<Vec<_>>();
        for path in removed_ssh_targets
            .into_iter()
            .chain(removed_remote_sessions.into_iter())
        {
            self.sessions.remove(&path);
            self.live_session_order.retain(|entry| entry != &path);
            if self.active_session_path.as_deref() == Some(path.as_str()) {
                self.active_session_path = None;
            }
        }

        self.remote_machines
            .retain(|machine| machine.machine_key != machine_key);
        removed
    }

    pub fn remove_live_session(&mut self, path: &str) -> anyhow::Result<bool> {
        let Some((resolved_key, session)) = self
            .sessions
            .get(path)
            .cloned()
            .map(|session| (path.to_string(), session))
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|(_, session)| {
                        session.source == SessionSource::LiveSsh && session.session_path == path
                    })
                    .map(|(key, session)| (key.clone(), session.clone()))
            })
        else {
            return Ok(false);
        };
        if session.source != SessionSource::LiveSsh {
            anyhow::bail!("only live sessions can be removed through this path");
        }
        self.sessions.remove(&resolved_key);
        self.live_session_order
            .retain(|existing| existing != &resolved_key);
        if self.active_session_path.as_deref() == Some(resolved_key.as_str())
            || self.active_session_path.as_deref() == Some(path)
        {
            self.active_session_path = self
                .live_session_order
                .iter()
                .find(|candidate| self.sessions.contains_key(candidate.as_str()))
                .cloned()
                .or_else(|| self.sessions.keys().next().cloned());
        }
        Ok(true)
    }

    pub fn set_session_title_hint(&mut self, session_path: &str, title: &str) {
        if let Some(session) = self.sessions.get_mut(session_path) {
            session.title = title.to_string();
        }
        for machine in &mut self.remote_machines {
            for scanned in &mut machine.sessions {
                if scanned.session_path == session_path {
                    scanned.title_hint = title.to_string();
                    return;
                }
            }
        }
    }

    pub fn set_session_precis_hint(&mut self, session_path: &str, precis: &str) {
        if let Some(session) = self.sessions.get_mut(session_path) {
            upsert_session_metadata(&mut session.preview.summary, "Precis", precis.to_string());
        }
        for machine in &mut self.remote_machines {
            for scanned in &mut machine.sessions {
                if scanned.session_path == session_path {
                    scanned.cached_precis = Some(precis.to_string());
                    return;
                }
            }
        }
    }

    pub fn set_session_summary_hint(&mut self, session_path: &str, summary: &str) {
        if let Some(session) = self.sessions.get_mut(session_path) {
            upsert_session_metadata(&mut session.preview.summary, "Summary", summary.to_string());
        }
        for machine in &mut self.remote_machines {
            for scanned in &mut machine.sessions {
                if scanned.session_path == session_path {
                    scanned.cached_summary = Some(summary.to_string());
                    return;
                }
            }
        }
    }

    pub fn restore_session_preview_if_empty(
        &mut self,
        session_path: &str,
        preview: SessionPreview,
        rendered_sections: Vec<SessionRenderedSection>,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(session_path) else {
            return false;
        };
        if !session.preview.blocks.is_empty() {
            return false;
        }
        session.preview = preview;
        session.rendered_sections = rendered_sections;
        true
    }

    pub fn remote_machines(&self) -> &[RemoteMachineSnapshot] {
        &self.remote_machines
    }

    pub fn live_sessions(&self) -> Vec<ManagedSessionView> {
        let sessions = self
            .live_session_order
            .iter()
            .filter_map(|key| self.sessions.get(key).cloned())
            .collect::<Vec<_>>();
        sessions
    }

    pub fn snapshot(&self) -> ServerUiSnapshot {
        let live_sessions = self
            .live_session_order
            .iter()
            .filter_map(|key| self.sessions.get(key))
            .cloned()
            .map(snapshot_session_view)
            .collect::<Vec<_>>();
        let active_session = self.active_session().cloned().or_else(|| {
            self.active_session_path.as_deref().and_then(|path| {
                synthesize_remote_active_session(
                    path,
                    &self.remote_machines,
                    self.backend,
                    self.theme,
                    self.ghostty_host.bridge_enabled,
                )
            })
        });
        let active_session_path = active_session
            .as_ref()
            .map(|session| session.session_path.clone())
            .or_else(|| self.active_session_path.clone());
        ServerUiSnapshot {
            active_session_path,
            active_session: active_session.map(snapshot_session_view),
            active_view_mode: self.active_view_mode,
            remote_machines: self.remote_machines.clone(),
            ssh_targets: self.ssh_targets.clone(),
            live_sessions,
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: ServerUiSnapshot) {
        self.active_view_mode = snapshot.active_view_mode;
        self.active_session_path = snapshot.active_session_path.clone();
        self.remote_machines = snapshot.remote_machines;
        self.ssh_targets = snapshot.ssh_targets;
        self.live_session_order = snapshot
            .live_sessions
            .iter()
            .map(|session| session.session_path.clone())
            .collect();
        self.sessions.clear();
        if let Some(active) = snapshot.active_session {
            let key = active.session_path.clone();
            self.sessions
                .insert(key, managed_session_from_snapshot(active));
        }
        for live in snapshot.live_sessions {
            let key = live.session_path.clone();
            self.sessions
                .insert(key, managed_session_from_snapshot(live));
        }
        if let Some(active_path) = self.active_session_path.clone()
            && !self.sessions.contains_key(&active_path)
            && let Some(session) = synthesize_remote_active_session(
                &active_path,
                &self.remote_machines,
                self.backend,
                self.theme,
                self.ghostty_host.bridge_enabled,
            )
        {
            if !self
                .live_session_order
                .iter()
                .any(|path| path == &active_path)
            {
                self.live_session_order.insert(0, active_path.clone());
            }
            self.sessions.insert(active_path, session);
        }
        if self
            .active_session_path
            .as_ref()
            .is_some_and(|path| !self.sessions.contains_key(path))
        {
            self.active_session_path = self.live_session_order.first().cloned();
        }
    }

    pub fn persisted_state(&self) -> PersistedDaemonState {
        let stored_sessions = self
            .sessions
            .iter()
            .filter_map(|(path, session)| {
                (session.source == SessionSource::Stored).then(|| PersistedStoredSession {
                    path: path.clone(),
                    kind: session.kind,
                    session_id: Some(session.id.clone()),
                    cwd: session
                        .metadata
                        .iter()
                        .find(|entry| entry.label == "Cwd")
                        .map(|entry| entry.value.clone()),
                    title_hint: Some(session.title.clone()),
                })
            })
            .collect();
        let live_sessions = self
            .live_session_order
            .iter()
            .filter_map(|key| self.sessions.get(key).map(|session| (key, session)))
            .filter_map(|(key, session)| {
                session
                    .ssh_target
                    .as_ref()
                    .map(|ssh_target| PersistedLiveSession {
                        key: key.clone(),
                        id: session.id.clone(),
                        title: session.title.clone(),
                        kind: session.kind,
                        ssh_target: ssh_target.clone(),
                        prefix: session.ssh_prefix.clone(),
                        cwd: session
                            .metadata
                            .iter()
                            .find(|entry| entry.label == "Cwd")
                            .map(|entry| entry.value.clone()),
                    })
            })
            .collect();

        PersistedDaemonState {
            active_session_path: self.active_session_path.clone(),
            active_view_mode: self.active_view_mode,
            ssh_targets: self.ssh_targets.clone(),
            remote_machines: self.remote_machines.clone(),
            stored_sessions,
            live_sessions,
        }
    }

    pub fn restore_persisted_state(
        &mut self,
        state: PersistedDaemonState,
        store: Option<&SessionStore>,
    ) {
        let perf_home = resolve_yggterm_home().ok();
        let total_restore_perf = perf_home
            .clone()
            .map(|home| PerfSpan::start(home, "server", "restore_persisted_state"));
        self.ssh_targets = state
            .ssh_targets
            .into_iter()
            .filter_map(|mut target| {
                target.ssh_target = canonicalize_ssh_target_alias(&target.ssh_target);
                if is_loopback_ssh_target(&target.ssh_target) {
                    return None;
                }
                target.label = ssh_machine_label(&target);
                Some(target)
            })
            .collect();
        for target in &mut self.ssh_targets {
            target.label = ssh_machine_label(target);
        }
        self.remote_machines = state
            .remote_machines
            .into_iter()
            .filter_map(|mut machine| {
                let legacy_machine_key = machine.machine_key.clone();
                machine.machine_key = normalize_machine_key(&machine.machine_key);
                machine.ssh_target = canonicalize_ssh_target_alias(&machine.ssh_target);
                if is_loopback_ssh_target(&machine.ssh_target) {
                    return None;
                }
                let mirrored_sessions =
                    load_remote_machine_sessions_from_mirror(&machine.machine_key)
                        .or_else(|_| load_remote_machine_sessions_from_mirror(&legacy_machine_key))
                        .unwrap_or_default();
                machine.sessions = if machine.sessions.is_empty() {
                    mirrored_sessions
                } else {
                    if !mirrored_sessions.is_empty() {
                        machine.sessions = overlay_mirrored_remote_sessions(
                            &machine.machine_key,
                            &machine.sessions,
                        );
                    }
                    let _ = machine
                        .sessions
                        .iter()
                        .cloned()
                        .map(|mut session| {
                            if let Some((session_machine_key, session_id)) =
                                parse_remote_scanned_session_path(&session.session_path)
                            {
                                let canonical_session_machine_key =
                                    normalize_machine_key(session_machine_key);
                                if canonical_session_machine_key != machine.machine_key {
                                    session.session_path = remote_scanned_session_path(
                                        &machine.machine_key,
                                        session_id,
                                    );
                                }
                            }
                            session
                        })
                        .collect::<Vec<_>>();
                    if !machine.sessions.is_empty() {
                        overlay_mirrored_remote_sessions(&machine.machine_key, &machine.sessions)
                    } else {
                        machine.sessions.clone()
                    }
                };
                if !machine.sessions.is_empty() {
                    machine.health = RemoteMachineHealth::Cached;
                }
                Some(machine)
            })
            .map(|mut machine| {
                if machine.health == RemoteMachineHealth::Healthy {
                    machine.health = RemoteMachineHealth::Cached;
                }
                machine
            })
            .collect();
        for target in self.ssh_targets.clone() {
            self.ensure_remote_machine_stub(&target);
        }
        for machine in &mut self.remote_machines {
            if machine.sessions.is_empty()
                && let Ok(mirrored) = load_remote_machine_sessions_from_mirror(&machine.machine_key)
                && !mirrored.is_empty()
            {
                machine.sessions = mirrored;
                machine.health = RemoteMachineHealth::Cached;
            }
        }
        let desired_active_path = state.active_session_path.clone();
        let normalized_desired_active_path = desired_active_path.as_deref().map(|path| {
            if let Some((machine_key, session_id)) = parse_remote_scanned_session_path(path) {
                remote_scanned_session_path(&normalize_machine_key(machine_key), session_id)
            } else {
                path.to_string()
            }
        });
        let stored_restore_perf = perf_home
            .clone()
            .map(|home| PerfSpan::start(home, "server", "restore_stored_sessions"));
        for session in state.stored_sessions {
            let path = if let Some((machine_key, session_id)) =
                parse_remote_scanned_session_path(&session.path)
            {
                remote_scanned_session_path(&normalize_machine_key(machine_key), session_id)
            } else {
                session.path.clone()
            };
            let hydration_mode = if session.kind == SessionKind::Document
                || normalized_desired_active_path
                    .as_deref()
                    .is_some_and(|active_path| active_path == path)
            {
                StoredPreviewHydrationMode::Eager
            } else {
                StoredPreviewHydrationMode::Deferred
            };
            let document = if session.kind == SessionKind::Document {
                store.and_then(|store| store.load_document(&path).ok().flatten())
            } else {
                None
            };
            let entry = self.sessions.entry(path.clone()).or_insert_with(|| {
                build_session(
                    session.kind,
                    &path,
                    session.session_id.as_deref(),
                    session.cwd.as_deref(),
                    session.title_hint.as_deref(),
                    document.as_ref(),
                    self.backend,
                    self.theme,
                    self.ghostty_host.bridge_enabled,
                    hydration_mode,
                )
            });
            entry.kind = session.kind;
            entry.backend = self.backend;
            if let Some(session_id) = session.session_id.as_deref() {
                entry.id = session_id.to_string();
            }
            if let Some(cwd) = session.cwd.as_deref() {
                entry.metadata.retain(|entry| entry.label != "Cwd");
                entry.metadata.push(SessionMetadataEntry {
                    label: "Cwd",
                    value: cwd.to_string(),
                });
            }
            if let Some(title_hint) = session.title_hint.as_deref() {
                entry.title = title_hint.to_string();
            }
            if let Some(document) = document.as_ref() {
                hydrate_document_session(entry, document);
            }
            entry.stored_preview_hydrated = session.kind == SessionKind::Document
                || matches!(hydration_mode, StoredPreviewHydrationMode::Eager);
        }
        if let Some(span) = stored_restore_perf {
            span.finish(serde_json::json!({
                "stored_sessions": self
                    .sessions
                    .values()
                    .filter(|session| session.source == SessionSource::Stored)
                    .count(),
            }));
        }
        let mut restored_live_fingerprints = Vec::<(SessionKind, String, Option<String>)>::new();
        for live in state.live_sessions {
            if is_legacy_demo_live_session(&live) {
                continue;
            }
            let fingerprint = (live.kind, live.ssh_target.clone(), live.prefix.clone());
            if restored_live_fingerprints
                .iter()
                .any(|existing| existing == &fingerprint)
            {
                continue;
            }
            restored_live_fingerprints.push(fingerprint);
            self.restore_live_session(live);
        }
        self.active_view_mode = state.active_view_mode;
        if let Some(path) = desired_active_path {
            let active_path =
                if let Some((machine_key, session_id)) = parse_remote_scanned_session_path(&path) {
                    remote_scanned_session_path(&normalize_machine_key(machine_key), session_id)
                } else {
                    path
                };
            if !self.sessions.contains_key(&active_path)
                && let Some(session) = synthesize_remote_active_session(
                    &active_path,
                    &self.remote_machines,
                    self.backend,
                    self.theme,
                    self.ghostty_host.bridge_enabled,
                )
            {
                if !self
                    .live_session_order
                    .iter()
                    .any(|existing| existing == &active_path)
                {
                    self.live_session_order.insert(0, active_path.clone());
                }
                self.sessions.insert(active_path.clone(), session);
            }
            if self.sessions.contains_key(&active_path) {
                self.active_session_path = Some(active_path.clone());
                if let Some(existing) = self.sessions.get(&active_path).cloned()
                    && existing.source == SessionSource::Stored
                    && !existing.stored_preview_hydrated
                {
                    let _ = self.refresh_stored_session_preview(&active_path, &existing);
                }
                if self.active_view_mode == WorkspaceViewMode::Terminal {
                    self.request_terminal_launch_for_path(&active_path);
                }
            }
        }
        if let Some(span) = total_restore_perf {
            span.finish(serde_json::json!({
                "sessions": self.sessions.len(),
                "remote_machines": self.remote_machines.len(),
                "live_sessions": self.live_session_order.len(),
            }));
        }
    }

    pub fn focus_live_session(&mut self, key: &str) {
        let resolved_key = if self.sessions.contains_key(key) {
            Some(key.to_string())
        } else {
            self.sessions
                .iter()
                .find(|(_, session)| {
                    session.source == SessionSource::LiveSsh && session.session_path == key
                })
                .map(|(session_key, _)| session_key.clone())
        };
        if let Some(resolved_key) = resolved_key {
            self.active_session_path = Some(resolved_key);
            self.active_view_mode = WorkspaceViewMode::Terminal;
            self.request_terminal_launch_for_active();
        }
    }

    pub fn connect_ssh_target(&mut self, target_ix: usize) -> (Option<String>, bool) {
        let Some(target) = self.ssh_targets.get(target_ix).cloned() else {
            return (None, false);
        };
        self.connect_ssh_like_target(&target)
    }

    pub fn connect_ssh_custom(
        &mut self,
        target: &str,
        prefix: Option<&str>,
    ) -> anyhow::Result<(String, bool)> {
        let ssh_target = canonicalize_ssh_target_alias(target);
        if ssh_target.is_empty() {
            anyhow::bail!("enter an SSH target such as dev, pi@raspberry, or user@ip");
        }
        let prefix = prefix
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let label = ssh_target
            .rsplit('@')
            .next()
            .unwrap_or(ssh_target.as_str())
            .to_string();
        let target = SshConnectTarget {
            label,
            kind: SessionKind::SshShell,
            ssh_target,
            prefix,
            cwd: None,
        };
        let (key, reused) = self.connect_ssh_like_target(&target);
        key.map(|key| (key, reused))
            .ok_or_else(|| anyhow::anyhow!("failed to create ssh session"))
    }

    pub fn start_ssh_session(
        &mut self,
        target: &str,
        prefix: Option<&str>,
        cwd: Option<&str>,
        title_hint: Option<&str>,
    ) -> anyhow::Result<String> {
        let ssh_target = canonicalize_ssh_target_alias(target);
        if ssh_target.is_empty() {
            anyhow::bail!("enter an SSH target such as dev, pi@raspberry, or user@ip");
        }
        let prefix = prefix
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let cwd = cwd
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let title = title_hint
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                let fallback = ssh_target
                    .rsplit('@')
                    .next()
                    .unwrap_or(ssh_target.as_str())
                    .to_string();
                live_session_default_title(SessionKind::SshShell, cwd.as_deref(), &fallback)
            });
        let target = SshConnectTarget {
            label: title.clone(),
            kind: SessionKind::SshShell,
            ssh_target,
            prefix,
            cwd,
        };
        self.upsert_ssh_target(&target);
        let uuid = Uuid::new_v4().to_string();
        let key = format!("live::{uuid}");
        self.insert_live_session(&key, &uuid, SessionKind::SshShell, &target, Some(title));
        Ok(key)
    }

    pub fn refresh_remote_machine_for_ssh_target(
        &mut self,
        target: &SshConnectTarget,
    ) -> anyhow::Result<()> {
        if is_loopback_ssh_target(&target.ssh_target) {
            return Ok(());
        }
        if let Ok(home) = resolve_yggterm_home() {
            append_trace_event(
                &home,
                "server",
                "remote_machine",
                "refresh_begin",
                serde_json::json!({
                    "ssh_target": target.ssh_target.clone(),
                    "prefix": target.prefix.clone(),
                }),
            );
        }
        let machine_key = machine_key_from_ssh_target(&target.ssh_target);
        let label = ssh_machine_label(target);
        let entry_ix = self.ensure_remote_machine_stub(target);
        let resolved_remote_launch =
            resolve_remote_yggterm_binary(&target.ssh_target, target.prefix.as_deref()).ok();
        let remote_binary_expr = resolved_remote_launch
            .as_ref()
            .map(|(binary_expr, _)| binary_expr.clone());
        let remote_deploy_state = resolved_remote_launch
            .map(|(_, deploy_state)| deploy_state)
            .unwrap_or(RemoteDeployState::Planned);
        match scan_remote_machine_sessions(target) {
            Ok(mut sessions) => {
                sessions.sort_by(|left, right| {
                    right
                        .modified_epoch
                        .cmp(&left.modified_epoch)
                        .then_with(|| right.started_at.cmp(&left.started_at))
                });
                sessions = overlay_mirrored_remote_sessions(&machine_key, &sessions);
                if let Err(mirror_error) = mirror_remote_machine_sessions(&machine_key, &sessions) {
                    warn!(machine_key, error=%mirror_error, "failed to mirror remote machine sessions locally");
                }
                self.remote_machines[entry_ix] = RemoteMachineSnapshot {
                    machine_key,
                    label,
                    ssh_target: target.ssh_target.clone(),
                    prefix: target.prefix.clone(),
                    remote_binary_expr,
                    remote_deploy_state,
                    health: RemoteMachineHealth::Healthy,
                    sessions,
                };
                if let Ok(home) = resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "server",
                        "remote_machine",
                        "refresh_end",
                        serde_json::json!({
                            "ssh_target": target.ssh_target,
                            "machine_key": self.remote_machines[entry_ix].machine_key,
                            "health": "healthy",
                            "session_count": self.remote_machines[entry_ix].sessions.len(),
                        }),
                    );
                }
                Ok(())
            }
            Err(error) => {
                let existing_sessions = if self.remote_machines[entry_ix].sessions.is_empty() {
                    load_remote_machine_sessions_from_mirror(&machine_key).unwrap_or_default()
                } else {
                    overlay_mirrored_remote_sessions(
                        &machine_key,
                        &self.remote_machines[entry_ix].sessions,
                    )
                };
                self.remote_machines[entry_ix] = RemoteMachineSnapshot {
                    machine_key,
                    label,
                    ssh_target: target.ssh_target.clone(),
                    prefix: target.prefix.clone(),
                    remote_binary_expr,
                    remote_deploy_state,
                    health: if existing_sessions.is_empty() {
                        RemoteMachineHealth::Offline
                    } else {
                        RemoteMachineHealth::Cached
                    },
                    sessions: existing_sessions,
                };
                if let Ok(home) = resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "server",
                        "remote_machine",
                        "refresh_error",
                        serde_json::json!({
                            "ssh_target": target.ssh_target,
                            "machine_key": self.remote_machines[entry_ix].machine_key,
                            "health": format!("{:?}", self.remote_machines[entry_ix].health),
                            "error": error.to_string(),
                        }),
                    );
                }
                Err(error)
            }
        }
    }

    pub fn refresh_remote_machine_by_key(&mut self, machine_key: &str) -> anyhow::Result<()> {
        let machine_key = normalize_machine_key(machine_key);
        let target = self.remote_target_for_machine_key(&machine_key)?;
        self.refresh_remote_machine_for_ssh_target(&target)
    }

    pub fn ensure_managed_cli_for_session_path(
        &self,
        path: &str,
    ) -> anyhow::Result<Option<String>> {
        let Some(key) = self.resolve_session_storage_key(path) else {
            return Ok(None);
        };
        let Some(session) = self.sessions.get(key) else {
            return Ok(None);
        };
        if path.starts_with("remote-session://") {
            // Remote session attach must stay fast. Background machine refreshes keep the managed
            // Codex toolchain current; PTY restore should not block on a synchronous SSH upgrade.
            return Ok(None);
        }
        let Some(tool) = ManagedCliTool::from_session_kind(session.kind) else {
            return Ok(None);
        };
        let status = ensure_local_managed_cli(tool)?;
        Ok(Some(summarize_managed_cli_report(
            "local",
            &ManagedCliRefreshReport {
                scope: "local".to_string(),
                background: false,
                statuses: vec![status],
            },
        )))
    }

    pub fn refresh_managed_cli(
        &self,
        machine_key: Option<&str>,
        background: bool,
    ) -> anyhow::Result<String> {
        let (scope, report) = match machine_key.map(str::trim).filter(|value| !value.is_empty()) {
            Some(machine_key) => {
                let machine_key = normalize_machine_key(machine_key);
                let target = self.remote_target_for_machine_key(&machine_key)?;
                let report = refresh_remote_managed_cli(
                    &target.ssh_target,
                    target.prefix.as_deref(),
                    background,
                )?;
                (machine_key, report)
            }
            None => ("local".to_string(), refresh_local_managed_cli(background)?),
        };
        Ok(summarize_managed_cli_report(&scope, &report))
    }

    pub fn queue_background_managed_cli_refresh(
        &self,
        machine_key: Option<&str>,
    ) -> anyhow::Result<String> {
        match machine_key.map(str::trim).filter(|value| !value.is_empty()) {
            Some(machine_key) => {
                let machine_key = normalize_machine_key(machine_key);
                let target = self.remote_target_for_machine_key(&machine_key)?;
                let ssh_target = target.ssh_target.clone();
                let prefix = target.prefix.clone();
                let queued_scope = machine_key.clone();
                std::thread::spawn(move || {
                    if let Err(error) =
                        refresh_remote_managed_cli(&ssh_target, prefix.as_deref(), true)
                    {
                        warn!(
                            machine_key = %queued_scope,
                            ssh_target = %ssh_target,
                            error = %error,
                            "background managed cli refresh failed"
                        );
                    }
                });
                Ok(format!("queued managed cli refresh for {machine_key}"))
            }
            None => {
                std::thread::spawn(|| {
                    if let Err(error) = refresh_local_managed_cli(true) {
                        warn!(error = %error, "background managed cli refresh failed");
                    }
                });
                Ok("queued managed cli refresh for local".to_string())
            }
        }
    }

    fn remote_target_for_machine_key(&self, machine_key: &str) -> anyhow::Result<SshConnectTarget> {
        let machine_key = normalize_machine_key(machine_key);
        self.ssh_targets
            .iter()
            .find(|target| machine_key_from_ssh_target(&target.ssh_target) == machine_key)
            .cloned()
            .or_else(|| {
                self.remote_machines
                    .iter()
                    .find(|machine| machine.machine_key == machine_key)
                    .map(|machine| SshConnectTarget {
                        label: machine.label.clone(),
                        kind: SessionKind::SshShell,
                        ssh_target: machine.ssh_target.clone(),
                        prefix: machine.prefix.clone(),
                        cwd: None,
                    })
            })
            .ok_or_else(|| anyhow::anyhow!("remote machine not found: {machine_key}"))
    }

    fn cached_remote_launch_for_target(
        &self,
        ssh_target: &str,
        prefix: Option<&str>,
    ) -> Option<(String, RemoteDeployState)> {
        self.remote_machines
            .iter()
            .find(|machine| machine.ssh_target == ssh_target && machine.prefix.as_deref() == prefix)
            .and_then(|machine| {
                machine
                    .remote_binary_expr
                    .clone()
                    .map(|binary_expr| (binary_expr, machine.remote_deploy_state))
            })
    }

    pub fn open_remote_scanned_session(
        &mut self,
        machine_key: &str,
        session_id: &str,
        cwd: Option<&str>,
        title_hint: Option<&str>,
    ) -> anyhow::Result<String> {
        self.open_remote_scanned_session_with_view(machine_key, session_id, cwd, title_hint, None)
    }

    pub fn open_remote_scanned_session_with_view(
        &mut self,
        machine_key: &str,
        session_id: &str,
        cwd: Option<&str>,
        title_hint: Option<&str>,
        view_mode: Option<WorkspaceViewMode>,
    ) -> anyhow::Result<String> {
        let launch_terminal = view_mode != Some(WorkspaceViewMode::Rendered);
        let machine_key = normalize_machine_key(machine_key);
        if let Ok(home) = resolve_yggterm_home() {
            append_trace_event(
                &home,
                "server",
                "remote_session",
                "open",
                serde_json::json!({
                    "machine_key": machine_key.clone(),
                    "session_id": session_id,
                    "cwd": cwd,
                }),
            );
        }
        let machine = self
            .remote_machines
            .iter()
            .find(|machine| machine.machine_key == machine_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("remote machine not found: {machine_key}"))?;
        let session_path = remote_scanned_session_path(&machine_key, session_id);
        let target = SshConnectTarget {
            label: machine.label.clone(),
            kind: SessionKind::SshShell,
            ssh_target: machine.ssh_target.clone(),
            prefix: machine.prefix.clone(),
            cwd: cwd.map(ToOwned::to_owned),
        };
        let (remote_binary, remote_deploy_state) = if launch_terminal {
            resolve_remote_yggterm_binary(&target.ssh_target, target.prefix.as_deref())
                .unwrap_or_else(|_| {
                    (
                        preferred_remote_binary_fallback(),
                        RemoteDeployState::Planned,
                    )
                })
        } else {
            machine
                .remote_binary_expr
                .clone()
                .map(|binary_expr| (binary_expr, machine.remote_deploy_state))
                .unwrap_or_else(|| {
                    (
                        preferred_remote_binary_fallback(),
                        machine.remote_deploy_state,
                    )
                })
        };
        let resolved_title = title_hint
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| short_session_id(session_id));
        if self.sessions.contains_key(&session_path) {
            if let Some(session) = self.sessions.get_mut(&session_path) {
                session.session_path = session_path.clone();
                session.title = resolved_title.clone();
                session.host_label = machine.label.clone();
                session.launch_command = remote_ssh_launch_command(
                    &target.ssh_target,
                    target.prefix.as_deref(),
                    &remote_binary,
                    &[
                        "server",
                        "remote",
                        "resume-codex",
                        session_id,
                        cwd.unwrap_or(""),
                    ],
                );
                session.terminal_lines = vec![
                    format!("$ {}", session.launch_command),
                    format!("Queue remote Yggterm resume {session_id}"),
                    format!("Target host: {}", target.ssh_target),
                    format!("Workspace: {}", cwd.unwrap_or("<unknown>")),
                    "Daemon PTY: request main viewport terminal stream".to_string(),
                ];
                upsert_session_metadata(
                    &mut session.metadata,
                    "Source",
                    "remote-codex".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Host", target.ssh_target.clone());
                upsert_session_metadata(
                    &mut session.metadata,
                    "Restore",
                    format!("yggterm server remote resume-codex {session_id}"),
                );
                if let Some(cwd) = cwd {
                    upsert_session_metadata(&mut session.metadata, "Cwd", cwd.to_string());
                }
                upsert_session_metadata(
                    &mut session.metadata,
                    "Status",
                    format!(
                        "remote resume queued · {}",
                        match remote_deploy_state {
                            RemoteDeployState::Ready => "remote yggterm ready",
                            RemoteDeployState::CopyingBinary => "copying yggterm binary",
                            RemoteDeployState::Planned => "remote bootstrap planned",
                            RemoteDeployState::NotRequired => "not required",
                        }
                    ),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Deploy",
                    match remote_deploy_state {
                        RemoteDeployState::Ready => "ready".to_string(),
                        RemoteDeployState::CopyingBinary => "copying".to_string(),
                        RemoteDeployState::Planned => "planned".to_string(),
                        RemoteDeployState::NotRequired => "not required".to_string(),
                    },
                );
                if let Some(scanned) = machine
                    .sessions
                    .iter()
                    .find(|scanned| scanned.session_id == session_id)
                {
                    apply_remote_scanned_session_preview(
                        session,
                        scanned,
                        &machine.label,
                        &target.ssh_target,
                    );
                    if !launch_terminal {
                        if !try_apply_remote_preview_head_payload(session, &target, scanned) {
                            clear_session_preview_for_loading(session);
                        }
                    }
                }
            }
            if launch_terminal {
                self.focus_live_session(&session_path);
            } else {
                self.active_session_path = Some(session_path.clone());
                self.active_view_mode = WorkspaceViewMode::Rendered;
            }
            if launch_terminal {
                self.request_terminal_launch_for_path(&session_path);
            }
            return Ok(session_path);
        }
        self.insert_live_session_with_launch(
            &session_path,
            session_id,
            SessionKind::SshShell,
            &target,
            Some(resolved_title.clone()),
            launch_terminal,
        );
        if let Some(session) = self.sessions.get_mut(&session_path) {
            session.session_path = session_path.clone();
            session.title = resolved_title;
            session.host_label = machine.label.clone();
            session.launch_command = remote_ssh_launch_command(
                &target.ssh_target,
                target.prefix.as_deref(),
                &remote_binary,
                &[
                    "server",
                    "remote",
                    "resume-codex",
                    session_id,
                    cwd.unwrap_or(""),
                ],
            );
            session.terminal_lines = vec![
                format!("$ {}", session.launch_command),
                format!("Queue remote Yggterm resume {session_id}"),
                format!("Target host: {}", target.ssh_target),
                format!("Workspace: {}", cwd.unwrap_or("<unknown>")),
                "Daemon PTY: request main viewport terminal stream".to_string(),
            ];
            upsert_session_metadata(&mut session.metadata, "Source", "remote-codex".to_string());
            upsert_session_metadata(&mut session.metadata, "Host", target.ssh_target.clone());
            upsert_session_metadata(
                &mut session.metadata,
                "Restore",
                format!("yggterm server remote resume-codex {session_id}"),
            );
            if let Some(cwd) = cwd {
                upsert_session_metadata(&mut session.metadata, "Cwd", cwd.to_string());
            }
            upsert_session_metadata(
                &mut session.metadata,
                "Status",
                format!(
                    "remote resume queued · {}",
                    match remote_deploy_state {
                        RemoteDeployState::Ready => "remote yggterm ready",
                        RemoteDeployState::CopyingBinary => "copying yggterm binary",
                        RemoteDeployState::Planned => "remote bootstrap planned",
                        RemoteDeployState::NotRequired => "not required",
                    }
                ),
            );
            upsert_session_metadata(
                &mut session.metadata,
                "Deploy",
                match remote_deploy_state {
                    RemoteDeployState::Ready => "ready".to_string(),
                    RemoteDeployState::CopyingBinary => "copying".to_string(),
                    RemoteDeployState::Planned => "planned".to_string(),
                    RemoteDeployState::NotRequired => "not required".to_string(),
                },
            );
            if let Some(scanned) = machine
                .sessions
                .iter()
                .find(|scanned| scanned.session_id == session_id)
            {
                apply_remote_scanned_session_preview(
                    session,
                    scanned,
                    &machine.label,
                    &target.ssh_target,
                );
                if !launch_terminal {
                    if !try_apply_remote_preview_head_payload(session, &target, scanned) {
                        clear_session_preview_for_loading(session);
                    }
                }
            }
        }
        if launch_terminal {
            self.focus_live_session(&session_path);
            self.request_terminal_launch_for_path(&session_path);
        } else {
            self.active_session_path = Some(session_path.clone());
            self.active_view_mode = WorkspaceViewMode::Rendered;
        }
        Ok(session_path)
    }

    pub fn stage_remote_scanned_session_with_view(
        &mut self,
        machine_key: &str,
        session_id: &str,
        view_mode: WorkspaceViewMode,
    ) -> Option<String> {
        let machine_key = normalize_machine_key(machine_key);
        let machine = self
            .remote_machines
            .iter()
            .find(|machine| machine.machine_key == machine_key)?
            .clone();
        let scanned = machine
            .sessions
            .iter()
            .find(|session| session.session_id == session_id)?
            .clone();
        let session_path = scanned.session_path.clone();
        let staged = synthesize_remote_scanned_session_view(
            &machine,
            &scanned,
            self.backend,
            self.theme,
            self.ghostty_host.bridge_enabled,
        );
        let mut staged = staged;
        if view_mode == WorkspaceViewMode::Rendered {
            clear_session_preview_for_loading(&mut staged);
        }
        self.sessions.insert(session_path.clone(), staged);
        if !self
            .live_session_order
            .iter()
            .any(|existing| existing == &session_path)
        {
            self.live_session_order.insert(0, session_path.clone());
        }
        self.active_session_path = Some(session_path.clone());
        self.active_view_mode = view_mode;
        Some(session_path)
    }

    pub fn start_local_session(
        &mut self,
        kind: SessionKind,
        cwd: Option<&str>,
        title_hint: Option<&str>,
    ) -> String {
        let uuid = Uuid::new_v4().to_string();
        let key = match kind {
            SessionKind::Codex => format!("codex::{uuid}"),
            SessionKind::CodexLiteLlm => format!("codex-litellm::{uuid}"),
            SessionKind::Shell => format!("local::{uuid}"),
            SessionKind::SshShell => format!("live::{uuid}"),
            SessionKind::Document => format!("document::{uuid}"),
        };
        let target = local_session_target(kind, cwd);
        let title = title_hint.map(ToOwned::to_owned).unwrap_or_else(|| {
            live_session_default_title(kind, target.cwd.as_deref(), &target.label)
        });
        self.insert_live_session(&key, &uuid, kind, &target, Some(title));
        key
    }

    pub fn start_command_session(
        &mut self,
        cwd: Option<&str>,
        title_hint: Option<&str>,
        launch_command: &str,
        source_label: Option<&str>,
    ) -> String {
        let uuid = Uuid::new_v4().to_string();
        let key = format!("local::{uuid}");
        let target = local_session_target(SessionKind::Shell, cwd);
        let title = title_hint
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "recipe shell".to_string());
        self.insert_live_session(
            &key,
            &uuid,
            SessionKind::Shell,
            &target,
            Some(title.clone()),
        );
        if let Some(session) = self.sessions.get_mut(&key) {
            session.title = title.clone();
            session.launch_command = launch_command.to_string();
            session.status_line = describe_status_line(
                session.backend,
                self.theme,
                session.source,
                TerminalLaunchPhase::Queued,
                session.remote_deploy_state,
                session.bridge_available,
            );
            upsert_session_metadata(
                &mut session.metadata,
                "Source",
                source_label.unwrap_or("recipe-session").to_string(),
            );
            upsert_session_metadata(&mut session.metadata, "Launch", launch_command.to_string());
            upsert_session_metadata(&mut session.metadata, "Status", "planned".to_string());
            session.terminal_lines = vec![
                format!("$ {launch_command}"),
                format!("Queue live shell session {}", session.id),
                format!(
                    "Workspace: {}",
                    target.cwd.clone().unwrap_or_else(local_default_cwd)
                ),
                "Daemon PTY: request main viewport terminal stream".to_string(),
            ];
        }
        self.request_terminal_launch_for_path(&key);
        key
    }

    pub fn restore_live_session(&mut self, live: PersistedLiveSession) {
        let remote_scanned_key =
            parse_remote_scanned_session_path(&live.key).map(|(raw_machine_key, session_id)| {
                let machine_key = normalize_machine_key(raw_machine_key);
                let normalized_live_key = remote_scanned_session_path(&machine_key, session_id);
                (machine_key, session_id.to_string(), normalized_live_key)
            });
        let target = SshConnectTarget {
            label: live
                .ssh_target
                .rsplit('@')
                .next()
                .unwrap_or(live.ssh_target.as_str())
                .to_string(),
            kind: live.kind,
            ssh_target: live.ssh_target,
            prefix: live.prefix,
            cwd: live.cwd,
        };
        self.upsert_ssh_target(&target);
        if let Some((machine_key, session_id, normalized_live_key)) = remote_scanned_key.as_ref()
            && let Some(machine) = self
                .remote_machines
                .iter()
                .find(|machine| machine.machine_key == *machine_key)
                .cloned()
            && let Some(scanned) = machine
                .sessions
                .iter()
                .find(|session| session.session_id == session_id.as_str())
                .cloned()
        {
            let mut session = synthesize_remote_scanned_session_view(
                &machine,
                &scanned,
                self.backend,
                self.theme,
                self.ghostty_host.bridge_enabled,
            );
            if !live.title.trim().is_empty() && !looks_like_generated_fallback_title(&live.title) {
                session.title = live.title.clone();
            }
            self.sessions.insert(normalized_live_key.clone(), session);
            self.live_session_order
                .retain(|existing| existing != normalized_live_key);
            self.live_session_order
                .insert(0, normalized_live_key.clone());
            return;
        }
        let live_key = remote_scanned_key
            .as_ref()
            .map(|(_, _, normalized_live_key)| normalized_live_key.as_str())
            .unwrap_or(live.key.as_str());
        self.insert_live_session_with_launch(
            live_key,
            &live.id,
            live.kind,
            &target,
            Some(live.title),
            false,
        );
        if let Some((_, _, normalized_live_key)) = remote_scanned_key
            && let Some(session) = self.sessions.get_mut(&normalized_live_key)
        {
            session.session_path = normalized_live_key;
        }
    }

    pub fn toggle_preview_block(&mut self, block_ix: usize) {
        let Some(path) = self.active_session_path.as_ref() else {
            return;
        };
        let Some(session) = self.sessions.get_mut(path) else {
            return;
        };
        let Some(block) = session.preview.blocks.get_mut(block_ix) else {
            return;
        };
        block.folded = !block.folded;
    }

    pub fn set_all_preview_blocks_folded(&mut self, folded: bool) {
        let Some(path) = self.active_session_path.as_ref() else {
            return;
        };
        let Some(session) = self.sessions.get_mut(path) else {
            return;
        };
        for block in &mut session.preview.blocks {
            block.folded = folded;
        }
    }

    pub fn request_terminal_launch_for_active(&mut self) {
        let Some(path) = self.active_session_path.as_ref() else {
            return;
        };
        let cached_live_ssh_launch = self.sessions.get(path).and_then(|session| {
            if session.kind != SessionKind::SshShell || session.source != SessionSource::LiveSsh {
                return None;
            }
            let ssh_target = session.ssh_target.clone()?;
            let ssh_prefix = session.ssh_prefix.clone();
            Some((
                ssh_target.clone(),
                ssh_prefix.clone(),
                self.cached_remote_launch_for_target(&ssh_target, ssh_prefix.as_deref()),
            ))
        });
        let Some(session) = self.sessions.get_mut(path) else {
            return;
        };
        if session.kind == SessionKind::Document {
            if let Some((launch_command, _cwd)) = recipe_terminal_spec(session) {
                self.active_view_mode = WorkspaceViewMode::Terminal;
                session.backend = TerminalBackend::Xterm;
                session.terminal_process_id = None;
                session.terminal_window_id = None;
                session.terminal_host_token = None;
                session.terminal_host_mode = GhosttyTerminalHostMode::Unsupported;
                session.embedded_surface_id = None;
                session.embedded_surface_detail = None;
                session.last_launch_error = None;
                session.last_window_error = None;
                session.launch_phase = TerminalLaunchPhase::Running;
                session.launch_command = launch_command.clone();
                session.terminal_lines = vec![
                    format!("$ {launch_command}"),
                    "This recipe is attached to a daemon-owned PTY and rendered inline through xterm.js.".to_string(),
                    "Switch back to Preview to edit the recipe document without losing the running terminal.".to_string(),
                ];
                upsert_session_metadata(&mut session.metadata, "Backend", "xterm.js".to_string());
                upsert_session_metadata(
                    &mut session.metadata,
                    "Launch PID",
                    "daemon pty".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Launch Error", "none".to_string());
                upsert_session_metadata(
                    &mut session.metadata,
                    "Ghostty Window",
                    "not used".to_string(),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Host Mode",
                    "embedded xterm.js".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Host Token", "daemon".to_string());
                upsert_session_metadata(
                    &mut session.metadata,
                    "Embedded Surface",
                    "webview".to_string(),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Embedded Host",
                    "xterm.js".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Window Error", "none".to_string());
                upsert_session_metadata(
                    &mut session.metadata,
                    "Status",
                    "recipe running".to_string(),
                );
                session.status_line = "xterm.js · recipe runtime attached".to_string();
                return;
            }

            self.active_view_mode = WorkspaceViewMode::Rendered;
            session.status_line = "document · preview only".to_string();
            upsert_session_metadata(&mut session.metadata, "Status", "preview only".to_string());
            return;
        }

        match session.source {
            SessionSource::Stored => {
                session.backend = TerminalBackend::Xterm;
                session.terminal_process_id = None;
                session.terminal_window_id = None;
                session.terminal_host_token = None;
                session.terminal_host_mode = GhosttyTerminalHostMode::Unsupported;
                session.embedded_surface_id = None;
                session.embedded_surface_detail = None;
                session.last_launch_error = None;
                session.last_window_error = None;
                session.launch_phase = TerminalLaunchPhase::Running;
                session.terminal_lines = vec![
                    "xterm.js terminal attached to the yggterm daemon PTY.".to_string(),
                    format!("$ {}", session.launch_command),
                    "Preview mode stays rendered in-process while terminal mode streams the PTY directly in the main viewport.".to_string(),
                ];
                upsert_session_metadata(
                    &mut session.metadata,
                    "Backend",
                    match session.backend {
                        TerminalBackend::Xterm => "xterm.js".to_string(),
                        TerminalBackend::Ghostty => "Ghostty".to_string(),
                        TerminalBackend::Mock => "Mock".to_string(),
                    },
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Launch PID",
                    "daemon pty".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Launch Error", "none".to_string());
                upsert_session_metadata(
                    &mut session.metadata,
                    "Ghostty Window",
                    "not used".to_string(),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Host Mode",
                    "embedded xterm.js".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Host Token", "daemon".to_string());
                upsert_session_metadata(
                    &mut session.metadata,
                    "Embedded Surface",
                    "webview".to_string(),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Embedded Host",
                    "xterm.js".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Window Error", "none".to_string());
                upsert_session_metadata(&mut session.metadata, "Status", "running".to_string());
                session.status_line = describe_status_line(
                    session.backend,
                    self.theme,
                    session.source,
                    session.launch_phase,
                    session.remote_deploy_state,
                    session.bridge_available,
                );
            }
            SessionSource::LiveSsh => {
                if session.kind == SessionKind::SshShell
                    && let Some(ssh_target) = session.ssh_target.clone()
                {
                    let (cached_ssh_target, cached_ssh_prefix, cached_launch) =
                        cached_live_ssh_launch.clone().unwrap_or((
                            ssh_target.clone(),
                            session.ssh_prefix.clone(),
                            None,
                        ));
                    let ssh_prefix = if cached_ssh_target == ssh_target {
                        cached_ssh_prefix
                    } else {
                        session.ssh_prefix.clone()
                    };
                    let (remote_binary, remote_deploy_state) = cached_launch.unwrap_or_else(|| {
                        resolve_remote_yggterm_binary(&ssh_target, ssh_prefix.as_deref())
                            .unwrap_or_else(|_| {
                                (
                                    preferred_remote_binary_fallback(),
                                    RemoteDeployState::Planned,
                                )
                            })
                    });
                    session.remote_deploy_state = remote_deploy_state;
                    session.launch_command =
                        if session.session_path.starts_with("remote-session://") {
                            let cwd = session_metadata_value(session, "Cwd").unwrap_or_default();
                            remote_ssh_launch_command(
                                &ssh_target,
                                ssh_prefix.as_deref(),
                                &remote_binary,
                                &["server", "remote", "resume-codex", &session.id, &cwd],
                            )
                        } else {
                            let cwd = session_metadata_value(session, "Cwd").unwrap_or_default();
                            remote_ssh_launch_command(
                                &ssh_target,
                                ssh_prefix.as_deref(),
                                &remote_binary,
                                &["server", "attach", &session.id, &cwd],
                            )
                        };
                    upsert_session_metadata(
                        &mut session.metadata,
                        "Deploy",
                        match remote_deploy_state {
                            RemoteDeployState::Ready => "ready".to_string(),
                            RemoteDeployState::CopyingBinary => "copying".to_string(),
                            RemoteDeployState::Planned => "planned".to_string(),
                            RemoteDeployState::NotRequired => "not required".to_string(),
                        },
                    );
                    upsert_session_metadata(
                        &mut session.metadata,
                        "Launch",
                        session.launch_command.clone(),
                    );
                }
                session.backend = TerminalBackend::Xterm;
                session.remote_deploy_state = if session.kind == SessionKind::SshShell {
                    session.remote_deploy_state
                } else {
                    RemoteDeployState::NotRequired
                };
                session.launch_phase = TerminalLaunchPhase::Running;
                session.terminal_lines = build_live_terminal_lines(session);
                upsert_session_metadata(
                    &mut session.metadata,
                    "Status",
                    if session.kind == SessionKind::SshShell {
                        "remote ready".to_string()
                    } else {
                        "running".to_string()
                    },
                );
                session.status_line = describe_status_line(
                    session.backend,
                    self.theme,
                    session.source,
                    session.launch_phase,
                    session.remote_deploy_state,
                    session.bridge_available,
                );
            }
        }
    }

    pub fn sync_external_terminal_window_for_active(&mut self) -> String {
        "external ghostty window support is inactive in the xterm.js terminal path".to_string()
    }

    pub fn raise_external_terminal_window_for_active(&mut self) -> String {
        "external ghostty window support is inactive in the xterm.js terminal path".to_string()
    }

    fn insert_live_session(
        &mut self,
        key: &str,
        session_id: &str,
        kind: SessionKind,
        target: &SshConnectTarget,
        title_override: Option<String>,
    ) {
        self.insert_live_session_with_launch(key, session_id, kind, target, title_override, true);
    }

    fn insert_live_session_with_launch(
        &mut self,
        key: &str,
        session_id: &str,
        kind: SessionKind,
        target: &SshConnectTarget,
        title_override: Option<String>,
        launch_now: bool,
    ) {
        let mut session = build_live_session(
            session_id,
            kind,
            target,
            self.backend,
            self.theme,
            self.ghostty_host.bridge_enabled,
        );
        if let Some(title_override) = title_override {
            session.title = title_override;
        }
        self.sessions.insert(key.to_string(), session);
        self.live_session_order.retain(|existing| existing != key);
        self.live_session_order.insert(0, key.to_string());
        if launch_now {
            self.active_session_path = Some(key.to_string());
            self.active_view_mode = WorkspaceViewMode::Terminal;
            self.request_terminal_launch_for_active();
        }
    }

    fn refresh_stored_session_preview(
        &mut self,
        path: &str,
        existing: &ManagedSessionView,
    ) -> anyhow::Result<()> {
        if existing.kind == SessionKind::Document {
            return Ok(());
        }
        let cwd = session_metadata_value(existing, "Cwd");
        let title_hint = (!looks_like_generated_fallback_title(&existing.title)
            && !existing.title.trim().is_empty())
        .then_some(existing.title.clone());
        let refreshed = build_session(
            existing.kind,
            path,
            Some(existing.id.as_str()),
            cwd.as_deref(),
            title_hint.as_deref(),
            None,
            self.backend,
            self.theme,
            self.ghostty_host.bridge_enabled,
            StoredPreviewHydrationMode::Eager,
        );
        if let Some(session) = self.sessions.get_mut(path) {
            session.preview = refreshed.preview;
            session.rendered_sections = refreshed.rendered_sections;
            for entry in refreshed.metadata {
                upsert_session_metadata(&mut session.metadata, entry.label, entry.value);
            }
            session.stored_preview_hydrated = true;
        }
        Ok(())
    }

    fn refresh_remote_scanned_session_preview(
        &mut self,
        machine_key: &str,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let machine_key = normalize_machine_key(machine_key);
        self.refresh_remote_machine_by_key(&machine_key)?;
        let Some(machine) = self
            .remote_machines
            .iter()
            .find(|machine| machine.machine_key == machine_key)
            .cloned()
        else {
            return Ok(());
        };
        let Some(scanned) = machine
            .sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .cloned()
        else {
            return self.refresh_remote_preview_from_active_session(&machine_key, session_id);
        };
        let path = remote_scanned_session_path(&machine_key, session_id);
        let mut refreshed_title = None::<String>;
        let mut refreshed_precis = None::<String>;
        let mut refreshed_summary = None::<String>;
        if let Some(session) = self.sessions.get_mut(&path) {
            let target = SshConnectTarget {
                label: machine.label.clone(),
                kind: SessionKind::SshShell,
                ssh_target: machine.ssh_target.clone(),
                prefix: machine.prefix.clone(),
                cwd: Some(scanned.cwd.clone()),
            };
            match fetch_remote_preview_payload(&target, &scanned.storage_path) {
                Ok(payload) => {
                    refreshed_title = payload.title_hint.clone();
                    refreshed_precis = payload.cached_precis.clone();
                    refreshed_summary = payload.cached_summary.clone();
                    apply_remote_preview_payload(session, payload);
                    upsert_session_metadata(
                        &mut session.metadata,
                        "Source",
                        "remote-codex".to_string(),
                    );
                    upsert_session_metadata(
                        &mut session.metadata,
                        "Host",
                        machine.ssh_target.clone(),
                    );
                    upsert_session_metadata(
                        &mut session.metadata,
                        "UUID",
                        scanned.session_id.clone(),
                    );
                    upsert_session_metadata(&mut session.metadata, "Cwd", scanned.cwd.clone());
                }
                Err(error) => {
                    warn!(machine_key, session_id, error=%error, "failed to fetch remote preview payload");
                    apply_remote_scanned_session_preview(
                        session,
                        &scanned,
                        &machine.label,
                        &machine.ssh_target,
                    );
                }
            }
        }
        if let Some(title) = refreshed_title.as_deref() {
            self.set_session_title_hint(&path, title);
        }
        if let Some(precis) = refreshed_precis.as_deref() {
            self.set_session_precis_hint(&path, precis);
        }
        if let Some(summary) = refreshed_summary.as_deref() {
            self.set_session_summary_hint(&path, summary);
        }
        Ok(())
    }

    fn refresh_remote_preview_from_active_session(
        &mut self,
        machine_key: &str,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let machine_key = normalize_machine_key(machine_key);
        let path = remote_scanned_session_path(&machine_key, session_id);
        let Some(session) = self.sessions.get_mut(&path) else {
            return Ok(());
        };
        let ssh_target = session
            .ssh_target
            .clone()
            .or_else(|| session_metadata_value(session, "Host"))
            .unwrap_or_else(|| machine_key.to_string());
        let cwd = session_metadata_value(session, "Cwd");
        let storage_path = session_metadata_value(session, "Storage").unwrap_or_default();
        if storage_path.trim().is_empty() {
            return Ok(());
        }
        let target = SshConnectTarget {
            label: machine_key.to_string(),
            kind: SessionKind::SshShell,
            ssh_target: ssh_target.clone(),
            prefix: session.ssh_prefix.clone(),
            cwd,
        };
        match fetch_remote_preview_payload(&target, &storage_path) {
            Ok(payload) => {
                let refreshed_title = payload.title_hint.clone();
                let refreshed_precis = payload.cached_precis.clone();
                let refreshed_summary = payload.cached_summary.clone();
                apply_remote_preview_payload(session, payload);
                upsert_session_metadata(
                    &mut session.metadata,
                    "Source",
                    "remote-codex".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Host", ssh_target);
                upsert_session_metadata(&mut session.metadata, "UUID", session_id.to_string());
                if let Some(title) = refreshed_title.as_deref() {
                    self.set_session_title_hint(&path, title);
                }
                if let Some(precis) = refreshed_precis.as_deref() {
                    self.set_session_precis_hint(&path, precis);
                }
                if let Some(summary) = refreshed_summary.as_deref() {
                    self.set_session_summary_hint(&path, summary);
                }
            }
            Err(error) => {
                warn!(machine_key, session_id, error=%error, "failed to fetch remote preview payload from active session");
            }
        }
        Ok(())
    }

    fn connect_ssh_like_target(&mut self, target: &SshConnectTarget) -> (Option<String>, bool) {
        let mut target = target.clone();
        target.ssh_target = canonicalize_ssh_target_alias(&target.ssh_target);
        self.upsert_ssh_target(&target);
        if let Some(existing_key) = self
            .sessions
            .iter()
            .find(|(_, session)| {
                session.source == SessionSource::LiveSsh
                    && session.kind == target.kind
                    && session.ssh_target.as_deref() == Some(target.ssh_target.as_str())
                    && session.ssh_prefix.as_deref() == target.prefix.as_deref()
            })
            .map(|(key, _)| key.clone())
        {
            self.focus_live_session(&existing_key);
            return (Some(existing_key), true);
        }

        let uuid = Uuid::new_v4().to_string();
        let key = match target.kind {
            SessionKind::Shell => format!("local::{uuid}"),
            _ => format!("live::{uuid}"),
        };
        self.insert_live_session(
            &key,
            &uuid,
            target.kind,
            &target,
            Some(target.label.clone()),
        );
        (Some(key), false)
    }

    fn upsert_ssh_target(&mut self, target: &SshConnectTarget) {
        if is_loopback_ssh_target(&target.ssh_target) {
            return;
        }
        if let Some(existing) = self.ssh_targets.iter_mut().find(|existing| {
            existing.kind == target.kind
                && existing.ssh_target == target.ssh_target
                && existing.prefix == target.prefix
        }) {
            existing.label = target.label.clone();
            existing.cwd = target.cwd.clone();
            self.ensure_remote_machine_stub(target);
            return;
        }
        self.ssh_targets.push(target.clone());
        self.ssh_targets.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.ssh_target.cmp(&right.ssh_target))
        });
        self.ensure_remote_machine_stub(target);
    }

    fn ensure_remote_machine_stub(&mut self, target: &SshConnectTarget) -> usize {
        let machine_key = machine_key_from_ssh_target(&target.ssh_target);
        if let Some(existing_ix) = self
            .remote_machines
            .iter()
            .position(|machine| machine.machine_key == machine_key)
        {
            let existing = &mut self.remote_machines[existing_ix];
            existing.label = ssh_machine_label(target);
            existing.ssh_target = target.ssh_target.clone();
            existing.prefix = target.prefix.clone();
            if existing.health == RemoteMachineHealth::Offline && !existing.sessions.is_empty() {
                existing.health = RemoteMachineHealth::Cached;
            }
            return existing_ix;
        }
        self.remote_machines.push(RemoteMachineSnapshot {
            machine_key: machine_key.clone(),
            label: ssh_machine_label(target),
            ssh_target: target.ssh_target.clone(),
            prefix: target.prefix.clone(),
            remote_binary_expr: None,
            remote_deploy_state: RemoteDeployState::Planned,
            health: RemoteMachineHealth::Cached,
            sessions: Vec::new(),
        });
        self.remote_machines.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.machine_key.cmp(&right.machine_key))
        });
        self.remote_machines
            .iter()
            .position(|machine| machine.machine_key == machine_key)
            .unwrap_or(0)
    }
}

fn is_legacy_demo_live_session(live: &PersistedLiveSession) -> bool {
    matches!(
        (live.kind, live.ssh_target.as_str(), live.prefix.as_deref()),
        (
            SessionKind::SshShell,
            "prod-app-01",
            Some("sudo machinectl shell prod")
        ) | (SessionKind::SshShell, "design-01", None)
            | (
                SessionKind::SshShell,
                "ghostty-admin",
                Some("tmux new-session -A -s yggterm")
            )
    )
}

fn snapshot_metadata_entries(entries: &[SessionMetadataEntry]) -> Vec<SnapshotMetadataEntry> {
    entries
        .iter()
        .map(|entry| SnapshotMetadataEntry {
            label: entry.label.to_string(),
            value: entry.value.clone(),
        })
        .collect()
}

fn ssh_machine_label(target: &SshConnectTarget) -> String {
    if !target.label.trim().is_empty() {
        target.label.trim().to_string()
    } else {
        target
            .ssh_target
            .rsplit('@')
            .next()
            .unwrap_or(target.ssh_target.as_str())
            .trim()
            .to_string()
    }
}

fn ssh_host_from_target(ssh_target: &str) -> &str {
    ssh_target.rsplit('@').next().unwrap_or(ssh_target).trim()
}

fn canonicalize_ssh_target_alias(ssh_target: &str) -> String {
    let ssh_target = ssh_target.trim();
    let (username, host) = match ssh_target.rsplit_once('@') {
        Some((username, host)) => (Some(username), host),
        None => (None, ssh_target),
    };
    let canonical_host = canonicalize_remote_machine_alias(host);
    match username {
        Some(username) => format!("{username}@{canonical_host}"),
        None => canonical_host,
    }
}

fn is_loopback_ssh_target(ssh_target: &str) -> bool {
    matches!(
        ssh_host_from_target(ssh_target),
        "localhost" | "127.0.0.1" | "::1"
    )
}

fn machine_key_from_ssh_target(ssh_target: &str) -> String {
    normalize_machine_key(
        canonicalize_remote_machine_alias(ssh_host_from_target(ssh_target)).as_str(),
    )
}

fn normalize_machine_key(raw_machine_key: &str) -> String {
    raw_machine_key
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
        .to_string()
}

fn canonicalize_remote_machine_alias(machine_key: &str) -> String {
    match machine_key.trim().to_ascii_lowercase().as_str() {
        "juju" | "jujo" => "jojo".to_string(),
        value => value.to_string(),
    }
}

fn remote_scanned_session_path(machine_key: &str, session_id: &str) -> String {
    format!("remote-session://{machine_key}/{session_id}")
}

fn parse_remote_scanned_session_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("remote-session://")?;
    let (machine_key, session_id) = rest.split_once('/')?;
    Some((machine_key, session_id))
}

fn remote_resume_shell_command(
    session_id: &str,
    cwd: Option<&str>,
    prefix: Option<&str>,
) -> String {
    let base = agent_launch_command(SessionKind::Codex, cwd, Some(session_id));
    match prefix.map(str::trim).filter(|value| !value.is_empty()) {
        Some(prefix) => format!("{prefix} && {base}"),
        None => base,
    }
}

fn remote_resume_picker_notice(session_id: &str) -> String {
    format!(
        "printf '%s\\n' {} >&2",
        shell_single_quote(&format!(
            "yggterm: saved Codex session {session_id} was not found; opening the resume picker."
        ))
    )
}

fn remote_resume_picker_shell_command(
    session_id: &str,
    cwd: Option<&str>,
    prefix: Option<&str>,
    persistent: bool,
) -> String {
    let base = managed_cli_shell_command(
        SessionKind::Codex,
        cwd,
        ManagedCliAction::ResumePicker { persistent },
    )
    .unwrap_or_else(|_| {
        let codex_command = if persistent {
            "exec codex resume"
        } else {
            "codex resume"
        };
        match cwd.filter(|cwd| !cwd.trim().is_empty()) {
            Some(cwd) => format!("cd {} && {}", shell_single_quote(cwd), codex_command),
            None => codex_command.to_string(),
        }
    });
    let with_notice = format!("{}; {}", remote_resume_picker_notice(session_id), base);
    match prefix.map(str::trim).filter(|value| !value.is_empty()) {
        Some(prefix) => format!("{prefix} && {with_notice}"),
        None => with_notice,
    }
}

fn remote_persistent_resume_shell_command(session_id: &str, cwd: Option<&str>) -> String {
    persistent_agent_resume_command(SessionKind::Codex, cwd, session_id)
}

fn resolve_remote_codex_home() -> std::path::PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".codex"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(".codex"))
}

fn remote_saved_codex_session_exists(session_id: &str) -> anyhow::Result<bool> {
    let mut files = Vec::new();
    collect_codex_session_files(&resolve_remote_codex_home().join("sessions"), &mut files)?;
    for path in files {
        if let Some((candidate_id, _cwd)) = read_codex_session_identity_fields(&path)?
            && candidate_id == session_id
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn remote_tmux_session_name(session_id: &str) -> String {
    let suffix = session_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(24)
        .collect::<String>();
    format!("yggterm-{suffix}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteMultiplexer {
    Tmux,
    Screen,
}

fn tmux_available() -> anyhow::Result<bool> {
    Ok(Command::new("sh")
        .arg("-lc")
        .arg("command -v tmux >/dev/null 2>&1")
        .status()
        .context("checking tmux availability")?
        .success())
}

fn screen_available() -> anyhow::Result<bool> {
    Ok(Command::new("sh")
        .arg("-lc")
        .arg("command -v screen >/dev/null 2>&1")
        .status()
        .context("checking screen availability")?
        .success())
}

fn preferred_remote_multiplexer() -> anyhow::Result<Option<RemoteMultiplexer>> {
    if tmux_available()? {
        return Ok(Some(RemoteMultiplexer::Tmux));
    }
    if screen_available()? {
        return Ok(Some(RemoteMultiplexer::Screen));
    }
    Ok(None)
}

fn tmux_has_session(session_name: &str) -> anyhow::Result<bool> {
    Ok(Command::new("tmux")
        .args(["has-session", "-t", session_name])
        .status()
        .with_context(|| format!("checking tmux session {session_name}"))?
        .success())
}

fn tmux_spawn_codex_session(session_name: &str, command: &str) -> anyhow::Result<()> {
    let status = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            session_name,
            "sh",
            "-lc",
            command,
        ])
        .status()
        .with_context(|| format!("creating tmux session {session_name}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("tmux failed to create session {session_name}: {status}");
    }
}

fn tmux_attach_session(session_name: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new("tmux")
            .args(["attach-session", "-t", session_name])
            .exec();
        Err(anyhow::anyhow!(
            "failed to exec tmux attach-session for {session_name}: {error}"
        ))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new("tmux")
            .args(["attach-session", "-t", session_name])
            .status()
            .with_context(|| format!("attaching tmux session {session_name}"))?;
        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("tmux attach-session failed for {session_name}: {status}");
        }
    }
}

fn tmux_snapshot_bytes(session_name: &str) -> anyhow::Result<Vec<u8>> {
    let output = Command::new("tmux")
        .args([
            "capture-pane",
            "-p",
            "-e",
            "-J",
            "-S",
            "-",
            "-t",
            session_name,
        ])
        .output()
        .with_context(|| format!("capturing tmux pane for {session_name}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "tmux capture-pane failed for {session_name}: {}",
            output.status
        );
    }
    Ok(sanitize_terminal_snapshot(&output.stdout))
}

fn tmux_send_keys(session_name: &str, text: &str) -> anyhow::Result<()> {
    let status = Command::new("tmux")
        .args(["send-keys", "-t", session_name, text, "Enter"])
        .status()
        .with_context(|| format!("sending tmux keys to {session_name}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("tmux send-keys failed for {session_name}: {status}");
    }
}

fn tmux_kill_session(session_name: &str) -> anyhow::Result<()> {
    let status = Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .status()
        .with_context(|| format!("killing tmux session {session_name}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("tmux kill-session failed for {session_name}: {status}");
    }
}

fn parse_screen_session_ref(screen_ls_output: &str, session_name: &str) -> Option<String> {
    for line in screen_ls_output.lines() {
        let token = line.split_whitespace().next()?.trim();
        if token == session_name || token.ends_with(&format!(".{session_name}")) {
            return Some(token.to_string());
        }
    }
    None
}

fn screen_session_ref(session_name: &str) -> anyhow::Result<Option<String>> {
    let output = Command::new("screen")
        .args(["-ls", session_name])
        .output()
        .with_context(|| format!("listing screen sessions for {session_name}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(parse_screen_session_ref(
        &format!("{stdout}\n{stderr}"),
        session_name,
    ))
}

fn screen_has_session(session_name: &str) -> anyhow::Result<bool> {
    Ok(screen_session_ref(session_name)?.is_some())
}

fn screen_spawn_codex_session(session_name: &str, command: &str) -> anyhow::Result<()> {
    let status = Command::new("screen")
        .args(["-DmS", session_name, "sh", "-lc", command])
        .status()
        .with_context(|| format!("creating screen session {session_name}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("screen failed to create session {session_name}: {status}");
    }
}

fn screen_attach_session(session_name: &str) -> anyhow::Result<()> {
    let target = screen_session_ref(session_name)?.unwrap_or_else(|| session_name.to_string());
    let attach_command = format!(
        "(sleep 0.2; \
            screen -S {} -X redisplay >/dev/null 2>&1 || true; \
            screen -S {} -X stuff \"$(printf '\\f')\" >/dev/null 2>&1 || true) \
         & exec screen -D -RR {}",
        shell_single_quote(&target),
        shell_single_quote(&target),
        shell_single_quote(&target)
    );

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new("sh").args(["-lc", &attach_command]).exec();
        Err(anyhow::anyhow!(
            "failed to exec screen attach for {target}: {error}"
        ))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new("sh")
            .args(["-lc", &attach_command])
            .status()
            .with_context(|| format!("attaching screen session {target}"))?;
        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("screen attach failed for {target}: {status}");
        }
    }
}

fn screen_send_keys(session_name: &str, text: &str) -> anyhow::Result<()> {
    let target = screen_session_ref(session_name)?.unwrap_or_else(|| session_name.to_string());
    let payload = format!("{text}\r");
    let status = Command::new("screen")
        .args(["-S", &target, "-X", "stuff", &payload])
        .status()
        .with_context(|| format!("sending screen keys to {target}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("screen stuff failed for {target}: {status}");
    }
}

fn screen_kill_session(session_name: &str) -> anyhow::Result<()> {
    let Some(target) = screen_session_ref(session_name)? else {
        return Ok(());
    };
    let status = Command::new("screen")
        .args(["-S", &target, "-X", "quit"])
        .status()
        .with_context(|| format!("killing screen session {target}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("screen quit failed for {target}: {status}");
    }
}

fn sanitize_terminal_snapshot(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .copied()
        .filter(|byte| matches!(byte, b'\n' | b'\r' | b'\t') || (0x20..=0x7e).contains(byte))
        .collect()
}

fn screen_snapshot_bytes(session_name: &str) -> anyhow::Result<Vec<u8>> {
    let Some(target) = screen_session_ref(session_name)? else {
        return Ok(Vec::new());
    };
    let path = std::env::temp_dir().join(format!(
        "yggterm-screen-{}-{}.txt",
        current_millis_u64(),
        Uuid::new_v4().simple()
    ));
    let status = Command::new("screen")
        .args(["-S", &target, "-X", "hardcopy", "-h"])
        .arg(&path)
        .status()
        .with_context(|| format!("capturing screen hardcopy for {target}"))?;
    if !status.success() {
        anyhow::bail!("screen hardcopy failed for {target}: {status}");
    }
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let _ = fs::remove_file(&path);
    Ok(sanitize_terminal_snapshot(&bytes))
}

fn emit_terminal_snapshot(bytes: &[u8]) -> anyhow::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut stdout = std::io::stdout();
    stdout
        .write_all(b"\x1b[2J\x1b[H")
        .context("clearing terminal before snapshot emit")?;
    stdout
        .write_all(bytes)
        .context("writing terminal snapshot")?;
    stdout.flush().context("flushing terminal snapshot")?;
    Ok(())
}

fn remote_ssh_launch_command(
    ssh_target: &str,
    prefix: Option<&str>,
    binary_expr: &str,
    args: &[&str],
) -> String {
    let mut inner = String::from(binary_expr);
    for arg in args {
        inner.push(' ');
        inner.push_str(&shell_single_quote(arg));
    }
    let env_exports = terminal_identity_shell_exports().join(" && ");
    let inner = if env_exports.is_empty() {
        inner
    } else {
        format!("{env_exports} && {inner}")
    };
    let remote = match prefix.map(str::trim).filter(|value| !value.is_empty()) {
        Some(prefix) => format!("{prefix} && {inner}"),
        None => inner,
    };
    if cfg!(windows) {
        return format!("ssh -tt {} {}", ssh_target, shell_single_quote(&remote));
    }
    let control_dir = "$HOME/.yggterm/ssh-control";
    let ssh = format!(
        "ssh -o ControlMaster=auto -o ControlPersist=60 -o ControlPath={} -tt {} {}",
        shell_single_quote(&format!("{control_dir}/%C")),
        ssh_target,
        shell_single_quote(&remote)
    );
    format!("mkdir -p {control_dir} >/dev/null 2>&1 && {ssh}")
}

#[derive(Debug, Serialize, Deserialize)]
struct RemoteSummaryLine {
    rollout_path: String,
    #[serde(default = "default_unknown")]
    id: String,
    #[serde(default = "default_unknown_cwd")]
    cwd: String,
    #[serde(default = "default_unknown")]
    started_at: String,
    #[serde(default)]
    modified_epoch: i64,
    #[serde(default)]
    event_count: usize,
    #[serde(default)]
    user_message_count: usize,
    #[serde(default)]
    assistant_message_count: usize,
    #[serde(default)]
    recent_context: String,
    #[serde(default)]
    title_hint: Option<String>,
    #[serde(default)]
    cached_precis: Option<String>,
    #[serde(default)]
    cached_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePreviewPayload {
    #[serde(default)]
    title_hint: Option<String>,
    #[serde(default)]
    cached_precis: Option<String>,
    #[serde(default)]
    cached_summary: Option<String>,
    preview: SnapshotPreview,
    #[serde(default)]
    rendered_sections: Vec<SnapshotRenderedSection>,
}

fn default_unknown() -> String {
    "unknown".to_string()
}

fn default_unknown_cwd() -> String {
    "<unknown>".to_string()
}

fn current_millis_u64() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn summarize_recent_context(messages: &[yggterm_core::TranscriptMessage]) -> String {
    generation_context_from_messages(messages)
}

fn recent_context_scaffold_line(trimmed: &str) -> bool {
    let lower = trimmed.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    [
        "<instructions>",
        "<cwd>",
        "<shell>",
        "<current_date>",
        "</current_date>",
        "<timezone>",
        "</timezone>",
        "<approval_policy>",
        "<sandbox_mode>",
        "<network_access>",
        "<environment_context",
        "</environment_context>",
        "<collaboration_mode>",
        "</collaboration_mode>",
        "<permissions instructions>",
        "</permissions instructions>",
    ]
    .iter()
    .any(|marker| lower.starts_with(marker) || lower.contains(marker))
        || lower.contains("you are now in default mode")
        || lower.contains("agents.md instructions for")
        || lower.contains("any previous instructions for other modes")
        || lower.contains("default mode you should strongly prefer")
        || lower.contains("if a decision is necessary and cannot be discovered from local context")
        || lower.contains("request_user_input")
        || lower.contains("filesystem sandboxing")
        || lower.contains("approvals are your mechanism to get user consent")
        || lower.contains("approval_policy is")
        || lower.contains("danger-full-access")
        || lower.contains("non-interactive mode where you may never ask the user for approval")
        || lower.contains("<turn_aborted>")
        || lower.contains("</turn_aborted>")
        || lower.contains("the user interrupted the previous turn on purpose")
        || lower.contains("any running unified exec processes were terminated")
}

fn preview_scaffold_line(trimmed: &str) -> bool {
    let lower = trimmed.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    if recent_context_scaffold_line(trimmed) {
        return true;
    }
    if matches!(
        lower.as_str(),
        "## skills" | "### available skills" | "### how to use skills" | "</instructions>"
    ) {
        return true;
    }
    if [
        "a skill is a set of local instructions to follow that is stored in a",
        "how to use a skill (progressive disclosure):",
        "coordination and sequencing:",
        "context hygiene:",
        "safety and fallback:",
        "trigger rules:",
        "discovery:",
        "missing/blocked:",
        "after deciding to use a skill, open its `skill.md`",
        "when `skill.md` references relative paths",
        "if `skill.md` points to extra folders such as `references/`",
        "if `scripts/` exist, prefer running or patching them",
        "if `assets/` or templates exist, reuse them",
        "if multiple skills apply, choose the minimal set",
        "announce which skill(s) you're using and why",
        "keep context small: summarize long sections instead of pasting them",
        "avoid deep reference-chasing",
        "when variants exist (frameworks, providers, domains)",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return true;
    }
    if lower.contains("/.codex/skills/") && lower.contains("file:") {
        return true;
    }
    lower.starts_with("- skill-") || lower.starts_with("skill-")
}

fn sanitize_recent_context_payload(recent_context: &str) -> String {
    let (goals, blocks, _sections) = parse_recent_context_sections(recent_context);
    let goals = dedupe_recent_context_lines(
        goals.into_iter()
            .filter_map(|goal| sanitize_recent_context_turn_content(&goal))
            .collect::<Vec<_>>(),
    );
    if goals.is_empty() && blocks.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    if !goals.is_empty() {
        lines.push("PRIMARY USER GOALS:".to_string());
        for goal in goals {
            lines.push(format!("- {goal}"));
        }
        if !blocks.is_empty() {
            lines.push(String::new());
        }
    }
    if !blocks.is_empty() {
        lines.push("RECENT SUBSTANTIVE TURNS:".to_string());
        for block in blocks {
            let label = match block.tone {
                PreviewTone::User => "USER",
                PreviewTone::Assistant => "ASSISTANT",
            };
            let text = block
                .lines
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !text.is_empty() {
                lines.push(format!("{label}: {text}"));
            }
        }
    }
    lines.join("\n")
}

fn sanitize_recent_context_turn_content(content: &str) -> Option<String> {
    let lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !preview_scaffold_line(line))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    Some(collapse_recent_context_inline_markup(&lines.join(" ")))
}

fn collapse_recent_context_inline_markup(content: &str) -> String {
    let mut remaining = content.trim();
    let mut out = String::new();

    loop {
        let Some(start) = remaining.find("<image name=[") else {
            out.push_str(remaining);
            break;
        };
        out.push_str(&remaining[..start]);
        let after = &remaining[start + "<image name=[".len()..];
        let Some(label_end) = after.find("]>") else {
            out.push_str(&remaining[start..]);
            break;
        };
        let label_text = after[..label_end].trim();
        let label = format!("[{label_text}]");
        out.push_str(&label);

        let mut tail = after[label_end + 2..].trim_start();
        if let Some(stripped) = tail.strip_prefix("</image>") {
            tail = stripped.trim_start();
        }
        if let Some(stripped) = tail.strip_prefix(&label) {
            tail = stripped.trim_start();
        }
        remaining = tail;
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_recent_context_semantic_text(content: &str) -> String {
    collapse_recent_context_inline_markup(content)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase()
}

fn dedupe_recent_context_lines(lines: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::<String>::new();
    let mut deduped = Vec::with_capacity(lines.len());
    for line in lines {
        let normalized = normalize_recent_context_semantic_text(&line);
        if normalized.is_empty() || !seen.insert(normalized) {
            continue;
        }
        deduped.push(line);
    }
    deduped
}

fn sanitize_preview_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .filter(|line| !preview_scaffold_line(line))
        .collect()
}

fn sanitize_session_preview_blocks(blocks: Vec<SessionPreviewBlock>) -> Vec<SessionPreviewBlock> {
    blocks
        .into_iter()
        .filter_map(|block| {
            let lines = sanitize_preview_lines(block.lines);
            if lines.is_empty() {
                return None;
            }
            Some(SessionPreviewBlock { lines, ..block })
        })
        .collect()
}

fn sanitize_snapshot_preview_blocks(
    blocks: Vec<SnapshotPreviewBlock>,
) -> Vec<SnapshotPreviewBlock> {
    blocks
        .into_iter()
        .filter_map(|block| {
            let lines = sanitize_preview_lines(block.lines);
            if lines.is_empty() {
                return None;
            }
            Some(SnapshotPreviewBlock { lines, ..block })
        })
        .collect()
}

fn is_placeholder_rendered_section_title(title: &str) -> bool {
    title.eq_ignore_ascii_case("Rendered Session")
        || title.eq_ignore_ascii_case("Server Notes")
        || title.eq_ignore_ascii_case("Recent Context")
}

fn sanitize_snapshot_rendered_sections(
    sections: Vec<SnapshotRenderedSection>,
) -> Vec<SnapshotRenderedSection> {
    sections
        .into_iter()
        .filter(|section| !is_placeholder_rendered_section_title(&section.title))
        .filter_map(|section| {
            let lines = sanitize_preview_lines(section.lines);
            if lines.is_empty() {
                return None;
            }
            Some(SnapshotRenderedSection { lines, ..section })
        })
        .collect()
}

fn parse_recent_context_sections(
    recent_context: &str,
) -> (
    Vec<String>,
    Vec<SessionPreviewBlock>,
    Vec<SessionRenderedSection>,
) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum RecentContextSection {
        None,
        Goals,
        Turns,
        Notes,
    }

    fn push_recent_context_block(
        blocks: &mut Vec<SessionPreviewBlock>,
        role: &'static str,
        tone: PreviewTone,
        content: &str,
    ) {
        let Some(compact) = sanitize_recent_context_turn_content(content).map(|content| {
            content
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string()
        }) else {
            return;
        };
        if compact.is_empty() {
            return;
        }
        blocks.push(SessionPreviewBlock {
            role,
            timestamp: "remote:scan".to_string(),
            tone,
            folded: false,
            lines: vec![compact],
        });
    }

    let mut section = RecentContextSection::None;
    let mut goals = Vec::new();
    let mut blocks = Vec::new();
    let mut notes = Vec::new();

    for line in recent_context.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            "PRIMARY USER GOALS:" => {
                section = RecentContextSection::Goals;
                continue;
            }
            "RECENT SUBSTANTIVE TURNS:" | "RECENT CONTEXT:" => {
                section = RecentContextSection::Turns;
                continue;
            }
            "SERVER NOTES:" => {
                section = RecentContextSection::Notes;
                continue;
            }
            _ => {}
        }

        if let Some(rest) = trimmed.strip_prefix("USER:") {
            push_recent_context_block(&mut blocks, "USER", PreviewTone::User, rest.trim());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("ASSISTANT:") {
            push_recent_context_block(
                &mut blocks,
                "ASSISTANT",
                PreviewTone::Assistant,
                rest.trim(),
            );
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("SYSTEM:") {
            push_recent_context_block(&mut blocks, "SYSTEM", PreviewTone::Assistant, rest.trim());
            continue;
        }

        match section {
            RecentContextSection::Goals => {
                if let Some(goal) =
                    sanitize_recent_context_turn_content(trimmed.strip_prefix("- ").unwrap_or(trimmed))
                {
                    goals.push(goal);
                }
            }
            RecentContextSection::Notes => {
                notes.push(trimmed.to_string());
            }
            RecentContextSection::Turns | RecentContextSection::None => {}
        }
    }

    if blocks.is_empty() {
        for line in recent_context.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("USER:") {
                push_recent_context_block(&mut blocks, "USER", PreviewTone::User, rest.trim());
            } else if let Some(rest) = trimmed.strip_prefix("ASSISTANT:") {
                push_recent_context_block(
                    &mut blocks,
                    "ASSISTANT",
                    PreviewTone::Assistant,
                    rest.trim(),
                );
            } else if let Some(rest) = trimmed.strip_prefix("SYSTEM:") {
                push_recent_context_block(
                    &mut blocks,
                    "SYSTEM",
                    PreviewTone::Assistant,
                    rest.trim(),
                );
            }
        }
    }

    let goals = dedupe_recent_context_lines(goals);
    let mut rendered_sections = Vec::new();
    if !goals.is_empty() {
        rendered_sections.push(SessionRenderedSection {
            title: "Primary User Goals",
            lines: goals.clone(),
        });
    }
    if !notes.is_empty() {
        rendered_sections.push(SessionRenderedSection {
            title: "Server Notes",
            lines: notes,
        });
    }

    (goals, blocks, rendered_sections)
}

fn modified_epoch_display(epoch: i64) -> String {
    OffsetDateTime::from_unix_timestamp(epoch)
        .map(format_display_datetime)
        .unwrap_or_else(|_| "unknown".to_string())
}

fn session_metadata_value(session: &ManagedSessionView, label: &str) -> Option<String> {
    session
        .metadata
        .iter()
        .find(|entry| entry.label == label)
        .map(|entry| entry.value.clone())
}

fn apply_remote_scanned_session_preview(
    session: &mut ManagedSessionView,
    scanned: &RemoteScannedSession,
    machine_label: &str,
    ssh_target: &str,
) {
    if !scanned.title_hint.trim().is_empty()
        && !looks_like_generated_fallback_title(&scanned.title_hint)
    {
        session.title = scanned.title_hint.clone();
    }
    let (primary_goals, preview_blocks, rendered_sections) =
        parse_recent_context_sections(&scanned.recent_context);
    session.preview.blocks = preview_blocks;
    session.rendered_sections = rendered_sections;
    let messages = format!(
        "{} user · {} assistant",
        scanned.user_message_count, scanned.assistant_message_count
    );
    upsert_session_metadata(
        &mut session.preview.summary,
        "Session",
        scanned.session_id.clone(),
    );
    upsert_session_metadata(
        &mut session.preview.summary,
        "Host",
        machine_label.to_string(),
    );
    upsert_session_metadata(&mut session.preview.summary, "Cwd", scanned.cwd.clone());
    upsert_session_metadata(
        &mut session.preview.summary,
        "Started",
        scanned.started_at.clone(),
    );
    upsert_session_metadata(&mut session.preview.summary, "Messages", messages.clone());
    upsert_session_metadata(
        &mut session.preview.summary,
        "Updated",
        modified_epoch_display(scanned.modified_epoch),
    );
    if let Some(goal) = primary_goals.first() {
        upsert_session_metadata(&mut session.preview.summary, "Goal", goal.clone());
    }
    if let Some(precis) = &scanned.cached_precis {
        upsert_session_metadata(&mut session.preview.summary, "Precis", precis.clone());
    }
    if let Some(summary) = &scanned.cached_summary {
        upsert_session_metadata(&mut session.preview.summary, "Summary", summary.clone());
    }

    upsert_session_metadata(&mut session.metadata, "Source", "remote-codex".to_string());
    upsert_session_metadata(&mut session.metadata, "Host", ssh_target.to_string());
    upsert_session_metadata(&mut session.metadata, "UUID", scanned.session_id.clone());
    upsert_session_metadata(&mut session.metadata, "Cwd", scanned.cwd.clone());
    upsert_session_metadata(
        &mut session.metadata,
        "Storage",
        scanned.storage_path.clone(),
    );
    upsert_session_metadata(&mut session.metadata, "Started", scanned.started_at.clone());
    upsert_session_metadata(
        &mut session.metadata,
        "Updated",
        modified_epoch_display(scanned.modified_epoch),
    );
    upsert_session_metadata(&mut session.metadata, "Messages", messages);
}

fn clear_session_preview_for_loading(session: &mut ManagedSessionView) {
    upsert_session_metadata(
        &mut session.metadata,
        "Preview Hydration",
        "loading".to_string(),
    );
}

fn apply_remote_preview_head_payload(
    session: &mut ManagedSessionView,
    payload: RemotePreviewPayload,
) {
    apply_remote_preview_payload(session, payload);
    upsert_session_metadata(&mut session.metadata, "Preview Hydration", "head".to_string());
}

fn try_apply_remote_preview_head_payload(
    session: &mut ManagedSessionView,
    target: &SshConnectTarget,
    scanned: &RemoteScannedSession,
) -> bool {
    match fetch_remote_preview_head_payload(target, &scanned.storage_path, 8) {
        Ok(payload) => {
            apply_remote_preview_head_payload(session, payload);
            true
        }
        Err(error) => {
            warn!(
                ssh_target = %target.ssh_target,
                storage_path = %scanned.storage_path,
                error = %error,
                "failed to fetch remote preview head payload"
            );
            false
        }
    }
}

fn apply_remote_preview_payload(session: &mut ManagedSessionView, payload: RemotePreviewPayload) {
    if let Some(title_hint) = payload
        .title_hint
        .filter(|value| !value.trim().is_empty() && !looks_like_generated_fallback_title(value))
    {
        session.title = title_hint;
    }
    session.preview = SessionPreview {
        summary: payload
            .preview
            .summary
            .into_iter()
            .map(|entry| SessionMetadataEntry {
                label: leak_label(entry.label),
                value: entry.value,
            })
            .collect(),
        blocks: sanitize_snapshot_preview_blocks(payload.preview.blocks)
            .into_iter()
            .map(|block| SessionPreviewBlock {
                role: leak_label(block.role),
                timestamp: block.timestamp,
                tone: block.tone,
                folded: block.folded,
                lines: block.lines,
            })
            .collect(),
    };
    session.rendered_sections = sanitize_snapshot_rendered_sections(payload.rendered_sections)
        .into_iter()
        .map(|section| SessionRenderedSection {
            title: leak_label(section.title),
            lines: section.lines,
        })
        .collect();
    upsert_session_metadata(&mut session.metadata, "Preview Hydration", "full".to_string());
}

fn build_remote_preview_payload_from_messages(
    session_id: &str,
    cwd: &str,
    path: &std::path::Path,
    title_store: &SessionTitleStore,
    title_hint: Option<String>,
    messages: Vec<yggterm_core::TranscriptMessage>,
) -> anyhow::Result<RemotePreviewPayload> {
    let started_at = messages
        .iter()
        .find_map(|message| message.timestamp.as_deref().map(parse_and_format_timestamp))
        .unwrap_or_else(|| format_display_datetime(OffsetDateTime::now_utc()));
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut metadata_entries = Vec::new();
    let mut blocks = Vec::new();
    for message in messages {
        let timestamp = message
            .timestamp
            .as_deref()
            .map(parse_and_format_timestamp)
            .unwrap_or_else(|| started_at.clone());
        push_preview_block(
            &mut blocks,
            &mut metadata_entries,
            &mut user_messages,
            &mut assistant_messages,
            message.role,
            message.lines,
            timestamp,
        );
    }
    Ok(RemotePreviewPayload {
        title_hint,
        cached_precis: title_store
            .get_precis(session_id)?
            .filter(|value| !value.trim().is_empty()),
        cached_summary: title_store
            .get_summary(session_id)?
            .filter(|value| !value.trim().is_empty()),
        preview: SnapshotPreview {
            summary: vec![
                SnapshotMetadataEntry {
                    label: "Session".to_string(),
                    value: session_id.to_string(),
                },
                SnapshotMetadataEntry {
                    label: "Storage".to_string(),
                    value: path.display().to_string(),
                },
                SnapshotMetadataEntry {
                    label: "Cwd".to_string(),
                    value: cwd.to_string(),
                },
                SnapshotMetadataEntry {
                    label: "Started".to_string(),
                    value: started_at,
                },
                SnapshotMetadataEntry {
                    label: "Messages".to_string(),
                    value: format!("{user_messages} user · {assistant_messages} assistant"),
                },
            ],
            blocks: blocks.into_iter().map(snapshot_preview_block).collect(),
        },
        rendered_sections: Vec::new(),
    })
}

pub fn fetch_remote_preview_payload(
    target: &SshConnectTarget,
    storage_path: &str,
) -> anyhow::Result<RemotePreviewPayload> {
    let output = run_remote_yggterm_command(
        &target.ssh_target,
        target.prefix.as_deref(),
        &["server", "remote", "preview", storage_path],
        None,
    )?;
    serde_json::from_str(&output).context("invalid remote preview payload")
}

pub fn fetch_remote_preview_head_payload(
    target: &SshConnectTarget,
    storage_path: &str,
    blocks: usize,
) -> anyhow::Result<RemotePreviewPayload> {
    let output = run_remote_yggterm_command(
        &target.ssh_target,
        target.prefix.as_deref(),
        &[
            "server",
            "remote",
            "preview-head",
            storage_path,
            &blocks.to_string(),
        ],
        None,
    )?;
    serde_json::from_str(&output).context("invalid remote preview head payload")
}

pub fn apply_remote_preview_payload_for_path(
    server: &mut YggtermServer,
    session_path: &str,
    payload: RemotePreviewPayload,
) -> bool {
    let mut applied = false;
    let refreshed_title = payload.title_hint.clone();
    let refreshed_precis = payload.cached_precis.clone();
    let refreshed_summary = payload.cached_summary.clone();
    if let Some(session) = server.sessions.get_mut(session_path) {
        apply_remote_preview_payload(session, payload);
        applied = true;
    }
    if applied {
        if let Some(title) = refreshed_title.as_deref() {
            server.set_session_title_hint(session_path, title);
        }
        if let Some(precis) = refreshed_precis.as_deref() {
            server.set_session_precis_hint(session_path, precis);
        }
        if let Some(summary) = refreshed_summary.as_deref() {
            server.set_session_summary_hint(session_path, summary);
        }
    }
    applied
}

pub fn fetch_remote_generation_context(
    target: &SshConnectTarget,
    storage_path: &str,
) -> anyhow::Result<String> {
    match run_remote_yggterm_command(
        &target.ssh_target,
        target.prefix.as_deref(),
        &["server", "remote", "generation-context", storage_path],
        None,
    ) {
        Ok(context) => Ok(context),
        Err(first_error) => {
            let cache_key = remote_cache_key(&target.ssh_target, target.prefix.as_deref());
            if let Ok(mut cache) = remote_command_cache().lock() {
                cache.remove(&cache_key);
            }
            let installed = bootstrap_remote_yggterm(&target.ssh_target, target.prefix.as_deref())?;
            run_remote_binary_command(
                &target.ssh_target,
                target.prefix.as_deref(),
                &installed,
                &["server", "remote", "generation-context", storage_path],
                None,
            )
            .with_context(|| {
                format!("retrying remote generation-context after bootstrap: {first_error:#}")
            })
        }
    }
}

fn collect_codex_session_files(
    root: &std::path::Path,
    out: &mut Vec<std::path::PathBuf>,
) -> anyhow::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("reading codex session dir {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_codex_session_files(&path, out)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
    Ok(())
}

fn remote_summary_for_path(
    path: &std::path::Path,
    title_store: &SessionTitleStore,
) -> anyhow::Result<Option<RemoteSummaryLine>> {
    let Some((session_id, cwd)) = read_codex_session_identity_fields(path)? else {
        return Ok(None);
    };
    let stat =
        fs::metadata(path).with_context(|| format!("reading metadata for {}", path.display()))?;
    let modified_epoch = stat
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let messages = read_codex_transcript_messages(path).unwrap_or_default();
    let user_message_count = messages
        .iter()
        .filter(|message| message.role == TranscriptRole::User)
        .count();
    let assistant_message_count = messages
        .iter()
        .filter(|message| message.role == TranscriptRole::Assistant)
        .count();
    let started_at = messages
        .iter()
        .find_map(|message| message.timestamp.clone())
        .unwrap_or_else(default_unknown);
    Ok(Some(RemoteSummaryLine {
        rollout_path: path.display().to_string(),
        id: session_id.clone(),
        cwd,
        started_at,
        modified_epoch,
        event_count: messages.len(),
        user_message_count,
        assistant_message_count,
        recent_context: summarize_recent_context(&messages),
        title_hint: title_store.get_title(&session_id)?,
        cached_precis: title_store.get_precis(&session_id)?,
        cached_summary: title_store.get_summary(&session_id)?,
    }))
}

fn remote_preview_payload_for_path(
    path: &std::path::Path,
    title_store: &SessionTitleStore,
) -> anyhow::Result<Option<RemotePreviewPayload>> {
    let Some((session_id, cwd)) = read_codex_session_identity_fields(path)? else {
        return Ok(None);
    };
    let title_hint = title_store
        .get_title(&session_id)?
        .filter(|value| !value.trim().is_empty() && !looks_like_generated_fallback_title(value));
    let session = build_session(
        SessionKind::Codex,
        &path.display().to_string(),
        Some(&session_id),
        Some(&cwd),
        title_hint.as_deref(),
        None,
        TerminalBackend::Xterm,
        UiTheme::ZedLight,
        false,
        StoredPreviewHydrationMode::Eager,
    );
    Ok(Some(RemotePreviewPayload {
        title_hint,
        cached_precis: title_store
            .get_precis(&session_id)?
            .filter(|value| !value.trim().is_empty()),
        cached_summary: title_store
            .get_summary(&session_id)?
            .filter(|value| !value.trim().is_empty()),
        preview: SnapshotPreview {
            summary: snapshot_metadata_entries(&session.preview.summary),
            blocks: session
                .preview
                .blocks
                .into_iter()
                .map(snapshot_preview_block)
                .collect(),
        },
        rendered_sections: sanitize_snapshot_rendered_sections(
            session
                .rendered_sections
                .into_iter()
                .map(|section| SnapshotRenderedSection {
                    title: section.title.to_string(),
                    lines: section.lines,
                })
                .collect(),
        ),
    }))
}

fn remote_preview_head_payload_for_path(
    path: &std::path::Path,
    title_store: &SessionTitleStore,
    max_blocks: usize,
) -> anyhow::Result<Option<RemotePreviewPayload>> {
    let Some((session_id, cwd)) = read_codex_session_identity_fields(path)? else {
        return Ok(None);
    };
    let title_hint = title_store
        .get_title(&session_id)?
        .filter(|value| !value.trim().is_empty() && !looks_like_generated_fallback_title(value));
    let messages = read_codex_transcript_messages_limited(path, max_blocks)
        .with_context(|| format!("reading remote transcript head {}", path.display()))?;
    Ok(Some(build_remote_preview_payload_from_messages(
        &session_id,
        &cwd,
        path,
        title_store,
        title_hint,
        messages,
    )?))
}

const REMOTE_SCAN_SCRIPT: &str = r#"
import json, os, sys, sqlite3
from pathlib import Path

def load_generated_copy():
    titles = {}
    precis = {}
    summaries = {}
    db_path = Path(os.path.expanduser("~/.yggterm/session-titles.db"))
    if not db_path.exists():
        return titles, precis, summaries
    conn = None
    try:
        conn = sqlite3.connect(str(db_path))
        for session_id, title in conn.execute("SELECT session_id, title FROM session_titles"):
            titles[session_id] = title
        for session_id, value in conn.execute("SELECT session_id, precis FROM session_precis"):
            precis[session_id] = value
        for session_id, value in conn.execute("SELECT session_id, summary FROM session_summaries"):
            summaries[session_id] = value
    except Exception:
        pass
    finally:
        if conn is not None:
            conn.close()
    return titles, precis, summaries

GENERATED_TITLES, GENERATED_PRECIS, GENERATED_SUMMARIES = load_generated_copy()

def summarize(path):
    session_id = "unknown"
    cwd = "<unknown>"
    started_at = "unknown"
    event_count = 0
    user_count = 0
    assistant_count = 0
    snippets = []
    def is_scaffold_line(text):
        lower = (text or "").strip().lower()
        if not lower:
            return False
        markers = (
            "<instructions>",
            "<cwd>",
            "<shell>",
            "<approval_policy>",
            "<sandbox_mode>",
            "<network_access>",
            "<environment_context",
            "</environment_context>",
            "<collaboration_mode>",
            "</collaboration_mode>",
            "<permissions instructions>",
            "</permissions instructions>",
        )
        return (
            any(marker in lower for marker in markers)
            or "you are now in default mode" in lower
            or "agents.md instructions for" in lower
            or "any previous instructions for other modes" in lower
            or "default mode you should strongly prefer" in lower
            or "if a decision is necessary and cannot be discovered from local context" in lower
            or "request_user_input" in lower
            or "filesystem sandboxing" in lower
            or "approvals are your mechanism to get user consent" in lower
            or "approval_policy is" in lower
            or "danger-full-access" in lower
            or "non-interactive mode where you may never ask the user for approval" in lower
            or "<turn_aborted>" in lower
            or "</turn_aborted>" in lower
            or "the user interrupted the previous turn on purpose" in lower
            or "any running unified exec processes were terminated" in lower
        )
    def normalize_lines(text):
        if not text:
            return []
        if isinstance(text, str):
            return [
                line.strip()
                for line in text.splitlines()
                if line.strip() and not is_scaffold_line(line)
            ]
        return []
    def extract_lines(payload):
        lines = []
        content = payload.get("content")
        if isinstance(content, str):
            lines.extend(normalize_lines(content))
        elif isinstance(content, list):
            for item in content:
                if isinstance(item, str):
                    lines.extend(normalize_lines(item))
                    continue
                if not isinstance(item, dict):
                    continue
                text = (
                    item.get("text")
                    or item.get("input_text")
                    or item.get("output_text")
                    or item.get("content")
                    or item.get("value")
                )
                lines.extend(normalize_lines(text))
        return lines
    def push_message(payload):
        role = payload.get("role") or "assistant"
        if role == "developer":
            return
        if role == "user":
            label = "USER"
        elif role == "assistant":
            label = "ASSISTANT"
        else:
            return
        lines = extract_lines(payload)
        if lines:
            snippets.append(f"{label}: {' '.join(lines)}")
    try:
        stat = path.stat()
        modified_epoch = int(stat.st_mtime)
        with path.open("r", encoding="utf-8", errors="replace") as fh:
            for raw in fh:
                raw = raw.strip()
                if not raw:
                    continue
                event_count += 1
                try:
                    value = json.loads(raw)
                except Exception:
                    continue
                ty = value.get("type")
                if ty == "session_meta":
                    payload = value.get("payload") or {}
                    session_id = payload.get("id") or session_id
                    cwd = payload.get("cwd") or cwd
                    started_at = payload.get("timestamp") or started_at
                elif ty == "response_item":
                    payload = value.get("payload") or {}
                    if payload.get("type") == "message":
                        role = payload.get("role")
                        if role == "user":
                            user_count += 1
                        elif role == "assistant":
                            assistant_count += 1
                        push_message(payload)
                elif ty == "compacted":
                    payload = value.get("payload") or {}
                    for msg in payload.get("replacement_history") or []:
                        if (msg or {}).get("type") != "message":
                            continue
                        role = msg.get("role")
                        if role == "user":
                            user_count += 1
                        elif role == "assistant":
                            assistant_count += 1
                        push_message(msg or {})
    except Exception:
        return None
    return {
        "rollout_path": str(path),
        "id": session_id,
        "cwd": cwd,
        "started_at": started_at,
        "modified_epoch": modified_epoch,
        "event_count": event_count,
        "user_message_count": user_count,
        "assistant_message_count": assistant_count,
        "recent_context": "\n".join(snippets[-6:]),
        "title_hint": GENERATED_TITLES.get(session_id),
        "cached_precis": GENERATED_PRECIS.get(session_id),
        "cached_summary": GENERATED_SUMMARIES.get(session_id),
    }

requested = sys.argv[1] if len(sys.argv) > 1 else "~/.codex"
codex_home = os.path.expanduser(requested)

def has_session_files(root: Path):
    try:
        for path in root.rglob("*.jsonl"):
            return True
    except Exception:
        return False
    return False

scan_roots = [Path(codex_home)]
if requested in ("", "~/.codex"):
    requested_root = Path(codex_home) / "sessions"
    if not requested_root.exists() or not has_session_files(requested_root):
        for parent in (Path("/home"), Path("/Users")):
            if not parent.exists():
                continue
            for child in parent.iterdir():
                candidate = child / ".codex"
                if candidate != Path(codex_home) and (candidate / "sessions").exists():
                    scan_roots.append(candidate)

seen = set()
for codex_root in scan_roots:
    root = codex_root / "sessions"
    if not root.exists():
        continue
    for path in root.rglob("*.jsonl"):
        path_str = str(path)
        if path_str in seen:
            continue
        seen.add(path_str)
        data = summarize(path)
        if data:
            print(json.dumps(data, ensure_ascii=False))
"#;

const REMOTE_UPSERT_GENERATED_COPY_SCRIPT: &str = r#"
import json, os, sqlite3, sys
from pathlib import Path

payload = json.loads(sys.argv[1])
home = Path(os.path.expanduser("~/.yggterm"))
home.mkdir(parents=True, exist_ok=True)
db_path = home / "session-titles.db"
conn = sqlite3.connect(str(db_path))
conn.executescript(
    """
    CREATE TABLE IF NOT EXISTS session_titles (
        session_id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        cwd TEXT,
        source TEXT,
        model TEXT,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS session_precis (
        session_id TEXT PRIMARY KEY,
        precis TEXT NOT NULL,
        cwd TEXT,
        source TEXT,
        model TEXT,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS session_summaries (
        session_id TEXT PRIMARY KEY,
        summary TEXT NOT NULL,
        cwd TEXT,
        source TEXT,
        model TEXT,
        updated_at TEXT NOT NULL
    );
    """
)

session_id = payload["session_id"]
cwd = payload.get("cwd") or ""
source = payload.get("source") or "interface-llm"
model = payload.get("model") or ""
updated_at = payload.get("updated_at") or ""

title = payload.get("title")
if title:
    conn.execute(
        """INSERT INTO session_titles (session_id, title, cwd, source, model, updated_at)
           VALUES (?, ?, ?, ?, ?, ?)
           ON CONFLICT(session_id) DO UPDATE SET
             title = excluded.title,
             cwd = excluded.cwd,
             source = excluded.source,
             model = excluded.model,
             updated_at = excluded.updated_at""",
        (session_id, title, cwd, source, model, updated_at),
    )

precis = payload.get("precis")
if precis:
    conn.execute(
        """INSERT INTO session_precis (session_id, precis, cwd, source, model, updated_at)
           VALUES (?, ?, ?, ?, ?, ?)
           ON CONFLICT(session_id) DO UPDATE SET
             precis = excluded.precis,
             cwd = excluded.cwd,
             source = excluded.source,
             model = excluded.model,
             updated_at = excluded.updated_at""",
        (session_id, precis, cwd, source, model, updated_at),
    )

summary = payload.get("summary")
if summary:
    conn.execute(
        """INSERT INTO session_summaries (session_id, summary, cwd, source, model, updated_at)
           VALUES (?, ?, ?, ?, ?, ?)
           ON CONFLICT(session_id) DO UPDATE SET
             summary = excluded.summary,
             cwd = excluded.cwd,
             source = excluded.source,
             model = excluded.model,
             updated_at = excluded.updated_at""",
        (session_id, summary, cwd, source, model, updated_at),
    )

conn.commit()
conn.close()
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteGeneratedCopyPayload {
    session_id: String,
    cwd: String,
    title: Option<String>,
    precis: Option<String>,
    summary: Option<String>,
    model: String,
    source: String,
    updated_at: String,
}

const REMOTE_METADATA_MIRROR_DB_FILENAME: &str = "remote-session-cache.db";

fn open_remote_metadata_mirror_store() -> anyhow::Result<Connection> {
    let home = resolve_yggterm_home()?;
    let db_path = home.join(REMOTE_METADATA_MIRROR_DB_FILENAME);
    let conn = Connection::open(&db_path).with_context(|| {
        format!(
            "failed to open remote metadata mirror {}",
            db_path.display()
        )
    })?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS remote_session_metadata (
            machine_key TEXT NOT NULL,
            session_id TEXT NOT NULL,
            cwd TEXT NOT NULL,
            started_at TEXT NOT NULL,
            modified_epoch INTEGER NOT NULL,
            event_count INTEGER NOT NULL,
            user_message_count INTEGER NOT NULL,
            assistant_message_count INTEGER NOT NULL,
            title_hint TEXT NOT NULL,
            recent_context TEXT NOT NULL,
            cached_precis TEXT,
            cached_summary TEXT,
            storage_path TEXT NOT NULL,
            synced_at TEXT NOT NULL,
            PRIMARY KEY(machine_key, session_id)
        );",
    )
    .context("failed to initialize remote metadata mirror schema")?;
    Ok(conn)
}

fn mirror_remote_machine_sessions(
    machine_key: &str,
    sessions: &[RemoteScannedSession],
) -> anyhow::Result<()> {
    let mut conn = open_remote_metadata_mirror_store()?;
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM remote_session_metadata WHERE machine_key = ?1",
        params![machine_key],
    )?;
    let synced_at =
        OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    for session in sessions {
        tx.execute(
            "INSERT INTO remote_session_metadata (
                machine_key, session_id, cwd, started_at, modified_epoch, event_count,
                user_message_count, assistant_message_count, title_hint, recent_context,
                cached_precis, cached_summary, storage_path, synced_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                machine_key,
                session.session_id,
                session.cwd,
                session.started_at,
                session.modified_epoch,
                session.event_count as i64,
                session.user_message_count as i64,
                session.assistant_message_count as i64,
                session.title_hint,
                session.recent_context,
                session.cached_precis,
                session.cached_summary,
                session.storage_path,
                synced_at,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn load_remote_machine_sessions_from_mirror(
    machine_key: &str,
) -> anyhow::Result<Vec<RemoteScannedSession>> {
    let conn = open_remote_metadata_mirror_store()?;
    let mut stmt = conn.prepare(
        "SELECT session_id, cwd, started_at, modified_epoch, event_count, user_message_count,
                assistant_message_count, title_hint, recent_context, cached_precis,
                cached_summary, storage_path
         FROM remote_session_metadata
         WHERE machine_key = ?1
         ORDER BY modified_epoch DESC, started_at DESC",
    )?;
    let rows = stmt.query_map(params![machine_key], |row| {
        let session_id: String = row.get(0)?;
        Ok(RemoteScannedSession {
            session_path: remote_scanned_session_path(machine_key, &session_id),
            session_id,
            cwd: row.get(1)?,
            started_at: row.get(2)?,
            modified_epoch: row.get(3)?,
            event_count: row.get::<_, i64>(4)? as usize,
            user_message_count: row.get::<_, i64>(5)? as usize,
            assistant_message_count: row.get::<_, i64>(6)? as usize,
            title_hint: row.get(7)?,
            recent_context: row.get(8)?,
            cached_precis: row.get(9)?,
            cached_summary: row.get(10)?,
            storage_path: row.get(11)?,
        })
    })?;
    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }
    Ok(dedupe_remote_scanned_sessions(sessions))
}

fn overlay_mirrored_remote_sessions(
    machine_key: &str,
    sessions: &[RemoteScannedSession],
) -> Vec<RemoteScannedSession> {
    let Ok(mirrored) = load_remote_machine_sessions_from_mirror(machine_key) else {
        return sessions.to_vec();
    };
    if mirrored.is_empty() {
        return sessions.to_vec();
    }
    let mirrored_by_id = mirrored
        .into_iter()
        .map(|session| (session.session_id.clone(), session))
        .collect::<std::collections::BTreeMap<_, _>>();
    dedupe_remote_scanned_sessions(
        sessions
            .iter()
            .cloned()
            .map(|mut session| {
                if let Some(mirrored) = mirrored_by_id.get(&session.session_id) {
                    if !mirrored.title_hint.trim().is_empty() {
                        session.title_hint = mirrored.title_hint.clone();
                    }
                    if session.recent_context.trim().is_empty()
                        && !mirrored.recent_context.trim().is_empty()
                    {
                        session.recent_context = mirrored.recent_context.clone();
                    }
                    if mirrored
                        .cached_precis
                        .as_ref()
                        .is_some_and(|value| !value.trim().is_empty())
                    {
                        session.cached_precis = mirrored.cached_precis.clone();
                    }
                    if mirrored
                        .cached_summary
                        .as_ref()
                        .is_some_and(|value| !value.trim().is_empty())
                    {
                        session.cached_summary = mirrored.cached_summary.clone();
                    }
                }
                session
            })
            .collect(),
    )
}

fn dedupe_remote_scanned_sessions(
    sessions: Vec<RemoteScannedSession>,
) -> Vec<RemoteScannedSession> {
    let mut by_id = std::collections::BTreeMap::<String, RemoteScannedSession>::new();
    for session in sessions {
        match by_id.entry(session.session_id.clone()) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(session);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let existing = slot.get_mut();
                let newer = session.modified_epoch > existing.modified_epoch
                    || (session.modified_epoch == existing.modified_epoch
                        && session.event_count > existing.event_count);
                let mut merged = if newer {
                    session.clone()
                } else {
                    existing.clone()
                };
                let other = if newer { existing.clone() } else { session };

                if looks_like_generated_fallback_title(&merged.title_hint)
                    && !looks_like_generated_fallback_title(&other.title_hint)
                {
                    merged.title_hint = other.title_hint;
                }
                if merged
                    .cached_precis
                    .as_ref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    merged.cached_precis = other.cached_precis;
                }
                if merged
                    .cached_summary
                    .as_ref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    merged.cached_summary = other.cached_summary;
                }
                if merged.recent_context.trim().is_empty()
                    || other.recent_context.len() > merged.recent_context.len()
                {
                    merged.recent_context = other.recent_context;
                }
                if merged.storage_path.trim().is_empty() {
                    merged.storage_path = other.storage_path;
                }
                *existing = merged;
            }
        }
    }
    let mut sessions = by_id.into_values().collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .modified_epoch
            .cmp(&left.modified_epoch)
            .then_with(|| right.started_at.cmp(&left.started_at))
            .then_with(|| right.event_count.cmp(&left.event_count))
    });
    sessions
}

fn update_remote_generated_copy_in_mirror(
    machine_key: &str,
    session_id: &str,
    title: Option<&str>,
    precis: Option<&str>,
    summary: Option<&str>,
) -> anyhow::Result<()> {
    let conn = open_remote_metadata_mirror_store()?;
    if let Some(title) = title {
        conn.execute(
            "UPDATE remote_session_metadata
             SET title_hint = ?3
             WHERE machine_key = ?1 AND session_id = ?2",
            params![machine_key, session_id, title],
        )?;
    }
    if let Some(precis) = precis {
        conn.execute(
            "UPDATE remote_session_metadata
             SET cached_precis = ?3
             WHERE machine_key = ?1 AND session_id = ?2",
            params![machine_key, session_id, precis],
        )?;
    }
    if let Some(summary) = summary {
        conn.execute(
            "UPDATE remote_session_metadata
             SET cached_summary = ?3
             WHERE machine_key = ?1 AND session_id = ?2",
            params![machine_key, session_id, summary],
        )?;
    }
    Ok(())
}

fn run_remote_python_lines(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    script: &str,
    args: &[String],
) -> anyhow::Result<Vec<String>> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("ConnectTimeout=5");
    cmd.arg("-o").arg("BatchMode=yes");
    let mut inner = String::from("python3 -");
    for arg in args {
        inner.push(' ');
        inner.push_str(&shell_single_quote(arg));
    }
    let remote = match exec_prefix
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
    {
        Some(prefix) => format!("{prefix} sh -c {}", shell_single_quote(&inner)),
        None => inner,
    };
    cmd.arg(ssh_target).arg(remote);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to start ssh python for {ssh_target}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(script.as_bytes())
            .with_context(|| format!("failed to send script to {ssh_target}"))?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed waiting for ssh python on {ssh_target}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "remote command failed for {}: {}",
            ssh_target,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

fn run_remote_python(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    script: &str,
    args: &[String],
) -> anyhow::Result<String> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("ConnectTimeout=5");
    cmd.arg("-o").arg("BatchMode=yes");
    let mut inner = String::from("python3 -");
    for arg in args {
        inner.push(' ');
        inner.push_str(&shell_single_quote(arg));
    }
    let remote = match exec_prefix
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
    {
        Some(prefix) => format!("{prefix} sh -c {}", shell_single_quote(&inner)),
        None => inner,
    };
    cmd.arg(ssh_target).arg(remote);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to start ssh python for {ssh_target}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(script.as_bytes())
            .with_context(|| format!("failed to send script to {ssh_target}"))?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed waiting for ssh python on {ssh_target}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "remote command failed for {}: {}",
            ssh_target,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn remote_shell_command(exec_prefix: Option<&str>, inner: &str) -> String {
    match exec_prefix
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
    {
        Some(prefix) => format!("{prefix} sh -c {}", shell_single_quote(inner)),
        None => inner.to_string(),
    }
}

const REMOTE_OUTPUT_SENTINEL: &str = "__YGGTERM_REMOTE_PAYLOAD__";

fn strip_remote_payload_noise(stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(stdout);
    if let Some(ix) = text.find(REMOTE_OUTPUT_SENTINEL) {
        let payload = &text[ix + REMOTE_OUTPUT_SENTINEL.len()..];
        return payload.trim().to_string();
    }
    text.trim().to_string()
}

fn remote_cache_key(ssh_target: &str, exec_prefix: Option<&str>) -> String {
    format!("{}|{}", ssh_target, exec_prefix.unwrap_or_default())
}

fn remote_command_cache()
-> &'static Mutex<std::collections::HashMap<String, RemoteCommandCacheEntry>> {
    REMOTE_YGGTERM_COMMAND_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn remote_command_resolve_locks()
-> &'static Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>> {
    REMOTE_YGGTERM_COMMAND_RESOLVE_LOCKS
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn remote_command_resolve_lock(cache_key: &str) -> Arc<Mutex<()>> {
    remote_command_resolve_locks()
        .lock()
        .expect("remote command resolve locks poisoned")
        .entry(cache_key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn run_remote_binary_command(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    binary_expr: &str,
    args: &[&str],
    stdin_bytes: Option<&[u8]>,
) -> anyhow::Result<String> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("ConnectTimeout=5");
    cmd.arg("-o").arg("BatchMode=yes");
    let mut binary_invocation = String::from(binary_expr);
    for arg in args {
        binary_invocation.push(' ');
        binary_invocation.push_str(&shell_single_quote(arg));
    }
    let inner = format!(
        "printf '%s\\n' {sentinel} ; {binary_invocation}",
        sentinel = shell_single_quote(REMOTE_OUTPUT_SENTINEL),
        binary_invocation = binary_invocation,
    );
    let remote = remote_shell_command(exec_prefix, &inner);
    cmd.arg(ssh_target).arg(remote);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to start remote yggterm command for {ssh_target}"))?;
    if let Some(stdin) = child.stdin.as_mut()
        && let Some(bytes) = stdin_bytes
    {
        stdin
            .write_all(bytes)
            .with_context(|| format!("failed to send remote yggterm payload to {ssh_target}"))?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed waiting for remote yggterm command on {ssh_target}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "remote yggterm command failed for {}: {}",
            ssh_target,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(strip_remote_payload_noise(&output.stdout))
}

fn run_remote_yggterm_command(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    args: &[&str],
    stdin_bytes: Option<&[u8]>,
) -> anyhow::Result<String> {
    let cache_key = remote_cache_key(ssh_target, exec_prefix);
    let resolved = resolve_remote_yggterm_binary(ssh_target, exec_prefix)?;
    match run_remote_binary_command(ssh_target, exec_prefix, &resolved.0, args, stdin_bytes) {
        Ok(output) => Ok(output),
        Err(first_error) => {
            if !should_fallback_to_python(&first_error) {
                return Err(first_error);
            }
            if let Ok(mut cache) = remote_command_cache().lock() {
                cache.remove(&cache_key);
            }
            let retried = resolve_remote_yggterm_binary(ssh_target, exec_prefix)?;
            run_remote_binary_command(ssh_target, exec_prefix, &retried.0, args, stdin_bytes)
                .with_context(|| {
                    format!("retrying remote yggterm command after cache reset: {first_error:#}")
                })
        }
    }
}

fn should_fallback_to_python(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}");
    message.contains("command not found")
        || message.contains("not found")
        || message.contains("No such file")
        || message.contains("Permission denied")
        || message.contains("permission denied")
        || message.contains("cannot execute")
        || message.contains("Exec format error")
        || message.contains("failed to upload yggterm binary")
        || message.contains("yggterm-headless only supports server subcommands")
        || message.contains("remote yggterm command failed")
}

fn looks_like_version(value: &str) -> bool {
    let text = value.trim();
    if text.is_empty() {
        return false;
    }
    let mut saw_dot = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            continue;
        }
        if ch == '.' {
            saw_dot = true;
            continue;
        }
        return false;
    }
    saw_dot
}

fn is_remote_protocol_probe_recoverable(output: &str) -> bool {
    let text = output.trim();
    if text.is_empty() {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("yggterm-headless only supports server subcommands")
        || lower.contains("supports server subcommands")
        || lower.contains("unrecognized command")
        || lower.contains("unknown command")
        || lower.contains("no such command")
        || lower.contains("not found")
        || lower.contains("command not found")
        || lower.contains("usage:")
        || lower.contains("not supported")
    {
        return true;
    }
    if looks_like_version(text) && text != daemon::SERVER_PROTOCOL_VERSION {
        return true;
    }
    false
}

fn check_remote_protocol_version(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    binary_expr: &str,
) -> anyhow::Result<()> {
    let descriptor = remote_protocol_descriptor_for_binary(ssh_target, exec_prefix, binary_expr)?;
    let normalized = descriptor.version.trim();
    let local_build_id = current_local_build_id();
    if normalized == daemon::SERVER_PROTOCOL_VERSION && descriptor.build_id == local_build_id {
        return Ok(());
    }
    if is_remote_protocol_probe_recoverable(normalized) {
        anyhow::bail!(
            "remote yggterm protocol mismatch for {} with {binary_expr}: expected {}, got {}",
            ssh_target,
            daemon::SERVER_PROTOCOL_VERSION,
            if normalized.is_empty() {
                "<empty>"
            } else {
                normalized
            }
        );
    }
    anyhow::bail!(
        "remote yggterm protocol mismatch for {} with {binary_expr}: expected {}@{}, got {}@{}",
        ssh_target,
        daemon::SERVER_PROTOCOL_VERSION,
        local_build_id,
        normalized,
        descriptor.build_id,
    )
}

fn current_local_build_id() -> u64 {
    local_remote_bootstrap_executable()
        .or_else(|| std::env::current_exe().ok())
        .and_then(|path| fs::read(path).ok())
        .map(|bytes| {
            const FNV_OFFSET: u64 = 0xcbf29ce484222325;
            const FNV_PRIME: u64 = 0x100000001b3;
            let mut hash = FNV_OFFSET;
            for byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(FNV_PRIME);
            }
            hash
        })
        .unwrap_or_default()
}

fn parse_remote_protocol_descriptor(text: &str) -> RemoteProtocolDescriptor {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let version = value
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or(trimmed)
            .to_string();
        let build_id = value
            .get("build_id")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        return RemoteProtocolDescriptor { version, build_id };
    }
    RemoteProtocolDescriptor {
        version: trimmed.to_string(),
        build_id: 0,
    }
}

fn remote_protocol_descriptor_for_binary(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    binary_expr: &str,
) -> anyhow::Result<RemoteProtocolDescriptor> {
    let output = run_remote_binary_command(
        ssh_target,
        exec_prefix,
        binary_expr,
        &["server", "remote", "protocol-version"],
        None,
    )?;
    Ok(parse_remote_protocol_descriptor(&output))
}

fn preferred_remote_binary_fallback() -> String {
    "$HOME/.yggterm/bin/yggterm".to_string()
}

fn bootstrap_remote_yggterm(ssh_target: &str, exec_prefix: Option<&str>) -> anyhow::Result<String> {
    let exe_path = local_remote_bootstrap_executable()
        .or_else(|| std::env::current_exe().ok())
        .context("resolving local yggterm remote executable")?;
    let payload = fs::read(&exe_path)
        .with_context(|| format!("reading local yggterm binary {}", exe_path.display()))?;
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("ConnectTimeout=10");
    cmd.arg("-o").arg("BatchMode=yes");
    let remote_path = "$HOME/.yggterm/bin/yggterm";
    let install_cmd = format!(
        "mkdir -p \"$HOME/.yggterm/bin\" && cat > \"$HOME/.yggterm/bin/yggterm.tmp\" && chmod +x \"$HOME/.yggterm/bin/yggterm.tmp\" && mv \"$HOME/.yggterm/bin/yggterm.tmp\" \"{remote_path}\" && printf \"%s\" \"{remote_path}\""
    );
    let remote = remote_shell_command(exec_prefix, &install_cmd);
    cmd.arg(ssh_target).arg(remote);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to start remote bootstrap for {ssh_target}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&payload)
            .with_context(|| format!("failed to upload yggterm binary to {ssh_target}"))?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed waiting for remote bootstrap on {ssh_target}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "remote bootstrap failed for {}: {}",
            ssh_target,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn local_remote_bootstrap_executable() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let current_ext = current
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let candidates = if current_ext.eq_ignore_ascii_case("exe") {
        vec![
            current.with_file_name("yggterm-headless.exe"),
            current.with_file_name("yggterm-headless-bin.exe"),
        ]
    } else {
        vec![
            current.with_file_name("yggterm-headless"),
            current.with_file_name("yggterm-headless-bin"),
        ]
    };
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn resolve_remote_yggterm_binary(
    ssh_target: &str,
    exec_prefix: Option<&str>,
) -> anyhow::Result<(String, RemoteDeployState)> {
    let perf_home = resolve_yggterm_home().ok();
    let mut perf_span = perf_home
        .as_ref()
        .map(|home| PerfSpan::start(home.clone(), "remote", "resolve_yggterm_binary"));
    let mut finish_span = |meta: serde_json::Value| {
        if let Some(span) = perf_span.take() {
            span.finish(meta);
        }
    };
    let cache_key = remote_cache_key(ssh_target, exec_prefix);
    let resolve_lock = remote_command_resolve_lock(&cache_key);
    let _resolve_guard = resolve_lock
        .lock()
        .expect("remote command resolve guard poisoned");
    if let Some(cached) = remote_command_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&cache_key).cloned())
    {
        let now_ms = current_millis_u64();
        if now_ms.saturating_sub(cached.verified_at_ms) <= REMOTE_COMMAND_CACHE_VERIFY_TTL_MS {
            finish_span(serde_json::json!({
                "ssh_target": ssh_target,
                "result": "cache_hit",
                "binary_expr": cached.binary_expr.clone(),
                "protocol_version": daemon::SERVER_PROTOCOL_VERSION,
            }));
            return Ok((cached.binary_expr, RemoteDeployState::Ready));
        }
        if check_remote_protocol_version(ssh_target, exec_prefix, &cached.binary_expr).is_ok() {
            if let Ok(mut cache) = remote_command_cache().lock() {
                cache.insert(
                    cache_key.clone(),
                    RemoteCommandCacheEntry {
                        binary_expr: cached.binary_expr.clone(),
                        verified_at_ms: now_ms,
                    },
                );
            }
            finish_span(serde_json::json!({
                "ssh_target": ssh_target,
                "result": "cache_revalidated",
                "binary_expr": cached.binary_expr.clone(),
                "protocol_version": daemon::SERVER_PROTOCOL_VERSION,
            }));
            return Ok((cached.binary_expr, RemoteDeployState::Ready));
        }
        if let Ok(mut cache) = remote_command_cache().lock() {
            cache.remove(&cache_key);
        }
    }

    let installed_binary = "$HOME/.yggterm/bin/yggterm";
    let local_build_id = current_local_build_id();

    match remote_protocol_descriptor_for_binary(ssh_target, exec_prefix, installed_binary) {
        Ok(descriptor)
            if descriptor.version.trim() == daemon::SERVER_PROTOCOL_VERSION
                && descriptor.build_id == local_build_id =>
        {
            if let Ok(mut cache) = remote_command_cache().lock() {
                cache.insert(
                    cache_key,
                    RemoteCommandCacheEntry {
                        binary_expr: installed_binary.to_string(),
                        verified_at_ms: current_millis_u64(),
                    },
                );
            }
            finish_span(serde_json::json!({
                "ssh_target": ssh_target,
                "result": "installed_path_match",
                "binary_expr": installed_binary,
                "protocol_version": descriptor.version.trim(),
                "build_id": descriptor.build_id,
            }));
            return Ok((installed_binary.to_string(), RemoteDeployState::Ready));
        }
        Ok(descriptor) => {
            if descriptor.version.trim() != daemon::SERVER_PROTOCOL_VERSION
                && !is_remote_protocol_probe_recoverable(&descriptor.version)
            {
                anyhow::bail!(
                    "remote yggterm protocol mismatch for {}: expected {}@{}, got {}@{}",
                    ssh_target,
                    daemon::SERVER_PROTOCOL_VERSION,
                    local_build_id,
                    descriptor.version.trim(),
                    descriptor.build_id
                );
            }
        }
        Err(error) if !should_fallback_to_python(&error) => return Err(error),
        Err(_) => {}
    }

    match remote_protocol_descriptor_for_binary(ssh_target, exec_prefix, "yggterm") {
        Ok(descriptor)
            if descriptor.version.trim() == daemon::SERVER_PROTOCOL_VERSION
                && descriptor.build_id == local_build_id =>
        {
            if let Ok(mut cache) = remote_command_cache().lock() {
                cache.insert(
                    cache_key,
                    RemoteCommandCacheEntry {
                        binary_expr: "yggterm".to_string(),
                        verified_at_ms: current_millis_u64(),
                    },
                );
            }
            finish_span(serde_json::json!({
                "ssh_target": ssh_target,
                "result": "path_match",
                "binary_expr": "yggterm",
                "protocol_version": descriptor.version.trim(),
                "build_id": descriptor.build_id,
            }));
            return Ok(("yggterm".to_string(), RemoteDeployState::Ready));
        }
        Ok(descriptor) => {
            if descriptor.version.trim() != daemon::SERVER_PROTOCOL_VERSION
                && !is_remote_protocol_probe_recoverable(&descriptor.version)
            {
                anyhow::bail!(
                    "remote yggterm protocol mismatch for {}: expected {}@{}, got {}@{}",
                    ssh_target,
                    daemon::SERVER_PROTOCOL_VERSION,
                    local_build_id,
                    descriptor.version.trim(),
                    descriptor.build_id
                );
            }
        }
        Err(error) if !should_fallback_to_python(&error) => return Err(error),
        Err(_) => {}
    }

    let installed = bootstrap_remote_yggterm(ssh_target, exec_prefix)?;
    let installed_descriptor =
        remote_protocol_descriptor_for_binary(ssh_target, exec_prefix, &installed)?;
    if installed_descriptor.version.trim() != daemon::SERVER_PROTOCOL_VERSION
        || installed_descriptor.build_id != local_build_id
    {
        anyhow::bail!(
            "remote yggterm protocol mismatch for {}: expected {}@{}, got {}@{}",
            ssh_target,
            daemon::SERVER_PROTOCOL_VERSION,
            local_build_id,
            installed_descriptor.version.trim(),
            installed_descriptor.build_id
        );
    }
    if let Ok(mut cache) = remote_command_cache().lock() {
        cache.insert(
            cache_key,
            RemoteCommandCacheEntry {
                binary_expr: installed.clone(),
                verified_at_ms: current_millis_u64(),
            },
        );
    }
    finish_span(serde_json::json!({
        "ssh_target": ssh_target,
        "result": "bootstrapped",
        "binary_expr": installed,
        "protocol_version": installed_descriptor.version.trim(),
        "build_id": installed_descriptor.build_id,
    }));
    Ok((installed, RemoteDeployState::Ready))
}

pub fn stage_remote_clipboard_png(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    png_bytes: &[u8],
) -> anyhow::Result<String> {
    match run_remote_yggterm_command(
        ssh_target,
        exec_prefix,
        &["server", "remote", "stage-clipboard-png"],
        Some(png_bytes),
    ) {
        Ok(path) => return Ok(path),
        Err(error) if !should_fallback_to_python(&error) => return Err(error),
        Err(_) => {}
    }
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("ConnectTimeout=5");
    cmd.arg("-o").arg("BatchMode=yes");
    let script = r#"import os, sys, time, uuid
home = os.path.expanduser("~/.yggterm/clipboard")
os.makedirs(home, exist_ok=True)
path = os.path.join(home, f"clipboard-{int(time.time() * 1000)}-{uuid.uuid4().hex[:8]}.png")
payload = sys.stdin.buffer.read()
if not payload:
    raise SystemExit("no clipboard image payload supplied")
with open(path, "wb") as handle:
    handle.write(payload)
print(path)
"#;
    let inner = format!("python3 -c {}", shell_single_quote(script));
    let remote = match exec_prefix
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
    {
        Some(prefix) => format!("{prefix} sh -c {}", shell_single_quote(&inner)),
        None => inner,
    };
    cmd.arg(ssh_target).arg(remote);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to start remote clipboard upload for {ssh_target}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(png_bytes)
            .with_context(|| format!("failed to send clipboard image to {ssh_target}"))?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed waiting for remote clipboard upload on {ssh_target}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "remote clipboard upload failed for {}: {}",
            ssh_target,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn scan_remote_machine_sessions(
    target: &SshConnectTarget,
) -> anyhow::Result<Vec<RemoteScannedSession>> {
    let python_args = [String::from("~/.codex")];
    let lines = match run_remote_yggterm_command(
        &target.ssh_target,
        target.prefix.as_deref(),
        &["server", "remote", "scan", "~/.codex"],
        None,
    ) {
        Ok(output) => output.lines().map(str::to_string).collect::<Vec<_>>(),
        Err(error) => match run_remote_python_lines(
            &target.ssh_target,
            target.prefix.as_deref(),
            REMOTE_SCAN_SCRIPT,
            &python_args,
        ) {
            Ok(lines) => lines,
            Err(_python_error) if !should_fallback_to_python(&error) => return Err(error),
            Err(python_error) => {
                return Err(python_error).with_context(|| {
                    format!("remote yggterm scan failed before python fallback: {error:#}")
                });
            }
        },
    };
    let machine_key = machine_key_from_ssh_target(&target.ssh_target);
    let mut sessions = Vec::new();
    for line in lines {
        let summary: RemoteSummaryLine =
            serde_json::from_str(&line).context("invalid remote summary line")?;
        let sanitized_recent_context = sanitize_recent_context_payload(&summary.recent_context);
        sessions.push(RemoteScannedSession {
            session_path: remote_scanned_session_path(&machine_key, &summary.id),
            session_id: summary.id.clone(),
            cwd: summary.cwd,
            started_at: summary.started_at,
            modified_epoch: summary.modified_epoch,
            event_count: summary.event_count,
            user_message_count: summary.user_message_count,
            assistant_message_count: summary.assistant_message_count,
            title_hint: summary
                .title_hint
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| short_session_id(&summary.id)),
            recent_context: sanitized_recent_context,
            cached_precis: summary
                .cached_precis
                .filter(|value| !value.trim().is_empty()),
            cached_summary: summary
                .cached_summary
                .filter(|value| !value.trim().is_empty()),
            storage_path: summary.rollout_path,
        });
    }
    Ok(dedupe_remote_scanned_sessions(sessions))
}

pub fn run_remote_stage_clipboard_png() -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let clipboard_dir = home.join("clipboard");
    fs::create_dir_all(&clipboard_dir)
        .with_context(|| format!("creating remote clipboard dir {}", clipboard_dir.display()))?;
    let filename = format!(
        "clipboard-{}-{}.png",
        current_millis_u64(),
        Uuid::new_v4().simple()
    );
    let path = clipboard_dir.join(filename);
    let mut payload = Vec::new();
    std::io::stdin()
        .read_to_end(&mut payload)
        .context("reading clipboard image payload from stdin")?;
    if payload.is_empty() {
        anyhow::bail!("no clipboard image payload supplied");
    }
    fs::write(&path, payload)
        .with_context(|| format!("writing remote clipboard image {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

pub fn run_remote_protocol_version() -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&json!({
            "version": daemon::SERVER_PROTOCOL_VERSION,
            "build_id": current_local_build_id(),
        }))?
    );
    Ok(())
}

pub fn run_remote_resume_codex(session_id: &str, cwd: Option<&str>) -> anyhow::Result<()> {
    let perf_home = resolve_yggterm_home().ok();
    let mut perf_span = perf_home
        .as_ref()
        .map(|home| PerfSpan::start(home.clone(), "remote", "resume_codex"));
    let mut finish_span = |meta: serde_json::Value| {
        if let Some(span) = perf_span.take() {
            span.finish(meta);
        }
    };
    let _ = ensure_local_managed_cli(ManagedCliTool::Codex)?;
    if let Some(multiplexer) = preferred_remote_multiplexer()? {
        let session_name = remote_tmux_session_name(session_id);
        let existing_session = match multiplexer {
            RemoteMultiplexer::Tmux => tmux_has_session(&session_name)?,
            RemoteMultiplexer::Screen => screen_has_session(&session_name)?,
        };
        if !existing_session {
            let saved_session_exists = remote_saved_codex_session_exists(session_id)?;
            let command = if saved_session_exists {
                remote_persistent_resume_shell_command(session_id, cwd)
            } else {
                remote_resume_picker_shell_command(session_id, cwd, None, true)
            };
            match multiplexer {
                RemoteMultiplexer::Tmux => tmux_spawn_codex_session(&session_name, &command)?,
                RemoteMultiplexer::Screen => screen_spawn_codex_session(&session_name, &command)?,
            }
            finish_span(serde_json::json!({
                "session_id": session_id,
                "cwd": cwd,
                "multiplexer": match multiplexer {
                    RemoteMultiplexer::Tmux => "tmux",
                    RemoteMultiplexer::Screen => "screen",
                },
                "mux_session": session_name,
                "mux_reused": false,
                "saved_session_exists": saved_session_exists,
                "mode": if saved_session_exists { "resume" } else { "resume_picker" },
            }));
        } else {
            if matches!(multiplexer, RemoteMultiplexer::Tmux) {
                let _ = tmux_snapshot_bytes(&session_name)
                    .and_then(|bytes| emit_terminal_snapshot(&bytes));
            }
            if matches!(multiplexer, RemoteMultiplexer::Screen) {
                let _ = screen_snapshot_bytes(&session_name)
                    .and_then(|bytes| emit_terminal_snapshot(&bytes));
            }
            finish_span(serde_json::json!({
                "session_id": session_id,
                "cwd": cwd,
                "multiplexer": match multiplexer {
                    RemoteMultiplexer::Tmux => "tmux",
                    RemoteMultiplexer::Screen => "screen",
                },
                "mux_session": session_name,
                "mux_reused": true,
                "mode": "attach_existing_multiplexer",
            }));
        }
        return match multiplexer {
            RemoteMultiplexer::Tmux => tmux_attach_session(&session_name),
            RemoteMultiplexer::Screen => screen_attach_session(&session_name),
        };
    }

    let saved_session_exists = remote_saved_codex_session_exists(session_id)?;
    let command = if saved_session_exists {
        remote_resume_shell_command(session_id, cwd, None)
    } else {
        remote_resume_picker_shell_command(session_id, cwd, None, true)
    };
    finish_span(serde_json::json!({
        "session_id": session_id,
        "cwd": cwd,
        "tmux": false,
        "saved_session_exists": saved_session_exists,
        "mode": if saved_session_exists { "resume" } else { "resume_picker" },
    }));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new("sh").arg("-lc").arg(command).exec();
        Err(anyhow::anyhow!(
            "failed to exec remote codex resume: {error}"
        ))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new("sh")
            .arg("-lc")
            .arg(command)
            .status()
            .context("running remote codex resume")?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "remote codex resume exited with status {status}"
            ))
        }
    }
}

fn refresh_remote_managed_cli(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    background: bool,
) -> anyhow::Result<ManagedCliRefreshReport> {
    let output = run_remote_yggterm_command(
        ssh_target,
        exec_prefix,
        &[
            "server",
            "remote",
            "refresh-managed-cli",
            if background {
                "background"
            } else {
                "foreground"
            },
        ],
        None,
    )?;
    serde_json::from_str(output.trim())
        .with_context(|| format!("parsing remote managed cli refresh report for {ssh_target}"))
}

pub fn run_remote_terminate_codex(session_id: &str) -> anyhow::Result<()> {
    let session_name = remote_tmux_session_name(session_id);
    match preferred_remote_multiplexer()? {
        Some(RemoteMultiplexer::Tmux) => {
            if !tmux_has_session(&session_name)? {
                return Ok(());
            }
            let _ = tmux_send_keys(&session_name, "/quit");
            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if !tmux_has_session(&session_name)? {
                    return Ok(());
                }
            }
            tmux_kill_session(&session_name)
        }
        Some(RemoteMultiplexer::Screen) => {
            if !screen_has_session(&session_name)? {
                return Ok(());
            }
            let _ = screen_send_keys(&session_name, "/quit");
            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if !screen_has_session(&session_name)? {
                    return Ok(());
                }
            }
            screen_kill_session(&session_name)
        }
        None => Ok(()),
    }
}

fn tail_text_file_lines(path: &std::path::Path, lines: usize) -> Vec<String> {
    read_trace_tail(path, lines.max(1))
}

pub fn request_app_control(
    home: &std::path::Path,
    command: AppControlCommand,
    timeout_ms: u64,
) -> anyhow::Result<AppControlResponse> {
    let request = enqueue_app_control_request(home, command, preferred_app_control_pid(home))?;
    wait_for_app_control_response(
        home,
        &request.request_id,
        std::time::Duration::from_millis(timeout_ms.max(250)),
    )
}

fn write_stdout_payload(payload: &str) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();
    if let Err(error) = stdout.write_all(payload.as_bytes()) {
        if error.kind() == ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(error.into());
    }
    if let Err(error) = stdout.write_all(b"\n") {
        if error.kind() == ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(error.into());
    }
    if let Err(error) = stdout.flush() {
        if error.kind() == ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(error.into());
    }
    Ok(())
}

fn parse_app_control_drag_placement(value: &str) -> Option<AppControlDragPlacement> {
    match value.trim().to_ascii_lowercase().as_str() {
        "before" => Some(AppControlDragPlacement::Before),
        "into" | "inside" => Some(AppControlDragPlacement::Into),
        "after" => Some(AppControlDragPlacement::After),
        _ => None,
    }
}

fn capture_embedded_app_screenshot(
    home: &std::path::Path,
    target: ScreenshotTarget,
    output_path: Option<std::path::PathBuf>,
    timeout_ms: u64,
) -> anyhow::Result<AppControlResponse> {
    let request =
        enqueue_screenshot_request(home, target, output_path, preferred_app_control_pid(home))?;
    wait_for_app_control_response(
        home,
        &request.request_id,
        std::time::Duration::from_millis(timeout_ms.max(250)),
    )
}

fn capture_embedded_app_screen_recording(
    home: &std::path::Path,
    output_path: Option<std::path::PathBuf>,
    duration_secs: u64,
    timeout_ms: u64,
) -> anyhow::Result<AppControlResponse> {
    let request = enqueue_screen_recording_request(
        home,
        output_path,
        duration_secs,
        preferred_app_control_pid(home),
    )?;
    wait_for_app_control_response(
        home,
        &request.request_id,
        std::time::Duration::from_millis(
            timeout_ms.max(duration_secs.saturating_mul(1_000) + 1_000),
        ),
    )
}

#[derive(Debug, Deserialize)]
struct ClientInstanceRecord {
    pid: u32,
    started_at_ms: u128,
}

fn client_instance_scope(endpoint: &ServerEndpoint) -> String {
    let raw = match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => format!("unix-{}", path.display()),
        ServerEndpoint::Tcp { host, port } => format!("tcp-{host}-{port}"),
    };
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn client_instances_dir(home: &Path, endpoint: &ServerEndpoint) -> PathBuf {
    home.join("client-instances")
        .join(client_instance_scope(endpoint))
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(not(unix))]
fn process_is_alive(pid: u32) -> bool {
    pid != 0
}

fn preferred_app_control_pid(home: &Path) -> Option<u32> {
    let endpoint = default_endpoint(home);
    let mut newest: Option<ClientInstanceRecord> = None;
    for record in active_client_instance_records(home, &endpoint).ok()? {
        if newest
            .as_ref()
            .is_none_or(|candidate| record.started_at_ms > candidate.started_at_ms)
        {
            newest = Some(record);
        }
    }
    newest.map(|record| record.pid)
}

pub(crate) fn active_client_instance_records(
    home: &Path,
    endpoint: &ServerEndpoint,
) -> anyhow::Result<Vec<ClientInstanceRecord>> {
    let dir = client_instances_dir(home, endpoint);
    let entries = fs::read_dir(&dir).or_else(|error| {
        if error.kind() == ErrorKind::NotFound {
            Ok(fs::read_dir(home.join(".missing-client-instances-dir"))?)
        } else {
            Err(error)
        }
    });
    let Ok(entries) = entries else {
        return Ok(Vec::new());
    };
    let mut active = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_slice::<ClientInstanceRecord>(&bytes) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        if process_is_alive(record.pid) {
            active.push(record);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(active)
}

fn capture_trace_screenshot(home: &std::path::Path) -> Option<std::path::PathBuf> {
    let snapshots_dir = home.join("trace-snapshots");
    fs::create_dir_all(&snapshots_dir).ok()?;
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let output_path = snapshots_dir.join(format!("trace-{ts_ms}.png"));
    let mut attempts = Vec::<(&str, Vec<String>)>::new();
    attempts.push((
        "import",
        vec![
            "-window".to_string(),
            "root".to_string(),
            output_path.display().to_string(),
        ],
    ));
    attempts.push((
        "gnome-screenshot",
        vec!["-f".to_string(), output_path.display().to_string()],
    ));
    attempts.push(("scrot", vec![output_path.display().to_string()]));
    attempts.push(("maim", vec![output_path.display().to_string()]));
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        attempts.push(("grim", vec![output_path.display().to_string()]));
    }
    for (binary, args) in attempts {
        let status = Command::new(binary)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.ok().is_some_and(|status| status.success())
            && fs::metadata(&output_path)
                .ok()
                .is_some_and(|metadata| metadata.is_file() && metadata.len() > 0)
        {
            return Some(output_path.clone());
        }
        let _ = fs::remove_file(&output_path);
    }
    None
}

fn trace_bundle(lines: usize, include_screenshot: bool) -> anyhow::Result<serde_json::Value> {
    let home = resolve_yggterm_home()?;
    let event_path = event_trace_path(&home);
    let perf_path = home.join(yggterm_core::PERF_TELEMETRY_FILENAME);
    let ui_path = home.join("ui-telemetry.jsonl");
    let panic_path = home.join("panic.log");
    let endpoint = default_endpoint(&home);
    let daemon_status = status(&endpoint)
        .ok()
        .and_then(|status| serde_json::to_value(status).ok());
    let daemon_snapshot = snapshot(&endpoint).ok().map(|(snapshot, message)| {
        serde_json::json!({
            "message": message,
            "active_session_path": snapshot.active_session_path,
            "active_view_mode": format!("{:?}", snapshot.active_view_mode),
            "live_sessions": snapshot.live_sessions.len(),
            "remote_machines": snapshot.remote_machines.len(),
            "ssh_targets": snapshot.ssh_targets.len(),
        })
    });
    let app_state = request_app_control(&home, AppControlCommand::DescribeState, 5_000)
        .ok()
        .and_then(|response| response.data);
    let screenshot_path = if include_screenshot {
        capture_embedded_app_screenshot(&home, ScreenshotTarget::App, None, 10_000)
            .ok()
            .and_then(|response| response.output_path)
            .or_else(|| capture_trace_screenshot(&home).map(|path| path.display().to_string()))
    } else {
        None
    };
    Ok(serde_json::json!({
        "generated_at_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
        "home_dir": home.display().to_string(),
        "display": std::env::var("DISPLAY").ok(),
        "wayland_display": std::env::var("WAYLAND_DISPLAY").ok(),
        "event_trace_path": event_path.display().to_string(),
        "event_tail": tail_text_file_lines(&event_path, lines),
        "perf_tail": tail_text_file_lines(&perf_path, lines),
        "ui_tail": tail_text_file_lines(&ui_path, lines),
        "panic_tail": tail_text_file_lines(&panic_path, lines.min(50)),
        "daemon_status": daemon_status,
        "daemon_snapshot": daemon_snapshot,
        "app_state": app_state,
        "screenshot_path": screenshot_path,
    }))
}

pub fn run_trace_tail(lines: usize) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let path = event_trace_path(&home);
    for line in tail_text_file_lines(&path, lines) {
        println!("{line}");
    }
    Ok(())
}

pub fn run_trace_follow(lines: usize, poll_ms: u64) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let path = event_trace_path(&home);
    follow_trace_lines(&path, lines, poll_ms);
}

pub fn run_trace_bundle(lines: usize, include_screenshot: bool) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&trace_bundle(lines, include_screenshot)?)?
    );
    Ok(())
}

pub fn run_screenshot_capture(
    target: &str,
    output_path: Option<&str>,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = match target {
        "app" => capture_embedded_app_screenshot(
            &home,
            ScreenshotTarget::App,
            output_path.map(std::path::PathBuf::from),
            timeout_ms,
        )?,
        "preview" | "preview_viewport" => capture_embedded_app_screenshot(
            &home,
            ScreenshotTarget::PreviewViewport,
            output_path.map(std::path::PathBuf::from),
            timeout_ms,
        )?,
        other => anyhow::bail!("unsupported screenshot target: {other}"),
    };
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_screenrecord_capture(
    target: &str,
    output_path: Option<&str>,
    timeout_ms: u64,
    duration_secs: u64,
) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = match target {
        "app" => capture_embedded_app_screen_recording(
            &home,
            output_path.map(std::path::PathBuf::from),
            duration_secs.max(1),
            timeout_ms,
        )?,
        other => anyhow::bail!("unsupported screenrecord target: {other}"),
    };
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_scroll_preview(
    top_px: Option<f64>,
    ratio: Option<f64>,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(
        &home,
        AppControlCommand::ScrollPreview { top_px, ratio },
        timeout_ms,
    )?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_describe_state(timeout_ms: u64) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(&home, AppControlCommand::DescribeState, timeout_ms)?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_dump_state(output_path: &str, timeout_ms: u64) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(&home, AppControlCommand::DescribeState, timeout_ms)?;
    let data = response
        .data
        .context("app control dump missing response data")?;
    let output_path = PathBuf::from(output_path);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating app dump dir {}", parent.display()))?;
    }
    fs::write(&output_path, serde_json::to_vec_pretty(&data)?)
        .with_context(|| format!("writing app dump {}", output_path.display()))?;
    write_stdout_payload(&serde_json::to_string_pretty(&json!({
        "request_id": response.request_id,
        "handled_by_pid": response.handled_by_pid,
        "completed_at_ms": response.completed_at_ms,
        "output_path": output_path,
        "error": response.error,
    }))?)?;
    Ok(())
}

pub fn run_app_control_focus_window(timeout_ms: u64) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(&home, AppControlCommand::FocusWindow, timeout_ms)?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_set_fullscreen(enabled: bool, timeout_ms: u64) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(
        &home,
        AppControlCommand::SetFullscreen { enabled },
        timeout_ms,
    )?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_describe_rows(timeout_ms: u64) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(&home, AppControlCommand::DescribeRows, timeout_ms)?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_set_row_expanded(
    row_path: &str,
    expanded: bool,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(
        &home,
        AppControlCommand::SetRowExpanded {
            row_path: row_path.to_string(),
            expanded,
        },
        timeout_ms,
    )?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_open_path(
    session_path: &str,
    view_mode: Option<AppControlViewMode>,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(
        &home,
        AppControlCommand::OpenPath {
            session_path: session_path.to_string(),
            view_mode,
        },
        timeout_ms,
    )?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_drag(
    action: &str,
    row_path: Option<&str>,
    placement: Option<&str>,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let command = match action {
        "begin" => AppControlCommand::Drag {
            command: AppControlDragCommand::Begin {
                row_path: row_path
                    .context("missing row path for server app drag begin")?
                    .to_string(),
            },
        },
        "hover" => AppControlCommand::Drag {
            command: AppControlDragCommand::Hover {
                row_path: row_path
                    .context("missing row path for server app drag hover")?
                    .to_string(),
                placement: parse_app_control_drag_placement(
                    placement.context("missing placement for server app drag hover")?,
                )
                .context("unsupported drag placement; use before, into, or after")?,
            },
        },
        "drop" => AppControlCommand::Drag {
            command: AppControlDragCommand::Drop,
        },
        "clear" | "cancel" => AppControlCommand::Drag {
            command: AppControlDragCommand::Clear,
        },
        other => anyhow::bail!("unsupported app drag action: {other}"),
    };
    let response = request_app_control(&home, command, timeout_ms)?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_create_terminal(
    machine_key: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(
        &home,
        AppControlCommand::CreateTerminal {
            machine_key: machine_key.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
        },
        timeout_ms,
    )?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_send_terminal_input(
    session_path: &str,
    data: &str,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(
        &home,
        AppControlCommand::SendTerminalInput {
            session_path: session_path.to_string(),
            data: data.to_string(),
        },
        timeout_ms,
    )?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_app_control_remove_session(session_path: &str, timeout_ms: u64) -> anyhow::Result<()> {
    let home = resolve_yggterm_home()?;
    let response = request_app_control(
        &home,
        AppControlCommand::RemoveSession {
            session_path: session_path.to_string(),
        },
        timeout_ms,
    )?;
    write_stdout_payload(&serde_json::to_string_pretty(&response)?)?;
    Ok(())
}

pub fn run_remote_refresh_managed_cli(background: bool) -> anyhow::Result<()> {
    let report = refresh_local_managed_cli(background)?;
    write_stdout_payload(&serde_json::to_string(&report)?)?;
    Ok(())
}

pub fn run_remote_ensure_managed_cli(tool: ManagedCliTool) -> anyhow::Result<()> {
    let status = ensure_local_managed_cli(tool)?;
    write_stdout_payload(&serde_json::to_string(&status)?)?;
    Ok(())
}

pub fn run_remote_scan(codex_home: Option<&str>) -> anyhow::Result<()> {
    let requested_home = codex_home
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.starts_with("~/") {
                std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(value.trim_start_matches("~/"))
            } else {
                std::path::PathBuf::from(value)
            }
        })
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".codex")
        });
    let scan_roots = remote_scan_roots(&requested_home, codex_home);
    let yggterm_home = resolve_yggterm_home()?;
    let title_store = SessionTitleStore::open(&yggterm_home)?;
    let mut files = Vec::new();
    for root in scan_roots {
        collect_codex_session_files(&root.join("sessions"), &mut files)?;
    }
    files.sort();
    files.dedup();
    for path in files {
        if let Some(summary) = remote_summary_for_path(&path, &title_store)? {
            write_stdout_line(&serde_json::to_string(&summary)?)?;
        }
    }
    Ok(())
}

fn remote_scan_roots(
    requested_home: &std::path::Path,
    raw_codex_home: Option<&str>,
) -> Vec<std::path::PathBuf> {
    remote_scan_roots_with_parents(requested_home, raw_codex_home, std::iter::empty())
}

fn remote_scan_roots_with_parents<'a>(
    requested_home: &std::path::Path,
    raw_codex_home: Option<&str>,
    parents: impl IntoIterator<Item = &'a std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    roots.push(requested_home.to_path_buf());

    let default_like = raw_codex_home
        .map(str::trim)
        .is_none_or(|value| value.is_empty() || value == "~/.codex");
    if !default_like {
        return roots;
    }
    let _ = parents.into_iter().count();
    roots
}

pub fn run_remote_preview(path: &str) -> anyhow::Result<()> {
    let yggterm_home = resolve_yggterm_home()?;
    let title_store = SessionTitleStore::open(&yggterm_home)?;
    let payload = remote_preview_payload_for_path(std::path::Path::new(path), &title_store)?
        .with_context(|| format!("no previewable codex session at {path}"))?;
    write_stdout_line(&serde_json::to_string(&payload)?)?;
    Ok(())
}

pub fn run_remote_preview_head(path: &str, blocks: usize) -> anyhow::Result<()> {
    let yggterm_home = resolve_yggterm_home()?;
    let title_store = SessionTitleStore::open(&yggterm_home)?;
    let payload =
        remote_preview_head_payload_for_path(std::path::Path::new(path), &title_store, blocks)?
            .with_context(|| format!("no previewable codex session at {path}"))?;
    write_stdout_line(&serde_json::to_string(&payload)?)?;
    Ok(())
}

pub fn run_remote_generation_context(path: &str) -> anyhow::Result<()> {
    let context = generation_context_from_messages(
        &read_codex_transcript_messages(std::path::Path::new(path))
            .with_context(|| format!("reading remote transcript {}", path))?,
    );
    write_stdout_line(&context)?;
    Ok(())
}

fn write_stdout_line(line: &str) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();
    match writeln!(stdout, "{line}") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn run_remote_upsert_generated_copy(payload_json: &str) -> anyhow::Result<()> {
    let payload: RemoteGeneratedCopyPayload =
        serde_json::from_str(payload_json).context("parsing remote generated copy payload")?;
    let home = resolve_yggterm_home()?;
    let db_path = home.join("session-titles.db");
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating title db dir {}", parent.display()))?;
    }
    let conn = Connection::open(&db_path)
        .with_context(|| format!("opening session title db {}", db_path.display()))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_titles (
            session_id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            cwd TEXT,
            source TEXT,
            model TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_precis (
            session_id TEXT PRIMARY KEY,
            precis TEXT NOT NULL,
            cwd TEXT,
            source TEXT,
            model TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_summaries (
            session_id TEXT PRIMARY KEY,
            summary TEXT NOT NULL,
            cwd TEXT,
            source TEXT,
            model TEXT,
            updated_at TEXT NOT NULL
        );",
    )?;
    if let Some(title) = payload
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        conn.execute(
            "INSERT INTO session_titles (session_id, title, cwd, source, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
               title = excluded.title,
               cwd = excluded.cwd,
               source = excluded.source,
               model = excluded.model,
               updated_at = excluded.updated_at",
            params![
                payload.session_id,
                title,
                payload.cwd,
                payload.source,
                payload.model,
                payload.updated_at,
            ],
        )?;
    }
    if let Some(precis) = payload
        .precis
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        conn.execute(
            "INSERT INTO session_precis (session_id, precis, cwd, source, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
               precis = excluded.precis,
               cwd = excluded.cwd,
               source = excluded.source,
               model = excluded.model,
               updated_at = excluded.updated_at",
            params![
                payload.session_id,
                precis,
                payload.cwd,
                payload.source,
                payload.model,
                payload.updated_at,
            ],
        )?;
    }
    if let Some(summary) = payload
        .summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        conn.execute(
            "INSERT INTO session_summaries (session_id, summary, cwd, source, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
               summary = excluded.summary,
               cwd = excluded.cwd,
               source = excluded.source,
               model = excluded.model,
               updated_at = excluded.updated_at",
            params![
                payload.session_id,
                summary,
                payload.cwd,
                payload.source,
                payload.model,
                payload.updated_at,
            ],
        )?;
    }
    Ok(())
}

pub fn persist_remote_generated_copy(
    machine: &RemoteMachineSnapshot,
    session_id: &str,
    cwd: &str,
    title: Option<&str>,
    precis: Option<&str>,
    summary: Option<&str>,
    model: &str,
) -> anyhow::Result<()> {
    let updated_at =
        time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    let payload = RemoteGeneratedCopyPayload {
        session_id: session_id.to_string(),
        cwd: cwd.to_string(),
        title: title.map(ToOwned::to_owned),
        precis: precis.map(ToOwned::to_owned),
        summary: summary.map(ToOwned::to_owned),
        model: model.to_string(),
        source: "interface-llm".to_string(),
        updated_at,
    };
    let payload_json = serde_json::to_string(&payload)?;
    match run_remote_yggterm_command(
        &machine.ssh_target,
        machine.prefix.as_deref(),
        &["server", "remote", "upsert-generated-copy", &payload_json],
        None,
    ) {
        Ok(_) => {}
        Err(error) if !should_fallback_to_python(&error) => return Err(error),
        Err(_) => {
            let args = vec![payload_json];
            let _ = run_remote_python(
                &machine.ssh_target,
                machine.prefix.as_deref(),
                REMOTE_UPSERT_GENERATED_COPY_SCRIPT,
                &args,
            )?;
        }
    }
    if let Err(error) = update_remote_generated_copy_in_mirror(
        &machine.machine_key,
        session_id,
        title,
        precis,
        summary,
    ) {
        warn!(machine_key=%machine.machine_key, session_id, error=%error, "failed to update local remote metadata mirror");
    }
    Ok(())
}

pub fn terminate_remote_codex_session(
    machine: &RemoteMachineSnapshot,
    session_id: &str,
) -> anyhow::Result<()> {
    match run_remote_yggterm_command(
        &machine.ssh_target,
        machine.prefix.as_deref(),
        &["server", "remote", "terminate-codex", session_id],
        None,
    ) {
        Ok(_) => Ok(()),
        Err(error) if !should_fallback_to_python(&error) => Err(error),
        Err(_) => {
            let session_name = remote_tmux_session_name(session_id);
            let command = format!(
                "if command -v tmux >/dev/null 2>&1 && tmux has-session -t {} 2>/dev/null; then tmux send-keys -t {} /quit Enter >/dev/null 2>&1 || true; sleep 2; tmux kill-session -t {} >/dev/null 2>&1 || true; \
                 elif command -v screen >/dev/null 2>&1 && screen -ls {} 2>/dev/null | grep -q '\\\\.'; then screen -S {} -X stuff '/quit\\015' >/dev/null 2>&1 || true; sleep 2; screen -S {} -X quit >/dev/null 2>&1 || true; fi",
                shell_single_quote(&session_name),
                shell_single_quote(&session_name),
                shell_single_quote(&session_name),
                shell_single_quote(&session_name),
                shell_single_quote(&session_name),
                shell_single_quote(&session_name),
            );
            let remote = remote_shell_command(machine.prefix.as_deref(), &command);
            let status = Command::new("ssh")
                .arg("-o")
                .arg("ConnectTimeout=8")
                .arg("-o")
                .arg("BatchMode=yes")
                .arg(&machine.ssh_target)
                .arg(remote)
                .status()
                .with_context(|| {
                    format!(
                        "failed to terminate remote codex session {} on {}",
                        session_id, machine.ssh_target
                    )
                })?;
            if status.success() {
                Ok(())
            } else {
                anyhow::bail!(
                    "failed to terminate remote codex session {} on {}: {}",
                    session_id,
                    machine.ssh_target,
                    status
                );
            }
        }
    }
}

fn snapshot_preview_block(block: SessionPreviewBlock) -> SnapshotPreviewBlock {
    SnapshotPreviewBlock {
        role: block.role.to_string(),
        timestamp: block.timestamp,
        tone: block.tone,
        folded: block.folded,
        lines: block.lines,
    }
}

fn snapshot_session_view(session: ManagedSessionView) -> SnapshotSessionView {
    let preview_blocks = sanitize_session_preview_blocks(session.preview.blocks);
    SnapshotSessionView {
        id: session.id,
        session_path: session.session_path,
        title: session.title,
        kind: session.kind,
        host_label: session.host_label,
        source: session.source,
        backend: session.backend,
        bridge_available: session.bridge_available,
        launch_phase: session.launch_phase,
        remote_deploy_state: session.remote_deploy_state,
        launch_command: session.launch_command,
        status_line: session.status_line,
        terminal_lines: session.terminal_lines,
        rendered_sections: session
            .rendered_sections
            .into_iter()
            .map(|section| SnapshotRenderedSection {
                title: section.title.to_string(),
                lines: section.lines,
            })
            .collect(),
        preview: SnapshotPreview {
            summary: snapshot_metadata_entries(&session.preview.summary),
            blocks: preview_blocks
                .into_iter()
                .map(snapshot_preview_block)
                .collect(),
        },
        metadata: snapshot_metadata_entries(&session.metadata),
        terminal_process_id: session.terminal_process_id,
        terminal_window_id: session.terminal_window_id,
        terminal_host_token: session.terminal_host_token,
        terminal_host_mode: session.terminal_host_mode,
        embedded_surface_id: session.embedded_surface_id,
        embedded_surface_detail: session.embedded_surface_detail,
        last_launch_error: session.last_launch_error,
        last_window_error: session.last_window_error,
        ssh_target: session.ssh_target,
        ssh_prefix: session.ssh_prefix,
    }
}

fn leak_label(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

fn managed_session_from_snapshot(session: SnapshotSessionView) -> ManagedSessionView {
    let preview_blocks = sanitize_snapshot_preview_blocks(session.preview.blocks);
    ManagedSessionView {
        id: session.id,
        session_path: session.session_path,
        title: session.title,
        kind: session.kind,
        host_label: session.host_label,
        source: session.source,
        backend: session.backend,
        bridge_available: session.bridge_available,
        launch_phase: session.launch_phase,
        remote_deploy_state: session.remote_deploy_state,
        launch_command: session.launch_command,
        status_line: session.status_line,
        terminal_lines: session.terminal_lines,
        rendered_sections: session
            .rendered_sections
            .into_iter()
            .map(|section| SessionRenderedSection {
                title: leak_label(section.title),
                lines: section.lines,
            })
            .collect(),
        preview: SessionPreview {
            summary: session
                .preview
                .summary
                .into_iter()
                .map(|entry| SessionMetadataEntry {
                    label: leak_label(entry.label),
                    value: entry.value,
                })
                .collect(),
            blocks: preview_blocks
                .into_iter()
                .map(|block| SessionPreviewBlock {
                    role: leak_label(block.role),
                    timestamp: block.timestamp,
                    tone: block.tone,
                    folded: block.folded,
                    lines: block.lines,
                })
                .collect(),
        },
        metadata: session
            .metadata
            .into_iter()
            .map(|entry| SessionMetadataEntry {
                label: leak_label(entry.label),
                value: entry.value,
            })
            .collect(),
        terminal_process_id: session.terminal_process_id,
        terminal_window_id: session.terminal_window_id,
        terminal_host_token: session.terminal_host_token,
        terminal_host_mode: session.terminal_host_mode,
        embedded_surface_id: session.embedded_surface_id,
        embedded_surface_detail: session.embedded_surface_detail,
        last_launch_error: session.last_launch_error,
        last_window_error: session.last_window_error,
        ssh_target: session.ssh_target,
        ssh_prefix: session.ssh_prefix,
        stored_preview_hydrated: true,
    }
}

struct StoredLeaf {
    kind: SessionNodeKind,
    path: String,
    session_id: String,
    cwd: String,
    title: String,
}

#[derive(Debug, Clone)]
struct StoredTranscript {
    started_at: String,
    user_messages: usize,
    assistant_messages: usize,
    metadata_entries: Vec<SessionMetadataEntry>,
    blocks: Vec<SessionPreviewBlock>,
}

#[derive(Debug, Clone)]
struct StoredFileSnapshot {
    updated_at: Option<String>,
    bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredPreviewHydrationMode {
    Deferred,
    Eager,
}

fn first_session_leaf(node: &SessionNode) -> Option<StoredLeaf> {
    if node.kind != SessionNodeKind::Group {
        return Some(StoredLeaf {
            kind: node.kind,
            path: node.path.display().to_string(),
            session_id: node.session_id.clone().unwrap_or_else(|| node.name.clone()),
            cwd: node
                .cwd
                .clone()
                .unwrap_or_else(|| session_preview_cwd(&node.path.display().to_string())),
            title: node.title.clone().unwrap_or_else(|| node.name.clone()),
        });
    }

    for child in &node.children {
        if let Some(path) = first_session_leaf(child) {
            return Some(path);
        }
    }

    None
}

fn build_session(
    kind: SessionKind,
    path: &str,
    session_id: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    document: Option<&WorkspaceDocument>,
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
    hydration_mode: StoredPreviewHydrationMode,
) -> ManagedSessionView {
    let session_id = session_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string());
    let title = title_hint
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| short_session_id(&session_id));
    let host_label = String::from("localhost");

    let appearance = match theme {
        UiTheme::ZedDark => "dark",
        UiTheme::ZedLight => "light",
    };
    let started_at = format_display_datetime(OffsetDateTime::now_utc());
    let backend_label = match backend {
        TerminalBackend::Xterm => "xterm.js",
        TerminalBackend::Ghostty => "Ghostty",
        TerminalBackend::Mock => "Mock",
    };
    let cwd = cwd
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| session_preview_cwd(path));
    let launch_command = stored_session_launch_command(kind, &cwd, &session_id);
    let should_hydrate_stored_preview = kind == SessionKind::Document
        || matches!(hydration_mode, StoredPreviewHydrationMode::Eager);
    let transcript = if !should_hydrate_stored_preview {
        None
    } else if kind == SessionKind::Document {
        None
    } else {
        parse_stored_transcript(path, &started_at)
    };
    let file_snapshot = if kind == SessionKind::Document {
        StoredFileSnapshot {
            updated_at: document.map(|value| value.updated_at.clone()),
            bytes: document.map(|value| value.body.len() as u64),
        }
    } else {
        stored_file_snapshot(path)
    };
    let started_at = transcript
        .as_ref()
        .map(|transcript| transcript.started_at.clone())
        .unwrap_or(started_at);
    let preview_block_count = transcript
        .as_ref()
        .map(|transcript| transcript.blocks.len())
        .unwrap_or(0);
    let message_count = transcript
        .as_ref()
        .map(|transcript| transcript.user_messages + transcript.assistant_messages)
        .unwrap_or(0);
    let mut preview_summary = vec![
        SessionMetadataEntry {
            label: if kind == SessionKind::Document {
                "Document"
            } else {
                "Session"
            },
            value: session_id.clone(),
        },
        SessionMetadataEntry {
            label: "Storage",
            value: path.to_string(),
        },
        SessionMetadataEntry {
            label: "Cwd",
            value: cwd.clone(),
        },
        SessionMetadataEntry {
            label: "Started",
            value: started_at.clone(),
        },
        SessionMetadataEntry {
            label: "Messages",
            value: transcript
                .as_ref()
                .map(|transcript| {
                    format!(
                        "{} user · {} assistant",
                        transcript.user_messages, transcript.assistant_messages
                    )
                })
                .unwrap_or_else(|| "preview unavailable".to_string()),
        },
        SessionMetadataEntry {
            label: "Updated",
            value: file_snapshot
                .updated_at
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        },
    ];
    if let Some(transcript) = &transcript {
        for entry in &transcript.metadata_entries {
            upsert_session_metadata(&mut preview_summary, entry.label, entry.value.clone());
        }
    }
    let preview = SessionPreview {
        summary: preview_summary,
        blocks: transcript
            .as_ref()
            .map(|transcript| transcript.blocks.clone())
            .filter(|blocks| !blocks.is_empty())
            .unwrap_or_else(|| {
                vec![
                    SessionPreviewBlock {
                        role: "USER",
                        timestamp: started_at.clone(),
                        tone: PreviewTone::User,
                        folded: false,
                        lines: vec![
                            if kind == SessionKind::Document {
                                format!("Open document {title}.")
                            } else {
                                format!("Resume Codex session {session_id}.")
                            },
                            if kind == SessionKind::Document {
                                "Documents live beside sessions in the same fast tree model.".to_string()
                            } else {
                                format!("Open the workspace rooted at {cwd}.")
                            },
                        ],
                    },
                    SessionPreviewBlock {
                        role: "ASSISTANT",
                        timestamp: "server:restore".to_string(),
                        tone: PreviewTone::Assistant,
                        folded: false,
                        lines: vec![
                            if kind == SessionKind::Document {
                                "Preview mode renders document content immediately from the local workspace store.".to_string()
                            } else {
                                format!("{backend_label} backend reserved for the live terminal surface.")
                            },
                            if kind == SessionKind::Document {
                                "Terminal mode is disabled for document nodes.".to_string()
                            } else {
                                "Rendered preview follows the session transcript first and tool activity second.".to_string()
                            },
                            if kind == SessionKind::Document {
                                format!("Document path: {path}")
                            } else {
                format!("Terminal launch command: {launch_command}")
                            },
                        ],
                    },
                ]
            }),
    };

    let mut metadata = vec![
        SessionMetadataEntry {
            label: "Source",
            value: if kind == SessionKind::Document {
                "document".to_string()
            } else {
                "stored".to_string()
            },
        },
        SessionMetadataEntry {
            label: "Host",
            value: host_label.clone(),
        },
        SessionMetadataEntry {
            label: if kind == SessionKind::Document {
                "Document"
            } else {
                "Session"
            },
            value: session_id.clone(),
        },
        SessionMetadataEntry {
            label: "Storage",
            value: path.to_string(),
        },
        SessionMetadataEntry {
            label: "Cwd",
            value: cwd.clone(),
        },
        SessionMetadataEntry {
            label: "Started",
            value: started_at.clone(),
        },
        SessionMetadataEntry {
            label: "Updated",
            value: file_snapshot
                .updated_at
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        },
        SessionMetadataEntry {
            label: "Bytes",
            value: file_snapshot
                .bytes
                .map(|bytes| bytes.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        },
        SessionMetadataEntry {
            label: "Messages",
            value: message_count.to_string(),
        },
        SessionMetadataEntry {
            label: "Preview Blocks",
            value: preview_block_count.to_string(),
        },
        SessionMetadataEntry {
            label: "Backend",
            value: backend_label.to_string(),
        },
        SessionMetadataEntry {
            label: "Launch PID",
            value: "not launched".to_string(),
        },
        SessionMetadataEntry {
            label: "Launch Error",
            value: "none".to_string(),
        },
        SessionMetadataEntry {
            label: "Status",
            value: "queued".to_string(),
        },
        SessionMetadataEntry {
            label: "Bridge",
            value: "daemon pty".to_string(),
        },
        SessionMetadataEntry {
            label: "Theme",
            value: appearance.to_string(),
        },
        SessionMetadataEntry {
            label: "Restore",
            value: launch_command.clone(),
        },
    ];
    if let Some(transcript) = &transcript {
        for entry in &transcript.metadata_entries {
            upsert_session_metadata(&mut metadata, entry.label, entry.value.clone());
        }
    }

    ManagedSessionView {
        id: session_id.clone(),
        session_path: path.to_string(),
        title: title.clone(),
        kind,
        host_label: host_label.clone(),
        source: SessionSource::Stored,
        backend,
        bridge_available: true,
        launch_phase: TerminalLaunchPhase::Queued,
        remote_deploy_state: RemoteDeployState::NotRequired,
        launch_command: launch_command.clone(),
        status_line: describe_status_line(
            backend,
            theme,
            SessionSource::Stored,
            TerminalLaunchPhase::Queued,
            RemoteDeployState::NotRequired,
            true,
        ),
        terminal_lines: vec![
            if kind == SessionKind::Document {
                "Document nodes do not launch a terminal surface.".to_string()
            } else {
                format!("$ cd {cwd}")
            },
            if kind == SessionKind::Document {
                format!("Open {title} in preview mode.")
            } else {
                format!("$ {launch_command}")
            },
            format!("Terminal host: {backend_label}"),
            if kind == SessionKind::Document {
                "Use yggterm doc write <path> to update this note from the CLI.".to_string()
            } else {
                "yggterm server launches an in-process PTY for terminal mode".to_string()
            },
            if kind == SessionKind::Document {
                "Documents stay render-first and load from ~/.yggterm/workspace.db.".to_string()
            } else {
                "xterm.js renders the active PTY directly in the main viewport.".to_string()
            },
            embedded_surface_note(ghostty_bridge_enabled),
        ],
        rendered_sections: vec![
            SessionRenderedSection {
                title: "Rendered Session",
                lines: vec![
                    if kind == SessionKind::Document {
                        "Preview mode renders the document body in place beside your session tree.".to_string()
                    } else {
                        "Preview mode renders the stored Codex transcript as a chat surface.".to_string()
                    },
                    if kind == SessionKind::Document {
                        "Use the CLI to create or edit notes quickly without slowing down startup.".to_string()
                    } else {
                        "Turn Preview off in the titlebar to hand the main viewport back to the embedded terminal.".to_string()
                    },
                    if kind == SessionKind::Document {
                        "This gives each problem space room for both sessions and nearby notes.".to_string()
                    } else {
                        "The terminal/server session stays authoritative underneath.".to_string()
                    },
                ],
            },
            SessionRenderedSection {
                title: "Server Notes",
                lines: vec![
                    "GUI selection asks the Yggterm server to open or focus the session.".to_string(),
                    "The server model is where restore, multiplexing, and remote orchestration live.".to_string(),
                ],
            },
        ],
        preview,
        metadata,
        terminal_process_id: None,
        terminal_window_id: None,
        terminal_host_token: None,
        terminal_host_mode: ghostty_host_mode(backend, ghostty_bridge_enabled),
        embedded_surface_id: None,
        embedded_surface_detail: None,
        last_launch_error: None,
        last_window_error: None,
        ssh_target: None,
        ssh_prefix: None,
        stored_preview_hydrated: should_hydrate_stored_preview,
    }
}

fn build_live_session(
    uuid: &str,
    kind: SessionKind,
    target: &SshConnectTarget,
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
) -> ManagedSessionView {
    let started_at = format_display_datetime(OffsetDateTime::now_utc());
    let shell_program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let default_cwd = target.cwd.clone().unwrap_or_else(|| local_default_cwd());
    let (launch_command, session_path, source_value, target_value, prefix_value, deploy_state) =
        match kind {
            SessionKind::Shell => (
                format!("exec {} -i", shell_single_quote(&shell_program)),
                format!("local://{uuid}"),
                "local-shell".to_string(),
                default_cwd.clone(),
                shell_program.clone(),
                RemoteDeployState::NotRequired,
            ),
            SessionKind::Codex => (
                agent_launch_command(SessionKind::Codex, Some(&default_cwd), None),
                format!("codex://{uuid}"),
                "local-codex".to_string(),
                default_cwd.clone(),
                "codex".to_string(),
                RemoteDeployState::NotRequired,
            ),
            SessionKind::CodexLiteLlm => (
                agent_launch_command(SessionKind::CodexLiteLlm, Some(&default_cwd), None),
                format!("codex-litellm://{uuid}"),
                "local-codex-litellm".to_string(),
                default_cwd.clone(),
                "codex-litellm".to_string(),
                RemoteDeployState::NotRequired,
            ),
            SessionKind::SshShell => (
                remote_ssh_launch_command(
                    &target.ssh_target,
                    target.prefix.as_deref(),
                    "yggterm",
                    &["server", "attach", uuid, default_cwd.as_str()],
                ),
                format!("ssh://{}/{}", target.ssh_target, uuid),
                "live-ssh".to_string(),
                target.ssh_target.clone(),
                target.prefix.clone().unwrap_or_else(|| "none".to_string()),
                RemoteDeployState::Planned,
            ),
            SessionKind::Document => (
                "document preview".to_string(),
                format!("document://{uuid}"),
                "document".to_string(),
                "document".to_string(),
                "none".to_string(),
                RemoteDeployState::NotRequired,
            ),
        };
    let preview_intro = match kind {
        SessionKind::Shell => {
            "This local shell should stay alive in the daemon while you browse elsewhere.".to_string()
        }
        SessionKind::Codex => {
            "This Codex session stays attached to the daemon and opens inline in the main terminal viewport.".to_string()
        }
        SessionKind::CodexLiteLlm => {
            "This Codex LiteLLM session uses the configured LiteLLM-friendly CLI path and stays attached to the daemon.".to_string()
        }
        SessionKind::SshShell => {
            "This session should land in the main viewport as an embedded xterm.js terminal.".to_string()
        }
        SessionKind::Document => "Documents stay in preview mode only.".to_string(),
    };
    let preview_runtime = match kind {
        SessionKind::SshShell => {
            "Remote Yggterm bootstrap installs or reuses a matching server binary before attach.".to_string()
        }
        SessionKind::Shell => {
            "This local shell uses the same PTY/runtime path as other embedded terminals.".to_string()
        }
        SessionKind::Codex => {
            "Codex is launched locally and will receive /quit when the daemon shuts down.".to_string()
        }
        SessionKind::CodexLiteLlm => {
            "Codex LiteLLM is launched locally with a dedicated CODEX_HOME and will receive /quit on shutdown.".to_string()
        }
        SessionKind::Document => "No terminal runtime is required.".to_string(),
    };
    let default_summary = live_session_default_summary(kind, target);

    ManagedSessionView {
        id: uuid.to_string(),
        session_path,
        title: uuid.to_string(),
        kind,
        host_label: target.label.clone(),
        source: SessionSource::LiveSsh,
        backend,
        bridge_available: ghostty_bridge_enabled,
        launch_phase: TerminalLaunchPhase::RemoteBootstrap,
        remote_deploy_state: deploy_state,
        launch_command: launch_command.clone(),
        status_line: describe_status_line(
            backend,
            theme,
            SessionSource::LiveSsh,
            TerminalLaunchPhase::RemoteBootstrap,
            deploy_state,
            ghostty_bridge_enabled,
        ),
        terminal_lines: vec![
            format!("$ {launch_command}"),
            format!("Queue live {} session {uuid}", session_kind_label(kind)),
            format!("Target: {target_value}"),
            format!("Command: {prefix_value}"),
            if kind == SessionKind::SshShell {
                "Remote bootstrap: copy yggterm binary if missing".to_string()
            } else {
                "Daemon runtime: local PTY managed directly by yggterm".to_string()
            },
            "Daemon PTY: request main viewport terminal stream".to_string(),
        ],
        rendered_sections: vec![],
        preview: SessionPreview {
            summary: vec![
                SessionMetadataEntry {
                    label: "UUID",
                    value: uuid.to_string(),
                },
                SessionMetadataEntry {
                    label: "Target",
                    value: target_value.clone(),
                },
                SessionMetadataEntry {
                    label: "Prefix",
                    value: prefix_value.clone(),
                },
                SessionMetadataEntry {
                    label: "Started",
                    value: started_at.clone(),
                },
                SessionMetadataEntry {
                    label: "Bridge",
                    value: if ghostty_bridge_enabled {
                        "available".to_string()
                    } else {
                        "not linked".to_string()
                    },
                },
                SessionMetadataEntry {
                    label: "Cwd",
                    value: default_cwd.clone(),
                },
                SessionMetadataEntry {
                    label: "Summary",
                    value: default_summary.clone(),
                },
            ],
            blocks: vec![
                SessionPreviewBlock {
                    role: "USER",
                    timestamp: started_at.clone(),
                    tone: PreviewTone::User,
                    folded: false,
                    lines: vec![
                        format!("Open live terminal {uuid} through the Yggterm server."),
                        preview_intro,
                    ],
                },
                SessionPreviewBlock {
                    role: "ASSISTANT",
                    timestamp: "server:launch".to_string(),
                    tone: PreviewTone::Assistant,
                    folded: false,
                    lines: vec![
                        format!("Launch command prepared: {launch_command}"),
                        preview_runtime,
                    ],
                },
            ],
        },
        metadata: vec![
            SessionMetadataEntry {
                label: "Source",
                value: source_value,
            },
            SessionMetadataEntry {
                label: "Host",
                value: target.label.clone(),
            },
            SessionMetadataEntry {
                label: "UUID",
                value: uuid.to_string(),
            },
            SessionMetadataEntry {
                label: "Target",
                value: target_value,
            },
            SessionMetadataEntry {
                label: "Cwd",
                value: default_cwd,
            },
            SessionMetadataEntry {
                label: "Prefix",
                value: prefix_value,
            },
            SessionMetadataEntry {
                label: "Deploy",
                value: match deploy_state {
                    RemoteDeployState::NotRequired => "not required".to_string(),
                    RemoteDeployState::Planned => "planned".to_string(),
                    RemoteDeployState::CopyingBinary => "copying".to_string(),
                    RemoteDeployState::Ready => "ready".to_string(),
                },
            },
            SessionMetadataEntry {
                label: "Launch PID",
                value: if kind == SessionKind::SshShell {
                    "remote".to_string()
                } else {
                    "daemon pty".to_string()
                },
            },
            SessionMetadataEntry {
                label: "Launch Error",
                value: "none".to_string(),
            },
            SessionMetadataEntry {
                label: "Status",
                value: "planned".to_string(),
            },
            SessionMetadataEntry {
                label: "Summary",
                value: default_summary,
            },
            SessionMetadataEntry {
                label: "Launch",
                value: launch_command,
            },
        ],
        terminal_process_id: None,
        terminal_window_id: None,
        terminal_host_token: None,
        terminal_host_mode: ghostty_host_mode(backend, ghostty_bridge_enabled),
        embedded_surface_id: None,
        embedded_surface_detail: None,
        last_launch_error: None,
        last_window_error: None,
        ssh_target: Some(target.ssh_target.clone()),
        ssh_prefix: target.prefix.clone(),
        stored_preview_hydrated: true,
    }
}

fn synthesize_remote_scanned_session_view(
    machine: &RemoteMachineSnapshot,
    scanned: &RemoteScannedSession,
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
) -> ManagedSessionView {
    let target = SshConnectTarget {
        label: machine.label.clone(),
        kind: SessionKind::SshShell,
        ssh_target: machine.ssh_target.clone(),
        prefix: machine.prefix.clone(),
        cwd: Some(scanned.cwd.clone()),
    };
    let mut session = build_live_session(
        &scanned.session_id,
        SessionKind::SshShell,
        &target,
        backend,
        theme,
        ghostty_bridge_enabled,
    );
    let (remote_binary, remote_deploy_state) = machine
        .remote_binary_expr
        .clone()
        .map(|binary_expr| (binary_expr, machine.remote_deploy_state))
        .unwrap_or_else(|| {
            (
                preferred_remote_binary_fallback(),
                machine.remote_deploy_state,
            )
        });
    session.remote_deploy_state = remote_deploy_state;
    session.session_path = scanned.session_path.clone();
    session.host_label = machine.label.clone();
    session.title = if scanned.title_hint.trim().is_empty() {
        short_session_id(&scanned.session_id)
    } else {
        scanned.title_hint.clone()
    };
    session.launch_command = remote_ssh_launch_command(
        &target.ssh_target,
        target.prefix.as_deref(),
        &remote_binary,
        &[
            "server",
            "remote",
            "resume-codex",
            &scanned.session_id,
            &scanned.cwd,
        ],
    );
    session.terminal_lines = vec![
        format!("$ {}", session.launch_command),
        format!("Queue remote Yggterm resume {}", scanned.session_id),
        format!("Target host: {}", target.ssh_target),
        format!("Workspace: {}", scanned.cwd),
        "Daemon PTY: request main viewport terminal stream".to_string(),
    ];
    upsert_session_metadata(&mut session.metadata, "Source", "remote-codex".to_string());
    upsert_session_metadata(&mut session.metadata, "Host", target.ssh_target.clone());
    upsert_session_metadata(&mut session.metadata, "UUID", scanned.session_id.clone());
    upsert_session_metadata(
        &mut session.metadata,
        "Restore",
        format!("yggterm server remote resume-codex {}", scanned.session_id),
    );
    upsert_session_metadata(
        &mut session.metadata,
        "Deploy",
        match remote_deploy_state {
            RemoteDeployState::Ready => "ready".to_string(),
            RemoteDeployState::CopyingBinary => "copying".to_string(),
            RemoteDeployState::Planned => "planned".to_string(),
            RemoteDeployState::NotRequired => "not required".to_string(),
        },
    );
    upsert_session_metadata(
        &mut session.metadata,
        "Status",
        format!(
            "remote resume queued · {}",
            match remote_deploy_state {
                RemoteDeployState::Ready => "remote yggterm ready",
                RemoteDeployState::CopyingBinary => "copying yggterm binary",
                RemoteDeployState::Planned => "remote bootstrap planned",
                RemoteDeployState::NotRequired => "not required",
            }
        ),
    );
    upsert_session_metadata(
        &mut session.metadata,
        "Launch",
        session.launch_command.clone(),
    );
    upsert_session_metadata(&mut session.metadata, "Cwd", scanned.cwd.clone());
    apply_remote_scanned_session_preview(&mut session, scanned, &machine.label, &target.ssh_target);
    session
}

fn synthesize_remote_active_session(
    active_path: &str,
    remote_machines: &[RemoteMachineSnapshot],
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
) -> Option<ManagedSessionView> {
    let (machine_key, session_id) = parse_remote_scanned_session_path(active_path)?;
    let machine_key = normalize_machine_key(machine_key);
    let machine = remote_machines
        .iter()
        .find(|machine| machine.machine_key == machine_key)?;
    let scanned = machine
        .sessions
        .iter()
        .find(|session| session.session_path == active_path || session.session_id == session_id)?;
    Some(synthesize_remote_scanned_session_view(
        machine,
        scanned,
        backend,
        theme,
        ghostty_bridge_enabled,
    ))
}

fn hydrate_document_session(session: &mut ManagedSessionView, document: &WorkspaceDocument) {
    session.kind = SessionKind::Document;
    session.id = document.id.clone();
    session.title = document.title.clone();
    session.session_path = document.virtual_path.clone();
    session.launch_command = "document preview".to_string();
    let kind_label = match document.kind {
        WorkspaceDocumentKind::Note => "note",
        WorkspaceDocumentKind::TerminalRecipe => "terminal recipe",
    };
    let mut preview_blocks = vec![SessionPreviewBlock {
        role: "NOTE",
        timestamp: document.updated_at.clone(),
        tone: PreviewTone::Assistant,
        folded: false,
        lines: document.body.lines().map(ToOwned::to_owned).collect(),
    }];
    if !document.replay_commands.is_empty() {
        preview_blocks.push(SessionPreviewBlock {
            role: "REPLAY",
            timestamp: "document:replay".to_string(),
            tone: PreviewTone::User,
            folded: false,
            lines: document.replay_commands.clone(),
        });
    }
    session.preview = SessionPreview {
        summary: vec![
            SessionMetadataEntry {
                label: "Document",
                value: document.title.clone(),
            },
            SessionMetadataEntry {
                label: "Kind",
                value: kind_label.to_string(),
            },
            SessionMetadataEntry {
                label: "Storage",
                value: document.virtual_path.clone(),
            },
            SessionMetadataEntry {
                label: "Updated",
                value: document.updated_at.clone(),
            },
            SessionMetadataEntry {
                label: "Replay",
                value: if document.replay_commands.is_empty() {
                    "none".to_string()
                } else {
                    format!("{} commands", document.replay_commands.len())
                },
            },
        ],
        blocks: preview_blocks,
    };
    session.rendered_sections = vec![SessionRenderedSection {
        title: "Document",
        lines: document.body.lines().map(ToOwned::to_owned).collect(),
    }];
    if !document.replay_commands.is_empty() {
        session.rendered_sections.push(SessionRenderedSection {
            title: "Replay Commands",
            lines: document.replay_commands.clone(),
        });
    }
    session.metadata.retain(|entry| {
        !matches!(
            entry.label,
            "Session"
                | "Document"
                | "Kind"
                | "Storage"
                | "Updated"
                | "Replay"
                | "Source Session"
                | "Source Kind"
                | "Status"
                | "Restore"
                | "Launch"
        )
    });
    upsert_session_metadata(&mut session.metadata, "Source", "document".to_string());
    upsert_session_metadata(&mut session.metadata, "Document", document.title.clone());
    upsert_session_metadata(&mut session.metadata, "Kind", kind_label.to_string());
    upsert_session_metadata(
        &mut session.metadata,
        "Storage",
        document.virtual_path.clone(),
    );
    upsert_session_metadata(
        &mut session.metadata,
        "Updated",
        document.updated_at.clone(),
    );
    if let Some(source_session_path) = document.source_session_path.clone() {
        upsert_session_metadata(&mut session.metadata, "Source Session", source_session_path);
    }
    if let Some(source_session_kind) = document.source_session_kind.clone() {
        upsert_session_metadata(&mut session.metadata, "Source Kind", source_session_kind);
    }
    if let Some(source_session_cwd) = document.source_session_cwd.clone() {
        upsert_session_metadata(&mut session.metadata, "Source Cwd", source_session_cwd);
    }
    upsert_session_metadata(
        &mut session.metadata,
        "Replay",
        if document.replay_commands.is_empty() {
            "none".to_string()
        } else {
            format!("{} commands", document.replay_commands.len())
        },
    );
    upsert_session_metadata(&mut session.metadata, "Status", "preview only".to_string());
    session.terminal_lines = vec![
        format!("Document {}", document.title),
        match document.kind {
            WorkspaceDocumentKind::TerminalRecipe => {
                "Use Terminal view or Run Recipe to execute these commands in a daemon-owned PTY."
                    .to_string()
            }
            WorkspaceDocumentKind::Note => {
                "Terminal mode is disabled for document nodes.".to_string()
            }
        },
        "Use yggterm doc write <path> to update this note from the CLI.".to_string(),
    ];
}

fn recipe_terminal_spec(session: &ManagedSessionView) -> Option<(String, Option<String>)> {
    if metadata_value(session, "Kind") != "terminal recipe" {
        return None;
    }
    let commands = session
        .rendered_sections
        .iter()
        .find(|section| section.title == "Replay Commands")
        .map(|section| {
            section
                .lines
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if commands.is_empty() {
        return None;
    }
    let cwd = metadata_value(session, "Source Cwd");
    Some((
        commands.join("\n"),
        if cwd.is_empty() { None } else { Some(cwd) },
    ))
}

fn describe_status_line(
    backend: TerminalBackend,
    theme: UiTheme,
    source: SessionSource,
    launch_phase: TerminalLaunchPhase,
    remote_deploy_state: RemoteDeployState,
    bridge_available: bool,
) -> String {
    let backend_label = match backend {
        TerminalBackend::Xterm => "xterm.js",
        TerminalBackend::Ghostty => "Ghostty",
        TerminalBackend::Mock => "Mock",
    };
    let appearance = match theme {
        UiTheme::ZedDark => "dark",
        UiTheme::ZedLight => "light",
    };
    let launch_status =
        describe_launch_phase(source, launch_phase, remote_deploy_state, bridge_available);
    format!("{backend_label} · {appearance} scheme requested · {launch_status}")
}

fn describe_launch_phase(
    source: SessionSource,
    launch_phase: TerminalLaunchPhase,
    remote_deploy_state: RemoteDeployState,
    bridge_available: bool,
) -> String {
    let runtime = if bridge_available {
        "daemon ready"
    } else {
        "runtime degraded"
    };
    match (source, launch_phase, remote_deploy_state) {
        (SessionSource::Stored, TerminalLaunchPhase::Queued, _) => {
            format!("stored attach queued · {runtime}")
        }
        (SessionSource::Stored, TerminalLaunchPhase::BridgePending, _) => {
            format!("stored attach pending terminal host · {runtime}")
        }
        (SessionSource::Stored, TerminalLaunchPhase::Running, _) => {
            format!("embedded terminal attached · {runtime}")
        }
        (SessionSource::LiveSsh, _, RemoteDeployState::Planned) => {
            format!("remote bootstrap planned · {runtime}")
        }
        (SessionSource::LiveSsh, _, RemoteDeployState::CopyingBinary) => {
            format!("copying yggterm binary · {runtime}")
        }
        (SessionSource::LiveSsh, TerminalLaunchPhase::Running, RemoteDeployState::NotRequired) => {
            format!("live terminal attached · {runtime}")
        }
        (SessionSource::LiveSsh, TerminalLaunchPhase::Running, RemoteDeployState::Ready) => {
            format!("remote terminal attached · {runtime}")
        }
        (SessionSource::LiveSsh, TerminalLaunchPhase::BridgePending, _) => {
            format!("waiting for terminal host · {runtime}")
        }
        _ => format!("launch queued · {runtime}"),
    }
}

fn build_live_terminal_lines(session: &ManagedSessionView) -> Vec<String> {
    let deploy = match session.remote_deploy_state {
        RemoteDeployState::NotRequired => "not required",
        RemoteDeployState::Planned => "planned",
        RemoteDeployState::CopyingBinary => "copying binary",
        RemoteDeployState::Ready => "ready",
    };
    let launch = match session.launch_phase {
        TerminalLaunchPhase::Queued => "queued",
        TerminalLaunchPhase::BridgePending => "bridge pending",
        TerminalLaunchPhase::RemoteBootstrap => "remote bootstrap",
        TerminalLaunchPhase::Running => "running",
    };
    vec![
        format!("$ {}", session.launch_command),
        format!(
            "Launching live {} session {}",
            session_kind_label(session.kind),
            session.id
        ),
        format!(
            "{}: {}",
            if matches!(
                session.kind,
                SessionKind::Shell | SessionKind::Codex | SessionKind::CodexLiteLlm
            ) {
                "Workspace"
            } else {
                "Target"
            },
            session.host_label
        ),
        format!("Deploy state: {deploy}"),
        format!("Launch phase: {launch}"),
        "Terminal surface: embedded xterm.js".to_string(),
    ]
}

fn stored_file_snapshot(path: &str) -> StoredFileSnapshot {
    let metadata = fs::metadata(path).ok();
    StoredFileSnapshot {
        updated_at: metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
            .map(format_system_time),
        bytes: metadata.as_ref().map(|metadata| metadata.len()),
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn legacy_agent_launch_command(
    kind: SessionKind,
    cwd: Option<&str>,
    session_id: Option<&str>,
) -> String {
    match (kind, session_id) {
        (SessionKind::Codex, Some(session_id)) => match cwd.filter(|cwd| !cwd.trim().is_empty()) {
            Some(cwd) => format!(
                "cd {} && codex resume {}",
                shell_single_quote(cwd),
                shell_single_quote(session_id)
            ),
            None => format!("codex resume {}", shell_single_quote(session_id)),
        },
        (SessionKind::CodexLiteLlm, Some(session_id)) => {
            match cwd.filter(|cwd| !cwd.trim().is_empty()) {
                Some(cwd) => format!(
                    "cd {} && CODEX_HOME=\"$HOME/.codex-litellm\" codex-litellm resume {}",
                    shell_single_quote(cwd),
                    shell_single_quote(session_id)
                ),
                None => format!(
                    "CODEX_HOME=\"$HOME/.codex-litellm\" codex-litellm resume {}",
                    shell_single_quote(session_id)
                ),
            }
        }
        (SessionKind::Codex, None) => match cwd.filter(|cwd| !cwd.trim().is_empty()) {
            Some(cwd) => format!("cd {} && codex", shell_single_quote(cwd)),
            None => "codex".to_string(),
        },
        (SessionKind::CodexLiteLlm, None) => match cwd.filter(|cwd| !cwd.trim().is_empty()) {
            Some(cwd) => format!(
                "cd {} && CODEX_HOME=\"$HOME/.codex-litellm\" codex-litellm",
                shell_single_quote(cwd)
            ),
            None => "CODEX_HOME=\"$HOME/.codex-litellm\" codex-litellm".to_string(),
        },
        _ => String::new(),
    }
}

fn agent_launch_command(kind: SessionKind, cwd: Option<&str>, session_id: Option<&str>) -> String {
    let action = session_id.map(|session_id| ManagedCliAction::Resume {
        session_id,
        persistent: false,
    });
    let managed = match action {
        Some(action) => managed_cli_shell_command(kind, cwd, action),
        None => managed_cli_shell_command(kind, cwd, ManagedCliAction::Launch),
    };
    managed.unwrap_or_else(|_| legacy_agent_launch_command(kind, cwd, session_id))
}

fn persistent_agent_resume_command(
    kind: SessionKind,
    cwd: Option<&str>,
    session_id: &str,
) -> String {
    managed_cli_shell_command(
        kind,
        cwd,
        ManagedCliAction::Resume {
            session_id,
            persistent: true,
        },
    )
    .unwrap_or_else(|_| legacy_agent_launch_command(kind, cwd, Some(session_id)))
}

fn stored_session_launch_command(kind: SessionKind, cwd: &str, session_id: &str) -> String {
    match kind {
        SessionKind::Codex | SessionKind::CodexLiteLlm => {
            agent_launch_command(kind, Some(cwd), Some(session_id))
        }
        SessionKind::Document => "document preview".to_string(),
        SessionKind::Shell | SessionKind::SshShell => format!(
            "cd {} && codex resume {}",
            shell_single_quote(cwd),
            session_id
        ),
    }
}

fn local_default_cwd() -> String {
    dirs::home_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "/".to_string())
}

fn local_session_target(kind: SessionKind, cwd: Option<&str>) -> SshConnectTarget {
    let cwd = Some(cwd.map(ToOwned::to_owned).unwrap_or_else(local_default_cwd));
    match kind {
        SessionKind::Codex => SshConnectTarget {
            label: "codex".to_string(),
            kind,
            ssh_target: "localhost".to_string(),
            prefix: None,
            cwd,
        },
        SessionKind::CodexLiteLlm => SshConnectTarget {
            label: "codex-litellm".to_string(),
            kind,
            ssh_target: "localhost".to_string(),
            prefix: None,
            cwd,
        },
        SessionKind::Shell => SshConnectTarget {
            label: "local-shell".to_string(),
            kind,
            ssh_target: "localhost".to_string(),
            prefix: None,
            cwd,
        },
        SessionKind::SshShell => SshConnectTarget {
            label: "ssh-shell".to_string(),
            kind,
            ssh_target: "localhost".to_string(),
            prefix: None,
            cwd,
        },
        SessionKind::Document => SshConnectTarget {
            label: "document".to_string(),
            kind,
            ssh_target: "localhost".to_string(),
            prefix: None,
            cwd: None,
        },
    }
}

fn session_kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Codex => "codex",
        SessionKind::CodexLiteLlm => "codex-litellm",
        SessionKind::Shell => "shell",
        SessionKind::SshShell => "ssh",
        SessionKind::Document => "document",
    }
}

fn default_persisted_stored_kind() -> SessionKind {
    SessionKind::Codex
}

fn default_persisted_live_kind() -> SessionKind {
    SessionKind::SshShell
}

fn embedded_surface_note(bridge_available: bool) -> String {
    if !bridge_available {
        "The active terminal path is now PTY + xterm.js, so no external host bridge is required."
            .to_string()
    } else {
        "Ghostty integration is optional now; the active viewport uses xterm.js for embedded terminals."
            .to_string()
    }
}

fn ghostty_host_mode(
    backend: TerminalBackend,
    ghostty_bridge_enabled: bool,
) -> GhosttyTerminalHostMode {
    if backend != TerminalBackend::Ghostty || !ghostty_bridge_enabled {
        GhosttyTerminalHostMode::Unsupported
    } else if cfg!(target_os = "macos") {
        GhosttyTerminalHostMode::EmbeddedSurface
    } else if cfg!(target_os = "linux") {
        GhosttyTerminalHostMode::ControlledDock
    } else {
        GhosttyTerminalHostMode::ExternalWindow
    }
}

fn upsert_session_metadata(
    metadata: &mut Vec<SessionMetadataEntry>,
    label: &'static str,
    value: String,
) {
    if let Some(entry) = metadata.iter_mut().find(|entry| entry.label == label) {
        entry.value = value;
    } else {
        metadata.push(SessionMetadataEntry { label, value });
    }
}

fn metadata_value(session: &ManagedSessionView, label: &str) -> String {
    session
        .metadata
        .iter()
        .find(|entry| entry.label == label)
        .map(|entry| entry.value.clone())
        .unwrap_or_default()
}

fn format_system_time(time: SystemTime) -> String {
    let datetime: OffsetDateTime = time.into();
    format_display_datetime(datetime)
}

fn parse_stored_transcript(path: &str, fallback_started_at: &str) -> Option<StoredTranscript> {
    let mut started_at = None;
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut metadata_entries = Vec::new();
    let mut blocks = Vec::new();

    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("session_meta")
            && let Some(payload) = value.get("payload")
        {
            started_at = payload
                .get("timestamp")
                .and_then(Value::as_str)
                .map(parse_and_format_timestamp)
                .or(started_at);
        }
    }

    for message in read_codex_transcript_messages(std::path::Path::new(path)).ok()? {
        let timestamp = message
            .timestamp
            .as_deref()
            .map(parse_and_format_timestamp)
            .unwrap_or_else(|| {
                started_at
                    .clone()
                    .unwrap_or_else(|| fallback_started_at.to_string())
            });
        push_preview_block(
            &mut blocks,
            &mut metadata_entries,
            &mut user_messages,
            &mut assistant_messages,
            message.role,
            message.lines,
            timestamp,
        );
    }

    Some(StoredTranscript {
        started_at: started_at.unwrap_or_else(|| fallback_started_at.to_string()),
        user_messages,
        assistant_messages,
        metadata_entries,
        blocks,
    })
}

fn push_preview_block(
    blocks: &mut Vec<SessionPreviewBlock>,
    metadata_entries: &mut Vec<SessionMetadataEntry>,
    user_messages: &mut usize,
    assistant_messages: &mut usize,
    role: TranscriptRole,
    lines: Vec<String>,
    timestamp: String,
) {
    let lines = match role {
        TranscriptRole::System => Vec::new(),
        TranscriptRole::User | TranscriptRole::Assistant => sanitize_preview_lines(lines),
    };
    if lines.is_empty() {
        return;
    }
    if role == TranscriptRole::Assistant
        && blocks.is_empty()
        && looks_like_session_metadata_block(&lines)
    {
        *metadata_entries = parse_session_metadata_lines(&lines);
        return;
    }

    match role {
        TranscriptRole::User => *user_messages += 1,
        TranscriptRole::Assistant => *assistant_messages += 1,
        TranscriptRole::System => {}
    }

    blocks.push(SessionPreviewBlock {
        role: session_role_label(role),
        timestamp,
        tone: session_preview_tone(role),
        folded: false,
        lines,
    });
}

fn looks_like_session_metadata_block(lines: &[String]) -> bool {
    let known_prefixes = [
        "Session ",
        "Storage ",
        "Cwd ",
        "Started ",
        "Updated ",
        "Messages ",
        "Host ",
        "Source ",
        "Backend ",
    ];
    let matches = lines
        .iter()
        .filter(|line| known_prefixes.iter().any(|prefix| line.starts_with(prefix)))
        .count();
    matches >= 3
}

fn parse_session_metadata_lines(lines: &[String]) -> Vec<SessionMetadataEntry> {
    const KNOWN_PREFIXES: [(&str, &'static str); 9] = [
        ("Session ", "Session"),
        ("Storage ", "Storage"),
        ("Cwd ", "Cwd"),
        ("Started ", "Started"),
        ("Updated ", "Updated"),
        ("Messages ", "Messages"),
        ("Host ", "Host"),
        ("Source ", "Source"),
        ("Backend ", "Backend"),
    ];

    lines
        .iter()
        .filter_map(|line| {
            KNOWN_PREFIXES.iter().find_map(|(prefix, label)| {
                line.strip_prefix(prefix).map(|value| SessionMetadataEntry {
                    label,
                    value: value.trim().to_string(),
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod recipe_tests {
    use super::*;

    fn sample_session() -> ManagedSessionView {
        ManagedSessionView {
            id: "doc-1".to_string(),
            session_path: "/documents/recipes/demo".to_string(),
            title: "demo recipe".to_string(),
            kind: SessionKind::Document,
            host_label: "document".to_string(),
            source: SessionSource::Stored,
            backend: TerminalBackend::Xterm,
            bridge_available: false,
            launch_phase: TerminalLaunchPhase::Queued,
            remote_deploy_state: RemoteDeployState::NotRequired,
            launch_command: "document preview".to_string(),
            status_line: "preview only".to_string(),
            terminal_lines: Vec::new(),
            rendered_sections: vec![],
            preview: SessionPreview {
                summary: vec![],
                blocks: vec![],
            },
            metadata: vec![],
            terminal_process_id: None,
            terminal_window_id: None,
            terminal_host_token: None,
            terminal_host_mode: GhosttyTerminalHostMode::Unsupported,
            embedded_surface_id: None,
            embedded_surface_detail: None,
            last_launch_error: None,
            last_window_error: None,
            ssh_target: None,
            ssh_prefix: None,
            stored_preview_hydrated: true,
        }
    }

    #[test]
    fn recipe_terminal_spec_uses_replay_commands_and_cwd() {
        let mut session = sample_session();
        session.metadata.push(SessionMetadataEntry {
            label: "Kind",
            value: "terminal recipe".to_string(),
        });
        session.metadata.push(SessionMetadataEntry {
            label: "Source Cwd",
            value: "/tmp/demo".to_string(),
        });
        session.rendered_sections.push(SessionRenderedSection {
            title: "Replay Commands",
            lines: vec!["echo hello".to_string(), "pwd".to_string()],
        });

        let spec = recipe_terminal_spec(&session);
        assert_eq!(
            spec,
            Some(("echo hello\npwd".to_string(), Some("/tmp/demo".to_string())))
        );
    }

    #[test]
    fn note_documents_do_not_expose_terminal_spec() {
        let mut session = sample_session();
        session.metadata.push(SessionMetadataEntry {
            label: "Kind",
            value: "note".to_string(),
        });
        session.rendered_sections.push(SessionRenderedSection {
            title: "Replay Commands",
            lines: vec!["echo nope".to_string()],
        });

        assert_eq!(recipe_terminal_spec(&session), None);
    }
}

fn format_display_datetime(datetime: OffsetDateTime) -> String {
    const DISPLAY_FORMAT: &[time::format_description::FormatItem<'static>] = format_description!(
        "[month repr:short] [day], [year] [hour repr:12 padding:zero]:[minute] [period] UTC[offset_hour sign:mandatory][offset_minute]"
    );
    let ist_offset = UtcOffset::from_hms(5, 30, 0).unwrap_or(UtcOffset::UTC);

    datetime
        .to_offset(ist_offset)
        .format(DISPLAY_FORMAT)
        .unwrap_or_else(|_| "unknown".to_string())
}

fn parse_and_format_timestamp(value: &str) -> String {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map(format_display_datetime)
        .unwrap_or_else(|_| value.to_string())
}

fn session_role_label(role: TranscriptRole) -> &'static str {
    match role {
        TranscriptRole::User => "USER",
        TranscriptRole::Assistant => "ASSISTANT",
        TranscriptRole::System => "SYSTEM",
    }
}

fn session_preview_tone(role: TranscriptRole) -> PreviewTone {
    match role {
        TranscriptRole::User => PreviewTone::User,
        TranscriptRole::Assistant | TranscriptRole::System => PreviewTone::Assistant,
    }
}

fn session_preview_cwd(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let parent = trimmed
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or(trimmed);
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy().to_string();
        if parent == home {
            return String::from("~");
        }
        let with_slash = format!("{home}/");
        if let Some(rest) = parent.strip_prefix(&with_slash) {
            return format!("~/{rest}");
        }
    }
    parent.to_string()
}

fn short_session_id(session_id: &str) -> String {
    let compact = session_id
        .chars()
        .filter(|ch| *ch != '-')
        .collect::<String>();
    if compact.len() >= 7 {
        format!("Q{}", &compact[compact.len() - 7..])
    } else {
        session_id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GhosttyHostSupport, GhosttyTerminalHostMode, PersistedDaemonState, PersistedLiveSession,
        PersistedStoredSession, PreviewTone, REMOTE_OUTPUT_SENTINEL, RemoteCommandCacheEntry,
        RemoteDeployState, RemoteMachineHealth, RemoteMachineSnapshot, RemotePreviewPayload,
        RemoteScannedSession, SessionKind, SessionNode, SessionNodeKind, SessionSource,
        SnapshotPreview, SnapshotPreviewBlock, SnapshotRenderedSection, SnapshotSessionView,
        SshConnectTarget, StoredPreviewHydrationMode, TerminalBackend, TerminalLaunchPhase,
        UiTheme, WorkspaceViewMode, YggtermServer, apply_remote_preview_payload, build_session,
        current_millis_u64, dedupe_remote_scanned_sessions,
        load_remote_machine_sessions_from_mirror, managed_session_from_snapshot,
        mirror_remote_machine_sessions, parse_recent_context_sections, parse_screen_session_ref,
        parse_stored_transcript, push_preview_block, remote_cache_key, remote_command_cache,
        remote_resume_shell_command, remote_saved_codex_session_exists,
        remote_scan_roots_with_parents, remote_scanned_session_path, remote_ssh_launch_command,
        sanitize_recent_context_payload, session_metadata_value, should_fallback_to_python,
        stored_session_launch_command, strip_remote_payload_noise,
        synthesize_remote_scanned_session_view,
    };
    use anyhow::Result;
    use std::fs;
    use std::path::PathBuf;
    use yggterm_core::TranscriptRole;

    #[test]
    fn parse_stored_transcript_counts_compacted_replacement_history() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "yggterm-server-compacted-{}-{}.jsonl",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(
            &path,
            [
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"session_meta","payload":{"id":"orig","timestamp":"2026-03-20T10:00:00Z","cwd":"/tmp/x"}}"#,
                r#"{"timestamp":"2026-03-20T10:00:01Z","type":"compacted","payload":{"replacement_history":[{"role":"user","type":"message","content":[{"type":"input_text","text":"first prompt"}]},{"role":"assistant","type":"message","content":[{"type":"output_text","text":"first answer"}]},{"role":"assistant","type":"message","content":[{"type":"output_text","text":"second answer"}]}]}}"#,
                r#"{"timestamp":"2026-03-20T10:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"follow-up"}]}}"#,
            ]
            .join("\n"),
        )?;

        let transcript = parse_stored_transcript(path.to_string_lossy().as_ref(), "fallback")
            .expect("transcript");
        assert_eq!(transcript.user_messages, 2);
        assert_eq!(transcript.assistant_messages, 2);
        assert_eq!(transcript.blocks.len(), 4);
        assert_eq!(transcript.blocks[1].lines[0], "first answer");
        assert_eq!(transcript.blocks[2].lines[0], "second answer");

        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn stored_codex_litellm_sessions_use_litellm_resume_command() {
        let command = stored_session_launch_command(
            SessionKind::CodexLiteLlm,
            "/tmp/workspace",
            "019caa6f-b32c-7a73-b4d3-db83225663dc",
        );
        assert!(command.contains("export NPM_CONFIG_PREFIX="));
        assert!(command.contains("export CODEX_HOME="));
        assert!(command.contains("codex-litellm resume"));
    }

    #[test]
    fn remote_resume_shell_command_wraps_prefix_and_cwd() {
        let command = remote_resume_shell_command(
            "019caa6f-b32c-7a73-b4d3-db83225663dc",
            Some("/srv/workspace"),
            Some("tmux new-session -A -s yggterm"),
        );
        assert!(command.contains("tmux new-session -A -s yggterm &&"));
        assert!(command.contains("cd '/srv/workspace' && export NPM_CONFIG_PREFIX="));
        assert!(command.contains("codex resume"));
        assert!(command.contains("'019caa6f-b32c-7a73-b4d3-db83225663dc'"));
    }

    #[test]
    fn remote_ssh_launch_command_wraps_binary_args_and_prefix() {
        let command = remote_ssh_launch_command(
            "jojo",
            Some("tmux new-session -A -s yggterm"),
            "$HOME/.yggterm/bin/yggterm",
            &["server", "remote", "resume-codex", "abc123", "/srv/app"],
        );
        assert!(command.starts_with("mkdir -p $HOME/.yggterm/ssh-control >/dev/null 2>&1 && ssh "));
        assert!(command.contains("-o ControlMaster=auto"));
        assert!(command.contains("-o ControlPersist=60"));
        assert!(command.contains("-o ControlPath='$HOME/.yggterm/ssh-control/%C'"));
        assert!(command.contains("-tt jojo "));
        assert!(command.contains("tmux new-session -A -s yggterm &&"));
        assert!(command.contains("$HOME/.yggterm/bin/yggterm"));
        assert!(command.contains("'resume-codex'"));
        assert!(command.contains("'/srv/app'"));
    }

    #[test]
    fn strip_remote_payload_noise_discards_shell_preamble() {
        let raw = format!("alias chicago='ssh'\nwelcome\n{REMOTE_OUTPUT_SENTINEL}\n2.0.12\n");
        assert_eq!(strip_remote_payload_noise(raw.as_bytes()), "2.0.12");
    }

    #[test]
    fn parse_screen_session_ref_matches_named_session_suffix() {
        let listing = "There are screens on:\n\t4046775.yggterm-abc123\t(03/28/26 21:42:13)\t(Detached)\n1 Socket in /run/screen/S-pi.\n";
        assert_eq!(
            parse_screen_session_ref(listing, "yggterm-abc123"),
            Some("4046775.yggterm-abc123".to_string())
        );
    }

    #[test]
    fn reopening_remote_scanned_session_refreshes_stale_launch_command() -> Result<()> {
        if let Ok(mut cache) = remote_command_cache().lock() {
            cache.insert(
                remote_cache_key("jojo", None),
                RemoteCommandCacheEntry {
                    binary_expr: "yggterm".to_string(),
                    verified_at_ms: current_millis_u64(),
                },
            );
        }
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "root".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.remote_machines.push(RemoteMachineSnapshot {
            machine_key: "jojo".to_string(),
            label: "jojo".to_string(),
            ssh_target: "jojo".to_string(),
            prefix: None,
            remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: Vec::new(),
        });

        let session_path = server.open_remote_scanned_session(
            "jojo",
            "019d09a4-c69e-7071-bd9a-8834060029a9",
            Some("/home/pi"),
            Some("Q60029a9"),
        )?;
        server
            .sessions
            .get_mut(&session_path)
            .expect("session")
            .launch_command =
            "ssh jojo 'yggterm server attach 019d09a4-c69e-7071-bd9a-8834060029a9'".to_string();

        let reopened = server.open_remote_scanned_session(
            "jojo",
            "019d09a4-c69e-7071-bd9a-8834060029a9",
            Some("/home/pi"),
            Some("Q60029a9"),
        )?;
        let session = server.sessions.get(&reopened).expect("reopened session");
        assert_eq!(reopened, session_path);
        assert!(session.launch_command.starts_with("ssh -tt jojo "));
        assert!(session.launch_command.contains("server"));
        assert!(session.launch_command.contains("resume-codex"));
        if let Ok(mut cache) = remote_command_cache().lock() {
            cache.remove(&remote_cache_key("jojo", None));
        }
        Ok(())
    }

    #[test]
    fn remote_metadata_mirror_round_trips_sessions() -> Result<()> {
        let home = std::env::temp_dir().join(format!(
            "yggterm-remote-mirror-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::create_dir_all(&home)?;
        let previous_home = std::env::var_os(yggterm_core::ENV_YGGTERM_HOME);
        unsafe {
            std::env::set_var(yggterm_core::ENV_YGGTERM_HOME, &home);
        }

        let sessions = vec![RemoteScannedSession {
            session_path: remote_scanned_session_path("jojo", "abc123"),
            session_id: "abc123".to_string(),
            cwd: "/srv/app".to_string(),
            started_at: "2026-03-24T12:00:00Z".to_string(),
            modified_epoch: 42,
            event_count: 9,
            user_message_count: 4,
            assistant_message_count: 5,
            title_hint: "Deploy Fix".to_string(),
            recent_context: "USER: test\nASSISTANT: reply".to_string(),
            cached_precis: Some("Short precis".to_string()),
            cached_summary: Some("Longer summary text".to_string()),
            storage_path: "/home/pi/.codex/sessions/test.jsonl".to_string(),
        }];
        mirror_remote_machine_sessions("jojo", &sessions)?;
        let loaded = load_remote_machine_sessions_from_mirror("jojo")?;

        if let Some(previous_home) = previous_home {
            unsafe {
                std::env::set_var(yggterm_core::ENV_YGGTERM_HOME, previous_home);
            }
        } else {
            unsafe {
                std::env::remove_var(yggterm_core::ENV_YGGTERM_HOME);
            }
        }
        let _ = fs::remove_dir_all(home);

        assert_eq!(loaded, sessions);
        Ok(())
    }

    #[test]
    fn remote_preview_open_does_not_queue_terminal_resume() -> Result<()> {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "root".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.remote_machines.push(RemoteMachineSnapshot {
            machine_key: "jojo".to_string(),
            label: "jojo".to_string(),
            ssh_target: "jojo".to_string(),
            prefix: None,
            remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: Vec::new(),
        });

        let session_path = server.open_remote_scanned_session_with_view(
            "jojo",
            "019cf00a-57bd-7480-a642-495ac1389b8e",
            Some("/home/pi"),
            Some("Passthrough Gpus Dev Set Intel"),
            Some(WorkspaceViewMode::Rendered),
        )?;

        let session = server.sessions.get(&session_path).expect("session");
        assert_eq!(session.launch_phase, TerminalLaunchPhase::RemoteBootstrap);
        assert_eq!(server.active_session_path(), Some(session_path.as_str()));
        assert_eq!(server.active_view_mode, WorkspaceViewMode::Rendered);
        Ok(())
    }

    #[test]
    fn remote_preview_open_keeps_synthesized_preview_while_loading() -> Result<()> {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "root".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.remote_machines.push(RemoteMachineSnapshot {
            machine_key: "jojo".to_string(),
            label: "jojo".to_string(),
            ssh_target: "jojo".to_string(),
            prefix: None,
            remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: vec![RemoteScannedSession {
                session_id: "019cf00a-57bd-7480-a642-495ac1389b8e".to_string(),
                session_path: "remote-session://jojo/019cf00a-57bd-7480-a642-495ac1389b8e".to_string(),
                cwd: "/home/pi".to_string(),
                started_at: "2026-03-24T12:00:00Z".to_string(),
                modified_epoch: 42,
                event_count: 9,
                user_message_count: 4,
                assistant_message_count: 5,
                title_hint: "Passthrough Gpus Dev Set Intel".to_string(),
                recent_context: "USER: stale summary-like first turn\nASSISTANT: stale reply".to_string(),
                cached_precis: Some("Short precis".to_string()),
                cached_summary: Some("Longer summary text".to_string()),
                storage_path: "/home/pi/.codex/sessions/test.jsonl".to_string(),
            }],
        });

        let session_path = server.open_remote_scanned_session_with_view(
            "jojo",
            "019cf00a-57bd-7480-a642-495ac1389b8e",
            Some("/home/pi"),
            Some("Passthrough Gpus Dev Set Intel"),
            Some(WorkspaceViewMode::Rendered),
        )?;

        let session = server.sessions.get(&session_path).expect("session");
        assert!(!session.preview.blocks.is_empty());
        assert!(session.rendered_sections.is_empty());
        assert_eq!(session.preview.blocks[0].tone, PreviewTone::User);
        assert!(
            session.preview.blocks[0]
                .lines
                .join(" ")
                .contains("stale summary-like first turn")
        );
        assert_eq!(
            session_metadata_value(session, "Preview Hydration").as_deref(),
            Some("loading")
        );
        Ok(())
    }

    #[test]
    fn recent_context_goals_do_not_become_fake_assistant_turns() {
        let (goals, blocks, sections) = parse_recent_context_sections(
            "PRIMARY USER GOALS:\n- Make a passwordless user pi and copy our dotfiles to this user.\n\nRECENT SUBSTANTIVE TURNS:\nUSER: Make a passwordless user pi and copy our ~/.{{all dot files with permissions and ownership adjusted}} to this user.\nASSISTANT: I'll create the pi account without a password, then copy the dotfiles and fix ownership.",
        );

        assert_eq!(
            goals,
            vec!["Make a passwordless user pi and copy our dotfiles to this user.".to_string()]
        );
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].tone, PreviewTone::User);
        assert!(
            blocks[0].lines[0].contains("Make a passwordless user pi and copy our ~/."),
            "unexpected first user preview block: {:?}",
            blocks[0].lines
        );
        assert_eq!(blocks[1].tone, PreviewTone::Assistant);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Primary User Goals");
    }

    #[test]
    fn recent_context_scaffold_lines_are_filtered_out_of_preview_turns() {
        let (_goals, blocks, _sections) = parse_recent_context_sections(
            "RECENT SUBSTANTIVE TURNS:\nUSER: # AGENTS.md instructions for /home/pi <INSTRUCTIONS> ## Skills ...\nUSER: <cwd>/home/pi</cwd>\nUSER: <shell>bash</shell>\nUSER: <approval_policy>never</approval_policy>\nUSER: Approvals are your mechanism to get user consent to run shell commands without the sandbox.\nUSER: <turn_aborted> The user interrupted the previous turn on purpose. Any running unified exec processes were terminated. </turn_aborted>\nUSER: I launched excel but it is a blank window.\nASSISTANT: You are now in Default mode. Any previous instructions for other modes (e.g. Plan mode) are no longer active.\nASSISTANT: I’m going to inspect the launch output and the window state.",
        );

        assert_eq!(blocks.len(), 2, "unexpected preview blocks: {blocks:?}");
        assert_eq!(blocks[0].tone, PreviewTone::User);
        assert_eq!(
            blocks[0].lines,
            vec!["I launched excel but it is a blank window.".to_string()]
        );
        assert_eq!(blocks[1].tone, PreviewTone::Assistant);
        assert_eq!(
            blocks[1].lines,
            vec!["I’m going to inspect the launch output and the window state.".to_string()]
        );
    }

    #[test]
    fn sanitize_recent_context_payload_rewrites_scaffolded_remote_summaries() {
        let text = sanitize_recent_context_payload(
            "PRIMARY USER GOALS:\n- Keep the real user request.\n- <turn_aborted> The user interrupted the previous turn on purpose. Any running unified exec processes were terminated. </turn_aborted>\n\nRECENT SUBSTANTIVE TURNS:\nUSER: Approvals are your mechanism to get user consent to run shell commands without the sandbox.\nUSER: # AGENTS.md instructions for /home/pi <INSTRUCTIONS> ...\nUSER: I launched excel but it is a blank window.\nASSISTANT: I’m sorry, but I can’t help with that.",
        );
        assert!(text.contains("Keep the real user request."));
        assert!(text.contains("USER: I launched excel but it is a blank window."));
        assert!(text.contains("ASSISTANT: I’m sorry, but I can’t help with that."));
        assert!(!text.contains("Approvals are your mechanism"));
        assert!(!text.contains("AGENTS.md instructions for"));
        assert!(!text.contains("<turn_aborted>"));
    }

    #[test]
    fn sanitize_recent_context_payload_dedupes_image_goal_markup() {
        let text = sanitize_recent_context_payload(
            "PRIMARY USER GOALS:\n- <image name=[Image #1]> </image> [Image #1] This is how excel looks.\n- [Image #1] This is how excel looks.\n\nRECENT SUBSTANTIVE TURNS:\nUSER: [Image #1] This is how excel looks.\nASSISTANT: I understand the screenshot.",
        );

        assert_eq!(text.matches("[Image #1] This is how excel looks.").count(), 2);
        assert!(!text.contains("<image name=[Image #1]>"));
        assert!(!text.contains("</image>"));
    }

    #[test]
    fn stored_transcript_preview_blocks_filter_scaffold_and_system_messages() {
        let mut blocks = Vec::new();
        let mut metadata_entries = Vec::new();
        let mut user_messages = 0;
        let mut assistant_messages = 0;

        push_preview_block(
            &mut blocks,
            &mut metadata_entries,
            &mut user_messages,
            &mut assistant_messages,
            TranscriptRole::User,
            vec![
                "<cwd>/home/pi</cwd>".to_string(),
                "<shell>bash</shell>".to_string(),
                "# AGENTS.md instructions for /home/pi <INSTRUCTIONS> ## Skills ...".to_string(),
                "Approvals are your mechanism to get user consent to run shell commands without the sandbox.".to_string(),
                "<turn_aborted> The user interrupted the previous turn on purpose. Any running unified exec processes were terminated. </turn_aborted>".to_string(),
                "I launched excel but it is a blank window.".to_string(),
            ],
            "Feb 07, 2026 08:41 PM UTC+0530".to_string(),
        );
        push_preview_block(
            &mut blocks,
            &mut metadata_entries,
            &mut user_messages,
            &mut assistant_messages,
            TranscriptRole::System,
            vec!["You are now in Default mode.".to_string()],
            "Feb 07, 2026 08:43 PM UTC+0530".to_string(),
        );

        assert_eq!(user_messages, 1);
        assert_eq!(assistant_messages, 0);
        assert_eq!(blocks.len(), 1, "unexpected preview blocks: {blocks:?}");
        assert_eq!(
            blocks[0].lines,
            vec!["I launched excel but it is a blank window.".to_string()]
        );
    }

    #[test]
    fn remote_preview_payload_apply_filters_scaffold_from_older_remote_binaries() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "root".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.open_or_focus_session(
            SessionKind::Codex,
            "remote-session://jojo/test",
            Some("test-session"),
            Some("/home/pi"),
            Some("Test Session"),
            None,
        );

        let payload = RemotePreviewPayload {
            title_hint: Some("Test Session".to_string()),
            cached_precis: None,
            cached_summary: None,
            preview: SnapshotPreview {
                summary: Vec::new(),
                blocks: vec![
                    SnapshotPreviewBlock {
                        role: "USER".to_string(),
                        timestamp: "Feb 07, 2026 02:56 PM UTC+0530".to_string(),
                        tone: PreviewTone::User,
                        folded: false,
                        lines: vec![
                            "<cwd>/home/pi</cwd>".to_string(),
                            "<shell>bash</shell>".to_string(),
                            "I launched excel but it is a blank window.".to_string(),
                        ],
                    },
                    SnapshotPreviewBlock {
                        role: "USER".to_string(),
                        timestamp: "Feb 07, 2026 02:59 PM UTC+0530".to_string(),
                        tone: PreviewTone::User,
                        folded: false,
                        lines: vec![
                            "You are now in Default mode.".to_string(),
                            "</collaboration_mode>".to_string(),
                        ],
                    },
                ],
            },
            rendered_sections: vec![SnapshotRenderedSection {
                title: "Recent Context".to_string(),
                lines: vec!["placeholder".to_string()],
            }],
        };

        let session = server
            .sessions
            .get_mut("remote-session://jojo/test")
            .expect("session");
        apply_remote_preview_payload(session, payload);

        assert_eq!(
            session.preview.blocks.len(),
            1,
            "{:?}",
            session.preview.blocks
        );
        assert_eq!(
            session.preview.blocks[0].lines,
            vec!["I launched excel but it is a blank window.".to_string()]
        );
    }

    #[test]
    fn remote_preview_payload_apply_filters_current_date_timezone_scaffold() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "root".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.open_or_focus_session(
            SessionKind::Codex,
            "remote-session://jojo/test-scaffold",
            Some("test-scaffold"),
            Some("/home/pi"),
            Some("Test Scaffold"),
            None,
        );

        let payload = RemotePreviewPayload {
            title_hint: Some("Test Scaffold".to_string()),
            cached_precis: None,
            cached_summary: None,
            preview: SnapshotPreview {
                summary: Vec::new(),
                blocks: vec![
                    SnapshotPreviewBlock {
                        role: "USER".to_string(),
                        timestamp: "Mar 20, 2026 10:40 AM UTC+0530".to_string(),
                        tone: PreviewTone::User,
                        folded: false,
                        lines: vec![
                            "<current_date>2026-03-20</current_date>".to_string(),
                            "<timezone>Asia/Kolkata</timezone>".to_string(),
                        ],
                    },
                    SnapshotPreviewBlock {
                        role: "USER".to_string(),
                        timestamp: "Mar 20, 2026 10:40 AM UTC+0530".to_string(),
                        tone: PreviewTone::User,
                        folded: false,
                        lines: vec!["Investigate the boot delay on manin.".to_string()],
                    },
                ],
            },
            rendered_sections: Vec::new(),
        };

        let session = server
            .sessions
            .get_mut("remote-session://jojo/test-scaffold")
            .expect("session");
        apply_remote_preview_payload(session, payload);

        assert_eq!(session.preview.blocks.len(), 1, "{:?}", session.preview.blocks);
        assert_eq!(
            session.preview.blocks[0].lines,
            vec!["Investigate the boot delay on manin.".to_string()]
        );
    }

    #[test]
    fn remote_preview_payload_apply_filters_placeholder_rendered_sections() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "root".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.open_or_focus_session(
            SessionKind::Codex,
            "remote-session://jojo/test-rendered-sections",
            Some("test-rendered-sections"),
            Some("/home/pi"),
            Some("Test Rendered Sections"),
            None,
        );

        let payload = RemotePreviewPayload {
            title_hint: Some("Test Rendered Sections".to_string()),
            cached_precis: None,
            cached_summary: None,
            preview: SnapshotPreview {
                summary: Vec::new(),
                blocks: vec![SnapshotPreviewBlock {
                    role: "USER".to_string(),
                    timestamp: "Mar 31, 2026 06:00 PM UTC+0530".to_string(),
                    tone: PreviewTone::User,
                    folded: false,
                    lines: vec!["Check the rendered preview.".to_string()],
                }],
            },
            rendered_sections: vec![
                SnapshotRenderedSection {
                    title: "Rendered Session".to_string(),
                    lines: vec![
                        "Preview mode renders the stored Codex transcript as a chat surface."
                            .to_string(),
                    ],
                },
                SnapshotRenderedSection {
                    title: "Server Notes".to_string(),
                    lines: vec![
                        "GUI selection asks the Yggterm server to open or focus the session."
                            .to_string(),
                    ],
                },
                SnapshotRenderedSection {
                    title: "Primary User Goals".to_string(),
                    lines: vec!["Make the preview stable.".to_string()],
                },
            ],
        };

        let session = server
            .sessions
            .get_mut("remote-session://jojo/test-rendered-sections")
            .expect("session");
        apply_remote_preview_payload(session, payload);

        assert_eq!(session.rendered_sections.len(), 1);
        assert_eq!(session.rendered_sections[0].title, "Primary User Goals");
        assert_eq!(
            session.rendered_sections[0].lines,
            vec!["Make the preview stable.".to_string()]
        );
    }

    #[test]
    fn managed_session_from_snapshot_filters_scaffold_preview_blocks() {
        let session = managed_session_from_snapshot(SnapshotSessionView {
            id: "019c38a8-e725-7cc0-81ed-db9595254739".to_string(),
            session_path: "remote-session://jojo/019c38a8-e725-7cc0-81ed-db9595254739".to_string(),
            title: "Router issue".to_string(),
            kind: SessionKind::Codex,
            host_label: "jojo".to_string(),
            source: SessionSource::Stored,
            backend: TerminalBackend::Xterm,
            bridge_available: false,
            launch_phase: TerminalLaunchPhase::Running,
            remote_deploy_state: RemoteDeployState::Ready,
            launch_command: String::new(),
            status_line: String::new(),
            terminal_lines: Vec::new(),
            rendered_sections: Vec::new(),
            preview: SnapshotPreview {
                summary: Vec::new(),
                blocks: vec![
                    SnapshotPreviewBlock {
                        role: "USER".to_string(),
                        timestamp: "Feb 07, 2026 08:41 PM UTC+0530".to_string(),
                        tone: PreviewTone::User,
                        folded: false,
                        lines: vec![
                            "<cwd>/home/pi</cwd>".to_string(),
                            "<shell>bash</shell>".to_string(),
                            "Just 30 mins ago, no client can connect with my GL-iNet router."
                                .to_string(),
                        ],
                    },
                    SnapshotPreviewBlock {
                        role: "USER".to_string(),
                        timestamp: "Feb 07, 2026 08:43 PM UTC+0530".to_string(),
                        tone: PreviewTone::User,
                        folded: false,
                        lines: vec![
                            "You are now in Default mode.".to_string(),
                            "If a decision is necessary and cannot be discovered from local context, ask the user directly.".to_string(),
                            "</collaboration_mode>".to_string(),
                        ],
                    },
                ],
            },
            metadata: Vec::new(),
            terminal_process_id: None,
            terminal_window_id: None,
            terminal_host_token: None,
            terminal_host_mode: GhosttyTerminalHostMode::Unsupported,
            embedded_surface_id: None,
            embedded_surface_detail: None,
            last_launch_error: None,
            last_window_error: None,
            ssh_target: None,
            ssh_prefix: None,
        });

        assert_eq!(
            session.preview.blocks.len(),
            1,
            "{:?}",
            session.preview.blocks
        );
        assert_eq!(
            session.preview.blocks[0].lines,
            vec!["Just 30 mins ago, no client can connect with my GL-iNet router.".to_string()]
        );
    }

    #[test]
    fn dedupe_remote_scanned_sessions_prefers_richer_newer_entry() {
        let sessions = vec![
            RemoteScannedSession {
                session_path: remote_scanned_session_path("oc", "abc123"),
                session_id: "abc123".to_string(),
                cwd: "/home/pi".to_string(),
                started_at: "2026-03-24T12:00:00Z".to_string(),
                modified_epoch: 10,
                event_count: 4,
                user_message_count: 2,
                assistant_message_count: 2,
                title_hint: "Qabc123".to_string(),
                recent_context: "short".to_string(),
                cached_precis: None,
                cached_summary: None,
                storage_path: "/one.jsonl".to_string(),
            },
            RemoteScannedSession {
                session_path: remote_scanned_session_path("oc", "abc123"),
                session_id: "abc123".to_string(),
                cwd: "/home/pi".to_string(),
                started_at: "2026-03-24T12:05:00Z".to_string(),
                modified_epoch: 11,
                event_count: 8,
                user_message_count: 4,
                assistant_message_count: 4,
                title_hint: "Container Mount Fix".to_string(),
                recent_context: "much longer context payload".to_string(),
                cached_precis: Some("precis".to_string()),
                cached_summary: Some("summary".to_string()),
                storage_path: "/two.jsonl".to_string(),
            },
        ];

        let deduped = dedupe_remote_scanned_sessions(sessions);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].session_id, "abc123");
        assert_eq!(deduped[0].title_hint, "Container Mount Fix");
        assert_eq!(deduped[0].cached_precis.as_deref(), Some("precis"));
        assert_eq!(deduped[0].cached_summary.as_deref(), Some("summary"));
        assert_eq!(deduped[0].storage_path, "/two.jsonl");
    }

    #[test]
    fn restore_persisted_state_filters_loopback_ssh_targets() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.restore_persisted_state(
            PersistedDaemonState {
                active_session_path: None,
                active_view_mode: WorkspaceViewMode::Rendered,
                ssh_targets: vec![SshConnectTarget {
                    label: "localhost".to_string(),
                    kind: SessionKind::SshShell,
                    ssh_target: "localhost".to_string(),
                    prefix: None,
                    cwd: None,
                }],
                remote_machines: vec![RemoteMachineSnapshot {
                    machine_key: "localhost".to_string(),
                    label: "localhost".to_string(),
                    ssh_target: "localhost".to_string(),
                    prefix: None,
                    remote_binary_expr: None,
                    remote_deploy_state: RemoteDeployState::Planned,
                    health: RemoteMachineHealth::Cached,
                    sessions: Vec::new(),
                }],
                stored_sessions: Vec::new(),
                live_sessions: Vec::new(),
            },
            None,
        );

        assert!(server.ssh_targets().is_empty());
        assert!(server.remote_machines().is_empty());
    }

    #[test]
    fn restore_persisted_state_rehydrates_remote_active_session_into_live_order() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let active_path = remote_scanned_session_path("dev", "abc123");
        server.restore_persisted_state(
            PersistedDaemonState {
                active_session_path: Some(active_path.clone()),
                active_view_mode: WorkspaceViewMode::Terminal,
                ssh_targets: vec![SshConnectTarget {
                    label: "dev".to_string(),
                    kind: SessionKind::SshShell,
                    ssh_target: "dev".to_string(),
                    prefix: None,
                    cwd: Some("/home/pi/gh/yggterm".to_string()),
                }],
                remote_machines: vec![RemoteMachineSnapshot {
                    machine_key: "dev".to_string(),
                    label: "dev".to_string(),
                    ssh_target: "dev".to_string(),
                    prefix: None,
                    remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
                    remote_deploy_state: RemoteDeployState::Ready,
                    health: RemoteMachineHealth::Healthy,
                    sessions: vec![RemoteScannedSession {
                        session_path: active_path.clone(),
                        session_id: "abc123".to_string(),
                        cwd: "/home/pi/gh/yggterm".to_string(),
                        started_at: "2026-03-28T12:00:00Z".to_string(),
                        modified_epoch: 1,
                        event_count: 8,
                        user_message_count: 4,
                        assistant_message_count: 4,
                        title_hint: "Restore Live Remote".to_string(),
                        recent_context: "USER: restore".to_string(),
                        cached_precis: Some("precis".to_string()),
                        cached_summary: Some("summary".to_string()),
                        storage_path: "/home/pi/.codex/sessions/abc123.jsonl".to_string(),
                    }],
                }],
                stored_sessions: Vec::new(),
                live_sessions: Vec::new(),
            },
            None,
        );

        let live_sessions = server.live_sessions();
        assert_eq!(server.active_session_path(), Some(active_path.as_str()));
        assert_eq!(live_sessions.len(), 1);
        assert_eq!(live_sessions[0].session_path, active_path);
    }

    #[test]
    fn restore_persisted_state_defers_inactive_stored_transcript_hydration() -> Result<()> {
        let transcript_dir = std::env::temp_dir().join(format!(
            "yggterm-server-restore-deferred-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::create_dir_all(&transcript_dir)?;
        let active_path = transcript_dir.join("active.jsonl");
        let inactive_path = transcript_dir.join("inactive.jsonl");
        let transcript = [
            r#"{"timestamp":"2026-03-20T10:00:00Z","type":"session_meta","payload":{"id":"orig","timestamp":"2026-03-20T10:00:00Z","cwd":"/tmp/x"}}"#,
            r#"{"timestamp":"2026-03-20T10:00:01Z","type":"compacted","payload":{"replacement_history":[{"role":"user","type":"message","content":[{"type":"input_text","text":"first prompt"}]},{"role":"assistant","type":"message","content":[{"type":"output_text","text":"first answer"}]}]}}"#,
        ]
        .join("\n");
        fs::write(&active_path, &transcript)?;
        fs::write(&inactive_path, &transcript)?;

        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.restore_persisted_state(
            PersistedDaemonState {
                active_session_path: Some(active_path.display().to_string()),
                active_view_mode: WorkspaceViewMode::Rendered,
                ssh_targets: Vec::new(),
                remote_machines: Vec::new(),
                stored_sessions: vec![
                    PersistedStoredSession {
                        path: inactive_path.display().to_string(),
                        kind: SessionKind::Codex,
                        session_id: Some("inactive".to_string()),
                        cwd: Some("/tmp/inactive".to_string()),
                        title_hint: Some("Inactive".to_string()),
                    },
                    PersistedStoredSession {
                        path: active_path.display().to_string(),
                        kind: SessionKind::Codex,
                        session_id: Some("active".to_string()),
                        cwd: Some("/tmp/active".to_string()),
                        title_hint: Some("Active".to_string()),
                    },
                ],
                live_sessions: Vec::new(),
            },
            None,
        );

        let active = server
            .sessions
            .get(active_path.to_string_lossy().as_ref())
            .expect("active session");
        assert!(active.stored_preview_hydrated);
        assert_eq!(active.preview.blocks[0].lines[0], "first prompt");

        let inactive = server
            .sessions
            .get(inactive_path.to_string_lossy().as_ref())
            .expect("inactive session");
        assert!(!inactive.stored_preview_hydrated);
        assert!(
            inactive.preview.blocks[0].lines[0].contains("Resume Codex session inactive."),
            "{:?}",
            inactive.preview.blocks
        );

        let _ = fs::remove_dir_all(transcript_dir);
        Ok(())
    }

    #[test]
    fn opening_deferred_stored_session_hydrates_preview_on_demand() -> Result<()> {
        let transcript_path = std::env::temp_dir().join(format!(
            "yggterm-server-open-deferred-{}-{}.jsonl",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(
            &transcript_path,
            [
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"session_meta","payload":{"id":"orig","timestamp":"2026-03-20T10:00:00Z","cwd":"/tmp/x"}}"#,
                r#"{"timestamp":"2026-03-20T10:00:01Z","type":"compacted","payload":{"replacement_history":[{"role":"user","type":"message","content":[{"type":"input_text","text":"first prompt"}]},{"role":"assistant","type":"message","content":[{"type":"output_text","text":"first answer"}]}]}}"#,
            ]
            .join("\n"),
        )?;

        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let path = transcript_path.to_string_lossy().to_string();
        server.sessions.insert(
            path.clone(),
            build_session(
                SessionKind::Codex,
                &path,
                Some("deferred"),
                Some("/tmp/work"),
                Some("Deferred"),
                None,
                TerminalBackend::Xterm,
                UiTheme::ZedLight,
                false,
                StoredPreviewHydrationMode::Deferred,
            ),
        );

        let before = server.sessions.get(&path).expect("before");
        assert!(!before.stored_preview_hydrated);

        server.open_or_focus_session(
            SessionKind::Codex,
            &path,
            Some("deferred"),
            Some("/tmp/work"),
            Some("Deferred"),
            None,
        );

        let after = server.sessions.get(&path).expect("after");
        assert!(after.stored_preview_hydrated);
        assert_eq!(after.preview.blocks[0].lines[0], "first prompt");

        let _ = fs::remove_file(transcript_path);
        Ok(())
    }

    #[test]
    fn remote_scan_roots_stays_scoped_to_requested_user_home() -> Result<()> {
        let base = std::env::temp_dir().join(format!(
            "yggterm-remote-scan-roots-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let requested = base.join("root-home").join(".codex");
        let parent = base.join("home");
        let machine_user_codex = parent.join("pi").join(".codex");
        fs::create_dir_all(requested.join("sessions"))?;
        fs::create_dir_all(machine_user_codex.join("sessions").join("2026").join("03"))?;
        fs::write(
            machine_user_codex
                .join("sessions")
                .join("2026")
                .join("03")
                .join("rollout.jsonl"),
            "{}\n",
        )?;

        let roots =
            remote_scan_roots_with_parents(&requested, Some("~/.codex"), [parent.as_path()]);

        assert_eq!(roots, vec![requested]);

        let _ = fs::remove_dir_all(base);
        Ok(())
    }

    #[test]
    fn restore_live_remote_session_preserves_remote_session_path_without_cached_scan() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.remote_machines.push(RemoteMachineSnapshot {
            machine_key: "jojo".to_string(),
            label: "jojo".to_string(),
            ssh_target: "jojo".to_string(),
            prefix: None,
            remote_binary_expr: None,
            remote_deploy_state: RemoteDeployState::Planned,
            health: RemoteMachineHealth::Cached,
            sessions: Vec::new(),
        });

        server.restore_live_session(PersistedLiveSession {
            key: "remote-session://jojo/abc123".to_string(),
            id: "abc123".to_string(),
            title: "Example".to_string(),
            kind: SessionKind::SshShell,
            ssh_target: "jojo".to_string(),
            prefix: None,
            cwd: Some("/srv/app".to_string()),
        });

        let session = server
            .sessions
            .get("remote-session://jojo/abc123")
            .expect("restored session");
        assert_eq!(session.session_path, "remote-session://jojo/abc123");
    }

    #[test]
    fn restore_live_remote_session_uses_cached_fallback_binary_without_resolve() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.remote_machines.push(RemoteMachineSnapshot {
            machine_key: "dev".to_string(),
            label: "dev".to_string(),
            ssh_target: "definitely-not-a-real-host.invalid".to_string(),
            prefix: None,
            remote_binary_expr: None,
            remote_deploy_state: RemoteDeployState::Planned,
            health: RemoteMachineHealth::Cached,
            sessions: vec![RemoteScannedSession {
                session_path: "remote-session://dev/abc123".to_string(),
                session_id: "abc123".to_string(),
                cwd: "/srv/app".to_string(),
                started_at: "2026-03-28T12:00:00Z".to_string(),
                modified_epoch: 1,
                event_count: 8,
                user_message_count: 4,
                assistant_message_count: 4,
                title_hint: "Remote Session".to_string(),
                recent_context: "USER: test".to_string(),
                cached_precis: None,
                cached_summary: None,
                storage_path: "/tmp/abc123.jsonl".to_string(),
            }],
        });

        server.restore_live_session(PersistedLiveSession {
            key: "remote-session://dev/abc123".to_string(),
            id: "abc123".to_string(),
            title: "Remote Session".to_string(),
            kind: SessionKind::SshShell,
            ssh_target: "definitely-not-a-real-host.invalid".to_string(),
            prefix: None,
            cwd: Some("/srv/app".to_string()),
        });

        let session = server
            .sessions
            .get("remote-session://dev/abc123")
            .expect("restored session");
        assert_eq!(session.remote_deploy_state, RemoteDeployState::Planned);
        assert!(session.launch_command.contains("resume-codex"));
        assert!(
            session
                .launch_command
                .contains("definitely-not-a-real-host.invalid")
        );
    }

    #[test]
    fn remote_session_stop_command_does_not_send_quit_into_attached_terminal() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.remote_machines.push(RemoteMachineSnapshot {
            machine_key: "dev".to_string(),
            label: "dev".to_string(),
            ssh_target: "dev".to_string(),
            prefix: None,
            remote_binary_expr: None,
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: Vec::new(),
        });
        server.restore_live_session(PersistedLiveSession {
            key: "remote-session://dev/abc123".to_string(),
            id: "abc123".to_string(),
            title: "Remote session".to_string(),
            kind: SessionKind::SshShell,
            ssh_target: "dev".to_string(),
            prefix: None,
            cwd: Some("/home/pi/gh".to_string()),
        });

        assert_eq!(
            server.terminal_stop_command("remote-session://dev/abc123"),
            None
        );
    }

    #[test]
    fn local_terminal_stop_commands_match_session_kind() {
        let tree = SessionNode {
            kind: SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        server.restore_live_session(PersistedLiveSession {
            key: "local://codex".to_string(),
            id: "codex".to_string(),
            title: "Codex".to_string(),
            kind: SessionKind::Codex,
            ssh_target: String::new(),
            prefix: None,
            cwd: Some("/home/pi/gh".to_string()),
        });
        server.restore_live_session(PersistedLiveSession {
            key: "local://shell".to_string(),
            id: "shell".to_string(),
            title: "Shell".to_string(),
            kind: SessionKind::Shell,
            ssh_target: String::new(),
            prefix: None,
            cwd: Some("/home/pi".to_string()),
        });

        assert_eq!(
            server.terminal_stop_command("local://codex"),
            Some("/quit\r".to_string())
        );
        assert_eq!(
            server.terminal_stop_command("local://shell"),
            Some("exit\r".to_string())
        );
    }

    #[test]
    fn synthesize_remote_scanned_session_uses_machine_launch_metadata() {
        let machine = RemoteMachineSnapshot {
            machine_key: "oc".to_string(),
            label: "oc".to_string(),
            ssh_target: "oc".to_string(),
            prefix: None,
            remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: Vec::new(),
        };
        let scanned = RemoteScannedSession {
            session_path: remote_scanned_session_path("oc", "abc123"),
            session_id: "abc123".to_string(),
            cwd: "/srv/app".to_string(),
            started_at: "2026-03-29T00:00:00Z".to_string(),
            modified_epoch: 1,
            event_count: 4,
            user_message_count: 2,
            assistant_message_count: 2,
            title_hint: "Example".to_string(),
            recent_context: "USER: example".to_string(),
            cached_precis: None,
            cached_summary: None,
            storage_path: "/home/pi/.codex/sessions/example.jsonl".to_string(),
        };

        let session = synthesize_remote_scanned_session_view(
            &machine,
            &scanned,
            TerminalBackend::Xterm,
            UiTheme::ZedLight,
            false,
        );

        assert!(
            session
                .launch_command
                .contains("$HOME/.yggterm/bin/yggterm")
        );
        assert_eq!(session.remote_deploy_state, RemoteDeployState::Ready);
    }

    #[test]
    fn remote_command_permission_denied_is_recoverable() {
        let error = anyhow::anyhow!(
            "remote yggterm command failed for oc: sh: 1: /home/pi/.yggterm/bin/yggterm: Permission denied"
        );
        assert!(should_fallback_to_python(&error));
    }

    #[test]
    fn remote_saved_codex_session_exists_checks_codex_home() -> Result<()> {
        let home = std::env::temp_dir().join(format!(
            "yggterm-remote-resume-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let sessions_dir = home.join("sessions").join("2026").join("03");
        fs::create_dir_all(&sessions_dir)?;
        fs::write(
            sessions_dir.join("rollout-test.jsonl"),
            "{\"id\":\"abc123\",\"cwd\":\"/srv/app\"}\n",
        )?;
        let previous_codex_home = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::set_var("CODEX_HOME", &home);
        }

        let exists = remote_saved_codex_session_exists("abc123")?;
        let missing = remote_saved_codex_session_exists("missing")?;

        if let Some(previous_codex_home) = previous_codex_home {
            unsafe {
                std::env::set_var("CODEX_HOME", previous_codex_home);
            }
        } else {
            unsafe {
                std::env::remove_var("CODEX_HOME");
            }
        }
        let _ = fs::remove_dir_all(home);

        assert!(exists);
        assert!(!missing);
        Ok(())
    }
}
