// Burn Scan: the scan flow on "/scan" and the saved items on "/items".
// Everything shown comes from the server's replies; this file adds no data of its own.
// Text is always set with textContent, never innerHTML.
(function () {
  "use strict";

  var STORE_KEY = "burnscan.v1";
  var SPACES = ["My room", "Cargo", "Habitat"];
  var LONG_EDGE = 1568; // px, before upload
  var THUMB_EDGE = 160; // px, kept in the browser with a saved item
  var UNSURE = 0.6; // below this the model's guess is flagged

  // Verdict colours. Full class names so Tailwind keeps them.
  var VERDICT = {
    burned: { text: "text-burn", edge: "border-burn" },
    went_out: { text: "text-amber", edge: "border-amber" },
    no_ignition: { text: "text-safe", edge: "border-safe" },
    unrecorded: { text: "text-fg", edge: "border-fg" },
    not_at_level: { text: "text-muted", edge: "border-line" },
    none: { text: "text-dim", edge: "border-line" },
  };

  // ---------- small helpers ----------

  function el(tag, attrs, kids) {
    var node = document.createElement(tag);
    Object.keys(attrs || {}).forEach(function (k) {
      var v = attrs[k];
      if (v === null || v === undefined || v === false) return;
      if (k === "text") node.textContent = v;
      else if (k === "class") node.className = v;
      else if (k.slice(0, 2) === "on") node.addEventListener(k.slice(2), v);
      else node.setAttribute(k, v === true ? "" : v);
    });
    (kids || []).forEach(function (kid) {
      if (kid) node.appendChild(typeof kid === "string" ? document.createTextNode(kid) : kid);
    });
    return node;
  }

  function clear(node) {
    while (node.firstChild) node.removeChild(node.firstChild);
  }

  function link(text, href) {
    var safe = typeof href === "string" && (href.indexOf("https://") === 0 || href.charAt(0) === "/");
    if (!safe) return el("span", { text: text });
    var external = href.charAt(0) !== "/";
    return el("a", { text: text, href: href, class: "text-sky underline underline-offset-4", target: external ? "_blank" : null, rel: external ? "noopener" : null });
  }

  // ---------- saved items (this browser only) ----------

  function loadStore() {
    try {
      var data = JSON.parse(localStorage.getItem(STORE_KEY) || "null");
      if (data && Array.isArray(data.items)) return data;
    } catch (e) { /* unreadable or blocked: start empty */ }
    return { items: [] };
  }

  function saveStore(data) {
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify(data));
      return true;
    } catch (e) {
      return false;
    }
  }

  function moonVerdict(material) {
    var hab = (material.habitats || []).filter(function (h) { return h.key === "moon_base"; })[0];
    return hab ? hab.verdict : "none";
  }

  function itemBurned(item) {
    return ((item.result || {}).materials || []).some(function (m) { return moonVerdict(m) === "burned"; });
  }

  function itemUntested(item) {
    return ((item.result || {}).materials || []).some(function (m) { return !m.tested; });
  }

  // ---------- photo handling ----------

  function drawScaled(source, width, height, edge) {
    var scale = Math.min(1, edge / Math.max(width, height));
    var canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    canvas.getContext("2d").drawImage(source, 0, 0, canvas.width, canvas.height);
    return canvas;
  }

  // Resolves to { blob, thumb }: a JPEG no longer than LONG_EDGE, and a small data URL.
  function prepare(file) {
    return new Promise(function (resolve, reject) {
      var url = URL.createObjectURL(file);
      var img = new Image();
      img.onload = function () {
        try {
          var big = drawScaled(img, img.naturalWidth, img.naturalHeight, LONG_EDGE);
          var thumb = drawScaled(img, img.naturalWidth, img.naturalHeight, THUMB_EDGE).toDataURL("image/jpeg", 0.6);
          URL.revokeObjectURL(url);
          big.toBlob(function (blob) {
            if (blob) resolve({ blob: blob, thumb: thumb });
            else reject(new Error("no blob"));
          }, "image/jpeg", 0.85);
        } catch (e) {
          reject(e);
        }
      };
      img.onerror = function () {
        URL.revokeObjectURL(url);
        reject(new Error("unreadable"));
      };
      img.src = url;
    });
  }


  // ---------- visuals on the result ----------

  var SVG_NS = "http://www.w3.org/2000/svg";
  function svg(tag, attrs, kids) {
    var node = document.createElementNS(SVG_NS, tag);
    Object.keys(attrs || {}).forEach(function (k) { node.setAttribute(k, attrs[k]); });
    (kids || []).forEach(function (kid) { if (kid) node.appendChild(kid); });
    return node;
  }
  function svgText(text, attrs) {
    var node = svg("text", attrs);
    node.textContent = text;
    return node;
  }

  // How a test ended: one colour and one shape each, so colour is never the only cue.
  var OUTCOMES = [
    { key: "burned", label: "Burned", color: "#ff5a26" },
    { key: "went_out", label: "Went out on its own", color: "#e0a03c" },
    { key: "no_ignition", label: "Did not ignite", color: "#4cc27a" },
    { key: "unrecorded", label: "Outcome not recorded", color: "#a8a496" },
  ];
  function outcomeMark(key, x, y, r) {
    var o = OUTCOMES.filter(function (it) { return it.key === key; })[0] || OUTCOMES[3];
    var ring = { stroke: "#0e0e0c", "stroke-width": 1.5 };
    var mark;
    if (key === "burned") mark = svg("circle", { cx: x, cy: y, r: r, fill: o.color });
    else if (key === "went_out") mark = svg("rect", { x: x - r * 0.8, y: y - r * 0.8, width: r * 1.6, height: r * 1.6, fill: o.color, transform: "rotate(45 " + x + " " + y + ")" });
    else if (key === "no_ignition") mark = svg("path", { d: "M" + x + " " + (y - r) + "L" + (x + r) + " " + (y + r * 0.8) + "L" + (x - r) + " " + (y + r * 0.8) + "Z", fill: o.color });
    else return svg("circle", { cx: x, cy: y, r: r - 1, fill: "#0e0e0c", stroke: o.color, "stroke-width": 2 });
    Object.keys(ring).forEach(function (k) { mark.setAttribute(k, ring[k]); });
    return mark;
  }


  // A ring split into coloured arcs. slices: [{ label, value, color }]. The number in the
  // middle is the total; arcs draw themselves in clockwise from the top.
  function donut(slices, size, centre, under) {
    slices = slices.filter(function (s) { return s.value > 0; });
    var total = slices.reduce(function (sum, s) { return sum + s.value; }, 0);
    if (!total) return null;
    var r = 40, width = 13, around = 2 * Math.PI * r, gap = slices.length > 1 ? 2.5 : 0, at = 0;
    var kids = [svg("circle", { cx: 50, cy: 50, r: r, fill: "none", stroke: "#2a2924", "stroke-width": width })];
    slices.forEach(function (s, i) {
      var length = (s.value / total) * around;
      var arc = svg("circle", {
        cx: 50, cy: 50, r: r, fill: "none", stroke: s.color, "stroke-width": width,
        "stroke-dasharray": Math.max(0.5, length - gap) + " " + around, "stroke-dashoffset": -at,
        transform: "rotate(-90 50 50)", class: "donut-arc",
      });
      arc.style.animationDelay = (0.15 + i * 0.12) + "s";
      var tip = svg("title", {});
      tip.textContent = s.label + ": " + s.value + " of " + total;
      arc.appendChild(tip);
      kids.push(arc);
      at += length;
    });
    kids.push(svgText(String(centre), { x: 50, y: under ? 51 : 56, "text-anchor": "middle", "font-size": 22, "font-weight": 700, fill: "#ece8dd" }));
    if (under) kids.push(svgText(under, { x: 50, y: 64, "text-anchor": "middle", "font-size": 9, fill: "#a8a496" }));
    return svg("svg", { viewBox: "0 0 100 100", width: size, height: size, class: "donut", role: "img",
      "aria-label": slices.map(function (s) { return s.label + " " + s.value; }).join(", ") + ", of " + total + "." }, kids);
  }

  function legendRow(color, text) {
    var dot = el("span", { class: "mat-dot" });
    dot.style.background = color;
    return el("li", { class: "flex items-center gap-2" }, [dot, el("span", { text: text })]);
  }

  // How NASA's tests of this material ended, as a ring. Counts every test, including the
  // ones with no oxygen level, so it can show more tests than the oxygen chart below it.
  function outcomeDonut(material) {
    var data = material.chart;
    if (!data || !data.outcomes || !data.tests) return null;
    var slices = OUTCOMES.map(function (o) { return { label: o.label, value: data.outcomes[o.key] || 0, color: o.color }; });
    var ring = donut(slices, 112, data.tests, data.tests === 1 ? "test" : "tests");
    if (!ring) return null;
    return el("figure", { class: "donut-figure" }, [
      ring,
      el("div", { class: "min-w-0" }, [
        el("figcaption", { class: "text-[13px] font-medium", text: "How NASA’s tests of it ended" }),
        el("ul", { class: "mt-2 space-y-1 text-[13px] text-muted" }, slices.filter(function (s) { return s.value > 0; }).map(function (s) {
          return legendRow(s.color, s.label + " · " + s.value);
        })),
        el("p", { class: "mt-2 text-[11px] leading-4 text-dim", text: "All tests in the database, at any oxygen level." }),
      ]),
    ]);
  }

  // How much of the item has published tests: in NASA's database, from other sources only, or none.
  // Names that cover many materials. Tests are filed per material, so these never match:
  // the person is asked which one it is. Every option has tests in the data.
  var FAMILIES = [
    { names: ["plastic", "plastics", "hard plastic", "soft plastic", "rigid plastic", "moulded plastic", "molded plastic"], word: "Plastic",
      options: ["ABS", "polypropylene", "polyethylene", "PVC", "polystyrene", "polycarbonate", "PET", "acrylic", "nylon"] },
    { names: ["fabric", "textile", "cloth", "synthetic fabric", "woven fabric", "knit fabric"], word: "Fabric",
      options: ["cotton", "polyester", "polyester/cotton blend", "nylon", "wool", "silk", "rayon", "linen", "spandex"] },
    { names: ["rubber", "synthetic rubber"], word: "Rubber",
      options: ["natural rubber", "silicone rubber", "neoprene", "nitrile rubber", "epdm rubber", "butyl rubber"] },
    { names: ["metal", "metals", "metal alloy"], word: "Metal",
      options: ["stainless steel", "carbon steel", "aluminium", "copper", "brass", "zinc", "titanium", "magnesium"] },
    { names: ["foam", "padding", "foam padding"], word: "Foam",
      options: ["polyurethane foam", "polyethylene foam", "expanded polystyrene foam", "melamine foam", "polyimide foam"] },
  ];
  function family(name) {
    var key = String(name || "").trim().toLowerCase();
    for (var i = 0; i < FAMILIES.length; i++) if (FAMILIES[i].names.indexOf(key) >= 0) return FAMILIES[i];
    return null;
  }

  function coverageDonut(result) {
    var all = result.materials || [];
    if (all.length < 1 || result.data !== "ok") return null;
    var tested = all.filter(function (m) { return m.tested; }).length;
    var elsewhere = all.filter(function (m) { return !m.tested && (m.other || []).length; }).length;
    var none = all.length - tested - elsewhere;
    var BLUE = "#7f93ff", TEAL = "#58b8c6", GREY = "#55524a";
    var ring = donut([
      { label: "In NASA’s microgravity database", value: tested, color: BLUE },
      { label: "Tests from other sources only", value: elsewhere, color: TEAL },
      { label: "No test found", value: none, color: GREY },
    ], 112, (tested + elsewhere) + "/" + all.length, "with tests");
    if (!ring) return null;
    var headline;
    if (tested === all.length) {
      headline = all.length === 1 ? "NASA has tested this material." : "NASA has tested all " + all.length + " of its materials.";
    } else if (!elsewhere) {
      headline = tested === 0 ? "None of its materials are in NASA’s tests." : "NASA has tested " + tested + " of its " + all.length + " materials.";
    } else {
      headline = (tested + elsewhere) + " of its " + all.length + (all.length === 1 ? " material" : " materials") + (tested + elsewhere === 1 ? " has" : " have") + " published tests: "
        + tested + " in NASA’s microgravity database, " + elsewhere + " from other sources only.";
    }
    return el("figure", { class: "donut-figure donut-figure-top" }, [
      ring,
      el("div", { class: "min-w-0" }, [
        el("figcaption", { class: "text-[17px] leading-6 font-semibold", text: headline }),
        el("ul", { class: "mt-2 space-y-1 text-[13px] text-muted" }, all.map(function (m) {
          var others = (m.other || []).length;
          if (m.tested) return legendRow(BLUE, m.query + " · in NASA’s database");
          if (others) return legendRow(TEAL, m.query + " · " + others + (others === 1 ? " test" : " tests") + " from other sources");
          return legendRow(GREY, m.query + (family(m.query) ? " · a family of materials, pick the exact one" : " · no test found"));
        })),
      ]),
    ]);
  }

  // ---------- the flammability map: oxygen against pressure ----------

  // The two habitats, from the challenge brief: 10.2 psi = 70.3 kPa, 14.7 psi = 101.4 kPa.
  var PINS = [
    { name: "Moon Base", o2: 30, kpa: 70.3, text: "30% O₂, 10.2 psi (70.3 kPa)" },
    { name: "ISS", o2: 21, kpa: 101.4, text: "21% O₂, 14.7 psi (101.4 kPa)" },
  ];
  var BAR_DASH = { burned: null, went_out: "7 4", no_ignition: "2 4", unrecorded: "1 5" };

  function outcomeOf(key) {
    return OUTCOMES.filter(function (o) { return o.key === key; })[0] || OUTCOMES[3];
  }
  function tidy(v) {
    return String(Math.round(v * 10) / 10);
  }
  function spanText(pair, unit) {
    return pair[1] > pair[0] ? tidy(pair[0]) + " to " + tidy(pair[1]) + unit : tidy(pair[0]) + unit;
  }
  function gravityWords(g) {
    return g === "Earth" ? "Earth gravity" : g === "microgravity" ? "weightlessness" : g || "gravity not stated";
  }

  // Oxygen limits from the other-sources layer, as labelled vertical lines. They are a different
  // kind of test from the points, so they never become points.
  function limitMarkers(material) {
    var seen = {}, out = [];
    (material.other || []).forEach(function (row) {
      if (typeof row.limit !== "number" || !(row.limit > 0 && row.limit <= 100)) return;
      var label = gravityWords(row.gravity) + ", " + row.publisher;
      var key = row.limit + "|" + label;
      if (seen[key]) return;
      seen[key] = true;
      out.push({ limit: row.limit, label: label, test: row.test });
    });
    return out.sort(function (a, b) { return a.limit - b.limit; });
  }

  // opts.compact: the small version on an atlas card. Same points, pins and axes; no detail
  // panel, legend or limit lines, and the marks are not separate buttons (the card is).
  function flammabilityMap(material, opts) {
    var compact = !!(opts && opts.compact);
    var data = material.map;
    if (!data || !data.points || !data.points.length) return null;
    var points = data.points;
    var markers = compact ? [] : limitMarkers(material);

    // Axes cover the tests and both habitats. Pressure switches to a log scale when the tests span more than a factor of 12.
    var o2s = [], kpas = [];
    points.forEach(function (p) { o2s.push(p.o2[0], p.o2[1]); kpas.push(p.kpa[0], p.kpa[1]); });
    PINS.forEach(function (h) { o2s.push(h.o2); kpas.push(h.kpa); });
    var lo = Math.min(15, Math.floor(Math.min.apply(null, o2s) / 5) * 5);
    var hi = Math.max(35, Math.ceil(Math.max.apply(null, o2s) / 5) * 5);
    var positive = kpas.filter(function (v) { return v > 0; });
    var kMin = Math.min.apply(null, positive), kMax = Math.max.apply(null, kpas);
    var log = kMax / kMin > 12;
    var yLo, yHi, yTicks = [];
    if (log) {
      yLo = Math.pow(10, Math.floor(Math.log10(kMin)));
      yHi = Math.pow(10, Math.ceil(Math.log10(kMax)));
      for (var d = yLo; d <= yHi; d *= 10) [1, 2, 5].forEach(function (m) { if (d * m >= yLo && d * m <= yHi) yTicks.push(d * m); });
    } else {
      var stepY = kMax - kMin > 120 ? 50 : kMax - kMin > 50 ? 20 : 10;
      yLo = Math.max(0, Math.floor((kMin - stepY * 0.6) / stepY) * stepY);
      yHi = Math.ceil((kMax + stepY * 0.6) / stepY) * stepY;
      for (var t = yLo; t <= yHi; t += stepY) yTicks.push(t);
    }
    var W = 328, left = 40, right = 12, top = 24, plotH = compact ? 118 : 196, bottom = top + plotH, H = bottom + 34;
    function x(v) { return left + ((Math.max(lo, Math.min(hi, v)) - lo) / (hi - lo)) * (W - left - right); }
    function y(v) {
      var f = log ? (Math.log10(Math.max(v, yLo)) - Math.log10(yLo)) / (Math.log10(yHi) - Math.log10(yLo)) : (v - yLo) / (yHi - yLo);
      return bottom - Math.max(0, Math.min(1, f)) * plotH;
    }

    var kids = [svg("rect", { x: left, y: top, width: W - left - right, height: plotH, fill: "#141412", stroke: "#3b3930", "stroke-width": 1 })];
    yTicks.forEach(function (v) {
      kids.push(svg("line", { x1: left, x2: W - right, y1: y(v), y2: y(v), stroke: "#2a2924", "stroke-width": 1 }));
      kids.push(svgText(tidy(v), { x: left - 5, y: y(v) + 3, "text-anchor": "end", "font-size": 9, fill: "#a8a496" }));
    });
    for (var tick = lo; tick <= hi; tick += (hi - lo > 50 ? 20 : hi - lo > 25 ? 10 : 5)) {
      kids.push(svg("line", { x1: x(tick), x2: x(tick), y1: top, y2: bottom, stroke: "#2a2924", "stroke-width": 1 }));
      kids.push(svgText(tick + "%", { x: x(tick), y: bottom + 13, "text-anchor": "middle", "font-size": 9, fill: "#a8a496" }));
    }
    kids.push(svgText("oxygen", { x: W - right, y: bottom + 27, "text-anchor": "end", "font-size": 9, fill: "#858174" }));
    kids.push(svgText("pressure, kPa" + (log ? " (log scale)" : ""), { x: 2, y: 12, "font-size": 9, fill: "#858174" }));

    // Limits from other sources: dashed lines with a number that the list below explains.
    markers.forEach(function (m, i) {
      m.on = m.limit >= lo && m.limit <= hi;
      if (!m.on) return;
      kids.push(svg("line", { x1: x(m.limit), x2: x(m.limit), y1: top, y2: bottom, stroke: "#e0a03c", "stroke-width": 1, "stroke-dasharray": "2 3", opacity: 0.9 }));
      kids.push(svg("rect", { x: x(m.limit) - 6, y: top + 2 + (i % 3) * 13, width: 12, height: 11, rx: 2, fill: "#141412", stroke: "#e0a03c", "stroke-width": 1 }));
      kids.push(svgText(String(i + 1), { x: x(m.limit), y: top + 10.5 + (i % 3) * 13, "text-anchor": "middle", "font-size": 8, "font-weight": 600, fill: "#e0a03c" }));
    });

    // The habitats: an open crosshair, so a test point at the same place still shows.
    PINS.forEach(function (h) {
      var px = x(h.o2), py = y(h.kpa);
      kids.push(svg("circle", { cx: px, cy: py, r: 9, fill: "none", stroke: "#ece8dd", "stroke-width": 1.2 }));
      kids.push(svg("path", { d: "M" + (px - 13) + " " + py + "H" + (px - 5) + "M" + (px + 5) + " " + py + "H" + (px + 13) + "M" + px + " " + (py - 13) + "V" + (py - 5) + "M" + px + " " + (py + 5) + "V" + (py + 13), stroke: "#ece8dd", "stroke-width": 1.2, fill: "none" }));
      var flip = px > W - 80;
      kids.push(svgText(h.name, { x: flip ? px - 15 : px + 15, y: py - 8, "text-anchor": flip ? "end" : "start", "font-size": 10, "font-weight": 600, fill: "#ece8dd" }));
    });

    var detail = el("div", { class: "map-detail", "aria-live": "polite" }, [
      el("p", { class: "text-[12px] leading-4 text-dim", text: "Tap a mark to see the paper, page and quote it comes from." }),
    ]);
    var chosen = null;
    function choose(mark, p) {
      if (chosen) chosen.removeAttribute("data-on");
      chosen = mark;
      mark.setAttribute("data-on", "true");
      clear(detail);
      var outcome = outcomeOf(p.outcome);
      detail.appendChild(el("p", { class: "flex items-center gap-2 text-[14px] font-semibold" }, [
        svg("svg", { width: 14, height: 14, viewBox: "0 0 14 14", class: "shrink-0", "aria-hidden": "true" }, [outcomeMark(p.outcome, 7, 7, 5)]),
        el("span", { text: outcome.label + " · " + spanText(p.o2, "% oxygen") + " · " + spanText(p.kpa, " kPa") }),
      ]));
      var stated = [p.o2_text ? "oxygen “" + p.o2_text + "”" : null, p.pressure_text ? "pressure “" + p.pressure_text + "”" : null].filter(Boolean);
      if (stated.length) detail.appendChild(el("p", { class: "mt-1 text-[12px] leading-4 text-muted", text: "As recorded: " + stated.join(", ") + (p.experiments && p.experiments.length ? " · " + p.experiments.join(" / ") : "") }));
      if (p.note) detail.appendChild(el("p", { class: "mt-2 text-[13px] leading-5", text: p.note }));
      if (p.quote) detail.appendChild(el("blockquote", { class: "map-quote", text: "“" + p.quote + "”" }));
      detail.appendChild(el("p", { class: "mt-2 text-[12px] leading-4" }, [
        p.href ? link(p.paper || "The paper", p.href) : el("span", { text: p.paper || "Paper not linked" }),
        p.page ? el("span", { class: "text-dim", text: " · page " + p.page }) : el("span", { class: "text-dim", text: " · page not recorded" }),
      ]));
    }

    points.forEach(function (p) {
      var outcome = outcomeOf(p.outcome);
      var x0 = x(p.o2[0]), x1 = x(p.o2[1]), y0 = y(p.kpa[0]), y1 = y(p.kpa[1]);
      var wide = p.o2[1] > p.o2[0], tall = p.kpa[1] > p.kpa[0];
      var shape;
      if (!wide && !tall) {
        shape = outcomeMark(p.outcome, x0, y0, 6);
      } else {
        // A stated range is a bar (or a box when both are ranges), never a point. The line pattern carries the outcome as well as the colour.
        var attrs = { stroke: outcome.color, "stroke-width": wide && tall ? 2 : 5, "stroke-linecap": "butt", fill: wide && tall ? outcome.color : "none", "fill-opacity": 0.14 };
        if (BAR_DASH[p.outcome]) attrs["stroke-dasharray"] = BAR_DASH[p.outcome];
        if (wide && tall) { attrs.x = x0; attrs.y = y1; attrs.width = x1 - x0; attrs.height = y0 - y1; shape = svg("rect", attrs); }
        else { attrs.x1 = x0; attrs.x2 = x1; attrs.y1 = y0; attrs.y2 = y1; shape = svg("line", attrs); }
      }
      var name = outcome.label + ", " + spanText(p.o2, " percent oxygen") + ", " + spanText(p.kpa, " kilopascals") + ". " + (p.paper || "");
      if (compact) { kids.push(shape); return; }
      var hit = svg("g", { class: "map-mark", tabindex: "0", role: "button", "aria-label": name }, [
        // A wider invisible target so a thin bar or small mark is easy to tap.
        wide || tall
          ? svg("rect", { x: Math.min(x0, x1) - 8, y: Math.min(y0, y1) - 8, width: Math.abs(x1 - x0) + 16, height: Math.abs(y1 - y0) + 16, fill: "transparent" })
          : svg("circle", { cx: x0, cy: y0, r: 13, fill: "transparent" }),
        shape,
      ]);
      hit.addEventListener("click", function () { choose(hit, p); });
      hit.addEventListener("keydown", function (e) { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); choose(hit, p); } });
      kids.push(hit);
    });

    var label = "Tests placed by oxygen, " + lo + " to " + hi + " percent, and pressure, " + tidy(yLo) + " to " + tidy(yHi) + " kilopascals. " + (data.sentence || "");
    if (compact) {
      return el("div", { class: "atlas-chart flam-map" }, [svg("svg", { viewBox: "0 0 " + W + " " + H, class: "w-full h-auto", role: "img", "aria-label": label }, kids)]);
    }
    var counts = {}, bars = false;
    points.forEach(function (p) { counts[p.outcome] = (counts[p.outcome] || 0) + 1; if (p.o2[1] > p.o2[0] || p.kpa[1] > p.kpa[0]) bars = true; });
    var legend = el("ul", { class: "mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[12px] text-muted" }, OUTCOMES.filter(function (o) { return counts[o.key]; }).map(function (o) {
      return el("li", { class: "flex items-center gap-1.5" }, [
        svg("svg", { width: 12, height: 12, viewBox: "0 0 12 12", "aria-hidden": "true" }, [outcomeMark(o.key, 6, 6, 4.5)]),
        el("span", { text: o.label }),
      ]);
    }).concat([el("li", { class: "flex items-center gap-1.5" }, [
      svg("svg", { width: 14, height: 14, viewBox: "0 0 14 14", "aria-hidden": "true" }, [svg("circle", { cx: 7, cy: 7, r: 4, fill: "none", stroke: "#ece8dd", "stroke-width": 1.2 }), svg("path", { d: "M0 7H3M11 7H14M7 0V3M7 11V14", stroke: "#ece8dd", "stroke-width": 1.2 })]),
      el("span", { text: "Habitat air" }),
    ])]));

    var notes = [];
    if (bars) notes.push("A bar or box is a test that states a range, not one value.");
    if (data.no_pressure) notes.push(data.no_pressure + (data.no_pressure === 1 ? " more test states" : " more tests state") + " an oxygen level but no pressure.");
    if (data.no_oxygen) notes.push(data.no_oxygen + (data.no_oxygen === 1 ? " test states" : " tests state") + " no oxygen level.");

    var figure = el("figure", { class: "o2-chart flam-map" }, [
      el("figcaption", { class: "text-[13px] font-medium", text: "NASA’s tests by oxygen and pressure" }),
      svg("svg", { viewBox: "0 0 " + W + " " + H, class: "mt-2 w-full h-auto", role: "group", "aria-label": label }, kids),
      legend,
      el("p", { class: "mt-2 text-[13px] leading-5", text: data.sentence || "" }),
      el("p", { class: "mt-1 text-[11px] leading-4 text-dim", text: PINS.map(function (h) { return h.name + ": " + h.text; }).join(" · ") + "." + (notes.length ? " " + notes.join(" ") : "") }),
      detail,
    ]);
    if (markers.length) {
      figure.appendChild(el("div", { class: "map-limits" }, [
        el("p", { class: "text-[12px] font-medium text-amber", text: "Oxygen limits from other sources (dashed lines)" }),
        el("p", { class: "mt-0.5 text-[11px] leading-4 text-dim", text: "A different kind of test from the marks above, at the gravity shown. Not NASA microgravity test points." }),
        el("ol", { class: "mt-1.5 space-y-0.5 text-[12px] leading-4 text-muted" }, markers.map(function (m, i) {
          return el("li", { text: (i + 1) + " · " + tidy(m.limit) + "% · " + m.label + (m.on ? "" : " (beyond this chart’s oxygen range)") });
        })),
      ]));
    }
    return figure;
  }

  // Tests whose stated conditions can't be read as numbers: listed, never placed.
  function unplottableList(material) {
    var rows = (material.map || {}).unplottable || [];
    if (!rows.length) return null;
    return el("details", { class: "map-unplottable" }, [
      el("summary", { class: "text-[13px]", text: "Conditions not plottable · " + rows.length + (rows.length === 1 ? " test" : " tests") }),
      el("ul", { class: "mt-2 space-y-2 text-[12px] leading-4 text-muted" }, rows.map(function (r) {
        return el("li", {}, [
          el("span", { text: outcomeOf(r.outcome).label + " · " + r.why + ". " }),
          r.href ? link(r.paper || "The paper", r.href) : null,
        ]);
      })),
    ]);
  }

  function oxygenChart(material, opts) {
    var data = material.chart;
    if (!data || !data.points || !data.points.length) return null;
    var points = data.points.slice().sort(function (a, b) { return a.o2 - b.o2; });
    var lo = Math.min(15, Math.floor(points[0].o2 / 5) * 5);
    var hi = Math.max(35, Math.ceil(points[points.length - 1].o2 / 5) * 5);
    var W = 328, left = 12, right = 12, R = 6, step = 14;
    var plot = W - left - right;
    function x(v) { return left + ((v - lo) / (hi - lo)) * plot; }

    // Stack marks that would overlap: same column, one above the other.
    var columns = {}, tallest = 1;
    points.forEach(function (p) {
      var col = Math.round(x(p.o2) / (R * 2 + 1));
      columns[col] = (columns[col] || 0) + 1;
      p.col = col;
      p.row = columns[col] - 1;
      tallest = Math.max(tallest, columns[col]);
    });
    var top = 26, rows = Math.min(tallest, 9), base = top + rows * step + 6, H = base + 22;
    var kids = [];

    // The two habitats: a band one point either side, which is what counts as "at that level".
    [{ v: 30, name: "Moon Base 30%" }, { v: 21, name: "ISS 21%" }].forEach(function (h) {
      kids.push(svg("rect", { x: x(h.v - 1), y: top - 8, width: x(h.v + 1) - x(h.v - 1), height: base - top + 8, rx: 3, fill: "#7f93ff", opacity: 0.16 }));
      kids.push(svg("line", { x1: x(h.v), x2: x(h.v), y1: top - 8, y2: base, stroke: "#7f93ff", "stroke-width": 1, "stroke-dasharray": "3 3", opacity: 0.8 }));
      kids.push(svgText(h.name, { x: x(h.v), y: 11, "text-anchor": "middle", "font-size": 10, "font-weight": 600, fill: "#ece8dd" }));
    });
    kids.push(svg("line", { x1: left, x2: W - right, y1: base, y2: base, stroke: "#3b3930", "stroke-width": 1 }));
    for (var tick = lo; tick <= hi; tick += (hi - lo > 50 ? 20 : hi - lo > 25 ? 10 : 5)) {
      kids.push(svg("line", { x1: x(tick), x2: x(tick), y1: base, y2: base + 4, stroke: "#3b3930", "stroke-width": 1 }));
      kids.push(svgText(tick + "%", { x: x(tick), y: base + 16, "text-anchor": "middle", "font-size": 10, fill: "#a8a496" }));
    }
    points.forEach(function (p) {
      if (p.row >= 9) return; // a very tall column is cut; the count below still includes it
      var mark = outcomeMark(p.outcome, p.col * (R * 2 + 1), base - 8 - p.row * step, R);
      var name = (OUTCOMES.filter(function (o) { return o.key === p.outcome; })[0] || OUTCOMES[3]).label;
      var tip = svg("title", {});
      tip.textContent = name + " at " + p.o2 + "% oxygen";
      mark.appendChild(tip);
      kids.push(mark);
    });

    var counts = {};
    points.forEach(function (p) { counts[p.outcome] = (counts[p.outcome] || 0) + 1; });
    var legend = el("ul", { class: "mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[12px] text-muted" }, OUTCOMES.filter(function (o) { return counts[o.key]; }).map(function (o) {
      return el("li", { class: "flex items-center gap-1.5" }, [
        svg("svg", { width: 12, height: 12, viewBox: "0 0 12 12", "aria-hidden": "true" }, [outcomeMark(o.key, 6, 6, 4.5)]),
        el("span", { text: o.label + " · " + counts[o.key] }),
      ]);
    }));
    var summary = OUTCOMES.filter(function (o) { return counts[o.key]; }).map(function (o) { return counts[o.key] + " " + o.label.toLowerCase(); }).join(", ");
    if (opts && opts.compact) {
      return el("div", { class: "atlas-chart o2-strip", "data-summary": summary }, [
        svg("svg", { viewBox: "0 0 " + W + " " + H, class: "w-full h-auto", role: "img", "aria-label": "Tests placed by oxygen level, from " + lo + "% to " + hi + "%: " + summary + "." }, kids),
      ]);
    }
    return el("figure", { class: "o2-chart" }, [
      el("figcaption", { class: "text-[13px] font-medium", text: "NASA’s tests by oxygen level" }),
      svg("svg", { viewBox: "0 0 " + W + " " + H, class: "mt-2 w-full h-auto", role: "img", "aria-label": "Tests placed by oxygen level, from " + lo + "% to " + hi + "%: " + summary + "." }, kids),
      legend,
      el("p", { class: "mt-1 text-[11px] leading-4 text-dim", text: "Each mark is one test at one oxygen level. Shaded bands are within one point of each habitat’s air."
        + (data.no_level ? " " + data.no_level + (data.no_level === 1 ? " more test has" : " more tests have") + " no oxygen level recorded." : "") }),
    ]);
  }

  // What the item is made of, part by part. Colour follows the material, in a fixed order.
  var MATERIAL_COLORS = ["#ece8dd", "#7f93ff", "#ff5a26", "#58b8c6", "#e0a03c", "#858174"];
  function materialsFigure(rows) {
    rows = (rows || []).filter(function (r) { return (r.material || "").trim(); });
    if (!rows.length) return null;
    var order = [];
    rows.forEach(function (r) {
      var key = r.material.trim().toLowerCase();
      if (order.indexOf(key) < 0) order.push(key);
    });
    function color(name) {
      var i = order.indexOf(name.trim().toLowerCase());
      return i < MATERIAL_COLORS.length ? MATERIAL_COLORS[i] : "#858174";
    }
    var parts = [], byPart = {};
    rows.forEach(function (r) {
      var part = (r.part || "").trim();
      if (!byPart[part]) { byPart[part] = []; parts.push(part); }
      byPart[part].push(r);
    });
    var box = el("figure", { class: "mat-figure" }, [el("figcaption", { class: "text-[13px] font-medium", text: "What it’s made of" })]);
    parts.forEach(function (part) {
      var list = byPart[part];
      var known = list.every(function (r) { return typeof r.share === "number" && r.share > 0; });
      var total = known ? list.reduce(function (sum, r) { return sum + r.share; }, 0) : 0;
      var line = el("div", { class: "mt-3" }, [part ? el("p", { class: "text-[12px] text-muted", text: part }) : null]);
      // A bar only when the shares are known; otherwise the materials are listed without sizes.
      if (known && total > 0) {
        line.appendChild(el("div", { class: "mat-bar", role: "img", "aria-label": list.map(function (r) { return r.material + " " + Math.round(r.share) + "%"; }).join(", ") },
          list.map(function (r) {
            var seg = el("span", { class: "mat-seg", title: r.material + " · " + Math.round(r.share) + "%" });
            seg.style.width = (r.share / Math.max(total, 100)) * 100 + "%";
            seg.style.background = color(r.material);
            return seg;
          })));
      }
      line.appendChild(el("ul", { class: "mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-[13px]" }, list.map(function (r) {
        var dot = el("span", { class: "mat-dot" });
        dot.style.background = color(r.material);
        return el("li", { class: "flex items-center gap-1.5" }, [dot, el("span", { text: r.material + (known ? " · " + Math.round(r.share) + "%" : "") })]);
      })));
      box.appendChild(line);
    });
    if (box.querySelector(".mat-bar")) {
      box.appendChild(el("p", { class: "mt-3 text-[11px] leading-4 text-dim", text: "Shares are rough estimates of each part, as you confirmed them. They are not measurements." }));
    }
    return box;
  }

  // Small pictures for the verdict tiles.
  var VERDICT_ICON = {
    burned: "M12 3c1 3-1 4.5-2.5 6.5C8 11.500 7.500 13 7.500 14.500a4.500 4.500 0 0 0 9 0c0-1.500-.6-2.800-1.500-4-.4 1-1 1.600-1.800 2 .5-3-.2-6.500-1.200-9.500z",
    went_out: "M5 12.500l4.500 4.500L19 7.500",
    no_ignition: "M12 3l7 3v5.500c0 4.300-2.900 7.700-7 9.500-4.100-1.800-7-5.200-7-9.500V6z",
    unrecorded: "M12 8v5M12 16.500v.01",
    not_at_level: "M5 12h14",
    none: "M9.500 9.500a2.500 2.500 0 1 1 3.700 2.200c-.8.500-1.200 1-1.200 1.800M12 17v.01",
  };
  function verdictIcon(code) {
    return svg("svg", { width: 22, height: 22, viewBox: "0 0 24 24", fill: code === "burned" ? "currentColor" : "none", stroke: "currentColor", "stroke-width": code === "burned" ? 0 : 2, "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true", class: "verdict-icon" },
      [svg("path", { d: VERDICT_ICON[code] || VERDICT_ICON.none })]);
  }


  // ---------- tests from other sources (optional layer) ----------

  // The oxygen scale for a limiting oxygen index: below the mark the material goes out,
  // above it it keeps burning. The two habitats are marked for comparison.
  function oxygenLimitBar(limit) {
    var W = 320, left = 10, right = 10, y = 26, h = 12;
    function x(v) { return left + (Math.max(0, Math.min(100, v)) / 100) * (W - left - right); }
    var kids = [
      svg("rect", { x: x(0), y: y, width: x(limit) - x(0), height: h, rx: 4, fill: "#4cc27a", opacity: 0.85 }),
      svg("rect", { x: x(limit) + 2, y: y, width: Math.max(0, x(100) - x(limit) - 2), height: h, rx: 4, fill: "#ff5a26", opacity: 0.9 }),
    ];
    [{ v: 21, name: "ISS" }, { v: 30, name: "Moon Base" }].forEach(function (hab, i) {
      kids.push(svg("line", { x1: x(hab.v), x2: x(hab.v), y1: y - 6, y2: y + h + 6, stroke: "#ece8dd", "stroke-width": 1.5, "stroke-dasharray": "2 2" }));
      kids.push(svgText(hab.name + " " + hab.v + "%", { x: x(hab.v), y: i === 0 ? 12 : y + h + 18, "text-anchor": i === 0 ? "end" : "start", "font-size": 10, "font-weight": 600, fill: "#ece8dd" }));
    });
    kids.push(svgText("0%", { x: x(0), y: y + h + 18, "font-size": 9, fill: "#a8a496" }));
    kids.push(svgText("100% oxygen", { x: x(100), y: 12, "text-anchor": "end", "font-size": 9, fill: "#a8a496" }));
    return svg("svg", { viewBox: "0 0 " + W + " 60", class: "mt-2 w-full h-auto", role: "img",
      "aria-label": "Oxygen scale from 0 to 100 percent. Goes out below " + limit + " percent, keeps burning above. ISS air is 21 percent, Moon Base 30 percent." }, kids);
  }

  function otherRow(row) {
    var limit = typeof row.limit === "number" ? row.limit : null;
    var long = /[a-z]{4}/i.test(String(row.value)) && /%/.test(String(row.value));
    var value = String(row.value) + (row.unit && row.unit !== "rating" && !long ? (/^%/.test(row.unit) ? row.unit : " " + row.unit) : "");
    var kids = [
      el("div", { class: "flex flex-wrap items-baseline gap-x-2 gap-y-1" }, [
        el("span", { class: "source-chip", text: row.publisher }),
        el("span", { class: "text-[12px] text-muted", text: row.test + (row.standard && row.standard.toLowerCase().indexOf(row.test.toLowerCase()) !== 0 ? " · " + row.standard : "") }),
      ]),
      el("p", { class: long ? "mt-1.5 text-[16px] leading-6 font-semibold" : "mt-1.5 text-[20px] leading-6 font-semibold", text: value }),
    ];
    if (limit !== null && limit > 0 && limit <= 100) {
      kids.push(el("p", { class: "mt-1 text-[13px] leading-5 text-muted", text: "In this test it stopped burning on its own at about " + limit + "% oxygen and below." }));
      kids.push(oxygenLimitBar(limit));
      kids.push(el("ul", { class: "mt-1 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted" }, [
        legendRow("#4cc27a", "Went out"), legendRow("#ff5a26", "Kept burning"),
      ]));
    }
    var facts = [row.conditions, row.gravity === "microgravity" ? "Run in weightlessness" : row.gravity === "Earth" ? "Run on Earth" : row.gravity ? "Run at " + row.gravity : null, row.note].filter(Boolean);
    if (facts.length) kids.push(el("p", { class: "mt-2 text-[12px] leading-4 text-dim", text: facts.join(" · ") }));
    kids.push(el("p", { class: "mt-2 text-[12px] leading-4" }, [
      link(row.source_title, row.url),
      row.page ? el("span", { class: "text-dim", text: " · " + row.page }) : null,
    ]));
    return el("li", { class: "other-row" }, kids);
  }

  function otherTests(material) {
    var rows = material.other || [];
    if (!rows.length) return null;
    var publishers = [];
    rows.forEach(function (r) { if (publishers.indexOf(r.publisher) < 0) publishers.push(r.publisher); });
    return el("div", { class: "other-block" }, [
      el("p", { class: "text-[15px] font-semibold", text: "Tests from other sources" }),
      el("p", { class: "mt-1 text-[12px] leading-4 text-dim", text: rows.length + (rows.length === 1 ? " published result from " : " published results from ") + publishers.join(", ")
        + ". These are not in the database above. Most were run on Earth, where flames behave differently than in weightlessness; the ones run in weightlessness say so. Samples differ in grade, thickness and shape, so results for one material can disagree." }),
      el("ul", { class: "mt-3 space-y-3" }, rows.map(otherRow)),
    ]);
  }

  // ---------- result card (used by both pages) ----------

  function verdictTile(hab) {
    var look = VERDICT[hab.verdict] || VERDICT.none;
    return el("div", { class: "verdict-tile " + look.edge + " " + look.text, "data-verdict": VERDICT[hab.verdict] ? hab.verdict : "none" }, [
      verdictIcon(hab.verdict),
      el("p", { class: "text-[13px] font-medium text-fg", text: hab.name }),
      el("p", { class: "text-[12px] text-muted", text: hab.air }),
      el("p", { class: "mt-2 text-[19px] leading-6 font-semibold " + look.text, text: hab.label }),
      hab.detail ? el("p", { class: "mt-1 text-[12px] leading-4 text-muted", text: hab.detail }) : null,
    ]);
  }

  function evidenceList(lines) {
    if (!lines || !lines.length) return null;
    return el("ul", { class: "mt-3 space-y-2 text-[13px] leading-5" }, lines.map(function (line) {
      return el("li", {}, [link(line.text, line.href), line.paper ? el("span", { class: "block text-[12px] text-dim", text: line.paper }) : null]);
    }));
  }

  function gasBlock(material, sources) {
    var box = el("div", { class: "mt-5" }, [el("h4", { class: "text-[14px] font-semibold", text: "What it gives off when it burns" })]);
    var gases = material.gases;
    if (!gases) {
      box.appendChild(el("p", { class: "mt-1 text-[13px] leading-5 text-muted" }, [
        "NASA’s gas table doesn’t list this material. ",
        link("Table I, p. 591", sources.analyzer),
      ]));
      return box;
    }
    box.appendChild(el("p", { class: "mt-1 text-[13px] leading-5 text-muted" }, [
      "Listed for " + gases.table_name + " in NASA’s ",
      link(sources.table_page, sources.analyzer),
      ".",
    ]));
    box.appendChild(el("ul", { class: "mt-2 space-y-2" }, gases.list.map(function (g) {
      var limits = g.limit_1h
        ? "Crew limit: " + g.limit_1h + " ppm for 1 hour · " + g.limit_24h + " ppm for 24 hours. "
        : "NASA’s limits document has no entry for this gas. ";
      return el("li", { class: "text-[13px] leading-5" }, [
        el("span", { class: "font-semibold", text: g.formula + " " }),
        el("span", { class: "text-muted", text: g.name + ". " + limits }),
        link(g.limit_1h ? "JSC 20584 Rev C, " + g.page : "JSC 20584 Rev C", sources.limits),
      ]);
    })));
    box.appendChild(el("p", { class: "mt-2 text-[12px] leading-4 text-dim", text: "These limits were set for Earth-like air (14.7 psi, 21% O₂) and may not apply at 30% O₂." }));
    return box;
  }

  function materialSection(material, result) {
    var section = el("section", { class: "result-card" }, [el("h3", { class: "text-[22px] leading-7 font-semibold break-words", text: material.query })]);
    if (material.tested) {
      if (material.fuels && material.fuels.length) {
        section.appendChild(el("p", { class: "mt-1 text-[12px] text-dim", text: "In NASA’s tests as: " + material.fuels.join(", ") }));
      }
      section.appendChild(el("div", { class: "mt-3 grid grid-cols-2 gap-3" }, material.habitats.map(verdictTile)));
      section.appendChild(el("p", { class: "mt-2 text-[12px] leading-4 text-dim", text: "Matched on oxygen level only. Each test’s pressure is on its line below." }));
      var ring = outcomeDonut(material);
      if (ring) section.appendChild(ring);
      var map = flammabilityMap(material);
      if (map) section.appendChild(map);
      var chart = oxygenChart(material);
      if (chart) section.appendChild(chart);
      if (!map) {
        var unread = ((material.map || {}).unplottable || []).length;
        section.appendChild(el("p", { class: "mt-2 text-[12px] leading-4 text-muted", text: !chart
          ? "No test of this material states an oxygen level that can be read as a number, so none can be placed on a chart."
          : unread
            ? "No test states both an oxygen level and a pressure that can be read as numbers, so they are placed by oxygen level only."
            : "Pressure not recorded for these tests, so they are placed by oxygen level only." }));
      }
      var unplottable = unplottableList(material);
      if (unplottable) section.appendChild(unplottable);
      section.appendChild(evidenceList(material.evidence));
    } else {
      var why = result.data !== "ok"
        ? "No NASA test data to check this against yet."
        : result.lookup === "synonyms"
          ? "No test in NASA’s data under this name or its listed other names."
        : result.lookup !== "ok"
          ? "No test in NASA’s data under exactly this name."
          : material.query + " isn’t in NASA’s tests.";
      var kind = family(material.query);
      if (kind && !(material.other || []).length) {
        why = kind.word + " is a family of materials that burn differently, so no test is filed under that name.";
      }
      section.appendChild(el("p", { class: "mt-2 text-[17px] leading-6 font-semibold text-muted", text: why }));
      if (kind && !(material.other || []).length) {
        section.appendChild(el("p", { class: "mt-2 text-[13px] leading-5 text-muted", text: "Tested materials in this family: " + kind.options.join(", ") + ". If you know which one this is, change the material to see its tests." }));
      }
      var near = material.closest;
      if (near) {
        var box = el("div", { class: "mt-4 border border-dashed border-fg p-4" }, [
          el("p", { class: "text-[14px] leading-5" }, [
            el("span", { class: "text-muted", text: "Closest tested: " }),
            el("span", { class: "font-semibold", text: near.fuel }),
            el("span", { class: "text-muted", text: near.note ? ", " + near.note : ", a different material" }),
          ]),
          el("p", { class: "mt-1 text-[12px] leading-4 text-amber", text: "These are " + near.fuel + " results, not " + material.query + " results." }),
          el("div", { class: "mt-3 grid grid-cols-2 gap-3" }, near.habitats.map(verdictTile)),
        ]);
        var lines = evidenceList(near.evidence);
        if (lines) box.appendChild(lines);
        section.appendChild(box);
      }
    }
    var others = otherTests(material);
    if (others) section.appendChild(others);
    section.appendChild(gasBlock(material, result.sources));
    return section;
  }

  // Two flames in freefall, drawn live by flame3d.js: the same sphere at each habitat's oxygen.
  // An illustration of "more oxygen, bigger and brighter", not a picture of any test.
  function flameFigure() {
    function stage(oxygen, caption) {
      var canvas = el("canvas", {});
      var view = el("div", { class: "chamber", style: "aspect-ratio: 4 / 3" }, [canvas, el("div", { class: "chamber-frame", "aria-hidden": "true" })]);
      // Mounted once the canvas is in the page and has a size.
      setTimeout(function () {
        var flame = window.Flame3D && window.Flame3D.mount(canvas, { gravity: 0, oxygen: oxygen });
        if (!flame) view.appendChild(el("p", { class: "chamber-fallback", text: "A round blue flame." }));
      }, 0);
      return el("div", {}, [view, el("p", { class: "mt-2 eyebrow text-center", text: caption })]);
    }
    return el("figure", { class: "mt-5", role: "img", "aria-label": "Two round blue flames on a wick, drawn in three dimensions. The one at 30% oxygen is larger and brighter than the one at 21%." }, [
      el("div", { class: "grid grid-cols-2 gap-3" }, [stage(1, "Moon Base · 30% O₂"), stage(0, "ISS · 21% O₂")]),
      el("figcaption", { class: "mt-2 text-[12px] leading-4 text-dim", text: "Drawn in your browser. Not NASA footage and not a measurement. Flames in freefall are round and blue; more oxygen makes them bigger and brighter." }),
    ]);
  }

  function resultBody(result) {
    var box = el("div", {});
    [result.data_message, result.lookup_message].forEach(function (message) {
      if (message) box.appendChild(el("p", { class: "notice mt-4", role: "status", text: message }));
    });
    var coverage = coverageDonut(result);
    if (coverage) box.appendChild(coverage);
    box.appendChild(flameFigure());
    if (!result.materials.length) {
      box.appendChild(el("p", { class: "mt-6 text-[16px] text-muted", text: "No materials to check." }));
    }
    result.materials.forEach(function (m) { box.appendChild(materialSection(m, result)); });
    box.appendChild(el("p", { class: "mt-6 text-[13px] leading-5 text-muted", text: "This is NASA spacecraft research, not a home safety rating." }));
    return box;
  }

  // What the vision model said it saw. It describes the photo only; it knows nothing about the tests.
  function summaryBlock(text) {
    if (!text) return document.createTextNode(""); // nothing to show, but safe to append
    return el("div", { class: "mt-4 border-l-[3px] border-fg pl-4 py-1" }, [
      el("p", { class: "eyebrow", text: "AI summary of the photo" }),
      el("p", { class: "mt-1 text-[15px] leading-6", text: text }),
    ]);
  }

  function fetchResult(names) {
    return fetch("/scan/result", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ materials: names }),
    }).then(function (r) {
      if (!r.ok) throw new Error("status " + r.status);
      return r.json();
    });
  }

  // ---------- the scan page ----------

  function scanPage(root) {
    var steps = { scan: root.querySelector("#step-scan"), camera: root.querySelector("#step-camera"), busy: root.querySelector("#step-busy"), edit: root.querySelector("#step-edit"), result: root.querySelector("#step-result") };
    var errorLine = root.querySelector("#scan-error");
    var state = { item: "", rows: [], thumb: null, notice: null, model: null, summary: null };

    function show(name) {
      Object.keys(steps).forEach(function (k) { steps[k].hidden = k !== name; });
      window.scrollTo(0, 0);
    }

    function fail(message) {
      errorLine.textContent = message;
      errorLine.hidden = false;
      show("scan");
    }

    // ----- live camera (laptops and desktops) -----
    // Phones open their own camera from the file input's `capture` attribute. A computer
    // ignores `capture` and shows a file picker, so there the webcam is opened in the page.
    var cam = { stream: null, mode: "item", fallback: null };
    var video = root.querySelector("#cam-video");
    var useLiveCamera = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia)
      && !(window.matchMedia && window.matchMedia("(pointer: coarse)").matches);

    function stopCamera() {
      if (cam.stream) cam.stream.getTracks().forEach(function (t) { t.stop(); });
      cam.stream = null;
      video.srcObject = null;
    }

    function openCamera(mode, fallbackInput) {
      cam.mode = mode;
      cam.fallback = fallbackInput;
      errorLine.hidden = true;
      navigator.mediaDevices.getUserMedia({ video: { facingMode: { ideal: "environment" }, width: { ideal: 1920 }, height: { ideal: 1080 } }, audio: false }).then(function (stream) {
        cam.stream = stream;
        video.srcObject = stream;
        root.querySelector("#cam-title").textContent = mode === "label" ? "Point the camera at the label" : "Point the camera at the item";
        show("camera");
        return video.play();
      }).catch(function () {
        // No camera, or permission refused: fall back to choosing a photo.
        stopCamera();
        useLiveCamera = false;
        errorLine.textContent = "Couldn’t open the camera. Allow camera access, or choose a photo instead.";
        errorLine.hidden = false;
        show("scan");
      });
    }

    root.querySelector("#cam-shoot").addEventListener("click", function () {
      if (!video.videoWidth) return;
      var canvas = document.createElement("canvas");
      canvas.width = video.videoWidth;
      canvas.height = video.videoHeight;
      canvas.getContext("2d").drawImage(video, 0, 0);
      var mode = cam.mode;
      stopCamera();
      canvas.toBlob(function (blob) {
        if (blob) handle(blob, mode);
        else fail("Couldn’t take the photo. Try again.");
      }, "image/jpeg", 0.92);
    });
    root.querySelector("#cam-cancel").addEventListener("click", function () {
      stopCamera();
      show("scan");
    });
    window.addEventListener("pagehide", stopCamera);

    function onPick(input, mode, opensCamera) {
      if (opensCamera) {
        input.addEventListener("click", function (event) {
          if (!useLiveCamera) return; // phone, or no webcam: the normal picker opens
          event.preventDefault();
          openCamera(mode, input);
        });
      }
      input.addEventListener("change", function () {
        var file = input.files && input.files[0];
        input.value = "";
        if (file) handle(file, mode);
      });
    }

    // One photo (file or camera frame), one path: busy screen → /scan/identify → edit step.
    function handle(file, mode) {
        errorLine.hidden = true;
        root.querySelector("#busy-text").textContent = mode === "label" ? "Reading the label…" : "Looking…";
        root.querySelector("#busy-thumb").removeAttribute("src");
        show("busy");
        prepare(file).then(function (photo) {
          state.thumb = photo.thumb;
          if (state.preview) URL.revokeObjectURL(state.preview);
          state.preview = URL.createObjectURL(photo.blob); // this tab's memory only, never stored
          root.querySelector("#busy-thumb").src = photo.thumb;
          return fetch("/scan/identify?mode=" + mode, { method: "POST", headers: { "Content-Type": "image/jpeg" }, body: photo.blob });
        }).then(function (r) {
          if (!r.ok) throw new Error("status " + r.status);
          return r.json();
        }).then(function (reply) {
          if (!reply.ok) {
            if (reply.reason === "bad_image") return fail(reply.message);
            // Scanner unavailable or failed: carry on by hand rather than stopping.
            state.item = "";
            state.rows = [];
            state.model = null;
            state.summary = null;
            state.notice = reply.reason === "error" ? reply.message + " You can type the materials in instead." : reply.message;
            return openEdit();
          }
          state.item = reply.item || "";
          state.model = reply.model || true; // the public site does not name the model
          state.summary = reply.unclear ? null : (reply.summary || null);
          state.rows = [];
          (reply.parts || []).forEach(function (part) {
            part.materials.forEach(function (m) {
              state.rows.push({ part: part.name || "", material: m.material, share: m.share, confidence: m.confidence });
            });
          });
          var unsure = reply.unclear || (typeof reply.confidence === "number" && reply.confidence < UNSURE);
          // "Couldn't read it" is only said when the model really could not; a low-confidence
          // reading is still a reading, shown with a request to check it.
          state.notice = reply.unclear
            ? (reply.message || "This photo is hard to read.") + " Take another photo, or type the materials below."
            : unsure
              ? "This is a best guess. Check the item and materials below and correct anything that’s off."
              : null;
          openEdit();
        }).catch(function () {
          fail("That photo couldn’t be read or sent. Try again.");
        });
    }

    function openEdit() {
      var box = steps.edit;
      clear(box);
      var nameInput = el("input", { class: "field mt-2 h-12 text-[18px]", value: state.item, placeholder: "Name this item", "aria-label": "Item name", maxlength: "60" });
      nameInput.addEventListener("input", function () { state.item = nameInput.value; });

      box.appendChild(el("div", { class: "flex items-center gap-4" }, [
        state.thumb ? el("img", { src: state.thumb, alt: "", class: "w-16 h-16 rounded-xl object-cover bg-ink-800 shrink-0" }) : null,
        el("div", { class: "min-w-0 flex-1" }, [el("p", { class: "text-[15px] text-muted", text: state.rows.length ? "Looks like…" : "What is it?" }), nameInput]),
      ]));
      if (state.notice) box.appendChild(el("p", { class: "notice mt-4", role: "status", text: state.notice }));
      box.appendChild(summaryBlock(state.summary));

      box.appendChild(el("h3", { class: "mt-6 text-[15px] font-semibold", text: "Made of" }));
      box.appendChild(el("p", { class: "text-[12px] text-dim", text: state.rows.length ? "A best guess from the photo. Tap to change, remove or add." : "Add each material, for example cotton or PVC." }));
      var list = el("ul", { class: "mt-3 space-y-2" });
      box.appendChild(list);
      if (state.model && state.rows.length) box.appendChild(el("p", { class: "mt-2 text-[12px] text-dim", text: (state.model === true ? "Identified by AI" : "Identified by " + state.model) + ". AI can be wrong: check each material." }));

      function drawRows() {
        clear(list);
        state.rows.forEach(function (row, i) {
          var material = el("input", { class: "field h-11 text-[16px]", value: row.material || "", placeholder: "Material", "aria-label": "Material " + (i + 1), maxlength: "60" });
          material.addEventListener("input", function () {
            row.material = material.value;
            row.confidence = null; // the user has taken over this one
            if (flag.parentNode) flag.parentNode.removeChild(flag);
          });
          var shareBox = el("input", { class: "field h-11 w-[72px] text-[16px] text-right", value: row.share ? String(Math.round(row.share)) : "", inputmode: "numeric", placeholder: "%", "aria-label": "Rough share in percent, material " + (i + 1), maxlength: "3" });
          shareBox.addEventListener("input", function () {
            var n = parseInt(shareBox.value, 10);
            row.share = n > 0 && n <= 100 ? n : null;
          });
          var flag = el("span", { class: "text-[12px] text-amber", text: "Not sure about this one" });
          var pick = el("div", { class: "mt-2" });
          function drawPick() {
            clear(pick);
            var kind = family(material.value);
            if (!kind) return;
            pick.appendChild(el("p", { class: "text-[12px] leading-4 text-muted", text: kind.word + " covers materials that burn differently. Pick the exact one if you know it:" }));
            pick.appendChild(el("div", { class: "mt-1.5 flex flex-wrap gap-2" }, kind.options.map(function (option) {
              return el("button", { type: "button", class: "switch-btn", text: option, onclick: function () {
                material.value = option;
                row.material = option;
                row.confidence = null;
                if (flag.parentNode) flag.parentNode.removeChild(flag);
                drawPick();
              } });
            })));
          }
          material.addEventListener("input", drawPick);
          drawPick();
          var unsure = typeof row.confidence === "number" && row.confidence < UNSURE;
          list.appendChild(el("li", {}, [
            row.part ? el("p", { class: "text-[12px] text-muted mb-1", text: row.part }) : null,
            el("div", { class: "flex items-center gap-2" }, [
              material,
              shareBox,
              el("button", { type: "button", class: "w-10 h-10 shrink-0 border border-fg text-fg text-[20px] leading-none hover:bg-fg hover:text-ink-950", "aria-label": "Remove " + (row.material || "material"), text: "×", onclick: function () { state.rows.splice(i, 1); drawRows(); } }),
            ]),
            unsure ? flag : null,
            pick,
          ]));
        });
      }
      drawRows();

      box.appendChild(el("button", { type: "button", class: "mt-3 text-[15px] text-sky underline underline-offset-4", text: "+ Add a material", onclick: function () {
        state.rows.push({ part: "", material: "", share: null, confidence: null });
        drawRows();
        var inputs = list.querySelectorAll("input");
        if (inputs.length > 1) inputs[inputs.length - 2].focus();
      } }));

      var problem = el("p", { class: "notice mt-4", role: "alert", hidden: true });
      box.appendChild(problem);
      var go = el("button", { type: "button", class: "btn-big w-full mt-6", text: "See the fire tests" });
      go.addEventListener("click", function () {
        var names = [];
        state.rows.forEach(function (row) {
          var name = (row.material || "").trim();
          if (name && names.map(function (n) { return n.toLowerCase(); }).indexOf(name.toLowerCase()) < 0) names.push(name);
        });
        if (!names.length) {
          problem.textContent = "Add at least one material.";
          problem.hidden = false;
          return;
        }
        problem.hidden = true;
        go.disabled = true;
        go.textContent = "Checking NASA’s tests…";
        fetchResult(names).then(function (result) {
          openResult(result, names);
        }).catch(function () {
          problem.textContent = "Couldn’t get the test results. Check your connection and try again.";
          problem.hidden = false;
        }).then(function () {
          go.disabled = false;
          go.textContent = "See the fire tests";
        });
      });
      box.appendChild(go);
      box.appendChild(el("button", { type: "button", class: "btn-big-ghost w-full mt-3", text: "Start over", onclick: function () { show("scan"); } }));
      show("edit");
    }

    function openResult(result, names) {
      var box = steps.result;
      clear(box);
      var title = (state.item || "").trim() || names.join(", ");
      if (state.preview || state.thumb) {
        box.appendChild(el("div", { class: "scan-hero" }, [
          el("img", { src: state.preview || state.thumb, alt: "" }),
          el("span", { class: "scan-hero-sweep", "aria-hidden": "true" }),
          el("h2", { class: "scan-hero-title", text: title }),
        ]));
      } else {
        box.appendChild(el("h2", { class: "text-[26px] leading-8 font-semibold break-words min-w-0", text: title }));
      }
      box.appendChild(summaryBlock(state.summary));
      var made = materialsFigure(state.rows);
      if (made) box.appendChild(made);
      box.appendChild(resultBody(result));

      var space = el("select", { class: "field h-11 text-[16px]", "aria-label": "Space" }, SPACES.map(function (name) { return el("option", { value: name, text: name }); }));
      var saved = el("p", { class: "mt-3 text-[14px]", role: "status", hidden: true });
      var saveButton = el("button", { type: "button", class: "btn-big shrink-0", text: "Save" });
      saveButton.addEventListener("click", function () {
        var data = loadStore();
        data.items.unshift({
          id: String(Date.now()) + "-" + Math.floor(Math.random() * 1e6),
          name: title,
          space: space.value,
          when: new Date().toISOString(),
          thumb: state.thumb,
          shares: state.rows.filter(function (r) { return (r.material || "").trim(); }).map(function (r) { return { material: r.material.trim(), share: r.share || null }; }),
          summary: state.summary || null,
          result: result,
        });
        clear(saved);
        if (saveStore(data)) {
          saved.appendChild(document.createTextNode("Saved to " + space.value + ". "));
          saved.appendChild(link("See my items", "/items"));
          saveButton.disabled = true;
          saveButton.classList.add("opacity-50");
        } else {
          saved.textContent = "Couldn’t save: this browser is blocking storage or it is full.";
        }
        saved.hidden = false;
      });
      box.appendChild(el("div", { class: "mt-8 border-t border-line pt-6" }, [
        el("p", { class: "text-[14px] font-semibold", text: "Save to a space" }),
        el("div", { class: "mt-2 flex gap-3" }, [space, saveButton]),
        saved,
      ]));
      box.appendChild(el("button", { type: "button", class: "btn-big-ghost w-full mt-4", text: "Change materials", onclick: openEdit }));
      box.appendChild(el("button", { type: "button", class: "btn-big-ghost w-full mt-3", text: "Scan another item", onclick: function () { show("scan"); } }));
      show("result");
    }

    onPick(root.querySelector("#pick-camera"), "item", true);
    onPick(root.querySelector("#pick-upload"), "item", false);
    onPick(root.querySelector("#pick-label"), "label", true);
    root.querySelector("#pick-manual").addEventListener("click", function () {
      state = { item: "", rows: [{ part: "", material: "", share: null, confidence: null }], thumb: null, notice: null, model: null, summary: null };
      errorLine.hidden = true;
      openEdit();
    });
  }

  // ---------- the habitat report (items page): what the saved scans say, rolled up ----------
  // REPORT-PURE-START (these functions read saved scan results and nothing else)

  // The level the report compares against. The server sends the same numbers with
  // /items/findings (HABITATS and O2_TOLERANCE in scan.rs); these stand in until it replies.
  var MOON_LEVEL = { name: "Moon Base", o2_pct: 30, psi: 10.2, kpa: 70.3, tolerance: 1 };
  // A stated pressure below this many kPa counts as reduced pressure.
  var REDUCED_BELOW_KPA = 101;

  // The oxygen levels of a material's NASA test points, with how each test ended.
  function reportOxygen(material) {
    return ((material.chart || {}).points || []).filter(function (p) { return p && typeof p.o2 === "number" && isFinite(p.o2); });
  }

  // Distinct values, lowest first, as "21%, 30%".
  function reportLevels(values) {
    var seen = {}, out = [];
    values.slice().sort(function (a, b) { return a - b; }).forEach(function (v) {
      var text = tidy(v) + "%";
      if (!seen[text]) { seen[text] = true; out.push(text); }
    });
    return out.join(", ");
  }

  // Which group a material is in, decided only from its saved test points.
  //   burned: a test point recorded as burned at or above the level (less the tolerance)
  //   below:  it has test points with oxygen levels and every one is below that line
  //   never:  no NASA microgravity test of it
  //   other:  tested, but neither of the first two; `detail` says why
  function reportGroup(material, level) {
    var floor = level.o2_pct - level.tolerance;
    if (!material.tested) return { group: "never", detail: "" };
    if (!material.chart) return { group: "other", detail: "the saved result has no oxygen level per test; use Check again on the item" };
    var points = reportOxygen(material);
    var burned = points.filter(function (p) { return p.outcome === "burned" && p.o2 >= floor; });
    if (burned.length) return { group: "burned", detail: "burned at " + reportLevels(burned.map(function (p) { return p.o2; })) + " O₂" };
    if (!points.length) return { group: "other", detail: "its tests record no oxygen level" };
    var max = Math.max.apply(null, points.map(function (p) { return p.o2; }));
    if (max < floor) return { group: "below", detail: "highest tested: " + tidy(max) + "% O₂" };
    return { group: "other", detail: "tested up to " + tidy(max) + "% O₂; no test at " + tidy(floor) + "% or above is recorded as burned" };
  }

  // What the saved data lacks for this material at the level's oxygen and pressure.
  // Each line names a gap in the data. An empty list means none of these gaps was found.
  function reportGaps(material, level) {
    if (!material.tested) return ["no microgravity test"];
    var gaps = [];
    var lo = level.o2_pct - level.tolerance, hi = level.o2_pct + level.tolerance;
    var near = false;
    if (!material.chart) {
      gaps.push("the saved result has no oxygen level per test");
    } else {
      var values = reportOxygen(material).map(function (p) { return p.o2; });
      if (!values.length) {
        gaps.push("no test states an oxygen level");
      } else if (values.some(function (v) { return v >= lo && v <= hi; })) {
        near = true;
      } else {
        var under = values.filter(function (v) { return v < lo; }), over = values.filter(function (v) { return v > hi; });
        if (!over.length) gaps.push("no test above " + tidy(Math.max.apply(null, under)) + "% O₂");
        else if (!under.length) gaps.push("no test below " + tidy(Math.min.apply(null, over)) + "% O₂");
        else gaps.push("no test between " + tidy(Math.max.apply(null, under)) + "% and " + tidy(Math.min.apply(null, over)) + "% O₂");
      }
    }
    var map = material.map;
    if (!map) {
      gaps.push("the saved result has no pressure per test");
    } else {
      var placed = (map.points || []).filter(function (p) {
        return p && Array.isArray(p.kpa) && Array.isArray(p.o2) && typeof p.kpa[0] === "number" && typeof p.o2[0] === "number" && typeof p.o2[1] === "number";
      });
      if (!placed.length) {
        // no_pressure counts tests that state oxygen and no pressure at all.
        gaps.push(map.tests > 0 && map.no_pressure === map.tests ? "no test states a pressure" : "no test states both an oxygen level and a pressure that could be read");
      } else {
        var reduced = placed.filter(function (p) { return p.kpa[0] < REDUCED_BELOW_KPA; });
        if (!reduced.length) {
          gaps.push("no test at reduced pressure (lowest stated: " + tidy(Math.min.apply(null, placed.map(function (p) { return p.kpa[0]; }))) + " kPa)");
        } else if (near && !reduced.some(function (p) { return p.o2[0] - level.tolerance <= level.o2_pct && level.o2_pct <= p.o2[1] + level.tolerance; })) {
          gaps.push("no single test has both oxygen within " + tidy(level.tolerance) + " point of " + tidy(level.o2_pct) + "% and pressure below " + REDUCED_BELOW_KPA + " kPa");
        }
      }
    }
    return gaps;
  }

  // Every material in these items, once each (names compared without case), with the items
  // that contain it. When two items hold the same material, the most recently checked
  // item's saved data is the one read.
  function reportRollup(items, level) {
    var byName = {}, rows = [];
    items.forEach(function (item) {
      var itemName = item.name || "Unnamed item";
      ((item.result || {}).materials || []).forEach(function (m) {
        var name = String((m && m.query) || "").trim();
        if (!name) return;
        var key = name.toLowerCase();
        var row = byName[key];
        if (!row) {
          row = byName[key] = { name: name, items: [], material: m, when: String(item.when || "") };
          rows.push(row);
        } else if (String(item.when || "") > row.when) {
          row.material = m;
          row.when = String(item.when || "");
        }
        if (row.items.indexOf(itemName) < 0) row.items.push(itemName);
      });
    });
    return rows.map(function (row) {
      var g = reportGroup(row.material, level);
      return { name: row.name, items: row.items, group: g.group, detail: g.detail, gaps: reportGaps(row.material, level) };
    });
  }
  // REPORT-PURE-END

  // The level from the server's reply when it is complete, otherwise the stand-in above.
  function reportLevel(reply) {
    var l = reply && reply.level;
    var ok = l && typeof l.o2_pct === "number" && typeof l.psi === "number" && typeof l.kpa === "number" && typeof l.tolerance === "number";
    return ok ? { name: l.name || MOON_LEVEL.name, o2_pct: l.o2_pct, psi: l.psi, kpa: l.kpa, tolerance: l.tolerance } : MOON_LEVEL;
  }

  function reportHead(n, text) {
    return el("div", { class: "mt-8 border-t-2 border-fg pt-2 flex items-baseline gap-3" }, [
      el("span", { class: "font-mono text-[11px] text-burn", text: n }),
      el("h3", { class: "text-[17px] leading-6 font-semibold", text: text }),
    ]);
  }

  function reportMaterialLine(row, extra) {
    return el("li", { class: "border-b border-line py-2 text-[13px] leading-5 break-words" }, [
      el("span", { class: "font-medium", text: row.name }),
      extra ? el("span", { class: "text-muted", text: " · " + extra }) : null,
      el("span", { class: "block text-[12px] leading-4 text-dim", text: "In: " + row.items.join(", ") }),
    ]);
  }

  // One finding exactly as the database has it, with its experiments and its paper.
  function reportFinding(f) {
    var meta = el("p", { class: "mt-1 font-mono text-[11px] leading-5 text-muted break-words" });
    (f.experiments || []).forEach(function (e) {
      meta.appendChild(link(e.acronym, e.href));
      meta.appendChild(document.createTextNode(" · "));
    });
    meta.appendChild(link(f.source || "Paper", f.paper_href));
    if (typeof f.confidence === "number") meta.appendChild(document.createTextNode(" · AI confidence " + Math.round(f.confidence * 100) + "%"));
    meta.appendChild(document.createTextNode(f.verified ? " · Verified" : " · Unverified"));
    return el("li", { class: "border-b border-line py-2" }, [el("p", { class: "text-[13px] leading-5 break-words", text: f.text }), meta]);
  }

  function reportFindings(state, reply) {
    var box = el("div", {});
    box.appendChild(reportHead("03", "NASA findings for habitat design"));
    box.appendChild(el("p", { class: "mt-2 text-[12px] leading-5 text-muted", text: "These are NASA’s findings from the experiments in the database, in the papers’ extracted wording. They are not specific to the items scanned here." }));
    if (state === "loading") {
      box.appendChild(el("p", { class: "mt-3 text-[13px] text-muted", role: "status", text: "Loading the findings…" }));
      return box;
    }
    if (state === "failed" || !reply || reply.data !== "ok") {
      box.appendChild(el("p", { class: "notice mt-3", role: "status", text: (reply && reply.data_message) || "The findings could not be loaded. The rest of the report is from your saved scans." }));
      return box;
    }
    var grid = el("div", { class: "mt-2 grid gap-x-6 sm:grid-cols-3" });
    (reply.groups || []).forEach(function (g) {
      var col = el("div", { class: "mt-4" }, [
        el("p", { class: "eyebrow", text: g.label + " · " + g.total + (g.total === 1 ? " finding" : " findings") }),
      ]);
      if (!g.total) col.appendChild(el("p", { class: "mt-2 text-[13px] text-dim", text: "None in the database." }));
      col.appendChild(el("ul", { class: "mt-1" }, (g.findings || []).map(reportFinding)));
      if (g.more > 0) col.appendChild(el("p", { class: "mt-2 text-[12px] leading-4 text-dim", text: "+ " + g.more + " more in the database" }));
      grid.appendChild(col);
    });
    box.appendChild(grid);
    box.appendChild(el("p", { class: "mt-4 text-[12px] leading-5 text-muted" }, [
      "Shown " + (reply.order || "in database order") + ". Every finding is on its experiment’s page: ",
      link("Experiments", "/explorer"), " · ", link("Papers", "/papers"),
    ]));
    return box;
  }

  // The whole report for the items in one space. `findings` is { state, reply }.
  function habitatReport(shown, space, findings) {
    var level = reportLevel(findings.reply);
    var floor = level.o2_pct - level.tolerance;
    var rows = reportRollup(shown, level);
    var air = level.name + ", " + tidy(level.o2_pct) + "% O₂ at " + tidy(level.psi) + " psi (" + tidy(level.kpa) + " kPa)";
    var box = el("section", { class: "mt-6", "aria-label": "Habitat report" });

    box.appendChild(el("div", { class: "section-head" }, [el("span", { class: "n", text: "Report" }), el("h2", { text: "Habitat cargo report" })]));
    box.appendChild(el("p", { class: "eyebrow mt-2", text:
      (space === "All" ? "All spaces" : space) + " · " + shown.length + (shown.length === 1 ? " item" : " items") + " · " + rows.length + (rows.length === 1 ? " material" : " materials") +
      " · against " + air + " · " + new Date().toLocaleDateString() }));
    box.appendChild(el("div", { class: "no-print mt-4" }, [
      el("button", { type: "button", class: "btn-big-ghost", text: "Print the report", onclick: function () { window.print(); } }),
    ]));

    if (!shown.length) {
      box.appendChild(el("p", { class: "mt-6 text-[15px] text-muted" }, [space === "All" ? "Nothing scanned yet, so there are no materials to report. " : "Nothing scanned in " + space + " yet, so there are no materials to report. ", link("Scan an item", "/scan")]));
    } else {
      // 01: materials in groups
      box.appendChild(reportHead("01", "Cargo materials in NASA’s microgravity fire tests"));
      box.appendChild(el("p", { class: "mt-2 text-[12px] leading-5 text-muted", text:
        "This oxygen level is " + level.name + ", " + tidy(level.o2_pct) + "% O₂. A test within " + tidy(level.tolerance) + " point of it counts as at that level, so the line is " + tidy(floor) + "%. " +
        "Each group is decided from the test points saved with the scan. A material in several items is read from the most recently checked one." }));
      var GROUPS = [
        { key: "burned", label: "Burned in NASA tests at or above this oxygen level", colour: "text-burn" },
        { key: "below", label: "Tested only below this level", colour: "text-amber" },
        { key: "never", label: "Never tested by NASA in microgravity", colour: "text-fg" },
      ];
      box.appendChild(el("div", { class: "mt-4 grid gap-x-6 gap-y-6 sm:grid-cols-3" }, GROUPS.map(function (g) {
        var mine = rows.filter(function (r) { return r.group === g.key; });
        return el("div", { class: "count-tile" }, [
          el("p", { class: "v " + g.colour, text: String(mine.length) }),
          el("p", { class: "mt-1 text-[12px] leading-4 text-muted", text: g.label }),
          el("ul", { class: "mt-2 border-t border-line" }, mine.map(function (r) { return reportMaterialLine(r, r.detail); })),
        ]);
      })));
      var rest = rows.filter(function (r) { return r.group === "other"; });
      if (rest.length) {
        box.appendChild(el("p", { class: "mt-4 text-[13px] leading-5 break-words" }, [
          el("span", { class: "font-medium", text: "Tested, not in the groups above: " }),
          rest.map(function (r) { return r.name + " (" + r.detail + "; in " + r.items.join(", ") + ")"; }).join(". ") + ".",
        ]));
      }

      // 02: gaps
      var gapped = rows.filter(function (r) { return r.gaps.length; });
      var clear2 = rows.filter(function (r) { return !r.gaps.length; });
      box.appendChild(reportHead("02", "Test next: gaps in the data at Moon habitat conditions"));
      box.appendChild(el("p", { class: "mt-2 text-[12px] leading-5 text-muted", text:
        "What the saved test data does not contain for each material, against " + air + ". Each line is a gap in the data. It says nothing about how the item would behave." }));
      if (gapped.length) {
        box.appendChild(el("ul", { class: "mt-3 border-t border-line" }, gapped.map(function (r) { return reportMaterialLine(r, r.gaps.join("; ")); })));
      } else {
        box.appendChild(el("p", { class: "mt-3 text-[13px] text-muted", text: "None of these gaps was found in the saved data." }));
      }
      if (clear2.length) {
        box.appendChild(el("p", { class: "mt-3 text-[12px] leading-5 text-dim break-words", text:
          "Not listed, because the saved data has a test within " + tidy(level.tolerance) + " point of " + tidy(level.o2_pct) + "% O₂ at a pressure below " + REDUCED_BELOW_KPA + " kPa: " + clear2.map(function (r) { return r.name; }).join(", ") + "." }));
      }
    }

    box.appendChild(reportFindings(findings.state, findings.reply));

    box.appendChild(reportHead("04", "Where the data comes from"));
    box.appendChild(el("p", { class: "mt-2 text-[13px] leading-6" }, [
      link("Experiments", "/explorer"), " · ", link("Papers", "/papers"), " · ", link("Atlas of tested materials", "/atlas"), " · ", link("Ask a question", "/ask"),
    ]));
    box.appendChild(el("p", { class: "mt-2 text-[12px] leading-5 text-dim", text:
      "Made from the scans saved in this browser. Test conditions and findings are extracted from NASA papers by AI and are not all reviewed by a person. This is NASA spacecraft research, not a home safety rating." }));
    return box;
  }

  // ---------- the items page ----------

  function itemsPage(root) {
    var current = "All";
    var open = {};
    var view = "items"; // or "report"
    var findings = { state: "idle", reply: null }; // NASA's findings, fetched once when the report is first opened

    function loadFindings() {
      findings.state = "loading";
      fetch("/items/findings").then(function (r) {
        if (!r.ok) throw new Error("status " + r.status);
        return r.json();
      }).then(function (reply) {
        findings = { state: "ready", reply: reply };
      }).catch(function () {
        findings = { state: "failed", reply: null };
      }).then(function () {
        if (view === "report") draw();
      });
    }

    function draw() {
      var data = loadStore();
      clear(root);

      var tabs = el("div", { class: "flex flex-wrap gap-2", role: "group", "aria-label": "Space" });
      ["All"].concat(SPACES).forEach(function (name) {
        var count = name === "All" ? data.items.length : data.items.filter(function (i) { return i.space === name; }).length;
        tabs.appendChild(el("button", { type: "button", class: "switch-btn", "aria-current": name === current ? "true" : null, text: name + " · " + count, onclick: function () { current = name; draw(); } }));
      });
      var views = el("div", { class: "mt-3 flex flex-wrap gap-2", role: "group", "aria-label": "View" });
      [["items", "Item list"], ["report", "Habitat report"]].forEach(function (v) {
        views.appendChild(el("button", { type: "button", class: "switch-btn", "aria-current": v[0] === view ? "true" : null, text: v[1], onclick: function () { view = v[0]; draw(); } }));
      });
      var controls = el("div", { class: "no-print" }, [tabs, views]);
      root.appendChild(controls);

      var shown = data.items.filter(function (i) { return current === "All" || i.space === current; });
      function tile(value, label, colour) {
        return el("div", { class: "count-tile" }, [
          el("p", { class: "v " + colour, text: String(value) }),
          el("p", { class: "mt-1 text-[12px] leading-4 text-muted", text: label }),
        ]);
      }
      controls.appendChild(el("div", { class: "mt-4 grid grid-cols-3 gap-3" }, [
        tile(shown.length, shown.length === 1 ? "item" : "items", "text-fg"),
        tile(shown.filter(itemBurned).length, "with a material that burned in tests at Moon Base oxygen", "text-burn"),
        tile(shown.filter(itemUntested).length, "with a material NASA hasn’t tested", "text-amber"),
      ]));

      if (view === "report") {
        if (findings.state === "idle") loadFindings();
        root.appendChild(habitatReport(shown, current, findings));
        return;
      }

      if (!shown.length) {
        root.appendChild(el("p", { class: "mt-8 text-[16px] text-muted" }, [current === "All" ? "Nothing saved yet. " : "Nothing in " + current + " yet. ", link("Scan an item", "/scan")]));
      }

      var list = el("ul", { class: "mt-6 space-y-3 no-print" });
      shown.forEach(function (item) {
        var materials = ((item.result || {}).materials || []).map(function (m) { return m.query; }).join(", ");
        var badge = itemBurned(item)
          ? el("span", { class: "text-burn", text: "Burned at Moon Base oxygen" })
          : itemUntested(item)
            ? el("span", { class: "text-amber", text: "Has an untested material" })
            : null;
        var details = el("div", { hidden: !open[item.id] });
        if (open[item.id] && item.result) {
          details.appendChild(summaryBlock(item.summary));
          details.appendChild(resultBody(item.result));
        }

        var move = el("select", { class: "field h-10 w-auto text-[14px]", "aria-label": "Move " + item.name + " to a space" }, SPACES.map(function (name) {
          return el("option", { value: name, text: name, selected: name === item.space });
        }));
        move.addEventListener("change", function () { update(item.id, function (i) { i.space = move.value; }); });

        var recheck = el("button", { type: "button", class: "chip", text: "Check again" });
        recheck.addEventListener("click", function () {
          recheck.disabled = true;
          recheck.textContent = "Checking…";
          fetchResult(((item.result || {}).materials || []).map(function (m) { return m.query; })).then(function (result) {
            update(item.id, function (i) { i.result = result; i.when = new Date().toISOString(); });
          }).catch(function () {
            recheck.disabled = false;
            recheck.textContent = "Couldn’t check. Try again";
          });
        });

        list.appendChild(el("li", { class: "border-t border-fg pt-4" }, [
          el("div", { class: "flex items-start gap-3" }, [
            item.thumb ? el("img", { src: item.thumb, alt: "", class: "w-14 h-14 rounded-lg object-cover bg-ink-800 shrink-0" }) : null,
            el("div", { class: "min-w-0 flex-1" }, [
              el("p", { class: "text-[17px] leading-6 font-semibold break-words", text: item.name }),
              el("p", { class: "text-[13px] leading-5 text-muted break-words", text: materials }),
              el("p", { class: "text-[13px] leading-5 font-medium" }, [badge]),
              el("p", { class: "text-[12px] text-dim", text: item.space + " · checked " + new Date(item.when).toLocaleDateString() }),
            ]),
          ]),
          el("div", { class: "mt-3 flex flex-wrap items-center gap-2" }, [
            el("button", { type: "button", class: "chip", "aria-expanded": open[item.id] ? "true" : "false", text: open[item.id] ? "Hide tests" : "Show tests", onclick: function () { open[item.id] = !open[item.id]; draw(); } }),
            recheck,
            move,
            el("button", { type: "button", class: "chip text-muted ml-auto", text: "Remove", onclick: function () {
              if (!window.confirm("Remove " + item.name + " from this device?")) return;
              var d = loadStore();
              d.items = d.items.filter(function (i) { return i.id !== item.id; });
              saveStore(d);
              draw();
            } }),
          ]),
          details,
        ]));
      });
      root.appendChild(list);
      root.appendChild(el("p", { class: "mt-8 text-[12px] leading-5 text-dim", text: "Saved in this browser only. No account, nothing uploaded. This is NASA spacecraft research, not a home safety rating." }));
    }

    function update(id, change) {
      var data = loadStore();
      data.items.forEach(function (i) { if (i.id === id) change(i); });
      saveStore(data);
      draw();
    }

    draw();
  }

  // ---------- the atlas page: one small map per tested material ----------

  function atlasPage(root) {
    var SORTS = [
      { key: "tests", label: "Most tests" },
      { key: "burned", label: "Burned at or above Moon Base oxygen" },
      { key: "name", label: "Name" },
    ];
    var FILTERS = [
      { key: "all", label: "All", keep: function () { return true; } },
      { key: "burned", label: "Burned at Moon Base level", keep: function (m) { return m.burned_moon > 0; } },
      { key: "below", label: "Tested only below", keep: function (m) { return m.only_below; } },
      { key: "none", label: "No outcome recorded", keep: function (m) { return m.no_outcome; } },
    ];
    var state = { sort: "tests", filter: "all" };
    var data = null;
    var dialog = el("dialog", { class: "atlas-dialog", "aria-label": "Full flammability map" });
    dialog.addEventListener("click", function (e) { if (e.target === dialog) dialog.close(); });

    function openFull(material) {
      clear(dialog);
      dialog.appendChild(el("button", { type: "button", class: "btn-big-ghost atlas-close", text: "Close", onclick: function () { dialog.close(); } }));
      dialog.appendChild(materialSection(material, data));
      dialog.appendChild(el("p", { class: "mt-4 text-[12px] leading-4 text-dim", text: "Listed under the name the papers use. Another spelling of the same material is a separate entry." }));
      if (dialog.showModal) dialog.showModal(); else dialog.setAttribute("open", "");
      dialog.scrollTop = 0;
    }

    function card(material) {
      var few = material.tests < 3;
      var count = (few ? "Only " : "") + material.tests + (material.tests === 1 ? " test" : " tests");
      var chart = flammabilityMap(material, { compact: true });
      var strip = chart ? null : oxygenChart(material, { compact: true });
      var summary = chart
        ? material.map.sentence
        : strip
          ? "By oxygen level only, one mark per stated level: " + strip.getAttribute("data-summary") + ". No test states a pressure that can be read as a number."
          : "No test of it states an oxygen level that can be read as a number, so there is nothing to place.";
      var facts = [count];
      if (material.burned_moon) facts.push(material.burned_moon + " burned at or above " + data.moon_o2 + "% oxygen");
      else if (material.only_below) facts.push("tested only below " + data.moon_o2 + "% oxygen");
      if (material.no_outcome) facts.push("no outcome recorded");
      var node = el("div", { class: "atlas-card", role: "button", tabindex: "0", "aria-label": material.query + ". " + facts.join(", ") + ". Open the full map." }, [
        el("h3", { class: "text-[17px] leading-6 font-semibold break-words", text: material.query }),
        el("p", { class: "mt-0.5 text-[12px] leading-4 " + (few ? "text-amber" : "text-muted"), text: facts.join(" · ") }),
        chart || strip,
        el("p", { class: "mt-2 text-[12px] leading-4 text-muted", text: summary }),
      ]);
      node.addEventListener("click", function () { openFull(material); });
      node.addEventListener("keydown", function (e) { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openFull(material); } });
      return node;
    }

    function switches(list, current, label, pick) {
      return el("div", { class: "flex flex-wrap items-center gap-2", role: "group", "aria-label": label }, [el("span", { class: "text-[12px] text-dim mr-1", text: label })].concat(list.map(function (item) {
        return el("button", { type: "button", class: "switch-btn", "aria-current": item.key === current ? "true" : null, text: item.label, onclick: function () { pick(item.key); } });
      })));
    }

    function draw() {
      clear(root);
      if (data.data !== "ok") {
        root.appendChild(el("p", { class: "notice", role: "status", text: data.data_message || "NASA’s test data could not be read just now." }));
        return;
      }
      function tile(value, text) {
        return el("div", { class: "count-tile" }, [
          el("p", { class: "v", text: String(value) }),
          el("p", { class: "mt-1 text-[12px] leading-4 text-muted", text: text }),
        ]);
      }
      root.appendChild(el("div", { class: "grid grid-cols-3 gap-3" }, [
        tile(data.counts.materials, "materials with NASA microgravity tests"),
        tile(data.counts.plotted, "tests placed by both oxygen and pressure"),
        tile(data.counts.no_moon, "materials with no test at Moon Base oxygen (" + data.moon_o2 + "%)"),
      ]));
      root.appendChild(el("div", { class: "mt-5 space-y-3" }, [
        switches(FILTERS, state.filter, "Show", function (key) { state.filter = key; draw(); }),
        switches(SORTS, state.sort, "Sort by", function (key) { state.sort = key; draw(); }),
      ]));
      var keep = FILTERS.filter(function (f) { return f.key === state.filter; })[0].keep;
      var shown = data.materials.filter(keep).sort(function (a, b) {
        if (state.sort === "name") return a.query.toLowerCase().localeCompare(b.query.toLowerCase());
        if (state.sort === "burned" && b.burned_moon !== a.burned_moon) return b.burned_moon - a.burned_moon;
        return b.tests - a.tests || a.query.toLowerCase().localeCompare(b.query.toLowerCase());
      });
      root.appendChild(el("p", { class: "mt-4 text-[12px] text-dim", role: "status", text: "Showing " + shown.length + " of " + data.materials.length + " materials. Each is listed under the name the papers use. Tap one for the full map and its sources." }));
      if (!shown.length) root.appendChild(el("p", { class: "mt-6 text-[15px] text-muted", text: "No material matches this filter." }));
      root.appendChild(el("div", { class: "atlas-grid" }, shown.map(card)));
      root.appendChild(dialog);
    }

    root.appendChild(el("p", { class: "text-[14px] text-muted", role: "status", text: "Loading NASA’s tests…" }));
    fetch("/atlas/data").then(function (r) {
      if (!r.ok) throw new Error("status " + r.status);
      return r.json();
    }).then(function (reply) {
      data = reply;
      draw();
    }).catch(function () {
      clear(root);
      root.appendChild(el("p", { class: "notice", role: "alert", text: "Couldn’t load the tests. Check your connection and reload the page." }));
    });
  }

  var atlasRoot = document.getElementById("burn-atlas");
  if (atlasRoot) atlasPage(atlasRoot);
  var scanRoot = document.getElementById("burn-scan");
  if (scanRoot) scanPage(scanRoot);
  var itemsRoot = document.getElementById("burn-items");
  if (itemsRoot) itemsPage(itemsRoot);
})();
