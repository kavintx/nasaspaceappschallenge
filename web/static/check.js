// Result page: switch habitat without reloading. The text for every habitat is
// already in the page (blocks marked data-hab); this shows the right one, moves
// the flame, and keeps the address, the mission link and the AI form in step.
(function () {
  var root = document.getElementById("check");
  if (!root) return;
  var buttons = root.querySelectorAll("[data-switch]");
  var flame = document.getElementById("flame");
  var context = document.getElementById("context-line");
  var mission = document.getElementById("mission-link");

  function show(btn) {
    var key = btn.getAttribute("data-switch");
    buttons.forEach(function (b) {
      if (b === btn) b.setAttribute("aria-current", "true");
      else b.removeAttribute("aria-current");
    });
    root.querySelectorAll("[data-hab]").forEach(function (el) {
      el.hidden = el.getAttribute("data-hab") !== key;
      if (!el.hidden) {
        // Replay the short entrance so the change is easy to see.
        el.querySelectorAll(".rise").forEach(function (r) {
          r.style.animation = "none";
          void r.offsetWidth;
          r.style.animation = "";
        });
      }
    });
    if (flame) flame.style.setProperty("--heat", btn.getAttribute("data-heat"));
    if (context) context.textContent = btn.getAttribute("data-context");
    if (mission) mission.setAttribute("href", btn.getAttribute("data-mission"));
    root.querySelectorAll("[data-hab-input]").forEach(function (i) { i.value = key; });
    history.replaceState(null, "", btn.getAttribute("href"));
  }

  buttons.forEach(function (btn) {
    btn.addEventListener("click", function (e) {
      if (e.metaKey || e.ctrlKey || e.shiftKey) return;
      e.preventDefault();
      show(btn);
    });
  });
})();
