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

use gtk::prelude::*;
use wry::{
    dpi::{LogicalPosition, LogicalSize, Position, Size},
    ProxyConfig, ProxyEndpoint, Rect, WebContext, WebViewBuilder,
};

struct Surface {
    // The overlay child that positions the webview. wry `build_gtk`s the webview
    // into this Fixed (put at 0,0); the Fixed is placed in the overlay via
    // margin-start/top + size-request.
    container: gtk::Fixed,
    webview: wry::WebView,
    // wry requires the WebContext to outlive the webview; co-own it here.
    _ctx: WebContext,
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
    /// localStorage); `None` = ephemeral. Bounds are logical pixels relative to
    /// the window's top-left.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        id: u64,
        url: &str,
        socks_port: Option<u16>,
        profile_dir: Option<&std::path::Path>,
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

        let webview = {
            use wry::WebViewBuilderExtUnix;
            builder
                .build_gtk(&container)
                .map_err(|e| format!("build surface webview: {e}"))?
        };
        container.show_all();

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
            s.container.set_margin_start(x.max(0));
            s.container.set_margin_top(y.max(0));
            s.container.set_size_request(w.max(1), h.max(1));
            let _ = s.webview.set_bounds(rect_logical(w, h));
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

    pub fn close(&self, id: u64) {
        if let Some(s) = self.surfaces.borrow_mut().remove(&id) {
            self.overlay.remove(&s.container);
            // Surface drops here: webview + WebContext torn down together.
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
