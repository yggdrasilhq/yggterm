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
        input_line_background: String,
        input_line_border: String,
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
        frame_like_hot: bool,
        cursor_y: u16,
        rows: u16,
        blank_rows_below_cursor: u16,
        render_health_status: String,
        render_health_reason: String,
        render_health_recovery_count: u32,
        render_health_recovery_pending: bool,
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
    ReadNudge {
        reason: String,
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
    ContextMenu {
        client_x: f64,
        client_y: f64,
    },
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
        #[serde(default)]
        frame_like_hot: bool,
        cursor_y: u16,
        rows: u16,
        blank_rows_below_cursor: u16,
        #[serde(default)]
        render_health_status: String,
        #[serde(default)]
        render_health_reason: String,
        #[serde(default)]
        render_health_recovery_count: u32,
        #[serde(default)]
        render_health_recovery_pending: bool,
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
    ReadNudge {
        #[serde(default)]
        reason: String,
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
    ContextMenu {
        #[serde(default)]
        client_x: f64,
        #[serde(default)]
        client_y: f64,
    },
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
                frame_like_hot,
                cursor_y,
                rows,
                blank_rows_below_cursor,
                render_health_status,
                render_health_reason,
                render_health_recovery_count,
                render_health_recovery_pending,
            } => TerminalJsEvent::HostHealth {
                cursor_line_text,
                text_tail,
                has_transport_error,
                frame_like_hot,
                cursor_y,
                rows,
                blank_rows_below_cursor,
                render_health_status,
                render_health_reason,
                render_health_recovery_count,
                render_health_recovery_pending,
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
            TerminalJsEventWire::ReadNudge { reason } => TerminalJsEvent::ReadNudge { reason },
            TerminalJsEventWire::Resize { cols, rows } => TerminalJsEvent::Resize { cols, rows },
            TerminalJsEventWire::Clipboard { action, chars } => {
                TerminalJsEvent::Clipboard { action, chars }
            }
            TerminalJsEventWire::ClipboardPasteRequest => TerminalJsEvent::ClipboardPasteRequest,
            TerminalJsEventWire::ClipboardImageRequest => TerminalJsEvent::ClipboardImageRequest,
            TerminalJsEventWire::ContextMenu { client_x, client_y } => {
                TerminalJsEvent::ContextMenu { client_x, client_y }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_health_defaults_frame_like_hot_to_false() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "host_health",
            "cursor_line_text": "pi@jojo:~$ ",
            "text_tail": "ready",
            "has_transport_error": false,
            "cursor_y": 12,
            "rows": 32,
            "blank_rows_below_cursor": 0
        }))
        .expect("legacy host health payloads should still deserialize");
        assert!(matches!(
            event,
            TerminalJsEvent::HostHealth {
                frame_like_hot: false,
                render_health_status,
                render_health_reason,
                ..
            } if render_health_status.is_empty() && render_health_reason.is_empty()
        ));
    }

    #[test]
    fn host_health_deserializes_frame_like_hot() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "host_health",
            "cursor_line_text": "",
            "text_tail": "Yggterm synthetic TUI CPU smoke frame",
            "has_transport_error": false,
            "frame_like_hot": true,
            "cursor_y": 0,
            "rows": 48,
            "blank_rows_below_cursor": 0
        }))
        .expect("frame-like host health payloads should deserialize");
        assert!(matches!(
            event,
            TerminalJsEvent::HostHealth {
                frame_like_hot: true,
                ..
            }
        ));
    }

    #[test]
    fn host_health_deserializes_render_health_fields() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "host_health",
            "cursor_line_text": "› prompt",
            "text_tail": "buffer text",
            "has_transport_error": false,
            "frame_like_hot": false,
            "cursor_y": 10,
            "rows": 40,
            "blank_rows_below_cursor": 1,
            "render_health_status": "unhealthy",
            "render_health_reason": "canvas_blank_with_buffer_text",
            "render_health_recovery_count": 1,
            "render_health_recovery_pending": true
        }))
        .expect("render-health host health payloads should deserialize");
        assert!(matches!(
            event,
            TerminalJsEvent::HostHealth {
                render_health_status,
                render_health_reason,
                render_health_recovery_count: 1,
                render_health_recovery_pending: true,
                ..
            } if render_health_status == "unhealthy"
                && render_health_reason == "canvas_blank_with_buffer_text"
        ));
    }

    #[test]
    fn context_menu_event_deserializes_with_coordinates() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "context_menu",
            "client_x": 42.5,
            "client_y": 84.25
        }))
        .expect("context-menu payloads should deserialize");
        assert!(matches!(
            event,
            TerminalJsEvent::ContextMenu {
                client_x,
                client_y,
            } if (client_x - 42.5).abs() < f64::EPSILON
                && (client_y - 84.25).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn input_event_preserves_whitespace_only_payloads() {
        for payload in [" ", "  ", "\t", "\r"] {
            let event: TerminalJsEvent = serde_json::from_value(json!({
                "kind": "input",
                "data": payload,
            }))
            .expect("terminal input whitespace should deserialize unchanged");
            assert!(matches!(event, TerminalJsEvent::Input { data } if data == payload));
        }
    }
}
