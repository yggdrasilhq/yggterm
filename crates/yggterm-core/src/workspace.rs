use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use time::OffsetDateTime;

const WORKSPACE_DB_FILENAME: &str = "workspace.db";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceDocumentKind {
    #[default]
    Note,
    TerminalRecipe,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceGroupKind {
    #[default]
    Folder,
    Separator,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceDocument {
    pub id: String,
    pub virtual_path: String,
    pub title: String,
    pub kind: WorkspaceDocumentKind,
    pub body: String,
    pub source_session_path: Option<String>,
    pub source_session_kind: Option<String>,
    pub source_session_cwd: Option<String>,
    pub replay_commands: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDocumentSummary {
    pub id: String,
    pub virtual_path: String,
    pub title: String,
    pub kind: WorkspaceDocumentKind,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGroup {
    pub id: String,
    pub virtual_path: String,
    pub title: String,
    pub kind: WorkspaceGroupKind,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceDocumentInput {
    pub title: Option<String>,
    pub kind: WorkspaceDocumentKind,
    pub body: String,
    pub source_session_path: Option<String>,
    pub source_session_kind: Option<String>,
    pub source_session_cwd: Option<String>,
    pub replay_commands: Vec<String>,
}

pub struct WorkspaceStore {
    conn: Connection,
}

impl WorkspaceStore {
    pub fn open(home: &Path) -> Result<Self> {
        let db_path = home.join(WORKSPACE_DB_FILENAME);
        let conn = Connection::open(&db_path)
            .with_context(|| format!("failed to open workspace db {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                virtual_path TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'note',
                body TEXT NOT NULL,
                source_session_path TEXT,
                source_session_kind TEXT,
                source_session_cwd TEXT,
                replay_commands TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_documents_virtual_path ON documents(virtual_path);
            CREATE TABLE IF NOT EXISTS workspace_groups (
                id TEXT PRIMARY KEY,
                virtual_path TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'folder',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_workspace_groups_virtual_path ON workspace_groups(virtual_path);",
        )
        .context("failed to initialize workspace db schema")?;
        ensure_optional_column(&conn, "documents", "kind", "TEXT NOT NULL DEFAULT 'note'")?;
        ensure_optional_column(&conn, "documents", "source_session_path", "TEXT")?;
        ensure_optional_column(&conn, "documents", "source_session_kind", "TEXT")?;
        ensure_optional_column(&conn, "documents", "source_session_cwd", "TEXT")?;
        ensure_optional_column(&conn, "documents", "replay_commands", "TEXT")?;
        ensure_optional_column(
            &conn,
            "workspace_groups",
            "kind",
            "TEXT NOT NULL DEFAULT 'folder'",
        )?;
        migrate_legacy_workspace_paths(&conn)?;
        migrate_workspace_order_paths(&conn)?;
        Ok(Self { conn })
    }

    pub fn list_documents(&self) -> Result<Vec<WorkspaceDocumentSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, virtual_path, title, kind, updated_at
             FROM documents
             ORDER BY virtual_path ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorkspaceDocumentSummary {
                id: row.get(0)?,
                virtual_path: row.get(1)?,
                title: row.get(2)?,
                kind: parse_document_kind(row.get::<_, String>(3)?),
                updated_at: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to read workspace document list")
    }

    pub fn list_groups(&self) -> Result<Vec<WorkspaceGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, virtual_path, title, kind, created_at, updated_at
             FROM workspace_groups
             ORDER BY virtual_path ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorkspaceGroup {
                id: row.get(0)?,
                virtual_path: row.get(1)?,
                title: row.get(2)?,
                kind: parse_group_kind(row.get::<_, String>(3)?),
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to read workspace group list")
    }

    pub fn get_document(&self, virtual_path: &str) -> Result<Option<WorkspaceDocument>> {
        let normalized = normalize_virtual_document_path(virtual_path);
        self.conn
            .query_row(
                "SELECT id, virtual_path, title, kind, body, source_session_path, source_session_kind, source_session_cwd, replay_commands, created_at, updated_at
                 FROM documents
                 WHERE virtual_path = ?1",
                params![normalized],
                |row| {
                    Ok(WorkspaceDocument {
                        id: row.get(0)?,
                        virtual_path: row.get(1)?,
                        title: row.get(2)?,
                        kind: parse_document_kind(row.get::<_, String>(3)?),
                        body: row.get(4)?,
                        source_session_path: row.get(5)?,
                        source_session_kind: row.get(6)?,
                        source_session_cwd: row.get(7)?,
                        replay_commands: row
                            .get::<_, Option<String>>(8)?
                            .as_deref()
                            .map(parse_replay_commands)
                            .unwrap_or_default(),
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .context("failed to query workspace document")
    }

    pub fn put_document(
        &self,
        virtual_path: &str,
        title: Option<&str>,
        body: &str,
    ) -> Result<WorkspaceDocument> {
        self.put_document_input(
            virtual_path,
            WorkspaceDocumentInput {
                title: title.map(ToOwned::to_owned),
                body: body.to_string(),
                ..WorkspaceDocumentInput::default()
            },
        )
    }

    pub fn put_document_input(
        &self,
        virtual_path: &str,
        input: WorkspaceDocumentInput,
    ) -> Result<WorkspaceDocument> {
        let normalized = normalize_virtual_document_path(virtual_path);
        let now =
            OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
        let existing = self.get_document(&normalized)?;
        let id = existing
            .as_ref()
            .map(|document| document.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let created_at = existing
            .as_ref()
            .map(|document| document.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let resolved_title = input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| default_document_title(&normalized));
        let replay_commands = if input.replay_commands.is_empty() {
            None
        } else {
            Some(serialize_replay_commands(&input.replay_commands)?)
        };

        self.conn.execute(
            "INSERT INTO documents (id, virtual_path, title, kind, body, source_session_path, source_session_kind, source_session_cwd, replay_commands, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(virtual_path) DO UPDATE SET
               title = excluded.title,
               kind = excluded.kind,
               body = excluded.body,
               source_session_path = excluded.source_session_path,
               source_session_kind = excluded.source_session_kind,
               source_session_cwd = excluded.source_session_cwd,
               replay_commands = excluded.replay_commands,
               updated_at = excluded.updated_at",
            params![
                id,
                normalized,
                resolved_title,
                document_kind_name(input.kind),
                input.body,
                input.source_session_path,
                input.source_session_kind,
                input.source_session_cwd,
                replay_commands,
                created_at,
                now
            ],
        )?;

        self.get_document(virtual_path)?
            .context("workspace document was not readable after save")
    }

    pub fn put_group(&self, virtual_path: &str, title: Option<&str>) -> Result<WorkspaceGroup> {
        self.put_group_with_kind(virtual_path, title, WorkspaceGroupKind::Folder)
    }

    pub fn put_group_with_kind(
        &self,
        virtual_path: &str,
        title: Option<&str>,
        kind: WorkspaceGroupKind,
    ) -> Result<WorkspaceGroup> {
        let normalized = normalize_virtual_group_path(virtual_path);
        let now =
            OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
        let existing = self
            .conn
            .query_row(
                "SELECT id, created_at FROM workspace_groups WHERE virtual_path = ?1",
                params![normalized],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .context("failed to query workspace group")?;
        let id = existing
            .as_ref()
            .map(|value| value.0.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let created_at = existing
            .as_ref()
            .map(|value| value.1.clone())
            .unwrap_or_else(|| now.clone());
        let resolved_title = title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| default_group_title(&normalized, kind));

        self.conn.execute(
            "INSERT INTO workspace_groups (id, virtual_path, title, kind, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(virtual_path) DO UPDATE SET
               title = excluded.title,
               kind = excluded.kind,
               updated_at = excluded.updated_at",
            params![
                id,
                normalized,
                resolved_title,
                group_kind_name(kind),
                created_at,
                now
            ],
        )?;

        self.conn
            .query_row(
                "SELECT id, virtual_path, title, kind, created_at, updated_at
                 FROM workspace_groups WHERE virtual_path = ?1",
                params![normalize_virtual_group_path(virtual_path)],
                |row| {
                    Ok(WorkspaceGroup {
                        id: row.get(0)?,
                        virtual_path: row.get(1)?,
                        title: row.get(2)?,
                        kind: parse_group_kind(row.get::<_, String>(3)?),
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .context("workspace group was not readable after save")
    }

    pub fn move_document(
        &self,
        from_virtual_path: &str,
        to_virtual_path: &str,
    ) -> Result<WorkspaceDocument> {
        let from_normalized = normalize_virtual_document_path(from_virtual_path);
        let to_normalized = normalize_virtual_document_path(to_virtual_path);
        if from_normalized == to_normalized {
            return self
                .get_document(&from_normalized)?
                .context("workspace document was not readable for move");
        }

        if self.get_document(&to_normalized)?.is_some() {
            bail!("destination document path already exists: {to_normalized}");
        }

        let Some(existing) = self.get_document(&from_normalized)? else {
            bail!("workspace document not found: {from_normalized}");
        };
        let now =
            OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
        self.conn.execute(
            "UPDATE documents
             SET virtual_path = ?1, updated_at = ?2
             WHERE virtual_path = ?3",
            params![to_normalized, now, from_normalized],
        )?;

        self.get_document(&to_normalized)?
            .context("workspace document was not readable after move")
            .map(|mut document| {
                document.title = existing.title;
                document
            })
    }

    pub fn delete_documents(&self, virtual_paths: &[String]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut deleted = 0usize;
        for path in virtual_paths {
            let normalized = normalize_virtual_document_path(path);
            deleted += tx.execute(
                "DELETE FROM documents WHERE virtual_path = ?1",
                params![normalized],
            )?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub fn move_group(
        &self,
        from_virtual_path: &str,
        to_virtual_path: &str,
    ) -> Result<WorkspaceGroup> {
        let from_normalized = normalize_virtual_group_path(from_virtual_path);
        let to_normalized = normalize_virtual_group_path(to_virtual_path);
        if from_normalized == to_normalized {
            return self
                .get_group(&from_normalized)?
                .context("workspace group was not readable for move");
        }
        if to_normalized == "/" || to_normalized.starts_with(&(from_normalized.clone() + "/")) {
            bail!("cannot move a folder into itself: {from_normalized} -> {to_normalized}");
        }
        if self.get_group(&to_normalized)?.is_some() {
            bail!("destination group path already exists: {to_normalized}");
        }
        if self.get_document(&to_normalized)?.is_some() {
            bail!("destination path already exists as a paper: {to_normalized}");
        }

        let Some(group) = self.get_group(&from_normalized)? else {
            bail!("workspace group not found: {from_normalized}");
        };

        let descendant_groups = self.list_groups()?;
        let descendant_documents = self.list_documents()?;
        for candidate in descendant_groups.iter().filter(|candidate| {
            is_same_or_descendant_path(&candidate.virtual_path, &from_normalized)
        }) {
            let rewritten =
                rewrite_virtual_prefix(&candidate.virtual_path, &from_normalized, &to_normalized);
            if rewritten != candidate.virtual_path
                && self.get_group(&rewritten)?.is_some()
                && !is_same_or_descendant_path(&rewritten, &from_normalized)
            {
                bail!("destination group path already exists: {rewritten}");
            }
        }
        for candidate in descendant_documents.iter().filter(|candidate| {
            is_same_or_descendant_path(&candidate.virtual_path, &from_normalized)
        }) {
            let rewritten =
                rewrite_virtual_prefix(&candidate.virtual_path, &from_normalized, &to_normalized);
            if rewritten != candidate.virtual_path && self.get_document(&rewritten)?.is_some() {
                bail!("destination paper path already exists: {rewritten}");
            }
        }

        let now =
            OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
        self.conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;
        let result: Result<()> = (|| {
            let descendants_prefix = format!("{from_normalized}/%");
            self.conn.execute(
                "UPDATE workspace_groups
                 SET virtual_path = REPLACE(virtual_path, ?1, ?2), updated_at = ?3
                 WHERE virtual_path = ?1 OR virtual_path LIKE ?4",
                params![from_normalized, to_normalized, now, descendants_prefix],
            )?;
            self.conn.execute(
                "UPDATE documents
                 SET virtual_path = REPLACE(virtual_path, ?1, ?2), updated_at = ?3
                 WHERE virtual_path LIKE ?4",
                params![from_normalized, to_normalized, now, descendants_prefix],
            )?;
            Ok(())
        })();

        match result {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }

        self.get_group(&to_normalized)?
            .context("workspace group was not readable after move")
            .map(|mut moved| {
                moved.title = group.title;
                moved
            })
    }

    pub fn delete_workspace_items(
        &self,
        document_paths: &[String],
        group_paths: &[String],
    ) -> Result<usize> {
        self.conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;
        let result: Result<usize> = (|| {
            let mut deleted = 0usize;
            for path in document_paths {
                let normalized = normalize_virtual_document_path(path);
                deleted += self.conn.execute(
                    "DELETE FROM documents WHERE virtual_path = ?1",
                    params![normalized],
                )?;
            }
            for path in group_paths {
                let normalized = normalize_virtual_group_path(path);
                let descendants_prefix = format!("{normalized}/%");
                deleted += self.conn.execute(
                    "DELETE FROM documents WHERE virtual_path LIKE ?1",
                    params![descendants_prefix],
                )?;
                deleted += self.conn.execute(
                    "DELETE FROM workspace_groups WHERE virtual_path = ?1 OR virtual_path LIKE ?2",
                    params![normalized, descendants_prefix],
                )?;
            }
            Ok(deleted)
        })();

        match result {
            Ok(deleted) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(deleted)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(error)
            }
        }
    }

    fn get_group(&self, virtual_path: &str) -> Result<Option<WorkspaceGroup>> {
        self.conn
            .query_row(
                "SELECT id, virtual_path, title, kind, created_at, updated_at
                 FROM workspace_groups WHERE virtual_path = ?1",
                params![normalize_virtual_group_path(virtual_path)],
                |row| {
                    Ok(WorkspaceGroup {
                        id: row.get(0)?,
                        virtual_path: row.get(1)?,
                        title: row.get(2)?,
                        kind: parse_group_kind(row.get::<_, String>(3)?),
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .context("failed to read workspace group")
    }
}

fn ensure_optional_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let existing = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    if existing.iter().any(|name| name == column) {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )
    .with_context(|| format!("failed to add column {table}.{column}"))?;
    Ok(())
}

fn migrate_legacy_workspace_paths(conn: &Connection) -> Result<()> {
    let document_paths = {
        let mut stmt =
            conn.prepare("SELECT virtual_path FROM documents ORDER BY virtual_path ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let group_paths = {
        let mut stmt =
            conn.prepare("SELECT virtual_path FROM workspace_groups ORDER BY virtual_path ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };

    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;
    let result: Result<()> = (|| {
        for path in document_paths {
            let rewritten = rewrite_legacy_workspace_path(&path);
            if rewritten == path {
                continue;
            }
            let target = normalize_virtual_document_path(&rewritten);
            let target_exists = conn
                .query_row(
                    "SELECT 1 FROM documents WHERE virtual_path = ?1",
                    params![target],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if target_exists {
                conn.execute(
                    "DELETE FROM documents WHERE virtual_path = ?1",
                    params![path],
                )?;
            } else {
                conn.execute(
                    "UPDATE documents SET virtual_path = ?1 WHERE virtual_path = ?2",
                    params![target, path],
                )?;
            }
        }

        for path in group_paths {
            let rewritten = rewrite_legacy_workspace_path(&path);
            if rewritten == path {
                continue;
            }
            let target = normalize_virtual_group_path(&rewritten);
            let target_exists = conn
                .query_row(
                    "SELECT 1 FROM workspace_groups WHERE virtual_path = ?1",
                    params![target],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if target_exists {
                conn.execute(
                    "DELETE FROM workspace_groups WHERE virtual_path = ?1",
                    params![path],
                )?;
            } else {
                conn.execute(
                    "UPDATE workspace_groups SET virtual_path = ?1 WHERE virtual_path = ?2",
                    params![target, path],
                )?;
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => conn.execute_batch("COMMIT;")?,
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(error);
        }
    }

    Ok(())
}

fn migrate_workspace_order_paths(conn: &Connection) -> Result<()> {
    let document_paths = {
        let mut stmt =
            conn.prepare("SELECT virtual_path FROM documents ORDER BY virtual_path ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let group_paths = {
        let mut stmt =
            conn.prepare("SELECT virtual_path FROM workspace_groups ORDER BY virtual_path ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };

    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;
    let result: Result<()> = (|| {
        for path in document_paths {
            let rewritten = simplify_workspace_order_path(&path);
            if rewritten == path {
                continue;
            }
            let target_exists = conn
                .query_row(
                    "SELECT 1 FROM documents WHERE virtual_path = ?1",
                    params![rewritten],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !target_exists {
                conn.execute(
                    "UPDATE documents SET virtual_path = ?1 WHERE virtual_path = ?2",
                    params![rewritten, path],
                )?;
            }
        }

        for path in group_paths {
            let rewritten = simplify_workspace_order_path(&path);
            if rewritten == path {
                continue;
            }
            let target_exists = conn
                .query_row(
                    "SELECT 1 FROM workspace_groups WHERE virtual_path = ?1",
                    params![rewritten],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !target_exists {
                conn.execute(
                    "UPDATE workspace_groups SET virtual_path = ?1 WHERE virtual_path = ?2",
                    params![rewritten, path],
                )?;
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => conn.execute_batch("COMMIT;")?,
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(error);
        }
    }

    Ok(())
}

fn simplify_workspace_order_path(path: &str) -> String {
    let mut normalized_segments = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        let parts = segment
            .split('~')
            .map(simplify_workspace_order_part)
            .collect::<Vec<_>>();
        normalized_segments.push(parts.join("~"));
    }
    if normalized_segments.is_empty() {
        "/documents/untitled".to_string()
    } else {
        format!("/{}", normalized_segments.join("/"))
    }
}

fn simplify_workspace_order_part(part: &str) -> String {
    let (prefix, mut rest) = if let Some(stripped) = part.strip_prefix('!') {
        ("!", stripped)
    } else {
        ("", part)
    };
    let mut kept_order_prefix = None;
    while rest.len() > 5
        && rest.as_bytes().get(4) == Some(&b'-')
        && rest.as_bytes()[0..4].iter().all(|byte| byte.is_ascii_digit())
    {
        let candidate = &rest[..5];
        if kept_order_prefix.is_none() {
            kept_order_prefix = Some(candidate);
        }
        rest = &rest[5..];
    }
    match kept_order_prefix {
        Some(order_prefix) => format!("{prefix}{order_prefix}{rest}"),
        None => format!("{prefix}{rest}"),
    }
}

fn rewrite_legacy_workspace_path(path: &str) -> String {
    let mut rewritten = path.to_string();
    if let Some(rest) = rewritten.strip_prefix("/local") {
        rewritten = if rest.is_empty() {
            "/".to_string()
        } else {
            rest.to_string()
        };
    }
    while rewritten.contains("/notes/notes/") {
        rewritten = rewritten.replace("/notes/notes/", "/notes/");
    }
    rewritten
}

fn document_kind_name(kind: WorkspaceDocumentKind) -> &'static str {
    match kind {
        WorkspaceDocumentKind::Note => "note",
        WorkspaceDocumentKind::TerminalRecipe => "terminal_recipe",
    }
}

fn parse_document_kind(raw: String) -> WorkspaceDocumentKind {
    match raw.as_str() {
        "terminal_recipe" => WorkspaceDocumentKind::TerminalRecipe,
        _ => WorkspaceDocumentKind::Note,
    }
}

fn group_kind_name(kind: WorkspaceGroupKind) -> &'static str {
    match kind {
        WorkspaceGroupKind::Folder => "folder",
        WorkspaceGroupKind::Separator => "separator",
    }
}

fn parse_group_kind(raw: String) -> WorkspaceGroupKind {
    match raw.as_str() {
        "separator" => WorkspaceGroupKind::Separator,
        _ => WorkspaceGroupKind::Folder,
    }
}

fn is_same_or_descendant_path(candidate: &str, parent: &str) -> bool {
    candidate == parent
        || candidate
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn rewrite_virtual_prefix(path: &str, from: &str, to: &str) -> String {
    if path == from {
        return to.to_string();
    }
    if let Some(rest) = path.strip_prefix(&(from.to_string() + "/")) {
        format!("{}/{}", to.trim_end_matches('/'), rest)
    } else {
        path.to_string()
    }
}

fn serialize_replay_commands(commands: &[String]) -> Result<String> {
    serde_json::to_string(commands).context("failed to serialize replay commands")
}

fn parse_replay_commands(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

pub fn normalize_virtual_document_path(input: &str) -> String {
    let mut segments = Vec::new();
    for segment in input.split('/') {
        let trimmed = segment.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
            continue;
        }
        segments.push(trimmed.replace(['\\', ':', '\0'], "_"));
    }
    if segments.is_empty() {
        "/documents/untitled".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

pub fn normalize_virtual_group_path(input: &str) -> String {
    normalize_virtual_document_path(input)
}

pub fn default_document_title(virtual_path: &str) -> String {
    virtual_path
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "document".to_string())
}

pub fn default_group_title(virtual_path: &str, kind: WorkspaceGroupKind) -> String {
    match kind {
        WorkspaceGroupKind::Folder => virtual_path
            .rsplit('/')
            .find(|segment| !segment.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "folder".to_string()),
        WorkspaceGroupKind::Separator => "separator".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_home() -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("yggterm-workspace-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temp home");
        path
    }

    #[test]
    fn persists_workspace_groups() {
        let home = temp_home();
        let store = WorkspaceStore::open(&home).expect("open workspace");
        let group = store
            .put_group("/projects/alpha/ideas", Some("Ideas"))
            .expect("save group");

        let groups = store.list_groups().expect("list groups");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].virtual_path, "/projects/alpha/ideas");
        assert_eq!(groups[0].title, "Ideas");
        assert_eq!(groups[0].kind, WorkspaceGroupKind::Folder);
        assert_eq!(group.title, "Ideas");

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn moves_workspace_documents() {
        let home = temp_home();
        let store = WorkspaceStore::open(&home).expect("open workspace");
        store
            .put_document("/projects/alpha/notes/todo", Some("Todo"), "body")
            .expect("save document");

        let moved = store
            .move_document("/projects/alpha/notes/todo", "/projects/beta/notes/todo")
            .expect("move document");

        assert_eq!(moved.virtual_path, "/projects/beta/notes/todo");
        assert!(
            store
                .get_document("/projects/alpha/notes/todo")
                .expect("read source")
                .is_none()
        );
        assert!(
            store
                .get_document("/projects/beta/notes/todo")
                .expect("read destination")
                .is_some()
        );

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn moves_workspace_group_subtree() {
        let home = temp_home();
        let store = WorkspaceStore::open(&home).expect("open workspace");
        store
            .put_group_with_kind(
                "/projects/alpha/folder-a",
                Some("Folder A"),
                WorkspaceGroupKind::Folder,
            )
            .expect("save folder");
        store
            .put_group_with_kind(
                "/projects/alpha/folder-a/divider",
                Some("Divider"),
                WorkspaceGroupKind::Separator,
            )
            .expect("save separator");
        store
            .put_document("/projects/alpha/folder-a/notes/todo", Some("Todo"), "body")
            .expect("save paper");

        let moved = store
            .move_group("/projects/alpha/folder-a", "/projects/beta/folder-a")
            .expect("move folder");

        assert_eq!(moved.virtual_path, "/projects/beta/folder-a");
        assert!(
            store
                .get_document("/projects/beta/folder-a/notes/todo")
                .expect("read moved paper")
                .is_some()
        );
        assert!(
            store
                .get_group("/projects/beta/folder-a/divider")
                .expect("read moved separator")
                .is_some()
        );

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn deletes_workspace_items_recursively() {
        let home = temp_home();
        let store = WorkspaceStore::open(&home).expect("open workspace");
        store
            .put_group_with_kind(
                "/projects/alpha/folder-a",
                Some("Folder A"),
                WorkspaceGroupKind::Folder,
            )
            .expect("save folder");
        store
            .put_document("/projects/alpha/folder-a/notes/todo", Some("Todo"), "body")
            .expect("save paper");
        store
            .put_document(
                "/projects/alpha/notes/standalone",
                Some("Standalone"),
                "body",
            )
            .expect("save standalone paper");

        let deleted = store
            .delete_workspace_items(
                &[String::from("/projects/alpha/notes/standalone")],
                &[String::from("/projects/alpha/folder-a")],
            )
            .expect("delete items");

        assert_eq!(deleted, 3);
        assert!(
            store
                .get_document("/projects/alpha/folder-a/notes/todo")
                .expect("read nested paper")
                .is_none()
        );
        assert!(
            store
                .get_document("/projects/alpha/notes/standalone")
                .expect("read standalone paper")
                .is_none()
        );

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn rewrites_legacy_workspace_paths() {
        assert_eq!(
            rewrite_legacy_workspace_path("/local/home/pi/gh/notes/separator-1"),
            "/home/pi/gh/notes/separator-1"
        );
        assert_eq!(
            rewrite_legacy_workspace_path("/home/pi/gh/notes/notes/untitled-1"),
            "/home/pi/gh/notes/untitled-1"
        );
    }

    #[test]
    fn simplifies_compounded_workspace_order_paths() {
        assert_eq!(
            simplify_workspace_order_path(
                "/home/pi/gh/notes/0000-0008-0000-untitled-1774119990~0002-0002-0001-note"
            ),
            "/home/pi/gh/notes/0000-untitled-1774119990~0002-note"
        );
        assert_eq!(
            simplify_workspace_order_path(
                "/home/pi/gh/notes/!0000-0000-separator-1774185969049440371"
            ),
            "/home/pi/gh/notes/!0000-separator-1774185969049440371"
        );
    }
}
