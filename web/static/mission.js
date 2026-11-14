// Mission replay: six steps of one fire, for kids. The server sends the habitat,
// the first chosen material, the protections and the facts (with their sources)
// as JSON. What happens depends on the oxygen level, the protections and the
// choices made. Text from the database is only ever set as text, never as HTML.
(function () {
  var raw = document.getElementById("mission-data");
  if (!raw) return;
  var d = JSON.parse(raw.textContent);
  var cards = document.querySelectorAll("#steps [data-step]");
  var question = document.getElementById("question");
  var choices = document.getElementById("choices");
  var hint = document.getElementById("hint");
  var progress = document.getElementById("progress");

  var BRIEF = "Source: NASA challenge brief.";
  var moreOxygen = d.habitat.o2 > d.habitat.earthO2;
  var state;

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text) n.textContent = text;
    return n;
  }

  function link(text, href) {
    var a = el("a", "text-sky hover:underline", text);
    a.href = href;
    if (/^https:/.test(href)) { a.target = "_blank"; a.rel = "noopener"; }
    return a;
  }

  // A fact from the research database, linked to its paper.
  function dbFact(f) {
    var p = el("span", "", "From the research: " + f.text + " ");
    p.appendChild(link(f.source || "Paper", f.href));
    return p;
  }

  function setHint(parts) {
    hint.textContent = "";
    parts.forEach(function (part, i) {
      if (i) hint.appendChild(document.createElement("br"));
      hint.appendChild(typeof part === "string" ? document.createTextNode(part) : part);
    });
  }

  function mark(i, status, tone) {
    var s = cards[i].querySelector("[data-status]");
    s.textContent = status;
    s.className = "text-[13px] font-medium mt-auto " + tone;
  }

  function button(label, primary, onClick) {
    var b = el("button", primary ? "btn-big" : "btn-big-ghost", label);
    b.type = "button";
    b.addEventListener("click", onClick);
    choices.appendChild(b);
    return b;
  }

  // Show what the choice led to, then a single button to go on.
  function result(i, status, tone, text) {
    mark(i, status, tone);
    cards[i].setAttribute("data-state", "done");
    question.textContent = text;
    choices.textContent = "";
    var last = i === steps.length - 1;
    button(last ? "See how it went" : "Next", true, function () { last ? finish() : enter(i + 1); }).focus();
  }

  var steps = [
    function spark(i) {
      return {
        card: "A wire gets too hot behind a panel.",
        ask: "A wire gets too hot behind a wall panel. A spark!",
        hint: ["This is the story for the game. The facts in the next steps have sources."],
        options: [["See what happens", true, function () { enter(i + 1, "Started", "text-dim"); }]],
      };
    },
    function flame(i) {
      // The chosen material is only said to burn when a NASA test recorded that it did.
      var burned = d.recorded && d.recorded.burned;
      var what = burned ? "The " + d.material : "Something near the panel";
      var air = moreOxygen
        ? "This air has more oxygen than Earth air: " + d.habitat.o2 + "% here, " + d.habitat.earthO2 + "% on Earth. More oxygen makes things burn more easily."
        : "This air has the same oxygen as Earth air: " + d.habitat.o2 + "%.";
      var hints = ["How well things burn depends on how much oxygen is in the air. " + BRIEF];
      if (d.recorded) {
        var rec = el("span", "", "Your " + d.material + " in a NASA test: " + d.recorded.text + ". ");
        if (d.recorded.href) rec.appendChild(link("See the paper", d.recorded.href));
        hints.push(rec);
      } else if (d.material) {
        hints.push("You picked " + d.material + ". No burn result is recorded for it yet.");
      }
      return {
        card: what + " catches fire.",
        ask: what + " catches fire. " + air,
        hint: hints,
        options: [["Next", true, function () {
          enter(i + 1, moreOxygen ? "Burns easily" : "Burning", moreOxygen ? "text-burn" : "text-amber");
        }]],
      };
    },
    function smoke(i) {
      var hints = ["NASA measured smoke in space to test smoke detectors, in experiments called SAME and SAME-R. " + BRIEF];
      if (d.facts.detection) hints.push(dbFact(d.facts.detection));
      if (!d.protections.detect) {
        return {
          card: "No smoke detector here.",
          ask: "This habitat has no smoke detector. Nobody knows about the fire yet.",
          hint: hints,
          options: [["Next", true, function () { state.early = false; enter(i + 1, "Not noticed", "text-amber"); }]],
        };
      }
      return {
        card: "The smoke detector beeps.",
        ask: "The smoke detector beeps! What do you do?",
        hint: hints,
        options: [
          ["Sound the alarm", true, function () { state.early = true; result(i, "Caught early", "text-sky", "The whole crew knows. You caught it early."); }],
          ["Wait and see", false, function () { state.early = false; result(i, "Lost time", "text-amber", "The fire keeps growing while you wait."); }],
        ],
      };
    },
    function airflow(i) {
      var hints = ["Hint: in space, how well things burn also depends on moving air. " + BRIEF];
      if (d.facts.prevention) hints.push(dbFact(d.facts.prevention));
      if (!d.protections.airflow) {
        return {
          card: "No airflow control here.",
          ask: "This habitat has no airflow control. The fans keep running.",
          hint: hints,
          options: [["Next", true, function () { enter(i + 1, "Fans stay on", "text-amber"); }]],
        };
      }
      return {
        card: "Cut the fans, or keep them on?",
        ask: "The fire is growing. What do you do with the fans?",
        hint: hints,
        options: [
          ["Cut the airflow", true, function () { result(i, "Fans off", "text-sky", "The fans stop. Less moving air reaches the fire."); }],
          ["Keep the fans on", false, function () { result(i, "Fans on", "text-amber", "The fans keep blowing air at the fire."); }],
        ],
      };
    },
    function putOut(i) {
      var hints = ["NASA's FLEX experiment put out flames by adding carbon dioxide or helium to the air. " + BRIEF];
      if (d.facts.suppression) hints.push(dbFact(d.facts.suppression));
      var leave = "The fire keeps burning. The crew leaves the room and shuts the hatch.";
      if (!d.protections.suppress) {
        return {
          card: "Nothing here to put it out.",
          ask: "This habitat has no fire suppression. The crew leaves the room and shuts the hatch.",
          hint: hints,
          options: [["Next", true, function () { state.out = false; enter(i + 1, "Not put out", "text-burn"); }]],
        };
      }
      return {
        card: "Can the crew stop it?",
        ask: "Time to stop the fire. What do you do?",
        hint: hints,
        options: [
          ["Use the extinguisher", true, function () { state.out = true; result(i, "Fire out", "text-safe", "The extinguisher gas thins out the air around the fire. The fire goes out."); }],
          ["Leave it", false, function () { state.out = false; result(i, "Still burning", "text-burn", leave); }],
        ],
      };
    },
    function air(i) {
      var start = state.out ? "The fire is out." : "The crew is safe behind the hatch.";
      var gases = d.gases
        ? " Burning " + d.gases.tableName + " gives off " + d.gases.list.map(function (g) { return g.name.toLowerCase() + " (" + g.formula + ")"; }).join(", ") + "."
        : "";
      var src = el("span", "", "After a fire NASA watches four gases: CO, HF, HCl and HCN. You cannot tell if they are there by looking. Sources: ");
      src.appendChild(link("the gas monitor paper", d.sources.analyzer));
      src.appendChild(document.createTextNode(" (" + d.sources.tablePage + ") and "));
      src.appendChild(link("NASA's gas limits", d.sources.limits));
      src.appendChild(document.createTextNode("."));
      return {
        card: "Masks are on. Is the air clean?",
        ask: start + " Burning things leave gases you cannot see." + gases + " Your mask is on. What now?",
        hint: [src],
        options: [
          ["Check the gas monitor", true, function () { state.checked = true; result(i, "Checked", "text-safe", "Good call. Masks stay on until the monitor says the air is clean."); }],
          ["Take the mask off", false, function () { state.checked = false; result(i, "Too soon", "text-burn", "Not yet! You cannot tell by looking. Mask back on, then check the monitor."); }],
        ],
      };
    },
  ];

  // Move to step i. The step before it gets its status, unless result() already set one.
  function enter(i, prevStatus, prevTone) {
    if (i > 0 && prevStatus) {
      mark(i - 1, prevStatus, prevTone);
      cards[i - 1].setAttribute("data-state", "done");
    }
    var s = steps[i](i);
    cards[i].setAttribute("data-state", "now");
    cards[i].querySelector("[data-text]").textContent = s.card;
    mark(i, "Your move", "text-fg");
    progress.textContent = "Step " + (i + 1) + " of " + steps.length;
    question.textContent = s.ask;
    setHint(s.hint);
    choices.textContent = "";
    s.options.forEach(function (o, n) {
      var b = button(o[0], o[1], o[2]);
      if (n === 0 && i > 0) b.focus();
    });
  }

  function finish() {
    progress.textContent = "Mission over";
    var lines = [];
    lines.push(state.early ? "You caught the fire early." : "The fire was not caught early.");
    lines.push(state.out ? "The crew put the fire out." : "The fire was not put out. The crew got away from it.");
    lines.push(state.checked ? "You checked the air before taking masks off." : "Next time, check the gas monitor before the mask comes off.");
    question.textContent = state.out && state.checked ? "Mission complete. The crew is safe." : "The crew is safe. Some things could go better.";
    setHint(lines);
    choices.textContent = "";
    button("Play again", true, start).focus();
    var back = el("a", "btn-big-ghost", "Change the habitat");
    back.href = d.checkHref;
    choices.appendChild(back);
  }

  function start() {
    state = { early: false, out: false, checked: false };
    cards.forEach(function (c) {
      c.setAttribute("data-state", "locked");
      c.querySelector("[data-text]").textContent = "";
      var s = c.querySelector("[data-status]");
      s.textContent = "Locked";
      s.className = "text-[13px] font-medium text-dim mt-auto";
    });
    enter(0);
  }

  start();
})();
