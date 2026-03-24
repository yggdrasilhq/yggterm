use crate::{AppSettings, read_codex_transcript_messages};
use anyhow::{Context, Result};
use reqwest::blocking::Client;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use time::OffsetDateTime;
use tracing::{info, warn};

const TITLE_DB_FILENAME: &str = "session-titles.db";

pub struct SessionTitleStore {
    conn: Connection,
}

pub struct SessionTitleResolver {
    store: SessionTitleStore,
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
            );
            CREATE TABLE IF NOT EXISTS session_precis (
                session_id TEXT PRIMARY KEY,
                precis TEXT NOT NULL,
                cwd TEXT,
                source TEXT,
                model TEXT,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS session_summaries (
                session_id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
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

    pub fn get_precis(&self, session_id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT precis FROM session_precis WHERE session_id = ?1")?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_summary(&self, session_id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT summary FROM session_summaries WHERE session_id = ?1")?;
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
        let updated_at =
            OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
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

    pub fn put_precis(
        &self,
        session_id: &str,
        cwd: &str,
        precis: &str,
        model: &str,
        source: &str,
    ) -> Result<()> {
        let updated_at =
            OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
        self.conn.execute(
            "INSERT INTO session_precis (session_id, precis, cwd, source, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
               precis = excluded.precis,
               cwd = excluded.cwd,
               source = excluded.source,
               model = excluded.model,
               updated_at = excluded.updated_at",
            params![session_id, precis, cwd, source, model, updated_at],
        )?;
        Ok(())
    }

    pub fn put_summary(
        &self,
        session_id: &str,
        cwd: &str,
        summary: &str,
        model: &str,
        source: &str,
    ) -> Result<()> {
        let updated_at =
            OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
        self.conn.execute(
            "INSERT INTO session_summaries (session_id, summary, cwd, source, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
               summary = excluded.summary,
               cwd = excluded.cwd,
               source = excluded.source,
               model = excluded.model,
               updated_at = excluded.updated_at",
            params![session_id, summary, cwd, source, model, updated_at],
        )?;
        Ok(())
    }

    pub fn delete_title(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM session_titles WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn delete_precis(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM session_precis WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn delete_summary(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM session_summaries WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }
}

impl SessionTitleResolver {
    pub fn new(home: &Path) -> Result<Self> {
        Ok(Self {
            store: SessionTitleStore::open(home)?,
        })
    }

    pub fn resolve_for_session(&self, session_id: &str) -> Result<Option<String>> {
        self.store.get_title(session_id)
    }

    pub fn resolve_precis_for_session(&self, session_id: &str) -> Result<Option<String>> {
        self.store.get_precis(session_id)
    }

    pub fn resolve_summary_for_session(&self, session_id: &str) -> Result<Option<String>> {
        self.store.get_summary(session_id)
    }

    pub fn generate_for_context(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        context: &str,
        force: bool,
    ) -> Result<Option<String>> {
        info!(session_id, force, context_chars = context.len(), "resolving context title");
        if !force {
            if let Some(title) = self.store.get_title(session_id)? {
                if !looks_like_generated_fallback_title(&title) {
                    return Ok(Some(title));
                }
                let _ = self.store.delete_title(session_id);
            }
        } else {
            let _ = self.store.delete_title(session_id);
        }

        if context.trim().is_empty() {
            warn!(session_id, "no context supplied for title generation");
            return Ok(None);
        }

        if !settings_ready(settings) {
            if let Some(title) = heuristic_title_from_context(context) {
                self.store
                    .put_title(session_id, cwd, &title, "heuristic", "heuristic")?;
                return Ok(Some(title));
            }
            warn!(session_id, "title settings are not configured");
            return Ok(None);
        }

        let title = request_litellm_title(settings, context)?;
        let Some(title) = sanitize_generated_title(&title) else {
            warn!(session_id, "model response sanitized to empty title");
            return Ok(None);
        };
        self.store.put_title(
            session_id,
            cwd,
            &title,
            &settings.interface_llm_model,
            "litellm",
        )?;
        Ok(Some(title))
    }

    pub fn generate_for_session(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        file_path: &Path,
        force: bool,
    ) -> Result<Option<String>> {
        info!(session_id, force, file_path=%file_path.display(), "resolving session title");
        if !force {
            if let Some(title) = self.store.get_title(session_id)? {
                if !looks_like_generated_fallback_title(&title) {
                    info!(session_id, "using cached session title");
                    return Ok(Some(title));
                }
                let _ = self.store.delete_title(session_id);
            }
        } else {
            let _ = self.store.delete_title(session_id);
        }

        if let Some(title) = self.store.get_title(session_id)? {
            if !looks_like_generated_fallback_title(&title) {
                return Ok(Some(title));
            }
            let _ = self.store.delete_title(session_id);
        }

        let context = extract_tail_context(file_path)?;
        if context.is_empty() {
            warn!(session_id, file_path=%file_path.display(), "no transcript context extracted for title generation");
            return Ok(None);
        }
        if !settings_ready(settings) {
            if let Some(title) = heuristic_title_from_context(&context) {
                self.store
                    .put_title(session_id, cwd, &title, "heuristic", "heuristic")?;
                return Ok(Some(title));
            }
            warn!(session_id, "title settings are not configured");
            return Ok(None);
        }
        info!(
            session_id,
            context_chars = context.len(),
            "requesting title from litellm"
        );

        let title = request_litellm_title(settings, &context)?;
        let Some(title) = sanitize_generated_title(&title) else {
            warn!(session_id, "model response sanitized to empty title");
            return Ok(None);
        };

        self.store.put_title(
            session_id,
            cwd,
            &title,
            &settings.interface_llm_model,
            "litellm",
        )?;
        Ok(Some(title))
    }

    pub fn generate_precis_for_session(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        file_path: &Path,
        force: bool,
    ) -> Result<Option<String>> {
        if !force {
            if let Some(precis) = self.store.get_precis(session_id)? {
                return Ok(Some(precis));
            }
        } else {
            let _ = self.store.delete_precis(session_id);
        }

        let context = extract_tail_context(file_path)?;
        if context.is_empty() {
            return Ok(None);
        }
        if !settings_ready(settings) {
            if let Some(precis) = heuristic_precis_from_context(&context) {
                self.store
                    .put_precis(session_id, cwd, &precis, "heuristic", "heuristic")?;
                return Ok(Some(precis));
            }
            return Ok(None);
        }
        let precis = request_litellm_precis(settings, &context)?;
        let Some(precis) = sanitize_generated_precis(&precis) else {
            return Ok(None);
        };
        self.store.put_precis(
            session_id,
            cwd,
            &precis,
            &settings.interface_llm_model,
            "litellm",
        )?;
        Ok(Some(precis))
    }

    pub fn generate_precis_for_context(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        context: &str,
        force: bool,
    ) -> Result<Option<String>> {
        if !force {
            if let Some(precis) = self.store.get_precis(session_id)? {
                return Ok(Some(precis));
            }
        } else {
            let _ = self.store.delete_precis(session_id);
        }

        if context.trim().is_empty() {
            return Ok(None);
        }
        if !settings_ready(settings) {
            if let Some(precis) = heuristic_precis_from_context(context) {
                self.store
                    .put_precis(session_id, cwd, &precis, "heuristic", "heuristic")?;
                return Ok(Some(precis));
            }
            return Ok(None);
        }

        let precis = request_litellm_precis(settings, context)?;
        let Some(precis) = sanitize_generated_precis(&precis) else {
            return Ok(None);
        };
        self.store.put_precis(
            session_id,
            cwd,
            &precis,
            &settings.interface_llm_model,
            "litellm",
        )?;
        Ok(Some(precis))
    }

    pub fn generate_summary_for_session(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        file_path: &Path,
        force: bool,
    ) -> Result<Option<String>> {
        if !force {
            if let Some(summary) = self.store.get_summary(session_id)? {
                return Ok(Some(summary));
            }
        } else {
            let _ = self.store.delete_summary(session_id);
        }

        let context = extract_tail_context(file_path)?;
        if context.is_empty() {
            return Ok(None);
        }
        if !settings_ready(settings) {
            if let Some(summary) = heuristic_summary_from_context(&context) {
                self.store
                    .put_summary(session_id, cwd, &summary, "heuristic", "heuristic")?;
                return Ok(Some(summary));
            }
            return Ok(None);
        }
        let summary = request_litellm_summary(settings, &context)?;
        let Some(summary) = sanitize_generated_summary(&summary) else {
            return Ok(None);
        };
        self.store.put_summary(
            session_id,
            cwd,
            &summary,
            &settings.interface_llm_model,
            "litellm",
        )?;
        Ok(Some(summary))
    }

    pub fn generate_summary_for_context(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        context: &str,
        force: bool,
    ) -> Result<Option<String>> {
        if !force {
            if let Some(summary) = self.store.get_summary(session_id)? {
                return Ok(Some(summary));
            }
        } else {
            let _ = self.store.delete_summary(session_id);
        }

        if context.trim().is_empty() {
            return Ok(None);
        }
        if !settings_ready(settings) {
            if let Some(summary) = heuristic_summary_from_context(context) {
                self.store
                    .put_summary(session_id, cwd, &summary, "heuristic", "heuristic")?;
                return Ok(Some(summary));
            }
            return Ok(None);
        }

        let summary = request_litellm_summary(settings, context)?;
        let Some(summary) = sanitize_generated_summary(&summary) else {
            return Ok(None);
        };
        self.store.put_summary(
            session_id,
            cwd,
            &summary,
            &settings.interface_llm_model,
            "litellm",
        )?;
        Ok(Some(summary))
    }
}

pub fn settings_ready(settings: &AppSettings) -> bool {
    !settings.litellm_endpoint.trim().is_empty()
        && !settings.litellm_api_key.trim().is_empty()
        && !settings.interface_llm_model.trim().is_empty()
}

fn extract_tail_context(path: &Path) -> Result<String> {
    let mut snippets = Vec::<String>::new();
    for message in read_codex_transcript_messages(path)? {
        snippets.push(format!(
            "{}: {}",
            message.role.display_label(),
            message.lines.join(" ")
        ));
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
        "max_tokens": 64,
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

    let response = match client
        .post(url)
        .bearer_auth(settings.litellm_api_key.trim())
        .json(&body)
        .send()
    {
        Ok(response) => match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                if let Some(title) = heuristic_title_from_context(context) {
                    return Ok(title);
                }
                return Err(error).context("LiteLLM returned an error status");
            }
        },
        Err(error) => {
            if let Some(title) = heuristic_title_from_context(context) {
                return Ok(title);
            }
            return Err(error).context("LiteLLM request failed");
        }
    };

    let value: Value = match response.json() {
        Ok(value) => value,
        Err(error) => {
            if let Some(title) = heuristic_title_from_context(context) {
                return Ok(title);
            }
            return Err(error).context("failed to parse LiteLLM response");
        }
    };
    let title = extract_completion_text(&value)
        .or_else(|| extract_reasoning_title(&value))
        .or_else(|| heuristic_title_from_context(context))
        .context("LiteLLM response did not contain a title")?;
    Ok(title)
}

fn request_litellm_precis(settings: &AppSettings, context: &str) -> Result<String> {
    let url = completions_url(&settings.litellm_endpoint);
    let client = Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .context("failed to build LiteLLM client")?;
    let body = serde_json::json!({
        "model": settings.interface_llm_model,
        "temperature": 0.2,
        "max_tokens": 96,
        "messages": [
            {
                "role": "system",
                "content": "Generate a short terminal-session precis for a desktop header. Return only one concise sentence or two very short sentences, no markdown, no bullets, no quotes."
            },
            {
                "role": "user",
                "content": format!("Create a concise UI precis from this recent session context:\n\n{context}")
            }
        ]
    });

    let response = match client
        .post(url)
        .bearer_auth(settings.litellm_api_key.trim())
        .json(&body)
        .send()
    {
        Ok(response) => match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                if let Some(precis) = heuristic_precis_from_context(context) {
                    return Ok(precis);
                }
                return Err(error).context("LiteLLM returned an error status");
            }
        },
        Err(error) => {
            if let Some(precis) = heuristic_precis_from_context(context) {
                return Ok(precis);
            }
            return Err(error).context("LiteLLM request failed");
        }
    };

    let value: Value = match response.json() {
        Ok(value) => value,
        Err(error) => {
            if let Some(precis) = heuristic_precis_from_context(context) {
                return Ok(precis);
            }
            return Err(error).context("failed to parse LiteLLM response");
        }
    };
    let precis = extract_completion_text(&value)
        .or_else(|| extract_reasoning_title(&value))
        .or_else(|| heuristic_precis_from_context(context))
        .context("LiteLLM response did not contain a precis")?;
    Ok(precis)
}

fn request_litellm_summary(settings: &AppSettings, context: &str) -> Result<String> {
    let url = completions_url(&settings.litellm_endpoint);
    let client = Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .context("failed to build LiteLLM client")?;
    let body = serde_json::json!({
        "model": settings.interface_llm_model,
        "temperature": 0.2,
        "max_tokens": 128,
        "messages": [
            {
                "role": "system",
                "content": "Generate a compact session summary for a desktop preview header. Return only 2 or 3 short sentences, no markdown, no bullets, no quotes. Focus on the current objective, active work, and likely next step."
            },
            {
                "role": "user",
                "content": format!("Create a concise preview summary from this recent session context:\n\n{context}")
            }
        ]
    });

    let response = match client
        .post(url)
        .bearer_auth(settings.litellm_api_key.trim())
        .json(&body)
        .send()
    {
        Ok(response) => match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                if let Some(summary) = heuristic_summary_from_context(context) {
                    return Ok(summary);
                }
                return Err(error).context("LiteLLM returned an error status");
            }
        },
        Err(error) => {
            if let Some(summary) = heuristic_summary_from_context(context) {
                return Ok(summary);
            }
            return Err(error).context("LiteLLM request failed");
        }
    };

    let value: Value = match response.json() {
        Ok(value) => value,
        Err(error) => {
            if let Some(summary) = heuristic_summary_from_context(context) {
                return Ok(summary);
            }
            return Err(error).context("failed to parse LiteLLM response");
        }
    };
    let summary = extract_completion_text(&value)
        .or_else(|| extract_reasoning_title(&value))
        .or_else(|| heuristic_summary_from_context(context))
        .context("LiteLLM response did not contain a summary")?;
    Ok(summary)
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

fn sanitize_generated_precis(raw: &str) -> Option<String> {
    let compact = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let sanitized = compact
        .trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`')
        .trim();
    if sanitized.is_empty() {
        return None;
    }
    Some(sanitized.chars().take(180).collect::<String>())
}

fn sanitize_generated_summary(raw: &str) -> Option<String> {
    let compact = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let sanitized = compact
        .trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`')
        .trim();
    if sanitized.is_empty() {
        return None;
    }
    Some(sanitized.chars().take(280).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::{
        extract_tail_context, looks_like_generated_fallback_title, sanitize_generated_summary,
    };
    use anyhow::Result;
    use std::fs;

    #[test]
    fn extract_tail_context_includes_compacted_replacement_history() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "yggterm-titles-compacted-{}-{}.jsonl",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(
            &path,
            [
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"compacted","payload":{"replacement_history":[{"role":"user","type":"message","content":[{"type":"input_text","text":"first prompt"}]},{"role":"assistant","type":"message","content":[{"type":"output_text","text":"first answer"}]}]}}"#,
                r#"{"timestamp":"2026-03-20T10:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"follow-up"}]}}"#,
            ]
            .join("\n"),
        )?;

        let context = extract_tail_context(&path)?;
        assert!(context.contains("USER: first prompt"));
        assert!(context.contains("ASSISTANT: first answer"));
        assert!(context.contains("USER: follow-up"));

        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn sanitize_generated_summary_compacts_lines() {
        let summary = sanitize_generated_summary("\"First line.\n\nSecond line.\"\n")
            .expect("summary");
        assert_eq!(summary, "First line. Second line.");
    }

    #[test]
    fn fallback_title_detection_matches_hash_titles() {
        assert!(looks_like_generated_fallback_title("Q4fc63d"));
        assert!(!looks_like_generated_fallback_title("Remove Them Entirely"));
    }
}

fn extract_completion_text(value: &Value) -> Option<String> {
    let choice = value.get("choices")?.as_array()?.first()?;
    let message = choice.get("message");

    message
        .and_then(|message| message.get("content"))
        .and_then(extract_text_fragment)
        .map(ToOwned::to_owned)
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            message
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(extract_text_fragment)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|text| !text.trim().is_empty())
        })
        .or_else(|| {
            message
                .and_then(|message| message.get("refusal"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .filter(|text| !text.trim().is_empty())
        })
        .or_else(|| {
            choice
                .get("text")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .filter(|text| !text.trim().is_empty())
        })
}

fn extract_reasoning_title(value: &Value) -> Option<String> {
    let choice = value.get("choices")?.as_array()?.first()?;
    let message = choice.get("message")?;
    let reasoning = message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .or_else(|| message.get("reasoning").and_then(Value::as_str))
        .or_else(|| {
            message
                .get("reasoning_details")
                .and_then(Value::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("text").and_then(Value::as_str))
                        .find(|text| !text.trim().is_empty())
                })
        })?;

    extract_quoted_candidate(reasoning)
}

fn extract_quoted_candidate(text: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let mut start = None;
        for (ix, ch) in text.char_indices() {
            if ch == quote {
                if let Some(open_ix) = start.take() {
                    let candidate = text[open_ix + ch.len_utf8()..ix].trim();
                    if let Some(title) = sanitize_generated_title(candidate) {
                        if plausible_title(&title) {
                            return Some(title);
                        }
                    }
                } else {
                    start = Some(ix);
                }
            }
        }
    }
    None
}

fn heuristic_title_from_context(context: &str) -> Option<String> {
    let line = context
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("USER: ") && !line.is_empty())
        .or_else(|| {
            context
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| !line.is_empty())
        })?;
    let normalized = line
        .strip_prefix("USER: ")
        .or_else(|| line.strip_prefix("ASSISTANT: "))
        .or_else(|| line.strip_prefix("MSG: "))
        .unwrap_or(line);

    let lower = normalized.to_ascii_lowercase();
    if lower.contains("shortcut") || lower.contains("shortcuts") {
        if let Some(quoted) = extract_quoted_candidate(normalized) {
            if quoted.split_whitespace().count() == 1 {
                let title = format!("{} Shortcuts", title_case_word(&quoted));
                if plausible_title(&title) {
                    return Some(title);
                }
            }
        }
        if lower.contains("excel") {
            return Some(String::from("Excel Shortcut Design"));
        }
        return Some(String::from("Shortcut Config Design"));
    }

    let words = normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .filter(|word| word.len() > 2)
        .filter(|word| {
            !matches!(
                word.to_ascii_lowercase().as_str(),
                "the"
                    | "and"
                    | "for"
                    | "with"
                    | "this"
                    | "that"
                    | "from"
                    | "into"
                    | "have"
                    | "will"
                    | "would"
                    | "should"
                    | "could"
                    | "about"
                    | "there"
                    | "their"
                    | "what"
                    | "when"
                    | "where"
                    | "which"
                    | "your"
                    | "want"
                    | "like"
                    | "just"
                    | "session"
            )
        })
        .take(5)
        .map(title_case_word)
        .collect::<Vec<_>>();

    if words.len() < 2 {
        return None;
    }

    let title = words.join(" ");
    plausible_title(&title).then_some(title)
}

fn looks_like_generated_fallback_title(title: &str) -> bool {
    let compact = title.trim();
    (compact.len() == 7 || compact.len() == 8)
        && compact.starts_with('Q')
        && compact
            .chars()
            .skip(1)
            .all(|ch| ch.is_ascii_hexdigit())
}

fn heuristic_copy_lines(context: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in context.lines().rev() {
        let line = raw
            .trim()
            .strip_prefix("USER: ")
            .or_else(|| raw.trim().strip_prefix("ASSISTANT: "))
            .or_else(|| raw.trim().strip_prefix("MSG: "))
            .unwrap_or(raw.trim())
            .trim();
        if line.is_empty() {
            continue;
        }
        let compact = line
            .replace('`', "")
            .replace(" - ", ", ")
            .replace("•", "")
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if compact.len() < 16 {
            continue;
        }
        if lines.iter().any(|existing| existing == &compact) {
            continue;
        }
        lines.push(compact);
        if lines.len() >= 3 {
            break;
        }
    }
    lines.reverse();
    lines
}

fn heuristic_precis_from_context(context: &str) -> Option<String> {
    let lines = heuristic_copy_lines(context);
    let first = lines.first()?;
    sanitize_generated_precis(first)
}

fn heuristic_summary_from_context(context: &str) -> Option<String> {
    let lines = heuristic_copy_lines(context);
    if lines.is_empty() {
        return None;
    }
    sanitize_generated_summary(&lines.join(" "))
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()
}

fn plausible_title(title: &str) -> bool {
    let word_count = title.split_whitespace().count();
    let lower = title.to_ascii_lowercase();
    (2..=6).contains(&word_count)
        && title.len() <= 72
        && !title.chars().any(|ch| ch.is_ascii_digit())
        && !matches!(
            lower.as_str(),
            "the user asks"
                | "user wants"
                | "we need"
                | "need generate"
                | "short ui title"
                | "terminal codex session"
                | "to words"
        )
}

fn extract_text_fragment(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("text").and_then(Value::as_str))
        .or_else(|| value.get("input_text").and_then(Value::as_str))
        .or_else(|| value.get("output_text").and_then(Value::as_str))
        .or_else(|| value.get("content").and_then(Value::as_str))
        .or_else(|| value.get("value").and_then(Value::as_str))
}
