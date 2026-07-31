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

// ===== the TIMELINE model =====
//
// A transcript is not a list of things people said. Roughly 57% of a Codex
// rollout and 96% of a Claude Code JSONL is the agent WORKING — commands it
// ran, files it changed, what it was thinking — and the flat
// `TranscriptMessage` model dropped all of it silently. A session view built on
// that model can only ever show half a conversation, which is why the web
// surface reads as stale next to the terminal beside it.
//
// So the reader's primary output is a TIMELINE of typed entries, and
// `TranscriptMessage` becomes a PROJECTION of it (`transcript_messages_from_entries`).
// One decode per CLI, two views over it — never two parsers.

/// What a timeline entry IS. The axis the flat message model never had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptEntryKind {
    /// Prose a person or the agent addressed to the other. `lines` is markdown.
    Message,
    /// The agent's own thinking. Both CLIs render this collapsed and so do we.
    Reasoning,
    /// A tool the agent ran: a command, a file edit, a search, an MCP call.
    ToolCall,
}

/// The tool half of a `ToolCall` entry.
///
/// `headline` is the ONE line a folded block shows and is the whole reason this
/// type exists: a tool call the reader cannot summarise in a line is a tool call
/// the user has to expand to identify, which defeats folding.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptToolCall {
    /// The tool's own name, as the CLI wrote it (`Bash`, `Edit`, `exec_command`,
    /// `apply_patch`). Never translated — a name the user cannot find in their
    /// CLI's own output is a name we invented.
    pub tool: String,
    /// One line, folded state. The command, the path, the query.
    pub headline: String,
    /// The body, shown when expanded. Output, arguments, the patch.
    pub detail: Vec<String>,
    /// Files this call changed, if it changed any.
    pub changed_files: Vec<String>,
    pub added_lines: usize,
    pub removed_lines: usize,
    /// The CLI reported this call as failed.
    pub failed: bool,
}

/// One entry on the timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub kind: TranscriptEntryKind,
    pub role: TranscriptRole,
    pub timestamp: Option<String>,
    /// Message/reasoning text. Empty for a tool call — its text lives on `tool`.
    pub lines: Vec<String>,
    pub tool: Option<TranscriptToolCall>,
    /// The CLI's own call id, used ONLY to pair a call with its output record.
    /// Never displayed.
    pub call_id: Option<String>,
}

impl TranscriptEntry {
    fn message(role: TranscriptRole, lines: Vec<String>, timestamp: Option<String>) -> Self {
        Self {
            kind: TranscriptEntryKind::Message,
            role,
            timestamp,
            lines,
            tool: None,
            call_id: None,
        }
    }

    fn reasoning(lines: Vec<String>, timestamp: Option<String>) -> Self {
        Self {
            kind: TranscriptEntryKind::Reasoning,
            role: TranscriptRole::Assistant,
            timestamp,
            lines,
            tool: None,
            call_id: None,
        }
    }

    fn tool_call(
        tool: TranscriptToolCall,
        timestamp: Option<String>,
        call_id: Option<String>,
    ) -> Self {
        Self {
            kind: TranscriptEntryKind::ToolCall,
            role: TranscriptRole::Assistant,
            timestamp,
            lines: Vec::new(),
            tool: Some(tool),
            call_id,
        }
    }
}

/// Project a timeline back onto the flat message model.
///
/// This is what keeps `TranscriptMessage` honest now that it is no longer parsed
/// directly: title/précis/summary generation, search fragments and the sidebar's
/// shallow preview all consume THIS, so they see exactly the messages the
/// timeline shows and cannot drift from it. Reasoning and tool calls are
/// deliberately absent — they are not what the session SAID, and feeding a
/// command log to a summariser produced titles about `rg`.
pub fn transcript_messages_from_entries(entries: &[TranscriptEntry]) -> Vec<TranscriptMessage> {
    let mut messages = Vec::new();
    for entry in entries {
        if entry.kind != TranscriptEntryKind::Message {
            continue;
        }
        push_message_lines(
            &mut messages,
            entry.role,
            entry.lines.clone(),
            entry.timestamp.clone(),
        );
    }
    messages
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
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line.with_context(|| format!("failed to read line from {}", path.display()))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        entries.clear();
        codex_entries_from_record(&value, &mut entries);
        for entry in entries.drain(..) {
            if entry.kind != TranscriptEntryKind::Message {
                continue;
            }
            push_message_lines(&mut messages, entry.role, entry.lines, entry.timestamp);
            if max_messages.is_some_and(|limit| messages.len() >= limit) {
                return Ok(messages);
            }
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

// ===== the CODEX decoder — ONE owner of "what is in a rollout record" =====
//
// Every Codex reader in this file drives this function: the whole-file message
// reader, the head-limited reader, the tail window, and the timeline reader.
// The match arms used to be transcribed twice, verbatim, in two functions that
// then drifted in their `max_messages` handling; a third copy for the timeline
// would have been the point where they stopped agreeing about what a transcript
// contains.

/// Decode ONE rollout record into the timeline entries it carries.
///
/// A record can yield several entries (`compacted` replays a whole history), or
/// none, or it can MUTATE an entry already in `out` — a `function_call_output`
/// is not an entry of its own, it is the result half of the call above it.
fn codex_entries_from_record(value: &Value, out: &mut Vec<TranscriptEntry>) {
    let record_timestamp = extract_timestamp_raw(value);
    match value.get("type").and_then(Value::as_str) {
        Some("response_item") => {
            let Some(payload) = value.get("payload") else {
                return;
            };
            let timestamp = extract_timestamp_raw(payload).or_else(|| record_timestamp.clone());
            match payload.get("type").and_then(Value::as_str) {
                Some("message") => {
                    let lines = message_lines_from_payload(payload);
                    if !lines.is_empty() {
                        out.push(TranscriptEntry::message(
                            normalized_message_role(payload),
                            lines,
                            timestamp,
                        ));
                    }
                }
                Some("reasoning") => {
                    // `summary` is the only readable half; `encrypted_content` is
                    // opaque by design and must never be surfaced as text.
                    let lines = payload
                        .get("summary")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(extract_text_fragment)
                                .flat_map(normalize_preview_text)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if !lines.is_empty() {
                        out.push(TranscriptEntry::reasoning(lines, timestamp));
                    }
                }
                Some("function_call") => {
                    let name = payload
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string();
                    let arguments = payload.get("arguments").and_then(Value::as_str).unwrap_or("");
                    out.push(TranscriptEntry::tool_call(
                        codex_tool_call(&name, arguments),
                        timestamp,
                        payload
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    ));
                }
                Some("custom_tool_call") => {
                    let name = payload
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string();
                    let input = payload.get("input").and_then(Value::as_str).unwrap_or("");
                    out.push(TranscriptEntry::tool_call(
                        codex_tool_call(&name, input),
                        timestamp,
                        payload
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    ));
                }
                Some("web_search_call") => {
                    let query = payload
                        .get("action")
                        .and_then(|action| action.get("query"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    out.push(TranscriptEntry::tool_call(
                        TranscriptToolCall {
                            tool: "web_search".to_string(),
                            headline: query.to_string(),
                            ..TranscriptToolCall::default()
                        },
                        timestamp,
                        payload
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    ));
                }
                Some("function_call_output") | Some("custom_tool_call_output") => {
                    let call_id = payload.get("call_id").and_then(Value::as_str);
                    attach_tool_output(out, call_id, tool_output_lines(payload.get("output")));
                }
                _ => {}
            }
        }
        Some("compacted") => {
            let Some(history) = value
                .get("payload")
                .and_then(|payload| payload.get("replacement_history"))
                .and_then(Value::as_array)
            else {
                return;
            };
            for item in history {
                if item.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                let lines = message_lines_from_payload(item);
                if lines.is_empty() {
                    continue;
                }
                out.push(TranscriptEntry::message(
                    normalized_message_role(item),
                    lines,
                    extract_timestamp_raw(item).or_else(|| record_timestamp.clone()),
                ));
            }
        }
        Some("event_msg") => {
            let Some(payload) = value.get("payload") else {
                return;
            };
            match payload.get("type").and_then(Value::as_str) {
                // The DIFF record. It arrives after the `apply_patch` call it
                // belongs to, so the stat lands on that call rather than
                // becoming a second block saying the same thing.
                Some("patch_apply_end") => {
                    let call_id = payload.get("call_id").and_then(Value::as_str);
                    let changed = payload
                        .get("changes")
                        .and_then(Value::as_object)
                        .map(|changes| {
                            let mut added = 0usize;
                            let mut removed = 0usize;
                            let files = changes
                                .iter()
                                .map(|(path, change)| {
                                    if let Some(diff) =
                                        change.get("unified_diff").and_then(Value::as_str)
                                    {
                                        let (plus, minus) = unified_diff_stat(diff);
                                        added += plus;
                                        removed += minus;
                                    } else if let Some(content) =
                                        change.get("content").and_then(Value::as_str)
                                    {
                                        added += content.lines().count();
                                    }
                                    path.clone()
                                })
                                .collect::<Vec<_>>();
                            (files, added, removed)
                        });
                    let Some((files, added, removed)) = changed else {
                        return;
                    };
                    let failed = payload.get("success").and_then(Value::as_bool) == Some(false);
                    attach_tool_change_stat(out, call_id, files, added, removed, failed);
                }
                _ => {
                    let Some((role, text)) = event_message_role_and_text(payload) else {
                        return;
                    };
                    let lines = normalize_preview_text(text);
                    if !lines.is_empty() {
                        out.push(TranscriptEntry::message(role, lines, record_timestamp));
                    }
                }
            }
        }
        _ => {}
    }
}

/// A tool call's folded line, from the CLI's own argument blob.
///
/// The argument blob is JSON for `function_call` and free text for
/// `custom_tool_call` (`apply_patch` sends a patch), so this tries JSON first
/// and treats the raw text as the headline when that fails. The keys tried are
/// the ones Codex actually writes — `cmd`, `command`, `path`, `query` — and the
/// fallback is the tool's own name, never an invented phrase.
fn codex_tool_call(name: &str, arguments: &str) -> TranscriptToolCall {
    let mut call = TranscriptToolCall {
        tool: name.to_string(),
        ..TranscriptToolCall::default()
    };
    match serde_json::from_str::<Value>(arguments) {
        Ok(parsed) => {
            call.headline = first_argument_headline(&parsed).unwrap_or_default();
            call.detail = normalize_preview_text(
                &serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| arguments.to_string()),
            );
        }
        Err(_) => {
            let mut lines = arguments.lines();
            call.headline = lines.next().unwrap_or_default().trim().to_string();
            call.detail = normalize_preview_text(arguments);
            // A patch body names its own files; reading them here means the
            // folded line can say WHICH file before the diff record arrives.
            call.changed_files = apply_patch_files(arguments);
        }
    }
    call
}

fn first_argument_headline(parsed: &Value) -> Option<String> {
    for key in ["cmd", "command", "file_path", "path", "query", "description"] {
        match parsed.get(key) {
            Some(Value::String(text)) => return Some(text.trim().to_string()),
            Some(Value::Array(items)) => {
                let joined = items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ");
                if !joined.trim().is_empty() {
                    return Some(joined);
                }
            }
            _ => {}
        }
    }
    None
}

/// The files an `apply_patch` body touches, from its own `*** … File:` markers.
fn apply_patch_files(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix("*** ")?;
            for marker in ["Add File: ", "Update File: ", "Delete File: ", "Move to: "] {
                if let Some(path) = rest.strip_prefix(marker) {
                    return Some(path.trim().to_string());
                }
            }
            None
        })
        .collect()
}

/// `+`/`-` counts from a unified diff, ignoring the `+++`/`---` headers.
fn unified_diff_stat(diff: &str) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

/// A tool output payload is either a string or a block array; both CLIs use both.
fn tool_output_lines(output: Option<&Value>) -> Vec<String> {
    let Some(output) = output else {
        return Vec::new();
    };
    if let Some(text) = output.as_str() {
        return normalize_preview_text(text);
    }
    if let Some(items) = output.as_array() {
        return items
            .iter()
            .filter_map(extract_text_fragment)
            .flat_map(normalize_preview_text)
            .collect();
    }
    extract_text_fragment(output)
        .map(normalize_preview_text)
        .unwrap_or_default()
}

/// How many output lines a folded/expanded tool block keeps.
///
/// A single `cargo build` output is tens of thousands of lines; carrying it into
/// a snapshot that crosses an IPC boundary once per refresh is how a transcript
/// view becomes a performance bug. The head is kept rather than the tail because
/// a command's first lines say what it did; a truncation note replaces the rest.
const TOOL_OUTPUT_LINE_BUDGET: usize = 40;

fn clamp_tool_output(mut lines: Vec<String>) -> Vec<String> {
    if lines.len() > TOOL_OUTPUT_LINE_BUDGET {
        let dropped = lines.len() - TOOL_OUTPUT_LINE_BUDGET;
        lines.truncate(TOOL_OUTPUT_LINE_BUDGET);
        lines.push(format!("… {dropped} more lines"));
    }
    lines
}

/// Attach an output record to the call it answers.
///
/// Matched by `call_id` when there is one, else to the most recent tool call
/// still without output — the CLIs interleave calls and results strictly, so
/// "most recent" is right, and a mismatched id attaches to nothing rather than
/// to the wrong call.
fn attach_tool_output(out: &mut [TranscriptEntry], call_id: Option<&str>, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    let Some(entry) = find_tool_call_mut(out, call_id) else {
        return;
    };
    if let Some(tool) = entry.tool.as_mut() {
        tool.detail = clamp_tool_output(lines);
    }
}

fn attach_tool_change_stat(
    out: &mut [TranscriptEntry],
    call_id: Option<&str>,
    files: Vec<String>,
    added: usize,
    removed: usize,
    failed: bool,
) {
    let Some(entry) = find_tool_call_mut(out, call_id) else {
        return;
    };
    if let Some(tool) = entry.tool.as_mut() {
        tool.changed_files = files;
        tool.added_lines = added;
        tool.removed_lines = removed;
        tool.failed = failed;
    }
}

fn find_tool_call_mut<'a>(
    out: &'a mut [TranscriptEntry],
    call_id: Option<&str>,
) -> Option<&'a mut TranscriptEntry> {
    out.iter_mut().rev().find(|entry| {
        entry.kind == TranscriptEntryKind::ToolCall
            && match call_id {
                Some(id) => entry.call_id.as_deref() == Some(id),
                None => true,
            }
    })
}

fn parse_transcript_message_lines<'a, I>(lines: I, max_messages: usize) -> Vec<TranscriptMessage>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut messages = VecDeque::new();
    let mut entries = Vec::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        entries.clear();
        codex_entries_from_record(&value, &mut entries);
        for entry in entries.drain(..) {
            if entry.kind != TranscriptEntryKind::Message {
                continue;
            }
            push_message_lines_deque(
                &mut messages,
                entry.role,
                entry.lines,
                entry.timestamp,
                max_messages,
            );
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
    Ok(transcript_messages_from_entries(
        &read_claude_code_transcript_entries(path)?,
    ))
}

/// Read a Claude Code transcript as a TIMELINE — prose, thinking and tool calls.
///
/// The doc comment above lists what the message projection drops and why. Those
/// reasons were about the flat model: `thinking` and `tool_use` are not prose,
/// and emitting them AS prose produced a wall of JSON. They are not dropped
/// here, because the timeline has a place to put them — a reasoning entry the
/// reader keeps folded, and a tool entry that shows one line until asked.
pub fn read_claude_code_transcript_entries(path: &Path) -> Result<Vec<TranscriptEntry>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read claude code transcript {}", path.display()))?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        claude_code_entries_from_record(&value, &mut entries);
    }
    Ok(entries)
}

/// Decode ONE Claude Code record into the timeline entries it carries.
///
/// A CC record is one message whose `content` is a list of blocks, and the
/// blocks are the interesting part: a single assistant record routinely carries
/// thinking, prose and a tool call at once. A user record carries the RESULTS of
/// the tool calls above it, which is why this can mutate `out` rather than only
/// append to it.
fn claude_code_entries_from_record(value: &Value, out: &mut Vec<TranscriptEntry>) {
    let role = match value.get("type").and_then(Value::as_str) {
        Some("user") => TranscriptRole::User,
        Some("assistant") => TranscriptRole::Assistant,
        _ => return,
    };
    if value.get("isMeta").and_then(Value::as_bool) == Some(true)
        || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return;
    }
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(message) = value.get("message") else {
        return;
    };

    // A bare-string `content` is the user typing. No blocks to walk.
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        let lines = normalize_preview_text(strip_local_command_caveat(text));
        if !lines.is_empty() && !lines_are_only_command_plumbing(&lines) {
            out.push(TranscriptEntry::message(role, lines, timestamp));
        }
        return;
    }

    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return;
    };
    // The rich result record: `toolUseResult` carries the structured patch a
    // `tool_result` block only describes in prose.
    let tool_use_result = value.get("toolUseResult");
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let Some(text) = block.get("text").and_then(Value::as_str) else {
                    continue;
                };
                let lines = normalize_preview_text(strip_local_command_caveat(text));
                if lines.is_empty() || lines_are_only_command_plumbing(&lines) {
                    continue;
                }
                out.push(TranscriptEntry::message(role, lines, timestamp.clone()));
            }
            Some("thinking") => {
                let Some(text) = block.get("thinking").and_then(Value::as_str) else {
                    continue;
                };
                let lines = normalize_preview_text(text);
                if lines.is_empty() {
                    continue;
                }
                out.push(TranscriptEntry::reasoning(lines, timestamp.clone()));
            }
            Some("tool_use") => {
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                out.push(TranscriptEntry::tool_call(
                    claude_code_tool_call(&name, &input),
                    timestamp.clone(),
                    block
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                ));
            }
            Some("tool_result") => {
                let call_id = block.get("tool_use_id").and_then(Value::as_str);
                attach_tool_output(out, call_id, tool_output_lines(block.get("content")));
                if block.get("is_error").and_then(Value::as_bool) == Some(true)
                    && let Some(entry) = find_tool_call_mut(out, call_id)
                    && let Some(tool) = entry.tool.as_mut()
                {
                    tool.failed = true;
                }
                if let Some(result) = tool_use_result
                    && let Some((files, added, removed)) = claude_code_change_stat(result)
                {
                    attach_tool_change_stat(out, call_id, files, added, removed, false);
                }
            }
            _ => {}
        }
    }
}

/// A CC tool call's folded line, from the tool's own input object.
fn claude_code_tool_call(name: &str, input: &Value) -> TranscriptToolCall {
    let headline = first_argument_headline(input)
        .or_else(|| {
            input
                .get("pattern")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| input.get("url").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    // `changed_files` is deliberately NOT filled from `file_path` here. A `Read`
    // names a file too, and a chip row that says "changed" under a call that only
    // looked is a lie the headline already told the truth about. The changed set
    // arrives from the RESULT record's `structuredPatch`, which only a call that
    // actually wrote something has.
    TranscriptToolCall {
        tool: name.to_string(),
        headline: headline.lines().next().unwrap_or_default().trim().to_string(),
        detail: normalize_preview_text(
            &serde_json::to_string_pretty(input).unwrap_or_else(|_| String::new()),
        ),
        ..TranscriptToolCall::default()
    }
}

/// `+`/`-` counts from CC's `structuredPatch` hunks.
fn claude_code_change_stat(result: &Value) -> Option<(Vec<String>, usize, usize)> {
    let hunks = result.get("structuredPatch").and_then(Value::as_array)?;
    let mut added = 0usize;
    let mut removed = 0usize;
    for hunk in hunks {
        let Some(lines) = hunk.get("lines").and_then(Value::as_array) else {
            continue;
        };
        for line in lines.iter().filter_map(Value::as_str) {
            if line.starts_with('+') {
                added += 1;
            } else if line.starts_with('-') {
                removed += 1;
            }
        }
    }
    let files = result
        .get("filePath")
        .and_then(Value::as_str)
        .map(|path| vec![path.to_string()])
        .unwrap_or_default();
    Some((files, added, removed))
}

/// Read a Codex transcript as a TIMELINE.
pub fn read_codex_transcript_entries(path: &Path) -> Result<Vec<TranscriptEntry>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read session transcript {}", path.display()))?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        codex_entries_from_record(&value, &mut entries);
    }
    Ok(entries)
}

/// Read whichever agent CLI owns `path`, as MESSAGES.
///
/// The message-shaped door onto the one dispatch. It exists so a caller that
/// only wants prose does not have to re-answer "which CLI wrote this file" with
/// a `match` of its own — which is how the loopback transcript server came to
/// carry the second copy of that decision.
pub fn read_agent_transcript_messages(path: &Path) -> Result<Vec<TranscriptMessage>> {
    Ok(transcript_messages_from_entries(
        &read_agent_transcript_entries(path)?,
    ))
}

/// Read whichever agent CLI owns `path`, as a timeline.
///
/// The dispatch is the agent-CLI registry's job, never a per-caller `match` —
/// this is the exact bug the session web view was built on: every consumer but
/// one called the CODEX reader unconditionally, and a Claude Code JSONL shares
/// no record type with a Codex rollout, so those calls returned `Ok(vec![])`
/// SILENTLY. A CC session's web view then fell through to a hardcoded
/// "Resume Codex session <uuid>." placeholder, which is what the surface has
/// been showing.
pub fn read_agent_transcript_entries(path: &Path) -> Result<Vec<TranscriptEntry>> {
    match transcript_reader_kind(path) {
        TranscriptReaderKind::Codex => read_codex_transcript_entries(path),
        TranscriptReaderKind::ClaudeCode => read_claude_code_transcript_entries(path),
    }
}

/// The TAIL of a transcript, at most `max_entries` entries.
///
/// The reason a transcript view needs this: a long session's JSONL is tens of
/// megabytes and the reader runs on every snapshot refresh. Reading the whole
/// file to show the last screen is the difference between a surface that opens
/// and one that hitches. Windowing at the BYTE level and re-widening (the shape
/// `read_codex_transcript_messages_tail_limited` already uses) keeps that cost
/// proportional to what is shown rather than to what exists.
pub fn read_agent_transcript_entries_tail_limited(
    path: &Path,
    max_entries: usize,
) -> Result<Vec<TranscriptEntry>> {
    const INITIAL_WINDOW_BYTES: u64 = 2 * 1024 * 1024;
    const MAX_WINDOW_BYTES: u64 = 64 * 1024 * 1024;

    let kind = transcript_reader_kind(path);
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
        // A window that did not start at byte 0 begins mid-record; that partial
        // line is dropped rather than parsed into a half-entry.
        let lines: Vec<&str> = if start > 0 {
            text.lines().skip(1).collect()
        } else {
            text.lines().collect()
        };

        let mut entries = Vec::new();
        for line in lines {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match kind {
                TranscriptReaderKind::Codex => codex_entries_from_record(&value, &mut entries),
                TranscriptReaderKind::ClaudeCode => {
                    claude_code_entries_from_record(&value, &mut entries)
                }
            }
        }
        if entries.len() >= max_entries || start == 0 || window >= MAX_WINDOW_BYTES {
            // Keep the TAIL: the newest entries are the ones a reader opens on.
            if entries.len() > max_entries {
                entries.drain(..entries.len() - max_entries);
            }
            return Ok(entries);
        }
        window = (window.saturating_mul(2))
            .min(MAX_WINDOW_BYTES)
            .min(file_len.max(1));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptReaderKind {
    Codex,
    ClaudeCode,
}

/// Which CLI's reader owns this file.
///
/// The agent-CLI registry answers first and is authoritative: it is the one
/// place that knows where each CLI keeps its sessions.
///
/// A path the registry cannot name — a transcript copied out of its store, a
/// fixture, a `local://<uuid>` row resolved by hand — is NOT guessed. Picking a
/// reader by hope is the exact failure this lane exists to fix: the wrong reader
/// returns `Ok(vec![])` and the caller cannot tell "empty session" from "wrong
/// parser". So the file is SNIFFED, which answers a different question (what
/// FORMAT is this?) from a different source (the bytes), and cannot silently
/// disagree with the registry because it is only consulted when the registry
/// declined.
fn transcript_reader_kind(path: &Path) -> TranscriptReaderKind {
    match crate::agent_cli_for_store_session_file(&path.display().to_string())
        .map(|descriptor| descriptor.kind)
    {
        Some(crate::SessionKind::Codex) | Some(crate::SessionKind::CodexLiteLlm) => {
            return TranscriptReaderKind::Codex;
        }
        Some(crate::SessionKind::ClaudeCode) => return TranscriptReaderKind::ClaudeCode,
        _ => {}
    }
    sniff_transcript_reader_kind(path)
}

/// How many leading records the format sniff reads. Both formats declare
/// themselves on their FIRST record (`session_meta` / a `user` turn); the margin
/// covers a file whose head is a record type neither branch names.
const TRANSCRIPT_SNIFF_RECORD_BUDGET: usize = 24;

/// The format of a transcript whose path the registry could not name.
///
/// Codex tags its records `session_meta` / `response_item` / `event_msg` /
/// `compacted`; Claude Code tags them `user` / `assistant` and nests the turn
/// under `message`. The two vocabularies do not overlap, so the first record
/// that matches either decides. A file that matches NEITHER falls to Claude
/// Code, which is the same answer as before this function existed — an honest
/// default, not a claim.
fn sniff_transcript_reader_kind(path: &Path) -> TranscriptReaderKind {
    let Ok(file) = fs::File::open(path) else {
        return TranscriptReaderKind::ClaudeCode;
    };
    for line in BufReader::new(file).lines().take(TRANSCRIPT_SNIFF_RECORD_BUDGET) {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("session_meta" | "response_item" | "event_msg" | "compacted" | "turn_context") => {
                return TranscriptReaderKind::Codex;
            }
            Some("user" | "assistant") if value.get("message").is_some() => {
                return TranscriptReaderKind::ClaudeCode;
            }
            _ => {}
        }
    }
    TranscriptReaderKind::ClaudeCode
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
mod timeline_tests {
    use super::*;

    fn write_at(dir: &std::path::Path, name: &str, lines: &[&str]) -> std::path::PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    fn scratch_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "yggterm-timeline-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// A Codex rollout is 57% tool activity, and the reader used to drop all of
    /// it: `payload.type != "message" => continue`. THE lock on that: a call
    /// must arrive as its own entry, wearing its command as a headline, its
    /// output attached from the SEPARATE record that carries it, and — when a
    /// patch landed — the diff stat from the `event_msg` that reports it.
    #[test]
    fn a_codex_tool_call_carries_its_command_its_output_and_its_diff_stat() {
        let root = scratch_root("codex").join(".codex/sessions/2026/08/01");
        let path = write_at(
            &root,
            "rollout-2026-08-01T00-00-00-abc.jsonl",
            &[
                r#"{"timestamp":"2026-08-01T00:00:00.000Z","type":"event_msg","payload":{"type":"user_message","message":"rename the flag"}}"#,
                r#"{"timestamp":"2026-08-01T00:00:01.000Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call_1","arguments":"{\"cmd\":\"rg -n needle src\",\"workdir\":\"/repo\"}"}}"#,
                r#"{"timestamp":"2026-08-01T00:00:02.000Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":[{"type":"text","text":"src/lib.rs:12: needle"}]}}"#,
                r#"{"timestamp":"2026-08-01T00:00:03.000Z","type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","call_id":"call_2","input":"*** Begin Patch\n*** Update File: /repo/src/lib.rs\n*** End Patch"}}"#,
                r#"{"timestamp":"2026-08-01T00:00:04.000Z","type":"event_msg","payload":{"type":"patch_apply_end","call_id":"call_2","success":true,"changes":{"/repo/src/lib.rs":{"type":"update","unified_diff":"--- a\n+++ b\n+added one\n+added two\n-removed one"}}}}"#,
                r#"{"timestamp":"2026-08-01T00:00:05.000Z","type":"event_msg","payload":{"type":"agent_message","message":"Renamed it."}}"#,
            ],
        );

        let entries = read_agent_transcript_entries(&path).unwrap();
        let tools = entries
            .iter()
            .filter(|entry| entry.kind == TranscriptEntryKind::ToolCall)
            .collect::<Vec<_>>();
        assert_eq!(tools.len(), 2, "both calls must survive: {entries:?}");

        let exec = tools[0].tool.as_ref().unwrap();
        assert_eq!(exec.tool, "exec_command");
        assert_eq!(exec.headline, "rg -n needle src");
        assert!(
            exec.detail.iter().any(|line| line.contains("src/lib.rs:12")),
            "the output record must attach to its call: {exec:?}"
        );

        let patch = tools[1].tool.as_ref().unwrap();
        assert_eq!(patch.tool, "apply_patch");
        assert_eq!(patch.changed_files, vec!["/repo/src/lib.rs".to_string()]);
        assert_eq!(
            (patch.added_lines, patch.removed_lines),
            (2, 1),
            "the `+++`/`---` headers are not changes: {patch:?}"
        );
        assert!(!patch.failed);

        // …and the prose is still exactly the two turns, in order.
        let messages = transcript_messages_from_entries(&entries);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, TranscriptRole::User);
        assert_eq!(messages[1].lines, vec!["Renamed it.".to_string()]);
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    /// The same lock for Claude Code, whose tool activity lives in CONTENT
    /// BLOCKS rather than in records of its own — one assistant record routinely
    /// carries thinking, prose and a call at once — and whose diff stat lives on
    /// the RESULT record's `structuredPatch`.
    #[test]
    fn a_claude_code_record_yields_thinking_prose_and_the_call_it_made() {
        let root = scratch_root("cc").join(".claude/projects/-repo");
        let path = write_at(
            &root,
            "session.jsonl",
            &[
                r#"{"type":"user","timestamp":"2026-08-01T00:00:00.000Z","message":{"role":"user","content":"rename the flag"}}"#,
                r#"{"type":"assistant","timestamp":"2026-08-01T00:00:01.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"the flag is in lib.rs"},{"type":"text","text":"Editing it now."},{"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"/repo/src/lib.rs","old_string":"a","new_string":"b"}}]}}"#,
                r#"{"type":"user","timestamp":"2026-08-01T00:00:02.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"applied"}]},"toolUseResult":{"filePath":"/repo/src/lib.rs","structuredPatch":[{"lines":["-a","+b","+c"," ctx"]}]}}"#,
            ],
        );

        let entries = read_agent_transcript_entries(&path).unwrap();
        let kinds = entries.iter().map(|entry| entry.kind).collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                TranscriptEntryKind::Message,
                TranscriptEntryKind::Reasoning,
                TranscriptEntryKind::Message,
                TranscriptEntryKind::ToolCall,
            ],
            "one record carries three blocks, in the order it wrote them: {entries:?}"
        );

        let tool = entries[3].tool.as_ref().unwrap();
        assert_eq!(tool.tool, "Edit");
        assert_eq!(tool.headline, "/repo/src/lib.rs");
        assert!(
            tool.detail.iter().any(|line| line.contains("applied")),
            "the tool_result block must attach to its call: {tool:?}"
        );
        assert_eq!(
            (tool.added_lines, tool.removed_lines),
            (2, 1),
            "a context line is not a change: {tool:?}"
        );

        // The prose projection is the two message entries and nothing else —
        // feeding a command log to the summariser produced titles about `rg`.
        let messages = transcript_messages_from_entries(&entries);
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert_eq!(messages[0].lines, vec!["rename the flag".to_string()]);
        assert_eq!(messages[1].lines, vec!["Editing it now.".to_string()]);
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    /// THE bug this lane exists to close: every transcript consumer but one
    /// called the CODEX reader unconditionally, and the two formats share no
    /// record type, so a Claude Code file parsed to `Ok(vec![])` SILENTLY.
    ///
    /// The lock is deliberately stated as "each file is read by its own CLI's
    /// reader, keyed by the registry", not "CC files work" — pointing either
    /// reader at the other's file must yield nothing, which is what made the
    /// failure invisible.
    #[test]
    fn a_transcript_is_read_by_the_reader_its_own_cli_registered() {
        let cc_root = scratch_root("dispatch-cc").join(".claude/projects/-repo");
        let cc = write_at(
            &cc_root,
            "session.jsonl",
            &[
                r#"{"type":"user","timestamp":"2026-08-01T00:00:00.000Z","message":{"role":"user","content":"hello from claude code"}}"#,
            ],
        );
        let codex_root = scratch_root("dispatch-codex").join(".codex/sessions/2026/08/01");
        let codex = write_at(
            &codex_root,
            "rollout-2026-08-01T00-00-00-abc.jsonl",
            &[
                r#"{"timestamp":"2026-08-01T00:00:00.000Z","type":"event_msg","payload":{"type":"user_message","message":"hello from codex"}}"#,
            ],
        );

        assert_eq!(read_agent_transcript_entries(&cc).unwrap().len(), 1);
        assert_eq!(read_agent_transcript_entries(&codex).unwrap().len(), 1);
        // The cross pairing is the silent hole: both are Ok, both are empty.
        assert!(read_codex_transcript_entries(&cc).unwrap().is_empty());
        assert!(
            read_claude_code_transcript_entries(&codex)
                .unwrap()
                .is_empty()
        );
        let _ = fs::remove_dir_all(cc_root.parent().unwrap());
        let _ = fs::remove_dir_all(codex_root.parent().unwrap());
    }

    /// A live session's transcript grows without limit and the reader runs on
    /// every refresh, so the tail reader must cost what is SHOWN. The lock: it
    /// returns the LAST `max` entries — a head-limited read would open the
    /// user's Web View on the start of a conversation they are in the middle of.
    #[test]
    fn the_tail_reader_returns_the_newest_entries() {
        let root = scratch_root("tail").join(".codex/sessions/2026/08/01");
        let lines = (0..50)
            .map(|index| {
                format!(
                    r#"{{"timestamp":"2026-08-01T00:00:00.000Z","type":"event_msg","payload":{{"type":"user_message","message":"turn {index}"}}}}"#
                )
            })
            .collect::<Vec<_>>();
        let path = write_at(
            &root,
            "rollout-2026-08-01T00-00-00-abc.jsonl",
            &lines.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let tail = read_agent_transcript_entries_tail_limited(&path, 5).unwrap();
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[4].lines, vec!["turn 49".to_string()]);
        assert_eq!(tail[0].lines, vec!["turn 45".to_string()]);
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    /// Tool output is unbounded (`cargo build` is tens of thousands of lines)
    /// and it crosses the snapshot IPC boundary once per refresh. The clamp is a
    /// correctness constraint on the surface, not a cosmetic choice, so it is
    /// locked: the HEAD is kept (a command's first lines say what it did) and
    /// the drop is NAMED rather than silent.
    #[test]
    fn tool_output_is_clamped_and_says_how_much_it_dropped() {
        let root = scratch_root("clamp").join(".codex/sessions/2026/08/01");
        let output = (0..500)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\\n");
        let path = write_at(
            &root,
            "rollout-2026-08-01T00-00-00-abc.jsonl",
            &[
                r#"{"timestamp":"2026-08-01T00:00:01.000Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"c1","arguments":"{\"cmd\":\"cargo build\"}"}}"#,
                &format!(
                    r#"{{"timestamp":"2026-08-01T00:00:02.000Z","type":"response_item","payload":{{"type":"function_call_output","call_id":"c1","output":"{output}"}}}}"#
                ),
            ],
        );
        let entries = read_agent_transcript_entries(&path).unwrap();
        let tool = entries[0].tool.as_ref().unwrap();
        assert_eq!(tool.detail.len(), 41, "40 lines plus the note: {tool:?}");
        assert_eq!(tool.detail[0], "line 0", "the HEAD is kept");
        assert_eq!(tool.detail[40], "… 460 more lines");
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }
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
