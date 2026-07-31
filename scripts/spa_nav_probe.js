/* Engine-agnostic SPA-navigation probe.
 *
 * Installed once per page. Everything it measures is reachable in BOTH
 * JavaScriptCore (WebKitGTK) and V8 (Chromium); nothing here depends on
 * longtask / event-timing / long-animation-frame, which WebKit does not
 * implement.
 *
 * Instruments
 *  - a 4 ms self-rescheduling timer. A timer that cannot fire is a main
 *    thread that is busy, so the gap distribution IS the main-thread
 *    blocking profile, in any engine.
 *  - a 20 ms poll of the element count and of the message-node count. This is
 *    the observer-FREE activity signal, so a run with `observe:"none"`
 *    still has a settle criterion. It exists to falsify the probe itself:
 *    a MutationObserver over a 4k-node subtree with attributes+characterData
 *    is not free, and if it were the cost being measured, `observe:"none"`
 *    would collapse the number.
 *  - the fetch/XHR wrappers and PerformanceResourceTiming for bytes.
 *
 * ⚠ transferSize/decodedBodySize are reported RAW and never used as a
 * cache-hit test: WebKitGTK reports BOTH as 0 on a hit where Chromium
 * reports transferSize 0 / decodedBodySize > 0.
 */
(function () {
  if (window.__spa) return "already";

  var MSG_SEL = '[id^="message-"]';

  var S = {
    armed: false,
    t0: 0,
    ticks: [],
    lastTick: 0,
    fetches: [],
    muts: [],           // [t, added, kind]
    mutKinds: { childList: 0, attributes: 0, characterData: 0 },
    poll: [],           // [t, nodeCount, msgCount]
    lastPollChange: -1,
    frames: [],
    routeAt: null,
    startPath: null,
    wantFrames: false,
    msgSettleAt: null,
    msgFirstAt: null,
  };
  window.__spaS = S;

  /* ---- main-thread jank sampler -------------------------------------- */
  function tick() {
    var now = performance.now();
    if (S.armed) {
      var gap = now - S.lastTick;
      if (gap > 16) S.ticks.push([S.lastTick, now]);
      if (S.routeAt === null && S.startPath !== null &&
          location.pathname !== S.startPath) {
        S.routeAt = now;
      }
    }
    S.lastTick = now;
    setTimeout(tick, 4);
  }
  S.lastTick = performance.now();
  setTimeout(tick, 4);

  /* ---- observer-free churn poll --------------------------------------- */
  var lastNodes = -1, lastMsgs = -1, msgStable = 0;
  function pollTick() {
    if (S.armed) {
      var now = performance.now();
      var n = document.getElementsByTagName("*").length;
      var m = document.querySelectorAll(MSG_SEL).length;
      if (n !== lastNodes || m !== lastMsgs) {
        S.lastPollChange = now;
        S.poll.push([Math.round((now - S.t0) * 10) / 10, n, m]);
        msgStable = now;
        if (m > 0 && S.msgFirstAt === null) S.msgFirstAt = now;
      } else if (m > 0 && S.msgSettleAt === null && now - msgStable > 200) {
        S.msgSettleAt = msgStable;
      }
      lastNodes = n; lastMsgs = m;
    }
    setTimeout(pollTick, 20);
  }
  setTimeout(pollTick, 20);

  /* ---- network -------------------------------------------------------- */
  var origFetch = window.fetch;
  window.fetch = function (input) {
    var url = typeof input === "string" ? input : (input && input.url) || "";
    var rec = { url: String(url).slice(0, 200), t0: performance.now(), t1: null, kind: "fetch" };
    if (S.armed) S.fetches.push(rec);
    return origFetch.apply(this, arguments).then(function (res) {
      rec.t1 = performance.now();
      return res;
    }, function (e) { rec.t1 = performance.now(); throw e; });
  };
  var origOpen = XMLHttpRequest.prototype.open;
  var origSend = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (m, u) {
    this.__spaUrl = String(u).slice(0, 200);
    return origOpen.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function () {
    var self = this;
    var rec = { url: self.__spaUrl || "", t0: performance.now(), t1: null, kind: "xhr" };
    if (S.armed) S.fetches.push(rec);
    self.addEventListener("loadend", function () { rec.t1 = performance.now(); });
    return origSend.apply(this, arguments);
  };

  /* ---- DOM churn ------------------------------------------------------ */
  var mo = new MutationObserver(function (list) {
    if (!S.armed) return;
    var added = 0, now = performance.now();
    for (var i = 0; i < list.length; i++) {
      added += list[i].addedNodes.length;
      S.mutKinds[list[i].type] = (S.mutKinds[list[i].type] || 0) + 1;
    }
    S.muts.push([now, added]);
  });

  function countNodes() { return document.getElementsByTagName("*").length; }

  /* ---- forced-reflow accounting --------------------------------------
   * Every one of these reads cannot answer without a clean layout, so a
   * read taken while layout is dirty forces a synchronous style recalc +
   * layout. Wrapping them gives the CALL COUNT and the TIME SPENT INSIDE
   * them, which is the difference between "the app is doing a lot of JS"
   * and "the app is thrashing layout and this engine's layout is dear".
   *
   * ⚠ The wrappers are not free. `reflow.calls` is reported so a run whose
   * wrapper overhead could matter is visible rather than assumed away, and
   * this is a SEPARATE arm — never mixed into the primary timing arm. */
  var REFLOW = { installed: false, on: false, calls: {}, ms: {}, stacks: [], stackFor: null };
  function bump(name, dt) {
    REFLOW.calls[name] = (REFLOW.calls[name] || 0) + 1;
    REFLOW.ms[name] = (REFLOW.ms[name] || 0) + dt;
    /* Name the caller for the few accessors that dominate. Capturing a stack
     * is itself expensive, so it is opt-in and restricted to accessors that
     * fire a handful of times per switch. */
    if (REFLOW.stackFor && REFLOW.stackFor.indexOf(name) >= 0 && REFLOW.stacks.length < 60) {
      var st = "";
      try { throw new Error("x"); } catch (e) { st = (e.stack || "").split("\n").slice(1, 7).join(" | "); }
      REFLOW.stacks.push([name, Math.round(dt * 10) / 10, st.slice(0, 600)]);
    }
  }
  function wrapGetter(proto, prop, label) {
    var d = Object.getOwnPropertyDescriptor(proto, prop);
    if (!d || !d.get) return;
    var orig = d.get;
    Object.defineProperty(proto, prop, {
      configurable: true, enumerable: d.enumerable,
      set: d.set,
      get: function () {
        if (!REFLOW.on) return orig.call(this);
        var t = performance.now();
        var v = orig.call(this);
        bump(label, performance.now() - t);
        return v;
      },
    });
  }
  function wrapMethod(obj, prop, label) {
    var orig = obj[prop];
    if (typeof orig !== "function") return;
    obj[prop] = function () {
      if (!REFLOW.on) return orig.apply(this, arguments);
      var t = performance.now();
      var v = orig.apply(this, arguments);
      bump(label, performance.now() - t);
      return v;
    };
  }
  function installReflow() {
    if (REFLOW.installed) return;
    REFLOW.installed = true;
    var E = Element.prototype, H = HTMLElement.prototype;
    ["clientHeight", "clientWidth", "clientTop", "clientLeft",
     "scrollHeight", "scrollWidth", "scrollTop", "scrollLeft"].forEach(function (p) {
      wrapGetter(E, p, "el." + p);
    });
    ["offsetHeight", "offsetWidth", "offsetTop", "offsetLeft", "offsetParent"].forEach(function (p) {
      wrapGetter(H, p, "el." + p);
    });
    wrapMethod(E, "getBoundingClientRect", "getBoundingClientRect");
    wrapMethod(E, "getClientRects", "getClientRects");
    wrapMethod(E, "scrollIntoView", "scrollIntoView");
    wrapMethod(E, "scrollTo", "el.scrollTo");
    wrapMethod(window, "getComputedStyle", "getComputedStyle");
    if (window.Range) wrapMethod(Range.prototype, "getBoundingClientRect", "range.gbcr");
  }
  function reflowSummary() {
    var total = 0, calls = 0, rows = [];
    for (var k in REFLOW.ms) {
      total += REFLOW.ms[k]; calls += REFLOW.calls[k];
      rows.push([k, REFLOW.calls[k], Math.round(REFLOW.ms[k] * 10) / 10]);
    }
    rows.sort(function (a, b) { return b[2] - a[2]; });
    return { total_ms: Math.round(total * 10) / 10, calls: calls, by: rows.slice(0, 14),
             stacks: REFLOW.stacks };
  }

  /* Time a FULL style recalc + layout. Nudging the root font-size dirties
   * every length in the document in both engines, which a class toggle does
   * not reliably do (a class nobody styles can be a no-op in one engine and
   * a full invalidation in the other — that asymmetry produced a bogus
   * 1 ms vs 30 ms reading on the first pass). */
  function forcedLayoutMs() {
    var de = document.documentElement;
    var prev = de.style.fontSize;
    de.getBoundingClientRect();
    var t0 = performance.now();
    de.style.fontSize = "16.013px";
    de.getBoundingClientRect();
    var t1 = performance.now();
    de.style.fontSize = prev;
    de.getBoundingClientRect();
    return t1 - t0;
  }

  function hist(pairs, width) {
    var out = {};
    for (var i = 0; i < pairs.length; i++) {
      var b = Math.floor((pairs[i][0] - S.t0) / width) * width;
      out[b] = (out[b] || 0) + 1;
    }
    return out;
  }

  function summarize(tEnd) {
    var blocking = 0, maxGap = 0, longs = [];
    for (var i = 0; i < S.ticks.length; i++) {
      var a = S.ticks[i][0], b = S.ticks[i][1], d = b - a;
      blocking += d - 4;
      if (d > maxGap) maxGap = d;
      if (d > 50) longs.push([Math.round((a - S.t0) * 10) / 10, Math.round(d * 10) / 10]);
    }
    var res = { n: 0, transfer: 0, decoded: 0, encoded: 0, dur: 0, max_end: null };
    try {
      var entries = performance.getEntriesByType("resource");
      for (var r = 0; r < entries.length; r++) {
        var e = entries[r];
        if (e.startTime < S.t0 - 1) continue;
        res.n++;
        res.transfer += e.transferSize || 0;
        res.decoded += e.decodedBodySize || 0;
        res.encoded += e.encodedBodySize || 0;
        res.dur += e.duration || 0;
        var end = e.startTime + e.duration;
        if (res.max_end === null || end > res.max_end) res.max_end = end;
      }
      res.dur = Math.round(res.dur * 10) / 10;
    } catch (e) {}

    var rel = function (t) {
      return t === null || t === undefined ? null : Math.round((t - S.t0) * 10) / 10;
    };
    var firstMut = S.muts.length ? S.muts[0][0] : null;
    var lastMut = S.muts.length ? S.muts[S.muts.length - 1][0] : null;
    var addedTotal = 0;
    for (var k = 0; k < S.muts.length; k++) addedTotal += S.muts[k][1];

    return {
      total_ms: Math.round((tEnd - S.t0) * 10) / 10,
      route_ms: rel(S.routeAt),
      content_first_ms: rel(S.msgFirstAt),
      content_settle_ms: rel(S.msgSettleAt),
      net: {
        requests: S.fetches.length,
        first_ms: S.fetches.length ? rel(S.fetches[0].t0) : null,
        rt_requests: res.n,
        rt_transfer_bytes: res.transfer,
        rt_decoded_bytes: res.decoded,
        rt_encoded_bytes: res.encoded,
        rt_sum_duration_ms: res.dur,
        rt_last_end_ms: rel(res.max_end),
        urls: S.fetches.map(function (f) {
          return { u: f.url.replace(/^https?:\/\/[^/]+/, "").slice(0, 90), s: rel(f.t0), e: rel(f.t1) };
        }).slice(0, 30),
      },
      dom: {
        first_mutation_ms: rel(firstMut),
        last_mutation_ms: rel(lastMut),
        mutation_records: S.muts.length,
        mutation_kinds: S.mutKinds,
        nodes_added: addedTotal,
        nodes_after: countNodes(),
        messages_after: document.querySelectorAll(MSG_SEL).length,
        poll_changes: S.poll.length,
        poll_last_change_ms: rel(S.lastPollChange),
        poll_trace: S.poll.slice(0, 60),
        mutation_hist_100ms: hist(S.muts, 100),
      },
      main_thread: {
        blocking_ms: Math.round(blocking * 10) / 10,
        max_gap_ms: Math.round(maxGap * 10) / 10,
        gaps_over_50ms: longs.length,
        long_gaps: longs.slice(0, 40),
      },
      frames: S.wantFrames ? { n: S.frames.length, last_ms: rel(S.frames[S.frames.length - 1]) } : null,
      forced_layout_ms: Math.round(forcedLayoutMs() * 100) / 100,
      visibility: document.visibilityState,
      hidden: document.hidden,
    };
  }

  window.__spa = {
    supported: function () {
      return {
        entryTypes: (window.PerformanceObserver && PerformanceObserver.supportedEntryTypes) || [],
        ua_engine: /AppleWebKit/.test(navigator.userAgent) && !/Chrome/.test(navigator.userAgent)
          ? "webkit" : "chromium-family",
        dpr: window.devicePixelRatio, vw: innerWidth, vh: innerHeight,
        visibility: document.visibilityState,
        hardware_concurrency: navigator.hardwareConcurrency,
      };
    },

    links: function () {
      var out = [], as = document.querySelectorAll('a[href^="/c/"]');
      for (var i = 0; i < as.length; i++) out.push(as[i].getAttribute("href"));
      return { path: location.pathname, chat_links: out, nodes: countNodes() };
    },

    /* Arm, click a sidebar chat link, wait for quiescence, report.
     * `spec` is a chat id, or {index:n} = "the nth chat link the sidebar is
     * showing RIGHT NOW that is not the one we are on". The sidebar reorders
     * by recency as you navigate, so a fixed id list rots into
     * link-not-found after a few switches. */
    run: function (spec, opts) {
      opts = opts || {};
      var quietMs = opts.quiet_ms || 400;
      var maxMs = opts.max_ms || 25000;
      var observe = opts.observe === undefined ? "full" : opts.observe;
      S.armed = false;
      S.ticks = []; S.fetches = []; S.muts = []; S.frames = []; S.poll = [];
      S.mutKinds = { childList: 0, attributes: 0, characterData: 0 };
      S.routeAt = null; S.lastPollChange = -1;
      S.msgFirstAt = null; S.msgSettleAt = null;
      S.startPath = location.pathname;
      S.wantFrames = !!opts.frames;
      lastNodes = -1; lastMsgs = -1;
      if (opts.reflow) { installReflow(); REFLOW.calls = {}; REFLOW.ms = {}; REFLOW.stacks = [];
        REFLOW.stackFor = opts.stack_for || null; }

      /* Mechanism probe: an optional stylesheet applied BEFORE the click.
       * If the cost really is document-wide layout invalidation forced by the
       * app's geometry reads, then scoping layout with `contain` must collapse
       * it — and if it does not, the hypothesis is wrong. Applied through one
       * <style> that is replaced, never appended, so repeated runs cannot
       * accumulate rules. */
      var st = document.getElementById("__spa_css");
      if (opts.pre_css) {
        if (!st) {
          st = document.createElement("style");
          st.id = "__spa_css";
          document.head.appendChild(st);
        }
        st.textContent = opts.pre_css;
      } else if (st) {
        st.textContent = "";
      }

      var el = null, sel = "";
      if (spec && typeof spec === "object" && typeof spec.index === "number") {
        var all = document.querySelectorAll('a[href^="/c/"]'), cand = [];
        for (var q = 0; q < all.length; q++) {
          if (all[q].getAttribute("href") !== location.pathname) cand.push(all[q]);
        }
        sel = "index:" + spec.index + "/" + cand.length;
        if (cand.length) el = cand[spec.index % cand.length];
      } else {
        sel = 'a[href="/c/' + spec + '"]';
        el = document.querySelector(sel);
      }
      if (!el) return Promise.resolve({ error: "link-not-found", sel: sel, path: location.pathname });
      var clickedHref = el.getAttribute("href");

      if (observe !== "none") {
        mo.observe(document.documentElement, {
          childList: true, subtree: true,
          characterData: observe === "full",
          attributes: observe === "full",
        });
      }

      return new Promise(function (resolve) {
        var t0 = performance.now();
        S.t0 = t0; S.lastTick = t0; msgStable = t0;
        S.armed = true;
        REFLOW.on = !!opts.reflow;

        if (S.wantFrames) {
          var raf = function () { if (S.armed) { S.frames.push(performance.now()); requestAnimationFrame(raf); } };
          requestAnimationFrame(raf);
        }

        if (opts.dispatch === "mouse") {
          var r = el.getBoundingClientRect();
          var cx = r.left + r.width / 2, cy = r.top + r.height / 2;
          ["pointerdown", "mousedown", "pointerup", "mouseup", "click"].forEach(function (type) {
            var C = type.indexOf("pointer") === 0 && window.PointerEvent ? PointerEvent : MouseEvent;
            el.dispatchEvent(new C(type, { bubbles: true, cancelable: true, composed: true, clientX: cx, clientY: cy, button: 0 }));
          });
        } else {
          el.click();
        }

        var lastActivity = performance.now();
        var poll = setInterval(function () {
          var now = performance.now();
          var lm = S.muts.length ? S.muts[S.muts.length - 1][0] : -1;
          var lg = S.ticks.length ? S.ticks[S.ticks.length - 1][1] : -1;
          var lp = S.lastPollChange;
          var pendingNet = S.fetches.some(function (f) { return f.t1 === null; });
          var act = Math.max(lm, lg, lp, pendingNet ? now : -1);
          if (act > lastActivity) lastActivity = act;
          if (now - lastActivity >= quietMs || now - t0 > maxMs) {
            clearInterval(poll);
            S.armed = false;
            REFLOW.on = false;
            mo.disconnect();
            var out = summarize(lastActivity);
            out.timed_out = now - t0 > maxMs;
            out.observe = observe;
            out.path_before = S.startPath;
            out.path_after = location.pathname;
            out.clicked = clickedHref;
            out.selector = sel;
            out.pre_css = opts.pre_css ? opts.pre_css.length : 0;
            out.reflow = opts.reflow ? reflowSummary() : null;
            resolve(out);
          }
        }, 20);
      });
    },
  };
  return "installed";
})();
