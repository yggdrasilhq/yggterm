use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRole {
    User,
    Assistant,
    System,
}

impl TranscriptRole {
    pub fn display_label(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Assistant => "ASSISTANT",
            Self::System => "SYSTEM",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptMessage {
    pub role: TranscriptRole,
    pub timestamp: Option<String>,
    pub lines: Vec<String>,
}

pub fn read_codex_transcript_messages(path: &Path) -> Result<Vec<TranscriptMessage>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read session transcript {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = line.with_context(|| format!("failed to read line from {}", path.display()))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        match value.get("type").and_then(Value::as_str) {
            Some("response_item") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                push_message(
                    &mut messages,
                    payload,
                    extract_timestamp_raw(payload).or_else(|| extract_timestamp_raw(&value)),
                );
            }
            Some("compacted") => {
                let Some(history) = value
                    .get("payload")
                    .and_then(|payload| payload.get("replacement_history"))
                    .and_then(Value::as_array)
                else {
                    continue;
                };
                let fallback_timestamp = extract_timestamp_raw(&value);
                for item in history {
                    if item.get("type").and_then(Value::as_str) != Some("message") {
                        continue;
                    }
                    push_message(
                        &mut messages,
                        item,
                        extract_timestamp_raw(item).or_else(|| fallback_timestamp.clone()),
                    );
                }
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
                messages.push(TranscriptMessage {
                    role: TranscriptRole::User,
                    timestamp: extract_timestamp_raw(&value),
                    lines,
                });
            }
            _ => {}
        }
    }

    Ok(messages)
}

pub fn message_lines_from_payload(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(text) = payload.get("content").and_then(Value::as_str) {
        lines.extend(normalize_preview_text(text));
    }
    if let Some(content_items) = payload.get("content").and_then(Value::as_array) {
        for item in content_items {
            if let Some(text) = extract_text_fragment(item) {
                lines.extend(normalize_preview_text(text));
            }
        }
    }
    lines
}

fn push_message(messages: &mut Vec<TranscriptMessage>, payload: &Value, timestamp: Option<String>) {
    let lines = message_lines_from_payload(payload);
    if lines.is_empty() {
        return;
    }
    messages.push(TranscriptMessage {
        role: normalized_message_role(payload),
        timestamp,
        lines,
    });
}

fn normalized_message_role(payload: &Value) -> TranscriptRole {
    match payload
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("assistant")
    {
        "user" | "developer" => TranscriptRole::User,
        "assistant" => TranscriptRole::Assistant,
        _ => TranscriptRole::System,
    }
}

fn extract_timestamp_raw(value: &Value) -> Option<String> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("timestamp"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
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

fn normalize_preview_text(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{TranscriptRole, read_codex_transcript_messages};
    use anyhow::Result;
    use std::fs;

    #[test]
    fn transcript_reader_preserves_compacted_message_sequence() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "yggterm-transcript-{}-{}.jsonl",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(
            &path,
            [
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"compacted","payload":{"replacement_history":[{"role":"user","type":"message","content":[{"type":"input_text","text":"first prompt"}]},{"role":"assistant","type":"message","content":[{"type":"output_text","text":"first answer"}]},{"role":"assistant","type":"message","content":[{"type":"output_text","text":"second answer"}]}]}}"#,
            ]
            .join("\n"),
        )?;

        let messages = read_codex_transcript_messages(&path)?;
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, TranscriptRole::User);
        assert_eq!(messages[1].role, TranscriptRole::Assistant);
        assert_eq!(messages[2].role, TranscriptRole::Assistant);
        assert_eq!(messages[1].lines[0], "first answer");
        assert_eq!(messages[2].lines[0], "second answer");

        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn transcript_reader_normalizes_developer_messages_as_user() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "yggterm-transcript-dev-{}-{}.jsonl",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(
            &path,
            [
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"safety instruction"}]}}"#,
            ]
            .join("\n"),
        )?;

        let messages = read_codex_transcript_messages(&path)?;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, TranscriptRole::User);

        let _ = fs::remove_file(path);
        Ok(())
    }
}
