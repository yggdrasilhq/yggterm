use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::{Duration as TimeDuration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const REMOTE_RUNTIME_DB_FILENAME: &str = "remote-runtime.db";
const REMOTE_RUNTIME_SESSIONS_DIR: &str = "sessions";
const TRANSCRIPT_LOG_FILENAME: &str = "transcript.log";
const PTY_LOG_FILENAME: &str = "pty.log";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteRuntimeKind {
    Codex,
    Shell,
    ClaudeCode,
}

impl RemoteRuntimeKind {
    fn as_db(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Shell => "shell",
            Self::ClaudeCode => "claude_code",
        }
    }

    fn from_db(value: &str) -> Self {
        match value {
            "shell" => Self::Shell,
            "claude_code" => Self::ClaudeCode,
            _ => Self::Codex,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteRuntimeSessionState {
    Created,
    Starting,
    RestoringContext,
    AttachingPty,
    Interactive,
    Degraded,
    Failed,
    Stopped,
}

impl RemoteRuntimeSessionState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Starting => "starting",
            Self::RestoringContext => "restoring_context",
            Self::AttachingPty => "attaching_pty",
            Self::Interactive => "interactive",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }

    fn from_db(value: &str) -> Self {
        match value {
            "starting" => Self::Starting,
            "restoring_context" => Self::RestoringContext,
            "attaching_pty" => Self::AttachingPty,
            "interactive" => Self::Interactive,
            "degraded" => Self::Degraded,
            "failed" => Self::Failed,
            "stopped" => Self::Stopped,
            _ => Self::Created,
        }
    }

    fn implied_health(self) -> RemoteRuntimeHealth {
        match self {
            Self::Interactive => RemoteRuntimeHealth::Healthy,
            Self::Degraded => RemoteRuntimeHealth::Degraded,
            Self::Failed => RemoteRuntimeHealth::Failed,
            _ => RemoteRuntimeHealth::Unknown,
        }
    }

    fn implies_interactive(self) -> bool {
        matches!(self, Self::Interactive)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteRuntimeHealth {
    Unknown,
    Healthy,
    Degraded,
    Failed,
}

impl RemoteRuntimeHealth {
    fn as_db(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }

    fn from_db(value: &str) -> Self {
        match value {
            "healthy" => Self::Healthy,
            "degraded" => Self::Degraded,
            "failed" => Self::Failed,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWriterLease {
    pub lease_id: String,
    pub client_id: String,
    pub acquired_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRuntimeSession {
    pub session_id: String,
    pub machine_key: String,
    pub runtime_kind: RemoteRuntimeKind,
    pub state: RemoteRuntimeSessionState,
    pub health: RemoteRuntimeHealth,
    pub title: String,
    pub cwd: Option<String>,
    pub summary: Option<String>,
    pub requires_terminal: bool,
    pub interactive_ready: bool,
    pub transcript_rel_path: String,
    pub pty_rel_path: String,
    pub writer_lease: Option<RemoteWriterLease>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_transition_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRuntimeSessionInput {
    pub session_id: Option<String>,
    pub machine_key: String,
    pub runtime_kind: RemoteRuntimeKind,
    pub title: String,
    pub cwd: Option<String>,
    pub summary: Option<String>,
    pub requires_terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRuntimeEvent {
    pub event_id: String,
    pub session_id: String,
    pub event_kind: String,
    pub from_state: Option<RemoteRuntimeSessionState>,
    pub to_state: Option<RemoteRuntimeSessionState>,
    pub detail: Option<String>,
    pub at: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRuntimePaths {
    pub db_path: PathBuf,
    pub sessions_dir: PathBuf,
}

pub struct RemoteRuntimeRegistry {
    conn: Connection,
    paths: RemoteRuntimePaths,
}

impl RemoteRuntimeRegistry {
    pub fn open(home: &Path) -> Result<Self> {
        let db_path = home.join(REMOTE_RUNTIME_DB_FILENAME);
        let sessions_dir = home.join(REMOTE_RUNTIME_SESSIONS_DIR);
        fs::create_dir_all(&sessions_dir)
            .with_context(|| format!("creating runtime sessions dir {}", sessions_dir.display()))?;
        let conn = Connection::open(&db_path)
            .with_context(|| format!("opening runtime db {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS runtime_sessions (
                session_id TEXT PRIMARY KEY,
                machine_key TEXT NOT NULL,
                runtime_kind TEXT NOT NULL,
                state TEXT NOT NULL,
                health TEXT NOT NULL,
                title TEXT NOT NULL,
                cwd TEXT,
                summary TEXT,
                requires_terminal INTEGER NOT NULL DEFAULT 1,
                interactive_ready INTEGER NOT NULL DEFAULT 0,
                transcript_rel_path TEXT NOT NULL,
                pty_rel_path TEXT NOT NULL,
                writer_lease_id TEXT,
                writer_client_id TEXT,
                writer_lease_acquired_at TEXT,
                writer_lease_expires_at TEXT,
                last_error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_transition_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_runtime_sessions_machine_key
                ON runtime_sessions(machine_key);
            CREATE INDEX IF NOT EXISTS idx_runtime_sessions_state
                ON runtime_sessions(state);
            CREATE TABLE IF NOT EXISTS runtime_events (
                event_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                event_kind TEXT NOT NULL,
                from_state TEXT,
                to_state TEXT,
                detail TEXT,
                at TEXT NOT NULL,
                payload_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_runtime_events_session
                ON runtime_events(session_id, at DESC);",
        )
        .context("initializing runtime db schema")?;
        Ok(Self {
            conn,
            paths: RemoteRuntimePaths {
                db_path,
                sessions_dir,
            },
        })
    }

    pub fn paths(&self) -> &RemoteRuntimePaths {
        &self.paths
    }

    pub fn register_session(
        &self,
        input: RemoteRuntimeSessionInput,
    ) -> Result<RemoteRuntimeSession> {
        let session_id = input
            .session_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let runtime_dir = self.paths.sessions_dir.join(&session_id);
        fs::create_dir_all(&runtime_dir)
            .with_context(|| format!("creating runtime dir {}", runtime_dir.display()))?;
        let transcript_rel_path = runtime_rel_path(
            &self.paths.sessions_dir,
            &runtime_dir.join(TRANSCRIPT_LOG_FILENAME),
        );
        let pty_rel_path = runtime_rel_path(
            &self.paths.sessions_dir,
            &runtime_dir.join(PTY_LOG_FILENAME),
        );
        let now = now_rfc3339()?;
        self.conn.execute(
            "INSERT INTO runtime_sessions (
                session_id, machine_key, runtime_kind, state, health, title, cwd, summary,
                requires_terminal, interactive_ready, transcript_rel_path, pty_rel_path,
                created_at, updated_at, last_transition_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?12, ?12, ?12)
            ON CONFLICT(session_id) DO UPDATE SET
                machine_key = excluded.machine_key,
                runtime_kind = excluded.runtime_kind,
                title = excluded.title,
                cwd = excluded.cwd,
                summary = excluded.summary,
                requires_terminal = excluded.requires_terminal,
                transcript_rel_path = excluded.transcript_rel_path,
                pty_rel_path = excluded.pty_rel_path,
                updated_at = excluded.updated_at",
            params![
                session_id,
                input.machine_key,
                input.runtime_kind.as_db(),
                RemoteRuntimeSessionState::Created.as_db(),
                RemoteRuntimeHealth::Unknown.as_db(),
                input.title,
                input.cwd,
                input.summary,
                bool_to_sql(input.requires_terminal),
                transcript_rel_path,
                pty_rel_path,
                now,
            ],
        )?;
        self.insert_event(
            &session_id,
            "session_registered",
            None,
            Some(RemoteRuntimeSessionState::Created),
            None,
            &json!({ "requires_terminal": input.requires_terminal }),
        )?;
        self.get_session(&session_id)?
            .with_context(|| format!("reloading runtime session {session_id} after register"))
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<RemoteRuntimeSession>> {
        self.conn
            .query_row(
                "SELECT
                    session_id, machine_key, runtime_kind, state, health, title, cwd, summary,
                    requires_terminal, interactive_ready, transcript_rel_path, pty_rel_path,
                    writer_lease_id, writer_client_id, writer_lease_acquired_at, writer_lease_expires_at,
                    last_error, created_at, updated_at, last_transition_at
                 FROM runtime_sessions
                 WHERE session_id = ?1",
                params![session_id],
                read_runtime_session,
            )
            .optional()
            .context("querying runtime session")
    }

    pub fn list_sessions(&self) -> Result<Vec<RemoteRuntimeSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                session_id, machine_key, runtime_kind, state, health, title, cwd, summary,
                requires_terminal, interactive_ready, transcript_rel_path, pty_rel_path,
                writer_lease_id, writer_client_id, writer_lease_acquired_at, writer_lease_expires_at,
                last_error, created_at, updated_at, last_transition_at
             FROM runtime_sessions
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], read_runtime_session)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("listing runtime sessions")
    }

    pub fn transition_session(
        &self,
        session_id: &str,
        to_state: RemoteRuntimeSessionState,
        detail: Option<&str>,
        payload: &Value,
    ) -> Result<RemoteRuntimeSession> {
        let current = self
            .get_session(session_id)?
            .with_context(|| format!("missing runtime session {session_id}"))?;
        let now = now_rfc3339()?;
        let health = to_state.implied_health();
        let last_error = match to_state {
            RemoteRuntimeSessionState::Degraded | RemoteRuntimeSessionState::Failed => {
                detail.map(ToOwned::to_owned)
            }
            _ => None,
        };
        self.conn.execute(
            "UPDATE runtime_sessions
             SET state = ?2,
                 health = ?3,
                 interactive_ready = ?4,
                 last_error = ?5,
                 updated_at = ?6,
                 last_transition_at = ?6
             WHERE session_id = ?1",
            params![
                session_id,
                to_state.as_db(),
                health.as_db(),
                bool_to_sql(to_state.implies_interactive()),
                last_error,
                now,
            ],
        )?;
        self.insert_event(
            session_id,
            "state_transition",
            Some(current.state),
            Some(to_state),
            detail,
            payload,
        )?;
        self.get_session(session_id)?
            .with_context(|| format!("reloading runtime session {session_id} after transition"))
    }

    pub fn list_events(&self, session_id: &str, limit: usize) -> Result<Vec<RemoteRuntimeEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, session_id, event_kind, from_state, to_state, detail, at, payload_json
             FROM runtime_events
             WHERE session_id = ?1
             ORDER BY at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], |row| {
            Ok(RemoteRuntimeEvent {
                event_id: row.get(0)?,
                session_id: row.get(1)?,
                event_kind: row.get(2)?,
                from_state: row
                    .get::<_, Option<String>>(3)?
                    .map(|value| RemoteRuntimeSessionState::from_db(&value)),
                to_state: row
                    .get::<_, Option<String>>(4)?
                    .map(|value| RemoteRuntimeSessionState::from_db(&value)),
                detail: row.get(5)?,
                at: row.get(6)?,
                payload: serde_json::from_str::<Value>(&row.get::<_, String>(7)?)
                    .unwrap_or(Value::Null),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("listing runtime events")
    }

    pub fn acquire_writer_lease(
        &self,
        session_id: &str,
        client_id: &str,
        ttl: Duration,
    ) -> Result<RemoteWriterLease> {
        let lease_id = Uuid::new_v4().to_string();
        let acquired_at = OffsetDateTime::now_utc();
        let expires_at = acquired_at + duration_to_time(ttl);
        let acquired_at_s = format_time(acquired_at)?;
        let expires_at_s = format_time(expires_at)?;
        let now = acquired_at_s.clone();
        let updated = self.conn.execute(
            "UPDATE runtime_sessions
             SET writer_lease_id = ?2,
                 writer_client_id = ?3,
                 writer_lease_acquired_at = ?4,
                 writer_lease_expires_at = ?5,
                 updated_at = ?4
             WHERE session_id = ?1
               AND (
                    writer_lease_expires_at IS NULL
                    OR writer_lease_expires_at <= ?6
                    OR writer_client_id = ?3
               )",
            params![
                session_id,
                lease_id,
                client_id,
                acquired_at_s,
                expires_at_s,
                now
            ],
        )?;
        if updated == 0 {
            bail!("session {session_id} already has an active writer lease");
        }
        self.insert_event(
            session_id,
            "writer_lease_acquired",
            None,
            None,
            Some(client_id),
            &json!({ "lease_id": lease_id, "expires_at": expires_at_s }),
        )?;
        Ok(RemoteWriterLease {
            lease_id,
            client_id: client_id.to_string(),
            acquired_at: now,
            expires_at: expires_at_s,
        })
    }

    pub fn release_writer_lease(&self, session_id: &str, lease_id: &str) -> Result<bool> {
        let now = now_rfc3339()?;
        let updated = self.conn.execute(
            "UPDATE runtime_sessions
             SET writer_lease_id = NULL,
                 writer_client_id = NULL,
                 writer_lease_acquired_at = NULL,
                 writer_lease_expires_at = NULL,
                 updated_at = ?3
             WHERE session_id = ?1 AND writer_lease_id = ?2",
            params![session_id, lease_id, now],
        )?;
        if updated > 0 {
            self.insert_event(
                session_id,
                "writer_lease_released",
                None,
                None,
                Some(lease_id),
                &Value::Null,
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn append_transcript_chunk(&self, session_id: &str, chunk: &[u8]) -> Result<usize> {
        self.append_session_log(session_id, chunk, true)
    }

    pub fn append_pty_chunk(&self, session_id: &str, chunk: &[u8]) -> Result<usize> {
        self.append_session_log(session_id, chunk, false)
    }

    fn append_session_log(
        &self,
        session_id: &str,
        chunk: &[u8],
        transcript: bool,
    ) -> Result<usize> {
        let session = self
            .get_session(session_id)?
            .with_context(|| format!("missing runtime session {session_id}"))?;
        let target = if transcript {
            self.paths.sessions_dir.join(&session.transcript_rel_path)
        } else {
            self.paths.sessions_dir.join(&session.pty_rel_path)
        };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating runtime log dir {}", parent.display()))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target)
            .with_context(|| format!("opening runtime log {}", target.display()))?;
        file.write_all(chunk)
            .with_context(|| format!("writing runtime log {}", target.display()))?;
        Ok(chunk.len())
    }

    fn insert_event(
        &self,
        session_id: &str,
        event_kind: &str,
        from_state: Option<RemoteRuntimeSessionState>,
        to_state: Option<RemoteRuntimeSessionState>,
        detail: Option<&str>,
        payload: &Value,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runtime_events (
                event_id, session_id, event_kind, from_state, to_state, detail, at, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                Uuid::new_v4().to_string(),
                session_id,
                event_kind,
                from_state.map(|value| value.as_db().to_string()),
                to_state.map(|value| value.as_db().to_string()),
                detail,
                now_rfc3339()?,
                serde_json::to_string(payload)?,
            ],
        )?;
        Ok(())
    }
}

fn read_runtime_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteRuntimeSession> {
    let writer_lease_id = row.get::<_, Option<String>>(12)?;
    let writer_client_id = row.get::<_, Option<String>>(13)?;
    let writer_lease_acquired_at = row.get::<_, Option<String>>(14)?;
    let writer_lease_expires_at = row.get::<_, Option<String>>(15)?;
    let writer_lease = match (
        writer_lease_id,
        writer_client_id,
        writer_lease_acquired_at,
        writer_lease_expires_at,
    ) {
        (Some(lease_id), Some(client_id), Some(acquired_at), Some(expires_at)) => {
            Some(RemoteWriterLease {
                lease_id,
                client_id,
                acquired_at,
                expires_at,
            })
        }
        _ => None,
    };
    Ok(RemoteRuntimeSession {
        session_id: row.get(0)?,
        machine_key: row.get(1)?,
        runtime_kind: RemoteRuntimeKind::from_db(&row.get::<_, String>(2)?),
        state: RemoteRuntimeSessionState::from_db(&row.get::<_, String>(3)?),
        health: RemoteRuntimeHealth::from_db(&row.get::<_, String>(4)?),
        title: row.get(5)?,
        cwd: row.get(6)?,
        summary: row.get(7)?,
        requires_terminal: row.get::<_, i64>(8)? != 0,
        interactive_ready: row.get::<_, i64>(9)? != 0,
        transcript_rel_path: row.get(10)?,
        pty_rel_path: row.get(11)?,
        writer_lease,
        last_error: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        last_transition_at: row.get(19)?,
    })
}

fn runtime_rel_path(root: &Path, full: &Path) -> String {
    full.strip_prefix(root)
        .unwrap_or(full)
        .to_string_lossy()
        .to_string()
}

fn format_time(value: OffsetDateTime) -> Result<String> {
    value
        .format(&Rfc3339)
        .context("formatting runtime timestamp")
}

fn now_rfc3339() -> Result<String> {
    format_time(OffsetDateTime::now_utc())
}

fn bool_to_sql(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn duration_to_time(value: Duration) -> TimeDuration {
    TimeDuration::seconds(i64::try_from(value.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> RemoteRuntimeRegistry {
        let root = std::env::temp_dir().join(format!("yggterm-runtime-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create temp runtime root");
        RemoteRuntimeRegistry::open(&root).expect("open runtime registry")
    }

    #[test]
    fn register_session_creates_layout_and_default_state() {
        let registry = test_registry();
        let session = registry
            .register_session(RemoteRuntimeSessionInput {
                session_id: Some("session-a".to_string()),
                machine_key: "jojo".to_string(),
                runtime_kind: RemoteRuntimeKind::Codex,
                title: "But Wait Litelm Stack Come".to_string(),
                cwd: Some("/home/pi".to_string()),
                summary: Some("resume target".to_string()),
                requires_terminal: true,
            })
            .expect("register session");

        assert_eq!(session.state, RemoteRuntimeSessionState::Created);
        assert_eq!(session.health, RemoteRuntimeHealth::Unknown);
        assert!(registry.paths().db_path.exists());
        assert!(
            registry
                .paths()
                .sessions_dir
                .join("session-a")
                .join(TRANSCRIPT_LOG_FILENAME)
                .parent()
                .expect("transcript parent exists")
                .exists()
        );
    }

    #[test]
    fn transition_session_persists_fsm_and_event_log() {
        let registry = test_registry();
        registry
            .register_session(RemoteRuntimeSessionInput {
                session_id: Some("session-b".to_string()),
                machine_key: "jojo".to_string(),
                runtime_kind: RemoteRuntimeKind::Codex,
                title: "Resume".to_string(),
                cwd: None,
                summary: None,
                requires_terminal: true,
            })
            .expect("register session");

        let session = registry
            .transition_session(
                "session-b",
                RemoteRuntimeSessionState::Interactive,
                Some("attached"),
                &json!({ "transport": "ssh" }),
            )
            .expect("transition session");
        let events = registry.list_events("session-b", 10).expect("list events");

        assert_eq!(session.state, RemoteRuntimeSessionState::Interactive);
        assert_eq!(session.health, RemoteRuntimeHealth::Healthy);
        assert!(session.interactive_ready);
        assert_eq!(events[0].event_kind, "state_transition");
        assert_eq!(
            events[0].to_state,
            Some(RemoteRuntimeSessionState::Interactive)
        );
    }

    #[test]
    fn writer_lease_is_single_writer_until_release() {
        let registry = test_registry();
        registry
            .register_session(RemoteRuntimeSessionInput {
                session_id: Some("session-c".to_string()),
                machine_key: "jojo".to_string(),
                runtime_kind: RemoteRuntimeKind::Shell,
                title: "Shell".to_string(),
                cwd: None,
                summary: None,
                requires_terminal: true,
            })
            .expect("register session");

        let lease = registry
            .acquire_writer_lease("session-c", "client-a", Duration::from_secs(30))
            .expect("acquire first lease");
        let second =
            registry.acquire_writer_lease("session-c", "client-b", Duration::from_secs(30));
        assert!(second.is_err());
        assert!(
            registry
                .release_writer_lease("session-c", &lease.lease_id)
                .expect("release lease")
        );
        let renewed = registry
            .acquire_writer_lease("session-c", "client-b", Duration::from_secs(30))
            .expect("acquire second lease");
        assert_eq!(renewed.client_id, "client-b");
    }

    #[test]
    fn append_logs_write_append_only_files() {
        let registry = test_registry();
        registry
            .register_session(RemoteRuntimeSessionInput {
                session_id: Some("session-d".to_string()),
                machine_key: "jojo".to_string(),
                runtime_kind: RemoteRuntimeKind::Codex,
                title: "Logs".to_string(),
                cwd: None,
                summary: None,
                requires_terminal: true,
            })
            .expect("register session");
        registry
            .append_transcript_chunk("session-d", b"hello")
            .expect("append transcript");
        registry
            .append_pty_chunk("session-d", b"world")
            .expect("append pty");
        let transcript = fs::read_to_string(
            registry
                .paths()
                .sessions_dir
                .join("session-d")
                .join(TRANSCRIPT_LOG_FILENAME),
        )
        .expect("read transcript");
        let pty = fs::read_to_string(
            registry
                .paths()
                .sessions_dir
                .join("session-d")
                .join(PTY_LOG_FILENAME),
        )
        .expect("read pty");
        assert_eq!(transcript, "hello");
        assert_eq!(pty, "world");
    }
}
