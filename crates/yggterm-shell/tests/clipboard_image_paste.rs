//! LOCKS for the image half of a page-surface paste.
//!
//! # Why the lock that already existed did not catch this
//!
//! `every_page_surface_is_built_able_to_paste` (vendor/dioxus-desktop) greps the
//! surface host for `.with_clipboard(true)` on every builder. It passed the
//! whole time the user could not paste an image, for two compounding reasons:
//!
//! 1. **It locks a CALL, not a capability.** The grant it checks turns on the
//!    async clipboard API. The `paste` EVENT is a different path in the engine
//!    and the grant does not touch it, so the lock was green while the symptom
//!    was real.
//! 2. **It lives in `vendor/dioxus-desktop`, which is a `[patch.crates-io]` path
//!    dependency and NOT a workspace member** — `cargo test` at the repo root
//!    never runs it, and CI names its regression tests one at a time and never
//!    names that one. It runs only if somebody types `cargo test -p
//!    dioxus-desktop` by hand. A lock nobody runs cannot go red.
//!
//! So this file lives in a workspace member, and it EXECUTES the shipped script
//! in a real JavaScriptCore context against a stub that reproduces the measured
//! engine behaviour. Delete the re-dispatch, the File, the cancel or the
//! degrade-when-refused latch and a named assertion here fails.
#![cfg(target_os = "linux")]

use javascriptcore::{Context, ContextExt, ExceptionExt, ValueExt};

/// The shipped script, imported from the crate that injects it — not a copy.
const SHIM: &str = dioxus_desktop::CLIPBOARD_IMAGE_PASTE_SHIM_JS;

/// The surface host's PRODUCT source: everything outside a `#[cfg(test)] mod`.
///
/// Same strip as the focus scan in `shell.rs`: a lock in that file may stage a
/// builder it never builds, and reading those as surface builders reports a
/// defect that does not exist.
fn web_surface_host_product_source() -> String {
    const HOST: &str = include_str!("../../../vendor/dioxus-desktop/src/web_surface.rs");
    let mut out = String::new();
    let mut in_test_module = false;
    let mut pending_test_attribute = false;
    for line in HOST.lines() {
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
        out.push_str(line);
        out.push('\n');
    }
    assert!(
        !out.contains("mod scriptlet_locks"),
        "the strip is not removing test modules, so every scan built on it is \
         reading a lock instead of the product"
    );
    out
}

/// A stub whose behaviour is the MEASUREMENT, not a guess (dev sandbox,
/// webkit2gtk 2.52.5, real Wayland clipboard, real page):
///
/// - an image-only clipboard fires a trusted `paste` whose DataTransfer is
///   `{types:[],items:[],files:[]}`;
/// - `navigator.clipboard.read()` on the very same clipboard answers
///   `[{types:["image/png"]}]` with a 124-byte blob;
/// - without the surface's clipboard grant that read answers `NotAllowedError`.
const HARNESS_JS: &str = r#"
globalThis.window = globalThis;
var CALLS = [];

function resolved(value) {
  return { then: function (ok, _err) {
    try { return resolved(ok ? ok(value) : value); } catch (e) { return rejected(e); }
  } };
}
function rejected(error) {
  return { then: function (_ok, err) {
    if (err) { return resolved(err(error)); }
    return rejected(error);
  } };
}

function FakeBlob(size) { this.size = size; }
globalThis.File = function (parts, name, options) {
  this.name = name;
  this.type = (options && options.type) || '';
  this.size = (parts && parts[0] && parts[0].size) || 0;
};
globalThis.DataTransfer = function () {
  var files = [];
  this.files = files;
  this.types = ['Files'];
  this.items = { add: function (file) { files.push(file); } };
};
globalThis.ClipboardEvent = function (type, init) {
  var self = this;
  this.type = type;
  this.clipboardData = init && init.clipboardData;
  this.bubbles = !!(init && init.bubbles);
  this.cancelable = !!(init && init.cancelable);
  this.defaultPrevented = false;
  this.preventDefault = function () { self.defaultPrevented = true; };
};
globalThis.URL = { createObjectURL: function () { return 'blob:stub-object-url'; } };

var pasteListeners = [];
globalThis.addEventListener = function (type, handler, _capture) {
  if (type === 'paste') { pasteListeners.push(handler); }
};

// CLIPBOARD_MODE is set by each scenario before the shim is loaded.
globalThis.navigator = {
  clipboard: {
    read: function () {
      CALLS.push('clipboard.read');
      if (CLIPBOARD_MODE === 'refused') { return rejected(new Error('NotAllowedError')); }
      if (CLIPBOARD_MODE === 'text-only') { return resolved([{ types: ['text/plain'] }]); }
      var bytes = CLIPBOARD_MODE === 'truncated' ? 0 : 124;
      return resolved([{
        types: ['image/png'],
        getType: function (type) {
          CALLS.push('getType:' + type);
          return resolved(new FakeBlob(bytes));
        }
      }]);
    }
  }
};

var appReceived = [];
var appConsumes = false;
var target = {
  isContentEditable: false,
  parentNode: null,
  dispatchEvent: function (event) {
    appReceived.push({
      type: event.type,
      files: (event.clipboardData && event.clipboardData.files || []).map(function (file) {
        return { name: file.name, type: file.type, size: file.size };
      })
    });
    if (appConsumes) { event.preventDefault(); }
    return !event.defaultPrevented;
  }
};

globalThis.document = {
  activeElement: target,
  body: target,
  execCommand: function (command, _ui, html) {
    CALLS.push('execCommand:' + command);
    globalThis.insertedHtml = html;
    return true;
  }
};

// The engine's own paste, exactly as it arrives: a trusted event carrying a
// DataTransfer the caller describes.
function firePaste(types, files) {
  var event = {
    type: 'paste',
    isTrusted: true,
    target: target,
    defaultPrevented: false,
    preventDefault: function () { event.defaultPrevented = true; },
    clipboardData: { types: types, files: files, items: [] }
  };
  for (var i = 0; i < pasteListeners.length; i++) { pasteListeners[i](event); }
  return event;
}
"#;

/// Run `<harness><shim><scenario>` in a real JSC context and hand back whatever
/// the scenario's last expression evaluates to.
fn run_scenario(setup: &str, scenario: &str) -> String {
    let context = Context::new();
    let program = format!("{setup}\n{HARNESS_JS}\n{SHIM}\n{scenario}");
    let value = context
        .evaluate(&program)
        .expect("the shim + harness must evaluate");
    if let Some(exception) = context.exception() {
        panic!(
            "the shim threw: {} (line {})",
            exception.message().unwrap_or_default(),
            exception.line_number()
        );
    }
    value.to_str().to_string()
}

/// THE BUG, END TO END.
///
/// An image-only clipboard, the empty DataTransfer WebKitGTK actually delivers,
/// and a page that does what every chat composer does: read
/// `event.clipboardData.files`. Before the shim it read nothing at all.
#[test]
fn clipboard_image_paste_shim_hands_the_page_the_file_the_engine_hid() {
    let answer = run_scenario(
        "var CLIPBOARD_MODE = 'image';",
        r#"
        var original = firePaste([], []);
        JSON.stringify({
          received: appReceived,
          originalCancelled: original.defaultPrevented,
          calls: CALLS
        });
        "#,
    );
    let seen: serde_json::Value = serde_json::from_str(&answer).expect("scenario answers JSON");

    let files = &seen["received"][0]["files"];
    assert_eq!(
        files[0]["type"], "image/png",
        "the page got no image file out of an image paste — the whole user \
         report. Answer was: {answer}"
    );
    assert_eq!(
        files[0]["size"], 124,
        "the file handed to the page is not the bytes on the clipboard: {answer}"
    );
    assert_eq!(
        seen["received"][0]["type"], "paste",
        "the file must arrive as a `paste`, the event a web app listens for: {answer}"
    );
    assert_eq!(
        seen["received"].as_array().map(Vec::len),
        Some(1),
        "the page must be handed the image ONCE: {answer}"
    );
    assert_eq!(
        seen["originalCancelled"], true,
        "the engine's own paste was left to run beside the re-delivered one, so \
         a rich composer gets the image twice (a blob <img> from the engine AND \
         the upload from the app): {answer}"
    );
}

/// The shim must be INVISIBLE on a port that gets this right. A second delivery
/// of a file the engine already handed over is a double upload.
#[test]
fn a_paste_that_already_carries_a_file_is_left_alone() {
    let answer = run_scenario(
        "var CLIPBOARD_MODE = 'image';",
        r#"
        var original = firePaste(['Files'], [{ name: 'engine.png', type: 'image/png', size: 9 }]);
        JSON.stringify({ received: appReceived, cancelled: original.defaultPrevented, calls: CALLS });
        "#,
    );
    let seen: serde_json::Value = serde_json::from_str(&answer).expect("scenario answers JSON");
    assert_eq!(
        seen["received"].as_array().map(Vec::len),
        Some(0),
        "the shim re-delivered a paste the engine had already delivered: {answer}"
    );
    assert_eq!(
        seen["cancelled"], false,
        "the shim cancelled a paste that was working: {answer}"
    );
    assert_eq!(
        seen["calls"].as_array().map(Vec::len),
        Some(0),
        "the shim read the clipboard on a paste it had no business touching: {answer}"
    );
}

/// A text paste is not the shim's business, and must not become slower or
/// stranger because the image path exists. (Text paste was never broken — only
/// the image half was reported, and the measurement agrees.)
#[test]
fn a_text_paste_is_never_cancelled_and_never_re_delivered() {
    let answer = run_scenario(
        "var CLIPBOARD_MODE = 'text-only';",
        r#"
        var original = firePaste(['text/plain'], []);
        JSON.stringify({ received: appReceived, cancelled: original.defaultPrevented });
        "#,
    );
    let seen: serde_json::Value = serde_json::from_str(&answer).expect("scenario answers JSON");
    assert_eq!(
        seen["cancelled"], false,
        "a plain text paste is now cancelled, which breaks pasting text into \
         every page: {answer}"
    );
    assert_eq!(
        seen["received"].as_array().map(Vec::len),
        Some(0),
        "a text-only clipboard produced a synthetic file paste: {answer}"
    );
}

/// DEGRADE, don't destroy. A surface built WITHOUT `.with_clipboard(true)`
/// cannot read the clipboard (measured: `NotAllowedError`), and the shim must
/// notice and get out of the engine's way instead of cancelling pastes it has
/// nothing to replace.
#[test]
fn a_refused_clipboard_read_stops_the_shim_from_cancelling_the_engines_paste() {
    let answer = run_scenario(
        "var CLIPBOARD_MODE = 'refused';",
        r#"
        var first = firePaste([], []);
        var second = firePaste([], []);
        JSON.stringify({
          firstCancelled: first.defaultPrevented,
          secondCancelled: second.defaultPrevented,
          received: appReceived,
          calls: CALLS
        });
        "#,
    );
    let seen: serde_json::Value = serde_json::from_str(&answer).expect("scenario answers JSON");
    assert_eq!(
        seen["secondCancelled"], false,
        "the shim keeps cancelling the engine's paste on a surface whose \
         clipboard read is refused, so those pages can no longer paste an image \
         AT ALL: {answer}"
    );
    assert_eq!(
        seen["calls"].as_array().map(Vec::len),
        Some(1),
        "the refusal did not latch: the shim re-asks the engine for a clipboard \
         it has already been told it may not have, on every paste: {answer}"
    );
    assert_eq!(
        seen["received"].as_array().map(Vec::len),
        Some(0),
        "a refused read still produced a synthetic paste: {answer}"
    );
}

/// The engine's own default (a blob `<img>` into a rich editor) is what gets
/// cancelled to make room for the re-delivery. If nobody wanted the file, it
/// has to come back — otherwise the shim TOOK a behaviour that worked.
#[test]
fn an_unclaimed_image_is_put_back_where_the_engine_would_have_put_it() {
    let answer = run_scenario(
        "var CLIPBOARD_MODE = 'image';",
        r#"
        target.isContentEditable = true;
        firePaste([], []);
        JSON.stringify({ inserted: globalThis.insertedHtml || null, calls: CALLS });
        "#,
    );
    let seen: serde_json::Value = serde_json::from_str(&answer).expect("scenario answers JSON");
    let inserted = seen["inserted"].as_str().unwrap_or_default();
    assert!(
        inserted.contains("<img") && inserted.contains("blob:"),
        "nothing handled the pasted image and the shim had cancelled the \
         engine's own paste, so the image vanished from a rich editor that used \
         to accept it: {answer}"
    );
}

/// …and the restore must NOT fire when the page did take the file, or a
/// composer that uploads the image also gets it pasted inline.
#[test]
fn a_claimed_image_is_not_also_inserted_by_the_shim() {
    let answer = run_scenario(
        "var CLIPBOARD_MODE = 'image';",
        r#"
        target.isContentEditable = true;
        appConsumes = true;
        firePaste([], []);
        JSON.stringify({ inserted: globalThis.insertedHtml || null, received: appReceived });
        "#,
    );
    let seen: serde_json::Value = serde_json::from_str(&answer).expect("scenario answers JSON");
    assert!(
        seen["inserted"].is_null(),
        "the page consumed the pasted image AND the shim inserted it inline: {answer}"
    );
}

/// A read can answer with the right TYPE and NO BYTES — observed in the
/// sandbox on a surface whose clipboard offer had gone stale. An empty File is
/// worse than no File: a chat composer uploads it and the user ships a 0-byte
/// screenshot.
#[test]
fn a_zero_byte_clipboard_read_is_never_handed_to_the_page() {
    let answer = run_scenario(
        "var CLIPBOARD_MODE = 'truncated';",
        r#"
        target.isContentEditable = true;
        firePaste([], []);
        JSON.stringify({ received: appReceived, inserted: globalThis.insertedHtml || null });
        "#,
    );
    let seen: serde_json::Value = serde_json::from_str(&answer).expect("scenario answers JSON");
    assert_eq!(
        seen["received"].as_array().map(Vec::len),
        Some(0),
        "the page was handed a 0-byte image file: {answer}"
    );
    assert!(
        seen["inserted"].is_null(),
        "a 0-byte read was pasted into the editor as an image: {answer}"
    );
}

/// PLACEMENT. Every builder that opens a page carries the grant AND the shim,
/// in the SAME chain — the two are one capability, and the last recurrence
/// happened because only one of them was locked.
#[test]
fn every_page_opening_surface_stages_the_clipboard_image_paste_shim() {
    const STAGE: &str = ".with_initialization_script_for_main_only(CLIPBOARD_IMAGE_PASTE_SHIM_JS";
    let host = web_surface_host_product_source();
    let lines: Vec<&str> = host.lines().collect();

    let grants: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains(".with_clipboard(true)"))
        .map(|(index, _)| index)
        .collect();
    assert!(
        !grants.is_empty(),
        "the scan lost its anchor: no `.with_clipboard(true)` in the surface host"
    );

    for grant in grants {
        // Walk this builder chain to its terminating `;`.
        let mut staged = false;
        for line in lines.iter().skip(grant) {
            if line.contains(STAGE) {
                staged = true;
                assert!(
                    line.trim_end().ends_with(", false)"),
                    "the shim is staged main-frame-only, so a page that runs its \
                     composer in an iframe still cannot paste an image (line: {})",
                    line.trim()
                );
                break;
            }
            if line.trim_end().ends_with(';') {
                break;
            }
        }
        assert!(
            staged,
            "a builder grants the clipboard but never stages \
             CLIPBOARD_IMAGE_PASTE_SHIM_JS (web_surface.rs line {}). The grant \
             alone is NOT an image paste: WebKitGTK hands the page an EMPTY \
             DataTransfer for an image-only clipboard, so this surface's pages \
             see nothing when the user pastes a screenshot.",
            grant + 1
        );
    }
}
