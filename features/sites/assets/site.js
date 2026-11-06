// architect-sites: what a reader does on a finished page. The page is
// complete without this; it adds search, the theme toggle, the phone
// menu and the contents rail's current heading.
(function () {
  "use strict";
  var doc = document.documentElement;

  // ── theme: the system's, until the reader picks ──────────────────────
  var themeButton = document.querySelector(".site-theme");
  if (themeButton) {
    themeButton.addEventListener("click", function () {
      var dark = doc.dataset.theme
        ? doc.dataset.theme === "dark"
        : matchMedia("(prefers-color-scheme: dark)").matches;
      var next = dark ? "light" : "dark";
      doc.dataset.theme = next;
      try { localStorage.setItem("site-theme", next); } catch (e) {}
    });
  }

  // ── the navigator, on a phone ─────────────────────────────────────────
  var menu = document.querySelector(".site-menu");
  function setNav(open) {
    document.body.classList.toggle("site-nav-open", open);
    if (menu) menu.setAttribute("aria-expanded", open ? "true" : "false");
  }
  if (menu) {
    menu.addEventListener("click", function () {
      setNav(!document.body.classList.contains("site-nav-open"));
    });
  }
  document.addEventListener("click", function (e) {
    if (e.target.closest("[data-site-close]")) setNav(false);
  });
  // The current page's row in view, scrolling only the navigator (not
  // the page, which `scrollIntoView` would also move).
  var nav = document.getElementById("site-nav");
  var current = nav && nav.querySelector(".is-active");
  if (current) {
    var offset = current.getBoundingClientRect().top - nav.getBoundingClientRect().top;
    if (offset > nav.clientHeight - 40) nav.scrollTop += offset - nav.clientHeight / 2;
  }

  // ── search ────────────────────────────────────────────────────────────
  // Task's guide search, kept: a title match beats a heading match beats
  // a text match, and reading order breaks ties.
  var input = document.getElementById("site-search");
  var results = document.getElementById("site-results");
  var indexUrl = (document.querySelector('meta[name="site-search"]') || {}).content;
  var index = null;
  var loading = null;
  var selected = -1;

  function load() {
    if (index || loading || !indexUrl) return loading;
    loading = fetch(indexUrl)
      .then(function (r) { return r.json(); })
      .then(function (data) { index = data; return data; });
    return loading;
  }

  function escapeHtml(s) {
    return s.replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  function mark(text, needle) {
    var lower = text.toLowerCase();
    var at = lower.indexOf(needle);
    if (at < 0) return escapeHtml(text);
    return escapeHtml(text.slice(0, at)) + "<mark>" + escapeHtml(text.slice(at, at + needle.length)) +
      "</mark>" + escapeHtml(text.slice(at + needle.length));
  }

  function excerpt(text, needle) {
    var at = text.toLowerCase().indexOf(needle);
    if (at < 0) return text.slice(0, 140);
    var start = Math.max(0, at - 50);
    var out = text.slice(start, at + needle.length + 90);
    return (start > 0 ? "…" : "") + out + (at + needle.length + 90 < text.length ? "…" : "");
  }

  function search(query) {
    var needle = query.trim().toLowerCase();
    if (needle.length < 2 || !index) return [];
    var hits = [];
    index.forEach(function (page, order) {
      if (page.t.toLowerCase().indexOf(needle) >= 0) {
        hits.push({ score: 300, order: order, url: page.u, title: page.t, where: "", text: page.d || "" });
        return;
      }
      var sections = page.s || [];
      for (var i = 0; i < sections.length; i++) {
        var s = sections[i];
        if (s[0] && s[1].toLowerCase().indexOf(needle) >= 0) {
          hits.push({ score: 200, order: order, url: page.u + "#" + s[0], title: page.t, where: s[1], text: excerpt(s[2], needle) });
          return;
        }
      }
      for (var j = 0; j < sections.length; j++) {
        var t = sections[j];
        if (t[2].toLowerCase().indexOf(needle) >= 0) {
          hits.push({
            score: 100, order: order, url: t[0] ? page.u + "#" + t[0] : page.u,
            title: page.t, where: t[1], text: excerpt(t[2], needle)
          });
          return;
        }
      }
    });
    hits.sort(function (a, b) { return b.score - a.score || a.order - b.order; });
    return hits.slice(0, 12).map(function (h) { h.needle = needle; return h; });
  }

  function draw(hits, query) {
    selected = -1;
    if (!query.trim()) { results.hidden = true; results.innerHTML = ""; return; }
    results.hidden = false;
    if (!hits.length) {
      results.innerHTML = '<div class="site-results-empty">Nothing matches “' + escapeHtml(query.trim()) + '”.</div>';
      return;
    }
    results.innerHTML = hits.map(function (h, i) {
      return '<a class="site-result" role="option" id="site-result-' + i + '" href="' + escapeHtml(h.url) + '">' +
        '<span class="site-result-title">' + mark(h.title, h.needle) +
        (h.where ? ' <span class="site-result-where">› ' + mark(h.where, h.needle) + "</span>" : "") + "</span>" +
        (h.text ? '<span class="site-result-text">' + mark(h.text, h.needle) + "</span>" : "") + "</a>";
    }).join("");
  }

  function select(step) {
    var items = results.querySelectorAll(".site-result");
    if (!items.length) return;
    if (selected >= 0 && items[selected]) items[selected].setAttribute("aria-selected", "false");
    selected = (selected + step + items.length) % items.length;
    items[selected].setAttribute("aria-selected", "true");
    items[selected].scrollIntoView({ block: "nearest" });
  }

  if (input && results) {
    input.addEventListener("focus", load);
    input.addEventListener("input", function () {
      var query = input.value;
      var run = function () { draw(search(query), query); };
      if (index) run(); else (load() || Promise.resolve()).then(run);
    });
    input.addEventListener("keydown", function (e) {
      if (e.key === "ArrowDown") { e.preventDefault(); select(1); }
      else if (e.key === "ArrowUp") { e.preventDefault(); select(-1); }
      else if (e.key === "Enter") {
        var items = results.querySelectorAll(".site-result");
        var go = items[selected >= 0 ? selected : 0];
        if (go) { e.preventDefault(); location.href = go.getAttribute("href"); }
      } else if (e.key === "Escape") { input.value = ""; draw([], ""); input.blur(); }
    });
    document.addEventListener("keydown", function (e) {
      var typing = /^(INPUT|TEXTAREA|SELECT)$/.test((e.target || {}).tagName || "");
      if (!typing && (e.key === "/" || ((e.metaKey || e.ctrlKey) && e.key === "k"))) {
        e.preventDefault();
        input.focus();
      }
    });
    document.addEventListener("click", function (e) {
      if (!e.target.closest(".site-search")) results.hidden = true;
    });
    // `?q=…` opens the page searching: a search is a link to share.
    var asked = new URLSearchParams(location.search).get("q");
    if (asked) {
      input.value = asked;
      (load() || Promise.resolve()).then(function () { draw(search(asked), asked); });
    }
  }

  // ── on this page: the heading being read ─────────────────────────────
  var links = Array.prototype.slice.call(document.querySelectorAll(".site-toc a"));
  if (links.length && "IntersectionObserver" in window) {
    var byId = {};
    links.forEach(function (a) { byId[decodeURIComponent(a.hash.slice(1))] = a; });
    var visible = {};
    var observer = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) { visible[entry.target.id] = entry.isIntersecting; });
      var first = links.find(function (a) { return visible[decodeURIComponent(a.hash.slice(1))]; });
      if (first) {
        links.forEach(function (a) { a.classList.toggle("is-current", a === first); });
      }
    }, { rootMargin: "-56px 0px -60% 0px" });
    Object.keys(byId).forEach(function (id) {
      var heading = document.getElementById(id);
      if (heading) observer.observe(heading);
    });
  }
})();
