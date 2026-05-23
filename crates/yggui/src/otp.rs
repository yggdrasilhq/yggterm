use dioxus::prelude::*;

pub const YGGUI_OTP_CODE_LEN: usize = 6;

pub const YGGUI_OTP_INPUT_ID: &str = "yggui-otp-input";

pub const YGGUI_OTP_CSS: &str = r#"
.yggui-otp-entry {
  position: relative;
}

.yggui-otp-grid {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: 8px;
  pointer-events: none;
}

.yggui-otp-input {
  position: absolute;
  inset: 0;
  z-index: 2;
  width: 100%;
  height: 100%;
  border: 0;
  background: transparent;
  color: transparent;
  caret-color: transparent;
  opacity: 0.02;
}

.yggui-otp-cell {
  display: grid;
  width: 100%;
  aspect-ratio: 1;
  min-height: 44px;
  place-items: center;
  border: 1px solid var(--yggui-otp-border, #cbd5dc);
  border-radius: 7px;
  background: var(--yggui-otp-cell-bg, #ffffff);
  color: var(--yggui-otp-ink, #1b232b);
  font-size: 24px;
  font-weight: 900;
  text-align: center;
  text-transform: uppercase;
}

.yggui-otp-cell.active {
  border-color: var(--yggui-otp-accent, #d89b00);
  box-shadow: 0 0 0 3px var(--yggui-otp-accent-halo, rgba(216, 155, 0, 0.18));
}

.yggui-otp-cell.cursor::after {
  content: "";
  display: inline-block;
  width: 2px;
  height: 28px;
  background: var(--yggui-otp-ink, #1b232b);
  border-radius: 1px;
  animation: yggui-otp-blink 1.1s step-start infinite;
}

@keyframes yggui-otp-blink {
  50% { opacity: 0; }
}

.yggui-otp-paste-btn {
  width: 100%;
  margin-top: 10px;
  padding: 11px 0;
  border: none;
  border-radius: 8px;
  background: var(--yggui-otp-paste-bg, #1f6f78);
  color: var(--yggui-otp-paste-fg, #ffffff);
  font-size: 15px;
  font-weight: 600;
  cursor: pointer;
}

.yggui-otp-paste-btn:active {
  background: var(--yggui-otp-paste-bg-active, #174f56);
}
"#;

/// Normalize a single OTP cell string to at most one uppercase alphanumeric char.
pub fn normalize_otp_cell(raw: &str) -> String {
    raw.chars()
        .find(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase().to_string())
        .unwrap_or_default()
}

/// Return a canonical 6-element cell vector from any slice.
pub fn normalized_otp_cells(current: &[String]) -> Vec<String> {
    let mut cells: Vec<String> = current
        .iter()
        .take(YGGUI_OTP_CODE_LEN)
        .map(|cell| normalize_otp_cell(cell))
        .collect();
    cells.resize(YGGUI_OTP_CODE_LEN, String::new());
    cells
}

/// Join the 6 cells into a single string (empty cells become "").
pub fn otp_value(cells: &[String]) -> String {
    normalized_otp_cells(cells).join("")
}

/// Return `Some(code)` only when all 6 cells are filled.
pub fn complete_otp(cells: &[String]) -> Option<String> {
    let code = otp_value(cells);
    (code.len() == YGGUI_OTP_CODE_LEN).then_some(code)
}

/// Index of the first empty cell (clamped to 0..5), used to show active focus.
pub fn otp_active_index(cells: &[String]) -> usize {
    let filled = normalized_otp_cells(cells)
        .into_iter()
        .take_while(|cell| !cell.is_empty())
        .count();
    filled.min(YGGUI_OTP_CODE_LEN - 1)
}

/// Parse a raw input string into a 6-cell array (uppercase, alphanumeric only).
pub fn apply_otp_input(raw: &str) -> Vec<String> {
    let chars: Vec<String> = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(YGGUI_OTP_CODE_LEN)
        .map(|ch| ch.to_ascii_uppercase().to_string())
        .collect();
    let mut cells = vec![String::new(); YGGUI_OTP_CODE_LEN];
    for (i, ch) in chars.into_iter().enumerate() {
        cells[i] = ch;
    }
    cells
}

/// JS script to intercept document-level paste events onto the OTP input.
/// Call once at app startup via `document::eval`. Idempotent.
pub fn install_otp_paste_bridge_script() -> String {
    let id = YGGUI_OTP_INPUT_ID;
    format!(
        r#"
return (() => {{
  if (window.yggui_otp_paste_installed) return "ok:already-installed";
  window.yggui_otp_paste_installed = true;
  function otp_chars(text) {{
    return String(text || "").replace(/[^a-z0-9]/gi, "").slice(0, 6).toUpperCase();
  }}
  document.addEventListener("paste", function (event) {{
    var target = event.target;
    if (!target || target.id !== "{id}") return;
    var clipboard = event.clipboardData || window.clipboardData;
    if (!clipboard) return;
    var chars = otp_chars(clipboard.getData("text"));
    if (!chars) return;
    event.preventDefault();
    target.value = chars;
    var inputEvent = typeof InputEvent === "function"
      ? new InputEvent("input", {{ bubbles: true, data: chars, inputType: "insertFromPaste" }})
      : new Event("input", {{ bubbles: true }});
    target.dispatchEvent(inputEvent);
  }}, true);
  return "ok";
}})();
"#
    )
}

/// JS script for the Android native clipboard bridge.
/// Reads `window.AndroidClipboard.getText()` (registered via `@JavascriptInterface`).
/// Returns `"ok:<CODE>"` or `"error:<reason>"`.
pub fn android_clipboard_read_script() -> &'static str {
    r#"
return (() => {
  if (!window.AndroidClipboard) return "error:no-bridge";
  try {
    var text = window.AndroidClipboard.getText();
    if (!text) return "error:empty";
    var chars = String(text).replace(/[^a-z0-9]/gi, "").slice(0, 6).toUpperCase();
    return chars ? "ok:" + chars : "error:no-code-chars";
  } catch (e) {
    return "error:" + e.message;
  }
})();
"#
}

async fn eval_to_string(eval: dioxus::document::Eval) -> String {
    match eval.await {
        Ok(value) => value.as_str().unwrap_or("").to_string(),
        Err(_) => String::new(),
    }
}

/// Six-cell OTP code entry widget.
///
/// Renders a hidden overlay input (for keyboard entry and web paste) over a
/// visual 6-cell grid. On mobile builds, a "Paste code" button reads the
/// clipboard via the Android native bridge (`window.AndroidClipboard`) and
/// calls `on_update` + `on_complete` without requiring any tap on "Continue".
///
/// # Required setup
/// Call `install_otp_paste_bridge_script()` once at app startup so that
/// web paste events (Ctrl+V / long-press → Paste on desktop) are intercepted.
///
/// # Android
/// Register `ClipboardInterface` in `MainActivity.onWebViewCreate`:
/// ```kotlin
/// webView.addJavascriptInterface(ClipboardInterface(this), "AndroidClipboard")
/// ```
/// See `assets/android/ClipboardInterface.kt` in the app repository.
#[component]
pub fn OtpCodeEntry(
    /// Current cell values (controlled). Use `apply_otp_input` to build from a
    /// raw string, or start with `vec![String::new(); YGGUI_OTP_CODE_LEN]`.
    cells: Vec<String>,
    /// Called on every keystroke or paste with the new cell array.
    on_update: EventHandler<Vec<String>>,
    /// Called whenever a complete 6-character code is present.
    on_complete: EventHandler<String>,
) -> Element {
    let code_value = otp_value(&cells);
    let code_active_index = otp_active_index(&cells);

    rsx! {
        div { class: "yggui-otp-entry",
            input {
                id: YGGUI_OTP_INPUT_ID,
                class: "yggui-otp-input",
                aria_label: "Email verification code",
                r#type: "text",
                inputmode: "text",
                autocomplete: "one-time-code",
                autocapitalize: "characters",
                enterkeyhint: "go",
                spellcheck: "false",
                value: "{code_value}",
                oninput: move |event| {
                    let new_cells = apply_otp_input(&event.value());
                    let maybe_complete = complete_otp(&new_cells);
                    on_update.call(new_cells);
                    if let Some(code) = maybe_complete {
                        on_complete.call(code);
                    }
                },
            }
            div { class: "yggui-otp-grid", aria_hidden: "true",
                for index in 0..YGGUI_OTP_CODE_LEN {
                    div {
                        class: {
                            let ch = cells.get(index).cloned().unwrap_or_default();
                            match (index == code_active_index, ch.is_empty()) {
                                (true, true) => "yggui-otp-cell active cursor",
                                (true, false) => "yggui-otp-cell active",
                                (false, _) => "yggui-otp-cell",
                            }
                        },
                        "{cells.get(index).cloned().unwrap_or_default()}"
                    }
                }
            }
        }
        if cfg!(feature = "mobile") {
            button {
                class: "yggui-otp-paste-btn",
                r#type: "button",
                onclick: move |_| {
                    let eval = document::eval(android_clipboard_read_script());
                    spawn(async move {
                        let msg = eval_to_string(eval).await;
                        if let Some(code_str) = msg.strip_prefix("ok:") {
                            let new_cells = apply_otp_input(code_str);
                            let maybe_complete = complete_otp(&new_cells);
                            on_update.call(new_cells);
                            if let Some(code) = maybe_complete {
                                on_complete.call(code);
                            }
                        }
                    });
                },
                "Paste code"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_otp_input_distributes_paste() {
        let cells = apply_otp_input("A1B2C3");
        assert_eq!(cells, vec!["A", "1", "B", "2", "C", "3"]);
    }

    #[test]
    fn apply_otp_input_strips_non_alnum_and_uppercases() {
        let cells = apply_otp_input("a1-b2 c3 extra");
        assert_eq!(cells, vec!["A", "1", "B", "2", "C", "3"]);
    }

    #[test]
    fn complete_otp_requires_all_six() {
        assert_eq!(complete_otp(&apply_otp_input("A1B2C")), None);
        assert_eq!(
            complete_otp(&apply_otp_input("A1B2C3")),
            Some("A1B2C3".to_string())
        );
    }

    #[test]
    fn otp_active_index_tracks_first_empty() {
        let cells: Vec<String> = vec!["A", "1", "B", "", "", ""]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(otp_active_index(&cells), 3);
        assert_eq!(otp_active_index(&apply_otp_input("")), 0);
        assert_eq!(otp_active_index(&apply_otp_input("A1B2C3")), 5);
    }

    #[test]
    fn paste_bridge_targets_yggui_otp_input_id() {
        let script = install_otp_paste_bridge_script();
        assert!(script.contains(YGGUI_OTP_INPUT_ID));
        assert!(script.contains("insertFromPaste"));
        assert!(script.contains("yggui_otp_paste_installed"));
    }

    #[test]
    fn android_clipboard_script_uses_native_bridge() {
        let script = android_clipboard_read_script();
        assert!(script.contains("AndroidClipboard"));
        assert!(script.contains("getText"));
        assert!(script.contains("ok:"));
    }
}
