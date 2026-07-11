use crate::{
    app::SharedContext,
    assets::AssetHandlerRegistry,
    file_upload::NativeFileHover,
    ipc::UserWindowEvent,
    query::QueryEngine,
    shortcut::{HotKey, HotKeyState, ShortcutHandle, ShortcutRegistryError},
    webview::PendingWebview,
    AssetRequest, Config, WindowCloseBehaviour, WryEventHandler,
};
use dioxus_core::{Callback, VirtualDom};
use std::{
    cell::Cell,
    future::{Future, IntoFuture},
    pin::Pin,
    rc::{Rc, Weak},
    sync::Arc,
};
use tao::{
    event::Event,
    event_loop::EventLoopWindowTarget,
    window::{Fullscreen as WryFullscreen, Window, WindowId},
};
use wry::{RequestAsyncResponder, WebView};

#[cfg(target_os = "ios")]
use tao::platform::ios::WindowExtIOS;

/// Get an imperative handle to the current window without using a hook
///
/// ## Panics
///
/// This function will panic if it is called outside of the context of a Dioxus App.
pub fn window() -> DesktopContext {
    dioxus_core::consume_context()
}

/// A handle to the [`DesktopService`] that can be passed around.
pub type DesktopContext = Rc<DesktopService>;

/// A weak handle to the [`DesktopService`] to ensure safe passing.
/// The problem without this is that the tao window is never dropped and therefore cannot be closed.
/// This was due to the Rc that had still references because of multiple copies when creating a webview.
pub type WeakDesktopContext = Weak<DesktopService>;

/// An imperative interface to the current window.
///
/// To get a handle to the current window, use the [`window`] function.
///
///
/// # Example
///
/// you can use `cx.consume_context::<DesktopContext>` to get this context
///
/// ```rust, ignore
///     let desktop = cx.consume_context::<DesktopContext>().unwrap();
/// ```
pub struct DesktopService {
    /// The wry/tao proxy to the current window
    pub webview: WebView,

    /// The tao window itself
    pub window: Arc<Window>,

    pub(crate) shared: Rc<SharedContext>,

    /// The receiver for queries about the current window
    pub(super) query: QueryEngine,
    pub(crate) asset_handlers: AssetHandlerRegistry,
    pub(crate) file_hover: NativeFileHover,
    pub(crate) close_behaviour: Rc<Cell<WindowCloseBehaviour>>,

    /// yggterm web surfaces (Linux/WebKitGTK): native child webviews layered
    /// over the main webview's page area. Installed once the main window's
    /// `gtk::Overlay` exists (see `WebviewInstance::new`).
    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    pub(crate) web_surface_host: std::cell::RefCell<Option<crate::web_surface::WebSurfaceHost>>,

    #[cfg(target_os = "ios")]
    pub(crate) views: Rc<std::cell::RefCell<Vec<*mut objc::runtime::Object>>>,
}

/// A smart pointer to the current window.
impl std::ops::Deref for DesktopService {
    type Target = Window;

    fn deref(&self) -> &Self::Target {
        &self.window
    }
}

impl DesktopService {
    pub(crate) fn new(
        webview: WebView,
        window: Arc<Window>,
        shared: Rc<SharedContext>,
        asset_handlers: AssetHandlerRegistry,
        file_hover: NativeFileHover,
        close_behaviour: WindowCloseBehaviour,
    ) -> Self {
        Self {
            window,
            webview,
            shared,
            asset_handlers,
            file_hover,
            close_behaviour: Rc::new(Cell::new(close_behaviour)),
            query: Default::default(),
            #[cfg(not(any(
                target_os = "windows",
                target_os = "macos",
                target_os = "ios",
                target_os = "android"
            )))]
            web_surface_host: std::cell::RefCell::new(None),
            #[cfg(target_os = "ios")]
            views: Default::default(),
        }
    }

    /// Install the web-surface host (Linux only; called once the main window's
    /// `gtk::Overlay` has been created in `WebviewInstance::new`).
    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    pub(crate) fn install_web_surface_host(&self, host: crate::web_surface::WebSurfaceHost) {
        *self.web_surface_host.borrow_mut() = Some(host);
    }

    /// Open (or replace) native web surface `id` over the page area, loading
    /// `url`, optionally egressing through `socks5://127.0.0.1:<port>` (the
    /// invoking host's tunnel). Bounds are logical pixels from the window's
    /// top-left. Errors on backends without the GTK/WebKit overlay path.
    #[allow(clippy::too_many_arguments)]
    pub fn open_web_surface(
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
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        {
            return match self.web_surface_host.borrow().as_ref() {
                Some(host) => host.open(
                    id,
                    url,
                    socks_port,
                    profile_dir,
                    userscripts,
                    adblock_ruleset,
                    user_agent,
                    signer_base,
                    x,
                    y,
                    w,
                    h,
                ),
                None => Err("web surface host not installed".to_string()),
            };
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        {
            let _ = (
                id,
                url,
                socks_port,
                profile_dir,
                userscripts,
                adblock_ruleset,
                user_agent,
                signer_base,
                x,
                y,
                w,
                h,
            );
            Err("web surfaces require the GTK/WebKit backend".to_string())
        }
    }

    /// Reposition/resize an open web surface (logical pixels from top-left).
    pub fn set_web_surface_bounds(&self, id: u64, x: i32, y: i32, w: i32, h: i32) {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        if let Some(host) = self.web_surface_host.borrow().as_ref() {
            host.set_bounds(id, x, y, w, h);
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let _ = (id, x, y, w, h);
    }

    /// Set the WebKit page-zoom factor for an open web surface (1.0 == 100%).
    pub fn set_web_surface_zoom(&self, id: u64, factor: f64) {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        if let Some(host) = self.web_surface_host.borrow().as_ref() {
            host.set_zoom(id, factor);
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let _ = (id, factor);
    }

    /// Show/hide an open web surface without destroying it (tab/session switch).
    pub fn set_web_surface_visible(&self, id: u64, visible: bool) {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        if let Some(host) = self.web_surface_host.borrow().as_ref() {
            host.set_visible(id, visible);
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let _ = (id, visible);
    }

    /// Navigate an open web surface to a new URL.
    pub fn navigate_web_surface(&self, id: u64, url: &str) {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        if let Some(host) = self.web_surface_host.borrow().as_ref() {
            host.navigate(id, url);
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let _ = (id, url);
    }

    /// Reload an open web surface's current page.
    pub fn reload_web_surface(&self, id: u64) {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        if let Some(host) = self.web_surface_host.borrow().as_ref() {
            host.reload(id);
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let _ = id;
    }

    /// Current (uri, title) of an open web surface's page, engine-reported.
    /// Follows in-page navigation the shell's nav model can't see.
    pub fn web_surface_page_state(&self, id: u64) -> Option<(String, String)> {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        {
            return self
                .web_surface_host
                .borrow()
                .as_ref()
                .and_then(|host| host.page_state(id));
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        {
            let _ = id;
            None
        }
    }

    /// Drain new-tab requests raised from inside surfaces (a link opened with
    /// middle-click / ctrl-click / `target="_blank"` / `window.open`). Each is
    /// `(surface_id, url, background)`; the shell opens the URL as a tab in the
    /// originating surface's session. See `WebSurfaceHost::take_new_tab_requests`.
    pub fn take_web_surface_new_tab_requests(&self) -> Vec<(u64, String, bool)> {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        {
            return self
                .web_surface_host
                .borrow()
                .as_ref()
                .map(|host| host.take_new_tab_requests())
                .unwrap_or_default();
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        {
            Vec::new()
        }
    }

    /// Destroy an open web surface.
    pub fn close_web_surface(&self, id: u64) {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        if let Some(host) = self.web_surface_host.borrow().as_ref() {
            host.close(id);
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let _ = id;
    }

    /// Stash an open web surface: detach it from the overlay, keeping the
    /// webview (page state) alive. See `WebSurfaceHost::stash`.
    pub fn stash_web_surface(&self, id: u64) -> Result<(), String> {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        {
            return match self.web_surface_host.borrow().as_ref() {
                Some(host) => host.stash(id),
                None => Err("web surface host not installed".to_string()),
            };
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        {
            let _ = id;
            Err("web surfaces require the GTK/WebKit backend".to_string())
        }
    }

    /// Re-attach a stashed web surface at the given bounds and show it.
    pub fn unstash_web_surface(&self, id: u64, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        {
            return match self.web_surface_host.borrow().as_ref() {
                Some(host) => host.unstash(id, x, y, w, h),
                None => Err("web surface host not installed".to_string()),
            };
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        {
            let _ = (id, x, y, w, h);
            Err("web surfaces require the GTK/WebKit backend".to_string())
        }
    }

    /// Evaluate JS in an open web surface's page; the callback gets the
    /// completion value as JSON, or the JS exception message.
    pub fn eval_web_surface(
        &self,
        id: u64,
        js: &str,
        callback: impl FnOnce(Result<String, String>) + 'static,
    ) -> Result<(), String> {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        {
            return match self.web_surface_host.borrow().as_ref() {
                Some(host) => host.eval(id, js, callback),
                None => Err("web surface host not installed".to_string()),
            };
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        {
            let _ = (id, js, callback);
            Err("web surfaces require the GTK/WebKit backend".to_string())
        }
    }

    /// Open/close the inspector (devtools) on an open web surface. Returns
    /// whether devtools are open after the call.
    pub fn set_web_surface_devtools(&self, id: u64, open: bool) -> Result<bool, String> {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        {
            return match self.web_surface_host.borrow().as_ref() {
                Some(host) => host.set_devtools_open(id, open),
                None => Err("web surface host not installed".to_string()),
            };
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        {
            let _ = (id, open);
            Err("web surfaces require the GTK/WebKit backend".to_string())
        }
    }

    /// Capture an open web surface's full document to a PNG at `path`; the
    /// callback fires when the file is written (or capture failed).
    pub fn snapshot_web_surface_full_page(
        &self,
        id: u64,
        path: std::path::PathBuf,
        callback: impl FnOnce(Result<(), String>) + 'static,
    ) -> Result<(), String> {
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        {
            return match self.web_surface_host.borrow().as_ref() {
                Some(host) => host.snapshot_full_page(id, path, callback),
                None => Err("web surface host not installed".to_string()),
            };
        }
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        {
            let _ = (id, path, callback);
            Err("web surfaces require the GTK/WebKit backend".to_string())
        }
    }

    /// Start the creation of a new window using the props and window builder
    ///
    /// Returns a future that resolves to the webview handle for the new window. You can use this
    /// to control other windows from the current window once the new window is created.
    ///
    /// Be careful to not create a cycle of windows, or you might leak memory.
    ///
    /// # Example
    ///
    /// ```rust, no_run
    /// use dioxus::prelude::*;
    /// fn popup() -> Element {
    ///     rsx! {
    ///         div { "This is a popup window!" }
    ///     }
    /// }
    ///
    /// # async fn app() {
    /// // Create a new window with a component that will be rendered in the new window.
    /// let dom = VirtualDom::new(popup);
    /// // Create and wait for the window
    /// let window = dioxus::desktop::window().new_window(dom, Default::default()).await;
    /// // Fullscreen the new window
    /// window.set_fullscreen(true);
    /// # }
    /// ```
    // Note: This method is asynchronous because webview2 does not support creating a new window from
    // inside of an existing webview callback. Dioxus runs event handlers synchronously inside of a webview
    // callback. See [this page](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/threading-model#reentrancy) for more information.
    //
    // Related issues:
    // - https://github.com/tauri-apps/wry/issues/583
    // - https://github.com/DioxusLabs/dioxus/issues/3080
    pub fn new_window(&self, dom: VirtualDom, cfg: Config) -> PendingDesktopContext {
        let (window, context) = PendingWebview::new(dom, cfg);

        self.shared
            .proxy
            .send_event(UserWindowEvent::NewWindow)
            .unwrap();

        self.shared.pending_webviews.borrow_mut().push(window);

        context
    }

    /// Create a sendable handle that wakes this window's event loop and polls the VirtualDom.
    pub fn poll_waker(&self) -> Arc<dyn Fn() + Send + Sync> {
        let proxy = self.shared.proxy.clone();
        let id = self.id();
        Arc::new(move || {
            let _ = proxy.send_event(UserWindowEvent::Poll(id));
        })
    }

    /// trigger the drag-window event
    ///
    /// Moves the window with the left mouse button until the button is released.
    ///
    /// you need use it in `onmousedown` event:
    /// ```rust, ignore
    /// onmousedown: move |_| { desktop.drag_window(); }
    /// ```
    pub fn drag(&self) {
        if self.window.fullscreen().is_none() {
            _ = self.window.drag_window();
        }
    }

    /// Toggle whether the window is maximized or not
    pub fn toggle_maximized(&self) {
        self.window.set_maximized(!self.window.is_maximized())
    }

    /// Set the close behavior of this window
    ///
    /// By default, windows close when the user clicks the close button.
    /// If this is set to `WindowCloseBehaviour::WindowHides`, the window will hide instead of closing.
    pub fn set_close_behavior(&self, behaviour: WindowCloseBehaviour) {
        self.close_behaviour.set(behaviour);
    }

    /// Close this window
    pub fn close(&self) {
        let _ = self
            .shared
            .proxy
            .send_event(UserWindowEvent::CloseWindow(self.id()));
    }

    /// Close a particular window, given its ID
    pub fn close_window(&self, id: WindowId) {
        let _ = self
            .shared
            .proxy
            .send_event(UserWindowEvent::CloseWindow(id));
    }

    /// change window to fullscreen
    pub fn set_fullscreen(&self, fullscreen: bool) {
        if let Some(handle) = &self.window.current_monitor() {
            self.window.set_fullscreen(
                fullscreen.then_some(WryFullscreen::Borderless(Some(handle.clone()))),
            );
        }
    }

    /// launch print modal
    pub fn print(&self) {
        if let Err(e) = self.webview.print() {
            tracing::warn!("Open print modal failed: {e}");
        }
    }

    /// Set the zoom level of the webview
    pub fn set_zoom_level(&self, level: f64) {
        if let Err(e) = self.webview.zoom(level) {
            tracing::warn!("Set webview zoom failed: {e}");
        }
    }

    /// opens DevTool window
    pub fn devtool(&self) {
        #[cfg(debug_assertions)]
        self.webview.open_devtools();

        #[cfg(not(debug_assertions))]
        tracing::warn!("Devtools are disabled in release builds");
    }

    /// Create a wry event handler that listens for wry events.
    /// This event handler is scoped to the currently active window and will only receive events that are either global or related to the current window.
    ///
    /// The id this function returns can be used to remove the event handler with [`Self::remove_wry_event_handler`]
    pub fn create_wry_event_handler(
        &self,
        handler: impl FnMut(&Event<UserWindowEvent>, &EventLoopWindowTarget<UserWindowEvent>) + 'static,
    ) -> WryEventHandler {
        self.shared.event_handlers.add(self.window.id(), handler)
    }

    /// Remove a wry event handler created with [`Self::create_wry_event_handler`]
    pub fn remove_wry_event_handler(&self, id: WryEventHandler) {
        self.shared.event_handlers.remove(id)
    }

    /// Create a global shortcut
    ///
    /// Linux: Only works on x11. See [this issue](https://github.com/tauri-apps/tao/issues/331) for more information.
    pub fn create_shortcut(
        &self,
        hotkey: HotKey,
        callback: impl FnMut(HotKeyState) + 'static,
    ) -> Result<ShortcutHandle, ShortcutRegistryError> {
        self.shared
            .shortcut_manager
            .add_shortcut(hotkey, Box::new(callback))
    }

    /// Remove a global shortcut
    pub fn remove_shortcut(&self, id: ShortcutHandle) {
        self.shared.shortcut_manager.remove_shortcut(id)
    }

    /// Remove all global shortcuts
    pub fn remove_all_shortcuts(&self) {
        self.shared.shortcut_manager.remove_all()
    }

    /// Provide a callback to handle asset loading yourself.
    /// If the ScopeId isn't provided, defaults to a global handler.
    /// Note that the handler is namespaced by name, not ScopeId.
    ///
    /// When the component is dropped, the handler is removed.
    ///
    /// See [`crate::use_asset_handler`] for a convenient hook.
    pub fn register_asset_handler(
        &self,
        name: String,
        handler: impl Fn(AssetRequest, RequestAsyncResponder) + 'static,
    ) {
        self.asset_handlers
            .register_handler(name, Callback::new(move |(req, resp)| handler(req, resp)))
    }

    /// Removes an asset handler by its identifier.
    ///
    /// Returns `None` if the handler did not exist.
    pub fn remove_asset_handler(&self, name: &str) -> Option<()> {
        self.asset_handlers.remove_handler(name).map(|_| ())
    }

    /// Push an objc view to the window
    #[cfg(target_os = "ios")]
    pub fn push_view(&self, view: objc_id::ShareId<objc::runtime::Object>) {
        let window = &self.window;

        unsafe {
            use objc::runtime::Object;
            use objc::*;
            assert!(is_main_thread());
            let ui_view = window.ui_view() as *mut Object;
            let ui_view_frame: *mut Object = msg_send![ui_view, frame];
            let _: () = msg_send![view, setFrame: ui_view_frame];
            let _: () = msg_send![view, setAutoresizingMask: 31];

            let ui_view_controller = window.ui_view_controller() as *mut Object;
            let _: () = msg_send![ui_view_controller, setView: view];
            self.views.borrow_mut().push(ui_view);
        }
    }

    /// Pop an objc view from the window
    #[cfg(target_os = "ios")]
    pub fn pop_view(&self) {
        let window = &self.window;

        unsafe {
            use objc::runtime::Object;
            use objc::*;
            assert!(is_main_thread());
            if let Some(view) = self.views.borrow_mut().pop() {
                let ui_view_controller = window.ui_view_controller() as *mut Object;
                let _: () = msg_send![ui_view_controller, setView: view];
            }
        }
    }
}

#[cfg(target_os = "ios")]
fn is_main_thread() -> bool {
    use objc::runtime::{Class, BOOL, NO};
    use objc::*;

    let cls = Class::get("NSThread").unwrap();
    let result: BOOL = unsafe { msg_send![cls, isMainThread] };
    result != NO
}

/// A [`DesktopContext`] that is pending creation.
///
/// # Example
/// ```rust, no_run
/// # use dioxus::prelude::*;
/// # async fn app() {
/// // Create a new window with a component that will be rendered in the new window.
/// let dom = VirtualDom::new(|| rsx!{ "popup!" });
///
/// // Create a new window asynchronously
/// let pending_context = dioxus::desktop::window().new_window(dom, Default::default());
///
/// // Wait for the context to be created
/// let window = pending_context.await;
///
/// // Now control the window
/// window.set_fullscreen(true);
/// # }
/// ```
pub struct PendingDesktopContext {
    pub(crate) receiver: futures_channel::oneshot::Receiver<DesktopContext>,
}

impl PendingDesktopContext {
    /// Resolve the pending context into a [`DesktopContext`].
    pub async fn resolve(self) -> DesktopContext {
        self.try_resolve()
            .await
            .expect("Failed to resolve pending desktop context")
    }

    /// Try to resolve the pending context into a [`DesktopContext`].
    pub async fn try_resolve(self) -> Result<DesktopContext, futures_channel::oneshot::Canceled> {
        self.receiver.await
    }
}

impl IntoFuture for PendingDesktopContext {
    type Output = DesktopContext;

    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.resolve())
    }
}
