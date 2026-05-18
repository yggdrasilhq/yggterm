use crate::terminal_write_policy::terminal_output_contains_codex_welcome_surface;

const TERMINAL_FRAME_BRIDGE_PENDING_MAX_BYTES: usize = 256 * 1024;

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

    pub(crate) fn flush_due(&mut self, now_ms: u64) -> Option<String> {
        if self.pending.is_empty() {
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
