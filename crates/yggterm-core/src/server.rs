use crate::{SessionNode, UiTheme};
use std::collections::BTreeMap;
use time::OffsetDateTime;
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
            active_view_mode: WorkspaceViewMode::Terminal,
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

        if let Some(first_session) = first_session_path(tree) {
            this.open_or_focus_session(&first_session);
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

    pub fn open_or_focus_session(&mut self, path: &str) {
        let entry = self.sessions.entry(path.to_string()).or_insert_with(|| {
            build_session(path, self.backend, self.theme, self.ghostty_bridge_enabled)
        });
        entry.backend = self.backend;
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
                session.launch_phase = if session.bridge_available {
                    TerminalLaunchPhase::Running
                } else {
                    TerminalLaunchPhase::BridgePending
                };
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
}

fn first_session_path(node: &SessionNode) -> Option<String> {
    if node.children.is_empty() {
        return Some(node.path.display().to_string());
    }

    for child in &node.children {
        if let Some(path) = first_session_path(child) {
            return Some(path);
        }
    }

    None
}

fn build_session(
    path: &str,
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
) -> ManagedSessionView {
    let title = path.rsplit('/').next().unwrap_or(path).to_string();
    let host_label = if path.contains("/prod/") {
        "prod-app-01"
    } else if path.contains("ghostty") {
        "ghostty-admin"
    } else if path.contains("local") {
        "localhost"
    } else {
        "ssh-target"
    }
    .to_string();

    let appearance = match theme {
        UiTheme::ZedDark => "dark",
        UiTheme::ZedLight => "light",
    };
    let started_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::from("unknown"));
    let backend_label = match backend {
        TerminalBackend::Ghostty => "Ghostty",
        TerminalBackend::Mock => "Mock",
    };
    let cwd = session_preview_cwd(path);
    let preview = SessionPreview {
        summary: vec![
            SessionMetadataEntry {
                label: "Session",
                value: title.clone(),
            },
            SessionMetadataEntry {
                label: "Path",
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
        ],
        blocks: vec![
            SessionPreviewBlock {
                role: "USER",
                timestamp: started_at.clone(),
                tone: PreviewTone::User,
                folded: false,
                lines: vec![
                    format!("Attach {title} on {host_label} and restore the last multiplexed shell."),
                    format!("Open the persisted workspace rooted at {cwd}."),
                ],
            },
            SessionPreviewBlock {
                role: "ASSISTANT",
                timestamp: "server:restore".to_string(),
                tone: PreviewTone::Assistant,
                folded: false,
                lines: vec![
                    format!("{backend_label} backend reserved for the live terminal surface."),
                    "Rendered preview follows codex-session-tui structure: scan browser first, inspect transcript second.".to_string(),
                    "The Yggterm server stays authoritative for restore, attach, and remote orchestration.".to_string(),
                ],
            },
            SessionPreviewBlock {
                role: "USER",
                timestamp: "session:notes".to_string(),
                tone: PreviewTone::User,
                folded: true,
                lines: vec![
                    "Future rich rendering will show screenshots, search hits, command summaries, and clipboard transfers here.".to_string(),
                    "Switch back to Terminal in the titlebar when the Ghostty surface is active.".to_string(),
                ],
            },
        ],
    };

    ManagedSessionView {
        id: format!("stored:{title}"),
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
        launch_command: format!("yggterm server attach {title}"),
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
            format!("$ attach {title}"),
            format!("Connected to {host_label}"),
            format!("$ cd {}", path.replace("sessions/", "~/gh/")),
            "$ cargo test -p session-ui".to_string(),
            "running 16 tests".to_string(),
            "test session_restore::persists_metadata ... ok".to_string(),
            "test ssh_clipboard::pastes_image_payloads ... pending".to_string(),
        ],
        rendered_sections: vec![
            SessionRenderedSection {
                title: "Rendered Session",
                lines: vec![
                    "This pane is the future Zed-rendered session view.".to_string(),
                    "Selections, rich transcript rendering, search, and command summaries land here.".to_string(),
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
        metadata: vec![
            SessionMetadataEntry {
                label: "Source",
                value: "stored".to_string(),
            },
            SessionMetadataEntry {
                label: "Host",
                value: host_label,
            },
            SessionMetadataEntry {
                label: "Session",
                value: title,
            },
            SessionMetadataEntry {
                label: "Storage",
                value: "~/.yggterm".to_string(),
            },
            SessionMetadataEntry {
                label: "Cwd",
                value: cwd,
            },
            SessionMetadataEntry {
                label: "Backend",
                value: backend_label.to_string(),
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
                value: "last_session".to_string(),
            },
        ],
    }
}

fn build_live_session(
    uuid: &str,
    target: &SshConnectTarget,
    backend: TerminalBackend,
    theme: UiTheme,
    ghostty_bridge_enabled: bool,
) -> ManagedSessionView {
    let started_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::from("unknown"));
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
                label: "Launch",
                value: launch_command,
            },
        ],
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
