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

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read as _, Write as _};

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

struct Surface {
    // The overlay child that positions the webview. wry `build_gtk`s the webview
    // into this Fixed (put at 0,0); the Fixed is placed in the overlay via
    // margin-start/top + size-request.
    container: gtk::Fixed,
    webview: wry::WebView,
    // wry requires the WebContext to outlive the webview; co-own it here.
    _ctx: WebContext,
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

/// Owns the main window's `gtk::Overlay` and the set of live surface webviews.
/// Held (Linux only) on `DesktopService`; driven from the shell via the
/// `open_web_surface` / `web_surface_*` methods on `DesktopContext`.
pub struct WebSurfaceHost {
    overlay: gtk::Overlay,
    surfaces: RefCell<HashMap<u64, Surface>>,
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
fn apply_bounds(surface: &Surface, x: i32, y: i32, w: i32, h: i32) {
    use wry::WebViewExtUnix as _;
    let (w, h) = (w.max(1), h.max(1));
    surface.container.set_margin_start(x.max(0));
    surface.container.set_margin_top(y.max(0));
    surface.container.set_size_request(w, h);
    surface.webview.webview().set_size_request(w, h);
    let _ = surface.webview.set_bounds(rect_logical(w, h));
}

impl WebSurfaceHost {
    pub(crate) fn new(overlay: gtk::Overlay) -> Self {
        Self {
            overlay,
            surfaces: RefCell::new(HashMap::new()),
        }
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
    /// cosmetic hiding, engine-native) is attached to this surface. Bounds are
    /// logical pixels relative to the window's top-left.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        id: u64,
        url: &str,
        socks_port: Option<u16>,
        profile_dir: Option<&std::path::Path>,
        userscripts: &[String],
        adblock_ruleset: Option<&std::path::Path>,
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
        self.surfaces.borrow_mut().insert(
            id,
            Surface {
                container,
                webview,
                _ctx: ctx,
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

    /// Current page (uri, title) as the ENGINE reports them. In-page
    /// navigations (link clicks, redirects, pushState) never pass through the
    /// shell's nav model, so this is the only truth for "where is this tab
    /// now"; the shell polls it to keep the address bar, tab titles and
    /// history honest.
    pub fn page_state(&self, id: u64) -> Option<(String, String)> {
        use webkit2gtk::WebViewExt as _;
        use wry::WebViewExtUnix as _;
        self.surfaces.borrow().get(&id).map(|s| {
            let webkit = s.webview.webview();
            (
                webkit.uri().map(|u| u.to_string()).unwrap_or_default(),
                webkit.title().map(|t| t.to_string()).unwrap_or_default(),
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
