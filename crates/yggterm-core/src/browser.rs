use crate::SessionNode;
use dirs::home_dir;
use std::path::Path;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRowKind {
    Group,
    Session,
}

#[derive(Debug, Clone)]
pub struct BrowserRow {
    pub kind: BrowserRowKind,
    pub full_path: String,
    pub label: String,
    pub detail_label: String,
    pub depth: usize,
    pub host_label: String,
    pub descendant_sessions: usize,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub struct SessionBrowserState {
    root: SessionNode,
    filter_query: String,
    expanded_paths: HashSet<String>,
    rows: Vec<BrowserRow>,
    selected_path: Option<String>,
}

impl SessionBrowserState {
    pub fn new(root: SessionNode) -> Self {
        let mut expanded_paths = HashSet::new();
        expand_all_groups(&root, &mut expanded_paths);

        let mut this = Self {
            root,
            filter_query: String::new(),
            expanded_paths,
            rows: Vec::new(),
            selected_path: None,
        };
        this.rebuild_rows();
        this.selected_path = this
            .rows
            .iter()
            .find(|row| row.kind == BrowserRowKind::Session)
            .map(|row| row.full_path.clone());
        this
    }

    pub fn rows(&self) -> &[BrowserRow] {
        &self.rows
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

    pub fn selected_row(&self) -> Option<&BrowserRow> {
        self.selected_path
            .as_deref()
            .and_then(|path| self.rows.iter().find(|row| row.full_path == path))
            .or_else(|| self.rows.first())
    }

    pub fn select_path(&mut self, path: impl Into<String>) {
        let path = path.into();
        if self.rows.iter().any(|row| row.full_path == path) {
            self.selected_path = Some(path);
        }
    }

    pub fn toggle_group(&mut self, path: &str) {
        if !self.expanded_paths.remove(path) {
            self.expanded_paths.insert(path.to_string());
        }
        self.rebuild_rows();
        self.ensure_selection();
    }

    pub fn total_sessions(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.kind == BrowserRowKind::Session)
            .count()
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
                .find(|row| row.kind == BrowserRowKind::Session)
                .map(|row| row.full_path.clone());
        }
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        let filter = self.filter_query.to_ascii_lowercase();
        flatten_rows(
            &self.root,
            0,
            &filter,
            &self.expanded_paths,
            &mut self.rows,
            true,
        );
    }
}

fn expand_all_groups(node: &SessionNode, expanded_paths: &mut HashSet<String>) {
    if !node.children.is_empty() {
        expanded_paths.insert(node.path.display().to_string());
        for child in &node.children {
            expand_all_groups(child, expanded_paths);
        }
    }
}

fn flatten_rows(
    node: &SessionNode,
    depth: usize,
    filter: &str,
    expanded_paths: &HashSet<String>,
    rows: &mut Vec<BrowserRow>,
    include_root: bool,
) -> bool {
    if !matches_filter(node, filter) {
        return false;
    }

    let full_path = node.path.display().to_string();
    let is_session = node.children.is_empty();
    let descendant_sessions = count_leaf_sessions(node);
    let expanded = is_session || expanded_paths.contains(&full_path);

    if include_root {
        rows.push(BrowserRow {
            kind: if is_session {
                BrowserRowKind::Session
            } else {
                BrowserRowKind::Group
            },
            label: format_row_label(node, depth, descendant_sessions, is_session),
            detail_label: detail_label_for_row(node, &full_path, is_session),
            full_path: full_path.clone(),
            depth,
            host_label: host_label_for_path(&full_path, depth),
            descendant_sessions,
            expanded,
        });
    }

    if !is_session && expanded {
        for child in &node.children {
            flatten_rows(child, depth + 1, filter, expanded_paths, rows, true);
        }
    }

    true
}

fn format_row_label(
    node: &SessionNode,
    depth: usize,
    descendant_sessions: usize,
    is_session: bool,
) -> String {
    if is_session {
        node.name.clone()
    } else if depth == 0 {
        node.name.clone()
    } else {
        format!("{} ({descendant_sessions})", node.name)
    }
}

fn detail_label_for_row(node: &SessionNode, full_path: &str, is_session: bool) -> String {
    if is_session {
        browser_display_path(
            node.path
                .parent()
                .and_then(|parent| parent.to_str())
                .unwrap_or(full_path),
        )
    } else {
        browser_display_path(full_path)
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

    node.children.iter().any(|child| matches_filter(child, filter))
}

fn count_leaf_sessions(node: &SessionNode) -> usize {
    if node.children.is_empty() {
        return 1;
    }

    node.children.iter().map(count_leaf_sessions).sum()
}

fn host_label_for_path(path: &str, depth: usize) -> String {
    if depth == 0 {
        return "workspace".to_string();
    }
    if depth == 1 {
        return "fleet".to_string();
    }
    if path.contains("/prod/") {
        return "prod-app-01".to_string();
    }
    if path.contains("codex") {
        return "codex".to_string();
    }
    if path.contains("ghostty") {
        return "ghostty".to_string();
    }
    if path.contains("local") {
        return "local".to_string();
    }
    "ssh".to_string()
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
