use crate::{SessionNode, UiTheme};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::process::{Command, Stdio};
use std::time::SystemTime;
use time::{OffsetDateTime, UtcOffset, macros::format_description};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceViewMode {
    Terminal,
    Rendered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalBackend {
    Ghostty,
    Mock,
}

#[derive(Debug, Clone)]
pub struct SessionMetadataEntry {
    pub label: &'static str,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct SessionRenderedSection {
    pub title: &'static str,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewTone {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct SessionPreviewBlock {
    pub role: &'static str,
    pub timestamp: String,
    pub tone: PreviewTone,
    pub folded: bool,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SessionPreview {
    pub summary: Vec<SessionMetadataEntry>,
    pub blocks: Vec<SessionPreviewBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSource {
    Stored,
    LiveSsh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalLaunchPhase {
    Queued,
    BridgePending,
    RemoteBootstrap,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDeployState {
    NotRequired,
    Planned,
    CopyingBinary,
    Ready,
}

#[derive(Debug, Clone)]
pub struct SshConnectTarget {
    pub label: String,
    pub ssh_target: String,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone)]
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
    pub last_launch_error: Option<String>,
    pub last_window_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct YggtermServer {
    sessions: BTreeMap<String, ManagedSessionView>,
    active_session_path: Option<String>,
    active_view_mode: WorkspaceViewMode,
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
    ssh_targets: Vec<SshConnectTarget>,
    live_session_order: Vec<String>,
}

impl YggtermServer {
    pub fn new(
        tree: &SessionNode,
        prefer_ghostty_backend: bool,
        ghostty_bridge_enabled: bool,
        theme: UiTheme,
    ) -> Self {
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
            ghostty_bridge_enabled,
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
    ) {
        let entry = self.sessions.entry(path.to_string()).or_insert_with(|| {
            build_session(
                path,
                session_id,
                cwd,
                self.backend,
                self.theme,
                self.ghostty_bridge_enabled,
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
        self.active_session_path = Some(path.to_string());
    }

    pub fn active_session(&self) -> Option<&ManagedSessionView> {
        self.active_session_path
            .as_ref()
            .and_then(|path| self.sessions.get(path))
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
        let session = build_live_session(
            &uuid,
            &target,
            self.backend,
            self.theme,
            self.ghostty_bridge_enabled,
        );
        self.sessions.insert(key.clone(), session);
        self.live_session_order.insert(0, key.clone());
        self.active_session_path = Some(key.clone());
        self.active_view_mode = WorkspaceViewMode::Terminal;
        self.request_terminal_launch_for_active();
        Some(key)
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
                    match spawn_local_ghostty(&session.launch_command) {
                        Ok(pid) => {
                            session.backend = TerminalBackend::Ghostty;
                            session.terminal_process_id = Some(pid);
                            session.terminal_window_id = None;
                            session.last_launch_error = None;
                            session.last_window_error = None;
                            session.launch_phase = TerminalLaunchPhase::Running;
                            session.terminal_lines = vec![
                                format!("$ {}", session.launch_command),
                                format!("ghostty pid {pid}"),
                                "server launched the terminal in an external Ghostty window"
                                    .to_string(),
                                embedded_surface_note(session.bridge_available),
                            ];
                            let _ = sync_external_window(session);
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
        match sync_external_window(session) {
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
            let _ = sync_external_window(session);
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
}

struct StoredLeaf {
    path: String,
    session_id: String,
    cwd: String,
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
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
) -> ManagedSessionView {
    let session_id = session_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string());
    let title = short_session_id(&session_id);
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
        last_launch_error: None,
        last_window_error: None,
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
    let launch_command = match &target.prefix {
        Some(prefix) => format!(
            "ssh {} '{}' 'yggterm server attach {}'",
            target.ssh_target, prefix, uuid
        ),
        None => format!("ssh {} 'yggterm server attach {}'", target.ssh_target, uuid),
    };

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
        last_launch_error: None,
        last_window_error: None,
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

fn spawn_local_ghostty(launch_command: &str) -> Result<u32, String> {
    let child = Command::new("ghostty")
        .arg("-e")
        .arg("bash")
        .arg("-lc")
        .arg(launch_command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to spawn ghostty: {error}"))?;
    Ok(child.id())
}

fn sync_external_window(session: &mut ManagedSessionView) -> Result<String, String> {
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
    let content = fs::read_to_string(path).ok()?;
    let mut started_at = None;
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut metadata_entries = Vec::new();
    let mut blocks = Vec::new();

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let event_type = value.get("type").and_then(Value::as_str);

        match event_type {
            Some("session_meta") => {
                if let Some(payload) = value.get("payload") {
                    started_at = payload
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .map(parse_and_format_timestamp)
                        .or(started_at);
                }
            }
            Some("response_item") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }

                let role = payload
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("assistant");
                let lines = message_lines_from_payload(payload);
                if lines.is_empty() {
                    continue;
                }
                if role == "assistant"
                    && blocks.is_empty()
                    && looks_like_session_metadata_block(&lines)
                {
                    metadata_entries = parse_session_metadata_lines(&lines);
                    continue;
                }

                match role {
                    "user" | "developer" => user_messages += 1,
                    "assistant" => assistant_messages += 1,
                    _ => {}
                }

                blocks.push(SessionPreviewBlock {
                    role: session_role_label(role),
                    timestamp: extract_timestamp(&value).unwrap_or_else(|| {
                        started_at
                            .clone()
                            .unwrap_or_else(|| fallback_started_at.to_string())
                    }),
                    tone: session_preview_tone(role),
                    folded: false,
                    lines,
                });
            }
            Some("event_msg") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) != Some("user_message") {
                    continue;
                }
                let Some(text) = payload.get("message").and_then(Value::as_str) else {
                    continue;
                };
                user_messages += 1;
                blocks.push(SessionPreviewBlock {
                    role: "USER",
                    timestamp: extract_timestamp(&value)
                        .unwrap_or_else(|| fallback_started_at.to_string()),
                    tone: PreviewTone::User,
                    folded: false,
                    lines: normalize_preview_text(text),
                });
            }
            _ => {}
        }
    }

    Some(StoredTranscript {
        started_at: started_at.unwrap_or_else(|| fallback_started_at.to_string()),
        user_messages,
        assistant_messages,
        metadata_entries,
        blocks,
    })
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

fn message_lines_from_payload(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(content_items) = payload.get("content").and_then(Value::as_array) {
        for item in content_items {
            if let Some(text) = item
                .get("text")
                .or_else(|| item.get("input_text"))
                .or_else(|| item.get("output_text"))
                .and_then(Value::as_str)
            {
                lines.extend(normalize_preview_text(text));
            }
        }
    }
    lines
}

fn embedded_surface_note(bridge_available: bool) -> String {
    if !bridge_available {
        return "libghostty is not linked in this build, so terminal mode stays on the external fallback path.".to_string();
    }

    if cfg!(target_os = "linux") {
        "libghostty is linked, but Ghostty's current embedded surface host only exposes macOS/iOS views, so Linux still falls back to an external Ghostty window.".to_string()
    } else {
        "libghostty is linked and the embedded surface host remains the active integration target."
            .to_string()
    }
}

fn normalize_preview_text(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn extract_timestamp(value: &Value) -> Option<String> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(parse_and_format_timestamp)
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("timestamp"))
                .and_then(Value::as_str)
                .map(parse_and_format_timestamp)
        })
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

fn session_role_label(role: &str) -> &'static str {
    match role {
        "user" | "developer" => "USER",
        "assistant" => "ASSISTANT",
        _ => "SYSTEM",
    }
}

fn session_preview_tone(role: &str) -> PreviewTone {
    match role {
        "user" | "developer" => PreviewTone::User,
        _ => PreviewTone::Assistant,
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
