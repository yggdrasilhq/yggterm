use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use yggterm_core::UiTheme;
use yggui_contract::YgguiClipboardContents;

use crate::SessionKind;

const APP_CONTROL_REQUESTS_DIR: &str = "app-control-requests";
const APP_CONTROL_RESPONSES_DIR: &str = "app-control-responses";
const APP_CONTROL_CAPTURES_DIR: &str = "screenshots";
const APP_CONTROL_RECORDINGS_DIR: &str = "recordings";
const STALE_TARGETED_APP_CONTROL_REQUEST_MS: u128 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotTarget {
    App,
    PreviewViewport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlViewMode {
    Preview,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlPreviewLayout {
    Chat,
    Graph,
}

/// NOT `Copy`: `AppPane` carries the app's own pane id. The same reasoning as
/// `yggterm-shell`'s `RightPanelMode` — a unit variant plus a separate id field
/// would be two encodings of one fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlRightPanelMode {
    Hidden,
    Connect,
    Notifications,
    Settings,
    Metadata,
    /// A pane CONTRIBUTED by the active libyggterm app, by the app's pane id.
    /// This is how an agent drives ychrome's vault pane headlessly; before it
    /// existed the only way in was to click the button with `app dom-eval`.
    AppPane { id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProbeTerminalViewportInputMode {
    #[default]
    Auto,
    Keyboard,
    Xterm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlDragPlacement {
    Before,
    Into,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppControlPointerButton {
    #[default]
    Primary,
    Middle,
    Secondary,
}

fn default_app_control_click_count() -> u8 {
    1
}

fn default_app_control_drag_steps() -> u16 {
    4
}

fn default_app_control_pointer_step_delay_ms() -> u64 {
    24
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AppControlDragCommand {
    Begin {
        row_path: String,
    },
    Hover {
        row_path: String,
        placement: AppControlDragPlacement,
    },
    Drop,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AppControlPointerCommand {
    Move {
        x: f64,
        y: f64,
    },
    Press {
        x: f64,
        y: f64,
        #[serde(default)]
        button: AppControlPointerButton,
    },
    Release {
        #[serde(default)]
        button: AppControlPointerButton,
    },
    Click {
        x: f64,
        y: f64,
        #[serde(default)]
        button: AppControlPointerButton,
        #[serde(default = "default_app_control_click_count")]
        count: u8,
    },
    Drag {
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        #[serde(default)]
        button: AppControlPointerButton,
        #[serde(default = "default_app_control_drag_steps")]
        steps: u16,
        #[serde(default = "default_app_control_pointer_step_delay_ms")]
        step_delay_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AppControlKeyCommand {
    Press { keys: Vec<String> },
    Type { text: String },
}

/// How a `do` verb names the element it wants — the ONE addressing type for
/// every selector-shaped field on [`WebSurfaceDoAction`].
///
/// Why it exists: gateway and bank UIs have no stable ids. BillDesk's bank rows
/// are anonymous `div`s; IDFC's continue button is an unnamed
/// `<button type=submit>`. A CSS selector cannot name either, and a *coordinate*
/// goes stale the moment the page reflows. Text and role/label are how a human
/// names those controls, so they are how an agent should too.
///
/// It is deliberately ONE type carried by the existing `selector` fields rather
/// than a parallel `ClickText` action: a second action variant would be a second
/// encoding of "which element", and the four call sites (click, focus-for-type,
/// focus-for-key, fill) would drift apart.
///
/// `#[serde(untagged)]` with `Css(String)` first is what keeps the wire
/// compatible: every previously-written payload spells the field as a BARE
/// STRING (`"selector":"#login"`) and still deserializes, into `Css`. That
/// back-compat is pinned by `a_bare_string_selector_still_parses_as_css`.
///
/// Resolution happens AT CLICK TIME, in the page, immediately before injection —
/// so a match is never carried across a reflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WebElementRef {
    /// A CSS selector, resolved with `document.querySelector` (first match).
    Css(String),
    /// The element whose visible text (or `aria-label`, or an input's `value`)
    /// matches. Ties are broken deterministically: candidates in document order,
    /// ancestors of another candidate dropped (so a `contains` match on `<body>`
    /// never wins over the button inside it), then `nth` (default 0).
    Text {
        text: String,
        /// Exact (trimmed, whitespace-collapsed) equality instead of substring.
        #[serde(default)]
        exact: bool,
        /// Restrict the candidate set to a CSS selector (e.g. `button`).
        #[serde(default)]
        tag: Option<String>,
        /// Which match to take when several remain. Default 0.
        #[serde(default)]
        nth: Option<usize>,
    },
    /// The element with an explicit or implicit ARIA `role` whose accessible
    /// label matches. Exact label matches are preferred over substring ones; the
    /// preference is a fixed rule, not a heuristic that can vary per run.
    Role {
        role: String,
        label: String,
        #[serde(default)]
        nth: Option<usize>,
    },
    /// The `nth` match of a CSS selector — `document.querySelectorAll(css)[nth]`.
    ///
    /// [`Self::Css`] IS this with `nth: 0`; both arms compile to the SAME
    /// matcher (there is one CSS resolution rule, not two). The bare-string form
    /// survives only because every payload ever written spells it that way.
    ///
    /// Why an index is needed at all: some portals render repeated party blocks
    /// and the opposite-party block with the SAME element ids (`#Name`,
    /// `#District`…). `querySelector` silently answers with the first, so an
    /// agent aiming at the second block drove the first — twice, in the measured
    /// filing run, with every response reporting success.
    CssNth {
        css: String,
        #[serde(default)]
        nth: usize,
    },
}

impl WebElementRef {
    /// A short, log-safe description of what was asked for. Never a page value.
    pub fn describe(&self) -> String {
        match self {
            Self::Css(selector) => format!("css:{selector}"),
            Self::Text { text, exact, .. } => {
                if *exact {
                    format!("text=:{text}")
                } else {
                    format!("text~:{text}")
                }
            }
            Self::Role { role, label, .. } => format!("role:{role}[{label}]"),
            // `nth: 0` describes exactly like the bare form, because it IS the
            // bare form.
            Self::CssNth { css, nth } if *nth == 0 => format!("css:{css}"),
            Self::CssNth { css, nth } => format!("css:{css}[{nth}]"),
        }
    }
}

/// Which way a `cookies` verb moves a jar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebCookieDirection {
    /// Read the surface's jar into a Netscape file. Read-only.
    Export,
    /// Write a Netscape file's cookies into the surface's jar.
    ///
    /// ⚠ The jar is per-PROFILE, and a surface with no explicit profile is
    /// `default` — the USER'S OWN browsing jar. Drive agent work on a
    /// `--profile agent-<n>` surface before importing anything.
    Import,
}

/// Which vault record a `fill-vault` verb reads from.
///
/// One command with a source, not two commands: the page-origin guard, the
/// injection path, the redaction rules and the response shape are identical —
/// only the door to the vault differs. Two commands would be a second encoding
/// of all of that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VaultFieldSource {
    /// A login item: `password`, `username`, `totp`, `notes`. Read from the
    /// `ychrome-vault` CLI, whose stdout carries the value.
    #[default]
    Login,
    /// A card item: `number`, `code`, `holder`, `exp-month`, `exp-year`,
    /// `expiry` (`MM/YY`).
    ///
    /// Read from the vault AGENT SOCKET (`{"op":"card-secret"}`), never the CLI
    /// — `ychrome-vault` deliberately has no verb that prints a PAN, because a
    /// number in a scrollback or an agent CLI's JSONL is durable and, unlike a
    /// password, cannot be rotated. Aiming this at the CLI is exactly the bug
    /// that produced `vault_cli_no_card_op` at a live gateway's card form.
    ///
    /// The only policy refusal is the LOCK (`vault_locked`); an unlocked vault
    /// serves a card to whoever reaches its socket, as every Bitwarden client
    /// does. See `ychrome/docs/vault.md`.
    Card,
}

/// HOW a `fill` puts the text into the field.
///
/// Two mechanisms exist because two different widget families need opposite
/// things, and picking the wrong one fails SILENTLY:
///
/// - **Real keys** drive a widget that keeps its own internal state (a segmented
///   OTP whose focus auto-advances, a component that ignores a scripted value
///   write). Measured on a live portal's segmented OTP input.
/// - **The native setter** (`HTMLInputElement.prototype.value` descriptor, then
///   bubbling `input`/`change`, then blur) is what a REACT CONTROLLED input
///   needs. Measured live 2026-07-26: a 19-character per-key fill left
///   the field holding `Ja` — React re-rendered from state between injected
///   keystrokes and threw the rest away, while the verb reported `chars: 19`.
///
/// `Auto` is the ONE rule that chooses; it is not a per-caller heuristic. See
/// `web_do_fill_mechanism` in the shell (the single owner of the decision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebFillMechanism {
    /// Let the rule decide from what the pinned element actually is.
    #[default]
    Auto,
    /// Force per-key GTK injection (`isTrusted: true`).
    RealKeys,
    /// Force the native value-setter + `input`/`change`/blur.
    NativeSetter,
}

/// One trusted action injected into a web surface's page (agent control plane
/// `do` verb, slice 2b). Delivered via GTK-level event synthesis into the target
/// webview — `isTrusted: true`, NO seat pointer moved — so a backgrounded
/// surface is actionable and the user's real cursor is never hijacked. This is
/// the ONE click-delivery primitive (docs/agent-control-plane.md F1); the older
/// synthetic `Pointer`/`Grid` JS-click paths are the untrusted, main-webview
/// legacy. Coordinates are **document-space CSS pixels**; the shell resolves
/// selectors and maps CSS→widget px (page zoom + scroll) before dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum WebSurfaceDoAction {
    /// Click at document-space CSS `(x, y)`.
    Click {
        x: f64,
        y: f64,
        #[serde(default)]
        button: AppControlPointerButton,
    },
    /// Click the element matching `selector` (engine resolves its rect + scrolls
    /// it into view, then clicks its center). Sugar over `Click`.
    ClickSelector {
        /// The element to click. A bare string is a CSS selector; an object
        /// addresses by text or role/label (see [`WebElementRef`]).
        selector: WebElementRef,
        #[serde(default)]
        button: AppControlPointerButton,
    },
    /// A real hover (drives `:hover`, tooltips, menu reveal) at CSS `(x, y)`.
    Move { x: f64, y: f64 },
    /// A smooth wheel scroll by `(dx, dy)` at CSS `(x, y)` (defaults to the
    /// viewport center). Positive `dy` scrolls content down.
    Scroll {
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        dx: f64,
        dy: f64,
    },
    /// Type `text` into the element matching `selector` (resolve + focus first);
    /// with no selector, types into the page's currently focused element.
    Type {
        text: String,
        #[serde(default)]
        selector: Option<WebElementRef>,
    },
    /// Press a single named key (e.g. `Enter`, `Tab`, `Escape`, `ArrowDown`, or
    /// a single character) with optional modifiers (`ctrl`, `shift`, `alt`,
    /// `meta`). With `selector`, the target element is focused first (so
    /// editing/navigation keys like `Backspace`/`ArrowDown` land on it); without
    /// it, the key goes to whatever the page currently focuses.
    Key {
        key: String,
        #[serde(default)]
        mods: Vec<String>,
        #[serde(default)]
        selector: Option<WebElementRef>,
    },
    /// **Set** a field to `text`: clear what is there with real keys, type the
    /// new value with real keys, then read the value back and report whether it
    /// took. `Type` APPENDS — this replaces.
    ///
    /// Why it is its own verb rather than a flag on `Type`: a field that already
    /// holds a value cannot be corrected by typing over it. Measured on
    /// a portal's 6-box segmented OTP input — writing `292244` over a prior
    /// `278347` produced **`278344`**, a MERGE of old and new digits, because
    /// nothing cleared first. The same run proved a JS/eval route cannot fix it:
    /// setting `.value` via the native setter plus `input`/`change` events, and
    /// even a synthetic `ClipboardEvent('paste')`, left the component's own
    /// internal state holding the stale digits. Only real input drives that
    /// class of widget, so clearing has to be real input too.
    ///
    /// `selectors` handles the SEGMENTED case (one box per character, focus
    /// auto-advancing): every listed box is cleared in order first, because
    /// clearing only the focused box leaves the others holding old digits —
    /// which is precisely the merge above. Then the first box is focused and the
    /// text is typed, letting the component's own auto-advance carry it.
    Fill {
        text: String,
        /// The single field to replace. Ignored when `selectors` is non-empty.
        #[serde(default)]
        selector: Option<WebElementRef>,
        /// A segmented input's boxes, in visual order.
        #[serde(default)]
        selectors: Vec<WebElementRef>,
        /// How to put the text in. Defaults to [`WebFillMechanism::Auto`], and
        /// the response always names the mechanism that actually ran.
        #[serde(default)]
        mechanism: WebFillMechanism,
        /// This text is a SECRET: keep it out of the response, out of any eval
        /// script it need not enter, and force the real-key mechanism (the
        /// native setter would put the value inside a script string). Set by
        /// `fill-vault`; a plain `do fill` may opt in with `--redact`.
        ///
        /// With it set the verification reports LENGTHS and a first-mismatch
        /// index instead of the requested/held strings — the failure is still
        /// nameable, the value still never crosses back out (F4).
        #[serde(default)]
        redact: bool,
    },
}

/// Which frame of a page a verb addresses.
///
/// Why it exists: a top-document query against a page whose content lives in an
/// iframe returns `[]` SILENTLY, and that silence reads as "the site does not
/// offer this". The BillDesk case is the measurement — its iframe held 107
/// elements while the top document had 17.
///
/// The top document is the frame at path `[]`, so "no frame" and "a frame" have
/// the same shape and a caller never has to branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebFrameRef {
    /// `window.frames[i]` of the top document.
    Index(usize),
    /// The first frame whose url contains this substring.
    UrlContains(String),
    /// An explicit descent, e.g. `[0, 2]` = the third frame of the first frame.
    /// This is the form `web frames` reports, so its output feeds straight back
    /// in.
    Path(Vec<usize>),
}

/// What structured view a `read` verb returns (agent control plane, rung 1 —
/// the cheapest, default observation an agent reaches for; docs/agent-control-
/// plane.md). Never mutates, never moves a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebSurfaceReadAs {
    /// The interactable-element tree: buttons, links, inputs, selects,
    /// textareas, `[role]`/`[contenteditable]` — each with a resolvable
    /// selector + rect, so `read` → pick → `do click --selector` composes. The
    /// default an agent reaches for first.
    #[default]
    Snapshot,
    /// Form fields only (inputs/selects/textareas) with name/type/value.
    Forms,
    /// Tables as row/col JSON.
    Tables,
    /// Article extraction — the main readable text.
    Readable,
    /// All `a[href]` links as `{text, href}`.
    Links,
    /// The page's visible text (`body.innerText`).
    Text,
    /// The serialized DOM (`outerHTML`).
    Html,
}

/// The condition a `wait` verb polls for (agent control plane, rung 2 — the
/// event-driven synchronization that replaces the screenshot-poll loop;
/// docs/agent-control-plane.md). The engine polls per-surface at a fixed
/// cadence until met or the wait times out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "until", rename_all = "snake_case")]
pub enum WebSurfaceWaitUntil {
    /// `document.readyState` is past `loading` (navigation committed).
    LoadCommitted,
    /// `document.readyState === 'complete'` (load finished).
    LoadFinished,
    /// No DOM mutations for `ms` (in-process quiescence heuristic — NOT DevTools
    /// network-idle; that is a `probe` capability, a later slice).
    Idle { ms: u64 },
    /// An element matches `css` (optionally requiring a non-zero rect).
    Selector {
        css: String,
        #[serde(default)]
        visible: bool,
    },
    /// A JS expression evaluates truthy (exceptions count as not-yet).
    Js { expr: String },
    /// The engine's CURRENT url matches `pattern` (a Rust regex, unanchored).
    ///
    /// Evaluated HOST-side from the UI process's own page-state property, with
    /// no page eval at all — which is what makes it the one predicate that
    /// survives a navigation. A 4-origin auto-submit chain (rtionline →
    /// merchant.sbi.bank.in → billdesk.com/pgidsk → pay.billdesk.com →
    /// auth.idfcfirst.bank.in) tears the content process down and rebuilds it
    /// at every hop, so any page-side predicate is unavailable exactly when the
    /// caller most needs to know where it landed.
    UrlMatches {
        pattern: String,
    },
    /// Nothing has changed for `ms`: the engine url is unchanged since the
    /// previous tick, the engine is not loading, AND the page's own mutation
    /// clock reads at least `ms`.
    ///
    /// Two observers, one predicate. The host half keeps answering while the
    /// page half is unavailable mid-navigation, and a url change resets the
    /// clock — so "settled" cannot be satisfied by a page that is quietly
    /// bouncing through redirects.
    Settled { ms: u64 },
}

fn default_grid_cols() -> u32 {
    12
}
fn default_grid_rows() -> u32 {
    8
}
fn default_grid_ttl_secs() -> u64 {
    120
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppControlGridRegion {
    #[default]
    Full,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppControlGridTarget {
    /// Pick what the user sees: `surface` when the active session has a live
    /// web surface, else `main`.
    #[default]
    Auto,
    Main,
    Surface,
}

/// Click-grid verbs — see docs/yggui-click-grid.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AppControlGridCommand {
    Show {
        #[serde(default = "default_grid_cols")]
        cols: u32,
        #[serde(default = "default_grid_rows")]
        rows: u32,
        #[serde(default)]
        region: AppControlGridRegion,
        #[serde(default)]
        target: AppControlGridTarget,
        #[serde(default = "default_grid_ttl_secs")]
        ttl_secs: u64,
    },
    Click {
        cell: String,
        #[serde(default)]
        button: AppControlPointerButton,
        #[serde(default = "default_app_control_click_count")]
        count: u8,
        /// Subdivide the cell into a labeled 3×3 instead of clicking.
        #[serde(default)]
        refine: bool,
        /// Keep the grid visible after the click (default hides it).
        #[serde(default)]
        keep: bool,
    },
    Hover {
        cell: String,
        #[serde(default)]
        keep: bool,
    },
    Hide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlStartAction {
    Agent,
    Terminal,
    Ssh,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppControlCommand {
    SetMainZoom {
        value: f32,
        #[serde(default)]
        view_mode: Option<AppControlViewMode>,
    },
    SetSearch {
        query: String,
        #[serde(default)]
        focused: Option<bool>,
    },
    SetRightPanelMode {
        mode: AppControlRightPanelMode,
    },
    SetUiTheme {
        theme: UiTheme,
    },
    SetThemeEditorOpen {
        open: bool,
    },
    ResetThemeEditor,
    SetThemeEditorValues {
        #[serde(default)]
        brightness: Option<f32>,
        #[serde(default)]
        alpha: Option<f32>,
        #[serde(default)]
        grain: Option<f32>,
    },
    TriggerUpdateCheck,
    /// Restart into a staged update.
    ///
    /// Refuses while an agent holds a live web-surface lease unless `force`.
    RestartPendingUpdate {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        force: bool,
    },
    CaptureScreenshot {
        target: ScreenshotTarget,
        output_path: String,
        /// `--backend os`: force an OS-compositor grab of the window so NATIVE
        /// child widgets (web-surface webviews) appear in the frame. The default
        /// composite/DOM backends are blind to them. Defaults false so requests
        /// from older CLIs keep the existing behavior.
        #[serde(default)]
        compositor: bool,
    },
    ScrollPreview {
        #[serde(default)]
        top_px: Option<f64>,
        #[serde(default)]
        ratio: Option<f64>,
    },
    ScrollRightPanel {
        #[serde(default)]
        top_px: Option<f64>,
        #[serde(default)]
        ratio: Option<f64>,
    },
    SetPreviewLayout {
        layout: AppControlPreviewLayout,
    },
    CaptureScreenRecording {
        output_path: String,
        duration_secs: u64,
    },
    SetMaximized {
        enabled: bool,
    },
    SetFullscreen {
        enabled: bool,
    },
    SetWindowChromeHover {
        active: bool,
    },
    SetClipboardContents {
        contents: YgguiClipboardContents,
    },
    BackgroundWindow,
    /// Monitoring override: make the GUI BEHAVE as foregrounded regardless of
    /// real OS focus/backgrounding — active session stays hot (reads un-paused,
    /// full write-frame budget, hot warmer running) and screenshots stay fresh.
    SetForceForeground {
        enabled: bool,
    },
    MoveWindowBy {
        delta_x: f64,
        delta_y: f64,
    },
    ResizeWindow {
        width: f64,
        height: f64,
    },
    CloseWindow,
    CloseWindowPreservingSessions {
        #[serde(default)]
        reason: Option<String>,
        /// Close anyway while an agent lease is live. Same guard as
        /// `RestartPendingUpdate` — guarding one deploy door and leaving the
        /// other open would be the surface inconsistency the house rules call
        /// a spec violation.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        force: bool,
    },
    Pointer {
        command: AppControlPointerCommand,
    },
    Key {
        command: AppControlKeyCommand,
    },
    /// Agent pointer-targeting overlay (docs/yggui-click-grid.md): draw a
    /// labeled grid over a yggui surface, resolve cells to coordinates
    /// server-side, dispatch clicks/hovers. Targets the main webview or the
    /// active session's native child webview (page coordinates).
    Grid {
        command: AppControlGridCommand,
    },
    /// Agent observability probe: evaluate JS in the MAIN webview (the Dioxus
    /// chrome — sidebar, picker overlays, terminal hosts) and return the
    /// script's completion value. The missing eye for main-webview DOM state
    /// (focus, rects, attribute reads) that `app web eval` (native child
    /// webviews) cannot see. The script body must `return` a JSON-serializable
    /// value.
    DomEval {
        script: String,
    },
    Drag {
        command: AppControlDragCommand,
    },
    ShowStartPage,
    StartAction {
        action: AppControlStartAction,
    },
    CreateTerminal {
        #[serde(default)]
        machine_key: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        title_hint: Option<String>,
        /// What this agent-plane session is FOR, in the agent's own words
        /// (`--purpose`). Folded with the request's agent identity into the
        /// row title when no explicit `--title` was given, so an agent's
        /// scratch row is never title-indistinguishable from a human's shell.
        #[serde(default)]
        purpose: Option<String>,
        #[serde(default)]
        session_kind: Option<SessionKind>,
        /// `Some(false)` = create WITHOUT switching the user's active view
        /// (agent-driven probe/background spawns). `None`/`Some(true)` = the
        /// existing activate-on-create behavior.
        #[serde(default)]
        activate: Option<bool>,
    },
    SendTerminalInput {
        session_path: String,
        data: String,
    },
    /// Readiness-gated prompt insertion: wait until the session is at an idle
    /// interactive prompt (up to `timeout_ms`), then send `data`. If it never
    /// becomes ready, send NOTHING and report it. The robust path for agent /
    /// automation prompt insertion — see TerminalManager::submit_prompt.
    SubmitTerminalPrompt {
        session_path: String,
        data: String,
        #[serde(default)]
        timeout_ms: u64,
    },
    ReclaimTerminalFocus {
        session_path: String,
    },
    RedrawTerminal {
        session_path: String,
    },
    /// Reconcile the client xterm.js buffer FROM the daemon's authoritative vt100
    /// screen: read `server terminal screen` and replay it into the client via the
    /// same `daemon_screen_snapshot` retained-replay path the reveal-reconcile uses.
    /// Unlike `RedrawTerminal` (which only re-fits/refreshes the renderer), this
    /// repaints CONTENT — so it closes a "squish" (client frame smaller than the
    /// daemon grid) or a broken-bottom where codex delta-rendered while the client
    /// was transiently mis-sized. One-shot + idempotent (the replay layer dedups).
    /// The on-demand primitive behind the squish/reveal reconcile fix (TODO-3).
    ReconcileTerminalFromDaemon {
        session_path: String,
    },
    /// Drive the terminal viewport scroll position directly (not synthetic wheel
    /// events), so an agent can scroll/navigate scrollback via app control and
    /// verify movement. `to` is "top", "bottom", or a signed line delta
    /// ("-10" up, "20" down). Sets UserScrollback intent for a delta/top so the
    /// position sticks; "bottom" returns to follow.
    ScrollTerminalViewport {
        session_path: String,
        to: String,
    },
    /// Read the xterm.js buffer text via the buffer API (term.buffer.active
    /// getLine/translateToString) — focus-independent, unlike DOM innerText
    /// which goes empty on an unfocused Wayland window. This is "endpoint B"
    /// (after xterm.js) for the before/after-xterm buffer-integrity comparison
    /// against `server terminal screen` (endpoint A, the daemon vt100 screen).
    /// `mode` = "screen" (visible rows only) or "full" (whole buffer incl
    /// scrollback).
    ReadTerminalBuffer {
        session_path: String,
        #[serde(default)]
        mode: String,
    },
    PasteTerminalClipboard {
        session_path: String,
    },
    PasteTerminalClipboardImage {
        session_path: String,
    },
    ProbeTerminalViewportInput {
        session_path: String,
        data: String,
        #[serde(default)]
        mode: ProbeTerminalViewportInputMode,
        #[serde(default)]
        per_char: bool,
        #[serde(default)]
        press_enter: bool,
        #[serde(default)]
        press_tab: bool,
        #[serde(default)]
        press_ctrl_c: bool,
        #[serde(default)]
        press_ctrl_e: bool,
        #[serde(default)]
        press_ctrl_u: bool,
    },
    ProbeTerminalViewportScroll {
        session_path: String,
        lines: i32,
    },
    ProbeTerminalViewportSelect {
        session_path: String,
    },
    ProbeTerminalPrimarySelectionPaste {
        session_path: String,
        data: String,
    },
    ProbeTerminalContextMenu {
        session_path: String,
    },
    RemoveSession {
        session_path: String,
    },
    /// Rename a session through the real tree-rename pipeline (sidebar title).
    /// Lets the agent drive + verify renames (e.g. Claude Code custom-title
    /// write-back) without a human gesture.
    RenameSession {
        session_path: String,
        title: String,
    },
    /// Force-restart a live session through the same path as the context-menu
    /// "Restart Session" action, so the agent can drive + verify restarts.
    RestartSession {
        session_path: String,
    },
    SetSessionKeepAlive {
        session_path: String,
        keep_alive: bool,
    },
    /// Create a split-view group ([[campaign-split-view-groups]]) from `members`
    /// (session paths in pane order), arranged along `axis`. Grouping forces
    /// keep-alive on every member. Drives the yggui split surface end to end.
    CreateSplitGroup {
        members: Vec<String>,
        /// "side-by-side" (default) or "stacked".
        #[serde(default)]
        axis: Option<String>,
    },
    /// Split one of a web-surface session's tabs into its own pane — split-tabs
    /// ([[campaign-libyggterm]] Phase 3): pane 0 keeps the session's own
    /// surface, pane 1 is PINNED to `tab`. Tabs are GUI chrome, so this is a
    /// GUI-side act with no app involvement.
    SplitWebTab {
        session_path: String,
        tab: u64,
        /// "side-by-side" (default) or "stacked".
        #[serde(default)]
        axis: Option<String>,
    },
    /// Dissolve a split-view group, restoring each member's pre-group keep-alive.
    UngroupSplitGroup {
        group_id: String,
    },
    /// Move a split group's divider ratio (fraction the first pane occupies).
    SetSplitGroupRatio {
        group_id: String,
        ratio: f32,
    },
    /// Focus a pane (make its session the input target) within its split group.
    /// `pane` names the pane INDEX for groups where a session seats two panes
    /// (split-tabs) — and is the ONLY way to focus a pinned web pane headlessly,
    /// since its native webview swallows pointer events.
    FocusSplitPane {
        session_path: String,
        #[serde(default)]
        pane: Option<usize>,
    },
    SetRowExpanded {
        row_path: String,
        expanded: bool,
    },
    SetTreeSelection {
        paths: Vec<String>,
        #[serde(default)]
        anchor_path: Option<String>,
    },
    /// Evaluate JS in a session's active web-surface tab (ychrome). The
    /// completion value comes back as JSON in `data.value`; a JS exception
    /// comes back as the error. Agent automation's page-scripting primitive.
    WebSurfaceEval {
        /// Session owning the surface; None = active session.
        #[serde(default)]
        session_path: Option<String>,
        script: String,
        /// Run in this frame instead of the top document.
        ///
        /// ⚠ A `#[serde(default)]` field is DROPPED without complaint by an
        /// older GUI, which would put `--frame` right back to querying the top
        /// document silently — the exact failure this exists to kill. So the
        /// response ECHOES `frame_resolved`, and the CLI hard-fails when the
        /// echo is missing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame: Option<WebFrameRef>,
    },
    /// Capture a session's active web-surface tab: the FULL DOCUMENT (whole
    /// page, beyond the visible viewport) rendered to a PNG by the engine.
    WebSurfaceScreenshot {
        #[serde(default)]
        session_path: Option<String>,
        output_path: String,
    },
    /// Run an ASYNC script and return its resolved value — the ONE async
    /// bridge.
    ///
    /// `eval` returns a script's COMPLETION value, and a Promise is not a
    /// serializable completion value: the engine answers
    /// `WEBKIT_JAVASCRIPT_ERROR_INVALID_RESULT`. Every caller that needed
    /// `fetch`, `await el.decode()`, or any other promise therefore invented
    /// the same workaround — stash the result on `window` and poll it with a
    /// second verb. This owns that idiom in ONE place so nobody writes it
    /// again, and so the two ways it goes wrong (a poll that fails mid-
    /// navigation, a document replaced under the stash) get honest answers
    /// instead of a fabricated one.
    WebSurfaceAwait {
        #[serde(default)]
        session_path: Option<String>,
        /// The body of an async function. `return` its value.
        script: String,
        /// How long to wait for the promise to settle.
        timeout_ms: u64,
    },
    /// Enumerate the page's frames: url, element counts, and whether each is
    /// reachable from the top document's realm.
    ///
    /// The instrument the records run lacked. A cross-origin frame is REPORTED
    /// (with `accessible: false` and the reason) rather than omitted — knowing
    /// a frame exists and cannot be read is a completely different fact from
    /// there being no frame.
    WebSurfaceFrames {
        #[serde(default)]
        session_path: Option<String>,
    },
    /// Move a session's web-surface cookie jar to or from a Netscape file.
    ///
    /// This is what makes "script it on curl, hand the session to a surface for
    /// the one interactive step, hand it back" possible. It was proven both
    /// necessary AND sufficient in the field: transplanting a single PHPSESSID
    /// into a browser made rtionline render the applicant's name and the fee.
    ///
    /// ⚠ The cookie manager is per-`WebContext` = per-PROFILE, and a surface
    /// with no explicit profile is `default`, i.e. the user's own browsing jar.
    /// The response reports which profile was written; the trace records
    /// domains and counts and NEVER values.
    WebSurfaceCookies {
        #[serde(default)]
        session_path: Option<String>,
        direction: WebCookieDirection,
        /// The Netscape jar file to read or write. Absolutized CLI-side.
        jar_path: String,
    },
    /// Rasterize ONE addressed element to a PNG, IN THE PAGE.
    ///
    /// `canvas.drawImage(el)` + `toDataURL()` — no compositor, no window
    /// mapping, no screenshot backend. That is the whole point: it works on an
    /// unmapped/headless surface today, which retires the "needs an offscreen
    /// renderer" deferral and unblocks the things an agent actually gets stuck
    /// on — captchas, QR codes, charts, signature pads.
    ///
    /// Both `drawImage` of a decoded image and `toDataURL` are SYNCHRONOUS, so
    /// the whole capture is one plain completion value and never touches the
    /// async bridge (`eval` cannot return a Promise).
    ///
    /// Only genuinely rasterizable elements work — `<img>`, `<canvas>`,
    /// `<video>`. There is no in-page rasterizer for arbitrary DOM and this
    /// verb does not pretend otherwise: a `div` gets
    /// `element_not_rasterizable`, an undecoded image gets `image_not_decoded`,
    /// and a cross-origin image without CORS gets `tainted_canvas` — three
    /// different facts, three different reasons.
    WebSurfaceCaptureElement {
        #[serde(default)]
        session_path: Option<String>,
        /// What to capture. Same addressing as `do` (see [`WebElementRef`]).
        target: WebElementRef,
        output_path: String,
        /// Also write `<out>-1.png … <out>-n.png`, the image cut into `n` equal
        /// vertical bands. The per-character captcha case.
        #[serde(default)]
        split: Option<usize>,
    },
    /// Open/close the WebKit inspector (devtools) on a session's active
    /// web-surface tab.
    WebSurfaceDevtools {
        #[serde(default)]
        session_path: Option<String>,
        open: bool,
    },
    /// Fill the login form on a session's active web-surface tab from the
    /// local password vault (rbw/Bitwarden). The GUI resolves the page's REAL
    /// origin from the engine (the page cannot lie about it), queries the
    /// vault for an exact-host match, and injects the credential — key
    /// material never leaves the GUI host process except into the matching
    /// page itself.
    WebSurfaceFill {
        #[serde(default)]
        session_path: Option<String>,
        /// Explicit vault entry NAME to fill (skips host matching — the user's
        /// override path). None = exact-host auto match.
        #[serde(default)]
        entry: Option<String>,
        /// Username disambiguator when several entries share `entry`'s name.
        #[serde(default)]
        user: Option<String>,
    },
    /// Type ONE named vault field into ONE addressed element, with real keys.
    ///
    /// The secret never reaches argv, stdout, a log, or the agent's transcript:
    /// the GUI shells out to `ychrome-vault` IN-PROCESS, holds the value only
    /// long enough to synthesize keystrokes into the page, and answers with a
    /// LENGTH and a page-side boolean. The trace event carries the item and
    /// field NAMES only.
    ///
    /// Distinct from `WebSurfaceFill`, which auto-matches a login form by host
    /// and writes both fields. This one is for the case a form cannot be
    /// auto-matched — a bank's login page, a gateway's card box — where the
    /// agent has already read the page and knows exactly which element it
    /// wants filled.
    WebSurfaceFillVault {
        #[serde(default)]
        session_path: Option<String>,
        /// The element to type into. Same addressing as `do` (see
        /// [`WebElementRef`]) — CSS, visible text, or role+label.
        target: WebElementRef,
        /// Vault entry NAME.
        item: String,
        /// Which field of it.
        field: String,
        /// Username disambiguator when several entries share `item`'s name.
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        source: VaultFieldSource,
        /// Pin the surface incarnation (F3), same meaning as on `WebSurfaceDo`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generation: Option<u64>,
    },
    /// Put a vault entry's current TOTP code into the page's one-time-code
    /// field (and onto the clipboard). Same entry/user semantics as fill.
    WebSurfaceTotp {
        #[serde(default)]
        session_path: Option<String>,
        #[serde(default)]
        entry: Option<String>,
        #[serde(default)]
        user: Option<String>,
    },
    /// Inject a TRUSTED action into a session's active web-surface tab — the
    /// agent control plane `do` verb (slice 2b). Delivered by GTK-level event
    /// synthesis into the target webview (`isTrusted: true`, NO seat pointer
    /// moved), so a backgrounded surface is actionable and the user's cursor is
    /// never hijacked. Reaches a soft-stashed (demoted, still mapped) surface by
    /// `--session`; a fully-hidden/unmapped one fails closed with
    /// `surface_not_mapped`. The ONE click-delivery primitive
    /// (docs/agent-control-plane.md F1).
    WebSurfaceDo {
        #[serde(default)]
        session_path: Option<String>,
        action: WebSurfaceDoAction,
        /// The surface incarnation this verb was issued against (F3). When set,
        /// the shell fails closed with `stale_handle` if the webview has been
        /// destroyed and rebuilt since — so a mutating verb can never land on a
        /// page the agent never observed. Omitted = address whatever is live
        /// (the interactive/exploratory case).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generation: Option<u64>,
        /// Open a NEW agent batch on this surface before admitting the verb —
        /// the reset the `preempted` refusal has always told callers to perform
        /// ("start a new batch after re-observing") without giving them any way
        /// to do it.
        ///
        /// It is needed because a batch id is NOT per-invocation: it comes from
        /// `resolve_agent_identity()`, which reads the GUI PROCESS's own argv,
        /// so every agent verb for the whole life of that GUI shares one id
        /// (`"anonymous"`, since the GUI is not launched with `--agent`). Once
        /// that single id lands in a lane's `preempted_batches`, `admit` refuses
        /// it forever and `forget()` only runs on surface close/recreate — so a
        /// preempt was an unrecoverable lockout, not a yield.
        ///
        /// Deliberately explicit: the agent is asserting it has re-observed the
        /// page, which is exactly the contract gate 9 wants before an agent
        /// resumes driving a surface a human may have touched.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        new_batch: bool,
    },
    /// Run N `do` actions inside ONE explicitly-opened agent batch (agent
    /// control plane `batch`).
    ///
    /// A `do` verb is one app-control round trip; a 31-field form is 31 of them,
    /// each paying resolve + gate + arm + read and each a fresh chance for the
    /// lane to close underneath. A batch resolves the surface and opens the lane
    /// once, then runs exactly the same per-action unit a single `do` runs.
    ///
    /// It buys throughput, NOT immunity: the surface's seat-input counter is
    /// re-read between actions, and real human input aborts the remainder with
    /// `preempted` and `remaining: n`. The human wins mid-batch.
    WebSurfaceBatch {
        #[serde(default)]
        session_path: Option<String>,
        actions: Vec<WebSurfaceDoAction>,
        /// Pin the surface incarnation the batch was planned against (F3), same
        /// meaning as on `WebSurfaceDo`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generation: Option<u64>,
        /// Stop at the first action that fails. Default false: a form fill
        /// where one optional field is missing should still deliver the other
        /// thirty, and the per-action report names what failed.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        stop_on_error: bool,
    },
    /// Structured, read-only observation of a session's active web-surface tab —
    /// the agent control plane `read` verb (slice 2b, rung 1). Returns the
    /// interactable tree / forms / tables / readable / links / text / html as
    /// JSON. Pure observation (no pointer, no mutation) → classified read-only.
    /// Secret field values are masked in the output (F4).
    WebSurfaceRead {
        #[serde(default)]
        session_path: Option<String>,
        #[serde(default, rename = "as")]
        mode: WebSurfaceReadAs,
        /// Read only this frame. OMITTED = read EVERY accessible frame,
        /// including the top document — because a silent `[]` from the top
        /// document is the failure mode, and searching everything by default is
        /// what stops an agent concluding "the site does not offer this".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame: Option<WebFrameRef>,
    },
    /// Block until a condition holds on a session's active web-surface tab — the
    /// agent control plane `wait` verb (slice 2b, rung 2). The engine polls the
    /// condition per-surface; kills the "screenshot until it looks done"
    /// anti-pattern. Read-only (no mutation). `timeout_ms` bounds the poll.
    WebSurfaceWait {
        #[serde(default)]
        session_path: Option<String>,
        until: WebSurfaceWaitUntil,
        timeout_ms: u64,
    },
    /// Claim a session's web surface for `ttl_secs` so the background reaper
    /// leaves it alone while unattended work runs (agent control plane `lease`,
    /// slice 2b). The lease only ever EXTENDS the background hold — reaping
    /// takes the later of the two — so it can never shorten a surface's life.
    /// `ttl_secs: 0` releases the lease and returns the surface to the hold.
    WebSurfaceLease {
        #[serde(default)]
        session_path: Option<String>,
        ttl_secs: u64,
    },
    /// Headless surface-create (agent control plane slice 2): materialize a
    /// BACKGROUNDED session's declared web surfaces now — created straight
    /// into the soft stash (demoted, never revealed, no page hole) and leased
    /// for `ttl_secs`, so an agent can drive them without the user's view
    /// ever changing.
    ///
    /// REFUSES `session_closed` when the session's runtime is gone AND the
    /// user's close of its row is remembered. Reviving there produces the one
    /// state that must not be representable — a live, leased page with no row
    /// the user can see or click into — and it happened. The remedy is the
    /// agent's OWN session; a closed row is never resurrected. Conjunctive on
    /// purpose, so the legitimate revivals (a live backgrounded session whose
    /// surface the reaper took; a session that never mounted a terminal host)
    /// still pass, and an unreachable owner counts as unknown, not dead.
    EnsureWebSurface {
        session_path: String,
        #[serde(default)]
        ttl_secs: Option<u64>,
    },
    /// Force a session's web surface into a NEW incarnation.
    ///
    /// Bumps the active tab's reload nonce; the reconciler's existing
    /// destroy-and-recreate branch does the work and mints a fresh generation.
    /// The recovery an agent previously had to reach `session remove` for.
    WebSurfaceReload {
        session_path: String,
    },
    /// Close a session's web surface.
    ///
    /// Also records the deliberate-close mark, which blocks a HEARTBEAT
    /// resurrection for a grace window but NOT an explicit `web ensure` — a
    /// heartbeat is liveness, an ensure is intent, and the rebuild path
    /// deliberately never consults that map. (Closing the SESSION is different
    /// and stronger: it destroys the surfaces outright and `ensure` then refuses
    /// `session_closed`.)
    WebSurfaceClose {
        session_path: String,
    },
    DescribeRows,
    OpenPath {
        session_path: String,
        #[serde(default)]
        view_mode: Option<AppControlViewMode>,
    },
    FocusWindow,
    DescribeState,
    /// Invoke a registry command by its stable id (e.g. `sidebar.toggle`,
    /// `notifications.toggle`, `session.next`) — the keyboard analogue of the
    /// click grid. The ALT+ KeyTips layer, this probe, and the settings modal
    /// are all VIEWS of the one command registry, so an agent drives shell
    /// commands by id instead of pixel-hunting a button. See
    /// `[[campaign-alt-keytips-layer]]`.
    InvokeCommand {
        id: String,
    },
    /// Enumerate the command registry: every command's id, title, and in-force
    /// KeyTip chord. Read-only; the discovery half of `InvokeCommand`.
    ListCommands,
    /// A well-formed request whose `kind` this build DOES know, but whose
    /// FIELDS it cannot read.
    ///
    /// `#[serde(other)]` above rescues an unknown *kind* only. It does nothing
    /// for a known kind whose payload changed shape — and that is the far more
    /// likely mismatch, because every field added to an existing command is one:
    /// a GUI predating `WebElementRef` types `selector` as a bare `String`, so
    /// `do click --text "Proceed to Pay"` sends `{"selector":{"text":"…"}}`, the
    /// whole request fails with `invalid type: map, expected a string`, and the
    /// old code DELETED the file — reproducing the exact bare timeout with no
    /// clue that `Unsupported` was written to kill.
    ///
    /// So the honest-refusal property is extended to the payload: the request is
    /// salvaged from the envelope, delivered, and refused with the serde error
    /// as the clue. It carries the error text (not the payload — the request
    /// file is still the one copy of that) because "which field, and what did it
    /// expect" is precisely what the caller needs and cannot otherwise get.
    ///
    /// It is an ordinary variant rather than `#[serde(skip)]` because the GUI
    /// SERIALIZES the command it is handling into the request trace; a skipped
    /// variant fails to serialize, and `json!` on a failing value panics. A
    /// refusal path must not be able to take the window down.
    Unreadable {
        /// The `kind` the request asked for. Named `requested_kind` because the
        /// enum's internal tag already owns `kind` on the wire.
        requested_kind: String,
        detail: String,
    },
    /// A well-formed request whose `kind` this build does not know.
    ///
    /// App-control is a FILESYSTEM DROPBOX, not RPC: a newer CLI writes a
    /// request file and an older GUI reads it. Before this variant existed,
    /// `take_next_app_control_request` failed to deserialize such a file,
    /// DELETED it, and moved on — so a version mismatch surfaced to the caller
    /// as a bare TIMEOUT with no clue that the verb simply was not implemented
    /// by the running window. `#[serde(other)]` turns that into an honest
    /// refusal: the request still parses (as this variant), the GUI answers,
    /// and the caller is told to swap the GUI binary.
    ///
    /// This is deserialize-only in practice — nothing constructs it — and it
    /// deliberately does NOT capture the unknown payload: the request file is
    /// still on disk while it is in flight, and a copy here would be a second
    /// encoding of it.
    ///
    /// Malformed JSON is still deleted; only a well-formed request with an
    /// unknown `kind` lands here.
    #[serde(other)]
    Unsupported,
}

impl AppControlCommand {
    /// True for PURE-OBSERVATION commands — they read/capture state without mutating
    /// any UI or session state, so handling one does NOT require a shell re-render.
    /// The render-churn investigation (campaign DOM-leak) found that every agent probe
    /// (screenshot/state/buffer read) currently force-re-renders the whole shell root
    /// via the app-control poll loop's schedule_ui_update(); gating that force-render on
    /// `!is_read_only()` would cut the probe-induced churn. This is the tested foundation
    /// for that gate — wiring it into the poll loop is a separate, verified step (the
    /// loop must still WAKE to process the request; only the forced re-render is skipped).
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::CaptureScreenshot { .. }
                | Self::CaptureScreenRecording { .. }
                | Self::DescribeRows
                | Self::DescribeState
                | Self::ReadTerminalBuffer { .. }
                | Self::WebSurfaceScreenshot { .. }
                | Self::WebSurfaceCaptureElement { .. }
                | Self::WebSurfaceRead { .. }
                | Self::WebSurfaceFrames { .. }
                | Self::WebSurfaceWait { .. }
                | Self::ListCommands
                // An unknown kind, and a kind whose payload could not be read,
                // are never executed — they can only be refused, so they mutate
                // nothing.
                | Self::Unsupported
                | Self::Unreadable { .. }
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SetMainZoom { .. } => "set_main_zoom",
            Self::SetSearch { .. } => "set_search",
            Self::SetRightPanelMode { .. } => "set_right_panel_mode",
            Self::SetUiTheme { .. } => "set_ui_theme",
            Self::SetThemeEditorOpen { .. } => "set_theme_editor_open",
            Self::ResetThemeEditor => "reset_theme_editor",
            Self::SetThemeEditorValues { .. } => "set_theme_editor_values",
            Self::TriggerUpdateCheck => "trigger_update_check",
            Self::RestartPendingUpdate { .. } => "restart_pending_update",
            Self::CaptureScreenshot { .. } => "capture_screenshot",
            Self::ScrollPreview { .. } => "scroll_preview",
            Self::ScrollRightPanel { .. } => "scroll_right_panel",
            Self::SetPreviewLayout { .. } => "set_preview_layout",
            Self::CaptureScreenRecording { .. } => "capture_screen_recording",
            Self::SetMaximized { .. } => "set_maximized",
            Self::SetFullscreen { .. } => "set_fullscreen",
            Self::SetWindowChromeHover { .. } => "set_window_chrome_hover",
            Self::SetClipboardContents { .. } => "set_clipboard_contents",
            Self::BackgroundWindow => "background_window",
            Self::SetForceForeground { .. } => "set_force_foreground",
            Self::MoveWindowBy { .. } => "move_window_by",
            Self::ResizeWindow { .. } => "resize_window",
            Self::CloseWindow => "close_window",
            Self::CloseWindowPreservingSessions { .. } => "close_window_preserving_sessions",
            Self::Pointer { .. } => "pointer",
            Self::DomEval { .. } => "dom-eval",
            Self::Grid { .. } => "grid",
            Self::Key { .. } => "key",
            Self::Drag { .. } => "drag",
            Self::ShowStartPage => "show_start_page",
            Self::StartAction { .. } => "start_action",
            Self::CreateTerminal { .. } => "create_terminal",
            Self::SendTerminalInput { .. } => "send_terminal_input",
            Self::SubmitTerminalPrompt { .. } => "submit_terminal_prompt",
            Self::ReclaimTerminalFocus { .. } => "reclaim_terminal_focus",
            Self::RedrawTerminal { .. } => "redraw_terminal",
            Self::ReconcileTerminalFromDaemon { .. } => "reconcile_terminal_from_daemon",
            Self::ScrollTerminalViewport { .. } => "scroll_terminal_viewport",
            Self::ReadTerminalBuffer { .. } => "read_terminal_buffer",
            Self::PasteTerminalClipboard { .. } => "paste_terminal_clipboard",
            Self::PasteTerminalClipboardImage { .. } => "paste_terminal_clipboard_image",
            Self::ProbeTerminalViewportInput { .. } => "probe_terminal_viewport_input",
            Self::ProbeTerminalViewportScroll { .. } => "probe_terminal_viewport_scroll",
            Self::ProbeTerminalViewportSelect { .. } => "probe_terminal_viewport_select",
            Self::ProbeTerminalPrimarySelectionPaste { .. } => {
                "probe_terminal_primary_selection_paste"
            }
            Self::ProbeTerminalContextMenu { .. } => "probe_terminal_context_menu",
            Self::RemoveSession { .. } => "remove_session",
            Self::RenameSession { .. } => "rename_session",
            Self::RestartSession { .. } => "restart_session",
            Self::SetSessionKeepAlive { .. } => "set_session_keep_alive",
            Self::CreateSplitGroup { .. } => "create_split_group",
            Self::SplitWebTab { .. } => "split_web_tab",
            Self::UngroupSplitGroup { .. } => "ungroup_split_group",
            Self::SetSplitGroupRatio { .. } => "set_split_group_ratio",
            Self::FocusSplitPane { .. } => "focus_split_pane",
            Self::SetRowExpanded { .. } => "set_row_expanded",
            Self::SetTreeSelection { .. } => "set_tree_selection",
            Self::WebSurfaceEval { .. } => "web_surface_eval",
            Self::WebSurfaceScreenshot { .. } => "web_surface_screenshot",
            Self::WebSurfaceCaptureElement { .. } => "web_surface_capture_element",
            Self::WebSurfaceCookies { .. } => "web_surface_cookies",
            Self::WebSurfaceDevtools { .. } => "web_surface_devtools",
            Self::WebSurfaceFill { .. } => "web_surface_fill",
            Self::WebSurfaceFillVault { .. } => "web_surface_fill_vault",
            Self::WebSurfaceTotp { .. } => "web_surface_totp",
            Self::WebSurfaceDo { .. } => "web_surface_do",
            Self::WebSurfaceBatch { .. } => "web_surface_batch",
            Self::WebSurfaceRead { .. } => "web_surface_read",
            Self::WebSurfaceFrames { .. } => "web_surface_frames",
            Self::WebSurfaceAwait { .. } => "web_surface_await",
            Self::WebSurfaceWait { .. } => "web_surface_wait",
            Self::WebSurfaceLease { .. } => "web_surface_lease",
            Self::EnsureWebSurface { .. } => "ensure_web_surface",
            Self::WebSurfaceReload { .. } => "web_surface_reload",
            Self::WebSurfaceClose { .. } => "web_surface_close",
            Self::DescribeRows => "describe_rows",
            Self::OpenPath { .. } => "open_path",
            Self::FocusWindow => "focus_window",
            Self::DescribeState => "describe_state",
            Self::InvokeCommand { .. } => "invoke_command",
            Self::ListCommands => "list_commands",
            Self::Unsupported => "unsupported",
            Self::Unreadable { .. } => "unreadable",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppControlRequest {
    pub request_id: String,
    pub created_at_ms: u128,
    #[serde(default)]
    pub preferred_pid: Option<u32>,
    /// Who is driving, for agent presence (cursor v1, `docs/agent-control-plane.md`
    /// slice 3): `--agent <id>` or `$YGGTERM_AGENT`. The window shows this
    /// agent's pointer as `agent-N` while the user watches the same session.
    /// Absent means "some agent" — every unnamed driver shares one identity,
    /// which is honest: the window genuinely cannot tell them apart.
    #[serde(default)]
    pub agent: Option<String>,
    pub command: AppControlCommand,
}

/// Process-wide agent identity, set once from `--agent` before any request is
/// built. A global rather than a parameter because every app-control call site
/// would otherwise have to thread an identity it does not care about, and they
/// all belong to the same driving agent anyway — one CLI invocation is one agent.
static AGENT_IDENTITY_OVERRIDE: std::sync::RwLock<Option<String>> =
    std::sync::RwLock::new(None);

/// Record the `--agent <id>` this invocation was given. Blank clears it.
pub fn set_agent_identity(identity: Option<&str>) {
    let cleaned = identity
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if let Ok(mut slot) = AGENT_IDENTITY_OVERRIDE.write() {
        *slot = cleaned;
    }
}

/// Agent identity stamped on outgoing app-control requests: `--agent` if this
/// invocation set one, else `$YGGTERM_AGENT`, else none. One resolver so every
/// call site agrees.
pub fn resolve_agent_identity() -> Option<String> {
    AGENT_IDENTITY_OVERRIDE
        .read()
        .ok()
        .and_then(|slot| slot.clone())
        .or_else(|| std::env::var("YGGTERM_AGENT").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Every agent-plane row title starts with this word, so a title-based probe
/// can find agent-owned rows without knowing which agent made them. The
/// original incident was the opposite: an agent's scratch row was titled from
/// its cwd exactly like a human's shell in the same directory, so every
/// title search for it missed and only the user's eyes found it.
pub const AGENT_PLANE_TITLE_PREFIX: &str = "Agent";

/// Identity used when a request carries no `--agent`/`$YGGTERM_AGENT`. The
/// request field's own contract is "absent means SOME agent", so the title
/// says that rather than pretending the row has no owner.
const AGENT_PLANE_TITLE_UNNAMED: &str = "unnamed";

/// Longest purpose fragment folded into a row title. A sidebar row is a label,
/// not a paragraph; the full purpose stays in the verb's response.
const AGENT_PLANE_TITLE_PURPOSE_MAX_CHARS: usize = 64;

/// Collapse a caller-supplied fragment to one line of printable text.
fn agent_plane_title_fragment(value: Option<&str>) -> Option<String> {
    let cleaned = value?
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|ch| !ch.is_control())
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!cleaned.is_empty()).then_some(cleaned)
}

fn agent_plane_title_truncated(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value;
    }
    value.chars().take(max_chars).collect::<String>()
}

/// The title a session created through the AGENT plane wears when the caller
/// gave no explicit `--title`.
///
/// The defect this exists for: `terminal new --kind shell` fell back to the
/// cwd, which the copy layer then humanized into `<Leaf> Shell` — the SAME
/// label a human's shell in the same directory gets. An agent could therefore
/// report a session "removed" while a row nobody could attribute stayed on
/// screen for hours.
///
/// The output must survive the copy layer, which throws away titles it judges
/// generated junk. That judgement has ONE owner
/// ([`looks_like_generated_fallback_title`]), so this asks it rather than
/// re-deriving the rule: if folding the caller's purpose in would produce a
/// title the copy layer would discard, the purpose is dropped and the bare
/// agent-and-kind form — which is fixed text this build controls — is used.
pub fn agent_plane_session_title(
    agent: Option<&str>,
    purpose: Option<&str>,
    kind: SessionKind,
) -> String {
    let identity = agent_plane_title_fragment(agent)
        .map(|value| agent_plane_title_truncated(value, AGENT_PLANE_TITLE_PURPOSE_MAX_CHARS))
        .unwrap_or_else(|| AGENT_PLANE_TITLE_UNNAMED.to_string());
    let kind_label = crate::session_kind_label(kind);
    let base = format!("{AGENT_PLANE_TITLE_PREFIX} {identity} {kind_label}");
    let Some(purpose) = agent_plane_title_fragment(purpose)
        .map(|value| agent_plane_title_truncated(value, AGENT_PLANE_TITLE_PURPOSE_MAX_CHARS))
    else {
        return base;
    };
    let with_purpose = format!("{base}: {purpose}");
    if yggterm_core::looks_like_generated_fallback_title(&with_purpose) {
        return base;
    }
    with_purpose
}

/// One process a session teardown was accountable for: the PTY child itself,
/// or anything it fathered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTeardownProcess {
    pub pid: i32,
    /// `/proc` command name at census time. Kept because it is what makes a
    /// later liveness re-probe able to refuse a RECYCLED pid, and because the
    /// report is useless to a human without it.
    pub command: String,
}

/// What was observed around a `session remove`, as facts rather than prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRemovalEvidence<'a> {
    /// Whether this row was a LIVE session before the remove. A stored row has
    /// no runtime to verify, so its removal is decided by the row check alone.
    pub row_was_live: bool,
    /// The PTY process id the owning daemon reported before the remove.
    /// `None` on a live row means nobody local could see the runtime — the
    /// preserved-owner (older daemon) case — and that is unverifiable, not
    /// clean.
    pub runtime_pid_before: Option<i32>,
    /// The PTY child plus every descendant, as seen before the remove.
    pub observed_before: &'a [SessionTeardownProcess],
    /// Of `observed_before`, the ones still running after the remove.
    pub still_running_after: &'a [SessionTeardownProcess],
    /// Whether the post-remove snapshot still lists the row.
    pub row_still_listed: bool,
}

/// Why a removal could not be verified. A machine-readable name, never prose:
/// the caller must be able to branch on it, and a daemon's message is not a
/// contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRemovalRefusal {
    /// The row is still in the live order after the remove.
    RowStillListed,
    /// Processes the session owned are still running.
    ProcessesSurvived,
    /// The row was live but no local runtime pid was visible, so there was
    /// nothing to check the teardown against. The owning daemon is older than
    /// this one, or does not report the pid at all.
    RuntimePidUnobservable,
}

impl SessionRemovalRefusal {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RowStillListed => "row_still_listed",
            Self::ProcessesSurvived => "processes_survived",
            Self::RuntimePidUnobservable => "runtime_pid_unobservable",
        }
    }
}

/// The answer a `session remove` is allowed to give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRemovalVerdict {
    pub verified: bool,
    pub refusal: Option<SessionRemovalRefusal>,
    /// Processes that were alive before and are gone after.
    pub reaped: Vec<SessionTeardownProcess>,
    /// Processes that outlived the teardown.
    pub still_running: Vec<SessionTeardownProcess>,
}

/// Decide whether a removal actually happened.
///
/// The rule this replaces was `"accepted": true` on any successful ROUND TRIP,
/// which is transport success and nothing more — it read true while the daemon
/// itself was saying "no live session for this path", and while the app the
/// session hosted was still running. Verified means all of: the row left the
/// live order, every process the session owned is gone, and there was a
/// runtime pid to check against in the first place. Anything short of that is
/// `verified: false` with a NAMED refusal and the surviving pids attached, so
/// an agent cannot truthfully-but-wrongly report a clean exit.
pub fn verify_session_removal(evidence: &SessionRemovalEvidence<'_>) -> SessionRemovalVerdict {
    let still_running = evidence.still_running_after.to_vec();
    let reaped = evidence
        .observed_before
        .iter()
        .filter(|process| !still_running.contains(process))
        .cloned()
        .collect::<Vec<_>>();
    let refusal = if evidence.row_still_listed {
        Some(SessionRemovalRefusal::RowStillListed)
    } else if !still_running.is_empty() {
        Some(SessionRemovalRefusal::ProcessesSurvived)
    } else if evidence.row_was_live && evidence.runtime_pid_before.is_none() {
        Some(SessionRemovalRefusal::RuntimePidUnobservable)
    } else {
        None
    };
    SessionRemovalVerdict {
        verified: refusal.is_none(),
        refusal,
        reaped,
        still_running,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppControlResponse {
    pub request_id: String,
    pub handled_by_pid: u32,
    pub completed_at_ms: u128,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn app_control_requests_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_REQUESTS_DIR)
}

pub fn app_control_requests_pending(home: &Path) -> bool {
    let requests_dir = app_control_requests_dir(home);
    let Ok(entries) = fs::read_dir(&requests_dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("inflight-"))
    })
}

pub fn app_control_requests_pending_for_worker(home: &Path, worker_pid: u32) -> bool {
    let requests_dir = app_control_requests_dir(home);
    let Ok(entries) = fs::read_dir(&requests_dir) else {
        return false;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json")
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("inflight-"))
        {
            continue;
        }
        let request = match fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AppControlRequest>(&bytes).ok())
        {
            Some(request) => request,
            None => {
                let _ = fs::remove_file(&path);
                continue;
            }
        };
        if let Some(preferred_pid) = request.preferred_pid
            && preferred_pid != worker_pid
        {
            remove_request_if_target_is_stale(&path, &request, preferred_pid);
            continue;
        }
        return true;
    }
    false
}

/// True if any request pending for this worker is a MUTATING (non-read-only) command —
/// i.e. handling it should force a shell re-render. Read-only probes (screenshot /
/// state / buffer reads) are processed via the waker (Poll event) without forcing a
/// whole-shell re-render, which cuts the ~4-renders-per-probe churn the DOM-leak
/// investigation found (heavy agent probing was re-rendering the giant shell tree on
/// every observation). Same scan/skip rules as `app_control_requests_pending_for_worker`.
pub fn app_control_pending_render_needed_for_worker(home: &Path, worker_pid: u32) -> bool {
    let requests_dir = app_control_requests_dir(home);
    let Ok(entries) = fs::read_dir(&requests_dir) else {
        return false;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json")
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("inflight-"))
        {
            continue;
        }
        let Some(request) = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AppControlRequest>(&bytes).ok())
        else {
            continue;
        };
        if let Some(preferred_pid) = request.preferred_pid
            && preferred_pid != worker_pid
        {
            continue;
        }
        if !request.command.is_read_only() {
            return true;
        }
    }
    false
}

pub fn app_control_responses_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_RESPONSES_DIR)
}

pub fn app_control_captures_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_CAPTURES_DIR)
}

pub fn app_control_recordings_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_RECORDINGS_DIR)
}

pub fn default_screenshot_output_path(home: &Path, request_id: &str) -> PathBuf {
    app_control_captures_dir(home).join(format!("app-{request_id}.png"))
}

pub fn default_recording_output_path(home: &Path, request_id: &str) -> PathBuf {
    app_control_recordings_dir(home).join(format!("app-{request_id}.mov"))
}

pub fn enqueue_app_control_request(
    home: &Path,
    command: AppControlCommand,
    preferred_pid: Option<u32>,
) -> Result<AppControlRequest> {
    let requests_dir = app_control_requests_dir(home);
    let captures_dir = app_control_captures_dir(home);
    let recordings_dir = app_control_recordings_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating app control requests dir {}",
            requests_dir.display()
        )
    })?;
    fs::create_dir_all(&captures_dir).with_context(|| {
        format!(
            "creating app control captures dir {}",
            captures_dir.display()
        )
    })?;
    fs::create_dir_all(&recordings_dir).with_context(|| {
        format!(
            "creating app control recordings dir {}",
            recordings_dir.display()
        )
    })?;
    let request = AppControlRequest {
        request_id: Uuid::new_v4().to_string(),
        created_at_ms: current_millis(),
        preferred_pid,
        agent: resolve_agent_identity(),
        command,
    };
    let final_path = requests_dir.join(format!("{}.json", request.request_id));
    let temp_path = requests_dir.join(format!("{}.json.tmp", request.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(&request)?)
        .with_context(|| format!("writing app control request {}", temp_path.display()))?;
    fs::rename(&temp_path, &final_path)
        .with_context(|| format!("publishing app control request {}", final_path.display()))?;
    Ok(request)
}

pub fn take_next_app_control_request(
    home: &Path,
    worker_pid: u32,
) -> Result<Option<(PathBuf, AppControlRequest)>> {
    let requests_dir = app_control_requests_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating app control requests dir {}",
            requests_dir.display()
        )
    })?;
    recover_stale_inflight_requests(&requests_dir)?;
    let mut entries = fs::read_dir(&requests_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("inflight-"))
        })
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let request = match serde_json::from_slice::<AppControlRequest>(&bytes) {
            Ok(request) => request,
            // A request this build cannot read is ANSWERED when it is
            // answerable, and only deleted when it is not. See
            // `salvage_unreadable_request`.
            Err(error) => match salvage_unreadable_request(&bytes, &error) {
                Some(request) => request,
                None => {
                    let _ = fs::remove_file(&path);
                    continue;
                }
            },
        };
        if let Some(preferred_pid) = request.preferred_pid
            && preferred_pid != worker_pid
        {
            remove_request_if_target_is_stale(&path, &request, preferred_pid);
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("request.json");
        let inflight_path = requests_dir.join(format!("inflight-{worker_pid}-{file_name}"));
        if fs::rename(&path, &inflight_path).is_err() {
            continue;
        }
        return Ok(Some((inflight_path, request)));
    }
    Ok(None)
}

/// Rescue a request whose ENVELOPE is intact but whose command payload this
/// build cannot deserialize, so it can be refused with a reason instead of
/// vanishing.
///
/// This is the other half of `AppControlCommand::Unsupported`. That variant
/// covers an unknown `kind`; this covers a KNOWN kind whose fields changed
/// shape — the mismatch a `#[serde(default)]` field on an existing command
/// produces, and the one that reproduced the bare timeout P0 was written to
/// kill (`do click --text` against a GUI that types `selector` as a string).
///
/// It salvages ONLY when the envelope is genuinely a request: an object with a
/// non-empty string `request_id` and an object `command`. Anything else —
/// truncated JSON, a stray file, a request with no `command` — is not a version
/// mismatch and is still deleted, so a corrupt file cannot become a
/// silently-accepted request. `created_at_ms` falls back to now, which only
/// affects the stale-target window and is the safe direction (a salvaged
/// request is not treated as ancient and dropped).
fn salvage_unreadable_request(
    bytes: &[u8],
    error: &serde_json::Error,
) -> Option<AppControlRequest> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    let object = value.as_object()?;
    let request_id = object.get("request_id")?.as_str()?.trim().to_string();
    if request_id.is_empty() {
        return None;
    }
    let command = object.get("command")?.as_object()?;
    let requested_kind = command
        .get("kind")
        .and_then(|kind| kind.as_str())
        .unwrap_or("")
        .to_string();
    Some(AppControlRequest {
        request_id,
        created_at_ms: object
            .get("created_at_ms")
            .and_then(serde_json::Value::as_u64)
            .map(u128::from)
            .unwrap_or_else(current_millis),
        preferred_pid: object
            .get("preferred_pid")
            .and_then(serde_json::Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok()),
        agent: object
            .get("agent")
            .and_then(|agent| agent.as_str())
            .map(ToOwned::to_owned),
        command: AppControlCommand::Unreadable {
            requested_kind,
            detail: error.to_string(),
        },
    })
}

fn remove_request_if_target_is_stale(path: &Path, request: &AppControlRequest, preferred_pid: u32) {
    let request_age_ms = current_millis().saturating_sub(request.created_at_ms);
    if !process_is_alive(preferred_pid) || request_age_ms > STALE_TARGETED_APP_CONTROL_REQUEST_MS {
        let _ = fs::remove_file(path);
    }
}

fn recover_stale_inflight_requests(requests_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(requests_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("inflight-")
            || path.extension().and_then(|ext| ext.to_str()) != Some("json")
        {
            continue;
        }
        let Some(worker_pid) = parse_inflight_worker_pid(name) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        if process_is_alive(worker_pid) {
            continue;
        }
        let Some(original_name) = name.splitn(3, '-').nth(2) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        let recovered_path = requests_dir.join(original_name);
        if recovered_path.exists() {
            let _ = fs::remove_file(&path);
            continue;
        }
        let _ = fs::rename(&path, &recovered_path);
    }
    Ok(())
}

fn parse_inflight_worker_pid(file_name: &str) -> Option<u32> {
    let rest = file_name.strip_prefix("inflight-")?;
    let pid = rest.split('-').next()?;
    pid.parse().ok()
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(not(unix))]
fn process_is_alive(pid: u32) -> bool {
    pid != 0
}

pub fn complete_app_control_request(
    home: &Path,
    inflight_path: &Path,
    response: &AppControlResponse,
) -> Result<PathBuf> {
    let responses_dir = app_control_responses_dir(home);
    fs::create_dir_all(&responses_dir).with_context(|| {
        format!(
            "creating app control responses dir {}",
            responses_dir.display()
        )
    })?;
    let response_path = responses_dir.join(format!("{}.json", response.request_id));
    let temp_path = responses_dir.join(format!("{}.json.tmp", response.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(response)?)
        .with_context(|| format!("writing app control response {}", temp_path.display()))?;
    fs::rename(&temp_path, &response_path).with_context(|| {
        format!(
            "publishing app control response {}",
            response_path.display()
        )
    })?;
    let _ = fs::remove_file(inflight_path);
    // Prune ORPHANED responses: a response is normally deleted by the CLI when it
    // reads it (await_app_control_response), but if the client TIMED OUT before the
    // response was written it never reads/deletes it → the file leaks forever (1177
    // accumulated during a heavy agent-probing session). Sweep responses older than
    // the TTL on each write so orphans can't accumulate unboundedly. Cheap + bounded.
    prune_stale_app_control_responses(&responses_dir, APP_CONTROL_RESPONSE_ORPHAN_TTL);
    Ok(response_path)
}

const APP_CONTROL_RESPONSE_ORPHAN_TTL: Duration = Duration::from_secs(120);

/// Remove app-control response files older than `ttl` — orphans whose client timed
/// out before reading them (the read path deletes the rest). Best-effort.
fn prune_stale_app_control_responses(responses_dir: &Path, ttl: Duration) {
    let Ok(entries) = fs::read_dir(responses_dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > ttl);
        if stale {
            let _ = fs::remove_file(&path);
        }
    }
}

pub fn wait_for_app_control_response(
    home: &Path,
    request_id: &str,
    timeout: Duration,
) -> Result<AppControlResponse> {
    let response_path = app_control_responses_dir(home).join(format!("{request_id}.json"));
    let started = std::time::Instant::now();
    while started.elapsed() <= timeout {
        if response_path.is_file() {
            let bytes = fs::read(&response_path).with_context(|| {
                format!("reading app control response {}", response_path.display())
            })?;
            let response =
                serde_json::from_slice::<AppControlResponse>(&bytes).with_context(|| {
                    format!("parsing app control response {}", response_path.display())
                })?;
            let _ = fs::remove_file(&response_path);
            return Ok(response);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!(
        "timed out waiting for app control response {} after {} ms",
        request_id,
        timeout.as_millis()
    )
}

pub fn enqueue_screenshot_request(
    home: &Path,
    target: ScreenshotTarget,
    output_path: Option<PathBuf>,
    preferred_pid: Option<u32>,
    compositor: bool,
) -> Result<AppControlRequest> {
    let request_id = Uuid::new_v4().to_string();
    let output_path = output_path
        .unwrap_or_else(|| default_screenshot_output_path(home, &request_id))
        .display()
        .to_string();
    let request = AppControlRequest {
        request_id,
        created_at_ms: current_millis(),
        preferred_pid,
        agent: resolve_agent_identity(),
        command: AppControlCommand::CaptureScreenshot {
            target,
            output_path,
            compositor,
        },
    };
    let requests_dir = app_control_requests_dir(home);
    let captures_dir = app_control_captures_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating app control requests dir {}",
            requests_dir.display()
        )
    })?;
    fs::create_dir_all(&captures_dir).with_context(|| {
        format!(
            "creating app control captures dir {}",
            captures_dir.display()
        )
    })?;
    let final_path = requests_dir.join(format!("{}.json", request.request_id));
    let temp_path = requests_dir.join(format!("{}.json.tmp", request.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(&request)?)
        .with_context(|| format!("writing app control request {}", temp_path.display()))?;
    fs::rename(&temp_path, &final_path)
        .with_context(|| format!("publishing app control request {}", final_path.display()))?;
    Ok(request)
}

pub fn enqueue_screen_recording_request(
    home: &Path,
    output_path: Option<PathBuf>,
    duration_secs: u64,
    preferred_pid: Option<u32>,
) -> Result<AppControlRequest> {
    let request_id = Uuid::new_v4().to_string();
    let output_path = output_path
        .unwrap_or_else(|| default_recording_output_path(home, &request_id))
        .display()
        .to_string();
    let request = AppControlRequest {
        request_id,
        created_at_ms: current_millis(),
        preferred_pid,
        agent: resolve_agent_identity(),
        command: AppControlCommand::CaptureScreenRecording {
            output_path,
            duration_secs: duration_secs.max(1),
        },
    };
    let requests_dir = app_control_requests_dir(home);
    let recordings_dir = app_control_recordings_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating app control requests dir {}",
            requests_dir.display()
        )
    })?;
    fs::create_dir_all(&recordings_dir).with_context(|| {
        format!(
            "creating app control recordings dir {}",
            recordings_dir.display()
        )
    })?;
    let final_path = requests_dir.join(format!("{}.json", request.request_id));
    let temp_path = requests_dir.join(format!("{}.json.tmp", request.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(&request)?)
        .with_context(|| format!("writing app control request {}", temp_path.display()))?;
    fs::rename(&temp_path, &final_path)
        .with_context(|| format!("publishing app control request {}", final_path.display()))?;
    Ok(request)
}

pub fn current_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request carrying a `kind` this build has never heard of must PARSE
    /// (into `Unsupported`) rather than fail deserialization.
    ///
    /// Why it matters: `take_next_app_control_request` deletes a request it
    /// cannot deserialize and moves on, so before `#[serde(other)]` a newer
    /// CLI talking to an older GUI produced a bare TIMEOUT — the caller could
    /// not tell "not implemented here" from "the window is wedged". Every new
    /// verb inherits that failure mode, which is why this lands first.
    ///
    /// This test FAILS without `#[serde(other)] Unsupported`: the payload
    /// below is a well-formed `AppControlRequest` whose command kind does not
    /// exist, and serde rejects the whole request with "unknown variant".
    #[test]
    fn a_command_kind_this_build_does_not_know_parses_as_unsupported() {
        let payload = r#"{
            "request_id": "r1",
            "created_at_ms": 0,
            "command": { "kind": "web_surface_from_the_future", "session_path": "x" }
        }"#;
        let request: AppControlRequest =
            serde_json::from_str(payload).expect("an unknown kind must parse, not error");
        assert_eq!(request.command, AppControlCommand::Unsupported);
        assert_eq!(request.command.name(), "unsupported");
        // It can only ever be refused, so it mutates nothing.
        assert!(request.command.is_read_only());
    }

    /// The other half of the contract: genuinely malformed JSON must still be
    /// rejected (and therefore still deleted by the taker). `#[serde(other)]`
    /// must not turn a corrupt file into a silently-accepted request.
    #[test]
    fn malformed_json_still_fails_to_parse() {
        assert!(serde_json::from_str::<AppControlRequest>("{ not json").is_err());
        // A request missing the `command` field is malformed, not "unknown kind".
        assert!(
            serde_json::from_str::<AppControlRequest>(r#"{"request_id":"r1","created_at_ms":0}"#)
                .is_err()
        );
    }

    /// The taker must hand an unknown kind THROUGH to the dispatcher instead of
    /// deleting the file — that is the behaviour change a caller feels.
    #[test]
    fn take_next_hands_an_unknown_kind_to_the_dispatcher() {
        let home = temp_home();
        let worker_pid = std::process::id();
        let dir = app_control_requests_dir(&home);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("00000000-unknown.json"),
            br#"{"request_id":"unknown-1","created_at_ms":0,"command":{"kind":"not_a_real_verb"}}"#,
        )
        .unwrap();

        let taken = take_next_app_control_request(&home, worker_pid).unwrap();
        let Some((inflight_path, request)) = taken else {
            panic!("an unknown kind must be delivered, not deleted");
        };
        assert_eq!(request.request_id, "unknown-1");
        assert_eq!(request.command, AppControlCommand::Unsupported);
        assert!(inflight_path.exists());

        let _ = fs::remove_dir_all(home);
    }

    /// P0's honest-refusal property, extended to the PAYLOAD.
    ///
    /// `#[serde(other)]` rescues an unknown `kind` only. A KNOWN kind whose
    /// fields this build cannot read still failed deserialization, and the
    /// taker DELETED the file — which is the very bare timeout `Unsupported`
    /// was written to kill, reached by the more likely mismatch: every field
    /// added to an existing command changes that command's shape. The worked
    /// case is `do click --text "Proceed to Pay"` against a GUI that types the
    /// selector as a bare `String`: `invalid type: map, expected a string`.
    ///
    /// The payload below is that shape against THIS build — an object where a
    /// string is expected on a kind it knows.
    ///
    /// Restore `Err(_) => { let _ = fs::remove_file(&path); continue; }` in
    /// `take_next_app_control_request` and this fails.
    #[test]
    fn a_known_kind_this_build_cannot_read_is_refused_rather_than_deleted() {
        let home = temp_home();
        let worker_pid = std::process::id();
        let dir = app_control_requests_dir(&home);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("00000000-unreadable.json");
        fs::write(
            &path,
            br#"{"request_id":"unreadable-1","created_at_ms":7,"agent":"agent-2",
                 "command":{"kind":"open_path","session_path":{"text":"Proceed to Pay"}}}"#,
        )
        .unwrap();

        let taken = take_next_app_control_request(&home, worker_pid).unwrap();
        let Some((inflight_path, request)) = taken else {
            panic!("a request this build cannot read must be delivered, not deleted");
        };
        assert_eq!(request.request_id, "unreadable-1");
        assert_eq!(request.agent.as_deref(), Some("agent-2"));
        assert_eq!(request.created_at_ms, 7);
        let AppControlCommand::Unreadable {
            requested_kind,
            detail,
        } = &request.command
        else {
            panic!("expected Unreadable, got {:?}", request.command);
        };
        assert_eq!(requested_kind, "open_path");
        assert!(
            detail.contains("invalid type: map"),
            "the serde error names the field and what it expected: {detail}"
        );
        // It is delivered for a refusal, so it must mutate nothing…
        assert!(request.command.is_read_only());
        assert_eq!(request.command.name(), "unreadable");
        // …and the file is in flight, not gone.
        assert!(inflight_path.exists());
        assert!(!path.exists());

        let _ = fs::remove_dir_all(home);
    }

    /// The other half: a file that is NOT a version mismatch is still deleted.
    /// A salvage that accepted anything would turn corruption into a request.
    #[test]
    fn a_corrupt_request_file_is_still_deleted_unread() {
        let home = temp_home();
        let worker_pid = std::process::id();
        let dir = app_control_requests_dir(&home);
        fs::create_dir_all(&dir).unwrap();
        let truncated = dir.join("00000000-truncated.json");
        let no_command = dir.join("00000001-no-command.json");
        let no_id = dir.join("00000002-no-id.json");
        fs::write(&truncated, b"{ not json").unwrap();
        fs::write(&no_command, br#"{"request_id":"r1","created_at_ms":0}"#).unwrap();
        fs::write(
            &no_id,
            br#"{"created_at_ms":0,"command":{"kind":"open_path","session_path":{}}}"#,
        )
        .unwrap();

        assert!(
            take_next_app_control_request(&home, worker_pid)
                .unwrap()
                .is_none(),
            "none of these is an answerable request"
        );
        assert!(!truncated.exists());
        assert!(!no_command.exists());
        assert!(!no_id.exists());

        let _ = fs::remove_dir_all(home);
    }

    /// The refusal path must not be able to take the window down: the GUI
    /// serializes the command it is handling into the request trace, and
    /// `json!` on a value that fails to serialize panics. That is why
    /// `Unreadable` is an ordinary variant rather than `#[serde(skip)]`.
    #[test]
    fn an_unreadable_command_can_be_serialized_for_the_trace() {
        let command = AppControlCommand::Unreadable {
            requested_kind: "web_surface_do".to_string(),
            detail: "invalid type: map, expected a string".to_string(),
        };
        let value = serde_json::to_value(&command).expect("the trace must not panic on it");
        assert_eq!(value["kind"], serde_json::json!("unreadable"));
        assert_eq!(value["requested_kind"], serde_json::json!("web_surface_do"));
    }

    /// WIRE BACK-COMPAT LOCK for `WebElementRef`.
    ///
    /// Every `do` payload written before text/role addressing existed spells
    /// the target as a BARE STRING. `#[serde(untagged)]` with `Css(String)`
    /// first is what keeps those parsing; a tagged enum would break every
    /// in-flight request file and every scripted caller on the day it landed.
    /// This test fails the moment someone reaches for a tagged representation.
    #[test]
    fn a_bare_string_selector_still_parses_as_css() {
        let old_payload = r##"{"verb":"click_selector","selector":"#login"}"##;
        let action: WebSurfaceDoAction = serde_json::from_str(old_payload).unwrap();
        assert_eq!(
            action,
            WebSurfaceDoAction::ClickSelector {
                selector: WebElementRef::Css("#login".into()),
                button: AppControlPointerButton::Primary,
            }
        );
        // …and the optional-selector fields too, which is the other half of the
        // wire: `type`/`key`/`fill` all carried `Option<String>`.
        let typed: WebSurfaceDoAction =
            serde_json::from_str(r##"{"verb":"type","text":"hi","selector":"#user"}"##).unwrap();
        assert_eq!(
            typed,
            WebSurfaceDoAction::Type {
                text: "hi".into(),
                selector: Some(WebElementRef::Css("#user".into())),
            }
        );
        let filled: WebSurfaceDoAction = serde_json::from_str(
            r##"{"verb":"fill","text":"292244","selectors":["#a","#b","#c"]}"##,
        )
        .unwrap();
        assert_eq!(
            filled,
            WebSurfaceDoAction::Fill {
                text: "292244".into(),
                selector: None,
                selectors: vec![
                    WebElementRef::Css("#a".into()),
                    WebElementRef::Css("#b".into()),
                    WebElementRef::Css("#c".into()),
                ],
                // The fidelity fields are ADDITIVE: a payload written before
                // they existed still parses, and still means what it meant.
                mechanism: WebFillMechanism::Auto,
                redact: false,
            }
        );
    }

    /// The fidelity fields must round-trip when they ARE spelled, or the
    /// vault path's "never the native setter, never echo the value" contract
    /// would be silently downgraded to the defaults on the wire.
    #[test]
    fn fill_mechanism_and_redaction_round_trip() {
        let secret: WebSurfaceDoAction = serde_json::from_str(
            r##"{"verb":"fill","text":"s","selector":"#pw","mechanism":"real_keys","redact":true}"##,
        )
        .unwrap();
        assert_eq!(
            secret,
            WebSurfaceDoAction::Fill {
                text: "s".into(),
                selector: Some(WebElementRef::Css("#pw".into())),
                selectors: Vec::new(),
                mechanism: WebFillMechanism::RealKeys,
                redact: true,
            }
        );
        let back = serde_json::to_string(&secret).unwrap();
        assert!(back.contains(r#""mechanism":"real_keys""#), "{back}");
        assert!(back.contains(r#""redact":true"#), "{back}");
        let native: WebSurfaceDoAction =
            serde_json::from_str(r##"{"verb":"fill","text":"s","mechanism":"native_setter"}"##)
                .unwrap();
        assert!(matches!(
            native,
            WebSurfaceDoAction::Fill {
                mechanism: WebFillMechanism::NativeSetter,
                redact: false,
                ..
            }
        ));
    }

    /// The new addressing shapes must be distinguishable from each other and
    /// from a bare selector — an untagged enum resolves by SHAPE, so this is
    /// the test that catches a field rename silently re-routing a payload.
    #[test]
    fn text_and_role_refs_parse_into_their_own_shapes() {
        let by_text: WebSurfaceDoAction = serde_json::from_str(
            r#"{"verb":"click_selector","selector":{"text":"Proceed to Pay","exact":true}}"#,
        )
        .unwrap();
        assert_eq!(
            by_text,
            WebSurfaceDoAction::ClickSelector {
                selector: WebElementRef::Text {
                    text: "Proceed to Pay".into(),
                    exact: true,
                    tag: None,
                    nth: None,
                },
                button: AppControlPointerButton::Primary,
            }
        );
        let by_role: WebSurfaceDoAction = serde_json::from_str(
            r#"{"verb":"click_selector","selector":{"role":"button","label":"Continue","nth":1}}"#,
        )
        .unwrap();
        assert_eq!(
            by_role,
            WebSurfaceDoAction::ClickSelector {
                selector: WebElementRef::Role {
                    role: "button".into(),
                    label: "Continue".into(),
                    nth: Some(1),
                },
                button: AppControlPointerButton::Primary,
            }
        );
        // Round-trips: a re-serialized Css ref is still a bare string, so an
        // old GUI reading a request written by a new CLI still understands the
        // ordinary case.
        assert_eq!(
            serde_json::to_value(WebElementRef::Css("#x".into())).unwrap(),
            serde_json::json!("#x")
        );
    }

    #[test]
    fn read_only_commands_are_pure_observation() {
        // Pure-observation: no UI/session mutation → no forced re-render needed.
        assert!(AppControlCommand::DescribeState.is_read_only());
        assert!(AppControlCommand::DescribeRows.is_read_only());
        assert!(
            AppControlCommand::ReadTerminalBuffer {
                session_path: "x".into(),
                mode: "screen".into(),
            }
            .is_read_only()
        );
        // Mutating commands must NOT be classified read-only (they change UI/session
        // state and legitimately need a re-render).
        assert!(!AppControlCommand::ShowStartPage.is_read_only());
        assert!(
            !AppControlCommand::SendTerminalInput {
                session_path: "x".into(),
                data: "y".into(),
            }
            .is_read_only()
        );
        assert!(
            !AppControlCommand::ReconcileTerminalFromDaemon {
                session_path: "x".into(),
            }
            .is_read_only(),
            "reconcile repaints the client → must re-render"
        );
        assert!(
            !AppControlCommand::ScrollTerminalViewport {
                session_path: "x".into(),
                to: "bottom".into(),
            }
            .is_read_only(),
            "scroll moves the viewport → must re-render"
        );
    }

    fn temp_home() -> PathBuf {
        let home = std::env::temp_dir().join(format!("yggterm-app-control-{}", Uuid::new_v4()));
        fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn preserving_close_command_serializes_as_distinct_restart_safe_kind() {
        let command = AppControlCommand::CloseWindowPreservingSessions {
            reason: Some("superseded-client-handoff".to_string()),
            force: false,
        };
        let value = serde_json::to_value(&command).expect("serialize preserving close");

        assert_eq!(
            value.get("kind").and_then(serde_json::Value::as_str),
            Some("close_window_preserving_sessions")
        );
        assert_eq!(
            value.get("reason").and_then(serde_json::Value::as_str),
            Some("superseded-client-handoff")
        );
        assert_eq!(command.name(), "close_window_preserving_sessions");
        // `force` defaults false and is omitted from the wire, so an OLDER GUI
        // reading a request from a newer CLI still means "do not force".
        assert!(value.get("force").is_none(), "force must not be serialized when false");
        let back: AppControlCommand = serde_json::from_value(value).unwrap();
        assert_eq!(back, command);
    }

    #[test]
    fn web_surface_do_serializes_with_nested_verb_tag() {
        let command = AppControlCommand::WebSurfaceDo {
            session_path: Some("local://abc".to_string()),
            action: WebSurfaceDoAction::ClickSelector {
                selector: WebElementRef::Css("button[type=submit]".to_string()),
                button: AppControlPointerButton::Primary,
            },
            generation: None,
            new_batch: false,
        };
        let value = serde_json::to_value(&command).expect("serialize web_surface_do");
        assert_eq!(
            value.get("kind").and_then(serde_json::Value::as_str),
            Some("web_surface_do")
        );
        assert_eq!(
            value
                .get("action")
                .and_then(|a| a.get("verb"))
                .and_then(serde_json::Value::as_str),
            Some("click_selector")
        );
        assert_eq!(
            value
                .get("action")
                .and_then(|a| a.get("selector"))
                .and_then(serde_json::Value::as_str),
            Some("button[type=submit]")
        );
        assert_eq!(command.name(), "web_surface_do");
        // A mutating action → must NOT be read-only (it needs a re-render gate).
        assert!(!command.is_read_only());
    }

    #[test]
    fn web_surface_do_click_and_key_round_trip() {
        for action in [
            WebSurfaceDoAction::Click {
                x: 12.0,
                y: 34.0,
                button: AppControlPointerButton::Secondary,
            },
            WebSurfaceDoAction::Key {
                key: "Enter".to_string(),
                mods: vec!["ctrl".to_string()],
                selector: None,
            },
            WebSurfaceDoAction::Scroll {
                x: None,
                y: None,
                dx: 0.0,
                dy: 120.0,
            },
            // `fill` in both shapes: one field, and a segmented box set.
            WebSurfaceDoAction::Fill {
                text: "292244".to_string(),
                selector: Some(WebElementRef::Css("#otp".to_string())),
                selectors: Vec::new(),
                mechanism: WebFillMechanism::Auto,
                redact: false,
            },
            WebSurfaceDoAction::Fill {
                text: "292244".to_string(),
                selector: None,
                selectors: (0..6)
                    .map(|i| WebElementRef::Css(format!("input.input-otp:nth-child({i})")))
                    .collect(),
                mechanism: WebFillMechanism::RealKeys,
                redact: true,
            },
        ] {
            let command = AppControlCommand::WebSurfaceDo {
                session_path: None,
                action: action.clone(),
                generation: None,
                new_batch: false,
            };
            let json = serde_json::to_string(&command).expect("serialize");
            let back: AppControlCommand = serde_json::from_str(&json).expect("deserialize");
            match back {
                AppControlCommand::WebSurfaceDo {
                    session_path,
                    action: round,
                    generation,
                    new_batch,
                } => {
                    assert_eq!(session_path, None);
                    assert_eq!(round, action);
                    assert_eq!(generation, None);
                    assert!(!new_batch, "the reset is opt-in, never the default");
                }
                other => panic!("round-tripped into the wrong variant: {other:?}"),
            }
        }
    }

    #[test]
    fn web_surface_read_is_read_only_and_serializes_as_mode() {
        let command = AppControlCommand::WebSurfaceRead {
            session_path: None,
            mode: WebSurfaceReadAs::Snapshot,
            frame: None,
        };
        // Pure observation → read-only (skips the forced re-render).
        assert!(command.is_read_only());
        assert_eq!(command.name(), "web_surface_read");
        // The wire field is `as` (not the Rust keyword-avoiding `mode`).
        let value = serde_json::to_value(&command).expect("serialize");
        assert_eq!(
            value.get("as").and_then(serde_json::Value::as_str),
            Some("snapshot")
        );
        // Round-trip each mode; `as` omitted defaults to snapshot.
        for (json, expect) in [
            (r#"{"kind":"web_surface_read","as":"forms"}"#, WebSurfaceReadAs::Forms),
            (r#"{"kind":"web_surface_read","as":"links"}"#, WebSurfaceReadAs::Links),
            (r#"{"kind":"web_surface_read"}"#, WebSurfaceReadAs::Snapshot),
        ] {
            match serde_json::from_str::<AppControlCommand>(json).expect("deserialize") {
                AppControlCommand::WebSurfaceRead { mode, frame, .. } => {
                    assert_eq!(mode, expect);
                    // No `frame` on the wire = the top document, which is what
                    // every previously-written request means.
                    assert_eq!(frame, None);
                }
                other => panic!("wrong variant: {other:?}"),
            }
        }
        // The three frame spellings survive the wire, and `frame` is omitted
        // when absent so an OLDER GUI reading a new request is unaffected.
        assert!(!serde_json::to_string(&command).unwrap().contains("frame"));
        for (json, expect) in [
            (
                r#"{"kind":"web_surface_read","frame":{"index":2}}"#,
                WebFrameRef::Index(2),
            ),
            (
                r#"{"kind":"web_surface_read","frame":{"path":[0,2]}}"#,
                WebFrameRef::Path(vec![0, 2]),
            ),
            (
                r#"{"kind":"web_surface_read","frame":{"url_contains":"billdesk"}}"#,
                WebFrameRef::UrlContains("billdesk".into()),
            ),
        ] {
            match serde_json::from_str::<AppControlCommand>(json).expect("deserialize frame") {
                AppControlCommand::WebSurfaceRead { frame, .. } => {
                    assert_eq!(frame, Some(expect));
                }
                other => panic!("wrong variant: {other:?}"),
            }
        }
    }

    #[test]
    fn web_surface_wait_serializes_nested_until_and_is_read_only() {
        let command = AppControlCommand::WebSurfaceWait {
            session_path: None,
            until: WebSurfaceWaitUntil::Selector {
                css: "#ready".to_string(),
                visible: true,
            },
            timeout_ms: 5000,
        };
        // Polling a condition mutates nothing → read-only.
        assert!(command.is_read_only());
        assert_eq!(command.name(), "web_surface_wait");
        let value = serde_json::to_value(&command).expect("serialize");
        assert_eq!(
            value.get("until").and_then(|u| u.get("until")).and_then(serde_json::Value::as_str),
            Some("selector")
        );
        // Round-trip a few condition kinds.
        for until in [
            WebSurfaceWaitUntil::LoadFinished,
            WebSurfaceWaitUntil::Idle { ms: 500 },
            WebSurfaceWaitUntil::Js { expr: "window.ready".to_string() },
        ] {
            let cmd = AppControlCommand::WebSurfaceWait {
                session_path: None,
                until: until.clone(),
                timeout_ms: 1000,
            };
            let json = serde_json::to_string(&cmd).unwrap();
            match serde_json::from_str::<AppControlCommand>(&json).unwrap() {
                AppControlCommand::WebSurfaceWait { until: back, .. } => assert_eq!(back, until),
                other => panic!("wrong variant: {other:?}"),
            }
        }
    }

    #[test]
    fn targeted_request_is_not_taken_by_wrong_worker_pid() {
        let home = temp_home();
        let request =
            enqueue_app_control_request(&home, AppControlCommand::DescribeState, Some(0)).unwrap();

        let taken = take_next_app_control_request(&home, std::process::id()).unwrap();
        assert!(taken.is_none());
        assert!(
            !app_control_requests_dir(&home)
                .join(format!("{}.json", request.request_id))
                .exists()
        );

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn render_needed_only_for_mutating_pending_requests() {
        // A read-only probe is pending-but-render-NOT-needed (processed via the waker,
        // no forced shell re-render — the churn cut). A mutating command IS render-needed.
        let home = temp_home();
        let pid = std::process::id();
        enqueue_app_control_request(&home, AppControlCommand::DescribeState, Some(pid)).unwrap();
        assert!(app_control_requests_pending_for_worker(&home, pid));
        assert!(
            !app_control_pending_render_needed_for_worker(&home, pid),
            "a read-only probe must NOT force a shell re-render"
        );
        enqueue_app_control_request(&home, AppControlCommand::ShowStartPage, Some(pid)).unwrap();
        assert!(
            app_control_pending_render_needed_for_worker(&home, pid),
            "a mutating command must force a re-render"
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn pending_for_worker_ignores_request_targeted_to_other_live_pid() {
        let home = temp_home();
        let target_pid = std::process::id();
        let worker_pid = target_pid.saturating_add(1);
        let request =
            enqueue_app_control_request(&home, AppControlCommand::DescribeState, Some(target_pid))
                .unwrap();

        assert!(app_control_requests_pending(&home));
        assert!(!app_control_requests_pending_for_worker(&home, worker_pid));
        assert!(
            app_control_requests_dir(&home)
                .join(format!("{}.json", request.request_id))
                .exists()
        );
        assert!(app_control_requests_pending_for_worker(&home, target_pid));

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn pending_for_worker_removes_stale_targeted_request() {
        let home = temp_home();
        let request =
            enqueue_app_control_request(&home, AppControlCommand::DescribeState, Some(0)).unwrap();

        assert!(!app_control_requests_pending_for_worker(
            &home,
            std::process::id()
        ));
        assert!(
            !app_control_requests_dir(&home)
                .join(format!("{}.json", request.request_id))
                .exists()
        );

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn targeted_request_is_taken_by_preferred_worker_pid() {
        let home = temp_home();
        let worker_pid = std::process::id();
        let request =
            enqueue_app_control_request(&home, AppControlCommand::DescribeState, Some(worker_pid))
                .unwrap();

        let taken = take_next_app_control_request(&home, worker_pid).unwrap();
        let Some((inflight_path, taken_request)) = taken else {
            panic!("expected preferred worker to take request");
        };
        assert_eq!(taken_request.request_id, request.request_id);
        assert_eq!(taken_request.preferred_pid, Some(worker_pid));
        assert!(inflight_path.exists());

        let _ = fs::remove_dir_all(home);
    }

    // ── Teardown honesty: agent rows name themselves, removal verifies ──────

    /// Every session kind this build knows, so a naming rule cannot be written
    /// for shells and quietly skip the rest.
    const EVERY_SESSION_KIND: [SessionKind; 6] = [
        SessionKind::Shell,
        SessionKind::SshShell,
        SessionKind::Codex,
        SessionKind::CodexLiteLlm,
        SessionKind::ClaudeCode,
        SessionKind::Document,
    ];

    /// The naming contract: an agent-plane row says WHO made it and WHAT FOR,
    /// and it does so for every kind, named driver or not.
    #[test]
    fn an_agent_plane_title_names_its_driver_and_purpose() {
        for kind in EVERY_SESSION_KIND {
            let named = agent_plane_session_title(
                Some("probe-7"),
                Some("reap leftover app processes"),
                kind,
            );
            assert!(
                named.starts_with(AGENT_PLANE_TITLE_PREFIX),
                "an agent row must be findable by a title probe that knows only the plane: {named}"
            );
            assert!(
                named.contains("probe-7"),
                "the driver's identity must survive into the title: {named}"
            );
            assert!(
                named.contains("reap leftover app processes"),
                "the purpose must survive into the title: {named}"
            );
            assert!(
                named.contains(crate::session_kind_label(kind)),
                "the title must say what kind of session it is: {named}"
            );

            // No `--agent` is still an agent. The request field's contract is
            // "absent means SOME agent", and the row must say so rather than
            // fall back to a name a human's session could also have.
            let anonymous = agent_plane_session_title(None, None, kind);
            assert!(
                anonymous.starts_with(AGENT_PLANE_TITLE_PREFIX),
                "an unnamed driver still gets an agent-plane title: {anonymous}"
            );
            assert_ne!(anonymous, named);
        }
    }

    /// The title must SURVIVE the copy layer, which discards titles it judges
    /// generated junk and falls back to the humanized cwd leaf — the very
    /// label this whole change exists to stop the row wearing. Hostile
    /// purposes included, because the purpose is caller text.
    #[test]
    fn an_agent_plane_title_is_never_thrown_away_as_generated_junk() {
        let purposes = [
            None,
            Some(""),
            Some("   "),
            // Ends on a syntax fragment — the copy layer discards these.
            Some("ship the logs to"),
            // Almost entirely noise words.
            Some("the and for with the"),
            // Question fragment.
            Some("why the build is slow"),
            Some("verify\u{7}the\u{1b}teardown"),
            Some(
                "an extremely long purpose that goes on and on well past anything a sidebar row \
                 could ever show a human being reading it",
            ),
        ];
        for kind in EVERY_SESSION_KIND {
            for agent in [None, Some(""), Some("probe-7"), Some("session")] {
                for purpose in purposes {
                    let title = agent_plane_session_title(agent, purpose, kind);
                    assert!(
                        !yggterm_core::looks_like_generated_fallback_title(&title),
                        "the copy layer would discard {title:?} \
                         (agent {agent:?}, purpose {purpose:?}) and rename the row \
                         after its cwd, which is the bug"
                    );
                    assert!(
                        title.starts_with(AGENT_PLANE_TITLE_PREFIX),
                        "title {title:?} lost the agent-plane prefix"
                    );
                }
            }
        }
    }

    /// Cross-version: `purpose` is new, so a request written by a build that
    /// never heard of it must still parse, and a request that carries it must
    /// survive a round trip rather than being silently dropped.
    #[test]
    fn a_create_terminal_request_without_a_purpose_still_parses() {
        let without: AppControlCommand = serde_json::from_str(
            r#"{"kind":"create_terminal","cwd":"/tmp","session_kind":"shell"}"#,
        )
        .expect("a create_terminal from an older build must parse");
        assert_eq!(
            without,
            AppControlCommand::CreateTerminal {
                machine_key: None,
                cwd: Some("/tmp".to_string()),
                title_hint: None,
                purpose: None,
                session_kind: Some(SessionKind::Shell),
                activate: None,
            }
        );

        let with = AppControlCommand::CreateTerminal {
            machine_key: None,
            cwd: Some("/tmp".to_string()),
            title_hint: None,
            purpose: Some("reap leftovers".to_string()),
            session_kind: Some(SessionKind::Shell),
            activate: Some(false),
        };
        let round_tripped: AppControlCommand =
            serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
        assert_eq!(round_tripped, with);
    }

    fn teardown_process(pid: i32, command: &str) -> SessionTeardownProcess {
        SessionTeardownProcess {
            pid,
            command: command.to_string(),
        }
    }

    /// Every way a teardown can fail must produce `verified: false` with a
    /// NAMED refusal — the whole point being that an agent cannot report a
    /// clean exit it did not get.
    #[test]
    fn a_removal_is_verified_only_when_the_row_and_its_processes_are_gone() {
        let census = [teardown_process(11, "bash"), teardown_process(12, "an-app")];

        let clean = verify_session_removal(&SessionRemovalEvidence {
            row_was_live: true,
            runtime_pid_before: Some(11),
            observed_before: &census,
            still_running_after: &[],
            row_still_listed: false,
        });
        assert!(clean.verified);
        assert_eq!(clean.refusal, None);
        assert_eq!(clean.reaped, census.to_vec());
        assert!(clean.still_running.is_empty());

        // The reported incident: the row survived the "removal".
        let row_alive = verify_session_removal(&SessionRemovalEvidence {
            row_was_live: true,
            runtime_pid_before: Some(11),
            observed_before: &census,
            still_running_after: &[],
            row_still_listed: true,
        });
        assert!(!row_alive.verified);
        assert_eq!(
            row_alive.refusal,
            Some(SessionRemovalRefusal::RowStillListed)
        );

        // The other half of the incident: the app under the shell outlived it.
        let survivors = [teardown_process(12, "an-app")];
        let app_alive = verify_session_removal(&SessionRemovalEvidence {
            row_was_live: true,
            runtime_pid_before: Some(11),
            observed_before: &census,
            still_running_after: &survivors,
            row_still_listed: false,
        });
        assert!(!app_alive.verified);
        assert_eq!(
            app_alive.refusal,
            Some(SessionRemovalRefusal::ProcessesSurvived)
        );
        assert_eq!(app_alive.still_running, survivors.to_vec());
        assert_eq!(app_alive.reaped, vec![teardown_process(11, "bash")]);

        // A live row whose runtime nobody local can see (an older daemon owns
        // it) is UNVERIFIABLE, never clean. This is the cross-version case
        // that has already failed quietly once.
        let unobservable = verify_session_removal(&SessionRemovalEvidence {
            row_was_live: true,
            runtime_pid_before: None,
            observed_before: &[],
            still_running_after: &[],
            row_still_listed: false,
        });
        assert!(!unobservable.verified);
        assert_eq!(
            unobservable.refusal,
            Some(SessionRemovalRefusal::RuntimePidUnobservable)
        );

        // A row that was never live has no runtime to verify, so the row check
        // alone decides it — otherwise every stored-row removal reports a
        // refusal it cannot act on.
        let stored = verify_session_removal(&SessionRemovalEvidence {
            row_was_live: false,
            runtime_pid_before: None,
            observed_before: &[],
            still_running_after: &[],
            row_still_listed: false,
        });
        assert!(stored.verified);
        assert_eq!(stored.refusal, None);
    }

    /// Every refusal must carry a distinct machine-readable name: the caller
    /// branches on it, and two refusals sharing a name is a silent merge.
    #[test]
    fn every_removal_refusal_has_its_own_name() {
        let refusals = [
            SessionRemovalRefusal::RowStillListed,
            SessionRemovalRefusal::ProcessesSurvived,
            SessionRemovalRefusal::RuntimePidUnobservable,
        ];
        let mut names = refusals
            .iter()
            .map(|refusal| refusal.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "two refusals share a name: {names:?}");
        assert!(names.iter().all(|name| !name.is_empty()));
    }
}
