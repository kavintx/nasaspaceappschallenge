// A flame drawn in 3D, live, in the page: no video, no library.
//
// It is an illustration of two things the site says in words. With gravity, hot gas rises
// and pulls a flame into a flickering teardrop. In freefall nothing rises, so the flame is a
// steady sphere, and more oxygen makes it bigger and brighter. It is not NASA footage and
// not a measurement: the shape comes from the two sliders, not from any test.
//
// How it is drawn: each pixel sends a ray through a glowing volume (a fragment shader). The
// volume is one shape that slides between a sphere (gravity 0) and a tapered, rising,
// flickering teardrop (gravity 1). Drag to turn it.
//
//   var flame = Flame3D.mount(canvas, { gravity: 1, oxygen: 0 });
//   flame.set({ gravity: 0, oxygen: 1 });   flame.glide({ gravity: 0 }, 2600);
(function () {
  "use strict";

  var VERTEX = "attribute vec2 p; void main() { gl_Position = vec4(p, 0.0, 1.0); }";

  var FRAGMENT = [
    "precision highp float;",
    "uniform vec2 uRes; uniform float uTime; uniform float uG; uniform float uO; uniform float uYaw;",
    "float hash(vec3 p) { p = fract(p * 0.3183099 + 0.1); p *= 17.0; return fract(p.x * p.y * p.z * (p.x + p.y + p.z)); }",
    "float noise(vec3 x) {",
    "  vec3 i = floor(x); vec3 f = fract(x); f = f * f * (3.0 - 2.0 * f);",
    "  return mix(mix(mix(hash(i), hash(i + vec3(1,0,0)), f.x), mix(hash(i + vec3(0,1,0)), hash(i + vec3(1,1,0)), f.x), f.y),",
    "             mix(mix(hash(i + vec3(0,0,1)), hash(i + vec3(1,0,1)), f.x), mix(hash(i + vec3(0,1,1)), hash(i + vec3(1,1,1)), f.x), f.y), f.z);",
    "}",
    "float fbm(vec3 p) { return 0.5 * noise(p) + 0.25 * noise(p * 2.03) + 0.125 * noise(p * 4.01); }",
    // Distance to the flame's surface (negative inside). R is its radius in freefall.
    "float flame(vec3 p, float R) {",
    "  float g = uG;",
    "  vec3 q = p - vec3(0.0, g * R * 0.95, 0.0);",          // buoyancy lifts the centre
    "  float h = q.y / R;",
    "  float narrow = 1.0 + g * 1.15 * smoothstep(-0.6, 2.2, h);", // and tapers it to a tip
    "  vec3 e = vec3(q.x * narrow, q.y / mix(1.0, 2.3, g), q.z * narrow);",
    "  e.x += g * 0.10 * sin(uTime * 5.3 + p.y * 3.0) * smoothstep(-1.0, 1.0, h);",
    "  float n = fbm(vec3(p.x * 2.6, p.y * 2.2 - uTime * 2.6, p.z * 2.6));", // flicker travels upward
    "  float wobble = g * 0.34 * (n - 0.45) * smoothstep(-1.0, 1.5, h);",
    "  float breathe = (1.0 - g) * 0.02 * sin(uTime * 1.3);",
    "  return length(e) - R * (1.0 + wobble + breathe);",
    "}",
    "float wick(vec3 p) { vec3 a = vec3(0.0, -1.6, 0.0); float t = clamp((p.y - a.y) / 1.6, 0.0, 1.0); return length(p - vec3(0.0, a.y + 1.6 * t, 0.0)) - 0.028; }",
    "void main() {",
    "  vec2 uv = (gl_FragCoord.xy * 2.0 - uRes) / min(uRes.x, uRes.y);",
    "  float R = mix(0.40, 0.60, uO);",
    "  float cy = cos(uYaw), sy = sin(uYaw);",
    "  vec3 target = vec3(0.0, 0.02 + 0.85 * uG, 0.0);",
    "  vec3 ro = target + vec3(sy * 4.2, 0.35, cy * 4.2);",
    "  vec3 fw = normalize(target - ro); vec3 rt = normalize(cross(fw, vec3(0,1,0))); vec3 up = cross(rt, fw);",
    "  vec3 rd = normalize(fw * mix(3.5, 2.15, uG) + rt * uv.x + up * uv.y);",  // closer in freefall, so the sphere fills the window
    "  vec3 col = vec3(0.043, 0.043, 0.051);",
    // The wick: a thin rod found by stepping along the ray.
    "  float tw = 0.0; float hit = -1.0;",
    "  for (int i = 0; i < 28; i++) { float d = wick(ro + rd * tw); if (d < 0.002) { hit = tw; break; } tw += d; if (tw > 9.0) break; }",
    "  float tEnd = 8.0;",
    "  if (hit > 0.0) { vec3 w = ro + rd * hit; float top = smoothstep(-0.5, 0.0, w.y); col = mix(vec3(0.20, 0.19, 0.17), vec3(0.02), top); tEnd = hit; }",
    // The flame: glow added up along the ray.
    "  vec3 glow = vec3(0.0);",
    "  float t = 2.2; float dt = (min(tEnd, 6.6) - 2.2) / 46.0;",
    "  if (dt > 0.0) {",
    "    for (int i = 0; i < 46; i++) {",
    "      vec3 p = ro + rd * (t + dt * 0.5);",
    "      float d = flame(p, R);",
    "      float shell = exp(-pow(d / (0.06 * R + 0.02), 2.0));",
    "      float inside = smoothstep(0.02, -0.35 * R, d);",
    "      float halo = exp(-max(d, 0.0) * 5.0) * 0.05;",
    "      float hh = (p.y + R) / (R * 3.2);",
    "      vec3 earth = mix(vec3(0.16, 0.36, 1.0), vec3(1.0, 0.80, 0.38), smoothstep(0.08, 0.42, hh));",
    "      earth = mix(earth, vec3(1.0, 0.40, 0.07), smoothstep(0.62, 1.15, hh));",
    "      vec3 eE = earth * (inside * 1.7 + shell * 0.55 + halo);",
    "      vec3 eM = vec3(0.16, 0.36, 1.0) * (shell * 1.55 + inside * 0.10 + halo) + vec3(1.0, 0.48, 0.14) * exp(-dot(p, p) / 0.016) * 0.9;",
    "      glow += mix(eM, eE, uG) * dt;",
    "      t += dt;",
    "    }",
    "  }",
    "  glow *= mix(0.85, 1.35, uO) * 2.1;",
    "  col += vec3(1.0) - exp(-glow);",
    "  gl_FragColor = vec4(col, 1.0);",
    "}",
  ].join("\n");

  function compile(gl, type, source) {
    var shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    return gl.getShaderParameter(shader, gl.COMPILE_STATUS) ? shader : null;
  }

  function still() {
    return document.documentElement.classList.contains("motion-off");
  }

  function mount(canvas, start) {
    var gl = null;
    try { gl = canvas.getContext("webgl", { antialias: false, alpha: false, powerPreference: "low-power" }); } catch (e) { gl = null; }
    if (!gl) return null;
    var vs = compile(gl, gl.VERTEX_SHADER, VERTEX), fs = compile(gl, gl.FRAGMENT_SHADER, FRAGMENT);
    if (!vs || !fs) return null;
    var program = gl.createProgram();
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return null;
    gl.useProgram(program);
    var quad = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    var at = gl.getAttribLocation(program, "p");
    gl.enableVertexAttribArray(at);
    gl.vertexAttribPointer(at, 2, gl.FLOAT, false, 0, 0);
    var u = {};
    ["uRes", "uTime", "uG", "uO", "uYaw"].forEach(function (name) { u[name] = gl.getUniformLocation(program, name); });

    var state = { gravity: start && typeof start.gravity === "number" ? start.gravity : 0, oxygen: start && typeof start.oxygen === "number" ? start.oxygen : 0 };
    var yaw = 0.35, dragging = false, lastX = 0, visible = true, running = false, lost = false;
    var glideFrom = null, glideTo = null, glideStart = 0, glideMs = 0, onChange = start && start.onChange;
    var born = performance.now();

    function size() {
      // Drawn a little under the screen's sharpness: a soft glow hides it and phones stay cool.
      var ratio = Math.min(window.devicePixelRatio || 1, 1.5) * 0.8;
      var w = Math.max(2, Math.round(canvas.clientWidth * ratio)), h = Math.max(2, Math.round(canvas.clientHeight * ratio));
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
      gl.viewport(0, 0, w, h);
    }

    function paint(now) {
      if (lost) return;
      size();
      if (glideTo) {
        var k = Math.min(1, (now - glideStart) / glideMs);
        var e = k * k * (3 - 2 * k);
        ["gravity", "oxygen"].forEach(function (key) { if (typeof glideTo[key] === "number") state[key] = glideFrom[key] + (glideTo[key] - glideFrom[key]) * e; });
        if (k >= 1) glideTo = null;
        if (onChange) onChange(state);
      }
      if (!dragging && !still()) yaw += 0.0016;
      gl.uniform2f(u.uRes, canvas.width, canvas.height);
      gl.uniform1f(u.uTime, still() ? 1.7 : (now - born) / 1000);
      gl.uniform1f(u.uG, state.gravity);
      gl.uniform1f(u.uO, state.oxygen);
      gl.uniform1f(u.uYaw, yaw);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    }

    function loop(now) {
      running = false;
      paint(now);
      // With animations switched off the picture only changes when something is moved.
      if (visible && (!still() || glideTo || dragging)) wake();
    }
    function wake() {
      if (running || lost) return;
      running = true;
      requestAnimationFrame(loop);
    }

    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (entries) { visible = entries[0].isIntersecting; if (visible) wake(); }).observe(canvas);
    }
    window.addEventListener("resize", wake);
    canvas.addEventListener("webglcontextlost", function (e) { e.preventDefault(); lost = true; });
    canvas.addEventListener("webglcontextrestored", function () { lost = false; wake(); });
    canvas.addEventListener("pointerdown", function (e) { dragging = true; lastX = e.clientX; if (canvas.setPointerCapture) canvas.setPointerCapture(e.pointerId); wake(); });
    canvas.addEventListener("pointermove", function (e) { if (!dragging) return; yaw -= (e.clientX - lastX) * 0.008; lastX = e.clientX; wake(); });
    ["pointerup", "pointercancel"].forEach(function (name) { canvas.addEventListener(name, function () { dragging = false; }); });
    wake();

    return {
      get: function () { return { gravity: state.gravity, oxygen: state.oxygen }; },
      set: function (next) {
        glideTo = null;
        if (typeof next.gravity === "number") state.gravity = Math.max(0, Math.min(1, next.gravity));
        if (typeof next.oxygen === "number") state.oxygen = Math.max(0, Math.min(1, next.oxygen));
        if (onChange) onChange(state);
        wake();
      },
      // Moves smoothly to new values. With animations off it jumps.
      glide: function (next, ms) {
        if (still() || !ms) { this.set(next); return; }
        glideFrom = { gravity: state.gravity, oxygen: state.oxygen };
        glideTo = next;
        glideStart = performance.now();
        glideMs = ms;
        wake();
      },
    };
  }

  // The home page's chamber: the flame starts on Earth and is let go.
  function setUpChamber(root) {
    var canvas = root.querySelector("canvas");
    var slider = root.querySelector("[data-gravity]");
    var readG = root.querySelector("[data-read-gravity]");
    var readO = root.querySelector("[data-read-oxygen]");
    var caption = root.querySelector("[data-caption]");
    var oxygenButtons = Array.prototype.slice.call(root.querySelectorAll("[data-oxygen]"));
    var again = root.querySelector("[data-drop]");
    var flame = mount(canvas, {
      gravity: 1,
      oxygen: 0,
      onChange: function (s) {
        if (slider && document.activeElement !== slider) slider.value = String(Math.round(s.gravity * 100));
        if (readG) readG.textContent = s.gravity.toFixed(2) + " g";
        if (caption) caption.textContent = s.gravity > 0.5
          ? "On Earth, hot gas rises and pulls a flame into a teardrop."
          : "In freefall nothing rises, so the flame becomes a sphere.";
      },
    });
    if (!flame) {
      var view = root.querySelector(".chamber");
      if (view) view.appendChild(Object.assign(document.createElement("p"), { className: "chamber-fallback", textContent: "This browser can’t draw the 3D flame. On Earth a flame is a teardrop; in freefall it is a blue sphere." }));
      return;
    }
    function drop() { flame.set({ gravity: 1 }); setTimeout(function () { flame.glide({ gravity: 0 }, 2600); }, 700); }
    if (slider) slider.addEventListener("input", function () { flame.set({ gravity: Number(slider.value) / 100 }); });
    oxygenButtons.forEach(function (button) {
      button.addEventListener("click", function () {
        oxygenButtons.forEach(function (b) { b.setAttribute("aria-pressed", b === button ? "true" : "false"); });
        flame.glide({ oxygen: Number(button.getAttribute("data-oxygen")) }, 700);
        if (readO) readO.textContent = button.getAttribute("data-label");
      });
    });
    if (again) again.addEventListener("click", drop);
    flame.set({ gravity: 1, oxygen: 0 });
    drop();
  }

  window.Flame3D = { mount: mount };
  function start() { Array.prototype.forEach.call(document.querySelectorAll("[data-chamber]"), setUpChamber); }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start); else start();
})();
