use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

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
    DropUnfocusedTuiFrame {
        tail: String,
        chars: usize,
        frame_like: bool,
        protocol_only: bool,
    },
    SetInputEnabled {
        enabled: bool,
        focus: bool,
    },
    Redraw {
        reason: String,
    },
    Refit,
}

#[derive(Debug, Clone)]
pub(crate) enum TerminalJsEvent {
    Ready,
    HostHealth {
        cursor_line_text: String,
        text_tail: String,
        has_transport_error: bool,
        cursor_y: u16,
        rows: u16,
        blank_rows_below_cursor: u16,
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
    ClipboardPasteRequest,
    ClipboardImageRequest,
    ClipboardError {
        action: String,
        message: String,
    },
    Debug {
        message: String,
    },
    Perf {
        name: String,
        payload: Value,
    },
    Ignored {
        reason: String,
        value: Value,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalJsEventWire {
    Ready,
    HostHealth {
        cursor_line_text: String,
        text_tail: String,
        has_transport_error: bool,
        cursor_y: u16,
        rows: u16,
        blank_rows_below_cursor: u16,
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
    ClipboardPasteRequest,
    ClipboardImageRequest,
    ClipboardError {
        action: String,
        message: String,
    },
    Debug {
        message: String,
    },
    Perf {
        name: String,
        payload: Value,
    },
}

impl From<TerminalJsEventWire> for TerminalJsEvent {
    fn from(value: TerminalJsEventWire) -> Self {
        match value {
            TerminalJsEventWire::Ready => TerminalJsEvent::Ready,
            TerminalJsEventWire::HostHealth {
                cursor_line_text,
                text_tail,
                has_transport_error,
                cursor_y,
                rows,
                blank_rows_below_cursor,
            } => TerminalJsEvent::HostHealth {
                cursor_line_text,
                text_tail,
                has_transport_error,
                cursor_y,
                rows,
                blank_rows_below_cursor,
            },
            TerminalJsEventWire::Paint {
                child_count,
                xterm_present,
                screen_present,
                viewport_present,
                rows_present,
                cols,
                rows,
            } => TerminalJsEvent::Paint {
                child_count,
                xterm_present,
                screen_present,
                viewport_present,
                rows_present,
                cols,
                rows,
            },
            TerminalJsEventWire::Input { data } => TerminalJsEvent::Input { data },
            TerminalJsEventWire::Resize { cols, rows } => TerminalJsEvent::Resize { cols, rows },
            TerminalJsEventWire::Clipboard { action, chars } => {
                TerminalJsEvent::Clipboard { action, chars }
            }
            TerminalJsEventWire::ClipboardPasteRequest => TerminalJsEvent::ClipboardPasteRequest,
            TerminalJsEventWire::ClipboardImageRequest => TerminalJsEvent::ClipboardImageRequest,
            TerminalJsEventWire::ClipboardError { action, message } => {
                TerminalJsEvent::ClipboardError { action, message }
            }
            TerminalJsEventWire::Debug { message } => TerminalJsEvent::Debug { message },
            TerminalJsEventWire::Perf { name, payload } => TerminalJsEvent::Perf { name, payload },
        }
    }
}

impl<'de> Deserialize<'de> for TerminalJsEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(
            match serde_json::from_value::<TerminalJsEventWire>(value.clone()) {
                Ok(event) => event.into(),
                Err(error) => TerminalJsEvent::Ignored {
                    reason: error.to_string(),
                    value,
                },
            },
        )
    }
}
