use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TERMINAL_TELEMETRY_DIRNAME: &str = "telemetry";
pub const TERMINAL_TELEMETRY_DB_FILENAME: &str = "terminal.sqlite3";

#[derive(Debug, Clone)]
pub struct TerminalTelemetryEvent {
    pub source: String,
    pub category: String,
    pub name: String,
    pub severity: String,
    pub session_path: Option<String>,
    pub runtime_key: Option<String>,
    pub host_id: Option<String>,
    pub gui_pid: Option<u32>,
    pub daemon_pid: Option<u32>,
    pub server_version: Option<String>,
    pub reason: Option<String>,
    pub payload: Value,
}

impl TerminalTelemetryEvent {
    pub fn new(
        source: impl Into<String>,
        category: impl Into<String>,
        name: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            source: source.into(),
            category: category.into(),
            name: name.into(),
            severity: "info".to_string(),
            session_path: None,
            runtime_key: None,
            host_id: None,
            gui_pid: None,
            daemon_pid: None,
            server_version: None,
            reason: None,
            payload,
        }
    }

    pub fn severity(mut self, severity: impl Into<String>) -> Self {
        self.severity = severity.into();
        self
    }

    pub fn session_path(mut self, session_path: impl Into<String>) -> Self {
        self.session_path = Some(session_path.into());
        self
    }

    pub fn runtime_key(mut self, runtime_key: Option<impl Into<String>>) -> Self {
        self.runtime_key = runtime_key.map(Into::into);
        self
    }

    pub fn host_id(mut self, host_id: impl Into<String>) -> Self {
        self.host_id = Some(host_id.into());
        self
    }

    pub fn gui_pid(mut self, gui_pid: u32) -> Self {
        self.gui_pid = Some(gui_pid);
        self
    }

    pub fn daemon_pid(mut self, daemon_pid: Option<u32>) -> Self {
        self.daemon_pid = daemon_pid;
        self
    }

    pub fn server_version(mut self, server_version: impl Into<String>) -> Self {
        self.server_version = Some(server_version.into());
        self
    }

    pub fn reason(mut self, reason: Option<impl Into<String>>) -> Self {
        self.reason = reason.map(Into::into);
        self
    }
}

pub fn terminal_telemetry_db_path(home: &Path) -> PathBuf {
    home.join(TERMINAL_TELEMETRY_DIRNAME)
        .join(TERMINAL_TELEMETRY_DB_FILENAME)
}

pub fn ensure_terminal_telemetry_schema(home: &Path) -> Result<PathBuf> {
    let db_path = terminal_telemetry_db_path(home);
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create telemetry dir {}", parent.display()))?;
    }
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open telemetry db {}", db_path.display()))?;
    configure_terminal_telemetry_connection(&conn)?;
    Ok(db_path)
}

pub fn append_terminal_telemetry_event(home: &Path, event: &TerminalTelemetryEvent) -> Result<()> {
    let db_path = terminal_telemetry_db_path(home);
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create telemetry dir {}", parent.display()))?;
    }
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open telemetry db {}", db_path.display()))?;
    configure_terminal_telemetry_connection(&conn)?;
    let payload_json =
        serde_json::to_string(&event.payload).context("failed to serialize telemetry payload")?;
    conn.execute(
        "INSERT INTO terminal_events (
            ts_ms,
            pid,
            source,
            category,
            name,
            severity,
            session_path,
            runtime_key,
            host_id,
            gui_pid,
            daemon_pid,
            server_version,
            reason,
            payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            now_ms_i64(),
            std::process::id() as i64,
            event.source.as_str(),
            event.category.as_str(),
            event.name.as_str(),
            event.severity.as_str(),
            event.session_path.as_deref(),
            event.runtime_key.as_deref(),
            event.host_id.as_deref(),
            event.gui_pid.map(|pid| pid as i64),
            event.daemon_pid.map(|pid| pid as i64),
            event.server_version.as_deref(),
            event.reason.as_deref(),
            payload_json,
        ],
    )
    .context("failed to insert terminal telemetry event")?;
    Ok(())
}

pub fn spawn_terminal_telemetry_event(home: PathBuf, event: TerminalTelemetryEvent) {
    let _ = std::thread::Builder::new()
        .name("yggterm-terminal-telemetry".to_string())
        .spawn(move || {
            let _ = append_terminal_telemetry_event(&home, &event);
        });
}

fn configure_terminal_telemetry_connection(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("failed to enable telemetry WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("failed to set telemetry synchronous mode")?;
    conn.busy_timeout(std::time::Duration::from_millis(250))
        .context("failed to set telemetry busy timeout")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS terminal_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts_ms INTEGER NOT NULL,
            pid INTEGER NOT NULL,
            source TEXT NOT NULL,
            category TEXT NOT NULL,
            name TEXT NOT NULL,
            severity TEXT NOT NULL DEFAULT 'info',
            session_path TEXT,
            runtime_key TEXT,
            host_id TEXT,
            gui_pid INTEGER,
            daemon_pid INTEGER,
            server_version TEXT,
            reason TEXT,
            payload_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_terminal_events_ts
            ON terminal_events(ts_ms);
        CREATE INDEX IF NOT EXISTS idx_terminal_events_session_ts
            ON terminal_events(session_path, ts_ms);
        CREATE INDEX IF NOT EXISTS idx_terminal_events_category_name_ts
            ON terminal_events(category, name, ts_ms);
        CREATE INDEX IF NOT EXISTS idx_terminal_events_severity_ts
            ON terminal_events(severity, ts_ms);
        "#,
    )
    .context("failed to create terminal telemetry schema")?;
    Ok(())
}

fn now_ms_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yggterm-terminal-telemetry-{}-{}-{}",
            label,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn terminal_telemetry_schema_creates_sqlite_db_under_telemetry_dir() {
        let dir = temp_test_dir("schema");
        let path = ensure_terminal_telemetry_schema(&dir).expect("create schema");

        assert_eq!(
            path,
            dir.join(TERMINAL_TELEMETRY_DIRNAME)
                .join(TERMINAL_TELEMETRY_DB_FILENAME)
        );
        assert!(path.exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn terminal_telemetry_append_records_terminal_event() {
        let dir = temp_test_dir("append");
        let event = TerminalTelemetryEvent::new(
            "gui",
            "terminal_open_attempt",
            "recovering",
            serde_json::json!({ "surface_problem": "active terminal host exists but xterm surface is empty" }),
        )
        .severity("warn")
        .session_path("remote-session://dev/test-session")
        .host_id("yggterm-terminal-test")
        .reason(Some("active terminal host exists but xterm surface is empty"));

        append_terminal_telemetry_event(&dir, &event).expect("append telemetry event");

        let conn =
            Connection::open(terminal_telemetry_db_path(&dir)).expect("open telemetry db for read");
        let (count, severity, reason): (i64, String, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(severity), MAX(reason) FROM terminal_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("query event");
        assert_eq!(count, 1);
        assert_eq!(severity, "warn");
        assert_eq!(
            reason,
            "active terminal host exists but xterm surface is empty"
        );

        let _ = fs::remove_dir_all(dir);
    }
}
