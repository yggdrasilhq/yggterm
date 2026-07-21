pub mod app_registry;
mod browser;
mod icon;
mod install;
mod perf;
mod retention;
mod session_kind;
mod telemetry;
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
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use titles::{SessionTitleResolver, settings_ready as litellm_settings_ready};
pub use yggui_contract::{UiTheme, YgguiThemeColorStop, YgguiThemeSpec};

pub use app_registry::{
    APP_REGISTRY_DIRNAME, AppManifest, AppVerb, app_registry_dir, scan_app_registry,
    write_app_manifest,
};
pub use browser::{
    BrowserMetrics, BrowserRow, BrowserRowKind, SessionBrowserState,
    unique_session_short_ids_for_pairs,
};
pub use icon::{
    AppIconAssets, LinuxInstalledIconSet, YGGTERM_ICON_ASSETS, install_linux_icon_assets,
};
pub use install::{
    ENV_YGGTERM_DIRECT_INSTALL_ROOT, ENV_YGGTERM_ENABLE_NATIVE_IME, InstallChannel, InstallContext,
    ReleaseUpdate,
    ReleaseUpdateInstallProgress, ReleaseUpdateInstallStage, UpdatePolicy, YGGTERM_DESKTOP_APP_ID,
    check_for_update, current_asset_label, current_version, detect_install_context,
    direct_install_root, install_mode_summary, install_release_update,
    install_release_update_with_progress, promote_direct_install_active_version,
    refresh_desktop_integration, update_command_hint, write_direct_install_state,
};
pub use perf::{
    PERF_INCIDENT_FILENAME, PERF_TELEMETRY_FILENAME, PERF_TELEMETRY_MAX_BYTES, PerfGuard,
    PerfSpan, PerfSpanSummary, append_bounded_jsonl_record, append_perf_event, detect_perf_incident,
    perf_profiling_enabled, perf_telemetry_path, record_perf_incident_if_hot,
    set_perf_profiling_enabled, summarize_perf_telemetry,
};
pub use retention::{
    DIAGNOSTIC_RETENTION_MAX_AGE_MS, JsonlRetention, append_retained_jsonl_record,
    jsonl_read_paths, prune_jsonl_generations, rotate_jsonl_with_retention,
};
pub use session_kind::SessionKind;
pub use telemetry::{
    TERMINAL_TELEMETRY_DB_FILENAME, TERMINAL_TELEMETRY_DIRNAME, TerminalTelemetryEvent,
    append_terminal_telemetry_event, ensure_terminal_telemetry_schema,
    spawn_terminal_telemetry_event, terminal_telemetry_db_path,
};
pub use titles::{
    SessionSummaryTimelineEntry, SessionTitleStore, best_effort_context_from_session_path,
    best_effort_precis_from_context, best_effort_summary_from_context,
    best_effort_title_from_context, looks_like_generated_fallback_title,
    looks_like_low_signal_generated_copy,
};
pub use trace::{
    EVENT_TRACE_FILENAME, EventTraceRecord, EventTraceSpan, append_trace_event, event_trace_path,
    follow_trace_lines, read_trace_tail,
};
pub use transcript::{
    TranscriptMessage, TranscriptRole, generation_context_from_messages,
    message_lines_from_payload, read_codex_transcript_messages,
    read_codex_transcript_messages_limited, read_codex_transcript_messages_tail_limited,
};
pub use workspace::{
    WorkspaceDocument, WorkspaceDocumentInput, WorkspaceDocumentKind, WorkspaceDocumentSummary,
    WorkspaceGroup, WorkspaceGroupKind, WorkspaceStore, default_document_title,
    normalize_virtual_document_path, normalize_virtual_group_path,
};

pub const ENV_YGGTERM_HOME: &str = "YGGTERM_HOME";
pub const ENV_YGGTERM_CODEX_HOME: &str = "YGGTERM_CODEX_HOME";
pub const DEFAULT_HOME_DIRNAME: &str = ".yggterm";
pub const DEFAULT_CODEX_HOME_DIRNAME: &str = ".codex";
pub const SESSIONS_DIRNAME: &str = "sessions";
pub const SETTINGS_FILENAME: &str = "settings.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    /// Authoritative `SessionKind` for agent-CLI session leaves. Carried
    /// from the source scanner (Codex, Claude Code, future CLIs) into the
    /// derived `BrowserRow.session_kind`. `None` for groups, documents, and
    /// historical tree paths where kind cannot be derived from the path
    /// alone. See [[spec-cwd-tree-agent-cli-unified]].
    #[serde(default)]
    pub session_kind: Option<SessionKind>,
    /// Optional detail string for session leaves (e.g. CC's first-user-message
    /// context hint). When `None`, callers compute a default from `cwd` and
    /// `session_id`. Carried into `BrowserRow.detail_label` by the tree
    /// flattener. See [[spec-cwd-tree-agent-cli-unified]].
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionNodeKind {
    #[default]
    Group,
    /// Leaf node representing a saved agent-CLI session (Codex, Claude Code,
    /// or future agent CLIs). The actual SessionKind is carried on
    /// `SessionNode.session_kind`. Variant name retained for serde compat
    /// with previously-persisted snapshots; new code should treat this as
    /// "agent session" regardless of which CLI wrote the file.
    #[serde(alias = "agent_session")]
    CodexSession,
    Document,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    home: PathBuf,
    sessions_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionCopyRegenerationReport {
    pub scanned: usize,
    pub title_generated: usize,
    pub precis_generated: usize,
    pub summary_generated: usize,
    pub summary_history_reset: usize,
    pub skipped: usize,
    pub failed: Vec<SessionCopyRegenerationFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCopyRegenerationFailure {
    pub session_id: String,
    pub path: String,
    pub stage: String,
    pub error: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionProfile {
    #[default]
    Codex,
    CodexLiteLlm,
}

/// How the two panes of a [`SplitGroup`] are arranged in the viewport, and
/// therefore how the compound sidebar row mirrors that geometry
/// ([[campaign-split-view-groups]]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    /// Left | Right. Panes sit side by side; the compound row shows cells
    /// separated by a vertical `|`; the divider is vertical.
    SideBySide,
    /// Top / Bottom. Panes are stacked; the compound row shows two stacked
    /// lines; the divider is horizontal.
    Stacked,
}

impl Default for SplitAxis {
    fn default() -> Self {
        SplitAxis::SideBySide
    }
}

/// Which of its member session's surfaces a split pane shows
/// ([[campaign-libyggterm]] Phase 3). This is PANE state, not session state:
/// a session already knows its own surfaces, but "this pane shows tab 4" is a
/// fact about the pane — the session has ONE active tab, and two panes may
/// show the same session through different views (split-tabs).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SplitMemberView {
    /// The session's own surface: terminal pixels, or whatever viewport
    /// surface the session's app currently declares (document / web active
    /// tab). The entire pre-3.x split world is this variant — it FOLLOWS the
    /// session rather than pinning anything, so an app opening or closing its
    /// surface inside a split pane behaves exactly like full-bleed.
    #[default]
    Terminal,
    /// Pinned to one document-pane view context (`?view=<context>` on the
    /// schema GET). Apps today declare only the default context; the variant
    /// exists so a second view lands as data, not a new mechanism.
    Document { context: String },
    /// Pinned to one web tab of the session's surface, independent of the
    /// surface's active tab (split-tabs: two tab webviews, two rects).
    Web { tab: u64 },
}

impl SplitMemberView {
    pub fn is_terminal(&self) -> bool {
        matches!(self, SplitMemberView::Terminal)
    }
}

/// One pane of a [`SplitGroup`]: which session, seen through which view.
///
/// Wire/disk compatibility: a `Terminal`-view member serializes as the BARE
/// session-path string (the pre-3.x format), so persisted settings and
/// app-control JSON stay byte-identical until a pinned view actually exists;
/// deserialization accepts both forms.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitMember {
    /// The member session's path (the daemon-owned PTY identity).
    pub session: String,
    /// Which of the session's surfaces this pane shows.
    pub view: SplitMemberView,
}

impl SplitMember {
    /// The default member: the session seen through its own surface.
    pub fn terminal(session: impl Into<String>) -> Self {
        SplitMember {
            session: session.into(),
            view: SplitMemberView::Terminal,
        }
    }
}

impl Serialize for SplitMember {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.view.is_terminal() {
            serializer.serialize_str(&self.session)
        } else {
            use serde::ser::SerializeStruct as _;
            let mut out = serializer.serialize_struct("SplitMember", 2)?;
            out.serialize_field("session", &self.session)?;
            out.serialize_field("view", &self.view)?;
            out.end()
        }
    }
}

impl<'de> Deserialize<'de> for SplitMember {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Path(String),
            Full {
                session: String,
                #[serde(default)]
                view: SplitMemberView,
            },
        }
        Ok(match Repr::deserialize(deserializer)? {
            Repr::Path(session) => SplitMember::terminal(session),
            Repr::Full { session, view } => SplitMember { session, view },
        })
    }
}

/// A GUI-level split-view group — the single source of truth for a split
/// ([[campaign-split-view-groups]]). Members keep their own daemon-owned PTYs
/// (possibly on different machines: cross-host split is the differentiator tmux
/// cannot match); only the GUI composes them into one surface. The compound
/// sidebar row, the viewport layout, and persistence all DERIVE from this
/// object — there are no per-session split flags anywhere.
///
/// MVP is 2 panes (`members.len() == 2`); 2×2 via drop-onto-cell is phase 2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitGroup {
    /// Stable identity, also the synthetic sidebar path (`split://<id>`).
    pub group_id: String,
    /// Pane arrangement (drives both viewport rects and the compound row).
    #[serde(default)]
    pub axis: SplitAxis,
    /// Fraction of the split axis the FIRST member occupies (0.0..1.0). The
    /// draggable divider writes this; persisted so a workspace reopens as built.
    #[serde(default = "default_split_ratio")]
    pub ratio: f32,
    /// Member panes in order: `(session, view)` each. MVP length is 2.
    pub members: Vec<SplitMember>,
    /// The last-focused pane index — which member gets input focus when the
    /// group is (re)activated. When the group is the active surface this stays
    /// in sync with `active_session_path`; it is genuinely distinct state only
    /// while the group is backgrounded.
    #[serde(default)]
    pub active_pane: usize,
    /// Each member's keep-alive setting BEFORE it was grouped. Grouping forces
    /// keep-alive on every member (the group is the keep-alive declaration);
    /// ungrouping restores these so a throwaway shell does not stay immortal
    /// after a brief split.
    #[serde(default)]
    pub prior_keep_alive: BTreeMap<String, bool>,
}

fn default_split_ratio() -> f32 {
    0.5
}

impl SplitGroup {
    /// Synthetic sidebar/tree path for a group id.
    pub fn path_for_id(group_id: &str) -> String {
        format!("split://{group_id}")
    }

    /// This group's synthetic sidebar path.
    pub fn synthetic_path(&self) -> String {
        Self::path_for_id(&self.group_id)
    }

    /// Does any pane of this group show `session_path` (through ANY view)?
    pub fn contains(&self, session_path: &str) -> bool {
        self.members
            .iter()
            .any(|member| member.session == session_path)
    }

    /// The member session paths in pane order. May repeat a session when two
    /// panes show it through different views — dedup at the call site when the
    /// consumer is per-session rather than per-pane.
    pub fn member_sessions(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|member| member.session.as_str())
    }

    /// Clamp `active_pane` and `ratio` into valid ranges (defensive against a
    /// persisted file from a newer/older build or a removed member).
    pub fn normalized(mut self) -> Self {
        if self.members.is_empty() {
            self.active_pane = 0;
        } else if self.active_pane >= self.members.len() {
            self.active_pane = self.members.len() - 1;
        }
        if !self.ratio.is_finite() {
            self.ratio = 0.5;
        }
        self.ratio = self.ratio.clamp(0.15, 0.85);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppSettings {
    pub theme: UiTheme,
    pub yggui_theme: YgguiThemeSpec,
    pub show_tree: bool,
    pub show_settings: bool,
    pub auto_hide_titlebar: bool,
    pub window_maximized: bool,
    pub tree_width: f32,
    /// Width of the right metadata/settings rail, in px. Independently draggable
    /// from the left tree ([[spec-sidebar-auto-hide-hover-overlay]]).
    pub rail_width: f32,
    pub rendered_font_size: f32,
    pub terminal_font_size: f32,
    /// Global zoom for native web surfaces (ychrome / libyggterm web apps),
    /// as percent points where 100.0 == 100%. Unlike the terminal/rendered
    /// font sizes this is a webview zoom factor (value / 100 handed to
    /// `WebView::zoom`), because a native web surface is an overlaid WebKit
    /// view, not DOM the shell can style. Per-site zoom (ychrome) overrides it.
    pub web_surface_zoom_percent: f32,
    /// Vertical-tabs browsing mode for web surfaces: the tabs move OUT of the
    /// viewport into a side rail (the tab tree, with virtual folders — the cwd
    /// tree's organizational grammar applied to tabs) and the top tab bar
    /// collapses. A persisted per-user preference; the surface chrome reads it,
    /// so it applies to every web-surface app (ychrome and future ones). The
    /// control for it is drawn by the app's own settings pane (page context in,
    /// `surface_prefs` out) — yggterm owns the tabs, so yggterm owns the pref.
    pub web_surface_vertical_tabs: bool,
    /// Reopen the previous visit's tabs when a web surface opens ("continue where
    /// you left off"). OFF by default: a fresh visit starts on the app's page.
    ///
    /// The saved structure is never lost either way. A tab FILED IN A FOLDER is
    /// organization, like a cwd-tree folder, and survives both modes; only the
    /// unfiled root tabs are the browsing session, and those are what a fresh
    /// start purges.
    pub web_surface_restore_tabs: bool,
    pub terminal_light_theme_name: String,
    pub terminal_dark_theme_name: String,
    pub ui_font_size: f32,
    pub prefer_ghostty_backend: bool,
    pub litellm_endpoint: String,
    pub litellm_api_key: String,
    pub interface_llm_model: String,
    pub codex_extra_args: String,
    pub claude_code_extra_args: String,
    pub default_agent_profile: AgentSessionProfile,
    pub in_app_notifications: bool,
    pub system_notifications: bool,
    pub notification_sound: bool,
    pub terminal_telemetry_enabled: bool,
    /// App profiling system: when on, timing spans on hot paths (terminal attach,
    /// persist, snapshot, render) are written to `perf-telemetry.jsonl` for
    /// `server perf-summary` analysis. Separate from terminal telemetry because it is
    /// a heavier, developer-facing diagnostic. Gates [`set_perf_profiling_enabled`].
    pub perf_profiling_enabled: bool,
    pub selected_browser_path: Option<String>,
    pub expanded_browser_paths: Vec<String>,
    /// Synthetic sidebar groups (machine roots, remote folders, Live Sessions)
    /// the user explicitly collapsed. Persisted so a collapse survives GUI
    /// restarts: the auto-reveal lanes (active-session visibility, dynamic
    /// top-level seeding) must respect this set across processes, not just
    /// within one.
    pub collapsed_synthetic_paths: Vec<String>,
    /// GUI-level split-view groups ([[campaign-split-view-groups]]). Persisted
    /// so a built workspace reopens as an intentional artifact; members resume
    /// via the normal per-pane handoff on restore.
    pub split_groups: Vec<SplitGroup>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: UiTheme::ZedLight,
            yggui_theme: YgguiThemeSpec::default(),
            show_tree: true,
            show_settings: false,
            auto_hide_titlebar: false,
            window_maximized: false,
            tree_width: 300.0,
            rail_width: 292.0,
            rendered_font_size: 10.0,
            terminal_font_size: 14.0,
            web_surface_zoom_percent: 100.0,
            web_surface_vertical_tabs: false,
            web_surface_restore_tabs: false,
            terminal_light_theme_name: "VS Code Light+".to_string(),
            terminal_dark_theme_name: "Dark+".to_string(),
            ui_font_size: 14.0,
            prefer_ghostty_backend: true,
            litellm_endpoint: String::new(),
            litellm_api_key: String::new(),
            interface_llm_model: String::new(),
            codex_extra_args: String::new(),
            claude_code_extra_args: String::new(),
            default_agent_profile: AgentSessionProfile::Codex,
            in_app_notifications: true,
            system_notifications: false,
            notification_sound: false,
            terminal_telemetry_enabled: true,
            perf_profiling_enabled: true,
            selected_browser_path: None,
            expanded_browser_paths: Vec::new(),
            collapsed_synthetic_paths: Vec::new(),
            split_groups: Vec::new(),
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
        // Preserved name for API compat; internally now builds the unified
        // local cwd tree from all agent-CLI scanners (Codex, Claude Code,
        // future). See [[spec-cwd-tree-agent-cli-unified]].
        build_local_cwd_tree(&self.home, settings)
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

    pub fn save_manual_summary_for_session_id(
        &self,
        session_id: &str,
        cwd: &str,
        summary: &str,
    ) -> Result<()> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.save_manual_summary_for_session(session_id, cwd, summary)
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

    pub fn summary_timeline_for_session_id(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionSummaryTimelineEntry>> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.summary_timeline_for_session(session_id, limit)
    }

    pub fn reset_summary_timeline_for_session_id(&self, session_id: &str) -> Result<()> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.reset_summary_timeline_for_session(session_id)
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

    /// Short-horizon variant for LIVE working sessions (summary timelines).
    pub fn summary_needs_refresh_for_live_session_id(
        &self,
        session_id: &str,
        source_updated_at: OffsetDateTime,
    ) -> Result<bool> {
        let resolver = SessionTitleResolver::new(&self.home)?;
        resolver.summary_needs_refresh_with_horizon(
            session_id,
            source_updated_at,
            time::Duration::minutes(30),
        )
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

    pub fn regenerate_codex_session_copy(
        &self,
        settings: &AppSettings,
        budget: usize,
        force: bool,
        reset_summary_history: bool,
    ) -> Result<SessionCopyRegenerationReport> {
        let codex_root = resolve_codex_sessions_root()?;
        let resolver = SessionTitleResolver::new(&self.home)?;
        let mut sessions = if codex_root.exists() {
            scan_local_codex_sessions(&codex_root, Some(&resolver))?
        } else {
            Vec::new()
        };
        sessions.sort_by(|a, b| {
            b.modified_epoch_ms
                .cmp(&a.modified_epoch_ms)
                .then_with(|| a.session_id.cmp(&b.session_id))
        });

        let mut report = SessionCopyRegenerationReport::default();
        let limit = if budget == 0 { usize::MAX } else { budget };
        for session in sessions.into_iter().take(limit) {
            report.scanned += 1;
            let path = session.file_path.to_string_lossy().to_string();
            let mut touched = false;
            if reset_summary_history {
                match resolver.reset_summary_timeline_for_session(&session.session_id) {
                    Ok(()) => {
                        report.summary_history_reset += 1;
                        touched = true;
                    }
                    Err(error) => {
                        report.failed.push(SessionCopyRegenerationFailure {
                            session_id: session.session_id.clone(),
                            path: path.clone(),
                            stage: "reset_summary_history".to_string(),
                            error: error.to_string(),
                        });
                        continue;
                    }
                }
            }

            let title_was_missing = resolver
                .resolve_for_session(&session.session_id)
                .ok()
                .flatten()
                .as_deref()
                .is_none_or(|title| {
                    title.trim().is_empty() || looks_like_generated_fallback_title(title)
                });
            if force || title_was_missing {
                match resolver.generate_for_session(
                    settings,
                    &session.session_id,
                    &session.cwd,
                    &session.file_path,
                    force,
                ) {
                    Ok(Some(_)) => {
                        report.title_generated += 1;
                        touched = true;
                    }
                    Ok(None) => {}
                    Err(error) => report.failed.push(SessionCopyRegenerationFailure {
                        session_id: session.session_id.clone(),
                        path: path.clone(),
                        stage: "title".to_string(),
                        error: error.to_string(),
                    }),
                }
            }

            let precis_was_missing = resolver
                .resolve_precis_for_session(&session.session_id)
                .ok()
                .flatten()
                .as_deref()
                .is_none_or(looks_like_low_signal_generated_copy);
            if force || precis_was_missing {
                match resolver.generate_precis_for_session(
                    settings,
                    &session.session_id,
                    &session.cwd,
                    &session.file_path,
                    force,
                ) {
                    Ok(Some(_)) => {
                        report.precis_generated += 1;
                        touched = true;
                    }
                    Ok(None) => {}
                    Err(error) => report.failed.push(SessionCopyRegenerationFailure {
                        session_id: session.session_id.clone(),
                        path: path.clone(),
                        stage: "precis".to_string(),
                        error: error.to_string(),
                    }),
                }
            }

            let source_updated_at = session
                .modified_epoch_ms
                .checked_div(1000)
                .and_then(|secs| i64::try_from(secs).ok())
                .and_then(|secs| OffsetDateTime::from_unix_timestamp(secs).ok());
            let summary_was_missing = resolver
                .resolve_summary_for_session(&session.session_id)
                .ok()
                .flatten()
                .as_deref()
                .is_none_or(looks_like_low_signal_generated_copy);
            let summary_stale = source_updated_at
                .and_then(|updated_at| {
                    resolver
                        .summary_needs_refresh(&session.session_id, updated_at)
                        .ok()
                })
                .unwrap_or(summary_was_missing);
            if force || reset_summary_history || summary_was_missing || summary_stale {
                match resolver.generate_summary_for_session(
                    settings,
                    &session.session_id,
                    &session.cwd,
                    &session.file_path,
                    force || reset_summary_history || summary_stale,
                ) {
                    Ok(Some(_)) => {
                        report.summary_generated += 1;
                        touched = true;
                    }
                    Ok(None) => {}
                    Err(error) => report.failed.push(SessionCopyRegenerationFailure {
                        session_id: session.session_id.clone(),
                        path: path.clone(),
                        stage: "summary".to_string(),
                        error: error.to_string(),
                    }),
                }
            }

            if !touched {
                report.skipped += 1;
            }
        }
        Ok(report)
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
    if let Some(value) = object.get("window_maximized") {
        settings.window_maximized =
            serde_json::from_value(value.clone()).context("failed to parse window_maximized")?;
    }
    if let Some(value) = object.get("tree_width") {
        settings.tree_width =
            serde_json::from_value(value.clone()).context("failed to parse tree_width")?;
    }
    if let Some(value) = object.get("rail_width") {
        settings.rail_width =
            serde_json::from_value(value.clone()).context("failed to parse rail_width")?;
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
    if let Some(value) = object.get("codex_extra_args") {
        settings.codex_extra_args =
            serde_json::from_value(value.clone()).context("failed to parse codex_extra_args")?;
    }
    if let Some(value) = object.get("claude_code_extra_args") {
        settings.claude_code_extra_args = serde_json::from_value(value.clone())
            .context("failed to parse claude_code_extra_args")?;
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
    if let Some(value) = object.get("terminal_telemetry_enabled") {
        settings.terminal_telemetry_enabled = serde_json::from_value(value.clone())
            .context("failed to parse terminal_telemetry_enabled")?;
    }
    if let Some(value) = object.get("perf_profiling_enabled") {
        settings.perf_profiling_enabled = serde_json::from_value(value.clone())
            .context("failed to parse perf_profiling_enabled")?;
    }
    if let Some(value) = object.get("selected_browser_path") {
        settings.selected_browser_path = serde_json::from_value(value.clone())
            .context("failed to parse selected_browser_path")?;
    }
    if let Some(value) = object.get("expanded_browser_paths") {
        settings.expanded_browser_paths = serde_json::from_value(value.clone())
            .context("failed to parse expanded_browser_paths")?;
    }
    if let Some(value) = object.get("collapsed_synthetic_paths") {
        settings.collapsed_synthetic_paths = serde_json::from_value(value.clone())
            .context("failed to parse collapsed_synthetic_paths")?;
    }
    if let Some(value) = object.get("split_groups") {
        settings.split_groups = serde_json::from_value::<Vec<SplitGroup>>(value.clone())
            .context("failed to parse split_groups")?
            .into_iter()
            .map(SplitGroup::normalized)
            .collect();
    }
    if let Some(value) = object.get("web_surface_zoom_percent") {
        settings.web_surface_zoom_percent = serde_json::from_value(value.clone())
            .context("failed to parse web_surface_zoom_percent")?;
    }
    if let Some(value) = object.get("web_surface_vertical_tabs") {
        settings.web_surface_vertical_tabs = serde_json::from_value(value.clone())
            .context("failed to parse web_surface_vertical_tabs")?;
    }
    if let Some(value) = object.get("web_surface_restore_tabs") {
        settings.web_surface_restore_tabs = serde_json::from_value(value.clone())
            .context("failed to parse web_surface_restore_tabs")?;
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
        "window_maximized": settings.window_maximized,
        "tree_width": settings.tree_width,
        "rail_width": settings.rail_width,
        "rendered_font_size": settings.rendered_font_size,
        "terminal_font_size": settings.terminal_font_size,
        "terminal_light_theme_name": settings.terminal_light_theme_name,
        "terminal_dark_theme_name": settings.terminal_dark_theme_name,
        "ui_font_size": settings.ui_font_size,
        "prefer_ghostty_backend": settings.prefer_ghostty_backend,
        "litellm_endpoint": settings.litellm_endpoint,
        "litellm_api_key": settings.litellm_api_key,
        "interface_llm_model": settings.interface_llm_model,
        "codex_extra_args": settings.codex_extra_args,
        "claude_code_extra_args": settings.claude_code_extra_args,
        "default_agent_profile": settings.default_agent_profile,
        "in_app_notifications": settings.in_app_notifications,
        "system_notifications": settings.system_notifications,
        "notification_sound": settings.notification_sound,
        "terminal_telemetry_enabled": settings.terminal_telemetry_enabled,
        "perf_profiling_enabled": settings.perf_profiling_enabled,
        "selected_browser_path": settings.selected_browser_path,
        "expanded_browser_paths": settings.expanded_browser_paths,
        "collapsed_synthetic_paths": settings.collapsed_synthetic_paths,
        "split_groups": settings.split_groups,
        // The web-surface prefs. They were absent here, so NONE of them survived a
        // restart — including the global zoom, which had shipped as "persisted"
        // for weeks. A hand-written serializer beside a hand-written parser is two
        // encodings of one struct, and a field added to only the struct is silently
        // never saved; `every_settings_field_is_written_to_the_file` now fails the
        // build when that happens.
        "web_surface_zoom_percent": settings.web_surface_zoom_percent,
        "web_surface_vertical_tabs": settings.web_surface_vertical_tabs,
        "web_surface_restore_tabs": settings.web_surface_restore_tabs,
    })
}

/// Single source of truth for "is an agent CLI actively working right now",
/// derived purely from the live screen text. CLI-agnostic: Codex renders
/// `Working (Ns • esc to interrupt)`, Claude Code renders
/// `✻ <gerund>… (Ns · esc to interrupt)` — the shared, unambiguous "I'm busy,
/// press esc to stop" signal is `esc to interrupt`, which neither CLI shows
/// when idle. Codex-only background-task indicators (`/stop to close`,
/// `background terminal running`) are kept as a fallback.
///
/// Both the sidebar working-indicator (GUI) and the hot-update idle gate
/// (daemon) MUST use this one function so the displayed "working" state and
/// the "is it safe to hot-update" decision can never silently diverge.
/// See [[finding-hot-update-interrupts-remote-sessions]].
pub fn screen_text_shows_agent_working(sample: &str) -> bool {
    sample.lines().rev().take(10).any(|line| {
        let line = line.trim();
        if line.is_empty() {
            return false;
        }
        let lower = line.to_ascii_lowercase();
        if lower.contains("worked for ") {
            // Codex completion summary ("Worked for Ns"), not active work.
            return false;
        }
        // Universal active-processing signal shown by both Codex and Claude Code.
        if lower.contains("esc to interrupt") {
            return true;
        }
        // Codex-only background-task indicators (no "esc to interrupt" line).
        lower.contains("working (")
            && (lower.contains("/stop to close")
                || lower.contains("background terminal running"))
    })
}

/// Single source of truth for "does the current input line hold an unsent
/// draft after feeding these input bytes", starting from `prev`. This is the
/// daemon-side safety signal that PROTECTS a session a user typed into but
/// never submitted: such text lives only in the running process / PTY line
/// buffer (never the agent JSONL), so a release+re-resume migration would
/// silently lose it. The flag is STICKY across calls — "typed then walked
/// away" stays protected until the line is actually submitted or killed.
///
/// Escape / control sequences are skipped so they can NEVER fabricate a draft:
/// arrow keys (`ESC [ A`), F-keys (`ESC O P`), mouse reports
/// (`ESC [ < 0;34;12 M`) and the bracketed-paste markers (`ESC [ 200~` /
/// `ESC [ 201~`) all carry printable-looking bytes (digits, letters) that must
/// not be mistaken for typed text. Note the PASTED CONTENT between the markers
/// is NOT skipped — a paste into the line genuinely is a draft.
///
/// Rules on the un-escaped stream:
/// - `\r` / `\n` (submit) and Ctrl-C (`0x03`) / Ctrl-U (`0x15`) clear the draft.
/// - Backspace (`0x08` / `0x7f`) is a no-op: we track a bool, not the line
///   contents, so we conservatively keep the draft (false-positive draft is
///   safe — it only delays a migration; a false-negative would lose work).
/// - Any printable, non-whitespace char sets the draft.
///
/// Mirrors the GUI's per-keystroke optimistic busy hint
/// (`terminal_input_busy_hint_decision`) but is byte-level, escape-aware, and
/// returns only the sticky-draft bit the migration predicate needs.
pub fn input_line_has_unsent_draft_after(prev: bool, data: &[u8]) -> bool {
    #[derive(Clone, Copy)]
    enum EscState {
        Normal,
        Escape,
        Csi,
        Ss3,
        Osc,
    }
    let mut draft = prev;
    let mut state = EscState::Normal;
    for &byte in data {
        match state {
            EscState::Normal => match byte {
                0x1b => state = EscState::Escape,
                b'\r' | b'\n' => draft = false,
                0x03 | 0x15 => draft = false,
                0x08 | 0x7f => {}
                // Printable, non-whitespace bytes (incl. UTF-8 continuation
                // bytes >= 0x80) are typed text. Control bytes (< 0x20) and
                // ASCII whitespace are ignored.
                b if b >= 0x20 && b != b' ' && b != b'\t' => draft = true,
                _ => {}
            },
            EscState::Escape => {
                state = match byte {
                    b'[' => EscState::Csi,
                    b'O' => EscState::Ss3,
                    b']' => EscState::Osc,
                    // Two-byte ESC sequence (e.g. Alt-key, ESC ESC) — consume
                    // this one byte and resume.
                    _ => EscState::Normal,
                };
            }
            EscState::Csi => {
                // CSI parameter/intermediate bytes run until a final byte in
                // 0x40..=0x7e. Bracketed-paste `ESC [ 200~` terminates at `~`,
                // so the pasted content that follows is parsed as Normal text.
                if (0x40..=0x7e).contains(&byte) {
                    state = EscState::Normal;
                }
            }
            EscState::Ss3 => state = EscState::Normal,
            EscState::Osc => {
                // OSC strings end at BEL; the ST form (ESC \\) resolves on the
                // ESC, which re-enters Escape and consumes the trailing `\\`.
                match byte {
                    0x07 => state = EscState::Normal,
                    0x1b => state = EscState::Escape,
                    _ => {}
                }
            }
        }
    }
    draft
}

pub fn resolve_yggterm_home() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os(ENV_YGGTERM_HOME) {
        let p = PathBuf::from(value);
        return Ok(expand_tilde(p));
    }

    let home_dir = dirs::home_dir().context("unable to resolve home directory")?;
    Ok(home_dir.join(DEFAULT_HOME_DIRNAME))
}

pub fn resolve_codex_home() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os(ENV_YGGTERM_CODEX_HOME) {
        let p = PathBuf::from(value);
        return Ok(expand_tilde(p));
    }

    let home_dir = dirs::home_dir().context("unable to resolve home directory")?;
    Ok(home_dir.join(DEFAULT_CODEX_HOME_DIRNAME))
}

pub fn resolve_codex_sessions_root() -> Result<PathBuf> {
    Ok(resolve_codex_home()?.join(SESSIONS_DIRNAME))
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
                path: entry_path,
                ..Default::default()
            });
        }
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(SessionNode {
        name,
        path: path.to_path_buf(),
        children,
        ..Default::default()
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

/// Unified summary of a saved agent-CLI session that the local cwd tree
/// builder consumes. One per session file, regardless of which CLI wrote it.
/// Per [[spec-cwd-tree-agent-cli-unified]]: adding a new agent CLI means
/// writing a `scan_local_<cli>_sessions() -> Vec<LocalAgentSessionSummary>`
/// scanner — never adding a parallel injection pass into the row list.
#[derive(Debug, Clone)]
pub struct LocalAgentSessionSummary {
    pub kind: SessionKind,
    pub file_path: PathBuf,
    pub session_id: String,
    pub cwd: String,
    pub title: Option<String>,
    /// Optional detail line (e.g. CC's first-user-message context hint).
    /// `None` means the tree flattener computes a default from cwd + id.
    pub detail: Option<String>,
    pub modified_epoch_ms: u128,
}

#[derive(Debug, Clone)]
struct CodexSessionIdentity {
    session_id: String,
    cwd: String,
}

#[derive(Debug, Clone)]
struct LocalAgentProjectBucket {
    cwd: String,
    sessions: Vec<LocalAgentSessionSummary>,
}

#[derive(Debug, Clone, Default)]
struct CodexBrowserTreeNode {
    name: String,
    full_path: String,
    explicit_title: Option<String>,
    document: Option<WorkspaceDocumentSummary>,
    group_kind: Option<WorkspaceGroupKind>,
    project: Option<LocalAgentProjectBucket>,
    children: BTreeMap<String, CodexBrowserTreeNode>,
}

/// Build the unified local cwd tree from all known agent-CLI session
/// scanners. Per [[spec-cwd-tree-agent-cli-unified]] every CLI's sessions
/// flow through the same tree-building pipeline; there is no per-CLI
/// injection pass. To add a new CLI: write a `scan_local_<cli>_sessions()`
/// that returns `Vec<LocalAgentSessionSummary>` and call it here.
fn build_local_cwd_tree(home: &Path, _settings: &AppSettings) -> Result<SessionNode> {
    let title_resolver = SessionTitleResolver::new(home).ok();

    let mut sessions: Vec<LocalAgentSessionSummary> = Vec::new();
    let codex_root = resolve_codex_sessions_root()?;
    if codex_root.exists() {
        sessions.extend(scan_local_codex_sessions(
            &codex_root,
            title_resolver.as_ref(),
        )?);
    }
    sessions.extend(scan_local_claude_code_sessions());

    let mut projects = BTreeMap::<String, Vec<LocalAgentSessionSummary>>::new();
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
        buckets.push(LocalAgentProjectBucket {
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

/// Scan local Codex session JSONL files and return unified summaries
/// ready for the cwd tree builder. Per [[spec-cwd-tree-agent-cli-unified]].
pub fn scan_local_codex_sessions(
    root: &Path,
    title_resolver: Option<&SessionTitleResolver>,
) -> Result<Vec<LocalAgentSessionSummary>> {
    let mut sessions = Vec::new();
    for entry in
        fs::read_dir(root).with_context(|| format!("failed to read dir {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            sessions.extend(scan_local_codex_sessions(&path, title_resolver)?);
        } else if is_codex_session_file(&path) {
            if let Some(summary) = read_local_codex_session_summary(&path, title_resolver)? {
                sessions.push(summary);
            }
        }
    }
    Ok(sessions)
}

/// Scan local Claude Code session JSONL files (~/.claude/projects) and
/// return them as `LocalAgentSessionSummary` records. Mirrors
/// `scan_local_codex_sessions` so both flow through the same tree builder.
/// Per [[spec-cwd-tree-agent-cli-unified]] this replaces the prior
/// post-hoc `inject_file_backed_cc_session_rows` injection path.
pub fn scan_local_claude_code_sessions() -> Vec<LocalAgentSessionSummary> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let projects_dir = home.join(".claude").join("projects");
    let Ok(project_entries) = fs::read_dir(&projects_dir) else {
        return Vec::new();
    };
    let mut sessions = Vec::new();
    for project_entry in project_entries.flatten() {
        let project_path = project_entry.path();
        if !project_path.is_dir() {
            continue;
        }
        let Ok(session_entries) = fs::read_dir(&project_path) else {
            continue;
        };
        for session_entry in session_entries.flatten() {
            let file_path = session_entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(Some((session_id, cwd))) = read_cc_session_identity_fields(&file_path) else {
                continue;
            };
            let title = read_cc_session_title(&file_path).ok().flatten();
            let detail = read_cc_session_context(&file_path)
                .ok()
                .filter(|s| !s.trim().is_empty());
            let modified_epoch_ms = fs::metadata(&file_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or_default();
            sessions.push(LocalAgentSessionSummary {
                kind: SessionKind::ClaudeCode,
                file_path,
                session_id,
                cwd,
                title,
                detail,
                modified_epoch_ms,
            });
        }
    }
    sessions
}

fn read_local_codex_session_summary(
    path: &Path,
    title_resolver: Option<&SessionTitleResolver>,
) -> Result<Option<LocalAgentSessionSummary>> {
    let Some(identity) = read_codex_session_identity(path)? else {
        return Ok(None);
    };

    Ok(Some(LocalAgentSessionSummary {
        kind: SessionKind::Codex,
        file_path: path.to_path_buf(),
        title: title_resolver.and_then(|resolver| {
            resolver
                .resolve_for_session(&identity.session_id)
                .ok()
                .flatten()
        }),
        // Codex doesn't carry a first-user-message context the way CC does;
        // the flattener will compute the default `short_id · cwd` detail.
        detail: None,
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

/// Resolve the on-disk JSONL path for a local Claude Code session by its id.
/// CC names each session file `<session-id>.jsonl` under
/// `~/.claude/projects/<encoded-cwd>/`, so a glob on the id avoids reproducing
/// CC's cwd-encoding. Returns the first match (ids are UUIDs → unique).
pub fn local_cc_session_jsonl_path(session_id: &str) -> Option<PathBuf> {
    local_cc_session_jsonl_path_in(&local_cc_projects_dir()?, session_id)
}

fn local_cc_projects_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join("projects"))
}

/// `local_cc_session_jsonl_path` against an explicit projects dir — the seam the
/// cwd regression test drives, so it need not mutate the process `$HOME`.
pub fn local_cc_session_jsonl_path_in(projects_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let file_name = format!("{session_id}.jsonl");
    for project_entry in fs::read_dir(projects_dir).ok()?.flatten() {
        let candidate = project_entry.path().join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The CWD a local Claude Code session must be resumed in, read from its own
/// transcript.
///
/// CC derives its project directory FROM THE PROCESS CWD
/// (`~/.claude/projects/<encoded-cwd>/<id>.jsonl`), so `claude --resume <id>` run
/// from the wrong directory searches the wrong project dir and refuses with
/// "No conversation found with session ID <id>" — even though the transcript
/// exists. The transcript is therefore the SINGLE SOURCE OF TRUTH for this cwd:
/// a session row's own cwd field must never be trusted for a resume, because a
/// relaunch path that rebuilds the row can default it to `$HOME`.
///
/// Live-evidenced on jojo 2026-07-11: `20e56a8b-…` was BORN in
/// `/home/pi/gh/yggterm` (spawn cwd correct, transcript written under
/// `-home-pi-gh-yggterm`), but every relaunch from the start page spawned in
/// `/home/pi`, so CC looked in `-home-pi`, found nothing, and refused. The
/// session was never lost — yggterm was knocking on the wrong door.
///
/// Returns `None` when the session has no transcript at all (quit before its
/// first turn), which is the caller's signal that there is nothing to resume.
pub fn local_cc_session_cwd(session_id: &str) -> Option<String> {
    local_cc_session_cwd_in(&local_cc_projects_dir()?, session_id)
}

/// `local_cc_session_cwd` against an explicit projects dir (test seam).
pub fn local_cc_session_cwd_in(projects_dir: &Path, session_id: &str) -> Option<String> {
    let path = local_cc_session_jsonl_path_in(projects_dir, session_id)?;
    let (_id, cwd) = read_cc_session_identity_fields(&path).ok().flatten()?;
    let cwd = cwd.trim();
    (!cwd.is_empty()).then(|| cwd.to_string())
}

/// Append a Claude Code user-rename to a session's JSONL — byte-compatible with
/// what CC itself writes for `/rename` (a `custom-title` record, which CC
/// displays over its auto `ai-title`, plus the parallel `agent-name` record).
/// This makes a yggterm rename indistinguishable from a CC rename and keeps
/// CC's JSONL the single source of truth. See memory
/// finding-cc-title-storage-custom-title.
pub fn append_cc_session_custom_title(
    jsonl_path: &Path,
    session_id: &str,
    title: &str,
) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        anyhow::bail!("refusing to write an empty Claude Code title");
    }
    let custom_title = serde_json::json!({
        "type": "custom-title",
        "customTitle": title,
        "sessionId": session_id,
    });
    let agent_name = serde_json::json!({
        "type": "agent-name",
        "agentName": title,
        "sessionId": session_id,
    });
    let mut payload = String::new();
    payload.push_str(&serde_json::to_string(&custom_title)?);
    payload.push('\n');
    payload.push_str(&serde_json::to_string(&agent_name)?);
    payload.push('\n');
    let mut file = fs::OpenOptions::new()
        .create(false)
        .append(true)
        .open(jsonl_path)
        .with_context(|| format!("failed to open cc session for rename {}", jsonl_path.display()))?;
    file.write_all(payload.as_bytes())
        .with_context(|| format!("failed to append cc rename to {}", jsonl_path.display()))?;
    Ok(())
}

pub fn read_cc_session_identity_fields(path: &Path) -> Result<Option<(String, String)>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read cc session {}", path.display()))?;
    let reader = BufReader::new(file);
    let fallback_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| String::from("unknown-session"));
    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    for line in reader.lines() {
        let line = line.with_context(|| format!("failed to read line from {}", path.display()))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if session_id.is_none() {
            session_id = value
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(ToOwned::to_owned);
        }
        if cwd.is_none() && value.get("type").and_then(Value::as_str) == Some("user") {
            cwd = value
                .get("cwd")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(ToOwned::to_owned);
        }
        if session_id.is_some() && cwd.is_some() {
            break;
        }
    }
    let cwd = match cwd {
        Some(c) => normalize_codex_cwd(c),
        None => return Ok(None),
    };
    Ok(Some((session_id.unwrap_or(fallback_id), cwd)))
}

pub fn read_cc_session_title(path: &Path) -> Result<Option<String>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read cc session {}", path.display()))?;
    let reader = BufReader::new(file);
    // Claude Code title precedence (verified against CC 2.1.x JSONL, see memory
    // finding-cc-title-storage-custom-title): a user rename (`/rename` or the
    // resume-picker Ctrl+R) appends a `custom-title` record, which CC itself
    // displays OVER its auto-generated `ai-title`. Both are append-only and
    // latest-wins. So scan the whole file and prefer the latest `custom-title`,
    // then the latest `ai-title`, then the first human prompt. (We can't early-
    // return on `ai-title` — a later `custom-title` must win, and the caller
    // reads the whole file for context immediately after anyway.)
    let mut custom_title: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut first_human_text: Option<String> = None;
    for line in reader.lines() {
        let line = line.with_context(|| format!("failed to read line from {}", path.display()))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("custom-title") {
            if let Some(title) = value.get("customTitle").and_then(Value::as_str) {
                let title = title.trim();
                if !title.is_empty() {
                    custom_title = Some(title.to_string());
                }
            }
        }
        if value.get("type").and_then(Value::as_str) == Some("ai-title") {
            if let Some(title) = value.get("aiTitle").and_then(Value::as_str) {
                let title = title.trim();
                if !title.is_empty() {
                    ai_title = Some(title.to_string());
                }
            }
        }
        // Collect first real human text as fallback (skipping protocol XML).
        if first_human_text.is_none()
            && value.get("type").and_then(Value::as_str) == Some("user")
            && value
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                == Some("user")
        {
            let content = value.get("message").and_then(|m| m.get("content"));
            let text = match content {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(parts)) => parts
                    .iter()
                    .filter_map(|part| {
                        if part.get("type").and_then(Value::as_str) == Some("text") {
                            part.get("text")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => continue,
            };
            // Strip protocol XML tags; use remainder if non-empty.
            let text = if let Some(pos) = text.find("</local-command-caveat>") {
                text[pos + "</local-command-caveat>".len()..].to_string()
            } else {
                text
            };
            let trimmed = text.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('<') {
                let title: String = trimmed.chars().take(80).collect();
                let title = if trimmed.chars().count() > 80 {
                    format!("{title}…")
                } else {
                    title
                };
                first_human_text = Some(title);
            }
        }
    }
    Ok(custom_title.or(ai_title).or(first_human_text))
}

pub fn read_cc_session_context(path: &Path) -> Result<String> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read cc session {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut parts: Vec<String> = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        if value
            .get("message")
            .and_then(|m| m.get("role"))
            .and_then(Value::as_str)
            != Some("user")
        {
            continue;
        }
        let content = value.get("message").and_then(|m| m.get("content"));
        let text = match content {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|p| {
                    if p.get("type").and_then(Value::as_str) == Some("text") {
                        p.get("text").and_then(Value::as_str).map(ToOwned::to_owned)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            _ => continue,
        };
        let text = if let Some(pos) = text.find("</local-command-caveat>") {
            text[pos + "</local-command-caveat>".len()..].to_string()
        } else {
            text
        };
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.starts_with('<') {
            continue;
        }
        let snippet: String = trimmed.chars().take(120).collect();
        parts.push(snippet);
        if parts.len() >= 3 {
            break;
        }
    }
    Ok(parts.join(" · "))
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

fn insert_codex_browser_project(
    root: &mut CodexBrowserTreeNode,
    project: &LocalAgentProjectBucket,
) {
    let segments = browser_tree_segments(&project.cwd);
    insert_codex_browser_path(root, &segments, project.clone());
}

fn insert_codex_browser_path(
    node: &mut CodexBrowserTreeNode,
    segments: &[String],
    project: LocalAgentProjectBucket,
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
            path: PathBuf::from(document.virtual_path.clone()),
            session_id: Some(document.id.clone()),
            cwd: Some(document.virtual_path.clone()),
            ..Default::default()
        };
    }

    let mut children = Vec::new();

    let mut nested_children = node
        .children
        .values()
        .map(codex_browser_tree_to_session_node)
        .collect::<Vec<_>>();
    nested_children.sort_by(cmp_browser_child_node);
    children.extend(nested_children);

    if let Some(project) = &node.project {
        // Per [[spec-cwd-tree-agent-cli-unified]]: every agent CLI session
        // becomes a leaf here, carrying its authoritative SessionKind and
        // optional detail. Display dispatch (icon/glyph/label/style) reads
        // from `session_kind` — never from path prefix.
        children.extend(project.sessions.iter().map(|session| SessionNode {
            kind: SessionNodeKind::CodexSession,
            name: short_session_id(&session.session_id),
            title: session.title.clone(),
            path: session.file_path.clone(),
            session_id: Some(session.session_id.clone()),
            cwd: Some(project.cwd.clone()),
            session_kind: Some(session.kind),
            detail: session.detail.clone(),
            ..Default::default()
        }));
    }

    SessionNode {
        kind: SessionNodeKind::Group,
        name: node.name.clone(),
        title: node.explicit_title.clone(),
        group_kind: node.group_kind,
        path: PathBuf::from(node.full_path.clone()),
        children,
        cwd: node.project.as_ref().map(|project| project.cwd.clone()),
        ..Default::default()
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

    /// A local CC session's cwd comes from its TRANSCRIPT, not from the row.
    ///
    /// Regression lock for the jojo 2026-07-11 "No conversation found with session ID"
    /// bug: `20e56a8b-…` was born in `/home/pi/gh/yggterm` but every relaunch spawned
    /// `claude --resume` in `/home/pi`, so CC resolved its project dir to `-home-pi`,
    /// looked for the transcript there, and refused a session that existed. CC keys its
    /// project dir on the process cwd, so resuming in the wrong dir is indistinguishable
    /// from the session not existing. The transcript records the true cwd — read it.
    #[test]
    fn local_cc_session_cwd_comes_from_the_transcript_not_the_row() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-cc-cwd-{}-{}",
            std::process::id(),
            line!()
        ));
        let projects = root.join("projects");
        // CC encodes the cwd into the project dir name; the transcript states it exactly.
        let project = projects.join("-home-pi-gh-yggterm");
        fs::create_dir_all(&project).expect("create project dir");
        let session_id = "20e56a8b-90f7-4c44-baee-b230e11f65f1";
        let transcript = project.join(format!("{session_id}.jsonl"));
        fs::write(
            &transcript,
            format!(
                "{}\n",
                serde_json::json!({
                    "type": "user",
                    "sessionId": session_id,
                    "cwd": "/home/pi/gh/yggterm",
                })
            ),
        )
        .expect("write transcript");

        assert_eq!(
            local_cc_session_cwd_in(&projects, session_id).as_deref(),
            Some("/home/pi/gh/yggterm"),
            "the resume cwd must be read from the transcript, never defaulted to $HOME"
        );
        // A session with no transcript has nothing to resume — the caller's signal to
        // re-birth it rather than hand the user CC's refusal.
        assert_eq!(
            local_cc_session_cwd_in(&projects, "ffffffff-0000-0000-0000-000000000000"),
            None
        );

        fs::remove_dir_all(&root).ok();
    }

    // The settings file has a hand-written writer AND a hand-written parser, so a
    // field added to `AppSettings` alone is silently NEVER SAVED. That is exactly
    // what happened to every `web_surface_*` pref: the global zoom shipped as
    // "persisted" and was not, and vertical tabs came back off after every
    // restart. Structural, not per-field: it compares the writer's keys against
    // the struct's own, so the next field to go missing fails the build.
    #[test]
    fn every_settings_field_is_written_to_the_file() {
        use std::collections::BTreeSet;
        let settings = AppSettings::default();
        let derived = serde_json::to_value(&settings).expect("derive-serialize settings");
        let written = serialize_settings_value(&settings);

        let mut expected: BTreeSet<String> = derived
            .as_object()
            .expect("settings serialize to an object")
            .keys()
            .cloned()
            .collect();
        // The file's two legacy names: `theme_mode` holds the UiTheme and `theme`
        // holds the yggui theme object (the parser accepts both spellings).
        expected.remove("theme");
        expected.remove("yggui_theme");
        expected.insert("theme_mode".to_string());
        expected.insert("theme".to_string());

        let written_keys: BTreeSet<String> = written
            .as_object()
            .expect("writer produces an object")
            .keys()
            .cloned()
            .collect();
        let missing: Vec<&String> = expected.difference(&written_keys).collect();
        assert!(
            missing.is_empty(),
            "these AppSettings fields would never survive a restart: {missing:?}"
        );
    }

    // And the values make it back, not just the keys.
    #[test]
    fn web_surface_prefs_survive_a_settings_round_trip() {
        let mut settings = AppSettings::default();
        settings.web_surface_zoom_percent = 125.0;
        settings.web_surface_vertical_tabs = true;
        settings.web_surface_restore_tabs = true;
        let parsed =
            parse_settings_value(&serialize_settings_value(&settings)).expect("round trip");
        assert_eq!(parsed.web_surface_zoom_percent, 125.0);
        assert!(parsed.web_surface_vertical_tabs);
        assert!(parsed.web_surface_restore_tabs);
    }

    #[test]
    fn split_group_survives_settings_round_trip() {
        // A built split-view workspace must reopen as the intentional artifact
        // the user shaped ([[campaign-split-view-groups]] persistence).
        let mut prior = BTreeMap::new();
        prior.insert("local://a".to_string(), false);
        prior.insert("local://b".to_string(), true);
        let mut settings = AppSettings::default();
        settings.split_groups = vec![SplitGroup {
            group_id: "g1".to_string(),
            axis: SplitAxis::Stacked,
            ratio: 0.42,
            members: vec![
                SplitMember::terminal("local://a"),
                SplitMember::terminal("local://b"),
            ],
            active_pane: 1,
            prior_keep_alive: prior.clone(),
        }];
        let value = serialize_settings_value(&settings);
        // Terminal-view members keep the pre-3.x bare-string wire format, so a
        // built workspace's settings file is byte-identical until a pinned
        // view exists (and an older build can still read it).
        assert_eq!(
            value["split_groups"][0]["members"],
            serde_json::json!(["local://a", "local://b"]),
        );
        let parsed = parse_settings_value(&value).expect("parse split_groups");
        assert_eq!(parsed.split_groups.len(), 1);
        let group = &parsed.split_groups[0];
        assert_eq!(group.group_id, "g1");
        assert_eq!(group.axis, SplitAxis::Stacked);
        assert!((group.ratio - 0.42).abs() < 1e-4);
        assert_eq!(
            group.members,
            vec![
                SplitMember::terminal("local://a"),
                SplitMember::terminal("local://b"),
            ],
        );
        assert_eq!(group.active_pane, 1);
        assert_eq!(group.prior_keep_alive, prior);
    }

    #[test]
    fn split_member_views_round_trip_and_accept_the_legacy_string_form() {
        // The explicit (session, view) member ([[campaign-libyggterm]] Phase
        // 3): a pinned view serializes as a struct and round-trips; the
        // legacy bare-string form still deserializes as a Terminal view.
        let members = vec![
            SplitMember::terminal("local://a"),
            SplitMember {
                session: "local://a".to_string(),
                view: SplitMemberView::Web { tab: 4 },
            },
            SplitMember {
                session: "local://d".to_string(),
                view: SplitMemberView::Document {
                    context: "diff".to_string(),
                },
            },
        ];
        let wire = serde_json::to_value(&members).expect("serialize members");
        assert_eq!(
            wire,
            serde_json::json!([
                "local://a",
                {"session": "local://a", "view": {"web": {"tab": 4}}},
                {"session": "local://d", "view": {"document": {"context": "diff"}}},
            ]),
        );
        let parsed: Vec<SplitMember> = serde_json::from_value(wire).expect("parse members");
        assert_eq!(parsed, members);
        let legacy: Vec<SplitMember> =
            serde_json::from_value(serde_json::json!(["local://old"])).expect("parse legacy");
        assert_eq!(legacy, vec![SplitMember::terminal("local://old")]);
    }

    #[test]
    fn split_group_normalized_clamps_ratio_and_active_pane() {
        // Defensive against a persisted file from a divergent build or a removed
        // member — the SSOT stays valid rather than panicking a render.
        let group = SplitGroup {
            group_id: "g".to_string(),
            axis: SplitAxis::SideBySide,
            ratio: 5.0,
            members: vec![SplitMember::terminal("a"), SplitMember::terminal("b")],
            active_pane: 9,
            prior_keep_alive: BTreeMap::new(),
        }
        .normalized();
        assert!(group.ratio <= 0.85 && group.ratio >= 0.15);
        assert_eq!(group.active_pane, 1);

        let empty = SplitGroup {
            group_id: "e".to_string(),
            axis: SplitAxis::SideBySide,
            ratio: f32::NAN,
            members: vec![],
            active_pane: 3,
            prior_keep_alive: BTreeMap::new(),
        }
        .normalized();
        assert_eq!(empty.active_pane, 0);
        assert!(empty.ratio.is_finite());
    }

    #[test]
    fn input_draft_sets_on_typed_text_and_clears_on_submit() {
        // Typing printable text raises the draft.
        assert!(input_line_has_unsent_draft_after(false, b"git status"));
        // Submitting (Enter) clears it.
        assert!(!input_line_has_unsent_draft_after(true, b"\r"));
        assert!(!input_line_has_unsent_draft_after(false, b"ls -la\n"));
        // Ctrl-C / Ctrl-U clear a pending line.
        assert!(!input_line_has_unsent_draft_after(true, b"\x03"));
        assert!(!input_line_has_unsent_draft_after(true, b"\x15"));
    }

    #[test]
    fn input_draft_is_sticky_across_calls() {
        let mut draft = false;
        draft = input_line_has_unsent_draft_after(draft, b"hel");
        assert!(draft, "partial typing is a draft");
        // A later no-op input (e.g. a redraw-triggering resize ack is not input,
        // but even an arrow key) must keep the draft sticky.
        draft = input_line_has_unsent_draft_after(draft, b"\x1b[A");
        assert!(draft, "arrow key must not clear the draft");
        draft = input_line_has_unsent_draft_after(draft, b"lo\r");
        assert!(!draft, "submit clears it");
    }

    #[test]
    fn input_draft_ignores_escape_sequences() {
        // Arrow keys, F-keys, and bare cursor moves carry no typed text.
        assert!(!input_line_has_unsent_draft_after(false, b"\x1b[A\x1b[B\x1b[C\x1b[D"));
        assert!(!input_line_has_unsent_draft_after(false, b"\x1bOP\x1bOQ"));
        // Alt-key (ESC + letter) is not a draft.
        assert!(!input_line_has_unsent_draft_after(false, b"\x1bb"));
    }

    #[test]
    fn input_draft_ignores_mouse_reports() {
        // SGR mouse report: ESC [ < 0 ; 34 ; 12 M — the digits must not look
        // like typed text, or a moused/scrolled session would never migrate.
        assert!(!input_line_has_unsent_draft_after(false, b"\x1b[<0;34;12M"));
        assert!(!input_line_has_unsent_draft_after(false, b"\x1b[<64;10;5m"));
    }

    #[test]
    fn input_draft_counts_pasted_content_between_brackets() {
        // Bracketed paste: markers skipped, content counts as a draft.
        assert!(input_line_has_unsent_draft_after(
            false,
            b"\x1b[200~echo hi\x1b[201~"
        ));
        // …but a paste that the user then submits is no longer a draft.
        assert!(!input_line_has_unsent_draft_after(
            false,
            b"\x1b[200~echo hi\x1b[201~\r"
        ));
    }

    #[test]
    fn input_draft_ignores_lone_whitespace() {
        assert!(!input_line_has_unsent_draft_after(false, b"   "));
        assert!(!input_line_has_unsent_draft_after(false, b"\t"));
    }

    #[test]
    fn agent_working_detects_esc_to_interrupt_for_both_clis() {
        // Codex
        assert!(screen_text_shows_agent_working(
            "some output\nWorking (12s • esc to interrupt)"
        ));
        // Claude Code
        assert!(screen_text_shows_agent_working(
            "✻ Pondering… (5s · esc to interrupt)"
        ));
        // Codex background task without an esc-to-interrupt line
        assert!(screen_text_shows_agent_working(
            "Working (background terminal running)"
        ));
    }

    #[test]
    fn agent_working_is_false_when_idle_or_completed() {
        // Completion summary must not read as active work.
        assert!(!screen_text_shows_agent_working(
            "• Worked for 42s\n› "
        ));
        // Plain idle shell prompt.
        assert!(!screen_text_shows_agent_working("user@host:~$ "));
        assert!(!screen_text_shows_agent_working(""));
        // "esc to interrupt" buried far above the visible tail is ignored.
        let mut buf = String::from("Working (1s • esc to interrupt)\n");
        for _ in 0..20 {
            buf.push_str("idle line\n");
        }
        assert!(!screen_text_shows_agent_working(&buf));
    }

    #[test]
    fn cc_title_prefers_latest_custom_title_over_ai_title() {
        let dir = std::env::temp_dir().join(format!("ygg-cc-title-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("sess.jsonl");
        // AI title (re-emitted), then a user rename (custom-title), then a
        // re-rename — latest custom-title must win.
        fs::write(
            &path,
            concat!(
                r#"{"type":"ai-title","aiTitle":"Auto Generated","sessionId":"s1"}"#,
                "\n",
                r#"{"type":"user","message":{"role":"user","content":"hello there"}}"#,
                "\n",
                r#"{"type":"ai-title","aiTitle":"Auto Generated","sessionId":"s1"}"#,
                "\n",
                r#"{"type":"custom-title","customTitle":"User Renamed","sessionId":"s1"}"#,
                "\n",
                r#"{"type":"custom-title","customTitle":"User Renamed Again","sessionId":"s1"}"#,
                "\n",
            ),
        )
        .unwrap();
        assert_eq!(
            read_cc_session_title(&path).unwrap().as_deref(),
            Some("User Renamed Again")
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn append_cc_custom_title_round_trips_through_reader() {
        let dir = std::env::temp_dir().join(format!("ygg-cc-rt-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("rt.jsonl");
        // Start with only an AI title (what yggterm would show before rename).
        fs::write(
            &path,
            "{\"type\":\"ai-title\",\"aiTitle\":\"Before\",\"sessionId\":\"abc\"}\n",
        )
        .unwrap();
        assert_eq!(read_cc_session_title(&path).unwrap().as_deref(), Some("Before"));
        // A yggterm rename writes a custom-title; the reader must now return it.
        append_cc_session_custom_title(&path, "abc", "Renamed By Yggterm").unwrap();
        assert_eq!(
            read_cc_session_title(&path).unwrap().as_deref(),
            Some("Renamed By Yggterm")
        );
        // And the file must carry both the custom-title and the parallel
        // agent-name record that CC's own /rename writes.
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains(r#""type":"custom-title""#));
        assert!(body.contains(r#""type":"agent-name""#));
        assert!(body.contains("Renamed By Yggterm"));
        let _ = fs::remove_file(&path);
    }

    /// Regression test for the class of bug where a new `AppSettings` field
    /// is declared but its read/write through `serialize_settings_value` /
    /// `parse_settings_value` is forgotten. Symptom: the field appears in
    /// the UI, the user sets it, but it silently drops on every restart.
    /// Was caught live for `claude_code_extra_args` (2026-05-26); this test
    /// guards against the same fault on every other field by mutating each
    /// to a distinct non-default value and asserting full round-trip.
    #[test]
    fn app_settings_round_trip_through_serialize_then_parse_preserves_all_fields() {
        let mut original = AppSettings::default();
        // Mutate every field to a distinct, non-default value. If a new
        // field is added to AppSettings and not handled in serialize/parse,
        // this test will surface the divergence as soon as the field is
        // mutated here.
        original.theme = UiTheme::ZedDark;
        original.show_tree = false;
        original.show_settings = true;
        original.auto_hide_titlebar = true;
        original.window_maximized = true;
        original.tree_width = 411.5;
        original.rendered_font_size = 11.5;
        original.terminal_font_size = 17.0;
        original.terminal_light_theme_name = "Solarized Light".to_string();
        original.terminal_dark_theme_name = "Monokai".to_string();
        original.ui_font_size = 15.5;
        original.prefer_ghostty_backend = false;
        original.litellm_endpoint = "https://litellm.example.test".to_string();
        original.litellm_api_key = "sk-test-1234".to_string();
        original.interface_llm_model = "gpt-test-5".to_string();
        original.codex_extra_args = "-s danger-full-access".to_string();
        original.claude_code_extra_args = "--dangerously-skip-permissions".to_string();
        original.in_app_notifications = false;
        original.system_notifications = true;
        original.notification_sound = true;
        original.terminal_telemetry_enabled = false;
        original.selected_browser_path = Some("__remote_machine__/dev".to_string());
        original.expanded_browser_paths =
            vec!["__remote_machine__/dev".to_string(), "/home/pi".to_string()];

        let json = serialize_settings_value(&original);
        let round_tripped = parse_settings_value(&json).expect("parse settings");

        assert_eq!(round_tripped, original);
    }

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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
                        ..Default::default()
                    }],
                    session_id: None,
                    cwd: None,
                    ..Default::default()
                }],
                session_id: None,
                cwd: None,
                ..Default::default()
            }],
            session_id: None,
            cwd: None,
            ..Default::default()
        };

        let mut browser = SessionBrowserState::new(root);
        browser.restore_ui_state(&["/workspace/machine-a/nested".to_string()], None);

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
    fn workspace_documents_sort_before_raw_codex_sessions_inside_project_folder() {
        let mut root = CodexBrowserTreeNode {
            name: String::from("local [ok]"),
            full_path: String::from("local"),
            explicit_title: Some(String::from("local")),
            document: None,
            group_kind: Some(WorkspaceGroupKind::Folder),
            project: None,
            children: BTreeMap::new(),
        };
        insert_workspace_document(
            &mut root,
            &WorkspaceDocumentSummary {
                id: "recipe-id".to_string(),
                virtual_path: "/home/pi/0000-local-shell".to_string(),
                title: "local-shell".to_string(),
                kind: WorkspaceDocumentKind::TerminalRecipe,
                updated_at: "2026-04-24T00:00:00Z".to_string(),
            },
        );
        insert_codex_browser_project(
            &mut root,
            &LocalAgentProjectBucket {
                cwd: "/home/pi".to_string(),
                sessions: vec![LocalAgentSessionSummary {
                    kind: SessionKind::Codex,
                    file_path: PathBuf::from("/home/pi/.codex/sessions/example.jsonl"),
                    session_id: "stored-session-id".to_string(),
                    cwd: "/home/pi".to_string(),
                    title: Some("Stored Codex".to_string()),
                    detail: None,
                    modified_epoch_ms: 0,
                }],
            },
        );
        compress_codex_browser_tree(&mut root, false);

        let mut browser = SessionBrowserState::new(codex_browser_tree_to_session_node(&root));
        browser.ensure_visible_path("/home/pi/0000-local-shell");
        let rows = browser.rows();
        let recipe_ix = rows
            .iter()
            .position(|row| row.full_path == "/home/pi/0000-local-shell")
            .expect("recipe row should be visible");
        let session_ix = rows
            .iter()
            .position(|row| row.full_path == "/home/pi/.codex/sessions/example.jsonl")
            .expect("stored session row should be visible");

        assert_eq!(rows[recipe_ix].kind, BrowserRowKind::Document);
        assert_eq!(
            rows[recipe_ix].document_kind,
            Some(WorkspaceDocumentKind::TerminalRecipe)
        );
        assert!(
            recipe_ix < session_ix,
            "workspace recipe should stay at the top of the project folder"
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

    #[test]
    fn settings_parse_window_maximized() {
        let parsed = parse_settings_value(&serde_json::json!({
            "window_maximized": true
        }))
        .expect("settings should parse");
        assert!(parsed.window_maximized);
    }

    #[test]
    fn settings_serialize_window_maximized() {
        let mut settings = AppSettings::default();
        settings.window_maximized = true;
        assert_eq!(
            serialize_settings_value(&settings).get("window_maximized"),
            Some(&serde_json::json!(true))
        );
    }

    #[test]
    fn settings_default_terminal_telemetry_enabled() {
        assert!(AppSettings::default().terminal_telemetry_enabled);
    }

    #[test]
    fn settings_parse_terminal_telemetry_toggle() {
        let parsed = parse_settings_value(&serde_json::json!({
            "terminal_telemetry_enabled": false
        }))
        .expect("settings should parse");
        assert!(!parsed.terminal_telemetry_enabled);
    }

    #[test]
    fn settings_serialize_terminal_telemetry_toggle() {
        let mut settings = AppSettings::default();
        settings.terminal_telemetry_enabled = false;
        assert_eq!(
            serialize_settings_value(&settings).get("terminal_telemetry_enabled"),
            Some(&serde_json::json!(false))
        );
    }

    #[test]
    fn settings_parse_codex_extra_args() {
        let parsed = parse_settings_value(&serde_json::json!({
            "codex_extra_args": "-s danger-full-access --profile \"field test\""
        }))
        .expect("settings should parse");
        assert_eq!(
            parsed.codex_extra_args,
            "-s danger-full-access --profile \"field test\""
        );
    }

    #[test]
    fn settings_serialize_codex_extra_args() {
        let mut settings = AppSettings::default();
        settings.codex_extra_args = "-s danger-full-access".to_string();
        assert_eq!(
            serialize_settings_value(&settings).get("codex_extra_args"),
            Some(&serde_json::json!("-s danger-full-access"))
        );
    }
}
