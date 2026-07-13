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
        /// Count of non-blank rows in the client's VISIBLE xterm buffer. A very
        /// low value while the daemon holds a full screen = an incomplete reveal
        /// (blank-frame-on-working-reveal): safe to force a corrective repaint
        /// because there is nothing good to tear.
        visible_nonblank_rows: u16,
        /// JSON describing a client-detected render FAIL PATTERN (e.g. a redraw
        /// burst with no session change), empty when none. Logged as a
        /// `render_fail_pattern` trace event for later inspection.
        render_anomaly: String,
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
        text: Option<String>,
    },
    ClipboardPasteRequest,
    ClipboardImageRequest,
    ContextMenu {
        client_x: f64,
        client_y: f64,
    },
    ContextMenuClose,
    ClipboardError {
        action: String,
        message: String,
    },
    Debug {
        message: String,
    },
    /// A clicked terminal link (OSC-8 hyperlink or detected plain-text URL).
    /// The Rust side validates the scheme and opens the OS browser — JS
    /// `window.open` is a no-op inside the wry webview.
    OpenUrl {
        url: String,
    },
    /// A terminal-driven attention signal: the BEL (`term.onBell`) or a
    /// notification OSC (9 / 777) emitted by the CLI running in the PTY — e.g.
    /// Claude Code / Codex pinging when a task finishes or needs input. Routed
    /// to the in-app toast + sound always, and an OS desktop notification only
    /// when the user is not already watching this session.
    Notify {
        source: String,
        title: Option<String>,
        body: Option<String>,
    },
    Perf {
        name: String,
        payload: Value,
    },
    /// libyggterm web-surface control emitted by a program in the PTY as
    /// OSC 7717 (ychrome pilot). Travels the existing byte relay (remote
    /// daemon → ssh bridge → local daemon → xterm.js), so it works for
    /// local and remote sessions uniformly and is invisible to plain
    /// terminals. `action` is `open` / `close` / `heartbeat`; `session`
    /// must match the emitting session's YGGTERM_SESSION_ID (weak
    /// anti-spoof gate: a cat'ed file doesn't know it).
    WebSurface {
        action: String,
        session: String,
        url: Option<String>,
        title: Option<String>,
        /// Host-owned profile name (ychrome `--profile`, default "default").
        /// Selects the surface's persistent storage jar.
        profile: Option<String>,
        /// The `url` is the app's own START PAGE, not a URL the user asked for
        /// (`ychrome` with no argument). Only the app knows the difference, and
        /// it decides what the GUI's tab restore is allowed to displace: with
        /// "continue where you left off" on, a start-page open lands on the tab
        /// the user was last reading instead of stacking a start page over the
        /// restored session. `ychrome <url>` is a REQUEST and always wins.
        start_page: bool,
    },
    /// libyggterm SIDEBAR-CONTRIBUTION control, OSC 7717 with the `sidebar`
    /// verb. The app DECLARES which panes it offers plus a loopback control
    /// endpoint; the GUI fetches each pane's schema from that endpoint when the
    /// pane is opened, so a 1100-row vault never rides the PTY.
    ///
    /// `action` is `declare` (idempotent, re-emitted on the heartbeat cadence
    /// as the liveness signal) or `close` (retires the contribution). An
    /// unswept contribution expires like a web surface.
    ///
    /// The declaration carries NO schema and NO secret — only what the rail
    /// needs to draw a button.
    SidebarContribution {
        action: String,
        session: String,
        /// Loopback control endpoint on the APP's host, e.g.
        /// `http://127.0.0.1:41234`. The GUI fetches it itself over a plain
        /// socket, so a remote session needs an `ssh -L` forward — never the
        /// webview's SOCKS proxy. See `resolve_control_endpoint_url`.
        control: Option<String>,
        panes: Vec<SidebarPaneDeclaration>,
        /// Opaque stamp over the app's web-surface policy (adblock ruleset +
        /// userscripts). The GUI refetches `<control>/policy` only when this
        /// changes, so a ~4s heartbeat never drags the ruleset across the wire.
        /// Absent ⇒ the app ships no policy and its surfaces get none.
        policy_version: Option<String>,
        /// The app's display name, shown on the main zoom control ("Ychrome
        /// Global Zoom"). The app names itself; yggterm never hardcodes it.
        app_name: Option<String>,
        /// Opaque stamp over the app's per-site zoom overrides, the same trick as
        /// `policy_version`: the GUI refetches `<control>/zoom` only when it
        /// moves. Absent ⇒ the app ships no per-site zoom.
        zoom_version: Option<String>,
        /// Opaque stamp over the app's chrome-appearance choice (general default
        /// + per-site overrides), same trick again: the GUI refetches
        /// `<control>/appearance` only when it moves. Absent ⇒ the app ships no
        /// appearance and the chrome falls back to Light.
        appearance_version: Option<String>,
    },
    /// A WebAuthn passkey ceremony (OSC 7717 `fido2 ; request`) asking for the
    /// user's presence. The app carries only the rpId and a display label — no
    /// challenge, no key. The GUI shows a native presence dialog and, on
    /// approval, POSTs the grant to the app's control endpoint (declared over
    /// `sidebar` on this same stream). See `Signer` in ychrome's `passkey.rs`.
    Fido2Request {
        /// `request` today; room for `cancel` if an app ever retracts one.
        action: String,
        session: String,
        /// Opaque per-ceremony id the app is blocking on; the grant echoes it.
        request_id: String,
        /// The relying party the credential is for, e.g. `github.com`.
        rp_id: String,
        /// A human label for the FIRST matched account. Kept for a single-account
        /// ceremony and as a fallback; the picker uses `accounts`.
        account: String,
        /// Every stored passkey that answers this `get()` — one entry ⇒ a plain
        /// Approve dialog, several ⇒ a picker. A `create()` always has one. The
        /// grant carries the chosen `credential_id`.
        accounts: Vec<Fido2Account>,
        /// `get` (sign in) or `create` (register) — changes the dialog wording.
        ceremony: String,
        /// The page origin the ceremony runs on, shown so the user can see the
        /// site asking. Diagnostic; the app already validated it against rpId.
        origin: String,
    },
    Ignored {
        reason: String,
        value: Value,
    },
}

/// One passkey the user may pick in the presence dialog. Secret-free: a stable
/// `credential_id` to echo back in the grant, and a human `label`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fido2Account {
    pub credential_id: String,
    pub label: String,
}

/// One pane an app offers, as the rail needs it: an id to fetch the schema
/// with, an icon glyph, and a tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarPaneDeclaration {
    pub id: String,
    pub icon: String,
    pub title: String,
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
        #[serde(default)]
        visible_nonblank_rows: u16,
        #[serde(default)]
        render_anomaly: String,
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
        #[serde(default)]
        text: Option<String>,
    },
    ClipboardPasteRequest,
    ClipboardImageRequest,
    ContextMenu {
        #[serde(default)]
        client_x: f64,
        #[serde(default)]
        client_y: f64,
    },
    ContextMenuClose,
    ClipboardError {
        action: String,
        message: String,
    },
    Debug {
        message: String,
    },
    OpenUrl {
        #[serde(default)]
        url: String,
    },
    Notify {
        source: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        body: Option<String>,
    },
    Perf {
        name: String,
        payload: Value,
    },
    WebSurface {
        action: String,
        session: String,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        profile: Option<String>,
        #[serde(default)]
        start_page: bool,
    },
    SidebarContribution {
        action: String,
        session: String,
        #[serde(default)]
        control: Option<String>,
        #[serde(default)]
        panes: Vec<SidebarPaneDeclarationWire>,
        #[serde(default)]
        policy_version: Option<String>,
        #[serde(default)]
        app_name: Option<String>,
        #[serde(default)]
        zoom_version: Option<String>,
        #[serde(default)]
        appearance_version: Option<String>,
    },
    Fido2Request {
        action: String,
        session: String,
        #[serde(default)]
        request_id: String,
        #[serde(default)]
        rp_id: String,
        #[serde(default)]
        account: String,
        #[serde(default)]
        accounts: Vec<Fido2AccountWire>,
        #[serde(default)]
        ceremony: String,
        #[serde(default)]
        origin: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fido2AccountWire {
    #[serde(default)]
    pub credential_id: String,
    #[serde(default)]
    pub label: String,
}

/// One pane an app offers. Only what the RAIL needs to draw a button — never
/// the pane's schema, which the GUI fetches from the control endpoint, and
/// never a secret.
#[derive(Debug, Clone, Deserialize)]
pub struct SidebarPaneDeclarationWire {
    pub id: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub title: String,
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
                visible_nonblank_rows,
                render_anomaly,
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
                visible_nonblank_rows,
                render_anomaly,
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
            TerminalJsEventWire::Clipboard {
                action,
                chars,
                text,
            } => TerminalJsEvent::Clipboard {
                action,
                chars,
                text,
            },
            TerminalJsEventWire::ClipboardPasteRequest => TerminalJsEvent::ClipboardPasteRequest,
            TerminalJsEventWire::ClipboardImageRequest => TerminalJsEvent::ClipboardImageRequest,
            TerminalJsEventWire::ContextMenu { client_x, client_y } => {
                TerminalJsEvent::ContextMenu { client_x, client_y }
            }
            TerminalJsEventWire::ContextMenuClose => TerminalJsEvent::ContextMenuClose,
            TerminalJsEventWire::ClipboardError { action, message } => {
                TerminalJsEvent::ClipboardError { action, message }
            }
            TerminalJsEventWire::Debug { message } => TerminalJsEvent::Debug { message },
            TerminalJsEventWire::OpenUrl { url } => TerminalJsEvent::OpenUrl { url },
            TerminalJsEventWire::Notify {
                source,
                title,
                body,
            } => TerminalJsEvent::Notify {
                source,
                title,
                body,
            },
            TerminalJsEventWire::Perf { name, payload } => TerminalJsEvent::Perf { name, payload },
            TerminalJsEventWire::WebSurface {
                action,
                session,
                url,
                title,
                profile,
                start_page,
            } => TerminalJsEvent::WebSurface {
                action,
                session,
                url,
                title,
                profile,
                start_page,
            },
            TerminalJsEventWire::SidebarContribution {
                action,
                session,
                control,
                panes,
                policy_version,
                app_name,
                zoom_version,
                appearance_version,
            } => TerminalJsEvent::SidebarContribution {
                action,
                session,
                control,
                policy_version,
                app_name,
                zoom_version,
                appearance_version,
                panes: panes
                    .into_iter()
                    .map(|pane| SidebarPaneDeclaration {
                        id: pane.id,
                        icon: pane.icon,
                        title: pane.title,
                    })
                    .collect(),
            },
            TerminalJsEventWire::Fido2Request {
                action,
                session,
                request_id,
                rp_id,
                account,
                accounts,
                ceremony,
                origin,
            } => TerminalJsEvent::Fido2Request {
                action,
                session,
                request_id,
                rp_id,
                account,
                accounts: accounts
                    .into_iter()
                    .map(|account| Fido2Account {
                        credential_id: account.credential_id,
                        label: account.label,
                    })
                    .collect(),
                ceremony,
                origin,
            },
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
    fn context_menu_close_event_deserializes() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "context_menu_close"
        }))
        .expect("terminal context-menu close payloads should deserialize");
        assert!(matches!(event, TerminalJsEvent::ContextMenuClose));
    }

    #[test]
    fn clipboard_event_deserializes_selection_text_without_requiring_legacy_payloads() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "clipboard",
            "action": "copy",
            "chars": 12,
            "text": "hello world!",
        }))
        .expect("terminal clipboard payload should deserialize");
        assert!(matches!(
            event,
            TerminalJsEvent::Clipboard {
                action,
                chars: 12,
                text: Some(selection),
            } if action == "copy" && selection == "hello world!"
        ));

        let legacy_event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "clipboard",
            "action": "copy",
            "chars": 12,
        }))
        .expect("legacy terminal clipboard payload should still deserialize");
        assert!(matches!(
            legacy_event,
            TerminalJsEvent::Clipboard {
                action,
                chars: 12,
                text: None,
            } if action == "copy"
        ));
    }

    #[test]
    fn web_surface_event_deserializes_open_and_close() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "web_surface",
            "action": "open",
            "session": "local/abc123",
            "url": "http://localhost:8000/",
            "title": "dev server",
            "profile": "work",
        }))
        .expect("web-surface open payload should deserialize");
        assert!(matches!(
            event,
            TerminalJsEvent::WebSurface {
                action, session, url: Some(url), title: Some(title), profile: Some(profile),
                // An app that predates the flag is not claiming a start page.
                start_page: false,
            }
                if action == "open"
                    && session == "local/abc123"
                    && url == "http://localhost:8000/"
                    && title == "dev server"
                    && profile == "work"
        ));

        let close: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "web_surface",
            "action": "close",
            "session": "local/abc123",
        }))
        .expect("web-surface close payload should deserialize without url/title");
        assert!(matches!(
            close,
            TerminalJsEvent::WebSurface { action, url: None, title: None, .. } if action == "close"
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

    // A libyggterm sidebar declaration must carry the app's display name and its
    // per-site zoom stamp through to the shell — the label ("Ychrome Global
    // Zoom") and the zoom refetch both depend on them arriving non-null.
    #[test]
    fn sidebar_declaration_carries_app_name_and_zoom_version() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "sidebar_contribution",
            "action": "declare",
            "session": "s1",
            "control": "http://127.0.0.1:41225",
            "app_name": "Ychrome",
            "policy_version": "abc",
            "zoom_version": "def",
            "appearance_version": "ghi",
            "panes": [{"id": "settings", "icon": "⚙", "title": "Settings"}],
        }))
        .expect("a full sidebar declaration should deserialize");
        match event {
            TerminalJsEvent::SidebarContribution {
                app_name,
                zoom_version,
                appearance_version,
                policy_version,
                ..
            } => {
                assert_eq!(app_name.as_deref(), Some("Ychrome"));
                assert_eq!(zoom_version.as_deref(), Some("def"));
                assert_eq!(appearance_version.as_deref(), Some("ghi"));
                assert_eq!(policy_version.as_deref(), Some("abc"));
            }
            other => panic!("expected a sidebar contribution, got {other:?}"),
        }
    }

    // An older app that ships neither field must still parse — they are optional,
    // and their absence means "no display name / no per-site zoom", not an error.
    #[test]
    fn sidebar_declaration_without_the_new_fields_still_parses() {
        let event: TerminalJsEvent = serde_json::from_value(json!({
            "kind": "sidebar_contribution",
            "action": "declare",
            "session": "s1",
            "panes": [],
        }))
        .expect("a minimal declaration should still deserialize");
        assert!(matches!(
            event,
            TerminalJsEvent::SidebarContribution {
                app_name: None,
                zoom_version: None,
                appearance_version: None,
                ..
            }
        ));
    }
}
