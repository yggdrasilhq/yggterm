use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use time::OffsetDateTime;

const WORKSPACE_DB_FILENAME: &str = "workspace.db";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceDocument {
    pub id: String,
    pub virtual_path: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDocumentSummary {
    pub id: String,
    pub virtual_path: String,
    pub title: String,
    pub updated_at: String,
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
                body TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_documents_virtual_path ON documents(virtual_path);",
        )
        .context("failed to initialize workspace db schema")?;
        Ok(Self { conn })
    }

    pub fn list_documents(&self) -> Result<Vec<WorkspaceDocumentSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, virtual_path, title, updated_at
             FROM documents
             ORDER BY virtual_path ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorkspaceDocumentSummary {
                id: row.get(0)?,
                virtual_path: row.get(1)?,
                title: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to read workspace document list")
    }

    pub fn get_document(&self, virtual_path: &str) -> Result<Option<WorkspaceDocument>> {
        let normalized = normalize_virtual_document_path(virtual_path);
        self.conn
            .query_row(
                "SELECT id, virtual_path, title, body, created_at, updated_at
                 FROM documents
                 WHERE virtual_path = ?1",
                params![normalized],
                |row| {
                    Ok(WorkspaceDocument {
                        id: row.get(0)?,
                        virtual_path: row.get(1)?,
                        title: row.get(2)?,
                        body: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
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
        let resolved_title = title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| default_document_title(&normalized));

        self.conn.execute(
            "INSERT INTO documents (id, virtual_path, title, body, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(virtual_path) DO UPDATE SET
               title = excluded.title,
               body = excluded.body,
               updated_at = excluded.updated_at",
            params![id, normalized, resolved_title, body, created_at, now],
        )?;

        self.get_document(virtual_path)?
            .context("workspace document was not readable after save")
    }
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
