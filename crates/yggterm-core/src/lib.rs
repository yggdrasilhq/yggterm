mod browser;
mod server;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

pub use browser::{BrowserRow, BrowserRowKind, SessionBrowserState};
pub use server::{
    ManagedSessionView, PreviewTone, RemoteDeployState, SessionMetadataEntry, SessionPreview,
    SessionPreviewBlock, SessionRenderedSection, SessionSource, SshConnectTarget, TerminalBackend,
    TerminalLaunchPhase, WorkspaceViewMode, YggtermServer,
};

pub const ENV_YGGTERM_HOME: &str = "YGGTERM_HOME";
pub const DEFAULT_HOME_DIRNAME: &str = ".yggterm";
pub const DEFAULT_CODEX_HOME_DIRNAME: &str = ".codex";
pub const SESSIONS_DIRNAME: &str = "sessions";
pub const SETTINGS_FILENAME: &str = "settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNode {
    pub name: String,
    pub path: PathBuf,
    pub children: Vec<SessionNode>,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    home: PathBuf,
    sessions_root: PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UiTheme {
    ZedDark,
    ZedLight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: UiTheme,
    pub show_tree: bool,
    pub show_settings: bool,
    pub tree_width: f32,
    pub terminal_font_size: f32,
    pub ui_font_size: f32,
    pub prefer_ghostty_backend: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: UiTheme::ZedLight,
            show_tree: true,
            show_settings: false,
            tree_width: 300.0,
            terminal_font_size: 13.0,
            ui_font_size: 14.0,
            prefer_ghostty_backend: true,
        }
    }
}

impl SessionStore {
    pub fn open_or_init() -> Result<Self> {
        let home = resolve_yggterm_home()?;
        let sessions_root = home.join(SESSIONS_DIRNAME);

        fs::create_dir_all(&sessions_root).with_context(|| {
            format!(
                "failed to create sessions root at {}",
                sessions_root.display()
            )
        })?;

        Ok(Self {
            home,
            sessions_root,
        })
    }

    pub fn home_dir(&self) -> &Path {
        &self.home
    }

    pub fn sessions_root(&self) -> &Path {
        &self.sessions_root
    }

    pub fn settings_path(&self) -> PathBuf {
        self.home.join(SETTINGS_FILENAME)
    }

    pub fn load_settings(&self) -> Result<AppSettings> {
        let path = self.settings_path();
        if !path.exists() {
            return Ok(AppSettings::default());
        }

        let data = fs::read_to_string(&path)
            .with_context(|| format!("failed to read settings file {}", path.display()))?;
        let parsed: AppSettings = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse settings file {}", path.display()))?;
        Ok(parsed)
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        let path = self.settings_path();
        let data =
            serde_json::to_string_pretty(settings).context("failed to serialize settings")?;
        fs::write(&path, data)
            .with_context(|| format!("failed to write settings file {}", path.display()))?;
        Ok(())
    }

    pub fn create_session_path(&self, relative: &str) -> Result<PathBuf> {
        let safe = sanitize_relative_session_path(relative)?;
        let full = self.sessions_root.join(safe);
        fs::create_dir_all(&full)
            .with_context(|| format!("failed to create session directory {}", full.display()))?;
        Ok(full)
    }

    pub fn load_tree(&self) -> Result<SessionNode> {
        walk_directory_tree(&self.sessions_root, false)
    }

    pub fn load_codex_tree(&self) -> Result<SessionNode> {
        let codex_root = resolve_codex_sessions_root()?;
        let sessions = if codex_root.exists() {
            walk_directory_tree(&codex_root, true)?
        } else {
            SessionNode {
                name: String::from("sessions"),
                path: codex_root.clone(),
                children: Vec::new(),
            }
        };

        Ok(SessionNode {
            name: String::from("local [ok]"),
            path: codex_root,
            children: vec![sessions],
        })
    }
}

pub fn resolve_yggterm_home() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os(ENV_YGGTERM_HOME) {
        let p = PathBuf::from(value);
        return Ok(expand_tilde(p));
    }

    let home_dir = dirs::home_dir().context("unable to resolve home directory")?;
    Ok(home_dir.join(DEFAULT_HOME_DIRNAME))
}

pub fn resolve_codex_sessions_root() -> Result<PathBuf> {
    let home_dir = dirs::home_dir().context("unable to resolve home directory")?;
    Ok(home_dir
        .join(DEFAULT_CODEX_HOME_DIRNAME)
        .join(SESSIONS_DIRNAME))
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    if path == Path::new("~") {
        return dirs::home_dir().unwrap_or(path);
    }

    let mut comps = path.components();
    if let Some(first) = comps.next() {
        if first.as_os_str() == "~" {
            if let Some(home) = dirs::home_dir() {
                let rest: PathBuf = comps.collect();
                return home.join(rest);
            }
        }
    }

    path
}

fn sanitize_relative_session_path(input: &str) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for segment in input.split('/') {
        let seg = segment.trim();
        if seg.is_empty() || seg == "." || seg == ".." {
            continue;
        }
        let clean: OsString = seg
            .chars()
            .map(|ch| match ch {
                ':' | '\\' | '\0' => '_',
                _ => ch,
            })
            .collect::<String>()
            .into();
        out.push(clean);
    }

    if out.as_os_str().is_empty() {
        anyhow::bail!("session path must contain at least one non-empty segment")
    }

    Ok(out)
}

fn walk_directory_tree(path: &Path, include_codex_files: bool) -> Result<SessionNode> {
    let name = session_node_name(path, include_codex_files);

    let mut children = Vec::new();
    for entry in
        fs::read_dir(path).with_context(|| format!("failed to read dir {}", path.display()))?
    {
        let entry = entry?;
        let entry_path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            children.push(walk_directory_tree(&entry_path, include_codex_files)?);
        } else if include_codex_files && is_codex_session_file(&entry_path) {
            children.push(SessionNode {
                name: codex_leaf_label(&entry_path),
                path: entry_path,
                children: Vec::new(),
            });
        }
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(SessionNode {
        name,
        path: path.to_path_buf(),
        children,
    })
}

fn session_node_name(path: &Path, include_codex_files: bool) -> String {
    if include_codex_files
        && path == resolve_codex_sessions_root().unwrap_or_else(|_| path.to_path_buf())
    {
        return String::from("sessions");
    }

    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| String::from("sessions"))
}

fn is_codex_session_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    file_name.starts_with("rollout-")
        && file_name.ends_with(".jsonl")
        && !file_name.contains(".bak.")
}

fn codex_leaf_label(path: &Path) -> String {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return String::from("session");
    };
    let stem = file_name.trim_end_matches(".jsonl");
    let uuid_candidate = stem.get(stem.len().saturating_sub(36)..).unwrap_or(stem);
    let compact = uuid_candidate
        .chars()
        .filter(|ch| *ch != '-')
        .collect::<String>();

    if compact.len() >= 7 {
        return format!("Q{}", &compact[compact.len() - 7..]);
    }

    stem.to_string()
}
