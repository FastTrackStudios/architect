// architect-sites: a share link's phone shell (`architect_sites::shell`).
// On a phone, the tabs at the foot of the page as the app's phone shell
// draws them (a pill of tabs, a round button beside it), the player's
// card floating above them. Which tabs the node says
// (`<script type="application/json" id="site-shell">`): the favourites
// the share's root note's type declares, else Home, Search and Browse.
// The tab shown is in the address (`#tab=<id>`), so a reload or a shared
// link lands on it. Inert without that element; on a desk the page keeps
// its header and navigator (the bar is a phone's, in the sheet).
(function () {
  "use strict";
  var doc = document;
  var el = doc.querySelector('script#site-shell[type="application/json"]');
  if (!el) return;
  var spec;
  try { spec = JSON.parse(el.textContent || "null"); } catch (e) { return; }
  if (!spec || !Array.isArray(spec.tabs) || !spec.tabs.length) return;
  var arch = (window.architect = window.architect || {});
  if (arch.shell) return;
  var body = doc.body;
  var home = typeof spec.home === "string" ? spec.home : "/";
  var here = location.pathname;
  var player = function () { return arch.player || null; };

  // A tab: what it opens. Only what this page can: a player's view when
  // it has a player, search when it has search, the navigator when it has
  // one; the rest are left out.
  var search = doc.getElementById("site-search");
  var nav = doc.getElementById("site-nav");
  function usable(t) {
    var to = t && t.target;
    if (!to || typeof t.id !== "string" || !/^[a-z0-9.-]{1,96}$/.test(t.id)) return false;
    if (to.view === "player.now" || to.view === "player.lyrics") return !!player() && player().tracks > 0;
    if (to.view === "search") return !!search;
    if (to.view === "browse") return !!nav;
    if (to.view === "page") return true;
    if (typeof to.href === "string") return to.href.indexOf(home) === 0;
    return false;
  }
  var tabs = spec.tabs.filter(usable).slice(0, 5);
  if (!tabs.length) return;

  body.classList.add("site-shell");
  var bar = doc.createElement("nav");
  bar.className = "site-tabs";
  bar.setAttribute("aria-label", "Sections");
  var pill = doc.createElement("div");
  pill.className = "site-tabs-pill";
  bar.appendChild(pill);
  var buttons = {};
  tabs.forEach(function (t) {
    var b = doc.createElement("button");
    b.type = "button";
    b.className = "site-tab";
    b.setAttribute("data-tab", t.id);
    // Its icon: the node's, drawn from Lucide's shapes (static markup).
    b.innerHTML = String(t.icon || "");
    var label = doc.createElement("span");
    label.textContent = String(t.label || t.id);
    b.appendChild(label);
    b.addEventListener("click", function () { go(t.id); });
    pill.appendChild(b);
    buttons[t.id] = b;
  });

  // The round button: its first face, and its second while its state is
  // on (playing).
  var round = null, primary = spec.primary;
  if (primary && primary.target && primary.target.action === "PLAYER_TOGGLE" && arch.playhead && player() && player().tracks > 0) {
    round = doc.createElement("button");
    round.type = "button";
    round.className = "site-tabs-round";
    round.addEventListener("click", function () {
      var s = arch.playhead.snapshot();
      if (s.playing) arch.playhead.pause(); else arch.playhead.play();
    });
    bar.appendChild(round);
    var shown = null;
    var face = function (on) {
      if (shown === on) return;
      shown = on;
      var f = on && primary.on ? primary.on : primary;
      round.innerHTML = String(f.icon || "");
      round.setAttribute("aria-label", String(f.label || ""));
      round.setAttribute("title", String(f.label || ""));
    };
    arch.playhead.subscribe(function (s) { face(primary.state === "playing" && !!s.playing); });
    // The card's own play is the round button's: one of them is enough.
    body.classList.add("site-shell-plays");
  }
  body.appendChild(bar);

  var current = null;
  function mark(id) {
    current = id;
    Object.keys(buttons).forEach(function (k) {
      if (k === id) buttons[k].setAttribute("aria-current", "page");
      else buttons[k].removeAttribute("aria-current");
    });
  }
  function remember(id) {
    try { history.replaceState(history.state, "", here + location.search + "#tab=" + encodeURIComponent(id)); } catch (e) {}
  }
  function tabOf(view) {
    for (var i = 0; i < tabs.length; i++) if (tabs[i].target.view === view) return tabs[i];
    return null;
  }
  function closeSearch() {
    body.classList.remove("site-searching");
  }
  // Show tab `id`.
  function go(id) {
    var t = tabs.filter(function (x) { return x.id === id; })[0];
    if (!t) return;
    var to = t.target;
    if (typeof to.href === "string") {
      location.href = to.href;
      return;
    }
    if (to.view === "page") {
      closeSearch();
      if (here !== home) {
        location.href = home + "#tab=" + encodeURIComponent(id);
        return;
      }
      if (player() && player().open_) player().close();
      mark(id);
      remember(id);
      window.scrollTo(0, 0);
      return;
    }
    if (to.view === "player.now" || to.view === "player.lyrics") {
      closeSearch();
      mark(id);
      remember(id);
      player().open({ tab: true, lyrics: to.view === "player.lyrics" });
      return;
    }
    if (to.view === "search") {
      body.classList.add("site-searching");
      search.focus();
      return;
    }
    if (to.view === "browse") {
      closeSearch();
      body.classList.toggle("site-nav-open", !body.classList.contains("site-nav-open"));
    }
  }
  // The search field closes when it is left empty.
  if (search) {
    search.addEventListener("blur", function () {
      if (!search.value) setTimeout(closeSearch, 150);
    });
  }
  // While a field is being typed in, the bars step aside (the keyboard
  // would leave them in the middle of the screen).
  doc.addEventListener("focusin", function (e) {
    if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) body.classList.add("site-typing");
  });
  doc.addEventListener("focusout", function () { body.classList.remove("site-typing"); });

  arch.shell = {
    go: go,
    // The card was tapped: its song's view is the tab that shows it.
    now: function () {
      var t = tabOf("player.now");
      if (!t) return false;
      go(t.id);
      return true;
    },
    // The player's view closed (its chevron, Escape): back to the page.
    closed: function () {
      var t = tabOf("page");
      if (t && current !== t.id) {
        mark(t.id);
        remember(t.id);
      }
    },
  };

  // Where the address says, else the page's own tab when this is its page.
  var asked = /(?:^|[#&])tab=([a-z0-9.-]+)/.exec(location.hash || "");
  var first = asked && buttons[asked[1]] ? asked[1] : null;
  if (first && tabs.filter(function (x) { return x.id === first; })[0].target.view !== "page") go(first);
  else {
    var page = tabOf("page");
    if (page && here === home) mark(page.id);
  }
})();
