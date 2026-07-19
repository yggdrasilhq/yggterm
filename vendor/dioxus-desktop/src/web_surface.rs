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
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
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
    // wry requires the WebContext to outlive the webview; co-own it here. A
    // POPUP has none of its own: it is built RELATED to its opener, which means
    // it shares the opener's context (its jar, its proxy, its web process) —
    // that sharing is exactly what a popup is.
    _ctx: Option<WebContext>,
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

/// Owns the main window's `gtk::Overlay` and the set of live surface webviews.
/// Held (Linux only) on `DesktopService`; driven from the shell via the
/// `open_web_surface` / `web_surface_*` methods on `DesktopContext`.
pub struct WebSurfaceHost {
    overlay: gtk::Overlay,
    /// Style provider on the overlay's base child (the native backdrop):
    /// `set_backdrop_color` reloads it with the theme background color.
    backdrop_css: gtk::CssProvider,
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
    popup_id: u64,
    opener: &webkit2gtk::WebView,
    opener_bounds: (i32, i32, i32, i32),
    visible: bool,
    userscripts: &[String],
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
    overlay.add_overlay(&container);
    restack_glass(overlay, glass);
    container.show();

    let mut builder = WebViewBuilder::new()
        .with_bounds(rect_logical(w, h))
        .with_devtools(true)
        // NO url: WebKit loads the request that asked for this window into the
        // view we hand back. Loading it ourselves would race that navigation.
        .with_related_view(opener.clone())
        .with_initialization_script_for_main_only(CLOSE_SHIM_JS, true);
    for script in userscripts {
        builder = builder.with_initialization_script_for_main_only(script.as_str(), true);
    }
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
        },
    );
    Some(webkit)
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
            glass: Rc::new(RefCell::new(None)),
            last_glass_holes: RefCell::new(None),
            surfaces: Rc::new(RefCell::new(HashMap::new())),
            next_id: Rc::new(Cell::new(1)),
            popups: Rc::new(RefCell::new(Vec::new())),
            close_requests: Rc::new(RefCell::new(Vec::new())),
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
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        id: u64,
        url: &str,
        socks_port: Option<u16>,
        profile_dir: Option<&std::path::Path>,
        userscripts: &[String],
        adblock_ruleset: Option<&std::path::Path>,
        user_agent: Option<&str>,
        signer_base: Option<&str>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<(), String> {
        // Replace any existing surface with this id.
        self.close(id);

        let container = gtk::Fixed::new();
        container.set_halign(gtk::Align::Start);
        container.set_valign(gtk::Align::Start);
        container.set_margin_start(x.max(0));
        container.set_margin_top(y.max(0));
        container.set_size_request(w.max(1), h.max(1));
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
        let mut ctx = match profile_dir {
            Some(dir) => WebContext::new(Some(dir.to_path_buf())),
            None => WebContext::new_ephemeral(),
        };
        // Devtools are always available on surfaces: the agent is a first-class
        // user and drives pages through the inspector/eval; the user opens it
        // per surface. (WebKitGTK: enables developer extras; the inspector
        // itself only appears via `set_devtools_open`.)
        let mut builder = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_bounds(rect_logical(w, h))
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
        for script in userscripts {
            builder = builder.with_initialization_script_for_main_only(script.as_str(), true);
        }
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
        if let Some(base) = signer_base {
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
            builder
                .build_gtk(&container)
                .map_err(|e| format!("build surface webview: {e}"))?
        };
        container.show_all();

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
                _ctx: Some(ctx),
            },
        );
        Ok(())
    }

    pub fn set_bounds(&self, id: u64, x: i32, y: i32, w: i32, h: i32) {
        if let Some(s) = self.surfaces.borrow().get(&id) {
            apply_bounds(s, x, y, w, h);
            self.overlay.queue_resize();
        }
    }

    pub fn set_visible(&self, id: u64, visible: bool) {
        if let Some(s) = self.surfaces.borrow().get(&id) {
            let _ = s.webview.set_visible(visible);
            s.container.set_visible(visible);
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

    pub fn close(&self, id: u64) {
        if let Some(s) = self.surfaces.borrow_mut().remove(&id) {
            // A stashed surface's container is already detached.
            if s.container.parent().is_some() {
                self.overlay.remove(&s.container);
            }
            // Surface drops here: webview + WebContext torn down together.
        }
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

    /// Re-attach a stashed surface at the given bounds and show it.
    pub fn unstash(&self, id: u64, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        let surfaces = self.surfaces.borrow();
        let s = surfaces.get(&id).ok_or("no such surface")?;
        if s.container.parent().is_none() {
            self.overlay.add_overlay(&s.container);
            restack_glass(&self.overlay, &self.glass);
        }
        apply_bounds(s, x, y, w, h);
        let _ = s.webview.set_visible(true);
        s.container.show_all();
        self.overlay.queue_resize();
        Ok(())
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
        callback: impl FnOnce(Result<String, String>) + 'static,
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
                Err(error) => Err(error.to_string()),
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
