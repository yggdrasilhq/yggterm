/* Controlled full-layout microbenchmark. Same document in both engines.
 * Builds N rows of prose in a scroll container, then times the two things
 * open-webui's chat switch actually does: a root style invalidation followed
 * by a geometry read (full style recalc + layout), and a bare `scrollWidth`
 * read on the container after a DOM mutation. */
(function () {
  function build(n, wordsPerRow) {
    document.body.innerHTML = "";
    var host = document.createElement("div");
    host.id = "host";
    /* ⚠ RELATIVE units, deliberately. With absolute lengths a root
     * font-size change invalidates nothing that Chromium has to
     * re-lay-out, so the full-layout probe reads 0.0 ms there and the
     * comparison is between "WebKit re-lays-out anyway" and "Chromium
     * correctly did nothing". The real app is Tailwind, i.e. rem
     * everywhere, so rem is also the faithful fixture. */
    host.style.cssText = "height:37.5rem;overflow:auto;width:56rem;font:0.875rem/1.5 sans-serif";
    var w = [];
    for (var k = 0; k < wordsPerRow; k++) w.push("token" + k);
    var text = w.join(" ");
    for (var i = 0; i < n; i++) {
      var d = document.createElement("div");
      d.className = "row";
      d.style.cssText = "padding:0.25rem 0.5rem;min-height:1.25rem";
      var s = document.createElement("span");
      s.textContent = text;
      d.appendChild(s);
      host.appendChild(d);
    }
    document.body.appendChild(host);
    return host;
  }
  /* ⚠ Every rep must set a font-size the element does not already have.
   * An earlier version alternated between two fixed values and took a
   * median; from rep 3 on it was assigning a string the style already held,
   * which invalidates nothing, so the median reported a NO-OP as the cost of
   * a layout. It read 0.0 ms in Chromium and 58 ms in WebKit and both were
   * fiction. A distinct value per rep is not a detail here — it is the
   * difference between measuring layout and measuring nothing. */
  function timeFullLayout(host, reps) {
    var de = document.documentElement, out = [];
    de.getBoundingClientRect();
    for (var i = 0; i < reps; i++) {
      var prev = de.style.fontSize;
      var t0 = performance.now();
      de.style.fontSize = (16 + (i + 1) * 0.017) + "px";
      de.getBoundingClientRect();
      out.push(Math.round((performance.now() - t0) * 100) / 100);
      de.style.fontSize = prev;
      de.getBoundingClientRect();
    }
    return out;
  }
  function timeScrollWidthAfterMutation(host, reps) {
    var out = [];
    for (var i = 0; i < reps; i++) {
      var d = document.createElement("div");
      d.className = "row";
      d.textContent = "mutation " + i;
      host.appendChild(d);           // dirty layout
      var t0 = performance.now();
      var v = host.scrollWidth;      // forced synchronous layout
      out.push(performance.now() - t0);
      if (v < 0) throw new Error("x");
      host.removeChild(d);
    }
    out.sort(function (a, b) { return a - b; });
    return Math.round(out[Math.floor(out.length / 2)] * 100) / 100;
  }
  /* The SHAPE arm. The flat fixture above says the two engines are within
   * ~2x on raw layout, which does not explain a 40x gap on the real app.
   * The real app's message tree is ~40 levels of nested flexbox; this builds
   * the same node budget at a chosen depth so shape can be priced apart from
   * size. */
  function buildDeep(leafRows, depth, wordsPerRow) {
    document.body.innerHTML = "";
    var host = document.createElement("div");
    host.id = "host";
    host.style.cssText = "height:37.5rem;overflow:auto;width:56rem;font:0.875rem/1.5 sans-serif";
    var w = [];
    for (var k = 0; k < wordsPerRow; k++) w.push("token" + k);
    var text = w.join(" ");
    for (var i = 0; i < leafRows; i++) {
      var top = document.createElement("div"), cur = top;
      for (var d = 0; d < depth; d++) {
        var n = document.createElement("div");
        n.style.cssText = "display:flex;flex-direction:column;min-width:0;flex:1 1 auto";
        cur.appendChild(n);
        cur = n;
      }
      var s = document.createElement("span");
      s.textContent = text;
      cur.appendChild(s);
      host.appendChild(top);
    }
    document.body.appendChild(host);
    return host;
  }

  var res = [];
  [[500, 12], [1000, 12], [2000, 12], [4000, 12], [4000, 40]].forEach(function (cfg) {
    var host = build(cfg[0], cfg[1]);
    var nodes = document.getElementsByTagName("*").length;
    res.push({
      shape: "flat", rows: cfg[0], words: cfg[1], nodes: nodes,
      full_layout_ms_series: timeFullLayout(host, 6),
      scrollwidth_after_mutation_ms: timeScrollWidthAfterMutation(host, 7),
    });
  });
  [[100, 10], [100, 20], [100, 40], [50, 80]].forEach(function (cfg) {
    var host = buildDeep(cfg[0], cfg[1], 12);
    var nodes = document.getElementsByTagName("*").length;
    res.push({
      shape: "deep-flex", rows: cfg[0], depth: cfg[1], nodes: nodes,
      full_layout_ms_series: timeFullLayout(host, 6),
      scrollwidth_after_mutation_ms: timeScrollWidthAfterMutation(host, 7),
    });
  });
  return JSON.stringify({ ua: navigator.userAgent.slice(0, 60), rows: res });
})();
