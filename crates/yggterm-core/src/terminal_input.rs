//! How a text payload reaches a terminal's composer — the ONE owner.
//!
//! **The fact this module exists for, measured live on jojo 2026-08-07 against a
//! real Claude Code TUI.** A payload with interior line breaks is written to the
//! PTY as `line\r line\r line\r`, and an agent CLI's composer treats every `\r`
//! as Enter. So a three-line brief becomes:
//!
//! ```text
//! ALPHA one       -> SUBMITTED, the agent starts a turn on this fragment alone
//! BRAVO two       -> "Press up to edit queued messages"
//! CHARLIE three   -> queued
//! ```
//!
//! From outside that reads as *"only line 1 was delivered"*, and it is worse
//! than truncation: the agent acts on a one-line fragment (usually the brief's
//! title) while the body arrives later as unrelated follow-up turns. Into a
//! plain shell the same payload is correct — N commands — which is why this is
//! keyed to the session KIND and not to the payload alone.
//!
//! **The remedy, proven in the same measurement.** Bracketed paste
//! (`ESC[200~ … ESC[201~`) puts the CLI's reader into paste mode, where a `\r`
//! is a soft newline in the composer instead of a submit; a single discrete
//! `\r` afterwards submits the whole block as ONE message. Verified: the
//! composer held all three lines unsubmitted, then submitted them together.
//!
//! ⚠ **Paste mode is the RECEIVER's state, not a property of one `write()`.**
//! `ESC[200~` may arrive in a different write than the text it opens; the
//! parser stays in paste mode until `ESC[201~`. That is why
//! [`is_bracketed_paste_block`] asks about the payload as a whole.

/// Opens bracketed paste — the receiving CLI stops treating `\r` as submit.
pub const BRACKETED_PASTE_BEGIN: &str = "\u{1b}[200~";
/// Closes bracketed paste.
pub const BRACKETED_PASTE_END: &str = "\u{1b}[201~";

/// The named refusal for a raw multi-line `terminal send` into an agent CLI.
///
/// Named rather than silent because the whole defect is that the old behaviour
/// looked like success: `accepted: true` with a `chunk_count` that quietly says
/// how many separate Enters were fired.
pub const MULTILINE_SEND_REFUSAL_REASON: &str = "multiline_send_into_agent_cli";

/// What the caller should do instead, in the caller's own terms.
pub const MULTILINE_SEND_REFUSAL_DETAIL: &str =
    "a payload with interior line breaks reaches an agent CLI composer as one Enter PER LINE: \
     line 1 is submitted on its own and the rest become queued messages. Use \
     `server app terminal submit <session> --stdin` (readiness-gated, delivers the whole block \
     as one message), or pass --allow-multiline to send it as separate submits deliberately.";

/// Whether this payload carries a line break that is not its final character.
///
/// A TRAILING newline is not interior — `"text\n"` is one line plus Enter, the
/// ordinary single-line send. `"a\nb"` is two submits and is what this catches.
/// `\r\n` counts once, because the PTY normaliser collapses it to a single `\r`.
pub fn payload_has_interior_line_break(data: &str) -> bool {
    data.trim_end_matches(['\r', '\n'])
        .contains(['\r', '\n'])
}

/// Wrap a payload so an agent CLI's composer receives its newlines as SOFT
/// newlines. The caller still owes a discrete `\r` afterwards to submit.
///
/// Idempotent: a payload that is already a bracketed-paste block is returned
/// unchanged, so a caller that wraps twice does not nest the markers (which the
/// receiver would render literally).
pub fn wrap_as_bracketed_paste(data: &str) -> String {
    if is_bracketed_paste_block(data) {
        return data.to_string();
    }
    format!("{BRACKETED_PASTE_BEGIN}{data}{BRACKETED_PASTE_END}")
}

/// Whether this payload is a bracketed-paste block.
///
/// Consumed by the write path to keep such a block in ONE chunk: the per-line
/// chunker exists to pace Enters, and there are no Enters inside a paste.
pub fn is_bracketed_paste_block(data: &str) -> bool {
    data.starts_with(BRACKETED_PASTE_BEGIN) && data.ends_with(BRACKETED_PASTE_END)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_newline_is_not_an_interior_break() {
        // The ordinary send: one line, then Enter. Refusing this would break
        // every single-line drive in the fleet.
        assert!(!payload_has_interior_line_break("echo hi\n"));
        assert!(!payload_has_interior_line_break("echo hi\r"));
        assert!(!payload_has_interior_line_break("echo hi\r\n"));
        assert!(!payload_has_interior_line_break("echo hi"));
        assert!(!payload_has_interior_line_break("\r"));
        assert!(!payload_has_interior_line_break(""));
    }

    #[test]
    fn an_interior_break_is_what_becomes_a_second_enter() {
        assert!(payload_has_interior_line_break("ALPHA one\nBRAVO two"));
        assert!(payload_has_interior_line_break("ALPHA one\nBRAVO two\n"));
        assert!(payload_has_interior_line_break("ALPHA one\r\nBRAVO two\r\n"));
        // A blank line INSIDE a brief is the common shape and must be caught —
        // this is the paragraph break in every runbook.
        assert!(payload_has_interior_line_break("ALPHA one\n\nBRAVO two"));
    }

    // Trailing Enters are not the defect this refuses, and pretending otherwise
    // would refuse a payload that behaves fine: a second Enter lands on a composer
    // the first one already emptied, so it submits nothing.
    #[test]
    fn repeated_trailing_enters_are_still_not_interior() {
        assert!(!payload_has_interior_line_break("ALPHA one\n\n"));
        assert!(!payload_has_interior_line_break("ALPHA one\r\r\n"));
    }

    #[test]
    fn wrapping_is_idempotent_so_a_double_wrap_cannot_nest_markers() {
        let once = wrap_as_bracketed_paste("a\rb");
        assert_eq!(once, format!("{BRACKETED_PASTE_BEGIN}a\rb{BRACKETED_PASTE_END}"));
        assert_eq!(wrap_as_bracketed_paste(&once), once);
        assert!(is_bracketed_paste_block(&once));
    }

    // A half-open block is NOT a block: `ESC[200~` with no terminator leaves the
    // receiver in paste mode forever, swallowing the submit Enter. The write path
    // must keep treating such a payload as ordinary text.
    #[test]
    fn a_half_open_paste_is_not_a_block() {
        assert!(!is_bracketed_paste_block(&format!("{BRACKETED_PASTE_BEGIN}a\rb")));
        assert!(!is_bracketed_paste_block(&format!("a\rb{BRACKETED_PASTE_END}")));
        assert!(!is_bracketed_paste_block("a\rb"));
    }

    // The refusal must name the verb that WORKS, not just say no. A refusal that
    // does not carry the alternative is how a caller learns to pass the escape
    // hatch instead of fixing the call.
    #[test]
    fn the_refusal_names_the_verb_that_works_and_the_escape_hatch() {
        assert!(MULTILINE_SEND_REFUSAL_DETAIL.contains("terminal submit"));
        assert!(MULTILINE_SEND_REFUSAL_DETAIL.contains("--allow-multiline"));
    }
}
