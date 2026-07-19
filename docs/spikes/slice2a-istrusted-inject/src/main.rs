//! Slice-2a spike (agent control plane, docs/agent-control-plane.md F2):
//! Can a GDK event synthesized in-process and delivered to a WebKitGTK WebView
//! widget (NO seat pointer moved) produce a DOM event with `isTrusted === true`?
//!
//!   GO   => the `do` verb ships on the GUI plane (trusted click/key without
//!           hijacking the user's real cursor — the whole point).
//!   NO-GO => `do` defers to the headless farm plane (slice 4); slice 2b still
//!           ships read/wait/capture/lease.
//!
//! Three probes, each reports `e.isTrusted` back through document.title +
//! window.__events:
//!   A) CONTROL — JS `dispatchEvent(new MouseEvent('click'))`. MUST be
//!      isTrusted=false (proves the readout distinguishes trusted from not).
//!   B) GDK BUTTON — `gdk_event_new(GDK_BUTTON_PRESS/RELEASE)` filled with the
//!      webview's GdkWindow + the seat pointer device, delivered via
//!      `WidgetExt::event(&webview, &event)`. This is the candidate `do` path.
//!   C) GDK KEY — `gtk::test_widget_send_key`, the GTK test-harness key synth.
//!
//! Run: `WEBKIT_DISABLE_DMABUF_RENDERER=1 xvfb-run -a cargo run --release`

use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use webkit2gtk::WebViewExt;

const PAGE_HTML: &str = r#"<!doctype html><body style="margin:0;height:100vh;background:#222;color:#eee;font:18px monospace">
<div id="log">waiting</div>
<button id="b" style="position:absolute;left:100px;top:100px;width:220px;height:90px">CLICK TARGET</button>
<script>
window.__events = [];
function rec(tag, e){
  var line = tag + ' isTrusted=' + e.isTrusted
    + (e.clientX!==undefined ? ' @'+Math.round(e.clientX)+','+Math.round(e.clientY) : '')
    + (e.key ? ' key='+e.key : '');
  window.__events.push(line);
  document.getElementById('log').textContent = line;
  document.title = line;
}
var b = document.getElementById('b');
b.addEventListener('mousedown', function(e){ rec('BTN-MOUSEDOWN', e); });
b.addEventListener('click',     function(e){ rec('BTN-CLICK', e); });
window.addEventListener('mousedown', function(e){ rec('WIN-MOUSEDOWN', e); });
window.addEventListener('keydown',   function(e){ rec('WIN-KEYDOWN', e); });
document.title = 'ready';
</script></body>"#;

/// Synthesize a GDK button event and deliver it straight to the webview widget.
/// Returns whether GTK reports the widget handled it.
unsafe fn synth_button(webview: &webkit2gtk::WebView, press: bool, x: f64, y: f64) -> bool {
    use glib::translate::{from_glib_full, ToGlibPtr};
    let Some(gdk_window) = webview.window() else {
        eprintln!("spike: webview has NO GdkWindow (not realized/windowless)");
        return false;
    };
    let etype = if press {
        gdk::ffi::GDK_BUTTON_PRESS
    } else {
        gdk::ffi::GDK_BUTTON_RELEASE
    };
    let ev_ptr = gdk::ffi::gdk_event_new(etype);
    let bev = ev_ptr as *mut gdk::ffi::GdkEventButton;
    (*bev).window = gdk_window.to_glib_full();
    (*bev).send_event = 0; // look like real windowing-system input, not SendEvent
    (*bev).time = 0; // GDK_CURRENT_TIME
    (*bev).x = x;
    (*bev).y = y;
    (*bev).x_root = x;
    (*bev).y_root = y;
    (*bev).button = 1;
    (*bev).state = 0;
    if let Some(device) = gdk::Display::default()
        .and_then(|d| d.default_seat())
        .and_then(|s| s.pointer())
    {
        (*bev).device = device.to_glib_full();
    } else {
        eprintln!("spike: no pointer device on the default seat (headless seat?)");
    }
    let event: gdk::Event = from_glib_full(ev_ptr);
    gtk::prelude::WidgetExt::event(webview, &event)
}

/// Interpret a webkit snapshot: dims + center-pixel RGB, so a caller can tell a
/// FRESH render (center ≈ magenta, the color we painted) from a blank/stale one.
fn describe_snapshot(surface: cairo::Surface) -> String {
    // Consume the surface (sole owner) so cairo lets us borrow its pixels.
    surface.flush();
    match cairo::ImageSurface::try_from(surface) {
        Ok(mut img) => {
            let (w, h, stride) = (img.width(), img.height(), img.stride());
            let fmt = img.format();
            let px = img.data().ok().and_then(|data| {
                if w > 0 && h > 0 {
                    let off = (h / 2) as usize * stride as usize + (w / 2) as usize * 4;
                    // cairo ARGB32 is premultiplied BGRA in memory (LE).
                    (off + 3 < data.len()).then(|| (data[off + 2], data[off + 1], data[off]))
                } else {
                    None
                }
            });
            match px {
                Some((r, g, b)) => format!(
                    "OK {w}x{h} fmt={fmt:?} center_rgb=({r},{g},{b}) [painted magenta≈(200,0,200) => FRESH]"
                ),
                None => format!("OK {w}x{h} fmt={fmt:?} (no pixel sample)"),
            }
        }
        Err(surface) => format!("OK non-image surface type={:?}", surface.type_()),
    }
}

fn read_title(webview: &webkit2gtk::WebView, label: &str, out: &Rc<RefCell<Vec<String>>>) {
    let t = webview.title().map(|g| g.to_string()).unwrap_or_default();
    let line = format!("{label}: title={t:?}");
    eprintln!("spike: {line}");
    out.borrow_mut().push(line);
}

fn main() {
    gtk::init().expect("gtk init");

    let win = gtk::Window::new(gtk::WindowType::Toplevel);
    win.set_default_size(500, 400);
    win.set_decorated(false);
    win.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });

    let webview = webkit2gtk::WebView::new();
    webview.load_html(PAGE_HTML, None);
    win.add(&webview);

    // GTK-level observability: did the synthesized event reach the widget at all?
    webview.connect_button_press_event(|_, ev| {
        eprintln!("spike: [gtk] webview button-press at {:?}", ev.position());
        glib::Propagation::Proceed
    });
    webview.connect_key_press_event(|_, ev| {
        eprintln!("spike: [gtk] webview key-press keyval={:?}", ev.keyval());
        glib::Propagation::Proceed
    });

    win.show_all();

    let results: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let step = Rc::new(RefCell::new(0u32));
    let snap_result: Rc<RefCell<String>> = Rc::new(RefCell::new("(not captured)".into()));
    let webview_c = webview.clone();
    let results_c = results.clone();
    let snap_c = snap_result.clone();
    let snap_c2 = snap_result.clone();

    // State machine, one tick / 500ms. Initial ticks let webkit load + realize.
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let s = { let mut b = step.borrow_mut(); *b += 1; *b };
        match s {
            1..=2 => { /* settle: load + realize */ }
            3 => {
                eprintln!("spike: --- probe A: JS-synthetic dispatchEvent (control) ---");
                webview_c.run_javascript(
                    "(function(){document.getElementById('b').dispatchEvent(new MouseEvent('click',{bubbles:true,clientX:210,clientY:145}));})()",
                    gtk::gio::Cancellable::NONE,
                    |_| {},
                );
            }
            4 => read_title(&webview_c, "A control (JS synthetic)", &results_c),
            5 => {
                eprintln!("spike: --- probe B: GDK button via WidgetExt::event ---");
                webview_c.grab_focus();
                unsafe {
                    let h1 = synth_button(&webview_c, true, 210.0, 145.0);
                    let h2 = synth_button(&webview_c, false, 210.0, 145.0);
                    eprintln!("spike: widget.event handled press={h1} release={h2}");
                }
            }
            6 => read_title(&webview_c, "B GDK button (widget.event)", &results_c),
            7 => {
                eprintln!("spike: --- probe C: gtk::test_widget_send_key ---");
                webview_c.grab_focus();
                let handled = gtk::test_widget_send_key(
                    &webview_c,
                    *gdk::keys::constants::a,
                    gdk::ModifierType::empty(),
                );
                eprintln!("spike: test_widget_send_key handled={handled}");
            }
            8 => read_title(&webview_c, "C GDK key (test_widget_send_key)", &results_c),
            9 => {
                // Dump the visible-phase DOM event log into the title.
                webview_c.run_javascript(
                    "(function(){document.title='EVENTS::'+JSON.stringify(window.__events);})()",
                    gtk::gio::Cancellable::NONE,
                    |_| {},
                );
            }
            10 => read_title(&webview_c, "VISIBLE event log", &results_c),

            // ---- HIDDEN PHASE: soft-stash / unmapped surface ----
            // Paint a distinctive color first so a snapshot's freshness is
            // testable, then HIDE the webview (unmap it) and re-probe.
            11 => {
                webview_c.run_javascript(
                    "(function(){document.body.style.background='#c800c8';window.__events=[];document.title='painted-magenta';})()",
                    gtk::gio::Cancellable::NONE,
                    |_| {},
                );
            }
            12 => {
                eprintln!("spike: --- HIDING webview (unmap) ---");
                webview_c.hide();
                let has_win = webview_c.window().is_some();
                let line = format!("hidden: webview.window().is_some()={has_win} (None => unmapped, injection needs a mapped target)");
                eprintln!("spike: {line}");
                results_c.borrow_mut().push(line);
            }
            13 => {
                // READ while hidden: eval must still return.
                eprintln!("spike: --- probe D: READ (eval) while hidden ---");
                webview_c.run_javascript(
                    "(function(){document.title='HIDDEN-READ ok bg='+getComputedStyle(document.body).backgroundColor;})()",
                    gtk::gio::Cancellable::NONE,
                    |_| {},
                );
            }
            14 => read_title(&webview_c, "D read-while-hidden", &results_c),
            15 => {
                // INJECT while hidden.
                eprintln!("spike: --- probe E: GDK inject while hidden ---");
                unsafe {
                    let h1 = synth_button(&webview_c, true, 210.0, 145.0);
                    let h2 = synth_button(&webview_c, false, 210.0, 145.0);
                    eprintln!("spike: hidden widget.event handled press={h1} release={h2}");
                }
                webview_c.run_javascript(
                    "(function(){document.title='HIDDEN-INJECT events='+JSON.stringify(window.__events);})()",
                    gtk::gio::Cancellable::NONE,
                    |_| {},
                );
            }
            16 => read_title(&webview_c, "E inject-while-hidden", &results_c),
            17 => {
                // CAPTURE while hidden: snapshot + sample the center pixel.
                eprintln!("spike: --- probe F: snapshot (capture) while hidden ---");
                let snap = snap_c.clone();
                webview_c.snapshot(
                    webkit2gtk::SnapshotRegion::FullDocument,
                    webkit2gtk::SnapshotOptions::NONE,
                    gtk::gio::Cancellable::NONE,
                    move |res| {
                        *snap.borrow_mut() = match res {
                            Ok(surface) => describe_snapshot(surface),
                            Err(e) => format!("snapshot ERR: {e}"),
                        };
                    },
                );
            }
            18 => {
                let s = snap_c2.borrow().clone();
                let line = format!("F capture-while-hidden: {s}");
                eprintln!("spike: {line}");
                results_c.borrow_mut().push(line);
            }
            19 => {
                eprintln!("\n================ SLICE-2a SPIKE RESULT (visible + hidden) ================");
                for line in results_c.borrow().iter() {
                    eprintln!("  {line}");
                }
                eprintln!("=========================================================================\n");
                gtk::main_quit();
                return glib::ControlFlow::Break;
            }
            _ => {
                gtk::main_quit();
                return glib::ControlFlow::Break;
            }
        }
        glib::ControlFlow::Continue
    });

    gtk::main();
}
