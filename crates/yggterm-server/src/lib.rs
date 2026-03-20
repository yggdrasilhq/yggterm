mod attach;
mod daemon;
mod host;

pub use attach::{AttachMetadata, run_attach};
pub use daemon::{
    ServerEndpoint, ServerRequest, ServerResponse, ServerRuntimeStatus, connect_ssh,
    default_endpoint, focus_live, open_stored_session, ping, raise_external_window, run_daemon,
    request_terminal_launch, set_all_preview_blocks_folded, set_view_mode, snapshot, status,
    sync_external_window, sync_theme, toggle_preview_block,
};
pub use host::{
    GhosttyHostKind, GhosttyHostSupport, GhosttyTerminalHostMode, detect_ghostty_host,
};

use yggterm_core::{SessionNode, TranscriptRole, UiTheme, read_codex_transcript_messages};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
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
    pub ssh_target: String,
    pub prefix: Option<String>,
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
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub title_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedLiveSession {
    pub key: String,
    pub id: String,
    pub title: String,
    pub ssh_target: String,
    pub prefix: Option<String>,
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
        let ghostty_bridge_enabled = ghostty_host.bridge_enabled;
        let backend = if prefer_ghostty_backend && ghostty_bridge_enabled {
            TerminalBackend::Ghostty
        } else {
            TerminalBackend::Mock
        };

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
                    ssh_target: "prod-app-01".to_string(),
                    prefix: Some("sudo machinectl shell prod".to_string()),
                },
                SshConnectTarget {
                    label: "design-01".to_string(),
                    ssh_target: "design-01".to_string(),
                    prefix: None,
                },
                SshConnectTarget {
                    label: "ghostty-admin".to_string(),
                    ssh_target: "ghostty-admin".to_string(),
                    prefix: Some("tmux new-session -A -s yggterm".to_string()),
                },
            ],
            live_session_order: Vec::new(),
        };

        if let Some(first_session) = first_session_leaf(tree) {
            this.open_or_focus_session(
                &first_session.path,
                Some(&first_session.session_id),
                Some(&first_session.cwd),
                Some(&first_session.title),
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
        path: &str,
        session_id: Option<&str>,
        cwd: Option<&str>,
        title_hint: Option<&str>,
    ) {
        let entry = self.sessions.entry(path.to_string()).or_insert_with(|| {
            build_session(
                path,
                session_id,
                cwd,
                title_hint,
                self.backend,
                self.theme,
                self.ghostty_host.bridge_enabled,
            )
        });
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
                    ssh_target: ssh_target.clone(),
                    prefix: session.ssh_prefix.clone(),
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

    pub fn restore_persisted_state(&mut self, state: PersistedDaemonState) {
        for session in state.stored_sessions {
            self.open_or_focus_session(
                &session.path,
                session.session_id.as_deref(),
                session.cwd.as_deref(),
                session.title_hint.as_deref(),
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
        if self.sessions.contains_key(key) {
            self.active_session_path = Some(key.to_string());
            self.active_view_mode = WorkspaceViewMode::Terminal;
            self.request_terminal_launch_for_active();
        }
    }

    pub fn connect_ssh_target(&mut self, target_ix: usize) -> Option<String> {
        let target = self.ssh_targets.get(target_ix)?.clone();
        let uuid = Uuid::new_v4().to_string();
        let key = format!("live::{uuid}");
        self.insert_live_session(&key, &uuid, &target, Some(target.label.clone()));
        Some(key)
    }

    pub fn restore_live_session(&mut self, live: PersistedLiveSession) {
        let target = SshConnectTarget {
            label: live.title.clone(),
            ssh_target: live.ssh_target,
            prefix: live.prefix,
        };
        self.insert_live_session(&live.key, &live.id, &target, Some(live.title));
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

        match session.source {
            SessionSource::Stored => {
                if session.terminal_process_id.is_none() {
                    match self.ghostty_host.launch_terminal(&session.launch_command) {
                        Ok(outcome) => {
                            session.backend = TerminalBackend::Ghostty;
                            session.terminal_process_id = outcome.process_id;
                            session.terminal_window_id = None;
                            session.terminal_host_token = outcome.host_token;
                            session.terminal_host_mode = outcome.host_mode;
                            session.embedded_surface_id = outcome.embedded_surface_id;
                            session.embedded_surface_detail = outcome.embedded_surface_detail;
                            session.last_launch_error = None;
                            session.last_window_error = None;
                            session.launch_phase = if outcome.embedded_surface_reserved {
                                TerminalLaunchPhase::BridgePending
                            } else {
                                TerminalLaunchPhase::Running
                            };
                            session.terminal_lines = outcome.lines;
                            session
                                .terminal_lines
                                .push(self.ghostty_host.integration_note());
                            if session.terminal_process_id.is_some() {
                                let _ = sync_external_window_id(session);
                            }
                        }
                        Err(error) => {
                            session.backend = TerminalBackend::Mock;
                            session.last_launch_error = Some(error.clone());
                            session.launch_phase = TerminalLaunchPhase::BridgePending;
                            session
                                .terminal_lines
                                .push(format!("ghostty launch error: {error}"));
                        }
                    }
                }
                upsert_session_metadata(
                    &mut session.metadata,
                    "Backend",
                    match session.backend {
                        TerminalBackend::Ghostty => "Ghostty".to_string(),
                        TerminalBackend::Mock => "Mock".to_string(),
                    },
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Launch PID",
                    session
                        .terminal_process_id
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "not launched".to_string()),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Launch Error",
                    session
                        .last_launch_error
                        .clone()
                        .unwrap_or_else(|| "none".to_string()),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Ghostty Window",
                    session
                        .terminal_window_id
                        .clone()
                        .unwrap_or_else(|| "not resolved".to_string()),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Host Mode",
                    describe_terminal_host_mode(session.terminal_host_mode).to_string(),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Host Token",
                    session
                        .terminal_host_token
                        .clone()
                        .unwrap_or_else(|| "none".to_string()),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Embedded Surface",
                    session
                        .embedded_surface_id
                        .clone()
                        .unwrap_or_else(|| "none".to_string()),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Embedded Host",
                    session
                        .embedded_surface_detail
                        .clone()
                        .unwrap_or_else(|| "none".to_string()),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Window Error",
                    session
                        .last_window_error
                        .clone()
                        .unwrap_or_else(|| "none".to_string()),
                );
                upsert_session_metadata(
                    &mut session.metadata,
                    "Status",
                    if session.terminal_process_id.is_some() {
                        "running".to_string()
                    } else {
                        "launch failed".to_string()
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
            SessionSource::LiveSsh => {
                session.remote_deploy_state = if session.bridge_available {
                    RemoteDeployState::Ready
                } else {
                    RemoteDeployState::CopyingBinary
                };
                session.launch_phase = if session.bridge_available {
                    TerminalLaunchPhase::Running
                } else {
                    TerminalLaunchPhase::BridgePending
                };
                session.terminal_lines = build_live_terminal_lines(session);
                upsert_session_metadata(
                    &mut session.metadata,
                    "Status",
                    if session.bridge_available {
                        "remote ready".to_string()
                    } else {
                        "copying binary".to_string()
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
        let Some(path) = self.active_session_path.as_ref() else {
            return "no active session".to_string();
        };
        let Some(session) = self.sessions.get_mut(path) else {
            return "no active session".to_string();
        };
        match sync_external_window_id(session) {
            Ok(window_id) => format!("ghostty window resolved: {window_id}"),
            Err(error) => error,
        }
    }

    pub fn raise_external_terminal_window_for_active(&mut self) -> String {
        let Some(path) = self.active_session_path.as_ref() else {
            return "no active session".to_string();
        };
        let Some(session) = self.sessions.get_mut(path) else {
            return "no active session".to_string();
        };

        if session.terminal_window_id.is_none() {
            let _ = sync_external_window_id(session);
        }

        let Some(window_id) = session.terminal_window_id.clone() else {
            return session
                .last_window_error
                .clone()
                .unwrap_or_else(|| "ghostty window not resolved".to_string());
        };

        match raise_x11_window(&window_id) {
            Ok(()) => {
                session.last_window_error = None;
                upsert_session_metadata(&mut session.metadata, "Window Error", "none".to_string());
                format!("ghostty window raised: {window_id}")
            }
            Err(error) => {
                session.last_window_error = Some(error.clone());
                upsert_session_metadata(&mut session.metadata, "Window Error", error.clone());
                error
            }
        }
    }

    fn insert_live_session(
        &mut self,
        key: &str,
        session_id: &str,
        target: &SshConnectTarget,
        title_override: Option<String>,
    ) {
        let mut session = build_live_session(
            session_id,
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
    if node.children.is_empty() {
        return Some(StoredLeaf {
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
    path: &str,
    session_id: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
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
        TerminalBackend::Ghostty => "Ghostty",
        TerminalBackend::Mock => "Mock",
    };
    let cwd = cwd
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| session_preview_cwd(path));
    let launch_command = format!(
        "cd '{}' && codex resume {}",
        cwd.replace('\'', "'\\''"),
        session_id
    );
    let transcript = parse_stored_transcript(path, &started_at);
    let file_snapshot = stored_file_snapshot(path);
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
            label: "Session",
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
                            format!("Resume Codex session {session_id}."),
                            format!("Open the workspace rooted at {cwd}."),
                        ],
                    },
                    SessionPreviewBlock {
                        role: "ASSISTANT",
                        timestamp: "server:restore".to_string(),
                        tone: PreviewTone::Assistant,
                        folded: false,
                        lines: vec![
                            format!("{backend_label} backend reserved for the live terminal surface."),
                            "Rendered preview follows the session transcript first and tool activity second.".to_string(),
                            format!("Terminal launch command: {launch_command}"),
                        ],
                    },
                ]
            }),
    };

    let mut metadata = vec![
        SessionMetadataEntry {
            label: "Source",
            value: "stored".to_string(),
        },
        SessionMetadataEntry {
            label: "Host",
            value: host_label.clone(),
        },
        SessionMetadataEntry {
            label: "Session",
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
            value: if ghostty_bridge_enabled {
                "available".to_string()
            } else {
                "not linked".to_string()
            },
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
        host_label: host_label.clone(),
        source: SessionSource::Stored,
        backend,
        bridge_available: ghostty_bridge_enabled,
        launch_phase: if ghostty_bridge_enabled {
            TerminalLaunchPhase::Queued
        } else {
            TerminalLaunchPhase::BridgePending
        },
        remote_deploy_state: RemoteDeployState::NotRequired,
        launch_command: launch_command.clone(),
        status_line: describe_status_line(
            backend,
            theme,
            SessionSource::Stored,
            if ghostty_bridge_enabled {
                TerminalLaunchPhase::Queued
            } else {
                TerminalLaunchPhase::BridgePending
            },
            RemoteDeployState::NotRequired,
            ghostty_bridge_enabled,
        ),
        terminal_lines: vec![
            format!("$ cd {cwd}"),
            format!("$ codex resume {session_id}"),
            format!("Ghostty terminal host: {backend_label}"),
            "yggterm server launches ghostty for terminal mode".to_string(),
            format!(
                "Host strategy: {}",
                describe_terminal_host_mode(ghostty_host_mode(backend, ghostty_bridge_enabled))
            ),
            embedded_surface_note(ghostty_bridge_enabled),
        ],
        rendered_sections: vec![
            SessionRenderedSection {
                title: "Rendered Session",
                lines: vec![
                    "Preview mode renders the stored Codex transcript as a chat surface.".to_string(),
                    "Turn Preview off in the titlebar to hand the main viewport back to Ghostty.".to_string(),
                    "The terminal/server session stays authoritative underneath.".to_string(),
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
    target: &SshConnectTarget,
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
) -> ManagedSessionView {
    let started_at = format_display_datetime(OffsetDateTime::now_utc());
    let remote_command = match &target.prefix {
        Some(prefix) => format!("{prefix} && yggterm server attach {uuid}"),
        None => format!("yggterm server attach {uuid}"),
    };
    let launch_command = format!(
        "ssh {} {}",
        target.ssh_target,
        shell_single_quote(&remote_command)
    );

    ManagedSessionView {
        id: uuid.to_string(),
        session_path: format!("ssh://{}/{}", target.ssh_target, uuid),
        title: uuid.to_string(),
        host_label: target.label.clone(),
        source: SessionSource::LiveSsh,
        backend,
        bridge_available: ghostty_bridge_enabled,
        launch_phase: TerminalLaunchPhase::RemoteBootstrap,
        remote_deploy_state: RemoteDeployState::Planned,
        launch_command: launch_command.clone(),
        status_line: describe_status_line(
            backend,
            theme,
            SessionSource::LiveSsh,
            TerminalLaunchPhase::RemoteBootstrap,
            RemoteDeployState::Planned,
            ghostty_bridge_enabled,
        ),
        terminal_lines: vec![
            format!("$ {launch_command}"),
            format!("Queue live SSH session {uuid}"),
            format!("Target: {}", target.ssh_target),
            format!(
                "Prefix: {}",
                target.prefix.clone().unwrap_or_else(|| "none".to_string())
            ),
            "Remote bootstrap: copy yggterm binary if missing".to_string(),
            "Ghostty bridge: request main viewport surface".to_string(),
            format!(
                "Host strategy: {}",
                describe_terminal_host_mode(ghostty_host_mode(backend, ghostty_bridge_enabled))
            ),
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
                    value: target.ssh_target.clone(),
                },
                SessionMetadataEntry {
                    label: "Prefix",
                    value: target.prefix.clone().unwrap_or_else(|| "none".to_string()),
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
                        "This session should land in the main viewport as a Ghostty-backed terminal.".to_string(),
                    ],
                },
                SessionPreviewBlock {
                    role: "ASSISTANT",
                    timestamp: "server:launch".to_string(),
                    tone: PreviewTone::Assistant,
                    folded: false,
                    lines: vec![
                        format!("Launch command prepared: {launch_command}"),
                        "Remote bootstrap will eventually ship the yggterm binary before attach.".to_string(),
                    ],
                },
            ],
        },
        metadata: vec![
            SessionMetadataEntry {
                label: "Source",
                value: "live-ssh".to_string(),
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
                value: target.ssh_target.clone(),
            },
            SessionMetadataEntry {
                label: "Prefix",
                value: target.prefix.clone().unwrap_or_else(|| "none".to_string()),
            },
            SessionMetadataEntry {
                label: "Deploy",
                value: "planned".to_string(),
            },
            SessionMetadataEntry {
                label: "Launch PID",
                value: "remote".to_string(),
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

fn describe_status_line(
    backend: TerminalBackend,
    theme: UiTheme,
    source: SessionSource,
    launch_phase: TerminalLaunchPhase,
    remote_deploy_state: RemoteDeployState,
    bridge_available: bool,
) -> String {
    let backend_label = match backend {
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
    let bridge = if bridge_available {
        "bridge ready"
    } else {
        "bridge missing"
    };
    match (source, launch_phase, remote_deploy_state) {
        (SessionSource::Stored, TerminalLaunchPhase::Queued, _) => {
            format!("stored attach queued · {bridge}")
        }
        (SessionSource::Stored, TerminalLaunchPhase::BridgePending, _) => {
            format!("stored attach pending bridge · {bridge}")
        }
        (SessionSource::Stored, TerminalLaunchPhase::Running, _) => {
            format!("stored terminal requested · {bridge}")
        }
        (SessionSource::LiveSsh, _, RemoteDeployState::Planned) => {
            format!("remote bootstrap planned · {bridge}")
        }
        (SessionSource::LiveSsh, _, RemoteDeployState::CopyingBinary) => {
            format!("copying yggterm binary · {bridge}")
        }
        (SessionSource::LiveSsh, TerminalLaunchPhase::Running, RemoteDeployState::Ready) => {
            format!("ghostty session requested · {bridge}")
        }
        (SessionSource::LiveSsh, TerminalLaunchPhase::BridgePending, _) => {
            format!("waiting for ghostty bridge · {bridge}")
        }
        _ => format!("launch queued · {bridge}"),
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
        format!("Launching live SSH session {}", session.id),
        format!("Target: {}", session.host_label),
        format!("Deploy state: {deploy}"),
        format!("Launch phase: {launch}"),
        format!(
            "Ghostty bridge: {}",
            if session.bridge_available {
                "available"
            } else {
                "not linked in this build"
            }
        ),
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

fn sync_external_window_id(session: &mut ManagedSessionView) -> Result<String, String> {
    let Some(pid) = session.terminal_process_id else {
        let error = "ghostty pid not available".to_string();
        session.last_window_error = Some(error.clone());
        upsert_session_metadata(
            &mut session.metadata,
            "Ghostty Window",
            "not resolved".to_string(),
        );
        upsert_session_metadata(&mut session.metadata, "Window Error", error.clone());
        return Err(error);
    };

    match resolve_x11_window_for_pid(pid) {
        Ok(window_id) => {
            session.terminal_window_id = Some(window_id.clone());
            session.last_window_error = None;
            upsert_session_metadata(&mut session.metadata, "Ghostty Window", window_id.clone());
            upsert_session_metadata(&mut session.metadata, "Window Error", "none".to_string());
            if !session
                .terminal_lines
                .iter()
                .any(|line| line.contains("x11 window"))
            {
                session
                    .terminal_lines
                    .push(format!("x11 window {window_id}"));
            }
            Ok(window_id)
        }
        Err(error) => {
            session.terminal_window_id = None;
            session.last_window_error = Some(error.clone());
            upsert_session_metadata(
                &mut session.metadata,
                "Ghostty Window",
                "not resolved".to_string(),
            );
            upsert_session_metadata(&mut session.metadata, "Window Error", error.clone());
            Err(error)
        }
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn embedded_surface_note(bridge_available: bool) -> String {
    if !bridge_available {
        "libghostty is not linked in this build, so terminal mode stays on the fallback host path."
            .to_string()
    } else {
        "The active host adapter decides whether this session lands in an embedded surface or an external Ghostty window."
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

fn describe_terminal_host_mode(mode: GhosttyTerminalHostMode) -> &'static str {
    match mode {
        GhosttyTerminalHostMode::EmbeddedSurface => "embedded surface",
        GhosttyTerminalHostMode::ControlledDock => "controlled dock",
        GhosttyTerminalHostMode::ExternalWindow => "external window",
        GhosttyTerminalHostMode::Unsupported => "unsupported",
    }
}

fn resolve_x11_window_for_pid(pid: u32) -> Result<String, String> {
    let output = Command::new("xdotool")
        .arg("search")
        .arg("--pid")
        .arg(pid.to_string())
        .output()
        .map_err(|error| format!("failed to run xdotool search: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("ghostty window for pid {pid} not found yet")
        } else {
            format!("ghostty window lookup failed: {stderr}")
        });
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .ok_or_else(|| format!("ghostty window for pid {pid} not found yet"))
}

fn raise_x11_window(window_id: &str) -> Result<(), String> {
    let output = Command::new("xdotool")
        .arg("windowactivate")
        .arg("--sync")
        .arg(window_id)
        .output()
        .map_err(|error| format!("failed to run xdotool windowactivate: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("failed to raise ghostty window {window_id}")
        } else {
            format!("failed to raise ghostty window {window_id}: {stderr}")
        })
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
