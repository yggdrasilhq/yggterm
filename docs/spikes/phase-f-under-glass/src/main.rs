//! Phase F spike: does webkit2gtk 2.52 on Wayland composite child webviews
//! IN-WIDGET (GTK z-order + alpha honored) or via subsurface (draws above all)?
//!
//! Layout mirrors the proposed production restack:
//!   gtk::Overlay
//!     base child     = PAGE webview (red page; turns yellow on click)
//!     overlay child  = SHELL webview (transparent bg; DOM paints a green
//!                      titlebar + opaque navy frame with a ROUNDED transparent
//!                      hole; titlebar turns magenta on click)
//! plus an INPUT-SHAPE hole punched in the shell's GdkWindow over the hole rect.
//!
//! PASS = screenshot shows red through a rounded hole, titlebar/frame cover the
//! page elsewhere; click in hole reaches the PAGE (yellow), click on titlebar
//! reaches the SHELL (magenta).

use gtk::prelude::*;
use webkit2gtk::WebViewExt;

const W: i32 = 800;
const H: i32 = 600;
// Hole rect (the "molded viewport"): must match the shell DOM inset below.
const HOLE_X: i32 = 40;
const HOLE_Y: i32 = 68;
const HOLE_W: i32 = W - 80;
const HOLE_H: i32 = H - 68 - 40;

const PAGE_HTML: &str = r#"<!doctype html><body style="margin:0;height:100vh;background:#dd1111">
<h1 style="color:#fff;font:bold 40px sans-serif;padding:140px 60px 0">PAGE LAYER</h1>
<script>
window.addEventListener('click', () => {
  document.body.style.background = '#eecc00';
  document.title = 'page-clicked';
});
document.addEventListener('keydown', (e) => {
  document.body.style.background = '#00aa44';
  document.querySelector('h1').textContent = 'PAGE GOT KEY: ' + e.key;
});
</script></body>"#;

const SHELL_HTML: &str = r#"<!doctype html><body style="margin:0;background:transparent">
<div id="tb" style="position:fixed;top:0;left:0;right:0;height:48px;background:#16a34a;
     color:#fff;font:bold 20px sans-serif;line-height:48px;text-align:center;z-index:2">
  TITLEBAR — shell DOM over the page
</div>
<div style="position:fixed;left:40px;top:68px;right:40px;bottom:40px;
     border:6px solid #2563eb;border-radius:24px;box-sizing:border-box;
     box-shadow:0 0 0 4000px rgba(15,23,42,0.97);pointer-events:none"></div>
<script>
document.getElementById('tb').addEventListener('click', () => {
  document.getElementById('tb').style.background = '#d946ef';
  document.title = 'shell-clicked';
});
// AC-content probe (F.0.1): ?ac in the html forces REAL accelerated
// compositing — a WebGL canvas + a 3D-transformed layer, like the
// production glass (WebGL xterm). Policy alone may not engage AC.
if (location.search.includes('ac') || window.__spikeAc) {
  const c = document.createElement('canvas');
  c.width = 64; c.height = 64;
  c.style.cssText = 'position:fixed;left:4px;top:4px;width:32px;height:32px;z-index:3';
  document.body.appendChild(c);
  const gl = c.getContext('webgl');
  if (gl) { gl.clearColor(1, 0.5, 0, 1); gl.clear(gl.COLOR_BUFFER_BIT); document.title = 'ac-webgl-on'; }
  const layer = document.createElement('div');
  layer.style.cssText = 'position:fixed;right:4px;top:4px;width:24px;height:24px;background:#0ff;transform:translateZ(0);will-change:transform;z-index:3';
  document.body.appendChild(layer);
}
document.addEventListener('keydown', (e) => {
  document.getElementById('tb').style.background = '#ff8800';
  document.getElementById('tb').textContent = 'SHELL GOT KEY: ' + e.key;
});
</script></body>"#;

fn main() {
    gtk::init().expect("gtk init");

    let win = gtk::Window::new(gtk::WindowType::Toplevel);
    win.set_default_size(W, H);
    win.set_decorated(false);
    // Window-visual probe (F.0.1): env SPIKE_WIN=rgba gives the toplevel an
    // RGBA visual + app_paintable, like tao's transparent window.
    if std::env::var("SPIKE_WIN").map(|v| v == "rgba").unwrap_or(false) {
        let screen: Option<gdk::Screen> = gtk::prelude::WidgetExt::screen(&win);
        if let Some(visual) = screen.and_then(|screen| screen.rgba_visual()) {
            win.set_visual(Some(&visual));
        }
        win.set_app_paintable(true);
        eprintln!("spike: toplevel RGBA visual + app_paintable");
    }
    win.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });

    let overlay = gtk::Overlay::new();
    win.add(&overlay);

    // Tree-shape probe (F.0.1): env SPIKE_TREE=prod assembles the PRODUCTION
    // widget tree (backdrop Box base child; page webview inside a gtk::Fixed
    // overlay child at a rect; shell webview inside a GtkBox overlay child,
    // reordered topmost) instead of the original spike tree (page = base
    // child, shell = direct overlay child).
    let prod_tree = std::env::var("SPIKE_TREE").map(|v| v == "prod").unwrap_or(false);

    // PAGE layer: the overlay's base child, fills the window (production: each
    // page webview sits in a gtk::Fixed at its rect; full-bleed is fine here).
    let page = webkit2gtk::WebView::new();
    page.load_html(PAGE_HTML, None);
    if prod_tree {
        let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
        overlay.add(&backdrop);
        let fixed = gtk::Fixed::new();
        fixed.set_halign(gtk::Align::Start);
        fixed.set_valign(gtk::Align::Start);
        fixed.set_margin_start(HOLE_X);
        fixed.set_margin_top(HOLE_Y);
        fixed.set_size_request(HOLE_W, HOLE_H);
        page.set_size_request(HOLE_W, HOLE_H);
        fixed.put(&page, 0, 0);
        overlay.add_overlay(&fixed);
        eprintln!("spike: PRODUCTION tree (backdrop base, page in Fixed overlay child)");
    } else {
        overlay.add(&page);
    }

    // SHELL layer: overlay child ABOVE the page, transparent background.
    let shell = webkit2gtk::WebView::new();
    shell.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    // AC-mode probe (F.0.1): env SPIKE_SHELL_AC=always forces the shell into
    // accelerated compositing — the production glass (WebGL xterm) always is.
    // Hypothesis under test: the AC backing-store paint blits with cairo
    // operator SOURCE, erasing the page pixels beneath instead of
    // compositing OVER them.
    let shell_ac = std::env::var("SPIKE_SHELL_AC").map(|v| v == "always").unwrap_or(false);
    if shell_ac {
        use webkit2gtk::SettingsExt as _;
        if let Some(settings) = WebViewExt::settings(&shell) {
            settings.set_hardware_acceleration_policy(
                webkit2gtk::HardwareAccelerationPolicy::Always,
            );
            eprintln!("spike: shell hardware-acceleration-policy = ALWAYS");
        }
    }
    let shell_html = if shell_ac {
        SHELL_HTML.replace("location.search.includes('ac')", "true")
    } else {
        SHELL_HTML.to_string()
    };
    shell.load_html(&shell_html, None);
    if prod_tree {
        let glass_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        glass_box.pack_start(&shell, true, true, 0);
        overlay.add_overlay(&glass_box);
        overlay.reorder_overlay(&glass_box, -1);
    } else {
        overlay.add_overlay(&shell);
    }

    // GTK-level input observability: who actually receives events?
    win.connect_button_press_event(|_, ev| {
        eprintln!("spike: WINDOW button-press at {:?}", ev.position());
        glib::Propagation::Proceed
    });
    page.connect_button_press_event(|_, ev| {
        eprintln!("spike: PAGE widget button-press at {:?}", ev.position());
        glib::Propagation::Proceed
    });
    shell.connect_button_press_event(|_, ev| {
        eprintln!("spike: SHELL widget button-press at {:?}", ev.position());
        glib::Propagation::Proceed
    });
    page.connect_key_press_event(|_, ev| {
        eprintln!("spike: PAGE widget key-press {:?}", ev.keyval());
        glib::Propagation::Proceed
    });
    shell.connect_key_press_event(|_, ev| {
        eprintln!("spike: SHELL widget key-press {:?}", ev.keyval());
        glib::Propagation::Proceed
    });
    page.connect_enter_notify_event(|_, _| {
        eprintln!("spike: PAGE enter");
        glib::Propagation::Proceed
    });
    shell.connect_enter_notify_event(|_, _| {
        eprintln!("spike: SHELL enter");
        glib::Propagation::Proceed
    });

    win.show_all();

    // Punch the input hole in the shell's GdkWindow once it's realized and
    // webkit has settled. Re-apply once more in case webkit re-creates windows.
    // Dump the GdkWindow tree: geometry + event masks, to understand picking.
    let win_for_dump = win.clone();
    glib::timeout_add_seconds_local(3, move || {
        fn dump(w: &gdk::Window, depth: usize) {
            let (x, y) = w.position();
            eprintln!(
                "spike: gdkwin depth={depth} pos=({x},{y}) size={}x{} events={:?}",
                w.width(),
                w.height(),
                w.events()
            );
            for c in w.children() {
                dump(&c, depth + 1);
            }
        }
        if let Some(w) = win_for_dump.window() {
            dump(&w, 0);
        }
        glib::ControlFlow::Break
    });

    let shell_for_shape = shell.clone();
    let win_for_shape = win.clone();
    let apply_shape = move || {
        let toplevel = win_for_shape.window();
        if let Some(gdk_win) = shell_for_shape.window() {
            let full = cairo::RectangleInt::new(0, 0, W, H);
            let hole = cairo::RectangleInt::new(HOLE_X, HOLE_Y, HOLE_W, HOLE_H);
            let region = cairo::Region::create_rectangle(&full);
            region.subtract_rectangle(&hole).ok();
            // Shape the shell's window AND every ancestor up to (excluding) the
            // toplevel: GtkOverlay wraps each overlay child in an intermediate
            // GdkWindow with an empty event mask — left unshaped it still picks,
            // and GDK then bubbles to the TOPLEVEL (an ancestor), never the page
            // (a sibling below).
            let mut w = Some(gdk_win);
            while let Some(cur) = w {
                if Some(&cur) == toplevel.as_ref() {
                    break; // never shape the toplevel itself
                }
                cur.input_shape_combine_region(&region, 0, 0);
                eprintln!(
                    "spike: input shape applied to win {}x{} (hole {HOLE_X},{HOLE_Y} {HOLE_W}x{HOLE_H})",
                    cur.width(),
                    cur.height()
                );
                w = cur.parent();
            }
        } else {
            eprintln!("spike: shell has no GdkWindow yet");
        }
        glib::ControlFlow::Break
    };
    glib::timeout_add_seconds_local(2, apply_shape.clone());
    glib::timeout_add_seconds_local(5, apply_shape);

    gtk::main();
}
