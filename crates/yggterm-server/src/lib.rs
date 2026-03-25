mod attach;
mod daemon;
mod host;
mod terminal;

pub use attach::{AttachMetadata, run_attach};
pub use daemon::{
    ServerEndpoint, ServerRequest, ServerResponse, ServerRuntimeStatus, TerminalStreamChunk,
    cleanup_legacy_daemons, connect_ssh, connect_ssh_custom, default_endpoint, focus_live,
    open_remote_session, open_stored_session, ping, raise_external_window,
    refresh_remote_machine, remove_ssh_target, request_terminal_launch, run_daemon,
    set_all_preview_blocks_folded, set_view_mode, shutdown, snapshot, start_command_session,
    start_local_session, start_local_session_at, status, switch_agent_session_mode,
    sync_external_window, sync_theme, terminal_ensure, terminal_read, terminal_resize,
    terminal_write, toggle_preview_block,
};
pub use host::{GhosttyHostKind, GhosttyHostSupport, GhosttyTerminalHostMode, detect_ghostty_host};
pub use terminal::{TerminalChunk, TerminalManager, TerminalReadResult};

use anyhow::Context;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;
use time::{OffsetDateTime, UtcOffset, macros::format_description};
use tracing::warn;
use uuid::Uuid;
use yggterm_core::{
    PerfSpan, SessionNode, SessionNodeKind, SessionStore, SessionTitleStore, TranscriptRole,
    UiTheme, WorkspaceDocument, WorkspaceDocumentKind, looks_like_generated_fallback_title,
    read_codex_session_identity_fields, read_codex_transcript_messages, resolve_yggterm_home,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceViewMode {
    Terminal,
    Rendered,
}

static REMOTE_YGGTERM_COMMAND_CACHE: OnceLock<Mutex<std::collections::HashMap<String, String>>> =
    OnceLock::new();

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
        if let Some((machine_key, session_id)) = parse_remote_scanned_session_path(&path) {
            self.refresh_remote_scanned_session_preview(machine_key, session_id)?;
            return Ok(());
        }
        let Some(session) = self.sessions.get(&path).cloned() else {
            return Ok(());
        };
        match session.source {
            SessionSource::Stored => self.refresh_stored_session_preview(&path, &session)?,
            SessionSource::LiveSsh => {
                if let Some((machine_key, session_id)) = parse_remote_scanned_session_path(&path) {
                    self.refresh_remote_scanned_session_preview(machine_key, session_id)?;
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
        self.active_session_path = Some(path.to_string());
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
        self.active_session_path = Some(path.to_string());
        self.request_terminal_launch_for_active();
    }

    pub fn terminal_spec(&self, path: &str) -> Option<(String, Option<String>)> {
        self.sessions.get(path).and_then(|session| {
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
        let session = self.sessions.get(path)?;
        if session.kind == SessionKind::Document {
            return recipe_terminal_spec(session).map(|_| "exit\r".to_string());
        }
        match session.kind {
            SessionKind::Codex | SessionKind::CodexLiteLlm => Some("/quit\r".to_string()),
            SessionKind::Shell | SessionKind::SshShell => Some("exit\r".to_string()),
            SessionKind::Document => None,
        }
    }

    pub fn ssh_targets(&self) -> &[SshConnectTarget] {
        &self.ssh_targets
    }

    pub fn remove_ssh_targets_for_machine(&mut self, machine_key: &str) -> usize {
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
        for machine in &mut self.remote_machines {
            for scanned in &mut machine.sessions {
                if scanned.session_path == session_path {
                    scanned.cached_summary = Some(summary.to_string());
                    return;
                }
            }
        }
    }

    pub fn remote_machines(&self) -> &[RemoteMachineSnapshot] {
        &self.remote_machines
    }

    pub fn live_sessions(&self) -> Vec<ManagedSessionView> {
        let mut sessions = self
            .live_session_order
            .iter()
            .filter_map(|key| self.sessions.get(key).cloned())
            .collect::<Vec<_>>();
        if let Some(active) = self
            .active_session()
            .cloned()
            .or_else(|| {
                self.active_session_path.as_deref().and_then(|path| {
                    synthesize_remote_active_session(
                        path,
                        &self.remote_machines,
                        self.backend,
                        self.theme,
                        self.ghostty_host.bridge_enabled,
                    )
                })
            })
        {
            let active_path = active.session_path.clone();
            if !sessions.iter().any(|session| session.session_path == active_path) {
                sessions.insert(0, active);
            }
        }
        sessions
    }

    pub fn snapshot(&self) -> ServerUiSnapshot {
        let mut live_sessions = self
            .live_session_order
            .iter()
            .filter_map(|key| self.sessions.get(key))
            .cloned()
            .map(snapshot_session_view)
            .collect::<Vec<_>>();
        let active_session = self
            .active_session()
            .cloned()
            .or_else(|| {
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
        if let Some(active) = active_session.clone().map(snapshot_session_view) {
            let active_path = active.session_path.clone();
            if !live_sessions
                .iter()
                .any(|session| session.session_path == active_path)
            {
                live_sessions.insert(0, active);
            }
        }
        ServerUiSnapshot {
            active_session_path: self.active_session_path.clone(),
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
            if !self.live_session_order.iter().any(|path| path == &active_path) {
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
        self.ssh_targets = state
            .ssh_targets
            .into_iter()
            .filter(|target| !is_loopback_ssh_target(&target.ssh_target))
            .collect();
        for target in &mut self.ssh_targets {
            target.label = ssh_machine_label(target);
        }
        self.remote_machines = state
            .remote_machines
            .into_iter()
            .filter(|machine| !is_loopback_ssh_target(&machine.ssh_target))
            .map(|mut machine| {
                if machine.health == RemoteMachineHealth::Healthy {
                    machine.health = RemoteMachineHealth::Cached;
                }
                machine.sessions = if machine.sessions.is_empty() {
                    load_remote_machine_sessions_from_mirror(&machine.machine_key).unwrap_or_default()
                } else {
                    overlay_mirrored_remote_sessions(&machine.machine_key, &machine.sessions)
                };
                if !machine.sessions.is_empty() {
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
        for session in state.stored_sessions {
            let document = if session.kind == SessionKind::Document {
                store.and_then(|store| store.load_document(&session.path).ok().flatten())
            } else {
                None
            };
            self.open_or_focus_session(
                session.kind,
                &session.path,
                session.session_id.as_deref(),
                session.cwd.as_deref(),
                session.title_hint.as_deref(),
                document.as_ref(),
            );
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
        if let Some(path) = state.active_session_path {
            if self.sessions.contains_key(&path) {
                self.active_session_path = Some(path);
            }
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
        let ssh_target = target.trim();
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
            .unwrap_or(ssh_target)
            .to_string();
        let target = SshConnectTarget {
            label,
            kind: SessionKind::SshShell,
            ssh_target: ssh_target.to_string(),
            prefix,
            cwd: None,
        };
        let (key, reused) = self.connect_ssh_like_target(&target);
        key.map(|key| (key, reused))
            .ok_or_else(|| anyhow::anyhow!("failed to create ssh session"))
    }

    pub fn refresh_remote_machine_for_ssh_target(
        &mut self,
        target: &SshConnectTarget,
    ) -> anyhow::Result<()> {
        if is_loopback_ssh_target(&target.ssh_target) {
            return Ok(());
        }
        let machine_key = machine_key_from_ssh_target(&target.ssh_target);
        let label = ssh_machine_label(target);
        let entry_ix = self.ensure_remote_machine_stub(target);
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
                    health: RemoteMachineHealth::Healthy,
                    sessions,
                };
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
                    health: if existing_sessions.is_empty() {
                        RemoteMachineHealth::Offline
                    } else {
                        RemoteMachineHealth::Cached
                    },
                    sessions: existing_sessions,
                };
                Err(error)
            }
        }
    }

    pub fn refresh_remote_machine_by_key(&mut self, machine_key: &str) -> anyhow::Result<()> {
        let target = self
            .ssh_targets
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
            .ok_or_else(|| anyhow::anyhow!("remote machine not found: {machine_key}"))?;
        self.refresh_remote_machine_for_ssh_target(&target)
    }

    pub fn open_remote_scanned_session(
        &mut self,
        machine_key: &str,
        session_id: &str,
        cwd: Option<&str>,
        title_hint: Option<&str>,
    ) -> anyhow::Result<String> {
        let machine = self
            .remote_machines
            .iter()
            .find(|machine| machine.machine_key == machine_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("remote machine not found: {machine_key}"))?;
        let session_path = remote_scanned_session_path(machine_key, session_id);
        let target = SshConnectTarget {
            label: machine.label.clone(),
            kind: SessionKind::SshShell,
            ssh_target: machine.ssh_target.clone(),
            prefix: machine.prefix.clone(),
            cwd: cwd.map(ToOwned::to_owned),
        };
        let (remote_binary, remote_deploy_state) =
            resolve_remote_yggterm_binary(&target.ssh_target, target.prefix.as_deref())
                .unwrap_or_else(|_| ("yggterm".to_string(), RemoteDeployState::Planned));
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
                    &["server", "remote", "resume-codex", session_id, cwd.unwrap_or("")],
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
                }
            }
            self.focus_live_session(&session_path);
            self.request_terminal_launch_for_path(&session_path);
            return Ok(session_path);
        }
        self.insert_live_session(
            &session_path,
            session_id,
            SessionKind::SshShell,
            &target,
            Some(resolved_title.clone()),
        );
        if let Some(session) = self.sessions.get_mut(&session_path) {
            session.session_path = session_path.clone();
            session.title = resolved_title;
            session.host_label = machine.label.clone();
            session.launch_command = remote_ssh_launch_command(
                &target.ssh_target,
                target.prefix.as_deref(),
                &remote_binary,
                &["server", "remote", "resume-codex", session_id, cwd.unwrap_or("")],
            );
            session.terminal_lines = vec![
                format!("$ {}", session.launch_command),
                format!("Queue remote Yggterm resume {session_id}"),
                format!("Target host: {}", target.ssh_target),
                format!(
                    "Workspace: {}",
                    cwd.unwrap_or("<unknown>")
                ),
                "Daemon PTY: request main viewport terminal stream".to_string(),
            ];
            upsert_session_metadata(&mut session.metadata, "Source", "remote-codex".to_string());
            upsert_session_metadata(
                &mut session.metadata,
                "Host",
                target.ssh_target.clone(),
            );
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
            }
        }
        self.request_terminal_launch_for_path(&session_path);
        Ok(session_path)
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
        let title = title_hint
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| target.label.clone());
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
        if let Some((machine_key, session_id)) = parse_remote_scanned_session_path(&live.key)
            && let Some(machine) = self
                .remote_machines
                .iter()
                .find(|machine| machine.machine_key == machine_key)
                .cloned()
            && let Some(scanned) = machine
                .sessions
                .iter()
                .find(|session| session.session_id == session_id)
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
            self.sessions.insert(live.key.clone(), session);
            self.live_session_order.retain(|existing| existing != &live.key);
            self.live_session_order.insert(0, live.key.clone());
            self.active_session_path = Some(live.key);
            self.active_view_mode = WorkspaceViewMode::Terminal;
            return;
        }
        self.insert_live_session_with_launch(
            &live.key,
            &live.id,
            live.kind,
            &target,
            Some(live.title),
            false,
        );
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
                    let (remote_binary, remote_deploy_state) =
                        resolve_remote_yggterm_binary(&ssh_target, session.ssh_prefix.as_deref())
                            .unwrap_or_else(|_| ("yggterm".to_string(), RemoteDeployState::Planned));
                    session.remote_deploy_state = remote_deploy_state;
                    session.launch_command = if session.session_path.starts_with("remote-session://") {
                        let cwd = session_metadata_value(session, "Cwd").unwrap_or_default();
                        remote_ssh_launch_command(
                            &ssh_target,
                            session.ssh_prefix.as_deref(),
                            &remote_binary,
                            &["server", "remote", "resume-codex", &session.id, &cwd],
                        )
                    } else {
                        remote_ssh_launch_command(
                            &ssh_target,
                            session.ssh_prefix.as_deref(),
                            &remote_binary,
                            &["server", "attach", &session.id],
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
        self.active_session_path = Some(key.to_string());
        self.active_view_mode = WorkspaceViewMode::Terminal;
        if launch_now {
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
        );
        if let Some(session) = self.sessions.get_mut(path) {
            session.preview = refreshed.preview;
            session.rendered_sections = refreshed.rendered_sections;
            for entry in refreshed.metadata {
                upsert_session_metadata(&mut session.metadata, entry.label, entry.value);
            }
        }
        Ok(())
    }

    fn refresh_remote_scanned_session_preview(
        &mut self,
        machine_key: &str,
        session_id: &str,
    ) -> anyhow::Result<()> {
        self.refresh_remote_machine_by_key(machine_key)?;
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
            return self.refresh_remote_preview_from_active_session(machine_key, session_id);
        };
        let path = remote_scanned_session_path(machine_key, session_id);
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
                    upsert_session_metadata(&mut session.metadata, "Source", "remote-codex".to_string());
                    upsert_session_metadata(&mut session.metadata, "Host", machine.ssh_target.clone());
                    upsert_session_metadata(&mut session.metadata, "UUID", scanned.session_id.clone());
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
        let path = remote_scanned_session_path(machine_key, session_id);
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
                upsert_session_metadata(&mut session.metadata, "Source", "remote-codex".to_string());
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
        self.upsert_ssh_target(target);
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
        self.insert_live_session(&key, &uuid, target.kind, target, Some(target.label.clone()));
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
        self.ssh_targets
            .sort_by(|left, right| left.label.cmp(&right.label).then_with(|| left.ssh_target.cmp(&right.ssh_target)));
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
            health: RemoteMachineHealth::Cached,
            sessions: Vec::new(),
        });
        self.remote_machines
            .sort_by(|left, right| left.label.cmp(&right.label).then_with(|| left.machine_key.cmp(&right.machine_key)));
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
    ssh_target
        .rsplit('@')
        .next()
        .unwrap_or(ssh_target)
        .trim()
}

fn is_loopback_ssh_target(ssh_target: &str) -> bool {
    matches!(ssh_host_from_target(ssh_target), "localhost" | "127.0.0.1" | "::1")
}

fn machine_key_from_ssh_target(ssh_target: &str) -> String {
    ssh_target
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

fn remote_scanned_session_path(machine_key: &str, session_id: &str) -> String {
    format!("remote-session://{machine_key}/{session_id}")
}

fn parse_remote_scanned_session_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("remote-session://")?;
    let (machine_key, session_id) = rest.split_once('/')?;
    Some((machine_key, session_id))
}

fn remote_resume_shell_command(session_id: &str, cwd: Option<&str>, prefix: Option<&str>) -> String {
    let base = match cwd.filter(|cwd| !cwd.trim().is_empty()) {
        Some(cwd) => format!("cd {} && codex resume {}", shell_single_quote(cwd), shell_single_quote(session_id)),
        None => format!("codex resume {}", shell_single_quote(session_id)),
    };
    match prefix
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(prefix) => format!("{prefix} && {base}"),
        None => base,
    }
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
    let remote = match prefix
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(prefix) => format!("{prefix} && {inner}"),
        None => inner,
    };
    format!("ssh -tt {} {}", ssh_target, shell_single_quote(&remote))
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
struct RemotePreviewPayload {
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
    messages
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|message| {
            format!(
                "{}: {}",
                message.role.display_label(),
                message.lines.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn preview_blocks_from_recent_context(recent_context: &str) -> Vec<SessionPreviewBlock> {
    recent_context
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (role, tone, content) = if let Some(rest) = trimmed.strip_prefix("USER:") {
                ("USER", PreviewTone::User, rest.trim())
            } else if let Some(rest) = trimmed.strip_prefix("ASSISTANT:") {
                ("ASSISTANT", PreviewTone::Assistant, rest.trim())
            } else if let Some(rest) = trimmed.strip_prefix("SYSTEM:") {
                ("SYSTEM", PreviewTone::Assistant, rest.trim())
            } else {
                ("ASSISTANT", PreviewTone::Assistant, trimmed)
            };
            (!content.is_empty()).then(|| SessionPreviewBlock {
                role,
                timestamp: "remote:scan".to_string(),
                tone,
                folded: false,
                lines: vec![content.to_string()],
            })
        })
        .collect()
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
    let preview_blocks = preview_blocks_from_recent_context(&scanned.recent_context);
    if !preview_blocks.is_empty() {
        session.preview.blocks = preview_blocks;
        session.rendered_sections = vec![SessionRenderedSection {
            title: "Recent Context",
            lines: scanned
                .recent_context
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect(),
        }];
    }
    let messages = format!(
        "{} user · {} assistant",
        scanned.user_message_count, scanned.assistant_message_count
    );
    upsert_session_metadata(&mut session.preview.summary, "Session", scanned.session_id.clone());
    upsert_session_metadata(&mut session.preview.summary, "Host", machine_label.to_string());
    upsert_session_metadata(&mut session.preview.summary, "Cwd", scanned.cwd.clone());
    upsert_session_metadata(&mut session.preview.summary, "Started", scanned.started_at.clone());
    upsert_session_metadata(&mut session.preview.summary, "Messages", messages.clone());
    upsert_session_metadata(
        &mut session.preview.summary,
        "Updated",
        modified_epoch_display(scanned.modified_epoch),
    );

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
        blocks: payload
            .preview
            .blocks
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
    session.rendered_sections = payload
        .rendered_sections
        .into_iter()
        .map(|section| SessionRenderedSection {
            title: leak_label(section.title),
            lines: section.lines,
        })
        .collect();
}

fn fetch_remote_preview_payload(
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

fn collect_codex_session_files(
    root: &std::path::Path,
    out: &mut Vec<std::path::PathBuf>,
) -> anyhow::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).with_context(|| format!("reading codex session dir {}", root.display()))? {
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
    let stat = fs::metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?;
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
        rendered_sections: session
            .rendered_sections
            .into_iter()
            .map(|section| SnapshotRenderedSection {
                title: section.title.to_string(),
                lines: section.lines,
            })
            .collect(),
    }))
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
    def normalize_lines(text):
        if not text:
            return []
        if isinstance(text, str):
            return [line.strip() for line in text.splitlines() if line.strip()]
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
        if role in ("user", "developer"):
            label = "USER"
        elif role == "assistant":
            label = "ASSISTANT"
        else:
            label = "SYSTEM"
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
                        if role in ("user", "developer"):
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
                        if role in ("user", "developer"):
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

codex_home = os.path.expanduser(sys.argv[1] if len(sys.argv) > 1 else "~/.codex")
root = Path(codex_home) / "sessions"
if root.exists():
    for path in root.rglob("*.jsonl"):
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
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open remote metadata mirror {}", db_path.display()))?;
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
    dedupe_remote_scanned_sessions(sessions
        .iter()
        .cloned()
        .map(|mut session| {
            if let Some(mirrored) = mirrored_by_id.get(&session.session_id) {
                if !mirrored.title_hint.trim().is_empty() {
                    session.title_hint = mirrored.title_hint.clone();
                }
                if session.recent_context.trim().is_empty() && !mirrored.recent_context.trim().is_empty() {
                    session.recent_context = mirrored.recent_context.clone();
                }
                if mirrored.cached_precis.as_ref().is_some_and(|value| !value.trim().is_empty()) {
                    session.cached_precis = mirrored.cached_precis.clone();
                }
                if mirrored.cached_summary.as_ref().is_some_and(|value| !value.trim().is_empty()) {
                    session.cached_summary = mirrored.cached_summary.clone();
                }
            }
            session
        })
        .collect())
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
                let mut merged = if newer { session.clone() } else { existing.clone() };
                let other = if newer { existing.clone() } else { session };

                if looks_like_generated_fallback_title(&merged.title_hint)
                    && !looks_like_generated_fallback_title(&other.title_hint)
                {
                    merged.title_hint = other.title_hint;
                }
                if merged.cached_precis.as_ref().is_none_or(|value| value.trim().is_empty()) {
                    merged.cached_precis = other.cached_precis;
                }
                if merged.cached_summary.as_ref().is_none_or(|value| value.trim().is_empty()) {
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

fn remote_cache_key(ssh_target: &str, exec_prefix: Option<&str>) -> String {
    format!("{}|{}", ssh_target, exec_prefix.unwrap_or_default())
}

fn remote_command_cache() -> &'static Mutex<std::collections::HashMap<String, String>> {
    REMOTE_YGGTERM_COMMAND_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
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
    let mut inner = String::from(binary_expr);
    for arg in args {
        inner.push(' ');
        inner.push_str(&shell_single_quote(arg));
    }
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
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_remote_yggterm_command(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    args: &[&str],
    stdin_bytes: Option<&[u8]>,
) -> anyhow::Result<String> {
    let resolved = resolve_remote_yggterm_binary(ssh_target, exec_prefix)?;
    run_remote_binary_command(
        ssh_target,
        exec_prefix,
        &resolved.0,
        args,
        stdin_bytes,
    )
}

fn should_fallback_to_python(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}");
    message.contains("command not found")
        || message.contains("not found")
        || message.contains("No such file")
        || message.contains("remote yggterm command failed")
}

fn remote_protocol_version_for_binary(
    ssh_target: &str,
    exec_prefix: Option<&str>,
    binary_expr: &str,
) -> anyhow::Result<String> {
    run_remote_binary_command(
        ssh_target,
        exec_prefix,
        binary_expr,
        &["server", "remote", "protocol-version"],
        None,
    )
}

fn bootstrap_remote_yggterm(ssh_target: &str, exec_prefix: Option<&str>) -> anyhow::Result<String> {
    let exe_path = local_remote_bootstrap_executable()
        .or_else(|| std::env::current_exe().ok())
        .context("resolving local yggterm remote executable")?;
    let payload =
        fs::read(&exe_path).with_context(|| format!("reading local yggterm binary {}", exe_path.display()))?;
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
    if let Some(stdin) = child.stdin.as_mut() {
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
    let headless = current.with_file_name("yggterm-headless");
    if headless.is_file() {
        return Some(headless);
    }
    None
}

fn resolve_remote_yggterm_binary(
    ssh_target: &str,
    exec_prefix: Option<&str>,
) -> anyhow::Result<(String, RemoteDeployState)> {
    let perf_home = resolve_yggterm_home().ok();
    let perf_span = perf_home
        .as_ref()
        .map(|home| PerfSpan::start(home.clone(), "remote", "resolve_yggterm_binary"));
    let cache_key = remote_cache_key(ssh_target, exec_prefix);
    if let Some(cached) = remote_command_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&cache_key).cloned())
    {
        if let Some(span) = perf_span {
            span.finish(serde_json::json!({
                "ssh_target": ssh_target,
                "result": "cache_hit",
                "binary_expr": cached.clone(),
            }));
        }
        return Ok((cached, RemoteDeployState::Ready));
    }

    match remote_protocol_version_for_binary(ssh_target, exec_prefix, "yggterm") {
        Ok(version) if version.trim() == daemon::SERVER_PROTOCOL_VERSION => {
            if let Ok(mut cache) = remote_command_cache().lock() {
                cache.insert(cache_key, "yggterm".to_string());
            }
            if let Some(span) = perf_span {
                span.finish(serde_json::json!({
                    "ssh_target": ssh_target,
                    "result": "path_match",
                    "binary_expr": "yggterm",
                }));
            }
            return Ok(("yggterm".to_string(), RemoteDeployState::Ready));
        }
        Ok(_) => {}
        Err(error) if !should_fallback_to_python(&error) => return Err(error),
        Err(_) => {}
    }

    let installed_binary = "$HOME/.yggterm/bin/yggterm";
    match remote_protocol_version_for_binary(ssh_target, exec_prefix, installed_binary) {
        Ok(version) if version.trim() == daemon::SERVER_PROTOCOL_VERSION => {
            if let Ok(mut cache) = remote_command_cache().lock() {
                cache.insert(cache_key, installed_binary.to_string());
            }
            if let Some(span) = perf_span {
                span.finish(serde_json::json!({
                    "ssh_target": ssh_target,
                    "result": "installed_path_match",
                    "binary_expr": installed_binary,
                }));
            }
            return Ok((installed_binary.to_string(), RemoteDeployState::Ready));
        }
        Ok(_) => {}
        Err(error) if !should_fallback_to_python(&error) => return Err(error),
        Err(_) => {}
    }

    let installed = bootstrap_remote_yggterm(ssh_target, exec_prefix)?;
    let installed_version =
        remote_protocol_version_for_binary(ssh_target, exec_prefix, &installed)?;
    if installed_version.trim() != daemon::SERVER_PROTOCOL_VERSION {
        anyhow::bail!(
            "remote yggterm protocol mismatch for {}: expected {}, got {}",
            ssh_target,
            daemon::SERVER_PROTOCOL_VERSION,
            installed_version.trim()
        );
    }
    if let Ok(mut cache) = remote_command_cache().lock() {
        cache.insert(cache_key, installed.clone());
    }
    if let Some(span) = perf_span {
        span.finish(serde_json::json!({
            "ssh_target": ssh_target,
            "result": "bootstrapped",
            "binary_expr": installed,
        }));
    }
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
    let remote = match exec_prefix.map(str::trim).filter(|prefix| !prefix.is_empty()) {
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
    let lines = match run_remote_yggterm_command(
        &target.ssh_target,
        target.prefix.as_deref(),
        &["server", "remote", "scan", "~/.codex"],
        None,
    ) {
        Ok(output) => output.lines().map(str::to_string).collect::<Vec<_>>(),
        Err(error) if !should_fallback_to_python(&error) => return Err(error),
        Err(_) => run_remote_python_lines(
            &target.ssh_target,
            target.prefix.as_deref(),
            REMOTE_SCAN_SCRIPT,
            &[String::from("~/.codex")],
        )?,
    };
    let machine_key = machine_key_from_ssh_target(&target.ssh_target);
    let mut sessions = Vec::new();
    for line in lines {
        let summary: RemoteSummaryLine =
            serde_json::from_str(&line).context("invalid remote summary line")?;
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
            recent_context: summary.recent_context,
            cached_precis: summary.cached_precis.filter(|value| !value.trim().is_empty()),
            cached_summary: summary.cached_summary.filter(|value| !value.trim().is_empty()),
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
    println!("{}", daemon::SERVER_PROTOCOL_VERSION);
    Ok(())
}

pub fn run_remote_resume_codex(session_id: &str, cwd: Option<&str>) -> anyhow::Result<()> {
    let command = remote_resume_shell_command(session_id, cwd, None);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new("sh").arg("-lc").arg(command).exec();
        Err(anyhow::anyhow!("failed to exec remote codex resume: {error}"))
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
            Err(anyhow::anyhow!("remote codex resume exited with status {status}"))
        }
    }
}

pub fn run_remote_scan(codex_home: Option<&str>) -> anyhow::Result<()> {
    let home = codex_home
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
    let root = home.join("sessions");
    let yggterm_home = resolve_yggterm_home()?;
    let title_store = SessionTitleStore::open(&yggterm_home)?;
    let mut files = Vec::new();
    collect_codex_session_files(&root, &mut files)?;
    files.sort();
    for path in files {
        if let Some(summary) = remote_summary_for_path(&path, &title_store)? {
            println!("{}", serde_json::to_string(&summary)?);
        }
    }
    Ok(())
}

pub fn run_remote_preview(path: &str) -> anyhow::Result<()> {
    let yggterm_home = resolve_yggterm_home()?;
    let title_store = SessionTitleStore::open(&yggterm_home)?;
    let payload = remote_preview_payload_for_path(std::path::Path::new(path), &title_store)?
        .with_context(|| format!("no previewable codex session at {path}"))?;
    println!("{}", serde_json::to_string(&payload)?);
    Ok(())
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
    if let Some(title) = payload.title.as_deref().filter(|value| !value.trim().is_empty()) {
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
    if let Some(precis) = payload.precis.as_deref().filter(|value| !value.trim().is_empty()) {
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
    if let Some(summary) = payload.summary.as_deref().filter(|value| !value.trim().is_empty()) {
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
            blocks: session
                .preview
                .blocks
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
            blocks: session
                .preview
                .blocks
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
    let transcript = if kind == SessionKind::Document {
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
                format!(
                    "cd {} && {} -l",
                    shell_single_quote(&default_cwd),
                    shell_program
                ),
                format!("local://{uuid}"),
                "local-shell".to_string(),
                default_cwd.clone(),
                shell_program.clone(),
                RemoteDeployState::NotRequired,
            ),
            SessionKind::Codex => (
                format!("cd {} && codex", shell_single_quote(&default_cwd)),
                format!("codex://{uuid}"),
                "local-codex".to_string(),
                default_cwd.clone(),
                "codex".to_string(),
                RemoteDeployState::NotRequired,
            ),
            SessionKind::CodexLiteLlm => (
                format!(
                    "cd {} && CODEX_HOME=\"$HOME/.codex-litellm\" codex-litellm",
                    shell_single_quote(&default_cwd)
                ),
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
                    &["server", "attach", uuid],
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
    let (remote_binary, remote_deploy_state) =
        resolve_remote_yggterm_binary(&target.ssh_target, target.prefix.as_deref())
            .unwrap_or_else(|_| ("yggterm".to_string(), RemoteDeployState::Planned));
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
        format!(
            "yggterm server remote resume-codex {}",
            scanned.session_id
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

fn stored_session_launch_command(kind: SessionKind, cwd: &str, session_id: &str) -> String {
    match kind {
        SessionKind::Codex => format!(
            "cd {} && codex resume {}",
            shell_single_quote(cwd),
            session_id
        ),
        SessionKind::CodexLiteLlm => format!(
            "cd {} && CODEX_HOME=\"$HOME/.codex-litellm\" codex-litellm resume {}",
            shell_single_quote(cwd),
            session_id
        ),
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
        GhosttyHostSupport, RemoteMachineHealth, RemoteMachineSnapshot, RemoteScannedSession,
        PersistedDaemonState, SessionKind, SessionNode, SessionNodeKind, SshConnectTarget,
        UiTheme, WorkspaceViewMode, YggtermServer,
        dedupe_remote_scanned_sessions, load_remote_machine_sessions_from_mirror,
        mirror_remote_machine_sessions,
        parse_stored_transcript, remote_resume_shell_command, remote_scanned_session_path,
        remote_ssh_launch_command, remote_command_cache, remote_cache_key,
        stored_session_launch_command,
    };
    use anyhow::Result;
    use std::fs;
    use std::path::PathBuf;

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
        assert!(command.contains("CODEX_HOME=\"$HOME/.codex-litellm\""));
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
        assert!(command.contains("cd '/srv/workspace' && codex resume"));
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
        assert!(command.starts_with("ssh -tt jojo "));
        assert!(command.contains("tmux new-session -A -s yggterm &&"));
        assert!(command.contains("$HOME/.yggterm/bin/yggterm"));
        assert!(command.contains("'resume-codex'"));
        assert!(command.contains("'/srv/app'"));
    }

    #[test]
    fn reopening_remote_scanned_session_refreshes_stale_launch_command() -> Result<()> {
        if let Ok(mut cache) = remote_command_cache().lock() {
            cache.insert(remote_cache_key("jojo", None), "yggterm".to_string());
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
            .launch_command = "ssh jojo 'yggterm server attach 019d09a4-c69e-7071-bd9a-8834060029a9'"
            .to_string();

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
}
