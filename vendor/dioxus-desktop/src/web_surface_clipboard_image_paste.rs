//! Ctrl+V of an IMAGE into a page surface — the repair for what WebKitGTK
//! hands the page.
//!
//! # The measurement (sandbox, 2026-07-31)
//!
//! With a PNG on the Wayland clipboard (`wl-copy --type image/png`), the
//! engine's own paste command fires a real, trusted `paste` event at the page
//! and the DataTransfer on it is **empty**:
//!
//! ```text
//! {"types":[],"items":[],"files":[],"ev":"paste","isTrusted":true}
//! ```
//!
//! …while the engine's DEFAULT action for that same event inserts
//! `<img src="blob:…">` into a rich editor. So the image is right there in the
//! engine — it is simply never offered to the page. A web app that uploads a
//! pasted image (open-webui, every chat composer) reads
//! `event.clipboardData.files`, finds nothing, and does nothing. That is the
//! user's report: "image pasting from clipboard to webapp does not work."
//!
//! The same page, same clipboard, same paste, in a THIRTY-LINE vanilla
//! webkit2gtk-4.1 window with no yggterm, no wry and no dioxus in the process,
//! prints the identical empty DataTransfer. **The defect is WebKitGTK's, not
//! ours** — its paste DataTransfer never carries a file at all: a clipboard
//! holding a `text/uri-list` of `file:///…/clip.png` arrives as
//! `items:[{kind:"string"}]`, `files:[]` too. Text is unaffected
//! (`types:["text/plain"]`), which is why only the IMAGE half was reported.
//!
//! # Why a shim is the fix and not a workaround
//!
//! The image is reachable from the page's own world by the OTHER clipboard
//! API. Measured on the same surface, same clipboard:
//!
//! ```text
//! navigator.clipboard.read() ⇒ [{types:["image/png"], blobs:[{size:124}]}]
//! ```
//!
//! …and that API works **only because the surface was built with
//! `.with_clipboard(true)`**: the same probe in a vanilla view without the
//! grant answers `NotAllowedError`. So the grant this crate already ships is
//! load-bearing for the repair, and the repair is: when a paste arrives with no
//! file, read the image the engine hid, and re-deliver the paste the way every
//! other browser would have delivered it in the first place.
//!
//! # What this does NOT do
//!
//! It does not touch a paste the engine got right (any DataTransfer that
//! already carries a file is left alone), it never invents content the system
//! clipboard does not hold, and it does not read the clipboard except inside a
//! paste the user just performed.

/// Injected into every page surface (and every popup a page opens), all frames,
/// the page's own world — it exists to be seen BY the page's listeners.
///
/// Control flow is deliberately callback-shaped rather than `async`/`await`:
/// the only promises it ever touches are the two the engine hands it
/// (`navigator.clipboard.read()` and `ClipboardItem.getType()`), which is what
/// lets `clipboard_image_paste_shim_hands_the_page_the_file_the_engine_hid`
/// drive the real shipped script to completion in a stub with no event loop.
pub const CLIPBOARD_IMAGE_PASTE_SHIM_JS: &str = r#"(function () {
  var FLAG = '__yggtermClipboardImagePasteShim';
  if (window[FLAG]) { return; }
  window[FLAG] = true;
  var SYNTHETIC = '__yggtermSyntheticClipboardImagePaste';
  // Latched off the first time the read is refused, so a surface built without
  // the clipboard grant degrades to exactly the engine's own behaviour instead
  // of cancelling a paste it cannot replace.
  var readUsable = true;

  function clipboardReadable() {
    return readUsable && !!(navigator.clipboard && navigator.clipboard.read);
  }

  function extensionFor(type) {
    var slash = String(type).indexOf('/');
    var ext = slash >= 0 ? String(type).slice(slash + 1) : '';
    ext = ext.split('+')[0].split(';')[0];
    return ext || 'png';
  }

  function readClipboardImage(done) {
    if (!clipboardReadable()) { done(null); return; }
    var failed = function () { readUsable = false; done(null); };
    var reading;
    try { reading = navigator.clipboard.read(); } catch (error) { failed(); return; }
    if (!reading || typeof reading.then !== 'function') { failed(); return; }
    reading.then(function (list) {
      var items = list || [];
      for (var i = 0; i < items.length; i++) {
        var item = items[i];
        var types = (item && item.types) || [];
        for (var j = 0; j < types.length; j++) {
          var type = String(types[j]);
          if (type.indexOf('image/') !== 0) { continue; }
          var fetching;
          try { fetching = item.getType(type); } catch (error) { failed(); return; }
          if (!fetching || typeof fetching.then !== 'function') { failed(); return; }
          fetching.then(function (blob) {
            // A ZERO-BYTE blob is not an image. WebKitGTK can answer a read
            // with the right TYPE and no bytes (observed in the sandbox once a
            // surface's clipboard offer had gone stale), and handing a page an
            // empty File is worse than handing it nothing: a chat composer
            // uploads it.
            if (!blob || !blob.size) { done(null); return; }
            var file;
            try {
              file = new File([blob], 'clipboard-image.' + extensionFor(type), { type: type });
            } catch (error) { done(null); return; }
            done(file);
          }, function () { done(null); });
          return;
        }
      }
      done(null);
    }, failed);
  }

  function editableAncestor(node) {
    var walked = 0;
    while (node && walked < 64) {
      if (node.isContentEditable) { return node; }
      node = node.parentNode || null;
      walked += 1;
    }
    return null;
  }

  window.addEventListener('paste', function (event) {
    if (!event || event[SYNTHETIC]) { return; }
    var data = event.clipboardData;
    // The engine got it right (or will, on a port that does): leave it alone.
    if (data && data.files && data.files.length > 0) { return; }
    var types = (data && data.types) || [];
    // An EMPTY DataTransfer is the image-only clipboard. The engine's own
    // default for it drops a blob <img> into a rich editor, which would land
    // BESIDE whatever the page does with the file about to be handed to it, so
    // this is the one case worth cancelling — and only when the read that
    // replaces it is actually available.
    var cancelled = types.length === 0 && clipboardReadable();
    if (cancelled) { event.preventDefault(); }
    var target = event.target && typeof event.target.dispatchEvent === 'function'
      ? event.target
      : (document.activeElement || document.body || document);
    readClipboardImage(function (file) {
      if (!file) { return; }
      var transfer;
      try {
        transfer = new DataTransfer();
        transfer.items.add(file);
      } catch (error) { return; }
      var synthetic;
      try {
        synthetic = new ClipboardEvent('paste', {
          clipboardData: transfer,
          bubbles: true,
          cancelable: true
        });
      } catch (error) { return; }
      synthetic[SYNTHETIC] = true;
      var delivered = target.dispatchEvent(synthetic);
      var consumed = delivered === false || synthetic.defaultPrevented === true;
      if (consumed || !cancelled) { return; }
      // Nobody wanted the file and the engine's paste was cancelled on its
      // behalf: put the image back exactly where the engine would have put it.
      if (!editableAncestor(target)) { return; }
      try {
        document.execCommand('insertHTML', false,
          '<img src="' + URL.createObjectURL(file) + '">');
      } catch (error) {}
    });
  }, true);
})();
"#;
