// Site-wide motion: scroll reveals, number count-ups, lazy video playback and the
// scroll-driven intro on the overview. No libraries. Everything is skipped for people
// who switch animations off in the footer, and the page is fully readable without it.
(function () {
  "use strict";

  // Animations run for everyone unless the visitor switched them off (footer link).
  var reduced = document.documentElement.classList.contains("motion-off");
  var hasIO = "IntersectionObserver" in window;

  // ---------- reveal on scroll ----------
  // Explicit: [data-reveal]. Groups: [data-reveal-group] staggers its children.
  // Pages that mark nothing get their top-level sections revealed.
  function setUpReveals() {
    var items = [];
    document.querySelectorAll("[data-reveal-group]").forEach(function (group) {
      Array.prototype.forEach.call(group.children, function (child, i) {
        child.setAttribute("data-reveal", "");
        child.style.setProperty("--reveal-delay", Math.min(i, 8) * 60 + "ms");
      });
    });
    document.querySelectorAll("[data-reveal]").forEach(function (n) { items.push(n); });
    if (!items.length) {
      document.querySelectorAll("main:not(#burn-scan) > section").forEach(function (n) {
        n.setAttribute("data-reveal", "");
        items.push(n);
      });
    }
    if (reduced || !hasIO) {
      items.forEach(function (n) { n.classList.add("is-in"); });
      return;
    }
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) {
          e.target.classList.add("is-in");
          io.unobserve(e.target);
        }
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.05 });
    items.forEach(function (n) { io.observe(n); });
  }

  // ---------- count up ----------
  function setUpCounts() {
    var nodes = document.querySelectorAll("[data-count]");
    if (!nodes.length || reduced || !hasIO) return;
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        io.unobserve(e.target);
        var node = e.target;
        var end = parseInt(node.textContent.replace(/[^0-9]/g, ""), 10);
        if (!end || end > 100000) return;
        var start = performance.now();
        (function tick(now) {
          var t = Math.min(1, (now - start) / 900);
          node.textContent = String(Math.round(end * (1 - Math.pow(1 - t, 3))));
          if (t < 1) requestAnimationFrame(tick);
        })(start);
      });
    }, { threshold: 0.6 });
    nodes.forEach(function (n) { io.observe(n); });
  }

  // ---------- looping clips: load and play only while on screen ----------
  function setUpClips() {
    var clips = document.querySelectorAll("video[data-autoplay]");
    if (!clips.length) return;
    if (reduced || !hasIO) {
      clips.forEach(function (v) { v.controls = true; });
      return;
    }
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        var v = e.target;
        if (e.isIntersecting) {
          var p = v.play();
          if (p && p.catch) p.catch(function () { v.controls = true; });
        } else {
          v.pause();
        }
      });
    }, { threshold: 0.25 });
    clips.forEach(function (v) { v.muted = true; io.observe(v); });
  }

  // ---------- overview intro: a tilted screen that flattens as you scroll ----------
  // Scroll progress through the section (0 at its top, 1 when its end reaches the bottom
  // of the window) is written to --p; CSS turns that into the tilt, scale and title slide.
  // On a computer the same progress also moves through the clip. On touch screens,
  // where seeking from scroll is unreliable, the clip plays once when it comes into view.
  function setUpIntro() {
    var intro = document.getElementById("intro");
    var video = document.getElementById("intro-video");
    if (!intro || !video) return;

    if (reduced) {
      intro.classList.add("is-flat", "is-late");
      video.removeAttribute("src");
      video.poster = video.getAttribute("data-end-poster") || video.poster;
      return;
    }

    var touch = window.matchMedia && window.matchMedia("(pointer: coarse)").matches;
    var duration = 0, seeking = false, wanted = 0, queued = false;
    video.addEventListener("loadedmetadata", function () { duration = video.duration || 0; update(); });
    video.addEventListener("seeked", function () { seeking = false; seek(); });

    function seek() {
      if (touch || !duration || seeking || video.readyState < 1) return;
      var t = wanted * Math.max(0, duration - 0.05);
      if (Math.abs(video.currentTime - t) > 0.015) {
        seeking = true;
        try { video.currentTime = t; } catch (e) { seeking = false; }
      }
    }
    function update() {
      queued = false;
      var rect = intro.getBoundingClientRect();
      var room = rect.height - window.innerHeight;
      var p = room > 0 ? Math.min(1, Math.max(0, -rect.top / room)) : 1;
      intro.style.setProperty("--p", p.toFixed(4));
      if (!touch) {
        wanted = p;
        intro.classList.toggle("is-late", p > 0.55);
        seek();
      }
    }
    window.addEventListener("scroll", function () {
      if (!queued) { queued = true; requestAnimationFrame(update); }
    }, { passive: true });
    window.addEventListener("resize", update);
    update();

    if (touch) {
      var played = false;
      var play = function () {
        if (played) return;
        var p = video.play();
        if (p && p.then) p.then(function () { played = true; }).catch(function () { /* retried on first touch */ });
      };
      video.addEventListener("timeupdate", function () {
        if (video.duration) intro.classList.toggle("is-late", video.currentTime / video.duration > 0.55);
      });
      if (hasIO) {
        var io = new IntersectionObserver(function (entries) {
          if (entries[0].isIntersecting) { play(); io.disconnect(); }
        }, { threshold: 0.5 });
        io.observe(video);
      } else {
        play();
      }
      document.addEventListener("touchstart", play, { once: true, passive: true });
    }
  }

  // Footer switch: lets a visitor turn the animations off (or back on) for themselves.
  function setUpSwitch() {
    var button = document.getElementById("motion-switch");
    var state = document.getElementById("motion-state");
    if (!button || !state) return;
    state.textContent = reduced ? "off" : "on";
    button.textContent = reduced ? "Turn on" : "Turn off";
    button.addEventListener("click", function () {
      try { localStorage.setItem("motion", reduced ? "on" : "off"); } catch (e) { /* storage blocked: nothing to remember */ }
      location.reload();
    });
  }

  function start() {
    setUpSwitch();
    setUpIntro();
    setUpReveals();
    setUpCounts();
    setUpClips();
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();
