//! The media probe: a `layer=webkit` emitter for the pages inside web surfaces.
//!
//! Grammar and rules: `docs/spec-trace-plane-contract.md`. The Rust boundary
//! that validates every record is `yggterm-core::trace_contract`, which already
//! accepts `webkit` — the contract reserved the tag before there was an emitter
//! for it, so this file is the reservation being spent, not a new wire.
//!
//! ## Why the terminal's emitter could not just be reused
//!
//! `trace_emitter.js` drains through a mounted terminal's IPC channel. A web
//! surface is a different `WebView` in a different content process showing a
//! page we did not write, and it has exactly one channel home:
//! `window.webkit.messageHandlers.yggtermSurface`. So the ring and the drain
//! discipline are copied deliberately (they are the part that must not be got
//! wrong) while the transport is the surface channel.
//!
//! ## ⛔ What this may not record, and why the rule is sharper here
//!
//! The terminal emitter sits over the user's own work. This one sits over
//! *arbitrary web pages*, where the URL, the document title, a track label and
//! a media `src` are all content — several of them are content that names what
//! someone was watching. §8 of the contract forbids recording the content
//! plane; here the practical rule is stricter and simpler:
//!
//! **No string that came from the page ever reaches a record.** Every field
//! below is a number, a boolean, or one of a fixed set of tags this file
//! defines. `src_kind` is `mse`/`direct`/`none` — a three-way classification of
//! a URL, never the URL. There is no redaction step to get wrong because
//! nothing that would need redacting is ever read.
//!
//! ⚠ That is also a privacy rule about the trace file itself, which is kept for
//! hours and quoted into reports.
//!
//! ## Rationing — an aggregate, plus boundaries and outliers
//!
//! The plane's retention is a byte budget, so a probe that doubles the write
//! rate halves how far back anyone can look. The split is the one the contract
//! prescribes:
//!
//! | record | when | why it earns its bytes |
//! |---|---|---|
//! | `media/playback_window` | every 5 s while playing | the always-on aggregate; keeps fps and clock-ratio honest without a record per frame |
//! | `media/quality_change` | the `resize` event | ⭐ a BOUNDARY, and the direct answer to "the quality dropped" — a resolution transition is the ladder moving |
//! | `media/stall` | `waiting`/`stalled` | an OUTLIER; the thing that did not happen in the steady state |
//! | `media/attach` / `media/ended` / `media/error` | lifecycle edges | boundaries; a window is uninterpretable without knowing what it belongs to |
//!
//! An element that is not playing costs nothing at all: the window timer is
//! self-suspending and the rest is event-driven.
//!
//! ## ⛔ Why frames are counted with `requestVideoFrameCallback`
//!
//! The obvious counter is `getVideoPlaybackQuality()`. It is unusable on this
//! engine, and not in a way that announces itself. Measured on the live host
//! against a playing 720p stream, two reads 5 s apart returned `totalVideoFrames`
//! 101 then 66 — the counter went BACKWARDS, so it is windowed rather than
//! cumulative, while `rVFC`'s own `presentedFrames` read 9701 across the same
//! interval. A consumer that trusts it does not get a noisy number; it gets
//! `droppedVideoFrames: 0` forever, because a counter that resets never
//! accumulates a drop. Two earlier investigations were misled by exactly that.
//!
//! ⇒ `rVFC` is the only honest frame source here, and its cost is one counter
//! increment per presented frame, armed ONLY while a video is actually playing.

/// The in-page half. Injected into every frame of every surface: media commonly
/// lives in an iframe, and the probe costs nothing in a frame that has none.
pub const MEDIA_TRACE_SHIM_JS: &str = r#"(function(){
  if (window.__yggtermMediaTrace) { return; }
  // ── ring + drain: the same three rules as the terminal emitter ──
  //   1. emit() does no I/O — it appends to a bounded ring and returns.
  //   2. the drain runs from a timer, never inline in a media event.
  //   3. the timer is SELF-SUSPENDING: no media, no wakeups, no cost.
  var RING_MAX = 256;
  var FLUSH_MS = 1000;
  var HIGH_WATER = 48;
  var WINDOW_MS = 5000;
  var ring = [];
  var seq = 0;
  var dropped = 0;
  var flushTimer = null;

  function post(batch) {
    // The one channel a surface page has to its host.
    window.webkit.messageHandlers.yggtermSurface.postMessage(JSON.stringify({
      type: "trace", records: batch
    }));
  }

  function drain() {
    flushTimer = null;
    if (!ring.length) { return; }
    var batch = ring.splice(0, ring.length);
    try {
      post(batch);
    } catch (e) {
      // The channel is gone (surface tearing down). Drop the batch rather than
      // grow without bound — the records describe a page that is going away.
      dropped += batch.length;
    }
  }

  function schedule(soon) {
    if (flushTimer !== null) { return; }
    flushTimer = setTimeout(drain, soon ? 0 : FLUSH_MS);
  }

  function emit(name, payload, kind) {
    if (ring.length >= RING_MAX) {
      // Drop the OLDEST: the newest records describe the state the page is in
      // now, which is the question being asked.
      ring.shift();
      dropped++;
    }
    var rec = {
      ts_ms: Date.now(),
      layer: "webkit",
      component: "web_surface",
      category: "media",
      name: name,
      kind: kind || "point",
      seq: ++seq,
      payload: payload
    };
    if (dropped) { rec.dropped = dropped; dropped = 0; }
    ring.push(rec);
    schedule(ring.length >= HIGH_WATER);
  }

  // ── element identity ──
  // An ordinal, assigned on first sight. ⛔ NEVER a selector, an id attribute
  // or a src: those are page content. The host stamps the row and tab, so
  // (row, tab, mid) addresses one element without the page naming anything.
  var ids = new WeakMap();
  var nextId = 0;
  function midOf(el) {
    var m = ids.get(el);
    if (m === undefined) { m = ++nextId; ids.set(el, m); }
    return m;
  }

  function srcKind(el) {
    var s = el.currentSrc || el.src || "";
    if (!s) { return "none"; }
    return s.lastIndexOf("blob:", 0) === 0 ? "mse" : "direct";
  }

  function bufferedAhead(el) {
    try {
      var b = el.buffered;
      if (!b || !b.length) { return 0; }
      return Math.round((b.end(b.length - 1) - el.currentTime) * 10) / 10;
    } catch (e) { return -1; }
  }

  function shape(el) {
    var r = el.getBoundingClientRect ? el.getBoundingClientRect() : null;
    return {
      mid: midOf(el),
      vw: el.videoWidth || 0,
      vh: el.videoHeight || 0,
      dw: r ? Math.round(r.width) : 0,
      dh: r ? Math.round(r.height) : 0,
      dpr: window.devicePixelRatio || 1
    };
  }

  // ── per-element playback window ──
  // One record per element per 5 s of PLAYING. Not a timer per element that
  // runs forever: `arm` starts it on `playing` and `disarm` stops it on
  // pause/ended/error, so an idle page schedules nothing.
  var live = new Map();

  function arm(el) {
    var mid = midOf(el);
    if (live.has(mid)) { return; }
    var st = {
      el: el, presented: 0, waits: 0, lastPresented: null,
      t0: performance.now(), m0: el.currentTime, timer: null, rvfc: true
    };
    // rVFC: one counter increment per presented frame. The engine's own
    // `presentedFrames` in the metadata is cumulative and trustworthy — unlike
    // getVideoPlaybackQuality() — so we keep the latest and difference it.
    if (typeof el.requestVideoFrameCallback === "function") {
      var cb = function (now, meta) {
        if (!live.has(mid)) { return; }   // disarmed: stop re-registering
        st.presented++;
        if (meta && typeof meta.presentedFrames === "number") {
          st.lastPresented = meta.presentedFrames;
        }
        try { el.requestVideoFrameCallback(cb); } catch (e) {}
      };
      try { el.requestVideoFrameCallback(cb); } catch (e) { st.rvfc = false; }
    } else {
      st.rvfc = false;
    }
    st.timer = setInterval(function () { closeWindow(mid, false); }, WINDOW_MS);
    live.set(mid, st);
  }

  function closeWindow(mid, final) {
    var st = live.get(mid);
    if (!st) { return; }
    var el = st.el;
    var t1 = performance.now();
    var wall = (t1 - st.t0) / 1000;
    var media = el.currentTime - st.m0;
    if (wall <= 0) { return; }
    var s = shape(el);
    emit("playback_window", {
      mid: s.mid, vw: s.vw, vh: s.vh, dw: s.dw, dh: s.dh, dpr: s.dpr,
      // ⛔ MEASURED, never the nominal 5000 — the interval can overrun when the
      // UI thread is busy, and dividing by the constant is wrong by exactly the
      // overrun, which is largest in the traces someone reads after an incident.
      window_ms: Math.round(t1 - st.t0),
      presented: st.presented,
      fps: Math.round((st.presented / wall) * 10) / 10,
      // The engine's cumulative frame count, if it gave us one.
      presented_total: st.lastPresented,
      // ⭐ The headline number: media seconds per wall second. 1.0 is healthy;
      // below 1.0 with no stall is the pipeline failing to keep real time.
      clock_ratio: Math.round((media / wall) * 1000) / 1000,
      waits: st.waits,
      buffered_ahead_s: bufferedAhead(el),
      rate: el.playbackRate,
      ready: el.readyState,
      net: el.networkState,
      src_kind: srcKind(el),
      final: !!final
    }, "window");
    if (final) {
      clearInterval(st.timer);
      live.delete(mid);
    } else {
      st.t0 = t1; st.m0 = el.currentTime; st.presented = 0; st.waits = 0;
    }
  }

  function disarm(el) { closeWindow(midOf(el), true); }

  // ── the hooks ──
  // Capture-phase listeners on the document. Media events do NOT bubble, but
  // capture still reaches them on the way DOWN, so one listener per event type
  // sees every element in the frame — including elements added later. That is
  // why there is no MutationObserver and no polling here.
  function on(type, fn) { document.addEventListener(type, fn, true); }
  function isMedia(t) { return t && (t.tagName === "VIDEO" || t.tagName === "AUDIO"); }

  on("loadedmetadata", function (e) {
    if (!isMedia(e.target)) { return; }
    var s = shape(e.target);
    emit("attach", {
      mid: s.mid, vw: s.vw, vh: s.vh, dw: s.dw, dh: s.dh, dpr: s.dpr,
      src_kind: srcKind(e.target),
      dur_s: Math.round(e.target.duration || 0),
      audio: e.target.tagName === "AUDIO"
    });
  });

  // ⭐ THE QUALITY PROBE. `resize` on a media element fires exactly when
  // videoWidth/videoHeight change — i.e. when the adaptive ladder moved. It is
  // event-driven, so it costs nothing to watch and cannot miss a transition
  // between two polls.
  var lastWH = new WeakMap();
  on("resize", function (e) {
    var el = e.target;
    if (!isMedia(el)) { return; }
    var prev = lastWH.get(el) || { w: 0, h: 0 };
    var s = shape(el);
    lastWH.set(el, { w: s.vw, h: s.vh });
    if (prev.w === s.vw && prev.h === s.vh) { return; }
    emit("quality_change", {
      mid: s.mid,
      from_w: prev.w, from_h: prev.h,
      to_w: s.vw, to_h: s.vh,
      // Which way the ladder moved. 0 is the first resolution seen.
      dir: prev.h === 0 ? 0 : (s.vh > prev.h ? 1 : (s.vh < prev.h ? -1 : 0)),
      dw: s.dw, dh: s.dh, dpr: s.dpr,
      // ⭐ The upscale the compositor is being asked for. >1 means the element
      // is bigger than the frames feeding it, which is what "it looks soft"
      // actually is, and it is invisible to every frame-rate check.
      upscale: s.vw ? Math.round((s.dw * s.dpr / s.vw) * 100) / 100 : 0,
      at_s: Math.round(el.currentTime || 0),
      buffered_ahead_s: bufferedAhead(el)
    });
  });

  function stall(kind) {
    return function (e) {
      var el = e.target;
      if (!isMedia(el)) { return; }
      var st = live.get(midOf(el));
      if (st) { st.waits++; }
      emit("stall", {
        mid: midOf(el), why: kind,
        at_s: Math.round(el.currentTime || 0),
        buffered_ahead_s: bufferedAhead(el),
        ready: el.readyState, net: el.networkState
      });
    };
  }
  on("waiting", stall("waiting"));
  on("stalled", stall("stalled"));

  on("playing", function (e) { if (isMedia(e.target)) { arm(e.target); } });
  on("pause", function (e) { if (isMedia(e.target)) { disarm(e.target); } });
  on("ended", function (e) {
    if (!isMedia(e.target)) { return; }
    disarm(e.target);
    emit("ended", { mid: midOf(e.target), at_s: Math.round(e.target.currentTime || 0) });
  });
  on("error", function (e) {
    var el = e.target;
    if (!isMedia(el)) { return; }
    disarm(el);
    // `code` is a small enum; `message` is engine text and is NOT recorded.
    emit("error", { mid: midOf(el), code: (el.error && el.error.code) || 0 });
  });
  on("ratechange", function (e) {
    if (!isMedia(e.target)) { return; }
    emit("ratechange", { mid: midOf(e.target), rate: e.target.playbackRate });
  });

  // A page going away should not strand a window that was mid-flight.
  window.addEventListener("pagehide", function () {
    live.forEach(function (_st, mid) { closeWindow(mid, true); });
    drain();
  }, true);

  window.__yggtermMediaTrace = { emit: emit };
})();
"#;
