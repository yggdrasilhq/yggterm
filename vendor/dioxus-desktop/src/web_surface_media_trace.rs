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
//! | `media/playback_window` | every 5 s while playing | the always-on aggregate; keeps fps and clock-ratio honest without a record per frame — and carries the `raf_*` compositor-cadence fields, because a healthy media clock with a hole-riddled compositor cadence is exactly the "stats say nothing, but it stutters" report and no presented-frame counter can see it |
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
      t0: performance.now(), m0: el.currentTime, timer: null,
      // Did the callback REGISTER, and did it ever actually FIRE? Two
      // different facts, and conflating them is what made this probe report a
      // healthy playback as zero frames. See `closeWindow`.
      rvfc: true, everFired: false,
      // The COMPOSITOR cadence half — see the rAF loop below for why rVFC
      // alone cannot answer "is it choppy". Exact per-window gaps, one array
      // of ≤ a few hundred small numbers, reset every window.
      rafArmed: false, rafFrames: 0, rafLast: null, rafGaps: []
    };
    // rVFC: one counter increment per presented frame. The engine's own
    // `presentedFrames` in the metadata is cumulative and trustworthy — unlike
    // getVideoPlaybackQuality() — so we keep the latest and difference it.
    if (typeof el.requestVideoFrameCallback === "function") {
      var cb = function (now, meta) {
        if (!live.has(mid)) { return; }   // disarmed: stop re-registering
        st.presented++;
        st.everFired = true;
        if (meta && typeof meta.presentedFrames === "number") {
          st.lastPresented = meta.presentedFrames;
        }
        try { el.requestVideoFrameCallback(cb); } catch (e) {}
      };
      try { el.requestVideoFrameCallback(cb); } catch (e) { st.rvfc = false; }
    } else {
      st.rvfc = false;
    }
    // ⭐ THE CADENCE HALF OF THE PROBE. rVFC counts the frames the MEDIA
    // pipeline presented, and a healthy pipeline presents every frame on its
    // own clock — measured on the live host through a visibly choppy
    // playback: rVFC said 60 fps and clock_ratio 1.0 while the page's
    // requestAnimationFrame cadence carried 250 ms holes. That is exactly the
    // "stats for nerds shows nothing dropped but it stutters" complaint, and
    // no media-pipeline counter can ever see it: the shortfall is in the
    // frames the COMPOSITOR walked in, not the frames the decoder made.
    //
    // So while the element is armed we also walk rAF — the page's own view of
    // the compositor — and keep each inter-frame gap. The loop re-registers
    // only while `live` holds this element, exactly like the rVFC loop above:
    // disarm stops it with no separate timer.
    if (typeof requestAnimationFrame === "function") {
      st.rafArmed = true;
      var raf = function (t) {
        if (!live.has(mid)) { return; }   // disarmed: stop re-registering
        if (st.rafLast !== null) {
          var gap = t - st.rafLast;
          if (gap >= 0) { st.rafGaps.push(gap); }
        }
        st.rafLast = t;
        st.rafFrames++;
        requestAnimationFrame(raf);
      };
      requestAnimationFrame(raf);
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
    var cad = rafStats(st, wall);
    emit("playback_window", {
      mid: s.mid, vw: s.vw, vh: s.vh, dw: s.dw, dh: s.dh, dpr: s.dpr,
      // ⛔ MEASURED, never the nominal 5000 — the interval can overrun when the
      // UI thread is busy, and dividing by the constant is wrong by exactly the
      // overrun, which is largest in the traces someone reads after an incident.
      window_ms: Math.round(t1 - st.t0),
      // ⛔⛔ A BLIND INSTRUMENT MUST NOT REPORT A NUMBER.
      //
      // `requestVideoFrameCallback` does not fire under every compositing
      // arrangement this shell can be launched in — measured: with under-glass
      // OFF it registered without error and never fired once, through a
      // playback whose media clock was advancing at 1.00x. Reporting
      // `fps: 0` there is not a noisy reading, it is a confident wrong one:
      // "no frames were presented" and "I cannot see frames" are opposite
      // findings and were the same number.
      //
      // ⇒ When the callback was armed and never fired, `presented` and `fps`
      // are null and `frames_blind` says why. A consumer that averages fps
      // across windows then skips these instead of averaging in zeros.
      frames_blind: st.rvfc && !st.everFired,
      presented: st.everFired ? st.presented : null,
      fps: st.everFired ? Math.round((st.presented / wall) * 10) / 10 : null,
      // The engine's cumulative frame count, if it gave us one.
      presented_total: st.lastPresented,
      // ⭐ The compositor cadence the eye actually watches: the page's own
      // requestAnimationFrame gaps over this window. `fps` above can read
      // perfect while these holes are huge — that combination IS "the stats
      // say nothing is wrong but it stutters".
      raf_frames: cad.raf_frames, raf_fps: cad.raf_fps,
      raf_p50_ms: cad.raf_p50_ms, raf_p95_ms: cad.raf_p95_ms,
      raf_max_ms: cad.raf_max_ms, raf_over_25: cad.raf_over_25,
      raf_blind: cad.raf_blind,
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
      st.rafFrames = 0; st.rafLast = null; st.rafGaps = [];
    }
  }

  function disarm(el) { closeWindow(midOf(el), true); }

  // The cadence aggregate for one closed window: exact percentiles from the
  // window's own gap list (≤ a few hundred entries — one sort per 5 s, no
  // reservoir math), or the blind shape. Same law as `frames_blind`: an
  // instrument that never saw a frame must not report a number that reads as
  // "perfectly smooth".
  function rafStats(st, wall) {
    if (!st.rafArmed || st.rafFrames === 0) {
      return { raf_frames: null, raf_fps: null, raf_p50_ms: null,
               raf_p95_ms: null, raf_max_ms: null, raf_over_25: null,
               raf_blind: st.rafArmed };
    }
    var gaps = st.rafGaps.slice().sort(function (a, b) { return a - b; });
    var pick = function (p) {
      if (!gaps.length) { return null; }
      return Math.round(gaps[Math.min(gaps.length - 1, Math.floor(p * gaps.length))] * 10) / 10;
    };
    var over25 = 0;
    for (var i = 0; i < gaps.length; i++) { if (gaps[i] > 25) { over25++; } }
    return {
      raf_frames: st.rafFrames,
      raf_fps: Math.round((st.rafFrames / wall) * 10) / 10,
      raf_p50_ms: pick(0.5),
      raf_p95_ms: pick(0.95),
      raf_max_ms: gaps.length ? Math.round(gaps[gaps.length - 1] * 10) / 10 : null,
      raf_over_25: over25,
      raf_blind: false
    };
  }

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

#[cfg(test)]
mod tests {
    use super::MEDIA_TRACE_SHIM_JS;

    /// The surface source, for the guards that are about WIRING rather than
    /// about the shim's own text.
    fn surface_source() -> &'static str {
        include_str!("web_surface.rs")
    }

    /// The shim with its `//` comments stripped — i.e. **what actually runs**.
    ///
    /// ⛔ Every banned-identifier guard below scans THIS and not the raw
    /// constant, and the distinction is not pedantry: the first version of
    /// these tests scanned the raw text and went red because the shim's own
    /// comment explaining *why* `getVideoPlaybackQuality` is forbidden
    /// contains the word `getVideoPlaybackQuality`.
    ///
    /// That is the same defect, mirrored, as a guard satisfied by the
    /// explanatory comment above the call it was meant to guard — a source
    /// scan that cannot tell code from prose is measuring the wrong document.
    /// The two ways it fails are a false red (delete the comment and the guard
    /// goes green having tested nothing) and a false green, and the second is
    /// the one nobody notices.
    ///
    /// ⇒ Strip the prose, keep the documentation, assert on the code.
    fn shim_code() -> String {
        MEDIA_TRACE_SHIM_JS
            .lines()
            .map(|line| match line.find("//") {
                // A `//` inside a string literal would be mangled by this, so
                // the shim deliberately contains no such literal — the one
                // scheme-ish string it needs is "blob:" (see `srcKind`).
                Some(at) => &line[..at],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// ⛔⛔ THE PRIVACY GUARD, and the reason it is a source scan rather than a
    /// review note.
    ///
    /// This shim runs inside pages the user chose and we did not write, and the
    /// trace file it writes into is kept for hours, read by agents, and quoted
    /// into reports. A URL, a document title, a track label and a media `src`
    /// are all content, and several of them name **what someone was watching**.
    ///
    /// The rule the module doc states is "no string that came from the page
    /// ever reaches a record", and the only way to keep that true through
    /// future edits is to make the page's string-bearing accessors unreachable
    /// from this file at all. A payload-shaped check would not do: the leak
    /// that matters arrives as someone adding `title: document.title` to an
    /// existing `emit` for a good reason, and a reviewer reads that as context
    /// rather than as content.
    ///
    /// ⚠ `currentSrc` is the deliberate exception and is why the exception is
    /// spelled out here: `srcKind` reads it to CLASSIFY it three ways and
    /// returns a tag. The guard therefore pins the classifier's shape too — a
    /// `srcKind` that started returning the URL would satisfy a naive
    /// "currentSrc appears once" check.
    #[test]
    fn no_page_authored_string_can_reach_a_record() {
        for banned in [
            "document.title",
            "location.href",
            "location.pathname",
            "location.hostname",
            "location.origin",
            "textContent",
            "innerText",
            "getAttribute",
            ".label",
            ".name",
        ] {
            assert!(
                !shim_code().contains(banned),
                "`{banned}` is page-authored content and must never be reachable from the \
                 media probe — the trace file outlives the page and is quoted into reports"
            );
        }
        // The one accessor that IS read, and the shape that makes it safe.
        assert_eq!(
            shim_code().matches("currentSrc").count(),
            1,
            "`currentSrc` may be read in exactly one place — the `srcKind` classifier"
        );
        assert!(
            MEDIA_TRACE_SHIM_JS.contains(r#"return s.lastIndexOf("blob:", 0) === 0 ? "mse" : "direct";"#),
            "`srcKind` must return a TAG, never the url it classified"
        );
    }

    /// ⛔ A probe wired on the tab path alone goes quiet at the exact moment
    /// the user moves the video somewhere more comfortable. A site that pops
    /// its player into its own window is ordinary, not exotic, and the surface
    /// that results is a full page with its own content process — invisible to
    /// a tab-only instrument, and invisible in the reassuring direction: the
    /// records simply stop, which reads as "nothing was playing".
    #[test]
    fn the_shim_is_injected_on_both_the_tab_and_the_popup_paths() {
        let injections = surface_source()
            .matches(".with_initialization_script_for_main_only(MEDIA_TRACE_SHIM_JS, false)")
            .count();
        assert_eq!(
            injections, 2,
            "the media probe must be injected on BOTH surface build paths (tab and popup); \
             found {injections}"
        );
    }

    /// ⛔ ALL FRAMES, and the `false` is the whole assertion.
    ///
    /// `with_initialization_script_for_main_only(js, true)` means main frame
    /// only, which is right for the shims around this one — a close request or
    /// a theme colour belongs to the top document. A player does not: an
    /// embedded one lives in an iframe, which is the single most common shape
    /// for video on the web. Flipping this to `true` would leave the probe
    /// silent on the majority case while every test that only checks
    /// "is it injected" stays green.
    #[test]
    fn the_shim_reaches_every_frame_because_players_live_in_iframes() {
        assert!(
            !surface_source()
                .contains(".with_initialization_script_for_main_only(MEDIA_TRACE_SHIM_JS, true)"),
            "the media probe must be injected for ALL frames (`false`): a player in an \
             iframe is the common case, and main-frame-only goes silent on it without erroring"
        );
    }

    /// ⛔ The page may not name its own surface, and the host must not let it.
    ///
    /// Same reasoning as the contract's `pid`: an emitter that could set its
    /// own identifier could set another's, and here the identifier is what the
    /// shell joins to a (row, tab). A batch that carried its own `surfaceId`
    /// would let any page attribute its records to any tab in the window — and
    /// the resulting trace would look perfectly well-formed.
    #[test]
    fn the_surface_id_is_stamped_by_the_host_not_read_from_the_message() {
        let source = surface_source();
        let start = source
            .find(r#"if message.get("type").and_then(|kind| kind.as_str()) == Some("trace")"#)
            .expect("the trace branch is gone from the surface message channel");
        let branch = &source[start..start + 700];
        assert!(
            branch.contains("surface_id,"),
            "the trace branch must stamp the handler's own `surface_id`"
        );
        for smuggled in ["surfaceId", "surface_id\")", "get(\"surface"] {
            assert!(
                !branch.contains(smuggled),
                "the trace branch must never read a surface id out of the page's message \
                 (`{smuggled}`) — a page that can name its own tab can name another's"
            );
        }
    }

    /// The three drain rules, asserted on the code that implements them.
    ///
    /// ⛔ `emit` doing I/O is the failure that froze the app once already
    /// through the older `debug` channel: a per-record path pays a lock and a
    /// write on the very thread whose stalls the probe exists to explain, and
    /// an instrument that perturbs what it measures does not return a noisy
    /// reading, it returns a reading of itself.
    #[test]
    fn emit_does_no_io_and_the_timer_self_suspends() {
        let emit_start = MEDIA_TRACE_SHIM_JS
            .find("function emit(")
            .expect("emit() is gone");
        let emit_end = MEDIA_TRACE_SHIM_JS[emit_start..]
            .find("\n  }")
            .expect("emit() body is unterminated")
            + emit_start;
        let emit_body = &MEDIA_TRACE_SHIM_JS[emit_start..emit_end];
        assert!(
            !emit_body.contains("postMessage"),
            "emit() must append to the ring and return — the drain does the I/O"
        );
        assert!(
            emit_body.contains("ring.push"),
            "emit() must append to the ring"
        );
        // Self-suspending: the only place a timer is armed is `schedule`, and
        // it arms nothing when one is already pending. An always-on interval
        // would wake an idle page forever to report that nothing happened.
        assert!(
            MEDIA_TRACE_SHIM_JS.contains("if (flushTimer !== null) { return; }"),
            "the flush timer must not stack — an idle page schedules nothing"
        );
    }

    /// ⛔ The frame counter, and why it may not be the obvious one.
    ///
    /// `getVideoPlaybackQuality()` is what every article reaches for. Measured
    /// on this engine against a playing stream, two reads five seconds apart
    /// returned `totalVideoFrames` 101 then 66 — it went BACKWARDS, so it is a
    /// windowed counter wearing a cumulative name, and `droppedVideoFrames`
    /// read from it is 0 forever because a counter that resets never
    /// accumulates a drop. Two investigations were misled by exactly that
    /// before the probe existed.
    #[test]
    fn frames_are_counted_with_rvfc_and_never_with_the_resetting_counter() {
        assert!(
            shim_code().contains("requestVideoFrameCallback"),
            "presented frames must come from rVFC"
        );
        assert!(
            !shim_code().contains("getVideoPlaybackQuality"),
            "getVideoPlaybackQuality() is windowed on this engine and resets; a record \
             built from it reports 0 dropped frames through a visible shortfall"
        );
    }

    /// ⛔⛔ A BLIND INSTRUMENT MUST NOT REPORT A NUMBER.
    ///
    /// Measured, and it is why this guard exists rather than being an idea:
    /// with under-glass compositing OFF, `requestVideoFrameCallback`
    /// registered without error and **never fired once**, through a playback
    /// whose media clock was advancing at 1.00x. The window reported
    /// `fps: 0` — and `fps: 0` on a playing video reads as a total
    /// presentation failure, which is the opposite of what was happening.
    ///
    /// Two A/B arms were then not comparable and the difference looked
    /// enormous. The same shape has burned this stack before: a compositing
    /// flag that silenced `rVFC` entirely while leaving the rendered frame
    /// byte-identical, so every pixel check passed and every frame count lied.
    ///
    /// ⇒ "no frames were presented" and "I cannot see frames" are opposite
    /// findings and must never be the same value.
    #[test]
    fn a_window_that_could_not_see_frames_reports_null_and_not_zero() {
        let code = shim_code();
        assert!(
            code.contains("frames_blind: st.rvfc && !st.everFired"),
            "a window must say when the frame callback never fired"
        );
        assert!(
            code.contains("fps: st.everFired ?"),
            "fps must be null when the callback never fired, never 0"
        );
        assert!(
            code.contains("presented: st.everFired ?"),
            "presented must be null when the callback never fired, never 0"
        );
        // Registering is not firing. Conflating them is the whole defect.
        assert!(
            code.contains("st.everFired = true;"),
            "the callback must record that it actually fired, not merely that it registered"
        );
    }

    /// ⛔ A window's span is MEASURED, never the nominal interval.
    ///
    /// The interval overruns whenever the UI thread is busy — which is exactly
    /// the trace someone reads after an incident. A consumer dividing by the
    /// constant computes a rate wrong by the whole overrun, and it is wrong
    /// most when it matters most.
    #[test]
    fn a_playback_window_reports_the_span_it_actually_covered() {
        assert!(
            MEDIA_TRACE_SHIM_JS.contains("window_ms: Math.round(t1 - st.t0)"),
            "window_ms must be the measured span between the window's own two reads"
        );
        assert!(
            !MEDIA_TRACE_SHIM_JS.contains("window_ms: WINDOW_MS"),
            "window_ms must never be the nominal interval"
        );
    }

    /// ⭐ The cadence half, and the complaint it exists for.
    ///
    /// Measured on the live host (2026-09-04) through a playback the user
    /// called choppy: rVFC presented 60 fps with `clock_ratio` 1.00 and zero
    /// media stalls, while the page's rAF cadence ran p95 32 ms with a
    /// 248 ms hole — and pausing the video cleaned the cadence (p95 17 ms)
    /// without touching the page. The media pipeline was innocent; the
    /// shortfall was in compositor frames. A probe that only counted
    /// presented frames would have called that playback perfect forever,
    /// which is precisely the "stats for nerds shows nothing dropped but it
    /// stutters" report this field group answers.
    #[test]
    fn the_window_carries_the_compositor_cadence_and_never_a_silent_zero() {
        let code = shim_code();
        assert!(
            code.contains("st.rafArmed = true;"),
            "arming playback must also arm the rAF cadence walker"
        );
        assert!(
            code.matches("if (!live.has(mid)) { return; }").count() >= 2,
            "the rAF loop must be disarmed by the same `live` gate as rVFC, or it \
             outlives the window and reports gaps from a paused element"
        );
        assert!(
            code.contains("raf_blind: st.rafArmed"),
            "a cadence that never fired must report blind, not smooth"
        );
        assert!(
            code.contains("raf_frames: null"),
            "blind cadence fields must be null, never 0 — null and zero are \
             opposite findings, as with `frames_blind`"
        );
        assert!(
            code.contains("raf_max_ms: gaps.length ?"),
            "max gap is exact, taken from the window's own gap list"
        );
    }
}
