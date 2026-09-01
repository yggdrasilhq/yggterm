// The mouse-mode probe — the observability witness for DECSET mouse tracking
// ([[spec opencode ACT V]], docs/pending-bugs.md "A DEAD SESSION'S SURFACE
// ECHOES MOUSE-TRACKING AS TEXT").
//
// WHY THIS EXISTS: mouse tracking (DECSET 1000/1002/1003/1006) is parsed and
// honored ENTIRELY inside the vendored xterm.js — yggterm has never had a
// witness at that parse boundary. The symptom "clicks do nothing" was
// undebuggable because nothing distinguished "the TUI never asked for mouse
// events" from "xterm never armed the mode" from "the mode armed but the
// events were dropped". A witness at the parse boundary turns the symptom
// into a fact stream: every mode transition the client actually parsed,
// with the session it belongs to.
//
// THE CONTRACT (observer-only):
// - The CSI handler MUST return `false` — the probe never consumes the
//   sequence; xterm.js applies the mode exactly as before. A probe that
//   changed behavior would violate the observer rule.
// - Only transitions are reported: the JS glue keeps the last-reported state
//   per mode and stays silent on re-assertions. Scrollback replay re-parses
//   old DECSETs in order, so without dedup one 10k-row replay would spam one
//   event per historical transition; with dedup, replay is silent unless the
//   final state differs from what was last reported.
// - The event carries the mode number and the enabled bit and NOTHING about
//   screen content — content-free telemetry, same discipline as the rest of
//   the trace plane.

/// The private DECSET modes this probe witnesses. Exactly the set ACT V
/// names: 1000 (normal tracking), 1002 (button-event tracking), 1003
/// (any-event tracking), 1006 (SGR pixel encoding). 1005/1015 (URXVT
/// encodings) are deliberately NOT witnessed — they are legacy encodings,
/// and a probe that widens its set silently is a probe nobody audits.
pub(crate) const WITNESSED_MOUSE_MODES: [u16; 4] = [1000, 1002, 1003, 1006];

/// Whether a DECSET private mode is one this probe reports.
pub(crate) fn is_witnessed(mode: u16) -> bool {
    WITNESSED_MOUSE_MODES.contains(&mode)
}

/// The trace event name the shell-side arm writes. A const so the guard test
/// and the Rust arm cannot drift apart silently.
pub(crate) const TRACE_EVENT_NAME: &str = "mouse_mode_probe";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn witnesses_exactly_the_act_v_modes() {
        for mode in [1000, 1002, 1003, 1006] {
            assert!(is_witnessed(mode), "{mode} must be witnessed");
        }
        for mode in [999, 1001, 1004, 1005, 1015, 1016, 2004] {
            assert!(!is_witnessed(mode), "{mode} must NOT be witnessed");
            // 2004 is bracketed paste: real, but a different probe's problem.
        }
    }

    #[test]
    fn mode_table_has_no_duplicates() {
        let mut seen = WITNESSED_MOUSE_MODES.to_vec();
        seen.dedup();
        assert_eq!(seen.len(), WITNESSED_MOUSE_MODES.len());
    }

    #[test]
    fn trace_event_name_is_stable() {
        assert_eq!(TRACE_EVENT_NAME, "mouse_mode_probe");
    }
}
