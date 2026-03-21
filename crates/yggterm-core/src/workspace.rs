use anyhow::{Context, Result};
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
            CREATE INDEX IF NOT EXISTS idx_documents_virtual_path ON documents(virtual_path);",
        )
        .context("failed to initialize workspace db schema")?;
        ensure_optional_column(
            &conn,
            "documents",
            "kind",
            "TEXT NOT NULL DEFAULT 'note'",
        )?;
        ensure_optional_column(&conn, "documents", "source_session_path", "TEXT")?;
        ensure_optional_column(&conn, "documents", "source_session_kind", "TEXT")?;
        ensure_optional_column(&conn, "documents", "source_session_cwd", "TEXT")?;
        ensure_optional_column(&conn, "documents", "replay_commands", "TEXT")?;
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
        let now = OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
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
}

fn ensure_optional_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
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

pub fn default_document_title(virtual_path: &str) -> String {
    virtual_path
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "document".to_string())
}
