// Streaming research chat. Talks to this app's /ask/stream, which relays the
// backend's event stream. All text is inserted with textContent, never as HTML.
(function () {
  var thread = document.getElementById("thread");
  var form = document.getElementById("chat-form");
  var input = document.getElementById("chat-input");
  var send = document.getElementById("chat-send");
  var scope = document.getElementById("scope");
  var starters = document.getElementById("starters");
  var conversationId = null;
  var busy = false;

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  // Light Markdown: headings, bullets, paragraphs.
  function renderAnswer(box, text) {
    box.textContent = "";
    text.split("\n").forEach(function (raw) {
      var line = raw.trim();
      if (!line || /^[-|:=* ]+$/.test(line)) return;
      var clean = function (t) { return t.replace(/\*\*|__|`/g, "").trim(); };
      if (line.charAt(0) === "#") {
        box.appendChild(el("h4", "mt-6 text-[17px] font-medium", clean(line.replace(/^#+/, ""))));
      } else if (/^[-*•] /.test(line)) {
        box.appendChild(el("p", "mt-2 pl-5 -indent-5 text-[15px] leading-relaxed text-muted", "•  " + clean(line.slice(2))));
      } else {
        box.appendChild(el("p", "mt-3 text-[15px] leading-relaxed text-muted", clean(line)));
      }
    });
  }

  function renderCitations(box, citations) {
    box.textContent = "";
    if (!citations || !citations.length) return;
    box.appendChild(el("div", "mt-7 eyebrow", "Cited NASA papers"));
    citations.forEach(function (c) {
      var row = el("div", "mt-3 pb-3 border-b border-line");
      var id = c.ntrs_id || c.paper_id;
      var title = el(id ? "a" : "span", "text-[15px] text-sky hover:underline", c.title || "Untitled NASA paper");
      if (id) title.href = "/papers/" + encodeURIComponent(id);
      row.appendChild(title);
      var meta = [];
      if (c.ntrs_id) meta.push("NTRS " + c.ntrs_id);
      if (c.page != null) meta.push("p. " + c.page);
      row.appendChild(el("div", "mt-1 eyebrow", meta.join(" · ")));
      if (c.quote) row.appendChild(el("p", "mt-2 text-[13px] leading-relaxed text-dim", "“" + c.quote + "”"));
      box.appendChild(row);
    });
  }

  function ask(message) {
    if (busy || !message.trim()) return;
    busy = true;
    send.disabled = true;
    send.textContent = "Answering…";
    starters.hidden = true;

    var turn = el("div", "py-7 border-b border-line");
    turn.appendChild(el("div", "eyebrow", "You asked"));
    turn.appendChild(el("p", "mt-2 text-[22px] leading-snug font-medium", message));
    var answer = el("div", "mt-4 whitespace-pre-wrap text-[15px] leading-relaxed text-muted", "Searching the papers…");
    var cites = el("div");
    turn.appendChild(answer);
    turn.appendChild(cites);
    thread.appendChild(turn);
    turn.scrollIntoView({ behavior: "smooth", block: "start" });

    var text = "";
    var failed = false;
    function fail(msg) {
      failed = true;
      answer.className = "mt-4 text-[14px] text-amber";
      answer.textContent = msg;
    }
    function handle(block) {
      var data = "";
      block.split("\n").forEach(function (l) {
        if (l.indexOf("data:") === 0) data += l.slice(5).trim();
      });
      if (!data) return;
      var ev;
      try { ev = JSON.parse(data); } catch (e) { return; }
      if (ev.type === "meta") {
        if (ev.conversation_id) conversationId = ev.conversation_id;
        renderCitations(cites, ev.citations);
      } else if (ev.type === "delta") {
        text += ev.text || "";
        answer.textContent = text;
      } else if (ev.type === "error") {
        fail(ev.error || "The AI service reported an error.");
      } else if (ev.type === "done" && ev.conversation_id) {
        conversationId = ev.conversation_id;
      }
    }

    fetch("/ask/stream", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ message: message, conversation_id: conversationId, experiment: scope.value || null }),
    })
      .then(function (res) {
        if (!res.ok) return res.text().then(function (t) { throw new Error(t || "Request failed (" + res.status + ")."); });
        var reader = res.body.getReader();
        var decoder = new TextDecoder();
        var buffer = "";
        function pump() {
          return reader.read().then(function (step) {
            if (step.done) { if (buffer.trim()) handle(buffer); return; }
            buffer += decoder.decode(step.value, { stream: true }).replace(/\r\n/g, "\n");
            var parts = buffer.split("\n\n");
            buffer = parts.pop();
            parts.forEach(handle);
            return pump();
          });
        }
        return pump();
      })
      .then(function () {
        if (failed) return;
        if (text.trim()) { answer.className = "mt-2"; renderAnswer(answer, text); }
        else fail("The AI service returned an empty answer.");
      })
      .catch(function (e) { fail(e.message || "Couldn't reach the AI service."); })
      .then(function () {
        busy = false;
        send.disabled = false;
        send.textContent = "Ask ↑";
        input.focus();
      });
  }

  form.addEventListener("submit", function (e) {
    e.preventDefault();
    var m = input.value;
    input.value = "";
    ask(m);
  });
  input.addEventListener("keydown", function (e) {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); form.requestSubmit(); }
  });
  document.querySelectorAll(".starter").forEach(function (b) {
    b.addEventListener("click", function () { ask(b.textContent); });
  });
  document.getElementById("new-chat").addEventListener("click", function () {
    if (busy) return;
    conversationId = null;
    thread.textContent = "";
    starters.hidden = false;
    input.focus();
  });
  // A question passed in the address (?q=...) is asked straight away.
  if (input.value.trim()) { var first = input.value; input.value = ""; ask(first); }
})();
