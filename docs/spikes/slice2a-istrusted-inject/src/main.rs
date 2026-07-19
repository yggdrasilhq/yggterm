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
    let webview_c = webview.clone();
    let results_c = results.clone();

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
                // Dump the full DOM event log into the title so we can read it.
                webview_c.run_javascript(
                    "(function(){document.title='EVENTS::'+JSON.stringify(window.__events);})()",
                    gtk::gio::Cancellable::NONE,
                    |_| {},
                );
            }
            10 => {
                read_title(&webview_c, "EVENT LOG", &results_c);
                eprintln!("\n================ SLICE-2a isTrusted SPIKE RESULT ================");
                for line in results_c.borrow().iter() {
                    eprintln!("  {line}");
                }
                eprintln!("================================================================\n");
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
