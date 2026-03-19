mod browser;
mod server;
mod titles;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use titles::{SessionTitleResolver, settings_ready as litellm_settings_ready};

pub use browser::{BrowserMetrics, BrowserRow, BrowserRowKind, SessionBrowserState};
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
    pub title: Option<String>,
    pub path: PathBuf,
    pub children: Vec<SessionNode>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppSettings {
    pub theme: UiTheme,
    pub show_tree: bool,
    pub show_settings: bool,
    pub tree_width: f32,
    pub terminal_font_size: f32,
    pub ui_font_size: f32,
    pub prefer_ghostty_backend: bool,
    pub litellm_endpoint: String,
    pub litellm_api_key: String,
    pub interface_llm_model: String,
    pub selected_browser_path: Option<String>,
    pub expanded_browser_paths: Vec<String>,
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
            litellm_endpoint: String::new(),
            litellm_api_key: String::new(),
            interface_llm_model: String::new(),
            selected_browser_path: None,
            expanded_browser_paths: Vec::new(),
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
        save_settings_file(&path, settings)
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

    pub fn load_codex_tree(&self, settings: &AppSettings) -> Result<SessionNode> {
        let codex_root = resolve_codex_sessions_root()?;
        build_codex_browser_tree(&self.home, &codex_root, settings)
    }

    pub fn generate_missing_codex_titles(
        &self,
        settings: &AppSettings,
        budget: usize,
    ) -> Result<usize> {
        if budget == 0 || !litellm_settings_ready(settings) {
            return Ok(0);
        }

        let codex_root = resolve_codex_sessions_root()?;
        if !codex_root.exists() {
            return Ok(0);
        }

        let resolver = SessionTitleResolver::new(&self.home)?;
        let mut remaining_budget = budget;
        generate_missing_codex_titles_recursive(
            &resolver,
            settings,
            &codex_root,
            &mut remaining_budget,
        )?;
        Ok(budget.saturating_sub(remaining_budget))
    }

    pub fn generate_title_for_session_path(
        &self,
        settings: &AppSettings,
        session_path: &str,
        force: bool,
    ) -> Result<Option<String>> {
        if !litellm_settings_ready(settings) {
            return Ok(None);
        }

        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }

        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.generate_for_session(
            settings,
            &identity.session_id,
            &identity.cwd,
            &path,
            force,
        )
    }
}

pub fn save_settings_file(path: &Path, settings: &AppSettings) -> Result<()> {
    let data = serde_json::to_string_pretty(settings).context("failed to serialize settings")?;
    fs::write(path, data)
        .with_context(|| format!("failed to write settings file {}", path.display()))?;
    Ok(())
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
                title: None,
                path: entry_path,
                children: Vec::new(),
                session_id: None,
                cwd: None,
            });
        }
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(SessionNode {
        name,
        title: None,
        path: path.to_path_buf(),
        children,
        session_id: None,
        cwd: None,
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

fn compress_session_tree(mut node: SessionNode, keep_root: bool) -> SessionNode {
    node.children = node
        .children
        .into_iter()
        .map(|child| compress_session_tree(child, false))
        .collect();

    if keep_root {
        return node;
    }

    while node.children.len() == 1 && !node.children[0].children.is_empty() {
        let child = node.children.pop().expect("single child exists");
        node.name = join_session_label(&node.name, &child.name);
        node.path = child.path;
        node.children = child.children;
    }

    node
}

fn join_session_label(left: &str, right: &str) -> String {
    if left == "/" {
        format!("/{right}")
    } else if left.ends_with('/') {
        format!("{left}{right}")
    } else {
        format!("{left}/{right}")
    }
}

#[derive(Debug, Clone)]
struct CodexSessionSummary {
    file_path: PathBuf,
    session_id: String,
    cwd: String,
    generated_title: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexSessionIdentity {
    session_id: String,
    cwd: String,
}

#[derive(Debug, Clone)]
struct CodexProjectBucket {
    cwd: String,
    sessions: Vec<CodexSessionSummary>,
}

#[derive(Debug, Clone, Default)]
struct CodexBrowserTreeNode {
    name: String,
    full_path: String,
    project: Option<CodexProjectBucket>,
    children: BTreeMap<String, CodexBrowserTreeNode>,
}

fn build_codex_browser_tree(
    home: &Path,
    codex_root: &Path,
    _settings: &AppSettings,
) -> Result<SessionNode> {
    let title_resolver = SessionTitleResolver::new(home).ok();
    let sessions = if codex_root.exists() {
        scan_codex_sessions(codex_root, title_resolver.as_ref())?
    } else {
        Vec::new()
    };

    let mut projects = BTreeMap::<String, Vec<CodexSessionSummary>>::new();
    for session in sessions {
        projects
            .entry(session.cwd.clone())
            .or_default()
            .push(session);
    }

    let mut buckets = Vec::new();
    for (cwd, mut project_sessions) in projects {
        project_sessions.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        buckets.push(CodexProjectBucket {
            cwd,
            sessions: project_sessions,
        });
    }

    let mut root = CodexBrowserTreeNode {
        name: String::from("local [ok]"),
        full_path: String::from("local"),
        project: None,
        children: BTreeMap::new(),
    };
    for bucket in buckets {
        insert_codex_browser_project(&mut root, &bucket);
    }
    compress_codex_browser_tree(&mut root, false);

    Ok(codex_browser_tree_to_session_node(&root))
}

fn scan_codex_sessions(
    root: &Path,
    title_resolver: Option<&SessionTitleResolver>,
) -> Result<Vec<CodexSessionSummary>> {
    let mut sessions = Vec::new();
    for entry in
        fs::read_dir(root).with_context(|| format!("failed to read dir {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            sessions.extend(scan_codex_sessions(&path, title_resolver)?);
        } else if is_codex_session_file(&path) {
            if let Some(summary) = read_codex_session_summary(&path, title_resolver)? {
                sessions.push(summary);
            }
        }
    }
    Ok(sessions)
}

fn read_codex_session_summary(
    path: &Path,
    title_resolver: Option<&SessionTitleResolver>,
) -> Result<Option<CodexSessionSummary>> {
    let Some(identity) = read_codex_session_identity(path)? else {
        return Ok(None);
    };

    Ok(Some(CodexSessionSummary {
        file_path: path.to_path_buf(),
        generated_title: title_resolver
            .and_then(|resolver| resolver.resolve_for_session(&identity.session_id).ok().flatten()),
        session_id: identity.session_id,
        cwd: identity.cwd,
    }))
}

fn read_codex_session_identity(path: &Path) -> Result<Option<CodexSessionIdentity>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read codex session {}", path.display()))?;

    let fallback_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.trim_start_matches("rollout-").to_string())
        .unwrap_or_else(|| String::from("unknown-session"));

    let mut session_id = None;
    let mut cwd = None;

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if session_id.is_none() {
            session_id = find_string_field(&value, &["id"]);
        }
        if cwd.is_none() {
            cwd = find_string_field(&value, &["cwd"]);
        }
        if session_id.is_some() && cwd.is_some() {
            break;
        }
    }

    let Some(cwd) = cwd.map(normalize_codex_cwd) else {
        return Ok(None);
    };

    Ok(Some(CodexSessionIdentity {
        session_id: session_id.unwrap_or(fallback_id),
        cwd,
    }))
}

fn generate_missing_codex_titles_recursive(
    resolver: &SessionTitleResolver,
    settings: &AppSettings,
    root: &Path,
    remaining_budget: &mut usize,
) -> Result<()> {
    if *remaining_budget == 0 {
        return Ok(());
    }

    for entry in
        fs::read_dir(root).with_context(|| format!("failed to read dir {}", root.display()))?
    {
        if *remaining_budget == 0 {
            break;
        }

        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            generate_missing_codex_titles_recursive(resolver, settings, &path, remaining_budget)?;
            continue;
        }

        if !is_codex_session_file(&path) {
            continue;
        }

        let Some(identity) = read_codex_session_identity(&path)? else {
            continue;
        };
        if resolver.resolve_for_session(&identity.session_id)?.is_some() {
            continue;
        }
        if resolver
            .generate_for_session(settings, &identity.session_id, &identity.cwd, &path, false)?
            .is_some()
        {
            *remaining_budget = remaining_budget.saturating_sub(1);
        }
    }

    Ok(())
}

fn find_string_field(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(Value::String(found)) = map.get(*key) {
                    if !found.trim().is_empty() {
                        return Some(found.clone());
                    }
                }
            }
            map.values()
                .find_map(|child| find_string_field(child, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_field(child, keys)),
        _ => None,
    }
}

fn normalize_codex_cwd(raw: String) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::from("/");
    }
    if trimmed == "/" {
        return String::from("/");
    }
    if trimmed.starts_with('/') {
        format!("/{}", trimmed.trim_start_matches('/'))
            .trim_end_matches('/')
            .to_string()
    } else {
        format!("/{}", trimmed.trim_matches('/'))
    }
}

fn insert_codex_browser_project(root: &mut CodexBrowserTreeNode, project: &CodexProjectBucket) {
    let segments = browser_tree_segments(&project.cwd);
    insert_codex_browser_path(root, &segments, project.clone());
}

fn insert_codex_browser_path(
    node: &mut CodexBrowserTreeNode,
    segments: &[String],
    project: CodexProjectBucket,
) {
    if segments.is_empty() {
        node.project = Some(project);
        return;
    }

    let segment = &segments[0];
    let child_path = browser_tree_child_path(&node.full_path, segment);
    let child = node
        .children
        .entry(segment.clone())
        .or_insert_with(|| CodexBrowserTreeNode {
            name: segment.clone(),
            full_path: child_path,
            ..Default::default()
        });
    insert_codex_browser_path(child, &segments[1..], project);
}

fn browser_tree_segments(cwd: &str) -> Vec<String> {
    if cwd == "/" {
        return vec![String::from("/")];
    }
    if cwd == "/root" {
        return vec![String::from("/root")];
    }
    if let Some(rest) = cwd.strip_prefix("/root/") {
        let mut parts = vec![String::from("/root")];
        parts.extend(rest.split('/').map(|part| part.to_string()));
        return parts;
    }

    let mut parts = vec![String::from("/")];
    parts.extend(
        cwd.trim_start_matches('/')
            .split('/')
            .filter(|part| !part.is_empty())
            .map(|part| part.to_string()),
    );
    parts
}

fn browser_tree_child_path(parent: &str, segment: &str) -> String {
    if segment == "/" {
        return format!("{}/", parent.trim_end_matches('/'));
    }
    if parent.ends_with('/') {
        format!("{parent}{segment}")
    } else {
        format!("{parent}/{segment}")
    }
}

fn compress_codex_browser_tree(node: &mut CodexBrowserTreeNode, can_compress_self: bool) {
    let child_keys = node.children.keys().cloned().collect::<Vec<_>>();
    for key in child_keys {
        if let Some(child) = node.children.get_mut(&key) {
            compress_codex_browser_tree(child, true);
        }
    }

    if !can_compress_self {
        return;
    }

    while node.project.is_none() && node.children.len() == 1 {
        let (_, child) = node.children.pop_first().expect("single child exists");
        node.name = join_session_label(&node.name, &child.name);
        node.full_path = child.full_path;
        node.project = child.project;
        node.children = child.children;
    }
}

fn codex_browser_tree_to_session_node(node: &CodexBrowserTreeNode) -> SessionNode {
    let mut children = Vec::new();

    if let Some(project) = &node.project {
        children.extend(project.sessions.iter().map(|session| SessionNode {
            name: short_session_id(&session.session_id),
            title: session.generated_title.clone(),
            path: session.file_path.clone(),
            children: Vec::new(),
            session_id: Some(session.session_id.clone()),
            cwd: Some(project.cwd.clone()),
        }));
    }

    children.extend(
        node.children
            .values()
            .map(codex_browser_tree_to_session_node)
            .collect::<Vec<_>>(),
    );

    SessionNode {
        name: node.name.clone(),
        title: None,
        path: PathBuf::from(node.full_path.clone()),
        children,
        session_id: None,
        cwd: node.project.as_ref().map(|project| project.cwd.clone()),
    }
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
