use crate::AppSettings;
use anyhow::{Context, Result};
use reqwest::blocking::Client;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::time::Duration;
use time::OffsetDateTime;

const TITLE_DB_FILENAME: &str = "session-titles.db";
const MAX_GENERATIONS_PER_LOAD: usize = 4;

pub struct SessionTitleStore {
    conn: Connection,
}

pub struct SessionTitleResolver {
    store: SessionTitleStore,
    settings: AppSettings,
    remaining_budget: usize,
}

impl SessionTitleStore {
    pub fn open(home: &Path) -> Result<Self> {
        let db_path = home.join(TITLE_DB_FILENAME);
        let conn = Connection::open(&db_path)
            .with_context(|| format!("failed to open title db {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_titles (
                session_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                cwd TEXT,
                source TEXT,
                model TEXT,
                updated_at TEXT NOT NULL
            );",
        )
        .context("failed to initialize title db schema")?;
        Ok(Self { conn })
    }

    pub fn get_title(&self, session_id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT title FROM session_titles WHERE session_id = ?1")?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn put_title(
        &self,
        session_id: &str,
        cwd: &str,
        title: &str,
        model: &str,
        source: &str,
    ) -> Result<()> {
        let updated_at = OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
        self.conn.execute(
            "INSERT INTO session_titles (session_id, title, cwd, source, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
               title = excluded.title,
               cwd = excluded.cwd,
               source = excluded.source,
               model = excluded.model,
               updated_at = excluded.updated_at",
            params![session_id, title, cwd, source, model, updated_at],
        )?;
        Ok(())
    }
}

impl SessionTitleResolver {
    pub fn new(home: &Path, settings: &AppSettings) -> Result<Self> {
        Ok(Self {
            store: SessionTitleStore::open(home)?,
            settings: settings.clone(),
            remaining_budget: MAX_GENERATIONS_PER_LOAD,
        })
    }

    pub fn resolve_for_session(
        &mut self,
        session_id: &str,
        cwd: &str,
        file_path: &Path,
    ) -> Result<Option<String>> {
        if let Some(title) = self.store.get_title(session_id)? {
            return Ok(Some(title));
        }

        if self.remaining_budget == 0 || !self.settings_ready() {
            return Ok(None);
        }

        let context = extract_tail_context(file_path)?;
        if context.is_empty() {
            return Ok(None);
        }

        let title = request_litellm_title(&self.settings, &context)?;
        let Some(title) = sanitize_generated_title(&title) else {
            return Ok(None);
        };

        self.store.put_title(
            session_id,
            cwd,
            &title,
            &self.settings.interface_llm_model,
            "litellm",
        )?;
        self.remaining_budget = self.remaining_budget.saturating_sub(1);
        Ok(Some(title))
    }

    fn settings_ready(&self) -> bool {
        !self.settings.litellm_endpoint.trim().is_empty()
            && !self.settings.litellm_api_key.trim().is_empty()
            && !self.settings.interface_llm_model.trim().is_empty()
    }
}

fn extract_tail_context(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read session transcript {}", path.display()))?;
    let mut snippets = Vec::<String>::new();

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let event_type = value.get("type").and_then(Value::as_str);
        match event_type {
            Some("response_item") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                let role = payload
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("assistant");
                let lines = message_lines_from_payload(payload);
                if lines.is_empty() {
                    continue;
                }
                snippets.push(format!("{}: {}", role.to_uppercase(), lines.join(" ")));
            }
            Some("event_msg") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) != Some("user_message") {
                    continue;
                }
                let Some(text) = payload.get("message").and_then(Value::as_str) else {
                    continue;
                };
                let lines = normalize_preview_text(text);
                if lines.is_empty() {
                    continue;
                }
                snippets.push(format!("USER: {}", lines.join(" ")));
            }
            _ => {}
        }
    }

    let tail = snippets
        .into_iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    Ok(tail.join("\n"))
}

fn request_litellm_title(settings: &AppSettings, context: &str) -> Result<String> {
    let url = completions_url(&settings.litellm_endpoint);
    let client = Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .context("failed to build LiteLLM client")?;
    let body = serde_json::json!({
        "model": settings.interface_llm_model,
        "temperature": 0.2,
        "max_tokens": 24,
        "messages": [
            {
                "role": "system",
                "content": "Generate a short UI title for a terminal or Codex session. Return only the title, 2 to 6 words, no quotes, no markdown, no trailing punctuation."
            },
            {
                "role": "user",
                "content": format!("Create a concise session title from this recent context:\n\n{context}")
            }
        ]
    });

    let response = client
        .post(url)
        .bearer_auth(settings.litellm_api_key.trim())
        .json(&body)
        .send()
        .context("LiteLLM request failed")?
        .error_for_status()
        .context("LiteLLM returned an error status")?;

    let value: Value = response.json().context("failed to parse LiteLLM response")?;
    let title = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .context("LiteLLM response did not contain a title")?;
    Ok(title)
}

fn completions_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

fn sanitize_generated_title(raw: &str) -> Option<String> {
    let first_line = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let sanitized = first_line
        .trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`')
        .trim_end_matches('.')
        .trim();
    if sanitized.is_empty() {
        return None;
    }
    let shortened = sanitized.chars().take(72).collect::<String>();
    Some(shortened)
}

fn message_lines_from_payload(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(content_items) = payload.get("content").and_then(Value::as_array) {
        for item in content_items {
            if let Some(text) = item
                .get("text")
                .or_else(|| item.get("input_text"))
                .or_else(|| item.get("output_text"))
                .and_then(Value::as_str)
            {
                lines.extend(normalize_preview_text(text));
            }
        }
    }
    lines
}

fn normalize_preview_text(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
