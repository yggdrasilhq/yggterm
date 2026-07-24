use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
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

    // Harness-quality fix (user mandate 2026-06-11): the old budget (8 turns /
    // 2600 chars) fed the LLM a sliver of a long session — titles/summaries
    // described the last few pokes, not the work. Budget raised to 24 turns /
    // 12000 chars, with a per-message cap so one giant assistant dump can't
    // consume the whole window. Live A/B against llm.example.com showed the
    // richer context yields summaries that name the actual project state.
    const RECENT_TURNS_MAX: usize = 24;
    const RECENT_CHARS_MAX: usize = 12_000;
    const PER_MESSAGE_CHARS_MAX: usize = 1_200;
    for message in messages.iter().rev() {
        let Some(text) = message_text_for_generation(message) else {
            continue;
        };
        if recent.iter().any(|(_, existing)| existing == &text) {
            continue;
        }
        let text = if text.chars().count() > PER_MESSAGE_CHARS_MAX {
            let mut clipped = text.chars().take(PER_MESSAGE_CHARS_MAX).collect::<String>();
            clipped.push('…');
            clipped
        } else {
            text
        };
        recent_chars += text.len();
        recent.push((message.role, text));
        if recent.len() >= RECENT_TURNS_MAX || recent_chars >= RECENT_CHARS_MAX {
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

pub fn read_codex_transcript_messages_tail_limited(
    path: &Path,
    max_messages: usize,
) -> Result<Vec<TranscriptMessage>> {
    const INITIAL_WINDOW_BYTES: u64 = 2 * 1024 * 1024;
    const MAX_WINDOW_BYTES: u64 = 64 * 1024 * 1024;

    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to read session transcript {}", path.display()))?;
    let file_len = file
        .metadata()
        .with_context(|| format!("failed to stat session transcript {}", path.display()))?
        .len();
    let mut window = INITIAL_WINDOW_BYTES.min(file_len.max(1));

    loop {
        let start = file_len.saturating_sub(window);
        file.seek(SeekFrom::Start(start))
            .with_context(|| format!("failed to seek session transcript {}", path.display()))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).with_context(|| {
            format!("failed to read session transcript tail {}", path.display())
        })?;
        let text = String::from_utf8_lossy(&bytes);
        let lines = if start > 0 {
            text.lines().skip(1).collect::<Vec<_>>()
        } else {
            text.lines().collect::<Vec<_>>()
        };
        let messages = parse_transcript_message_lines(lines, max_messages);
        if messages.len() >= max_messages || start == 0 || window >= MAX_WINDOW_BYTES {
            return Ok(messages);
        }
        window = (window.saturating_mul(2))
            .min(MAX_WINDOW_BYTES)
            .min(file_len.max(1));
    }
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

fn push_message_deque(
    messages: &mut VecDeque<TranscriptMessage>,
    payload: &Value,
    timestamp: Option<String>,
    max_messages: usize,
) {
    push_message_lines_deque(
        messages,
        normalized_message_role(payload),
        message_lines_from_payload(payload),
        timestamp,
        max_messages,
    );
}

fn parse_transcript_message_lines<'a, I>(lines: I, max_messages: usize) -> Vec<TranscriptMessage>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut messages = VecDeque::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
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
                push_message_deque(
                    &mut messages,
                    payload,
                    extract_timestamp_raw(payload).or_else(|| extract_timestamp_raw(&value)),
                    max_messages,
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
                    push_message_deque(
                        &mut messages,
                        item,
                        extract_timestamp_raw(item).or_else(|| fallback_timestamp.clone()),
                        max_messages,
                    );
                }
            }
            Some("event_msg") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                let Some((role, text)) = event_message_role_and_text(payload) else {
                    continue;
                };
                push_message_lines_deque(
                    &mut messages,
                    role,
                    normalize_preview_text(text),
                    extract_timestamp_raw(&value),
                    max_messages,
                );
            }
            _ => {}
        }
    }
    messages.into_iter().collect()
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

fn push_message_lines_deque(
    messages: &mut VecDeque<TranscriptMessage>,
    role: TranscriptRole,
    lines: Vec<String>,
    timestamp: Option<String>,
    max_messages: usize,
) {
    if lines.is_empty() {
        return;
    }
    let candidate_key = normalized_transcript_message_key(role, &lines);
    if let Some(last) = messages.back() {
        let last_key = normalized_transcript_message_key(last.role, &last.lines);
        if last.role == role && last_key == candidate_key {
            return;
        }
    }
    messages.push_back(TranscriptMessage {
        role,
        timestamp,
        lines,
    });
    while messages.len() > max_messages {
        messages.pop_front();
    }
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
        .filter(|line| !preview_transcript_scaffold_line(line))
        .map(ToOwned::to_owned)
        .collect()
}

fn preview_transcript_scaffold_line(trimmed: &str) -> bool {
    let lower = trimmed.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    [
        "<turn_id>",
        "</turn_id>",
        "<reason>",
        "</reason>",
        "<guidance>",
        "</guidance>",
        "<turn_aborted>",
        "</turn_aborted>",
        "the user interrupted the previous turn on purpose",
        "any running unified exec processes were terminated",
    ]
    .iter()
    .any(|needle| lower.starts_with(needle) || lower.contains(needle))
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
    let min_len = match role {
        TranscriptRole::User => 8,
        TranscriptRole::Assistant => 12,
        TranscriptRole::System => return true,
    };
    if lower.len() < min_len {
        return true;
    }
    if matches!(
        lower.as_str(),
        "ok" | "okay" | "thanks" | "thank you" | "yes" | "no" | "hi" | "hello" | "done"
    ) {
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
        read_codex_transcript_messages, read_codex_transcript_messages_tail_limited,
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

    #[test]
    fn transcript_reader_filters_interrupted_turn_scaffold() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "yggterm-transcript-interrupted-{}-{}.jsonl",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(
            &path,
            [
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<turn_id>8</turn_id>\n<reason>interrupted</reason>\n<guidance>The user interrupted the previous turn on purpose. Any running unified exec processes were terminated.</guidance>"}]}}"#,
                r#"{"timestamp":"2026-03-20T10:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"real follow-up"}]}}"#,
            ]
            .join("\n"),
        )?;

        let messages = read_codex_transcript_messages(&path)?;
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert_eq!(messages[0].role, TranscriptRole::User);
        assert_eq!(messages[0].lines, vec!["real follow-up".to_string()]);

        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn generation_context_keeps_short_substantive_user_question() {
        let messages = vec![TranscriptMessage {
            role: TranscriptRole::User,
            timestamp: Some("2026-04-17T10:00:00Z".to_string()),
            lines: vec!["Who are you?".to_string()],
        }];

        let context = generation_context_from_messages(&messages);

        assert!(context.contains("USER: Who are you?"));
    }

    #[test]
    fn transcript_reader_tail_limit_keeps_latest_messages() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "yggterm-transcript-tail-{}-{}.jsonl",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(
            &path,
            [
                r#"{"timestamp":"2026-03-20T10:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]}}"#,
                r#"{"timestamp":"2026-03-20T10:00:01Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"second"}]}}"#,
                r#"{"timestamp":"2026-03-20T10:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"third"}]}}"#,
                r#"{"timestamp":"2026-03-20T10:00:03Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"fourth"}]}}"#,
            ]
            .join("\n"),
        )?;

        let messages = read_codex_transcript_messages_tail_limited(&path, 2)?;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].lines, vec!["third".to_string()]);
        assert_eq!(messages[1].lines, vec!["fourth".to_string()]);

        let _ = fs::remove_file(path);
        Ok(())
    }
}

// ── Claude Code transcripts ──────────────────────────────────────────────────
//
// The codex reader above walks `response_item` payloads; Claude Code writes a
// different JSONL, so it needs its own walk. Both land on [`TranscriptMessage`]
// — one shape, so everything downstream (title generation, the rendered
// transcript view) is CLI-agnostic.
//
// This belongs on `AgentCliDescriptor` eventually: "how do I read this CLI's
// transcript into messages" is per-CLI data exactly like `read_store_entry`
// (docs/spec-agent-cli-harness.md §3). It is left as a free function until a
// second caller needs it polymorphically — inventing the registry field with
// one consumer would be guessing at the signature.

/// Read a Claude Code transcript into messages, oldest first.
///
/// What is deliberately dropped, and why:
/// - **`thinking` blocks.** Private reasoning the CLI itself renders collapsed.
///   Showing it in a transcript view would surface something the user did not
///   choose to see.
/// - **`tool_use` / `tool_result` blocks.** The timeline renders tool activity
///   as its own kind of entry, not as message text; emitting them as prose
///   would produce a wall of JSON. Rendering them properly is follow-up work,
///   and dropping them is honest in the meantime.
/// - **`isMeta` records** (the `<local-command-caveat>` wrappers) and
///   **`isSidechain` records** (sub-agent chatter): neither is the conversation
///   the user had.
pub fn read_claude_code_transcript_messages(path: &Path) -> Result<Vec<TranscriptMessage>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read claude code transcript {}", path.display()))?;
    let mut messages = Vec::new();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let role = match value.get("type").and_then(Value::as_str) {
            Some("user") => TranscriptRole::User,
            Some("assistant") => TranscriptRole::Assistant,
            _ => continue,
        };
        if value.get("isMeta").and_then(Value::as_bool) == Some(true)
            || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let lines = claude_code_message_lines(message);
        if lines.is_empty() || lines_are_only_command_plumbing(&lines) {
            continue;
        }
        push_message_lines(
            &mut messages,
            role,
            lines,
            value
                .get("timestamp")
                .and_then(Value::as_str)
                .map(str::to_string),
        );
    }
    Ok(messages)
}

/// A CC message's `content` is either a bare string (user turns) or a block
/// array (assistant turns). Only `text` blocks carry displayable prose.
fn claude_code_message_lines(message: &Value) -> Vec<String> {
    let content = message.get("content");
    if let Some(text) = content.and_then(Value::as_str) {
        return normalize_preview_text(strip_local_command_caveat(text));
    }
    let Some(blocks) = content.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            lines.extend(normalize_preview_text(strip_local_command_caveat(text)));
        }
    }
    lines
}

/// Slash-command bookkeeping the CLI records as user turns: the command it
/// ran, the arguments, and the command's own stdout. Claude Code renders these
/// as an invocation chip, not as something the user said — reproduced verbatim
/// in a transcript they are XML-ish noise at the top of the conversation, which
/// is exactly how the first real-transcript run looked.
const COMMAND_PLUMBING_TAGS: &[&str] = &[
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<local-command-stdout>",
    "<local-command-stderr>",
];

/// True when EVERY non-empty line is command plumbing. Deliberately not "any":
/// a user turn that happens to quote a tag alongside real prose is still a
/// message, and dropping it would lose what they actually asked.
fn lines_are_only_command_plumbing(lines: &[String]) -> bool {
    let mut saw_content = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        saw_content = true;
        if !COMMAND_PLUMBING_TAGS
            .iter()
            .any(|tag| trimmed.starts_with(tag))
        {
            return false;
        }
    }
    saw_content
}

/// A slash command run in the CLI is recorded as a user turn wrapped in a
/// caveat block explaining that the text was machine-generated. The user's
/// actual words follow it; the wrapper is noise in a transcript view.
fn strip_local_command_caveat(text: &str) -> &str {
    const END: &str = "</local-command-caveat>";
    match text.find(END) {
        Some(index) => text[index + END.len()..].trim_start(),
        None => text,
    }
}

/// One message as the rendered-transcript view consumes it.
///
/// This is the Rust side of `TranscriptMessage` in
/// `third_party/t3code-timeline/src/mount.tsx`. The two must agree, so the
/// field names are the JSON contract and are locked by a test — a silent rename
/// here would leave the surface rendering an empty timeline with no error.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TranscriptViewMessage {
    pub id: String,
    pub role: &'static str,
    pub text: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

/// Project transcript messages into the view's shape.
///
/// `session_id` seeds stable per-message ids: the renderer keys its virtual
/// list on them, so ids that change between refreshes would remount every row
/// and lose scroll position on a live-updating transcript.
///
/// Messages with no timestamp are DROPPED rather than dated `now`. The timeline
/// sorts and displays `createdAt`, so an invented time reorders real history —
/// a lie that looks like data.
pub fn transcript_view_messages(
    session_id: &str,
    messages: &[TranscriptMessage],
) -> Vec<TranscriptViewMessage> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            let created_at = message.timestamp.clone()?;
            let text = message.lines.join("\n");
            if text.trim().is_empty() {
                return None;
            }
            Some(TranscriptViewMessage {
                id: format!("{session_id}:{index}"),
                role: match message.role {
                    TranscriptRole::User => "user",
                    TranscriptRole::Assistant => "assistant",
                    TranscriptRole::System => "system",
                },
                text,
                created_at,
            })
        })
        .collect()
}

#[cfg(test)]
mod claude_code_transcript_tests {
    use super::*;

    fn write(lines: &[&str]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "yggterm-cc-transcript-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn reads_user_and_assistant_turns_with_their_timestamps() {
        let path = write(&[
            r#"{"type":"user","timestamp":"2026-07-25T12:00:00.000Z","message":{"role":"user","content":"fix the toggle"}}"#,
            r#"{"type":"assistant","timestamp":"2026-07-25T12:00:04.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Found it."}]}}"#,
        ]);
        let messages = read_claude_code_transcript_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, TranscriptRole::User);
        assert_eq!(messages[0].lines, vec!["fix the toggle".to_string()]);
        assert_eq!(
            messages[0].timestamp.as_deref(),
            Some("2026-07-25T12:00:00.000Z")
        );
        assert_eq!(messages[1].role, TranscriptRole::Assistant);
        assert_eq!(messages[1].lines, vec!["Found it.".to_string()]);
        let _ = fs::remove_file(&path);
    }

    // Thinking is what the CLI shows collapsed, and tool blocks are activity
    // rather than prose. Emitting either as message text would put something in
    // front of the user that they did not choose to see.
    #[test]
    fn drops_thinking_and_tool_blocks_but_keeps_the_text_beside_them() {
        let path = write(&[
            // ONE line: this is JSONL, and a pretty-printed fixture would not
            // parse — which is a fixture bug that looks exactly like a reader bug.
            r#"{"type":"assistant","timestamp":"2026-07-25T12:00:00.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"private reasoning"},{"type":"text","text":"Here is the answer."},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
        ]);
        let messages = read_claude_code_transcript_messages(&path).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].lines, vec!["Here is the answer.".to_string()]);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn skips_meta_and_sidechain_records_and_the_slash_command_caveat() {
        let path = write(&[
            r#"{"type":"user","isMeta":true,"timestamp":"2026-07-25T12:00:00.000Z","message":{"role":"user","content":"system noise"}}"#,
            r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-07-25T12:00:01.000Z","message":{"role":"assistant","content":[{"type":"text","text":"subagent chatter"}]}}"#,
            r#"{"type":"user","timestamp":"2026-07-25T12:00:02.000Z","message":{"role":"user","content":"<local-command-caveat>Caveat: generated</local-command-caveat>\nthe real ask"}}"#,
            r#"{"type":"summary","summary":"not a message"}"#,
        ]);
        let messages = read_claude_code_transcript_messages(&path).unwrap();
        assert_eq!(messages.len(), 1, "only the real user turn survives");
        assert_eq!(messages[0].lines, vec!["the real ask".to_string()]);
        let _ = fs::remove_file(&path);
    }

    // Slash-command bookkeeping is not conversation. Caught on a REAL
    // transcript: the view opened with `<command-name>/login</command-name>`
    // and `<local-command-stdout>` before anything the user said.
    #[test]
    fn drops_turns_that_are_only_slash_command_plumbing() {
        let path = write(&[
            r#"{"type":"user","timestamp":"2026-07-25T12:00:00.000Z","message":{"role":"user","content":"<command-name>/login</command-name>\n<command-message>login</command-message>"}}"#,
            r#"{"type":"user","timestamp":"2026-07-25T12:00:01.000Z","message":{"role":"user","content":"<local-command-stdout>Login successful</local-command-stdout>"}}"#,
            r#"{"type":"user","timestamp":"2026-07-25T12:00:02.000Z","message":{"role":"user","content":"continue yggterm campaign"}}"#,
        ]);
        let messages = read_claude_code_transcript_messages(&path).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].lines, vec!["continue yggterm campaign".to_string()]);
        let _ = fs::remove_file(&path);
    }

    // "Only plumbing", not "any plumbing": a turn that quotes a tag ALONGSIDE
    // real prose is still something the user said.
    #[test]
    fn keeps_a_turn_that_merely_mentions_a_command_tag() {
        let path = write(&[
            r#"{"type":"user","timestamp":"2026-07-25T12:00:00.000Z","message":{"role":"user","content":"<command-name>/login</command-name>\nwhy did this fail?"}}"#,
        ]);
        let messages = read_claude_code_transcript_messages(&path).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].lines.iter().any(|line| line.contains("why did this fail?")));
        let _ = fs::remove_file(&path);
    }

    // The renderer keys its virtual list on `id`, so ids must be stable across
    // refreshes or every row remounts and the scroll position is lost.
    #[test]
    fn view_message_ids_are_stable_for_the_same_transcript() {
        let messages = vec![
            TranscriptMessage {
                role: TranscriptRole::User,
                timestamp: Some("2026-07-25T12:00:00.000Z".into()),
                lines: vec!["one".into()],
            },
            TranscriptMessage {
                role: TranscriptRole::Assistant,
                timestamp: Some("2026-07-25T12:00:01.000Z".into()),
                lines: vec!["two".into()],
            },
        ];
        let first = transcript_view_messages("abc", &messages);
        let second = transcript_view_messages("abc", &messages);
        assert_eq!(first, second);
        assert_eq!(first[0].id, "abc:0");
        assert_eq!(first[1].id, "abc:1");
    }

    // An invented timestamp reorders real history, because the timeline sorts
    // and displays this field. Dropping the message is the honest failure.
    #[test]
    fn a_message_without_a_timestamp_is_dropped_rather_than_dated_now() {
        let messages = vec![TranscriptMessage {
            role: TranscriptRole::User,
            timestamp: None,
            lines: vec!["undated".into()],
        }];
        assert!(transcript_view_messages("abc", &messages).is_empty());
    }

    /// THE CROSS-LANGUAGE CONTRACT. These key names are what
    /// `third_party/t3code-timeline/src/mount.tsx` destructures. A rename on
    /// either side produces an empty timeline and no error anywhere, so it is
    /// pinned by exact JSON.
    #[test]
    fn view_json_matches_the_shape_the_renderer_destructures() {
        let messages = vec![TranscriptMessage {
            role: TranscriptRole::Assistant,
            timestamp: Some("2026-07-25T12:00:00.000Z".into()),
            lines: vec!["hello".into()],
        }];
        let json = serde_json::to_string(&transcript_view_messages("s", &messages)).unwrap();
        assert_eq!(
            json,
            r#"[{"id":"s:0","role":"assistant","text":"hello","createdAt":"2026-07-25T12:00:00.000Z"}]"#
        );
    }
}
