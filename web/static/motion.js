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
  // Scroll progress through the section runs from 0 (its top at the top of the window) to 1
  // (its end at the bottom of the window). The screen does not jump to that value: every
  // frame it glides part of the way there, so a mouse wheel's 100-pixel steps still come
  // out as one continuous movement. The screen ends up filling the whole window. The
  // tilt, scale and title are set directly as
  // transforms; on a computer the same eased value also moves through the clip.
  function setUpIntro() {
    var intro = document.getElementById("intro");
    var video = document.getElementById("intro-video");
    if (!intro || !video) return;
    var head = intro.querySelector(".cscroll-head");
    var card = intro.querySelector(".cscroll-card");
    var caption = intro.querySelector(".cscroll-caption");

    if (reduced) {
      intro.classList.add("is-flat", "is-late");
      video.removeAttribute("src");
      video.poster = video.getAttribute("data-end-poster") || video.poster;
      return;
    }

    var touch = window.matchMedia && window.matchMedia("(pointer: coarse)").matches;
    var wide = window.matchMedia && window.matchMedia("(min-width: 768px)");
    var target = 0, shown = -1, last = 0, running = false;
    var duration = 0, seeking = false;
    var GLIDE_MS = 110; // time constant of the glide: larger is floatier

    video.addEventListener("loadedmetadata", function () { duration = video.duration || 0; kick(); });
    video.addEventListener("seeked", function () { seeking = false; });

    function ease(p) { return p * p * (3 - 2 * p); } // gentle start and finish

    function paint(p) {
      var e = ease(p);
      // Starts small, tilted back and low enough to clear the title; ends flat at full size.
      var big = wide && wide.matches;
      var scale = (big ? 0.6 : 0.66) + (big ? 0.4 : 0.34) * e;
      var drop = (big ? 21 : 19) * (1 - e); // vh
      card.style.transform = "translate3d(0," + drop.toFixed(3) + "vh,0) rotateX(" + ((1 - e) * 20).toFixed(3) + "deg) scale(" + scale.toFixed(4) + ")";
      card.style.borderRadius = (30 - 30 * Math.max(0, (e - 0.8) / 0.2)).toFixed(1) + "px";
      // The title slides up and is gone by the time the screen is two thirds of the way.
      var gone = Math.min(1, e / 0.65);
      head.style.transform = "translate3d(0," + (-60 * gone).toFixed(2) + "px,0)";
      head.style.opacity = (1 - gone).toFixed(3);
      // Near the end the caption fades, as the next part of the page slides up over the screen.
      if (caption) caption.style.opacity = (1 - Math.min(1, Math.max(0, (p - 0.9) / 0.1))).toFixed(3);
      if (!touch) {
        intro.classList.toggle("is-late", p > 0.55);
        if (duration && !seeking && video.readyState >= 1) {
          var t = p * Math.max(0, duration - 0.05);
          if (Math.abs(video.currentTime - t) > 0.02) {
            seeking = true;
            try { video.currentTime = t; } catch (err) { seeking = false; }
          }
        }
      }
    }

    function measure() {
      var rect = intro.getBoundingClientRect();
      var room = rect.height - window.innerHeight;
      // Finished a little before the end: the last stretch is where the next block slides over.
      target = room > 0 ? Math.min(1, Math.max(0, -rect.top / (room * 0.8))) : 1;
    }

    function frame(now) {
      var dt = Math.min(64, now - last);
      last = now;
      if (shown < 0) shown = target; // first frame: start where the page is
      shown += (target - shown) * (1 - Math.exp(-dt / GLIDE_MS));
      var done = Math.abs(target - shown) < 0.0004 && !seeking;
      if (done) shown = target;
      paint(shown);
      if (done) { running = false; return; }
      requestAnimationFrame(frame);
    }

    function kick() {
      measure();
      if (running) return;
      running = true;
      last = performance.now();
      requestAnimationFrame(frame);
    }

    window.addEventListener("scroll", kick, { passive: true });
    window.addEventListener("resize", kick);
    kick();

    // Touch screens: seeking a video from scroll is unreliable there, so the clip
    // plays once when it comes into view while the screen still tilts with the scroll.
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
