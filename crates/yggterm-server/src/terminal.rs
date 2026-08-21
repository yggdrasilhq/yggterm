use crate::app_declare::{
    AppDeclareLog, AppDeclareRecord, AppDeclareScanner, attach_replay_neutralizes_web_surface_open,
    rewrite_consumed_web_surface_opens,
};
use crate::codex_cli::{
    TerminalIdentityColorProfile, normalize_terminal_identity_color,
    terminal_identity_appearance_from_environment,
    terminal_identity_color_profile_from_environment, terminal_identity_env_pairs,
    terminal_identity_env_removals,
};
use crate::pty_adoption::PtyChildHandle;
use anyhow::{Context, Result, bail};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vt100::Parser as Vt100Parser;
use yggterm_core::{append_bounded_jsonl_record, append_trace_event, resolve_yggterm_home};

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
    /// A HUMAN is typing into this composer, so nothing was written and nothing
    /// was cleared.
    ///
    /// ⛔ **The probe is destructive to a person mid-sentence and that is why
    /// this exists.** It writes a marker, and when the marker does not echo it
    /// sends Ctrl+U — which wipes the line the human is composing — then does it
    /// again every ~300 ms for the whole timeout. Against a row someone is
    /// typing at, a single 30 s submit is ~100 injected markers and ~100 erased
    /// lines: the viewport flickers, the keystrokes come out interleaved with
    /// `yggterm_ready_probe`, and the row is unusable until the timeout ends.
    /// Reported live 2026-08-14 as *"blinking profusely and I could not type"*.
    HumanTyping { waited_ms: u64 },
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
        Self::from_launch_command_with_host_profile(
            launch_command,
            terminal_identity_color_profile_from_environment(),
        )
    }

    /// The host's ambient color profile is an INPUT, not an ambient read.
    ///
    /// It used to be read inside this function, which made every test of the
    /// launch-command path depend on the environment the suite happened to run
    /// in — and the suite is usually run from a terminal INSIDE yggterm, whose
    /// PTY carries `YGGTERM_TERMINAL_COLOR_*`. Three colour tests failed only
    /// there, which is the worst kind of failure: it looks like the change under
    /// test broke something.
    fn from_launch_command_with_host_profile(
        launch_command: &str,
        host_profile: Option<TerminalIdentityColorProfile>,
    ) -> Self {
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
            .or(host_profile)
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
        self.process_with_cursor(data, profile, None)
    }

    fn process_with_cursor(
        &mut self,
        data: &str,
        profile: TerminalProtocolProfile,
        cursor_position: Option<(u16, u16)>,
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

        // CSI DSR/DA queries: filter them from visible and synthesize responses.
        // These are the queries that cause prompt_toolkit's CPR timeout when the
        // client xterm is not mounted (background session). The daemon answers
        // them directly from its own vt100 model so the TUI never waits 2 s.
        // Known queries:
        //   ESC[6n  -> CPR (cursor position report) -> ESC[row;colR (1-based)
        //   ESC[5n  -> DSR status -> ESC[0n (ready)
        //   ESC[c / ESC[0c -> DA primary -> ESC[?1;2c (VT100)
        let osc_visible = visible;
        let mut filtered_visible = String::with_capacity(osc_visible.len());
        let mut i = 0usize;
        while i < osc_visible.len() {
            if osc_visible[i..].starts_with("\u{1b}[") {
                let rest = &osc_visible[i + 2..];
                if let Some(term_offset) = rest.find(|c: char| c.is_ascii_alphabetic()) {
                    let seq_end = i + 2 + term_offset + 1;
                    let seq = &osc_visible[i..seq_end];
                    // DSR 6n - cursor position report
                    if seq == "\u{1b}[6n" {
                        let (r, c) = cursor_position.unwrap_or((0, 0));
                        // vt100 parser is 0-based, DSR is 1-based
                        let row = r.saturating_add(1).max(1);
                        let col = c.saturating_add(1).max(1);
                        responses.push(format!("\u{1b}[{row};{col}R"));
                        i = seq_end;
                        continue;
                    }
                    // DSR 5n - device status report
                    if seq == "\u{1b}[5n" {
                        responses.push("\u{1b}[0n".to_string());
                        i = seq_end;
                        continue;
                    }
                    // DA - device attributes (primary)
                    if seq == "\u{1b}[c" || seq == "\u{1b}[0c" {
                        responses.push("\u{1b}[?1;2c".to_string());
                        i = seq_end;
                        continue;
                    }
                    // Keep other CSI as visible
                    filtered_visible.push_str(seq);
                    i = seq_end;
                    continue;
                }
                // Incomplete CSI - treat rest as pending? For now keep as visible
                filtered_visible.push_str(&osc_visible[i..]);
                break;
            } else {
                let ch = osc_visible[i..].chars().next().unwrap();
                filtered_visible.push(ch);
                i += ch.len_utf8();
            }
        }

        TerminalProtocolFilterResult {
            data: filtered_visible,
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

/// One live session's handoff inputs, gathered while this daemon still owns it.
///
/// ⛔ **`master_fd` is BORROWED.** It is the runtime's own descriptor, valid
/// only while that runtime is still in the map. It is deliberately not an
/// `OwnedFd`: the sender must not close it, because `sendmsg` duplicates it
/// into the receiver and the predecessor then releases it by EXITING, which is
/// the ownership decision the spike settled.
///
/// ⛔ **Do not "tidy" this by dropping the runtime after a successful send.**
/// A `PtySessionRuntime`'s reader thread holds its OWN cloned master, so
/// dropping the runtime leaves that thread alive, still holding the pty open
/// and still consuming bytes that now belong to the successor — two daemons
/// reading one PTY, and the shell never seeing EOF. The predecessor hands off
/// everything and then retires as a process; that is what releases the
/// descriptors, and it is why the handoff is all-or-nothing rather than
/// per-session.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub struct HandoffTakeout {
    pub master_fd: std::os::fd::RawFd,
    pub shell_pid: u32,
    pub shell_start_time: u64,
    pub cols: u16,
    pub rows: u16,
    pub screen: String,
    pub launch_command: String,
    pub cwd: Option<String>,
}

/// What seating a handed-over PTY under a key would do.
///
/// Three answers, and collapsing any two of them has already cost this project
/// a daemon that could never retire — see [`TerminalManager::seat_verdict`].
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatVerdict {
    /// Nothing live under this key, or only an exited husk. Seat it.
    Vacant,
    /// This key is already seated on the very process being handed over: the
    /// earlier adoption succeeded and only its acknowledgement was lost.
    AlreadySeated,
    /// A DIFFERENT live child holds this key. Seating would kill one pty to
    /// install another.
    Conflict,
}

/// The one wording of a seat conflict, so the pre-commit refusal and the
/// post-commit one cannot describe the same fact in two ways.
#[cfg(target_os = "linux")]
pub fn seat_conflict_reason(key: &str) -> String {
    format!("refusing to adopt {key}: this daemon already runs a live PTY for it")
}

#[derive(Debug, Clone)]
pub struct TerminalShutdownSummary {
    pub stopped: usize,
    pub errors: Vec<String>,
}

/// The switch that separates **holding** a pty descriptor from **serving** it.
///
/// ⛔ **This exists because a handover used to conflate the two.** A retiring
/// daemon released its master fds the only way it could — by exiting — so it let
/// go on the successor's *acceptance* and never on its *survival*. Anything that
/// killed a young successor in that window destroyed every session on the host,
/// with no process left holding anything.
///
/// Parking a reader stops it consuming bytes while its runtime, and therefore
/// every descriptor that runtime owns, stays exactly where it is. That is what
/// lets a predecessor wait out a settle interval before it exits: it is holding
/// the pty open for a successor it has not yet trusted, without stealing a
/// single byte from it.
///
/// **Why it is a poll and not a flag.** A flag can only be read *after* a
/// blocking `read` returns — which means the parked reader still swallows one
/// chunk that now belongs to the successor, and an idle session swallows it
/// whenever its next output happens to arrive, possibly seconds later. The
/// reader therefore blocks in `poll` over the pty **and** a wake descriptor, so
/// the park is observed with no bytes in hand and the pending output stays in
/// the kernel buffer for whoever reads next. [`Self::stolen_after_park`] counts
/// the bytes that still slipped through — the race between `poll` reporting
/// readable and the park landing — because a window this code claims is closed
/// should be measured rather than asserted.
pub struct ReaderPark {
    parked: AtomicBool,
    /// Set by the reader while it is standing down, cleared when it resumes.
    /// The park is a REQUEST until this says the reader acted on it.
    stood_down: AtomicBool,
    stolen_after_park: AtomicU64,
    /// The wake side of the reader's `poll`. `None` where the platform has no
    /// such primitive — there the park degrades to a flag the reader notices
    /// after its next read, which is all the non-Linux builds need because the
    /// pty handoff is Linux-only.
    wake: Option<std::os::fd::OwnedFd>,
}

impl ReaderPark {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            parked: AtomicBool::new(false),
            stood_down: AtomicBool::new(false),
            stolen_after_park: AtomicU64::new(0),
            wake: new_reader_wake_fd(),
        })
    }

    /// A park with no reader behind it — what a session whose pty already died
    /// looks like to a sweep. Only a test has any use for one.
    #[cfg(all(target_os = "linux", test))]
    pub fn detached_for_test() -> Arc<Self> {
        Self::new()
    }

    /// Ask the reader to stop consuming. Returns immediately: use
    /// [`Self::has_stood_down`] to learn whether it actually has.
    pub fn park(&self) {
        self.parked.store(true, Ordering::SeqCst);
        self.wake();
    }

    /// Resume serving. Safe to call on a reader that never parked.
    pub fn unpark(&self) {
        self.parked.store(false, Ordering::SeqCst);
        self.wake();
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    /// Whether the reader has been seen standing down. `false` also means "this
    /// reader thread is gone" — a dead pty's reader exited long ago and will
    /// never answer, which is why no caller may block until every park is
    /// acknowledged.
    pub fn has_stood_down(&self) -> bool {
        self.stood_down.load(Ordering::SeqCst)
    }

    /// Bytes consumed after the park was requested. Expected to be 0; a nonzero
    /// value is the poll/park race actually happening, and it is a hole in the
    /// successor's transcript.
    pub fn stolen_after_park(&self) -> u64 {
        self.stolen_after_park.load(Ordering::SeqCst)
    }

    fn wake(&self) {
        #[cfg(target_os = "linux")]
        if let Some(wake) = self.wake.as_ref() {
            use std::os::fd::AsRawFd;
            let value: u64 = 1;
            // Best effort by construction: a full counter already means "wake",
            // and the reader re-reads `parked` after every wake anyway.
            unsafe {
                libc::write(
                    wake.as_raw_fd(),
                    std::ptr::addr_of!(value).cast(),
                    std::mem::size_of::<u64>(),
                );
            }
        }
    }
}

/// The wake descriptor a parked reader blocks on. An `eventfd` rather than a
/// pipe: one fd instead of two, and its counter is level-triggered, so a wake
/// that arrives before the reader reaches `poll` is not lost.
#[cfg(target_os = "linux")]
fn new_reader_wake_fd() -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    (fd >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
}

#[cfg(not(target_os = "linux"))]
fn new_reader_wake_fd() -> Option<std::os::fd::OwnedFd> {
    None
}

/// What the reader should do next.
enum ReaderGateVerdict {
    /// The pty has bytes, or has hung up — take the same read the loop always
    /// took.
    Read,
    /// The gate itself failed. Reported through the loop's existing read-error
    /// branch rather than a second encoding of "this reader is finished".
    Failed(String),
}

/// The reader thread's half of [`ReaderPark`]: it owns the descriptor it polls.
///
/// A **dup** of the master, not the master itself: the runtime's own master can
/// be dropped while this thread is still alive, and polling a closed fd number
/// is how a thread ends up waiting on whatever file was opened next.
struct ReaderGate {
    park: Arc<ReaderPark>,
    #[cfg(target_os = "linux")]
    poll_fd: Option<std::os::fd::OwnedFd>,
}

impl ReaderGate {
    /// Block until there is something to read, standing down for as long as the
    /// park is held. Never returns while parked, and never touches the pty while
    /// parked — that is the whole contract.
    #[cfg(target_os = "linux")]
    fn wait(&self) -> ReaderGateVerdict {
        use std::os::fd::AsRawFd;
        loop {
            let parked = self.park.parked.load(Ordering::SeqCst);
            self.park.stood_down.store(parked, Ordering::SeqCst);
            let (Some(poll_fd), Some(wake_fd)) = (self.poll_fd.as_ref(), self.park.wake.as_ref())
            else {
                // No gate on this session: behave exactly as the pre-park
                // reader did and let the blocking read be the wait.
                return ReaderGateVerdict::Read;
            };
            let mut fds = [
                libc::pollfd {
                    fd: wake_fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    // ⛔ A parked reader watches ONLY the wake fd. Watching the
                    // pty as well would be harmless for `poll` itself, but it
                    // costs a wakeup per byte the successor is being handed.
                    fd: if parked { -1 } else { poll_fd.as_raw_fd() },
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return ReaderGateVerdict::Failed(format!("polling pty reader: {error}"));
            }
            if fds[0].revents != 0 {
                drain_reader_wake(wake_fd.as_raw_fd());
                continue;
            }
            if parked {
                continue;
            }
            if fds[1].revents != 0 {
                // POLLIN, POLLHUP and POLLERR all mean "read now": hangup is
                // delivered to the existing `Ok(0)` / `Err` branches, which own
                // what a finished pty means.
                return ReaderGateVerdict::Read;
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn wait(&self) -> ReaderGateVerdict {
        ReaderGateVerdict::Read
    }
}

#[cfg(target_os = "linux")]
fn drain_reader_wake(fd: std::os::fd::RawFd) {
    let mut value: u64 = 0;
    unsafe {
        libc::read(
            fd,
            std::ptr::addr_of_mut!(value).cast(),
            std::mem::size_of::<u64>(),
        );
    }
}

/// A dup of the master purely for readiness. Shares the open file description
/// with the reader's own clone, so it reports exactly the readiness that clone
/// would see.
#[cfg(target_os = "linux")]
fn dup_master_for_poll(master: &(dyn MasterPty + Send)) -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let raw = master.as_raw_fd()?;
    let duped = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 0) };
    (duped >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(duped) })
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
        // ⛔ THE PTY IS CREATED AT THE GRID THE VIEWER ACTUALLY HAS — no per-CLI
        // clamp. A previous revision shrank eight named CLIs to 120x40 here, on
        // the premise that they "render at a fixed width (e.g. 100 cols)" and so
        // a smaller PTY would make them fill the viewport. Measured against the
        // daemon's own vt100 (`scripts/cli-viewport-probe`), the premise is false
        // and the arithmetic never worked:
        //   * given a 173x63 PTY, grok paints to column 171, opencode to 172 and
        //     pi to 173 — they fill whatever grid they are handed;
        //   * the one CLI that genuinely renders narrow paints the same 102
        //     columns at 120 as at 173, so the clamp did not help it either;
        //   * shrinking the PTY cannot make a TUI fill the VIEWPORT, because the
        //     viewport is xterm's grid and that is unchanged — it only shrinks
        //     the app's world and leaves the remainder dead.
        // The dead margin that motivated the clamp is the STALE-GEOMETRY bug
        // (a PTY left at a default/preserved size while the client renders
        // wider), and clamping made that symptom permanent instead of curing it:
        // the restart path has no grid resync behind it, so every clamped row
        // stayed 120x40 inside a full-size viewport until something re-attached.
        // ⇒ An axis here must be a property of WHERE THE PTY LIVES, never of
        // WHICH CLI is talking (`agent_arm_shell_matrix.rs`). Reading the CLI was
        // the hole. Locked by `pty_is_created_at_the_requested_grid_for_every_cli`.
        let runtime = PtySessionRuntime::spawn(key, launch_command, cwd, initial_size)?;
        self.sessions.insert(key.to_string(), runtime);
        Ok(())
    }

    /// What seating `key` would do — decided WITHOUT a descriptor in hand.
    ///
    /// ⛔⛔ **THE ONE ENCODING OF THE SEAT RULE, AND IT MUST STAY ONE.** Two
    /// callers ask this question at two different moments: the handoff listener
    /// asks it BEFORE the fd crosses so it can answer the predecessor while a
    /// refusal is still free, and [`Self::adopt_session`] asks it again with the
    /// fd already in hand, because the two moments are not the same instant and
    /// another handoff can seat the key in between. A second copy of the
    /// predicate would let those two answers drift, and the drift is invisible:
    /// both paths would still look right in isolation.
    ///
    /// ⚠ **IDENTITY, NOT PRESENCE — AND THE DIFFERENCE PINS WHOLE DAEMONS.**
    /// The handoff has a window between the successor SEATING a pty and the
    /// predecessor receiving the ack. When the ack is lost, the predecessor
    /// books a failure and keeps its runtime, then retries — and every retry
    /// landed on "I already have this key", read as a conflict. **It is the
    /// opposite: it is proof the earlier adoption succeeded.** The predecessor
    /// could therefore never complete the move, and because a failure makes the
    /// sweep `Partial` (`classify_handoff_sweep`), a single key in this state
    /// pinned EVERY other session on that daemon and it could never reach the
    /// empty hands that let it retire.
    ///
    /// Measured live 2026-08-14 during a version bump: the same key failing once
    /// a minute, `NoneMoved`, `readers_stood_down: 11` and `moved: 0` — eleven
    /// healthy runtimes held hostage by one.
    ///
    /// ⚠ So the check is the CHILD, not the key. "A live pty exists under this
    /// key" is also true when this daemon built its OWN pty for the session (an
    /// independent re-resume), and calling that success would let the
    /// predecessor drop a runtime whose child is still on the far end — closing
    /// a pty out from under a live process. Same pid, AND the same start time so
    /// a recycled pid cannot impersonate it.
    #[cfg(target_os = "linux")]
    pub fn seat_verdict(&self, key: &str, shell_pid: u32, shell_start_time: u64) -> SeatVerdict {
        let Some(existing) = self.sessions.get(key) else {
            return SeatVerdict::Vacant;
        };
        if !existing.is_running() {
            return SeatVerdict::Vacant;
        }
        let same_child = existing.process_id() == Some(shell_pid)
            && crate::pty_adoption::process_start_time(shell_pid)
                .is_some_and(|start| start == shell_start_time);
        if same_child {
            SeatVerdict::AlreadySeated
        } else {
            SeatVerdict::Conflict
        }
    }

    /// Install a session whose PTY was received from another daemon.
    ///
    /// Refuses to displace a RUNNING runtime under the same key: the whole
    /// point of the handoff is that the successor did not already have this
    /// session, and silently replacing a live one would kill a PTY to install
    /// another. An exited husk under the key is replaced, exactly as
    /// [`Self::ensure_session_with_size`] does.
    ///
    /// ⚠ Still asks [`Self::seat_verdict`] even when the listener already did:
    /// the pre-commit answer was true a moment ago, and this is the moment that
    /// actually seats the pty.
    #[cfg(target_os = "linux")]
    #[allow(clippy::too_many_arguments)]
    pub fn adopt_session(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
        fd: std::os::fd::OwnedFd,
        shell_pid: u32,
        shell_start_time: u64,
        seed: Option<&str>,
    ) -> Result<()> {
        match self.seat_verdict(key, shell_pid, shell_start_time) {
            SeatVerdict::Vacant => {}
            SeatVerdict::AlreadySeated => {
                // Idempotent success. `fd` drops here, closing this duplicate
                // descriptor for a pty we already hold open — which is why it
                // cannot hang up on the child.
                trace_terminal_event(
                    "adopt_already_seated",
                    serde_json::json!({
                        "path": key,
                        "shell_pid": shell_pid,
                        "shell_start_time": shell_start_time,
                        "reason": "this key is already seated on the very process being \
                                   handed over — the earlier adoption succeeded and only \
                                   its acknowledgement was lost",
                    }),
                );
                return Ok(());
            }
            SeatVerdict::Conflict => bail!("{}", seat_conflict_reason(key)),
        }
        if let Some(runtime) = self.sessions.remove(key) {
            let _ = runtime.shutdown(None);
        }
        let runtime = PtySessionRuntime::adopt(
            key,
            launch_command,
            cwd,
            cols,
            rows,
            fd,
            shell_pid,
            shell_start_time,
            seed,
        )?;
        self.sessions.insert(key.to_string(), runtime);
        Ok(())
    }

    /// Stand one session's reader down and hand back the switch that wakes it.
    ///
    /// The runtime is untouched: the descriptors stay open, the writer stays
    /// live, the child keeps running. Only the consuming stops. That is the
    /// distinction the pty handoff needs and the one the old code could not
    /// make — see [`ReaderPark`].
    #[cfg(target_os = "linux")]
    pub fn park_reader(&self, key: &str) -> Option<Arc<ReaderPark>> {
        let park = Arc::clone(&self.sessions.get(key)?.reader_park);
        park.park();
        Some(park)
    }

    /// Everything a handoff needs about one live session, gathered while we
    /// still own it.
    ///
    /// `master_fd` is BORROWED, not owned: it stays valid only while this
    /// runtime is still in the map. That is deliberate — see
    /// [`Self::handoff_takeout`].
    #[cfg(target_os = "linux")]
    pub fn handoff_takeout(&self, key: &str) -> Option<HandoffTakeout> {
        let runtime = self.sessions.get(key)?;
        let shell_pid = runtime.process_id()?;
        let shell_start_time = crate::pty_adoption::process_start_time(shell_pid)?;
        let cols = runtime.current_cols.load(Ordering::SeqCst);
        let rows = runtime.current_rows.load(Ordering::SeqCst);
        Some(HandoffTakeout {
            master_fd: runtime.master.lock().ok()?.as_raw_fd()?,
            shell_pid,
            shell_start_time,
            cols,
            rows,
            screen: runtime.screen_snapshot(),
            launch_command: runtime.launch_command.clone(),
            cwd: runtime.cwd.clone(),
        })
    }

    /// Suspend/wake recovery: kill and immediately respawn every RUNNING
    /// ssh-carried session (remote resume/attach bridges, ssh shells). After a
    /// laptop suspend the bridges' TCP connections are dead but ssh hangs
    /// silently — ServerAlive takes ~45s to notice, and only then does the
    /// exit-driven re-resume lane fire. The wake watcher calls this the moment
    /// a suspend gap is detected, so recovery costs one ssh handshake instead
    /// of a keepalive timeout. Local (non-ssh) sessions are untouched — their
    /// PTYs survive suspend fine.
    pub fn respawn_ssh_carried_sessions(&mut self) -> Vec<(String, bool)> {
        let keys: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_key, session)| {
                launch_command_is_ssh_carried(&session.launch_command) && session.is_running()
            })
            .map(|(key, _session)| key.clone())
            .collect();
        let mut results = Vec::new();
        for key in keys {
            let Some(runtime) = self.sessions.remove(&key) else {
                continue;
            };
            let launch_command = runtime.launch_command.clone();
            let cwd = runtime.cwd.clone();
            let cols = runtime.current_cols.load(Ordering::SeqCst);
            let rows = runtime.current_rows.load(Ordering::SeqCst);
            let size = (cols > 0 && rows > 0).then_some((cols, rows));
            trace_terminal_event(
                "suspend_wake_bridge_respawn",
                serde_json::json!({
                    "path": key,
                    "cols": cols,
                    "rows": rows,
                }),
            );
            let _ = runtime.shutdown(None);
            let respawned = self
                .ensure_session_with_size(&key, &launch_command, cwd.as_deref(), size)
                .is_ok();
            results.push((key, respawned));
        }
        results
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

    /// Has this session's PTY child EXITED?
    ///
    /// ⛔ Deliberately NOT `!session_is_running()`. That accessor folds a failed
    /// probe into `false`, which is right for a display question and wrong for
    /// anything that CHANGES STATE on the answer: `waitpid` really can fail, and
    /// a swallowed error would then read as "the process exited" and let a
    /// caller mark a perfectly healthy row dead. Callers that act must be able
    /// to tell "it exited" from "I could not find out".
    ///
    /// `None` = no such session here, or the probe itself failed.
    pub fn session_has_exited(&self, key: &str) -> Option<bool> {
        let session = self.sessions.get(key)?;
        let mut child = session.child.lock().ok()?;
        child.is_running().ok().map(|running| !running)
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

    /// The current runtime's spawn id (0 = no runtime). See `PtySessionRuntime::spawn_id`.
    pub fn session_runtime_spawn_id(&self, key: &str) -> u64 {
        self.sessions
            .get(key)
            .map(|session| session.spawn_id)
            .unwrap_or(0)
    }

    pub fn session_idle_for_ms(&self, key: &str) -> Option<u64> {
        self.sessions.get(key).map(|session| session.idle_for_ms())
    }

    /// How long since the CHILD last produced output.
    ///
    /// ⛔ Not the same question as [`Self::session_idle_for_ms`], which reads a
    /// field the writer also stamps — so a row that has stopped reading its PTY
    /// reports near-zero idle for as long as someone keeps typing at it. Any
    /// caller asking *"is this session in use?"* wants THIS one: being typed at
    /// is not being in use, and a deaf row answering "active" is what let an
    /// unusable session block a deploy indefinitely.
    pub fn session_output_idle_for_ms(&self, key: &str) -> Option<u64> {
        self.sessions
            .get(key)
            .map(|session| now_millis().saturating_sub(session.last_output_ms.load(Ordering::SeqCst)))
    }

    /// `Some(true)` when the owned session has typed-but-unsent input on its
    /// current line, `Some(false)` when the line is clean, `None` when this
    /// daemon does not own the session (so the migration predicate must bias to
    /// "not migratable"). See `PtySessionRuntime::has_pending_input_draft`.
    pub fn session_has_pending_input_draft(&self, key: &str) -> Option<bool> {
        self.sessions
            .get(key)
            .map(|session| session.has_pending_input_draft())
    }

    /// Atomic conditional submit — see
    /// [`PtySessionRuntime::submit_if_line_equals`]. `NotOwned` when this
    /// daemon does not hold the runtime, which a caller must not confuse with a
    /// refusal.
    pub fn session_submit_if_line_equals(
        &self,
        key: &str,
        expected: &str,
    ) -> SubmitIffLineVerdict {
        match self.sessions.get(key) {
            Some(session) => session.submit_if_line_equals(expected),
            None => SubmitIffLineVerdict::NotOwned,
        }
    }

    pub fn session_process_id(&self, key: &str) -> Option<u32> {
        self.sessions
            .get(key)
            .and_then(|session| session.process_id())
    }

    /// Did this runtime arrive by ADOPTION — i.e. do we SHARE its child with
    /// whoever handed it to us?
    ///
    /// ⛔⛔ THE DISTINCTION IS FATAL AND IT IS WHY THIS EXISTS. A runtime we
    /// spawned owns its child alone, so dropping it is cleanup. An ADOPTED
    /// runtime is the same process on the same pty as the predecessor's copy —
    /// so asking that predecessor to drop its side **kills the child we are
    /// serving**, because a drop is `remove_session` → `shutdown` → `kill`.
    ///
    /// Measured 2026-08-14: the duplicate-prune fired twice with
    /// `removed_terminal: true`, and the two outcomes were opposite. Against a
    /// runtime created by a genuine re-launch it removed the STALE side and the
    /// live process survived — correct. Against a runtime we had ADOPTED after a
    /// lost handoff ack, it removed the side a live agent was mid-turn on, and
    /// the transcript's last write lands in the same second as the drop.
    ///
    /// ⚠ In that second case **neither side was stale**, so "drop the duplicate"
    /// had no right answer to give — which is exactly why the caller must not
    /// ask the question rather than the prune trying to choose better.
    #[cfg(target_os = "linux")]
    pub fn session_is_adopted(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.child_is_adopted())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn session_is_adopted(&self, _key: &str) -> bool {
        false
    }

    /// The pid of the PTY's foreground process group leader — what the user
    /// would be typing at if they opened this row. Tenant accounting starts its
    /// "what is running in here" answer from this, and it is the only value the
    /// daemon can get without walking anything.
    pub fn session_foreground_process_group_leader(&self, key: &str) -> Option<u32> {
        self.sessions
            .get(key)
            .and_then(|session| session.foreground_process_group_leader())
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

    /// The session's VISIBLE screen as plain-text rows — the rendered grid a
    /// person sees, not the escape stream that paints it. See
    /// `PtySessionRuntime::vt_screen_plain_rows` for why every screen
    /// classifier reads this and not `session_screen_snapshot`.
    pub fn session_screen_plain_rows(&self, key: &str) -> Option<Vec<String>> {
        self.sessions
            .get(key)
            .map(|session| session.screen_plain_rows())
    }

    /// Does this session's composer hold typed-but-unsent text?
    ///
    /// ⭐ ONE OWNER FOR THE ONE QUESTION A WRITER MUST NOT GET WRONG, and it is
    /// a UNION of two independent readings because each is blind where the
    /// other sees:
    ///
    /// * `pending_input_draft` — reconstructed from the bytes forwarded through
    ///   this daemon's `write`. Exact, byte-level, TOCTOU-free. ⛔ But it is
    ///   built from zero when a runtime is created, so a session ADOPTED by a
    ///   newer daemon starts reading "clean" while the person's sentence is
    ///   still standing in the composer. Handovers are routine, so on this
    ///   arm alone the guard fails OPEN after every one of them.
    /// * the RENDERED COMPOSER ROW — what a person can see, which survives any
    ///   handover because it lives in the vt100 grid rather than in a counter.
    ///   ⛔ But it cannot see a line the CLI has not drawn yet.
    ///
    /// ⇒ Either arm saying "there is text there" is enough to refuse, and
    /// `None` means neither could answer — which is not permission. The costs
    /// are not symmetric: a needless refusal delays a wake, and a wrong "it is
    /// empty" spends somebody's unsent sentence.
    pub fn session_composer_holds_draft(&self, key: &str) -> Option<bool> {
        let keystrokes = self.session_has_pending_input_draft(key);
        let grid = self
            .session_screen_plain_rows(key)
            .and_then(|rows| yggterm_core::composer_row_holds_text(&rows));
        match (keystrokes, grid) {
            (None, None) => None,
            (left, right) => Some(left.unwrap_or(false) || right.unwrap_or(false)),
        }
    }

    /// The libyggterm declares the daemon has retained for this session (the
    /// app's latest `web-surface` / `sidebar` payloads). Empty for a plain
    /// shell, and empty again once the app emits its `close`.
    pub fn session_app_declares(&self, key: &str) -> Option<Vec<AppDeclareRecord>> {
        self.sessions.get(key).map(|session| session.app_declares())
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

    /// Write text THIS DAEMON authored — a readiness probe, its line-clear, or a
    /// repair's own prompt. ⛔ Use this and never `write` for daemon-authored
    /// text: `write` reconstructs the human's unsent-draft flag from whatever
    /// passes through it, so a probe sent that way is recorded as a person
    /// typing and the next draft check refuses on the probe's own marker.
    pub fn write_daemon_originated(&self, key: &str, data: &str) -> Result<()> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        session.write_daemon_originated(data)
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
        submit_prompt_echo_verified_with(
            // ⛔ NOT `self.write`: every write below is this daemon's own — the
            //    probe marker, its Ctrl+U, and the prompt — and `write` would
            //    record them as a human's unsent draft, which the very next
            //    draft check reads back as a person typing.
            |text| self.write_daemon_originated(key, text),
            || self.session_screen_snapshot(key),
            // ⛔⛔ THE KEYSTROKE ARM ON PURPOSE, AND IT MUST STAY THAT WAY.
            //    Every other reader of "does this composer hold a draft" was
            //    moved to `session_composer_holds_draft`, the union with the
            //    rendered composer row, because the counter is zeroed by a
            //    handover. NOT THIS ONE: the closure is re-consulted INSIDE the
            //    probe loop, after this very function has typed its marker into
            //    the composer — so the grid arm would read our own probe as a
            //    person mid-sentence and abort the submit it was asked to make.
            //    The counter is the correct instrument here precisely because
            //    `write_daemon_originated` is invisible to it.
            || self.session_has_pending_input_draft(key),
            data,
            timeout,
        )
    }
}

/// The echo-verified submit, driven through two closures instead of a registry.
///
/// ⚠ **Extracted so there is exactly ONE statement of the contract**, not two.
/// The daemon's §5 `continue` repair cannot call the method above: it would have
/// to hold the daemon's runtime lock for the whole probe-and-wait, freezing every
/// other session on the host for up to the timeout. Re-implementing the loop
/// there instead would have left two copies of a rule that took a live root-cause
/// to get right ([[finding-the-enter-key-is-a-separate-write-of-cr]]), free to
/// drift the first time either was touched. The closures lock per call and
/// release before every sleep.
pub fn submit_prompt_echo_verified_with(
    write: impl Fn(&str) -> Result<()>,
    snapshot: impl Fn() -> Option<String>,
    human_draft: impl Fn() -> Option<bool>,
    data: &str,
    timeout: Duration,
) -> Result<PromptSubmitOutcome> {
    {
        // Distinctive enough not to collide with real surface text; cleared via Ctrl+U.
        const PROBE: &str = "yggterm_ready_probe";
        const CLEAR_LINE: &str = "\u{15}"; // Ctrl+U — clears the composer line
        const PROBE_SETTLE: Duration = Duration::from_millis(180);
        const RETRY_INTERVAL: Duration = Duration::from_millis(120);
        if snapshot().is_none() {
            return Ok(PromptSubmitOutcome::NoSession);
        }
        let start = Instant::now();
        // ⛔ CHECKED BEFORE EVERY WRITE, never once at the top. A human can start
        // typing at any point during a 30 s wait, and the whole failure being
        // fixed here is a probe that kept writing while somebody was mid-word.
        // `Some(false)` = confirmed no draft; `None` = unknown, which is NOT
        // permission — an unreadable composer is the case where we can least
        // afford to be wrong, so it refuses too.
        let human_is_typing = || human_draft() != Some(false);
        if human_is_typing() {
            return Ok(PromptSubmitOutcome::HumanTyping { waited_ms: 0 });
        }
        let mut retry_backoff = RETRY_INTERVAL;
        loop {
            if human_is_typing() {
                return Ok(PromptSubmitOutcome::HumanTyping {
                    waited_ms: start.elapsed().as_millis() as u64,
                });
            }
            write(PROBE)?;
            thread::sleep(PROBE_SETTLE);
            let echoed = snapshot().is_some_and(|screen| screen.contains(PROBE));
            if echoed {
                // The program is consuming input. Clear the probe, then submit AS A
                // HUMAN DOES: type the text, then a DISTINCT Enter keypress. codex
                // treats a \r concatenated with text in one write as a pasted newline
                // (composer content), NOT a submit — so the Enter must be its own
                // write after the text settles (verified live 2026-06-04).
                write(CLEAR_LINE)?;
                thread::sleep(Duration::from_millis(60));
                let text = data.trim_end_matches(['\r', '\n']);
                write(text)?;
                thread::sleep(Duration::from_millis(80));
                write("\r")?;
                return Ok(PromptSubmitOutcome::Submitted {
                    waited_ms: start.elapsed().as_millis() as u64,
                });
            }
            // Not consuming yet: clear any buffered probe so it can't pile up, then
            // wait and retry (or give up at the deadline, leaving the surface clean).
            // ⛔ Re-checked: the echo wait is 180 ms during which a human may have
            // started, and CLEAR_LINE would erase what they just typed.
            if human_is_typing() {
                return Ok(PromptSubmitOutcome::HumanTyping {
                    waited_ms: start.elapsed().as_millis() as u64,
                });
            }
            let _ = write(CLEAR_LINE);
            if start.elapsed() >= timeout {
                return Ok(PromptSubmitOutcome::NotReady {
                    waited_ms: start.elapsed().as_millis() as u64,
                });
            }
            // ⛔ BACK OFF. A flat 120 ms retry against a row that is simply busy
            // means ~100 marker writes and ~100 line clears across a 30 s
            // timeout — the "viewport blinking" symptom is literally this loop
            // painting and erasing three times a second. A row that has not
            // answered in seconds will not answer in the next 120 ms, so the
            // interval doubles to a 2 s ceiling: same deadline, ~12 writes
            // instead of ~100, and the surface is left alone in between.
            thread::sleep(retry_backoff.min(Duration::from_secs(2)));
            retry_backoff = (retry_backoff * 2).min(Duration::from_secs(2));
        }
    }
}

impl TerminalManager {
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

    /// Is `key` a runtime this manager HOLDS — running or exited-but-retained?
    ///
    /// Deliberately weaker than [`Self::has_session`], which answers "is it
    /// running". `read`/`write`/`resize`/`remove_session` address the map, not
    /// the process: an exited runtime still answers a read with its retained
    /// screen, and a caller choosing which spelling of a key to address must
    /// know the map holds it — otherwise it picks a name nothing answers to and
    /// the read fails with `terminal session not found`.
    pub fn holds_session(&self, key: &str) -> bool {
        self.sessions.contains_key(key)
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

    /// Milliseconds this row has been written to without answering, or `None`
    /// when it is answering normally (or is not held here).
    pub fn input_unanswered_ms(&self, key: &str) -> Option<u64> {
        self.sessions.get(key)?.input_unanswered_ms()
    }

    /// A row that has been written to and has said nothing back for longer than
    /// `threshold`. ⚠ A trigger for the definitive check, not a verdict — see
    /// `PtySessionRuntime::wedge_suspected`.
    pub fn wedge_suspected(&self, key: &str, threshold: Duration) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.wedge_suspected(threshold))
    }

    pub fn restart_session(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        stop_command: Option<&str>,
    ) -> Result<TerminalRestartOutcome> {
        self.restart_session_with_size(key, launch_command, cwd, stop_command, None)
    }

    pub fn restart_session_with_size(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        stop_command: Option<&str>,
        initial_size: Option<(u16, u16)>,
    ) -> Result<TerminalRestartOutcome> {
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
        // ⛔ NO PER-CLI CLAMP HERE EITHER, AND THIS IS THE SITE THAT MADE IT
        // PERMANENT. The attach path resizes the PTY to the client's grid right
        // after `ensure_session_with_size` (the D1 `reattach_grid_resync`), so a
        // clamp applied there self-healed on the next attach. Nothing resyncs
        // behind a RESTART — and the client only emits a Resize when its own grid
        // CHANGES, which a daemon-side restart does not do. So a row restarted at
        // a clamped grid kept painting 120x40 inside a full-size viewport for the
        // rest of its life, which is the "TUI does not cover the viewport" fault.
        // See the note in `ensure_session_with_size` for the measurements.
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
        // ⛔ WHETHER ANYTHING WAS SHUT DOWN IS THE ANSWER, NOT A DETAIL.
        // `remove` returns None whenever the key does not resolve — an orphaned
        // key, or a session whose runtime is owned by a DIFFERENT daemon (every
        // `remote-*` row is served by the daemon on its own host). The restart
        // then shuts nothing down and spawns a replacement anyway, so the
        // process that was serving this key is still alive and now orphaned
        // beside its successor. That is not a restart, and reporting it as one
        // is how a wedged row survives the remedy that claims to clear it:
        // `input-check` names the wedge correctly, recommends this verb, and
        // the verb answered "restarted" while the wedged CLI kept its PTY.
        let replaced_existing = if let Some(runtime) = self.sessions.remove(key) {
            runtime.shutdown(stop_command)?;
            true
        } else {
            trace_terminal_event(
                "restart_replaced_nothing",
                serde_json::json!({
                    "path": key,
                    "reason": "no runtime under this key — nothing was shut down, and any \
                               process still serving it is now orphaned beside the replacement",
                }),
            );
            false
        };
        let runtime = PtySessionRuntime::spawn(key, launch_command, cwd, effective_initial_size)?;
        self.sessions.insert(key.to_string(), runtime);
        Ok(TerminalRestartOutcome { replaced_existing })
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
            // ⛔ NEVER TEAR DOWN A RUNTIME WE HAVE ALREADY HANDED OVER. A parked
            // reader means another daemon is serving this pty and we are only
            // still holding the descriptor (see [`ReaderPark`]). Stopping it
            // would kill a shell that a live daemon is currently painting for
            // the user — and our own exit does NOT do that, because exiting
            // merely closes our copies and the child re-parents to init.
            if self
                .sessions
                .get(&key)
                .is_some_and(|session| session.reader_park.is_parked())
            {
                trace_terminal_event(
                    "shutdown_skipped_handed_off_runtime",
                    serde_json::json!({ "path": key }),
                );
                continue;
            }
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

/// What `submit_if_line_equals` did, and why.
///
/// ⛔ No variant carries the line's TEXT. The whole point of the guard is that
/// the line may be the human's own half-typed sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitIffLineVerdict {
    /// The line matched and the Enter was enqueued.
    Submitted,
    /// The line did not match; nothing was written.
    LineMismatch { line_len: usize, expected_len: usize },
    /// The line matched but the Enter could not be enqueued.
    WriteFailed { error: String },
    /// This daemon does not hold the runtime, so it cannot answer. ⚠ NOT a
    /// refusal — the caller must not read it as "the line differed".
    NotOwned,
}

struct PtySessionRuntime {
    key: String,
    // Unique per PTY spawn (across daemon restarts too — time-based). The
    // client uses it as the cold-re-resume signal for the vacuum guard: a
    // snapshot whose spawn id differs from the one the client buffer was
    // seeded from came from a REPLACED runtime (exited+re-resumed or a
    // daemon-restart re-resume), so a sparse frame must not wipe the richer
    // client transcript. A same-spawn snapshot is a normal reveal and must
    // never be guarded (the 2.8.64 blanket-ratio regression).
    spawn_id: u64,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer_tx: SyncSender<TerminalWriteRequest>,
    /// Owned (we spawned it) or Adopted (we received its master fd and it is
    /// init's child now). `Child::wait`/`try_wait` cannot answer for a process
    /// that is not ours, which is why this is an enum rather than a trait
    /// object — see `crate::pty_adoption`.
    child: Arc<Mutex<PtyChildHandle>>,
    chunks: Arc<Mutex<VecDeque<TerminalChunk>>>,
    retained_bytes: Arc<AtomicUsize>,
    seq: Arc<AtomicU64>,
    started_at_ms: u64,
    last_activity_ms: Arc<AtomicU64>,
    /// When the CHILD last produced output — stamped only on the reader side.
    ///
    /// ⛔ `last_activity_ms` cannot answer this: the writer stamps it too, so
    /// input alone refreshes it. A row that has stopped reading its PTY while a
    /// human types into it therefore looks MAXIMALLY ACTIVE on that field — which
    /// is how a wedged row was listed `recently_active` by the hot-restart gate
    /// and blocked a deploy while being completely unusable.
    /// ⇒ input-versus-output is the only pair that can tell a live session from
    /// a wedged one, and it costs one atomic store on a path that already stores.
    last_output_ms: Arc<AtomicU64>,
    // Sticky "the current input line holds typed-but-unsent text" flag,
    // reconstructed from forwarded input bytes in `write()` via
    // `yggterm_core::input_line_has_unsent_draft_after`. Protects a drafted
    // prompt (which lives only in the PTY line buffer, never the agent JSONL)
    // from a release+re-resume session migration. See
    // [[finding-daemon-authoritative-working-state-2945]].
    pending_input_draft: Arc<AtomicBool>,
    /// The bytes standing on the current input line, reconstructed from the
    /// same walk that maintains `pending_input_draft`.
    ///
    /// ⛔ THE LOCK IS THE ATOMICITY. `submit_if_line_equals` compares and
    /// enqueues the Enter while holding it, and `write` takes it to append —
    /// so a keystroke that has reached this daemon can never land BETWEEN the
    /// comparison and the submit. That gap is what put a supervision tool's
    /// text into the middle of a half-typed sentence and sent it.
    pending_input_line: Arc<Mutex<Vec<u8>>>,
    runtime_output_seen: Arc<AtomicBool>,
    eof_without_output: Arc<AtomicBool>,
    attach_ready_seen: Arc<AtomicBool>,
    resize_count: Arc<AtomicU64>,
    last_resize_seq: Arc<AtomicU64>,
    current_cols: Arc<AtomicU16>,
    current_rows: Arc<AtomicU16>,
    screen_state: Arc<Mutex<TerminalScreenState>>,
    /// Memo for [`PtySessionRuntime::screen_snapshot`], keyed by
    /// [`ScreenSnapshotKey`] — every input the snapshot is a function of.
    screen_snapshot_memo: Arc<Mutex<Option<(ScreenSnapshotKey, Arc<str>)>>>,
    /// The latest libyggterm OSC 7717 declare per verb, lifted off this
    /// session's stream by the daemon itself. The GUI's xterm parser reads the
    /// same bytes, but only while a client host is mounted — this copy is what
    /// lets a never-revealed session's surfaces be materialized, and a reaped
    /// one be rebuilt, with no reveal. See [`crate::app_declare`].
    app_declares: Arc<Mutex<AppDeclareLog>>,
    /// Stand this session's reader down without giving up its descriptors —
    /// see [`ReaderPark`]. Held here so a handoff can park every reader it is
    /// about to hand over, and un-park them all again if the successor it just
    /// trusted does not survive.
    reader_park: Arc<ReaderPark>,
    launch_command: String,
    cwd: Option<String>,
}

/// What a restart actually DID, as opposed to what it was asked to do.
///
/// ⛔ `replaced_existing: false` means the restart shut NOTHING down: no runtime
/// resolved under that key, so whatever was serving it is still alive and is now
/// orphaned beside the freshly spawned replacement. A caller that reports such a
/// call as "restarted" is telling the operator their wedged row was cleared when
/// it was not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalRestartOutcome {
    pub replaced_existing: bool,
}

struct TerminalWriteRequest {
    data: Vec<u8>,
    completion_tx: Option<mpsc::Sender<std::result::Result<(), String>>>,
    /// Set only by the reader thread as it exits, to retire the writer that
    /// pairs with it. The writer's `rx.recv()` returns `Err` when every
    /// `SyncSender` clone has dropped — but the clone the terminal entry holds
    /// outlives a dead PTY, so without this the writer parks on `recv()`
    /// forever. It cannot be a timeout poll: a writer that wakes to check a
    /// flag is exactly the idle cost this thread must not add.
    shutdown: bool,
}

/// Unique id per PTY spawn: time-based so it stays unique across daemon
/// process restarts (a counter alone would restart at 0 and could collide
/// with the id a client recorded from the previous daemon), with a process
/// counter folded in so two spawns within the same millisecond still differ.
fn next_runtime_spawn_id(started_at_ms: u64) -> u64 {
    static RUNTIME_SPAWN_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let counter =
        RUNTIME_SPAWN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 1000;
    started_at_ms.saturating_mul(1000).saturating_add(counter)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
enum TerminalWriteAckMode {
    Enqueued,
    Flushed,
}

/// Display width of one character in terminal cells.
///
/// East-Asian Wide/Fullwidth count 2; everything else counts 1 — including the
/// AMBIGUOUS class (box drawing, arrows, the glyphs the agent CLIs draw their
/// frames with), which is what xterm.js does by default. Combining marks are
/// rare enough in CLI frames that treating them as 1 only ever over-estimates,
/// and this measurement is used to DETECT an overflow, so over-estimating is
/// the safe direction.
fn formatted_screen_cell_width(ch: char) -> u16 {
    let code = ch as u32;
    let wide = (0x1100..=0x115F).contains(&code)
        || (0x2E80..=0xA4CF).contains(&code) && code != 0x303F
        || (0xAC00..=0xD7A3).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
        || (0xFE30..=0xFE6F).contains(&code)
        || (0xFF00..=0xFF60).contains(&code)
        || (0xFFE0..=0xFFE6).contains(&code)
        || (0x1F300..=0x1F64F).contains(&code)
        || (0x1F900..=0x1F9FF).contains(&code)
        || (0x20000..=0x3FFFD).contains(&code);
    if wide { 2 } else { 1 }
}

/// One step of walking a daemon "formatted screen" payload
/// (`state_formatted`), reported to a caller that wants to measure or rewrite
/// it. The payload is absolutely positioned (`CSI r;cH`) with `CSI nC` used for
/// runs of blanks, so a naive `text.len()` says nothing about where a row ends.
enum FormattedScreenStep<'a> {
    /// A control sequence: carries state (color, position) but paints nothing.
    Control(&'a str),
    /// A printable character landing at `col` (1-based) and `width` cells wide.
    Print { text: &'a str, col: u16, width: u16 },
}

/// Walk a formatted screen, tracking the cursor, and hand each step to `step`.
///
/// Shared by [`formatted_screen_max_column`] and
/// [`clip_formatted_screen_to_width`] so the measurement and the rewrite can
/// never disagree about where a character lands.
/// Walk a formatted screen, tracking the column each printed cell lands in.
///
/// ⛔ `grid_cols` IS THE GRID THE PAYLOAD WAS FORMATTED AGAINST, and passing the
/// wrong one silently destroys content.
///
/// A vt100 formatter emits a WRAPPED row as one continuous run with no line
/// break between its parts — deliberately, because that is what makes the
/// receiving terminal re-wrap it at its own width instead of hard-breaking it at
/// ours. So a 400-character line on a 170-column grid arrives as 400 printable
/// bytes in a row, and a walker that counts columns monotonically reads its last
/// cell as **column 400 on a 170-column screen**.
///
/// That is not a hypothesis. Measured on the GUI host 2026-08-04: 504
/// `screen_snapshot_clipped_to_pty_width` events across two shells, every one of
/// them reporting `pty_cols: 170` against a `screen_max_column` of 334 and 260 —
/// constant, because the content was not changing; it was the same wrapped lines
/// being mis-measured on every snapshot. The clip that fires on that reading
/// then DELETES every wrapped continuation, so a re-attaching client gets a
/// screen with the second and third visual row of every long line missing.
///
/// Wrapping here at the grid's own width is what makes the measurement mean
/// "which column of the grid", which is the only thing the callers ever wanted.
/// It also keeps the check that this code was written for honest: a STALE model
/// wider than the PTY wraps at its own wider width, so its ghost cells still
/// land past the PTY's edge and are still found.
///
/// `grid_cols == 0` means "unknown" and disables wrapping — the pre-2026-08-04
/// behaviour, kept only so a caller with genuinely no width is explicit about
/// it rather than passing a guess.
fn walk_formatted_screen(
    screen_text: &str,
    grid_cols: u16,
    mut step: impl FnMut(FormattedScreenStep<'_>),
) {
    let bytes = screen_text.as_bytes();
    let mut col: u16 = 1;
    let mut i = 0usize;
    while i < screen_text.len() {
        if bytes[i] == 0x1b {
            // CSI: ESC [ params final. OSC: ESC ] ... BEL|ST. Anything else is a
            // two-byte escape. None of them paint a cell.
            let end = if screen_text[i..].starts_with("\u{1b}[") {
                let mut j = i + 2;
                while j < screen_text.len() && !bytes[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let final_byte = bytes.get(j).copied();
                let params = &screen_text[i + 2..j.min(screen_text.len())];
                match final_byte {
                    // Cursor position: column is the SECOND parameter.
                    Some(b'H') | Some(b'f') => {
                        let mut parts = params.split(';');
                        let _row = parts.next();
                        col = parts
                            .next()
                            .and_then(|value| value.parse::<u16>().ok())
                            .unwrap_or(1)
                            .max(1);
                    }
                    // Cursor forward — the payload's way of spelling blanks.
                    Some(b'C') => {
                        let n = params.parse::<u16>().unwrap_or(1).max(1);
                        col = col.saturating_add(n);
                    }
                    Some(b'G') => {
                        col = params.parse::<u16>().unwrap_or(1).max(1);
                    }
                    Some(b'D') => {
                        let n = params.parse::<u16>().unwrap_or(1).max(1);
                        col = col.saturating_sub(n).max(1);
                    }
                    _ => {}
                }
                (j + 1).min(screen_text.len())
            } else if screen_text[i..].starts_with("\u{1b}]") {
                let mut j = i + 2;
                while j < screen_text.len() && bytes[j] != 0x07 {
                    j += 1;
                }
                (j + 1).min(screen_text.len())
            } else {
                (i + 2).min(screen_text.len())
            };
            step(FormattedScreenStep::Control(&screen_text[i..end]));
            i = end;
            continue;
        }
        let ch = screen_text[i..].chars().next().unwrap_or('\0');
        let len = ch.len_utf8();
        match ch {
            '\n' => {
                col = 1;
                step(FormattedScreenStep::Control(&screen_text[i..i + len]));
            }
            '\r' => {
                col = 1;
                step(FormattedScreenStep::Control(&screen_text[i..i + len]));
            }
            _ => {
                let width = formatted_screen_cell_width(ch);
                // THE WRAP, before the cell is placed: a terminal that cannot
                // fit this cell in the current row starts the next one, and the
                // cell lands in column 1 there.
                if grid_cols > 0 && col.saturating_add(width).saturating_sub(1) > grid_cols {
                    col = 1;
                }
                step(FormattedScreenStep::Print {
                    text: &screen_text[i..i + len],
                    col,
                    width,
                });
                col = col.saturating_add(width);
            }
        }
        i += len;
    }
}

/// Everything [`PtySessionRuntime::screen_snapshot`] is a function of.
///
/// `output_seq` is bumped at exactly the points where
/// `screen_state.process(bytes)` runs (reader thread and `seed_snapshot`,
/// under the same chunk lock), so an unchanged seq means the vt100 model was
/// not fed anything — the one generation counter here that cannot go stale.
///
/// It is NOT the whole key, and that is the dangerous part. `resize` mutates
/// the model without touching seq, through two branches: the ordinary path,
/// and the `resize_screen_model_repaired` branch that fixes a model painting
/// wider than its PTY. And the snapshot is CLIPPED to the PTY width. A memo
/// keyed on seq alone would serve a screen clipped to the OLD width across a
/// resize — precisely the frame-corruption class the clip was added to fix
/// (docs/xterm-bugs.md#screen-model-wider-than-viewer), and it would read as a
/// regression of that fix rather than of this cache.
///
/// So the key also carries the clip width, the model's own size, and the
/// resize counter. The counter closes the one hole the sizes leave: a resize
/// AWAY and back to the same grid with no output in between returns every size
/// to its old value while the model has been re-laid-out (columns dropped at
/// the narrow step do not come back), so a size-only key would serve cells the
/// model no longer holds. The counter does not cover the repair branch, which
/// returns before incrementing it — that one is caught by `model_size`. Between
/// them nothing has to remember to invalidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScreenSnapshotKey {
    output_seq: u64,
    resize_seq: u64,
    pty_cols: u16,
    model_size: Option<(u16, u16)>,
}

/// The rightmost column a formatted screen paints into, in cells.
///
/// A screen wider than the terminal it is written into is CORRUPTION, not a
/// cosmetic overflow: each over-long row wraps, which shifts every row below it,
/// and the payload's later absolute `CSI r;cH` jumps then land on that spill —
/// where `CSI nC` (blank runs) leaves the spilled characters showing through.
/// The result is text merged out of two different frames. Measured live on guihost
/// 2026-07-25: a screen reaching column 204 painted into a 168-column viewer.
///
/// ⚠ `grid_cols` is the width of the GRID THIS TEXT WAS FORMATTED AGAINST — the
/// daemon's vt100 model, not the viewer. Read [`walk_formatted_screen`] before
/// passing anything else: a wrapped line has no break in it, so a walker given
/// the wrong width reports a 400-column screen for a 170-column grid and every
/// caller downstream acts on a number that describes nothing.
pub fn formatted_screen_max_column(screen_text: &str, grid_cols: u16) -> u16 {
    let mut max_col = 0u16;
    walk_formatted_screen(screen_text, grid_cols, |step| {
        if let FormattedScreenStep::Print { col, width, .. } = step {
            max_col = max_col.max(col.saturating_add(width).saturating_sub(1));
        }
    });
    max_col
}

/// Drop everything a formatted screen paints beyond `cols`.
///
/// Control sequences are kept verbatim (color/attribute state must survive), so
/// only the printable cells past the edge are dropped. Nothing legitimate lives
/// there: the CLI cannot paint wider than the PTY it was handed, so a cell
/// beyond the viewer's width is a ghost from when the grid was wider.
///
/// ⛔ THAT LAST SENTENCE IS ONLY TRUE WHEN `grid_cols` IS RIGHT. Given the
/// viewer's width where the model's belonged, this deletes the continuation of
/// every wrapped line — the cells are not ghosts, they are the rest of the
/// sentence. Both arguments are required for that reason: there is no default
/// that is safe to guess.
pub fn clip_formatted_screen_to_width(screen_text: &str, grid_cols: u16, cols: u16) -> String {
    if cols == 0 {
        return screen_text.to_string();
    }
    let mut out = String::with_capacity(screen_text.len());
    walk_formatted_screen(screen_text, grid_cols, |step| match step {
        FormattedScreenStep::Control(text) => out.push_str(text),
        FormattedScreenStep::Print { text, col, width } => {
            if col.saturating_add(width).saturating_sub(1) <= cols {
                out.push_str(text);
            }
        }
    });
    out
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

    /// The model's own grid, `(rows, cols)`. The PTY's size is tracked
    /// separately (`current_cols`/`current_rows`); they are supposed to agree,
    /// and `resize` above is the only thing that keeps them agreeing — so the
    /// resize fast path has to be able to ask.
    fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
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
    /// The VISIBLE viewport as plain-text rows — one entry per screen row, in
    /// top-to-bottom order, trailing blanks on each row trimmed.
    ///
    /// ⛔⛔ THIS IS WHAT A CLASSIFIER MUST READ, and `screen_snapshot` is not.
    /// That one returns the vt100 FORMATTED state: the escape sequence stream
    /// that repaints this screen. A TUI draws with absolute cursor moves rather
    /// than newlines, so on that stream a modal's nine visible rows arrive as
    /// TWO `\n`-delimited lines — measured 2026-08-21 on a first-run gate: 11
    /// cursor-position moves, zero newlines between the rows they address, one
    /// run of 870 characters. Every line-shaped question then answers about
    /// something other than the screen: "the last N lines" is not a window over
    /// the display, and "these two phrases on the SAME line" is either
    /// trivially true or impossible depending only on how the CLI chose to
    /// paint. Words of vertically adjacent rows also run together, so a needle
    /// can straddle a seam that does not exist on screen.
    ///
    /// ⭐ The vt100 model has already done this work — it is the emulator whose
    /// grid the GUI paints. Asking it for rows is both cheaper and more
    /// faithful than any regex over the stream, and it is the same call
    /// `vt_scrollback_plain_rows` makes for history.
    ///
    /// Blank rows are KEPT. A caller filtering them is choosing a window; a
    /// reader printing the screen wants the shape a person would see.
    fn vt_screen_plain_rows(&mut self) -> Vec<String> {
        let screen = self.parser.screen_mut();
        // Read the LIVE viewport, not wherever a previous reader left the
        // scrollback offset, and put it back afterwards: this is a read-only
        // question and must not move what anyone else is looking at.
        let saved_offset = screen.scrollback();
        screen.set_scrollback(0);
        let (_, cols) = screen.size();
        let rows: Vec<String> = screen
            .rows(0, cols)
            .map(|row| row.trim_end().to_string())
            .collect();
        screen.set_scrollback(saved_offset);
        rows
    }

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

    /// Viewport-only reconcile payload: clear the visible screen and repaint
    /// it from the daemon's authoritative vt100 state (which restores modes
    /// AND the cursor position). Appended after a raw retained-chunk initial
    /// seed: a budget-truncated chunk tail starts mid-stream, so a TUI that
    /// paints with relative cursor motion (Claude Code frames are `\r\x1b[nB`
    /// moves + `\x1b[K` erases) replays against the wrong origin and leaves
    /// shifted/merged rows and blanked cells that persist — the TUI then
    /// diffs against a screen it never actually drew. Ending the seed with
    /// this payload pins the client viewport and cursor to daemon truth so
    /// every subsequent live diff anchors correctly, while the replayed tail
    /// still populates scrollback. No history here — the tail already carries
    /// it, and normal-buffer history must never be injected under an
    /// alternate-screen TUI (see the reverted chunk-ring-gap resync).
    /// The payload deliberately LEADS with `\x1b[?25l`: the GUI's batch
    /// sanitizers only forward a chunk verbatim when it carries a control
    /// marker (alt-screen switch, hide-cursor, high-volume frame), and vt100's
    /// `state_formatted` starts with `\x1b[?25h` when the cursor is visible —
    /// without the lead-in the reconcile itself could be line-rejoined.
    /// `state_formatted` re-asserts the true visibility immediately after, so
    /// the final cursor state is the daemon's.
    fn viewport_reconcile_replay(&self) -> Option<String> {
        let formatted = self.formatted.trim_matches('\0');
        if !terminal_chunk_has_visible_text(formatted) {
            return None;
        }
        Some(format!("\x1b[?25l\x1b[2J\x1b[H{formatted}"))
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
                if request.shutdown {
                    // The PTY is gone. Retiring here is what frees the thread;
                    // a shutdown is not activity, so it must not touch
                    // `last_activity_ms` — an idle-window gate reads that field.
                    break;
                }
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
        shutdown: false,
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

        let command = shell_command(launch_command, cwd, Some(key));
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("spawning terminal session {key}"))?;

        Self::assemble(
            key,
            launch_command,
            cwd,
            initial_cols,
            initial_rows,
            pair.master,
            PtyChildHandle::owned(child),
            None,
        )
    }

    /// Build the runtime around a master this daemon ALREADY holds.
    ///
    /// Extracted from [`PtySessionRuntime::spawn`] so that adopting a PTY
    /// received from another daemon is a wiring change rather than a second
    /// copy of the reader/writer/ring machinery. Everything below the point
    /// where a master and a child handle exist is identical whether we opened
    /// the pty ourselves or were handed it — and the one rule this crate does
    /// not bend is that a concept may not have two encodings.
    ///
    /// `seed` is the predecessor's screen, replayed into the parser and the
    /// ring before the reader thread starts. The fd alone hands over a live
    /// terminal with an EMPTY transcript; without this the user gets a working
    /// shell that has forgotten everything it just said.
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        initial_cols: u16,
        initial_rows: u16,
        master: Box<dyn MasterPty + Send>,
        child: PtyChildHandle,
        seed: Option<&str>,
    ) -> Result<Self> {
        let mut reader = master.try_clone_reader().context("cloning pty reader")?;
        let writer = master.take_writer().context("taking pty writer")?;
        let park = ReaderPark::new();
        let gate = ReaderGate {
            park: Arc::clone(&park),
            #[cfg(target_os = "linux")]
            poll_fd: dup_master_for_poll(master.as_ref()),
        };
        let chunks = Arc::new(Mutex::new(VecDeque::new()));
        let retained_bytes = Arc::new(AtomicUsize::new(0));
        let seq = Arc::new(AtomicU64::new(0));
        let started_at_ms = now_millis();
        let last_activity_ms = Arc::new(AtomicU64::new(started_at_ms));
        let last_output_ms = Arc::new(AtomicU64::new(started_at_ms));
        let pending_input_draft = Arc::new(AtomicBool::new(false));
        let pending_input_line: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
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
        // Replay the predecessor's screen BEFORE the reader thread starts, so
        // the first live byte lands after the carried history rather than
        // racing it. Seeding after the thread is spawned would interleave.
        if let Some(seed) = seed.filter(|seed| !seed.is_empty()) {
            if let Ok(mut screen) = screen_state.lock() {
                screen.process(seed.as_bytes());
            }
            let seq_value = seq.fetch_add(1, Ordering::SeqCst) + 1;
            let mut seeded = chunks.lock().expect("pty chunk lock poisoned");
            seeded.push_back(TerminalChunk {
                seq: seq_value,
                data: seed.to_string(),
            });
            retained_bytes.store(seed.len(), Ordering::SeqCst);
        }
        let reader_chunks = Arc::clone(&chunks);
        let reader_retained_bytes = Arc::clone(&retained_bytes);
        let reader_seq = Arc::clone(&seq);
        let reader_activity = Arc::clone(&last_activity_ms);
        let reader_output = Arc::clone(&last_output_ms);
        let reader_runtime_output_seen = Arc::clone(&runtime_output_seen);
        let reader_eof_without_output = Arc::clone(&eof_without_output);
        let reader_attach_ready_seen = Arc::clone(&attach_ready_seen);
        let reader_screen_state = Arc::clone(&screen_state);
        let app_declares = Arc::new(Mutex::new(AppDeclareLog::new()));
        let reader_app_declares = Arc::clone(&app_declares);
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
                let mut agent_error_scanner = AgentSessionErrorScanner::default();
                let mut app_declare_scanner = AppDeclareScanner::new();
                let mut saw_any_output = false;
                loop {
                    // Stand down here rather than inside the read: a parked
                    // reader must own no bytes at all, so that everything the
                    // pty produces from this instant belongs to whoever holds
                    // the descriptor next.
                    let read_result = match gate.wait() {
                        ReaderGateVerdict::Read => reader.read(&mut buffer),
                        ReaderGateVerdict::Failed(error) => Err(std::io::Error::other(error)),
                    };
                    if let Ok(bytes) = read_result.as_ref()
                        && *bytes > 0
                        && gate.park.is_parked()
                    {
                        // The race the park is meant to close, measured instead
                        // of assumed: `poll` said readable and the park landed
                        // before the read completed. Bounded to one chunk, and
                        // that chunk is a hole in the successor's transcript.
                        gate.park
                            .stolen_after_park
                            .fetch_add(*bytes as u64, Ordering::SeqCst);
                    }
                    match read_result {
                        Ok(0) => {
                            let raw_data = flush_terminal_utf8_pending(&mut pending_utf8);
                            let cursor_pos = reader_screen_state
                                .lock()
                                .ok()
                                .map(|state| state.parser.screen().cursor_position());
                            let protocol_result = protocol_filter.process_with_cursor(
                                &raw_data,
                                terminal_protocol_profile,
                                cursor_pos,
                            );
                            enqueue_terminal_protocol_responses(
                                &reader_writer_tx,
                                &key_label,
                                terminal_protocol_profile,
                                &protocol_result,
                            );
                            protocol_filter.discard_pending();
                            let data = protocol_result.data;
                            if !data.is_empty() {
                                // Same commit-lock discipline as the streaming
                                // branch: screen + seq + ring move together.
                                let mut chunks =
                                    reader_chunks.lock().expect("pty chunk lock poisoned");
                                if let Ok(mut screen_state) = reader_screen_state.lock() {
                                    screen_state.process(data.as_bytes());
                                }
                                for hit in agent_error_scanner
                                    .scan(&strip_terminal_control_sequences(&data), now_millis())
                                {
                                    record_agent_session_error(
                                        &key_label,
                                        &launch_command_label,
                                        &hit,
                                    );
                                }
                                reader_runtime_output_seen.store(true, Ordering::SeqCst);
                                reader_activity.store(now_millis(), Ordering::SeqCst);
                                reader_output.store(now_millis(), Ordering::SeqCst);
                                let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
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
                                reader_output.store(now_millis(), Ordering::SeqCst);
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
                            let cursor_pos = reader_screen_state
                                .lock()
                                .ok()
                                .map(|state| state.parser.screen().cursor_position());
                            let protocol_result = protocol_filter.process_with_cursor(
                                &data,
                                terminal_protocol_profile,
                                cursor_pos,
                            );
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
                                reader_output.store(now_millis(), Ordering::SeqCst);
                                }
                                continue;
                            }
                            // Hold the chunks lock across the vt100 update AND
                            // the seq+ring commit. `read(0)` holds the same lock
                            // while it snapshots the screen against `seq`, so
                            // this keeps "screen state == chunks 1..=seq"
                            // invariant — without it an attach mid-chunk could
                            // seed a screen that already contains a chunk the
                            // cursor says is still pending, double-applying a
                            // relative-cursor frame (row-shift garble).
                            let mut chunks = reader_chunks.lock().expect("pty chunk lock poisoned");
                            if let Ok(mut screen_state) = reader_screen_state.lock() {
                                screen_state.process(data.as_bytes());
                            }
                            ingest_app_declares(
                                &mut app_declare_scanner,
                                &reader_app_declares,
                                &key_label,
                                &data,
                            );
                            for hit in agent_error_scanner
                                .scan(&strip_terminal_control_sequences(&data), now_millis())
                            {
                                record_agent_session_error(&key_label, &launch_command_label, &hit);
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
                            // ⛔ Our OWN control heartbeat is not session
                            // activity. Everything else about this chunk is
                            // unchanged — it still reaches the screen, the ring
                            // and the declare log; only the idle clock, which
                            // the hot-restart gate reads as "a human or an agent
                            // is doing something here", declines to move.
                            // See [`crate::app_declare::chunk_is_only_app_declares`].
                            if !crate::app_declare::chunk_is_only_app_declares(&data) {
                                reader_activity.store(now_millis(), Ordering::SeqCst);
                                reader_output.store(now_millis(), Ordering::SeqCst);
                            }
                            let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
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
                            reader_output.store(now_millis(), Ordering::SeqCst);
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
                // Past the loop, so this covers EOF *and* the read-error exit.
                // `try_send` rather than `send`: if the queue is full the writer
                // is mid-write against a dead PTY, will fail and break on its
                // own, and blocking here would strand the reader instead.
                let _ = reader_writer_tx.try_send(TerminalWriteRequest {
                    data: Vec::new(),
                    completion_tx: None,
                    shutdown: true,
                });
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
            spawn_id: next_runtime_spawn_id(started_at_ms),
            master: Arc::new(Mutex::new(master)),
            writer_tx,
            child: Arc::new(Mutex::new(child)),
            chunks,
            retained_bytes,
            seq,
            started_at_ms,
            last_activity_ms,
            last_output_ms,
            pending_input_draft,
            pending_input_line,
            runtime_output_seen,
            eof_without_output,
            attach_ready_seen,
            resize_count,
            last_resize_seq,
            current_cols,
            current_rows,
            screen_state,
            screen_snapshot_memo: Arc::new(Mutex::new(None)),
            app_declares,
            reader_park: park,
            launch_command: launch_command.to_string(),
            cwd: cwd.map(|value| value.to_string()),
        })
    }

    /// Take ownership of a PTY whose master fd arrived from another daemon.
    ///
    /// This is the receiving half of level (b): the predecessor sends its
    /// screen, then the master fd, then drops its runtime without killing the
    /// child, which re-parents to init. We drive the fd from here on and track
    /// the process by `(pid, start_time)` — `waitpid` is gone forever for this
    /// session, which is exactly why [`PtyChildHandle`] is an enum.
    ///
    /// Refuses rather than guesses when the process cannot be pinned: adopting
    /// a pid with no confirmable identity is how a later `kill` lands on a
    /// stranger after pid reuse.
    #[cfg(target_os = "linux")]
    fn adopt(
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
        fd: std::os::fd::OwnedFd,
        shell_pid: u32,
        shell_start_time: u64,
        seed: Option<&str>,
    ) -> Result<Self> {
        if cols == 0 || rows == 0 {
            bail!("adopted terminal size must be greater than zero");
        }
        let master = crate::pty_adoption::ReceivedMasterPty::new(fd, shell_pid, shell_start_time);
        let mut child = master.child_handle();
        // Identity, not optimism: an unconfirmable pid must refuse the adoption
        // rather than install a runtime that will later signal a stranger.
        if !child.is_running().unwrap_or(false) {
            bail!(
                "refusing to adopt {key}: pid {shell_pid} is not alive with start time \
                 {shell_start_time} — the process died in transit or the identity is stale"
            );
        }
        trace_terminal_event(
            "adopt",
            serde_json::json!({
                "path": key,
                "shell_pid": shell_pid,
                "shell_start_time": shell_start_time,
                "cols": cols,
                "rows": rows,
                "seed_bytes": seed.map(|seed| seed.len()).unwrap_or(0),
            }),
        );
        Self::assemble(
            key,
            launch_command,
            cwd,
            cols,
            rows,
            Box::new(master),
            child,
            seed,
        )
    }

    /// The declares this session's app currently stands behind.
    fn app_declares(&self) -> Vec<AppDeclareRecord> {
        self.app_declares
            .lock()
            .map(|log| log.records())
            .unwrap_or_default()
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
        // A probe error reads as "not running" HERE and only here, which is
        // exactly what the pre-split `match child.try_wait() { … Err(_) => false }`
        // did: this accessor answers a yes/no display question for callers that
        // have no channel for an error. The TEARDOWN paths below deliberately do
        // NOT use it — they take the `Result` and either propagate it or trace
        // it, because there a swallowed probe error becomes a false "it exited"
        // and a shutdown that reports success over a live process.
        child.is_running().unwrap_or(false)
    }

    fn process_id(&self) -> Option<u32> {
        let child = self.child.lock().expect("pty child lock poisoned");
        child.process_id()
    }

    /// Whether our child handle is an ADOPTED pid rather than one we forked.
    #[cfg(target_os = "linux")]
    fn child_is_adopted(&self) -> bool {
        let child = self.child.lock().expect("pty child lock poisoned");
        child.is_adopted()
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

    /// `true` when the user has typed text on the current input line but not yet
    /// submitted it (sticky; see `pending_input_draft`). The migration predicate
    /// treats this as PROTECTED — releasing such a session would lose the draft.
    fn has_pending_input_draft(&self) -> bool {
        self.pending_input_draft.load(Ordering::SeqCst)
    }

    /// Press Enter IFF the composer's current line is exactly `expected`.
    ///
    /// ⛔ WHY THIS CANNOT BE DONE BY THE CALLER. Typing text and submitting it
    /// are two discrete writes, and a human's keystrokes land in the gap: on
    /// 2026-08-20 a supervision tool's text was spliced into the middle of a
    /// half-typed sentence and submitted, because the caller's own guard read an
    /// empty line while those keystrokes were still in flight. A caller-side
    /// screen read-back narrows that window and cannot close it — the screen
    /// answers what the program has ECHOED, which lags the input stream.
    ///
    /// Here the comparison and the Enter happen under one lock, against a line
    /// derived from the bytes this daemon has forwarded, so nothing that has
    /// reached this daemon can land between them.
    ///
    /// ⚠ The residual gap is honest and named: a keystroke still travelling
    /// from the client has reached nobody, and no daemon-side check can see it.
    fn submit_if_line_equals(&self, expected: &str) -> SubmitIffLineVerdict {
        let mut line = self
            .pending_input_line
            .lock()
            .expect("pty input line lock poisoned");
        if line.as_slice() != expected.as_bytes() {
            // ⛔ The lengths, never the text. This refusal is about the human's
            // own half-typed sentence, and a diagnostic that quotes it puts it
            // in a log the way the bug put it on screen.
            return SubmitIffLineVerdict::LineMismatch {
                line_len: line.len(),
                expected_len: expected.len(),
            };
        }
        // Still holding the lock: the Enter is a SEPARATE write of `\r`, and
        // enqueueing it here is what makes the pair indivisible.
        match self.write_daemon_originated("\r") {
            Ok(()) => {
                // ⛔ AND CLEAR WHAT THE ENTER JUST SENT, which the daemon-authored
                // write path deliberately does not do. That path skips the input
                // walk so a readiness probe cannot fabricate a draft — correct for
                // a probe, wrong here: this `\r` really did submit the composer.
                // Left unclear, the line would still read the submitted text, so a
                // second identical call would press Enter AGAIN on a line the
                // composer no longer holds, and the draft flag would stay true
                // forever, refusing every later guarded write on a row with an
                // empty composer.
                // Cleared in place, under the SAME guard the comparison held —
                // dropping it first would let a keystroke for the NEXT line land
                // and then be erased by this clear.
                line.clear();
                self.pending_input_draft.store(false, Ordering::SeqCst);
                SubmitIffLineVerdict::Submitted
            }
            Err(error) => SubmitIffLineVerdict::WriteFailed {
                error: error.to_string(),
            },
        }
    }

    fn snapshot(&self) -> String {
        let chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>()
    }

    /// The daemon's authoritative visible screen, as a replayable payload.
    ///
    /// Clipped to the session's own PTY width, because a screen wider than the
    /// PTY is not content — the CLI cannot paint wider than the grid it was
    /// handed, so anything out there is a ghost left over from when the model
    /// was wider. Serving those ghosts is the frame-corruption bug: the client
    /// wraps each over-long row, every row below shifts, and the payload's later
    /// absolute jumps land on the spill with blank-runs that leave it showing
    /// through. Clipping HERE (the one place the screen is served) covers every
    /// client path at once, rather than each replay call site remembering to.
    fn screen_snapshot(&self) -> String {
        // Three hot callers ask for this on every snapshot response, every
        // working-flags poll and every chore tick, and between two asks the
        // answer is usually byte-identical: the format walk plus the clip
        // rewrite run over the whole screen each time for nothing.
        let key = self.screen_snapshot_key();
        if let Some((memo_key, memo)) = self
            .screen_snapshot_memo
            .lock()
            .expect("pty screen snapshot memo lock poisoned")
            .as_ref()
            && *memo_key == key
        {
            return memo.to_string();
        }
        let snapshot = self.render_screen_snapshot(key.pty_cols);
        *self
            .screen_snapshot_memo
            .lock()
            .expect("pty screen snapshot memo lock poisoned") =
            Some((key, Arc::from(snapshot.as_str())));
        snapshot
    }

    fn screen_snapshot_key(&self) -> ScreenSnapshotKey {
        ScreenSnapshotKey {
            output_seq: self.seq.load(Ordering::SeqCst),
            resize_seq: self.resize_count.load(Ordering::SeqCst),
            pty_cols: self.current_cols.load(Ordering::SeqCst),
            model_size: self.screen_state.lock().ok().map(|state| state.size()),
        }
    }

    fn render_screen_snapshot(&self, pty_cols: u16) -> String {
        // BOTH WIDTHS, read under one lock. The model's is what the text was
        // formatted against (so it is where a wrapped line breaks); the PTY's is
        // what the CLI was told it had (so it is where a legitimate cell must
        // stop). Only their DIFFERENCE is a ghost. Reading one and guessing the
        // other is exactly the mistake that made this clip destructive.
        let (formatted, model_cols) = {
            let screen_state = self
                .screen_state
                .lock()
                .expect("pty screen state lock poisoned");
            (
                screen_state.formatted.trim_matches('\0').to_string(),
                screen_state.size().1,
            )
        };
        let max_column = formatted_screen_max_column(&formatted, model_cols);
        if pty_cols == 0 || max_column <= pty_cols {
            return formatted;
        }
        trace_terminal_event(
            "screen_snapshot_clipped_to_pty_width",
            serde_json::json!({
                "path": self.key,
                "pty_cols": pty_cols,
                "screen_max_column": max_column,
                // THE DISCRIMINATOR THIS EVENT WAS MISSING. Without it, 504
                // firings on the live host could not distinguish "the model is
                // stale and wide" (the bug this clip is for) from "the walker
                // cannot see a line wrap" (what it actually was). An anomaly
                // event that cannot separate its own two causes costs a session.
                "screen_model_cols": model_cols,
            }),
        );
        clip_formatted_screen_to_width(&formatted, model_cols, pty_cols)
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

    /// The rendered viewport grid, one entry per visible row.
    fn screen_plain_rows(&self) -> Vec<String> {
        let mut screen_state = self
            .screen_state
            .lock()
            .expect("pty screen state lock poisoned");
        screen_state.vt_screen_plain_rows()
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

    /// The destructive full-screen replay a desynced client asks for.
    ///
    /// ⛔ IT IS CLIPPED HERE, AND ONLY HERE. The GUI used to clip this payload
    /// itself, against the VIEWER's width — the only number it has — which
    /// deletes the continuation of every wrapped line, because a wrapped line
    /// carries no break for a walker to find. The daemon is the one place that
    /// holds BOTH widths (the model the text was formatted against, and the PTY
    /// the CLI was handed), so it is the only place that can tell a ghost cell
    /// from the rest of a sentence. One owner, with the information — not two,
    /// one of them guessing.
    fn screen_reconcile_chunk(&self, next_cursor: u64) -> Option<TerminalChunk> {
        let (payload, model_cols) = {
            let screen_state = self
                .screen_state
                .lock()
                .expect("pty screen state lock poisoned");
            (
                screen_state.viewport_reconcile_replay()?,
                screen_state.size().1,
            )
        };
        let pty_cols = self.current_cols.load(Ordering::SeqCst);
        let max_column = formatted_screen_max_column(&payload, model_cols);
        let payload = if pty_cols > 0 && max_column > pty_cols {
            trace_terminal_event(
                "screen_reconcile_clipped_to_pty_width",
                serde_json::json!({
                    "path": self.key,
                    "pty_cols": pty_cols,
                    "screen_max_column": max_column,
                    "screen_model_cols": model_cols,
                }),
            );
            clip_formatted_screen_to_width(&payload, model_cols, pty_cols)
        } else {
            payload
        };
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
        let mut seeded_from_screen_snapshot = false;
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
            seeded_from_screen_snapshot = true;
        }
        if effective_cursor == 0
            && !seeded_from_screen_snapshot
            && !prefer_initial_screen_snapshot
            && !chunks
                .iter()
                .any(|chunk| terminal_chunk_has_visible_text(&chunk.data))
            && let Some(snapshot_chunk) = self.screen_snapshot_chunk(next_cursor)
        {
            chunks = vec![snapshot_chunk];
            seeded_from_screen_snapshot = true;
        }
        // A raw retained-chunk seed is a budget-truncated mid-stream replay:
        // faithful for scrollback, WRONG for the final viewport of a TUI that
        // paints with relative cursor motion (the persistent hole/interleave
        // garble a GUI restart used to leave on busy Claude Code sessions —
        // guihost 2026-07-10). Pin the viewport + cursor to the daemon's vt100
        // truth so subsequent live diffs anchor correctly. Snapshot seeds
        // already end at daemon truth. Codex resume attaches
        // (`prefer_initial_screen_snapshot`) are excluded: their runtime
        // re-runs `codex resume` and repaints in full, and their restored
        // vt100 screen can be STALER than the retained tail (an idle-prompt
        // frame painted over newer prose would fabricate readiness — see the
        // stale-prose-tail attach test).
        if effective_cursor == 0
            && !seeded_from_screen_snapshot
            && !prefer_initial_screen_snapshot
            && chunks
                .iter()
                .any(|chunk| terminal_chunk_has_visible_text(&chunk.data))
            && let Some(reconcile_chunk) = self.screen_reconcile_chunk(next_cursor)
        {
            chunks.push(reconcile_chunk);
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
        // Mid-stream gap resync (docs/xterm-bugs.md#chunk-ring-trim-drops-mid-
        // stream, the live-path variant of the 2.10.4 attach-seed fix): the
        // ring trimmed the contiguous middle, so the tail above replays
        // against a base the CLI never painted for THIS client — every cell a
        // subsequent diff frame skips (CUF / relative moves) then keeps stale
        // content, permanently. That is the busiest-CC character-interleave
        // corruption: the daemon vt100 stays clean (it consumed every byte),
        // the GUI forwards faithfully, and only the client base is wrong.
        // Anchor the client by appending the viewport reconcile AFTER the
        // tail: the tail still populates scrollback, and the final
        // clear+repaint pins viewport+cursor to daemon truth. Viewport-only
        // on purpose — normal-buffer history must never be injected under an
        // alternate-screen TUI (the reverted history+screen gap resync). The
        // attach-seed codex staleness exclusion does not apply here: on the
        // live path the vt100 state has consumed every ring byte, so it is
        // never staler than the tail it reconciles.
        if resync_required && let Some(reconcile_chunk) = self.screen_reconcile_chunk(next_cursor) {
            trace_terminal_event(
                "mid_stream_gap_reconciled",
                serde_json::json!({
                    "path": self.key,
                    "cursor": effective_cursor,
                    "oldest_surviving_seq": retained_chunks.front().map(|chunk| chunk.seq),
                    "next_cursor": next_cursor,
                    "tail_chunks": chunks.len(),
                }),
            );
            chunks.push(reconcile_chunk);
        }
        // A consumed web-surface `open` must never replay AS AN OPEN. The
        // declare was consumed into the retained record when it arrived
        // (`ingest_app_declares`); the raw bytes stay in the ring only as
        // scrollback transcript, and this cursor-0 attach seed is a REPLAY of
        // them. Served verbatim they re-execute launch intent on a client that
        // is merely re-attaching — which re-minted the launch tab over the
        // user's page (and, with tab restore off, deleted that page's saved
        // row). Serve them as `seen` instead — same length, so the chunk
        // skeleton (count, seqs, byte lengths) is untouched. The record's own
        // action gates the sliver where the `open` IS current; catch-up reads
        // (cursor > 0) are deliberately left verbatim — both rules live in
        // `attach_replay_neutralizes_web_surface_open`.
        if effective_cursor == 0 {
            let record_action = self
                .app_declares
                .lock()
                .expect("app declare log poisoned")
                .records()
                .into_iter()
                .find(|record| record.verb == "web-surface")
                .map(|record| record.action);
            if attach_replay_neutralizes_web_surface_open(record_action.as_deref()) {
                let rewritten = neutralize_replayed_web_surface_opens(&mut chunks);
                if rewritten > 0 {
                    trace_terminal_event(
                        "web_surface_open_replay_neutralized",
                        serde_json::json!({
                            "path": self.key,
                            "sequences": rewritten,
                            "record_action": record_action,
                        }),
                    );
                }
            }
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
        // Reconstruct the sticky "unsent draft on the current line" flag from
        // the forwarded input. This is the ONLY input path the client drives;
        // daemon-internal protocol auto-responses (DA/DSR replies) bypass it,
        // so they never fabricate a draft. See `pending_input_draft`.
        let prev_draft = self.pending_input_draft.load(Ordering::SeqCst);
        let next = {
            let mut line = self
                .pending_input_line
                .lock()
                .expect("pty input line lock poisoned");
            let next = yggterm_core::input_line_after(prev_draft, &line, data.as_bytes());
            *line = next.line.clone();
            next
        };
        if next.draft != prev_draft {
            self.pending_input_draft.store(next.draft, Ordering::SeqCst);
        }
        self.write_daemon_originated(data)
    }

    /// A write THIS DAEMON authored, which must not be mistaken for a human's.
    ///
    /// ⛔⛔ THE READINESS PROBE WAS BEING COUNTED AS THE HUMAN IT PROTECTS.
    /// `pending_input_draft` is reconstructed from whatever goes through
    /// `write`, and the echo-verified submit writes its marker through that same
    /// `write` — so the probe set the flag, slept 180 ms, read the flag it had
    /// just set, and refused its own submit with `HumanTyping { waited_ms: 180 }`.
    /// The number was the mechanism's signature: 180 ms is `PROBE_SETTLE`.
    ///
    /// ⚠ It bit exactly where echo-verification exists to help. When the CLI is
    /// already consuming input the marker echoes and the submit completes before
    /// the check, so healthy rows kept working; what broke is the composer that
    /// has drawn its prompt but is not yet reading — the first retry aborts, so a
    /// slow-starting composer can never be waited out, which is the original
    /// "prompt shown before the input loop is live" bug left unprotected again.
    ///
    /// ⇒ The exemption is the one the comment above already describes for DA/DSR
    /// auto-responses, applied to the other writes this daemon authors. It
    /// changes only who may SET the flag; every reader still refuses on a real
    /// human draft, which is what the probe must never type over.
    fn write_daemon_originated(&self, data: &str) -> Result<()> {
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
        // Same commit-lock discipline as the reader thread: chunks lock first,
        // then screen — screen state and the ring stay consistent under `read`.
        let mut chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        if let Ok(mut screen_state) = self.screen_state.lock() {
            screen_state.process(data.as_bytes());
        }
        self.runtime_output_seen.store(true, Ordering::SeqCst);
        self.last_activity_ms.store(now_millis(), Ordering::SeqCst);
        self.last_output_ms.store(now_millis(), Ordering::SeqCst);
        let seq_value = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
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

    /// How long input has gone UNANSWERED: the gap between the last byte written
    /// toward the child and the last byte it produced. `None` when the child has
    /// answered at least as recently as it was written to — the healthy case.
    ///
    /// ⭐ This is the passive form of what `terminal input-check` establishes by
    /// typing a marker and waiting for the echo. It costs nothing, writes nothing
    /// into anyone's session, and needs no human to notice a row has gone deaf.
    /// A wedged row is otherwise INVISIBLE: it renders `idle`, holds a composer,
    /// and its process sits in `epoll_wait` exactly as a healthy one does.
    fn input_unanswered_ms(&self) -> Option<u64> {
        let input = self.last_activity_ms.load(Ordering::SeqCst);
        let output = self.last_output_ms.load(Ordering::SeqCst);
        (input > output).then(|| input.saturating_sub(output))
    }

    /// Input has gone unanswered for longer than `threshold` — the row has been
    /// written to and has said nothing back.
    ///
    /// ⚠ SUSPECTED, never asserted: a child may legitimately take input and stay
    /// silent (a password prompt, a long-running command with echo off). This is
    /// the cheap trigger for the definitive check, not the verdict — the verdict
    /// costs a marker and an echo, and is worth paying only once something points
    /// at a row.
    fn wedge_suspected(&self, threshold: Duration) -> bool {
        self.input_unanswered_ms()
            .is_some_and(|gap| gap > threshold.as_millis() as u64)
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
        // The vt100 SCREEN MODEL is the third size in play, and the one the
        // client actually paints from (`state_formatted`). The old fast path
        // asked only about the PTY, so a model that had drifted wider stayed
        // wider FOREVER: every later resize to the same size answered
        // `resize_noop` and never touched it. That is why "two real SIGWINCHes
        // did not repair the frame" — the SIGWINCH repaired the PTY and the CLI,
        // and left the stale model to serve ghost cells past the new right edge.
        // Live on guihost 2026-07-25: a 168x63 PTY whose model still rendered to
        // column 204, which is the frame-corruption class in
        // docs/xterm-bugs.md#screen-model-wider-than-viewer.
        let screen_model_size = self
            .screen_state
            .lock()
            .ok()
            .map(|screen_state| screen_state.size());
        let screen_model_matches_request = screen_model_size == Some((rows, cols));
        if cache_matches_request && observed_before == Some((cols, rows)) {
            if screen_model_matches_request {
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
            if let Ok(mut screen_state) = self.screen_state.lock() {
                screen_state.resize(rows, cols);
            }
            trace_terminal_event(
                "resize_screen_model_repaired",
                serde_json::json!({
                    "path": self.key,
                    "cols": cols,
                    "rows": rows,
                    "stale_model_cols": screen_model_size.map(|(_, model_cols)| model_cols),
                    "stale_model_rows": screen_model_size.map(|(model_rows, _)| model_rows),
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

    /// Ask the child to exit the way a terminal emulator does when its window
    /// closes: SIGHUP, then SIGTERM. Shells and agent CLIs both handle these and
    /// flush their state. Returns true if the child was gone before we escalate.
    ///
    /// This replaces writing `/exit\r` (Claude Code), `/quit\r` (codex), or
    /// `exit\r` (shells) into the PTY. Synthetic input is APPENDED TO WHATEVER
    /// THE USER HAS ALREADY TYPED, so a half-written prompt got submitted with
    /// `/exit` stuck on the end — the agent then acted on it before dying
    /// (user-reported, 2026-07-09). It also never bought the graceful exit it
    /// was there for: the old code waited at most 300ms before SIGKILL, and no
    /// agent CLI shuts down that fast, so the injected text was nearly pure
    /// downside. A signal cannot collide with the user's input.
    #[cfg(unix)]
    fn signal_child_to_exit(&self, child: &mut PtyChildHandle) -> Result<bool> {
        for signal in [libc::SIGHUP, libc::SIGTERM] {
            // `signal` identity-checks an ADOPTED pid before firing (start time,
            // not just the number) and is a plain kill for an owned child.
            if !child.signal(signal) {
                return Ok(false);
            }
            for _ in 0..20 {
                // "Has it finished" — NOT "what was its status". An adopted
                // child can answer the first and never the second. The `?` is
                // the pre-split behaviour, kept: a probe that FAILED is not a
                // process that exited, and reporting it as one here would end
                // the shutdown early over something still running.
                if !child.is_running().context("checking terminal exit state")? {
                    return Ok(true);
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
        Ok(false)
    }
    #[cfg(not(unix))]
    fn signal_child_to_exit(&self, _child: &mut PtyChildHandle) -> Result<bool> {
        Ok(false)
    }

    fn shutdown(&self, stop_command: Option<&str>) -> Result<()> {
        let mut child = self.child.lock().expect("pty child lock poisoned");
        if let Some(command) = stop_command
            && !command.is_empty()
        {
            // Non-interactive runners only (recipe documents). Anything with a
            // prompt is closed by signal — see `signal_child_to_exit`.
            let _ = self.write(command);
            for _ in 0..2 {
                if !child.is_running().context("checking terminal exit state")? {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(50));
            }
        } else if self.signal_child_to_exit(&mut child)? {
            return Ok(());
        }

        // An ADOPTED child is nobody's: dropping the master fd only SIGHUPs the
        // foreground group, so this explicit kill is the only thing that ends it.
        let _ = child.kill();
        child.wait_for_exit();
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
            // Non-interactive runner (recipe document); see terminal_stop_command.
            let _ = self.write(command);
        } else {
            // Prompt-bearing session: ask the process to exit rather than typing
            // into the user's draft. The loop below still force-kills on timeout.
            #[cfg(unix)]
            {
                let mut child = self.child.lock().expect("pty child lock poisoned");
                // Identity-checked for an adopted pid; a plain kill for our own.
                child.signal(libc::SIGHUP);
            }
        }
        let key = self.key.clone();
        thread::spawn(move || {
            let started = Instant::now();
            loop {
                {
                    let mut child = self.child.lock().expect("pty child lock poisoned");
                    match child.is_running() {
                        Ok(false) => {
                            trace_terminal_event(
                                "graceful_shutdown_completed",
                                serde_json::json!({ "path": key }),
                            );
                            return;
                        }
                        Ok(true) if started.elapsed() >= force_after => {
                            let _ = child.kill();
                            child.wait_for_exit();
                            trace_terminal_event(
                                "graceful_shutdown_forced",
                                serde_json::json!({
                                    "path": key,
                                    "force_after_ms": force_after.as_millis(),
                                }),
                            );
                            return;
                        }
                        // Still running, not yet past the force deadline.
                        Ok(true) => {}
                        // A probe that FAILED is not a process that exited. An
                        // owned child is probed with `waitpid`, which really can
                        // fail, so folding the error into `false` would trace
                        // `graceful_shutdown_completed` over a process nobody
                        // has any evidence about — the teardown-lies bug class.
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

// ---- agent session resume-error telemetry ----------------------------------
// Claude Code and codex sporadically refuse to resume a conversation with
// errors like "Error: Session <uuid> is already in use", "No conversation
// found with session ID <uuid>", or "session ... not found / does not exist".
// The user hits these often enough to hurt but we had NO record of how many or
// when. Every PTY reader scans its control-stripped output for these shapes
// and records a throttled `agent_session_error` trace event plus a durable row
// in `agent-incidents.jsonl` — a tiny stream that outlives event-trace
// rotation by months, so occurrences can be counted across weeks.
// This is observation only: nothing about session behavior changes.

const AGENT_INCIDENT_FILENAME: &str = "agent-incidents.jsonl";
const AGENT_INCIDENT_ROTATED_FILENAME: &str = "agent-incidents.previous.jsonl";
const AGENT_INCIDENT_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// A TUI redraws its error screen on every resize/frame; one event per pattern
/// per minute per session is plenty to count occurrences without spam.
const AGENT_SESSION_ERROR_THROTTLE_MS: u64 = 60_000;
/// Unterminated tail kept between chunks so a phrase split across PTY reads
/// still matches. Bounded so a newline-free TUI stream cannot grow it.
const AGENT_SESSION_ERROR_CARRY_MAX_CHARS: usize = 400;

#[derive(Debug, PartialEq)]
struct AgentSessionErrorHit {
    pattern: &'static str,
    uuid: Option<String>,
    sample: String,
}

#[derive(Default)]
struct AgentSessionErrorScanner {
    carry: String,
    last_hit_ms: HashMap<&'static str, u64>,
}

impl AgentSessionErrorScanner {
    fn scan(&mut self, stripped: &str, now_ms: u64) -> Vec<AgentSessionErrorHit> {
        let combined = if self.carry.is_empty() {
            stripped.to_string()
        } else {
            format!("{}{}", self.carry, stripped)
        };
        let mut hits = Vec::new();
        for line in combined.split(['\n', '\r']) {
            let Some(hit) = agent_session_error_in_line(line) else {
                continue;
            };
            let due = self
                .last_hit_ms
                .get(hit.pattern)
                .copied()
                .map_or(true, |last| {
                    now_ms.saturating_sub(last) >= AGENT_SESSION_ERROR_THROTTLE_MS
                });
            if due {
                self.last_hit_ms.insert(hit.pattern, now_ms);
                hits.push(hit);
            }
        }
        let tail = combined.rsplit(['\n', '\r']).next().unwrap_or("");
        self.carry = if tail.chars().count() > AGENT_SESSION_ERROR_CARRY_MAX_CHARS {
            let skip = tail.chars().count() - AGENT_SESSION_ERROR_CARRY_MAX_CHARS;
            tail.chars().skip(skip).collect()
        } else {
            tail.to_string()
        };
        hits
    }
}

/// Whether a control-stripped, whitespace-normalized, lowercased line has the SHAPE of
/// a CLI session refusal rather than prose that merely mentions one.
///
/// TERSENESS is the discriminator. A real refusal is a short status line
/// (`error: session id <uuid> is already in use.` = 8 words; `that session does not
/// exist anymore` = 6). A rendered conversation line that MENTIONS a refusal is a
/// sentence (the user's "…greeted with session already in use or does not exist" = 28
/// words; the agent's reply explaining the bug = 30). Prefix-matching cannot separate
/// them — real CLI errors do not all lead with `error:` — but length does, cleanly.
///
/// Residual gap (accepted, documented for the next campaign run): prose terse enough to
/// fit the budget still counts. If that shows up in `agent-incidents.jsonl`, tighten by
/// requiring a uuid or a leading error token.
fn agent_session_error_line_looks_like_cli_error(normalized: &str) -> bool {
    const CLI_ERROR_LINE_MAX_WORDS: usize = 16;
    const CLI_ERROR_LINE_MAX_CHARS: usize = 200;
    normalized.chars().count() <= CLI_ERROR_LINE_MAX_CHARS
        && normalized.split_whitespace().count() <= CLI_ERROR_LINE_MAX_WORDS
}

fn agent_session_error_in_line(line: &str) -> Option<AgentSessionErrorHit> {
    let normalized = line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    // yggterm's own missing-runtime error — traced through its own channel.
    if normalized.contains("terminal session not found") {
        return None;
    }
    // The PTY stream we scan CONTAINS the agent's rendered conversation, so a plain
    // substring match fires on any prose that merely MENTIONS these errors — the
    // user typing "greeted with session already in use or does not exist", or the
    // agent's own reply explaining the bug. Three of guihost's 21 recorded incidents
    // were exactly this self-inflicted noise (2026-07-11 telemetry campaign), which
    // corrupts the very count the probe exists to produce. A real CLI refusal is a
    // terse line that STARTS with the error; prose mentions it mid-sentence. Gate on
    // that shape before classifying.
    if !agent_session_error_line_looks_like_cli_error(&normalized) {
        return None;
    }
    let uuid = find_uuid_in_text(&normalized);
    let mentions_session = normalized.contains("session") || normalized.contains("conversation");
    let pattern = if normalized.contains("already in use") && (mentions_session || uuid.is_some())
    {
        "session_already_in_use"
    } else if normalized.contains("already active") && (mentions_session || uuid.is_some()) {
        "session_already_active"
    } else if normalized.contains("no conversation found") || normalized.contains("no rollout found")
    {
        "session_not_found"
    } else if mentions_session
        && (normalized.contains("not found")
            || normalized.contains("does not exist")
            || normalized.contains("doesn't exist"))
    {
        "session_not_found"
    } else if uuid.is_some()
        && (normalized.contains("not found")
            || normalized.contains("does not exist")
            || normalized.contains("doesn't exist")
            || normalized.contains("in use"))
    {
        "session_uuid_error"
    } else {
        return None;
    };
    Some(AgentSessionErrorHit {
        pattern,
        uuid,
        sample: truncate_terminal_trace_sample(&normalized),
    })
}

/// First canonical 8-4-4-4-12 UUID in the text, if any. Byte-safe: every
/// matched byte is ASCII hex or a dash, so slicing at the match is valid UTF-8.
fn find_uuid_in_text(text: &str) -> Option<String> {
    const UUID_LEN: usize = 36;
    const DASH_OFFSETS: [usize; 4] = [8, 13, 18, 23];
    let bytes = text.as_bytes();
    if bytes.len() < UUID_LEN {
        return None;
    }
    'outer: for start in 0..=bytes.len() - UUID_LEN {
        for offset in 0..UUID_LEN {
            let byte = bytes[start + offset];
            let ok = if DASH_OFFSETS.contains(&offset) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            };
            if !ok {
                continue 'outer;
            }
        }
        return Some(text[start..start + UUID_LEN].to_string());
    }
    None
}

/// Lift libyggterm declares out of a chunk and retain the latest per verb.
///
/// Runs on every PTY chunk, so the scanner's no-escape fast path matters: the
/// cost on ordinary output is one `contains('\x1b')`. Only a state CHANGE is
/// traced — an app heartbeats its full payload every ~4s, and tracing each one
/// would bury the trace in noise for zero diagnostic value.
fn ingest_app_declares(
    scanner: &mut AppDeclareScanner,
    log: &Arc<Mutex<AppDeclareLog>>,
    path: &str,
    data: &str,
) {
    let messages = scanner.scan(data);
    if messages.is_empty() {
        return;
    }
    let now_ms = now_millis();
    // Identity of the retained STATE, ignoring the timestamp and seq a
    // heartbeat refreshes — otherwise every heartbeat looks like a change.
    let state_of = |records: &[AppDeclareRecord]| {
        records
            .iter()
            .map(|record| {
                (
                    record.verb.clone(),
                    record.action.clone(),
                    record.payload.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    for message in messages {
        let Ok(mut log) = log.lock() else {
            return;
        };
        let before = state_of(&log.records());
        let verb = message.verb.clone();
        let action = message.action.clone();
        log.ingest(message, now_ms);
        let after = log.records();
        if before != state_of(&after) {
            trace_terminal_event(
                "app_declare_ingested",
                serde_json::json!({
                    "path": path,
                    "verb": verb,
                    "action": action,
                    "retained_verbs": after.iter().map(|record| record.verb.clone()).collect::<Vec<_>>(),
                }),
            );
        }
    }
}

fn record_agent_session_error(path: &str, launch_command: &str, hit: &AgentSessionErrorHit) {
    let payload = serde_json::json!({
        "path": path,
        "pattern": hit.pattern,
        "uuid": hit.uuid,
        "sample": hit.sample,
        "launch_command": launch_command,
    });
    trace_terminal_event("agent_session_error", payload.clone());
    if let Ok(home) = resolve_yggterm_home() {
        let record = serde_json::json!({
            "ts_ms": now_millis(),
            "kind": "agent_session_error",
            "path": path,
            "pattern": hit.pattern,
            "uuid": hit.uuid,
            "sample": hit.sample,
            "launch_command": launch_command,
        });
        append_bounded_jsonl_record(
            &home.join(AGENT_INCIDENT_FILENAME),
            AGENT_INCIDENT_ROTATED_FILENAME,
            AGENT_INCIDENT_MAX_BYTES,
            &record,
        );
    }
}

/// Session-identity handshake for libyggterm apps (the `$TMUX` pattern):
/// every daemon-owned PTY exports which yggterm session it is and where the
/// yggterm CLI binary lives, so a program like `ychrome` can detect it is
/// inside yggterm and drive the daemon (e.g. `server web-surface`) without
/// re-deriving endpoint/protocol knowledge the CLI already owns.
fn apply_session_identity_env(command: &mut CommandBuilder, session_key: Option<&str>) {
    let Some(session_key) = session_key else {
        return;
    };
    command.env("YGGTERM_SESSION_ID", session_key);
    // The iTerm2 `LC_TERMINAL` trick: a user-typed `ssh <host>` strips the
    // environment, but stock OpenSSH forwards `LC_*` (client `SendEnv LANG
    // LC_*`, server `AcceptEnv LANG LC_*` — the Debian defaults), so a
    // libyggterm app on the far side of a MANUAL ssh hop can still detect it
    // is inside a yggterm surface (user report 2026-07-23: yedit said "not
    // inside yggterm" after `ssh` from a local yggterm terminal). Apps check
    // `YGGTERM_SESSION_ID` first, then this mirror. NOTE: detection is only
    // half the remote story — the GUI still needs a route to the app's
    // loopback control endpoint (see docs/pending-bugs.md, manual-ssh
    // control-channel attribution).
    command.env("LC_YGGTERM_SESSION_ID", session_key);
    if let Ok(exe) = std::env::current_exe() {
        command.env("YGGTERM_BIN", exe.as_os_str());
    }
}

fn shell_command(
    launch_command: &str,
    cwd: Option<&str>,
    session_key: Option<&str>,
) -> CommandBuilder {
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
        apply_session_identity_env(&mut command, session_key);
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
    apply_session_identity_env(&mut command, session_key);
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

/// True when a launch command is a REMOTE AGENT resume/start attach.
///
/// ⚠ Fixed 2026-07-27 (harness spec §7.3, phase 3): this hand-listed
/// `resume-codex` / `start-codex`, so a remote CLAUDE CODE attach matched
/// neither this nor the `remote-session://` key check beside it — BOTH halves of
/// the retained-chunk preservation guards missed remote-cc. Now derived from the
/// wrapper subcommand registry, so a new CLI is covered by registering it.
fn launch_command_looks_like_remote_resume_attach(launch_command: &str) -> bool {
    yggterm_core::agent_cli::AGENT_CLIS.iter().any(|descriptor| {
        [
            crate::remote_agent_resume_subcommand(descriptor.kind),
            crate::remote_agent_start_subcommand(descriptor.kind),
        ]
        .into_iter()
        // A local-only CLI yields no verbs at all, which is the honest empty
        // set — not codex's pair borrowed on its behalf.
        .flatten()
        .any(|subcommand| {
            launch_command.contains(&format!(
                "server'\\'' '\\''remote'\\'' '\\''{subcommand}"
            ))
        })
    })
}

/// True when a terminal key names a runtime that lives on another machine and
/// belongs to an agent CLI — the remote agent ROW schemes plus their RUNTIME
/// keys, derived from the scheme registry.
///
/// Replaces three hand-written `remote-session:// || codex-runtime://` lists
/// that each skipped Claude Code (harness spec §7.3).
fn terminal_key_is_remote_agent(key: &str) -> bool {
    yggterm_core::agent_scheme::remote_agent_schemes()
        .any(|scheme| key.starts_with(scheme.prefix))
}

/// True when a session's PTY child is carried over ssh (remote agent bridges
/// use `ssh -tt …`; plain ssh shells start with `ssh `). These are the
/// sessions whose transport dies across a laptop suspend — see
/// `respawn_ssh_carried_sessions`.
fn launch_command_is_ssh_carried(launch_command: &str) -> bool {
    launch_command.contains("ssh -tt ") || launch_command.trim_start().starts_with("ssh ")
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
    terminal_key_is_remote_agent(key)
        || launch_command_looks_like_remote_resume_attach(launch_command)
}

fn initial_remote_attach_should_preserve_retained_chunks(
    key: &str,
    launch_command: &str,
    chunks: &[TerminalChunk],
) -> bool {
    if !(terminal_key_is_remote_agent(key)
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
    if !(terminal_key_is_remote_agent(key)
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

/// Rewrite every consumed web-surface `open` in a SERVED attach seed to `seen`.
///
/// Operates on the joined tail because a declare can straddle chunk boundaries
/// (the PTY reader hands the ring arbitrary splits — the same reason
/// `AppDeclareScanner` reassembles at ingest); a per-chunk replace would miss
/// exactly those. The swap is same-length by construction, so the joined
/// result re-slices at the ORIGINAL chunk byte lengths: chunk count, seqs and
/// sizes are all unchanged, and only the served COPY is touched — the ring
/// itself keeps its faithful transcript. An `open` whose terminator has not
/// reached the ring yet is not in this tail at all; its remainder arrives on a
/// later read as live bytes, which is the just-launched case and correct.
///
/// Returns how many sequences were rewritten (0 = bytes untouched).
fn neutralize_replayed_web_surface_opens(chunks: &mut [TerminalChunk]) -> usize {
    let joined: String = chunks
        .iter()
        .map(|chunk| chunk.data.as_str())
        .collect();
    let Some((rewritten, count)) = rewrite_consumed_web_surface_opens(&joined) else {
        return 0;
    };
    let mut offset = 0;
    for chunk in chunks.iter_mut() {
        let len = chunk.data.len();
        // Same-length swap of ASCII for ASCII: every original boundary is
        // still a char boundary in the rewritten stream.
        chunk.data = rewritten[offset..offset + len].to_string();
        offset += len;
    }
    count
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
    // XTERM-BUG: content-clip-on-reveal (campaign #1). codex's composer / input
    // row — a line beginning with the `›` prompt glyph, INCLUDING when it only
    // shows a rotating placeholder hint ("Summarize recent commits", "Explain
    // this codebase", …) — is the LIVE INPUT ROW the user types into, not
    // disposable idle chrome. The generic-attach-prompt / idle-footer matchers
    // below were classifying it disposable, so the initial-attach replay tail got
    // trimmed of the composer; the revealed surface then showed a "broken bottom"
    // (gray composer bar, no `›` text / footer) while the daemon screen had the
    // full composer, and idle codex never re-emits to repaint it. Wrapper-vs-manual
    // parity (project-purpose) requires the reveal render exactly what
    // `ssh codex resume` shows, which includes this composer. So a chunk carrying
    // it is NEVER a disposable suffix.
    if terminal_chunk_carries_codex_composer_input_row(&stripped) {
        return false;
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

/// True when the (control-stripped) chunk carries codex's composer / current
/// input row — a line beginning with the `›` prompt glyph. This is the live row
/// the user types into; it must survive initial-attach suffix trimming even when
/// it only shows a placeholder hint. See content-clip-on-reveal (campaign #1).
fn terminal_chunk_carries_codex_composer_input_row(stripped: &str) -> bool {
    stripped.lines().any(|line| {
        let Some(rest) = line.trim_start().strip_prefix('›') else {
            return false;
        };
        // A real composer carries actual text after the `›` glyph — a placeholder
        // hint or user input. Reject two non-composer cases: (a) a bare `›` with no
        // real text, and (b) a `›` line that is actually leaked terminal-negotiation
        // noise (device-attribute / color-query / cursor-report responses, e.g.
        // "› ^[[?1;2c^[]10;rgb:cccc/cccc/cccc^[[1;1R") which the low-signal detector
        // already recognizes.
        if !rest.chars().any(|ch| ch.is_alphanumeric()) {
            return false;
        }
        let lower = rest.to_ascii_lowercase();
        !(lower.contains("^[[?")
            || lower.contains("^[]10;")
            || lower.contains("^[]11;")
            || lower.contains("rgb:")
            || lower.contains("[1;1r")
            || lower.contains("[?1;2c"))
    })
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
mod screen_width_tests {
    use super::{clip_formatted_screen_to_width, formatted_screen_max_column};

    /// The exact shape the daemon serves (measured live on guihost 2026-07-25):
    /// absolute positioning per row, `CSI C` for every run of blanks, and rows
    /// that reach past the viewer's right edge.
    const WIDE_SCREEN: &str = "\u{1b}[H\u{1b}[J\u{1b}[1;1Habc\u{1b}[Cdef\u{1b}[2;1Hxy";

    /// The grid WIDE_SCREEN was formatted against. Wide enough that nothing in
    /// it wraps, so these cases test the measurement and not the wrap.
    const GRID: u16 = 80;

    #[test]
    fn max_column_counts_blank_runs_not_bytes() {
        // "abc" + one skipped cell + "def" = column 7. Byte length says 30-odd,
        // which is exactly why `screen_text.len()` could never have caught this.
        assert_eq!(formatted_screen_max_column(WIDE_SCREEN, GRID), 7);
        assert!(WIDE_SCREEN.len() > 20);
    }

    #[test]
    fn a_screen_that_fits_is_returned_unchanged() {
        assert_eq!(clip_formatted_screen_to_width(WIDE_SCREEN, GRID, 7), WIDE_SCREEN);
        assert_eq!(clip_formatted_screen_to_width(WIDE_SCREEN, GRID, 80), WIDE_SCREEN);
    }

    /// The fix: cells past the viewer's width are dropped, and every control
    /// sequence survives — colour/attribute state must not be lost with them.
    #[test]
    fn clipping_drops_only_the_cells_past_the_edge() {
        let clipped = clip_formatted_screen_to_width(WIDE_SCREEN, GRID, 5);
        assert_eq!(clipped, "\u{1b}[H\u{1b}[J\u{1b}[1;1Habc\u{1b}[Cd\u{1b}[2;1Hxy");
        assert_eq!(formatted_screen_max_column(&clipped, GRID), 5);
        // The row that already fit is untouched.
        assert!(clipped.contains("\u{1b}[2;1Hxy"));
    }

    #[test]
    fn colour_state_survives_the_clip() {
        let text = "\u{1b}[1;1H\u{1b}[31mred\u{1b}[32mgreen\u{1b}[m";
        let clipped = clip_formatted_screen_to_width(text, GRID, 3);
        assert_eq!(clipped, "\u{1b}[1;1H\u{1b}[31mred\u{1b}[32m\u{1b}[m");
    }

    #[test]
    fn wide_glyphs_count_two_cells() {
        // A fullwidth character straddling the edge is dropped whole, never
        // half-painted.
        let text = "\u{1b}[1;1Ha\u{4e00}b";
        assert_eq!(formatted_screen_max_column(text, GRID), 4);
        assert_eq!(clip_formatted_screen_to_width(text, GRID, 2), "\u{1b}[1;1Ha");
    }

    /// The measurement must survive a payload it does not fully understand:
    /// an OSC string carries a `;` and digits that would look like CSI params.
    #[test]
    fn osc_sequences_do_not_move_the_cursor() {
        let text = "\u{1b}[1;1H\u{1b}]0;a window title\u{7}ok";
        assert_eq!(formatted_screen_max_column(text, GRID), 2);
    }

    /// ★★★ A WRAPPED LINE IS NOT A WIDE SCREEN, AND THE CLIP MUST NOT EAT IT.
    ///
    /// This is the 2026-08-04 bug, in the smallest form that shows it. A vt100
    /// formatter emits a wrapped row as ONE continuous run with no break — that
    /// is what makes the receiving terminal re-wrap it at its own width instead
    /// of hard-breaking it at ours — so a walker counting columns straight
    /// through reads the last cell of a 400-character line on a 170-column grid
    /// as "column 400", and the clip then deletes everything past 170: the
    /// second and third visual row of the line.
    ///
    /// On the live host that read as 504 `screen_snapshot_clipped_to_pty_width`
    /// events across two shells, `pty_cols: 170` against "screen_max_column"
    /// 334 and 260 — CONSTANT, because nothing was changing; it was the same
    /// wrapped lines being mis-measured on every single snapshot.
    #[test]
    fn a_wrapped_line_measures_to_the_grid_and_is_never_clipped() {
        let long: String = std::iter::repeat('x').take(400).collect();
        let wrapped = format!("\x1b[1;1H{long}");
        // The whole point: 400 printable cells on a 170-column grid are 170
        // columns wide, not 400.
        assert_eq!(formatted_screen_max_column(&wrapped, 170), 170);
        // …so nothing is dropped when the model and the PTY agree, which is
        // every healthy session.
        assert_eq!(clip_formatted_screen_to_width(&wrapped, 170, 170), wrapped);
    }

    /// …and the check this code was WRITTEN for still works. A stale model
    /// wider than the PTY wraps at its own wider width, so its cells really do
    /// land past the PTY's edge and are still found and dropped.
    #[test]
    fn a_model_wider_than_the_pty_is_still_caught() {
        // 200 cells on a 200-column model: no wrap, so they occupy columns
        // 1..200 — of which everything past 170 is a ghost the CLI could never
        // have painted.
        let row: String = std::iter::repeat('y').take(200).collect();
        let stale = format!("\x1b[1;1H{row}");
        assert_eq!(formatted_screen_max_column(&stale, 200), 200);
        let clipped = clip_formatted_screen_to_width(&stale, 200, 170);
        assert_eq!(
            clipped.chars().filter(|c| *c == 'y').count(),
            170,
            "the ghost cells past the PTY edge must go, and only those"
        );
    }

    /// A wrap must not survive a cursor jump: `CSI r;cH` re-seats the column,
    /// and the row it lands on starts counting again.
    #[test]
    fn an_absolute_jump_reseats_the_column_after_a_wrap() {
        let long: String = std::iter::repeat('z').take(30).collect();
        let text = format!("\x1b[1;1H{long}\x1b[3;5Hq");
        // 30 cells on a 10-column grid wrap to three rows; the jump then puts
        // `q` at column 5, which is where the measurement must end up.
        assert_eq!(formatted_screen_max_column(&text, 10), 10);
        assert_eq!(clip_formatted_screen_to_width(&text, 10, 10), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use std::io;
    use std::sync::mpsc;
    use std::time::Instant;

    /// ⛔ THE PROBE MAY NOT TYPE OVER A HUMAN.
    ///
    /// Reported live 2026-08-14: *"blinking profusely and I could not type"*,
    /// and the owner's own next message arrived shredded —
    /// `yggterm_ready_probeBy yggterm_ready_probese…` — his keystrokes
    /// interleaved with our marker. The submit path writes `yggterm_ready_probe`
    /// and, when it does not echo, sends Ctrl+U (which erases the line the human
    /// is composing) and retries, ~3×/s for the whole 30 s timeout. Against a
    /// row a person is typing at, that is ~100 injected markers and ~100 erased
    /// lines, and it is also the "viewport blinking" symptom: the loop painting
    /// and wiping the composer three times a second.
    ///
    /// ⭐ Asserts on the WRITES, not on the return value. A version that returned
    /// the right enum after already stomping the composer would pass a
    /// verdict-only test and still ruin the sentence someone was typing — the
    /// damage IS the write, so the write is what the test has to watch.
    #[test]
    fn the_readiness_probe_never_writes_into_a_composer_a_human_is_using() {
        let writes = Arc::new(Mutex::new(Vec::<String>::new()));
        let record = Arc::clone(&writes);
        let outcome = submit_prompt_echo_verified_with(
            move |text| {
                record.lock().unwrap().push(text.to_string());
                Ok(())
            },
            // A live session, so the refusal cannot be mistaken for "no session".
            || Some(String::from("$ ")),
            // A human has an unsent draft in the composer.
            || Some(true),
            "continue",
            Duration::from_millis(500),
        )
        .expect("probe must not error");

        assert!(
            matches!(outcome, PromptSubmitOutcome::HumanTyping { .. }),
            "a composer with a human draft must yield HumanTyping, got {outcome:?}"
        );
        assert!(
            writes.lock().unwrap().is_empty(),
            "NOTHING may be written at a human mid-sentence — not the probe, and \
             above all not the Ctrl+U that erases their line. Wrote: {:?}",
            writes.lock().unwrap()
        );
    }

    /// The other half: with no human draft the probe still works exactly as
    /// before. A guard that refused everything would "fix" the symptom by
    /// breaking every automated submit on the fleet.
    #[test]
    fn the_readiness_probe_still_submits_when_no_human_is_typing() {
        let writes = Arc::new(Mutex::new(Vec::<String>::new()));
        let record = Arc::clone(&writes);
        let screen = Arc::new(Mutex::new(String::from("$ ")));
        let echo = Arc::clone(&screen);
        let outcome = submit_prompt_echo_verified_with(
            move |text| {
                record.lock().unwrap().push(text.to_string());
                // The child is consuming input: it echoes what it is sent.
                echo.lock().unwrap().push_str(text);
                Ok(())
            },
            move || Some(screen.lock().unwrap().clone()),
            // Confirmed empty composer.
            || Some(false),
            "continue",
            Duration::from_secs(2),
        )
        .expect("probe must not error");

        assert!(
            matches!(outcome, PromptSubmitOutcome::Submitted { .. }),
            "an idle composer must still be submitted to, got {outcome:?}"
        );
        let wrote = writes.lock().unwrap().clone();
        assert!(
            wrote.iter().any(|w| w == "continue"),
            "the payload must actually be written: {wrote:?}"
        );
        assert!(
            wrote.last().is_some_and(|w| w == "\r"),
            "Enter is a SEPARATE write of \\r and must come last: {wrote:?}"
        );
    }

    /// The daemon's vt100 mirror and the client's xterm must agree on how many
    /// cells an emoji occupies. When they disagree, every line carrying one
    /// drifts, and a partial repaint strands the old glyph in the orphaned
    /// column — the owner's "weird characters appearing here and there"
    /// (docs/pending-bugs.md).
    ///
    /// ⭐ On 2026-08-11 the two DID disagree, and it was the client that was
    /// wrong: the vendored xterm registered only Unicode v6 and scored ⭐, ⛔,
    /// ✅ and 🚀 at ONE cell, while this mirror — `vt100` on `unicode-width`,
    /// a modern UAX #11 table — has always scored them at two, agreeing with
    /// every agent CLI that writes them. 3.0.108 fixed the client side.
    ///
    /// This pins the daemon half so a future dependency bump cannot silently
    /// move it and re-open the gap from the other direction. Its twin is
    /// `tools/xterm-harness/emoji_cell_width.test.js`, which pins the client
    /// half against the real bundle; the pair is the actual invariant, and
    /// neither test alone can see a divergence.
    #[test]
    fn the_daemon_screen_gives_an_emoji_two_cells_like_the_client_does() {
        // Written at column 0, a two-cell glyph puts the cursor at column 2.
        for (label, glyph) in [
            ("⭐ U+2B50", "\u{2B50}"),
            ("⛔ U+26D4", "\u{26D4}"),
            ("✅ U+2705", "\u{2705}"),
            ("🚀 U+1F680", "\u{1F680}"),
            ("中 U+4E2D", "\u{4E2D}"),
        ] {
            let mut parser = Vt100Parser::new(4, 40, 0);
            parser.process(glyph.as_bytes());
            assert_eq!(
                parser.screen().cursor_position().1,
                2,
                "{label} must occupy two cells in the daemon mirror, or it \
                 disagrees with the client and every line carrying it drifts"
            );
        }
        // ⛔ The control, and it is the one that proves the boundary rather than
        // the rule: U+26A0 is TEXT presentation. It is one cell, it rendered
        // correctly in the owner's own frames while ⭐ and ⛔ did not, and
        // widening it would cause the identical drift in the opposite
        // direction. Both sides must leave it narrow.
        let mut parser = Vt100Parser::new(4, 40, 0);
        parser.process("\u{26A0}".as_bytes());
        assert_eq!(
            parser.screen().cursor_position().1,
            1,
            "U+26A0 ⚠ is text-presentation and must stay ONE cell on both sides"
        );
    }

    /// A child that asks the terminal for its foreground colour and reports
    /// whether the answer matched `YGG_EXPECT_FG_HEX` (hex so no escape
    /// sequence has to survive shell quoting).
    // ⛔ THE DEADLINE THAT MATTERS IS HERE, IN THE CHILD, not in the Rust poll
    // that waits for this to print. The child fired the OSC-10 query and gave the
    // daemon a single 2.0 s `select` to answer; under `cargo test --workspace`
    // the daemon's reply misses that window, the child reads nothing, prints
    // COLOR_BAD, and the outer poll (which breaks on any "COLOR_") fails the
    // COLOR_OK assert. Raising the OUTER poll did nothing — it was guarding the
    // wrong moment. Now the child accumulates across a wall-clock deadline and
    // stops the instant the full reply has arrived, so a slow schedule is a late
    // answer, never a wrong one.
    const OSC_COLOR_QUERY_CHILD: &str = r#"python3 - <<'PY'
import os
import select
import sys
import termios
import time
import tty

fd = os.open('/dev/tty', os.O_RDWR | getattr(os, 'O_NOCTTY', 0))
old = termios.tcgetattr(fd)
data = b''
try:
    tty.setraw(fd)
    os.write(fd, b'\x1b]10;?\x1b\\')
    expected = bytes.fromhex(os.environ['YGG_EXPECT_FG_HEX'])
    deadline = time.monotonic() + 10.0
    while data != expected:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        ready, _, _ = select.select([fd], [], [], min(1.0, remaining))
        if ready:
            data += os.read(fd, 64)
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
    os.close(fd)

expected = bytes.fromhex(os.environ['YGG_EXPECT_FG_HEX'])
sys.stdout.write('COLOR_OK\n' if data == expected else f'COLOR_BAD:{data!r} want {expected!r}\n')
PY"#;

    /// The protocol profile for a launch command, with NO host colour profile.
    ///
    /// The suite is normally run from a terminal inside yggterm, whose PTY
    /// carries `YGGTERM_TERMINAL_COLOR_*`; reading those made three colour
    /// assertions fail there and nowhere else. Tests state their own input.
    fn test_protocol_profile(launch_command: &str) -> TerminalProtocolProfile {
        TerminalProtocolProfile::from_launch_command_with_host_profile(launch_command, None)
    }

    // ── Scheme-registry predicate locks (harness spec §2.3/§8 phase 0) ──────

    #[test]
    fn scheme_registry_lock_terminal_key_prefers_initial_screen_snapshot() {
        use yggterm_core::agent_scheme::{self, SchemeLocality};
        let name = "terminal_key_prefers_initial_screen_snapshot";
        let in_scope = |s: &agent_scheme::SchemeDescriptor| {
            s.agent && !s.legacy && s.locality == SchemeLocality::Remote
        };
        for scheme in agent_scheme::SESSION_PATH_SCHEMES.iter().filter(|s| in_scope(s)) {
            let covered = terminal_key_prefers_initial_screen_snapshot(scheme.example, "");
            let hole = agent_scheme::predicate_hole_allowed(name, scheme.prefix);
            assert!(
                covered || hole,
                "{name} ignores {} and no hole is recorded — fix it or record it",
                scheme.prefix
            );
            assert!(
                !(covered && hole),
                "STALE HOLE: {name}×{} — delete the KNOWN_PREDICATE_HOLES row",
                scheme.prefix
            );
        }
        for hole in agent_scheme::predicate_holes_for(name) {
            let scheme = agent_scheme::scheme_for_prefix(hole.scheme)
                .expect("hole names a registered scheme");
            assert!(in_scope(scheme), "{name}'s hole row {} out of scope", hole.scheme);
        }
    }

    // The remote-resume-attach recognizer must know every agent kind's resume
    // AND start subcommand (the kind table in lib.rs is the SSOT it should
    // derive from — §7.4). The lock keys holes on the kind's remote ROW
    // scheme, since the subcommand itself is not a scheme.
    #[test]
    fn scheme_registry_lock_launch_command_looks_like_remote_resume_attach() {
        use yggterm_core::agent_scheme::{self, SchemeLocality, SchemeRole};
        let name = "launch_command_looks_like_remote_resume_attach";
        for scheme in agent_scheme::SESSION_PATH_SCHEMES.iter().filter(|s| {
            s.agent
                && !s.legacy
                && s.locality == SchemeLocality::Remote
                && matches!(s.role, SchemeRole::RowIdentity)
        }) {
            let kind = scheme.kind.expect("remote agent row schemes are kind-specific");
            for subcommand in [
                crate::remote_agent_resume_subcommand(kind),
                crate::remote_agent_start_subcommand(kind),
            ]
            .into_iter()
            // A registered remote ROW scheme implies a wrapper slug, so this
            // flatten drops nothing today; it is here so a local-only CLI that
            // ever grew a row scheme would be skipped rather than tested
            // against codex's verbs.
            .flatten()
            {
                let launch_command = format!(
                    "ssh -tt devhost -- sh -lc 'exec \"$HOME\"/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''{subcommand}'\\'' …'"
                );
                let covered = launch_command_looks_like_remote_resume_attach(&launch_command);
                let hole = agent_scheme::predicate_hole_allowed(name, scheme.prefix);
                assert!(
                    covered || hole,
                    "{name} does not recognize `{subcommand}` and no hole is recorded for {}",
                    scheme.prefix
                );
                assert!(
                    !(covered && hole),
                    "STALE HOLE: {name}×{} — delete the KNOWN_PREDICATE_HOLES row",
                    scheme.prefix
                );
            }
        }
    }

    #[test]
    fn scheme_registry_lock_initial_remote_attach_should_preserve_retained_chunks() {
        use yggterm_core::agent_scheme::{self, SchemeLocality, SchemeRole};
        let name = "initial_remote_attach_should_preserve_retained_chunks";
        // A chunk that clears the scrollback-text bar (DEFAULT_ROWS+4
        // non-empty lines), so the key arm is the only variable under test.
        let scrollback: String = (0..usize::from(DEFAULT_ROWS) + 5)
            .map(|n| format!("scrollback line {n} with real words\r\n"))
            .collect();
        let chunks = vec![TerminalChunk {
            seq: 1,
            data: scrollback,
        }];
        for scheme in agent_scheme::SESSION_PATH_SCHEMES.iter().filter(|s| {
            s.agent
                && !s.legacy
                && s.locality == SchemeLocality::Remote
                && matches!(s.role, SchemeRole::RowIdentity)
        }) {
            let covered =
                initial_remote_attach_should_preserve_retained_chunks(scheme.example, "", &chunks);
            let hole = agent_scheme::predicate_hole_allowed(name, scheme.prefix);
            assert!(
                covered || hole,
                "{name} ignores {} and no hole is recorded — fix it or record it",
                scheme.prefix
            );
            assert!(
                !(covered && hole),
                "STALE HOLE: {name}×{} — delete the KNOWN_PREDICATE_HOLES row",
                scheme.prefix
            );
        }
    }

    // The session-identity handshake must survive a MANUAL `ssh <host>` hop:
    // stock OpenSSH forwards LC_* (SendEnv/AcceptEnv defaults), so the LC_
    // mirror is what lets a libyggterm app on the far side detect the surface
    // (user report 2026-07-23: yedit said "not inside yggterm" after ssh).
    // Both names carry the SAME key — a divergence would be two identities.
    #[test]
    fn session_identity_env_exports_the_lc_mirror_for_ssh_hops() {
        let mut command = CommandBuilder::new("true");
        apply_session_identity_env(&mut command, Some("local://abc"));
        assert_eq!(
            command.get_env("YGGTERM_SESSION_ID").and_then(|v| v.to_str()),
            Some("local://abc")
        );
        assert_eq!(
            command
                .get_env("LC_YGGTERM_SESSION_ID")
                .and_then(|v| v.to_str()),
            Some("local://abc"),
            "the LC_ mirror is the only identity that survives a user-typed ssh hop"
        );
        // "Sets nothing" — asserted as a DIFF, not as absence: a CommandBuilder
        // inherits this process' environment, and this suite is usually run
        // from a terminal inside yggterm, which exports the very variable the
        // old `is_none()` demanded be missing.
        let mut absent = CommandBuilder::new("true");
        let inherited = absent
            .get_env("LC_YGGTERM_SESSION_ID")
            .map(ToOwned::to_owned);
        apply_session_identity_env(&mut absent, None);
        assert_eq!(
            absent
                .get_env("LC_YGGTERM_SESSION_ID")
                .map(ToOwned::to_owned),
            inherited,
            "a session-less launch must not ADD an identity of its own"
        );
    }

    #[test]
    fn daemon_vt100_preserves_composer_bg_across_column_resize() {
        // Regression lock + FALSIFICATION of the long-standing "reflow drops cell
        // bg" theory for the composer bg-split (issue #2). The daemon's vt100
        // set_size preserves every already-painted cell's bg across a column
        // resize in BOTH directions — only newly-exposed cells are default. So the
        // split is NOT produced by the daemon emulator's reflow (nor xterm's — see
        // tools/xterm-harness behavior.test.js). The real producer is frame tearing
        // of codex's synchronized-output repaint. finding-codex-composer-bg-split-reflow.
        let gray = "\x1b[39;48;2;64;67;75m";
        let row_is_uniform_gray = |state: &TerminalScreenState, row: u16, upto: u16| -> bool {
            let screen = state.parser.screen();
            (0..upto).all(|c| matches!(screen.cell(row, c).map(|cell| cell.bgcolor()),
                Some(vt100::Color::Rgb(64, 67, 75))))
        };
        for (start, end) in [(120u16, 159u16), (159u16, 120u16)] {
            let mut state = TerminalScreenState::new(10, start);
            // Composer row painted uniformly gray (codex style: bg inherited across
            // an absolute move + trailing \e[K).
            let frame = format!(
                "\x1b[2J\x1b[H\x1b[8;1H{gray} \x1b[9;1H\x1b[1m\u{203a}\x1b[22m \x1b[2mFind and fix a bug\x1b[9;20H{gray}\x1b[K"
            );
            state.process(frame.as_bytes());
            assert!(row_is_uniform_gray(&state, 8, start), "precondition: composer row uniform gray at {start}");
            state.resize(10, end);
            let preserved = end.min(start);
            assert!(
                row_is_uniform_gray(&state, 8, preserved),
                "vt100 resize {start}->{end} must preserve the composer-row bg (no reflow drop)"
            );
        }
    }

    // Cold-re-resume vacuum guard (sum-total run #3): every PTY spawn gets a
    // unique runtime spawn id — the client compares the id a snapshot was read
    // from against the id its buffer was seeded from to detect a replaced
    // runtime. Uniqueness must hold across rapid consecutive spawns (counter
    // component) and ids must be non-zero (0 = the "unknown, fail open" value).
    #[test]
    fn runtime_spawn_ids_are_unique_and_nonzero() {
        let now = now_millis();
        let a = next_runtime_spawn_id(now);
        let b = next_runtime_spawn_id(now);
        let c = next_runtime_spawn_id(now);
        assert!(a != 0 && b != 0 && c != 0, "spawn ids must be non-zero");
        assert!(a != b && b != c && a != c, "same-millisecond spawns must still differ");
    }

    /// Wait until `probe` accepts the runtime's served screen, or fail.
    /// Returns the accepted snapshot so the caller can assert on it.
    fn wait_for_screen_snapshot(
        runtime: &PtySessionRuntime,
        what: &str,
        probe: impl Fn(&str) -> bool,
    ) -> String {
        for _ in 0..600 {
            let screen = runtime.screen_snapshot();
            if probe(&screen) {
                return screen;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what} on the served screen");
    }

    /// Let the child go quiet, so the phases below can assert that exactly ONE
    /// key input moved. Settles when `output_seq` holds still.
    fn settle_pty_output(runtime: &PtySessionRuntime) {
        for _ in 0..200 {
            let before = runtime.screen_snapshot_key().output_seq;
            std::thread::sleep(Duration::from_millis(25));
            if runtime.screen_snapshot_key().output_seq == before {
                return;
            }
        }
        panic!("pty never went quiet");
    }

    /// The memo may only serve a stored screen when EVERY input is unchanged —
    /// driven through the REAL `screen_snapshot()` / `screen_snapshot_key()` on
    /// a REAL pty, because a test that only compares hand-built keys is a
    /// tautology over `#[derive(PartialEq)]` and stays green while the key is
    /// broken.
    ///
    /// Each phase moves exactly one key input and asserts the SERVED SCREEN
    /// followed:
    ///
    /// - `output_seq`: a late paint from the child must reach the screen.
    /// - `resize_seq`: resized away and back to the same grid, every size is
    ///   back to its old value, but the narrow step dropped columns the model
    ///   never gets back — a size-only key would serve those dead cells.
    /// - `model_size` + `pty_cols`: the model drifts WIDER than its pty (the
    ///   live Round 21 shape: 168-col pty, 204-col model). Nothing else moves,
    ///   so `model_size` is the only signal the memo has, and the screen it
    ///   serves must be clipped to the pty width — serving the ghost columns
    ///   IS `docs/xterm-bugs.md#screen-model-wider-than-viewer`.
    /// - the `resize_screen_model_repaired` branch returns BEFORE
    ///   `resize_count.fetch_add`, asserted here against the real `resize()`
    ///   rather than trusted from a comment.
    #[test]
    fn screen_snapshot_memo_follows_output_resize_and_model_drift_on_a_real_pty() {
        let late_paint_gate = std::env::temp_dir().join(format!(
            "yggterm-screen-memo-gate-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let _ = std::fs::remove_file(&late_paint_gate);
        let runtime = PtySessionRuntime::spawn(
            "local://screen-memo",
            // The second paint is gated on a file the TEST creates, not on a
            // sleep: a timing gap would make "the late paint cannot be here
            // yet" a race under a loaded parallel suite, and a lock that can
            // flake is a lock nobody trusts.
            &format!(
                "printf 'BASELINE\\033[3;100HFARCOL'; \
                 while [ ! -f '{gate}' ]; do sleep 0.05; done; \
                 printf '\\033[7;1HLATEPAINT'; sleep 600",
                gate = late_paint_gate.display()
            ),
            None,
            Some((120, 24)),
        )
        .expect("spawn screen memo test runtime");

        // --- output_seq: the memo must not outlive the child's next paint. ---
        let early = wait_for_screen_snapshot(&runtime, "the first paint", |screen| {
            screen.contains("BASELINE") && screen.contains("FARCOL")
        });
        assert!(
            !early.contains("LATEPAINT"),
            "precondition: the late paint is gated and the gate is not open yet"
        );
        assert_eq!(
            early,
            runtime.screen_snapshot(),
            "an untouched session must serve the same screen twice (the memo hit)"
        );
        std::fs::write(&late_paint_gate, b"go").expect("open the late-paint gate");
        wait_for_screen_snapshot(&runtime, "the late paint", |screen| {
            screen.contains("LATEPAINT")
        });
        let _ = std::fs::remove_file(&late_paint_gate);
        settle_pty_output(&runtime);

        // --- resize_seq: away and back, with no output in between. ---
        let before_round_trip = runtime.screen_snapshot_key();
        let served_before_round_trip = runtime.screen_snapshot();
        assert!(
            served_before_round_trip.contains("FARCOL"),
            "precondition: column 100 is painted before the narrow step"
        );
        runtime.resize(60, 24).expect("narrow the pty");
        runtime.resize(120, 24).expect("widen the pty back");
        let after_round_trip = runtime.screen_snapshot_key();
        assert_eq!(
            after_round_trip.output_seq, before_round_trip.output_seq,
            "a resize round trip must produce no child output"
        );
        assert_eq!(
            after_round_trip.pty_cols, before_round_trip.pty_cols,
            "the pty is back to its old width"
        );
        assert_eq!(
            after_round_trip.model_size, before_round_trip.model_size,
            "the model is back to its old grid"
        );
        assert_ne!(
            after_round_trip.resize_seq, before_round_trip.resize_seq,
            "resize_seq is the ONLY input that sees a round trip"
        );
        let served_after_round_trip = runtime.screen_snapshot();
        assert!(
            served_after_round_trip.contains("BASELINE"),
            "column 1 survives a narrowing"
        );
        assert!(
            !served_after_round_trip.contains("FARCOL"),
            "the narrow step destroyed column 100; the memo must not serve it back"
        );

        // --- model_size + pty_cols: the model drifts wider than its pty. ---
        let before_drift = runtime.screen_snapshot_key();
        assert!(
            !runtime.screen_snapshot().contains("GHOSTCOL"),
            "precondition: nothing is painted past the pty width yet"
        );
        {
            let mut screen_state = runtime
                .screen_state
                .lock()
                .expect("pty screen state lock poisoned");
            screen_state.resize(24, 200);
            screen_state.process(b"\x1b[5;1HLEFTEDGE\x1b[5;150HGHOSTCOL");
        }
        let after_drift = runtime.screen_snapshot_key();
        assert_eq!(
            after_drift.output_seq, before_drift.output_seq,
            "the drift is a model-only mutation"
        );
        assert_eq!(
            after_drift.resize_seq, before_drift.resize_seq,
            "the drift never went through resize()"
        );
        assert_eq!(
            after_drift.pty_cols, before_drift.pty_cols,
            "the pty did not move"
        );
        assert_ne!(
            after_drift.model_size, before_drift.model_size,
            "model_size is the ONLY input that sees a model-only mutation"
        );
        let drifted = runtime.screen_snapshot();
        assert!(
            drifted.contains("LEFTEDGE"),
            "the memo must re-render when only the model moved: {drifted:?}"
        );
        assert!(
            !drifted.contains("GHOSTCOL"),
            "the served screen must be clipped to the PTY width, not the model's"
        );
        // Measured against the CLIPPED text's own grid, which is now the PTY's:
        // once the ghosts past column 120 are gone, nothing in it wraps at 120
        // either, so this reads the same number a viewer would.
        assert!(
            formatted_screen_max_column(&drifted, 120) <= 120,
            "clipped screen still paints past the pty: {}",
            formatted_screen_max_column(&drifted, 120)
        );

        // --- the repair branch never touches resize_seq. ---
        let before_repair = runtime.screen_snapshot_key();
        runtime
            .resize(120, 24)
            .expect("resize to the size the pty already is");
        let after_repair = runtime.screen_snapshot_key();
        assert_eq!(
            after_repair.resize_seq, before_repair.resize_seq,
            "resize_screen_model_repaired returns before resize_count.fetch_add — \
             model_size is the only key input that can see that branch"
        );
        assert_eq!(after_repair.output_seq, before_repair.output_seq);
        assert_eq!(after_repair.pty_cols, before_repair.pty_cols);
        assert_eq!(
            after_repair.model_size,
            Some((24, 120)),
            "the repair narrowed the model back onto its pty"
        );
        assert_ne!(after_repair.model_size, before_repair.model_size);
        let repaired = runtime.screen_snapshot();
        assert!(repaired.contains("LEFTEDGE"));
        assert!(!repaired.contains("GHOSTCOL"));

        runtime.shutdown(None).expect("shutdown test runtime");
    }

    /// ⛔ THE WHOLE CHAIN, ON A REAL PTY, THROUGH THE CALL THE DAEMON MAKES.
    ///
    /// The fixture test above proves bytes -> grid -> classifier. This one
    /// proves the part a unit test on a hand-built screen cannot: that
    /// `TerminalManager::session_screen_plain_rows` — the exact method the
    /// gate-screen reading calls — returns the rendered grid for a session that
    /// really exists, with a real child process painting a real terminal.
    ///
    /// Worth its seconds because the mapping between "the model can render" and
    /// "the daemon serves it" is precisely where this repository has lost
    /// working code before: a handler that existed, compiled and passed its own
    /// tests while nothing could reach it.
    #[test]
    fn the_manager_serves_a_rendered_grid_for_a_live_pty() {
        let key = "local://screen-grid-test";
        let mut manager = TerminalManager::new();
        // Paint with ABSOLUTE CURSOR MOVES and no newlines between the rows —
        // the drawing grammar that makes a modal illegible on the raw stream —
        // then hold the pty open so the screen can be read.
        manager
            .ensure_session(
                key,
                "bash -lc 'printf \"\\033[2J\\033[3;2HQuick safety check: is this a project you trust?\
                 \\033[5;2H1. Yes, I trust this folder\\033[7;2H2. No, exit\"; sleep 30'",
                None,
            )
            .expect("spawn a pty for the grid test");
        // ⛔ WAIT FOR CONTENT, THEN FOR QUIET. `settle_pty_output` alone returns
        // instantly here: it asks whether output STOPPED changing, and before the
        // child has written its first byte the answer is trivially yes. A quiet
        // screen and a not-yet-started one are indistinguishable to it.
        let mut rows = Vec::new();
        for _ in 0..200 {
            rows = manager.session_screen_plain_rows(key).unwrap_or_default();
            if rows
                .iter()
                .any(|row| row.to_ascii_lowercase().contains("quick safety check"))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let runtime = manager
            .sessions
            .get(key)
            .expect("the manager kept the session it just spawned");
        settle_pty_output(runtime);
        let rows = manager
            .session_screen_plain_rows(key)
            .expect("a live session must have a readable grid");
        let _ = &rows;
        let populated: Vec<&String> = rows.iter().filter(|row| !row.trim().is_empty()).collect();
        assert!(
            populated.len() >= 3,
            "the three painted rows must arrive as three rows, got {populated:?}",
        );
        let joined = rows.join("\n");
        assert!(
            joined.to_ascii_lowercase().contains("quick safety check"),
            "the grid must carry the words a person can see: {joined:?}",
        );
        assert_eq!(
            yggterm_core::screen_state::classify_screen(Some(&joined)),
            yggterm_core::screen_state::RowScreenState::StartupGate,
            "the daemon's own call must name the state",
        );

        // ⛔ And the raw snapshot must NOT be legible, or the grid is buying
        // nothing and this whole change is ceremony.
        let raw = manager
            .session_screen_snapshot(key)
            .expect("a live session must have a raw screen too");
        assert!(
            raw.lines().count() < populated.len(),
            "the raw stream must fuse rows the grid recovers: {} raw lines vs {} \
             populated rows",
            raw.lines().count(),
            populated.len(),
        );

        manager.remove_session(key, None).ok();
    }

    #[test]
    fn missing_session_reports_spawn_id_zero() {
        let manager = TerminalManager::new();
        assert_eq!(
            manager.session_runtime_spawn_id("local://nope"),
            0,
            "no runtime => spawn id 0 (client guard fails open)"
        );
    }

    /// ⛔⛔ THE RAW SCREEN DOES NOT CONTAIN THE WORDS ON IT, AND THE GRID DOES.
    ///
    /// Driven with the real bytes of a first-run gate (captured 2026-08-21,
    /// identifying path replaced). The CLI paints its nine visible rows with
    /// absolute cursor moves and emits single spaces as cursor-forward, so on
    /// the raw stream:
    ///
    /// * the whole modal arrives as TWO `\n`-delimited lines, one of them ~870
    ///   characters long — so "the last N lines" is not a window over the
    ///   display, and "these two phrases on the same line" is meaningless;
    /// * `quick safety check` is not present as a substring AT ALL, because the
    ///   spaces between its words are escape sequences.
    ///
    /// Every screen classifier read `false` on the live row for exactly this
    /// reason. Feeding them the rendered grid is the fix, and this test fails if
    /// anyone points them back at the snapshot.
    #[test]
    fn a_gate_is_illegible_on_the_raw_screen_and_legible_on_the_grid() {
        let mut state = TerminalScreenState::new(24, 120);
        state.process(include_bytes!("../tests/fixtures/startup-gate-screen.bin"));

        let raw = state.formatted.trim_matches('\0').to_string();
        assert!(
            !raw.to_ascii_lowercase().contains("quick safety check"),
            "the raw stream must genuinely lack the phrase, or this test proves nothing",
        );
        assert!(
            !yggterm_core::screen_text_shows_agent_startup_gate(&raw),
            "the classifier must genuinely miss on the raw stream",
        );

        let grid = state.vt_screen_plain_rows();
        let joined = grid.join("\n");
        assert!(
            joined.to_ascii_lowercase().contains("quick safety check"),
            "the rendered grid must carry the words a person can see",
        );
        assert!(
            yggterm_core::screen_text_shows_agent_startup_gate(&joined),
            "the classifier must name the gate once it reads real rows",
        );
        assert_eq!(
            yggterm_core::screen_state::classify_screen(Some(&joined)),
            yggterm_core::screen_state::RowScreenState::StartupGate,
        );

        // The shape itself: many more visible rows than the raw stream has
        // lines, which is the whole reason a line-based window was measuring
        // something other than the screen.
        let populated = grid.iter().filter(|row| !row.trim().is_empty()).count();
        assert!(
            populated >= 5,
            "expected the modal's rows to survive rendering, got {populated}",
        );
        assert!(
            populated > raw.lines().count(),
            "the grid must recover rows the raw stream fused: {} populated rows \
             vs {} raw lines",
            populated,
            raw.lines().count(),
        );
    }

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
    fn viewport_reconcile_replay_restores_daemon_screen_and_cursor_on_desynced_client() {
        // Daemon sees the FULL stream: absolute positioning plus the
        // relative-cursor frame style Claude Code paints with.
        let full_stream = "\x1b[2J\x1b[H\
line-one on the real screen\r\n\
line-two on the real screen\r\n\
\x1b[5;1Hstatus row painted absolutely\
\x1b[1;10H\x1b[K\r\x1b[1Bmid-frame relative move\x1b[K";
        let mut daemon = TerminalScreenState::new(24, 80);
        daemon.process(full_stream.as_bytes());

        // Client attaches from a budget-truncated MID-STREAM tail (the raw
        // retained-chunk seed): relative moves replay against the wrong
        // origin — the pre-fix persistent hole/interleave garble.
        let tail_start = full_stream.find("\x1b[1;10H").expect("tail marker");
        let mut client = Vt100Parser::new(24, 80, 0);
        client.process(full_stream[tail_start..].as_bytes());
        assert_ne!(
            client.screen().contents(),
            daemon.parser.screen().contents(),
            "a mid-stream tail replay must actually desync the client for this test to prove anything"
        );

        // The appended reconcile payload must pin the client viewport AND
        // cursor to the daemon's authoritative screen.
        let payload = daemon
            .viewport_reconcile_replay()
            .expect("reconcile payload for a non-empty screen");
        client.process(payload.as_bytes());
        assert_eq!(
            client.screen().contents(),
            daemon.parser.screen().contents(),
            "reconcile must repaint the client viewport to daemon truth"
        );
        assert_eq!(
            client.screen().cursor_position(),
            daemon.parser.screen().cursor_position(),
            "reconcile must restore the daemon cursor so subsequent relative diffs anchor correctly"
        );
    }

    #[test]
    fn viewport_reconcile_replay_is_none_for_blank_screen() {
        let state = TerminalScreenState::new(24, 80);
        assert!(state.viewport_reconcile_replay().is_none());
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
        let profile = test_protocol_profile(
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
            test_protocol_profile("export COLORFGBG='15;0'; codex");
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
        let profile = test_protocol_profile(
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
        let profile = test_protocol_profile(launch_command);
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
            test_protocol_profile("export COLORFGBG='15;0'; codex");
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
            test_protocol_profile("export COLORFGBG='15;0'; codex");
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
            test_protocol_profile("export COLORFGBG='15;0'; codex");
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

    // End-to-end through a REAL pty: an app writes its OSC 7717 declare to its
    // own stdout, and the daemon — with no GUI, no xterm, no client of any kind
    // in the picture — retains it. That is the whole point: this is the state a
    // never-revealed session is in, and it is what `web ensure` rebuilds from.
    #[test]
    fn pty_runtime_retains_an_app_declare_with_no_client_attached() {
        let payload = base64::engine::general_purpose::STANDARD.encode(
            serde_json::json!({"session": "probe", "url": "https://example.test/ingested"})
                .to_string()
                .as_bytes(),
        );
        let runtime = PtySessionRuntime::spawn(
            "local://declare-ingest",
            &format!("printf '\\033]7717;web-surface;open;{payload}\\007'; sleep 5"),
            None,
            None,
        )
        .expect("spawn declare test runtime");

        let mut records = Vec::new();
        for _ in 0..80 {
            records = runtime.app_declares();
            if !records.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let screen = runtime.screen_snapshot();
        runtime.shutdown(None).expect("shutdown test runtime");

        assert_eq!(records.len(), 1, "expected one retained declare");
        assert_eq!(records[0].verb, "web-surface");
        assert_eq!(records[0].action, "open");
        assert_eq!(records[0].payload["url"], "https://example.test/ingested");
        assert!(
            !screen.contains("7717"),
            "the OSC must stay invisible on the screen: {screen:?}"
        );
    }

    #[test]
    fn pty_runtime_answers_default_color_query_to_child() {
        // ⛔ THE LAUNCH COMMAND MUST STATE ITS OWN COLOURS. The daemon answers
        // OSC-10 from `from_launch_command`, and when the command carries no
        // explicit `YGGTERM_TERMINAL_COLOR_*` that resolver falls through to
        // `terminal_identity_color_profile_from_environment()` — a read of the
        // PROCESS env. The `expected` here read the same env. Standalone the two
        // reads agreed, but under `cargo test --workspace` a concurrent test
        // mutates those env vars (`env::set_var`/`remove_var` are process-global
        // in Rust), so between computing `expected` and spawning the runtime the
        // foreground flipped from the dark profile's #e5e5e5 to the built-in base
        // #cccccc — a WRONG colour, not a late one, which is why raising the read
        // deadline did nothing. Carrying the full profile in the command makes
        // `terminal_identity_color_profile_from_launch_command` resolve it and
        // short-circuit the env entirely, so both sides are deterministic
        // regardless of what any neighbour does to the environment.
        let color_exports = "\
            export YGGTERM_TERMINAL_APPEARANCE=dark; \
            export YGGTERM_TERMINAL_COLOR_FOREGROUND='#e5e5e5'; \
            export YGGTERM_TERMINAL_COLOR_BACKGROUND='#262a33'; \
            export YGGTERM_TERMINAL_COLOR_0='#000000'; \
            export YGGTERM_TERMINAL_COLOR_1='#cd3131'; \
            export YGGTERM_TERMINAL_COLOR_2='#05bc79'; \
            export YGGTERM_TERMINAL_COLOR_3='#e5e512'; \
            export YGGTERM_TERMINAL_COLOR_4='#2472c8'; \
            export YGGTERM_TERMINAL_COLOR_5='#bc3fbc'; \
            export YGGTERM_TERMINAL_COLOR_6='#0fa8cd'; \
            export YGGTERM_TERMINAL_COLOR_7='#e5e5e5'; \
            export YGGTERM_TERMINAL_COLOR_8='#666666'; \
            export YGGTERM_TERMINAL_COLOR_9='#cd3131'; \
            export YGGTERM_TERMINAL_COLOR_10='#05bc79'; \
            export YGGTERM_TERMINAL_COLOR_11='#e5e512'; \
            export YGGTERM_TERMINAL_COLOR_12='#2472c8'; \
            export YGGTERM_TERMINAL_COLOR_13='#bc3fbc'; \
            export YGGTERM_TERMINAL_COLOR_14='#0fa8cd'; \
            export YGGTERM_TERMINAL_COLOR_15='#e5e5e5'; ";
        // host_profile None: the command carries the colours, so no ambient read.
        let (r, g, b) =
            TerminalProtocolProfile::from_launch_command_with_host_profile(color_exports, None)
                .foreground;
        let expected_response =
            format!("\u{1b}]10;rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\u{1b}\\");
        let expected_hex: String = expected_response
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let launch_command = format!(
            "{color_exports}export YGG_EXPECT_FG_HEX={expected_hex}; {OSC_COLOR_QUERY_CHILD}"
        );
        let runtime = PtySessionRuntime::spawn(
            "local://osc-color-query",
            &launch_command,
            None,
            None,
        )
        .expect("spawn OSC color query test runtime");
        // Wait for the child to print its verdict. Its OSC read has its own
        // deadline; this ceiling only has to outlast it, so the child's own
        // COLOR_BAD is what fails the assert, not a premature give-up here.
        // Breaks the instant "COLOR_" appears — free on the happy path. The read
        // is cumulative (re-collects from offset 0 every pass), so a slow child
        // is a late verdict, never a lost byte.
        let mut combined = String::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
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
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''",
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
    fn codex_composer_input_row_is_not_a_disposable_attach_suffix() {
        // XTERM-BUG content-clip-on-reveal (campaign #1): the live codex composer /
        // input row (a `›` line, even showing a rotating placeholder) must NOT be
        // trimmed as disposable attach suffix — it's the row the user types into,
        // and wrapper-vs-manual parity requires showing it. Pre-fix this FAILED:
        // terminal_chunk_mentions_generic_attach_prompt matched the placeholder
        // "Summarize recent commits" and marked the composer disposable.
        let placeholder_composer = "\x1b[60;1H\x1b[39;48;2;64;67;75m \x1b[K\x1b[61;1H\x1b[1m\u{203a}\x1b[22m \x1b[2mSummarize recent commits\x1b[22m\x1b[K";
        assert!(
            !terminal_chunk_is_disposable_initial_attach_suffix(placeholder_composer),
            "live codex composer with a placeholder must be preserved on attach"
        );
        let typed_composer = "\x1b[61;1H\x1b[1m\u{203a}\x1b[22m fix the flaky integration test\x1b[K";
        assert!(
            !terminal_chunk_is_disposable_initial_attach_suffix(typed_composer),
            "a user-typed composer must be preserved on attach"
        );
        // No regression: genuinely low-signal trailing chrome is still disposable.
        assert!(terminal_chunk_is_disposable_initial_attach_suffix("   "));
        assert!(terminal_chunk_is_disposable_initial_attach_suffix(
            "\x1b[K\x1b[?25l"
        ));
    }

    #[test]
    fn initial_attach_replay_keeps_codex_composer_off_the_trim() {
        // End-to-end: a meaningful transcript anchor followed by the live composer
        // as a trailing chunk -> the composer survives select_initial_attach_chunks
        // (pre-fix it was popped by trim_initial_attach_low_signal_suffix).
        let mut chunks = VecDeque::new();
        chunks.push_back(TerminalChunk {
            seq: 1,
            data: "Implemented and pushed the change.\r\nWhat changed:\r\n- Added the new selector\r\n- Updated the test suite\r\nValidation: all checks passed.\r\n".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 2,
            data: "\x1b[60;1H\x1b[39;48;2;64;67;75m \x1b[K\x1b[61;1H\x1b[1m\u{203a}\x1b[22m \x1b[2mSummarize recent commits\x1b[22m\x1b[K".to_string(),
        });
        let selected = select_initial_attach_chunks(&chunks);
        let joined = selected
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();
        assert!(
            joined.contains('\u{203a}'),
            "composer input row must survive the attach trim, got: {joined:?}"
        );
        assert!(
            joined.contains("Summarize recent commits"),
            "composer placeholder must survive the attach trim"
        );
    }

    // The served attach seed never carries a consumed web-surface `open` — even
    // when the PTY reader split the declare across chunk boundaries. EVERY
    // split point is exercised (the scanner's own split test discipline): a
    // per-chunk replace would miss the straddles, and a straddled replay is
    // exactly the shape a reviewer can construct on demand. The chunk skeleton
    // (count, seqs, per-chunk byte lengths) must come through untouched, since
    // downstream consumers filter whole chunks by seq.
    #[test]
    fn a_replayed_open_is_neutralized_across_every_chunk_split() {
        let payload = base64::engine::general_purpose::STANDARD
            .encode(br#"{"session":"s","url":"https://app.example/start"}"#);
        let stream = format!(
            "MOCK_READY\r\n\x1b]7717;web-surface;open;{payload}\x07after the declare\r\n"
        );
        for cut in 1..stream.len() {
            if !stream.is_char_boundary(cut) {
                continue;
            }
            let mut chunks = vec![
                TerminalChunk {
                    seq: 7,
                    data: stream[..cut].to_string(),
                },
                TerminalChunk {
                    seq: 8,
                    data: stream[cut..].to_string(),
                },
            ];
            let lens: Vec<usize> = chunks.iter().map(|chunk| chunk.data.len()).collect();
            let rewritten = neutralize_replayed_web_surface_opens(&mut chunks);
            assert_eq!(rewritten, 1, "cut {cut} missed the straddled open");
            let joined: String = chunks.iter().map(|chunk| chunk.data.as_str()).collect();
            assert!(
                !joined.contains("\x1b]7717;web-surface;open;"),
                "cut {cut} served the consumed open verbatim"
            );
            assert!(
                joined.contains("\x1b]7717;web-surface;seen;"),
                "cut {cut} lost the declare instead of neutralizing it"
            );
            assert!(
                joined.contains(&payload) && joined.contains("after the declare"),
                "cut {cut} disturbed bytes outside the action token"
            );
            assert_eq!(
                chunks.iter().map(|chunk| chunk.data.len()).collect::<Vec<_>>(),
                lens,
                "cut {cut} moved a chunk boundary"
            );
            assert_eq!(
                chunks.iter().map(|chunk| chunk.seq).collect::<Vec<_>>(),
                vec![7, 8],
                "cut {cut} renumbered the chunks"
            );
        }

        // And a seed with nothing to neutralize is left byte-identical.
        let mut chunks = vec![TerminalChunk {
            seq: 1,
            data: "plain output\r\n".to_string(),
        }];
        assert_eq!(neutralize_replayed_web_surface_opens(&mut chunks), 0);
        assert_eq!(chunks[0].data, "plain output\r\n");
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

    // Boring retained reveal (spec-boring-session-loads lane 1): a resumed read
    // from a live cursor must deliver ONLY the contiguous delta (seq > cursor),
    // never re-deliver consumed chunks — the client APPENDS the result into an
    // already-painted buffer, so any re-delivery would double-paint on reveal.
    #[test]
    fn pty_read_from_live_cursor_returns_only_the_unconsumed_delta() {
        let runtime = PtySessionRuntime::spawn(
            "local://cursor-resume-delta-test",
            "bash -lc 'printf phase-one; sleep 0.5; printf phase-two; sleep 2'",
            None,
            None,
        )
        .expect("spawn cursor resume test runtime");
        let mut first_cursor = 0_u64;
        let mut first_data = String::new();
        for _ in 0..80 {
            let read = runtime.read(0);
            first_data = read
                .chunks
                .iter()
                .map(|chunk| chunk.data.as_str())
                .collect::<String>();
            first_cursor = read.cursor;
            if first_data.contains("phase-one") {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(first_data.contains("phase-one"), "{first_data:?}");
        let mut resumed_chunks = Vec::new();
        let mut resumed_data = String::new();
        for _ in 0..120 {
            let read = runtime.read(first_cursor);
            resumed_data = read
                .chunks
                .iter()
                .map(|chunk| chunk.data.as_str())
                .collect::<String>();
            resumed_chunks = read.chunks.clone();
            assert!(!read.resync_required, "no trim happened in this tiny stream");
            if resumed_data.contains("phase-two") {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        runtime.shutdown(None).expect("shutdown test runtime");
        assert!(resumed_data.contains("phase-two"), "{resumed_data:?}");
        assert!(
            !resumed_data.contains("phase-one"),
            "resumed read must not re-deliver consumed chunks: {resumed_data:?}"
        );
        for chunk in &resumed_chunks {
            assert!(
                chunk.seq > first_cursor,
                "resumed chunk seq {} must be past the consumed cursor {first_cursor}",
                chunk.seq
            );
        }
    }

    // Live-path variant of the 2.10.4 attach-seed fix: when the ring trims
    // the contiguous middle past a resuming client's cursor, the read must
    // end with the viewport reconcile so the client base is re-anchored to
    // daemon truth — returning the bare discontiguous tail is what left the
    // permanent character-interleave corruption on busy CC sessions.
    #[test]
    fn pty_read_with_trimmed_middle_appends_viewport_reconcile_after_tail() {
        let runtime = PtySessionRuntime::spawn(
            "local://gap-resync-test",
            "bash -lc 'printf base-frame; sleep 2'",
            None,
            None,
        )
        .expect("spawn gap resync test runtime");
        let mut first_cursor = 0_u64;
        for _ in 0..80 {
            let read = runtime.read(0);
            first_cursor = read.cursor;
            if read
                .chunks
                .iter()
                .any(|chunk| chunk.data.contains("base-frame"))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(first_cursor > 0, "runtime produced no output");
        // Simulate the high-throughput ring trim: drop everything the client
        // has not consumed yet and append a tail chunk far past its cursor.
        let tail_seq = first_cursor + 50;
        {
            let mut chunks = runtime.chunks.lock().expect("chunk lock");
            chunks.clear();
            chunks.push_back(TerminalChunk {
                seq: tail_seq,
                data: "\r\x1b[2Btail-after-gap\x1b[K".to_string(),
            });
        }
        runtime.seq.store(tail_seq, Ordering::SeqCst);
        let read = runtime.read(first_cursor);
        runtime.shutdown(None).expect("shutdown test runtime");
        assert!(
            read.resync_required,
            "a trimmed contiguous middle must signal resync"
        );
        let last = read.chunks.last().expect("chunks must not be empty");
        assert!(
            last.data.starts_with("\x1b[?25l\x1b[2J\x1b[H"),
            "the read must END with the viewport reconcile payload, got {:?}",
            last.data
        );
        assert!(
            read.chunks
                .iter()
                .any(|chunk| chunk.data.contains("tail-after-gap")),
            "the surviving tail must still be delivered for scrollback"
        );
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

        // content-clip-on-reveal (campaign #1): the live composer
        // "›Explain this codebase" is the input row and MUST be preserved — the
        // suffix trim now stops at it instead of dropping it (previously this
        // locked seqs==[1], hiding the composer = the broken-bottom reveal).
        assert!(seqs.contains(&5));
        assert!(combined.contains("origin/main updated successfully"));
        assert!(combined.contains("Explain this codebase"));
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

        // content-clip-on-reveal (campaign #1): the live composer
        // "› Write tests for @filename" is the input row and MUST be preserved —
        // this assertion was previously inverted (it locked the trim that produced
        // the broken-bottom reveal). The trailing model "% left" footer fragment is
        // still dropped as low-signal chrome.
        assert!(combined.contains("origin/main updated successfully"));
        assert!(combined.contains("Write tests for @filename"));
        assert!(!combined.contains("100% left"));
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
        let launch_command = "ssh -tt guihost 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''019ce5d8-c94c-7b62-ae19-3818ae400b65'\\'' '\\''/home/user'\\'''";
        let start_command = "ssh -tt guihost 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''start-codex'\\'' '\\''019ce5d8-c94c-7b62-ae19-3818ae400b65'\\'' '\\''/home/user'\\'''";

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
        let launch_command = "__yggterm_initial_tty_size=$(stty size 2>/dev/null || true); unset __yggterm_initial_tty_size; exec ssh -tt guihost 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''";

        let wrapped = remote_resume_attach_shell_command(launch_command);

        assert!(wrapped.starts_with(
            "stty raw -echo opost onlcr </dev/tty >/dev/tty 2>/dev/null || true; __yggterm_initial_tty_size="
        ));
        assert!(!wrapped.contains("; exec __yggterm_initial_tty_size="));
        assert!(wrapped.contains("; exec ssh -tt guihost"));
        assert!(wrapped.contains("'resume-codex'"));
    }

    #[test]
    fn remote_resume_attach_shell_command_execs_plain_ssh_command() {
        let launch_command = "ssh -tt guihost 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''";

        let wrapped = remote_resume_attach_shell_command(launch_command);

        assert!(wrapped.starts_with(
            "stty raw -echo opost onlcr </dev/tty >/dev/tty 2>/dev/null || true; exec ssh -tt guihost"
        ));
    }

    #[test]
    fn runtime_owned_terminal_keys_prefer_initial_screen_snapshot() {
        assert!(terminal_key_prefers_initial_screen_snapshot(
            "remote-session://guihost/test",
            "bash -lc 'sleep 30'",
        ));
        assert!(terminal_key_prefers_initial_screen_snapshot(
            "codex-runtime://test",
            "bash -lc 'sleep 30'",
        ));
        assert!(terminal_key_prefers_initial_screen_snapshot(
            "local://legacy-resume",
            "ssh -tt guihost 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''",
        ));
        assert!(terminal_key_prefers_initial_screen_snapshot(
            "local://fresh-start",
            "ssh -tt guihost 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''start-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''",
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
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''",
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

    /// Input and output must be timed SEPARATELY, or a wedged row reads as busy.
    ///
    /// `last_activity_ms` is stamped by the writer as well as the reader, so a
    /// row that has stopped reading its PTY looks MAXIMALLY ACTIVE for exactly as
    /// long as a human keeps typing into it. That is not hypothetical: a wedged
    /// agent row was listed `recently_active` by the hot-restart gate — and so
    /// blocked a deploy — while being completely unusable, and the owner's own
    /// keystrokes were what kept it looking alive.
    ///
    /// ⛔ Asserts BOTH halves: a row that answers is not suspected, and a row
    /// written to that stays silent is — otherwise a detector wired to a
    /// constant would pass.
    #[test]
    fn input_that_goes_unanswered_is_visible_without_typing_a_marker() {
        let mut manager = TerminalManager::new();
        let key = "wedge-signal-probe";
        manager
            .restart_session(key, "sh -lc 'sleep 30'", None, None)
            .expect("spawn wedge-signal session");
        manager
            .seed_session(key, "boot banner\n")
            .expect("seed initial output");

        // Half 1 — HEALTHY. Output is at least as recent as input, so nothing
        // is outstanding. Reporting a gap here would make every live row look
        // wedged.
        assert_eq!(manager.input_unanswered_ms(key), None);
        assert!(!manager.wedge_suspected(key, Duration::from_millis(0)));

        // ⭐ THE INVARIANT THE SPLIT EXISTS FOR: a write must move the INPUT
        // clock and must NOT move the output clock. While both lived in one
        // field, typing into a deaf row refreshed the very timestamp used to
        // decide the row was alive.
        let output_before_write = {
            let session = manager.sessions.get(key).expect("session held here");
            session.last_output_ms.load(Ordering::SeqCst)
        };
        manager.write(key, "x").expect("write to the session");
        let (activity_after, output_after) = {
            let session = manager.sessions.get(key).expect("session held here");
            (
                session.last_activity_ms.load(Ordering::SeqCst),
                session.last_output_ms.load(Ordering::SeqCst),
            )
        };
        assert_eq!(
            output_after, output_before_write,
            "a write must not stamp the OUTPUT clock — that conflation is the \
             whole defect: it makes a row that says nothing back look busy"
        );
        assert!(
            activity_after >= output_after,
            "a write must stamp the input clock"
        );

        // Half 2 — WEDGED. Drive the runtime's own clocks to the shape a deaf
        // row has: written to AFTER its last output. This is the state the
        // single conflated timestamp cannot represent at all.
        {
            let session = manager
                .sessions
                .get(key)
                .expect("session is held by this manager");
            let output_at = session.last_output_ms.load(Ordering::SeqCst);
            session
                .last_activity_ms
                .store(output_at + 5_000, Ordering::SeqCst);
        }
        assert_eq!(
            manager.input_unanswered_ms(key),
            Some(5_000),
            "input newer than output IS the wedge signal, and its size is how \
             long the row has been deaf"
        );
        assert!(
            manager.wedge_suspected(key, Duration::from_millis(2_000)),
            "5s of unanswered input must trip a 2s threshold"
        );
        assert!(
            !manager.wedge_suspected(key, Duration::from_secs(30)),
            "and must NOT trip a 30s one — a threshold that always fires is not \
             a detector"
        );

        manager
            .remove_session(key, None)
            .expect("shutdown wedge-signal probe");
    }

    /// Being typed AT is not being in use — the gate must read OUTPUT idle.
    ///
    /// This is the exact state that jammed a deploy: the row had gone deaf, the
    /// owner kept typing into it, every keystroke refreshed the conflated
    /// activity field, and the hot-restart gate read that field and reported the
    /// unusable session as `recently_active`. The one thing that would have
    /// cleared the wedge was blocked by the wedge's own symptom.
    ///
    /// ⛔ Asserts BOTH clocks from the SAME state, so a reader wired to either
    /// field alone cannot pass: activity says "busy 0 ms ago", output says
    /// "silent for 5 s".
    #[test]
    fn a_row_being_typed_at_is_idle_by_output_even_though_activity_says_busy() {
        let mut manager = TerminalManager::new();
        let key = "output-idle-gate-probe";
        manager
            .restart_session(key, "sh -lc 'sleep 30'", None, None)
            .expect("spawn output-idle session");
        manager
            .seed_session(key, "last thing it ever said\n")
            .expect("seed the final output");

        // The deaf row's shape: written to now, silent for five seconds.
        {
            let session = manager.sessions.get(key).expect("session held here");
            let output_at = session.last_output_ms.load(Ordering::SeqCst);
            session
                .last_output_ms
                .store(output_at.saturating_sub(5_000), Ordering::SeqCst);
            session
                .last_activity_ms
                .store(now_millis(), Ordering::SeqCst);
        }

        let activity_idle = manager
            .session_idle_for_ms(key)
            .expect("activity idle is readable");
        let output_idle = manager
            .session_output_idle_for_ms(key)
            .expect("output idle is readable");

        assert!(
            activity_idle < 1_000,
            "the conflated field reports the row as just-active, because the \
             keystrokes stamped it — this is the reading that jammed the gate"
        );
        assert!(
            output_idle >= 5_000,
            "output idle must show the row has said nothing for 5s: {output_idle}"
        );
        assert!(
            output_idle > activity_idle,
            "the two clocks must disagree in this state, or the gate cannot tell \
             a deaf row from a busy one"
        );

        manager
            .remove_session(key, None)
            .expect("shutdown output-idle probe");
    }

    /// A restart must say whether it SHUT ANYTHING DOWN.
    ///
    /// `sessions.remove(key)` answers `None` for a key this manager does not
    /// hold — an orphaned key, or a `remote-*` row whose runtime belongs to the
    /// daemon on its own host. The restart then shuts nothing down and spawns a
    /// replacement anyway, leaving the process that was serving that key alive
    /// and orphaned beside its successor.
    ///
    /// Observed live: a wedged agent row was told to restart, the reply said
    /// `restarted …`, and the wedged CLI kept its PTY and its wedge. The remedy
    /// that `input-check` recommends for a wedge could not clear one.
    ///
    /// ⛔ Asserts BOTH halves, so it cannot pass by always reporting one value.
    #[test]
    fn a_restart_reports_whether_it_replaced_anything() {
        let mut manager = TerminalManager::new();
        let key = "restart-outcome-probe";

        // Half 1: nothing under this key — the restart replaces NOTHING, and
        // must say so rather than reporting a restart it did not perform.
        let fresh = manager
            .restart_session(key, "sh -lc 'sleep 30'", None, None)
            .expect("restart with no prior runtime still spawns");
        assert!(
            !fresh.replaced_existing,
            "a restart that found no runtime under its key shut nothing down; \
             reporting it as a restart is what let a wedged row survive its own \
             remedy"
        );

        // Half 2: now a runtime IS held under the key, so the restart genuinely
        // replaces it. Without this half the test would pass on a constant false.
        let replaced = manager
            .restart_session(key, "sh -lc 'sleep 30'", None, None)
            .expect("restart over a live runtime");
        assert!(
            replaced.replaced_existing,
            "a restart over a runtime this manager holds must report that it \
             shut the old one down"
        );

        manager
            .remove_session(key, None)
            .expect("shutdown restart-outcome probe");
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

    /// End-to-end proof, on a real PTY, that `remove_session` returning `true`
    /// is not evidence that the session's processes are gone — and that the
    /// census-plus-liveness pair the removal verb reports with catches it.
    ///
    /// `PtySessionRuntime::shutdown` signals only the DIRECT PTY child, so a
    /// process that child forked and that ignores the hangup outlives the
    /// teardown. That is the reported incident in miniature: the terminal
    /// "closed", the app under it kept running, and the caller was told the
    /// work session had been removed.
    ///
    /// If the teardown ever grows a process-tree kill, this lock goes red —
    /// deliberately. That would be a change in what `session remove` DOES to a
    /// user's shell, and it must be decided, not absorbed.
    #[test]
    fn a_process_the_pty_child_forked_outlives_remove_session_and_the_census_says_so() {
        use yggterm_core::render_probe::{observe_process_tree_stats, process_still_running};

        let mut manager = TerminalManager::new();
        let key = "local://teardown-census";
        // `trap "" HUP` sets SIGHUP to SIG_IGN, and an ignored disposition
        // survives both fork and exec — so the forked child keeps running when
        // the PTY master closes, exactly like a backgrounded app does.
        manager
            .ensure_session(key, "sh -c 'trap \"\" HUP; sleep 30 & wait'", None)
            .expect("spawn a runtime that forks");
        let pty_pid = manager
            .session_process_id(key)
            .expect("a running runtime reports its PTY child") as i32;

        // Wait for the forked worker by NAME. Waiting for "more than one
        // process" catches whatever transient the shell startup happens to be
        // running and then asserts against something already exiting.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut census = Vec::new();
        let mut forked = None;
        while Instant::now() < deadline {
            census = observe_process_tree_stats(pty_pid);
            forked = census
                .iter()
                .find(|stat| stat.pid != pty_pid && stat.comm == "sleep")
                .cloned();
            if forked.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let forked =
            forked.expect("the PTY child must fork the worker for this lock to mean anything");

        assert!(
            manager.remove_session(key, None).expect("remove session"),
            "the runtime was present, so the removal reports true — which is the \
             claim under examination, not the evidence"
        );

        let survivors = census
            .iter()
            .filter(|stat| process_still_running(stat.pid, &stat.comm))
            .collect::<Vec<_>>();
        assert!(
            survivors.iter().any(|stat| stat.pid == forked.pid),
            "the forked process should have outlived the teardown: census {census:?}"
        );
        assert!(
            !survivors.iter().any(|stat| stat.pid == pty_pid),
            "the PTY child itself must be gone: census {census:?}"
        );

        // SAFETY: `forked` is a process this test spawned and just proved is
        // still running; a failed kill (already reaped) is ignored.
        #[cfg(unix)]
        unsafe {
            libc::kill(forked.pid as libc::pid_t, libc::SIGKILL);
        }
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

    /// Collect everything the session has produced since `cursor`, waiting up
    /// to `budget` for it to appear. Returns as soon as anything arrives.
    #[cfg(target_os = "linux")]
    fn drain_terminal(
        manager: &TerminalManager,
        key: &str,
        cursor: &mut u64,
        budget: Duration,
    ) -> String {
        let deadline = Instant::now() + budget;
        let mut text = String::new();
        while Instant::now() < deadline {
            let result = manager.read(key, *cursor).expect("read session");
            *cursor = result.cursor;
            for chunk in result.chunks {
                text.push_str(&chunk.data);
            }
            if !text.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        text
    }

    /// ⛔ THE CONTRACT THE HOT-RESTART SETTLE WINDOW RESTS ON.
    ///
    /// A parked reader must stop consuming **without losing anything**: the pty
    /// stays open, the child keeps running, and the bytes written while it was
    /// parked are still there when it wakes. That is what lets a retiring daemon
    /// hold its descriptors while it waits to see whether the successor
    /// survives — the alternative, two daemons reading one pty for the whole
    /// interval, silently eats half the user's output.
    ///
    /// Without the poll gate this test fails in a specific way: the reader is
    /// blocked inside `read`, so it swallows the post-park write and only then
    /// notices the flag. Falsified in exactly that shape before it was trusted.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_parked_reader_consumes_nothing_and_loses_nothing() {
        // ⛔ Spawning a pty READS the process-wide terminal-identity env, and
        // `codex_cli::env_test_guard` is the crate's one lock over it: a test
        // that reads it while another rewrites it makes BOTH flaky. Measured
        // here — without this, the two identity tests in `lib.rs` failed on
        // every full-suite run and passed alone.
        let _env = crate::codex_cli::env_test_guard();
        let mut manager = TerminalManager::new();
        let key = "local://parked-reader";
        manager
            .ensure_session(key, "sh -lc 'cat'", None)
            .expect("spawn a session that echoes what it is given");
        let mut cursor = 0u64;

        manager.write(key, "BEFORE-PARK\n").expect("write");
        let before = drain_terminal(&manager, key, &mut cursor, Duration::from_secs(3));
        assert!(
            before.contains("BEFORE-PARK"),
            "the reader must be serving before it is parked, got: {before:?}"
        );

        let park = manager.park_reader(key).expect("park the reader");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !park.has_stood_down() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            park.has_stood_down(),
            "a parked reader must reach the gate rather than stay in a read"
        );

        manager.write(key, "DURING-PARK\n").expect("write");
        let during = drain_terminal(&manager, key, &mut cursor, Duration::from_millis(600));
        assert!(
            during.is_empty(),
            "a parked reader must consume nothing, got: {during:?}"
        );
        assert!(
            manager.session_is_running(key),
            "parking must not touch the child — the descriptors are still held"
        );

        park.unpark();
        let after = drain_terminal(&manager, key, &mut cursor, Duration::from_secs(3));
        assert!(
            after.contains("DURING-PARK"),
            "the bytes written while parked must survive in the kernel buffer \
             and arrive on wake, got: {after:?}"
        );
        assert_eq!(
            park.stolen_after_park(),
            0,
            "nothing should have been consumed after the park was requested"
        );

        manager.remove_session(key, None).expect("remove session");
    }

    /// ⛔ A runtime this daemon has HANDED OVER must survive this daemon's own
    /// teardown. During the settle window another daemon is painting that pty
    /// for the user; stopping it here kills a live session that is not ours to
    /// stop — and our own exit does not, because exiting only closes our copies
    /// of the descriptors.
    #[cfg(target_os = "linux")]
    #[test]
    fn shutting_down_leaves_a_handed_off_runtime_alone() {
        // ⛔ Spawning a pty READS the process-wide terminal-identity env, and
        // `codex_cli::env_test_guard` is the crate's one lock over it: a test
        // that reads it while another rewrites it makes BOTH flaky. Measured
        // here — without this, the two identity tests in `lib.rs` failed on
        // every full-suite run and passed alone.
        let _env = crate::codex_cli::env_test_guard();
        let mut manager = TerminalManager::new();
        let handed_off = "local://handed-off-runtime";
        let still_ours = "local://still-our-runtime";
        manager
            .ensure_session(handed_off, "sh -lc 'sleep 30'", None)
            .expect("spawn the runtime that will be handed over");
        manager
            .ensure_session(still_ours, "sh -lc 'sleep 30'", None)
            .expect("spawn the runtime we keep");

        let park = manager.park_reader(handed_off).expect("park the reader");
        let summary = manager.shutdown_all(|_| None);

        assert_eq!(
            summary.stopped, 1,
            "only the runtime this daemon still serves may be stopped"
        );
        assert!(
            manager.session_is_running(handed_off),
            "the handed-off pty must still be running — another daemon is \
             serving it right now"
        );
        assert!(
            !manager.session_is_running(still_ours),
            "the runtime we still owned must actually have been stopped"
        );

        park.unpark();
        manager
            .remove_session(handed_off, None)
            .expect("clean up the surviving runtime");
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
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''",
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
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''",
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
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''stale-prose'\\'' '\\''/home/user'\\'''",
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
        // Visible text only: the appended viewport-reconcile chunk carries
        // escape params with digits; the guard is that the CHILD never saw
        // NO_COLOR=1, i.e. no visible "1" was printed.
        assert!(!strip_terminal_control_sequences(&combined).contains('1'));
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
            "ssh -tt oc 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''test-session'\\'' '\\''/home/user'\\'''",
        );
        let combined = selected
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("Done. Added these in the ThinkBook x layer."));
        assert!(!combined.contains("^[[?1;2c"));
        assert!(!combined.contains("^[]10;rgb:cccc/cccc/cccc"));
    }

    /// Signals on drop. The writer is MOVED into the writer thread, so this
    /// fires exactly when that thread's closure ends — the only observable that
    /// separates "the thread retired" from "the thread is parked on `recv()`".
    struct SignalOnDrop {
        dropped: mpsc::Sender<()>,
    }

    impl Write for SignalOnDrop {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Drop for SignalOnDrop {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }

    /// A dead PTY used to leak its writer thread: the reader's clone drops at
    /// EOF, but the clone the terminal entry holds does not, so `rx.recv()`
    /// never returns `Err` and the thread parks forever. Measured on the GUI
    /// host as 22 `pty-writer-*` threads against 19 `pty-reader-*`.
    ///
    /// Both halves are asserted, because only the pair proves the shutdown flag
    /// is what retires the thread rather than the send that carries it.
    #[test]
    fn a_writer_retires_on_shutdown_even_while_another_sender_is_alive() {
        // Half 1: a surviving sender alone must NOT retire the writer.
        let (idle_tx, idle_rx) = mpsc::channel();
        let idle_writer_tx = spawn_terminal_writer_thread(
            "local://retire-idle".to_string(),
            Box::new(SignalOnDrop { dropped: idle_tx }),
            Arc::new(AtomicU64::new(0)),
            4,
        )
        .expect("spawn idle writer");
        let idle_entry_clone = idle_writer_tx.clone();
        drop(idle_writer_tx);
        assert!(
            idle_rx.recv_timeout(Duration::from_millis(250)).is_err(),
            "a writer whose entry still holds a sender must stay parked — if this \
             passes trivially the second half proves nothing"
        );

        // Half 2: the shutdown the reader sends at exit DOES retire it, with
        // that same sender still alive.
        let (tx, rx) = mpsc::channel();
        let writer_tx = spawn_terminal_writer_thread(
            "local://retire".to_string(),
            Box::new(SignalOnDrop { dropped: tx }),
            Arc::new(AtomicU64::new(0)),
            4,
        )
        .expect("spawn writer");
        let entry_clone = writer_tx.clone();
        writer_tx
            .send(TerminalWriteRequest {
                data: Vec::new(),
                completion_tx: None,
                shutdown: true,
            })
            .expect("send shutdown");
        rx.recv_timeout(Duration::from_secs(2))
            .expect("writer must retire on shutdown while a sender is still alive");

        drop(entry_clone);
        drop(idle_entry_clone);
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

    #[test]
    fn agent_session_error_detects_claude_session_already_in_use() {
        let mut scanner = AgentSessionErrorScanner::default();
        let hits = scanner.scan(
            "Error: Session 52317975-9c66-40ef-8028-901b6415250e is already in use\n",
            1_000,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].pattern, "session_already_in_use");
        assert_eq!(
            hits[0].uuid.as_deref(),
            Some("52317975-9c66-40ef-8028-901b6415250e")
        );
    }

    #[test]
    fn agent_session_error_detects_missing_conversation_and_codex_rollout() {
        assert_eq!(
            agent_session_error_in_line(
                "No conversation found with session ID 47919c4a-92f4-4edd-bc11-ab6a250d947f"
            )
            .expect("hit")
            .pattern,
            "session_not_found"
        );
        assert_eq!(
            agent_session_error_in_line("error: no rollout found for the requested session")
                .expect("hit")
                .pattern,
            "session_not_found"
        );
        assert_eq!(
            agent_session_error_in_line("that session does not exist anymore")
                .expect("hit")
                .pattern,
            "session_not_found"
        );
    }

    #[test]
    fn agent_session_error_ignores_yggterm_internal_and_plain_output() {
        assert!(
            agent_session_error_in_line(
                "Error: terminal session not found: local://52317975-9c66-40ef-8028-901b6415250e"
            )
            .is_none(),
            "yggterm's own missing-runtime error has its own trace channel"
        );
        assert!(agent_session_error_in_line("cargo build finished in 3.2s").is_none());
        assert!(agent_session_error_in_line("file not found: ./missing.txt").is_none());
    }

    #[test]
    fn agent_session_error_ignores_conversation_prose_mentioning_the_error() {
        // The scanned PTY stream contains the agent's RENDERED CONVERSATION, so prose
        // that merely mentions a refusal used to be counted as one. These three samples
        // are verbatim from guihost's agent-incidents.jsonl (2026-07-11 telemetry campaign):
        // the user describing the bug, and the agent's own reply explaining it. Counting
        // them corrupts the incident count the probe exists to produce.
        assert!(
            agent_session_error_in_line(
                "so broken, tui not recognized, that i quit it and launched it from the startpge to be only greeted with session alreay in use or does not exist. i"
            )
            .is_none(),
            "the user's own prose about the bug is not an incident"
        );
        assert!(
            agent_session_error_in_line(
                "\"error: session <uuid> is already in use\" is claude code's own lock error - meaning when you quit the broken tui and relaunched from the startpage, yggterm ran claude -r"
            )
            .is_none(),
            "the agent's own explanation of the bug is not an incident"
        );
        assert!(
            agent_session_error_in_line(
                "i think the session 1965f8d5-bc71-432d-b9e5-398aff2815ef does not exist because we never wrote it, but let me verify that against the transcript first"
            )
            .is_none(),
            "prose that quotes a real uuid mid-sentence is not an incident"
        );

        // ...while the genuine refusals (also verbatim from guihost, same session) still count.
        let real_in_use = agent_session_error_in_line(
            "error: session id 1965f8d5-bc71-432d-b9e5-398aff2815ef is already in use.",
        )
        .expect("the real CLI lock refusal must still be detected");
        assert_eq!(real_in_use.pattern, "session_already_in_use");
        let real_missing = agent_session_error_in_line(
            "no conversation found with session id: 1965f8d5-bc71-432d-b9e5-398aff2815ef",
        )
        .expect("the real CLI missing-conversation refusal must still be detected");
        assert_eq!(real_missing.pattern, "session_not_found");

        // A TUI gutter glyph in front of the error must not hide it.
        assert!(
            agent_session_error_in_line(
                "⎿ Error: Session 52317975-9c66-40ef-8028-901b6415250e is already in use"
            )
            .is_some(),
            "leading TUI gutter glyphs are trimmed before the shape test"
        );
    }

    #[test]
    fn agent_session_error_matches_phrase_split_across_chunks() {
        let mut scanner = AgentSessionErrorScanner::default();
        assert!(
            scanner
                .scan("Error: Session 52317975-9c66-40ef-8028-901b64", 1_000)
                .is_empty()
        );
        let hits = scanner.scan("15250e is already in use", 2_000);
        assert_eq!(hits.len(), 1, "carry must join the split phrase");
        assert_eq!(hits[0].pattern, "session_already_in_use");
    }

    #[test]
    fn agent_session_error_throttles_tui_redraw_repeats() {
        let mut scanner = AgentSessionErrorScanner::default();
        let line = "Session 52317975-9c66-40ef-8028-901b6415250e is already in use\n";
        assert_eq!(scanner.scan(line, 1_000).len(), 1);
        assert!(
            scanner.scan(line, 30_000).is_empty(),
            "redraw within the throttle window must not re-fire"
        );
        assert_eq!(
            scanner.scan(line, 62_000).len(),
            1,
            "a hit after the window counts again"
        );
    }

    /// THE ACCEPTANCE TEST for the receiving half of level (b): a shell spawned
    /// by one owner keeps working after its master fd crosses a socket and a
    /// different owner adopts it.
    ///
    /// This is the in-tree version of the spike's headline result. It uses the
    /// real wire (`pty_handoff_wire`), a real `bash`, and the real
    /// `TerminalManager::adopt_session` — nothing about the fd is faked, which
    /// is the only way this proves the send side will be safe to build.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_adopted_shell_still_answers_after_its_fd_crosses_a_socket() {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        // The predecessor's side: open a pty and put a shell on it.
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .expect("openpty");
        let mut cmd = CommandBuilder::new("bash");
        cmd.arg("--norc");
        cmd.arg("-i");
        cmd.env("PS1", "");
        let child = pair.slave.spawn_command(cmd).expect("spawn bash");
        let shell_pid = child.process_id().expect("bash pid");
        let start_time =
            crate::pty_adoption::process_start_time(shell_pid).expect("bash start time");

        // Move the master fd across a real socket, exactly as a handoff would.
        let (send, recv) = UnixStream::pair().expect("socketpair");
        let raw = pair.master.as_raw_fd().expect("master raw fd");
        crate::pty_handoff_wire::send_master_fd(
            &send,
            raw,
            format!("pid={shell_pid} start={start_time}").as_bytes(),
        )
        .expect("send_master_fd");
        let (received, token) =
            crate::pty_handoff_wire::recv_master_fd(&recv).expect("recv_master_fd");
        assert!(String::from_utf8_lossy(&token).contains(&format!("pid={shell_pid}")));

        // The predecessor drops its master. The shell must NOT die: the
        // successor now holds the only master, which is the whole premise.
        drop(pair);

        // The successor adopts it, carrying the predecessor's screen.
        let mut manager = TerminalManager::new();
        let key = "local://adopted-in-test";
        manager
            .adopt_session(
                key,
                "bash",
                None,
                80,
                24,
                received,
                shell_pid,
                start_time,
                Some("CARRIED-HISTORY"),
            )
            .expect("adopt_session");

        // The carried transcript is there before a single live byte arrives.
        let seeded = manager.read(key, 0).expect("read adopted session");
        assert!(
            seeded
                .chunks
                .iter()
                .any(|chunk| chunk.data.contains("CARRIED-HISTORY")),
            "the predecessor's screen must survive the handoff, got: {:?}",
            seeded.chunks
        );

        // And the shell still ANSWERS through the adopted master.
        // The expected text must NOT be a substring of what we type: a PTY
        // echoes input, so asserting on a literal marker passes even if the
        // shell never ran. Arithmetic the shell must EVALUATE closes that hole
        // — "MARK42END" cannot appear unless bash executed the line.
        manager
            .write(key, "printf 'RESULT-%s\\n' $((6*7))\n")
            .expect("write to adopted pty");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut transcript = String::new();
        while std::time::Instant::now() < deadline {
            if let Ok(result) = manager.read(key, 0) {
                transcript = result
                    .chunks
                    .iter()
                    .map(|chunk| chunk.data.as_str())
                    .collect::<String>();
            }
            if transcript.contains("RESULT-42") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            transcript.contains("RESULT-42"),
            "the adopted shell must still EVALUATE and answer, not merely echo; \
             transcript: {transcript}"
        );

        let _ = manager.shutdown_all(|_key| None::<String>);
    }

    /// ⛔⛔ A RE-SENT ADOPTION OF THE **SAME CHILD** IS A LOST ACK, NOT A CONFLICT.
    ///
    /// The handoff has a window between the successor seating a pty and the
    /// predecessor receiving the ack. When that ack is lost the predecessor
    /// retries — and the retry used to be refused with *"this daemon already
    /// runs a live PTY for it"*, which is not a conflict at all: it is proof the
    /// first adoption worked. Because one failure aborts the whole sweep, a
    /// single key stuck in that state pinned every other session on the daemon,
    /// and it could never reach the empty hands that let it retire.
    ///
    /// Both arms, because presence and identity are exactly what must not be
    /// confused: the same child re-adopts idempotently, a DIFFERENT child under
    /// the same key is still refused. The second arm is the one that keeps this
    /// fix from becoming "close whatever pty someone names".
    #[cfg(target_os = "linux")]
    #[test]
    fn re_adopting_the_same_child_succeeds_but_a_different_child_is_still_refused() {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        // ⛔ A TEST THAT SPAWNS A PTY READS THE PROCESS-WIDE TERMINAL-IDENTITY ENV,
        // and without this guard it does not break itself — it breaks the identity
        // tests in `lib.rs`, which is far harder to attribute. Caught here: the
        // full suite went red on
        // `local_cc_relaunch_rebuild_collapses_poisoned_identity_to_row_id` while
        // that test passed alone AND passed paired with this one, so only the
        // whole-suite run showed it.
        let _env = crate::codex_cli::env_test_guard();

        fn spawn_shell_on_a_pty() -> (portable_pty::PtyPair, Box<dyn portable_pty::Child + Send + Sync>, u32, u64)
        {
            let pair = native_pty_system()
                .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
                .expect("openpty");
            let mut cmd = CommandBuilder::new("bash");
            cmd.arg("--norc");
            cmd.arg("-i");
            cmd.env("PS1", "");
            let child = pair.slave.spawn_command(cmd).expect("spawn bash");
            let pid = child.process_id().expect("bash pid");
            let start = crate::pty_adoption::process_start_time(pid).expect("bash start time");
            (pair, child, pid, start)
        }

        fn ferry_master(pair: &portable_pty::PtyPair) -> std::os::fd::OwnedFd {
            let (send, recv) = UnixStream::pair().expect("socketpair");
            let raw = pair.master.as_raw_fd().expect("master raw fd");
            crate::pty_handoff_wire::send_master_fd(&send, raw, b"t").expect("send_master_fd");
            crate::pty_handoff_wire::recv_master_fd(&recv).expect("recv_master_fd").0
        }

        let (pair_a, _child_a, pid_a, start_a) = spawn_shell_on_a_pty();
        let first = ferry_master(&pair_a);
        // A second descriptor for the SAME pty — what the predecessor still holds
        // and re-sends when it never hears the ack.
        let again = first.try_clone().expect("dup the master fd");
        drop(pair_a);

        let mut manager = TerminalManager::new();
        let key = "local://readopt-in-test";
        manager
            .adopt_session(key, "bash", None, 80, 24, first, pid_a, start_a, None)
            .expect("the first adoption seats the pty");

        // THE FIX: the retry is idempotent success, not a refusal.
        manager
            .adopt_session(key, "bash", None, 80, 24, again, pid_a, start_a, None)
            .expect(
                "re-adopting the SAME live child must succeed — the predecessor is \
                 retrying a handoff whose ack was lost, and refusing it pins the \
                 whole daemon",
            );

        // ⭐ And the duplicate descriptor closing must not hang up the child:
        // the shell has to still EVALUATE, not merely echo.
        manager
            .write(key, "printf 'READOPT-%s\\n' $((6*7))\n")
            .expect("write to re-adopted pty");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut transcript = String::new();
        while std::time::Instant::now() < deadline {
            if let Ok(result) = manager.read(key, 0) {
                transcript = result
                    .chunks
                    .iter()
                    .map(|chunk| chunk.data.as_str())
                    .collect::<String>();
            }
            if transcript.contains("READOPT-42") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            transcript.contains("READOPT-42"),
            "the shell must survive the redundant adoption and still answer; \
             transcript: {transcript}"
        );

        // THE NEGATIVE ARM: a DIFFERENT live child under the same key is still a
        // real conflict. Accepting it would let a predecessor drop a runtime
        // whose child is still on the far end of its own pty.
        let (pair_b, mut child_b, pid_b, start_b) = spawn_shell_on_a_pty();
        assert_ne!(pid_a, pid_b, "the two shells must be distinct processes");
        let other = ferry_master(&pair_b);
        drop(pair_b);
        let refused =
            manager.adopt_session(key, "bash", None, 80, 24, other, pid_b, start_b, None);
        assert!(
            refused.is_err(),
            "a DIFFERENT child under an occupied key must still be refused — \
             identity is the check, not presence"
        );

        let _ = child_b.kill();
        let _ = manager.shutdown_all(|_key| None::<String>);
    }

    /// A predecessor, a successor, and one real socket between them.
    ///
    /// Everything the handoff tests below need and nothing they do not: a live
    /// `bash` on a real pty for the predecessor, a listener the successor
    /// serves with the REAL [`crate::pty_handoff::serve_handoff`], and the real
    /// [`crate::pty_handoff::send_session`] driving it from the other side.
    #[cfg(target_os = "linux")]
    struct HandoffRig {
        socket: std::path::PathBuf,
        pair: portable_pty::PtyPair,
        _child: Box<dyn portable_pty::Child + Send + Sync>,
        shell_pid: u32,
        shell_start_time: u64,
    }

    #[cfg(target_os = "linux")]
    impl HandoffRig {
        fn new(label: &str) -> Self {
            use portable_pty::{CommandBuilder, PtySize, native_pty_system};
            let pair = native_pty_system()
                .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
                .expect("openpty");
            let mut cmd = CommandBuilder::new("bash");
            cmd.arg("--norc");
            cmd.arg("-i");
            cmd.env("PS1", "");
            let child = pair.slave.spawn_command(cmd).expect("spawn bash");
            let shell_pid = child.process_id().expect("bash pid");
            let shell_start_time =
                crate::pty_adoption::process_start_time(shell_pid).expect("bash start time");
            let socket = std::env::temp_dir().join(format!(
                "ygg-handoff-{label}-{}-{shell_pid}.sock",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&socket);
            Self {
                socket,
                pair,
                _child: child,
                shell_pid,
                shell_start_time,
            }
        }

        fn metadata(&self, key: &str, precommit_verdict: bool) -> crate::pty_handoff::HandoffMetadata {
            crate::pty_handoff::HandoffMetadata {
                version: crate::pty_handoff::HANDOFF_WIRE_VERSION,
                runtime_key: key.to_string(),
                launch_command: "bash".to_string(),
                cwd: None,
                cols: 80,
                rows: 24,
                shell_pid: self.shell_pid,
                shell_start_time: self.shell_start_time,
                screen: "CARRIED\r\n".to_string(),
                precommit_verdict,
            }
        }

        fn master_fd(&self) -> std::os::fd::RawFd {
            use std::os::fd::AsRawFd;
            self.pair.master.as_raw_fd().expect("master raw fd")
        }

        /// The predecessor's OWN master must still drive its shell. Arithmetic,
        /// never a literal marker: a pty echoes what is written to it, so a
        /// marker "arrives" even from a shell that died — only a value bash had
        /// to EVALUATE proves the far end is alive.
        fn shell_still_answers(&self, marker: &str) -> bool {
            use std::io::{Read, Write};
            let mut writer = self.pair.master.take_writer().expect("master writer");
            let mut reader = self.pair.master.try_clone_reader().expect("master reader");
            writer
                .write_all(format!("printf '{marker}-%s\n' $((6*7))\n").as_bytes())
                .expect("write to the predecessor's master");
            writer.flush().ok();
            let want = format!("{marker}-42");
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut seen = String::new();
                let mut buf = [0u8; 1024];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if seen.contains(&want) {
                        let _ = tx.send(true);
                        return;
                    }
                }
                let _ = tx.send(false);
            });
            rx.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap_or(false)
        }
    }

    /// Serve exactly one connection with the real successor half, and report
    /// what it decided.
    ///
    /// `occupied_by` is the child the successor is already running under the
    /// key — `None` for a vacant successor.
    ///
    /// ⚠ Whether a verdict goes out is NOT a parameter here, and that is the
    /// contract under test: the successor answers only when the predecessor's
    /// metadata says it is listening.
    #[cfg(target_os = "linux")]
    fn serve_one_handoff(
        socket: &std::path::Path,
        occupied_by: Option<(u32, u64)>,
    ) -> std::thread::JoinHandle<crate::pty_handoff::HandoffServed> {
        let listener =
            std::os::unix::net::UnixListener::bind(socket).expect("bind the handoff socket");
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept the handoff");
            crate::pty_handoff::serve_handoff(
                &stream,
                &mut |metadata| {
                    // The real predicate's shape: a DIFFERENT live child under
                    // the key is a conflict, the same one is not.
                    match occupied_by {
                        Some((pid, start))
                            if (pid, start) != (metadata.shell_pid, metadata.shell_start_time) =>
                        {
                            Some(seat_conflict_reason(&metadata.runtime_key))
                        }
                        _ => None,
                    }
                },
                &mut |_metadata, fd| {
                    drop(fd);
                    Ok(())
                },
            )
        })
    }

    /// ⛔⛔⛔ A REFUSAL EVALUATED AFTER THE COMMIT POINT COSTS A WHOLE DAEMON.
    ///
    /// The successor's seat check is right and does not change: a DIFFERENT
    /// live child under the same key must not be displaced. It used to run only
    /// once the descriptor had already crossed, so the predecessor was told
    /// `committed: true` — *"the fd is gone"* — about a refusal that could have
    /// been free.
    ///
    /// ⚠ **And "the fd is gone" was never true**, which is why this was misread
    /// for so long: [`HandoffTakeout::master_fd`] is BORROWED, `sendmsg` moves a
    /// DUPLICATE, and a successor that refuses drops that duplicate. The real
    /// cost is that the sweep books a failure it can never clear, so the
    /// predecessor never reaches `AllMoved` and never retires — pinned for life
    /// holding every session it owns.
    ///
    /// So this asserts BOTH halves, and the second is the one that was already
    /// silently true: the refusal must land before the commit point, AND the
    /// predecessor's own master must still drive its shell afterwards.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_refused_handoff_is_answered_before_the_descriptor_moves() {
        let _env = crate::codex_cli::env_test_guard();

        let rig = HandoffRig::new("refused");
        let key = "local://contested-in-test";
        // The successor already runs a DIFFERENT child under this key: a real
        // conflict, and the only case the seat rule refuses.
        let served = serve_one_handoff(&rig.socket, Some((rig.shell_pid + 1, 7)));

        let mut support = crate::pty_handoff::PrecommitSupport::default();
        let error = crate::pty_handoff::send_session(
            &rig.socket,
            &rig.metadata(key, true),
            rig.master_fd(),
            &mut support,
        )
        .expect_err("a different live child under the key must be refused");

        assert!(
            !error.committed,
            "the refusal must land BEFORE the commit point — a sweep that books \
             a committed failure can never reach AllMoved, and the daemon that \
             booked it never retires. Got: {error}"
        );
        assert_eq!(
            support,
            crate::pty_handoff::PrecommitSupport::Speaks,
            "a successor that answered must be remembered as answering, or every \
             later session in the sweep pays the timeout again"
        );
        let served = served.join().expect("successor thread");
        assert!(
            served.refused_before_commit,
            "the successor must record which side of the commit point it refused on"
        );
        assert!(!served.adopted);

        // ⭐ THE HALF THAT MATTERS TO A HUMAN: the session is untouched.
        assert!(
            rig.shell_still_answers("REFUSED"),
            "a refused handoff must leave the predecessor's own master driving \
             its shell — the agent on the far end never finds out"
        );
        let _ = std::fs::remove_file(&rig.socket);
    }

    /// ⛔ COMPATIBILITY, DIRECTION 1: a NEW predecessor against an OLD successor.
    ///
    /// An older successor knows nothing about a verdict and simply blocks in
    /// `recvmsg`. The new predecessor must not hang waiting for a line that
    /// will never come — it waits a bounded interval, records that this peer is
    /// silent, and proceeds exactly as it does today.
    ///
    /// ⚠ The memo is the point, not the timeout: a sweep hands over every
    /// runtime it owns to the same successor, so asking per session would cost
    /// one timeout per session on a daemon holding the host's PTYs.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_successor_that_never_answers_still_gets_the_descriptor() {
        let _env = crate::codex_cli::env_test_guard();

        let rig = HandoffRig::new("silent");
        let socket = rig.socket.clone();
        let listener =
            std::os::unix::net::UnixListener::bind(&socket).expect("bind the handoff socket");
        // An OLD successor, hand-written: it reads the line and goes straight
        // for the descriptor, exactly as every build before the verdict does.
        let old = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let metadata = crate::pty_handoff::receive_metadata(&stream).expect("metadata");
            let fd = crate::pty_handoff::receive_descriptor(&stream, &metadata).expect("fd");
            drop(fd);
            crate::pty_handoff::send_ack(&stream, &crate::pty_handoff::HandoffAck::adopted_here())
                .expect("ack");
        });

        let mut support = crate::pty_handoff::PrecommitSupport::default();
        crate::pty_handoff::send_session(
            &socket,
            &rig.metadata("local://silent-successor", true),
            rig.master_fd(),
            &mut support,
        )
        .expect("an older successor must still complete the handoff");
        old.join().expect("old successor thread");

        assert_eq!(
            support,
            crate::pty_handoff::PrecommitSupport::OneSilence,
            "one silence must be REMEMBERED but must not yet conclude anything: \
             an old build and a successor briefly behind its own runtime lock \
             look identical here"
        );
        // ⭐ A SECOND silence is what settles it, and only then does the sweep
        // stop waiting. Proving the transition needs the same peer twice,
        // because the whole point is that one strike decides nothing.
        // ⛔ The first listener's socket FILE outlives its listener; a bind over
        // it is AddrInUse, not a fresh listener.
        let _ = std::fs::remove_file(&socket);
        let listener =
            std::os::unix::net::UnixListener::bind(&socket).expect("rebind the handoff socket");
        let old = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let metadata = crate::pty_handoff::receive_metadata(&stream).expect("metadata");
            let fd = crate::pty_handoff::receive_descriptor(&stream, &metadata).expect("fd");
            drop(fd);
            crate::pty_handoff::send_ack(&stream, &crate::pty_handoff::HandoffAck::adopted_here())
                .expect("ack");
        });
        crate::pty_handoff::send_session(
            &socket,
            &rig.metadata("local://silent-successor-2", true),
            rig.master_fd(),
            &mut support,
        )
        .expect("still completes");
        old.join().expect("old successor thread");
        assert_eq!(
            support,
            crate::pty_handoff::PrecommitSupport::Silent,
            "two silences settle it — from here the sweep stops paying the wait"
        );
        let _ = std::fs::remove_file(&socket);
    }

    /// ⛔ COMPATIBILITY, DIRECTION 2: an OLD predecessor against a NEW successor.
    ///
    /// **This is the direction that fails silently if it fails at all.** An
    /// older predecessor reads the first line after its own `sendmsg` as its
    /// ack. A successor that volunteered a verdict would hand it a line it
    /// cannot parse, and a handover that works today would start reporting
    /// "unreadable ack" — a regression caused entirely by the fix.
    ///
    /// The metadata's `precommit_verdict` flag is what prevents it, and
    /// `#[serde(default)]` is what makes an old line say `false` without ever
    /// having heard of the field.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_predecessor_that_never_asks_is_never_sent_a_verdict() {
        use std::io::Write;
        let _env = crate::codex_cli::env_test_guard();

        let rig = HandoffRig::new("unasked");
        let key = "local://unasked-in-test";
        // A conflict, so the successor has something it WANTS to say early.
        let served = serve_one_handoff(&rig.socket, Some((rig.shell_pid + 1, 7)));

        // An OLD predecessor, hand-written: no flag, no verdict read, and the
        // first line it sees after the descriptor is taken to be its ack.
        let mut stream =
            std::os::unix::net::UnixStream::connect(&rig.socket).expect("connect");
        let mut line = serde_json::to_string(&rig.metadata(key, false)).expect("encode");
        line.push('\n');
        stream.write_all(line.as_bytes()).expect("send metadata");
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));

        let mut first = Vec::new();
        {
            use std::io::Read;
            let mut byte = [0u8; 1];
            while let Ok(1) = (&stream).read(&mut byte) {
                if byte[0] == b'\n' {
                    break;
                }
                first.push(byte[0]);
            }
        }
        let first = String::from_utf8_lossy(&first).into_owned();
        let ack: crate::pty_handoff::HandoffAck = serde_json::from_str(first.trim()).expect(
            "the FIRST line an unasking predecessor sees must be an ack it can parse — \
             a volunteered verdict lands here and breaks a handover that works today",
        );
        assert!(!ack.adopted, "the conflict is still refused, just not early");

        let served = served.join().expect("successor thread");
        assert!(
            served.refused_before_commit,
            "the successor still declines to take a descriptor it cannot seat, \
             whether or not anyone asked it to say so first"
        );
        // And the old predecessor's session is untouched either way.
        assert!(rig.shell_still_answers("UNASKED"));
        let _ = std::fs::remove_file(&rig.socket);
    }

    /// ⛔⛔ THE DISCRIMINATOR THE DUPLICATE-PRUNE RESTS ON.
    ///
    /// A runtime we SPAWNED owns its child alone, so telling its other owner to
    /// drop it is cleanup. An ADOPTED runtime is the same process on the same
    /// pty as the predecessor's copy, and a drop is `remove_session` ->
    /// `shutdown` -> `kill` — so pruning it kills the agent we are serving.
    /// Measured fatal 2026-08-14: the prune fired against an adopted runtime
    /// after a lost handoff ack and the transcript's last write lands in the
    /// same second as the drop.
    ///
    /// If this ever reports `false` for an adopted runtime, the prune loses its
    /// only way to tell "stale copy" from "the live process, twice".
    #[cfg(target_os = "linux")]
    #[test]
    fn an_adopted_runtime_says_so_and_a_spawned_one_does_not() {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let _env = crate::codex_cli::env_test_guard();

        let mut manager = TerminalManager::new();

        // A runtime this manager spawned itself.
        let spawned_key = "local://spawned-not-adopted";
        manager
            .ensure_session_with_size(spawned_key, "bash --norc -i", None, Some((80, 24)))
            .expect("spawn a normal session");
        assert!(
            !manager.session_is_adopted(spawned_key),
            "a session we forked ourselves must NOT report as adopted — the prune \
             would then refuse to clean up a genuinely stale duplicate"
        );

        // A runtime adopted over the real wire.
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .expect("openpty");
        let mut cmd = CommandBuilder::new("bash");
        cmd.arg("--norc");
        cmd.arg("-i");
        cmd.env("PS1", "");
        let child = pair.slave.spawn_command(cmd).expect("spawn bash");
        let pid = child.process_id().expect("bash pid");
        let start = crate::pty_adoption::process_start_time(pid).expect("start time");
        let (send, recv) = UnixStream::pair().expect("socketpair");
        let raw = pair.master.as_raw_fd().expect("master raw fd");
        crate::pty_handoff_wire::send_master_fd(&send, raw, b"t").expect("send");
        let (received, _token) =
            crate::pty_handoff_wire::recv_master_fd(&recv).expect("recv");
        drop(pair);

        let adopted_key = "local://adopted-shares-its-child";
        manager
            .adopt_session(adopted_key, "bash", None, 80, 24, received, pid, start, None)
            .expect("adopt");
        assert!(
            manager.session_is_adopted(adopted_key),
            "an ADOPTED session must say so — it shares its child with whoever \
             handed it over, and the prune kills that child if it does not know"
        );

        let _ = manager.shutdown_all(|_key| None::<String>);
    }

    /// The PTY is created at the grid the VIEWER has, whatever CLI is talking.
    ///
    /// This locks the removal of a per-CLI clamp that shrank eight named agent
    /// CLIs to 120x40 at spawn. Its stated premise was that those TUIs "render
    /// at a fixed width (e.g. 100 cols)", so a smaller PTY would let them fill
    /// the viewport. Measured against the daemon's own vt100 parser
    /// (`scripts/cli-viewport-probe`), three of the clamped CLIs paint to within
    /// two columns of whatever grid they are handed, and the one that really
    /// does render narrow painted the same 102 columns at 120 as at 173 — so the
    /// clamp helped nobody. It also could not do what it claimed: the viewport is
    /// xterm's grid, which a smaller PTY does not change, so the clamp only
    /// shrank the app's world and left the rest of the screen dead.
    ///
    /// ⇒ The invariant is the one `agent_arm_shell_matrix.rs` states: an axis
    /// here is a property of WHERE THE PTY LIVES, never of WHICH CLI is talking.
    /// A test that only checked one CLI would pass against a re-introduced clamp
    /// that named a different one, so this walks the whole former list, by BOTH
    /// routes the old predicate matched — the row scheme and a bare binary name
    /// anywhere in the launch command.
    #[test]
    fn pty_is_created_at_the_requested_grid_for_every_cli() {
        let mut manager = TerminalManager::new();
        // Invented row schemes, one per CLI family the clamp used to name.
        let schemes = [
            "grok-runtime://",
            "opencode-runtime://",
            "qwen-runtime://",
            "kimi-runtime://",
            "muse-runtime://",
            "agy-runtime://",
            "pi-runtime://",
            "remote-grok://",
            "remote-opencode://",
        ];
        for (index, scheme) in schemes.iter().enumerate() {
            let key = format!("{scheme}sized-{index}");
            manager
                .ensure_session_with_size(&key, "sh -c 'sleep 5'", None, Some((173, 63)))
                .expect("spawn at the caller's grid");
            assert_eq!(
                manager.session_size(&key),
                Some((173, 63)),
                "{key} must hold the grid the viewer has; a per-CLI clamp here \
                 paints the TUI into a corner of the viewport and leaves the rest dead"
            );
        }
        // The second route: the old predicate also matched a bare CLI binary name
        // ANYWHERE in the launch command, on a basename compare — so a path token
        // ending in one of those names pulled in rows that were never agent rows
        // at all, plain shells included.
        let key = "local://token-route";
        manager
            .ensure_session_with_size(&key, "sh -c 'sleep 5; : /opt/demo/grok'", None, Some((173, 63)))
            .expect("spawn a row whose command merely mentions a CLI name");
        assert_eq!(
            manager.session_size(key),
            Some((173, 63)),
            "a launch command that merely CONTAINS a CLI's name must not resize \
             anyone's terminal — the old matcher compared basenames, so an \
             ordinary path token was enough to shrink a row"
        );
        let _ = manager.shutdown_all(|_key| None::<String>);
    }

    /// A restart keeps the client's grid — the path where the clamp was permanent.
    ///
    /// The attach path resizes the PTY to the client's grid immediately after
    /// `ensure_session_with_size` (the D1 `reattach_grid_resync` in `daemon.rs`),
    /// so a clamp applied there was corrected on the next attach and looked
    /// survivable. Nothing resyncs behind a RESTART, and the client emits a
    /// Resize only when its OWN grid changes — which a daemon-side restart does
    /// not do. So a restarted row kept the clamped grid for the rest of its life,
    /// painting into 120x40 of a full-size viewport. Since a daemon hot-update
    /// restarts every live row, that is the state the affected CLIs were usually
    /// found in.
    #[test]
    fn a_restart_keeps_the_client_grid_for_a_formerly_clamped_cli() {
        let mut manager = TerminalManager::new();
        let key = "opencode-runtime://restart-grid";
        manager
            .ensure_session_with_size(key, "sh -c 'sleep 5'", None, Some((173, 63)))
            .expect("spawn");
        assert_eq!(manager.session_size(key), Some((173, 63)));
        manager
            .restart_session_with_size(key, "sh -c 'sleep 5'", None, None, Some((173, 63)))
            .expect("restart with the client's grid");
        assert_eq!(
            manager.session_size(key),
            Some((173, 63)),
            "a restart must hand the row back at the grid the client is showing"
        );
        // And with no explicit grid the outgoing runtime's size carries forward,
        // so a restart never silently narrows a row either.
        manager
            .restart_session_with_size(key, "sh -c 'sleep 5'", None, None, None)
            .expect("restart with no explicit grid");
        assert_eq!(
            manager.session_size(key),
            Some((173, 63)),
            "a restart with no caller grid must preserve the running one"
        );
        let _ = manager.shutdown_all(|_key| None::<String>);
    }
    // ⛔ THE RACE THIS CLOSES, in the shape it actually happened: a supervision
    // tool types text, a human's keystroke lands in the gap, and the tool's
    // Enter submits BOTH glued together. The guard refuses unless the line is
    // exactly what the tool wrote — and it compares the forwarded input stream,
    // not the screen, so it is right even before anything has echoed.
    #[test]
    fn a_conditional_submit_presses_enter_only_on_the_line_it_was_promised() {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .expect("openpty");
        let mut cmd = CommandBuilder::new("bash");
        cmd.arg("--norc");
        cmd.arg("-i");
        cmd.env("PS1", "");
        let child = pair.slave.spawn_command(cmd).expect("spawn bash");
        let pid = child.process_id().expect("bash pid");
        let start = crate::pty_adoption::process_start_time(pid).expect("bash start time");
        let (send, recv) = UnixStream::pair().expect("socketpair");
        let raw = pair.master.as_raw_fd().expect("master raw fd");
        crate::pty_handoff_wire::send_master_fd(&send, raw, b"t").expect("send_master_fd");
        let master = crate::pty_handoff_wire::recv_master_fd(&recv)
            .expect("recv_master_fd")
            .0;
        drop(pair);

        let mut manager = TerminalManager::new();
        let key = "local://submit-iff-line-test";
        manager
            .adopt_session(key, "bash", None, 80, 24, master, pid, start, None)
            .expect("adopt_session");

        // The tool types its text. Nothing has echoed yet and it does not matter.
        manager.write(key, "boot the row").expect("write boot text");

        // A line that is not what we wrote is refused, and the refusal reports
        // LENGTHS — never the text, which may be the human's own sentence.
        match manager.session_submit_if_line_equals(key, "something else") {
            SubmitIffLineVerdict::LineMismatch { line_len, expected_len } => {
                assert_eq!(line_len, "boot the row".len());
                assert_eq!(expected_len, "something else".len());
            }
            other => panic!("a differing line must refuse, got {other:?}"),
        }

        // The human's keystroke lands in the gap — the exact incident shape.
        manager.write(key, "x").expect("write the human keystroke");
        assert_eq!(
            manager.session_submit_if_line_equals(key, "boot the row"),
            SubmitIffLineVerdict::LineMismatch {
                line_len: "boot the rowx".len(),
                expected_len: "boot the row".len(),
            },
            "a keystroke in the gap must abort the submit — this is the whole bug"
        );

        // With the line exactly as promised, it submits.
        manager.write(key, "\x7f").expect("the human backspaces");
        assert_eq!(
            manager.session_submit_if_line_equals(key, "boot the row"),
            SubmitIffLineVerdict::Submitted
        );

        // ⛔ And it cannot submit twice: the Enter cleared the line, so the same
        // call now refuses against an empty composer. Left unclear, this would
        // press Enter again on text the composer no longer holds.
        assert_eq!(
            manager.session_submit_if_line_equals(key, "boot the row"),
            SubmitIffLineVerdict::LineMismatch {
                line_len: 0,
                expected_len: "boot the row".len(),
            }
        );
        assert_eq!(
            manager.session_has_pending_input_draft(key),
            Some(false),
            "the submit cleared the draft, or every later guarded write refuses forever"
        );

        // A row this daemon does not hold answers NotOwned — which a caller must
        // not read as a refusal.
        assert_eq!(
            manager.session_submit_if_line_equals("local://not-here", "boot the row"),
            SubmitIffLineVerdict::NotOwned
        );
    }

}
