use crate::{SessionNode, UiTheme};
use std::collections::BTreeMap;
use time::OffsetDateTime;

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

#[derive(Debug, Clone)]
pub struct ManagedSessionView {
    pub session_path: String,
    pub title: String,
    pub host_label: String,
    pub backend: TerminalBackend,
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
            session.status_line = format!(
                "{} host active · {} scheme requested",
                match session.backend {
                    TerminalBackend::Ghostty => "Ghostty",
                    TerminalBackend::Mock => "Mock",
                },
                appearance
            );
        }
    }

    pub fn open_or_focus_session(&mut self, path: &str) {
        let entry = self
            .sessions
            .entry(path.to_string())
            .or_insert_with(|| build_session(path, self.backend, self.theme));
        entry.backend = self.backend;
        self.active_session_path = Some(path.to_string());
    }

    pub fn active_session(&self) -> Option<&ManagedSessionView> {
        self.active_session_path
            .as_ref()
            .and_then(|path| self.sessions.get(path))
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

fn build_session(path: &str, backend: TerminalBackend, theme: UiTheme) -> ManagedSessionView {
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
        session_path: path.to_string(),
        title: title.clone(),
        host_label: host_label.clone(),
        backend,
        status_line: format!("{backend_label} host active · {appearance} scheme requested"),
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

fn session_preview_cwd(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let parent = trimmed.rsplit_once('/').map(|(parent, _)| parent).unwrap_or(trimmed);
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
