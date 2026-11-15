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
    { key: "burned", label: "Burned", color: "#ff6a3d" },
    { key: "went_out", label: "Went out on its own", color: "#e5b67a" },
    { key: "no_ignition", label: "Did not ignite", color: "#4fd38a" },
    { key: "unrecorded", label: "Outcome not recorded", color: "#a5a8b5" },
  ];
  function outcomeMark(key, x, y, r) {
    var o = OUTCOMES.filter(function (it) { return it.key === key; })[0] || OUTCOMES[3];
    var ring = { stroke: "#090d18", "stroke-width": 1.5 };
    var mark;
    if (key === "burned") mark = svg("circle", { cx: x, cy: y, r: r, fill: o.color });
    else if (key === "went_out") mark = svg("rect", { x: x - r * 0.8, y: y - r * 0.8, width: r * 1.6, height: r * 1.6, fill: o.color, transform: "rotate(45 " + x + " " + y + ")" });
    else if (key === "no_ignition") mark = svg("path", { d: "M" + x + " " + (y - r) + "L" + (x + r) + " " + (y + r * 0.8) + "L" + (x - r) + " " + (y + r * 0.8) + "Z", fill: o.color });
    else return svg("circle", { cx: x, cy: y, r: r - 1, fill: "#090d18", stroke: o.color, "stroke-width": 2 });
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
    var kids = [svg("circle", { cx: 50, cy: 50, r: r, fill: "none", stroke: "#1a2030", "stroke-width": width })];
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
    kids.push(svgText(String(centre), { x: 50, y: under ? 51 : 56, "text-anchor": "middle", "font-size": 22, "font-weight": 700, fill: "#f5f5f7" }));
    if (under) kids.push(svgText(under, { x: 50, y: 64, "text-anchor": "middle", "font-size": 9, fill: "#a5a8b5" }));
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

  // How much of the item NASA's data covers: materials with at least one test, out of all of them.
  function coverageDonut(result) {
    var all = result.materials || [];
    if (all.length < 1 || result.data !== "ok") return null;
    var tested = all.filter(function (m) { return m.tested; }).length;
    var ring = donut([
      { label: "Has NASA tests", value: tested, color: "#2997ff" },
      { label: "No NASA test", value: all.length - tested, color: "#3a4256" },
    ], 112, tested + "/" + all.length, "tested");
    if (!ring) return null;
    return el("figure", { class: "donut-figure donut-figure-top" }, [
      ring,
      el("div", { class: "min-w-0" }, [
        el("figcaption", { class: "text-[17px] leading-6 font-semibold", text: tested === all.length
          ? (all.length === 1 ? "NASA has tested this material." : "NASA has tested all " + all.length + " of its materials.")
          : tested === 0 ? "None of its materials are in NASA’s tests."
          : "NASA has tested " + tested + " of its " + all.length + " materials." }),
        el("ul", { class: "mt-2 space-y-1 text-[13px] text-muted" }, all.map(function (m) {
          return legendRow(m.tested ? "#2997ff" : "#3a4256", m.query + (m.tested ? "" : " · no test"));
        })),
      ]),
    ]);
  }

  // Every NASA test of this material placed on an oxygen scale, with the two habitats marked.
  function oxygenChart(material) {
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
      kids.push(svg("rect", { x: x(h.v - 1), y: top - 8, width: x(h.v + 1) - x(h.v - 1), height: base - top + 8, rx: 3, fill: "#2997ff", opacity: 0.16 }));
      kids.push(svg("line", { x1: x(h.v), x2: x(h.v), y1: top - 8, y2: base, stroke: "#2997ff", "stroke-width": 1, "stroke-dasharray": "3 3", opacity: 0.8 }));
      kids.push(svgText(h.name, { x: x(h.v), y: 11, "text-anchor": "middle", "font-size": 10, "font-weight": 600, fill: "#f5f5f7" }));
    });
    kids.push(svg("line", { x1: left, x2: W - right, y1: base, y2: base, stroke: "#2a3040", "stroke-width": 1 }));
    for (var tick = lo; tick <= hi; tick += (hi - lo > 50 ? 20 : hi - lo > 25 ? 10 : 5)) {
      kids.push(svg("line", { x1: x(tick), x2: x(tick), y1: base, y2: base + 4, stroke: "#2a3040", "stroke-width": 1 }));
      kids.push(svgText(tick + "%", { x: x(tick), y: base + 16, "text-anchor": "middle", "font-size": 10, fill: "#a5a8b5" }));
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
    return el("figure", { class: "o2-chart" }, [
      el("figcaption", { class: "text-[13px] font-medium", text: "NASA’s tests by oxygen level" }),
      svg("svg", { viewBox: "0 0 " + W + " " + H, class: "mt-2 w-full h-auto", role: "img", "aria-label": "Tests placed by oxygen level, from " + lo + "% to " + hi + "%: " + summary + "." }, kids),
      legend,
      el("p", { class: "mt-1 text-[11px] leading-4 text-dim", text: "Each mark is one test at one oxygen level. Shaded bands are within one point of each habitat’s air."
        + (data.no_level ? " " + data.no_level + (data.no_level === 1 ? " more test has" : " more tests have") + " no oxygen level recorded." : "") }),
    ]);
  }

  // What the item is made of, part by part. Colour follows the material, in a fixed order.
  var MATERIAL_COLORS = ["#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#9085e9"];
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
      return i < MATERIAL_COLORS.length ? MATERIAL_COLORS[i] : "#8b8f9d";
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
      var chart = oxygenChart(material);
      if (chart) section.appendChild(chart);
      section.appendChild(evidenceList(material.evidence));
    } else {
      var why = result.data !== "ok"
        ? "No NASA test data to check this against yet."
        : result.lookup === "synonyms"
          ? "No test in NASA’s data under this name or its listed other names."
        : result.lookup !== "ok"
          ? "No test in NASA’s data under exactly this name."
          : material.query + " isn’t in NASA’s tests.";
      section.appendChild(el("p", { class: "mt-2 text-[17px] leading-6 font-semibold text-muted", text: why }));
      var near = material.closest;
      if (near) {
        var box = el("div", { class: "mt-4 rounded-xl border border-dashed border-line p-4" }, [
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
    section.appendChild(gasBlock(material, result.sources));
    return section;
  }

  function flameFigure() {
    // A short silent loop. It plays by itself, except for people who ask their device for
    // reduced motion: they get the still frame and a play button.
    var still = document.documentElement.classList.contains("motion-off");
    function stage(name, caption) {
      var picture = el("video", {
        src: "/static/img/" + name + ".mp4", poster: "/static/img/" + name + ".webp",
        autoplay: !still, controls: still, loop: true, muted: true, playsinline: true, preload: "metadata",
        class: "w-full aspect-[4/3] object-cover rounded-xl bg-black",
      });
      picture.muted = true; // the attribute alone doesn't mute a video made in script
      return el("div", {}, [picture, el("p", { class: "mt-2 text-[12px] text-muted text-center", text: caption })]);
    }
    return el("figure", { class: "mt-5", role: "img", "aria-label": "Two clips of a round blue flame on a wick. The one at 30% oxygen is larger and brighter than the one at 21%." }, [
      el("div", { class: "grid grid-cols-2 gap-3" }, [stage("flame-o2-30", "Moon Base · 30% O₂"), stage("flame-o2-21", "ISS · 21% O₂")]),
      el("figcaption", { class: "mt-2 text-[12px] leading-4 text-dim", text: "Illustration, made with AI. Not NASA footage and not a measurement. Flames in weightlessness are round and blue; more oxygen makes them bigger and brighter." }),
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
    return el("div", { class: "mt-4 rounded-xl bg-ink-800 border border-line p-4" }, [
      el("p", { class: "text-[12px] font-medium text-sky", text: "AI summary of the photo" }),
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
          state.model = reply.model || null;
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
      if (state.model && state.rows.length) box.appendChild(el("p", { class: "mt-2 text-[12px] text-dim", text: "Identified by " + state.model + ". AI can be wrong: check each material." }));

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
          var unsure = typeof row.confidence === "number" && row.confidence < UNSURE;
          list.appendChild(el("li", {}, [
            row.part ? el("p", { class: "text-[12px] text-muted mb-1", text: row.part }) : null,
            el("div", { class: "flex items-center gap-2" }, [
              material,
              shareBox,
              el("button", { type: "button", class: "w-11 h-11 shrink-0 rounded-md border border-line text-muted text-[20px] leading-none", "aria-label": "Remove " + (row.material || "material"), text: "×", onclick: function () { state.rows.splice(i, 1); drawRows(); } }),
            ]),
            unsure ? flag : null,
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

  // ---------- the items page ----------

  function itemsPage(root) {
    var current = "All";
    var open = {};

    function draw() {
      var data = loadStore();
      clear(root);

      var tabs = el("div", { class: "flex flex-wrap gap-2", role: "group", "aria-label": "Space" });
      ["All"].concat(SPACES).forEach(function (name) {
        var count = name === "All" ? data.items.length : data.items.filter(function (i) { return i.space === name; }).length;
        tabs.appendChild(el("button", { type: "button", class: "switch-btn", "aria-current": name === current ? "true" : null, text: name + " · " + count, onclick: function () { current = name; draw(); } }));
      });
      root.appendChild(tabs);

      var shown = data.items.filter(function (i) { return current === "All" || i.space === current; });
      function tile(value, label, colour) {
        return el("div", { class: "rounded-xl bg-ink-800 border border-line p-3" }, [
          el("p", { class: "text-[30px] leading-9 font-semibold " + colour, text: String(value) }),
          el("p", { class: "mt-1 text-[12px] leading-4 text-muted", text: label }),
        ]);
      }
      root.appendChild(el("div", { class: "mt-4 grid grid-cols-3 gap-3" }, [
        tile(shown.length, shown.length === 1 ? "item" : "items", "text-fg"),
        tile(shown.filter(itemBurned).length, "with a material that burned in tests at Moon Base oxygen", "text-burn"),
        tile(shown.filter(itemUntested).length, "with a material NASA hasn’t tested", "text-amber"),
      ]));

      if (!shown.length) {
        root.appendChild(el("p", { class: "mt-8 text-[16px] text-muted" }, [current === "All" ? "Nothing saved yet. " : "Nothing in " + current + " yet. ", link("Scan an item", "/scan")]));
      }

      var list = el("ul", { class: "mt-6 space-y-3" });
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

        list.appendChild(el("li", { class: "rounded-xl bg-ink-850 border border-line p-4" }, [
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

  var scanRoot = document.getElementById("burn-scan");
  if (scanRoot) scanPage(scanRoot);
  var itemsRoot = document.getElementById("burn-items");
  if (itemsRoot) itemsPage(itemsRoot);
})();
