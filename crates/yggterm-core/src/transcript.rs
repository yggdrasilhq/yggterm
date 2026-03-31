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

pub fn generation_context_from_messages(messages: &[TranscriptMessage]) -> String {
    let mut goals = Vec::<String>::new();
    let mut recent = Vec::<(TranscriptRole, String)>::new();
    let mut recent_chars = 0usize;

    for message in messages {
        let Some(text) = message_text_for_generation(message) else {
            continue;
        };
        if message.role == TranscriptRole::User
            && text.len() >= 28
            && !goals.iter().any(|existing| existing == &text)
        {
            goals.push(text.clone());
        }
    }

    for message in messages.iter().rev() {
        let Some(text) = message_text_for_generation(message) else {
            continue;
        };
        if recent.iter().any(|(_, existing)| existing == &text) {
            continue;
        }
        recent_chars += text.len();
        recent.push((message.role, text));
        if recent.len() >= 8 || recent_chars >= 2600 {
            break;
        }
    }
    recent.reverse();

    let mut sections = Vec::new();
    let goal_tail = goals
        .into_iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    if !goal_tail.is_empty() {
        sections.push(format!(
            "PRIMARY USER GOALS:\n{}",
            goal_tail
                .iter()
                .map(|goal| format!("- {goal}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !recent.is_empty() {
        sections.push(format!(
            "RECENT SUBSTANTIVE TURNS:\n{}",
            recent
                .iter()
                .map(|(role, text)| format!("{}: {}", role.display_label(), text))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    sections.join("\n\n")
}

pub fn read_codex_transcript_messages(path: &Path) -> Result<Vec<TranscriptMessage>> {
    read_codex_transcript_messages_with_limit(path, None)
}

pub fn read_codex_transcript_messages_limited(
    path: &Path,
    max_messages: usize,
) -> Result<Vec<TranscriptMessage>> {
    read_codex_transcript_messages_with_limit(path, Some(max_messages))
}

fn read_codex_transcript_messages_with_limit(
    path: &Path,
    max_messages: Option<usize>,
) -> Result<Vec<TranscriptMessage>> {
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
                    if max_messages.is_some_and(|limit| messages.len() >= limit) {
                        break;
                    }
                }
            }
            Some("event_msg") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                let Some((role, text)) = event_message_role_and_text(payload) else {
                    continue;
                };
                push_message_lines(
                    &mut messages,
                    role,
                    normalize_preview_text(text),
                    extract_timestamp_raw(&value),
                );
            }
            _ => {}
        }

        if max_messages.is_some_and(|limit| messages.len() >= limit) {
            break;
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
    push_message_lines(
        messages,
        normalized_message_role(payload),
        message_lines_from_payload(payload),
        timestamp,
    );
}

fn push_message_lines(
    messages: &mut Vec<TranscriptMessage>,
    role: TranscriptRole,
    lines: Vec<String>,
    timestamp: Option<String>,
) {
    if lines.is_empty() {
        return;
    }
    let candidate_key = normalized_transcript_message_key(role, &lines);
    if let Some(last) = messages.last() {
        let last_key = normalized_transcript_message_key(last.role, &last.lines);
        if last.role == role && last_key == candidate_key {
            return;
        }
    }
    messages.push(TranscriptMessage {
        role,
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
        "user" => TranscriptRole::User,
        "assistant" => TranscriptRole::Assistant,
        _ => TranscriptRole::System,
    }
}

fn event_message_role_and_text(payload: &Value) -> Option<(TranscriptRole, &str)> {
    let role = match payload.get("type").and_then(Value::as_str) {
        Some("user_message") => TranscriptRole::User,
        Some("agent_message") => TranscriptRole::Assistant,
        _ => return None,
    };
    payload
        .get("message")
        .and_then(Value::as_str)
        .map(|text| (role, text))
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

fn message_text_for_generation(message: &TranscriptMessage) -> Option<String> {
    let joined = message
        .lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let compact = joined
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let compact = normalize_generation_semantic_text(&compact);
    if compact.is_empty() || looks_like_generation_noise(&compact, message.role) {
        return None;
    }
    Some(compact)
}

fn normalize_generation_semantic_text(text: &str) -> String {
    collapse_named_image_markup(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn normalized_transcript_message_key(role: TranscriptRole, lines: &[String]) -> String {
    let text = lines
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    format!("{role:?}:{}", normalize_generation_semantic_text(&text))
}

fn collapse_named_image_markup(text: &str) -> String {
    let mut remaining = text.trim();
    let mut out = String::new();

    loop {
        let Some(start) = remaining.find("<image name=[") else {
            out.push_str(remaining);
            break;
        };
        out.push_str(&remaining[..start]);
        let after = &remaining[start + "<image name=[".len()..];
        let Some(label_end) = after.find("]>") else {
            out.push_str(&remaining[start..]);
            break;
        };
        let label_text = after[..label_end].trim();
        let label = format!("[{label_text}]");
        out.push_str(&label);

        let mut tail = after[label_end + 2..].trim_start();
        if let Some(stripped) = tail.strip_prefix("</image>") {
            tail = stripped.trim_start();
        }
        if let Some(stripped) = tail.strip_prefix(&label) {
            tail = stripped.trim_start();
        }
        remaining = tail;
    }

    out
}

fn looks_like_generation_noise(text: &str, role: TranscriptRole) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.len() < 16 {
        return true;
    }
    if role == TranscriptRole::System {
        return true;
    }
    [
        "<collaboration_mode>",
        "</collaboration_mode>",
        "collaboration_mode>#",
        "collaboration mode:",
        "filesystem sandboxing",
        "request_user_input",
        "environment_context",
        "<environment_context>",
        "</environment_context>",
        "<timezone>",
        "open live terminal",
        "this session should land in the main viewport",
        "launch command prepared",
        "remote bootstrap will eventually",
        "server launch",
        "viewed image",
        "it's a screenshot of",
        "the main visible text shows",
        "other visible ui details",
        "can you read this image",
        "clipboard/clipboard-",
        "@/home/",
        "i’m opening the image now",
        "i'm opening the image now",
        "extract the text or key contents",
        "heads up, you have less than",
        "run /status for a breakdown",
        "model to change",
        "rate limits until",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::{
        TranscriptMessage, TranscriptRole, generation_context_from_messages,
        read_codex_transcript_messages,
    };
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
    fn transcript_reader_treats_developer_messages_as_system() -> Result<()> {
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
        assert_eq!(messages[0].role, TranscriptRole::System);

        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn transcript_reader_dedupes_response_and_event_message_pairs() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "yggterm-transcript-dedupe-{}-{}.jsonl",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(
            &path,
            [
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"continue."}]}}"#,
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"continue."}}"#,
                r#"{"timestamp":"2026-03-20T10:00:01Z","type":"event_msg","payload":{"type":"agent_message","message":"I fixed it."}}"#,
                r#"{"timestamp":"2026-03-20T10:00:01Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"I fixed it."}]}}"#,
            ]
            .join("\n"),
        )?;

        let messages = read_codex_transcript_messages(&path)?;
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert_eq!(messages[0].role, TranscriptRole::User);
        assert_eq!(messages[0].lines, vec!["continue.".to_string()]);
        assert_eq!(messages[1].role, TranscriptRole::Assistant);
        assert_eq!(messages[1].lines, vec!["I fixed it.".to_string()]);

        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn generation_context_filters_noise_and_keeps_goals() {
        let messages = vec![
            TranscriptMessage {
                role: TranscriptRole::User,
                timestamp: None,
                lines: vec!["Can you change the timezone of this host and ssh dev to Asia/Kolkata?".into()],
            },
            TranscriptMessage {
                role: TranscriptRole::Assistant,
                timestamp: None,
                lines: vec!["Open live terminal 019... through the Yggterm server.".into()],
            },
            TranscriptMessage {
                role: TranscriptRole::Assistant,
                timestamp: None,
                lines: vec!["I changed the dev SSH target from Etc/UTC to Asia/Kolkata and verified it.".into()],
            },
            TranscriptMessage {
                role: TranscriptRole::Assistant,
                timestamp: None,
                lines: vec!["It's a screenshot of a terminal/app window titled Can You Change Timezone Host.".into()],
            },
        ];

        let context = generation_context_from_messages(&messages);
        assert!(context.contains("PRIMARY USER GOALS"));
        assert!(context.contains("Can you change the timezone"));
        assert!(context.contains("I changed the dev SSH target"));
        assert!(!context.contains("Open live terminal"));
        assert!(!context.contains("It's a screenshot of"));
    }
}
