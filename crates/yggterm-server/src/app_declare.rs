//! Daemon-side ingestion of libyggterm's OSC 7717 control channel.
//!
//! An app (ychrome, yedit, …) declares its surfaces by writing
//! `ESC ] 7717 ; <verb> ; <action> ; <base64-json> BEL` to its own stdout, and
//! until now ONLY the client-side xterm parser read it. That made the declare
//! as mortal as the client host: a session that was never revealed has no
//! xterm host at all, and a surface that the background reaper collected could
//! not be rebuilt, because the bytes that would have rebuilt it were consumed
//! by a parser that no longer existed (`web ensure` → `tabs:0`, unrecoverable —
//! finding #2 in docs/agent-control-plane.md, the ceiling on unattended
//! co-browse).
//!
//! The daemon owns the PTY, so it sees every one of those bytes whether or not
//! a GUI is looking. It keeps the LATEST declare per verb — nothing more: no
//! history, no schema (the GUI still GETs that from the app's control
//! endpoint), and no replay of anything that needs a human (`fido2`). That is
//! enough for the GUI to rebuild a surface on an explicit request.
//!
//! Retention is deliberately "latest wins, close clears": the app re-emits its
//! full payload on every ~4s heartbeat, so the retained record is never more
//! than a heartbeat stale, and an app that exits cleanly leaves nothing behind.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// `ESC ] 7717 ;` — the start of a libyggterm control sequence.
const OSC_PREFIX: &str = "\x1b]7717;";
/// The wire spelling of a web-surface `open` declare, up to its payload.
pub const WEB_SURFACE_OPEN_SEQUENCE: &str = "\x1b]7717;web-surface;open;";
/// What an attach replay serves in its place: the SAME bytes with the action
/// `seen` — deliberately the same length as `open`, so the rewrite never moves
/// a byte and retained chunk boundaries stay exactly where they were.
///
/// Why the rewrite exists: the daemon CONSUMES a declare the moment it arrives
/// ([`AppDeclareLog`] retains the latest state), but the raw bytes stay in the
/// retained chunk ring as scrollback transcript, and a cursor-0 attach replays
/// that ring. Served verbatim, the run's original `open` re-executes LAUNCH
/// intent against a client that is merely re-attaching — the client cannot
/// tell a replayed `open` from a live one (live incident 2026-07-23: a
/// retained-scrollback replay delivered a stale declare before the fresh one).
/// `seen` says what the daemon knows: this app declared, and the declare was
/// already consumed — liveness, not intent. A client maps it to a re-attach; a
/// plain terminal ignores it like every other unknown OSC.
pub const WEB_SURFACE_SEEN_SEQUENCE: &str = "\x1b]7717;web-surface;seen;";
/// A partial sequence longer than this is junk (a real declare is a URL, a
/// title and a handful of pane labels), so the scanner drops it rather than
/// growing a buffer on a stream that happens to contain the prefix bytes.
const MAX_PENDING_BYTES: usize = 64 * 1024;

/// One control message lifted off a PTY stream.
#[derive(Debug, Clone, PartialEq)]
pub struct AppDeclareMessage {
    pub verb: String,
    pub action: String,
    pub payload: serde_json::Value,
}

/// A retained declare, as handed to the GUI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppDeclareRecord {
    pub verb: String,
    pub action: String,
    pub payload: serde_json::Value,
    /// When the daemon last saw this declare (heartbeats refresh it).
    pub at_ms: u64,
    /// Monotonic per-session counter, so a consumer can tell "the same declare
    /// again" from "a new one" without diffing payloads.
    pub seq: u64,
}

/// What a `<verb>;<action>` pair means for retention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retention {
    /// Remember it as the verb's current state (open/heartbeat/declare).
    Store,
    /// The app retired this surface — remembering it would resurrect a thing
    /// the app deliberately closed.
    Clear,
    /// Not rebuildable, or not ours to replay.
    Ignore,
}

/// SSOT for what the daemon retains. Anything not named here is ignored: a new
/// verb must opt IN, so an app cannot get its messages stored (and later
/// replayed) by accident.
fn retention_for(verb: &str, action: &str) -> Retention {
    match (verb, action) {
        ("web-surface", "open" | "heartbeat") => Retention::Store,
        ("web-surface", "close") => Retention::Clear,
        // `seen` is minted by the daemon itself at attach-replay serve time
        // ([`WEB_SURFACE_SEEN_SEQUENCE`]) — never an app's own word. It exists
        // only in SERVED copies, never in the ring this scanner reads, so this
        // arm is defensive: an app that emits it anyway must not overwrite the
        // record's live action with a word that means "already consumed".
        ("web-surface", "seen") => Retention::Ignore,
        // A picker is a native prompt awaiting a human choice, not a surface
        // that can be rebuilt behind their back.
        ("web-surface", "pick") => Retention::Ignore,
        ("sidebar", "declare") => Retention::Store,
        ("sidebar", "close") => Retention::Clear,
        // A WebAuthn ceremony asks for the user's PRESENCE. A retained copy
        // could be replayed at a moment nobody is there to consent — never
        // store one.
        ("fido2", _) => Retention::Ignore,
        _ => Retention::Ignore,
    }
}

/// Incremental OSC 7717 extractor for a byte stream that arrives in chunks.
#[derive(Debug, Default)]
pub struct AppDeclareScanner {
    pending: String,
}

impl AppDeclareScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next decoded chunk; returns every complete message in it.
    ///
    /// A sequence split across chunk boundaries is held until its terminator
    /// arrives (the PTY reader hands us ~arbitrary boundaries, and a declare
    /// straddling one used to be the difference between "the app declared" and
    /// "the app never declared").
    pub fn scan(&mut self, data: &str) -> Vec<AppDeclareMessage> {
        let mut out = Vec::new();
        if self.pending.is_empty() && !data.contains('\x1b') {
            // Fast path: the overwhelming majority of terminal output carries
            // no escape at all, so it can never start a sequence.
            return out;
        }
        self.pending.push_str(data);
        loop {
            let Some(start) = self.pending.find(OSC_PREFIX) else {
                self.keep_possible_prefix_tail();
                break;
            };
            let body_start = start + OSC_PREFIX.len();
            let Some((body_len, terminator_len)) = find_terminator(&self.pending[body_start..])
            else {
                // Incomplete: keep from the sequence start, drop what precedes.
                if start > 0 {
                    self.pending.drain(..start);
                }
                if self.pending.len() > MAX_PENDING_BYTES {
                    self.pending.clear();
                }
                break;
            };
            if let Some(message) =
                parse_declare_body(&self.pending[body_start..body_start + body_len])
            {
                out.push(message);
            }
            self.pending
                .drain(..body_start + body_len + terminator_len);
        }
        out
    }

    /// Keep only what could still be the beginning of `OSC_PREFIX`, so a
    /// prefix split across chunks survives without retaining the whole stream.
    fn keep_possible_prefix_tail(&mut self) {
        let keep = OSC_PREFIX.len().saturating_sub(1);
        if self.pending.len() <= keep {
            return;
        }
        let mut cut = self.pending.len() - keep;
        while cut < self.pending.len() && !self.pending.is_char_boundary(cut) {
            cut += 1;
        }
        self.pending.drain(..cut);
    }
}

/// Is this chunk NOTHING but libyggterm's own OSC 7717 control traffic?
///
/// ⛔ **This is the answer to "was the SESSION active", and the two are not the
/// same question.** The daemon's PTY reader stamps `last_activity_ms` on every
/// chunk it sees, and the hot-restart idle gate then asks whether every owned
/// session has been silent for 300 s. But a declaring app re-emits its full
/// payload on a **~4 s heartbeat** (see this module's header), so a session
/// hosting one is never silent for more than four seconds — by our own design,
/// not by anything the human or the agent did.
///
/// Measured on `dev`, 2026-08-10: five `bash -i` shells that no human had
/// touched in weeks reported `idle_ms` of 266, 1079, 1387, 1711 and 3433 against
/// a 300,000 ms threshold, every one of them a ychrome launcher whose
/// daemonised child still held the PTY and heartbeated `web-surface;pick` onto
/// it. The gate is an AND over owned sessions, so it could never open; the
/// daemon could never retire; and the pile reached **27 daemons on one machine,
/// burning 8.1 cores and 23 GB between them** — one per deploy, going back 27
/// days. THE QUIET-GATE LAW with the app as the thing that is never quiet.
///
/// ⚠ **The bias is deliberately one-directional.** A sequence split across two
/// chunks is recognised in neither half, so both halves count as activity — a
/// late retire, which costs nothing. The opposite error would discount real
/// output and let a daemon cold-shutdown a session mid-turn, which is the bug
/// the gate exists to prevent. Never widen this to "no visible text": an agent
/// CLI's spinner frame is pure cursor movement and IS the session working.
pub fn chunk_is_only_app_declares(data: &str) -> bool {
    if data.is_empty() {
        return false;
    }
    let mut rest = data;
    let mut saw_declare = false;
    while let Some(start) = rest.find(OSC_PREFIX) {
        // Anything before the sequence is real output from the session.
        if start > 0 {
            return false;
        }
        let body_start = OSC_PREFIX.len();
        let Some((body_len, terminator_len)) = find_terminator(&rest[body_start..]) else {
            // Incomplete tail: cannot prove it is ours, so it is activity.
            return false;
        };
        saw_declare = true;
        rest = &rest[body_start + body_len + terminator_len..];
    }
    saw_declare && rest.is_empty()
}

/// BEL or ST, whichever comes first. Returns (body length, terminator length).
fn find_terminator(rest: &str) -> Option<(usize, usize)> {
    let bel = rest.find('\x07').map(|index| (index, 1));
    let st = rest.find("\x1b\\").map(|index| (index, 2));
    match (bel, st) {
        (Some(bel), Some(st)) => Some(if bel.0 <= st.0 { bel } else { st }),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

fn parse_declare_body(body: &str) -> Option<AppDeclareMessage> {
    let mut parts = body.splitn(3, ';');
    let verb = parts.next()?.trim();
    let action = parts.next()?.trim();
    if verb.is_empty() || action.is_empty() {
        return None;
    }
    if retention_for(verb, action) == Retention::Ignore {
        return None;
    }
    let payload = match parts.next() {
        Some(encoded) if !encoded.trim().is_empty() => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .ok()?;
            serde_json::from_slice::<serde_json::Value>(&bytes).ok()?
        }
        _ => serde_json::Value::Null,
    };
    Some(AppDeclareMessage {
        verb: verb.to_string(),
        action: action.to_string(),
        payload,
    })
}

/// Must a cursor-0 attach replay neutralize retained web-surface `open`
/// declares, given the retained record's CURRENT action for the verb?
///
/// - `Some("heartbeat")` — the run outlived its launch (a live app re-declares
///   every ~4s), so any `open` in the replayed tail is consumed history: YES.
/// - `None` — no record. Either nothing ever declared (then the tail holds no
///   `open` and the rewrite is a no-op) or a `close` cleared it, in which case
///   an `open` in the tail belongs to a FINISHED run — equally history: YES.
/// - `Some("open")` — the app launched within the last heartbeat interval and
///   the replayed `open` IS the current declare: NO, serve it verbatim. This
///   is the same deterministic sliver rule the GUI's retained-declare rebuild
///   applies (a record whose latest action is `open` classifies as a launch).
///
/// Catch-up reads (cursor > 0) are OUT of this policy on purpose: they deliver
/// bytes this client has never consumed, so an `open` there carries the same
/// launch intent it would have carried live (e.g. an app launched while the
/// session was backgrounded).
pub fn attach_replay_neutralizes_web_surface_open(record_action: Option<&str>) -> bool {
    record_action != Some("open")
}

/// Rewrite every consumed web-surface `open` in a replayed stream to `seen`.
///
/// Returns `None` when the stream holds no such sequence (the overwhelmingly
/// common case — the caller keeps its original bytes untouched). The swap is
/// same-length by construction ([`WEB_SURFACE_SEEN_SEQUENCE`]), so the result
/// is byte-for-byte the same size and every original index keeps its meaning —
/// the caller may re-slice it at the original chunk boundaries.
pub fn rewrite_consumed_web_surface_opens(stream: &str) -> Option<(String, usize)> {
    const _: () = assert!(WEB_SURFACE_OPEN_SEQUENCE.len() == WEB_SURFACE_SEEN_SEQUENCE.len());
    if !stream.contains(WEB_SURFACE_OPEN_SEQUENCE) {
        return None;
    }
    let count = stream.matches(WEB_SURFACE_OPEN_SEQUENCE).count();
    Some((
        stream.replace(WEB_SURFACE_OPEN_SEQUENCE, WEB_SURFACE_SEEN_SEQUENCE),
        count,
    ))
}

/// The latest declare per verb for ONE session.
#[derive(Debug, Default)]
pub struct AppDeclareLog {
    seq: u64,
    latest: BTreeMap<String, AppDeclareRecord>,
}

impl AppDeclareLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest(&mut self, message: AppDeclareMessage, now_ms: u64) {
        match retention_for(&message.verb, &message.action) {
            Retention::Store => {
                self.seq += 1;
                let seq = self.seq;
                self.latest.insert(
                    message.verb.clone(),
                    AppDeclareRecord {
                        verb: message.verb,
                        action: message.action,
                        payload: message.payload,
                        at_ms: now_ms,
                        seq,
                    },
                );
            }
            Retention::Clear => {
                self.latest.remove(&message.verb);
            }
            Retention::Ignore => {}
        }
    }

    pub fn records(&self) -> Vec<AppDeclareRecord> {
        self.latest.values().cloned().collect()
    }
}

// ─── Generic OSC class witness (NOT 7717-specific) ──────────────────────────
//
// The owner's 2026-09-03 question — "do we have probes that let us see what
// the CLI emits for OSC?" — had no witness anywhere: DECSETs are observed at
// the mount-script boundary, but OSC sequences passed through unwitnessed on
// every path. This is the daemon-side answer, beside the declare scanner
// whose per-chunk discipline it shares: one pass over chunks that contain
// ESC, classes only (a title carries the cwd, a hyperlink carries the URL —
// both stay in the chunk, only the id travels), first-sight-per-reader
// emission onto the ytrace bus as `cli/osc_witness`.
//
// ⛔ Honest limitations, stated not buried: a numeric opener whose digits run
// to the chunk's end is HELD, not classified (the id may be split); a
// non-numeric opener is skipped, not witnessed (exotic, and a stray `ESC ]`
// in binary output must not become a false `other`). A reader restart
// (park/handover) resets the seen-set — a restart is a new observation
// epoch, and a re-emitted class after one says the CLI re-emitted it.

/// Map a numeric OSC id to its witness class. Content-free by construction.
fn osc_witness_class(id: u32) -> &'static str {
    match id {
        0 | 1 | 2 => "title",
        4 | 10 | 11 | 12 => "color",
        8 => "hyperlink",
        52 => "clipboard",
        9 | 99 | 777 => "notify",
        133 => "shell_integration",
        7717 => "app_declare",
        _ => "other",
    }
}

/// What a trailing chunk-suffix must look like to be retained as a possibly
/// split OSC opener (`ESC`, `ESC ]`, or `ESC ]` + digits, nothing else).
fn is_split_opener_suffix(suffix: &str) -> bool {
    let bytes = suffix.as_bytes();
    let (mut i, n) = (0, bytes.len());
    if n == 0 || bytes[0] != 0x1b {
        return false;
    }
    i += 1;
    if i < n {
        if bytes[i] != b']' {
            return false;
        }
        i += 1;
    }
    while i < n {
        if !bytes[i].is_ascii_digit() {
            return false;
        }
        i += 1;
    }
    true
}

#[derive(Debug, Default)]
pub struct OscWitness {
    seen: std::collections::HashSet<&'static str>,
    pending: String,
}

impl OscWitness {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe one decoded PTY chunk; emit `cli/osc_witness` for every OSC
    /// class this reader sees for the first time, and return those classes
    /// so tests assert state rather than bus traffic. Both maps stay tiny:
    /// at most eight classes ever enter `seen`, and `pending` holds only an
    /// incomplete trailing opener (a few bytes).
    pub fn observe(&mut self, path: &str, data: &str) -> Vec<&'static str> {
        if data.is_empty() && self.pending.is_empty() {
            return Vec::new();
        }
        if !data.contains('\x1b') && self.pending.is_empty() {
            return Vec::new();
        }
        self.pending.push_str(data);
        let bytes = self.pending.as_bytes();
        let mut found: Vec<&'static str> = Vec::new();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == 0x1b && bytes[i + 1] == b']' {
                let mut j = i + 2;
                let mut id: u32 = 0;
                let mut digits = 0;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    id = id
                        .saturating_mul(10)
                        .saturating_add((bytes[j] - b'0') as u32);
                    digits += 1;
                    j += 1;
                }
                if digits == 0 {
                    i += 2;
                    continue;
                }
                if j >= bytes.len() {
                    // Digits run to the buffer's end: the id may be split
                    // across the chunk boundary. Stop: the tail policy
                    // below retains the opener for the next chunk.
                    break;
                }
                let class = osc_witness_class(id);
                if !found.contains(&class) {
                    found.push(class);
                }
                i = j;
            } else {
                i += 1;
            }
        }
        // Retain only what is still classifiable: a trailing suffix that
        // could still become an OSC opener. Bound by construction (≤16
        // bytes); anything else is drained so `pending` can never grow with
        // the stream. Slicing at the `rfind` hit is char-boundary safe
        // (ESC is single-byte ASCII), and `is_split_opener_suffix` only
        // byte-compares, so no multibyte split can panic here.
        let tail_start = self.pending.rfind('\x1b').unwrap_or(self.pending.len());
        let tail = &self.pending[tail_start..];
        self.pending = if tail.len() <= 16 && is_split_opener_suffix(tail) {
            tail.to_string()
        } else {
            String::new()
        };
        if found.is_empty() {
            return Vec::new();
        }
        let kind = yggterm_core::agent_scheme::session_kind_for_path(path)
            .map(yggterm_core::agent_cli::session_kind_label);
        let mut fresh = Vec::new();
        for class in found {
            if self.seen.insert(class) {
                fresh.push(class);
                yggterm_core::perf::ytrace_emit_event(
                    "daemon",
                    "cli",
                    "osc_witness",
                    serde_json::json!({
                        "session_path": path,
                        "kind": kind,
                        "osc_class": class,
                    }),
                );
            }
        }
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osc(verb: &str, action: &str, payload: serde_json::Value) -> String {
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(payload.to_string().as_bytes());
        format!("\x1b]7717;{verb};{action};{encoded}\x07")
    }

    /// The OSC gap, closed: classes travel, parameters never do — and a
    /// split opener across the chunk boundary is held, not misclassified.
    #[test]
    fn osc_witness_reports_classes_once_and_never_parameters() {
        let mut witness = OscWitness::new();
        // Ordinary output: nothing.
        assert!(witness.observe("opencode-runtime://s", "hello\r\nworld\r\n").is_empty());
        // Title + hyperlink in one chunk: both classes, no cwd, no URL.
        let fresh = witness.observe(
            "opencode-runtime://s",
            "hi\x1b]0;/home/user/secret-proj\x07mid\x1b]8;;https://internal.example/x\x1b\\done",
        );
        assert_eq!(fresh, vec!["title", "hyperlink"]);
        // Second sight is silent (edge-triggered per reader).
        assert!(witness
            .observe("opencode-runtime://s", "\x1b]0;another\x07")
            .is_empty());
        // A new class still speaks.
        assert_eq!(
            witness.observe("opencode-runtime://s", "\x1b]52;c;QUJD\x07"),
            vec!["clipboard"]
        );
    }

    #[test]
    fn osc_witness_holds_a_split_opener_instead_of_misclassifying_it() {
        let mut witness = OscWitness::new();
        // `771` alone must not read as `other`: the id may continue.
        assert!(witness.observe("opencode-runtime://s", "out\x1b]77").is_empty());
        // Continuation completes 7717 → app_declare, exactly once.
        assert_eq!(
            witness.observe("opencode-runtime://s", "17;web-surface;open;e30=\x07"),
            vec!["app_declare"]
        );
    }

    #[test]
    fn scanner_extracts_a_declare_and_leaves_ordinary_output_alone() {
        let mut scanner = AppDeclareScanner::new();
        let stream = format!(
            "hello\r\n{}world\r\n",
            osc(
                "web-surface",
                "open",
                serde_json::json!({"session": "s", "url": "https://example.test/"}),
            )
        );
        let messages = scanner.scan(&stream);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].verb, "web-surface");
        assert_eq!(messages[0].action, "open");
        assert_eq!(messages[0].payload["url"], "https://example.test/");
    }

    // The PTY reader hands over arbitrary chunk boundaries; a declare cut in
    // half must still arrive, or "the app declared" becomes a coin flip on
    // read() sizes.
    #[test]
    fn scanner_reassembles_a_sequence_split_across_chunks() {
        let full = osc(
            "sidebar",
            "declare",
            serde_json::json!({"session": "s", "control": "http://127.0.0.1:1/"}),
        );
        for cut in 1..full.len() {
            if !full.is_char_boundary(cut) {
                continue;
            }
            let mut scanner = AppDeclareScanner::new();
            assert!(scanner.scan(&full[..cut]).is_empty(), "cut {cut} too eager");
            let messages = scanner.scan(&full[cut..]);
            assert_eq!(messages.len(), 1, "cut {cut} lost the declare");
            assert_eq!(messages[0].verb, "sidebar");
        }
    }

    #[test]
    fn scanner_accepts_the_st_terminator_and_back_to_back_sequences() {
        let mut scanner = AppDeclareScanner::new();
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"{\"session\":\"s\"}");
        let stream = format!(
            "\x1b]7717;web-surface;heartbeat;{encoded}\x1b\\\x1b]7717;web-surface;close;{encoded}\x07"
        );
        let messages = scanner.scan(&stream);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].action, "heartbeat");
        assert_eq!(messages[1].action, "close");
    }

    #[test]
    fn scanner_drops_a_runaway_partial_instead_of_growing_forever() {
        let mut scanner = AppDeclareScanner::new();
        assert!(scanner.scan("\x1b]7717;web-surface;open;").is_empty());
        assert!(scanner.scan(&"A".repeat(MAX_PENDING_BYTES + 16)).is_empty());
        assert!(scanner.pending.is_empty(), "runaway buffer must be dropped");
    }

    // A fido2 ceremony asks for a human's presence — a retained copy could be
    // replayed with nobody at the keyboard, so it must never even be parsed
    // into the log.
    #[test]
    fn a_fido2_request_is_never_retained() {
        let mut scanner = AppDeclareScanner::new();
        let messages = scanner.scan(&osc(
            "fido2",
            "request",
            serde_json::json!({"session": "s", "rp_id": "example.test"}),
        ));
        assert!(messages.is_empty());
    }

    // The daemon's own attach-serve word must never become retained state: a
    // `seen` fed back (a hostile or confused app echoing it) would overwrite
    // the record's live action with "already consumed".
    #[test]
    fn a_seen_action_is_never_retained() {
        let mut scanner = AppDeclareScanner::new();
        let messages = scanner.scan(&osc(
            "web-surface",
            "seen",
            serde_json::json!({"session": "s", "url": "https://example.test/"}),
        ));
        assert!(messages.is_empty(), "`seen` is not an app's word to say");
    }

    // The replay-neutralization policy, exactly: a record still on `open` is
    // the just-launched sliver and serves verbatim; everything else (a
    // heartbeating run, a cleared record) is consumed history.
    #[test]
    fn only_a_record_still_on_open_keeps_the_replayed_open_verbatim() {
        assert!(attach_replay_neutralizes_web_surface_open(Some("heartbeat")));
        assert!(attach_replay_neutralizes_web_surface_open(None));
        assert!(!attach_replay_neutralizes_web_surface_open(Some("open")));
    }

    // The rewrite is a same-length action swap and nothing else: every other
    // byte — heartbeats, closes, ordinary output — is untouched, and a stream
    // with no consumed open reports None so the caller keeps its bytes.
    #[test]
    fn the_open_rewrite_swaps_only_the_action_and_never_moves_a_byte() {
        let payload = serde_json::json!({"session": "s", "url": "https://example.test/"});
        let stream = format!(
            "hello\r\n{}world{}\r\n{}",
            osc("web-surface", "open", payload.clone()),
            osc("web-surface", "heartbeat", payload.clone()),
            osc("web-surface", "open", payload),
        );
        let (rewritten, count) =
            rewrite_consumed_web_surface_opens(&stream).expect("two opens to rewrite");
        assert_eq!(count, 2);
        assert_eq!(
            rewritten.len(),
            stream.len(),
            "the swap must be same-length so chunk boundaries keep their meaning"
        );
        assert!(!rewritten.contains(WEB_SURFACE_OPEN_SEQUENCE));
        assert_eq!(
            rewritten.matches(WEB_SURFACE_SEEN_SEQUENCE).count(),
            2,
            "both consumed opens serve as seen"
        );
        assert!(
            rewritten.contains("\x1b]7717;web-surface;heartbeat;"),
            "a heartbeat is already liveness and serves verbatim"
        );
        assert!(rewritten.starts_with("hello\r\n") && rewritten.contains("world"));

        assert_eq!(
            rewrite_consumed_web_surface_opens("plain output, no declares"),
            None,
            "a stream without a consumed open keeps its original bytes"
        );
    }

    #[test]
    fn log_keeps_the_latest_per_verb_and_a_close_clears_it() {
        let mut log = AppDeclareLog::new();
        log.ingest(
            AppDeclareMessage {
                verb: "web-surface".to_string(),
                action: "open".to_string(),
                payload: serde_json::json!({"url": "https://one.test/"}),

            },
            10,
        );
        log.ingest(
            AppDeclareMessage {
                verb: "web-surface".to_string(),
                action: "heartbeat".to_string(),
                payload: serde_json::json!({"url": "https://two.test/"}),
            },
            20,
        );
        let records = log.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].payload["url"], "https://two.test/");
        assert_eq!(records[0].at_ms, 20);
        assert_eq!(records[0].seq, 2, "a refresh must advance the seq");

        log.ingest(
            AppDeclareMessage {
                verb: "web-surface".to_string(),
                action: "close".to_string(),
                payload: serde_json::Value::Null,
            },
            30,
        );
        assert!(
            log.records().is_empty(),
            "a deliberate close must leave nothing to rebuild from"
        );
    }

    /// The heartbeat that made 27 daemons immortal.
    ///
    /// The literal bytes are the ones measured coming off a ychrome launcher
    /// PTY on `dev` (2026-08-10), payload replaced with an invented one.
    #[test]
    fn a_lone_app_declare_heartbeat_is_not_session_activity() {
        let payload = base64::engine::general_purpose::STANDARD
            .encode(br#"{"session":"local://11111111-2222-3333-4444-555555555555","url":"https://example.test/"}"#);
        let beat = format!("\x1b]7717;web-surface;pick;{payload}\x07");
        assert!(
            super::chunk_is_only_app_declares(&beat),
            "our own ~4s control heartbeat must not move the session idle clock"
        );
        // `pick` is Retention::Ignore, and that must not change the verdict:
        // the question is whose bytes these are, not whether we retain them.
        let stored = format!("\x1b]7717;web-surface;heartbeat;{payload}\x07");
        assert!(super::chunk_is_only_app_declares(&stored));
        // ST-terminated is the same sequence in its other legal spelling.
        let st = format!("\x1b]7717;web-surface;pick;{payload}\x1b\\");
        assert!(super::chunk_is_only_app_declares(&st));
        // Two beats coalesced into one read.
        assert!(super::chunk_is_only_app_declares(&format!("{beat}{beat}")));
    }

    /// ⛔ The one-directional bias. Each of these MUST count as activity —
    /// discounting any of them would let the gate cold-shutdown a live turn.
    #[test]
    fn anything_that_is_not_purely_our_own_control_traffic_is_activity() {
        let payload = base64::engine::general_purpose::STANDARD.encode(br#"{"a":1}"#);
        let beat = format!("\x1b]7717;web-surface;pick;{payload}\x07");

        for (data, why) in [
            ("", "an empty chunk has nothing to discount"),
            ("hello\n", "plain output"),
            ("\x1b[2K\x1b[G", "a spinner frame is pure control and IS the agent working"),
            ("\x1b]0;a title\x07", "somebody else's OSC is not ours to discount"),
            ("\x1b]7717;web-surface;pick;dGVzdA==", "an unterminated tail cannot be proven ours"),
        ] {
            assert!(
                !super::chunk_is_only_app_declares(data),
                "must count as activity: {why}"
            );
        }

        assert!(
            !super::chunk_is_only_app_declares(&format!("output {beat}")),
            "output before a declare is still output"
        );
        assert!(
            !super::chunk_is_only_app_declares(&format!("{beat} output")),
            "output after a declare is still output"
        );
    }
}
