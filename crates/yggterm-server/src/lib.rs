mod attach;
mod daemon;
mod host;
mod terminal;

pub use attach::{AttachMetadata, run_attach};
pub use daemon::{
    ServerEndpoint, ServerRequest, ServerResponse, ServerRuntimeStatus, TerminalStreamChunk,
    connect_ssh,
    default_endpoint, focus_live, open_stored_session, ping, raise_external_window, run_daemon,
    request_terminal_launch, set_all_preview_blocks_folded, set_view_mode, snapshot,
    start_command_session, start_local_session, start_local_session_at, status,
    shutdown, sync_external_window, sync_theme, terminal_ensure, terminal_read, terminal_resize,
    terminal_write, toggle_preview_block,
};
pub use host::{
    GhosttyHostKind, GhosttyHostSupport, GhosttyTerminalHostMode, detect_ghostty_host,
};
pub use terminal::{TerminalChunk, TerminalManager, TerminalReadResult};

use yggterm_core::{
    SessionNode, SessionNodeKind, SessionStore, TranscriptRole, UiTheme, WorkspaceDocument,
    WorkspaceDocumentKind, read_codex_transcript_messages,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::time::SystemTime;
use time::{OffsetDateTime, UtcOffset, macros::format_description};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceViewMode {
    Terminal,
    Rendered,
}

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
    pub active_session_path: Option<String>,
    pub active_session: Option<SnapshotSessionView>,
    pub active_view_mode: WorkspaceViewMode,
    pub ssh_targets: Vec<SshConnectTarget>,
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
    pub active_session_path: Option<String>,
    pub active_view_mode: WorkspaceViewMode,
    pub stored_sessions: Vec<PersistedStoredSession>,
    pub live_sessions: Vec<PersistedLiveSession>,
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
            ssh_targets: vec![
                SshConnectTarget {
                    label: "prod-app-01".to_string(),
                    kind: SessionKind::SshShell,
                    ssh_target: "prod-app-01".to_string(),
                    prefix: Some("sudo machinectl shell prod".to_string()),
                    cwd: None,
                },
                SshConnectTarget {
                    label: "design-01".to_string(),
                    kind: SessionKind::SshShell,
                    ssh_target: "design-01".to_string(),
                    prefix: None,
                    cwd: None,
                },
                SshConnectTarget {
                    label: "ghostty-admin".to_string(),
                    kind: SessionKind::SshShell,
                    ssh_target: "ghostty-admin".to_string(),
                    prefix: Some("tmux new-session -A -s yggterm".to_string()),
                    cwd: None,
                },
                SshConnectTarget {
                    label: "local-shell".to_string(),
                    kind: SessionKind::Shell,
                    ssh_target: "localhost".to_string(),
                    prefix: None,
                    cwd: dirs::home_dir().map(|path| path.display().to_string()),
                },
            ],
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

    pub fn live_sessions(&self) -> Vec<ManagedSessionView> {
        self.live_session_order
            .iter()
            .filter_map(|key| self.sessions.get(key).cloned())
            .collect()
    }

    pub fn snapshot(&self) -> ServerUiSnapshot {
        ServerUiSnapshot {
            active_session_path: self.active_session_path.clone(),
            active_session: self.active_session().cloned().map(snapshot_session_view),
            active_view_mode: self.active_view_mode,
            ssh_targets: self.ssh_targets.clone(),
            live_sessions: self
                .live_session_order
                .iter()
                .filter_map(|key| self.sessions.get(key))
                .cloned()
                .map(snapshot_session_view)
                .collect(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: ServerUiSnapshot) {
        self.active_view_mode = snapshot.active_view_mode;
        self.active_session_path = snapshot.active_session_path.clone();
        self.ssh_targets = snapshot.ssh_targets;
        self.live_session_order = snapshot
            .live_sessions
            .iter()
            .map(|session| session.session_path.clone())
            .collect();
        self.sessions.clear();
        if let Some(active) = snapshot.active_session {
            let key = active.session_path.clone();
            self.sessions.insert(key, managed_session_from_snapshot(active));
        }
        for live in snapshot.live_sessions {
            let key = live.session_path.clone();
            self.sessions.insert(key, managed_session_from_snapshot(live));
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
                session.ssh_target.as_ref().map(|ssh_target| PersistedLiveSession {
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
            stored_sessions,
            live_sessions,
        }
    }

    pub fn restore_persisted_state(&mut self, state: PersistedDaemonState, store: Option<&SessionStore>) {
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
        for live in state.live_sessions {
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
                .find(|(_, session)| session.source == SessionSource::LiveSsh && session.session_path == key)
                .map(|(session_key, _)| session_key.clone())
        };
        if let Some(resolved_key) = resolved_key {
            self.active_session_path = Some(resolved_key);
            self.active_view_mode = WorkspaceViewMode::Terminal;
            self.request_terminal_launch_for_active();
        }
    }

    pub fn connect_ssh_target(&mut self, target_ix: usize) -> Option<String> {
        let target = self.ssh_targets.get(target_ix)?.clone();
        let uuid = Uuid::new_v4().to_string();
        let key = match target.kind {
            SessionKind::Shell => format!("local::{uuid}"),
            _ => format!("live::{uuid}"),
        };
        self.insert_live_session(&key, &uuid, target.kind, &target, Some(target.label.clone()));
        Some(key)
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
        self.insert_live_session(&key, &uuid, SessionKind::Shell, &target, Some(title.clone()));
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
            label: live.title.clone(),
            kind: live.kind,
            ssh_target: live.ssh_target,
            prefix: live.prefix,
            cwd: live.cwd,
        };
        self.insert_live_session(&live.key, &live.id, live.kind, &target, Some(live.title));
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
                upsert_session_metadata(&mut session.metadata, "Launch PID", "daemon pty".to_string());
                upsert_session_metadata(&mut session.metadata, "Launch Error", "none".to_string());
                upsert_session_metadata(&mut session.metadata, "Ghostty Window", "not used".to_string());
                upsert_session_metadata(
                    &mut session.metadata,
                    "Host Mode",
                    "embedded xterm.js".to_string(),
                );
                upsert_session_metadata(&mut session.metadata, "Host Token", "daemon".to_string());
                upsert_session_metadata(&mut session.metadata, "Embedded Surface", "webview".to_string());
                upsert_session_metadata(&mut session.metadata, "Embedded Host", "xterm.js".to_string());
                upsert_session_metadata(&mut session.metadata, "Window Error", "none".to_string());
                upsert_session_metadata(&mut session.metadata, "Status", "recipe running".to_string());
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
                upsert_session_metadata(
                    &mut session.metadata,
                    "Launch Error",
                    "none".to_string(),
                );
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
                upsert_session_metadata(
                    &mut session.metadata,
                    "Host Token",
                    "daemon".to_string(),
                );
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
                upsert_session_metadata(
                    &mut session.metadata,
                    "Window Error",
                    "none".to_string(),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Status",
                    "running".to_string(),
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
            SessionSource::LiveSsh => {
                session.backend = TerminalBackend::Xterm;
                session.remote_deploy_state = if session.kind == SessionKind::SshShell {
                    RemoteDeployState::Ready
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
        self.request_terminal_launch_for_active();
    }
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
    let launch_command = match kind {
        SessionKind::Document => "document preview".to_string(),
        _ => format!(
            "cd '{}' && codex resume {}",
            cwd.replace('\'', "'\\''"),
            session_id
        ),
    };
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
            label: if kind == SessionKind::Document { "Document" } else { "Session" },
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
            label: if kind == SessionKind::Document { "Document" } else { "Session" },
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
                format!("$ codex resume {session_id}")
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
    let default_cwd =
        target.cwd.clone().unwrap_or_else(|| local_default_cwd());
    let remote_command = match &target.prefix {
        Some(prefix) => format!("{prefix} && yggterm server attach {uuid}"),
        None => format!("yggterm server attach {uuid}"),
    };
    let (launch_command, session_path, source_value, target_value, prefix_value, deploy_state) =
        match kind {
            SessionKind::Shell => (
                format!("cd {} && {} -l", shell_single_quote(&default_cwd), shell_program),
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
                format!("ssh {} {}", target.ssh_target, shell_single_quote(&remote_command)),
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
            "Remote bootstrap will eventually ship the yggterm binary before attach.".to_string()
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
    upsert_session_metadata(&mut session.metadata, "Storage", document.virtual_path.clone());
    upsert_session_metadata(&mut session.metadata, "Updated", document.updated_at.clone());
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

fn local_default_cwd() -> String {
    dirs::home_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "/".to_string())
}

fn local_session_target(kind: SessionKind, cwd: Option<&str>) -> SshConnectTarget {
    let cwd = Some(
        cwd.map(ToOwned::to_owned)
            .unwrap_or_else(local_default_cwd),
    );
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
    if role == TranscriptRole::Assistant && blocks.is_empty() && looks_like_session_metadata_block(&lines) {
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
    use super::parse_stored_transcript;
    use anyhow::Result;
    use std::fs;

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
}
