// Architect landing page: theme toggle, sheet index, the site plan's rooms, looping details.
(function () {
  "use strict";
  var root = document.documentElement;
  root.classList.add("js");

  // Label each sheet's zones: 1-8 along the top, A-F down the left
  document.querySelectorAll(".sheet").forEach(function (sheet) {
    [["zones zones-top", "12345678"], ["zones zones-left", "ABCDEF"]].forEach(function (z) {
      var row = document.createElement("div");
      row.className = z[0];
      row.setAttribute("aria-hidden", "true");
      z[1].split("").forEach(function (c) { var s = document.createElement("span"); s.dataset.z = c; row.appendChild(s); });
      sheet.appendChild(row);
    });
  });
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  var darkPref = window.matchMedia("(prefers-color-scheme: dark)");

  // Plot (light) / Model (dark)
  var toggle = document.querySelector("[data-plot-toggle]");
  function isModel() {
    var t = root.dataset.theme;
    return t ? t === "model" : darkPref.matches;
  }
  function syncToggle() {
    if (toggle) toggle.setAttribute("aria-pressed", isModel() ? "true" : "false");
    var meta = document.querySelectorAll('meta[name="theme-color"]');
    meta.forEach(function (m) { m.setAttribute("content", isModel() ? "#0c0e10" : "#f3f5f2"); });
  }
  if (toggle) {
    toggle.addEventListener("click", function () {
      var next = isModel() ? "plot" : "model";
      root.dataset.theme = next;
      try { localStorage.setItem("architect-plot", next); } catch (e) {}
      syncToggle();
    });
  }
  darkPref.addEventListener("change", syncToggle);
  syncToggle();

  // Sheet index: disclosure on narrow screens, current sheet while scrolling
  var nav = document.querySelector(".bar-nav");
  var navBtn = nav && nav.querySelector(".bar-toggle");
  var current = document.querySelector("[data-current]");
  function closeNav() { if (nav) { nav.classList.remove("open"); navBtn.setAttribute("aria-expanded", "false"); } }
  if (navBtn) {
    navBtn.addEventListener("click", function () {
      var open = !nav.classList.contains("open");
      nav.classList.toggle("open", open);
      navBtn.setAttribute("aria-expanded", open ? "true" : "false");
    });
    nav.querySelectorAll(".sheet-list a").forEach(function (a) { a.addEventListener("click", closeNav); });
    document.addEventListener("keydown", function (e) { if (e.key === "Escape" && nav.classList.contains("open")) { closeNav(); navBtn.focus(); } });
    document.addEventListener("click", function (e) { if (!nav.contains(e.target)) closeNav(); });
  }
  var links = {};
  document.querySelectorAll(".sheet-list a").forEach(function (a) { links[a.getAttribute("href").slice(1)] = a; });
  if ("IntersectionObserver" in window) {
    var seen = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (!en.isIntersecting) return;
        var id = en.target.id;
        Object.keys(links).forEach(function (k) {
          if (k === id) links[k].setAttribute("aria-current", "true"); else links[k].removeAttribute("aria-current");
        });
        if (current && en.target.dataset.sheet) current.textContent = en.target.dataset.sheet;
      });
    }, { rootMargin: "-45% 0px -50% 0px" });
    document.querySelectorAll(".sheet[data-sheet]").forEach(function (s) { seen.observe(s); });
  }

  // The site plan: a room and its keynote light together; its leg of the line goes red
  var plan = document.querySelector(".plan-svg");
  var legs = {
    desk: [0], node: [0, 1, 2], phone: [1], web: [2], laptop: [3]
  };
  var paths = plan ? plan.querySelectorAll(".sync") : [];
  function light(room) {
    document.querySelectorAll("[data-room]").forEach(function (el) {
      el.classList.toggle("is-active", el.dataset.room === room);
    });
    paths.forEach(function (p, i) {
      p.classList.toggle("is-active", !!room && legs[room].indexOf(i) !== -1);
    });
  }
  document.querySelectorAll("[data-room]").forEach(function (el) {
    var room = el.dataset.room;
    el.addEventListener("pointerenter", function () { light(room); });
    el.addEventListener("pointerleave", function () { light(null); });
    el.addEventListener("focus", function () { light(room); });
    el.addEventListener("blur", function () { light(null); });
  });
  if (plan) {
    plan.querySelectorAll(".room").forEach(function (a) {
      a.addEventListener("click", function (e) {
        var id = a.getAttribute("href").slice(1);
        var kn = document.getElementById(id);
        if (kn && window.matchMedia("(min-width: 1241px)").matches) { e.preventDefault(); light(a.dataset.room); }
      });
    });
    requestAnimationFrame(function () { plan.classList.add("is-drawn"); });
    // On a phone the plan scrolls sideways: open it centred on the node
    var scroller = plan.parentNode;
    if (scroller && scroller.scrollWidth > scroller.clientWidth) {
      scroller.scrollLeft = (scroller.scrollWidth - scroller.clientWidth) / 2;
    }
  }

  // Looping details: play when on screen, never under reduced motion unless asked
  var videos = Array.prototype.slice.call(document.querySelectorAll("video[data-loop]"));
  function label(v) {
    var b = v.parentNode.querySelector("[data-loop-btn]");
    if (!b) return;
    b.textContent = v.paused ? "Play" : "Pause";
    b.setAttribute("aria-pressed", v.paused ? "false" : "true");
  }
  videos.forEach(function (v) {
    v.dataset.user = "";
    var b = v.parentNode.querySelector("[data-loop-btn]");
    if (b) b.addEventListener("click", function () {
      if (v.paused) { v.dataset.user = "play"; v.play().catch(function () {}); }
      else { v.dataset.user = "pause"; v.pause(); }
    });
    v.addEventListener("play", function () { label(v); });
    v.addEventListener("pause", function () { label(v); });
    label(v);
  });
  if ("IntersectionObserver" in window) {
    var vo = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        var v = en.target;
        if (en.isIntersecting) {
          if (v.dataset.user === "pause" || (reduce.matches && v.dataset.user !== "play")) return;
          v.preload = "auto";
          v.play().catch(function () {});
        } else if (!v.paused) {
          v.pause();
        }
      });
    }, { threshold: 0.35 });
    videos.forEach(function (v) { vo.observe(v); });
  }
})();
