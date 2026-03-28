use crate::{SessionNode, SessionNodeKind, WorkspaceDocumentKind, WorkspaceGroupKind};
use dirs::home_dir;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRowKind {
    Group,
    Separator,
    Session,
    Document,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRow {
    pub kind: BrowserRowKind,
    pub full_path: String,
    pub label: String,
    pub detail_label: String,
    pub document_kind: Option<WorkspaceDocumentKind>,
    pub group_kind: Option<WorkspaceGroupKind>,
    pub session_title: Option<String>,
    pub depth: usize,
    pub host_label: String,
    pub descendant_sessions: usize,
    pub expanded: bool,
    pub session_id: Option<String>,
    pub session_cwd: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionBrowserState {
    root: SessionNode,
    filter_query: String,
    expanded_paths: HashSet<String>,
    rows: Vec<BrowserRow>,
    selected_path: Option<String>,
    metrics: BrowserMetrics,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BrowserMetrics {
    pub row_count: usize,
    pub rebuild_count: usize,
    pub last_rebuild_ms: f32,
}

impl SessionBrowserState {
    pub fn new(root: SessionNode) -> Self {
        let expanded_paths = default_level_one_expanded_paths(&root);
        let mut this = Self {
            root,
            filter_query: String::new(),
            expanded_paths,
            rows: Vec::new(),
            selected_path: None,
            metrics: BrowserMetrics::default(),
        };
        this.rebuild_rows();
        this.selected_path = first_session_path(&this.root);
        this
    }

    pub fn rows(&self) -> &[BrowserRow] {
        &self.rows
    }

    pub fn search_rows(&self) -> Vec<BrowserRow> {
        let filter = self.filter_query.to_ascii_lowercase();
        let short_ids = unique_session_short_ids(&self.root);
        let mut expanded_paths = HashSet::new();
        collect_group_paths(&self.root, &mut expanded_paths);
        let mut rows = Vec::new();
        flatten_rows(
            &self.root,
            0,
            &filter,
            &expanded_paths,
            &short_ids,
            &mut rows,
            true,
        );
        rows
    }

    pub fn root(&self) -> &SessionNode {
        &self.root
    }

    pub fn filter_query(&self) -> &str {
        &self.filter_query
    }

    pub fn set_filter_query(&mut self, query: impl Into<String>) {
        self.filter_query = query.into();
        self.rebuild_rows();
        self.ensure_selection();
    }

    pub fn selected_path(&self) -> Option<&str> {
        self.selected_path.as_deref()
    }

    pub fn expanded_paths(&self) -> Vec<String> {
        let mut paths = self.expanded_paths.iter().cloned().collect::<Vec<_>>();
        paths.sort();
        paths
    }

    pub fn expanded_path_set(&self) -> HashSet<String> {
        self.expanded_paths.clone()
    }

    pub fn selected_row(&self) -> Option<&BrowserRow> {
        self.selected_path
            .as_deref()
            .and_then(|path| self.rows.iter().find(|row| row.full_path == path))
            .or_else(|| self.rows.first())
    }

    pub fn metrics(&self) -> BrowserMetrics {
        self.metrics
    }

    pub fn select_path(&mut self, path: impl Into<String>) {
        let path = path.into();
        if self.rows.iter().any(|row| row.full_path == path) {
            self.selected_path = Some(path);
        }
    }

    pub fn restore_ui_state(&mut self, expanded_paths: &[String], selected_path: Option<&str>) {
        self.expanded_paths = default_level_one_expanded_paths(&self.root);
        self.expanded_paths.extend(
            expanded_paths
                .iter()
                .filter(|path| is_level_one_group_path(&self.root, path))
                .cloned(),
        );
        self.rebuild_rows();
        if let Some(path) = selected_path {
            self.select_path(path.to_string());
        }
        self.ensure_selection();
    }

    pub fn toggle_group(&mut self, path: &str) {
        if !self.expanded_paths.remove(path) {
            self.expanded_paths.insert(path.to_string());
        }
        self.rebuild_rows();
        self.ensure_selection();
    }

    pub fn toggle_virtual_group(&mut self, path: &str) {
        if !self.expanded_paths.remove(path) {
            self.expanded_paths.insert(path.to_string());
        }
    }

    pub fn ensure_expanded_paths<I>(&mut self, paths: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut changed = false;
        for path in paths {
            changed |= self.expanded_paths.insert(path);
        }
        if changed {
            self.rebuild_rows();
            self.ensure_selection();
        }
    }

    pub fn ensure_visible_path(&mut self, path: &str) {
        let mut changed = false;
        for ancestor in Path::new(path).ancestors().skip(1) {
            changed |= self.expanded_paths.insert(ancestor.display().to_string());
        }
        if changed {
            self.rebuild_rows();
            self.ensure_selection();
        }
        self.select_path(path.to_string());
    }

    pub fn total_sessions(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row.kind, BrowserRowKind::Session | BrowserRowKind::Document))
            .count()
    }

    pub fn total_session_count(&self) -> usize {
        count_leaf_sessions(&self.root)
    }

    pub fn selected_index(&self) -> Option<usize> {
        let path = self.selected_path.as_deref()?;
        self.rows.iter().position(|row| row.full_path == path)
    }

    pub fn selected_session_index(&self) -> Option<usize> {
        self.selected_path.as_deref().and_then(|path| {
            self.rows.iter().position(|row| {
                matches!(row.kind, BrowserRowKind::Session | BrowserRowKind::Document)
                    && row.full_path == path
            })
        })
    }

    pub fn select_next_session(&mut self) -> Option<usize> {
        self.select_session_relative(1)
    }

    pub fn select_previous_session(&mut self) -> Option<usize> {
        self.select_session_relative(-1)
    }

    fn ensure_selection(&mut self) {
        let selected_missing = self
            .selected_path
            .as_deref()
            .is_none_or(|path| !self.rows.iter().any(|row| row.full_path == path));
        if selected_missing {
            self.selected_path = self
                .rows
                .iter()
                .find(|row| matches!(row.kind, BrowserRowKind::Session | BrowserRowKind::Document))
                .map(|row| row.full_path.clone());
        }
    }

    fn rebuild_rows(&mut self) {
        let started_at = Instant::now();
        self.rows.clear();
        let filter = self.filter_query.to_ascii_lowercase();
        let short_ids = unique_session_short_ids(&self.root);
        flatten_rows(
            &self.root,
            0,
            &filter,
            &self.expanded_paths,
            &short_ids,
            &mut self.rows,
            true,
        );
        self.metrics.row_count = self.rows.len();
        self.metrics.rebuild_count += 1;
        self.metrics.last_rebuild_ms = started_at.elapsed().as_secs_f32() * 1000.0;
    }

    fn select_session_relative(&mut self, delta: isize) -> Option<usize> {
        let session_indexes = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(ix, row)| {
                matches!(row.kind, BrowserRowKind::Session | BrowserRowKind::Document).then_some(ix)
            })
            .collect::<Vec<_>>();
        if session_indexes.is_empty() {
            return None;
        }

        let current_position = self
            .selected_session_index()
            .and_then(|selected_ix| session_indexes.iter().position(|ix| *ix == selected_ix))
            .unwrap_or(0);

        let len = session_indexes.len() as isize;
        let next_position = (current_position as isize + delta).rem_euclid(len) as usize;
        let next_ix = session_indexes[next_position];
        self.selected_path = Some(self.rows[next_ix].full_path.clone());
        Some(next_ix)
    }
}

fn default_level_one_expanded_paths(root: &SessionNode) -> HashSet<String> {
    let mut expanded_paths = HashSet::new();
    if !root.children.is_empty() {
        expanded_paths.insert(root.path.display().to_string());
        for child in &root.children {
            if child.kind == SessionNodeKind::Group {
                expanded_paths.insert(child.path.display().to_string());
            }
        }
    }
    expanded_paths
}

fn is_level_one_group_path(root: &SessionNode, path: &str) -> bool {
    root.children.iter().any(|child| {
        child.kind == SessionNodeKind::Group && child.path.display().to_string() == path
    })
}

fn first_session_path(node: &SessionNode) -> Option<String> {
    if node.kind != SessionNodeKind::Group && node.session_id.is_some() {
        return Some(node.path.display().to_string());
    }

    for child in &node.children {
        if let Some(path) = first_session_path(child) {
            return Some(path);
        }
    }

    None
}

fn collect_group_paths(node: &SessionNode, expanded_paths: &mut HashSet<String>) {
    if node.kind == SessionNodeKind::Group {
        expanded_paths.insert(node.path.display().to_string());
    }
    for child in &node.children {
        collect_group_paths(child, expanded_paths);
    }
}

fn flatten_rows(
    node: &SessionNode,
    depth: usize,
    filter: &str,
    expanded_paths: &HashSet<String>,
    short_ids: &HashMap<String, String>,
    rows: &mut Vec<BrowserRow>,
    include_root: bool,
) -> bool {
    let is_leaf = node.kind != SessionNodeKind::Group;
    let full_path = node.path.display().to_string();
    if !matches_filter(node, filter) || (is_leaf && node.session_id.is_none()) {
        return false;
    }

    let descendant_sessions = count_leaf_sessions(node);
    if !is_leaf
        && node.group_kind != Some(WorkspaceGroupKind::Separator)
        && descendant_sessions == 0
        && node.title.is_none()
    {
        return false;
    }
    let expanded = is_leaf || !filter.is_empty() || expanded_paths.contains(&full_path);

    if include_root {
        rows.push(BrowserRow {
            kind: if is_leaf {
                match node.kind {
                    SessionNodeKind::CodexSession => BrowserRowKind::Session,
                    SessionNodeKind::Document => BrowserRowKind::Document,
                    SessionNodeKind::Group => BrowserRowKind::Group,
                }
            } else if node.group_kind == Some(WorkspaceGroupKind::Separator) {
                BrowserRowKind::Separator
            } else {
                BrowserRowKind::Group
            },
            label: format_row_label(node, short_ids, &full_path, is_leaf),
            detail_label: detail_label_for_row(node, &full_path, is_leaf),
            session_title: if is_leaf { node.title.clone() } else { None },
            document_kind: node.document_kind,
            group_kind: node.group_kind,
            full_path: full_path.clone(),
            depth,
            host_label: host_label_for_row(node, depth),
            descendant_sessions,
            expanded,
            session_id: node.session_id.clone(),
            session_cwd: node.cwd.clone(),
        });
    }

    if !is_leaf && expanded {
        for child in &node.children {
            flatten_rows(
                child,
                depth + 1,
                filter,
                expanded_paths,
                short_ids,
                rows,
                true,
            );
        }
    }

    true
}

fn format_row_label(
    node: &SessionNode,
    short_ids: &HashMap<String, String>,
    full_path: &str,
    is_session: bool,
) -> String {
    if is_session {
        match node.kind {
            SessionNodeKind::Document => node.title.clone().unwrap_or_else(|| node.name.clone()),
            SessionNodeKind::CodexSession => node.title.clone().unwrap_or_else(|| {
                short_ids
                    .get(full_path)
                    .cloned()
                    .or_else(|| {
                        node.session_id
                            .as_deref()
                            .map(|id| session_id_suffix(id, 7))
                    })
                    .unwrap_or_else(|| node.name.clone())
            }),
            SessionNodeKind::Group => node.title.clone().unwrap_or_else(|| node.name.clone()),
        }
    } else {
        node.title.clone().unwrap_or_else(|| node.name.clone())
    }
}

fn detail_label_for_row(node: &SessionNode, full_path: &str, is_session: bool) -> String {
    if is_session {
        if node.kind == SessionNodeKind::Document {
            return String::new();
        }
        let path_label = node
            .cwd
            .as_deref()
            .map(browser_display_path)
            .unwrap_or_else(|| browser_display_path(full_path));
        if node.title.is_some() {
            let short_id = node
                .session_id
                .as_deref()
                .map(|id| session_id_suffix(id, 7))
                .unwrap_or_default();
            if short_id.is_empty() {
                path_label
            } else {
                format!("{short_id} · {path_label}")
            }
        } else {
            path_label
        }
    } else {
        if full_path.starts_with("live::") {
            "remote runtime".to_string()
        } else {
            String::new()
        }
    }
}

fn matches_filter(node: &SessionNode, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }

    let name = node.name.to_ascii_lowercase();
    let path = node.path.display().to_string().to_ascii_lowercase();
    if name.contains(filter) || path.contains(filter) {
        return true;
    }

    node.children
        .iter()
        .any(|child| matches_filter(child, filter))
}

fn count_leaf_sessions(node: &SessionNode) -> usize {
    if node.children.is_empty() {
        return usize::from(node.session_id.is_some());
    }

    node.children.iter().map(count_leaf_sessions).sum()
}

fn host_label_for_row(node: &SessionNode, depth: usize) -> String {
    if depth == 0 || depth == 1 {
        return String::new();
    }
    if node.path.display().to_string().starts_with("live::") {
        return "live".to_string();
    }
    String::new()
}

fn browser_display_path(path: &str) -> String {
    let normalized = if path == "/" {
        String::from("/")
    } else if path.starts_with('/') {
        format!("/{}", path.trim_start_matches('/'))
    } else {
        path.to_string()
    };

    if normalized == "/" {
        return normalized;
    }

    if let Some(home) = home_dir() {
        let home = home.to_string_lossy().to_string();
        if normalized == home {
            return String::from("~");
        }
        let with_slash = format!("{home}/");
        if let Some(rest) = normalized.strip_prefix(&with_slash) {
            return format!("~/{rest}");
        }
    }

    let path = Path::new(&normalized);
    let mut parts = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if parts.len() > 4 {
        let tail = parts.split_off(parts.len() - 4);
        return format!("…/{}", tail.join("/"));
    }

    normalized
}

fn unique_session_short_ids(root: &SessionNode) -> HashMap<String, String> {
    let mut sessions = Vec::<(String, String)>::new();
    collect_session_ids(root, &mut sessions);

    unique_session_short_ids_for_pairs(&sessions)
}

/// Resolve the shortest visible per-session identifier that is unique within the
/// current session set.
///
/// The UI starts from the compact trailing hash users already recognize, but it
/// must widen until unique. If two sessions still collide all the way through
/// the shared tail, this falls back to the full session id rather than letting
/// two visible rows share the same short hash.
///
/// Keep this rule portable: `codex-session-tui` and any future browser/tree UI
/// should use the same widening behavior so placeholder hash titles never
/// collide in one visible list.
pub fn unique_session_short_ids_for_pairs(
    sessions: &[(String, String)],
) -> HashMap<String, String> {
    let mut out = HashMap::new();

    for (path, session_id) in sessions {
        let id_len = session_id.chars().count();
        let mut width = 7usize.min(id_len).max(1);
        while width < id_len {
            let suffix = session_id_suffix(session_id, width);
            let collision = sessions.iter().any(|(other_path, other_id)| {
                other_path != path && session_id_suffix(other_id, width) == suffix
            });
            if !collision {
                break;
            }
            width += 1;
        }
        out.insert(path.clone(), session_id_suffix(session_id, width));
    }
    out
}

fn collect_session_ids(node: &SessionNode, out: &mut Vec<(String, String)>) {
    if node.children.is_empty() {
        if let Some(session_id) = node.session_id.as_ref() {
            out.push((node.path.display().to_string(), session_id.clone()));
        }
        return;
    }

    for child in &node.children {
        collect_session_ids(child, out);
    }
}

fn session_id_suffix(id: &str, width: usize) -> String {
    let chars = id.chars().collect::<Vec<_>>();
    let start = chars.len().saturating_sub(width);
    chars[start..].iter().collect::<String>()
}
