//! The reveal cover: hold live PTY bytes for one reveal window so a
//! position-addressed TUI repaint never paints fragments over a curing
//! canvas.
//!
//! Owner steer 2026-09-04 ("no ghost frames"): the mid-cure composite —
//! measured 2026-09-03 on a remote opencode switch-in — is the defect: the
//! repaint nudge ([11.39]) wakes an idle fullscreen TUI, and the TUI's
//! addressed fragments (`53;6H`-style moves + full SGR) arrive while the
//! client canvas is still blank/settling, painting duplicates and black
//! bands that no later frame fully repairs. The remedy the entry files as
//! the pixel-paint cache, in write form: from the nudge emit until the
//! TUI's first full post-nudge frame, HOLD the live bytes instead of
//! painting them, then flush in one write — the canvas goes
//! blank → full frame, never blank → fragments.
//!
//! Release is deterministic, never guessed: the first hold whose cumulative
//! bytes reach one full screen area (cols×rows — a TUI redraw after a
//! SIGWINCH is at least that), or a fixed deadline (the TUI's repaint may
//! be slow over ssh; the bytes flush regardless, nothing is dropped), or a
//! newer arm superseding the old one. The gate never arms outside a
//! reveal: every arming site is a repaint-nudge emit, and the hold window
//! is bounded so keystroke echo can be delayed by at most one deadline.

/// How long held bytes may wait for the full frame before flushing anyway.
/// A TUI repaint over ssh measured well under this; the cap exists so a
/// TUI that never repaints cannot hold the reveal hostage.
pub(crate) const REVEAL_COVER_DEADLINE_MS: u64 = 1000;

#[derive(Debug, Default)]
pub(crate) struct RevealCoverGate {
    /// 0 = disarmed. Each arm takes the next monotonic generation so a
    /// stale deadline release can never flush a newer hold early — the
    /// counter must NOT reset on release, or two consecutive windows would
    /// share a number and the first window's late deadline would release
    /// the second.
    armed_gen: u64,
    last_gen: u64,
    armed_at_ms: u64,
    threshold_bytes: usize,
    buffer: String,
}

impl RevealCoverGate {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_holding(&self) -> bool {
        self.armed_gen != 0
    }

    /// Arm one hold window. Returns the generation the matching deadline
    /// release must carry.
    pub(crate) fn arm(&mut self, now_ms: u64, cols: u16, rows: u16) -> u64 {
        self.last_gen = self.last_gen.wrapping_add(1);
        self.armed_gen = self.last_gen;
        self.armed_at_ms = now_ms;
        self.threshold_bytes = usize::from(cols) * usize::from(rows);
        self.buffer.clear();
        self.armed_gen
    }

    /// Buffer one live write while holding. Returns whether the hold must
    /// release now: the write pushed the buffer past one full screen area,
    /// or the deadline elapsed.
    pub(crate) fn hold(&mut self, data: &str, now_ms: u64) -> bool {
        if !self.is_holding() {
            return false;
        }
        self.buffer.push_str(data);
        self.full_frame_ready() || self.deadline_elapsed(now_ms)
    }

    /// The TUI's full frame has arrived: every byte of one screen area is
    /// in the buffer.
    pub(crate) fn full_frame_ready(&self) -> bool {
        self.is_holding() && self.buffer.len() >= self.threshold_bytes
    }

    pub(crate) fn deadline_elapsed(&self, now_ms: u64) -> bool {
        self.is_holding() && now_ms.saturating_sub(self.armed_at_ms) >= REVEAL_COVER_DEADLINE_MS
    }

    /// A deadline message for `gen` arrives off-loop: only the CURRENT
    /// generation may release through it.
    pub(crate) fn release_if_gen(&mut self, generation: u64, now_ms: u64) -> bool {
        self.is_holding() && self.armed_gen == generation && self.deadline_elapsed(now_ms)
    }

    /// Take the held bytes and disarm. Empty when nothing was held.
    pub(crate) fn take_flush(&mut self) -> String {
        self.armed_gen = 0;
        self.armed_at_ms = 0;
        self.threshold_bytes = 0;
        std::mem::take(&mut self.buffer)
    }

    #[cfg(test)]
    pub(crate) fn held_len(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLS: u16 = 80;
    const ROWS: u16 = 24;
    const FULL: usize = 80 * 24;

    #[test]
    fn a_disarmed_gate_holds_nothing() {
        let mut gate = RevealCoverGate::new();
        assert!(!gate.is_holding());
        assert!(!gate.hold("\x1b[1;1Hhello", 1000));
        assert_eq!(gate.held_len(), 0);
        assert!(gate.take_flush().is_empty());
    }

    #[test]
    fn addressed_fragments_are_held_until_one_full_screen_of_bytes_arrives() {
        let mut gate = RevealCoverGate::new();
        let generation = gate.arm(1000, COLS, ROWS);
        assert_eq!(generation, 1);
        // The TUI's first addressed fragments — exactly the mid-cure
        // composite's bytes — must NOT flush.
        assert!(!gate.hold("\x1b[53;6H\x1b[38;2;10;10;10m▌", 1100));
        assert!(!gate.full_frame_ready());
        assert_eq!(gate.held_len(), 26, "held bytes count UTF-8, the ▌ is 3");
        // The TUI's full redraw crosses the screen-area threshold: flush.
        let big_frame = "x".repeat(FULL);
        assert!(gate.hold(&big_frame, 1150));
        assert!(gate.full_frame_ready());
        let flushed = gate.take_flush();
        assert_eq!(flushed.len(), 26 + FULL);
        assert!(!gate.is_holding(), "release disarms");
    }

    #[test]
    fn the_deadline_releases_a_hold_that_never_saw_a_full_frame() {
        let mut gate = RevealCoverGate::new();
        gate.arm(1000, COLS, ROWS);
        assert!(!gate.hold("tiny", 1500), "before the deadline: still held");
        assert!(gate.hold("more", 1000 + REVEAL_COVER_DEADLINE_MS));
        let flushed = gate.take_flush();
        assert_eq!(flushed, "tinymore");
        assert!(!gate.is_holding());
    }

    #[test]
    fn a_stale_deadline_generation_cannot_release_a_newer_hold() {
        let mut gate = RevealCoverGate::new();
        let first = gate.arm(1000, COLS, ROWS);
        gate.take_flush(); // first window released by its full frame
        gate.arm(2000, COLS, ROWS);
        // The first window's deadline task fires late.
        assert!(
            !gate.release_if_gen(first, 2000 + REVEAL_COVER_DEADLINE_MS),
            "a superseded generation must not flush the newer hold"
        );
        assert!(gate.is_holding());
        assert!(gate.release_if_gen(2, 2000 + REVEAL_COVER_DEADLINE_MS));
    }

    #[test]
    fn rearming_clears_the_previous_windows_bytes() {
        let mut gate = RevealCoverGate::new();
        gate.arm(1000, COLS, ROWS);
        gate.hold("stale fragments", 1050);
        gate.arm(2000, COLS, ROWS);
        assert_eq!(gate.held_len(), 0, "a new reveal starts from a clean hold");
    }
}
