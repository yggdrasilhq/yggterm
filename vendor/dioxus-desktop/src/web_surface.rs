//! yggterm web surfaces: native child webviews layered over the main webview's
//! page area. Each surface is its own wry `WebView` with its own `WebContext`
//! (so it can carry an independent SOCKS proxy — the egress rule) added to the
//! main window's `gtk::Overlay` as an overlay child.
//!
//! This is the Linux/WebKitGTK path. `build_as_child` is unavailable here (the
//! Linux dioxus-desktop build compiles wry WITHOUT the `x11` feature, and jojo
//! is native Wayland), so surfaces attach via `build_gtk` into a `gtk::Fixed`
//! overlay child, positioned with margins + size-request. A per-surface Fixed
//! (rather than one shared full-page Fixed) means each surface only occupies —
//! and only captures input within — its own rect; everywhere else the overlay
//! falls through to the main webview, keeping the chrome interactive.
#![cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
)))]

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use wry::{
    dpi::{LogicalPosition, LogicalSize, Position, Size},
    http::{Request, Response},
    ProxyConfig, ProxyEndpoint, Rect, RequestAsyncResponder, WebContext, WebViewBuilder,
};

/// The custom URI scheme an app's in-page shim uses to reach its own control
/// endpoint from inside a surface, bypassing WebKit's https→http mixed-content
/// block. The GUI registers it as SECURE and proxies it to the app's
/// GUI-reachable control endpoint. See `app_control_proxy`.
const APP_CONTROL_SCHEME: &str = "yggterm-appctl";

/// The script-message channel every surface page can reach its host on
/// (`window.webkit.messageHandlers.yggtermSurface`). Today it carries exactly
/// one message: `"close"`.
const SURFACE_MESSAGE_HANDLER: &str = "yggtermSurface";

/// `window.close()`, reported to the host.
///
/// WebKitGTK does not emit its `close` signal for a `window.close()` call — not
/// even for a window a script opened, which is the one case every browser
/// honors. (Proven on the harness: `load-changed` fires on the very same
/// webview object while `close` never does, so this is the engine's refusal, not
/// a missed connection.) A browser that cannot close a popup strands every
/// OAuth sign-in ever written: the callback page hands the token back to its
/// opener and closes itself, and the window just sits there.
///
/// So the page tells us directly, and the HOST decides — which is also where the
/// decision belongs. The shim only reports; the shell honors a close request
/// only for a tab that a script actually opened (Chrome's rule: a page may close
/// a window it opened, and nothing else). The native `close` signal is still
/// connected alongside this, so if the engine ever starts emitting it, the same
/// door is already open.
///
/// The native `close()` is deliberately NOT called. WebKitGTK's own
/// `window.close()` tears the page down (the view goes blank) while telling the
/// embedder nothing — so a refusal that still called it would leave the user
/// staring at a white rectangle where their tab used to be. The request goes to
/// the host and nowhere else: if the host agrees, it destroys the webview; if it
/// refuses, the page is untouched, which is what "refused" has to mean.
///
/// The message carries WHO is asking, because the channel cannot say. A popup is
/// built related to its opener, and WebKit gives a related view its opener's
/// user-content manager — so the popup's message arrives on the OPENER's handler
/// (proven on the harness: the popup was surface 2, its close arrived as surface
/// 1). The page therefore states its own URL and whether a script opened it, and
/// the shell resolves which tab that is.
const CLOSE_SHIM_JS: &str = r#"(function(){
  if (window.__yggtermCloseShim) { return; }
  window.__yggtermCloseShim = true;
  window.close = function() {
    try {
      window.webkit.messageHandlers.yggtermSurface.postMessage(JSON.stringify({
        type: 'close',
        href: String(location.href),
        scriptOpened: !!window.opener,
      }));
    } catch (e) {}
  };
})();"#;

/// A page asking to be closed. Which page is `href` + `script_opened`, said by
/// the page itself — the channel cannot say (see `CLOSE_SHIM_JS`). `surface_id`
/// is the surface whose channel it arrived on: the sender, or the sender's
/// opener. The shell resolves the tab and decides.
pub struct SurfaceCloseRequest {
    /// The surface whose message channel carried this — the sender, or (for a
    /// popup, which shares its opener's channel) the sender's opener.
    pub surface_id: u64,
    /// The page's own URL, as it reported it.
    pub href: String,
    /// The page says a script opened it (`window.opener` is live). A page that
    /// says otherwise is asking to close a window the USER opened, which no
    /// browser honors.
    pub script_opened: bool,
}

/// Wire a surface's page->host channel: the `window.close()` shim plus the
/// script-message handler it speaks to. Every surface gets it — a popup because
/// it is the whole point, a normal tab because the shell must be able to tell
/// the two apart and refuse the one it should refuse.
fn attach_surface_message_channel(
    webview: &wry::WebView,
    surface_id: u64,
    close_requests: &Rc<RefCell<Vec<SurfaceCloseRequest>>>,
) {
    use webkit2gtk::{UserContentManagerExt as _, WebViewExt as _};
    use wry::WebViewExtUnix as _;
    let webkit = webview.webview();
    let Some(manager) = webkit.user_content_manager() else {
        return;
    };
    // Connect BEFORE registering — WebKit's own documented order, and the order
    // wry's ipc channel uses. Registering first can drop the first message.
    let close_requests = close_requests.clone();
    manager.connect_script_message_received(Some(SURFACE_MESSAGE_HANDLER), move |_, result| {
        let Some(value) = result.js_value() else {
            return;
        };
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&value.to_string()) else {
            return;
        };
        if message.get("type").and_then(|kind| kind.as_str()) != Some("close") {
            return;
        }
        close_requests.borrow_mut().push(SurfaceCloseRequest {
            surface_id,
            href: message
                .get("href")
                .and_then(|href| href.as_str())
                .unwrap_or_default()
                .to_string(),
            script_opened: message
                .get("scriptOpened")
                .and_then(|flag| flag.as_bool())
                .unwrap_or(false),
        });
    });
    manager.register_script_message_handler(SURFACE_MESSAGE_HANDLER);
}

struct Surface {
    // The overlay child that positions the webview. wry `build_gtk`s the webview
    // into this Fixed (put at 0,0); the Fixed is placed in the overlay via
    // margin-start/top + size-request.
    container: gtk::Fixed,
    webview: wry::WebView,
    // wry requires the WebContext to outlive the webview; co-own it here. SHARED
    // with every sibling surface on the same jar+egress+control endpoint (see
    // `web_context_key`), which is what makes two tabs of one session one web
    // process pool and — crucially — ONE cookie jar. A POPUP has none of its
    // own: it is built RELATED to its opener, which means it shares the opener's
    // context (its jar, its proxy, its web process) — that sharing is exactly
    // what a popup is.
    _ctx: Option<Rc<RefCell<WebContext>>>,
    // TRUE while THIS HOST is holding the inner webview hidden so the ENGINE
    // believes the page is off screen. That is the whole Page Visibility
    // contract: an unmapped WebKitGTK view reports
    // `document.visibilityState === 'hidden'`, `requestAnimationFrame` stops and
    // timers throttle. It is the only thing that makes a background page cheap,
    // and a surface nobody has revealed must have it.
    //
    // The host's SINGLE record of "we hid this": a hidden `open`, `set_visible`
    // and `set_throttled` write it; `set_visible(true)`, `set_throttled(false)`
    // and `unstash` are the only clearers. The one consumer that must not guess
    // is `engine_webview_for_injection` — an unmapped view silently drops
    // synthesized events, so the agent drive path wakes a view WE hid (and only
    // one WE hid) for the length of the burst.
    //
    // A DETACHED (hard-stash) surface is unmapped too and this stays false: its
    // container has no parent, so showing the webview would not map it, and
    // injection must keep failing closed there exactly as it did before.
    engine_hidden: Cell<bool>,
    // Re-arm token for the injection wake's re-hide, the same shape as
    // `FOCUS_GIVEBACK_TOKEN`: every injected event bumps it and schedules a
    // re-hide, and a re-hide holding a stale token belongs to an earlier event
    // in the burst and does nothing. Per surface, so two agents driving two
    // surfaces cannot re-hide each other's.
    wake_token: Cell<u64>,
}

/// What an injection has to do to a surface's webview before it may deliver an
/// event to it. See [`injection_map_plan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionMapPlan {
    /// Mapped, and nobody is holding it hidden: deliver, nothing to give back.
    Deliver,
    /// Mapped, but WE are still holding it hidden — an earlier event in this
    /// burst woke it. Deliver, and RE-ARM the re-hide so the loan ends when the
    /// burst does instead of part-way through it.
    DeliverAndRehide,
    /// Hidden by us and unmapped: show it, deliver, arm the re-hide.
    WakeAndRehide,
    /// Unmapped and NOT ours to wake — a detached (hard-stashed) container,
    /// whose child cannot be mapped by showing it. Refuse: an event delivered
    /// into an unmapped view is silently dropped, and a lie of success is worse
    /// than a refusal.
    Refuse,
}

/// Decide the above from the two ENGINE readings, taken for the surface about
/// to be driven: are we holding it hidden, and is its webview mapped right now.
///
/// The cell that carries the whole design is `(engine_hidden: false, mapped:
/// false) => Refuse`. Page-visibility throttling unmaps every surface nobody is
/// being shown, and those are exactly the surfaces agents drive — so injection
/// may no longer treat "unmapped" as "unreachable". But it may only wake what
/// IT hid: a detached container has no parent to map into, and a surface hidden
/// by something else is not ours to reveal.
fn injection_map_plan(engine_hidden: bool, mapped: bool) -> InjectionMapPlan {
    match (engine_hidden, mapped) {
        (false, true) => InjectionMapPlan::Deliver,
        (true, true) => InjectionMapPlan::DeliverAndRehide,
        (true, false) => InjectionMapPlan::WakeAndRehide,
        (false, false) => InjectionMapPlan::Refuse,
    }
}

/// How long after the last injected event a surface woken for injection goes
/// back to engine-hidden. Long enough to cover a click's press+release and the
/// next event of a batch, short enough that a page nobody revealed is never
/// left painting. Same order as [`FOCUS_GIVEBACK_DELAY_MS`], and for the same
/// reason: the loan ends when the burst does.
const ENGINE_REHIDE_DELAY_MS: u64 = 400;

/// Arm (or re-arm) the re-hide of a surface woken for injection.
///
/// Copied deliberately from [`schedule_focus_giveback`], including its refusal
/// to take back something that is no longer ours: if the reconciler REVEALED
/// the surface while the burst was in flight then `engine_hidden` is already
/// clear and this does nothing, because re-hiding a revealed page would blank
/// the view the user is looking at.
fn schedule_engine_rehide(surfaces: &Rc<RefCell<HashMap<u64, Surface>>>, id: u64) {
    let token = {
        let map = surfaces.borrow();
        let Some(surface) = map.get(&id) else {
            return;
        };
        let next = surface.wake_token.get().wrapping_add(1);
        surface.wake_token.set(next);
        next
    };
    let surfaces = surfaces.clone();
    gtk::glib::timeout_add_local_once(
        std::time::Duration::from_millis(ENGINE_REHIDE_DELAY_MS),
        move || {
            let map = surfaces.borrow();
            let Some(surface) = map.get(&id) else {
                return; // closed since
            };
            if surface.wake_token.get() != token {
                return; // a later injection re-armed this; that one re-hides
            }
            if !surface.engine_hidden.get() {
                return; // revealed since — those pixels are the user's now
            }
            let _ = surface.webview.set_visible(false);
        },
    );
}

/// Which surfaces may share one `WebKitWebContext`, as a deterministic key.
/// `None` means "never share".
///
/// A `WebContext` is not just a cookie jar. It is a process pool, a
/// `WebsiteDataManager`, a network-proxy setting, and a custom-scheme registry,
/// and every one of those is per-context. So the sharing unit is the
/// intersection of all four:
///
/// - **profile dir** — the on-disk jar. Different jars must never mix.
/// - **socks port** — wry sets the proxy on the context's `WebsiteDataManager`
///   at build time (`webkitgtk/mod.rs`), so two surfaces egressing through
///   different tunnels cannot share one. A remote session's tabs share ONE
///   `ssh -N -D` tunnel (`adopt_web_surface_session_socks`), so they agree here;
///   a local session has no proxy and agrees trivially. Sessions on different
///   hosts have different tunnels and correctly do not share.
/// - **signer base** — the `yggterm-appctl://` scheme is registered on the
///   CONTEXT and proxies to exactly one session's control endpoint. Two sessions
///   sharing a profile must not share a context, or one session's page would
///   reach the other's endpoint. wry also refuses a duplicate registration
///   outright (`DuplicateCustomProtocol`), so this is a correctness bound, not a
///   preference.
///
/// Ephemeral surfaces (`profile_dir == None`) return `None` and each get their
/// own context: an ephemeral jar that two surfaces shared would not be
/// ephemeral in the sense the caller asked for.
///
/// The key is built with unit separators around components that can each be
/// arbitrary text, so no two distinct inputs can collide by concatenation.
fn web_context_key(
    profile_dir: Option<&std::path::Path>,
    socks_port: Option<u16>,
    signer_base: Option<&str>,
) -> Option<String> {
    let dir = profile_dir?;
    Some(format!(
        "{}\u{1f}{}\u{1f}{}",
        dir.display(),
        socks_port.map(|p| p.to_string()).unwrap_or_default(),
        signer_base.unwrap_or_default()
    ))
}

/// Engine-native ad/tracker blocking (AdGuard-class network + cosmetic rules)
/// via WebKit's declarative content filters — the mechanism GNOME Web uses.
/// The webkit2gtk 2.0.2 SAFE binding does not bind UserContentFilterStore /
/// add_filter (only the error enum), so this goes through `webkit2gtk::ffi`
/// directly. One ruleset per GUI process, compiled once (async, on the GTK
/// main loop) into a bytecode store dir and attached to every surface opened
/// with adblock on; surfaces that open while compilation is in flight get the
/// filter attached from the completion callback (page loads are slower than
/// the compile, so the first navigation is still covered in practice).
mod adblock {
    use gtk::glib::translate::ToGlibPtr as _;
    use std::cell::RefCell;
    use webkit2gtk::ffi as wk;

    thread_local! {
        // (compiled filter, compile started). GTK-main-thread only, like every
        // other surface path in this module.
        static STATE: RefCell<(Option<*mut wk::WebKitUserContentFilter>, bool)> =
            const { RefCell::new((None, false)) };
        // Webviews that opened with adblock on before compilation finished;
        // drained by the compile-completion callback. Holding the engine
        // WebView (a GObject clone) keeps this independent of surface
        // lifetime bookkeeping — attaching to an already-destroyed webview is
        // a harmless no-op on a still-live GObject.
        static PENDING: RefCell<Vec<webkit2gtk::WebView>> = const { RefCell::new(Vec::new()) };
    }

    fn attach_to(webkit: &webkit2gtk::WebView, filter: *mut wk::WebKitUserContentFilter) {
        use webkit2gtk::WebViewExt as _;
        if let Some(manager) = webkit.user_content_manager() {
            unsafe {
                wk::webkit_user_content_manager_add_filter(manager.to_glib_none().0, filter);
            }
        }
    }

    /// Attach the compiled filter to a surface webview now, or queue it for
    /// attachment when compilation finishes. Returns whether it attached now.
    pub(super) fn attach(webview: &wry::WebView) -> bool {
        use wry::WebViewExtUnix as _;
        let webkit = webview.webview();
        let filter = STATE.with(|s| s.borrow().0);
        match filter {
            Some(filter) => {
                attach_to(&webkit, filter);
                true
            }
            None => {
                PENDING.with(|p| p.borrow_mut().push(webkit));
                false
            }
        }
    }

    /// Kick off (once per process) async compilation of the content-blocker
    /// JSON at `ruleset` into `store_dir`. Completion caches the filter and
    /// drains the pending-webview queue. No-op if compilation already started.
    pub(super) fn ensure_compiled(ruleset: &std::path::Path, store_dir: &std::path::Path) {
        let started = STATE.with(|s| std::mem::replace(&mut s.borrow_mut().1, true));
        if started {
            return;
        }
        let json = match std::fs::read(ruleset) {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("yggterm adblock: read {}: {err}", ruleset.display());
                return;
            }
        };
        let _ = std::fs::create_dir_all(store_dir);
        let bytes = gtk::glib::Bytes::from_owned(json);
        let store_path = std::ffi::CString::new(store_dir.to_string_lossy().as_bytes())
            .expect("store path has no NUL");
        let identifier = std::ffi::CString::new("yggterm-adblock").unwrap();

        unsafe extern "C" fn save_done(
            source: *mut gtk::glib::gobject_ffi::GObject,
            result: *mut gtk::gio::ffi::GAsyncResult,
            _user_data: gtk::glib::ffi::gpointer,
        ) {
            let mut error: *mut gtk::glib::ffi::GError = std::ptr::null_mut();
            let filter = unsafe {
                wk::webkit_user_content_filter_store_save_finish(
                    source as *mut wk::WebKitUserContentFilterStore,
                    result,
                    &mut error,
                )
            };
            if filter.is_null() {
                let message = if error.is_null() {
                    "unknown error".to_string()
                } else {
                    let err: gtk::glib::Error =
                        unsafe { gtk::glib::translate::from_glib_full(error) };
                    err.to_string()
                };
                eprintln!("yggterm adblock: ruleset compile failed: {message}");
                PENDING.with(|p| p.borrow_mut().clear());
                return;
            }
            STATE.with(|s| s.borrow_mut().0 = Some(filter));
            let pending = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
            for webkit in pending {
                attach_to(&webkit, filter);
            }
        }

        unsafe {
            let store = wk::webkit_user_content_filter_store_new(store_path.as_ptr());
            wk::webkit_user_content_filter_store_save(
                store,
                identifier.as_ptr(),
                bytes.to_glib_none().0,
                std::ptr::null_mut(),
                Some(save_done),
                std::ptr::null_mut(),
            );
            // The store object stays alive for the async op via its own ref;
            // we deliberately leak our ref (one store per process, tiny).
        }
    }
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------
//
// What a download used to do in a surface, stated correctly because this
// comment is the record: it LANDED SOMEWHERE ELSE, silently. wry's
// `WebViewAttributes::default()` carried `download_started_handler:
// Some(Box::new(|_, _| true))`, `attach_handlers` registered it on the shared
// context, and its `decide-destination` computed the path itself —
// `dirs::download_dir().unwrap_or_else(current_dir)` and then
// `PathBuf::push(suggested)`. So under a bare compositor, where
// `XDG_DOWNLOAD_DIR` is unset, the file went to the GUI's CWD, under the
// server's raw suggested name (`../../x` walks straight out of a `push`), with
// no toast, no trace row, and nothing anywhere to say a transfer had happened.
// The transfer was never "dropped on the floor" — it was UNOWNED AND
// UNANNOUNCED, which is worse, because there was nothing to notice.
//
// The whole plane now lives here, and deliberately in ONE place:
//
//   * the DESTINATION is decided by `download_destination` and nothing else —
//     wry's default handler (see above) is switched off in the vendored wry, so
//     this is the only policy on the signal;
//   * the plumbing is connected ONCE PER `WebContext`, not per webview, because
//     the tabs of one session SHARE a context and `download-started` is a
//     context signal — N connections would decide one transfer N times;
//   * a transfer that ends, however it ends, produces exactly ONE terminal
//     event, from `finish_download_transfer`.
//
// The shell drains `take_downloads` each reconcile tick and turns each event
// into a toast + a trace row.

/// Where downloads land, under `$HOME`. Created on first use.
const DOWNLOADS_DIR_NAME: &str = "Downloads";

/// The name a download falls back to when the page suggests nothing usable
/// (empty, all dots, all separators — see `sanitize_download_file_name`).
const DOWNLOAD_FALLBACK_NAME: &str = "download";

/// Longest file name (in bytes) this host will write. ext4/btrfs/xfs all cap a
/// single name at 255 bytes, and a name over the cap fails the OPEN — i.e. the
/// download dies at the last moment with a filesystem error nobody can act on.
const DOWNLOAD_NAME_MAX_BYTES: usize = 200;

/// What happened to one download, as the surface host saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceDownloadPhase {
    /// The destination is decided and the transfer is running.
    Started,
    /// The file is complete at its destination.
    Completed {
        /// Bytes received, as the engine counted them.
        bytes: u64,
    },
    /// The transfer ended without the file, and the partial has been swept.
    Failed {
        /// The engine's own words (a `GError` message), never a generic
        /// "download failed" — the user has to be able to tell "disk full" from
        /// "connection reset" from "cancelled".
        reason: String,
    },
}

/// One transition of one download, drained by the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceDownloadEvent {
    /// The surface whose page started it, when the engine still names a view.
    /// `None` for a transfer whose view has already gone (a download outlives
    /// the tab that started it — see `DownloadInFlight`).
    pub surface_id: Option<u64>,
    /// The file name actually used on disk — sanitized and uniquified, so this
    /// is what the user will find, not what the server asked for.
    pub file_name: String,
    /// Absolute destination path.
    pub destination: PathBuf,
    /// The URL the transfer came from.
    pub url: String,
    /// Which transition this is.
    pub phase: SurfaceDownloadPhase,
}

/// A transfer still running, and the engine it is running on.
///
/// The `Rc<RefCell<WebContext>>` is the POINT: closing the tab that started a
/// download must not truncate the file. Holding the context here keeps its
/// network process alive (and keeps `prune_contexts`, which sweeps on
/// `strong_count == 1`, from taking it) until the transfer ends on its own
/// terms — WebKitGTK offers no "detach" verb, so *outliving the surface* is
/// spelled as an owner that outlives it. The entry is dropped on the terminal
/// signal, which is also what breaks the transient ref cycle it makes
/// (context -> `download-started` closure -> this list -> context).
///
/// TEARDOWN, both halves, because only one of them is ours to decide: if the
/// engine carries the transfer on, it finishes here and the user gets the
/// completion; if the engine gives up when the view it was started from is
/// destroyed, that arrives as `failed` and `finish_download_transfer` SWEEPS
/// THE PArecordsAL. What is ruled out either way is the third outcome — a
/// truncated file left sitting under the full name.
///
/// The type parameter exists ONLY so this entry can be DRIVEN in a test: a real
/// `WebContext` needs a display and a network process, which no test on this
/// host has, and a rule that can only be source-scanned is a rule nobody can
/// prove. `Rc` cannot tell a stand-in from an engine, and neither can
/// `retain_held_contexts`, which is the point.
struct DownloadInFlight<C = RefCell<WebContext>> {
    /// Monotonic, host-local. Identity here is deliberately NOT the `Download`
    /// object: WebKit owns that, and dropping the last reference to one inside
    /// its own signal handler is a use-after-free waiting to happen. No handle
    /// to it is kept for the same reason — this entry is a LIFETIME, not a
    /// remote control.
    id: u64,
    /// The engine that must not die under the transfer.
    _ctx: Rc<C>,
}

/// THE context sweep rule, and the whole detach policy in one line: an engine
/// survives while ANYONE besides the map itself holds it.
///
/// The map's own entry is a strong ref, so "nobody wants this any more" is
/// `strong_count == 1`. Holders are live surfaces — and RUNNING DOWNLOADS, via
/// `DownloadInFlight`. That second kind of holder is how "closing the tab does
/// not truncate the file" is spelled: with the tab gone but a transfer running,
/// the count is still 2, the sweep leaves the network process standing, and the
/// engine is taken by the NEXT sweep after the transfer releases it.
///
/// Free and generic rather than inline in `prune_contexts` so the rule can be
/// DRIVEN by a test instead of only source-scanned: a real `WebContext` needs a
/// display this host does not have, and `Rc` cannot tell a stand-in from an
/// engine.
fn retain_held_contexts<K: Eq + std::hash::Hash, C>(contexts: &mut HashMap<K, Rc<C>>) {
    contexts.retain(|_, ctx| Rc::strong_count(ctx) > 1);
}

/// `$HOME/Downloads`, created if missing.
///
/// Deliberately NOT `XDG_DOWNLOAD_DIR`: that is a per-desktop-session variable
/// which is frequently unset under a bare compositor, and a downloads directory
/// that moves depending on how the GUI was launched is the kind of
/// non-determinism this workspace refuses. `$HOME` is the only input.
fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = home.join(DOWNLOADS_DIR_NAME);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing::warn!(?error, ?dir, "web surface: downloads directory unusable");
    }
    dir
}

/// Reduce a server-suggested file name to a PLAIN BASENAME that cannot name
/// anywhere but the downloads directory.
///
/// The suggestion is attacker-controlled (`Content-Disposition: filename=...`,
/// or the tail of a URL), and the engine hands it over verbatim. Every rule
/// here exists because the alternative writes a file somewhere the user did not
/// ask for:
///
///   * path separators are cut, keeping only the last segment, so
///     `../../.ssh/authorized_keys` becomes `authorized_keys` INSIDE
///     `~/Downloads`. Backslashes count as separators too — the name may have
///     come from a Windows server, and `..\..\x` must not survive as one
///     segment;
///   * leading dots are stripped, so a download cannot silently become a
///     dotfile (`.bashrc`), and so `..` cannot survive as a name at all;
///   * control characters (including NUL, which would truncate the path at the
///     syscall boundary) and the separators' own leftovers go;
///   * an empty result falls back to a fixed name rather than to anything
///     derived from the URL — a fallback that can still be steered is not a
///     fallback.
fn sanitize_download_file_name(suggested: &str) -> String {
    let last_segment = suggested
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    let mut cleaned: String = last_segment
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '/' && *ch != '\\')
        .collect();
    while cleaned.starts_with('.') {
        cleaned.remove(0);
    }
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return DOWNLOAD_FALLBACK_NAME.to_string();
    }
    // Truncate on a char boundary, keeping the head: the extension may be lost
    // on an absurd name, but the name is still openable, which a name the
    // filesystem rejects is not.
    let mut truncated = cleaned.to_string();
    while truncated.len() > DOWNLOAD_NAME_MAX_BYTES {
        truncated.pop();
    }
    if truncated.is_empty() {
        return DOWNLOAD_FALLBACK_NAME.to_string();
    }
    truncated
}

/// Split a sanitized name into (stem, extension-with-dot) for uniquifying.
/// First dot wins, so `archive.tar.gz` uniquifies to `archive (1).tar.gz`
/// rather than to `archive.tar (1).gz`.
fn split_download_name(file_name: &str) -> (&str, &str) {
    match file_name.find('.') {
        Some(index) if index > 0 => (&file_name[..index], &file_name[index..]),
        _ => (file_name, ""),
    }
}

/// "Is this name already taken?" — THE production answer, and deliberately not
/// `Path::exists`.
///
/// `Path::exists` FOLLOWS symlinks, so it answers `false` for a DANGLING one: a
/// symlink at `~/Downloads/report.pdf` pointing at `~/.ssh/authorized_keys`
/// (which does not exist yet) reads as a free name, the uniquifier hands it to
/// the engine, and the write lands OUTSIDE the downloads directory — undoing
/// the whole point of `sanitize_download_file_name`. `symlink_metadata` does not
/// follow, so anything at the name at all — file, directory, live link, dangling
/// link — counts as taken and the transfer uniquifies past it.
fn download_name_is_taken(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// The collision policy: NEVER overwrite. `x.txt` taken ⇒ `x (1).txt`, then
/// `x (2).txt`, ... — the idiom every browser uses, and the one a user reading
/// their downloads folder can decode without being told.
///
/// `exists` is injected so the rule is a pure function of the directory's
/// contents; production passes `download_name_is_taken` and so does every lock.
fn unique_download_path(dir: &Path, file_name: &str, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    let candidate = dir.join(file_name);
    if !exists(&candidate) {
        return candidate;
    }
    let (stem, ext) = split_download_name(file_name);
    let mut counter = 1u32;
    loop {
        let candidate = dir.join(format!("{stem} ({counter}){ext}"));
        if !exists(&candidate) {
            return candidate;
        }
        counter += 1;
        // A directory that answers "taken" a million times running is a
        // filesystem lying to us; write the collision rather than spin forever.
        if counter > 1_000_000 {
            return candidate;
        }
    }
}

/// THE destination policy: sanitize, then uniquify, always inside `dir`.
/// One function, called from exactly one place in production
/// (`connect_download_plumbing`'s `decide-destination`), so "where does a
/// download go" has a single answer.
fn download_destination(dir: &Path, suggested: &str, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    unique_download_path(dir, &sanitize_download_file_name(suggested), exists)
}

/// End a transfer: the ONE place that decides what a finished download became
/// and what is left on disk.
///
/// A failure sweeps the partial file. WebKitGTK writes straight to the
/// destination (there is no `.part` staging), so a transfer that dies halfway
/// leaves a truncated file with the right name, the right icon and the wrong
/// contents — a file that MASQUERADES AS COMPLETE. Deleting it is what makes
/// the failed event the whole truth.
///
/// THE SWEEP IS GATED ON OWNERSHIP, and this is not a nicety. Because
/// `decide-destination` sets `set_allow_overwrite(false)`, "the destination
/// already exists" is a FIRST-CLASS engine failure — and in exactly that
/// failure the file sitting at `destination` is not ours: another transfer
/// decided the same name in the same main-loop turn (the uniquifier reads the
/// directory, and WebKit does not create the file until after
/// `decide-destination` returns), or another process wrote it in that window.
/// Sweeping unconditionally would delete a stranger's file on the one code path
/// where the failure MEANS a stranger's file. `destination_was_created`
/// carries the engine's own answer — WebKit's `created-destination` signal,
/// which fires when and only when it created the file it is about to write —
/// so this deletes a partial only when there is a partial of ours to delete.
fn finish_download_transfer(
    surface_id: Option<u64>,
    url: String,
    file_name: String,
    destination: PathBuf,
    bytes: u64,
    failure: Option<String>,
    destination_was_created: bool,
) -> SurfaceDownloadEvent {
    let phase = match failure {
        Some(reason) => {
            if destination_was_created {
                if let Err(error) = std::fs::remove_file(&destination) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(
                            ?error,
                            ?destination,
                            "web surface: partial download could not be swept"
                        );
                    }
                }
            }
            SurfaceDownloadPhase::Failed { reason }
        }
        None => SurfaceDownloadPhase::Completed { bytes },
    };
    SurfaceDownloadEvent {
        surface_id,
        file_name,
        destination,
        url,
        phase,
    }
}

/// Which surface (if any) a webkit view belongs to. A download names its view;
/// the shell wants the session, and the surface id is the only bridge.
fn surface_id_for_view(
    surfaces: &Rc<RefCell<HashMap<u64, Surface>>>,
    view: &webkit2gtk::WebView,
) -> Option<u64> {
    use wry::WebViewExtUnix as _;
    // `try_borrow`: this runs inside a WebKit signal handler, and a naming
    // convenience must never be able to panic the GUI over a borrow it does not
    // control. An unnamed surface is a lesser failure than a dead process.
    surfaces
        .try_borrow()
        .ok()?
        .iter()
        .find(|(_, surface)| &surface.webview.webview() == view)
        .map(|(id, _)| *id)
}

/// Wire WebKit's download signals for one `WebContext`. Called ONCE per context,
/// from `open`, on the surface that created it.
///
/// Connection topology, and why it is this shape:
///
///   * `download-started` is a signal on the CONTEXT, and yggterm shares one
///     context between the tabs of a session — so this is per context, guarded
///     by `context_is_new`, never per surface;
///   * `decide-destination` answers with our path and returns `true` (handled).
///     Nothing else is connected to it in this process — the vendored wry's
///     default handler is switched off — so the answer is unambiguous;
///   * `created-destination` fires when the ENGINE created the file. That is
///     the only trustworthy answer to "is the thing at this path ours?", and
///     the failure sweep is gated on it — see `finish_download_transfer`;
///   * `failed` fires BEFORE `finished` and carries the reason; `finished`
///     always fires. So the reason is parked and the single terminal event is
///     emitted from `finished`, which is why a transfer can never produce both
///     a "completed" and a "failed" row.
fn connect_download_plumbing(
    context: &webkit2gtk::WebContext,
    ctx_cell: &Rc<RefCell<WebContext>>,
    surfaces: &Rc<RefCell<HashMap<u64, Surface>>>,
    events: &Rc<RefCell<Vec<SurfaceDownloadEvent>>>,
    in_flight: &Rc<RefCell<Vec<DownloadInFlight>>>,
    next_transfer_id: &Rc<Cell<u64>>,
) {
    use webkit2gtk::{DownloadExt as _, URIRequestExt as _, WebContextExt as _};

    // WEAK, deliberately: the closure is owned by the context, so a strong ref
    // here would be a permanent cycle and the engine would never be freed. The
    // strong ref is taken per transfer, into `in_flight`, and released when the
    // transfer ends.
    let ctx_weak = Rc::downgrade(ctx_cell);
    let surfaces = surfaces.clone();
    let events = events.clone();
    let in_flight = in_flight.clone();
    let next_transfer_id = next_transfer_id.clone();

    context.connect_download_started(move |_context, download| {
        let transfer_id = next_transfer_id.get();
        next_transfer_id.set(transfer_id + 1);
        if let Some(ctx) = ctx_weak.upgrade() {
            in_flight.borrow_mut().push(DownloadInFlight {
                id: transfer_id,
                _ctx: ctx,
            });
        }

        let url = download
            .request()
            .and_then(|request| request.uri())
            .map(|uri| uri.to_string())
            .unwrap_or_default();
        let surface_id = download
            .web_view()
            .and_then(|view| surface_id_for_view(&surfaces, &view));
        // Written by `decide-destination`, read by the terminal handler: the
        // path and name the user will actually see.
        let landed = Rc::new(RefCell::new(None::<(String, PathBuf)>));
        // Parked by `failed`, consumed by `finished`.
        let failure = Rc::new(RefCell::new(None::<String>));
        // Did WE create the file at the destination? Only the engine knows, and
        // it says so: `created-destination` fires when WebKit has created the
        // file it is about to write into. Without this the failure sweep would
        // delete whatever is at the destination — and the canonical
        // `set_allow_overwrite(false)` failure is precisely "something else is
        // already there".
        let created_destination = Rc::new(Cell::new(false));
        download.connect_created_destination({
            let created_destination = created_destination.clone();
            move |_download, _path| {
                created_destination.set(true);
            }
        });

        download.connect_decide_destination({
            let events = events.clone();
            let landed = landed.clone();
            let url = url.clone();
            move |download, suggested_filename| {
                let dir = downloads_dir();
                let destination =
                    download_destination(&dir, suggested_filename, &download_name_is_taken);
                // Belt to the uniquifier's braces: even if the name were taken
                // between the check and the open, the engine must not clobber.
                download.set_allow_overwrite(false);
                download.set_destination(&destination.to_string_lossy());
                let file_name = destination
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| DOWNLOAD_FALLBACK_NAME.to_string());
                *landed.borrow_mut() = Some((file_name.clone(), destination.clone()));
                events.borrow_mut().push(SurfaceDownloadEvent {
                    surface_id,
                    file_name,
                    destination,
                    url: url.clone(),
                    phase: SurfaceDownloadPhase::Started,
                });
                // Handled. Returning false would leave the transfer to whatever
                // else is on the signal — which is nothing, i.e. no destination
                // and a download that dies silently.
                true
            }
        });

        download.connect_failed({
            let failure = failure.clone();
            move |_download, error| {
                // The engine's own words. `finished` follows and emits.
                *failure.borrow_mut() = Some(error.to_string());
            }
        });

        download.connect_finished({
            let events = events.clone();
            let in_flight = in_flight.clone();
            let landed = landed.clone();
            let failure = failure.clone();
            let created_destination = created_destination.clone();
            move |download| {
                let (file_name, destination) = landed.borrow().clone().unwrap_or_else(|| {
                    let path = download
                        .destination()
                        .map(|value| PathBuf::from(value.to_string()))
                        .unwrap_or_default();
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| DOWNLOAD_FALLBACK_NAME.to_string());
                    (name, path)
                });
                events.borrow_mut().push(finish_download_transfer(
                    surface_id,
                    url.clone(),
                    file_name,
                    destination,
                    download.received_data_length(),
                    failure.borrow_mut().take(),
                    created_destination.get(),
                ));
                // Release the engine hold OUTSIDE this handler: dropping the
                // last reference to the download (or to its context) while
                // WebKit is still emitting on it is a use-after-free.
                let in_flight = in_flight.clone();
                gtk::glib::idle_add_local_once(move || {
                    in_flight.borrow_mut().retain(|entry| entry.id != transfer_id);
                });
            }
        });
    });
}

/// A window a page opened from inside a surface — `window.open`, a
/// `target="_blank"` link, a middle/ctrl-click.
///
/// The webview ALREADY EXISTS by the time the shell hears about this: WebKit's
/// `create` signal must be answered synchronously with the view that will run
/// the new window, so the surface host builds it in the handler (RELATED to the
/// opener, which is what gives it a live `window.opener`) and hands it back.
/// The shell's job is to adopt it as a tab, not to open one.
pub struct SurfacePopup {
    /// The surface whose page asked for the window: the tab the popup belongs
    /// beside, and whose profile/egress it shares.
    pub opener_id: u64,
    /// The already-built popup webview, registered in the host under this id.
    pub popup_id: u64,
    /// The URL the window was opened on. WebKit is already loading it into the
    /// popup's webview — this is for the tab's model, not a navigation to make.
    pub url: String,
    /// A middle/ctrl-click means "open it, but do not go there" (Chrome's
    /// grammar). A `window.open` is a foreground request.
    pub background: bool,
}

/// What a find request asks WebKit's find controller to do — the engine half of
/// the shell's `web_find::FindStep`.
///
/// Two names for one idea is the price of the crate boundary: a vendored
/// dioxus-desktop cannot depend on `yggterm-shell`, so the shell owns find
/// POLICY (the option mask, the match cap, the position cycle) and maps into
/// this at the one call site. The mapping is total and exhaustive in both
/// directions, so neither side can grow a case the other cannot express.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindAction {
    /// A fresh search: highlight every match and select the first.
    Search,
    /// Move the selection to the next match of the CURRENT search.
    Next,
    /// Move it to the previous one.
    Previous,
    /// End the search — `search_finish`, which is what drops the highlights.
    Close,
}

/// Owns the main window's `gtk::Overlay` and the set of live surface webviews.
/// Held (Linux only) on `DesktopService`; driven from the shell via the
/// `open_web_surface` / `web_surface_*` methods on `DesktopContext`.
pub struct WebSurfaceHost {
    overlay: gtk::Overlay,
    /// Style provider on the overlay's base child (the native backdrop):
    /// `set_backdrop_color` reloads it with the theme background color.
    backdrop_css: gtk::CssProvider,
    /// Same theme color, painted by EVERY surface container's draw handler
    /// (GtkFixed renders no CSS background — it needs an explicit fill): an
    /// unpainted webview (fresh create, first load in flight) composites
    /// nothing under DMABuf, and with backgrounded pages left attached under
    /// the glass (the soft stash) whatever sits below would show through the
    /// hole — a STALE OTHER PAGE, not the backdrop (live-caught: a new
    /// surface's hole showed the previous session's page until first paint).
    /// The fill restores the first-paint contract: theme background until
    /// the page's first frame. `None` (legacy — `set_backdrop_color` only
    /// runs under glass) draws nothing, exactly the old behavior.
    backdrop_rgb: Rc<Cell<Option<(u8, u8, u8)>>>,
    /// The shell webview's container ("the glass") when Phase F under-glass
    /// stacking is active: pages sit BELOW it, chrome DOM draws over them, and
    /// an input-shape hole per page routes pointer events through. `None` =
    /// legacy stacking (pages above the shell), either because the host was
    /// built before `install_glass` ran or because the self-probe demoted it.
    glass: Rc<RefCell<Option<gtk::Widget>>>,
    /// Last input-hole set pushed to the glass — region pushes are gated on
    /// change so the per-tick reconciler doesn't spam the compositor.
    last_glass_holes: RefCell<Option<(Vec<(i32, i32, i32, i32)>, Vec<(i32, i32, i32, i32)>)>>,
    surfaces: Rc<RefCell<HashMap<u64, Surface>>>,
    /// Live `WebContext`s, keyed by `web_context_key`. THE single owner of "which
    /// engine context backs this jar+egress+endpoint"; surfaces borrow from here
    /// and never construct their own.
    ///
    /// This map is why two tabs of one session are one web process pool and one
    /// cookie jar. It used to be one context PER SURFACE, unconditionally, which
    /// meant two tabs of a single ychrome invocation ran two WebKitWebProcesses,
    /// two WebKitNetworkProcesses, and — the real bug — two independent
    /// in-memory cookie stores writing the same Netscape-text `cookies` file:
    /// a login in tab A was invisible in tab B, and whichever flushed last won.
    ///
    /// Entries are strong refs, so they are swept on `close` once nothing else
    /// holds them (see `prune_contexts`) — the last surface leaving still tears
    /// the context down, exactly as before, just no longer the only surface.
    contexts: Rc<RefCell<HashMap<String, Rc<RefCell<WebContext>>>>>,
    /// Native surface ids. The HOST allocates them, because it is no longer the
    /// only thing that creates surfaces: a popup is born inside a WebKit signal
    /// handler, and two allocators would eventually hand out the same id.
    next_id: Rc<Cell<u64>>,
    /// Popups a page opened from inside a surface, drained by the shell each
    /// reconcile tick and adopted as tabs of the opener's session.
    ///
    /// The webview is built here, in the `create` handler, and NOT by the shell.
    /// That is the whole point: WebKit will only give a new window a live
    /// `window.opener` if it is answered with a view RELATED to the opener, and
    /// that answer has to be synchronous. Reopening the URL later in a fresh
    /// webview (what this used to do) produced a popup with `window.opener ===
    /// null` — so an OAuth callback's `opener.postMessage(...)` went nowhere and
    /// its `window.close()` closed nothing: the sign-in completed, the popup sat
    /// there forever, and the page that started it never learned it had won.
    popups: Rc<RefCell<Vec<SurfacePopup>>>,
    /// Pages that called `window.close()`. A script-opened window is allowed to
    /// close itself, and a browser that ignores that strands every OAuth popup
    /// ever written.
    close_requests: Rc<RefCell<Vec<SurfaceCloseRequest>>>,
    /// Download transitions since the shell last drained them — one `Started`
    /// and exactly one terminal event per transfer.
    downloads: Rc<RefCell<Vec<SurfaceDownloadEvent>>>,
    /// Transfers still running, each holding its engine alive so closing the
    /// tab that started a download cannot truncate the file. See
    /// `DownloadInFlight`.
    downloads_in_flight: Rc<RefCell<Vec<DownloadInFlight>>>,
    /// Host-local transfer ids: identity for an in-flight entry that never
    /// touches the `Download` object's own lifetime.
    next_transfer_id: Rc<Cell<u64>>,
    /// F.1 reveal trigger. With the titlebar clamp gone, the auto-hide hover
    /// zone sits INSIDE the input hole, so the shell webview never sees the
    /// mousemove. Each page webview gets a GTK motion observer (Proceed —
    /// observe, never consume) that calls this when the pointer enters the
    /// window's top edge zone; the notifier forwards into the shell webview,
    /// which runs its normal reveal logic.
    edge_motion: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

/// The top-edge motion zone (window coords, logical px) that forwards to the
/// shell's titlebar reveal. Twin of the shell's
/// TITLEBAR_AUTOHIDE_SENSOR_HEIGHT_PX (6px) plus slack for the window border
/// inset — over-forwarding is harmless (the reveal is idempotent and the
/// shell still decides), under-forwarding makes the titlebar unreachable
/// over a maximized page.
const GLASS_EDGE_REVEAL_ZONE_PX: f64 = 8.0;

/// Observe pointer motion on a page webview and forward top-edge entry to the
/// shell's reveal logic. `Propagation::Proceed` always — the page's own input
/// is untouched; this only watches. Gated at EVENT time on the glass being
/// armed (a runtime demotion silences it without disconnecting anything).
/// Fires on the out→in zone transition only, mirroring `mouseenter`.
fn connect_edge_motion_observer(
    webkit: &webkit2gtk::WebView,
    container: &gtk::Fixed,
    glass: &Rc<RefCell<Option<gtk::Widget>>>,
    edge_motion: &Rc<RefCell<Option<Rc<dyn Fn()>>>>,
) {
    let glass = glass.clone();
    let edge_motion = edge_motion.clone();
    let container = container.clone();
    let in_zone = Cell::new(false);
    // The webview was just built (unrealized): motion events can still be
    // added. WebKit requests them itself for hover, but do not depend on it.
    webkit.add_events(gdk::EventMask::POINTER_MOTION_MASK);
    webkit.connect_motion_notify_event(move |_, event| {
        if glass.borrow().is_some() {
            let (_, y) = event.position();
            let window_y = container.margin_top() as f64 + y;
            let zone = window_y <= GLASS_EDGE_REVEAL_ZONE_PX;
            if zone && !in_zone.get() {
                let notify = edge_motion.borrow().clone();
                if let Some(notify) = notify {
                    notify();
                }
            }
            in_zone.set(zone);
        }
        gtk::glib::Propagation::Proceed
    });
}

// ===========================================================================
// Seat-input detection (agent control plane, acceptance gate 9).
//
// The agent's `do` verbs inject GDK events with `send_event = 0` and the real
// seat device, precisely so WebKit treats them as genuine — which means
// `isTrusted` is TRUE for them and a page-side listener **cannot** tell agent
// input from human input. The distinction has to be made HERE, where we know
// which events we ourselves produced.
//
// How: every injection ends in a *synchronous* `WidgetExt::event(...)` call, so
// wrapping that one call in a flag is exact — anything the observer sees while
// the flag is clear came from the seat. GTK delivery is single-threaded and
// synchronous, so this is a lexical scope, NOT a timing window (the repo forbids
// timing-dependent behavior).
// ===========================================================================

thread_local! {
    /// Set only for the duration of one synchronous injected-event delivery.
    static INJECTING_EVENT: Cell<bool> = const { Cell::new(false) };
    /// Per-webview count of real seat inputs observed but not yet consumed.
    static SEAT_INPUT_COUNTS: RefCell<HashMap<u64, u64>> = RefCell::new(HashMap::new());
    /// Injected events handed to GTK for a surface but not yet seen by the
    /// observer, as the MILLISECOND each credit was granted (oldest first). The
    /// backstop for the lexical flag above — see [`spend_injection_credit_at`].
    static INJECTED_CREDITS: RefCell<HashMap<u64, VecDeque<u64>>> = RefCell::new(HashMap::new());
    /// Who owned the toplevel's KEYBOARD focus before an agent injection
    /// borrowed it. See [`note_focus_owner_before_injection`].
    static BORROWED_FOCUS: RefCell<Option<BorrowedFocus>> = const { RefCell::new(None) };
    /// Bumped by every injection; only the give-back timer still holding the
    /// current value acts, so one burst of injected events gives the focus back
    /// ONCE, after its last event, instead of per keystroke.
    static FOCUS_GIVEBACK_TOKEN: Cell<u64> = const { Cell::new(0) };
}

/// How long after the LAST injected event the borrowed keyboard focus goes back.
/// Long enough that a multi-key fill (select-all, delete, N characters — all
/// separate `inject_key` calls milliseconds apart) is one loan and therefore one
/// `blur` for the page; short enough that the human never notices the gap.
const FOCUS_GIVEBACK_DELAY_MS: u64 = 150;

/// The toplevel keyboard focus an agent injection borrowed, and who to hand it
/// back to.
struct BorrowedFocus {
    window: gtk::Window,
    /// The widget that owned `window`'s keyboard focus before the injection —
    /// on the live host that is the SHELL's own webview, i.e. the user's
    /// terminal.
    previous: gtk::Widget,
    /// The surface webview the focus was lent to.
    borrower: gtk::Widget,
}

/// Record who owns the toplevel's keyboard focus BEFORE an agent injection takes
/// it, so [`schedule_focus_giveback`] can hand it straight back.
///
/// ⚠ **This is the fifth focus-theft path** (2026-07-26; the first four are in
/// `docs/pending-bugs.md`). It is the one no JS-side probe could ever see,
/// because it is not a JS `focus()` at all: `gtk_widget_grab_focus` on a surface
/// webview sets the **GtkWindow's focus widget**, which takes keyboard focus off
/// the shell's own webview. The old note on `inject_key` said the grab was
/// "widget-local — it does not move the seat's global focus on screen". That is
/// true of the SEAT and false of the toplevel: the window stays active, the
/// shell's DOM `activeElement` stays on the xterm helper textarea, and yet
/// `document.hasFocus()` in the shell goes false and every keystroke the user
/// types lands in the agent's invisible page. Live-caught on jojo with a
/// simultaneous two-point read: shell `hasFocus:false` while a never-revealed
/// agent surface reported `hasFocus:true`.
///
/// An injected event is the AGENT's, not the human's. It may borrow the focus it
/// needs; it may not keep it. A surface the human actually clicks still takes
/// focus the normal way — that is a real seat event and never comes through here.
fn note_focus_owner_before_injection(webview: &webkit2gtk::WebView) {
    let borrower: gtk::Widget = webview.clone().upcast();
    let Some(window) = borrower
        .toplevel()
        .and_then(|top| top.downcast::<gtk::Window>().ok())
    else {
        return;
    };
    let Some(previous) = gtk::prelude::GtkWindowExt::focused_widget(&window) else {
        return;
    };
    if previous == borrower {
        return; // already ours: mid-burst, or the human handed it over
    }
    BORROWED_FOCUS.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(BorrowedFocus {
                window,
                previous,
                borrower,
            });
        }
    });
}

/// Arm (or re-arm) the give-back of a borrowed keyboard focus. Every injection
/// calls this, so a burst gives back once, [`FOCUS_GIVEBACK_DELAY_MS`] after its
/// last event.
fn schedule_focus_giveback() {
    let token = FOCUS_GIVEBACK_TOKEN.with(|token| {
        let next = token.get().wrapping_add(1);
        token.set(next);
        next
    });
    gtk::glib::timeout_add_local_once(
        std::time::Duration::from_millis(FOCUS_GIVEBACK_DELAY_MS),
        move || {
            if FOCUS_GIVEBACK_TOKEN.with(|current| current.get()) != token {
                return; // a later injection re-armed this; that one gives back
            }
            let Some(loan) = BORROWED_FOCUS.with(|slot| slot.borrow_mut().take()) else {
                return;
            };
            // Only ever take focus back OFF the widget it was lent to. If the
            // human clicked something in the meantime, that is THEIR focus now
            // and the agent has no business moving it.
            if gtk::prelude::GtkWindowExt::focused_widget(&loan.window).as_ref()
                != Some(&loan.borrower)
            {
                return;
            }
            let previous_still_here = loan
                .previous
                .toplevel()
                .and_then(|top| top.downcast::<gtk::Window>().ok())
                .is_some_and(|top| top == loan.window);
            if !previous_still_here || !loan.previous.can_focus() {
                return; // the lender is gone — leave GTK's own choice alone
            }
            gtk::prelude::GtkWindowExt::set_focus(&loan.window, Some(&loan.previous));
        },
    );
}

/// Grant one credit per injected event about to be delivered to `surface_id`.
///
/// The lexical `INJECTING_EVENT` flag is only exact if GTK emits
/// `button-press-event` synchronously inside `WidgetExt::event`. If any injected
/// event is instead observed after that call returns, the flag is already clear
/// and the observer books OUR OWN injection as the human taking the surface —
/// which preempts the agent's batch, and (because a batch id is per-GUI-process)
/// locks that agent out of the surface permanently. That is the single-shot
/// `do` defect: verb 1 lands, verbs 2..N are all refused `preempted`.
///
/// Credits make the suppression count-based rather than dispatch-order-based, so
/// it holds either way.
///
/// # Why a credit EXPIRES ([`INJECTION_CREDIT_TTL_MS`])
///
/// The credit was originally held until the next [`take_seat_input_count`], and
/// the shell only reads that counter at the START of the next verb — never at
/// the end of the verb that granted the credits. So a fill that granted a dozen
/// credits (select-all, delete, ten characters), every one of them already
/// suppressed by the lexical flag because delivery is synchronous, left a dozen
/// unspent credits sitting in the INTER-VERB GAP. The user's next real
/// keystrokes were then spent against them: their characters landed in the page
/// with no preempt and no journal, and the arbiter — the whole mechanism that
/// exists to let the human take a surface back — saw nothing at all.
///
/// A credit exists to cover ONE injected event that GTK may deliver to the
/// observer after `WidgetExt::event` returns. That is a queue hop, not a wait:
/// anything still unspent [`INJECTION_CREDIT_TTL_MS`] after its grant cannot
/// belong to that dispatch any more, so it is dropped before it can be spent
/// against a human gesture. The TTL is a SECOND, tighter bound — the
/// end-of-verb drop in [`take_seat_input_count`] stays exactly as it was.
///
/// Determinism: the clock is a parameter (`now_ms`), never read inside the
/// bookkeeping, so tests drive expiry exactly rather than sleeping. The `pub`
/// wrappers are the only things that read a clock.
///
/// `pub` because the seat-input accounting is only HALF of the human-preempt
/// gate: the other half is the shell's `web_do_gate` / batch loop, which
/// consumes the count this produces. A lock that drives the shell half with a
/// literal count is not a lock at all (it synthesizes the defect away), so the
/// shell's tests drive the REAL accounting through these entry points. They are
/// pure thread-local bookkeeping — no GTK, no webview.
pub fn grant_injection_credits(surface_id: u64, count: u64) {
    grant_injection_credits_at(surface_id, count, monotonic_millis());
}

/// [`grant_injection_credits`] with the grant instant supplied — the entry point
/// a test drives so credit expiry is exercised without sleeping.
pub fn grant_injection_credits_at(surface_id: u64, count: u64, now_ms: u64) {
    INJECTED_CREDITS.with(|credits| {
        let mut credits = credits.borrow_mut();
        let granted = credits.entry(surface_id).or_default();
        for _ in 0..count {
            granted.push_back(now_ms);
        }
    });
}

/// How long an unspent injection credit can still be OURS.
///
/// An injected event is handed to GTK synchronously; the credit covers only the
/// case where the observer sees it after the delivery call returned. A quarter
/// of a second is orders of magnitude more than that queue hop and far less than
/// the gap between two agent verbs, which is the window the user types in.
pub const INJECTION_CREDIT_TTL_MS: u64 = 250;

/// Spend one credit for `surface_id`, returning true when the observed event is
/// one of ours rather than the seat's. Credits older than
/// [`INJECTION_CREDIT_TTL_MS`] are dropped first and can never be spent.
fn spend_injection_credit_at(surface_id: u64, now_ms: u64) -> bool {
    INJECTED_CREDITS.with(|credits| {
        let mut credits = credits.borrow_mut();
        let Some(granted) = credits.get_mut(&surface_id) else {
            return false;
        };
        // Grants are pushed in time order, so the expired ones are a prefix.
        while granted
            .front()
            .is_some_and(|granted_at| now_ms.saturating_sub(*granted_at) >= INJECTION_CREDIT_TTL_MS)
        {
            granted.pop_front();
        }
        granted.pop_front().is_some()
    })
}

/// Deliver an injected event with the "this is ours" flag set, so the seat-input
/// observer does not mistake the agent's own injection for the human.
fn deliver_injected_event(webview: &webkit2gtk::WebView, event: &gdk::Event) {
    INJECTING_EVENT.with(|flag| flag.set(true));
    gtk::prelude::WidgetExt::event(webview, event);
    INJECTING_EVENT.with(|flag| flag.set(false));
}

/// The seat-input observer's entry point: an input event was seen on this
/// surface. Books it as the human unless it is one of our own injections.
///
/// `pub` for the same reason as [`grant_injection_credits`] — see its note.
pub fn note_seat_input(surface_id: u64) {
    note_seat_input_at(surface_id, monotonic_millis());
}

/// [`note_seat_input`] with the observation instant supplied — the entry point a
/// test drives so credit expiry is exercised without sleeping.
pub fn note_seat_input_at(surface_id: u64, now_ms: u64) {
    if INJECTING_EVENT.with(|flag| flag.get()) {
        return; // our own injection, not the human
    }
    if spend_injection_credit_at(surface_id, now_ms) {
        return; // our own injection, observed after the lexical scope closed
    }
    SEAT_INPUT_COUNTS.with(|counts| {
        *counts.borrow_mut().entry(surface_id).or_insert(0) += 1;
    });
}

/// Milliseconds since this process first asked. Monotonic (`Instant`), so a
/// wall-clock step never makes a credit look fresh — and read ONLY by the two
/// `pub` wrappers above, never by the bookkeeping they call.
fn monotonic_millis() -> u64 {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    EPOCH
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Consume the count of real seat inputs seen on this surface since the last
/// call. Non-zero means the human touched it — the agent's batch is preempted.
pub fn take_seat_input_count(surface_id: u64) -> u64 {
    // One verb's credits belong to that verb. Anything unspent by now was
    // suppressed by the lexical flag (synchronous delivery) or never arrived;
    // carrying it forward would let it swallow a LATER real gesture, turning a
    // fix for the agent into a bug for the user.
    INJECTED_CREDITS.with(|credits| {
        credits.borrow_mut().remove(&surface_id);
    });
    SEAT_INPUT_COUNTS.with(|counts| counts.borrow_mut().remove(&surface_id).unwrap_or(0))
}

/// Forget a closed surface's seat-input tally.
pub fn forget_seat_input(surface_id: u64) {
    SEAT_INPUT_COUNTS.with(|counts| {
        counts.borrow_mut().remove(&surface_id);
    });
    INJECTED_CREDITS.with(|credits| {
        credits.borrow_mut().remove(&surface_id);
    });
}

// ===========================================================================
// JS dialog guard.
//
// WebKit BLOCKS the page's whole web process until a script dialog is answered,
// and one web process serves EVERY surface sharing a profile. Our surfaces are
// routinely invisible — soft-stashed, or created headless for an agent — and an
// invisible surface has nowhere to show a dialog and nobody to click it. So an
// unanswered `alert()` wedges that surface and every sibling on its profile,
// permanently: the process sits idle in `S`, and every eval/read/screenshot
// times out. Measured live (2026-07-24) on the services-desk AIS portal, then
// reproduced deliberately with a one-line `alert()` on a throwaway surface.
//
// The shell therefore answers every dialog itself, deterministically:
//   alert        -> OK (nothing else to say)
//   confirm      -> CANCEL — never confirm on behalf of a human who was never
//                   shown the question; cancel is the answer that changes least
//   beforeunload -> LEAVE. This one is the opposite call on purpose: the
//                   navigation was already asked for (by the page, the user, or
//                   an agent verb), and answering "stay" would PIN the surface
//                   on its current page forever. Measured: the services-desk
//                   portal arms `beforeunload`, so a "stay" answer silently
//                   killed every SSO hand-off out of it — the failure looked
//                   exactly like a navigation that never committed.
//   prompt       -> cancel with empty text
// The message is not lost: it is recorded per surface so the shell can trace it
// and an agent can read what the page asked.
// ===========================================================================

/// One JS dialog a page raised and the shell answered for it.
#[derive(Clone, Debug)]
pub struct ScriptDialogRecord {
    pub surface_id: u64,
    /// `alert` | `confirm` | `prompt` | `beforeunload` | `unknown`
    pub kind: &'static str,
    pub message: String,
    pub uri: String,
    /// The answer the shell gave the page.
    pub answered: &'static str,
}

thread_local! {
    /// Dialogs answered but not yet consumed by the shell.
    static SCRIPT_DIALOGS: RefCell<Vec<ScriptDialogRecord>> = const { RefCell::new(Vec::new()) };
}

/// Consume the dialogs answered since the last call. The shell polls this so a
/// page's question reaches a human (or an agent's log) instead of vanishing.
pub fn take_script_dialogs() -> Vec<ScriptDialogRecord> {
    SCRIPT_DIALOGS.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

/// Answer JS dialogs on this surface instead of letting WebKit block on one.
/// See the section comment above — this is what keeps an invisible surface from
/// becoming permanently unresponsive.
fn connect_script_dialog_guard(webkit: &webkit2gtk::WebView, surface_id: u64) {
    use webkit2gtk::{ScriptDialogType, WebViewExt as _};
    webkit.connect_script_dialog(move |view, dialog| {
        let (kind, answered) = match dialog.dialog_type() {
            ScriptDialogType::Alert => ("alert", "ok"),
            ScriptDialogType::Confirm => {
                dialog.confirm_set_confirmed(false);
                ("confirm", "cancel")
            }
            ScriptDialogType::BeforeUnloadConfirm => {
                // TRUE = leave the page. See the section comment: answering
                // "stay" pins the surface on its current page for good.
                dialog.confirm_set_confirmed(true);
                ("beforeunload", "leave")
            }
            ScriptDialogType::Prompt => {
                dialog.prompt_set_text("");
                ("prompt", "cancel")
            }
            _ => ("unknown", "ok"),
        };
        let message = dialog.message().map(|m| m.to_string()).unwrap_or_default();
        let uri = view.uri().map(|u| u.to_string()).unwrap_or_default();
        tracing::warn!(
            surface_id,
            kind,
            answered,
            %uri,
            "web surface: answered a page JS dialog ({message})"
        );
        SCRIPT_DIALOGS.with(|log| {
            let mut log = log.borrow_mut();
            // A page in a dialog loop must not grow this without bound.
            if log.len() < 64 {
                log.push(ScriptDialogRecord {
                    surface_id,
                    kind,
                    message,
                    uri,
                    answered,
                });
            }
        });
        // TRUE = handled here; WebKit resumes the page as soon as we return.
        true
    });
}

/// Observe real seat input on a webview: button presses, key presses, scrolls
/// and touch — the gestures that mean "the human took this surface back".
///
/// Pointer MOTION is deliberately excluded: the pointer drifting across a
/// window is not intent, and counting it would preempt agent batches constantly.
fn connect_seat_input_observer(webkit: &webkit2gtk::WebView, surface_id: u64) {
    webkit.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::KEY_PRESS_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::TOUCH_MASK,
    );
    webkit.connect_button_press_event(move |_, _| {
        note_seat_input(surface_id);
        gtk::glib::Propagation::Proceed
    });
    webkit.connect_key_press_event(move |_, _| {
        note_seat_input(surface_id);
        gtk::glib::Propagation::Proceed
    });
    webkit.connect_scroll_event(move |_, _| {
        note_seat_input(surface_id);
        gtk::glib::Propagation::Proceed
    });
}

fn rect_logical(w: i32, h: i32) -> Rect {
    Rect {
        position: Position::Logical(LogicalPosition::new(0.0, 0.0)),
        size: Size::Logical(LogicalSize::new(w.max(1) as f64, h.max(1) as f64)),
    }
}

/// Place a surface at `(x, y)` and size it to `w × h`.
///
/// The webview's own **size request** must be updated, not just the container's.
/// `wry`'s `WebView::set_bounds` on a `GtkFixed` parent only `size_allocate`s the
/// webview; it never touches the size request that `add_to_container` set when the
/// webview was built. `GtkFixed` allocates children at their natural size, and the
/// natural size of a widget with a size request IS that request — so the very next
/// layout pass (the `queue_resize` every caller issues right after) snapped the
/// webview straight back to the size it was born with.
///
/// The surface could therefore be MOVED but never RESIZED. Opening the right rail
/// over a live web surface left the page painted across it, because a native child
/// widget draws above all DOM; closing the rail left a gap. Neither was visible to
/// `app screenshot`'s default backend, which composites the DOM and is blind to
/// native children — only `--backend os` shows it.
/// The glass input region, as a PURE function (the reconciler's rects in,
/// cairo region out — unit-tested; the GdkWindow application is separate).
/// Full window minus holes (page rects) plus covers (chrome declared over
/// pages). Empty holes ⇒ the FULL region: the safety invariant — zero pages
/// or any upstream doubt resolves to "chrome owns all input", never a dead
/// zone in the chrome.
fn glass_input_region(
    full: (i32, i32),
    holes: &[(i32, i32, i32, i32)],
    covers: &[(i32, i32, i32, i32)],
) -> cairo::Region {
    let full = cairo::RectangleInt::new(0, 0, full.0.max(1), full.1.max(1));
    let region = cairo::Region::create_rectangle(&full);
    if !holes.is_empty() {
        for &(x, y, w, h) in holes {
            if w > 0 && h > 0 {
                let _ = region.subtract_rectangle(&cairo::RectangleInt::new(x, y, w, h));
            }
        }
        for &(x, y, w, h) in covers {
            if w > 0 && h > 0 {
                let _ = region.union_rectangle(&cairo::RectangleInt::new(x, y, w, h));
            }
        }
    }
    region
}

#[cfg(test)]
mod glass_region_tests {
    use super::glass_input_region;

    fn contains(region: &cairo::Region, x: i32, y: i32) -> bool {
        region.contains_point(x, y)
    }

    #[test]
    fn a_single_hole_routes_its_rect_to_the_page_and_nothing_else() {
        let region = glass_input_region((800, 600), &[(100, 100, 200, 150)], &[]);
        assert!(!contains(&region, 200, 175), "hole center must pass through");
        assert!(contains(&region, 50, 50), "chrome outside the hole stays shell");
        assert!(contains(&region, 99, 100), "one px left of the hole stays shell");
        assert!(!contains(&region, 100, 100), "hole top-left passes through");
        assert!(contains(&region, 300, 100), "one px right of the hole stays shell");
    }

    #[test]
    fn a_cover_over_a_hole_stays_shell_interactive() {
        let region = glass_input_region(
            (800, 600),
            &[(100, 100, 400, 400)],
            &[(250, 20, 320, 90)], // toast overlapping the hole's top edge
        );
        assert!(contains(&region, 300, 105), "covered strip inside the hole is shell");
        assert!(!contains(&region, 300, 130), "uncovered hole below the toast passes");
    }

    #[test]
    fn a_cover_fully_inside_a_hole_is_an_island() {
        let region = glass_input_region(
            (800, 600),
            &[(0, 0, 800, 600)],
            &[(300, 200, 200, 100)], // dialog floating over a full-bleed page
        );
        assert!(contains(&region, 400, 250), "dialog rect is shell");
        assert!(!contains(&region, 100, 100), "page around the dialog passes");
    }

    #[test]
    fn two_holes_and_a_pinned_pane_all_pass() {
        let region = glass_input_region(
            (800, 600),
            &[(0, 0, 390, 600), (410, 0, 390, 600)],
            &[],
        );
        assert!(!contains(&region, 100, 300), "left pane passes");
        assert!(!contains(&region, 700, 300), "right pane passes");
        assert!(contains(&region, 400, 300), "the gutter between panes stays shell");
    }

    #[test]
    fn zero_holes_is_the_full_region_even_with_covers() {
        let region = glass_input_region((800, 600), &[], &[(10, 10, 50, 50)]);
        assert!(contains(&region, 5, 5));
        assert!(contains(&region, 400, 300));
        assert!(contains(&region, 799, 599), "full region: chrome owns everything");
    }

    #[test]
    fn degenerate_rects_are_ignored() {
        let region = glass_input_region((800, 600), &[(100, 100, 0, 150), (200, 200, 50, -1)], &[]);
        assert!(contains(&region, 100, 150), "zero-width hole is ignored");
        assert!(contains(&region, 225, 210), "negative-height hole is ignored");
    }
}

/// Keep the glass the TOP overlay child. Called after every surface attach —
/// `add_overlay` appends on top, so a page or popup attached after the glass
/// would silently draw above the chrome (legacy stacking for that one
/// surface). `reorder_overlay(.., -1)` moves the glass to the end.
fn restack_glass(overlay: &gtk::Overlay, glass: &Rc<RefCell<Option<gtk::Widget>>>) {
    let glass = glass.borrow();
    if let Some(glass) = glass.as_ref() {
        if glass.parent().is_some() {
            overlay.reorder_overlay(glass, -1);
        } else {
            tracing::warn!("web surface: glass installed but not parented to the overlay");
        }
    }
}

/// Apply `region` as the input shape of `widget`'s GdkWindow and every
/// ancestor window up to but EXCLUDING the toplevel (see
/// `set_glass_input_holes` for why both bounds matter).
fn apply_input_region_up_to_toplevel(widget: &gtk::Widget, region: &cairo::Region) {
    let toplevel_window = widget.toplevel().and_then(|toplevel| toplevel.window());
    let mut current = widget.window();
    while let Some(window) = current {
        if Some(&window) == toplevel_window.as_ref() {
            break;
        }
        window.input_shape_combine_region(region, 0, 0);
        current = window.parent();
    }
}

/// The demotion itself (free fn: also called from probe closures that cannot
/// hold `&self`). Existing pages sit at overlay indices above 0, so moving
/// the glass to 0 restores pages-above-chrome for every open surface at once.
fn demote_glass(overlay: &gtk::Overlay, glass: &Rc<RefCell<Option<gtk::Widget>>>) {
    let glass = glass.borrow_mut().take();
    if let Some(glass) = glass {
        clear_glass_input_shape(&glass);
        if glass.parent().is_some() {
            overlay.reorder_overlay(&glass, 0);
        }
    }
}

/// Self-probe stage 2: any NATIVE GdkWindow inside a webview's window
/// subtree means the engine is not compositing in-widget on this stack (a
/// native child window on Wayland is a subsurface — it draws above the
/// toplevel regardless of GTK z-order), so under-glass stacking cannot be
/// trusted.
fn window_subtree_has_native(window: &gdk::Window) -> bool {
    if window.has_native() {
        return true;
    }
    window.children().iter().any(window_subtree_has_native)
}

/// Diagnostic (env `YGGTERM_WEB_SURFACE_DEBUG_TREE=1`): dump the overlay's
/// widget children (order = paint order) and the toplevel GdkWindow subtree
/// (order = stacking truth) to the log. The instrument that told us WHY an
/// under-glass hole showed the compositor instead of the page.
fn debug_dump_overlay_tree(overlay: &gtk::Overlay, label: &str) {
    if std::env::var("YGGTERM_WEB_SURFACE_DEBUG_TREE").map(|v| v == "1") != Ok(true) {
        return;
    }
    use gtk::glib::prelude::ObjectExt as _;
    let mut lines = Vec::new();
    for (index, child) in overlay.children().iter().enumerate() {
        let alloc = child.allocation();
        lines.push(format!(
            "widget[{index}] {} visible={} mapped={} alloc=({},{} {}x{}) window={} app_paintable={}",
            child.type_().name(),
            child.is_visible(),
            child.is_mapped(),
            alloc.x(),
            alloc.y(),
            alloc.width(),
            alloc.height(),
            child.window().is_some(),
            child.is_app_paintable(),
        ));
    }
    fn walk(window: &gdk::Window, depth: usize, lines: &mut Vec<String>) {
        let (x, y) = window.position();
        let describe = |region: Option<cairo::Region>, label: &str| -> String {
            match region {
                Some(region) => {
                    let first = (region.num_rectangles() > 0)
                        .then(|| region.rectangle(0))
                        .map(|r| format!("({},{} {}x{})", r.x(), r.y(), r.width(), r.height()))
                        .unwrap_or_default();
                    format!(
                        "{label}[n={} empty={} {first}]",
                        region.num_rectangles(),
                        region.is_empty()
                    )
                }
                None => format!("{label}[None]"),
            }
        };
        let clip = describe(window.clip_region(), "clip");
        let visible = describe(window.visible_region(), "vis");
        lines.push(format!(
            "{}gdkwin type={:?} pos=({x},{y}) size={}x{} visible={} native={} {clip} {visible}",
            "  ".repeat(depth),
            window.window_type(),
            window.width(),
            window.height(),
            window.is_visible(),
            window.has_native(),
        ));
        for child in window.children() {
            walk(&child, depth + 1, lines);
        }
    }
    if let Some(toplevel) = overlay.toplevel().and_then(|w| w.window()) {
        walk(&toplevel, 0, &mut lines);
    }
    tracing::info!("web surface debug tree [{label}]:\n{}", lines.join("\n"));
}

/// Reset the glass subtree's input shape to "everything" (used on demotion).
fn clear_glass_input_shape(glass: &gtk::Widget) {
    let alloc = glass.allocation();
    let full = cairo::RectangleInt::new(0, 0, alloc.width().max(1), alloc.height().max(1));
    let region = cairo::Region::create_rectangle(&full);
    apply_input_region_up_to_toplevel(glass, &region);
}

/// Paint the theme backdrop under a surface's webview (see `backdrop_rgb`):
/// a normal `connect_draw` handler runs BEFORE the class closure that draws
/// the children, so the fill lands beneath the webview, never over it.
/// GtkFixed renders no CSS background of its own, hence cairo. `None` (the
/// legacy default — only the under-glass reconcile path sets a color) draws
/// nothing at all.
fn install_container_fill(container: &gtk::Fixed, backdrop_rgb: &Rc<Cell<Option<(u8, u8, u8)>>>) {
    let backdrop_rgb = backdrop_rgb.clone();
    container.connect_draw(move |widget, cr| {
        if let Some((r, g, b)) = backdrop_rgb.get() {
            let alloc = widget.allocation();
            cr.set_source_rgb(
                r as f64 / 255.0,
                g as f64 / 255.0,
                b as f64 / 255.0,
            );
            cr.rectangle(0.0, 0.0, alloc.width() as f64, alloc.height() as f64);
            let _ = cr.fill();
        }
        gtk::glib::Propagation::Proceed
    });
}

/// One userscript and the three facts that decide WHERE it runs.
///
/// The engine half of the scriptlet plane. The app's host parses the
/// Greasemonkey metadata block and ships these decisions already made; this
/// struct is what arrives, and nothing below it re-derives anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SurfaceUserscript {
    /// The script source, injected at document-start.
    pub body: String,
    /// URL match patterns in WebKit's own syntax, passed through VERBATIM —
    /// the ENGINE does the matching, so nothing here can disagree with it about
    /// what a pattern means. EMPTY = every URL.
    pub matches: Vec<String>,
    /// Inject into sub-frames as well as the top frame.
    pub all_frames: bool,
    /// Run in a private JavaScript world (same DOM, private globals) rather than
    /// the page's own. A script that patches an API the PAGE then calls —
    /// `window.fetch`, `navigator.credentials` — must NOT set this, because a
    /// patch made in an isolated world is invisible from the page.
    pub isolated_world: bool,
}

/// The one isolated world every isolated userscript on a surface shares.
///
/// Shared rather than per-script on purpose. The boundary that matters is
/// PAGE vs SCRIPTS: the page is untrusted and must not be able to read or
/// overwrite a userscript, while the scripts themselves all came out of the same
/// directory on the host the app runs on and are as trusted as each other.
/// Giving each its own world would also cost a world per script per frame for a
/// separation nobody asked for.
const USERSCRIPT_WORLD: &str = "yggterm-userscripts";

/// Stage `scripts` on a webview builder, each with its own patterns, frames and
/// world.
///
/// ONE owner: both the page surface and the popup it opens go through here, so
/// a popup can never end up with a different placement rule than the page that
/// spawned it. (The popup path re-attaches policy because a fresh view gets a
/// fresh user-content manager — a popup with no passkey shim is precisely the
/// window a passkey is needed in.)
fn attach_userscripts<'a>(
    mut builder: WebViewBuilder<'a>,
    scripts: &[SurfaceUserscript],
) -> WebViewBuilder<'a> {
    for script in scripts {
        builder = builder.with_initialization_script_options(
            script.body.as_str(),
            !script.all_frames,
            script.matches.clone(),
            script
                .isolated_world
                .then(|| USERSCRIPT_WORLD.to_string()),
        );
    }
    builder
}

fn apply_bounds(surface: &Surface, x: i32, y: i32, w: i32, h: i32) {
    use wry::WebViewExtUnix as _;
    let (w, h) = (w.max(1), h.max(1));
    surface.container.set_margin_start(x.max(0));
    surface.container.set_margin_top(y.max(0));
    surface.container.set_size_request(w, h);
    surface.webview.webview().set_size_request(w, h);
    let _ = surface.webview.set_bounds(rect_logical(w, h));
}

/// Build the webview for a popup: RELATED to its opener, parented into its own
/// overlay child, and registered in `surfaces` under `popup_id`.
///
/// Related is the load-bearing word. `webkit_web_view_new_with_related_view`
/// puts the new view in the opener's web process and context, which is what
/// makes `window.opener` a live handle rather than `null`. Everything a popup
/// needs to be a real browser window follows from that: the same cookie jar (so
/// a sign-in it completes is a sign-in the opener has), the same proxy (so a
/// remote session's egress rule still holds), and a channel home.
///
/// The page policy (userscripts, the passkey shim, the ad filter) is re-attached
/// here, because a fresh view gets a fresh user-content manager. A popup with no
/// passkey shim is precisely the window a passkey is needed in.
#[allow(clippy::too_many_arguments)]
fn build_popup_webview(
    overlay: &gtk::Overlay,
    glass: &Rc<RefCell<Option<gtk::Widget>>>,
    surfaces: &Rc<RefCell<HashMap<u64, Surface>>>,
    close_requests: &Rc<RefCell<Vec<SurfaceCloseRequest>>>,
    edge_motion: &Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    backdrop_rgb: &Rc<Cell<Option<(u8, u8, u8)>>>,
    popup_id: u64,
    opener: &webkit2gtk::WebView,
    opener_bounds: (i32, i32, i32, i32),
    visible: bool,
    userscripts: &[SurfaceUserscript],
    adblock_ruleset: Option<&std::path::Path>,
) -> Option<webkit2gtk::WebView> {
    use webkit2gtk::WebViewExt as _;
    use wry::WebViewBuilderExtUnix as _;
    use wry::WebViewExtUnix as _;

    let (x, y, w, h) = opener_bounds;
    let container = gtk::Fixed::new();
    container.set_halign(gtk::Align::Start);
    container.set_valign(gtk::Align::Start);
    container.set_margin_start(x.max(0));
    container.set_margin_top(y.max(0));
    container.set_size_request(w.max(1), h.max(1));
    install_container_fill(&container, backdrop_rgb);
    overlay.add_overlay(&container);
    restack_glass(overlay, glass);
    container.show();

    let mut builder = WebViewBuilder::new()
        .with_bounds(rect_logical(w, h))
        // Same rule as `WebSurfaceHost::open`: never let wry's `focused: true`
        // default grab the toplevel's keyboard focus for a popup nobody is
        // looking at. A popup on a stashed surface is exactly as invisible as
        // its opener.
        .with_focused(visible)
        .with_devtools(true)
        // NO url: WebKit loads the request that asked for this window into the
        // view we hand back. Loading it ourselves would race that navigation.
        .with_related_view(opener.clone())
        .with_initialization_script_for_main_only(CLOSE_SHIM_JS, true);
    builder = attach_userscripts(builder, userscripts);
    // The custom `yggterm-appctl://` scheme is registered on the WEB CONTEXT,
    // which a related view shares — so the popup can reach the app's control
    // endpoint (the passkey signer) without re-registering anything.
    let webview = match builder.build_gtk(&container) {
        Ok(webview) => webview,
        Err(error) => {
            tracing::warn!(?error, "web surface: popup webview build failed");
            overlay.remove(&container);
            return None;
        }
    };
    if adblock_ruleset.is_some() {
        adblock::attach(&webview);
    }
    // A popup replaces the page in the same rect: it needs the same top-edge
    // reveal forward as the page it covers.
    connect_edge_motion_observer(&webview.webview(), &container, glass, edge_motion);
    // Gate 9: notice when the HUMAN takes this popup, so a queued agent batch
    // stops instead of landing behind them.
    connect_seat_input_observer(&webview.webview(), popup_id);
    // Same dialog guard as a page surface: a popup is just as invisible when the
    // surface it covers is stashed.
    connect_script_dialog_guard(&webview.webview(), popup_id);
    // `window.close()`: the page's own report (the engine will not tell us), plus
    // the native signal in case it ever does. A script-opened window may close
    // itself, and the tab it became must go with it.
    attach_surface_message_channel(&webview, popup_id, close_requests);
    let webkit = webview.webview();
    {
        let close_requests = close_requests.clone();
        webkit.connect_close(move |view| {
            close_requests.borrow_mut().push(SurfaceCloseRequest {
                surface_id: popup_id,
                href: view.uri().map(|uri| uri.to_string()).unwrap_or_default(),
                // The engine only ever emits this for a window a script opened.
                script_opened: true,
            });
        });
    }
    if visible {
        webkit.show_all();
    } else {
        container.hide();
    }
    surfaces.borrow_mut().insert(
        popup_id,
        Surface {
            container,
            webview,
            _ctx: None,
            // A popup opened by an unrevealed opener is hidden by hiding its
            // CONTAINER (see the branch above), which is the hard-stash shape,
            // not ours: page visibility is already correct for it (an unmapped
            // view reports hidden), and the injection wake must keep refusing,
            // because showing the inner view of a hidden container would not
            // map it. So this is false even when the popup is invisible.
            engine_hidden: Cell::new(false),
            wake_token: Cell::new(0),
        },
    );
    Some(webkit)
}

/// WHY an eval failed, as distinct from WHAT the engine said about it.
///
/// The engine funnels everything through one `GError`, and the shell used to
/// stringify it — so `"js: Unsupported result type"` was emitted BOTH for a
/// script that returned a Promise or a DOM object AND for a webview whose
/// content process was gone. Two completely different problems, one string, and
/// the last field run spent ten minutes on the wrong one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalFailureKind {
    /// `WEBKIT_JAVASCRIPT_ERROR_INVALID_RESULT` — the script RAN and returned
    /// something that cannot cross the bridge (a Promise, a DOM node, a
    /// function). The page is healthy; the script is wrong.
    UnsupportedResultType,
    /// The script threw.
    ScriptException,
    /// The engine rejected the call itself.
    InvalidParameter,
    /// Anything outside the JavaScript error quark — the engine, not the
    /// script.
    EngineError,
}

/// An eval failure with its classification kept alongside the engine's own
/// message, so a caller never has to pattern-match on English.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalFailure {
    /// What class of failure this is.
    pub kind: EvalFailureKind,
    /// The engine's own message, preserved verbatim.
    pub message: String,
}

impl EvalFailure {
    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    fn classify(error: &gtk::glib::Error) -> Self {
        use webkit2gtk::JavascriptError;
        let kind = if error.matches(JavascriptError::InvalidResult) {
            EvalFailureKind::UnsupportedResultType
        } else if error.matches(JavascriptError::ScriptFailed) {
            EvalFailureKind::ScriptException
        } else if error.matches(JavascriptError::InvalidParameter) {
            EvalFailureKind::InvalidParameter
        } else {
            EvalFailureKind::EngineError
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

/// Three separate facts about a surface, deliberately not collapsed into a
/// single "alive" boolean — see [`WebSurfaceHost::surface_liveness`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceLiveness {
    /// The host still holds an entry for this surface id.
    pub present: bool,
    /// The engine webview widget is realized and mapped.
    pub mapped: bool,
    /// The engine believes its web content process is answering.
    pub web_process_responsive: bool,
}

/// One cookie as the ENGINE layer knows it.
///
/// Deliberately plain and file-format-agnostic: the vendored engine layer must
/// not learn what a Netscape jar looks like, and the jar codec
/// (`yggterm-shell::netscape_cookie_jar`) must not learn about libsoup. This
/// struct is the seam between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieRecord {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// A leading `.` means "and subdomains", matching both libsoup and the jar
    /// file — one encoding of that fact, all the way through.
    pub domain: String,
    /// Path scope.
    pub path: String,
    /// `None` is a session cookie.
    pub expires_unix: Option<i64>,
    /// Https-only.
    pub secure: bool,
    /// Not readable from `document.cookie`.
    pub http_only: bool,
}

impl CookieRecord {
    /// Identity for de-duplication: the tuple a browser treats as ONE cookie.
    /// The value is deliberately not part of it — the same cookie returned by
    /// the http and https queries is one cookie, not two.
    fn same_cookie(&self, other: &Self) -> bool {
        self.name == other.name && self.domain == other.domain && self.path == other.path
    }

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    fn from_soup(cookie: &mut soup::Cookie) -> Self {
        Self {
            name: cookie.name().map(|v| v.to_string()).unwrap_or_default(),
            value: cookie.value().map(|v| v.to_string()).unwrap_or_default(),
            domain: cookie.domain().map(|v| v.to_string()).unwrap_or_default(),
            path: cookie.path().map(|v| v.to_string()).unwrap_or_default(),
            expires_unix: cookie.expires().map(|when| when.to_unix()),
            secure: cookie.is_secure(),
            http_only: cookie.is_http_only(),
        }
    }

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    fn to_soup(&self) -> soup::Cookie {
        // max_age -1 = a session cookie; an absolute expiry is set explicitly
        // below, so there is one place that decides a cookie's lifetime.
        let mut cookie = soup::Cookie::new(&self.name, &self.value, &self.domain, &self.path, -1);
        cookie.set_secure(self.secure);
        cookie.set_http_only(self.http_only);
        if let Some(expires) = self.expires_unix {
            if let Ok(when) = gtk::glib::DateTime::from_unix_utc(expires) {
                cookie.set_expires(&when);
            }
        }
        cookie
    }
}

impl WebSurfaceHost {
    pub(crate) fn new(overlay: gtk::Overlay, backdrop: gtk::Widget) -> Self {
        let backdrop_css = gtk::CssProvider::new();
        backdrop.style_context().add_provider(
            &backdrop_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        Self {
            overlay,
            backdrop_css,
            backdrop_rgb: Rc::new(Cell::new(None)),
            glass: Rc::new(RefCell::new(None)),
            last_glass_holes: RefCell::new(None),
            surfaces: Rc::new(RefCell::new(HashMap::new())),
            contexts: Rc::new(RefCell::new(HashMap::new())),
            next_id: Rc::new(Cell::new(1)),
            popups: Rc::new(RefCell::new(Vec::new())),
            close_requests: Rc::new(RefCell::new(Vec::new())),
            downloads: Rc::new(RefCell::new(Vec::new())),
            downloads_in_flight: Rc::new(RefCell::new(Vec::new())),
            next_transfer_id: Rc::new(Cell::new(1)),
            edge_motion: Rc::new(RefCell::new(None)),
        }
    }

    /// Install the edge-motion forward target (the shell webview's reveal
    /// hook). Set once at host construction, before any surface opens.
    pub(crate) fn set_edge_motion_notifier(&self, notify: impl Fn() + 'static) {
        *self.edge_motion.borrow_mut() = Some(Rc::new(notify));
    }

    /// Paint the native backdrop (the overlay's base child) in the app's
    /// theme background color. Under-glass pages sit above it, so it shows
    /// only where a page hasn't painted yet — turning the first-paint flash
    /// theme-colored instead of white.
    pub fn set_backdrop_color(&self, r: u8, g: u8, b: u8) {
        let css = format!("box {{ background-color: rgb({r},{g},{b}); }}");
        if let Err(error) = self.backdrop_css.load_from_data(css.as_bytes()) {
            tracing::warn!(?error, "web surface: backdrop css failed to load");
        }
        // Container draw-fill twin (see `backdrop_rgb`): repaint live
        // containers so a theme change lands without waiting for damage.
        self.backdrop_rgb.set(Some((r, g, b)));
        for surface in self.surfaces.borrow().values() {
            surface.container.queue_draw();
        }
    }

    /// Arm under-glass stacking: remember the shell webview's container and
    /// restack it to the TOP of the overlay. From here on, every surface
    /// attach point restacks below it (the shell-topmost invariant — three
    /// writers: `open`, `unstash`, popup-create).
    pub(crate) fn install_glass(&self, glass: gtk::Widget) {
        *self.glass.borrow_mut() = Some(glass);
        restack_glass(&self.overlay, &self.glass);
    }

    /// Whether under-glass stacking is active (pages below the shell).
    pub fn under_glass(&self) -> bool {
        self.glass.borrow().is_some()
    }

    /// Apply the glass input region: full window minus `holes` plus `covers`
    /// (all logical px, glass-local coords — the same coords the reconciler
    /// samples off `[data-ws-page]`). Empty holes ⇒ the FULL region is
    /// applied, i.e. the shape is effectively removed — the safety invariant:
    /// any doubt resolves to "chrome owns all input, pages temporarily
    /// mouse-unreachable", never a dead zone in the chrome.
    ///
    /// Shapes the glass's GdkWindow and every ancestor up to but EXCLUDING
    /// the toplevel: GtkOverlay wraps each overlay child in an intermediate
    /// GdkWindow with an empty event mask; left unshaped it still picks, and
    /// GDK then bubbles unhandled events to the TOPLEVEL (an ancestor), never
    /// the page (a sibling below). Never shape the toplevel itself — on X11
    /// its parent is the root window and a shaped toplevel drops clicks
    /// through the whole application. (Both spike-caught.)
    pub fn set_glass_input_holes(
        &self,
        holes: &[(i32, i32, i32, i32)],
        covers: &[(i32, i32, i32, i32)],
    ) {
        let glass = self.glass.borrow();
        let Some(glass) = glass.as_ref() else {
            return;
        };
        let key = (holes.to_vec(), covers.to_vec());
        if self.last_glass_holes.borrow().as_ref() == Some(&key) {
            return;
        }
        let alloc = self.overlay.allocation();
        let region = glass_input_region(
            (alloc.width().max(1), alloc.height().max(1)),
            holes,
            covers,
        );
        apply_input_region_up_to_toplevel(glass, &region);
        *self.last_glass_holes.borrow_mut() = Some(key);
    }

    /// F.1 synchronous cover push: cover rects arrive OUT OF TICK from the
    /// shell's MutationObserver the instant chrome mounts/unmounts/resizes
    /// over a page — the tick's own covers sample remains as idempotent
    /// self-heal. Holes stay whatever the reconciler last applied: two
    /// cadences, one applier, one change gate.
    pub fn set_glass_covers(&self, covers: &[(i32, i32, i32, i32)]) {
        let holes = self
            .last_glass_holes
            .borrow()
            .as_ref()
            .map(|(holes, _)| holes.clone())
            .unwrap_or_default();
        self.set_glass_input_holes(&holes, covers);
    }

    /// The ONE allocator of native surface ids. The shell asks for one before
    /// `open`; the create handler takes one for a popup it builds itself.
    pub fn allocate_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    /// Drain the popups pages opened since the last call. Their webviews are
    /// already live — see `WebSurfaceHost::popups`.
    pub fn take_popups(&self) -> Vec<SurfacePopup> {
        std::mem::take(&mut self.popups.borrow_mut())
    }

    /// Drain the pages that called `window.close()`.
    pub fn take_close_requests(&self) -> Vec<SurfaceCloseRequest> {
        std::mem::take(&mut self.close_requests.borrow_mut())
    }

    /// Drain the download transitions since the last call. The shell turns each
    /// into a toast and a trace row; this host keeps no history, because a
    /// second record of "what downloaded" would be a second thing to keep true.
    pub fn take_downloads(&self) -> Vec<SurfaceDownloadEvent> {
        std::mem::take(&mut self.downloads.borrow_mut())
    }

    /// How many transfers are running right now. The instrument for the
    /// detach rule: a surface can close while this is non-zero, and the file
    /// must still complete.
    pub fn downloads_in_flight(&self) -> usize {
        self.downloads_in_flight.borrow().len()
    }


    /// Open (or replace) surface `id` at the given page-relative bounds, loading
    /// `url`. If `socks_port` is set the surface egresses through
    /// `socks5://127.0.0.1:<port>` (the invoking host's tunnel) — the egress
    /// rule. `profile_dir` is the surface's persistent storage jar (cookies/
    /// localStorage); `None` = ephemeral. `userscripts` are injected into the
    /// TOP frame at document-start on every page this surface loads (the
    /// userscript/content-policy substrate: SponsorBlock-class scripts,
    /// cosmetic filters, autofill). `adblock_ruleset` = path to a WebKit
    /// content-blocker JSON; when set, the compiled filter (network blocks +
    /// cosmetic hiding, engine-native) is attached to this surface. `user_agent`
    /// overrides WebKitGTK's default UA, whose "Safari on X11/Linux" shape names
    /// a browser that does not exist and is 403'd outright by UA-allowlisting
    /// edges (claude.ai answers it "Request not allowed"); `None` keeps the
    /// engine default. Bounds are logical pixels relative to the window's
    /// top-left.
    ///
    /// `visible` is "is this surface being REVEALED to someone right now", and
    /// it decides two things at birth: whether the view may take the toplevel's
    /// keyboard focus, and whether the ENGINE is told the page is on screen. A
    /// surface created for a session nobody is looking at is born hidden — same
    /// rule, same shape as `build_popup_webview` — so its
    /// `document.visibilityState` reads `hidden` from the first frame and its
    /// `requestAnimationFrame` never starts. Creating it visible and hiding it a
    /// tick later would be a lie the page has already acted on: a spinner on a
    /// never-revealed surface measured 0.85 cores that way.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        id: u64,
        url: &str,
        socks_port: Option<u16>,
        profile_dir: Option<&std::path::Path>,
        userscripts: &[SurfaceUserscript],
        adblock_ruleset: Option<&std::path::Path>,
        user_agent: Option<&str>,
        signer_base: Option<&str>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        visible: bool,
    ) -> Result<(), String> {
        // Replace any existing surface with this id.
        self.close(id);

        let container = gtk::Fixed::new();
        container.set_halign(gtk::Align::Start);
        container.set_valign(gtk::Align::Start);
        container.set_margin_start(x.max(0));
        container.set_margin_top(y.max(0));
        container.set_size_request(w.max(1), h.max(1));
        install_container_fill(&container, &self.backdrop_rgb);
        self.overlay.add_overlay(&container);
        restack_glass(&self.overlay, &self.glass);
        container.show();

        // Persistent per-profile storage when a jar is given; ephemeral
        // otherwise. Recreating a surface with the SAME profile_dir reuses the
        // on-disk cookies/localStorage, so destroy+recreate (reload, proxy or
        // profile change) is lossless. `None` MUST be the engine's true
        // ephemeral mode — `WebContext::new(None)` is NOT that (it silently
        // shares WebKit's default on-disk jar), which would leak temp-profile
        // browsing onto disk.
        //
        // SHARED across surfaces that agree on jar + egress + control endpoint
        // (`web_context_key`). A context created here is the FIRST surface on
        // that key; `context_is_new` is false for every sibling that follows,
        // which matters because the custom scheme below may only be registered
        // once per context.
        let ctx_key = web_context_key(profile_dir, socks_port, signer_base);
        let (ctx_cell, context_is_new) = match ctx_key.as_ref() {
            Some(key) => {
                let mut contexts = self.contexts.borrow_mut();
                match contexts.get(key) {
                    Some(existing) => (existing.clone(), false),
                    None => {
                        let dir = profile_dir.expect("a keyed context always has a profile dir");
                        let created =
                            Rc::new(RefCell::new(WebContext::new(Some(dir.to_path_buf()))));
                        contexts.insert(key.clone(), created.clone());
                        (created, true)
                    }
                }
            }
            None => (Rc::new(RefCell::new(WebContext::new_ephemeral())), true),
        };
        let mut ctx = ctx_cell.borrow_mut();
        // Devtools are always available on surfaces: the agent is a first-class
        // user and drives pages through the inspector/eval; the user opens it
        // per surface. (WebKitGTK: enables developer extras; the inspector
        // itself only appears via `set_devtools_open`.)
        let mut builder = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_bounds(rect_logical(w, h))
            // wry's DEFAULT is `focused: true`, and on GTK that means
            // `grab_focus()` the instant the webview is built — which sets the
            // TOPLEVEL's focus widget. A headless surface (`web ensure`, created
            // and demoted in the same tick, never revealed) took the keyboard
            // focus off the user's terminal at birth and kept it: the user's
            // "the shadow session spawn took focus away from my viewport",
            // 2026-07-26. A surface only gets the focus when it is being shown
            // TO SOMEONE.
            .with_focused(visible)
            .with_devtools(true)
            // Every surface reports `window.close()`. A normal tab's request is
            // REFUSED by the shell (Chrome's rule), but the shell can only refuse
            // what it hears — and it must hear it from the page, because the
            // engine never says a word.
            .with_initialization_script_for_main_only(CLOSE_SHIM_JS, true)
            .with_url(url);
        if let Some(port) = socks_port {
            builder = builder.with_proxy_config(ProxyConfig::Socks5(ProxyEndpoint {
                host: "127.0.0.1".to_string(),
                port: port.to_string(),
            }));
        }
        builder = attach_userscripts(builder, userscripts);
        if let Some(user_agent) = user_agent.filter(|value| !value.trim().is_empty()) {
            builder = builder.with_user_agent(user_agent);
        }

        // In-page "new window" requests (a link middle-clicked, ctrl-clicked,
        // `target="_blank"`, or `window.open`) become TABS of this surface's
        // session rather than detached GTK windows — but the webview is built
        // HERE, related to this one, and handed straight back to WebKit.
        //
        // This used to deny the window and let the shell reopen the URL in a
        // fresh webview. That produced a tab, but not a POPUP: with no relation
        // to the opener, `window.opener` was `null` and `window.close()` had
        // nothing to close. Every popup-based sign-in (claude.ai -> Google) hung
        // there: the user authenticated, the callback tried to hand the result
        // back through `opener.postMessage(...)`, hit `null`, and the page that
        // started the flow waited forever while the "successful" popup refused
        // to go away. (The cookie landed, so the NEXT launch was silently signed
        // in — which is how a broken channel disguised itself as a flaky login.)
        {
            let popups = self.popups.clone();
            let surfaces = self.surfaces.clone();
            let close_requests = self.close_requests.clone();
            let overlay = self.overlay.clone();
            let glass = self.glass.clone();
            let edge_motion = self.edge_motion.clone();
            let backdrop_rgb = self.backdrop_rgb.clone();
            let ids = self.next_id.clone();
            let popup_scripts = userscripts.to_vec();
            let popup_adblock = adblock_ruleset.map(|path| path.to_path_buf());
            let surface_id = id;
            builder = builder.with_new_window_req_handler(move |url, features| {
                let popup_id = {
                    let next = ids.get();
                    ids.set(next + 1);
                    next
                };
                let bounds = surfaces
                    .borrow()
                    .get(&surface_id)
                    .map(|surface| {
                        let (w, h) = surface.container.size_request();
                        (
                            surface.container.margin_start(),
                            surface.container.margin_top(),
                            w,
                            h,
                        )
                    })
                    .unwrap_or((0, 0, 1, 1));
                match build_popup_webview(
                    &overlay,
                    &glass,
                    &surfaces,
                    &close_requests,
                    &edge_motion,
                    &backdrop_rgb,
                    popup_id,
                    &features.opener.webview,
                    bounds,
                    !features.background,
                    &popup_scripts,
                    popup_adblock.as_deref(),
                ) {
                    Some(webview) => {
                        popups.borrow_mut().push(SurfacePopup {
                            opener_id: surface_id,
                            popup_id,
                            url,
                            background: features.background,
                        });
                        wry::NewWindowResponse::Create { webview }
                    }
                    // Refusing is the honest failure: a detached GTK window would
                    // escape the viewport entirely, and a tab with no view is a
                    // row that does nothing.
                    None => wry::NewWindowResponse::Deny,
                }
            });
        }

        // App-control bridge from inside a surface. WebKitGTK blocks an https
        // page from `fetch`-ing `http://127.0.0.1` (mixed content), so an app's
        // in-page shim (e.g. the passkey `navigator.credentials` polyfill) cannot
        // reach its own control endpoint directly. This registers a SECURE custom
        // scheme `yggterm-appctl://` that the GUI proxies to the app's
        // GUI-reachable control endpoint (already `ssh -L`-resolved for a remote
        // app). Async: a `/fido2/get` blocks up to two minutes for the presence
        // dialog, so the forward runs off the GTK main thread — a blocking handler
        // would freeze the very dialog it is waiting on.
        //
        // Registered ONCE per context. wry refuses a second registration of the
        // same scheme on a context (`DuplicateCustomProtocol`), and the refusal
        // is stored as the builder's error — so a sibling tab re-registering
        // would fail to build at all. It is also unnecessary: the key includes
        // `signer_base`, so every surface sharing this context proxies to the
        // same control endpoint the first one registered.
        if let Some(base) = signer_base.filter(|_| context_is_new) {
            let base = base.trim_end_matches('/').to_string();
            builder = builder.with_asynchronous_custom_protocol(
                APP_CONTROL_SCHEME.to_string(),
                move |_webview_id, request, responder| {
                    app_control_proxy(base.clone(), request, responder);
                },
            );
        }

        let webview = {
            use wry::WebViewBuilderExtUnix;
            match builder.build_gtk(&container) {
                Ok(webview) => webview,
                Err(e) => {
                    // A context created for a surface that never built has no
                    // owner but the map; sweep it rather than leave a pool
                    // behind for a jar nobody opened.
                    drop(ctx);
                    self.prune_contexts();
                    return Err(format!("build surface webview: {e}"));
                }
            }
        };
        // The builder's borrow of the shared context ends here; everything below
        // touches the webview, and a sibling `open` needs the cell free.
        drop(ctx);
        // Downloads, wired to the ENGINE this surface runs on. Once per context
        // — `download-started` is a context signal and the tabs of one session
        // share a context, so connecting per surface would decide one transfer
        // once per tab.
        //
        // This call is the ONLY download policy in the process: the vendored
        // wry's default `download_started_handler` is switched off precisely so
        // that is true (before that it answered `decide-destination` itself,
        // saving to `dirs::download_dir()` — or, unset, the GUI's CWD — under
        // the server's raw suggested name, and telling nobody). So without this
        // call nothing answers, and a download link does nothing at all.
        if context_is_new {
            use webkit2gtk::WebViewExt as _;
            use wry::WebViewExtUnix as _;
            if let Some(context) = webview.webview().context() {
                connect_download_plumbing(
                    &context,
                    &ctx_cell,
                    &self.surfaces,
                    &self.downloads,
                    &self.downloads_in_flight,
                    &self.next_transfer_id,
                );
            }
        }
        container.show_all();
        // ...but only a surface being REVEALED is shown to the ENGINE. Hiding
        // the inner view unmaps it, which is how WebKitGTK derives page
        // visibility (there is no page-visibility setter on this API), so an
        // unrevealed surface's page reads `hidden` and throttles itself from its
        // first frame. The container stays shown-and-attached — it is what
        // `demote` reorders and what an instant raise-reveal needs — and it
        // paints only `install_container_fill`'s flat backdrop, below the glass.
        if !visible {
            let _ = webview.set_visible(false);
        }

        if let Some(ruleset) = adblock_ruleset {
            let store_dir = ruleset
                .parent()
                .map(|dir| dir.join("compiled"))
                .unwrap_or_else(|| std::path::PathBuf::from("compiled"));
            adblock::ensure_compiled(ruleset, &store_dir);
            adblock::attach(&webview);
        }

        {
            let overlay = self.overlay.clone();
            gtk::glib::timeout_add_seconds_local(2, move || {
                debug_dump_overlay_tree(&overlay, "post-open");
                gtk::glib::ControlFlow::Break
            });
        }
        // Self-probe stage 2, per surface: on Wayland, a native GdkWindow in
        // the page webview's subtree is a subsurface — it draws above the
        // toplevel regardless of GTK z-order, so under-glass stacking cannot
        // be trusted and the host demotes itself to legacy. X11 native child
        // windows honor restacking (spike-proven) and are not disqualifying.
        // Deferred 1s: WebKit realizes its windows lazily.
        if self.under_glass() {
            let is_wayland = gdk::Display::default()
                .map(|display| {
                    use gtk::glib::prelude::ObjectExt as _;
                    display.type_().name() == "GdkWaylandDisplay"
                })
                .unwrap_or(false);
            if is_wayland {
                let overlay = self.overlay.clone();
                let glass = self.glass.clone();
                let webkit = {
                    use wry::WebViewExtUnix as _;
                    webview.webview()
                };
                gtk::glib::timeout_add_seconds_local(1, move || {
                    if glass.borrow().is_some() {
                        if let Some(window) = webkit.window() {
                            if window_subtree_has_native(&window) {
                                tracing::warn!(
                                    "web surface: native child window detected on Wayland — \
                                     demoting to legacy stacking"
                                );
                                demote_glass(&overlay, &glass);
                            }
                        }
                    }
                    gtk::glib::ControlFlow::Break
                });
            }
        }
        // F.1 reveal trigger: forward top-edge motion over this page to the
        // shell's titlebar reveal (see `connect_edge_motion_observer`).
        {
            use wry::WebViewExtUnix as _;
            connect_edge_motion_observer(
                &webview.webview(),
                &container,
                &self.glass,
                &self.edge_motion,
            );
            // Gate 9: real seat input on this surface preempts any agent batch
            // driving it. Attached here, next to the webview, because only this
            // layer can tell the agent's own injection from the human (the
            // injected events are deliberately indistinguishable to the page).
            connect_seat_input_observer(&webview.webview(), id);
            // A page dialog must never be able to wedge this surface (and every
            // sibling on its profile) just because nothing is on screen to
            // answer it.
            connect_script_dialog_guard(&webview.webview(), id);
        }
        // `window.close()`: the page's report (the engine will not tell us) plus
        // the native signal in case it ever does. What the shell DOES with it is
        // the shell's call — a normal tab may not close itself.
        attach_surface_message_channel(&webview, id, &self.close_requests);
        {
            use webkit2gtk::WebViewExt as _;
            use wry::WebViewExtUnix as _;
            let close_requests = self.close_requests.clone();
            webview.webview().connect_close(move |view| {
                close_requests.borrow_mut().push(SurfaceCloseRequest {
                    surface_id: id,
                    href: view.uri().map(|uri| uri.to_string()).unwrap_or_default(),
                    script_opened: true,
                });
            });
        }
        self.surfaces.borrow_mut().insert(
            id,
            Surface {
                container,
                webview,
                _ctx: Some(ctx_cell),
                // Born hidden when nobody is being shown it, and recorded as
                // OURS so the agent drive path can wake it for a burst.
                engine_hidden: Cell::new(!visible),
                wake_token: Cell::new(0),
            },
        );
        Ok(())
    }

    /// Drop every shared `WebContext` no surface holds any more.
    ///
    /// The map's own entry is a strong ref, so "unused" is `strong_count == 1`.
    /// This preserves the pre-sharing lifetime exactly — a context died when its
    /// surface closed, and still dies when its LAST surface closes — while
    /// making the intermediate state (N tabs, one context) possible at all.
    ///
    /// A RUNNING DOWNLOAD is also a holder (`DownloadInFlight`), which is how
    /// "the file finishes even though the tab is gone" is spelled: the sweep
    /// finds `strong_count > 1` and leaves the engine standing until the
    /// transfer ends, then the next sweep takes it.
    ///
    /// The rule itself is `retain_held_contexts`, which is where it can be
    /// driven; this method is the host's borrow around it and nothing more.
    fn prune_contexts(&self) {
        retain_held_contexts(&mut self.contexts.borrow_mut());
    }

    /// How many distinct `WebContext`s are alive right now.
    ///
    /// The instrument for the sharing invariant: with N tabs open on one
    /// session this must read 1, not N. Read against the shell's own
    /// `web_surface_views` (the reconciler's applied map owns "how many
    /// surfaces"; this host does not publish a second count of it), the pair is
    /// the only in-process way to tell "two tabs, one process pool" from "two
    /// tabs, two of everything" without going to `/proc` and guessing from
    /// `comm`.
    pub fn web_context_count(&self) -> usize {
        self.contexts.borrow().len()
    }

    pub fn set_bounds(&self, id: u64, x: i32, y: i32, w: i32, h: i32) {
        if let Some(s) = self.surfaces.borrow().get(&id) {
            apply_bounds(s, x, y, w, h);
            self.overlay.queue_resize();
        }
    }

    /// Show or hide surface `id` — BOTH the widget and, through it, the page's
    /// own `document.visibilityState`. The two are the same fact, and this is
    /// one of the four places allowed to write it (`open`, `set_throttled` and
    /// `unstash` are the others).
    pub fn set_visible(&self, id: u64, visible: bool) {
        if let Some(s) = self.surfaces.borrow().get(&id) {
            let _ = s.webview.set_visible(visible);
            s.container.set_visible(visible);
            s.engine_hidden.set(!visible);
        }
    }

    pub fn navigate(&self, id: u64, url: &str) {
        if let Some(s) = self.surfaces.borrow().get(&id) {
            let _ = s.webview.load_url(url);
        }
    }

    pub fn reload(&self, id: u64) {
        if let Some(s) = self.surfaces.borrow().get(&id) {
            let _ = s.webview.reload();
        }
    }

    /// Set the WebKit zoom factor for surface `id` (1.0 == 100%). This is the
    /// page zoom the shell's "Web View" / "Ychrome Global" zoom control drives;
    /// a native web surface is an overlaid WebKit view, so it cannot be scaled
    /// with the DOM `zoom:` the rendered document surface uses.
    pub fn set_zoom(&self, id: u64, factor: f64) {
        if let Some(s) = self.surfaces.borrow().get(&id) {
            let _ = s.webview.zoom(factor);
        }
    }

    /// Current page (uri, title, loading) as the ENGINE reports them. In-page
    /// navigations (link clicks, redirects, pushState) never pass through the
    /// shell's nav model, so this is the only truth for "where is this tab
    /// now"; the shell polls it to keep the address bar, tab titles, history
    /// and the tab's loading light honest.
    pub fn page_state(&self, id: u64) -> Option<(String, String, bool)> {
        use webkit2gtk::WebViewExt as _;
        use wry::WebViewExtUnix as _;
        self.surfaces.borrow().get(&id).map(|s| {
            let webkit = s.webview.webview();
            (
                webkit.uri().map(|u| u.to_string()).unwrap_or_default(),
                webkit.title().map(|t| t.to_string()).unwrap_or_default(),
                webkit.is_loading(),
            )
        })
    }

    /// Is surface `id` PLAYING AUDIO right now, as the engine sees it?
    ///
    /// `webkit_web_view_is_playing_audio` is WebKit's own answer, taken from the
    /// media session inside the web process — it is true for a `<video>`, an
    /// `<audio>`, and a WebAudio graph alike, and it goes false on pause. It is
    /// the only honest source: nothing the shell can observe from outside (the
    /// URL, the title, whether the tab was ever visible) can tell a playing
    /// playlist from a parked one.
    ///
    /// The shell uses this to VETO DESTROY, never to veto throttling: an unseen
    /// page must still stop painting (that is free and is the whole point of the
    /// soft stash), but it must not be killed while the user is listening to it.
    /// A missing surface reads `false` — there is nothing left to protect.
    ///
    /// Deliberately NOT paired with `set_is_muted`. Muting a page to save CPU
    /// would be the exact failure this exists to prevent.
    pub fn is_playing_audio(&self, id: u64) -> bool {
        use webkit2gtk::WebViewExt as _;
        use wry::WebViewExtUnix as _;
        self.surfaces
            .borrow()
            .get(&id)
            .is_some_and(|s| s.webview.webview().is_playing_audio())
    }

    pub fn close(&self, id: u64) {
        if let Some(s) = self.surfaces.borrow_mut().remove(&id) {
            // A stashed surface's container is already detached.
            if s.container.parent().is_some() {
                self.overlay.remove(&s.container);
            }
            // Surface drops here: the webview is torn down, and its share of the
            // WebContext is released. The context itself survives while a
            // sibling tab still holds it.
        }
        self.prune_contexts();
        // Do not leave a seat-input tally behind for a surface that is gone —
        // ids are reused, and a stale count would preempt the next agent batch
        // on the new surface for something the user did to the old one.
        forget_seat_input(id);
    }

    /// Stash surface `id`: detach its container from the overlay WITHOUT
    /// destroying the webview. The web process (DOM, scroll, playback state)
    /// stays alive; detaching unmaps the widget, which — unlike
    /// `set_visible(false)` — makes the shared WebKitGTK compositor actually
    /// release its pixels (the stuck-composite/reload-white family). The
    /// background-hold path: unstash on return, destroy on hold expiry.
    pub fn stash(&self, id: u64) -> Result<(), String> {
        let surfaces = self.surfaces.borrow();
        let s = surfaces.get(&id).ok_or("no such surface")?;
        if s.container.parent().is_some() {
            self.overlay.remove(&s.container);
        }
        Ok(())
    }

    /// Re-attach a stashed surface at the given bounds and show it. A
    /// soft-stashed surface (under glass — never detached, see `demote`) is
    /// RAISED instead: with backgrounded pages left attached, containers
    /// overlap, and the revealed one must top the page stack (still below
    /// the glass) or a stale background page shows through the hole.
    pub fn unstash(&self, id: u64, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        let surfaces = self.surfaces.borrow();
        let s = surfaces.get(&id).ok_or("no such surface")?;
        if s.container.parent().is_none() {
            self.overlay.add_overlay(&s.container);
        } else {
            self.overlay.reorder_overlay(&s.container, -1);
        }
        restack_glass(&self.overlay, &self.glass);
        apply_bounds(s, x, y, w, h);
        let _ = s.webview.set_visible(true);
        s.container.show_all();
        // Reveal clears the engine-hidden fact (as do `set_visible(true)` and
        // `set_throttled(false)`). Leaving it set would let a re-hide armed by
        // an in-flight agent burst blank the page the user just switched to.
        s.engine_hidden.set(false);
        self.overlay.queue_resize();
        Ok(())
    }

    /// Push surface `id`'s container to the BOTTOM of the page stack (still
    /// above the overlay's base child). The under-glass soft stash: a
    /// backgrounded page stays attached and composited — the opaque glass
    /// covers it (no hole) — so switch-back needs no re-map/re-composite.
    /// Demoting it keeps every later-revealed or popup-created page above
    /// it; without this a backgrounded page (or its script popup, which
    /// attaches topmost) would occlude the active page through the hole.
    pub fn demote(&self, id: u64) -> Result<(), String> {
        let surfaces = self.surfaces.borrow();
        let s = surfaces.get(&id).ok_or("no such surface")?;
        if s.container.parent().is_some() {
            self.overlay.reorder_overlay(&s.container, 0);
        }
        Ok(())
    }

    /// Throttle a soft-stashed surface's CPU WITHOUT detaching it. Hiding the
    /// inner webview widget unmaps it, so WebKitGTK marks the page hidden
    /// (`document.visibilityState === 'hidden'`) and stops driving it at the
    /// compositor frame rate: `requestAnimationFrame` pauses and background
    /// timers throttle. That is the difference between a demoted-but-live page
    /// burning a whole core on an animation and the same page sitting idle
    /// (measured author-note: ">1 core while invisible").
    ///
    /// The CONTAINER stays attached and demoted below the glass, so this is NOT
    /// the detach stash: reveal is still a raise + `set_visible(true)` (see
    /// `unstash`), not an overlay re-add and WebKit re-composite. Page STATE
    /// (DOM, scroll, JS heap) is untouched — only rendering pauses — and
    /// explicit JS eval still runs on a hidden view, so agent read/eval/wait
    /// keep working. The stale-pixel hazard that makes a plain hidden webview
    /// unsafe over a revealed session does not apply here: the surface is
    /// demoted beneath the opaque glass, so nobody ever sees those pixels.
    ///
    /// Not conditional on a lease. A lease is an agent's claim that the surface
    /// must keep EXISTING; it is not evidence that anyone is LOOKING at it, and
    /// the leased-and-never-revealed case is exactly the one that was measured
    /// burning 0.85 cores on one spinner. Agent reach survives regardless: eval
    /// and capture run on a hidden view, and injection wakes it for the burst
    /// (`engine_webview_for_injection`).
    pub fn set_throttled(&self, id: u64, throttled: bool) -> Result<(), String> {
        let surfaces = self.surfaces.borrow();
        let s = surfaces.get(&id).ok_or("no such surface")?;
        let _ = s.webview.set_visible(!throttled);
        s.engine_hidden.set(throttled);
        Ok(())
    }

    /// What is actually true about surface `id` right now.
    ///
    /// `is_open` answers only "does the entry exist", and an entry whose web
    /// CONTENT PROCESS has died is not empty — which is how `ensure` came to
    /// hand a caller back the same corpse and report success. These are three
    /// separate facts and they must not be collapsed:
    ///
    /// - `present`: we still hold the surface entry.
    /// - `mapped`: the widget is realized and mapped (injection needs this).
    /// - `web_process_responsive`: the engine's own view of its content
    ///   process.
    ///
    /// All three are UI-PROCESS properties, so all three can read healthy over
    /// a content process that will never answer another script. That is why the
    /// shell follows this with a bounded eval round trip before believing it —
    /// the same class of mistake as trusting `page_state` to prove a page is
    /// alive.
    pub fn surface_liveness(&self, id: u64) -> SurfaceLiveness {
        use webkit2gtk::WebViewExt as _;
        use wry::WebViewExtUnix as _;
        let surfaces = self.surfaces.borrow();
        let Some(surface) = surfaces.get(&id) else {
            return SurfaceLiveness {
                present: false,
                mapped: false,
                web_process_responsive: false,
            };
        };
        let webkit = surface.webview.webview();
        SurfaceLiveness {
            present: true,
            mapped: gtk::prelude::WidgetExt::is_mapped(&webkit),
            web_process_responsive: webkit.is_web_process_responsive(),
        }
    }

    pub fn is_open(&self, id: u64) -> bool {
        self.surfaces.borrow().contains_key(&id)
    }

    /// Evaluate JS in surface `id`'s page. The callback receives
    /// `Ok(json)` — the completion value serialized as JSON — or `Err(msg)`
    /// for a JS exception. Goes straight to the engine (wry's own eval
    /// swallows errors into an empty string, useless for automation).
    pub fn eval(
        &self,
        id: u64,
        js: &str,
        callback: impl FnOnce(Result<String, EvalFailure>) + 'static,
    ) -> Result<(), String> {
        use javascriptcore::ValueExt as _;
        use webkit2gtk::WebViewExt as _;
        let surfaces = self.surfaces.borrow();
        let surface = surfaces.get(&id).ok_or("no such surface")?;
        let webkit = {
            use wry::WebViewExtUnix;
            surface.webview.webview()
        };
        let cancellable: Option<&gtk::gio::Cancellable> = None;
        #[allow(deprecated)]
        webkit.run_javascript(js, cancellable, move |result| {
            let outcome = match result {
                Ok(js_result) => Ok(js_result
                    .js_value()
                    .and_then(|value| value.to_json(0))
                    .map(|json| json.to_string())
                    .unwrap_or_default()),
                Err(error) => Err(EvalFailure::classify(&error)),
            };
            callback(outcome);
        });
        Ok(())
    }

    /// Open/close the WebKit inspector (devtools) for surface `id`. Returns
    /// whether devtools are open after the call.
    pub fn set_devtools_open(&self, id: u64, open: bool) -> Result<bool, String> {
        let surfaces = self.surfaces.borrow();
        let surface = surfaces.get(&id).ok_or("no such surface")?;
        if open {
            surface.webview.open_devtools();
        } else {
            surface.webview.close_devtools();
        }
        Ok(surface.webview.is_devtools_open())
    }

    /// Find-in-page on surface `id`, through WebKit's OWN find controller.
    ///
    /// `webkit_web_view_get_find_controller` + `search` / `search_next` /
    /// `search_previous` / `search_finish` + the `counted-matches` signal —
    /// exactly the five doors Epiphany's find bar drives, reached through the
    /// safe `webkit2gtk` binding that already ships them (`FindControllerExt`).
    /// This module drops to `webkit2gtk::ffi` only where the safe binding has no
    /// door at all (see `mod adblock`, which says so in its own header); adding
    /// a hand-rolled `extern "C"` block for symbols the binding already declares
    /// would be a SECOND declaration of the same ABI, free to drift from wry's.
    ///
    /// A JavaScript/regex re-implementation was never on the table: it would be
    /// a second encoding of "what counts as a match" that could disagree with
    /// the engine's own highlights, and it could not highlight at all.
    ///
    /// **The count is asynchronous and the callback is the only answer.**
    /// `count_matches` returns immediately and the number arrives later on
    /// `counted-matches`; the handler is one-shot (disconnected from inside its
    /// own emission, which GObject keeps the closure alive for) and a watchdog
    /// disconnects it if the engine never speaks, so an unanswered find can
    /// neither hang the caller nor accumulate handlers on the controller.
    ///
    /// `options` and `max_match_count` come from the shell's `web_find` module,
    /// which is the single owner of find POLICY (case-insensitivity, wrap, and
    /// the cap that decides whether a reported count is the truth). This layer
    /// hands the engine whatever it is given and returns the engine's number
    /// verbatim.
    ///
    /// Nothing here touches the keyboard: the find controller moves the
    /// SELECTION, not the focus, so the borrow-and-give-back protocol
    /// (`note_focus_owner_before_injection` / `schedule_focus_giveback`) has
    /// nothing to protect on this path.
    pub fn find(
        &self,
        id: u64,
        text: &str,
        action: FindAction,
        options: u32,
        max_match_count: u32,
        callback: impl FnOnce(Result<u32, String>) + 'static,
    ) -> Result<(), String> {
        use gtk::glib::prelude::ObjectExt as _;
        use webkit2gtk::{FindControllerExt as _, WebViewExt as _};
        let surfaces = self.surfaces.borrow();
        let surface = surfaces.get(&id).ok_or("no such surface")?;
        let webkit = {
            use wry::WebViewExtUnix;
            surface.webview.webview()
        };
        let controller = webkit
            .find_controller()
            .ok_or("engine gave this webview no find controller")?;

        // Closing (and an emptied field, which means the same thing) is
        // synchronous and has no count: `search_finish` is the call that DROPS
        // the highlights, and a bar that closes without it leaves the page
        // painted yellow with nothing on screen to explain why.
        if matches!(action, FindAction::Close) || text.is_empty() {
            controller.search_finish();
            callback(Ok(0));
            return Ok(());
        }

        // The ENGINE is the source of truth for what it is currently searching.
        // A `next` whose text has moved on (the user typed another letter, or a
        // reload wiped the controller) must restart rather than step whatever
        // the controller still holds.
        let engine_text = controller
            .search_text()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let same_query = engine_text == text;

        let pending: Rc<RefCell<Option<Box<dyn FnOnce(Result<u32, String>)>>>> =
            Rc::new(RefCell::new(Some(Box::new(callback))));
        let handler: Rc<RefCell<Option<gtk::glib::SignalHandlerId>>> =
            Rc::new(RefCell::new(None));
        {
            let pending = pending.clone();
            let handler_slot = handler.clone();
            let signal = controller.connect_counted_matches(move |controller, count| {
                if let Some(signal) = handler_slot.borrow_mut().take() {
                    controller.disconnect(signal);
                }
                if let Some(answer) = pending.borrow_mut().take() {
                    answer(Ok(count));
                }
            });
            *handler.borrow_mut() = Some(signal);
        }
        {
            // The watchdog: a content process that died mid-count would
            // otherwise leave the caller waiting and the handler connected
            // forever. Longer than the shell's own await so the engine's answer
            // wins whenever there is one.
            let pending = pending.clone();
            let handler_slot = handler.clone();
            let controller = controller.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(12), move || {
                if let Some(signal) = handler_slot.borrow_mut().take() {
                    controller.disconnect(signal);
                }
                if let Some(answer) = pending.borrow_mut().take() {
                    answer(Err("engine never reported a match count".to_string()));
                }
            });
        }

        match action {
            FindAction::Search => controller.search(text, options, max_match_count),
            FindAction::Next if same_query => controller.search_next(),
            FindAction::Previous if same_query => controller.search_previous(),
            // Direction is `search_previous`'s job, never the option mask's:
            // BACKWARDS plus search_previous double-reverses.
            FindAction::Next | FindAction::Previous => {
                controller.search(text, options, max_match_count)
            }
            FindAction::Close => unreachable!("handled above"),
        }
        controller.count_matches(text, options, max_match_count);
        Ok(())
    }

    /// One cookie as the engine layer knows it.
    ///
    /// Deliberately plain: this layer must not learn what a jar FILE looks
    /// like. The Netscape codec lives in `yggterm-shell` and converts.
    ///
    /// (Defined next to the two methods that use it so the whole cookie bridge
    /// reads in one place.)

    /// Read surface `id`'s cookie jar.
    ///
    /// HONEST LIMITATION, reported rather than hidden: WebKitGTK 4.x has no
    /// dump-the-whole-jar API. `cookies()` is per-URI and libsoup enforces the
    /// cookie's PATH against that URI, so querying each domain at its root
    /// returns every root-path cookie and misses path-scoped ones. The shell
    /// labels the result `export_scope: "root_path_per_domain"` instead of
    /// implying completeness.
    ///
    /// Reaching into the on-disk sqlite jar to close that gap would be a second
    /// encoding of the cookie store AND blind to unflushed in-memory state, so
    /// it is deliberately not done.
    pub fn cookies_export(
        &self,
        id: u64,
        callback: impl FnOnce(Result<Vec<CookieRecord>, String>) + 'static,
    ) -> Result<(), String> {
        use webkit2gtk::CookieManagerExt as _;
        let manager = self.cookie_manager(id)?;
        let cancellable: Option<gtk::gio::Cancellable> = None;
        let manager_for_domains = manager.clone();
        // Fired EXACTLY once, whichever path finishes: the domain enumeration
        // failing, an empty jar, or the last per-domain query completing. A
        // caller left waiting forever is worse than a refusal.
        let deliver = std::rc::Rc::new(std::cell::RefCell::new(Some(callback)));
        let deliver_once = {
            let deliver = deliver.clone();
            move |outcome: Result<Vec<CookieRecord>, String>| {
                if let Some(callback) = deliver.borrow_mut().take() {
                    callback(outcome);
                }
            }
        };
        manager.domains_with_cookies(cancellable.as_ref(), move |domains| {
            let domains: Vec<gtk::glib::GString> = match domains {
                Ok(domains) => domains,
                Err(error) => return deliver_once(Err(error.to_string())),
            };
            if domains.is_empty() {
                return deliver_once(Ok(Vec::new()));
            }
            // Both schemes per domain: a `secure` cookie is only returned for
            // an https URI, and a non-secure one set on a plain-http site is
            // only returned for http. Querying one scheme silently loses half
            // the jar.
            let outstanding = std::rc::Rc::new(std::cell::Cell::new(domains.len() * 2));
            let collected = std::rc::Rc::new(std::cell::RefCell::new(Vec::<CookieRecord>::new()));
            let deliver_once = std::rc::Rc::new(deliver_once);
            for domain in domains {
                for scheme in ["https", "http"] {
                    let host = domain.trim_start_matches('.').to_string();
                    let uri = format!("{scheme}://{host}/");
                    let outstanding = outstanding.clone();
                    let collected = collected.clone();
                    let deliver_once = deliver_once.clone();
                    let cancellable: Option<gtk::gio::Cancellable> = None;
                    manager_for_domains.cookies(&uri, cancellable.as_ref(), move |result| {
                        if let Ok(cookies) = result {
                            let mut collected = collected.borrow_mut();
                            for mut cookie in cookies {
                                let record = CookieRecord::from_soup(&mut cookie);
                                // Union by identity: the same cookie comes back
                                // from both scheme queries.
                                if !collected
                                    .iter()
                                    .any(|existing: &CookieRecord| existing.same_cookie(&record))
                                {
                                    collected.push(record);
                                }
                            }
                        }
                        outstanding.set(outstanding.get().saturating_sub(1));
                        if outstanding.get() == 0 {
                            let mut cookies = collected.borrow().clone();
                            // Deterministic order: the same jar must export
                            // byte-identically every time.
                            cookies.sort_by(|a, b| {
                                (&a.domain, &a.path, &a.name).cmp(&(&b.domain, &b.path, &b.name))
                            });
                            deliver_once(Ok(cookies));
                        }
                    });
                }
            }
        });
        Ok(())
    }

    /// Write cookies into surface `id`'s jar.
    ///
    /// The jar belongs to the surface's `WebContext`, i.e. its PROFILE — a
    /// surface with no explicit profile is `default`, which is the USER'S OWN
    /// browsing jar. The shell reports which profile was written for exactly
    /// that reason.
    pub fn cookies_import(
        &self,
        id: u64,
        cookies: Vec<CookieRecord>,
        callback: impl FnOnce(Result<usize, String>) + 'static,
    ) -> Result<(), String> {
        use webkit2gtk::CookieManagerExt as _;
        let manager = self.cookie_manager(id)?;
        if cookies.is_empty() {
            callback(Ok(0));
            return Ok(());
        }
        let outstanding = std::rc::Rc::new(std::cell::Cell::new(cookies.len()));
        let added = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let callback = std::rc::Rc::new(std::cell::RefCell::new(Some(callback)));
        for record in cookies {
            let mut cookie = record.to_soup();
            let outstanding = outstanding.clone();
            let added = added.clone();
            let callback = callback.clone();
            let cancellable: Option<gtk::gio::Cancellable> = None;
            manager.add_cookie(&mut cookie, cancellable.as_ref(), move |result| {
                if result.is_ok() {
                    added.set(added.get() + 1);
                }
                outstanding.set(outstanding.get().saturating_sub(1));
                if outstanding.get() == 0 {
                    if let Some(callback) = callback.borrow_mut().take() {
                        callback(Ok(added.get()));
                    }
                }
            });
        }
        Ok(())
    }

    /// The cookie manager for surface `id`'s web context.
    ///
    /// The jar is per-`WebContext`, and the sharing unit for a context is
    /// `web_context_key` — jar + egress + control endpoint — NOT the profile
    /// alone. Surfaces that agree on that key see one another's cookies; ones
    /// that differ on any component cannot.
    ///
    /// ⚠ This comment used to read "per-`WebContext` means per-PROFILE: two
    /// surfaces on the same profile share one jar". That was false and it hid a
    /// real bug: `open` built a context per SURFACE, so two tabs of one session
    /// held two in-memory cookie stores over one on-disk `cookies` file, a login
    /// in one was invisible in the other, and the last flush won.
    fn cookie_manager(&self, id: u64) -> Result<webkit2gtk::CookieManager, String> {
        use webkit2gtk::{WebContextExt as _, WebViewExt as _};
        use wry::WebViewExtUnix as _;
        let surfaces = self.surfaces.borrow();
        let surface = surfaces.get(&id).ok_or("no such surface")?;
        surface
            .webview
            .webview()
            .context()
            .ok_or("surface has no web context")?
            .cookie_manager()
            .ok_or_else(|| "web context has no cookie manager".to_string())
    }

    /// Capture surface `id`'s FULL DOCUMENT (whole page, not just the visible
    /// viewport) to a PNG at `path` via the engine's snapshot API. Async: the
    /// callback fires on the GTK main loop with `Ok(())` once the PNG is
    /// written, or `Err(msg)`.
    pub fn snapshot_full_page(
        &self,
        id: u64,
        path: std::path::PathBuf,
        callback: impl FnOnce(Result<(), String>) + 'static,
    ) -> Result<(), String> {
        use webkit2gtk::WebViewExt as _;
        let surfaces = self.surfaces.borrow();
        let surface = surfaces.get(&id).ok_or("no such surface")?;
        let webkit = {
            use wry::WebViewExtUnix;
            surface.webview.webview()
        };
        let cancellable: Option<&gtk::gio::Cancellable> = None;
        webkit.snapshot(
            webkit2gtk::SnapshotRegion::FullDocument,
            webkit2gtk::SnapshotOptions::empty(),
            cancellable,
            move |result| {
                let outcome = result.map_err(|e| e.to_string()).and_then(|surface| {
                    let image = cairo::ImageSurface::try_from(surface)
                        .map_err(|_| "snapshot is not an image surface".to_string())?;
                    let mut file = std::fs::File::create(&path)
                        .map_err(|e| format!("create {}: {e}", path.display()))?;
                    image
                        .write_to_png(&mut file)
                        .map_err(|e| format!("encode png: {e}"))
                });
                callback(outcome);
            },
        );
        Ok(())
    }

    // ---- Trusted input injection (agent control plane `do` verb, slice 2b) ----
    //
    // Deliver a synthesized GDK event STRAIGHT to a surface's engine webview
    // widget. NO seat pointer is moved and no seat key is pressed, so a
    // backgrounded/occluded (but still mapped) surface is actionable and the
    // user's real cursor/focus is never hijacked (the Helium-incident class
    // cannot recur through this path). WebKit treats the delivered event as real
    // windowing-system input, so the resulting DOM event carries
    // `isTrusted: true` — proven on webkit2gtk 2.52 by the slice-2a spike
    // (`docs/spikes/slice2a-istrusted-inject`). `x`/`y` are CSS-viewport pixels;
    // page zoom → widget px is applied here, next to the webview.

    /// Resolve surface `id`'s engine webview for an injected event, WAKING it
    /// first if this host is the one holding it hidden.
    ///
    /// An unmapped webview silently drops synthesized events (slice-2a
    /// hidden-phase proof: `events == []`), so injection has always failed
    /// closed with `surface_not_mapped` rather than lie about success. That
    /// refusal is still right for a surface we did not hide — a hard-stashed
    /// (detached) container cannot be mapped by showing its child, and there is
    /// nothing to wake.
    ///
    /// But page-visibility throttling deliberately unmaps every unrevealed
    /// surface, and those are precisely the surfaces agents drive. Visibility
    /// gates RENDERING; it must never gate the drive path. So a view WE hid is
    /// shown for the burst and re-hidden [`ENGINE_REHIDE_DELAY_MS`] after its
    /// last event — the same borrow-and-give-back shape as the keyboard focus
    /// loan, including the rule that a give-back only ever takes back what is
    /// still ours.
    ///
    /// GTK maps a shown widget synchronously when its parent is mapped and
    /// realized, so the map is re-checked immediately and the wake is UNDONE
    /// (and the injection refused) if it did not take. A refusal is honest; an
    /// event delivered into an unmapped view is not.
    ///
    /// The deliberate consequence: a burst flips `visibilitychange` twice. The
    /// page really is briefly presentable, which is a truth, and it is strictly
    /// better than the alternative of claiming `visible` forever.
    fn engine_webview_for_injection(&self, id: u64) -> Result<webkit2gtk::WebView, String> {
        use wry::WebViewExtUnix as _;
        let surfaces = self.surfaces.borrow();
        let surface = surfaces.get(&id).ok_or("no such surface")?;
        let webkit = surface.webview.webview();
        // Both readings taken HERE, from the engine, for THIS surface — the
        // decision is made in `injection_map_plan` so it is lockable, and this
        // is the wiring that lock cannot reach.
        let plan = injection_map_plan(
            surface.engine_hidden.get(),
            gtk::prelude::WidgetExt::is_mapped(&webkit),
        );
        match plan {
            InjectionMapPlan::Refuse => return Err("surface_not_mapped".to_string()),
            InjectionMapPlan::WakeAndRehide => {
                let _ = surface.webview.set_visible(true);
                // GTK maps a shown widget synchronously under a mapped, realized
                // parent — but if it did not, undo the wake and refuse rather
                // than deliver into a view that will drop the event.
                if !gtk::prelude::WidgetExt::is_mapped(&webkit) {
                    let _ = surface.webview.set_visible(false);
                    return Err("surface_not_mapped".to_string());
                }
            }
            InjectionMapPlan::Deliver | InjectionMapPlan::DeliverAndRehide => {}
        }
        if matches!(
            plan,
            InjectionMapPlan::WakeAndRehide | InjectionMapPlan::DeliverAndRehide
        ) {
            // Re-armed by EVERY event of the burst, so the loan is given back
            // once, after the last one — never part-way through a batch.
            drop(surfaces);
            schedule_engine_rehide(&self.surfaces, id);
        }
        Ok(webkit)
    }

    /// A left/middle/right button click (press + release on the same point;
    /// WebKit synthesizes the `click` from the pair). `button` is the GDK
    /// button number (1 left, 2 middle, 3 right). `(x, y)` are CSS-viewport px
    /// (post-scroll); zoom→widget mapping happens here, next to the webview.
    pub fn inject_click(&self, id: u64, x: f64, y: f64, button: u32) -> Result<(), String> {
        let webkit = self.engine_webview_for_injection(id)?;
        let (wx, wy) = css_viewport_to_widget(&webkit, x, y);
        // WebKit focuses the widget itself on a button press, so this path takes
        // the toplevel's keyboard focus even though nothing here calls
        // `grab_focus`. Book the lender before the press and give it back after.
        note_focus_owner_before_injection(&webkit);
        // Only the PRESS is observed as seat input (the observer watches
        // button-press, key-press and scroll — not release), so one credit.
        grant_injection_credits(id, 1);
        unsafe {
            synth_button(&webkit, true, wx, wy, button)?;
            synth_button(&webkit, false, wx, wy, button)?;
        }
        schedule_focus_giveback();
        Ok(())
    }

    /// A pointer move (real hover — drives `:hover`, tooltips, menu reveal).
    pub fn inject_move(&self, id: u64, x: f64, y: f64) -> Result<(), String> {
        let webkit = self.engine_webview_for_injection(id)?;
        let (wx, wy) = css_viewport_to_widget(&webkit, x, y);
        unsafe { synth_motion(&webkit, wx, wy) }
    }

    /// A smooth-scroll wheel event at CSS-viewport `(x, y)` with the given
    /// deltas (positive `dy` scrolls the page content down, like a real wheel).
    pub fn inject_scroll(&self, id: u64, x: f64, y: f64, dx: f64, dy: f64) -> Result<(), String> {
        let webkit = self.engine_webview_for_injection(id)?;
        let (wx, wy) = css_viewport_to_widget(&webkit, x, y);
        note_focus_owner_before_injection(&webkit);
        grant_injection_credits(id, 1);
        let result = unsafe { synth_scroll(&webkit, wx, wy, dx, dy) };
        schedule_focus_giveback();
        result
    }

    /// A single key press OR release. `keyval` is the GDK keyval (the shell maps
    /// key names / characters to it); `state` is the GDK modifier bitmask.
    pub fn inject_key(&self, id: u64, press: bool, keyval: u32, state: u32) -> Result<(), String> {
        let webkit = self.engine_webview_for_injection(id)?;
        // A key event needs keyboard focus in the target webview, so BORROW it:
        // note the lender first, grab, and hand it back when the burst ends. The
        // grab is NOT widget-local — it sets the GtkWindow's focus widget, which
        // is how an invisible agent surface came to swallow the keystrokes the
        // user was typing into their terminal (see
        // `note_focus_owner_before_injection`).
        note_focus_owner_before_injection(&webkit);
        gtk::prelude::WidgetExt::grab_focus(&webkit);
        // Only presses are observed; a release costs nothing.
        if press {
            grant_injection_credits(id, 1);
        }
        let result = unsafe { synth_key(&webkit, press, keyval, state) };
        schedule_focus_giveback();
        result
    }
}

/// Map CSS-viewport pixels to the webview WIDGET's GDK coordinate space. WebKit
/// page zoom (`zoom_level`) scales page content in the widget, so a CSS-px point
/// at viewport `(x, y)` lands at widget `(x·z, y·z)`. The HiDPI device scale is
/// handled by GDK below the event-coordinate layer, so it does not enter here.
fn css_viewport_to_widget(webkit: &webkit2gtk::WebView, x: f64, y: f64) -> (f64, f64) {
    use webkit2gtk::WebViewExt as _;
    let z = webkit.zoom_level();
    let z = if z > 0.0 { z } else { 1.0 };
    (x * z, y * z)
}

/// Synthesize a GDK button event and hand it to the webview widget (no seat
/// pointer). See the injection block on `WebSurfaceHost` for the trust/no-warp
/// rationale.
unsafe fn synth_button(
    webview: &webkit2gtk::WebView,
    press: bool,
    x: f64,
    y: f64,
    button: u32,
) -> Result<(), String> {
    use gtk::glib::translate::{from_glib_full, ToGlibPtr};
    let gdk_window = gtk::prelude::WidgetExt::window(webview)
        .ok_or("webview has no GdkWindow (unrealized)")?;
    let etype = if press {
        gdk::ffi::GDK_BUTTON_PRESS
    } else {
        gdk::ffi::GDK_BUTTON_RELEASE
    };
    let ev_ptr = gdk::ffi::gdk_event_new(etype);
    let bev = ev_ptr as *mut gdk::ffi::GdkEventButton;
    // Event coords belong to `event->window`, which is only the webview's own
    // window when it has one (legacy stacking) — see `widget_to_event_window`.
    let (x, y) = widget_to_event_window(webview, x, y);
    (*bev).window = gdk_window.to_glib_full();
    (*bev).send_event = 0; // look like windowing-system input, not SendEvent
    (*bev).time = 0; // GDK_CURRENT_TIME
    (*bev).x = x;
    (*bev).y = y;
    (*bev).x_root = x;
    (*bev).y_root = y;
    (*bev).button = button;
    (*bev).state = 0;
    if let Some(device) = default_seat_pointer() {
        (*bev).device = device.to_glib_full();
    }
    let event: gdk::Event = from_glib_full(ev_ptr);
    deliver_injected_event(webview, &event);
    Ok(())
}

/// Synthesize a GDK motion (hover) event.
unsafe fn synth_motion(webview: &webkit2gtk::WebView, x: f64, y: f64) -> Result<(), String> {
    use gtk::glib::translate::{from_glib_full, ToGlibPtr};
    let gdk_window = gtk::prelude::WidgetExt::window(webview)
        .ok_or("webview has no GdkWindow (unrealized)")?;
    let ev_ptr = gdk::ffi::gdk_event_new(gdk::ffi::GDK_MOTION_NOTIFY);
    let mev = ev_ptr as *mut gdk::ffi::GdkEventMotion;
    let (x, y) = widget_to_event_window(webview, x, y);
    (*mev).window = gdk_window.to_glib_full();
    (*mev).send_event = 0;
    (*mev).time = 0;
    (*mev).x = x;
    (*mev).y = y;
    (*mev).x_root = x;
    (*mev).y_root = y;
    (*mev).state = 0;
    (*mev).is_hint = 0;
    if let Some(device) = default_seat_pointer() {
        (*mev).device = device.to_glib_full();
    }
    let event: gdk::Event = from_glib_full(ev_ptr);
    deliver_injected_event(webview, &event);
    Ok(())
}

/// Synthesize a GDK smooth-scroll event.
unsafe fn synth_scroll(
    webview: &webkit2gtk::WebView,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
) -> Result<(), String> {
    use gtk::glib::translate::{from_glib_full, ToGlibPtr};
    let gdk_window = gtk::prelude::WidgetExt::window(webview)
        .ok_or("webview has no GdkWindow (unrealized)")?;
    let ev_ptr = gdk::ffi::gdk_event_new(gdk::ffi::GDK_SCROLL);
    let sev = ev_ptr as *mut gdk::ffi::GdkEventScroll;
    let (x, y) = widget_to_event_window(webview, x, y);
    (*sev).window = gdk_window.to_glib_full();
    (*sev).send_event = 0;
    (*sev).time = 0;
    (*sev).x = x;
    (*sev).y = y;
    (*sev).x_root = x;
    (*sev).y_root = y;
    (*sev).state = 0;
    (*sev).direction = gdk::ffi::GDK_SCROLL_SMOOTH;
    (*sev).delta_x = dx;
    (*sev).delta_y = dy;
    if let Some(device) = default_seat_pointer() {
        (*sev).device = device.to_glib_full();
    }
    let event: gdk::Event = from_glib_full(ev_ptr);
    deliver_injected_event(webview, &event);
    Ok(())
}

/// Synthesize a GDK key event (press or release).
unsafe fn synth_key(
    webview: &webkit2gtk::WebView,
    press: bool,
    keyval: u32,
    state: u32,
) -> Result<(), String> {
    use gtk::glib::translate::{from_glib_full, ToGlibPtr};
    let gdk_window = gtk::prelude::WidgetExt::window(webview)
        .ok_or("webview has no GdkWindow (unrealized)")?;
    let etype = if press {
        gdk::ffi::GDK_KEY_PRESS
    } else {
        gdk::ffi::GDK_KEY_RELEASE
    };
    let ev_ptr = gdk::ffi::gdk_event_new(etype);
    let kev = ev_ptr as *mut gdk::ffi::GdkEventKey;
    (*kev).window = gdk_window.to_glib_full();
    (*kev).send_event = 0;
    (*kev).time = 0;
    (*kev).state = state;
    (*kev).keyval = keyval;
    // A synthetic key event MUST carry a real hardware keycode, not 0. WebKit
    // builds the DOM `keydown`/`keyup` straight from `keyval` — so a keycode-0
    // event still fires a correct, isTrusted event and printable text still
    // inserts — but EDITING COMMANDS (DeleteBackward, MoveLeft, …) come from
    // GTK binding activation, which translates the event back through the
    // keymap using `hardware_keycode`/`group`. Keycode 0 translates to nothing,
    // no binding matches, and the command never runs: live-caught 2026-07-20,
    // where `do key --key Backspace` delivered `{key:"Backspace",
    // isTrusted:true}` to the page yet deleted no character. Reverse-map the
    // keyval through the display's keymap to fill both fields.
    let (hardware_keycode, group) = keyval_hardware_key(keyval);
    (*kev).hardware_keycode = hardware_keycode;
    (*kev).group = group;
    if let Some(device) = gdk::Display::default()
        .and_then(|d| d.default_seat())
        .and_then(|s| s.keyboard())
    {
        gdk::ffi::gdk_event_set_device(ev_ptr, device.to_glib_full());
    }
    let event: gdk::Event = from_glib_full(ev_ptr);
    deliver_injected_event(webview, &event);
    Ok(())
}

/// Translate widget-local coordinates into the coordinate space of the GdkWindow
/// a synthesized event will carry (`WidgetExt::window`), which is NOT always the
/// widget's own window.
///
/// GDK event coordinates are relative to `event->window`. A widget that owns its
/// window (`has_window`) takes widget-local coords unchanged; a WINDOWLESS widget
/// shares its nearest ancestor's window, and GTK defines its allocation to be in
/// that same window's space — so the allocation origin is exactly the offset to
/// add.
///
/// This is the difference between the two web stackings, and it silently broke
/// injection: LEGACY page webviews own a NATIVE GdkWindow (the very thing the
/// under-glass self-probe looks for), so widget-local == window-local and clicks
/// landed. UNDER GLASS there is deliberately no native subwindow, so unadjusted
/// widget-local coords addressed a point somewhere else in the ancestor window
/// and WebKit dropped the event — while the verb still reported success. Caught
/// live 2026-07-20, when a `do click` that had "passed" for weeks turned out to
/// have only ever run against a GUI that had fallen back to legacy stacking.
fn widget_to_event_window(webview: &webkit2gtk::WebView, x: f64, y: f64) -> (f64, f64) {
    use gtk::prelude::WidgetExt as _;
    if webview.has_window() {
        return (x, y);
    }
    let allocation = webview.allocation();
    (x + f64::from(allocation.x()), y + f64::from(allocation.y()))
}

/// Reverse-map a keyval to the `(hardware_keycode, group)` that produces it on
/// this display's keymap, so a synthesized key event can activate GTK key
/// bindings (WebKit's editing commands) and not just fire a DOM event. Falls
/// back to `(0, 0)` — the DOM event still carries the right `key`, only the
/// editing command is lost — when the keyval is not on the layout at all
/// (e.g. a codepoint key the user's layout cannot type) or there is no keymap.
fn keyval_hardware_key(keyval: u32) -> (u16, u8) {
    let Some(keymap) = gdk::Display::default().and_then(|display| gdk::Keymap::for_display(&display))
    else {
        return (0, 0);
    };
    keymap
        .entries_for_keyval(keyval)
        .into_iter()
        // Prefer the unshifted entry: a shifted level would need the matching
        // modifier in `state`, which the caller owns and did not ask for.
        .min_by_key(|key| (key.level(), key.group()))
        .map(|key| {
            (
                u16::try_from(key.keycode()).unwrap_or(0),
                u8::try_from(key.group()).unwrap_or(0),
            )
        })
        .unwrap_or((0, 0))
}

/// The default seat's pointer device, or None on a headless seat.
fn default_seat_pointer() -> Option<gdk::Device> {
    gdk::Display::default()
        .and_then(|d| d.default_seat())
        .and_then(|s| s.pointer())
}

/// Proxy one `yggterm-appctl://` request to the app's control endpoint `base`
/// (e.g. `http://127.0.0.1:38749`, already GUI-reachable). Runs on the GTK main
/// thread when called, so the blocking forward is moved to its own thread and
/// answers through the async `responder` — a `/fido2/get` blocks up to two
/// minutes waiting for the presence dialog, which lives on this very thread.
fn app_control_proxy(base: String, request: Request<Vec<u8>>, responder: RequestAsyncResponder) {
    // A cross-origin fetch from the RP's https page preflights; answer OPTIONS
    // ourselves rather than forwarding it.
    if request.method() == "OPTIONS" {
        responder.respond(cors_response(204, Vec::new()));
        return;
    }
    // Path (+ query) is what the app's control server routes on; the scheme host
    // is ignored (the app is identified by `base`, one per surface).
    let mut path = request.uri().path().to_string();
    if let Some(query) = request.uri().query() {
        path.push('?');
        path.push_str(query);
    }
    let method = request.method().as_str().to_string();
    // Forward only the headers the signer cares about — the bearer token gate and
    // the content type. Everything else (Origin, Sec-*, etc.) is browser noise.
    let token = request
        .headers()
        .get("X-Ychrome-Fido2")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let content_type = request
        .headers()
        .get("Content-Type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let body = request.into_body();

    std::thread::spawn(move || {
        let (status, payload) = match forward_to_control(&base, &method, &path, &content_type, token.as_deref(), &body) {
            Ok(result) => result,
            Err(error) => (
                502,
                format!("{{\"error\":\"app control unreachable: {error}\"}}").into_bytes(),
            ),
        };
        responder.respond(cors_response(status, payload));
    });
}

/// One blocking HTTP request to `base` (`http://host:port`), returning the status
/// and body. Hand-rolled over `TcpStream` — the app's control server is dep-light
/// and one request at a time, and this mirrors the shell's own `control_request`.
fn forward_to_control(
    base: &str,
    method: &str,
    path: &str,
    content_type: &str,
    token: Option<&str>,
    body: &[u8],
) -> Result<(u16, Vec<u8>), String> {
    let authority = base
        .strip_prefix("http://")
        .ok_or_else(|| "control base must be http://".to_string())?;
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().map_err(|_| "bad port".to_string())?),
        None => (authority, 80),
    };
    let mut stream = std::net::TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    // A get() ceremony can wait two minutes for the user; give the read room.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(180)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(30)));

    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n"
    );
    if !body.is_empty() || method == "POST" {
        head.push_str(&format!("Content-Type: {content_type}\r\n"));
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    if let Some(token) = token {
        head.push_str(&format!("X-Ychrome-Fido2: {token}\r\n"));
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
    stream.write_all(body).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "no header/body split".to_string())?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(502);
    Ok((status, raw[split + 4..].to_vec()))
}

/// A JSON response with the CORS headers the RP's https page needs to read a
/// cross-origin custom-scheme reply. CORS is not the security boundary here (the
/// bearer token and the request-id are), and the shim sends no credentials, so
/// `*` is safe.
fn cors_response(status: u16, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header("Access-Control-Allow-Headers", "Content-Type, X-Ychrome-Fido2")
        .header("Cache-Control", "no-store")
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

#[cfg(test)]
mod web_context_key_tests {
    use super::web_context_key;
    use std::path::Path;

    /// THE SHARING RULE. Two tabs of one ychrome session agree on jar, egress
    /// and control endpoint by construction (`web_surface_new_tab` copies the
    /// first tab's profile), so they must land on ONE key — one WebContext, one
    /// process pool, one cookie jar.
    ///
    /// Before this, `open` called `WebContext::new(profile_dir)` unconditionally
    /// per surface, so those two tabs got two contexts pointing at the same
    /// directory: two WebKitWebProcesses, two WebKitNetworkProcesses, and two
    /// independent in-memory cookie stores writing one Netscape-text file. A
    /// login in tab A was invisible in tab B and whichever flushed last won.
    #[test]
    fn two_tabs_of_one_session_share_exactly_one_context() {
        let jar = Path::new("/home/u/.yggterm/web-profiles/default");
        let a = web_context_key(Some(jar), None, Some("http://127.0.0.1:9001"));
        let b = web_context_key(Some(jar), None, Some("http://127.0.0.1:9001"));
        assert!(a.is_some());
        assert_eq!(a, b, "same jar + egress + endpoint is ONE context");
    }

    /// The three components are each load-bearing, and each for its own reason:
    /// the jar is the storage boundary, the proxy is fixed on the context's
    /// WebsiteDataManager at build time, and the `yggterm-appctl://` scheme is
    /// registered on the context and proxies to exactly ONE session's control
    /// endpoint. Collapsing any of them would be a real bug, not a cost saving.
    #[test]
    fn jar_egress_and_control_endpoint_each_split_the_context() {
        let jar = Path::new("/home/u/.yggterm/web-profiles/default");
        let other = Path::new("/home/u/.yggterm/web-profiles/work");
        let base = web_context_key(Some(jar), Some(1080), Some("http://127.0.0.1:9001"));
        assert_ne!(
            base,
            web_context_key(Some(other), Some(1080), Some("http://127.0.0.1:9001")),
            "different jars must never mix"
        );
        assert_ne!(
            base,
            web_context_key(Some(jar), Some(1081), Some("http://127.0.0.1:9001")),
            "the proxy is per-context: a different tunnel is a different context"
        );
        assert_ne!(
            base,
            web_context_key(Some(jar), Some(1080), Some("http://127.0.0.1:9002")),
            "appctl:// proxies to ONE session's endpoint; sharing would cross sessions"
        );
        assert_ne!(
            base,
            web_context_key(Some(jar), None, Some("http://127.0.0.1:9001")),
            "no proxy is not the same egress as a proxy"
        );
    }

    /// Ephemeral means ephemeral. A shared ephemeral context would be a jar two
    /// surfaces could see each other through, which is precisely what the caller
    /// asked not to have.
    #[test]
    fn ephemeral_surfaces_never_share() {
        assert_eq!(web_context_key(None, None, None), None);
        assert_eq!(
            web_context_key(None, Some(1080), Some("http://127.0.0.1:9001")),
            None
        );
    }

    /// The components are arbitrary text (a path, a URL), so concatenation must
    /// not be able to forge a match. A separator that could appear inside a
    /// component would let `("/a\u{1f}b", None, "")` collide with `("/a", None,
    /// "b")` — different jars reading one another's cookies.
    #[test]
    fn components_cannot_collide_by_concatenation() {
        let a = web_context_key(Some(Path::new("/a")), None, Some("b"));
        let b = web_context_key(Some(Path::new("/a\u{1f}\u{1f}b")), None, None);
        assert_ne!(a, b);
        assert_ne!(
            web_context_key(Some(Path::new("/a")), Some(10), Some("80")),
            web_context_key(Some(Path::new("/a")), Some(1080), None)
        );
    }
}

#[cfg(test)]
mod seat_input_tests {
    use super::*;

    /// The discrimination that gate 9 rests on. The dangerous direction is a
    /// FALSE POSITIVE: if the agent's own injection were counted as human,
    /// every agent batch would preempt itself on its second verb and the `do`
    /// verb would be unusable. `deliver_injected_event` sets the flag around the
    /// synchronous GTK delivery, so anything observed inside it is ours.
    #[test]
    fn injected_events_are_not_counted_as_seat_input() {
        let id = 4242;
        take_seat_input_count(id); // clear

        // Simulate what happens INSIDE deliver_injected_event: the flag is set
        // for the duration of the synchronous delivery, during which the GTK
        // handler fires and calls note_seat_input.
        INJECTING_EVENT.with(|f| f.set(true));
        note_seat_input(id);
        note_seat_input(id);
        INJECTING_EVENT.with(|f| f.set(false));
        assert_eq!(
            take_seat_input_count(id),
            0,
            "the agent's own injection must never register as the human"
        );

        // A real seat event arrives outside any injection.
        note_seat_input(id);
        assert_eq!(take_seat_input_count(id), 1);
    }

    /// The regression the credit backstop exists for, and the case the test
    /// above CANNOT see: an injected event observed AFTER `deliver_injected_event`
    /// has returned (queued GTK delivery). With only the lexical flag, this
    /// books our own injection as the human, which preempts the agent's batch —
    /// and since the batch id is per-GUI-process, that is a permanent lockout.
    ///
    /// This drives TWO consecutive verbs, because one verb never reproduced it.
    #[test]
    fn a_late_observed_injection_is_still_not_seat_input() {
        let id = 4244;
        take_seat_input_count(id);

        // Verb 1: credit granted at inject_*, event observed after the flag
        // cleared (the flag is NOT set here — that is the whole point).
        grant_injection_credits(id, 1);
        note_seat_input(id);
        assert_eq!(
            take_seat_input_count(id),
            0,
            "verb 1's own injection must not read as the human"
        );

        // Verb 2 must therefore still be admitted — under the old code this is
        // where the lane was preempted and the agent locked out for good.
        grant_injection_credits(id, 1);
        note_seat_input(id);
        assert_eq!(
            take_seat_input_count(id),
            0,
            "verb 2 must not see verb 1's injection as a human takeover"
        );
    }

    /// The credit must never outlive its verb, or a fix for the agent becomes a
    /// bug for the user: a real gesture after an unspent credit must still count.
    #[test]
    fn an_unspent_credit_cannot_swallow_a_later_human_gesture() {
        let id = 4245;
        take_seat_input_count(id);

        // Synchronous delivery: the lexical flag suppressed it, so the credit
        // granted for that event is never spent.
        grant_injection_credits(id, 1);
        INJECTING_EVENT.with(|f| f.set(true));
        note_seat_input(id);
        INJECTING_EVENT.with(|f| f.set(false));
        assert_eq!(take_seat_input_count(id), 0);

        // The human now clicks. It must be counted, not eaten by the leftover.
        note_seat_input(id);
        assert_eq!(
            take_seat_input_count(id),
            1,
            "a stale credit must not mask the human taking the surface"
        );
    }

    /// THE BATCH LOCK, engine half. `web batch` fires up to N injections behind
    /// ONE gate, so the credit machinery has to hold for a whole run — not just
    /// for the two consecutive verbs the earlier test drives.
    ///
    /// Twenty injections, each observed AFTER its delivery scope closed (the
    /// queued-delivery shape that produced the single-shot `do` bug), must read
    /// as ZERO seat input every single time. Revert `spend_injection_credit` to
    /// `false` and this fails at iteration 1, which is what makes it a lock
    /// rather than a decoration.
    #[test]
    fn twenty_injections_never_read_as_the_human() {
        let id = 4247;
        take_seat_input_count(id); // clear

        for iteration in 0..20 {
            grant_injection_credits(id, 1);
            note_seat_input(id);
            assert_eq!(
                take_seat_input_count(id),
                0,
                "injection {iteration} of a batch was booked as the human"
            );
        }

        // …and the human is still heard afterwards: throughput must not cost
        // the user the surface.
        note_seat_input(id);
        assert_eq!(take_seat_input_count(id), 1);
    }

    /// THE INTER-VERB GAP LOCK. The end-of-verb drop in
    /// [`take_seat_input_count`] only runs when the shell reads the counter,
    /// which it does at the START of the next verb — so between two verbs a
    /// burst's unspent credits used to sit there and eat the user's keystrokes.
    ///
    /// Same surface, no `take_seat_input_count` in between (that is the gap):
    /// a dozen credits granted and suppressed by the lexical flag, then the
    /// human types [`INJECTION_CREDIT_TTL_MS`] later. Every one of those
    /// keystrokes must be counted. Drop the expiry from
    /// `spend_injection_credit_at` and this reads 0 — the stray-characters
    /// incident, reproduced.
    #[test]
    fn credits_do_not_outlive_their_burst_into_the_gap_the_user_types_in() {
        let id = 4248;
        take_seat_input_count(id); // clear

        // A `fill` burst: select-all + delete + ten characters, all delivered
        // synchronously, so the lexical flag suppresses every observation and
        // not one of the twelve credits is spent.
        let burst_at = 10_000;
        for _ in 0..12 {
            grant_injection_credits_at(id, 1, burst_at);
            INJECTING_EVENT.with(|f| f.set(true));
            note_seat_input_at(id, burst_at);
            INJECTING_EVENT.with(|f| f.set(false));
        }

        // The verb ends. The shell will not read the counter until the NEXT
        // verb, and the user types now.
        let types_at = burst_at + INJECTION_CREDIT_TTL_MS;
        for offset in 0..12 {
            note_seat_input_at(id, types_at + offset);
        }

        assert_eq!(
            take_seat_input_count(id),
            12,
            "the user's keystrokes in the inter-verb gap were booked as agent injections"
        );
    }

    /// ...and the expiry must not cost the agent the thing credits exist for:
    /// an injection observed a queue hop after its delivery is still ours.
    #[test]
    fn a_credit_still_covers_its_own_late_delivery_inside_the_ttl() {
        let id = 4249;
        take_seat_input_count(id);

        let granted_at = 20_000;
        grant_injection_credits_at(id, 1, granted_at);
        note_seat_input_at(id, granted_at + INJECTION_CREDIT_TTL_MS - 1);
        assert_eq!(
            take_seat_input_count(id),
            0,
            "a late-delivered injection inside the TTL must still read as ours"
        );
    }

    /// Expiry walks the OLDEST grants first, so a stale credit can never shield
    /// a fresh injection's own late delivery — the queue is time-ordered and the
    /// drop is a prefix, not a scan.
    #[test]
    fn an_expired_credit_is_dropped_rather_than_spent_by_a_later_injection() {
        let id = 4250;
        take_seat_input_count(id);

        grant_injection_credits_at(id, 1, 30_000); // verb 1, never spent
        grant_injection_credits_at(id, 1, 30_000 + INJECTION_CREDIT_TTL_MS); // verb 2

        // Verb 2's own late delivery spends verb 2's credit; verb 1's expired
        // one is discarded on the way past. No `take_seat_input_count` in
        // between — that call drops the whole ledger, which would make this
        // pass whatever the spend order is.
        note_seat_input_at(id, 30_000 + INJECTION_CREDIT_TTL_MS);
        // The human, one millisecond later: nothing is left to swallow them.
        note_seat_input_at(id, 30_000 + INJECTION_CREDIT_TTL_MS + 1);
        assert_eq!(
            take_seat_input_count(id),
            1,
            "verb 1's stale credit was spent instead of dropped, so the human's \
             gesture went to the agent's ledger"
        );
    }

    #[test]
    fn taking_the_count_consumes_it() {
        let id = 4243;
        take_seat_input_count(id);
        note_seat_input(id);
        note_seat_input(id);
        assert_eq!(take_seat_input_count(id), 2);
        // Consumed: a second read reports no NEW input, so one human click
        // preempts once rather than forever.
        assert_eq!(take_seat_input_count(id), 0);
    }

    #[test]
    fn surfaces_count_seat_input_independently() {
        let (a, b) = (4244, 4245);
        take_seat_input_count(a);
        take_seat_input_count(b);
        note_seat_input(a);
        assert_eq!(take_seat_input_count(b), 0, "input on A must not preempt B");
        assert_eq!(take_seat_input_count(a), 1);
    }

    #[test]
    fn forgetting_a_closed_surface_clears_its_tally() {
        let id = 4246;
        note_seat_input(id);
        forget_seat_input(id);
        assert_eq!(take_seat_input_count(id), 0);
    }
}

/// LOCKS for engine page-visibility — the fact that decides whether a surface's
/// page paints at all.
///
/// WebKitGTK derives `document.visibilityState` from widget mapping; there is
/// no page-visibility setter on this API. So "is this page allowed to animate"
/// is decided by exactly four writes in this file (a hidden `open`,
/// `set_visible`, `set_throttled`, `unstash`) and consumed by one reader
/// (`engine_webview_for_injection`). None of the five can be exercised without a
/// live GtkWindow, so the decision is a pure function and the WIRING is scanned:
/// a reverted write or a reverted reading changes the text these needles look
/// for.
#[cfg(test)]
mod engine_visibility_locks {
    use super::*;

    /// PRODUCT lines of this file: everything outside a `#[cfg(test)] mod`
    /// block. Without it every needle below would be satisfied by the assertion
    /// that names it — the source-scan failure this workspace has already
    /// shipped once. Local rather than shared because a vendored engine crate
    /// must not depend on the shell's crates.
    pub(super) fn product_lines() -> Vec<String> {
        let source = include_str!("web_surface.rs");
        let mut out = Vec::new();
        let mut in_test_module = false;
        let mut pending_test_attribute = false;
        for line in source.lines() {
            if in_test_module {
                if line == "}" {
                    in_test_module = false;
                }
                continue;
            }
            if line.starts_with("#[cfg(test)]") {
                pending_test_attribute = true;
                continue;
            }
            if pending_test_attribute {
                pending_test_attribute = false;
                if line.starts_with("mod ") || line.starts_with("pub mod ") {
                    in_test_module = true;
                    continue;
                }
            }
            out.push(line.to_string());
        }
        out
    }

    /// The body of the named free function or method, from its signature line to
    /// the first line that closes it at the same indent.
    pub(super) fn body_of(product: &[String], signature: &str) -> String {
        let start = product
            .iter()
            .position(|line| line.contains(signature))
            .unwrap_or_else(|| panic!("`{signature}` is gone from this file"));
        let indent = product[start].len() - product[start].trim_start().len();
        let end = product[start + 1..]
            .iter()
            .position(|line| line.trim() == "}" && line.len() - line.trim_start().len() == indent)
            .map(|offset| start + 1 + offset)
            .unwrap_or_else(|| panic!("unterminated `{signature}`"));
        assert!(
            end - start > 3,
            "the captured body of `{signature}` is too short to be the real one",
        );
        product[start..=end].join("\n")
    }

    /// Injection may wake what WE hid, and only what we hid. The cell that
    /// carries the design is `(engine_hidden: false, mapped: false)`: a detached
    /// container's child cannot be mapped by showing it, so that case must still
    /// fail closed exactly as it did before page-visibility throttling existed.
    #[test]
    fn only_a_surface_this_host_hid_is_woken_for_an_injection() {
        assert_eq!(
            injection_map_plan(false, true),
            InjectionMapPlan::Deliver,
            "a revealed, mapped surface needs nothing done to it",
        );
        assert_eq!(
            injection_map_plan(false, false),
            InjectionMapPlan::Refuse,
            "unmapped and not ours to wake (detached / hard-stashed): refusing is \
             the honest answer, because the engine drops the event silently",
        );
        assert_eq!(
            injection_map_plan(true, false),
            InjectionMapPlan::WakeAndRehide,
            "a surface WE hid for page visibility is the ordinary agent target — \
             visibility gates rendering, never the drive path",
        );
        assert_eq!(
            injection_map_plan(true, true),
            InjectionMapPlan::DeliverAndRehide,
            "mid-burst: already woken, still ours, so the re-hide must be re-armed \
             or the loan ends part-way through the batch",
        );
    }

    /// The reader's wiring. The decision above is worth nothing if the resolver
    /// stops taking its two readings from the engine, stops undoing a wake that
    /// did not map, or stops arming the give-back.
    #[test]
    fn the_injection_resolver_reads_the_engine_and_gives_the_wake_back() {
        let product = product_lines();
        assert!(
            !product
                .iter()
                .any(|line| line.contains("mod engine_visibility_locks")),
            "the scan is reading this test module, so every needle below would be \
             satisfied by the assertion that names it",
        );
        let body = body_of(&product, "fn engine_webview_for_injection(");
        for needle in [
            // Both readings, taken here, for THIS surface. A hardcoded either
            // side turns the wake permanently on or permanently off.
            "surface.engine_hidden.get(),",
            "gtk::prelude::WidgetExt::is_mapped(&webkit),",
            "let plan = injection_map_plan(",
            // Refuse must still be a refusal, with the same string the agent
            // control plane already documents.
            "InjectionMapPlan::Refuse => return Err(\"surface_not_mapped\".to_string()),",
            // The wake, and the fail-closed re-check that undoes it.
            "let _ = surface.webview.set_visible(true);",
            "let _ = surface.webview.set_visible(false);",
            // The give-back. Without it a driven surface stays mapped and
            // painting forever, which is the bug wearing a different hat.
            "schedule_engine_rehide(&self.surfaces, id);",
        ] {
            assert!(
                body.contains(needle),
                "the injection resolver no longer does `{needle}`:\n{body}",
            );
        }
    }

    /// The four writers. Page visibility has ONE record in this host
    /// (`Surface::engine_hidden`) precisely so a second one cannot disagree with
    /// it — and the give-back consults that record before re-hiding, so a
    /// writer that stops maintaining it can blank a page the user just revealed.
    #[test]
    fn every_visibility_write_records_what_the_engine_was_told() {
        let product = product_lines();
        let open = body_of(&product, "pub fn open(");
        assert!(
            open.contains("if !visible {") && open.contains("let _ = webview.set_visible(false);"),
            "a surface is no longer born hidden when nobody is being shown it — \
             creating it visible and hiding it a tick later is a lie the page has \
             already acted on",
        );
        assert!(
            open.contains("engine_hidden: Cell::new(!visible),"),
            "the create no longer records whether it hid the surface, so the \
             injection wake cannot tell its own hiding from a hard stash",
        );

        let set_visible = body_of(&product, "pub fn set_visible(");
        assert!(
            set_visible.contains("s.engine_hidden.set(!visible);"),
            "set_visible no longer records what the engine was told",
        );

        let set_throttled = body_of(&product, "pub fn set_throttled(");
        assert!(
            set_throttled.contains("let _ = s.webview.set_visible(!throttled);")
                && set_throttled.contains("s.engine_hidden.set(throttled);"),
            "the throttle no longer hides the inner webview and records it — \
             hiding the webview IS the page-visibility mechanism",
        );

        let unstash = body_of(&product, "pub fn unstash(");
        assert!(
            unstash.contains("let _ = s.webview.set_visible(true);")
                && unstash.contains("s.engine_hidden.set(false);"),
            "reveal no longer clears the engine-hidden record, so a re-hide armed \
             by an in-flight agent burst can blank the page the user switched to",
        );

        let rehide = body_of(&product, "fn schedule_engine_rehide(");
        assert!(
            rehide.contains("if !surface.engine_hidden.get() {"),
            "the re-hide no longer checks that the surface is still ours to hide",
        );
        assert!(
            rehide.contains("if surface.wake_token.get() != token {"),
            "the re-hide no longer honours the re-arm token, so a burst gives its \
             wake back part-way through",
        );
    }
}

/// LOCKS for the SCRIPTLET PLANE — the placement half of a userscript.
///
/// The decision is driven end to end on a real `WebViewBuilder`: patterns, world
/// and frames are staged on a builder here and read back off it, which is the
/// last point before the FFI that a test on this host can reach (building the
/// view itself needs a display and an engine). The FFI call that consumes those
/// staged facts — `UserScript::new` / `UserScript::for_world` — is scanned out
/// of wry's own source, because a `webkit_user_script_*` call cannot be made
/// off the GTK main thread.
#[cfg(test)]
mod scriptlet_locks {
    use super::engine_visibility_locks::product_lines;
    use super::*;

    fn script(body: &str) -> SurfaceUserscript {
        SurfaceUserscript {
            body: body.to_string(),
            ..Default::default()
        }
    }

    /// Every placement fact reaches the builder, and reaches it INVERTED where
    /// the two vocabularies disagree: the plane says "all frames", wry says
    /// "main frame only".
    #[test]
    fn a_scripts_patterns_world_and_frames_reach_the_builder() {
        let scripts = vec![
            SurfaceUserscript {
                matches: vec!["https://*.youtube.com/*".to_string()],
                all_frames: true,
                isolated_world: true,
                ..script("scoped")
            },
            SurfaceUserscript {
                isolated_world: false,
                ..script("page-world")
            },
        ];
        let builder = attach_userscripts(WebViewBuilder::new(), &scripts);
        let staged = builder.initialization_scripts();
        assert_eq!(staged.len(), 2);

        assert_eq!(staged[0].script, "scoped");
        assert_eq!(
            staged[0].allow_list,
            vec!["https://*.youtube.com/*".to_string()],
            "the @match patterns never reached the engine, so a YouTube script \
             is running on every tab",
        );
        assert!(
            !staged[0].for_main_frame_only,
            "@all-frames must become NOT main-frame-only",
        );
        assert_eq!(staged[0].world_name.as_deref(), Some(USERSCRIPT_WORLD));

        // A main-world script must carry NO world name: `new_for_world` always
        // resolves a name to an isolated world, so the page's own world is the
        // absence of a name, never a name that spells it.
        assert_eq!(staged[1].script, "page-world");
        assert!(
            staged[1].world_name.is_none(),
            "a @world main script was put in an isolated world, where its patch \
             to a page API is invisible to the page that calls it",
        );
        assert!(staged[1].for_main_frame_only);
        assert!(staged[1].allow_list.is_empty(), "no patterns = every URL");
    }

    /// The OLD entry point must keep meaning what it always meant: every URL,
    /// the page's own world. wry's ipc bridge and this file's `window.close()`
    /// shim both go through it, and both exist to be reached BY THE PAGE — give
    /// either one a world or a pattern and it stops being there.
    ///
    /// (This is a wry-level invariant, checked from here: wry is a path
    /// dependency, not a workspace member, so `cargo test -p wry` cannot run and
    /// a lock left in that crate would never execute.)
    #[test]
    fn the_unscoped_entry_point_still_means_every_url_in_the_pages_world() {
        let builder =
            WebViewBuilder::new().with_initialization_script_for_main_only(CLOSE_SHIM_JS, true);
        let staged = builder.initialization_scripts();
        assert_eq!(staged.len(), 1);
        assert!(
            staged[0].allow_list.is_empty(),
            "the close shim acquired match patterns, so pages outside them can \
             no longer report window.close()",
        );
        assert!(
            staged[0].world_name.is_none(),
            "the close shim moved to an isolated world, where the page cannot \
             see the function it is supposed to call",
        );
    }

    /// Isolated scripts SHARE one world. Two of them must land in the same one,
    /// or a script cannot see a helper another script installed and the plane
    /// silently becomes per-script sandboxes.
    #[test]
    fn isolated_scripts_share_one_world() {
        let builder = attach_userscripts(
            WebViewBuilder::new(),
            &[
                SurfaceUserscript {
                    isolated_world: true,
                    ..script("a")
                },
                SurfaceUserscript {
                    isolated_world: true,
                    ..script("b")
                },
            ],
        );
        let staged = builder.initialization_scripts();
        assert_eq!(staged[0].world_name, staged[1].world_name);
        assert!(staged[0].world_name.is_some());
    }

    /// Order is injection order, and injection order is what decides which of
    /// two scripts patching the same thing wins. The passkey shim is first on
    /// the wire for exactly that reason.
    #[test]
    fn scripts_are_staged_in_the_order_they_arrived() {
        let builder = attach_userscripts(
            WebViewBuilder::new(),
            &[script("first"), script("second"), script("third")],
        );
        let bodies: Vec<&str> = builder
            .initialization_scripts()
            .iter()
            .map(|staged| staged.script.as_str())
            .collect();
        assert_eq!(bodies, vec!["first", "second", "third"]);
    }

    /// BOTH surfaces go through the one helper. A popup that staged its scripts
    /// its own way would run them unscoped in the page's world while the page
    /// that opened it ran them scoped and isolated — and a popup is exactly the
    /// window a sign-in shim is needed in.
    ///
    /// APPEND-PROOF: it is not enough that the helper is called somewhere in the
    /// file. Neither call site may ALSO still be pushing raw bodies through the
    /// unscoped entry point, which is what a careless merge of this change onto
    /// the old loop would leave behind.
    #[test]
    fn the_page_and_the_popup_both_stage_scripts_through_the_one_helper() {
        let product = product_lines();
        assert!(
            !product
                .iter()
                .any(|line| line.contains("mod scriptlet_locks")),
            "the scan is reading this test module, so every needle below would \
             be satisfied by the assertion that names it",
        );
        let calls = product
            .iter()
            .filter(|line| line.contains("attach_userscripts(builder,"))
            .count();
        assert_eq!(
            calls, 2,
            "the page surface and the popup must each stage their scripts \
             through `attach_userscripts`; found {calls} call sites",
        );
        assert!(
            !product
                .iter()
                .any(|line| line.contains("with_initialization_script_for_main_only(script")),
            "a call site is still pushing a raw userscript body through the \
             unscoped entry point, which drops its @match and its @world",
        );
    }

    /// The FFI half, scanned out of wry: the staged `allow_list` must reach
    /// BOTH `UserScript` constructors, and the world must pick between them.
    /// This is the line that was hardcoded to `&[]` — the whole reason `@match`
    /// did not exist.
    #[test]
    fn wrys_user_script_constructors_take_the_allow_list_and_the_world() {
        let source = include_str!("../../wry/src/webkitgtk/mod.rs");
        let body: String = source
            .lines()
            .skip_while(|line| !line.contains("fn init_script(&self"))
            .take_while(|line| line.trim() != "}")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("fn init_script(&self"),
            "`init_script` is gone from wry's webkitgtk backend",
        );
        assert!(
            body.contains("UserScript::for_world("),
            "the isolated-world constructor is gone; every script is back in \
             the page's world",
        );
        assert!(
            body.contains("UserScript::new("),
            "the page-world constructor is gone; a shim that must be visible to \
             the page can no longer be injected",
        );
        // The allow-list must be threaded into BOTH constructors. `&[]` on
        // either one silently un-scopes every script that takes that branch —
        // which is precisely the state this change found the code in.
        assert_eq!(
            body.matches("&allow_list,").count(),
            2,
            "both `UserScript` constructors must receive the allow-list",
        );
        assert!(
            !body.contains("        &[],\n        &[],"),
            "a constructor is back to hardcoding an empty allow-list, so \
             @match is silently ignored again",
        );
    }
}

/// LOCKS for downloads — the plane that decides where a file a page hands us
/// lands, what it is called, and what the user is told about it.
///
/// The WebKit half (a real `decide-destination` on a real transfer) needs a
/// display and an engine, which no test on this host has; so the split is the
/// one this file already uses for engine-side mechanisms: the DECISIONS are
/// driven end to end here — against a real localhost server answering with a
/// real `Content-Disposition`, against a real transfer killed mid-flight,
/// against `downloads_dir()` itself under a `HOME` a lock owns (in a child
/// process, since the environment is process-global and these run eight at a
/// time), and against the real sweep rule with a real registry entry — and the
/// WIRING that hands those decisions to WebKit is scanned out of the product
/// source.
#[cfg(test)]
mod download_locks {
    use super::engine_visibility_locks::{body_of, product_lines};
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Set on the CHILD half of the directory-policy lock: seeing it means
    /// "print what `downloads_dir()` decided and stop", which is how a test can
    /// own `HOME` without racing the seven other test threads.
    const DOWNLOADS_DIR_PROBE_VAR: &str = "YGGTERM_DOWNLOADS_DIR_PROBE";

    /// How the child's answer is picked out of libtest's own output.
    const DOWNLOADS_DIR_PROBE_PREFIX: &str = "yggterm-downloads-dir-probe: ";

    /// A scratch directory that stands in for `~/Downloads` for one test.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "yggterm-download-lock-{}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
                label
            );
            let dir = std::env::temp_dir().join(unique);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch downloads dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Answer exactly one request with `headers` (CRLF-joined, no terminator)
    /// and then `body_written` bytes of `body`. Writing FEWER bytes than the
    /// `Content-Length` the headers announce and then closing is how a transfer
    /// is killed mid-flight without a mock in sight.
    fn serve_once(headers: &str, body: Vec<u8>, body_written: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
        let addr = listener.local_addr().expect("fixture addr");
        let headers = headers.to_string();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let _ = stream.write_all(headers.as_bytes());
            let _ = stream.write_all(b"\r\n\r\n");
            let _ = stream.write_all(&body[..body_written.min(body.len())]);
            let _ = stream.flush();
            // Dropping the stream here IS the close: a short body followed by a
            // close is exactly what a server dying mid-transfer looks like.
        });
        format!("http://{addr}/file")
    }

    /// What a client got from the fixture. The `Content-Disposition` parse is
    /// the ENGINE'S job in production (WebKit hands the parsed name to
    /// `decide-destination`); it happens here only to feed a real server's real
    /// header into the real policy.
    struct Fetched {
        suggested_filename: String,
        announced_length: Option<usize>,
        body: Vec<u8>,
    }

    fn fetch(url: &str) -> Fetched {
        let addr = url
            .trim_start_matches("http://")
            .split('/')
            .next()
            .expect("fixture host");
        let mut stream = TcpStream::connect(addr).expect("connect to fixture");
        let request =
            format!("GET /file HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .expect("send request");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("read response");
        let split = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("response has a header terminator");
        let headers = String::from_utf8_lossy(&raw[..split]).to_string();
        let body = raw[split + 4..].to_vec();
        let suggested_filename = headers
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("content-disposition:"))
            .and_then(|line| line.split("filename=").nth(1))
            .map(|value| value.trim().trim_matches('"').to_string())
            .unwrap_or_default();
        let announced_length = headers
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
            .and_then(|line| line.split(':').nth(1))
            .and_then(|value| value.trim().parse::<usize>().ok());
        Fetched {
            suggested_filename,
            announced_length,
            body,
        }
    }

    /// A REAL download: a server on localhost answers with a real
    /// `Content-Disposition`, and the bytes it sent end up in the downloads
    /// directory under the name the policy chose, byte for byte.
    ///
    /// The transfer itself is WebKit's in production; what this drives is the
    /// decision WebKit asks for and then obeys — `download_destination` answers
    /// `decide-destination`, and the engine writes the response body to exactly
    /// that path.
    #[test]
    fn a_real_download_lands_in_the_downloads_directory_with_its_content() {
        let dir = ScratchDir::new("lands");
        let body = b"%PDF-1.7\nquarterly numbers\n".to_vec();
        let url = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\n\
             Content-Disposition: attachment; filename=\"quarterly report.pdf\"\r\n\
             Content-Length: 26",
            body.clone(),
            body.len(),
        );
        let fetched = fetch(&url);
        assert_eq!(fetched.suggested_filename, "quarterly report.pdf");
        assert_eq!(fetched.body, body, "the fixture served its whole body");

        let destination =
            download_destination(dir.path(), &fetched.suggested_filename, &download_name_is_taken);
        std::fs::write(&destination, &fetched.body).expect("engine writes to the destination");

        assert_eq!(
            destination,
            dir.path().join("quarterly report.pdf"),
            "a download must land in the downloads directory under its own name",
        );
        assert_eq!(
            std::fs::read(&destination).expect("downloaded file"),
            body,
            "the file on disk must be the bytes the server sent",
        );
    }

    /// The suggested name is ATTACKER CONTROLLED. A server answering
    /// `filename="../../../../etc/passwd"` does not get to name a path; it gets
    /// to name a file, inside the downloads directory, and nowhere else.
    #[test]
    fn a_malicious_suggested_filename_cannot_escape_the_downloads_directory() {
        let dir = ScratchDir::new("escape");
        let body = b"pwned".to_vec();
        let url = serve_once(
            "HTTP/1.1 200 OK\r\n\
             Content-Disposition: attachment; filename=\"../../../../etc/passwd\"\r\n\
             Content-Length: 5",
            body.clone(),
            body.len(),
        );
        let fetched = fetch(&url);
        assert_eq!(fetched.suggested_filename, "../../../../etc/passwd");
        let destination =
            download_destination(dir.path(), &fetched.suggested_filename, &download_name_is_taken);
        assert_eq!(
            destination,
            dir.path().join("passwd"),
            "a traversal in the suggested name must become a plain basename",
        );
        assert_eq!(
            destination.parent(),
            Some(dir.path()),
            "the destination's parent is the downloads directory, always",
        );

        // The rest of the hostile shapes, each there for its own reason.
        for (suggested, expected) in [
            ("../../x", "x"),
            ("..\\..\\windows.exe", "windows.exe"),
            ("/etc/shadow", "shadow"),
            (".bashrc", "bashrc"),
            ("..", DOWNLOAD_FALLBACK_NAME),
            (".", DOWNLOAD_FALLBACK_NAME),
            ("", DOWNLOAD_FALLBACK_NAME),
            ("   ", DOWNLOAD_FALLBACK_NAME),
            ("/", DOWNLOAD_FALLBACK_NAME),
            ("....//", DOWNLOAD_FALLBACK_NAME),
            ("evil\u{0}.sh", "evil.sh"),
            ("re\nport.txt", "report.txt"),
        ] {
            assert_eq!(
                sanitize_download_file_name(suggested),
                expected,
                "`{suggested}` must sanitize to `{expected}`",
            );
            let landed = download_destination(dir.path(), suggested, &|_| false);
            assert_eq!(
                landed.parent(),
                Some(dir.path()),
                "`{suggested}` escaped the downloads directory",
            );
        }

        // A name so long the filesystem would refuse it is truncated here, not
        // passed through to fail at `open` with nothing the user can act on.
        let long = format!("{}.bin", "a".repeat(400));
        assert!(sanitize_download_file_name(&long).len() <= DOWNLOAD_NAME_MAX_BYTES);
    }

    /// Downloading the same file twice must never destroy the first one. The
    /// browser idiom — `report.pdf`, then `report (1).pdf` — and the file
    /// already on disk is byte-identical afterwards.
    #[test]
    fn a_collision_uniquifies_rather_than_overwriting() {
        let dir = ScratchDir::new("collision");
        let first = dir.path().join("report.pdf");
        std::fs::write(&first, b"the original").expect("pre-place the first download");

        let second = download_destination(dir.path(), "report.pdf", &download_name_is_taken);
        assert_eq!(
            second,
            dir.path().join("report (1).pdf"),
            "a taken name must uniquify",
        );
        std::fs::write(&second, b"the second").expect("write the second download");
        assert_eq!(
            std::fs::read(&first).expect("the first download"),
            b"the original",
            "the file already on disk must be untouched",
        );

        let third = download_destination(dir.path(), "report.pdf", &download_name_is_taken);
        assert_eq!(
            third,
            dir.path().join("report (2).pdf"),
            "the counter must keep climbing, not reuse (1)",
        );

        // Multi-dot names keep their whole extension: `archive (1).tar.gz`, not
        // `archive.tar (1).gz`.
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, b"x").expect("pre-place an archive");
        assert_eq!(
            download_destination(dir.path(), "archive.tar.gz", &download_name_is_taken),
            dir.path().join("archive (1).tar.gz"),
        );

        // A name with no extension still uniquifies.
        let plain = dir.path().join("LICENSE");
        std::fs::write(&plain, b"x").expect("pre-place a plain file");
        assert_eq!(
            download_destination(dir.path(), "LICENSE", &download_name_is_taken),
            dir.path().join("LICENSE (1)"),
        );
    }

    /// A transfer killed mid-flight: the fixture announces 4096 bytes, writes
    /// 11, and closes. The event must NAME the reason, and — the part that
    /// matters on disk — the truncated file must be gone. WebKitGTK writes
    /// straight to the destination, so a partial left behind is a file with the
    /// right name and the wrong contents: a download that masquerades as
    /// complete.
    #[test]
    fn a_transfer_killed_mid_flight_names_its_reason_and_leaves_no_partial() {
        let dir = ScratchDir::new("killed");
        let body = vec![b'z'; 4096];
        let url = serve_once(
            "HTTP/1.1 200 OK\r\n\
             Content-Disposition: attachment; filename=\"big.iso\"\r\n\
             Content-Length: 4096",
            body,
            11,
        );
        let fetched = fetch(&url);
        assert_eq!(fetched.announced_length, Some(4096));
        assert_eq!(
            fetched.body.len(),
            11,
            "the fixture died after 11 bytes, which is the whole point",
        );

        let destination =
            download_destination(dir.path(), &fetched.suggested_filename, &download_name_is_taken);
        // What the engine has already done by the time it gives up: a partial
        // file sitting at the destination, which it told us it created
        // (`created-destination`) before it started writing.
        std::fs::write(&destination, &fetched.body).expect("partial on disk");
        assert!(destination.exists());

        let event = finish_download_transfer(
            Some(7),
            url.clone(),
            "big.iso".to_string(),
            destination.clone(),
            fetched.body.len() as u64,
            Some("Error reading from the underlying stream".to_string()),
            true,
        );

        assert_eq!(
            event.phase,
            SurfaceDownloadPhase::Failed {
                reason: "Error reading from the underlying stream".to_string(),
            },
            "a killed transfer must fail with the engine's own reason, not a \
             generic apology",
        );
        assert_eq!(event.file_name, "big.iso", "the failure must name the file");
        assert!(
            !destination.exists(),
            "the truncated file is still on disk, masquerading as a complete \
             download",
        );

        // The other direction, so the sweep cannot simply always run: a
        // transfer that ENDED WELL keeps its file and reports its size.
        let good = dir.path().join("good.iso");
        std::fs::write(&good, b"complete").expect("finished download");
        let event = finish_download_transfer(
            Some(7),
            url,
            "good.iso".to_string(),
            good.clone(),
            8,
            None,
            true,
        );
        assert_eq!(event.phase, SurfaceDownloadPhase::Completed { bytes: 8 });
        assert!(good.exists(), "a completed download must keep its file");
    }

    /// THE OWNERSHIP GATE on the sweep, which is the one place in this plane
    /// where a bug DESTROYS DATA rather than misplacing it.
    ///
    /// `decide-destination` sets `set_allow_overwrite(false)`, so "the
    /// destination already exists" is a first-class WebKit failure — and it is
    /// exactly the failure in which the file at the destination is SOMEBODY
    /// ELSE'S: a sibling transfer that decided the same name in the same
    /// main-loop turn (the uniquifier reads the directory, and WebKit does not
    /// create the file until after `decide-destination` returns), or any other
    /// process that wrote it in that window. A sweep that fires on every
    /// failure deletes that file.
    ///
    /// The engine answers the question itself — `created-destination` fires
    /// when and only when WebKit created the file — and that answer is what
    /// gates the `remove_file`.
    #[test]
    fn a_failure_sweeps_only_a_file_this_transfer_created() {
        let dir = ScratchDir::new("ownership");

        // Somebody else's file, at the name this transfer wanted. The transfer
        // never got as far as `created-destination`, so it never owned it.
        let theirs = dir.path().join("report.pdf");
        std::fs::write(&theirs, b"someone else's report").expect("pre-place a stranger's file");
        let event = finish_download_transfer(
            Some(4),
            "https://example.test/report.pdf".to_string(),
            "report.pdf".to_string(),
            theirs.clone(),
            0,
            Some("Cannot determine destination URI".to_string()),
            false,
        );
        assert_eq!(
            event.phase,
            SurfaceDownloadPhase::Failed {
                reason: "Cannot determine destination URI".to_string(),
            },
        );
        assert!(
            theirs.exists(),
            "a failed download deleted a file it never created — the \
             `allow_overwrite(false)` failure mode means the file at the \
             destination belongs to someone else",
        );
        assert_eq!(
            std::fs::read(&theirs).expect("the stranger's file"),
            b"someone else's report",
            "the stranger's file was replaced rather than left alone",
        );

        // And the other direction, so the gate cannot simply be "never sweep":
        // a partial THIS transfer created is still swept.
        let ours = dir.path().join("ours.iso");
        std::fs::write(&ours, b"half a").expect("our own partial");
        let event = finish_download_transfer(
            Some(4),
            "https://example.test/ours.iso".to_string(),
            "ours.iso".to_string(),
            ours.clone(),
            6,
            Some("Error reading from the underlying stream".to_string()),
            true,
        );
        assert!(matches!(event.phase, SurfaceDownloadPhase::Failed { .. }));
        assert!(
            !ours.exists(),
            "the partial this transfer created must still be swept — otherwise \
             a truncated file masquerades as a complete download",
        );
    }

    /// THE DIRECTORY POLICY, driven for real. `downloads_dir()` itself is
    /// called — not a scratch dir injected past it — under a `HOME` this test
    /// owns, with `XDG_DOWNLOAD_DIR` pointed at a decoy.
    ///
    /// In a CHILD PROCESS, because the environment is process-global and this
    /// binary runs its tests eight at a time: mutating `HOME` in-process would
    /// be a race against every other test in the crate. The child is this same
    /// test binary, re-entered on this same test name, which is why the
    /// function it prints is the production one and not a copy of the rule.
    #[test]
    fn the_downloads_directory_is_home_downloads_and_ignores_xdg_download_dir() {
        // The child half: say what production decided, and stop.
        if std::env::var_os(DOWNLOADS_DIR_PROBE_VAR).is_some() {
            println!("{DOWNLOADS_DIR_PROBE_PREFIX}{}", downloads_dir().display());
            return;
        }

        let home = ScratchDir::new("home");
        let decoy = ScratchDir::new("xdg-decoy");
        // Derived, not spelled: a renamed module or test must not silently turn
        // this lock into a no-op that "passes" because nothing matched.
        let test_name = format!(
            "{}::the_downloads_directory_is_home_downloads_and_ignores_xdg_download_dir",
            module_path!()
                .split_once("::")
                .map(|(_crate_name, path)| path)
                .expect("this test lives inside a module"),
        );
        let output = std::process::Command::new(
            std::env::current_exe().expect("this test binary's own path"),
        )
        .args(["--exact", "--nocapture", "--test-threads=1", &test_name])
        .env(DOWNLOADS_DIR_PROBE_VAR, "1")
        .env("HOME", home.path())
        .env("XDG_DOWNLOAD_DIR", decoy.path())
        .output()
        .expect("re-run this test as a child process");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        assert!(
            stdout.contains("1 passed"),
            "the child did not run this test (filter drift?):\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        // `--nocapture` interleaves the print with libtest's own progress line,
        // so the answer is found mid-line and read to the end of it.
        let decided = stdout
            .split_once(DOWNLOADS_DIR_PROBE_PREFIX)
            .map(|(_, rest)| rest.lines().next().unwrap_or_default().trim())
            .filter(|answer| !answer.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("the child printed no decision:\n{stdout}"));

        assert_eq!(
            decided,
            home.path().join(DOWNLOADS_DIR_NAME),
            "downloads must land in $HOME/Downloads",
        );
        assert!(
            decided.is_dir(),
            "the downloads directory must be CREATED when missing — a policy \
             that names a directory nobody made fails at the first `open`",
        );
        assert!(
            !decided.starts_with(decoy.path()),
            "XDG_DOWNLOAD_DIR was honoured: the downloads folder now moves \
             depending on how the GUI was launched, which is the exact \
             non-determinism this policy refuses",
        );
    }

    /// `Path::exists` FOLLOWS symlinks, so a DANGLING one reads as a free name.
    /// A symlink planted at `~/Downloads/report.pdf` pointing outside the
    /// directory would then be handed to the engine as the destination and the
    /// write would land wherever it points — the traversal that
    /// `sanitize_download_file_name` exists to prevent, arriving by the other
    /// door.
    #[test]
    fn a_dangling_symlink_at_the_name_is_still_a_taken_name() {
        let dir = ScratchDir::new("symlink");
        let outside = dir.path().join("not-created-yet-outside");
        let planted = dir.path().join("report.pdf");
        std::os::unix::fs::symlink(&outside, &planted).expect("plant a dangling symlink");
        assert!(
            !planted.exists(),
            "the fixture is wrong: a dangling symlink must read as absent to \
             `Path::exists`, which is the whole hazard",
        );
        assert!(
            planted.symlink_metadata().is_ok(),
            "the fixture is wrong: the link itself must be on disk",
        );

        assert!(
            download_name_is_taken(&planted),
            "a dangling symlink is a name that is TAKEN — writing to it writes \
             through it, outside the downloads directory",
        );
        // The damage, driven: take the path the policy returns and write to it
        // exactly as the engine would.
        let landed = download_destination(dir.path(), "report.pdf", &download_name_is_taken);
        std::fs::write(&landed, b"payload").expect("the engine writes to the destination");
        assert!(
            !outside.exists(),
            "the download was written THROUGH the planted symlink and landed at \
             {}, outside the downloads directory (destination was {})",
            outside.display(),
            landed.display(),
        );
        assert_eq!(
            landed,
            dir.path().join("report (1).pdf"),
            "the uniquifier must step past a planted symlink, not hand the \
             engine a path that resolves outside the downloads directory",
        );

        // A live symlink is taken too, for the same reason and by the same rule.
        let target = dir.path().join("real.bin");
        std::fs::write(&target, b"x").expect("link target");
        let live = dir.path().join("linked.bin");
        std::os::unix::fs::symlink(&target, &live).expect("plant a live symlink");
        assert!(download_name_is_taken(&live));
    }

    /// The WIRING. Every decision above is worth nothing if WebKit is never
    /// asked, is asked twice, or is answered by someone else. Needles are
    /// anchored to the enclosing body, so an APPEND elsewhere in the file
    /// cannot satisfy them.
    #[test]
    fn the_engine_is_wired_to_this_download_policy_and_to_no_other() {
        let product = product_lines();
        assert!(
            !product.iter().any(|line| line.contains("mod download_locks")),
            "the scan is reading this test module, so every needle below would be \
             satisfied by the assertion that names it",
        );

        // 1. `open` connects the plumbing, ONCE PER CONTEXT. Dropping the guard
        //    would decide one transfer once per tab of the session.
        let open = body_of(&product, "pub fn open(");
        assert!(
            open.contains("if context_is_new {") && open.contains("connect_download_plumbing("),
            "`open` no longer connects the download plumbing: with the vendored \
             wry's default handler switched off, nothing in this process answers \
             `decide-destination` and a download link does nothing at all",
        );

        // 2. The destination answer: our policy, our path, and `true` so the
        //    signal is HANDLED.
        let plumbing = body_of(&product, "fn connect_download_plumbing(");
        for needle in [
            "context.connect_download_started(move |_context, download| {",
            "download.connect_decide_destination({",
            // The DIRECTORY, at the call site — swapping this for any other
            // path is the mutation the destination locks cannot see, because
            // they are handed a directory rather than choosing one.
            "let dir = downloads_dir();",
            // ...and the TAKEN test, which is `symlink_metadata` and not
            // `Path::exists` for the dangling-symlink reason.
            "download_destination(&dir, suggested_filename, &download_name_is_taken);",
            "download.set_allow_overwrite(false);",
            "download.set_destination(&destination.to_string_lossy());",
            "phase: SurfaceDownloadPhase::Started,",
            // OWNERSHIP of the destination file, straight from the engine: what
            // gates the failure sweep, so a failure never deletes a file this
            // transfer did not create.
            "download.connect_created_destination({",
            "created_destination.set(true);",
            "created_destination.get(),",
            // The failure REASON, taken from the engine's own error.
            "*failure.borrow_mut() = Some(error.to_string());",
            // Exactly one terminal event, through the one function that decides
            // what a transfer became.
            "events.borrow_mut().push(finish_download_transfer(",
            "download.received_data_length(),",
            "failure.borrow_mut().take(),",
            // The detach rule: the engine is held for the transfer's lifetime,
            // weakly by the handler so the context is not immortal.
            "let ctx_weak = Rc::downgrade(ctx_cell);",
            "in_flight.borrow_mut().push(DownloadInFlight {",
            // Releasing that hold from inside the signal would drop the engine
            // under WebKit's feet.
            "gtk::glib::idle_add_local_once(move || {",
        ] {
            assert!(
                plumbing.contains(needle),
                "the download plumbing no longer does `{needle}`:\n{plumbing}",
            );
        }

        // 3. Nobody else may answer `decide-destination`. The vendored wry
        //    installs a default handler that computes its own destination with
        //    `PathBuf::push` (which a `../../` name walks out of) and returns
        //    `true`, stopping ours; it is switched OFF at the source, and this
        //    is the lock that survives a re-vendor.
        let wry = include_str!("../../wry/src/lib.rs");
        assert!(
            wry.contains("      download_started_handler: None,\n"),
            "wry's default download handler is back: there are now two download \
             policies on one signal, and the unsanitized one wins",
        );
        assert!(
            !wry.contains("download_started_handler: Some(Box::new(|_, _| true)),"),
            "wry's upstream allow-everything download handler has returned",
        );
        // The re-arm is an OR (`webkitgtk/mod.rs`: register if EITHER handler
        // is some), so a default COMPLETED handler would put wry's whole
        // download path back on the context even with the started one off.
        assert!(
            wry.contains("      download_completed_handler: None,\n"),
            "wry now defaults a download COMPLETED handler, which re-arms its \
             own `register_download_handler` — a second policy on one signal by \
             the other door",
        );

        // 5. POPUPS are covered by the same one policy, with no wiring of their
        //    own: a popup is built RELATED to its opener, and a related view
        //    shares the opener's `WebKitWebContext` — which is where
        //    `download-started` is connected. If either half drifts, a download
        //    started from a `target="_blank"` window silently has no policy.
        let popup = body_of(&product, "fn build_popup_webview(");
        assert!(
            popup.contains(".with_related_view(opener.clone())"),
            "a popup is no longer built RELATED to its opener, so it gets a \
             fresh context — and the download plumbing, which is connected per \
             context, does not cover it",
        );
        let wry_gtk = include_str!("../../wry/src/webkitgtk/mod.rs");
        assert!(
            wry_gtk.contains(
                "if let Some(related_view) = &pl_attrs.related_view {\n      \
                 builder = builder.related_view(related_view);"
            ),
            "wry no longer builds a related view from `related_view`, so a \
             popup would not share its opener's context",
        );

        // 4. The read side exists and DRAINS — a queue nobody empties is a leak
        //    that also tells the user nothing. Anchored to the signature's own
        //    line, so the drain cannot be satisfied by a `mem::take` elsewhere.
        assert!(
            product.join("\n").contains(
                "pub fn take_downloads(&self) -> Vec<SurfaceDownloadEvent> {\n        \
                 std::mem::take(&mut self.downloads.borrow_mut())\n    }"
            ),
            "the download queue is no longer drained",
        );
    }

    /// THE DETACH RULE, DRIVEN — the tab closes mid-transfer and the engine is
    /// still standing afterwards.
    ///
    /// The production registry entry (`DownloadInFlight`) and the production
    /// sweep (`retain_held_contexts`, which is all `prune_contexts` does) are
    /// the ones under test. The only stand-in is the thing being kept alive: a
    /// real `WebContext` needs a display and a network process this host does
    /// not have, and `Rc::strong_count` — the entire mechanism — cannot tell a
    /// stand-in from an engine.
    ///
    /// What this does NOT prove, stated so nobody reads more into it: no real
    /// WebKit transfer has ever been observed continuing past its view's
    /// destruction. That half is the engine's, needs a display, and is listed
    /// as live proof still owed.
    #[test]
    fn a_running_download_keeps_its_engine_alive_after_the_tab_is_gone() {
        // The engine, as three holders see it: the host's context map, the tab
        // that opened it, and the transfer it started.
        let engine = Rc::new(RefCell::new(String::from("session-a engine")));
        let mut contexts: HashMap<String, Rc<RefCell<String>>> = HashMap::new();
        contexts.insert("session-a".to_string(), engine.clone());
        let tab_hold = engine.clone();
        let mut in_flight: Vec<DownloadInFlight<RefCell<String>>> = vec![DownloadInFlight {
            id: 1,
            _ctx: engine.clone(),
        }];
        drop(engine);

        // Steady state: a tab, a transfer, one engine.
        retain_held_contexts(&mut contexts);
        assert_eq!(contexts.len(), 1, "the sweep took a context still in use");

        // THE TAB CLOSES, mid-transfer. This is the whole case.
        drop(tab_hold);
        retain_held_contexts(&mut contexts);
        assert_eq!(
            contexts.len(),
            1,
            "the engine was swept out from under a running download — closing \
             the tab now truncates the file",
        );
        let held = contexts.get("session-a").expect("the engine is still mapped");
        assert_eq!(
            Rc::strong_count(held),
            2,
            "the only holders left must be the map and the transfer itself",
        );

        // The transfer ends and releases its hold; the NEXT sweep takes the
        // engine, exactly as it did before downloads existed.
        in_flight.clear();
        retain_held_contexts(&mut contexts);
        assert!(
            contexts.is_empty(),
            "an engine nobody holds must not outlive its last holder — the \
             download hold is a LIFETIME, not immortality",
        );

        // And the sweep the host runs is this rule and not a second spelling of
        // it: `prune_contexts` is the borrow around it and nothing more.
        let product = product_lines();
        assert!(
            product.join("\n").contains(
                "fn prune_contexts(&self) {\n        \
                 retain_held_contexts(&mut self.contexts.borrow_mut());\n    }"
            ),
            "the host's context sweep no longer goes through the rule under \
             test, so what is proven above is no longer what runs",
        );

        // The registry entry holds the context strongly (that IS the hold) and
        // never holds the `Download` itself (whose last reference must stay
        // WebKit's).
        let source = product.join("\n");
        let start = source
            .find("struct DownloadInFlight<C = RefCell<WebContext>> {")
            .expect("`DownloadInFlight` is gone, or no longer defaults to the engine type");
        let end = source[start..]
            .find("\n}\n")
            .map(|offset| start + offset)
            .expect("unterminated `DownloadInFlight`");
        let entry = &source[start..end];
        assert!(
            entry.contains("_ctx: Rc<C>,"),
            "an in-flight download no longer holds its engine: closing the tab \
             now truncates the file",
        );
        assert!(
            !entry.contains("webkit2gtk::Download"),
            "the registry holds a `Download` handle again — dropping the last \
             reference to one inside its own signal handler is a use-after-free",
        );
    }
}
