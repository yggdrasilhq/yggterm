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

#[cfg(test)]
mod tests {
    use super::*;

    fn osc(verb: &str, action: &str, payload: serde_json::Value) -> String {
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(payload.to_string().as_bytes());
        format!("\x1b]7717;{verb};{action};{encoded}\x07")
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
}
