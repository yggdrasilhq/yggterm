mod browser;
mod icon;
mod install;
mod perf;
mod titles;
mod trace;
mod transcript;
mod workspace;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use titles::{SessionTitleResolver, settings_ready as litellm_settings_ready};
pub use yggui_contract::{UiTheme, YgguiThemeColorStop, YgguiThemeSpec};

pub use browser::{
    BrowserMetrics, BrowserRow, BrowserRowKind, SessionBrowserState,
    unique_session_short_ids_for_pairs,
};
pub use icon::{
    AppIconAssets, LinuxInstalledIconSet, YGGTERM_ICON_ASSETS, install_linux_icon_assets,
};
pub use install::{
    ENV_YGGTERM_DIRECT_INSTALL_ROOT, InstallChannel, InstallContext, ReleaseUpdate,
    ReleaseUpdateInstallProgress, ReleaseUpdateInstallStage, UpdatePolicy, YGGTERM_DESKTOP_APP_ID,
    check_for_update, current_asset_label, current_version, detect_install_context,
    direct_install_root, install_mode_summary, install_release_update,
    install_release_update_with_progress, refresh_desktop_integration, update_command_hint,
    write_direct_install_state,
};
pub use perf::{
    PERF_TELEMETRY_FILENAME, PERF_TELEMETRY_MAX_BYTES, PERF_TELEMETRY_ROTATED_FILENAME, PerfSpan,
    append_bounded_jsonl_record, append_perf_event, perf_telemetry_path,
};
pub use titles::{
    SessionTitleStore, best_effort_context_from_session_path, best_effort_precis_from_context,
    best_effort_summary_from_context, best_effort_title_from_context,
    looks_like_generated_fallback_title, looks_like_low_signal_generated_copy,
};
pub use trace::{
    EVENT_TRACE_FILENAME, EventTraceRecord, EventTraceSpan, append_trace_event, event_trace_path,
    follow_trace_lines, read_trace_tail,
};
pub use transcript::{
    TranscriptMessage, TranscriptRole, generation_context_from_messages,
    message_lines_from_payload, read_codex_transcript_messages,
    read_codex_transcript_messages_limited,
};
pub use workspace::{
    WorkspaceDocument, WorkspaceDocumentInput, WorkspaceDocumentKind, WorkspaceDocumentSummary,
    WorkspaceGroup, WorkspaceGroupKind, WorkspaceStore, default_document_title,
    normalize_virtual_document_path, normalize_virtual_group_path,
};

pub const ENV_YGGTERM_HOME: &str = "YGGTERM_HOME";
pub const DEFAULT_HOME_DIRNAME: &str = ".yggterm";
pub const DEFAULT_CODEX_HOME_DIRNAME: &str = ".codex";
pub const SESSIONS_DIRNAME: &str = "sessions";
pub const SETTINGS_FILENAME: &str = "settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNode {
    pub kind: SessionNodeKind,
    pub name: String,
    pub title: Option<String>,
    pub document_kind: Option<WorkspaceDocumentKind>,
    pub group_kind: Option<WorkspaceGroupKind>,
    pub path: PathBuf,
    pub children: Vec<SessionNode>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionNodeKind {
    Group,
    CodexSession,
    Document,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    home: PathBuf,
    sessions_root: PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionProfile {
    #[default]
    Codex,
    CodexLiteLlm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppSettings {
    pub theme: UiTheme,
    pub yggui_theme: YgguiThemeSpec,
    pub show_tree: bool,
    pub show_settings: bool,
    pub auto_hide_titlebar: bool,
    pub tree_width: f32,
    pub rendered_font_size: f32,
    pub terminal_font_size: f32,
    pub terminal_light_theme_name: String,
    pub terminal_dark_theme_name: String,
    pub ui_font_size: f32,
    pub prefer_ghostty_backend: bool,
    pub litellm_endpoint: String,
    pub litellm_api_key: String,
    pub interface_llm_model: String,
    pub default_agent_profile: AgentSessionProfile,
    pub in_app_notifications: bool,
    pub system_notifications: bool,
    pub notification_sound: bool,
    pub selected_browser_path: Option<String>,
    pub expanded_browser_paths: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: UiTheme::ZedLight,
            yggui_theme: YgguiThemeSpec::default(),
            show_tree: true,
            show_settings: false,
            auto_hide_titlebar: false,
            tree_width: 300.0,
            rendered_font_size: 10.0,
            terminal_font_size: 14.0,
            terminal_light_theme_name: "VS Code Light+".to_string(),
            terminal_dark_theme_name: "Dark+".to_string(),
            ui_font_size: 14.0,
            prefer_ghostty_backend: true,
            litellm_endpoint: String::new(),
            litellm_api_key: String::new(),
            interface_llm_model: String::new(),
            default_agent_profile: AgentSessionProfile::Codex,
            in_app_notifications: true,
            system_notifications: false,
            notification_sound: false,
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
        let value: Value = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse settings file {}", path.display()))?;
        parse_settings_value(&value)
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

    pub fn list_documents(&self) -> Result<Vec<WorkspaceDocumentSummary>> {
        WorkspaceStore::open(&self.home)?.list_documents()
    }

    pub fn list_groups(&self) -> Result<Vec<WorkspaceGroup>> {
        WorkspaceStore::open(&self.home)?.list_groups()
    }

    pub fn load_document(&self, virtual_path: &str) -> Result<Option<WorkspaceDocument>> {
        WorkspaceStore::open(&self.home)?.get_document(virtual_path)
    }

    pub fn save_group(&self, virtual_path: &str, title: Option<&str>) -> Result<WorkspaceGroup> {
        WorkspaceStore::open(&self.home)?.put_group(virtual_path, title)
    }

    pub fn save_group_with_kind(
        &self,
        virtual_path: &str,
        title: Option<&str>,
        kind: WorkspaceGroupKind,
    ) -> Result<WorkspaceGroup> {
        WorkspaceStore::open(&self.home)?.put_group_with_kind(virtual_path, title, kind)
    }

    pub fn move_document(
        &self,
        from_virtual_path: &str,
        to_virtual_path: &str,
    ) -> Result<WorkspaceDocument> {
        WorkspaceStore::open(&self.home)?.move_document(from_virtual_path, to_virtual_path)
    }

    pub fn delete_documents(&self, virtual_paths: &[String]) -> Result<usize> {
        WorkspaceStore::open(&self.home)?.delete_documents(virtual_paths)
    }

    pub fn move_group(
        &self,
        from_virtual_path: &str,
        to_virtual_path: &str,
    ) -> Result<WorkspaceGroup> {
        WorkspaceStore::open(&self.home)?.move_group(from_virtual_path, to_virtual_path)
    }

    pub fn delete_workspace_items(
        &self,
        document_paths: &[String],
        group_paths: &[String],
    ) -> Result<usize> {
        WorkspaceStore::open(&self.home)?.delete_workspace_items(document_paths, group_paths)
    }

    pub fn save_document(
        &self,
        virtual_path: &str,
        title: Option<&str>,
        body: &str,
    ) -> Result<WorkspaceDocument> {
        WorkspaceStore::open(&self.home)?.put_document(virtual_path, title, body)
    }

    pub fn save_document_input(
        &self,
        virtual_path: &str,
        input: WorkspaceDocumentInput,
    ) -> Result<WorkspaceDocument> {
        WorkspaceStore::open(&self.home)?.put_document_input(virtual_path, input)
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
        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }

        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.generate_for_session(settings, &identity.session_id, &identity.cwd, &path, force)
    }

    pub fn generate_title_for_context(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        context: &str,
        force: bool,
    ) -> Result<Option<String>> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.generate_for_context(settings, session_id, cwd, context, force)
    }

    pub fn save_manual_title_for_session_id(
        &self,
        session_id: &str,
        cwd: &str,
        title: &str,
    ) -> Result<()> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.save_manual_title_for_session(session_id, cwd, title)
    }

    pub fn save_manual_title_for_session_path(
        &self,
        session_path: &str,
        title: &str,
    ) -> Result<Option<String>> {
        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }
        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.save_manual_title_for_session(&identity.session_id, &identity.cwd, title)?;
        Ok(Some(identity.session_id))
    }

    pub fn clear_title_for_session_id(&self, session_id: &str) -> Result<()> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.clear_title_for_session(session_id)
    }

    pub fn clear_title_for_session_path(&self, session_path: &str) -> Result<Option<String>> {
        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }
        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.clear_title_for_session(&identity.session_id)?;
        Ok(Some(identity.session_id))
    }

    pub fn resolve_precis_for_session_path(&self, session_path: &str) -> Result<Option<String>> {
        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }

        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.resolve_precis_for_session(&identity.session_id)
    }

    pub fn resolve_title_for_session_id(&self, session_id: &str) -> Result<Option<String>> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.resolve_for_session(session_id)
    }

    pub fn resolve_precis_for_session_id(&self, session_id: &str) -> Result<Option<String>> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.resolve_precis_for_session(session_id)
    }

    pub fn resolve_title_for_session_path(&self, session_path: &str) -> Result<Option<String>> {
        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }

        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.resolve_for_session(&identity.session_id)
    }

    pub fn resolve_summary_for_session_id(&self, session_id: &str) -> Result<Option<String>> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.resolve_summary_for_session(session_id)
    }

    pub fn resolve_summary_for_session_path(&self, session_path: &str) -> Result<Option<String>> {
        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }

        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.resolve_summary_for_session(&identity.session_id)
    }

    pub fn precis_needs_refresh_for_session_id(
        &self,
        session_id: &str,
        source_updated_at: OffsetDateTime,
    ) -> Result<bool> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.precis_needs_refresh(session_id, source_updated_at)
    }

    pub fn summary_needs_refresh_for_session_id(
        &self,
        session_id: &str,
        source_updated_at: OffsetDateTime,
    ) -> Result<bool> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.summary_needs_refresh(session_id, source_updated_at)
    }

    pub fn generate_precis_for_session_path(
        &self,
        settings: &AppSettings,
        session_path: &str,
        force: bool,
    ) -> Result<Option<String>> {
        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }

        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.generate_precis_for_session(
            settings,
            &identity.session_id,
            &identity.cwd,
            &path,
            force,
        )
    }

    pub fn generate_precis_for_context(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        context: &str,
        force: bool,
    ) -> Result<Option<String>> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.generate_precis_for_context(settings, session_id, cwd, context, force)
    }

    pub fn generate_summary_for_session_path(
        &self,
        settings: &AppSettings,
        session_path: &str,
        force: bool,
    ) -> Result<Option<String>> {
        let path = PathBuf::from(session_path);
        if !path.exists() || !is_codex_session_file(&path) {
            return Ok(None);
        }

        let Some(identity) = read_codex_session_identity(&path)? else {
            return Ok(None);
        };
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.generate_summary_for_session(
            settings,
            &identity.session_id,
            &identity.cwd,
            &path,
            force,
        )
    }

    pub fn generate_summary_for_context(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        context: &str,
        force: bool,
    ) -> Result<Option<String>> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.generate_summary_for_context(settings, session_id, cwd, context, force)
    }
}

pub fn save_settings_file(path: &Path, settings: &AppSettings) -> Result<()> {
    let data = serde_json::to_string_pretty(&serialize_settings_value(settings))
        .context("failed to serialize settings")?;
    fs::write(path, data)
        .with_context(|| format!("failed to write settings file {}", path.display()))?;
    Ok(())
}

fn parse_settings_value(value: &Value) -> Result<AppSettings> {
    let mut settings = AppSettings::default();
    let Some(object) = value.as_object() else {
        anyhow::bail!("settings file must contain a JSON object");
    };

    if let Some(theme_mode) = object
        .get("theme_mode")
        .or_else(|| object.get("theme").filter(|value| value.is_string()))
    {
        settings.theme =
            serde_json::from_value(theme_mode.clone()).context("failed to parse theme_mode")?;
    }
    if let Some(theme_spec) = object
        .get("theme")
        .filter(|value| value.is_object())
        .or_else(|| object.get("yggui_theme"))
    {
        settings.yggui_theme =
            serde_json::from_value(theme_spec.clone()).context("failed to parse theme object")?;
    }
    if let Some(value) = object.get("show_tree") {
        settings.show_tree =
            serde_json::from_value(value.clone()).context("failed to parse show_tree")?;
    }
    if let Some(value) = object.get("show_settings") {
        settings.show_settings =
            serde_json::from_value(value.clone()).context("failed to parse show_settings")?;
    }
    if let Some(value) = object.get("auto_hide_titlebar") {
        settings.auto_hide_titlebar =
            serde_json::from_value(value.clone()).context("failed to parse auto_hide_titlebar")?;
    }
    if let Some(value) = object.get("tree_width") {
        settings.tree_width =
            serde_json::from_value(value.clone()).context("failed to parse tree_width")?;
    }
    if let Some(value) = object.get("rendered_font_size") {
        settings.rendered_font_size =
            serde_json::from_value(value.clone()).context("failed to parse rendered_font_size")?;
    }
    if let Some(value) = object.get("terminal_font_size") {
        settings.terminal_font_size =
            serde_json::from_value(value.clone()).context("failed to parse terminal_font_size")?;
    }
    if let Some(value) = object.get("terminal_light_theme_name") {
        settings.terminal_light_theme_name = serde_json::from_value(value.clone())
            .context("failed to parse terminal_light_theme_name")?;
    }
    if let Some(value) = object.get("terminal_dark_theme_name") {
        settings.terminal_dark_theme_name = serde_json::from_value(value.clone())
            .context("failed to parse terminal_dark_theme_name")?;
    }
    if let Some(value) = object.get("terminal_theme_name")
        && !object.contains_key("terminal_light_theme_name")
        && !object.contains_key("terminal_dark_theme_name")
    {
        let shared_theme = serde_json::from_value::<String>(value.clone())
            .context("failed to parse terminal_theme_name")?;
        settings.terminal_light_theme_name = shared_theme.clone();
        settings.terminal_dark_theme_name = shared_theme;
    }
    if let Some(value) = object.get("ui_font_size") {
        settings.ui_font_size =
            serde_json::from_value(value.clone()).context("failed to parse ui_font_size")?;
    }
    if let Some(value) = object.get("prefer_ghostty_backend") {
        settings.prefer_ghostty_backend = serde_json::from_value(value.clone())
            .context("failed to parse prefer_ghostty_backend")?;
    }
    if let Some(value) = object.get("litellm_endpoint") {
        settings.litellm_endpoint =
            serde_json::from_value(value.clone()).context("failed to parse litellm_endpoint")?;
    }
    if let Some(value) = object.get("litellm_api_key") {
        settings.litellm_api_key =
            serde_json::from_value(value.clone()).context("failed to parse litellm_api_key")?;
    }
    if let Some(value) = object.get("interface_llm_model") {
        settings.interface_llm_model =
            serde_json::from_value(value.clone()).context("failed to parse interface_llm_model")?;
    }
    if let Some(value) = object.get("default_agent_profile") {
        settings.default_agent_profile = serde_json::from_value(value.clone())
            .context("failed to parse default_agent_profile")?;
    }
    if let Some(value) = object.get("in_app_notifications") {
        settings.in_app_notifications = serde_json::from_value(value.clone())
            .context("failed to parse in_app_notifications")?;
    }
    if let Some(value) = object.get("system_notifications") {
        settings.system_notifications = serde_json::from_value(value.clone())
            .context("failed to parse system_notifications")?;
    }
    if let Some(value) = object.get("notification_sound") {
        settings.notification_sound =
            serde_json::from_value(value.clone()).context("failed to parse notification_sound")?;
    }
    if let Some(value) = object.get("selected_browser_path") {
        settings.selected_browser_path = serde_json::from_value(value.clone())
            .context("failed to parse selected_browser_path")?;
    }
    if let Some(value) = object.get("expanded_browser_paths") {
        settings.expanded_browser_paths = serde_json::from_value(value.clone())
            .context("failed to parse expanded_browser_paths")?;
    }
    Ok(settings)
}

fn serialize_settings_value(settings: &AppSettings) -> Value {
    serde_json::json!({
        "theme_mode": settings.theme,
        "theme": settings.yggui_theme,
        "show_tree": settings.show_tree,
        "show_settings": settings.show_settings,
        "auto_hide_titlebar": settings.auto_hide_titlebar,
        "tree_width": settings.tree_width,
        "rendered_font_size": settings.rendered_font_size,
        "terminal_font_size": settings.terminal_font_size,
        "terminal_light_theme_name": settings.terminal_light_theme_name,
        "terminal_dark_theme_name": settings.terminal_dark_theme_name,
        "ui_font_size": settings.ui_font_size,
        "prefer_ghostty_backend": settings.prefer_ghostty_backend,
        "litellm_endpoint": settings.litellm_endpoint,
        "litellm_api_key": settings.litellm_api_key,
        "interface_llm_model": settings.interface_llm_model,
        "default_agent_profile": settings.default_agent_profile,
        "in_app_notifications": settings.in_app_notifications,
        "system_notifications": settings.system_notifications,
        "notification_sound": settings.notification_sound,
        "selected_browser_path": settings.selected_browser_path,
        "expanded_browser_paths": settings.expanded_browser_paths,
    })
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
                kind: SessionNodeKind::CodexSession,
                name: codex_leaf_label(&entry_path),
                title: None,
                document_kind: None,
                group_kind: None,
                path: entry_path,
                children: Vec::new(),
                session_id: None,
                cwd: None,
            });
        }
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(SessionNode {
        kind: SessionNodeKind::Group,
        name,
        title: None,
        document_kind: None,
        group_kind: None,
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
    modified_epoch_ms: u128,
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
    explicit_title: Option<String>,
    document: Option<WorkspaceDocumentSummary>,
    group_kind: Option<WorkspaceGroupKind>,
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
        project_sessions.sort_by(|a, b| {
            b.modified_epoch_ms
                .cmp(&a.modified_epoch_ms)
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        buckets.push(CodexProjectBucket {
            cwd,
            sessions: project_sessions,
        });
    }

    let mut root = CodexBrowserTreeNode {
        name: String::from("local [ok]"),
        full_path: String::from("local"),
        explicit_title: Some(String::from("local")),
        document: None,
        group_kind: Some(WorkspaceGroupKind::Folder),
        project: None,
        children: BTreeMap::new(),
    };

    if let Ok(workspace) = WorkspaceStore::open(home) {
        for document in workspace.list_documents().unwrap_or_default() {
            insert_workspace_document(&mut root, &document);
        }

        for group in workspace.list_groups().unwrap_or_default() {
            insert_workspace_group(&mut root, &group);
        }
    }

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
        generated_title: title_resolver.and_then(|resolver| {
            resolver
                .resolve_for_session(&identity.session_id)
                .ok()
                .flatten()
        }),
        session_id: identity.session_id,
        cwd: identity.cwd,
        modified_epoch_ms: fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|ts| ts.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
    }))
}

pub fn read_codex_session_identity_fields(path: &Path) -> Result<Option<(String, String)>> {
    Ok(read_codex_session_identity(path)?.map(|identity| (identity.session_id, identity.cwd)))
}

fn read_codex_session_identity(path: &Path) -> Result<Option<CodexSessionIdentity>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read codex session {}", path.display()))?;
    let reader = BufReader::new(file);

    let fallback_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.trim_start_matches("rollout-").to_string())
        .unwrap_or_else(|| String::from("unknown-session"));

    let mut session_id = None;
    let mut cwd = None;

    for line in reader.lines() {
        let line = line.with_context(|| format!("failed to read line from {}", path.display()))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
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
        if resolver
            .resolve_for_session(&identity.session_id)?
            .is_some()
        {
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
            explicit_title: None,
            document: None,
            group_kind: None,
            ..Default::default()
        });
    insert_codex_browser_path(child, &segments[1..], project);
}

fn insert_workspace_group(root: &mut CodexBrowserTreeNode, group: &WorkspaceGroup) {
    let segments = browser_tree_segments(&group.virtual_path);
    insert_workspace_group_path(root, &segments, group);
}

fn insert_workspace_group_path(
    node: &mut CodexBrowserTreeNode,
    segments: &[String],
    group: &WorkspaceGroup,
) {
    if segments.is_empty() {
        node.explicit_title = Some(group.title.clone());
        node.group_kind = Some(group.kind);
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
            explicit_title: None,
            document: None,
            group_kind: None,
            ..Default::default()
        });
    insert_workspace_group_path(child, &segments[1..], group);
}

fn insert_workspace_document(root: &mut CodexBrowserTreeNode, document: &WorkspaceDocumentSummary) {
    let segments = browser_tree_segments(&document.virtual_path);
    insert_workspace_document_path(root, &segments, document);
}

fn insert_workspace_document_path(
    node: &mut CodexBrowserTreeNode,
    segments: &[String],
    document: &WorkspaceDocumentSummary,
) {
    if segments.is_empty() {
        node.explicit_title = Some(document.title.clone());
        node.document = Some(document.clone());
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
            explicit_title: None,
            document: None,
            group_kind: None,
            ..Default::default()
        });
    insert_workspace_document_path(child, &segments[1..], document);
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
    if parent == "local" && segment == "/" {
        return "/".to_string();
    }
    if parent == "local" {
        return format!("/{}", segment.trim_matches('/'));
    }
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

    while node.project.is_none()
        && node.explicit_title.is_none()
        && node.document.is_none()
        && node.children.len() == 1
    {
        let (_, child) = node.children.pop_first().expect("single child exists");
        if child.explicit_title.is_some() || child.document.is_some() {
            node.children.insert(child.name.clone(), child);
            break;
        }
        node.name = join_session_label(&node.name, &child.name);
        node.full_path = child.full_path;
        node.document = child.document;
        node.project = child.project;
        node.children = child.children;
    }
}

fn codex_browser_tree_to_session_node(node: &CodexBrowserTreeNode) -> SessionNode {
    if let Some(document) = &node.document {
        return SessionNode {
            kind: SessionNodeKind::Document,
            name: document.title.clone(),
            title: Some(document.title.clone()),
            document_kind: Some(document.kind),
            group_kind: None,
            path: PathBuf::from(document.virtual_path.clone()),
            children: Vec::new(),
            session_id: Some(document.id.clone()),
            cwd: Some(document.virtual_path.clone()),
        };
    }

    let mut children = Vec::new();

    if let Some(project) = &node.project {
        children.extend(project.sessions.iter().map(|session| SessionNode {
            kind: SessionNodeKind::CodexSession,
            name: short_session_id(&session.session_id),
            title: session.generated_title.clone(),
            document_kind: None,
            group_kind: None,
            path: session.file_path.clone(),
            children: Vec::new(),
            session_id: Some(session.session_id.clone()),
            cwd: Some(project.cwd.clone()),
        }));
    }

    let mut nested_children = node
        .children
        .values()
        .map(codex_browser_tree_to_session_node)
        .collect::<Vec<_>>();
    nested_children.sort_by(cmp_browser_child_node);
    children.extend(nested_children);

    SessionNode {
        kind: SessionNodeKind::Group,
        name: node.name.clone(),
        title: node.explicit_title.clone(),
        document_kind: None,
        group_kind: node.group_kind,
        path: PathBuf::from(node.full_path.clone()),
        children,
        session_id: None,
        cwd: node.project.as_ref().map(|project| project.cwd.clone()),
    }
}

fn cmp_browser_child_node(left: &SessionNode, right: &SessionNode) -> std::cmp::Ordering {
    browser_child_sort_key(left)
        .cmp(&browser_child_sort_key(right))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.path.cmp(&right.path))
}

fn browser_child_sort_key(node: &SessionNode) -> String {
    node.path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&node.name)
        .to_ascii_lowercase()
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
    use super::*;

    #[test]
    fn browser_child_sort_respects_raw_virtual_path_order() {
        let folder = SessionNode {
            kind: SessionNodeKind::Group,
            name: "notes".to_string(),
            title: Some("Notes".to_string()),
            document_kind: None,
            group_kind: Some(WorkspaceGroupKind::Folder),
            path: PathBuf::from("/workspace/notes~0001"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };
        let document = SessionNode {
            kind: SessionNodeKind::Document,
            name: "paper".to_string(),
            title: Some("Paper".to_string()),
            document_kind: Some(WorkspaceDocumentKind::Note),
            group_kind: None,
            path: PathBuf::from("/workspace/notes~0000-paper"),
            children: Vec::new(),
            session_id: Some("paper-id".to_string()),
            cwd: Some("/workspace/paper".to_string()),
        };
        let separator = SessionNode {
            kind: SessionNodeKind::Group,
            name: "separator-1".to_string(),
            title: Some("Separator".to_string()),
            document_kind: None,
            group_kind: Some(WorkspaceGroupKind::Separator),
            path: PathBuf::from("/workspace/!separator-1"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
        };

        let mut nodes = vec![separator, document, folder];
        nodes.sort_by(cmp_browser_child_node);

        assert_eq!(nodes[0].path, PathBuf::from("/workspace/!separator-1"));
        assert_eq!(nodes[1].path, PathBuf::from("/workspace/notes~0000-paper"));
        assert_eq!(nodes[2].path, PathBuf::from("/workspace/notes~0001"));
    }

    #[test]
    fn browser_restore_only_keeps_level_one_groups_expanded() {
        let root = SessionNode {
            kind: SessionNodeKind::Group,
            name: "root".to_string(),
            title: None,
            document_kind: None,
            group_kind: Some(WorkspaceGroupKind::Folder),
            path: PathBuf::from("/workspace"),
            children: vec![SessionNode {
                kind: SessionNodeKind::Group,
                name: "machine-a".to_string(),
                title: Some("machine-a".to_string()),
                document_kind: None,
                group_kind: Some(WorkspaceGroupKind::Folder),
                path: PathBuf::from("/workspace/machine-a"),
                children: vec![SessionNode {
                    kind: SessionNodeKind::Group,
                    name: "nested".to_string(),
                    title: Some("nested".to_string()),
                    document_kind: None,
                    group_kind: Some(WorkspaceGroupKind::Folder),
                    path: PathBuf::from("/workspace/machine-a/nested"),
                    children: vec![SessionNode {
                        kind: SessionNodeKind::CodexSession,
                        name: "session-1".to_string(),
                        title: Some("session-1".to_string()),
                        document_kind: None,
                        group_kind: None,
                        path: PathBuf::from("/workspace/machine-a/nested/session-1"),
                        children: Vec::new(),
                        session_id: Some("session-1".to_string()),
                        cwd: Some("/workspace/machine-a/nested".to_string()),
                    }],
                    session_id: None,
                    cwd: None,
                }],
                session_id: None,
                cwd: None,
            }],
            session_id: None,
            cwd: None,
        };

        let mut browser = SessionBrowserState::new(root);
        browser.restore_ui_state(
            &["/workspace/machine-a/nested".to_string()],
            Some("/workspace/machine-a/nested/session-1"),
        );

        assert!(
            browser
                .rows()
                .iter()
                .any(|row| row.full_path == "/workspace/machine-a")
        );
        assert!(
            !browser
                .rows()
                .iter()
                .any(|row| row.full_path == "/workspace/machine-a/nested/session-1")
        );
    }

    #[test]
    fn settings_upgrade_legacy_terminal_theme_into_both_modes() {
        let parsed = parse_settings_value(&serde_json::json!({
            "terminal_theme_name": "Aardvark Blue"
        }))
        .expect("legacy settings should parse");
        assert_eq!(parsed.terminal_light_theme_name, "Aardvark Blue");
        assert_eq!(parsed.terminal_dark_theme_name, "Aardvark Blue");
    }

    #[test]
    fn settings_parse_titlebar_auto_hide_toggle() {
        let parsed = parse_settings_value(&serde_json::json!({
            "auto_hide_titlebar": true
        }))
        .expect("settings should parse");
        assert!(parsed.auto_hide_titlebar);
    }

    #[test]
    fn settings_serialize_titlebar_auto_hide_toggle() {
        let mut settings = AppSettings::default();
        settings.auto_hide_titlebar = true;
        assert_eq!(
            serialize_settings_value(&settings).get("auto_hide_titlebar"),
            Some(&serde_json::json!(true))
        );
    }
}
