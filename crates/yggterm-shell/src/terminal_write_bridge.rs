use crate::terminal_write_policy::{
    terminal_output_contains_codex_welcome_surface,
    terminal_output_ends_inside_synchronized_frame,
};

const TERMINAL_FRAME_BRIDGE_PENDING_MAX_BYTES: usize = 256 * 1024;

/// Hard upper bound on how long the bridge will hold a flush waiting for a
/// synchronized-update region to close (`\e[?2026l`). A real codex frame closes
/// within a frame or two; this only bites if the ESU never arrives (codex died
/// mid-frame, dropped bytes) — without it a torn-but-stuck buffer would stall
/// forever. Generous vs. the ~16ms frame budget so it never clips a real frame.
const TERMINAL_SYNC_FRAME_MAX_HOLD_MS: u64 = 250;

#[derive(Debug)]
pub(crate) struct TerminalWriteBridge {
    frame_ms: u64,
    pending: String,
    last_frame_flush_ms: u64,
    pending_started_ms: u64,
    alt_screen_active: bool,
    cursor_hidden_active: bool,
}

impl TerminalWriteBridge {
    pub(crate) fn new(frame_ms: u64) -> Self {
        Self {
            frame_ms,
            pending: String::new(),
            last_frame_flush_ms: 0,
            pending_started_ms: 0,
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

        if self.pending.is_empty() {
            self.pending_started_ms = now_ms;
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

    pub(crate) fn flush_due(&mut self, now_ms: u64) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        // Synchronized-output atomicity: if the buffer ends mid-frame (an open
        // \e[?2026h with no matching \e[?2026l), hold the flush so xterm never
        // paints a torn frame — the vendored xterm.js doesn't implement mode
        // 2026, so the bridge enforces it. Bounded by TERMINAL_SYNC_FRAME_MAX_HOLD_MS
        // so a never-closing frame (codex died mid-repaint) can't stall forever.
        if terminal_output_ends_inside_synchronized_frame(&self.pending)
            && now_ms.saturating_sub(self.pending_started_ms) < TERMINAL_SYNC_FRAME_MAX_HOLD_MS
        {
            return None;
        }
        if self.last_frame_flush_ms == 0
            || now_ms.saturating_sub(self.last_frame_flush_ms) >= self.frame_ms
        {
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
