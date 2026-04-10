use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TerminalJsCommand {
    Reset {
        title: String,
        background: String,
        foreground: String,
        cursor: String,
        cursor_muted: String,
        cursor_text: String,
        dim_foreground: String,
        selection: String,
        black: String,
        red: String,
        green: String,
        yellow: String,
        blue: String,
        magenta: String,
        cyan: String,
        white: String,
        bright_black: String,
        bright_red: String,
        bright_green: String,
        bright_yellow: String,
        bright_blue: String,
        bright_magenta: String,
        bright_cyan: String,
        bright_white: String,
        font_family: String,
        font_weight: u16,
        font_weight_bold: u16,
        line_height: f32,
        minimum_contrast_ratio: f32,
        font_size: f32,
    },
    Write {
        data: String,
    },
    SetInputEnabled {
        enabled: bool,
        focus: bool,
    },
    Refit,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TerminalJsEvent {
    Ready,
    HostHealth {
        cursor_line_text: String,
        text_tail: String,
        has_transport_error: bool,
    },
    Paint {
        child_count: usize,
        xterm_present: bool,
        screen_present: bool,
        viewport_present: bool,
        rows_present: bool,
        cols: u16,
        rows: u16,
    },
    Input {
        data: String,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Clipboard {
        action: String,
        chars: usize,
    },
    ClipboardImageRequest,
    ClipboardError {
        action: String,
        message: String,
    },
    Debug {
        message: String,
    },
}
