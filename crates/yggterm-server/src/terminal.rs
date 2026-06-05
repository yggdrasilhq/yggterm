use crate::codex_cli::{
    TerminalIdentityColorProfile, normalize_terminal_identity_color,
    terminal_identity_appearance_from_environment,
    terminal_identity_color_profile_from_environment, terminal_identity_env_pairs,
    terminal_identity_env_removals,
};
use anyhow::{Context, Result, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vt100::Parser as Vt100Parser;
use yggterm_core::{append_trace_event, resolve_yggterm_home};

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 36;
const MAX_CHUNKS: usize = 512;
// Per [[spec-tmux-parity-and-beyond]]: raw-byte retention is the
// substrate for GUI-restart history replay. Was 2 MB pre-2026-05-26;
// bumped to 16 MB so plain-shell sessions retain ~50–100x more history
// (real lines, not redraws). TUI sessions still primarily benefit from
// the daemon-side vt100 scrollback ring (see TerminalScreenState).
pub const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;
/// Per-session daemon-side scrollback ring depth (rows) tracked by the
/// vt100 parser. Mirrors xterm.js's scrollback config in shell.rs.
/// Per [[spec-tmux-parity-and-beyond]] — this is the tmux `history-limit`
/// equivalent. 10 000 rows is the practical sweet spot for shells.
pub const DAEMON_VT_SCROLLBACK_ROWS: usize = 10_000;
pub const IDLE_TRIM_MAX_CHUNKS: usize = 64;
pub const IDLE_TRIM_MAX_BYTES: usize = 128 * 1024;
const INITIAL_ATTACH_MAX_CHUNKS: usize = 192;
const INITIAL_ATTACH_MAX_BYTES: usize = 512 * 1024;
const INITIAL_ATTACH_TRAILING_NOISE_CHUNKS: usize = 16;
const ATTACH_READY_MARKER: &str = "__YGGTERM_ATTACH_READY__\n";
const TERMINAL_WRITE_QUEUE_CAPACITY: usize = 64;
const TERMINAL_WRITE_FLUSH_ACK_TIMEOUT_MS: u64 = 1_500;
const TERMINAL_PROTOCOL_MAX_PENDING_BYTES: usize = 256;
const OSC_PALETTE_CODE: u16 = 4;
const OSC_COLOR_FOREGROUND_CODE: u16 = 10;
const OSC_COLOR_BACKGROUND_CODE: u16 = 11;

#[derive(Debug, Clone)]
pub struct TerminalChunk {
    pub seq: u64,
    pub data: String,
}

/// Outcome of a readiness-gated `TerminalManager::submit_prompt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptSubmitOutcome {
    /// The session reached a ready interactive prompt and `data` was written.
    /// `waited_ms` is how long readiness took (0 if it was already ready).
    Submitted { waited_ms: u64 },
    /// The session never reached a ready prompt within the timeout; NOTHING was
    /// written. The caller should retry later or skip.
    NotReady { waited_ms: u64 },
    /// No such session (key absent).
    NoSession,
}

#[derive(Debug, Clone)]
pub struct TerminalReadResult {
    pub cursor: u64,
    pub chunks: Vec<TerminalChunk>,
    pub running: bool,
    pub runtime_output_seen: bool,
    pub eof_without_output: bool,
    pub post_resize_output_seen: bool,
    pub last_resize_seq: u64,
    /// True when the live chunk ring trimmed chunks BELOW this read's cursor, so
    /// the returned `chunks` skip a contiguous middle range (the client fell behind
    /// the ring while output kept flowing — e.g. a backgrounded session streaming
    /// past MAX_CHUNKS). The bytes are gone from the raw ring but recoverable from
    /// the daemon vt100 scrollback (DAEMON_VT_SCROLLBACK_ROWS) via a clean
    /// re-attach. The client MUST treat this as "re-sync required" (re-attach at
    /// cursor 0) rather than appending the discontiguous chunks. Without this flag
    /// the gap was SILENT — the middle simply vanished
    /// (docs/xterm-bugs.md#chunk-ring-trim-drops-mid-stream).
    pub resync_required: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalBufferStats {
    pub session_count: usize,
    pub retained_chunks: usize,
    pub retained_bytes: usize,
}

fn decode_terminal_utf8_chunk(pending: &mut Vec<u8>, bytes: &[u8]) -> String {
    pending.extend_from_slice(bytes);
    let mut decoded = String::new();
    loop {
        match std::str::from_utf8(pending) {
            Ok(text) => {
                decoded.push_str(text);
                pending.clear();
                break;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    decoded.push_str(
                        std::str::from_utf8(&pending[..valid_up_to]).expect("valid UTF-8 prefix"),
                    );
                    pending.drain(..valid_up_to);
                    continue;
                }
                if let Some(error_len) = error.error_len() {
                    decoded.push('\u{fffd}');
                    pending.drain(..error_len);
                    continue;
                }
                break;
            }
        }
    }
    decoded
}

fn flush_terminal_utf8_pending(pending: &mut Vec<u8>) -> String {
    if pending.is_empty() {
        return String::new();
    }
    let decoded = String::from_utf8_lossy(pending).to_string();
    pending.clear();
    decoded
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalProtocolProfile {
    appearance: &'static str,
    foreground: (u8, u8, u8),
    background: (u8, u8, u8),
    palette: [(u8, u8, u8); 16],
}

impl TerminalProtocolProfile {
    fn from_launch_command(launch_command: &str) -> Self {
        let appearance = infer_terminal_appearance_from_launch_command(launch_command)
            .or_else(
                || match terminal_identity_appearance_from_environment().as_str() {
                    "dark" => Some("dark"),
                    "light" => Some("light"),
                    _ => None,
                },
            )
            .unwrap_or("light");
        let base = match appearance {
            "dark" => Self {
                appearance: "dark",
                foreground: (0xcc, 0xcc, 0xcc),
                background: (0x1e, 0x1e, 0x1e),
                palette: TERMINAL_PROTOCOL_DARK_PALETTE,
            },
            _ => Self {
                appearance: "light",
                foreground: (0x15, 0x1b, 0x23),
                background: (0xfb, 0xfb, 0xfd),
                palette: TERMINAL_PROTOCOL_LIGHT_PALETTE,
            },
        };
        terminal_identity_color_profile_from_launch_command(launch_command)
            .or_else(terminal_identity_color_profile_from_environment)
            .and_then(|profile| base.with_color_profile(&profile))
            .unwrap_or(base)
    }

    fn with_color_profile(self, profile: &TerminalIdentityColorProfile) -> Option<Self> {
        if profile.palette.len() != 16 {
            return None;
        }
        let mut palette = [(0u8, 0u8, 0u8); 16];
        for (index, value) in profile.palette.iter().enumerate() {
            palette[index] = parse_terminal_protocol_hex_color(value)?;
        }
        Some(Self {
            appearance: self.appearance,
            foreground: parse_terminal_protocol_hex_color(&profile.foreground)?,
            background: parse_terminal_protocol_hex_color(&profile.background)?,
            palette,
        })
    }

    fn osc_color_response(self, query: TerminalProtocolColorQuery) -> Option<String> {
        let color = match query.code {
            OSC_COLOR_FOREGROUND_CODE => self.foreground,
            OSC_COLOR_BACKGROUND_CODE => self.background,
            OSC_PALETTE_CODE => *self.palette.get(usize::from(query.slot))?,
            _ => return None,
        };
        let response_slot = if query.code == OSC_PALETTE_CODE {
            query.slot.to_string()
        } else {
            query.code.to_string()
        };
        Some(format!(
            "\u{1b}]{};rgb:{}/{}/{}\u{1b}\\",
            if query.code == OSC_PALETTE_CODE {
                format!("4;{response_slot}")
            } else {
                response_slot
            },
            osc_rgb_component(color.0),
            osc_rgb_component(color.1),
            osc_rgb_component(color.2)
        ))
    }
}

fn terminal_identity_color_profile_from_launch_command(
    launch_command: &str,
) -> Option<TerminalIdentityColorProfile> {
    let foreground =
        launch_command_assignment_value(launch_command, "YGGTERM_TERMINAL_COLOR_FOREGROUND")
            .and_then(|value| normalize_terminal_identity_color(&value))?;
    let background =
        launch_command_assignment_value(launch_command, "YGGTERM_TERMINAL_COLOR_BACKGROUND")
            .and_then(|value| normalize_terminal_identity_color(&value))?;
    let mut palette = Vec::with_capacity(16);
    for index in 0..16 {
        let key = format!("YGGTERM_TERMINAL_COLOR_{index}");
        palette.push(
            launch_command_assignment_value(launch_command, &key)
                .and_then(|value| normalize_terminal_identity_color(&value))?,
        );
    }
    Some(TerminalIdentityColorProfile {
        foreground,
        background,
        palette,
    })
}

fn parse_terminal_protocol_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let normalized = normalize_terminal_identity_color(value)?;
    let hex = normalized.strip_prefix('#')?;
    Some((
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}

const TERMINAL_PROTOCOL_DARK_PALETTE: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xcd, 0x31, 0x31),
    (0x0d, 0xbc, 0x79),
    (0xe5, 0xe5, 0x10),
    (0x24, 0x72, 0xc8),
    (0xbc, 0x3f, 0xbc),
    (0x11, 0xa8, 0xcd),
    (0xe5, 0xe5, 0xe5),
    (0x66, 0x66, 0x66),
    (0xf1, 0x4c, 0x4c),
    (0x23, 0xd1, 0x8b),
    (0xf5, 0xf5, 0x43),
    (0x3b, 0x8e, 0xea),
    (0xd6, 0x70, 0xd6),
    (0x29, 0xbf, 0xd6),
    (0xe5, 0xe5, 0xe5),
];

const TERMINAL_PROTOCOL_LIGHT_PALETTE: [(u8, u8, u8); 16] = [
    (0x24, 0x29, 0x2f),
    (0xa1, 0x26, 0x0d),
    (0x0c, 0x64, 0x28),
    (0x7a, 0x4f, 0x00),
    (0x04, 0x51, 0xa5),
    (0x69, 0x36, 0xaa),
    (0x0e, 0x65, 0x70),
    (0x57, 0x60, 0x6a),
    (0x6e, 0x77, 0x81),
    (0xa1, 0x26, 0x0d),
    (0x0c, 0x64, 0x28),
    (0x74, 0x49, 0x00),
    (0x04, 0x51, 0xa5),
    (0x73, 0x40, 0xb3),
    (0x0e, 0x65, 0x70),
    (0x8c, 0x95, 0x9f),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalProtocolColorQuery {
    code: u16,
    slot: u16,
}

impl TerminalProtocolColorQuery {
    fn label(self) -> String {
        if self.code == OSC_PALETTE_CODE {
            format!("4:{}", self.slot)
        } else {
            self.code.to_string()
        }
    }
}

#[derive(Debug, Default)]
struct TerminalProtocolFilter {
    pending: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct TerminalProtocolFilterResult {
    data: String,
    responses: Vec<String>,
    answered_queries: Vec<TerminalProtocolColorQuery>,
}

impl TerminalProtocolFilter {
    fn process(
        &mut self,
        data: &str,
        profile: TerminalProtocolProfile,
    ) -> TerminalProtocolFilterResult {
        if data.is_empty() && self.pending.is_empty() {
            return TerminalProtocolFilterResult::default();
        }
        let mut combined = String::new();
        if !self.pending.is_empty() {
            combined.push_str(&self.pending);
            self.pending.clear();
        }
        combined.push_str(data);

        let mut visible = String::with_capacity(combined.len());
        let mut responses = Vec::new();
        let mut answered_queries = Vec::new();
        let mut cursor = 0usize;
        while let Some(relative_start) = combined[cursor..].find("\u{1b}]") {
            let sequence_start = cursor + relative_start;
            visible.push_str(&combined[cursor..sequence_start]);
            let content_start = sequence_start + "\u{1b}]".len();
            if !osc_sequence_might_need_filtering(&combined[content_start..]) {
                visible.push_str("\u{1b}]");
                cursor = content_start;
                continue;
            }
            let Some((terminator_start, terminator_len)) =
                find_osc_terminator(&combined, content_start)
            else {
                self.pending = combined[sequence_start..].to_string();
                if self.pending.len() > TERMINAL_PROTOCOL_MAX_PENDING_BYTES {
                    visible.push_str(&self.pending);
                    self.pending.clear();
                }
                cursor = combined.len();
                break;
            };
            let content = &combined[content_start..terminator_start];
            let Some(queries) = parse_osc_color_query_content(content) else {
                visible.push_str(&combined[sequence_start..terminator_start + terminator_len]);
                cursor = terminator_start + terminator_len;
                continue;
            };
            for query in queries {
                if let Some(response) = profile.osc_color_response(query) {
                    responses.push(response);
                    answered_queries.push(query);
                }
            }
            cursor = terminator_start + terminator_len;
        }
        if cursor < combined.len() {
            visible.push_str(&combined[cursor..]);
        }

        TerminalProtocolFilterResult {
            data: visible,
            responses,
            answered_queries,
        }
    }

    fn discard_pending(&mut self) {
        self.pending.clear();
    }
}

pub(crate) fn infer_terminal_appearance_from_launch_command(
    launch_command: &str,
) -> Option<&'static str> {
    if launch_command_has_assignment(launch_command, "YGGTERM_TERMINAL_APPEARANCE", "dark")
        || launch_command_has_assignment(launch_command, "YGGTERM_APPEARANCE", "dark")
        || launch_command_has_assignment(launch_command, "COLORFGBG", "15;0")
    {
        return Some("dark");
    }
    if launch_command_has_assignment(launch_command, "YGGTERM_TERMINAL_APPEARANCE", "light")
        || launch_command_has_assignment(launch_command, "YGGTERM_APPEARANCE", "light")
        || launch_command_has_assignment(launch_command, "COLORFGBG", "0;15")
    {
        return Some("light");
    }
    None
}

fn launch_command_has_assignment(launch_command: &str, key: &str, value: &str) -> bool {
    let plain = format!("{key}={value}");
    let single_quoted = format!("{key}='{value}'");
    let double_quoted = format!("{key}=\"{value}\"");
    let exported_plain = format!("export {plain}");
    let exported_single = format!("export {single_quoted}");
    let exported_double = format!("export {double_quoted}");
    launch_command.contains(&plain)
        || launch_command.contains(&single_quoted)
        || launch_command.contains(&double_quoted)
        || launch_command.contains(&exported_plain)
        || launch_command.contains(&exported_single)
        || launch_command.contains(&exported_double)
}

fn launch_command_assignment_value(launch_command: &str, key: &str) -> Option<String> {
    for prefix in [format!("export {key}="), format!("{key}=")] {
        let Some(start) = launch_command.find(&prefix) else {
            continue;
        };
        let value_start = start + prefix.len();
        let rest = &launch_command[value_start..];
        if let Some(stripped) = rest.strip_prefix('\'') {
            let end = stripped.find('\'')?;
            return Some(stripped[..end].to_string());
        }
        if let Some(stripped) = rest.strip_prefix('"') {
            let end = stripped.find('"')?;
            return Some(stripped[..end].to_string());
        }
        let end = rest
            .find(|ch: char| ch == ';' || ch.is_whitespace())
            .unwrap_or(rest.len());
        return Some(rest[..end].to_string());
    }
    None
}

fn osc_sequence_might_need_filtering(content: &str) -> bool {
    content.starts_with("10;?") || content.starts_with("11;?") || content.starts_with("4;")
}

fn parse_osc_color_query_content(content: &str) -> Option<Vec<TerminalProtocolColorQuery>> {
    match content {
        "10;?" => {
            return Some(vec![TerminalProtocolColorQuery {
                code: OSC_COLOR_FOREGROUND_CODE,
                slot: OSC_COLOR_FOREGROUND_CODE,
            }]);
        }
        "11;?" => {
            return Some(vec![TerminalProtocolColorQuery {
                code: OSC_COLOR_BACKGROUND_CODE,
                slot: OSC_COLOR_BACKGROUND_CODE,
            }]);
        }
        _ => {}
    }

    let rest = content.strip_prefix("4;")?;
    let mut parts = rest.split(';');
    let mut queries = Vec::new();
    while let Some(slot_value) = parts.next() {
        let request = parts.next()?;
        if request != "?" {
            return None;
        }
        let slot = slot_value.parse::<u16>().ok()?;
        if slot > 15 {
            return None;
        }
        queries.push(TerminalProtocolColorQuery {
            code: OSC_PALETTE_CODE,
            slot,
        });
    }
    if queries.is_empty() {
        None
    } else {
        Some(queries)
    }
}

fn find_osc_terminator(data: &str, start: usize) -> Option<(usize, usize)> {
    let rest = &data[start..];
    let bel = rest.find('\u{7}').map(|offset| (start + offset, 1usize));
    let st = rest.find("\u{1b}\\").map(|offset| (start + offset, 2usize));
    match (bel, st) {
        (Some(bel), Some(st)) => Some(if bel.0 < st.0 { bel } else { st }),
        (Some(bel), None) => Some(bel),
        (None, Some(st)) => Some(st),
        (None, None) => None,
    }
}

fn osc_rgb_component(value: u8) -> String {
    format!("{value:02x}{value:02x}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalTrimSummary {
    pub trimmed_sessions: usize,
    pub reclaimed_bytes: usize,
}

pub struct TerminalManager {
    sessions: HashMap<String, PtySessionRuntime>,
}

#[derive(Debug, Clone)]
pub struct TerminalShutdownSummary {
    pub stopped: usize,
    pub errors: Vec<String>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn ensure_session(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
    ) -> Result<()> {
        self.ensure_session_with_size(key, launch_command, cwd, None)
    }

    pub fn ensure_session_with_size(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        initial_size: Option<(u16, u16)>,
    ) -> Result<()> {
        if self
            .sessions
            .get(key)
            .is_some_and(|session| session.is_running())
        {
            return Ok(());
        }
        if let Some(runtime) = self.sessions.remove(key) {
            trace_terminal_event(
                "replace_exited_runtime",
                serde_json::json!({
                    "path": key,
                    "launch_command": launch_command,
                }),
            );
            let _ = runtime.shutdown(None);
        }
        let runtime = PtySessionRuntime::spawn(key, launch_command, cwd, initial_size)?;
        self.sessions.insert(key.to_string(), runtime);
        Ok(())
    }

    pub fn session_matches_spec(&self, key: &str, launch_command: &str, cwd: Option<&str>) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.matches_spec(launch_command, cwd))
    }

    pub fn session_matches_remote_resume_spec(&self, key: &str, cwd: Option<&str>) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.matches_remote_resume_spec(cwd))
    }

    pub fn session_is_running(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.is_running())
    }

    pub fn session_has_output(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.has_output())
    }

    pub fn session_has_runtime_output(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.has_runtime_output())
    }

    pub fn session_hit_eof_without_output(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.hit_eof_without_output())
    }

    pub fn session_initial_read_has_scrollback(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.initial_read_has_scrollback())
    }

    pub fn session_runtime_age_ms(&self, key: &str) -> Option<u64> {
        self.sessions.get(key).map(|session| session.age_ms())
    }

    pub fn session_idle_for_ms(&self, key: &str) -> Option<u64> {
        self.sessions.get(key).map(|session| session.idle_for_ms())
    }

    pub fn session_process_id(&self, key: &str) -> Option<u32> {
        self.sessions
            .get(key)
            .and_then(|session| session.process_id())
    }

    pub fn session_foreground_process_active(&self, key: &str) -> Option<bool> {
        self.sessions
            .get(key)
            .and_then(|session| session.foreground_process_active())
    }

    pub fn session_snapshot(&self, key: &str) -> Option<String> {
        self.sessions.get(key).map(|session| session.snapshot())
    }

    pub fn session_screen_snapshot(&self, key: &str) -> Option<String> {
        self.sessions
            .get(key)
            .map(|session| session.screen_snapshot())
    }

    /// The session's clean scrolled-off history rows (vt100 scrollback ring).
    /// See `PtySessionRuntime::history_rows` — near-empty for cursor-addressed
    /// in-place repaint TUIs (codex), populated for genuinely-scrolling output.
    pub fn session_history_rows(&self, key: &str) -> Option<Vec<String>> {
        self.sessions.get(key).map(|session| session.history_rows())
    }

    pub fn session_keys(&self) -> Vec<String> {
        let mut keys = self
            .sessions
            .iter()
            .filter(|(_key, session)| session.is_running())
            .map(|(key, _session)| key.clone())
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    pub fn read(&self, key: &str, cursor: u64) -> Result<TerminalReadResult> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        Ok(session.read(cursor))
    }

    pub fn write(&self, key: &str, data: &str) -> Result<()> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        session.write(data)
    }

    /// Readiness-gated prompt insertion — the robustness contract behind agent /
    /// automation prompt insertion (timer-fired prompts must never land in a
    /// menu / busy / onboarding / update surface and do the wrong thing). Poll the
    /// session's current vt100 screen with `is_ready` until it reports the session
    /// is sitting at an idle interactive prompt, THEN write `data`. If the session
    /// isn't ready within `timeout`, write NOTHING and report `NotReady` so the
    /// caller can retry later or skip.
    ///
    /// `is_ready` is the injected readiness POLICY (e.g. the codex current-input-row
    /// recognizer the GUI uses) so this primitive stays agnostic of CLI-specific
    /// prompt shapes and keeps the recognizer's single source of truth in the
    /// caller's crate. Driven blocking (the caller runs it off the UI thread); the
    /// live `server app terminal send`/automation paths supply the predicate.
    pub fn submit_prompt(
        &self,
        key: &str,
        data: &str,
        is_ready: impl Fn(&str) -> bool,
        timeout: Duration,
    ) -> Result<PromptSubmitOutcome> {
        const POLL_INTERVAL: Duration = Duration::from_millis(120);
        let start = Instant::now();
        loop {
            let Some(screen) = self.session_screen_snapshot(key) else {
                return Ok(PromptSubmitOutcome::NoSession);
            };
            if is_ready(&screen) {
                self.write(key, data)?;
                return Ok(PromptSubmitOutcome::Submitted {
                    waited_ms: start.elapsed().as_millis() as u64,
                });
            }
            if start.elapsed() >= timeout {
                return Ok(PromptSubmitOutcome::NotReady {
                    waited_ms: start.elapsed().as_millis() as u64,
                });
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// ECHO-VERIFIED prompt insertion — the robust readiness check. A displayed
    /// prompt does NOT mean the program is reading input: a just-resumed codex draws
    /// its composer seconds-to-minutes before its input loop is live, so a prompt
    /// written then is silently dropped (root-caused 2026-06-04, see
    /// [[finding-fresh-restarted-codex-no-input]]). Instead of trusting "prompt
    /// shown", PROVE the program is consuming input: write a distinctive probe and
    /// confirm it ECHOES into the surface; only then clear it (Ctrl+U) and submit the
    /// real prompt. If the probe never echoes within `timeout`, the real prompt is
    /// NEVER written. Self-healing across retries: a Ctrl+U after each probe prevents
    /// buffered probes from accumulating once the program starts reading.
    pub fn submit_prompt_echo_verified(
        &self,
        key: &str,
        data: &str,
        timeout: Duration,
    ) -> Result<PromptSubmitOutcome> {
        // Distinctive enough not to collide with real surface text; cleared via Ctrl+U.
        const PROBE: &str = "yggterm_ready_probe";
        const CLEAR_LINE: &str = "\u{15}"; // Ctrl+U — clears the composer line
        const PROBE_SETTLE: Duration = Duration::from_millis(180);
        const RETRY_INTERVAL: Duration = Duration::from_millis(120);
        if self.session_screen_snapshot(key).is_none() {
            return Ok(PromptSubmitOutcome::NoSession);
        }
        let start = Instant::now();
        loop {
            self.write(key, PROBE)?;
            thread::sleep(PROBE_SETTLE);
            let echoed = self
                .session_screen_snapshot(key)
                .is_some_and(|screen| screen.contains(PROBE));
            if echoed {
                // The program is consuming input. Clear the probe, then submit AS A
                // HUMAN DOES: type the text, then a DISTINCT Enter keypress. codex
                // treats a \r concatenated with text in one write as a pasted newline
                // (composer content), NOT a submit — so the Enter must be its own
                // write after the text settles (verified live 2026-06-04).
                self.write(key, CLEAR_LINE)?;
                thread::sleep(Duration::from_millis(60));
                let text = data.trim_end_matches(['\r', '\n']);
                self.write(key, text)?;
                thread::sleep(Duration::from_millis(80));
                self.write(key, "\r")?;
                return Ok(PromptSubmitOutcome::Submitted {
                    waited_ms: start.elapsed().as_millis() as u64,
                });
            }
            // Not consuming yet: clear any buffered probe so it can't pile up, then
            // wait and retry (or give up at the deadline, leaving the surface clean).
            let _ = self.write(key, CLEAR_LINE);
            if start.elapsed() >= timeout {
                return Ok(PromptSubmitOutcome::NotReady {
                    waited_ms: start.elapsed().as_millis() as u64,
                });
            }
            thread::sleep(RETRY_INTERVAL);
        }
    }

    pub fn resize(&self, key: &str, cols: u16, rows: u16) -> Result<()> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        session.resize(cols, rows)
    }

    /// Current PTY grid (cols, rows) for a session, as tracked by the runtime.
    /// Exposed for restart/re-resume size-preservation checks and tests.
    pub fn session_size(&self, key: &str) -> Option<(u16, u16)> {
        self.sessions.get(key).map(|session| {
            (
                session.current_cols.load(Ordering::SeqCst),
                session.current_rows.load(Ordering::SeqCst),
            )
        })
    }

    pub fn session_post_resize_output_seen(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.post_resize_output_seen())
    }

    pub fn session_last_resize_seq(&self, key: &str) -> u64 {
        self.sessions
            .get(key)
            .map(|session| session.last_resize_seq())
            .unwrap_or(0)
    }

    pub fn has_session(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.is_running())
    }

    pub fn rename_session(&mut self, from: &str, to: &str) -> bool {
        if from == to || self.sessions.contains_key(to) {
            return false;
        }
        let Some(mut runtime) = self.sessions.remove(from) else {
            return false;
        };
        trace_terminal_event(
            "rename",
            serde_json::json!({
                "from": from,
                "to": to,
            }),
        );
        runtime.key = to.to_string();
        self.sessions.insert(to.to_string(), runtime);
        true
    }

    pub fn seed_session(&self, key: &str, data: &str) -> Result<()> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        session.seed_snapshot(data);
        Ok(())
    }

    pub fn stats(&self) -> TerminalBufferStats {
        let mut stats = TerminalBufferStats {
            session_count: self
                .sessions
                .values()
                .filter(|session| session.is_running())
                .count(),
            ..TerminalBufferStats::default()
        };
        for session in self.sessions.values() {
            let (chunks, bytes) = session.buffer_usage();
            stats.retained_chunks += chunks;
            stats.retained_bytes += bytes;
        }
        stats
    }

    pub fn trim_idle_buffers(&self, within: Duration) -> TerminalTrimSummary {
        let mut summary = TerminalTrimSummary::default();
        for session in self.sessions.values() {
            let reclaimed = session.trim_idle_buffer(within);
            if reclaimed > 0 {
                summary.trimmed_sessions += 1;
                summary.reclaimed_bytes += reclaimed;
            }
        }
        summary
    }

    pub fn recent_activity(&self, key: &str, within: Duration) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.recent_activity(within))
    }

    pub fn restart_session(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        stop_command: Option<&str>,
    ) -> Result<()> {
        self.restart_session_with_size(key, launch_command, cwd, stop_command, None)
    }

    pub fn restart_session_with_size(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        stop_command: Option<&str>,
        initial_size: Option<(u16, u16)>,
    ) -> Result<()> {
        // PRESERVE the outgoing session's grid across a restart. Without an explicit
        // initial_size, re-creating the PTY at the DEFAULT 120x36 left the new PTY
        // narrower than the client's real grid (e.g. 159x63). The client would then
        // try to resize, but the daemon's resize no-op check (cache + observed size)
        // could mismatch the swap and skip the actual ioctl — leaving the program
        // (codex) rendering squished. Carrying the old size forward re-creates the
        // PTY at the right dimensions directly, with no dependence on a follow-up
        // resize. (For a full daemon-process restart the old size is gone with the
        // process; the client re-sends its grid on the rewound-cursor re-attach.)
        let preserved_size = self.sessions.get(key).and_then(|runtime| {
            let cols = runtime.current_cols.load(Ordering::SeqCst);
            let rows = runtime.current_rows.load(Ordering::SeqCst);
            (cols > 0 && rows > 0).then_some((cols, rows))
        });
        let effective_initial_size = initial_size.or(preserved_size);
        let (initial_cols, initial_rows) =
            effective_initial_size.unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        trace_terminal_event(
            "restart",
            serde_json::json!({
                "path": key,
                "cwd": cwd,
                "launch_command": launch_command,
                "stop_command": stop_command,
                "initial_cols": initial_cols,
                "initial_rows": initial_rows,
                "preserved_size": preserved_size.is_some() && initial_size.is_none(),
            }),
        );
        if let Some(runtime) = self.sessions.remove(key) {
            runtime.shutdown(stop_command)?;
        }
        let runtime = PtySessionRuntime::spawn(key, launch_command, cwd, effective_initial_size)?;
        self.sessions.insert(key.to_string(), runtime);
        Ok(())
    }

    pub fn remove_session(&mut self, key: &str, stop_command: Option<&str>) -> Result<bool> {
        let Some(runtime) = self.sessions.remove(key) else {
            return Ok(false);
        };
        runtime.shutdown(stop_command)?;
        Ok(true)
    }

    pub fn remove_session_gracefully_with_force_after(
        &mut self,
        key: &str,
        stop_command: Option<&str>,
        force_after: Duration,
    ) -> Result<bool> {
        let Some(runtime) = self.sessions.remove(key) else {
            return Ok(false);
        };
        runtime.shutdown_with_force_after(stop_command, force_after)?;
        Ok(true)
    }

    pub fn shutdown_all<F>(&mut self, stop_command: F) -> TerminalShutdownSummary
    where
        F: Fn(&str) -> Option<String>,
    {
        let keys = self.sessions.keys().cloned().collect::<Vec<_>>();
        let mut stopped = 0usize;
        let mut errors = Vec::new();
        let worker_limit = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .clamp(1, 4);
        let mut pending = Vec::new();

        let flush_pending = |pending: &mut Vec<(String, thread::JoinHandle<Result<()>>)>,
                             stopped: &mut usize,
                             errors: &mut Vec<String>| {
            for (key, handle) in pending.drain(..) {
                match handle.join() {
                    Ok(Ok(())) => *stopped += 1,
                    Ok(Err(error)) => errors.push(format!("{key}: {error}")),
                    Err(_) => errors.push(format!("{key}: terminal shutdown thread panicked")),
                }
            }
        };

        for key in keys {
            let Some(runtime) = self.sessions.remove(&key) else {
                continue;
            };
            let stop = stop_command(&key);
            pending.push((
                key,
                thread::spawn(move || runtime.shutdown(stop.as_deref())),
            ));
            if pending.len() >= worker_limit {
                flush_pending(&mut pending, &mut stopped, &mut errors);
            }
        }
        flush_pending(&mut pending, &mut stopped, &mut errors);
        TerminalShutdownSummary { stopped, errors }
    }
}

struct PtySessionRuntime {
    key: String,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer_tx: SyncSender<TerminalWriteRequest>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    chunks: Arc<Mutex<VecDeque<TerminalChunk>>>,
    retained_bytes: Arc<AtomicUsize>,
    seq: Arc<AtomicU64>,
    started_at_ms: u64,
    last_activity_ms: Arc<AtomicU64>,
    runtime_output_seen: Arc<AtomicBool>,
    eof_without_output: Arc<AtomicBool>,
    attach_ready_seen: Arc<AtomicBool>,
    resize_count: Arc<AtomicU64>,
    last_resize_seq: Arc<AtomicU64>,
    current_cols: Arc<AtomicU16>,
    current_rows: Arc<AtomicU16>,
    screen_state: Arc<Mutex<TerminalScreenState>>,
    launch_command: String,
    cwd: Option<String>,
}

struct TerminalWriteRequest {
    data: Vec<u8>,
    completion_tx: Option<mpsc::Sender<std::result::Result<(), String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
enum TerminalWriteAckMode {
    Enqueued,
    Flushed,
}

struct TerminalScreenState {
    parser: Vt100Parser,
    formatted: String,
}

impl TerminalScreenState {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            // Per [[spec-tmux-parity-and-beyond]] the daemon's vt100 parser
            // tracks DAEMON_VT_SCROLLBACK_ROWS of scrolled-off rows so the
            // GUI can restore real terminal history after restart (matching
            // tmux's `history-limit` semantics).
            parser: Vt100Parser::new(rows, cols, DAEMON_VT_SCROLLBACK_ROWS),
            formatted: String::new(),
        }
    }

    fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.refresh_formatted();
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
        self.refresh_formatted();
    }

    fn refresh_formatted(&mut self) {
        self.formatted = String::from_utf8_lossy(&self.parser.screen().state_formatted()).into();
    }

    /// Walk the vt100 scrollback ring (rows that have scrolled off the
    /// visible viewport) oldest-to-newest and return them as plain-text
    /// rows. Uses `set_scrollback(k)` round-trips because vt100's public
    /// API caps `visible_rows()` at viewport size — to enumerate the full
    /// ring we step the scrollback offset down from the actual count to 1
    /// and grab the topmost visible row each step.
    ///
    /// Per [[spec-tmux-parity-and-beyond]] — this is what closes the
    /// tmux-parity gap: the daemon retains real scrollback across GUI
    /// restart, and on attach we prepend this history before the
    /// formatted viewport so the user sees their real terminal history,
    /// not just the last frame.
    fn vt_scrollback_plain_rows(&mut self) -> Vec<String> {
        let screen = self.parser.screen_mut();
        let saved_offset = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        let total = screen.scrollback();
        if total == 0 {
            screen.set_scrollback(saved_offset);
            return Vec::new();
        }
        let (_, cols) = screen.size();
        let mut rows = Vec::with_capacity(total);
        for k in (1..=total).rev() {
            screen.set_scrollback(k);
            if let Some(text) = screen.rows(0, cols).next() {
                rows.push(text.trim_end().to_string());
            }
        }
        screen.set_scrollback(saved_offset);
        rows
    }

    /// Build a single replay payload combining the scrollback history
    /// (as plain text rows) with the formatted viewport state. The
    /// payload is shaped so xterm.js on the GUI side renders history
    /// into its scrollback and then repaints the current viewport via
    /// the formatted-state escape sequence. Returns `None` when the
    /// session has neither scrollback nor visible viewport content.
    fn history_and_screen_replay(&mut self) -> Option<String> {
        let history = self.vt_scrollback_plain_rows();
        let history: Vec<String> = history
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect();
        let formatted = self.formatted.trim_matches('\0').to_string();
        let formatted_has_visible = formatted
            .chars()
            .any(|ch| !ch.is_control() && !ch.is_whitespace());
        if history.is_empty() && !formatted_has_visible {
            return None;
        }
        let mut payload = String::with_capacity(history.iter().map(|l| l.len() + 2).sum::<usize>() + formatted.len() + 8);
        for line in &history {
            payload.push_str(line);
            payload.push_str("\r\n");
        }
        // \x1b[2J\x1b[H clears the visible viewport (not scrollback) and
        // homes the cursor; matches what the GUI-side
        // `terminal_retained_history_screen_replay_payload` writes between
        // history and screen.
        if !history.is_empty() {
            payload.push_str("\x1b[2J\x1b[H");
        }
        payload.push_str(&formatted);
        Some(payload)
    }
}

fn spawn_terminal_writer_thread(
    key: String,
    writer: Box<dyn Write + Send>,
    last_activity_ms: Arc<AtomicU64>,
    capacity: usize,
) -> Result<SyncSender<TerminalWriteRequest>> {
    let (tx, rx) = mpsc::sync_channel::<TerminalWriteRequest>(capacity);
    thread::Builder::new()
        .name(format!("pty-writer-{key}"))
        .spawn(move || {
            let mut writer = writer;
            while let Ok(request) = rx.recv() {
                last_activity_ms.store(now_millis(), Ordering::SeqCst);
                let byte_count = request.data.len();
                let write_result = writer
                    .write_all(&request.data)
                    .and_then(|()| writer.flush())
                    .map_err(|error| error.to_string());
                if let Some(completion_tx) = request.completion_tx {
                    let _ = completion_tx.send(write_result.clone());
                }
                if let Err(error) = write_result {
                    trace_terminal_event(
                        "write_failed",
                        serde_json::json!({
                            "path": key,
                            "bytes": byte_count,
                            "error": error,
                        }),
                    );
                    break;
                }
            }
        })
        .context("spawning pty writer thread")?;
    Ok(tx)
}

fn enqueue_terminal_write(
    writer_tx: &SyncSender<TerminalWriteRequest>,
    key: &str,
    data: &str,
    capacity: usize,
    ack_mode: TerminalWriteAckMode,
) -> Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    let bytes = data.as_bytes().to_vec();
    let byte_count = bytes.len();
    let (completion_tx, completion_rx) = if ack_mode == TerminalWriteAckMode::Flushed {
        let (tx, rx) = mpsc::channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    let request = TerminalWriteRequest {
        data: bytes,
        completion_tx,
    };
    match writer_tx.try_send(request) {
        Ok(()) => {
            let Some(completion_rx) = completion_rx else {
                return Ok(());
            };
            match completion_rx
                .recv_timeout(Duration::from_millis(TERMINAL_WRITE_FLUSH_ACK_TIMEOUT_MS))
            {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => {
                    trace_terminal_event(
                        "write_flush_failed",
                        serde_json::json!({
                            "path": key,
                            "bytes": byte_count,
                            "error": error,
                        }),
                    );
                    bail!("terminal writer failed for {key}: {error}")
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    trace_terminal_event(
                        "write_flush_timeout",
                        serde_json::json!({
                            "path": key,
                            "bytes": byte_count,
                            "timeout_ms": TERMINAL_WRITE_FLUSH_ACK_TIMEOUT_MS,
                        }),
                    );
                    bail!(
                        "terminal writer did not flush input for {key} within {TERMINAL_WRITE_FLUSH_ACK_TIMEOUT_MS}ms"
                    )
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("terminal writer exited before flushing input for {key}")
                }
            }
        }
        Err(TrySendError::Full(_)) => {
            trace_terminal_event(
                "write_backpressure",
                serde_json::json!({
                    "path": key,
                    "bytes": byte_count,
                    "queue_capacity": capacity,
                }),
            );
            bail!("terminal input queue is full for {key}; child process is not accepting input")
        }
        Err(TrySendError::Disconnected(_)) => {
            bail!("terminal writer is no longer available for {key}")
        }
    }
}

fn enqueue_terminal_protocol_responses(
    writer_tx: &SyncSender<TerminalWriteRequest>,
    key: &str,
    profile: TerminalProtocolProfile,
    result: &TerminalProtocolFilterResult,
) {
    if result.responses.is_empty() {
        return;
    }
    for response in &result.responses {
        if let Err(error) = enqueue_terminal_write(
            writer_tx,
            key,
            response,
            TERMINAL_WRITE_QUEUE_CAPACITY,
            TerminalWriteAckMode::Enqueued,
        ) {
            trace_terminal_event(
                "protocol_color_response_failed",
                serde_json::json!({
                    "path": key,
                    "appearance": profile.appearance,
                    "queries": result
                        .answered_queries
                        .iter()
                        .map(|query| query.label())
                        .collect::<Vec<_>>(),
                    "error": error.to_string(),
                }),
            );
            return;
        }
    }
    trace_terminal_event(
        "protocol_color_response_sent",
        serde_json::json!({
            "path": key,
            "appearance": profile.appearance,
            "queries": result
                .answered_queries
                .iter()
                .map(|query| query.label())
                .collect::<Vec<_>>(),
            "response_count": result.responses.len(),
        }),
    );
}

impl PtySessionRuntime {
    fn spawn(
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        initial_size: Option<(u16, u16)>,
    ) -> Result<Self> {
        let (initial_cols, initial_rows) = initial_size.unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        if initial_cols == 0 || initial_rows == 0 {
            bail!("terminal size must be greater than zero");
        }
        trace_terminal_event(
            "spawn",
            serde_json::json!({
                "path": key,
                "cwd": cwd,
                "launch_command": launch_command,
                "initial_cols": initial_cols,
                "initial_rows": initial_rows,
            }),
        );
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: initial_rows,
                cols: initial_cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("opening pty")?;

        let command = shell_command(launch_command, cwd);
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("spawning terminal session {key}"))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("cloning pty reader")?;
        let writer = pair.master.take_writer().context("taking pty writer")?;
        let chunks = Arc::new(Mutex::new(VecDeque::new()));
        let retained_bytes = Arc::new(AtomicUsize::new(0));
        let seq = Arc::new(AtomicU64::new(0));
        let started_at_ms = now_millis();
        let last_activity_ms = Arc::new(AtomicU64::new(started_at_ms));
        let runtime_output_seen = Arc::new(AtomicBool::new(false));
        let eof_without_output = Arc::new(AtomicBool::new(false));
        let attach_ready_seen = Arc::new(AtomicBool::new(false));
        let resize_count = Arc::new(AtomicU64::new(0));
        let last_resize_seq = Arc::new(AtomicU64::new(0));
        let current_cols = Arc::new(AtomicU16::new(initial_cols));
        let current_rows = Arc::new(AtomicU16::new(initial_rows));
        let screen_state = Arc::new(Mutex::new(TerminalScreenState::new(
            initial_rows,
            initial_cols,
        )));
        let reader_chunks = Arc::clone(&chunks);
        let reader_retained_bytes = Arc::clone(&retained_bytes);
        let reader_seq = Arc::clone(&seq);
        let reader_activity = Arc::clone(&last_activity_ms);
        let reader_runtime_output_seen = Arc::clone(&runtime_output_seen);
        let reader_eof_without_output = Arc::clone(&eof_without_output);
        let reader_attach_ready_seen = Arc::clone(&attach_ready_seen);
        let reader_screen_state = Arc::clone(&screen_state);
        let key_label = key.to_string();
        let launch_command_label = launch_command.to_string();
        let terminal_protocol_profile =
            TerminalProtocolProfile::from_launch_command(&launch_command_label);
        let writer_tx = spawn_terminal_writer_thread(
            key.to_string(),
            writer,
            Arc::clone(&last_activity_ms),
            TERMINAL_WRITE_QUEUE_CAPACITY,
        )
        .context("spawning pty writer thread")?;
        let reader_writer_tx = writer_tx.clone();

        thread::Builder::new()
            .name(format!("pty-reader-{key}"))
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                let mut pending_utf8 = Vec::<u8>::new();
                let mut protocol_filter = TerminalProtocolFilter::default();
                let mut saw_any_output = false;
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => {
                            let raw_data = flush_terminal_utf8_pending(&mut pending_utf8);
                            let protocol_result =
                                protocol_filter.process(&raw_data, terminal_protocol_profile);
                            enqueue_terminal_protocol_responses(
                                &reader_writer_tx,
                                &key_label,
                                terminal_protocol_profile,
                                &protocol_result,
                            );
                            protocol_filter.discard_pending();
                            let data = protocol_result.data;
                            if !data.is_empty() {
                                if let Ok(mut screen_state) = reader_screen_state.lock() {
                                    screen_state.process(data.as_bytes());
                                }
                                reader_runtime_output_seen.store(true, Ordering::SeqCst);
                                reader_activity.store(now_millis(), Ordering::SeqCst);
                                let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
                                let mut chunks =
                                    reader_chunks.lock().expect("pty chunk lock poisoned");
                                let mut retained =
                                    reader_retained_bytes.load(Ordering::SeqCst);
                                chunks.push_back(TerminalChunk {
                                    seq: seq_value,
                                    data,
                                });
                                retained = retained.saturating_add(
                                    chunks.back().map(|chunk| chunk.data.len()).unwrap_or(0),
                                );
                                trim_chunk_buffer(
                                    &mut chunks,
                                    &mut retained,
                                    MAX_CHUNKS,
                                    MAX_BUFFER_BYTES,
                                );
                                reader_retained_bytes.store(retained, Ordering::SeqCst);
                            }
                            break;
                        }
                        Ok(bytes) => {
                            let raw_data =
                                decode_terminal_utf8_chunk(&mut pending_utf8, &buffer[..bytes]);
                            if raw_data.is_empty() {
                                reader_activity.store(now_millis(), Ordering::SeqCst);
                                continue;
                            }
                            let (data, stripped_attach_ready_marker) =
                                if launch_command_looks_like_remote_resume_attach(
                                    &launch_command_label,
                                ) {
                                    terminal_data_without_attach_ready_markers(&raw_data)
                                } else {
                                    (raw_data, false)
                                };
                            if stripped_attach_ready_marker {
                                reader_attach_ready_seen.store(true, Ordering::SeqCst);
                            }
                            let protocol_result =
                                protocol_filter.process(&data, terminal_protocol_profile);
                            enqueue_terminal_protocol_responses(
                                &reader_writer_tx,
                                &key_label,
                                terminal_protocol_profile,
                                &protocol_result,
                            );
                            let answered_terminal_protocol =
                                !protocol_result.answered_queries.is_empty();
                            let data = protocol_result.data;
                            if data.is_empty() {
                                if stripped_attach_ready_marker || answered_terminal_protocol {
                                    reader_activity.store(now_millis(), Ordering::SeqCst);
                                }
                                continue;
                            }
                            if let Ok(mut screen_state) = reader_screen_state.lock() {
                                screen_state.process(data.as_bytes());
                            }
                            if !saw_any_output {
                                saw_any_output = true;
                                trace_terminal_event(
                                    "first_bytes",
                                    serde_json::json!({
                                        "path": key_label,
                                        "bytes": bytes,
                                        "launch_command": launch_command_label,
                                        "visible_text": terminal_chunk_has_visible_text(&data),
                                        "sample": truncate_terminal_trace_sample(&strip_terminal_control_sequences(&data)),
                                    }),
                                );
                            }
                            reader_runtime_output_seen.store(true, Ordering::SeqCst);
                            reader_activity.store(now_millis(), Ordering::SeqCst);
                            let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
                            let mut chunks = reader_chunks.lock().expect("pty chunk lock poisoned");
                            let mut retained = reader_retained_bytes.load(Ordering::SeqCst);
                            chunks.push_back(TerminalChunk {
                                seq: seq_value,
                                data,
                            });
                            retained = retained.saturating_add(chunks.back().map(|chunk| chunk.data.len()).unwrap_or(0));
                            trim_chunk_buffer(&mut chunks, &mut retained, MAX_CHUNKS, MAX_BUFFER_BYTES);
                            reader_retained_bytes.store(retained, Ordering::SeqCst);
                        }
                        Err(error) => {
                            if !saw_any_output {
                                trace_terminal_event(
                                    "reader_error_before_output",
                                    serde_json::json!({
                                        "path": key_label,
                                        "launch_command": launch_command_label,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                            reader_runtime_output_seen.store(true, Ordering::SeqCst);
                            reader_activity.store(now_millis(), Ordering::SeqCst);
                            let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
                            let mut chunks = reader_chunks.lock().expect("pty chunk lock poisoned");
                            let mut retained = reader_retained_bytes.load(Ordering::SeqCst);
                            chunks.push_back(TerminalChunk {
                                seq: seq_value,
                                data: format!("\r\n[yggterm] terminal reader stopped for {key_label}: {error}\r\n"),
                            });
                            retained = retained.saturating_add(chunks.back().map(|chunk| chunk.data.len()).unwrap_or(0));
                            trim_chunk_buffer(&mut chunks, &mut retained, MAX_CHUNKS, MAX_BUFFER_BYTES);
                            reader_retained_bytes.store(retained, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                if !saw_any_output {
                    reader_eof_without_output.store(true, Ordering::SeqCst);
                    trace_terminal_event(
                        "eof_without_output",
                        serde_json::json!({
                            "path": key_label,
                            "launch_command": launch_command_label,
                        }),
                    );
                }
            })
            .context("spawning pty reader thread")?;

        Ok(Self {
            key: key.to_string(),
            master: Arc::new(Mutex::new(pair.master)),
            writer_tx,
            child: Arc::new(Mutex::new(child)),
            chunks,
            retained_bytes,
            seq,
            started_at_ms,
            last_activity_ms,
            runtime_output_seen,
            eof_without_output,
            attach_ready_seen,
            resize_count,
            last_resize_seq,
            current_cols,
            current_rows,
            screen_state,
            launch_command: launch_command.to_string(),
            cwd: cwd.map(|value| value.to_string()),
        })
    }

    fn matches_spec(&self, launch_command: &str, cwd: Option<&str>) -> bool {
        self.launch_command == launch_command && self.cwd.as_deref() == cwd
    }

    fn matches_remote_resume_spec(&self, cwd: Option<&str>) -> bool {
        self.cwd.as_deref() == cwd
            && launch_command_looks_like_remote_resume_attach(&self.launch_command)
    }

    fn is_running(&self) -> bool {
        let mut child = self.child.lock().expect("pty child lock poisoned");
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => false,
            Err(_) => false,
        }
    }

    fn process_id(&self) -> Option<u32> {
        let child = self.child.lock().expect("pty child lock poisoned");
        child.process_id()
    }

    #[cfg(unix)]
    fn foreground_process_group_leader(&self) -> Option<u32> {
        let master = self.master.lock().expect("pty master lock poisoned");
        let fd = master.as_raw_fd()?;
        let pgid = unsafe { libc::tcgetpgrp(fd) };
        (pgid > 0).then_some(pgid as u32)
    }

    #[cfg(not(unix))]
    fn foreground_process_group_leader(&self) -> Option<u32> {
        None
    }

    fn foreground_process_active(&self) -> Option<bool> {
        if !self.is_running() {
            return Some(false);
        }
        let child_pid = self.process_id()?;
        let foreground_pgid = self.foreground_process_group_leader()?;
        Some(foreground_pgid != child_pid)
    }

    fn has_output(&self) -> bool {
        self.seq.load(Ordering::SeqCst) > 0
            || self.retained_bytes.load(Ordering::SeqCst) > 0
            || !self
                .chunks
                .lock()
                .expect("pty chunk lock poisoned")
                .is_empty()
    }

    fn has_runtime_output(&self) -> bool {
        self.runtime_output_seen.load(Ordering::SeqCst)
    }

    fn last_resize_seq(&self) -> u64 {
        self.last_resize_seq.load(Ordering::SeqCst)
    }

    fn post_resize_output_seen(&self) -> bool {
        self.resize_count.load(Ordering::SeqCst) == 0
            || self.seq.load(Ordering::SeqCst) > self.last_resize_seq()
    }

    fn hit_eof_without_output(&self) -> bool {
        self.eof_without_output.load(Ordering::SeqCst)
    }

    fn age_ms(&self) -> u64 {
        now_millis().saturating_sub(self.started_at_ms)
    }

    /// Milliseconds since this session last produced PTY output. The reader
    /// loop stamps `last_activity_ms` on every chunk, so this is the most
    /// reliable daemon-side "how recently was this session active" signal —
    /// used by the hot-update idle gate to avoid interrupting agents that are
    /// mid-turn or just finished. See [[finding-hot-update-interrupts-remote-sessions]].
    fn idle_for_ms(&self) -> u64 {
        now_millis().saturating_sub(self.last_activity_ms.load(Ordering::SeqCst))
    }

    fn snapshot(&self) -> String {
        let chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>()
    }

    fn screen_snapshot(&self) -> String {
        self.screen_state
            .lock()
            .expect("pty screen state lock poisoned")
            .formatted
            .trim_matches('\0')
            .to_string()
    }

    /// The daemon's CLEAN scrolled-off history rows (vt100 scrollback ring),
    /// oldest-to-newest, blank rows dropped. Read-only (restores the scrollback
    /// offset). This is the history that CAN be loaded into the client's xterm
    /// scrollback on reveal (so base_y > 0). For a cursor-addressed in-place
    /// repaint TUI (e.g. codex redrawing its window via absolute cursor moves /
    /// \x1b[2J without scrolling) this is near-empty BY DESIGN — nothing scrolled
    /// off — which is why such sessions reveal with base_y == 0 (no scrollback to
    /// scroll into), not a pipeline bug.
    fn history_rows(&self) -> Vec<String> {
        let mut screen_state = self
            .screen_state
            .lock()
            .expect("pty screen state lock poisoned");
        screen_state
            .vt_scrollback_plain_rows()
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect()
    }

    fn screen_snapshot_chunk(&self, next_cursor: u64) -> Option<TerminalChunk> {
        let mut screen_state = self
            .screen_state
            .lock()
            .expect("pty screen state lock poisoned");
        // Per [[spec-tmux-parity-and-beyond]]: emit history+viewport, not
        // just viewport. Without this the GUI shows only the last frame
        // after restart and loses everything that scrolled off.
        let payload = screen_state.history_and_screen_replay()?;
        if !terminal_chunk_has_visible_text(&payload) {
            return None;
        }
        Some(TerminalChunk {
            seq: next_cursor.saturating_add(1),
            data: payload,
        })
    }

    fn read(&self, cursor: u64) -> TerminalReadResult {
        let retained_chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        let next_cursor = self.seq.load(Ordering::SeqCst);
        let effective_cursor = if cursor > next_cursor { 0 } else { cursor };
        // Mid-stream gap detection: a resuming client (cursor > 0) expects its next
        // chunk to be `cursor + 1`. If the live ring's oldest surviving chunk is
        // already past that, the ring trimmed the contiguous middle while the client
        // was behind — those chunks are gone from the raw ring (recoverable only via
        // a clean re-attach off the vt100 scrollback). Signal it instead of silently
        // returning the discontiguous tail. See
        // docs/xterm-bugs.md#chunk-ring-trim-drops-mid-stream.
        let resync_required = effective_cursor > 0
            && retained_chunks
                .front()
                .is_some_and(|oldest| oldest.seq > effective_cursor + 1);
        // NOTE: the chunk-ring-gap resync (docs/xterm-bugs.md#chunk-ring-trim-drops-
        // mid-stream) was reverted — replaying history+screen on a gap corrupted
        // ALTERNATE-SCREEN TUIs (codex) on switch-back (normal-buffer history written
        // into the alt screen) → broken render → indefinite non-prompt gate. The gap
        // fix needs to be alt-screen-aware (replay screen-only when in the alternate
        // buffer; vt100::Screen::alternate_screen() can gate it) before it ships.
        let prefer_initial_screen_snapshot =
            terminal_key_prefers_initial_screen_snapshot(&self.key, &self.launch_command);
        let mut chunks = if effective_cursor == 0 {
            if prefer_initial_screen_snapshot {
                let retained_initial = select_remote_retained_initial_chunks(
                    &self.key,
                    &self.launch_command,
                    &retained_chunks,
                );
                if initial_remote_attach_should_preserve_retained_chunks(
                    &self.key,
                    &self.launch_command,
                    &retained_initial,
                ) {
                    retained_initial
                } else {
                    retained_initial
                }
            } else {
                select_remote_retained_initial_chunks(
                    &self.key,
                    &self.launch_command,
                    &retained_chunks,
                )
            }
        } else {
            retained_chunks
                .iter()
                .filter(|chunk| chunk.seq > effective_cursor)
                .cloned()
                .collect()
        };
        if effective_cursor == 0 && chunks.is_empty() {
            chunks =
                select_initial_attach_chunks_for_launch(&retained_chunks, &self.launch_command);
        }
        if effective_cursor == 0
            && let Some(snapshot_chunk) = self.screen_snapshot_chunk(next_cursor)
            && initial_attach_should_replay_screen_snapshot(
                &self.key,
                &self.launch_command,
                &chunks,
                &snapshot_chunk.data,
            )
        {
            chunks = vec![snapshot_chunk];
        }
        if effective_cursor == 0
            && !prefer_initial_screen_snapshot
            && !chunks
                .iter()
                .any(|chunk| terminal_chunk_has_visible_text(&chunk.data))
            && let Some(snapshot_chunk) = self.screen_snapshot_chunk(next_cursor)
        {
            chunks = vec![snapshot_chunk];
        }
        if effective_cursor == 0
            && self.is_running()
            && prefer_initial_screen_snapshot
            && self.attach_ready_seen.load(Ordering::SeqCst)
        {
            chunks.push(TerminalChunk {
                seq: next_cursor.saturating_add(1),
                data: ATTACH_READY_MARKER.to_string(),
            });
        }
        TerminalReadResult {
            cursor: next_cursor,
            chunks,
            running: self.is_running(),
            runtime_output_seen: self.has_runtime_output(),
            eof_without_output: self.eof_without_output.load(Ordering::SeqCst),
            post_resize_output_seen: self.post_resize_output_seen(),
            last_resize_seq: self.last_resize_seq(),
            resync_required,
        }
    }

    fn initial_read_has_scrollback(&self) -> bool {
        self.read(0)
            .chunks
            .iter()
            .any(|chunk| terminal_chunk_has_scrollback_text(&chunk.data))
    }

    fn buffer_usage(&self) -> (usize, usize) {
        let chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        (chunks.len(), self.retained_bytes.load(Ordering::SeqCst))
    }

    fn write(&self, data: &str) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.last_activity_ms.store(now_millis(), Ordering::SeqCst);
        enqueue_terminal_write(
            &self.writer_tx,
            &self.key,
            data,
            TERMINAL_WRITE_QUEUE_CAPACITY,
            TerminalWriteAckMode::Flushed,
        )
    }

    fn seed_snapshot(&self, data: &str) {
        if data.is_empty() {
            return;
        }
        if let Ok(mut screen_state) = self.screen_state.lock() {
            screen_state.process(data.as_bytes());
        }
        self.runtime_output_seen.store(true, Ordering::SeqCst);
        self.last_activity_ms.store(now_millis(), Ordering::SeqCst);
        let seq_value = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        let mut retained = self.retained_bytes.load(Ordering::SeqCst);
        chunks.push_back(TerminalChunk {
            seq: seq_value,
            data: data.to_string(),
        });
        retained = retained.saturating_add(data.len());
        trim_chunk_buffer(&mut chunks, &mut retained, MAX_CHUNKS, MAX_BUFFER_BYTES);
        self.retained_bytes.store(retained, Ordering::SeqCst);
    }

    fn recent_activity(&self, within: Duration) -> bool {
        let now = now_millis();
        let last = self.last_activity_ms.load(Ordering::SeqCst);
        now.saturating_sub(last) <= within.as_millis() as u64
    }

    fn trim_idle_buffer(&self, within: Duration) -> usize {
        if self.recent_activity(within)
            || launch_command_looks_like_remote_resume_attach(&self.launch_command)
        {
            return 0;
        }
        let mut chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        let mut retained = self.retained_bytes.load(Ordering::SeqCst);
        let before = retained;
        trim_chunk_buffer(
            &mut chunks,
            &mut retained,
            IDLE_TRIM_MAX_CHUNKS,
            IDLE_TRIM_MAX_BYTES,
        );
        self.retained_bytes.store(retained, Ordering::SeqCst);
        before.saturating_sub(retained)
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        if cols == 0 || rows == 0 {
            bail!("terminal size must be greater than zero");
        }
        let previous_cols = self.current_cols.load(Ordering::SeqCst);
        let previous_rows = self.current_rows.load(Ordering::SeqCst);
        let master = self.master.lock().expect("pty master lock poisoned");
        let observed_before = master.get_size().ok().map(|size| (size.cols, size.rows));
        let cache_matches_request = previous_cols == cols && previous_rows == rows;
        if cache_matches_request && observed_before == Some((cols, rows)) {
            trace_terminal_event(
                "resize_noop",
                serde_json::json!({
                    "path": self.key,
                    "cols": cols,
                    "rows": rows,
                    "actual_cols": cols,
                    "actual_rows": rows,
                }),
            );
            return Ok(());
        }
        if cache_matches_request {
            trace_terminal_event(
                "resize_cache_mismatch_repair",
                serde_json::json!({
                    "path": self.key,
                    "requested_cols": cols,
                    "requested_rows": rows,
                    "cached_cols": previous_cols,
                    "cached_rows": previous_rows,
                    "actual_cols": observed_before.map(|(actual_cols, _)| actual_cols),
                    "actual_rows": observed_before.map(|(_, actual_rows)| actual_rows),
                }),
            );
        }
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("resizing pty")?;
        let observed_after = master.get_size().ok().map(|size| (size.cols, size.rows));
        let (effective_cols, effective_rows) = observed_after.unwrap_or((cols, rows));
        self.current_cols.store(effective_cols, Ordering::SeqCst);
        self.current_rows.store(effective_rows, Ordering::SeqCst);
        if let Ok(mut screen_state) = self.screen_state.lock() {
            screen_state.resize(effective_rows, effective_cols);
        }
        let seq = self.seq.load(Ordering::SeqCst);
        self.last_resize_seq.store(seq, Ordering::SeqCst);
        self.resize_count.fetch_add(1, Ordering::SeqCst);
        trace_terminal_event(
            if observed_after == Some((cols, rows)) || observed_after.is_none() {
                "resize"
            } else {
                "resize_actual_mismatch"
            },
            serde_json::json!({
                "path": self.key,
                "requested_cols": cols,
                "requested_rows": rows,
                "cached_cols": previous_cols,
                "cached_rows": previous_rows,
                "actual_before_cols": observed_before.map(|(actual_cols, _)| actual_cols),
                "actual_before_rows": observed_before.map(|(_, actual_rows)| actual_rows),
                "actual_after_cols": observed_after.map(|(actual_cols, _)| actual_cols),
                "actual_after_rows": observed_after.map(|(_, actual_rows)| actual_rows),
                "effective_cols": effective_cols,
                "effective_rows": effective_rows,
            }),
        );
        Ok(())
    }

    fn shutdown(&self, stop_command: Option<&str>) -> Result<()> {
        let mut child = self.child.lock().expect("pty child lock poisoned");
        if let Some(command) = stop_command
            && !command.is_empty()
        {
            let _ = self.write(command);
            let normalized = command.trim();
            let (attempts, sleep_ms) = if normalized == "exit" {
                (2usize, 50u64)
            } else {
                (4usize, 75u64)
            };
            for _ in 0..attempts {
                if child
                    .try_wait()
                    .context("checking terminal exit state")?
                    .is_some()
                {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(sleep_ms));
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }

    fn shutdown_with_force_after(
        self,
        stop_command: Option<&str>,
        force_after: Duration,
    ) -> Result<()> {
        if let Some(command) = stop_command
            && !command.is_empty()
        {
            let _ = self.write(command);
        }
        let key = self.key.clone();
        thread::spawn(move || {
            let started = Instant::now();
            loop {
                {
                    let mut child = self.child.lock().expect("pty child lock poisoned");
                    match child.try_wait() {
                        Ok(Some(_)) => {
                            trace_terminal_event(
                                "graceful_shutdown_completed",
                                serde_json::json!({ "path": key }),
                            );
                            return;
                        }
                        Ok(None) if started.elapsed() >= force_after => {
                            let _ = child.kill();
                            let _ = child.wait();
                            trace_terminal_event(
                                "graceful_shutdown_forced",
                                serde_json::json!({
                                    "path": key,
                                    "force_after_ms": force_after.as_millis(),
                                }),
                            );
                            return;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            trace_terminal_event(
                                "graceful_shutdown_probe_failed",
                                serde_json::json!({
                                    "path": key,
                                    "error": error.to_string(),
                                }),
                            );
                            return;
                        }
                    }
                }
                thread::sleep(Duration::from_secs(5));
            }
        });
        Ok(())
    }
}

fn truncate_terminal_trace_sample(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 180 {
        return trimmed.to_string();
    }
    trimmed.chars().take(180).collect::<String>()
}

fn trace_terminal_event(name: &str, payload: serde_json::Value) {
    if let Ok(home) = resolve_yggterm_home() {
        append_trace_event(&home, "server", "terminal_runtime", name, payload);
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn shell_command(launch_command: &str, cwd: Option<&str>) -> CommandBuilder {
    if cfg!(windows) {
        let mut command = CommandBuilder::new("cmd.exe");
        command.arg("/C");
        command.arg(launch_command);
        for key in terminal_identity_env_removals() {
            command.env_remove(key);
        }
        for (key, value) in terminal_identity_env_pairs() {
            command.env(key, value);
        }
        if let Some(cwd) = cwd {
            command.cwd(cwd);
        }
        return command;
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut command = CommandBuilder::new(shell);
    command.arg("-c");
    let wrapped_launch_command = if launch_command_looks_like_remote_resume_attach(launch_command) {
        remote_resume_attach_shell_command(launch_command)
    } else {
        launch_command.to_string()
    };
    command.arg(wrapped_launch_command);
    for key in terminal_identity_env_removals() {
        command.env_remove(key);
    }
    for (key, value) in terminal_identity_env_pairs() {
        command.env(key, value);
    }
    if let Some(cwd) = cwd {
        if shell_uses_bash_prompt_cwd() {
            command.env("YGGTERM_START_CWD", cwd);
            command.env(
                "PROMPT_COMMAND",
                r#"cd -- "$YGGTERM_START_CWD"; unset PROMPT_COMMAND"#,
            );
        }
        command.cwd(cwd);
    }
    command
}

fn launch_command_looks_like_remote_resume_attach(launch_command: &str) -> bool {
    launch_command.contains("server'\\'' '\\''remote'\\'' '\\''resume-codex")
        || launch_command.contains("server'\\'' '\\''remote'\\'' '\\''start-codex")
}

fn remote_resume_attach_shell_command(launch_command: &str) -> String {
    let trimmed = launch_command.trim_start();
    let launch =
        if trimmed.starts_with("exec ") || trimmed.starts_with("__yggterm_initial_tty_size=") {
            launch_command.to_string()
        } else {
            format!("exec {launch_command}")
        };
    format!("stty raw -echo opost onlcr </dev/tty >/dev/tty 2>/dev/null || true; {launch}")
}

fn terminal_key_prefers_initial_screen_snapshot(key: &str, launch_command: &str) -> bool {
    key.starts_with("remote-session://")
        || key.starts_with("codex-runtime://")
        || launch_command_looks_like_remote_resume_attach(launch_command)
}

fn initial_remote_attach_should_preserve_retained_chunks(
    key: &str,
    launch_command: &str,
    chunks: &[TerminalChunk],
) -> bool {
    if !(key.starts_with("remote-session://")
        || launch_command_looks_like_remote_resume_attach(launch_command))
    {
        return false;
    }
    chunks
        .iter()
        .any(|chunk| terminal_chunk_has_scrollback_text(&chunk.data))
}

fn select_remote_retained_initial_chunks(
    key: &str,
    launch_command: &str,
    chunks: &VecDeque<TerminalChunk>,
) -> Vec<TerminalChunk> {
    let mut selected = select_initial_attach_chunks_for_launch(chunks, launch_command);
    if !(key.starts_with("remote-session://")
        || launch_command_looks_like_remote_resume_attach(launch_command))
        || selected
            .iter()
            .any(|chunk| terminal_chunk_has_scrollback_text(&chunk.data))
    {
        return selected;
    }
    let Some(seed) = chunks
        .iter()
        .find(|chunk| terminal_chunk_has_scrollback_text(&chunk.data))
        .cloned()
    else {
        return selected;
    };
    selected.retain(|chunk| chunk.seq != seed.seq);
    let mut merged = Vec::with_capacity(selected.len().saturating_add(1));
    merged.push(seed);
    merged.extend(selected);
    merged
}

fn shell_uses_bash_prompt_cwd() -> bool {
    std::env::var("SHELL")
        .ok()
        .and_then(|value| {
            std::path::Path::new(&value)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("bash"))
        })
        .unwrap_or(true)
}

fn trim_chunk_buffer(
    chunks: &mut VecDeque<TerminalChunk>,
    retained_bytes: &mut usize,
    max_chunks: usize,
    max_bytes: usize,
) {
    while chunks.len() > max_chunks || *retained_bytes > max_bytes {
        let Some(chunk) = chunks.pop_front() else {
            *retained_bytes = 0;
            break;
        };
        *retained_bytes = retained_bytes.saturating_sub(chunk.data.len());
    }
}

fn select_initial_attach_chunks(chunks: &VecDeque<TerminalChunk>) -> Vec<TerminalChunk> {
    if chunks.is_empty() {
        return Vec::new();
    }

    let mut trailing_noise = Vec::new();
    let mut anchor_index = None;
    for (ix, chunk) in chunks.iter().enumerate().rev() {
        if terminal_chunk_has_visible_text(&chunk.data) {
            anchor_index = Some(ix);
            break;
        }
        if trailing_noise.len() < INITIAL_ATTACH_TRAILING_NOISE_CHUNKS {
            trailing_noise.push(ix);
        }
    }

    let Some(anchor_index) = anchor_index else {
        return select_initial_attach_tail(chunks, None);
    };

    let preserved_trailing = trailing_noise.into_iter().rev().collect::<Vec<_>>();
    let trailing_chunk_budget = preserved_trailing.len();
    let trailing_byte_budget = preserved_trailing
        .iter()
        .filter_map(|ix| chunks.get(*ix))
        .map(|chunk| chunk.data.len())
        .sum::<usize>();

    let available_chunk_budget = INITIAL_ATTACH_MAX_CHUNKS.saturating_sub(trailing_chunk_budget);
    let available_byte_budget = INITIAL_ATTACH_MAX_BYTES.saturating_sub(trailing_byte_budget);
    let leading = select_initial_attach_tail(
        chunks,
        Some((anchor_index, available_chunk_budget, available_byte_budget)),
    );

    let mut selected = leading;
    for ix in preserved_trailing {
        if let Some(chunk) = chunks.get(ix).cloned() {
            selected.push(chunk);
        }
    }
    trim_initial_attach_low_signal_suffix(&mut selected);
    selected
}

fn select_initial_attach_chunks_for_launch(
    chunks: &VecDeque<TerminalChunk>,
    launch_command: &str,
) -> Vec<TerminalChunk> {
    if launch_command_looks_like_remote_resume_attach(launch_command) {
        return select_initial_attach_chunks(chunks);
    }
    select_initial_attach_chunks(chunks)
}

fn initial_attach_should_replay_screen_snapshot(
    key: &str,
    launch_command: &str,
    retained_initial: &[TerminalChunk],
    snapshot_data: &str,
) -> bool {
    if !terminal_snapshot_looks_like_full_screen_surface(snapshot_data) {
        return false;
    }
    // Per per [[project-purpose]] wrapper-vs-manual parity: this gate used
    // to check `terminal_chunk_has_scrollback_text` PER CHUNK (>= 40 non-
    // empty lines in a SINGLE chunk). Codex emits many small chunks, so
    // every chunk failed the per-chunk test → the snapshot replaced the
    // historical chunks → user lost scrollback. The equivalent manual
    // `ssh -t <machine> codex resume <UUID>` typed into a local shell
    // skipped this gate entirely (local:// keys don't match the third
    // condition below) and served raw chunks, giving full scrollback in
    // xterm.js naturally. To restore parity, evaluate scrollback content
    // across the COMBINED retained chunks. When the union has enough
    // non-empty lines to count as a scrollback-worthy session, prefer the
    // raw chunks over the viewport-only snapshot so the GUI sees the
    // same byte stream the manual case sees.
    if retained_initial
        .iter()
        .any(|chunk| terminal_chunk_has_scrollback_text(&chunk.data))
    {
        return false;
    }
    let combined_non_empty_lines = retained_initial
        .iter()
        .map(|chunk| {
            let stripped = strip_terminal_control_sequences(&chunk.data);
            stripped
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .sum::<usize>();
    if combined_non_empty_lines >= usize::from(DEFAULT_ROWS).saturating_add(4) {
        return false;
    }
    key.starts_with("live::")
        || terminal_key_prefers_initial_screen_snapshot(key, launch_command)
        || launch_command_looks_like_remote_resume_attach(launch_command)
}

fn terminal_snapshot_looks_like_full_screen_surface(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 3 {
        return false;
    }
    let normalized = lines.join("\n").to_ascii_lowercase();
    if normalized.contains("yggterm tui smoke")
        || normalized.contains("f1help")
        || normalized.contains("f10quit")
        || normalized.contains("openai codex")
        || normalized.contains("working")
        || normalized.contains("htop")
    {
        return true;
    }
    let printable = stripped
        .chars()
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .count();
    let max_line_len = lines.iter().map(|line| line.len()).max().unwrap_or(0);
    printable >= 72 && max_line_len >= 20
}

fn trim_initial_attach_low_signal_suffix(selected: &mut Vec<TerminalChunk>) {
    if selected.is_empty()
        || !selected
            .iter()
            .any(|chunk| terminal_chunk_has_meaningful_attach_text(&chunk.data))
    {
        return;
    }
    while selected.len() > 1 {
        let Some(last) = selected.last() else {
            break;
        };
        if !terminal_chunk_is_disposable_initial_attach_suffix(&last.data) {
            break;
        }
        selected.pop();
    }
}

fn select_initial_attach_tail(
    chunks: &VecDeque<TerminalChunk>,
    anchor: Option<(usize, usize, usize)>,
) -> Vec<TerminalChunk> {
    let mut selected = Vec::new();
    let mut bytes = 0usize;
    let (limit_index, chunk_budget, byte_budget) = anchor.unwrap_or((
        chunks.len().saturating_sub(1),
        INITIAL_ATTACH_MAX_CHUNKS,
        INITIAL_ATTACH_MAX_BYTES,
    ));
    for (ix, chunk) in chunks.iter().enumerate().rev() {
        if ix > limit_index {
            continue;
        }
        let chunk_len = chunk.data.len();
        if !selected.is_empty()
            && (selected.len() >= chunk_budget || bytes.saturating_add(chunk_len) > byte_budget)
        {
            break;
        }
        bytes = bytes.saturating_add(chunk_len);
        selected.push(chunk.clone());
    }
    selected.reverse();
    selected
}

fn terminal_chunk_has_visible_text(data: &str) -> bool {
    let (data, _) = terminal_data_without_attach_ready_markers(data);
    let stripped = strip_terminal_control_sequences(&data);
    stripped.chars().any(|ch| !ch.is_whitespace())
}

pub fn terminal_data_has_scrollback_text(data: &str) -> bool {
    terminal_chunk_has_scrollback_text(data)
}

fn terminal_chunk_has_scrollback_text(data: &str) -> bool {
    let (data, _) = terminal_data_without_attach_ready_markers(data);
    let stripped = strip_terminal_control_sequences(&data);
    let non_empty_lines = stripped
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    non_empty_lines >= usize::from(DEFAULT_ROWS).saturating_add(4)
}

fn terminal_chunk_has_meaningful_attach_text(data: &str) -> bool {
    let (data, _) = terminal_data_without_attach_ready_markers(data);
    let stripped = strip_terminal_control_sequences(&data);
    let lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() || terminal_chunk_has_generic_attach_idle_footer(&stripped) {
        return false;
    }
    let printable = stripped
        .chars()
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .count();
    let word_count = lines
        .iter()
        .map(|line| line.split_whitespace().count())
        .sum::<usize>();
    let prompt_like = lines.len() <= 2
        && printable < 40
        && lines.iter().any(|line| {
            line.starts_with('›')
                || line.ends_with('$')
                || line.ends_with('#')
                || line.ends_with('>')
                || line.ends_with('%')
        });
    if prompt_like {
        return false;
    }
    printable >= 48 || lines.len() >= 2 || word_count >= 8
}

fn terminal_chunk_is_disposable_initial_attach_suffix(data: &str) -> bool {
    let (data, saw_attach_ready_marker) = terminal_data_without_attach_ready_markers(data);
    if saw_attach_ready_marker && data.trim().is_empty() {
        return true;
    }
    let stripped = strip_terminal_control_sequences(&data);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return true;
    }
    if terminal_chunk_has_generic_attach_idle_footer(&stripped) {
        return true;
    }
    if terminal_chunk_is_attach_model_footer_fragment(&stripped) {
        return true;
    }
    if terminal_chunk_mentions_generic_attach_prompt(&stripped) {
        return true;
    }
    if terminal_chunk_has_meaningful_attach_text(&stripped) {
        return false;
    }
    terminal_chunk_is_low_signal_attach_fragment(&stripped)
}

fn terminal_data_without_attach_ready_markers(data: &str) -> (String, bool) {
    if !data.contains("__YGGTERM_ATTACH_READY__") {
        return (data.to_string(), false);
    }
    let mut cleaned = data
        .lines()
        .filter(|line| !line.contains("__YGGTERM_ATTACH_READY__"))
        .collect::<Vec<_>>()
        .join("\n");
    if !cleaned.is_empty() && data.ends_with('\n') {
        cleaned.push('\n');
    }
    (cleaned, true)
}

fn terminal_chunk_is_low_signal_attach_fragment(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized = stripped.trim().to_ascii_lowercase();
    if normalized.contains("^[[?")
        || normalized.contains("^[]10;")
        || normalized.contains("^[[1;1r")
        || (normalized.contains("rgb:") && normalized.contains("cccc/cccc/cccc"))
    {
        return true;
    }
    let lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return true;
    }
    let printable = stripped
        .chars()
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .count();
    let max_line_len = lines.iter().map(|line| line.len()).max().unwrap_or(0);
    printable <= 6 || (lines.len() == 1 && max_line_len <= 18)
}

fn terminal_chunk_has_generic_attach_idle_footer(data: &str) -> bool {
    let lines = data
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.len() > 5 {
        return false;
    }
    let normalized = lines.join("\n").to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    let mentions_generic_prompt = terminal_chunk_mentions_generic_attach_prompt(data);
    let mentions_model_footer = (normalized.contains("gpt-5")
        || normalized.contains("gpt-4")
        || normalized.contains("claude"))
        && normalized.contains("% left");
    mentions_generic_prompt && mentions_model_footer
}

fn terminal_chunk_is_attach_model_footer_fragment(data: &str) -> bool {
    let normalized = data.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    (normalized.contains("gpt-5") || normalized.contains("gpt-4") || normalized.contains("claude"))
        && normalized.contains("% left")
}

fn terminal_chunk_mentions_generic_attach_prompt(data: &str) -> bool {
    data.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.starts_with('›')
                && (lower.contains("implement {feature}")
                    || lower.contains("explain this codebase")
                    || lower.contains("find and fix a bug")
                    || lower.contains("resume a previous session")
                    || lower.contains("write tests for")
                    || lower.contains("@filename")
                    || lower.contains("review my changes")
                    || lower.contains("summarize recent commits")
                    || lower.contains("create a pr"))
        })
}

fn strip_terminal_control_sequences(input: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Normal,
        Escape,
        Csi,
        Osc,
        OscEscape,
        StringTerminator,
    }

    let mut state = State::Normal;
    let mut out = String::with_capacity(input.len());

    for ch in input.chars() {
        match state {
            State::Normal => {
                if ch == '\u{1b}' {
                    state = State::Escape;
                } else if !ch.is_control() || matches!(ch, '\n' | '\r' | '\t') {
                    out.push(ch);
                }
            }
            State::Escape => match ch {
                '[' => state = State::Csi,
                ']' => state = State::Osc,
                'P' | 'X' | '^' | '_' => state = State::StringTerminator,
                _ => state = State::Normal,
            },
            State::Csi => {
                if ('@'..='~').contains(&ch) {
                    state = State::Normal;
                }
            }
            State::Osc => match ch {
                '\u{7}' => state = State::Normal,
                '\u{1b}' => state = State::OscEscape,
                _ => {}
            },
            State::OscEscape => {
                state = if ch == '\\' {
                    State::Normal
                } else {
                    State::Osc
                };
            }
            State::StringTerminator => {
                if ch == '\u{1b}' {
                    state = State::OscEscape;
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::mpsc;
    use std::time::Instant;

    #[test]
    fn vt_scrollback_returns_empty_when_no_lines_have_scrolled_off() {
        let mut state = TerminalScreenState::new(24, 80);
        state.process(b"line one\r\nline two\r\n");
        assert!(state.vt_scrollback_plain_rows().is_empty());
    }

    fn parse_history_line_number(text: &str) -> Option<u32> {
        text.trim().strip_prefix("line ")?.parse::<u32>().ok()
    }

    #[test]
    fn vt_scrollback_returns_scrolled_off_rows_oldest_first() {
        let rows: u16 = 5;
        let mut state = TerminalScreenState::new(rows, 80);
        for i in 1..=12 {
            state.process(format!("line {i}\r\n").as_bytes());
        }
        let history = state.vt_scrollback_plain_rows();
        assert!(
            history.len() >= 6,
            "expected at least 6 scrolled-off rows, got {}",
            history.len()
        );
        assert_eq!(history.first().map(|s| s.as_str()), Some("line 1"));
        let history_nums: Vec<u32> = history
            .iter()
            .filter_map(|line| parse_history_line_number(line))
            .collect();
        assert!(
            history_nums.windows(2).all(|w| w[0] < w[1]),
            "history must be strictly increasing (oldest-first), got {:?}",
            history_nums
        );
        let max_history = *history_nums.last().unwrap_or(&0);
        assert!(
            max_history <= 12,
            "history should not contain lines beyond what was written"
        );
    }

    #[test]
    fn history_and_screen_replay_returns_none_when_terminal_is_empty() {
        let mut state = TerminalScreenState::new(24, 80);
        assert!(state.history_and_screen_replay().is_none());
    }

    #[test]
    fn history_and_screen_replay_prepends_scrollback_before_clear_and_viewport() {
        let mut state = TerminalScreenState::new(4, 40);
        for i in 1..=10 {
            state.process(format!("hist-{i}\r\n").as_bytes());
        }
        let replay = state.history_and_screen_replay().expect("payload");
        assert!(replay.contains("hist-1"), "oldest scrollback row must be present");
        assert!(replay.contains("hist-3"), "intermediate scrollback row must be present");
        let clear_idx = replay
            .find("\x1b[2J\x1b[H")
            .expect("clear-visible escape between history and viewport must be present");
        let hist3_idx = replay
            .find("hist-3")
            .expect("history must precede clear-visible escape");
        assert!(
            hist3_idx < clear_idx,
            "history rows must appear before the clear-screen-and-home escape"
        );
    }

    #[test]
    fn terminal_utf8_decoder_preserves_box_drawing_across_read_boundaries() {
        let mut pending = Vec::new();
        let first = decode_terminal_utf8_chunk(&mut pending, &[0xe2, 0x95]);
        assert_eq!(first, "");
        assert_eq!(pending, vec![0xe2, 0x95]);

        let second = decode_terminal_utf8_chunk(&mut pending, &[0xad, b'\n', 0xe2]);
        assert_eq!(second, "╭\n");
        assert_eq!(pending, vec![0xe2]);

        let third = decode_terminal_utf8_chunk(&mut pending, &[0x94, 0x80]);
        assert_eq!(third, "─");
        assert!(pending.is_empty());
    }

    #[test]
    fn terminal_utf8_decoder_flushes_incomplete_trailing_bytes_once() {
        let mut pending = Vec::new();
        assert_eq!(
            decode_terminal_utf8_chunk(&mut pending, &[b'a', 0xe2, 0x95]),
            "a"
        );
        assert_eq!(flush_terminal_utf8_pending(&mut pending), "\u{fffd}");
        assert!(pending.is_empty());
    }

    #[test]
    fn terminal_protocol_filter_answers_default_color_queries() {
        let profile = TerminalProtocolProfile::from_launch_command(
            "export YGGTERM_TERMINAL_APPEARANCE=dark; codex",
        );
        let mut filter = TerminalProtocolFilter::default();

        let result = filter.process("hello\u{1b}]10;?\u{1b}\\mid\u{1b}]11;?\u{7}done", profile);

        assert_eq!(result.data, "hellomiddone");
        assert_eq!(
            result.responses,
            vec![
                "\u{1b}]10;rgb:cccc/cccc/cccc\u{1b}\\".to_string(),
                "\u{1b}]11;rgb:1e1e/1e1e/1e1e\u{1b}\\".to_string(),
            ]
        );
        assert_eq!(
            result
                .answered_queries
                .iter()
                .map(|query| query.label())
                .collect::<Vec<_>>(),
            vec!["10".to_string(), "11".to_string()]
        );
    }

    #[test]
    fn terminal_protocol_filter_holds_split_color_query() {
        let profile =
            TerminalProtocolProfile::from_launch_command("export COLORFGBG='15;0'; codex");
        let mut filter = TerminalProtocolFilter::default();

        let first = filter.process("left\u{1b}]11;?", profile);
        let second = filter.process("\u{1b}\\right", profile);

        assert_eq!(first.data, "left");
        assert!(first.responses.is_empty());
        assert_eq!(second.data, "right");
        assert_eq!(
            second.responses,
            vec!["\u{1b}]11;rgb:1e1e/1e1e/1e1e\u{1b}\\".to_string()]
        );
    }

    #[test]
    fn terminal_protocol_filter_answers_palette_queries_without_visible_leak() {
        let profile = TerminalProtocolProfile::from_launch_command(
            "export YGGTERM_TERMINAL_APPEARANCE=dark; codex",
        );
        let mut filter = TerminalProtocolFilter::default();

        let result = filter.process("pre\u{1b}]4;0;?;1;?;15;?\u{1b}\\post", profile);

        assert_eq!(result.data, "prepost");
        assert_eq!(
            result.responses,
            vec![
                "\u{1b}]4;0;rgb:0000/0000/0000\u{1b}\\".to_string(),
                "\u{1b}]4;1;rgb:cdcd/3131/3131\u{1b}\\".to_string(),
                "\u{1b}]4;15;rgb:e5e5/e5e5/e5e5\u{1b}\\".to_string(),
            ]
        );
        assert_eq!(
            result
                .answered_queries
                .iter()
                .map(|query| query.label())
                .collect::<Vec<_>>(),
            vec!["4:0".to_string(), "4:1".to_string(), "4:15".to_string()]
        );
    }
    #[test]
    fn terminal_protocol_profile_uses_synced_theme_colors_from_launch_command() {
        let launch_command = "\
            export YGGTERM_TERMINAL_APPEARANCE=dark; \
            export YGGTERM_TERMINAL_COLOR_FOREGROUND='#e5e5e5'; \
            export YGGTERM_TERMINAL_COLOR_BACKGROUND='#262a33'; \
            export YGGTERM_TERMINAL_COLOR_0='#111111'; \
            export YGGTERM_TERMINAL_COLOR_1='#222222'; \
            export YGGTERM_TERMINAL_COLOR_2='#333333'; \
            export YGGTERM_TERMINAL_COLOR_3='#444444'; \
            export YGGTERM_TERMINAL_COLOR_4='#555555'; \
            export YGGTERM_TERMINAL_COLOR_5='#666666'; \
            export YGGTERM_TERMINAL_COLOR_6='#777777'; \
            export YGGTERM_TERMINAL_COLOR_7='#888888'; \
            export YGGTERM_TERMINAL_COLOR_8='#999999'; \
            export YGGTERM_TERMINAL_COLOR_9='#aaaaaa'; \
            export YGGTERM_TERMINAL_COLOR_10='#bbbbbb'; \
            export YGGTERM_TERMINAL_COLOR_11='#cccccc'; \
            export YGGTERM_TERMINAL_COLOR_12='#dddddd'; \
            export YGGTERM_TERMINAL_COLOR_13='#eeeeee'; \
            export YGGTERM_TERMINAL_COLOR_14='#ababab'; \
            export YGGTERM_TERMINAL_COLOR_15='#fefefe'; edit";
        let profile = TerminalProtocolProfile::from_launch_command(launch_command);
        let mut filter = TerminalProtocolFilter::default();

        let result = filter.process("\u{1b}]11;?\u{1b}\\\u{1b}]4;0;?;15;?\u{1b}\\", profile);

        assert_eq!(
            result.responses,
            vec![
                "\u{1b}]11;rgb:2626/2a2a/3333\u{1b}\\".to_string(),
                "\u{1b}]4;0;rgb:1111/1111/1111\u{1b}\\".to_string(),
                "\u{1b}]4;15;rgb:fefe/fefe/fefe\u{1b}\\".to_string(),
            ]
        );
    }

    #[test]
    fn terminal_protocol_filter_holds_split_palette_query() {
        let profile =
            TerminalProtocolProfile::from_launch_command("export COLORFGBG='15;0'; codex");
        let mut filter = TerminalProtocolFilter::default();

        let first = filter.process("left\u{1b}]4;0;?;1;?", profile);
        let second = filter.process("\u{1b}\\right", profile);

        assert_eq!(first.data, "left");
        assert!(first.responses.is_empty());
        assert_eq!(second.data, "right");
        assert_eq!(
            second
                .answered_queries
                .iter()
                .map(|query| query.label())
                .collect::<Vec<_>>(),
            vec!["4:0".to_string(), "4:1".to_string()]
        );
        assert!(
            second
                .responses
                .iter()
                .all(|response| response.starts_with("\u{1b}]4;")),
            "{:?}",
            second.responses
        );
    }

    #[test]
    fn terminal_protocol_filter_preserves_palette_set_sequences_for_xterm() {
        let profile =
            TerminalProtocolProfile::from_launch_command("export COLORFGBG='15;0'; codex");
        let mut filter = TerminalProtocolFilter::default();
        let payload = "pre\u{1b}]4;1;rgb:1111/2222/3333\u{1b}\\post";

        let result = filter.process(payload, profile);

        assert_eq!(result.data, payload);
        assert!(result.responses.is_empty());
        assert!(result.answered_queries.is_empty());
    }

    #[test]
    fn terminal_protocol_filter_keeps_cat_crlf_after_palette_query() {
        let profile =
            TerminalProtocolProfile::from_launch_command("export COLORFGBG='15;0'; codex");
        let mut filter = TerminalProtocolFilter::default();

        let result = filter.process(
            "\u{1b}]4;0;?;1;?\u{1b}\\alpha\r\nbeta\r\ngamma\r\n",
            profile,
        );

        assert_eq!(result.data, "alpha\r\nbeta\r\ngamma\r\n");
        assert_eq!(result.responses.len(), 2);
        assert_eq!(
            result
                .answered_queries
                .iter()
                .map(|query| query.label())
                .collect::<Vec<_>>(),
            vec!["4:0".to_string(), "4:1".to_string()]
        );
    }

    #[test]
    fn pty_runtime_answers_default_color_query_to_child() {
        let runtime = PtySessionRuntime::spawn(
            "local://osc-color-query",
            r#"export YGGTERM_TERMINAL_APPEARANCE=dark; python3 - <<'PY'
import os
import select
import sys
import termios
import tty

fd = os.open('/dev/tty', os.O_RDWR | getattr(os, 'O_NOCTTY', 0))
old = termios.tcgetattr(fd)
data = b''
try:
    tty.setraw(fd)
    os.write(fd, b'\x1b]10;?\x1b\\')
    ready, _, _ = select.select([fd], [], [], 2.0)
    if ready:
        data = os.read(fd, 64)
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
    os.close(fd)

expected = b'\x1b]10;rgb:cccc/cccc/cccc\x1b\\'
sys.stdout.write('COLOR_OK\n' if data == expected else f'COLOR_BAD:{data!r}\n')
PY"#,
            None,
            None,
        )
        .expect("spawn OSC color query test runtime");
        let mut combined = String::new();
        for _ in 0..80 {
            let read = runtime.read(0);
            combined = read
                .chunks
                .iter()
                .map(|chunk| chunk.data.as_str())
                .collect::<String>();
            if combined.contains("COLOR_") {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        runtime.shutdown(None).expect("shutdown test runtime");

        assert!(combined.contains("COLOR_OK"), "{combined:?}");
        assert!(!combined.contains("\u{1b}]10;?\u{1b}\\"));
    }

    struct BlockingFirstWrite {
        first_started: mpsc::Sender<()>,
        release_first: mpsc::Receiver<()>,
        writes: Arc<AtomicUsize>,
    }

    impl Write for BlockingFirstWrite {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
                let _ = self.first_started.send(());
                let _ = self.release_first.recv();
            }
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn trim_chunk_buffer_enforces_byte_budget() {
        // Sized so total > MAX_BUFFER_BYTES (135%) — three chunks each at
        // 45% of the budget. After dropping the oldest, total drops to 90%
        // (under budget), so exactly two chunks remain. Mirrors the original
        // pre-bump ratio (900 KB chunks vs old 2 MB budget).
        let chunk_size = (MAX_BUFFER_BYTES * 45) / 100;
        let mut chunks = VecDeque::from([
            TerminalChunk {
                seq: 1,
                data: "a".repeat(chunk_size),
            },
            TerminalChunk {
                seq: 2,
                data: "b".repeat(chunk_size),
            },
            TerminalChunk {
                seq: 3,
                data: "c".repeat(chunk_size),
            },
        ]);
        let mut retained = chunk_size * 3;
        trim_chunk_buffer(&mut chunks, &mut retained, MAX_CHUNKS, MAX_BUFFER_BYTES);
        assert!(retained <= MAX_BUFFER_BYTES);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks.front().map(|chunk| chunk.seq), Some(2));
    }

    #[test]
    fn trim_chunk_buffer_enforces_idle_budget() {
        let mut chunks = VecDeque::from(
            (0..96)
                .map(|ix| TerminalChunk {
                    seq: ix,
                    data: "x".repeat(4096),
                })
                .collect::<Vec<_>>(),
        );
        let mut retained = 96 * 4096;
        trim_chunk_buffer(
            &mut chunks,
            &mut retained,
            IDLE_TRIM_MAX_CHUNKS,
            IDLE_TRIM_MAX_BYTES,
        );
        assert!(chunks.len() <= IDLE_TRIM_MAX_CHUNKS);
        assert!(retained <= IDLE_TRIM_MAX_BYTES);
    }

    #[test]
    fn terminal_manager_renames_runtime_without_respawning_child() {
        let mut manager = TerminalManager::new();
        manager
            .ensure_session("local://codex", "sleep 5", None)
            .expect("spawn test session");
        let pid_before = manager.session_process_id("local://codex");

        assert!(manager.rename_session("local://codex", "codex-runtime://codex"));

        assert!(!manager.has_session("local://codex"));
        assert!(manager.has_session("codex-runtime://codex"));
        assert_eq!(
            manager.session_process_id("codex-runtime://codex"),
            pid_before
        );
        manager
            .remove_session("codex-runtime://codex", None)
            .expect("remove renamed session");
    }

    #[test]
    fn idle_trim_skips_remote_resume_attach_sessions() {
        let runtime = PtySessionRuntime::spawn(
            "remote-session://oc/test",
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''",
            None,
            None,
        )
        .expect("spawn test runtime");
        for seq in 0..96 {
            runtime.seed_snapshot(&format!("chunk-{seq}\n"));
        }
        let before = runtime.buffer_usage().1;
        let reclaimed = runtime.trim_idle_buffer(Duration::from_millis(0));
        let after = runtime.buffer_usage().1;
        assert_eq!(reclaimed, 0);
        assert_eq!(before, after);
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn initial_attach_falls_back_to_screen_snapshot_when_local_chunk_buffer_is_empty() {
        let runtime = PtySessionRuntime::spawn(
            "local://test-shell",
            "printf 'pi@dev:~/gh/yggterm$ echo ready\n'",
            None,
            None,
        )
        .expect("spawn test runtime");
        runtime.seed_snapshot("pi@dev:~/gh/yggterm$ echo ready\n");
        runtime
            .chunks
            .lock()
            .expect("pty chunk lock poisoned")
            .clear();
        runtime.retained_bytes.store(0, Ordering::SeqCst);

        let read = runtime.read(0);
        let combined = read
            .chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("pi@dev:~/gh/yggterm$ echo ready"));
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn spawned_terminal_uses_requested_initial_size() {
        let runtime = PtySessionRuntime::spawn(
            "local://sized-test",
            "bash -lc 'printf sized'",
            None,
            Some((104, 48)),
        )
        .expect("spawn sized test runtime");
        let size = runtime
            .screen_state
            .lock()
            .expect("pty screen state lock poisoned")
            .parser
            .screen()
            .size();

        assert_eq!(size, (48, 104));
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[cfg(unix)]
    #[test]
    fn pty_resize_repairs_kernel_size_when_cache_already_matches_request() {
        let runtime = PtySessionRuntime::spawn(
            "local://resize-cache-drift",
            "bash -lc 'sleep 5'",
            None,
            Some((120, 36)),
        )
        .expect("spawn resize drift test runtime");

        runtime.resize(110, 50).expect("initial resize");
        {
            let master = runtime.master.lock().expect("pty master lock poisoned");
            let size = master.get_size().expect("read resized pty size");
            assert_eq!((size.cols, size.rows), (110, 50));
            master
                .resize(PtySize {
                    rows: 36,
                    cols: 120,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("simulate kernel/cache drift");
        }
        runtime.current_cols.store(110, Ordering::SeqCst);
        runtime.current_rows.store(50, Ordering::SeqCst);

        runtime
            .resize(110, 50)
            .expect("same-size resize should repair drift");
        {
            let master = runtime.master.lock().expect("pty master lock poisoned");
            let size = master.get_size().expect("read repaired pty size");
            assert_eq!((size.cols, size.rows), (110, 50));
        }
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn pty_read_replays_initial_chunks_when_client_cursor_is_from_previous_runtime() {
        let runtime = PtySessionRuntime::spawn(
            "local://cursor-rewind-test",
            "bash -lc 'printf restarted'",
            None,
            None,
        )
        .expect("spawn cursor rewind test runtime");
        let mut combined = String::new();
        for _ in 0..80 {
            let read = runtime.read(9999);
            combined = read
                .chunks
                .iter()
                .map(|chunk| chunk.data.as_str())
                .collect::<String>();
            if combined.contains("restarted") {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        runtime.shutdown(None).expect("shutdown test runtime");

        assert!(combined.contains("restarted"), "{combined:?}");
    }

    #[test]
    fn initial_attach_selection_keeps_last_meaningful_surface_ahead_of_trailing_noise() {
        let mut chunks = VecDeque::new();
        chunks.push_back(TerminalChunk {
            seq: 1,
            data: "saved transcript line\n".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 2,
            data: "\u{1b}[2J\u{1b}[HOpenAI Codex (v0.118.0)\n/model to change\n".to_string(),
        });
        for seq in 3..260 {
            chunks.push_back(TerminalChunk {
                seq,
                data: "\u{1b}[20;3H \r \n".to_string(),
            });
        }

        let selected = select_initial_attach_chunks(&chunks);
        let combined = selected
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("OpenAI Codex"));
    }

    #[test]
    fn initial_attach_selection_trims_low_signal_suffix_after_meaningful_transcript() {
        let mut chunks = VecDeque::new();
        chunks.push_back(TerminalChunk {
            seq: 1,
            data: "  - Push: origin/main updated successfully (2f6b4ac..f49ab56)\n".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 2,
            data: "B".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 3,
            data: "oo".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 4,
            data: "rvco".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 5,
            data: "›Explain this codebase  gpt-5.4 high fast · 100% left · ~/git\n".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 6,
            data: "\n".to_string(),
        });

        let selected = select_initial_attach_chunks(&chunks);
        let seqs = selected.iter().map(|chunk| chunk.seq).collect::<Vec<_>>();
        let combined = selected
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert_eq!(seqs, vec![1]);
        assert!(combined.contains("origin/main updated successfully"));
        assert!(!combined.contains("Explain this codebase"));
    }

    #[test]
    fn initial_attach_selection_trims_write_tests_footer_suffix() {
        let mut chunks = VecDeque::new();
        chunks.push_back(TerminalChunk {
            seq: 1,
            data: "  - Push: origin/main updated successfully (2f6b4ac..f49ab56)\n".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 2,
            data: "› Write tests for @filename\n".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 3,
            data: "  gpt-5.4 high fast · 100% left · ~/git\n".to_string(),
        });

        let selected = select_initial_attach_chunks(&chunks);
        let combined = selected
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("origin/main updated successfully"));
        assert!(!combined.contains("Write tests for @filename"));
    }

    #[test]
    fn initial_attach_selection_keeps_prompt_only_surface_when_no_meaningful_history_exists() {
        let mut chunks = VecDeque::new();
        chunks.push_back(TerminalChunk {
            seq: 1,
            data: "pi@oc:~$ ".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 2,
            data: "\u{1b}[?25h".to_string(),
        });

        let selected = select_initial_attach_chunks(&chunks);
        let seqs = selected.iter().map(|chunk| chunk.seq).collect::<Vec<_>>();

        assert_eq!(seqs, vec![1, 2]);
    }

    #[test]
    fn terminal_chunk_visible_text_ignores_ansi_noise() {
        assert!(!terminal_chunk_has_visible_text(
            "\u{1b}[20;3H \r \n\u{1b}[K"
        ));
        assert!(terminal_chunk_has_visible_text(
            "\u{1b}[2J\u{1b}[HOpenAI Codex (v0.118.0)\n"
        ));
    }

    #[test]
    fn launch_command_detects_remote_resume_attach() {
        let launch_command = "ssh -tt jojo 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''019ce5d8-c94c-7b62-ae19-3818ae400b65'\\'' '\\''/home/pi'\\'''";
        let start_command = "ssh -tt jojo 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''start-codex'\\'' '\\''019ce5d8-c94c-7b62-ae19-3818ae400b65'\\'' '\\''/home/pi'\\'''";

        assert!(launch_command_looks_like_remote_resume_attach(
            launch_command
        ));
        assert!(launch_command_looks_like_remote_resume_attach(
            start_command
        ));
        assert!(!launch_command_looks_like_remote_resume_attach(
            "bash -lc 'ls'"
        ));
    }

    #[test]
    fn remote_resume_attach_shell_command_preserves_tty_settle_prefix() {
        let launch_command = "__yggterm_initial_tty_size=$(stty size 2>/dev/null || true); unset __yggterm_initial_tty_size; exec ssh -tt jojo 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''";

        let wrapped = remote_resume_attach_shell_command(launch_command);

        assert!(wrapped.starts_with(
            "stty raw -echo opost onlcr </dev/tty >/dev/tty 2>/dev/null || true; __yggterm_initial_tty_size="
        ));
        assert!(!wrapped.contains("; exec __yggterm_initial_tty_size="));
        assert!(wrapped.contains("; exec ssh -tt jojo"));
        assert!(wrapped.contains("'resume-codex'"));
    }

    #[test]
    fn remote_resume_attach_shell_command_execs_plain_ssh_command() {
        let launch_command = "ssh -tt jojo 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''";

        let wrapped = remote_resume_attach_shell_command(launch_command);

        assert!(wrapped.starts_with(
            "stty raw -echo opost onlcr </dev/tty >/dev/tty 2>/dev/null || true; exec ssh -tt jojo"
        ));
    }

    #[test]
    fn runtime_owned_terminal_keys_prefer_initial_screen_snapshot() {
        assert!(terminal_key_prefers_initial_screen_snapshot(
            "remote-session://jojo/test",
            "bash -lc 'sleep 30'",
        ));
        assert!(terminal_key_prefers_initial_screen_snapshot(
            "codex-runtime://test",
            "bash -lc 'sleep 30'",
        ));
        assert!(terminal_key_prefers_initial_screen_snapshot(
            "local://legacy-resume",
            "ssh -tt jojo 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''",
        ));
        assert!(terminal_key_prefers_initial_screen_snapshot(
            "local://fresh-start",
            "ssh -tt jojo 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''start-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''",
        ));
        assert!(!terminal_key_prefers_initial_screen_snapshot(
            "local://plain",
            "bash -lc 'sleep 30'",
        ));
    }

    #[test]
    fn initial_remote_resume_attach_trims_to_tail_budget() {
        let mut chunks = VecDeque::new();
        for seq in 1..=260 {
            chunks.push_back(TerminalChunk {
                seq,
                data: format!("chunk-{seq}\n"),
            });
        }

        let selected = select_initial_attach_chunks_for_launch(
            &chunks,
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''",
        );

        assert!(selected.len() < chunks.len());
        assert_eq!(selected.first().map(|chunk| chunk.seq), Some(69));
        assert_eq!(selected.last().map(|chunk| chunk.seq), Some(260));
    }

    #[test]
    fn initial_remote_resume_attach_preserves_retained_scrollback() {
        let runtime = PtySessionRuntime::spawn(
            "remote-session://dev/retained-scrollback",
            "sh -lc 'sleep 30'",
            None,
            None,
        )
        .expect("spawn test runtime");
        let seeded_scrollback = (1..=80)
            .map(|line| format!("YGG_REMOTE_RETAINED_SCROLLBACK_{line:03}\n"))
            .collect::<String>();
        runtime.seed_snapshot(&seeded_scrollback);
        runtime.attach_ready_seen.store(true, Ordering::SeqCst);

        let result = runtime.read(0);
        let combined = result
            .chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("YGG_REMOTE_RETAINED_SCROLLBACK_001"));
        assert!(combined.contains("YGG_REMOTE_RETAINED_SCROLLBACK_080"));
        assert!(combined.contains("__YGGTERM_ATTACH_READY__"));
        assert!(
            combined.matches("YGG_REMOTE_RETAINED_SCROLLBACK_").count() >= 80,
            "{combined:?}"
        );
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn terminal_manager_retained_snapshot_exposes_full_history_for_ui_replay() {
        let mut manager = TerminalManager::new();
        let key = "remote-session://dev/ui-retained-history";
        manager
            .ensure_session(key, "sh -lc 'sleep 30'", None)
            .expect("spawn retained history session");
        let retained = (1..=96)
            .map(|line| format!("YGG_UI_RETAINED_HISTORY_{line:03}\n"))
            .collect::<String>();
        manager
            .seed_session(key, &retained)
            .expect("seed retained history");

        let snapshot = manager
            .session_snapshot(key)
            .expect("retained snapshot exists");

        assert!(snapshot.contains("YGG_UI_RETAINED_HISTORY_001"));
        assert!(snapshot.contains("YGG_UI_RETAINED_HISTORY_096"));
        assert!(
            snapshot.matches("YGG_UI_RETAINED_HISTORY_").count() >= 96,
            "{snapshot:?}"
        );
        manager
            .remove_session(key, None)
            .expect("shutdown retained history session");
    }

    #[test]
    fn terminal_manager_reports_post_resize_output_fence() {
        let mut manager = TerminalManager::new();
        let key = "remote-session://dev/resize-fence";
        manager
            .ensure_session(key, "sh -lc 'sleep 30'", None)
            .expect("spawn resize fence session");
        manager
            .seed_session(key, "pre-resize retained separator\n")
            .expect("seed pre-resize output");

        let before_resize = manager.read(key, 0).expect("read before resize");
        assert!(before_resize.post_resize_output_seen);
        assert_eq!(before_resize.last_resize_seq, 0);

        manager.resize(key, 110, 50).expect("resize session");
        let after_resize = manager.read(key, 0).expect("read after resize");
        assert!(!after_resize.post_resize_output_seen);
        assert_eq!(after_resize.last_resize_seq, before_resize.cursor);
        assert!(!manager.session_post_resize_output_seen(key));

        manager
            .seed_session(key, "post-resize prompt surface\n")
            .expect("seed post-resize output");
        let after_output = manager.read(key, 0).expect("read after output");
        assert!(after_output.post_resize_output_seen);
        assert_eq!(after_output.last_resize_seq, after_resize.last_resize_seq);
        assert!(
            after_output
                .chunks
                .iter()
                .any(|chunk| chunk.seq > after_output.last_resize_seq),
            "{:?}",
            after_output.chunks
        );

        manager
            .remove_session(key, None)
            .expect("shutdown resize fence session");
    }

    #[test]
    fn terminal_same_size_resize_after_sized_restart_does_not_open_resize_fence() {
        let mut manager = TerminalManager::new();
        let key = "remote-session://dev/sized-restart";
        manager
            .restart_session_with_size(key, "sh -lc 'sleep 30'", None, None, Some((110, 50)))
            .expect("spawn sized restart session");
        manager
            .seed_session(key, "post-restart prompt surface\n")
            .expect("seed prompt output");

        let before_resize = manager.read(key, 0).expect("read before same-size resize");
        assert!(before_resize.post_resize_output_seen);
        assert_eq!(before_resize.last_resize_seq, 0);

        manager
            .resize(key, 110, 50)
            .expect("same-size resize should be a no-op");
        let after_resize = manager.read(key, 0).expect("read after same-size resize");
        assert!(
            after_resize.post_resize_output_seen,
            "same-size resize must not fence fresh restart output"
        );
        assert_eq!(
            after_resize.last_resize_seq, 0,
            "same-size resize must not mark retained prompt output pre-resize"
        );

        manager
            .remove_session(key, None)
            .expect("shutdown sized restart session");
    }

    #[test]
    fn terminal_manager_session_keys_exclude_exited_runtime() {
        let mut manager = TerminalManager::new();
        let key = "local://exited-runtime";
        manager
            .ensure_session(key, "sh -lc 'printf exited'", None)
            .expect("spawn short runtime");

        let deadline = Instant::now() + Duration::from_secs(3);
        while manager.session_is_running(key) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(
            !manager.session_is_running(key),
            "short runtime should exit during the test"
        );
        assert!(
            !manager.session_keys().iter().any(|value| value == key),
            "exited runtime must not be advertised as a live terminal session"
        );
    }

    #[test]
    fn terminal_manager_ensure_restarts_exited_runtime() {
        let mut manager = TerminalManager::new();
        let key = "local://restart-exited-runtime";
        manager
            .ensure_session(key, "sh -lc 'printf first'", None)
            .expect("spawn first short runtime");

        let deadline = Instant::now() + Duration::from_secs(3);
        while manager.session_is_running(key) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !manager.session_is_running(key),
            "first runtime should exit during the test"
        );

        manager
            .ensure_session(key, "sh -lc 'sleep 30'", None)
            .expect("ensure should replace an exited runtime");
        assert!(
            manager.session_is_running(key),
            "ensure_session must recreate an exited runtime"
        );
        manager.remove_session(key, None).expect("remove session");
    }

    #[test]
    fn initial_remote_resume_attach_recovers_older_seed_scrollback_after_tail_noise() {
        let mut chunks = VecDeque::new();
        let seed = (1..=80)
            .map(|line| format!("YGG_REMOTE_SEED_SCROLLBACK_{line:03}\n"))
            .collect::<String>();
        chunks.push_back(TerminalChunk { seq: 1, data: seed });
        for seq in 2..260 {
            chunks.push_back(TerminalChunk {
                seq,
                data: format!("\u{1b}[Htail-frame-{seq}\n"),
            });
        }

        let selected = select_remote_retained_initial_chunks(
            "remote-session://dev/retained-scrollback",
            "sh -lc 'sleep 30'",
            &chunks,
        );
        let combined = selected
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("YGG_REMOTE_SEED_SCROLLBACK_001"));
        assert!(combined.contains("YGG_REMOTE_SEED_SCROLLBACK_080"));
        assert!(combined.contains("tail-frame-259"));
    }

    #[test]
    fn terminal_manager_reports_missing_remote_scrollback_after_tail_only() {
        let mut manager = TerminalManager::new();
        let key = "remote-session://dev/retained-switch-tail-only";
        manager
            .ensure_session(key, "sh -lc 'sleep 30'", None)
            .expect("spawn test runtime");

        manager
            .seed_session(key, "› Use /skills to list available skills\n")
            .expect("seed tail-only runtime");
        assert!(!manager.session_initial_read_has_scrollback(key));

        let seeded_scrollback = (1..=80)
            .map(|line| format!("YGG_REMOTE_SWITCH_SCROLLBACK_{line:03}\n"))
            .collect::<String>();
        manager
            .seed_session(key, &seeded_scrollback)
            .expect("seed retained scrollback");
        assert!(manager.session_initial_read_has_scrollback(key));

        let summary = manager.shutdown_all(|_| None);
        assert_eq!(summary.errors, Vec::<String>::new());
    }

    #[test]
    fn attach_ready_markers_do_not_count_as_visible_scrollback() {
        let marker_only = "__YGGTERM_ATTACH_READY__\n".repeat(80);

        assert!(!terminal_chunk_has_visible_text(&marker_only));
        assert!(!terminal_chunk_has_scrollback_text(&marker_only));
        assert!(!terminal_chunk_has_meaningful_attach_text(&marker_only));
        assert!(terminal_chunk_is_disposable_initial_attach_suffix(
            &marker_only
        ));

        let (cleaned, saw_marker) = terminal_data_without_attach_ready_markers(&format!(
            "real output\n{}next output\n",
            marker_only
        ));
        assert!(saw_marker);
        assert_eq!(cleaned, "real output\nnext output\n");
    }

    #[test]
    fn initial_remote_resume_attach_appends_attach_ready_marker() {
        let runtime = PtySessionRuntime::spawn(
            "remote-session://oc/test",
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''",
            None,
            None,
        )
        .expect("spawn test runtime");
        runtime.seed_snapshot(
            "› Use /skills to list available skills\n\n  gpt-5.4 high fast · 100% left · ~/git\n",
        );
        runtime.attach_ready_seen.store(true, Ordering::SeqCst);

        let result = runtime.read(0);
        let combined = result
            .chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("__YGGTERM_ATTACH_READY__"));
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn initial_remote_resume_attach_uses_raw_pty_bytes_not_screen_snapshot_state() {
        let runtime = PtySessionRuntime::spawn(
            "remote-session://oc/test",
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''",
            None,
            None,
        )
        .expect("spawn test runtime");
        runtime.seed_snapshot("abcdef\rXYZ");

        let result = runtime.read(0);
        let combined = result
            .chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("abcdef\rXYZ"));
        assert!(!combined.contains("XYZdef"));
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn initial_remote_resume_attach_does_not_fabricate_screen_snapshot_over_stale_prose_tail() {
        let runtime = PtySessionRuntime::spawn(
            "remote-session://oc/stale-prose",
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''stale-prose'\\'' '\\''/home/pi'\\'''",
            None,
            Some((100, 50)),
        )
        .expect("spawn test runtime");
        runtime.seed_snapshot(
            "\u{1b}[2J\u{1b}[H\u{1b}[48;1H› Write tests for @filename\n  gpt-5.5 xhigh · ~/gh/yggterm",
        );
        {
            let stale_tail = "The commit and signed tag are pushed. I’m creating the GitHub release directly with the Linux installer archive, companion binaries, `.deb`, and checksums so the curl installer can resolve `v2.1.44` immediately; the tag workflow can still add any matrix artifacts afterward.\n";
            let mut chunks = runtime.chunks.lock().expect("pty chunk lock poisoned");
            chunks.clear();
            chunks.push_back(TerminalChunk {
                seq: 1,
                data: stale_tail.to_string(),
            });
            runtime
                .retained_bytes
                .store(stale_tail.len(), Ordering::SeqCst);
            runtime.seq.store(1, Ordering::SeqCst);
        }

        let result = runtime.read(0);
        let combined = result
            .chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();
        let visible = strip_terminal_control_sequences(&combined);

        assert!(!combined.contains("__YGGTERM_ATTACH_READY__"));
        assert!(!visible.contains("› Write tests for @filename"));
        assert!(visible.contains("GitHub release directly"));
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn initial_runtime_owned_attach_keeps_raw_retained_tail_instead_of_screen_snapshot() {
        let runtime = PtySessionRuntime::spawn(
            "remote-session://oc/test",
            "bash -lc 'sleep 30'",
            None,
            Some((100, 64)),
        )
        .expect("spawn test runtime");
        runtime.seed_snapshot(
            "\u{1b}[2J\u{1b}[H\u{1b}[61;1H› Run /review on my current changes\n  gpt-5.5 medium · ~/gh/yggterm",
        );
        {
            let mut chunks = runtime.chunks.lock().expect("pty chunk lock poisoned");
            chunks.clear();
            let stale_tail = "\u{1b}[60;1H›\u{1b}[61;1H› Run /review on my current changes\n";
            chunks.push_back(TerminalChunk {
                seq: 1,
                data: stale_tail.to_string(),
            });
            runtime
                .retained_bytes
                .store(stale_tail.len(), Ordering::SeqCst);
            runtime.seq.store(1, Ordering::SeqCst);
        }

        let result = runtime.read(0);
        let combined = result
            .chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();
        let visible = strip_terminal_control_sequences(&combined);

        assert!(!combined.contains("__YGGTERM_ATTACH_READY__"));
        assert!(combined.contains("\u{1b}[60;1H›"));
        assert_eq!(visible.matches('›').count(), 2, "{visible:?}");
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn initial_live_tui_attach_replays_current_screen_snapshot_over_incremental_tail() {
        let runtime = PtySessionRuntime::spawn(
            "live::tui-reattach",
            "bash -lc 'sleep 30'",
            None,
            Some((100, 36)),
        )
        .expect("spawn test runtime");
        let full_frame = "\u{1b}[2J\
\u{1b}[1;1HYGGTERM TUI SMOKE frame 104\
\u{1b}[2;1HTasks: smoke heavy terminal\
\u{1b}[3;1HMem[||||||||||||||||||||                    ] 52%\
\u{1b}[4;1HF1Help F2Setup F10Quit";
        let incremental_delta = "\u{1b}[1;25H418\u{1b}[3;5H||||||||||||||||||||||";
        runtime.seed_snapshot(full_frame);
        runtime.seed_snapshot(incremental_delta);
        {
            let mut chunks = runtime.chunks.lock().expect("pty chunk lock poisoned");
            chunks.clear();
            chunks.push_back(TerminalChunk {
                seq: 2,
                data: incremental_delta.to_string(),
            });
            runtime
                .retained_bytes
                .store(incremental_delta.len(), Ordering::SeqCst);
            runtime.seq.store(2, Ordering::SeqCst);
        }

        let result = runtime.read(0);
        let combined = result
            .chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();
        let visible = strip_terminal_control_sequences(&combined);

        assert!(
            visible.contains("YGGTERM TUI SMOKE frame 418"),
            "{visible:?}"
        );
        assert!(
            visible.contains("Tasks: smoke heavy terminal"),
            "{visible:?}"
        );
        assert!(visible.contains("F1Help F2Setup F10Quit"), "{visible:?}");
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn initial_local_attach_does_not_append_attach_ready_marker() {
        let runtime =
            PtySessionRuntime::spawn("local://test", "bash -lc 'printf hello'", None, None)
                .expect("spawn local test runtime");
        runtime.seed_snapshot("hello\n");

        let result = runtime.read(0);
        let combined = result
            .chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(!combined.contains("__YGGTERM_ATTACH_READY__"));
        runtime.shutdown(None).expect("shutdown test runtime");
    }

    #[test]
    fn spawned_terminal_shell_removes_no_color_from_child_env() {
        let previous = std::env::var_os("NO_COLOR");
        unsafe {
            std::env::set_var("NO_COLOR", "1");
        }
        let runtime = PtySessionRuntime::spawn(
            "local://env-test",
            "python3 -c 'import os,sys; sys.stdout.write(os.getenv(\"NO_COLOR\", \"<unset>\"))'",
            None,
            None,
        )
        .expect("spawn env test runtime");
        let mut combined = String::new();
        for _ in 0..40 {
            let read = runtime.read(0);
            combined = read
                .chunks
                .iter()
                .map(|chunk| chunk.data.as_str())
                .collect::<String>();
            if !combined.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        runtime.shutdown(None).expect("shutdown test runtime");
        match previous {
            Some(value) => unsafe { std::env::set_var("NO_COLOR", value) },
            None => unsafe { std::env::remove_var("NO_COLOR") },
        }
        assert!(combined.contains("<unset>"));
        assert!(!combined.contains("1"));
    }

    #[test]
    fn remote_resume_initial_attach_drops_terminal_negotiation_suffix() {
        let mut chunks = VecDeque::new();
        chunks.push_back(TerminalChunk {
            seq: 1,
            data: "Done. Added these in the ThinkBook x layer.\n".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 2,
            data: "› ^[[?1;2c^[]10;rgb:cccc/cccc/cccc^[\\^[[1;1R\n".to_string(),
        });

        let selected = select_initial_attach_chunks_for_launch(
            &chunks,
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/pi'\\'''",
        );
        let combined = selected
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("Done. Added these in the ThinkBook x layer."));
        assert!(!combined.contains("^[[?1;2c"));
        assert!(!combined.contains("^[]10;rgb:cccc/cccc/cccc"));
    }

    #[test]
    fn terminal_write_queue_reports_backpressure_without_blocking_request_thread() {
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let writes = Arc::new(AtomicUsize::new(0));
        let writer = BlockingFirstWrite {
            first_started: first_started_tx,
            release_first: release_first_rx,
            writes: Arc::clone(&writes),
        };
        let writer_tx = spawn_terminal_writer_thread(
            "local://blocked".to_string(),
            Box::new(writer),
            Arc::new(AtomicU64::new(0)),
            1,
        )
        .expect("spawn writer");

        enqueue_terminal_write(
            &writer_tx,
            "local://blocked",
            "first",
            1,
            TerminalWriteAckMode::Enqueued,
        )
        .expect("enqueue first write");
        first_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer should start first write");
        enqueue_terminal_write(
            &writer_tx,
            "local://blocked",
            "second",
            1,
            TerminalWriteAckMode::Enqueued,
        )
        .expect("enqueue second write behind blocked writer");

        let started = Instant::now();
        let error = enqueue_terminal_write(
            &writer_tx,
            "local://blocked",
            "third",
            1,
            TerminalWriteAckMode::Enqueued,
        )
        .expect_err("full queue should fail fast");

        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(error.to_string().contains("terminal input queue is full"));
        release_first_tx.send(()).expect("release blocked writer");
        drop(writer_tx);
    }

    #[test]
    fn terminal_write_flush_ack_waits_for_writer_thread() {
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let writes = Arc::new(AtomicUsize::new(0));
        let writer = BlockingFirstWrite {
            first_started: first_started_tx,
            release_first: release_first_rx,
            writes,
        };
        let writer_tx = spawn_terminal_writer_thread(
            "local://flush-ack".to_string(),
            Box::new(writer),
            Arc::new(AtomicU64::new(0)),
            1,
        )
        .expect("spawn writer");

        let write_tx = writer_tx.clone();
        thread::spawn(move || {
            let result = enqueue_terminal_write(
                &write_tx,
                "local://flush-ack",
                "first",
                1,
                TerminalWriteAckMode::Flushed,
            )
            .map_err(|error| error.to_string());
            result_tx.send(result).expect("send write result");
        });

        first_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer should start first write");
        assert!(
            result_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "flushed terminal writes must not acknowledge before the writer flushes the PTY"
        );

        release_first_tx.send(()).expect("release blocked writer");
        result_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("write should finish after writer flushes")
            .expect("write should succeed");
        drop(writer_tx);
    }
}
