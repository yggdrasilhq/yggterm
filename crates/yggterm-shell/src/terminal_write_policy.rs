use crate::terminal_observe::{
    terminal_chunk_has_codex_prompt_output, terminal_chunk_is_codex_prompt_surface,
};

pub(crate) const TERMINAL_FRAME_BRIDGE_MIN_BYTES: usize = 256;
pub(crate) const TERMINAL_FRAME_BRIDGE_CSI_MIN_COUNT: usize = 24;
pub(crate) const TERMINAL_INLINE_STATUS_ANIMATION_HOT_MS: u64 = 1_000;

pub(crate) fn terminal_write_should_frame_budget(
    data: &str,
    _is_remote_resume_session: bool,
    _remote_overlay_dismissed: bool,
    protocol_only_output: bool,
) -> bool {
    let protocol_only_repaint =
        protocol_only_output && terminal_output_has_synchronized_repaint_frame(data);
    (protocol_only_repaint
        || (!protocol_only_output && terminal_output_is_high_volume_frame_like(data)))
        && !terminal_output_contains_interactive_codex_surface(data)
}

pub(crate) fn terminal_output_is_high_volume_frame_like(data: &str) -> bool {
    !terminal_output_is_inline_status_rewrite_frame(data)
        && (terminal_output_has_synchronized_repaint_frame(data)
            || data.len() >= TERMINAL_FRAME_BRIDGE_MIN_BYTES)
        && (terminal_last_frame_anchor_index(data).is_some()
            || data.contains("\x1b[?25l")
            || terminal_output_has_synchronized_repaint_frame(data)
            || terminal_csi_count_at_least(data, TERMINAL_FRAME_BRIDGE_CSI_MIN_COUNT))
}

#[cfg(test)]
pub(crate) fn terminal_output_is_small_synchronized_repaint_frame(data: &str) -> bool {
    terminal_output_has_synchronized_repaint_frame(data)
        && data.len() < TERMINAL_FRAME_BRIDGE_MIN_BYTES
        && !data.contains("\x1b[?1049h")
        && !data.contains("\x1b[?1049l")
}

pub(crate) fn terminal_output_is_inline_status_animation_frame(data: &str) -> bool {
    if !terminal_output_is_inline_status_rewrite_frame(data) {
        return false;
    }
    let visible_text = terminal_visible_text_for_inline_animation_detection(data, 256);
    visible_text.contains("Working")
}

pub(crate) fn terminal_output_is_inline_status_rewrite_frame(data: &str) -> bool {
    if data.is_empty()
        || data.len() > 8_192
        || data.contains("\x1b[?1049h")
        || data.contains("\x1b[?1049l")
        || data.contains("\x1b[2J")
    {
        return false;
    }
    if terminal_output_has_synchronized_repaint_frame(data)
        && (terminal_synchronized_output_has_row_addressing(data)
            || terminal_csi_count_at_least(data, 8))
    {
        return false;
    }
    let inline_rewrite_control = data.contains('\r')
        || data.contains('\x08')
        || data.contains("\x1b[K")
        || data.contains("\x1b[2K")
        || data.contains("\x1b[G")
        || data.contains("\x1b[1G")
        || data.contains("\x1b[?25l");
    if !inline_rewrite_control {
        return false;
    }
    let visible_text = terminal_visible_text_for_inline_animation_detection(data, 256);
    if data.contains("\x1b[H") && visible_text.chars().count() >= 240 {
        return false;
    }
    !visible_text.trim().is_empty()
}

pub(crate) fn terminal_synchronized_output_has_row_addressing(data: &str) -> bool {
    let bytes = data.as_bytes();
    let mut index = 0usize;
    while index + 2 < bytes.len() {
        if bytes[index] != 0x1b || bytes[index + 1] != b'[' {
            index += 1;
            continue;
        }
        let mut cursor = index + 2;
        let mut saw_semicolon = false;
        let mut saw_digit_before_semicolon = false;
        let mut saw_digit_after_semicolon = false;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            if byte.is_ascii_digit() {
                if saw_semicolon {
                    saw_digit_after_semicolon = true;
                } else {
                    saw_digit_before_semicolon = true;
                }
                cursor += 1;
                continue;
            }
            if byte == b';' {
                saw_semicolon = true;
                cursor += 1;
                continue;
            }
            if (byte == b'H' || byte == b'f')
                && saw_semicolon
                && saw_digit_before_semicolon
                && saw_digit_after_semicolon
            {
                return true;
            }
            break;
        }
        index += 2;
    }
    false
}

pub(crate) fn terminal_inline_status_animation_budget_state(
    data: &str,
    now_ms: u64,
    hot_until_ms: u64,
) -> (bool, u64) {
    let starts_animation = terminal_output_is_inline_status_animation_frame(data)
        || terminal_output_is_high_volume_frame_like(data);
    let continues_animation = !starts_animation
        && now_ms < hot_until_ms
        && terminal_output_is_inline_status_rewrite_frame(data);
    if starts_animation || continues_animation {
        (
            true,
            now_ms.saturating_add(TERMINAL_INLINE_STATUS_ANIMATION_HOT_MS),
        )
    } else {
        (false, hot_until_ms)
    }
}

fn terminal_visible_text_for_inline_animation_detection(data: &str, max_chars: usize) -> String {
    let mut visible = String::new();
    let mut chars = data.chars().peekable();
    while let Some(ch) = chars.next() {
        if visible.chars().count() >= max_chars {
            break;
        }
        if ch == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for seq in chars.by_ref() {
                        if ('@'..='~').contains(&seq) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut previous_was_escape = false;
                    for seq in chars.by_ref() {
                        if seq == '\x07' || (previous_was_escape && seq == '\\') {
                            break;
                        }
                        previous_was_escape = seq == '\x1b';
                    }
                }
                Some('(') | Some(')') => {
                    chars.next();
                    let _ = chars.next();
                }
                _ => {}
            }
            continue;
        }
        if ch == '\r' || ch == '\n' || ch == '\t' {
            visible.push(' ');
        } else if !ch.is_control() {
            visible.push(ch);
        }
    }
    visible
}

pub(crate) fn terminal_output_contains_codex_welcome_surface(data: &str) -> bool {
    data.contains("OpenAI Codex") && data.contains("/model to change")
}

pub(crate) fn terminal_output_contains_interactive_codex_surface(data: &str) -> bool {
    terminal_output_contains_codex_welcome_surface(data)
        || terminal_chunk_is_codex_prompt_surface(data)
        || terminal_chunk_has_codex_prompt_output(data)
}

#[cfg(test)]
pub(crate) fn coalesce_high_volume_terminal_frames(data: &str) -> String {
    data.to_string()
}

#[cfg(test)]
pub(crate) fn trim_high_volume_terminal_frame_buffer(data: &str) -> String {
    data.to_string()
}

pub(crate) fn terminal_output_has_synchronized_repaint_frame(data: &str) -> bool {
    data.contains("\x1b[?2026h")
        && data.contains("\x1b[?2026l")
        && (data.contains("\x1b[K")
            || data.contains("\x1b[2K")
            || terminal_csi_count_at_least(data, 8))
}

/// xterm.js 6 implements synchronized output (DEC mode 2026) natively, so the
/// write bridge no longer stands in for it — the old hold-and-guess helpers
/// (`terminal_output_ends_inside_synchronized_frame`,
/// `terminal_synchronized_output_complete_prefix_len`) were retired in 2.9.40.
#[cfg(test)]
pub(crate) fn terminal_synchronized_output_frame_ranges(data: &str) -> Vec<(usize, usize)> {
    let start_marker = "\x1b[?2026h";
    let end_marker = "\x1b[?2026l";
    let mut frames = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative_start) = data[search_start..].find(start_marker) {
        let frame_start = search_start + relative_start;
        let frame_body_start = frame_start + start_marker.len();
        let Some(relative_end) = data[frame_body_start..].find(end_marker) else {
            break;
        };
        let frame_end = frame_body_start + relative_end + end_marker.len();
        frames.push((frame_start, frame_end));
        search_start = frame_end;
    }
    frames
}

fn terminal_last_frame_anchor_index(data: &str) -> Option<usize> {
    terminal_frame_anchors()
        .iter()
        .filter_map(|anchor| data.rfind(anchor))
        .max()
}

fn terminal_frame_anchors() -> [&'static str; 5] {
    ["\x1b[H", "\x1b[1;1H", "\x1b[1;1f", "\x1b[;H", "\x1b[0;0H"]
}

pub(crate) fn terminal_csi_count_at_least(data: &str, threshold: usize) -> bool {
    data.match_indices("\x1b[").take(threshold).count() >= threshold
}
