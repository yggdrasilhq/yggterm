use crate::{
    AppSettings, generation_context_from_messages,
    transcript::read_codex_transcript_messages_tail_limited,
};
use anyhow::{Context, Result};
use reqwest::blocking::Client;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use time::{Duration as TimeDuration, OffsetDateTime};
use tracing::{info, warn};

const TITLE_DB_FILENAME: &str = "session-titles.db";

pub struct SessionTitleStore {
    conn: Connection,
}

pub struct SessionTitleResolver {
    store: SessionTitleStore,
}

#[derive(Debug, Clone)]
pub(crate) struct GeneratedCopyRecord {
    value: String,
    updated_at: OffsetDateTime,
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
        Ok(self
            .get_precis_record(session_id)?
            .map(|record| record.value))
    }

    pub(crate) fn get_precis_record(
        &self,
        session_id: &str,
    ) -> Result<Option<GeneratedCopyRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT precis, updated_at FROM session_precis WHERE session_id = ?1")?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            let updated_at = row.get::<_, String>(1)?;
            Ok(Some(GeneratedCopyRecord {
                value: row.get(0)?,
                updated_at: parse_copy_timestamp(&updated_at)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_summary(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .get_summary_record(session_id)?
            .map(|record| record.value))
    }

    pub(crate) fn get_summary_record(
        &self,
        session_id: &str,
    ) -> Result<Option<GeneratedCopyRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT summary, updated_at FROM session_summaries WHERE session_id = ?1")?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            let updated_at = row.get::<_, String>(1)?;
            Ok(Some(GeneratedCopyRecord {
                value: row.get(0)?,
                updated_at: parse_copy_timestamp(&updated_at)?,
            }))
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

    pub fn put_manual_title(&self, session_id: &str, cwd: &str, title: &str) -> Result<()> {
        self.put_title(session_id, cwd, title, "manual", "manual")
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

    pub fn save_manual_title_for_session(
        &self,
        session_id: &str,
        cwd: &str,
        title: &str,
    ) -> Result<()> {
        self.store.put_manual_title(session_id, cwd, title)
    }

    pub fn clear_title_for_session(&self, session_id: &str) -> Result<()> {
        self.store.delete_title(session_id)
    }

    pub fn resolve_precis_for_session(&self, session_id: &str) -> Result<Option<String>> {
        self.store.get_precis(session_id)
    }

    pub fn resolve_summary_for_session(&self, session_id: &str) -> Result<Option<String>> {
        self.store.get_summary(session_id)
    }

    pub fn precis_needs_refresh(
        &self,
        session_id: &str,
        source_updated_at: OffsetDateTime,
    ) -> Result<bool> {
        let Some(record) = self.store.get_precis_record(session_id)? else {
            return Ok(true);
        };
        Ok(looks_like_low_signal_generated_copy(&record.value)
            || source_updated_at - record.updated_at > TimeDuration::days(5))
    }

    pub fn summary_needs_refresh(
        &self,
        session_id: &str,
        source_updated_at: OffsetDateTime,
    ) -> Result<bool> {
        let Some(record) = self.store.get_summary_record(session_id)? else {
            return Ok(true);
        };
        Ok(looks_like_low_signal_generated_copy(&record.value)
            || source_updated_at - record.updated_at > TimeDuration::days(5))
    }

    pub fn generate_for_context(
        &self,
        settings: &AppSettings,
        session_id: &str,
        cwd: &str,
        context: &str,
        force: bool,
    ) -> Result<Option<String>> {
        info!(
            session_id,
            force,
            context_chars = context.len(),
            "resolving context title"
        );
        if !force {
            if let Some(title) = self.store.get_title(session_id)? {
                if !title_is_low_signal_for_cwd(&title, cwd) {
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
        let title = if title_is_low_signal_for_cwd(&title, cwd) {
            if let Some(heuristic) = heuristic_title_from_context(context) {
                if title_is_low_signal_for_cwd(&heuristic, cwd) {
                    warn!(session_id, generated_title=%title, "discarding low-signal generated title");
                    return Ok(None);
                }
                heuristic
            } else {
                warn!(session_id, generated_title=%title, "discarding low-signal generated title");
                return Ok(None);
            }
        } else {
            title
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
                if !title_is_low_signal_for_cwd(&title, cwd) {
                    info!(session_id, "using cached session title");
                    return Ok(Some(title));
                }
                let _ = self.store.delete_title(session_id);
            }
        } else {
            let _ = self.store.delete_title(session_id);
        }

        if let Some(title) = self.store.get_title(session_id)? {
            if !title_is_low_signal_for_cwd(&title, cwd) {
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
        let title = if looks_like_generated_fallback_title(&title) {
            if let Some(heuristic) = heuristic_title_from_context(&context) {
                if looks_like_generated_fallback_title(&heuristic) {
                    warn!(session_id, generated_title=%title, "discarding low-signal generated title");
                    return Ok(None);
                }
                heuristic
            } else {
                warn!(session_id, generated_title=%title, "discarding low-signal generated title");
                return Ok(None);
            }
        } else {
            title
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
                if !looks_like_low_signal_generated_copy(&precis) {
                    return Ok(Some(precis));
                }
                let _ = self.store.delete_precis(session_id);
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
        let precis = if looks_like_low_signal_generated_copy(&precis) {
            if let Some(heuristic) = heuristic_precis_from_context(&context) {
                if looks_like_low_signal_generated_copy(&heuristic) {
                    return Ok(None);
                }
                heuristic
            } else {
                return Ok(None);
            }
        } else {
            precis
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
                if !looks_like_low_signal_generated_copy(&precis) {
                    return Ok(Some(precis));
                }
                let _ = self.store.delete_precis(session_id);
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
        let precis = if looks_like_low_signal_generated_copy(&precis) {
            if let Some(heuristic) = heuristic_precis_from_context(context) {
                if looks_like_low_signal_generated_copy(&heuristic) {
                    return Ok(None);
                }
                heuristic
            } else {
                return Ok(None);
            }
        } else {
            precis
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
                if !looks_like_low_signal_generated_copy(&summary) {
                    return Ok(Some(summary));
                }
                let _ = self.store.delete_summary(session_id);
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
        let summary = if looks_like_low_signal_generated_copy(&summary) {
            if let Some(heuristic) = heuristic_summary_from_context(&context) {
                if looks_like_low_signal_generated_copy(&heuristic) {
                    return Ok(None);
                }
                heuristic
            } else {
                return Ok(None);
            }
        } else {
            summary
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
                if !looks_like_low_signal_generated_copy(&summary) {
                    return Ok(Some(summary));
                }
                let _ = self.store.delete_summary(session_id);
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
        let summary = if looks_like_low_signal_generated_copy(&summary) {
            if let Some(heuristic) = heuristic_summary_from_context(context) {
                if looks_like_low_signal_generated_copy(&heuristic) {
                    return Ok(None);
                }
                heuristic
            } else {
                return Ok(None);
            }
        } else {
            summary
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
    Ok(generation_context_from_messages(
        &read_codex_transcript_messages_tail_limited(path, 96)?,
    ))
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
                "content": "Generate a short, high-signal tab title for a long-running coding or terminal session. Infer the real job from the overall objective, the latest concrete progress, and the strongest user intent. Prefer the larger effort over temporary substeps like screenshot reading, launch notes, status checks, or one-off UI pokes. Use a specific engineering noun phrase, 2 to 6 words, no quotes, no markdown, no trailing punctuation. Good: 'Yggterm Titlebar Fix', 'Daemon Lifecycle Leak Audit', 'WezTerm APT Install'. Bad: 'Dev Sta', 'Fix Issue', 'Work Session', 'Debug UI', 'Need Help'."
            },
            {
                "role": "user",
                "content": format!("Create a concise session title from this structured session context.\nPrioritize: 1) the main user goal, 2) the active system/repo, and 3) the concrete engineering work happening now.\nIf the latest turns are screenshot inspection or modal polish inside a longer debugging effort, title the larger effort.\nDo not echo raw metadata, shell paths, or cute placeholder labels.\nReturn the title only.\n\n{context}")
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

fn parse_copy_timestamp(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .with_context(|| format!("invalid generated-copy timestamp: {value}"))
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
                "content": "Generate a short desktop-header precis for a long-running coding or terminal session. State the current task and the most important current progress in one or two crisp sentences. Prefer the overarching task over subordinate screenshot or image-inspection substeps. Ignore boilerplate, launch/bootstrap notes, policy text, and metadata scaffolding unless they are central to the work. Write like a strong engineer updating another engineer, not like a transcript. No markdown, no bullets, no quotes."
            },
            {
                "role": "user",
                "content": format!("Create a concise UI precis from this structured session context.\nFocus on what the operator is currently trying to achieve and what has already been established.\nIf there is a temporary screenshot or image-reading turn inside a longer workflow, do not center the precis on that substep.\nAvoid low-value scaffold copy like raw Target/Command metadata.\nGood precis example: 'Investigating the Yggterm memory leak and hardening the daemon lifecycle. Stale deleted-binary daemons were identified and cleanup plus stress coverage has been added.'\n\n{context}")
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
        "max_tokens": 220,
        "messages": [
            {
                "role": "system",
                "content": "Generate a concise but useful desktop session summary for a long-running coding or terminal session. Return one short paragraph of 3 to 4 sentences, plain prose only. Sentence 1: the main objective. Sentence 2: concrete verified progress or findings. Sentence 3: the current blocker, open issue, or likely next step. Optional sentence 4 only if it adds real signal. Prefer the overarching work over screenshot or image-inspection substeps. Ignore boilerplate, launch/bootstrap notes, policy text, and metadata scaffolding unless they are central to the work. Write like a strong engineering handoff, not a transcript recap."
            },
            {
                "role": "user",
                "content": format!("Create a concise preview summary from this structured session context.\nDo not summarize the instructions themselves unless they are the subject of the work.\nPrefer the real task, verified findings, and latest progress.\nIf the latest turns are just a screenshot or image check inside a larger task, keep the summary centered on the larger task.\nAvoid raw metadata or placeholder lines like Target/Command/Launch prepared.\nGood summary style: main objective first, then concrete result, then the next step or active blocker.\nDo not sound generic or childlike.\n\n{context}")
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
    if looks_like_low_signal_generated_copy(&summary) {
        if let Some(summary) = heuristic_summary_from_context(context) {
            return Ok(summary);
        }
    }
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
    let without_aux = strip_auxiliary_image_sentences(sanitized);
    let final_text = if without_aux.is_empty() {
        sanitized
    } else {
        without_aux.as_str()
    };
    Some(final_text.chars().take(180).collect::<String>())
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
    let sanitized_owned = strip_auxiliary_image_sentences(sanitized);
    let sanitized = if sanitized_owned.is_empty() {
        sanitized.to_string()
    } else {
        sanitized_owned
    };
    const MAX_SUMMARY_CHARS: usize = 560;
    let bounded = sanitized
        .chars()
        .take(MAX_SUMMARY_CHARS)
        .collect::<String>();
    if sanitized.chars().count() <= MAX_SUMMARY_CHARS {
        return Some(bounded);
    }
    if let Some(ix) = bounded.char_indices().rev().find_map(|(ix, ch)| {
        ((ch == '.' || ch == '!' || ch == '?') && ix >= 160).then_some(ix + ch.len_utf8())
    }) {
        return Some(bounded[..ix].trim().to_string());
    }
    if let Some(ix) = bounded
        .char_indices()
        .rev()
        .find_map(|(ix, ch)| ch.is_whitespace().then_some(ix))
    {
        return Some(format!("{}…", bounded[..ix].trim_end()));
    }
    Some(format!("{}…", bounded.trim_end()))
}

fn strip_auxiliary_image_sentences(text: &str) -> String {
    let kept = text
        .split_terminator(['.', '!', '?'])
        .map(str::trim)
        .filter(|sentence| {
            let lower = sentence.to_ascii_lowercase();
            !lower.contains("can you read this image")
                && !lower.contains("clipboard/clipboard-")
                && !lower.contains("@/home/")
                && !lower.contains("it's a screenshot of")
                && !lower.contains("it’s a screenshot of")
                && !lower.contains("i'm opening the image now")
                && !lower.contains("i’m opening the image now")
                && !lower.contains("extract the text or key contents")
                && !lower.contains("other visible ui details")
                && !lower.contains("the main visible text shows")
        })
        .collect::<Vec<_>>();
    if kept.is_empty() {
        String::new()
    } else {
        format!("{}.", kept.join(". "))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SessionTitleResolver, best_effort_precis_from_context, best_effort_summary_from_context,
        best_effort_title_from_context, extract_tail_context, heuristic_title_from_context,
        looks_like_generated_fallback_title, looks_like_low_signal_generated_title,
        sanitize_generated_summary,
    };
    use crate::AppSettings;
    use anyhow::Result;
    use std::fs;
    use std::path::PathBuf;

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
        let summary =
            sanitize_generated_summary("\"First line.\n\nSecond line.\"\n").expect("summary");
        assert_eq!(summary, "First line. Second line.");
    }

    #[test]
    fn low_signal_copy_rejects_ansi_shell_prompt_text() {
        assert!(crate::looks_like_low_signal_generated_copy(
            "\u{1b}[32;1mpi@jojo\u{1b}[m:\u{1b}[34;1m~\u{1b}[m$"
        ));
    }

    #[test]
    fn low_signal_copy_rejects_plain_shell_prompt_text() {
        assert!(crate::looks_like_low_signal_generated_copy(
            "pi@jojo:~/gh/yggterm$"
        ));
    }

    #[test]
    fn low_signal_copy_rejects_shell_command_script_text() {
        assert!(crate::looks_like_low_signal_generated_copy(
            "Printf '\\033[. 1049h\\033[. 25lhc'; sleep 1; printf '\\033[. 25h\\033[. 1049l."
        ));
    }

    #[test]
    fn fallback_title_detection_matches_hash_titles() {
        assert!(looks_like_generated_fallback_title("Q4fc63d"));
        assert!(looks_like_generated_fallback_title("25663dc"));
        assert!(looks_like_generated_fallback_title(
            "local::ddf8f1ee-8e64-4201-ab3a-2b07424f9b77"
        ));
        assert!(looks_like_generated_fallback_title(
            "document::ddf8f1ee-8e64-4201-ab3a-2b07424f9b77"
        ));
        assert!(looks_like_generated_fallback_title("local [ok] shell"));
        assert!(looks_like_generated_fallback_title(
            "Local Shell Stay Alive Daemon"
        ));
        assert!(looks_like_generated_fallback_title("Remote Codex 019cf82b"));
        assert!(looks_like_generated_fallback_title(
            "Remote Codex LiteLLM 019cf82b"
        ));
        assert!(!looks_like_generated_fallback_title("Remove Them Entirely"));
    }

    #[test]
    fn heuristic_title_uses_shell_prompt_command_context() {
        let context = [
            "pi@dev:~$ echo 'Live local shell title generation proof'",
            "Live local shell title generation proof",
            "pi@dev:~$",
        ]
        .join("\n");
        assert_eq!(
            heuristic_title_from_context(&context).as_deref(),
            Some("Live Title Generation")
        );
    }

    fn temp_title_home(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "yggterm-title-tests-{name}-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ))
    }

    #[test]
    fn manual_title_short_circuits_context_generation_until_forced() -> Result<()> {
        let home = temp_title_home("manual-title");
        fs::create_dir_all(&home)?;
        let resolver = SessionTitleResolver::new(&home)?;
        resolver.save_manual_title_for_session(
            "session-1",
            "/home/pi/gh/yggterm",
            "Patch To Upstream",
        )?;

        let cached = resolver.generate_for_context(
            &AppSettings::default(),
            "session-1",
            "/home/pi/gh/yggterm",
            "USER: fix daemon churn",
            false,
        )?;
        assert_eq!(cached.as_deref(), Some("Patch To Upstream"));

        let forced = resolver.generate_for_context(
            &AppSettings::default(),
            "session-1",
            "/home/pi/gh/yggterm",
            "USER: fix daemon churn",
            true,
        )?;
        assert_ne!(forced.as_deref(), Some("Patch To Upstream"));

        let _ = fs::remove_dir_all(home);
        Ok(())
    }

    #[test]
    fn best_effort_summary_ignores_target_command_scaffold() {
        let context = [
            "Target: /home/pi",
            "Command: /bin/bash",
            "Launch command prepared: exec '/bin/bash' -i",
            "Investigating the yggterm daemon leak on oc and tightening the cleanup path.",
            "Added lifecycle stress coverage and verified the deleted-binary daemon is reaped.",
        ]
        .join("\n");
        let summary = best_effort_summary_from_context(&context).expect("summary");
        assert!(summary.contains("Investigating the yggterm daemon leak"));
        assert!(!summary.contains("Target: /home/pi"));
        assert!(!summary.contains("Command: /bin/bash"));
    }

    #[test]
    fn best_effort_title_ignores_target_command_scaffold() {
        let context = [
            "Target: /home/pi",
            "Command: /bin/bash",
            "Working on the yggterm titlebar modal styling and harness regressions.",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_ne!(title, "Target");
        assert!(!looks_like_generated_fallback_title(&title));
    }

    #[test]
    fn best_effort_title_polishes_commandish_requests() {
        let context = [
            "USER: Please sudo apt install wezterm on this Debian box and verify the repo path.",
            "ASSISTANT: I will confirm the official install flow and then fix the apt source.",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_eq!(title, "Install WezTerm");
    }

    #[test]
    fn best_effort_title_prefers_goal_over_screenshot_substep() {
        let context = [
            "PRIMARY USER GOALS:",
            "- Fix the Yggterm titlebar modal styling and harden the smoke harness.",
            "",
            "RECENT SUBSTANTIVE TURNS:",
            "USER: The title/summary modal is still broken and the harness should catch it.",
            "ASSISTANT: I am checking the screenshot and the current focused titlebar state.",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_eq!(title, "Fix Yggterm Titlebar Modal Styling");
    }

    #[test]
    fn best_effort_copy_handles_short_substantive_question_context() {
        let context = ["RECENT SUBSTANTIVE TURNS:", "USER: Who are you?"].join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        let precis = best_effort_precis_from_context(&context).expect("precis");
        let summary = best_effort_summary_from_context(&context).expect("summary");
        assert_eq!(title, "Who Are You");
        assert!(precis.contains("Who are you"));
        assert!(summary.contains("Who are you"));
    }

    #[test]
    fn best_effort_summary_prefers_goal_and_progress_over_image_chatter() {
        let context = [
            "PRIMARY USER GOALS:",
            "- Investigate the yggterm daemon memory leak and harden lifecycle cleanup.",
            "",
            "RECENT SUBSTANTIVE TURNS:",
            "ASSISTANT: Added lifecycle stress coverage and proved deleted-binary daemons are reaped.",
            "USER: It’s a screenshot of the titlebar state.",
        ]
        .join("\n");
        let summary = best_effort_summary_from_context(&context).expect("summary");
        assert!(summary.contains("Investigate the yggterm daemon memory leak"));
        assert!(summary.contains("deleted-binary daemons are reaped"));
        assert!(!summary.to_ascii_lowercase().contains("screenshot"));
    }

    #[test]
    fn low_signal_title_detection_rejects_polite_command_titles() {
        assert!(looks_like_low_signal_generated_title(
            "Please Sudo Apt Install Wezterm"
        ));
        assert!(looks_like_low_signal_generated_title(
            "Prune You Want Next Can"
        ));
        assert!(looks_like_low_signal_generated_title(
            "Why You May Not Have"
        ));
        assert!(!looks_like_low_signal_generated_title("Install WezTerm"));
    }

    #[test]
    fn best_effort_summary_includes_objective_progress_and_remaining_work() {
        let context = [
            "PRIMARY USER GOALS:",
            "- Fix the Yggterm title/summary modal styling and harden the smoke harness.",
            "",
            "RECENT SUBSTANTIVE TURNS:",
            "ASSISTANT: The session-shell smoke now keys off the real search dropdown instead of the always-mounted search lane.",
            "ASSISTANT: Focused-state screenshot seam checks and button fill checks now pass on the live :10 client.",
            "ASSISTANT: Title and summary generation quality still needs a stricter fixture-based evaluation pass.",
        ]
        .join("\n");
        let summary = best_effort_summary_from_context(&context).expect("summary");
        assert!(summary.contains("Fix the Yggterm title/summary modal styling"));
        assert!(summary.contains("search dropdown"));
        assert!(summary.contains("fixture-based evaluation pass"));
    }

    #[test]
    fn best_effort_title_prefers_system_load_theme_over_assistant_chatter() {
        let context = [
            "RECENT SUBSTANTIVE TURNS:",
            "USER: Can you figure out what processes are making my laptop fan spin hard?",
            "ASSISTANT: I’ll check the current highest CPU and memory consumers right now.",
            "USER: okay and waht is eating ram and cpu in ssh manin",
            "ASSISTANT: In this snapshot, the RAM/CPU pressure is not from SSH itself.",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_eq!(title, "Diagnose Laptop Fan Load");
    }

    #[test]
    fn best_effort_title_prefers_theme_over_shell_noise() {
        let context = [
            "PRIMARY USER GOALS:",
            "- Can you check why manin is sluggishly slow now?",
            "",
            "RECENT SUBSTANTIVE TURNS:",
            "USER: jojo is this machine. manin is what I asked you for.",
            "USER: Can you check why manin is sluggishly slow now?",
            "ASSISTANT: I’ve identified two confirmed hogs and I’m taking one more disk-I/O sample.",
            "ASSISTANT: pi@manin:~$ ps -eo pid,%cpu,%mem,cmd --sort=-%cpu | head",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_eq!(title, "Investigate Manin Slowness");
    }

    #[test]
    fn best_effort_title_prefers_boundaries_decision_table_theme() {
        let context = [
            "RECENT SUBSTANTIVE TURNS:",
            "USER: Can we now resume the lack of boundaries discussion where we left off?",
            "ASSISTANT: I’m continuing the discussion in the same boundary and social-sorting thread.",
            "USER: Yes, I was about to ask about the decision making table you said earlier.",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_eq!(title, "Build Boundaries Decision Table");
    }

    #[test]
    fn best_effort_title_prefers_chat_export_theme_over_note_process_chatter() {
        let context = [
            "RECENT SUBSTANTIVE TURNS:",
            "USER: # instagram",
            "USER: I put a new chat export. It is the same chat with additional data and threads.",
            "ASSISTANT: Regenerated the chat notes from all exports so the latest file replaces the older version.",
            "USER: Re-read Cutting toxic ties and add new information where needed.",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_eq!(title, "Update Instagram Chat Notes");
    }

    #[test]
    fn best_effort_title_prefers_import_pipeline_theme_for_instagram_exports() {
        let context = [
            "RECENT SUBSTANTIVE TURNS:",
            "USER: I put a new chat export. It is the same chat with additional data and threads.",
            "ASSISTANT: Regenerated the chat notes from all exports so the latest file replaces the older version.",
            "USER: You need to fix the import script which should handle the problems. It is a merge conflict from importing the same file twice.",
            "ASSISTANT: Resolved the merge-conflict fallout and fixed the import pipeline so it won’t duplicate chats again.",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_eq!(title, "Fix Instagram Chat Import Pipeline");
    }

    #[test]
    fn best_effort_title_prefers_social_pruning_theme_over_edge_chatter() {
        let context = [
            "PRIMARY USER GOALS:",
            "- Can you use/update the git/harness or do computer use in any other way and actually do the instagram task that we agreed on.",
            "- Why should I treat the Instagram pruning different from facebook pruning? How should Avisankha be treated?",
            "",
            "RECENT SUBSTANTIVE TURNS:",
            "ASSISTANT: Instagram is mostly an attention edge, Facebook is more often an archive/access edge, and WhatsApp is a reciprocity edge.",
            "ASSISTANT: For Anik, reduce expectation and preserve low-cost access.",
            "ASSISTANT: For Avisankha, reduce surface area across channels because the tie itself is bad for your mind.",
        ]
        .join("\n");
        let title = best_effort_title_from_context(&context).expect("title");
        assert_eq!(title, "Design Social Pruning Rules");
    }

    #[test]
    fn generated_copy_quality_fixture_suite() {
        struct Fixture<'a> {
            name: &'a str,
            context: &'a str,
            title_keywords: &'a [&'a str],
            title_forbidden: &'a [&'a str],
            summary_keywords: &'a [&'a str],
            summary_forbidden: &'a [&'a str],
        }

        let fixtures = [
            Fixture {
                name: "wezterm install",
                context: "USER: Please sudo apt install wezterm on this Debian box and verify the repo path.\nASSISTANT: I will confirm the official install flow and then fix the apt source.",
                title_keywords: &["Install", "WezTerm"],
                title_forbidden: &["Please", "Sudo"],
                summary_keywords: &["install", "repo"],
                summary_forbidden: &["screenshot", "Target:"],
            },
            Fixture {
                name: "titlebar smoke",
                context: "PRIMARY USER GOALS:\n- Fix the Yggterm title/summary modal styling and harden the smoke harness.\n\nRECENT SUBSTANTIVE TURNS:\nASSISTANT: The session-shell smoke now keys off the real search dropdown instead of the always-mounted search lane.\nASSISTANT: Focused-state screenshot seam checks and button fill checks now pass on the live :10 client.\nASSISTANT: Title and summary generation quality still needs a stricter fixture-based evaluation pass.",
                title_keywords: &["Titlebar", "Modal"],
                title_forbidden: &["Screenshot", "Image"],
                summary_keywords: &["search dropdown", "seam checks", "evaluation pass"],
                summary_forbidden: &["image", "clipboard"],
            },
            Fixture {
                name: "daemon leak",
                context: "PRIMARY USER GOALS:\n- Investigate the yggterm daemon memory leak and harden lifecycle cleanup.\n\nRECENT SUBSTANTIVE TURNS:\nASSISTANT: Added lifecycle stress coverage and proved deleted-binary daemons are reaped.\nASSISTANT: The remaining work is measuring the base GUI/runtime RSS on the live desktop client.",
                title_keywords: &["Daemon", "Leak"],
                title_forbidden: &["Screenshot", "Work Session"],
                summary_keywords: &[
                    "lifecycle stress",
                    "deleted-binary daemons",
                    "remaining work",
                ],
                summary_forbidden: &["Target:", "Command:"],
            },
            Fixture {
                name: "fan load",
                context: "RECENT SUBSTANTIVE TURNS:\nUSER: Can you figure out what processes are making my laptop fan spin hard?\nASSISTANT: I’ll check the current highest CPU and memory consumers right now.\nUSER: okay and waht is eating ram and cpu in ssh manin\nASSISTANT: In this snapshot, the RAM/CPU pressure is not from SSH itself.",
                title_keywords: &["Diagnose", "Load"],
                title_forbidden: &["Asked", "You", "Snapshot"],
                summary_keywords: &["cpu", "ram", "ssh"],
                summary_forbidden: &["token_count", "approval policy"],
            },
            Fixture {
                name: "boundaries decision table",
                context: "RECENT SUBSTANTIVE TURNS:\nUSER: Can we now resume the lack of boundaries discussion where we left off?\nASSISTANT: I’m continuing the discussion in the same boundary and social-sorting thread.\nUSER: Yes, I was about to ask about the decision making table you said earlier.",
                title_keywords: &["Boundaries", "Decision", "Table"],
                title_forbidden: &["Describing", "Resuming", "Thread"],
                summary_keywords: &["boundaries", "decision table"],
                summary_forbidden: &["approval policy", "environment_context"],
            },
            Fixture {
                name: "instagram export notes",
                context: "RECENT SUBSTANTIVE TURNS:\nUSER: # instagram\nUSER: I put a new chat export. It is the same chat with additional data and threads.\nASSISTANT: Regenerated the chat notes from all exports so the latest file replaces the older version.\nUSER: Re-read Cutting toxic ties and add new information where needed.",
                title_keywords: &["Instagram", "Chat", "Notes"],
                title_forbidden: &["Regenerated", "Latest", "Version"],
                summary_keywords: &["chat export", "cutting toxic ties"],
                summary_forbidden: &["approval policy", "collaboration mode"],
            },
            Fixture {
                name: "instagram import pipeline",
                context: "RECENT SUBSTANTIVE TURNS:\nUSER: I put a new chat export. It is the same chat with additional data and threads.\nASSISTANT: Regenerated the chat notes from all exports so the latest file replaces the older version.\nUSER: You need to fix the import script which should handle the problems. It is a merge conflict from importing the same file twice.\nASSISTANT: Resolved the merge-conflict fallout and fixed the import pipeline so it won’t duplicate chats again.",
                title_keywords: &["Instagram", "Import", "Pipeline"],
                title_forbidden: &["Prune", "Want", "Next"],
                summary_keywords: &["chat export", "import pipeline", "duplicate"],
                summary_forbidden: &["approval policy", "collaboration mode"],
            },
            Fixture {
                name: "social edge pruning",
                context: "PRIMARY USER GOALS:\n- Can you use/update the git/harness or do computer use in any other way and actually do the instagram task that we agreed on.\n- Why should I treat the Instagram pruning different from facebook pruning? How should Avisankha be treated?\n\nRECENT SUBSTANTIVE TURNS:\nASSISTANT: Instagram is mostly an attention edge, Facebook is more often an archive/access edge, and WhatsApp is a reciprocity edge.\nASSISTANT: For Anik, reduce expectation and preserve low-cost access.\nASSISTANT: For Avisankha, reduce surface area across channels because the tie itself is bad for your mind.",
                title_keywords: &["Social", "Pruning", "Rules"],
                title_forbidden: &["Prune", "Want", "Next"],
                summary_keywords: &["instagram", "facebook", "whatsapp"],
                summary_forbidden: &["approval policy", "collaboration mode"],
            },
        ];

        let mut passed = 0usize;
        for fixture in &fixtures {
            let title = best_effort_title_from_context(fixture.context).expect("title");
            let summary = best_effort_summary_from_context(fixture.context).expect("summary");
            let title_ok = fixture
                .title_keywords
                .iter()
                .all(|needle| title.contains(needle))
                && fixture
                    .title_forbidden
                    .iter()
                    .all(|needle| !title.contains(needle));
            let summary_lower = summary.to_ascii_lowercase();
            let summary_ok = fixture
                .summary_keywords
                .iter()
                .all(|needle| summary_lower.contains(&needle.to_ascii_lowercase()))
                && fixture
                    .summary_forbidden
                    .iter()
                    .all(|needle| !summary_lower.contains(&needle.to_ascii_lowercase()));
            eprintln!(
                "[copy-eval] {} | title={} | summary={}",
                fixture.name, title, summary
            );
            assert!(title_ok, "title quality fixture failed: {}", fixture.name);
            assert!(
                summary_ok,
                "summary quality fixture failed: {}",
                fixture.name
            );
            passed += 1;
        }
        eprintln!("[copy-eval] passed {passed}/{}", fixtures.len());
        assert_eq!(passed, fixtures.len());
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

fn title_noise_word(word: &str) -> bool {
    matches!(
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
            | "please"
            | "sudo"
            | "repo"
            | "debian"
            | "ubuntu"
            | "need"
            | "needs"
            | "make"
            | "sure"
            | "using"
            | "through"
            | "session"
            | "path"
            | "paths"
            | "source"
            | "sources"
            | "line"
            | "lines"
            | "verify"
            | "check"
            | "box"
            | "host"
            | "machine"
            | "flow"
            | "official"
            | "local"
            | "remote"
            | "apt"
            | "deb"
            | "rpm"
            | "via"
            | "over"
            | "under"
            | "onto"
    )
}

fn action_title_from_line(normalized: &str) -> Option<String> {
    let lower = normalized.to_ascii_lowercase();
    let (ix, verb, max_subject_words) = [
        ("install", 2usize),
        ("update", 2usize),
        ("remove", 2usize),
        ("restore", 2usize),
        ("configure", 3usize),
        ("fix", 4usize),
        ("debug", 4usize),
        ("repair", 4usize),
        ("investigate", 4usize),
        ("review", 4usize),
        ("design", 4usize),
        ("polish", 4usize),
        ("refine", 4usize),
    ]
    .into_iter()
    .find_map(|(verb, max_subject_words)| {
        lower.find(verb).map(|ix| (ix, verb, max_subject_words))
    })?;
    let suffix = normalized[ix + verb.len()..].trim();
    let mut subject_words = suffix
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .filter(|word| word.len() > 2)
        .filter(|word| !title_noise_word(word))
        .map(title_case_word)
        .filter(|word| !word.is_empty())
        .take(max_subject_words)
        .collect::<Vec<_>>();
    if subject_words.len() >= 3 && subject_words[1] == "Title" && subject_words[2] == "Summary" {
        subject_words.splice(1..=2, [String::from("Titlebar")]);
    }
    if subject_words.len() >= 2 && subject_words[0] == "Title" && subject_words[1] == "Summary" {
        subject_words.splice(0..=1, [String::from("Titlebar")]);
    }
    if subject_words.is_empty() {
        return None;
    }
    let action = title_case_word(verb);
    let title = std::iter::once(action)
        .chain(subject_words)
        .collect::<Vec<_>>()
        .join(" ");
    plausible_title(&title).then_some(title)
}

fn title_is_low_signal_for_cwd(title: &str, cwd: &str) -> bool {
    let trimmed = title.trim();
    let cwd_trimmed = cwd.trim();
    trimmed.is_empty()
        || looks_like_generated_fallback_title(trimmed)
        || looks_like_low_signal_generated_title(trimmed)
        || (!cwd_trimmed.is_empty() && trimmed == cwd_trimmed)
}

fn looks_like_low_signal_generated_title(title: &str) -> bool {
    let trimmed = title.trim();
    let lower = trimmed.to_ascii_lowercase();
    let words = trimmed
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let low_signal_word_count = words
        .iter()
        .filter(|word| {
            title_noise_word(word)
                || matches!(
                    word.as_str(),
                    "you"
                        | "can"
                        | "may"
                        | "not"
                        | "asked"
                        | "again"
                        | "next"
                        | "also"
                        | "continue"
                        | "into"
                        | "now"
                        | "old"
                        | "side"
                        | "wrote"
                )
        })
        .count();
    lower.starts_with("please ")
        || lower.starts_with("can you ")
        || lower.starts_with("need to ")
        || lower.starts_with("help ")
        || lower.contains("asked you")
        || (words.len() >= 3 && low_signal_word_count * 2 >= words.len())
        || (trimmed.split_whitespace().count() > 5
            && ["sudo", "apt", "apt-get", "brew", "dnf", "yum"]
                .iter()
                .any(|needle| lower.contains(needle)))
}

fn line_is_generation_noise(lower: &str) -> bool {
    lower.starts_with("target: ")
        || lower.starts_with("command: ")
        || lower.starts_with("host: ")
        || lower.starts_with("prefix: ")
        || lower.starts_with("cwd: ")
        || lower.starts_with("launch: ")
        || lower.contains("launch command prepared")
        || lower.contains("this local shell should stay alive")
        || lower.contains("this local shell uses the same pty/runtime path")
        || lower.contains("open live terminal ")
        || lower.contains("can you read this image")
        || lower.contains("clipboard/clipboard-")
        || lower.contains("@/home/")
        || lower.contains("it’s a screenshot of")
        || lower.contains("it's a screenshot of")
        || lower.contains("i’m opening the image now")
        || lower.contains("i'm opening the image now")
        || lower.contains("extract the text or key contents")
        || lower.contains("the main visible text shows")
        || lower.contains("other visible ui details")
        || lower.contains("collaboration mode:")
        || lower.contains("current_date>")
        || lower.contains("environment_context")
}

#[derive(Clone, Debug)]
struct CopyCandidate {
    line: String,
    score: i32,
    is_progress: bool,
    is_blocker: bool,
    is_objective: bool,
}

fn line_has_progress_signal(lower: &str) -> bool {
    [
        "added ",
        "fixed ",
        "proved ",
        "verified ",
        "implemented ",
        "identified ",
        "restored ",
        "hardened ",
        "tightened ",
        "captured ",
        "reaped ",
        "killed ",
        "closed ",
        "passed ",
        "now ",
        "cpu-heavy",
        "ram-heavy",
        "pressure is not from ssh itself",
        "not from ssh itself",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn line_has_blocker_signal(lower: &str) -> bool {
    [
        "still ",
        "remaining ",
        "next step",
        "next ",
        "blocker",
        "not yet",
        "needs ",
        "need to ",
        "i will ",
        "failing ",
        "fails ",
        "retry",
        "follow up",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn copy_candidate_entries(context: &str) -> Vec<CopyCandidate> {
    let mut section = "";
    let mut candidates = Vec::<CopyCandidate>::new();
    for raw in context.lines() {
        let raw_trimmed = raw.trim();
        if raw_trimmed.is_empty() || raw_trimmed.ends_with(':') {
            match raw_trimmed {
                "PRIMARY USER GOALS:" => section = "PRIMARY USER GOALS:",
                "RECENT SUBSTANTIVE TURNS:" => section = "RECENT SUBSTANTIVE TURNS:",
                "LIVE PREVIEW CONTEXT:" => section = "LIVE PREVIEW CONTEXT:",
                "REMOTE SESSION CONTEXT:" => section = "REMOTE SESSION CONTEXT:",
                _ => {}
            }
            continue;
        }
        let is_user = raw_trimmed.starts_with("USER: ");
        let is_assistant = raw_trimmed.starts_with("ASSISTANT: ");
        let line = raw_trimmed
            .strip_prefix("- ")
            .or_else(|| raw_trimmed.strip_prefix("USER: "))
            .or_else(|| raw_trimmed.strip_prefix("ASSISTANT: "))
            .or_else(|| raw_trimmed.strip_prefix("MSG: "))
            .unwrap_or(raw_trimmed)
            .trim();
        if line.len() < 8 {
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
        if compact.len() < 8 || looks_like_low_signal_generated_copy(&compact) {
            continue;
        }
        let lower = compact.to_ascii_lowercase();
        if line_is_generation_noise(&lower) {
            continue;
        }
        if candidates.iter().any(|existing| existing.line == compact) {
            continue;
        }
        let is_progress = line_has_progress_signal(&lower);
        let is_blocker = line_has_blocker_signal(&lower);
        let is_objective = section == "PRIMARY USER GOALS:"
            || (is_user
                && [
                    "fix ",
                    "debug ",
                    "investigate ",
                    "install ",
                    "review ",
                    "design ",
                    "refine ",
                ]
                .iter()
                .any(|needle| lower.contains(needle)))
            || [
                "investigating ",
                "working on ",
                "fixing ",
                "debugging ",
                "installing ",
                "reviewing ",
                "designing ",
                "refining ",
                "hardening ",
                "auditing ",
            ]
            .iter()
            .any(|needle| lower.starts_with(needle));
        let mut score = match section {
            "PRIMARY USER GOALS:" => 46,
            "RECENT SUBSTANTIVE TURNS:" => 28,
            "LIVE PREVIEW CONTEXT:" | "REMOTE SESSION CONTEXT:" => 8,
            _ => 14,
        };
        if is_objective {
            score += 8;
        }
        if is_progress {
            score += if is_assistant { 12 } else { 7 };
        }
        if is_blocker {
            score += 6;
        }
        if lower.contains("screenshot") || lower.contains("image ") {
            score -= if is_progress { 2 } else { 8 };
        }
        candidates.push(CopyCandidate {
            line: compact,
            score,
            is_progress,
            is_blocker,
            is_objective,
        });
    }
    candidates.sort_by(|left, right| right.score.cmp(&left.score));
    candidates
}

fn sentence_case_line(line: &str) -> String {
    let trimmed = line.trim().trim_end_matches(['.', '!', '?']).trim();
    let trimmed = trimmed
        .strip_prefix("Please ")
        .or_else(|| trimmed.strip_prefix("please "))
        .or_else(|| trimmed.strip_prefix("Can you "))
        .or_else(|| trimmed.strip_prefix("can you "))
        .or_else(|| trimmed.strip_prefix("Need to "))
        .or_else(|| trimmed.strip_prefix("need to "))
        .unwrap_or(trimmed)
        .trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    let commandish_request = lower.starts_with("sudo ")
        || lower.starts_with("apt ")
        || lower.starts_with("brew ")
        || lower.starts_with("dnf ")
        || lower.starts_with("yum ")
        || lower.contains(" apt install ")
        || lower.contains(" brew install ")
        || lower.contains(" dnf install ")
        || lower.contains(" yum install ");
    if commandish_request {
        if let Some(title) = action_title_from_line(trimmed) {
            if let Some(ix) = lower.find("verify ") {
                let suffix = trimmed[ix..].trim();
                if !suffix.is_empty() {
                    return format!("{title} and {suffix}.");
                }
            }
            return format!("{title}.");
        }
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut text = first.to_uppercase().collect::<String>();
    text.push_str(chars.as_str());
    text.push('.');
    text
}

fn title_candidate_lines(context: &str) -> Vec<String> {
    let mut section = "";
    let mut candidates = Vec::<(i32, String)>::new();
    for raw in context.lines() {
        let raw_trimmed = raw.trim();
        if raw_trimmed.is_empty() {
            continue;
        }
        match raw_trimmed {
            "PRIMARY USER GOALS:"
            | "RECENT SUBSTANTIVE TURNS:"
            | "LIVE PREVIEW CONTEXT:"
            | "REMOTE SESSION CONTEXT:" => {
                section = raw_trimmed;
                continue;
            }
            _ => {}
        }
        let is_user = raw_trimmed.starts_with("USER: ");
        let is_assistant = raw_trimmed.starts_with("ASSISTANT: ");
        let line = raw_trimmed
            .strip_prefix("- ")
            .or_else(|| raw_trimmed.strip_prefix("USER: "))
            .or_else(|| raw_trimmed.strip_prefix("ASSISTANT: "))
            .or_else(|| raw_trimmed.strip_prefix("MSG: "))
            .unwrap_or(raw_trimmed)
            .trim();
        if line.len() < 8 {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.contains("<turn_aborted>")
            || looks_like_low_signal_generated_copy(line)
            || line_is_generation_noise(&lower)
        {
            continue;
        }
        let mut score = match section {
            "PRIMARY USER GOALS:" => 40,
            "RECENT SUBSTANTIVE TURNS:" => 24,
            "LIVE PREVIEW CONTEXT:" | "REMOTE SESSION CONTEXT:" => 8,
            _ => 12,
        };
        if is_user {
            score += 14;
        }
        if is_assistant {
            score -= 4;
            if assistant_line_is_title_process_noise(&lower) {
                score -= 18;
            }
        }
        if lower.contains("fix ")
            || lower.contains("debug ")
            || lower.contains("investigate ")
            || lower.contains("install ")
            || lower.contains("restore ")
            || lower.contains("review ")
            || lower.contains("design ")
            || lower.contains("polish ")
            || lower.contains("refine ")
        {
            score += 8;
        }
        if lower.contains("screenshot") || lower.contains("image ") {
            score -= 6;
        }
        candidates.push((score, line.to_string()));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    let mut deduped = Vec::new();
    for (_, candidate) in candidates {
        if deduped
            .iter()
            .any(|existing: &String| existing == &candidate)
        {
            continue;
        }
        deduped.push(candidate);
        if deduped.len() >= 6 {
            break;
        }
    }
    deduped
}

fn assistant_line_is_title_process_noise(lower: &str) -> bool {
    lower.starts_with("i'm ")
        || lower.starts_with("i am ")
        || lower.starts_with("i’ll ")
        || lower.starts_with("i'll ")
        || lower.starts_with("i have ")
        || lower.starts_with("i’ve ")
        || lower.starts_with("if you want")
        || lower.starts_with("the shell is ")
        || lower.starts_with("the note ")
        || lower.starts_with("the note paths ")
        || lower.starts_with("i hit an unexpected change")
        || lower.starts_with("updated ")
        || lower.starts_with("regenerated ")
        || lower.starts_with("split ")
        || lower.starts_with("restored ")
        || lower.starts_with("i'm resuming ")
        || lower.starts_with("what you are describing ")
}

fn context_has_any(lower: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| lower.contains(needle))
}

fn themed_title_from_context(context: &str) -> Option<String> {
    let lower = context.to_ascii_lowercase();
    if context_has_any(
        &lower,
        &[
            "fan spin hard",
            "eating ram and cpu",
            "cpu and ram in ssh",
            "ssh manin",
        ],
    ) {
        if lower.contains("fan") {
            return Some(String::from("Diagnose Laptop Fan Load"));
        }
        return Some(String::from("Diagnose System Load"));
    }
    if context_has_any(
        &lower,
        &[
            "sluggishly slow",
            "sluggish and slow",
            "why manin is sluggishly slow",
            "manin is sluggish",
        ],
    ) {
        return Some(String::from("Investigate Manin Slowness"));
    }
    if context_has_any(
        &lower,
        &["lack of boundaries discussion", "boundaries discussion"],
    ) {
        if lower.contains("decision table") || lower.contains("decision making table") {
            return Some(String::from("Build Boundaries Decision Table"));
        }
        return Some(String::from("Resume Boundaries Discussion"));
    }
    if context_has_any(
        &lower,
        &[
            "instagram pruning different from facebook pruning",
            "attention edge",
            "archive/access edge",
            "reciprocity edge",
            "reduce surface area across channels",
        ],
    ) {
        return Some(String::from("Design Social Pruning Rules"));
    }
    if context_has_any(
        &lower,
        &[
            "import script",
            "import pipeline",
            "merge-conflict fallout",
            "same file twice",
        ],
    ) && context_has_any(
        &lower,
        &["chat export", "instagram", "chats/", "cutting toxic ties"],
    ) {
        return Some(String::from("Fix Instagram Chat Import Pipeline"));
    }
    if context_has_any(
        &lower,
        &[
            "chat export",
            "all exports",
            "latest file replaces the older version",
        ],
    ) && context_has_any(
        &lower,
        &[
            "instagram",
            "chat notes",
            "transcript content",
            "characters/",
            "cutting toxic ties",
        ],
    ) {
        if lower.contains("instagram") {
            return Some(String::from("Update Instagram Chat Notes"));
        }
        return Some(String::from("Update Chat Notes From Export"));
    }
    None
}

fn themed_summary_from_context(context: &str) -> Option<String> {
    let lower = context.to_ascii_lowercase();
    if context_has_any(
        &lower,
        &[
            "fan spin hard",
            "eating ram and cpu",
            "cpu and ram in ssh",
            "ssh manin",
        ],
    ) {
        let summary = if lower.contains("ssh") {
            "Diagnose which processes are driving laptop fan load and SSH-side CPU and RAM pressure."
        } else {
            "Diagnose which processes are driving the laptop fan and overall system load."
        };
        return sanitize_generated_summary(summary);
    }
    if context_has_any(
        &lower,
        &["lack of boundaries discussion", "boundaries discussion"],
    ) {
        let summary = if lower.contains("decision table") || lower.contains("decision making table")
        {
            "Resume the boundaries discussion and turn it into a concrete decision table."
        } else {
            "Resume the boundaries discussion from the prior thread and continue the model."
        };
        return sanitize_generated_summary(summary);
    }
    if context_has_any(
        &lower,
        &[
            "instagram pruning different from facebook pruning",
            "attention edge",
            "archive/access edge",
            "reciprocity edge",
            "reduce surface area across channels",
        ],
    ) {
        let summary = "Define channel-specific social pruning rules by separating Instagram attention, Facebook archive access, and WhatsApp reciprocity.";
        return sanitize_generated_summary(summary);
    }
    if context_has_any(
        &lower,
        &[
            "import script",
            "import pipeline",
            "merge-conflict fallout",
            "same file twice",
        ],
    ) && context_has_any(
        &lower,
        &["chat export", "instagram", "chats/", "cutting toxic ties"],
    ) {
        let summary = "Refresh the latest Instagram chat export and fix the import pipeline so duplicate chat merges stop recurring.";
        return sanitize_generated_summary(summary);
    }
    if context_has_any(
        &lower,
        &[
            "chat export",
            "all exports",
            "latest file replaces the older version",
        ],
    ) && context_has_any(
        &lower,
        &[
            "instagram",
            "chat notes",
            "transcript content",
            "characters/",
            "cutting toxic ties",
        ],
    ) {
        let summary = "Refresh the Instagram chat notes from the latest chat export and update Cutting Toxic Ties with the new threads.";
        return sanitize_generated_summary(summary);
    }
    None
}

fn heuristic_title_from_context(context: &str) -> Option<String> {
    if let Some(title) = themed_title_from_context(context) {
        return Some(title);
    }
    if let Some(title) = heuristic_title_from_shell_context(context) {
        return Some(title);
    }

    for normalized in title_candidate_lines(context) {
        let lower = normalized.to_ascii_lowercase();
        if lower.contains("shortcut") || lower.contains("shortcuts") {
            if let Some(quoted) = extract_quoted_candidate(&normalized) {
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

        if let Some(title) = action_title_from_line(&normalized) {
            return Some(title);
        }

        let words = normalized
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .filter(|word| word.len() > 2)
            .filter(|word| !title_noise_word(word))
            .take(5)
            .map(title_case_word)
            .collect::<Vec<_>>();

        if words.len() < 2 {
            continue;
        }

        let preferred_len = match words.first().map(String::as_str) {
            Some("Install") | Some("Update") | Some("Remove") | Some("Restore") => 2,
            Some("Fix") | Some("Debug") | Some("Repair") | Some("Polish") | Some("Review")
            | Some("Design") | Some("Refine") | Some("Investigate") => 4,
            _ => words.len(),
        };
        let title = words
            .into_iter()
            .take(preferred_len)
            .collect::<Vec<_>>()
            .join(" ");
        if plausible_title(&title) {
            return Some(title);
        }
    }
    None
}

fn heuristic_title_from_shell_context(context: &str) -> Option<String> {
    for line in context
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(command) = extract_prompt_command(line)
            && let Some(title) = heuristic_title_from_shell_command(command)
        {
            return Some(title);
        }
        if line.starts_with("$ ") || line.starts_with("# ") || line.starts_with("> ") {
            let command = line[2..].trim();
            if let Some(title) = heuristic_title_from_shell_command(command) {
                return Some(title);
            }
        }
    }
    None
}

fn extract_prompt_command(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    ["$ ", "# ", "% ", "> "]
        .iter()
        .filter_map(|marker| {
            trimmed
                .rfind(marker)
                .map(|idx| &trimmed[idx + marker.len()..])
        })
        .map(str::trim)
        .find(|command| !command.is_empty())
}

fn heuristic_title_from_shell_command(command: &str) -> Option<String> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }

    if let Some(quoted) = extract_quoted_candidate(command)
        && let Some(title) = title_from_phrase(&quoted)
    {
        return Some(title);
    }

    let mut tokens = command
        .split_whitespace()
        .map(|token| token.trim_matches(|ch: char| ch == '"' || ch == '\'' || ch == '`'))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    while matches!(tokens.first(), Some(token) if *token == "sudo" || token.contains('=')) {
        tokens.remove(0);
    }
    let primary = tokens.first()?.to_ascii_lowercase();

    if matches!(primary.as_str(), "apt" | "apt-get" | "dnf" | "yum" | "brew")
        && tokens.get(1).is_some_and(|token| *token == "install")
        && let Some(package) = tokens.get(2)
    {
        let package = package
            .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
            .trim();
        if !package.is_empty() {
            return Some(format!("Install {}", title_case_word(package)));
        }
    }

    if primary == "cargo" {
        match tokens.get(1).copied() {
            Some("test") => return Some(String::from("Run Cargo Tests")),
            Some("build") => return Some(String::from("Build Rust Project")),
            Some("run") => return Some(String::from("Run Rust App")),
            Some("fmt") => return Some(String::from("Format Rust Code")),
            Some("check") => return Some(String::from("Check Rust Build")),
            _ => {}
        }
    }

    if primary == "git" {
        match tokens.get(1).copied() {
            Some("status") => return Some(String::from("Review Git Status")),
            Some("diff") => return Some(String::from("Review Git Diff")),
            Some("commit") => return Some(String::from("Commit Git Changes")),
            Some("push") => return Some(String::from("Push Git Changes")),
            Some("pull") => return Some(String::from("Pull Git Changes")),
            _ => {}
        }
    }

    title_from_phrase(command)
}

fn title_from_phrase(phrase: &str) -> Option<String> {
    let words = phrase
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .filter(|word| word.len() > 2)
        .filter(|word| {
            !matches!(
                word.to_ascii_lowercase().as_str(),
                "echo"
                    | "sudo"
                    | "bash"
                    | "sh"
                    | "zsh"
                    | "fish"
                    | "then"
                    | "else"
                    | "done"
                    | "true"
                    | "false"
                    | "local"
                    | "shell"
                    | "proof"
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

pub fn looks_like_generated_fallback_title(title: &str) -> bool {
    let compact = title.trim();
    let lower = compact.to_ascii_lowercase();
    let words = compact.split_whitespace().collect::<Vec<_>>();
    let prefixed_session_uuid = [
        "local::",
        "live::",
        "document::",
        "codex::",
        "codex-litellm::",
    ]
    .iter()
    .find_map(|prefix| compact.strip_prefix(prefix))
    .is_some_and(|tail| {
        tail.len() == 36 && tail.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-')
    });
    let prefixed_hash = (compact.len() == 7 || compact.len() == 8)
        && compact.starts_with('Q')
        && compact.chars().skip(1).all(|ch| ch.is_ascii_hexdigit());
    let bare_hash = (compact.len() == 7 || compact.len() == 8)
        && compact.chars().all(|ch| ch.is_ascii_hexdigit());
    let remote_codex_runtime_title = words.len() == 3
        && words[0].eq_ignore_ascii_case("remote")
        && words[1].eq_ignore_ascii_case("codex")
        && words[2].len() == 8
        && words[2].chars().all(|ch| ch.is_ascii_hexdigit());
    let remote_codex_litellm_runtime_title = words.len() == 4
        && words[0].eq_ignore_ascii_case("remote")
        && words[1].eq_ignore_ascii_case("codex")
        && words[2].eq_ignore_ascii_case("litellm")
        && words[3].len() == 8
        && words[3].chars().all(|ch| ch.is_ascii_hexdigit());
    let generic_runtime_title = matches!(
        lower.as_str(),
        "local shell"
            | "local [ok] shell"
            | "local codex"
            | "local [ok] codex"
            | "local codex litellm"
            | "local [ok] codex litellm"
            | "local shell stay alive daemon"
            | "command bin bash"
            | "daemon pty request main viewport"
    );
    prefixed_session_uuid
        || prefixed_hash
        || bare_hash
        || remote_codex_runtime_title
        || remote_codex_litellm_runtime_title
        || generic_runtime_title
}

pub fn looks_like_low_signal_generated_copy(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return true;
    }
    if contains_terminal_control_bytes(text) || looks_like_shell_prompt_copy(text) {
        return true;
    }
    if looks_like_shell_command_copy(text) {
        return true;
    }
    [
        "collaboration mode:",
        "filesystem sandboxing",
        "request_user_input",
        "environment_context",
        "this local shell should stay alive in the daemon while you browse elsewhere.",
        "this local shell uses the same pty/runtime path as other embedded terminals.",
        "daemon pty managed directly by yggterm",
        "local pty managed directly by yggterm",
        "queue live shell session",
        "launching live shell session",
        "launching live codex session",
        "launching live ssh session",
        "workspace: localhost",
        "deploy state:",
        "launch phase:",
        "terminal surface: embedded xterm.js",
        "daemon runtime:",
        "daemon pty:",
        "request main viewport terminal stream",
        "$ exec ",
        "local codex terminal rooted at ",
        "ssh terminal on ",
        "open live terminal",
        "this session should land in the main viewport",
        "launch command prepared",
        "remote bootstrap will eventually",
        "server launch",
        "viewed image",
        "it's a screenshot of",
        "the main visible text shows",
        "other visible ui details",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn contains_terminal_control_bytes(text: &str) -> bool {
    text.chars()
        .any(|ch| ch == '\u{1b}' || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
}

fn strip_terminal_control_bytes(text: &str) -> String {
    text.chars()
        .filter(|ch| *ch != '\u{1b}' && (!ch.is_control() || matches!(ch, '\n' | '\r' | '\t')))
        .collect()
}

fn looks_like_shell_prompt_copy(text: &str) -> bool {
    let sanitized = strip_terminal_control_bytes(text);
    let lines = sanitized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.len() > 2 {
        return false;
    }
    let prompt = lines.last().copied().unwrap_or_default();
    if prompt.len() > 96
        || !["$", "#", "%", ">"]
            .iter()
            .any(|suffix| prompt.ends_with(suffix))
    {
        return false;
    }
    let has_prompt_markers = prompt.contains('@')
        || prompt.contains(':')
        || prompt.contains('~')
        || prompt.contains('/')
        || prompt.contains('\\');
    let allowed = prompt.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                ' ' | '@' | ':' | '/' | '\\' | '.' | '_' | '~' | '-' | '[' | ']' | '(' | ')'
                    | '{' | '}' | '+' | '$' | '#' | '%' | '>'
            )
    });
    has_prompt_markers && allowed
}

fn looks_like_shell_command_copy(text: &str) -> bool {
    let sanitized = strip_terminal_control_bytes(text);
    let lines = sanitized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return false;
    }
    let lower_lines = lines
        .iter()
        .map(|line| line.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let shell_verbs = [
        "printf ",
        "echo ",
        "exec ",
        "cd ",
        "ssh ",
        "cargo ",
        "python ",
        "python3 ",
        "npm ",
        "git ",
        "tmux ",
        "screen ",
        "mkdir ",
        "rm ",
        "cp ",
        "mv ",
        "cat ",
        "sed ",
        "rg ",
        "find ",
        "ls ",
        "export ",
        "source ",
    ];
    let looks_like_command_line = |line: &str| {
        let trimmed = line.trim_start_matches("$ ").trim();
        if trimmed.is_empty() {
            return false;
        }
        shell_verbs.iter().any(|verb| trimmed.starts_with(verb))
            || (trimmed.contains("\\033") || trimmed.contains("\\x1b"))
                && (trimmed.contains(';') || trimmed.contains("&&") || trimmed.contains("||"))
    };
    if lower_lines.iter().all(|line| looks_like_command_line(line)) {
        return true;
    }
    lower_lines.iter().any(|line| {
        let punctuation_heavy = line.contains(';') || line.contains("&&") || line.contains("||");
        let escape_heavy = line.contains("\\033") || line.contains("\\x1b");
        punctuation_heavy && escape_heavy && looks_like_command_line(line)
    })
}

fn heuristic_precis_from_context(context: &str) -> Option<String> {
    let candidates = copy_candidate_entries(context);
    let objective = candidates
        .iter()
        .find(|candidate| candidate.is_objective)
        .or_else(|| candidates.first())?;
    let progress = candidates
        .iter()
        .find(|candidate| candidate.is_progress && candidate.line != objective.line);
    let precis = if let Some(progress) = progress {
        format!(
            "{} {}",
            sentence_case_line(&objective.line),
            sentence_case_line(&progress.line)
        )
    } else {
        sentence_case_line(&objective.line)
    };
    sanitize_generated_precis(&precis)
}

fn heuristic_summary_from_context(context: &str) -> Option<String> {
    if let Some(summary) = themed_summary_from_context(context) {
        return Some(summary);
    }
    let candidates = copy_candidate_entries(context);
    let objective = candidates
        .iter()
        .find(|candidate| candidate.is_objective)
        .or_else(|| candidates.first())?;
    let progress_lines = candidates
        .iter()
        .filter(|candidate| candidate.is_progress && candidate.line != objective.line)
        .map(|candidate| candidate.line.clone())
        .collect::<Vec<_>>();
    let blocker = candidates.iter().find(|candidate| {
        candidate.is_blocker
            && candidate.line != objective.line
            && !progress_lines.iter().any(|line| line == &candidate.line)
    });
    let mut lines = vec![sentence_case_line(&objective.line)];
    for progress in progress_lines.into_iter().take(2) {
        lines.push(sentence_case_line(&progress));
    }
    if let Some(blocker) = blocker
        && lines.len() < 4
    {
        lines.push(sentence_case_line(&blocker.line));
    }
    sanitize_generated_summary(&lines.join(" "))
}

pub fn best_effort_title_from_context(context: &str) -> Option<String> {
    let title = heuristic_title_from_context(context)?;
    let title = sanitize_generated_title(&title)?;
    (!looks_like_generated_fallback_title(&title)).then_some(title)
}

pub fn best_effort_context_from_session_path(file_path: &Path) -> Result<String> {
    extract_tail_context(file_path)
}

pub fn best_effort_precis_from_context(context: &str) -> Option<String> {
    let precis = heuristic_precis_from_context(context)?;
    let precis = sanitize_generated_precis(&precis)?;
    (!looks_like_low_signal_generated_copy(&precis)).then_some(precis)
}

pub fn best_effort_summary_from_context(context: &str) -> Option<String> {
    let summary = heuristic_summary_from_context(context)?;
    let summary = sanitize_generated_summary(&summary)?;
    (!looks_like_low_signal_generated_copy(&summary)).then_some(summary)
}

fn title_case_word(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    match lower.as_str() {
        "api" => return "API".to_string(),
        "apt" => return "APT".to_string(),
        "cli" => return "CLI".to_string(),
        "codex" => return "Codex".to_string(),
        "json" => return "JSON".to_string(),
        "jsonl" => return "JSONL".to_string(),
        "llm" => return "LLM".to_string(),
        "litellm" => return "LiteLLM".to_string(),
        "pty" => return "PTY".to_string(),
        "ssh" => return "SSH".to_string(),
        "tui" => return "TUI".to_string(),
        "ui" => return "UI".to_string(),
        "ux" => return "UX".to_string(),
        "vscode" => return "VS Code".to_string(),
        "wezterm" => return "WezTerm".to_string(),
        "x11" => return "X11".to_string(),
        "xterm" => return "Xterm".to_string(),
        "yggterm" => return "Yggterm".to_string(),
        _ => {}
    }
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
