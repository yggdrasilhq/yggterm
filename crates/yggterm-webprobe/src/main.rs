//! `yggterm-webprobe` — the launch-decomposition instrument for web surfaces.
//!
//! # Why this exists
//!
//! "Our webapps launch slower than Chromium" is an observation about a
//! *duration*. Every instrument this repo already owns measures something else:
//! `render_probe` measures CPU **cores** over a window, `perf-summary`
//! aggregates spans that mostly live in the daemon, and `ps %CPU` is a lifetime
//! average (the standing trap — see `docs/optimization-pass.md`). None of them
//! can say where the milliseconds between "user clicks" and "page is usable"
//! actually go.
//!
//! This binary answers exactly that, by being a process whose entire lifetime is
//! ONE launch. It reports:
//!
//! | phase | source of truth |
//! |---|---|
//! | `gtk_init_ms` | monotonic clock around `gtk::init` |
//! | `web_context_ms` | monotonic clock around `WebContext::new` (builds the `WebsiteDataManager`, the process pool, applies the cache model) |
//! | `webview_build_ms` | monotonic clock around `WebViewBuilder::build_gtk` |
//! | `network_process_spawn_ms` / `web_process_spawn_ms` | first appearance of a `WebKitNetworkProcess` / `WebKitWebProcess` **child of this pid** in `/proc` (a real spawn observation, not an inference) |
//! | `load_started_ms` / `load_finished_ms` | wry's `PageLoadEvent` |
//! | everything inside the page | the page's OWN `PerformanceTiming` / `PerformanceResourceTiming` / paint entries, posted back over IPC |
//!
//! The in-page half is deliberately the standard web platform instrument rather
//! than something bespoke: **Chromium implements the identical API**, so the
//! Helium/Chromium arm of the comparison reads the same fields off the same
//! fixture, and the two engines are compared like for like instead of against
//! two different bespoke definitions of "loaded".
//!
//! # The cache question this was built to settle
//!
//! `transferSize == 0 && decodedBodySize > 0` is the web platform's own
//! statement that a resource was served from cache without touching the
//! network. Summed over a run it gives `cached_bytes` / `network_bytes`, which
//! is what turns "is our disk cache working?" from an argument into a number.
//!
//! Separately, the fixture brackets every script tag in `performance.mark`s, so
//! for a resource that WAS a cache hit the mark interval is
//! parse+compile+execute with the network removed. That interval, compared
//! between the two engines on the same bytes, is the size of the V8-code-cache
//! gap — the one thing a disk cache cannot close.
//!
//! # Reproducing
//!
//! ```sh
//! scripts/webapp_launch_bench.py --help
//! ```
//!
//! Do not run the arms by hand: the cold/warm protocol (which directories are
//! wiped between runs) is the measurement, and getting it wrong silently turns
//! a cold run into a warm one.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("yggterm-webprobe is Linux/WebKitGTK only");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::RefCell;
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use gtk::prelude::*;
    use wry::{
        dpi::{LogicalPosition, LogicalSize},
        PageLoadEvent, Rect, WebContext, WebViewBuilder, WebViewBuilderExtUnix,
    };

    /// Injected at document-start. Collects the page's own timing view and posts
    /// it over wry's IPC. Idempotent: a timeout-driven `__yggProbeCollect()` and
    /// the `load` listener can both fire, and only the first wins.
    const PROBE_JS: &str = r#"
(function () {
  if (window.__yggProbeInstalled) { return; }
  window.__yggProbeInstalled = true;
  var sent = false;
  function num(v) { return (typeof v === 'number' && isFinite(v)) ? v : null; }
  window.__yggProbeCollect = function (reason) {
    if (sent) { return 'already-sent'; }
    sent = true;
    var out = { reason: reason || 'unknown', ok: true };
    try {
      var nav = performance.getEntriesByType('navigation')[0];
      if (nav) {
        out.nav = {
          startTime: num(nav.startTime),
          fetchStart: num(nav.fetchStart),
          domainLookupStart: num(nav.domainLookupStart),
          domainLookupEnd: num(nav.domainLookupEnd),
          connectStart: num(nav.connectStart),
          connectEnd: num(nav.connectEnd),
          requestStart: num(nav.requestStart),
          responseStart: num(nav.responseStart),
          responseEnd: num(nav.responseEnd),
          domInteractive: num(nav.domInteractive),
          domContentLoadedEventStart: num(nav.domContentLoadedEventStart),
          domContentLoadedEventEnd: num(nav.domContentLoadedEventEnd),
          domComplete: num(nav.domComplete),
          loadEventEnd: num(nav.loadEventEnd),
          transferSize: num(nav.transferSize),
          decodedBodySize: num(nav.decodedBodySize)
        };
      }
      out.paint = (performance.getEntriesByType('paint') || []).map(function (e) {
        return { name: e.name, startTime: num(e.startTime) };
      });
      out.marks = (performance.getEntriesByType('mark') || []).map(function (e) {
        return { name: e.name, startTime: num(e.startTime) };
      });
      out.measures = (performance.getEntriesByType('measure') || []).map(function (e) {
        return { name: e.name, startTime: num(e.startTime), duration: num(e.duration) };
      });
      out.resources = (performance.getEntriesByType('resource') || []).map(function (e) {
        return {
          name: e.name,
          initiatorType: e.initiatorType,
          startTime: num(e.startTime),
          responseStart: num(e.responseStart),
          responseEnd: num(e.responseEnd),
          duration: num(e.duration),
          transferSize: num(e.transferSize),
          encodedBodySize: num(e.encodedBodySize),
          decodedBodySize: num(e.decodedBodySize)
        };
      });
      out.now = num(performance.now());
      // Absolute epoch of navigation start. This is what makes the two engines
      // comparable end to end: the orchestrator knows the wall-clock instant it
      // exec'd the browser, so `timeOrigin - spawn` is process startup and
      // `timeOrigin + loadEventEnd - spawn` is the whole launch — measured the
      // same way on both sides, with no polling granularity in it.
      out.timeOrigin = num(performance.timeOrigin);
      out.href = String(location.href);
    } catch (err) {
      out.ok = false;
      out.error = String(err);
    }
    try { window.ipc.postMessage(JSON.stringify(out)); } catch (e) {}
    return 'sent';
  };
  // `load` is the honest "the launch is over" edge for a webapp: it is after
  // every render-blocking script has parsed, compiled and run. One turn of the
  // task queue afterwards so `loadEventEnd` is actually populated.
  window.addEventListener('load', function () {
    setTimeout(function () { window.__yggProbeCollect('load'); }, 0);
  });
})();
"#;

    struct Args {
        url: String,
        profile: PathBuf,
        label: String,
        timeout_ms: u64,
        width: u32,
        height: u32,
        out: Option<PathBuf>,
        settle_ms: u64,
        /// Optional SECOND launch inside the same process. This is the
        /// experiment that decides whether the first launch's cost is a
        /// once-per-process cost (⇒ prewarming fixes it) or a
        /// once-per-surface cost (⇒ it does not).
        second_url: Option<String>,
        /// When set, the second launch gets its OWN `WebContext` on this
        /// profile — which is what opening a webapp under a DIFFERENT profile
        /// does in the real GUI, since contexts are keyed per profile jar.
        /// When unset the second launch shares the first context, which is
        /// what a new tab in the same session does.
        second_profile: Option<PathBuf>,
    }

    fn usage() -> ! {
        eprintln!(
            "usage: yggterm-webprobe --url <URL> --profile <DIR> [--label NAME] \
             [--timeout-ms 60000] [--settle-ms 0] [--size WxH] [--out FILE]\n\
             \x20                     [--second-url <URL> [--second-profile <DIR>]]\n\
             \n\
             Emits one JSON object on stdout decomposing a single web-surface launch.\n\
             The profile directory is used verbatim; the CALLER owns the cold/warm\n\
             protocol (wipe it for cold, keep it for warm). See\n\
             scripts/webapp_launch_bench.py.\n\
             \n\
             --second-url launches a SECOND surface in the same process after the\n\
             first has loaded. With --second-profile it gets its own WebContext\n\
             (what a different ychrome profile does); without, it shares the first\n\
             (what a new tab does). Comparing the two answers whether the first\n\
             launch's cost is per-process or per-surface."
        );
        std::process::exit(2)
    }

    fn parse_args() -> Args {
        let mut url = None;
        let mut profile = None;
        let mut label = "run".to_string();
        let mut timeout_ms = 60_000u64;
        let mut settle_ms = 0u64;
        let mut width = 1280u32;
        let mut height = 900u32;
        let mut out = None;
        let mut second_url = None;
        let mut second_profile = None;
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--url" => url = it.next(),
                "--profile" => profile = it.next().map(PathBuf::from),
                "--second-url" => second_url = it.next(),
                "--second-profile" => second_profile = it.next().map(PathBuf::from),
                "--label" => label = it.next().unwrap_or_else(|| usage()),
                "--timeout-ms" => {
                    timeout_ms = it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage())
                }
                "--settle-ms" => {
                    settle_ms = it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage())
                }
                "--out" => out = it.next().map(PathBuf::from),
                "--size" => {
                    let raw = it.next().unwrap_or_else(|| usage());
                    let (w, h) = raw.split_once('x').unwrap_or_else(|| usage());
                    width = w.parse().unwrap_or_else(|_| usage());
                    height = h.parse().unwrap_or_else(|_| usage());
                }
                "-h" | "--help" => usage(),
                _ => usage(),
            }
        }
        Args {
            url: url.unwrap_or_else(|| usage()),
            profile: profile.unwrap_or_else(|| usage()),
            label,
            timeout_ms,
            settle_ms,
            width,
            height,
            out,
            second_url,
            second_profile,
        }
    }

    /// Mirror of `configure_linux_webkit_memory_policy` in `apps/yggterm`.
    ///
    /// ⚠ This is a SECOND encoding of that policy and the repo's single-source-
    /// of-truth rule says so out loud. It is deliberate and bounded: the probe
    /// must be able to measure an ARM (`YGGTERM_WEBKIT_CACHE_MODEL=...` set by
    /// the caller) and must therefore never overwrite what the caller set. The
    /// value actually used is echoed into the report as `env`, so a run that
    /// drifted from the app's policy is visible in its own output rather than
    /// having to be trusted. If the app's policy changes, this changes with it.
    fn apply_default_webkit_env() {
        let mem_total_kb = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines().find_map(|l| {
                    l.strip_prefix("MemTotal:")?
                        .split_whitespace()
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
            })
            .unwrap_or(0);
        let limit_mb = if mem_total_kb > 0 {
            ((mem_total_kb / 1024) / 8).clamp(768, 3072)
        } else {
            1024
        };
        let defaults = [
            ("YGGTERM_WEBKIT_CACHE_MODEL", "web-browser".to_string()),
            ("YGGTERM_WEBKIT_MEMORY_LIMIT_MB", limit_mb.to_string()),
            ("YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD", "0.75".into()),
            ("YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD", "0.90".into()),
            ("YGGTERM_WEBKIT_MEMORY_POLL_INTERVAL_SEC", "30.0".into()),
        ];
        for (key, value) in defaults {
            if std::env::var_os(key).is_none() {
                unsafe { std::env::set_var(key, value) };
            }
        }
    }

    fn env_snapshot() -> serde_json::Value {
        let keys = [
            "YGGTERM_WEBKIT_CACHE_MODEL",
            "YGGTERM_WEBKIT_MEMORY_LIMIT_MB",
            "YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD",
            "YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD",
            "YGGTERM_WEBKIT_MEMORY_POLL_INTERVAL_SEC",
            "WEBKIT_DISABLE_DMABUF_RENDERER",
            "WEBKIT_DISABLE_COMPOSITING_MODE",
            "LIBGL_ALWAYS_SOFTWARE",
            "GALLIUM_DRIVER",
        ];
        let mut map = serde_json::Map::new();
        for key in keys {
            map.insert(
                key.to_string(),
                match std::env::var(key) {
                    Ok(v) => serde_json::Value::String(v),
                    Err(_) => serde_json::Value::Null,
                },
            );
        }
        serde_json::Value::Object(map)
    }

    /// First-appearance times for the WebKit auxiliary processes, observed by
    /// polling `/proc` for children of THIS pid.
    ///
    /// This is deliberately an observation and not an inference. `build_gtk`
    /// returning tells you a `WebKitWebView` exists; it does not tell you when
    /// the WebProcess it needs actually came up, and on a cold start those can
    /// be tens of milliseconds apart. Poll interval is 1 ms — the quantum is
    /// therefore ±1 ms, which is reported rather than smoothed away.
    #[derive(Default)]
    struct SpawnWatch {
        web_process_ms: Option<f64>,
        network_process_ms: Option<f64>,
        web_process_count: u32,
        network_process_count: u32,
        /// Every auxiliary-process appearance, in order, as (comm, ms since t0).
        /// The counts alone cannot say whether a SECOND surface spawned its own
        /// pair or reused the first's, and that is the whole question when
        /// asking what prewarming would buy.
        events: Vec<(String, f64)>,
    }

    fn spawn_watcher(t0: Instant, stop: Arc<AtomicBool>) -> Arc<Mutex<SpawnWatch>> {
        let out = Arc::new(Mutex::new(SpawnWatch::default()));
        let handle = Arc::clone(&out);
        let me = std::process::id();
        std::thread::spawn(move || {
            let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
            while !stop.load(Ordering::Relaxed) {
                if let Ok(entries) = std::fs::read_dir("/proc") {
                    for entry in entries.flatten() {
                        let Some(pid) = entry
                            .file_name()
                            .to_str()
                            .and_then(|n| n.parse::<u32>().ok())
                        else {
                            continue;
                        };
                        if seen.contains(&pid) {
                            continue;
                        }
                        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status"))
                        else {
                            continue;
                        };
                        let ppid = status.lines().find_map(|l| {
                            l.strip_prefix("PPid:")?.trim().parse::<u32>().ok()
                        });
                        if ppid != Some(me) {
                            continue;
                        }
                        let comm = status
                            .lines()
                            .find_map(|l| l.strip_prefix("Name:"))
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        seen.insert(pid);
                        let at = t0.elapsed().as_secs_f64() * 1000.0;
                        let mut guard = handle.lock().unwrap();
                        // /proc `Name:` is truncated to 15 chars, hence the prefixes.
                        if comm.starts_with("WebKitWebProces") {
                            guard.web_process_count += 1;
                            guard.web_process_ms.get_or_insert(at);
                            guard.events.push(("web".into(), at));
                        } else if comm.starts_with("WebKitNetworkPr") {
                            guard.network_process_count += 1;
                            guard.network_process_ms.get_or_insert(at);
                            guard.events.push(("network".into(), at));
                        } else {
                            guard.events.push((comm.clone(), at));
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        out
    }

    fn dir_bytes(path: &std::path::Path) -> u64 {
        let mut total = 0u64;
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                total += dir_bytes(&entry.path());
            } else {
                total += meta.len();
            }
        }
        total
    }

    /// `--adblock <rules.json> --adblock-store <dir>`: time WebKit's content
    /// filter store on the two paths the GUI can take at startup.
    ///
    /// This exists because the difference between them is not a detail. The
    /// content-blocker `save` COMPILES; it is not "write if absent". Against
    /// the production ruleset that is a ~16 s window at every single launch,
    /// during which nothing is filtered — while `load` of the same, already
    /// compiled, is milliseconds. `vendor/dioxus-desktop/src/web_surface.rs`
    /// used to call `save` unconditionally; it now loads first, keyed on a
    /// content stamp. This arm is how that claim stays checkable.
    ///
    /// Run twice: the first run compiles and reports `save`, the second finds
    /// the store populated and reports `load`.
    fn run_adblock(ruleset: &std::path::Path, store_dir: &std::path::Path) -> ! {
        use gtk::glib::translate::ToGlibPtr as _;
        use webkit2gtk_sys as wk;

        if gtk::init().is_err() {
            eprintln!("yggterm-webprobe: gtk::init failed (no DISPLAY?)");
            std::process::exit(3);
        }
        let json = std::fs::read(ruleset).expect("read ruleset");
        std::fs::create_dir_all(store_dir).ok();
        // Same stamping rule as the product path, so this measures the real
        // identifier and not an idealized one.
        let stamp = gtk::glib::compute_checksum_for_data(gtk::glib::ChecksumType::Sha256, &json)
            .map(|s| s.as_str()[..32].to_string())
            .unwrap_or_else(|| "unstamped".into());
        let identifier =
            std::ffi::CString::new(format!("yggterm-adblock-{stamp}")).expect("no NUL");
        let store_path = std::ffi::CString::new(store_dir.to_string_lossy().as_bytes()).unwrap();
        let rules_bytes = json.len();
        let bytes = gtk::glib::Bytes::from_owned(json);

        thread_local! {
            static RESULT: RefCell<Option<(String, f64, bool)>> = const { RefCell::new(None) };
        }
        thread_local! {
            static T0: RefCell<Option<Instant>> = const { RefCell::new(None) };
        }
        T0.with(|t| *t.borrow_mut() = Some(Instant::now()));

        unsafe extern "C" fn done_load(
            source: *mut gtk::glib::gobject_ffi::GObject,
            result: *mut gtk::gio::ffi::GAsyncResult,
            _user: gtk::glib::ffi::gpointer,
        ) {
            let mut error = std::ptr::null_mut();
            let filter = unsafe {
                wk::webkit_user_content_filter_store_load_finish(source as *mut _, result, &mut error)
            };
            let elapsed = T0.with(|t| t.borrow().unwrap().elapsed().as_secs_f64() * 1000.0);
            RESULT.with(|r| *r.borrow_mut() = Some(("load".into(), elapsed, !filter.is_null())));
            gtk::main_quit();
        }
        unsafe extern "C" fn done_save(
            source: *mut gtk::glib::gobject_ffi::GObject,
            result: *mut gtk::gio::ffi::GAsyncResult,
            _user: gtk::glib::ffi::gpointer,
        ) {
            let mut error = std::ptr::null_mut();
            let filter = unsafe {
                wk::webkit_user_content_filter_store_save_finish(source as *mut _, result, &mut error)
            };
            let elapsed = T0.with(|t| t.borrow().unwrap().elapsed().as_secs_f64() * 1000.0);
            RESULT.with(|r| *r.borrow_mut() = Some(("save".into(), elapsed, !filter.is_null())));
            gtk::main_quit();
        }

        let store = unsafe { wk::webkit_user_content_filter_store_new(store_path.as_ptr()) };
        unsafe {
            wk::webkit_user_content_filter_store_load(
                store,
                identifier.as_ptr(),
                std::ptr::null_mut(),
                Some(done_load),
                std::ptr::null_mut(),
            );
        }
        gtk::main();
        let mut report = RESULT.with(|r| r.borrow().clone()).unwrap();

        if !report.2 {
            // Load missed: this is the compile path. Time it separately.
            T0.with(|t| *t.borrow_mut() = Some(Instant::now()));
            unsafe {
                wk::webkit_user_content_filter_store_save(
                    store,
                    identifier.as_ptr(),
                    bytes.to_glib_none().0,
                    std::ptr::null_mut(),
                    Some(done_save),
                    std::ptr::null_mut(),
                );
            }
            gtk::main();
            report = RESULT.with(|r| r.borrow().clone()).unwrap();
        }

        let store_bytes = dir_bytes(store_dir);
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "mode": "adblock",
                "ruleset": ruleset.to_string_lossy(),
                "ruleset_bytes": rules_bytes,
                "identifier": identifier.to_string_lossy(),
                "path": report.0,
                "elapsed_ms": round2(report.1),
                "ok": report.2,
                "store_dir": store_dir.to_string_lossy(),
                "store_bytes": store_bytes,
            }))
            .unwrap()
        );
        std::io::stdout().flush().ok();
        std::process::exit(0);
    }

    pub fn run() {
        // The adblock arm is a different measurement entirely and shares only
        // the process, so it is dispatched before the launch-probe arg parse
        // rather than being bolted into it.
        let raw: Vec<String> = std::env::args().collect();
        if let Some(index) = raw.iter().position(|a| a == "--adblock") {
            let ruleset = PathBuf::from(raw.get(index + 1).expect("--adblock needs a path"));
            let store = raw
                .iter()
                .position(|a| a == "--adblock-store")
                .and_then(|i| raw.get(i + 1))
                .map(PathBuf::from)
                .expect("--adblock-store is required");
            run_adblock(&ruleset, &store);
        }
        let args = parse_args();
        apply_default_webkit_env();

        let cache_dir = args.profile.join("WebKitCache");
        let cache_bytes_before = dir_bytes(&cache_dir);

        let t0 = Instant::now();
        let stop = Arc::new(AtomicBool::new(false));
        let spawns = spawn_watcher(t0, Arc::clone(&stop));

        let t = Instant::now();
        if gtk::init().is_err() {
            eprintln!("yggterm-webprobe: gtk::init failed (no DISPLAY / WAYLAND_DISPLAY?)");
            std::process::exit(3);
        }
        let gtk_init_ms = t.elapsed().as_secs_f64() * 1000.0;

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(args.width as i32, args.height as i32);
        let fixed = gtk::Fixed::new();
        window.add(&fixed);
        window.show_all();
        let window_ms = t0.elapsed().as_secs_f64() * 1000.0;

        std::fs::create_dir_all(&args.profile).ok();
        let t = Instant::now();
        let mut context = WebContext::new(Some(args.profile.clone()));
        let web_context_ms = t.elapsed().as_secs_f64() * 1000.0;

        let report: Rc<RefCell<serde_json::Map<String, serde_json::Value>>> =
            Rc::new(RefCell::new(serde_json::Map::new()));
        let page: Rc<RefCell<Option<serde_json::Value>>> = Rc::new(RefCell::new(None));
        let load_started: Rc<RefCell<Option<f64>>> = Rc::new(RefCell::new(None));
        let load_finished: Rc<RefCell<Option<f64>>> = Rc::new(RefCell::new(None));

        let ipc_page = Rc::clone(&page);
        let started = Rc::clone(&load_started);
        let finished = Rc::clone(&load_finished);

        let t = Instant::now();
        let webview = WebViewBuilder::new_with_web_context(&mut context)
            .with_bounds(Rect {
                position: LogicalPosition::new(0, 0).into(),
                size: LogicalSize::new(args.width, args.height).into(),
            })
            .with_initialization_script(PROBE_JS)
            .with_ipc_handler(move |request| {
                let body = request.into_body();
                match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(value) => *ipc_page.borrow_mut() = Some(value),
                    Err(err) => {
                        *ipc_page.borrow_mut() = Some(serde_json::json!({
                            "ok": false,
                            "error": format!("probe payload was not JSON: {err}"),
                        }))
                    }
                }
            })
            .with_on_page_load_handler(move |event, _url| {
                let at = t0.elapsed().as_secs_f64() * 1000.0;
                match event {
                    PageLoadEvent::Started => {
                        let mut slot = started.borrow_mut();
                        if slot.is_none() {
                            *slot = Some(at);
                        }
                    }
                    PageLoadEvent::Finished => *finished.borrow_mut() = Some(at),
                }
            })
            .with_url(&args.url)
            .build_gtk(&fixed)
            .expect("build webview");
        let webview_build_ms = t.elapsed().as_secs_f64() * 1000.0;
        let navigation_requested_ms = t0.elapsed().as_secs_f64() * 1000.0;

        // Deadline + settle. The loop exits as soon as the page has posted its
        // report AND `settle_ms` has elapsed since that post — the settle window
        // exists so a page whose real work happens after `load` (a SPA hydrating)
        // is not scored on its shell alone.
        let deadline = Instant::now() + Duration::from_millis(args.timeout_ms);
        let settle = Duration::from_millis(args.settle_ms);
        let page_tick = Rc::clone(&page);
        let asked_late = Rc::new(RefCell::new(false));
        let settled_at: Rc<RefCell<Option<Instant>>> = Rc::new(RefCell::new(None));
        let webview_tick = Rc::new(webview);
        let webview_for_tick = Rc::clone(&webview_tick);
        gtk::glib::timeout_add_local(Duration::from_millis(10), move || {
            let has_page = page_tick.borrow().is_some();
            if has_page {
                let mut slot = settled_at.borrow_mut();
                if slot.is_none() {
                    *slot = Some(Instant::now());
                }
                if slot.map(|at| at.elapsed() >= settle).unwrap_or(false) {
                    gtk::main_quit();
                    return gtk::glib::ControlFlow::Break;
                }
            }
            if Instant::now() >= deadline {
                // One last chance: the page may be alive but never fired `load`
                // (a stalled subresource). Ask it directly, then give the IPC a
                // beat to land before quitting.
                if !*asked_late.borrow() {
                    *asked_late.borrow_mut() = true;
                    let _ = webview_for_tick.evaluate_script(
                        "window.__yggProbeCollect && window.__yggProbeCollect('timeout')",
                    );
                    return gtk::glib::ControlFlow::Continue;
                }
                gtk::main_quit();
                return gtk::glib::ControlFlow::Break;
            }
            gtk::glib::ControlFlow::Continue
        });

        gtk::main();
        let first_done_ms = t0.elapsed().as_secs_f64() * 1000.0;

        // ---- optional SECOND launch, same process ---------------------------
        // The point of this arm: if launch #2 is dramatically cheaper than
        // launch #1, the expensive part is per-PROCESS and a prewarmed engine
        // removes it. If it costs the same, prewarming buys nothing and the
        // cost is per-surface. There is no way to tell those apart from a
        // single launch, and guessing between them is how a whole lane gets
        // built for the wrong one.
        let second = args.second_url.as_ref().map(|second_url| {
            let t1 = Instant::now();
            let mut second_context = args.second_profile.as_ref().map(|dir| {
                std::fs::create_dir_all(dir).ok();
                let t = Instant::now();
                let ctx = WebContext::new(Some(dir.clone()));
                (ctx, t.elapsed().as_secs_f64() * 1000.0)
            });
            let second_context_ms = second_context.as_ref().map(|(_, ms)| *ms);

            let page2: Rc<RefCell<Option<serde_json::Value>>> = Rc::new(RefCell::new(None));
            let started2: Rc<RefCell<Option<f64>>> = Rc::new(RefCell::new(None));
            let finished2: Rc<RefCell<Option<f64>>> = Rc::new(RefCell::new(None));
            let ipc2 = Rc::clone(&page2);
            let s2 = Rc::clone(&started2);
            let f2 = Rc::clone(&finished2);

            let fixed2 = gtk::Fixed::new();
            window.add(&fixed2);
            fixed2.show_all();

            let build = |builder: WebViewBuilder<'_>| -> wry::WebView {
                builder
                    .with_bounds(Rect {
                        position: LogicalPosition::new(0, 0).into(),
                        size: LogicalSize::new(args.width, args.height).into(),
                    })
                    .with_initialization_script(PROBE_JS)
                    .with_ipc_handler(move |request| {
                        if let Ok(value) =
                            serde_json::from_str::<serde_json::Value>(&request.into_body())
                        {
                            *ipc2.borrow_mut() = Some(value);
                        }
                    })
                    .with_on_page_load_handler(move |event, _url| {
                        let at = t1.elapsed().as_secs_f64() * 1000.0;
                        match event {
                            PageLoadEvent::Started => {
                                let mut slot = s2.borrow_mut();
                                if slot.is_none() {
                                    *slot = Some(at);
                                }
                            }
                            PageLoadEvent::Finished => *f2.borrow_mut() = Some(at),
                        }
                    })
                    .with_url(second_url)
                    .build_gtk(&fixed2)
                    .expect("build second webview")
            };

            let t = Instant::now();
            let _webview2 = match second_context.as_mut() {
                Some((ctx, _)) => build(WebViewBuilder::new_with_web_context(ctx)),
                None => build(WebViewBuilder::new_with_web_context(&mut context)),
            };
            let second_build_ms = t.elapsed().as_secs_f64() * 1000.0;
            let second_nav_ms = t1.elapsed().as_secs_f64() * 1000.0;

            let deadline2 = Instant::now() + Duration::from_millis(args.timeout_ms);
            let page_tick2 = Rc::clone(&page2);
            gtk::glib::timeout_add_local(Duration::from_millis(10), move || {
                if page_tick2.borrow().is_some() || Instant::now() >= deadline2 {
                    gtk::main_quit();
                    return gtk::glib::ControlFlow::Break;
                }
                gtk::glib::ControlFlow::Continue
            });
            gtk::main();

            serde_json::json!({
                "url": second_url,
                "own_context": args.second_profile.is_some(),
                "profile": args.second_profile.as_ref().map(|p| p.to_string_lossy().to_string()),
                "phases_ms": {
                    "web_context_new": second_context_ms.map(round2),
                    "webview_build": round2(second_build_ms),
                    "navigation_requested_at": round2(second_nav_ms),
                    "load_started_at": started2.borrow().map(round2),
                    "load_finished_at": finished2.borrow().map(round2),
                    "total": round2(t1.elapsed().as_secs_f64() * 1000.0),
                },
                "page": page2.borrow().clone(),
            })
        });

        let total_ms = t0.elapsed().as_secs_f64() * 1000.0;
        stop.store(true, Ordering::Relaxed);

        let spawns = spawns.lock().unwrap();
        let cache_bytes_after = dir_bytes(&cache_dir);
        let page_value = page.borrow().clone();

        {
            let mut r = report.borrow_mut();
            r.insert("label".into(), args.label.clone().into());
            r.insert("url".into(), args.url.clone().into());
            r.insert(
                "profile".into(),
                args.profile.to_string_lossy().to_string().into(),
            );
            r.insert("engine".into(), "webkitgtk".into());
            r.insert("env".into(), env_snapshot());
            r.insert(
                "host".into(),
                serde_json::json!({
                    "webkit_runtime": webkit_runtime_version(),
                    "display": std::env::var("DISPLAY").ok(),
                    "wayland": std::env::var("WAYLAND_DISPLAY").ok(),
                }),
            );
            r.insert(
                "phases_ms".into(),
                serde_json::json!({
                    "gtk_init": round2(gtk_init_ms),
                    "window_realize_at": round2(window_ms),
                    "web_context_new": round2(web_context_ms),
                    "webview_build": round2(webview_build_ms),
                    "navigation_requested_at": round2(navigation_requested_ms),
                    "network_process_spawn_at": spawns.network_process_ms.map(round2),
                    "web_process_spawn_at": spawns.web_process_ms.map(round2),
                    "load_started_at": load_started.borrow().map(round2),
                    "load_finished_at": load_finished.borrow().map(round2),
                    "first_done_at": round2(first_done_ms),
                    "total": round2(total_ms),
                }),
            );
            r.insert(
                "processes".into(),
                serde_json::json!({
                    "web_process_count": spawns.web_process_count,
                    "network_process_count": spawns.network_process_count,
                    "spawns": spawns
                        .events
                        .iter()
                        .map(|(kind, at)| serde_json::json!({"proc": kind, "at_ms": round2(*at)}))
                        .collect::<Vec<_>>(),
                }),
            );
            if let Some(second) = second {
                r.insert("second".into(), second);
            }
            r.insert(
                "disk_cache".into(),
                serde_json::json!({
                    "dir": cache_dir.to_string_lossy(),
                    "bytes_before": cache_bytes_before,
                    "bytes_after": cache_bytes_after,
                    "bytes_written": cache_bytes_after.saturating_sub(cache_bytes_before),
                }),
            );
            r.insert(
                "page".into(),
                page_value.unwrap_or(serde_json::Value::Null),
            );
        }

        let text = serde_json::to_string_pretty(&serde_json::Value::Object(
            report.borrow().clone(),
        ))
        .expect("serialize report");
        if let Some(path) = &args.out {
            if let Ok(mut file) = std::fs::File::create(path) {
                let _ = file.write_all(text.as_bytes());
                let _ = file.write_all(b"\n");
            }
        }
        println!("{text}");
        // GTK/WebKit teardown of a live WebProcess can take longer than the
        // measurement itself and has nothing to do with launch latency.
        // The report is already flushed, so leave.
        std::io::stdout().flush().ok();
        std::process::exit(0);
    }

    fn round2(v: f64) -> f64 {
        (v * 100.0).round() / 100.0
    }

    /// The WebKit the process actually linked, read from the loaded shared
    /// object rather than from `dpkg` — the packaged version and the loaded
    /// library are different questions and this report is about the latter.
    fn webkit_runtime_version() -> serde_json::Value {
        let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
            return serde_json::Value::Null;
        };
        for line in maps.lines() {
            if let Some(idx) = line.find("libwebkit2gtk-") {
                let path = line[idx..].split_whitespace().next().unwrap_or("");
                return serde_json::Value::String(path.to_string());
            }
        }
        serde_json::Value::Null
    }
}
