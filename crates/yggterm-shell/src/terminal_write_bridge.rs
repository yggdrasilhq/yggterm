use crate::terminal_write_policy::terminal_output_contains_codex_welcome_surface;

const TERMINAL_FRAME_BRIDGE_PENDING_MAX_BYTES: usize = 256 * 1024;

/// Coalesces terminal output into frame-budget batches before handing it to the
/// webview's `term.write`. This is now a PURE IPC batcher: it only reduces the
/// number of Rust→JS eval round-trips on bursty output.
///
/// Synchronized-output (DEC mode 2026) atomicity is no longer the bridge's job —
/// the vendored xterm.js 6 implements mode 2026 natively, so xterm buffers each
/// `\e[?2026h…\e[?2026l` frame itself and never paints a partial. The old
/// hold-and-guess (hold a buffer ending mid-frame, flush only the complete-frame
/// prefix, 250ms last-resort cap) stood in for that missing feature and was the
/// source of the long-running-turn blink/freeze: once the frame budget relaxed
/// (250–500ms) it batched a large run of frames and could paint a torn one. With
/// native 2026 the bridge may flush mid-frame freely — xterm reassembles the open
/// frame from the next write and defers the paint to the closing ESU.
#[derive(Debug)]
pub(crate) struct TerminalWriteBridge {
    frame_ms: u64,
    pending: String,
    last_frame_flush_ms: u64,
    alt_screen_active: bool,
    cursor_hidden_active: bool,
}

impl TerminalWriteBridge {
    pub(crate) fn new(frame_ms: u64) -> Self {
        Self {
            frame_ms,
            pending: String::new(),
            last_frame_flush_ms: 0,
            alt_screen_active: false,
            cursor_hidden_active: false,
        }
    }

    pub(crate) fn set_frame_ms(&mut self, frame_ms: u64) {
        self.frame_ms = frame_ms;
    }

    pub(crate) fn stage_or_immediate(
        &mut self,
        data: String,
        now_ms: u64,
        frame_budget: bool,
    ) -> Vec<String> {
        let enters_alt_screen = data.contains("\x1b[?1049h");
        let exits_alt_screen = data.contains("\x1b[?1049l");
        let hides_cursor = data.contains("\x1b[?25l");
        let shows_cursor = data.contains("\x1b[?25h");
        let frame_mode_was_active = self.alt_screen_active || self.cursor_hidden_active;
        if enters_alt_screen {
            self.alt_screen_active = true;
        }
        if hides_cursor {
            self.cursor_hidden_active = true;
        }
        let frame_mode_active =
            self.alt_screen_active || self.cursor_hidden_active || frame_mode_was_active;
        let effective_frame_budget = frame_budget
            || (frame_mode_active
                && !exits_alt_screen
                && !terminal_output_contains_codex_welcome_surface(&data));

        if !effective_frame_budget || self.frame_ms == 0 {
            let mut writes = Vec::new();
            if let Some(pending) = self.flush_all(now_ms) {
                writes.push(pending);
            }
            writes.push(data);
            if exits_alt_screen {
                self.alt_screen_active = false;
                self.cursor_hidden_active = false;
            } else if shows_cursor && !hides_cursor && !self.alt_screen_active {
                self.cursor_hidden_active = false;
            }
            return writes;
        }

        self.pending.push_str(&data);
        if self.pending.len() > TERMINAL_FRAME_BRIDGE_PENDING_MAX_BYTES {
            return self.flush_all(now_ms).into_iter().collect();
        }

        let writes: Vec<String> = self.flush_due(now_ms).into_iter().collect();
        if exits_alt_screen {
            self.alt_screen_active = false;
            self.cursor_hidden_active = false;
        } else if shows_cursor && !hides_cursor && !self.alt_screen_active {
            self.cursor_hidden_active = false;
        }
        writes
    }

    /// Flush the staged buffer once the frame budget elapses. No synchronized-frame
    /// inspection: xterm.js 6 reassembles `\e[?2026h…\e[?2026l` frames itself, so a
    /// flush that ends mid-frame is harmless (xterm holds the paint until the ESU).
    pub(crate) fn flush_due(&mut self, now_ms: u64) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let frame_due = self.last_frame_flush_ms == 0
            || now_ms.saturating_sub(self.last_frame_flush_ms) >= self.frame_ms;
        if frame_due {
            return self.flush_all(now_ms);
        }
        None
    }

    fn flush_all(&mut self, now_ms: u64) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        self.last_frame_flush_ms = now_ms;
        Some(std::mem::take(&mut self.pending))
    }

    pub(crate) fn frame_budget_mode_active(&self) -> bool {
        !self.pending.is_empty() || self.alt_screen_active || self.cursor_hidden_active
    }

    pub(crate) fn frame_ms(&self) -> u64 {
        self.frame_ms
    }

    #[cfg(test)]
    pub(crate) fn pending_for_test(&self) -> &str {
        &self.pending
    }
}
