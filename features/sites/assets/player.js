// architect-sites: the player (#511). Plays the page's audio and video
// (`[data-site-media]`), and on a share link the link's media in list
// order (`<script type="application/json" id="site-queue">`, written by
// the node per request; only that element, never a block's `#site-queue`),
// and gives every block on the page the playhead:
//
//   architect.playhead.time / playing / rate / duration / src / page / here
//   architect.playhead.seek(t[, {src}]), .play(), .pause(), .next(), .previous()
//   architect.playhead.subscribe(fn) -> unsubscribe   (fn(snapshot), at once and on change)
//   document "architect:playhead" event, detail = the same snapshot
//   architect.playhead.attach(source) -> { from, changed(), detach() }
//
// An outside clock (an audio engine running on the page, #528) attaches
// as the playhead's source: its `time`, `playing`, `rate`, `duration`
// (values, or functions read each frame while it plays) are the
// playhead, `seek(t)`, `play()`, `pause()` are asked of it, and `src`
// says which media it stands in for. While it is attached the page's
// own player yields (its media paused, its bar hidden); detached, the
// page's player takes over where the source was.
//
// A block needs no script to follow it: a `[data-follow-playhead]`
// element's `[data-at]` children (seconds) are marked aria-current="time"
// at the playhead, scrolled to inside the nearest box that scrolls them,
// and seek there on a click. They may be HTML or SVG elements (a chart's
// `<rect data-at>` per bar, keyflow#59).
//
// A playlist (`[data-playlist]`, a block's list of songs) makes the page
// a player: its tracks (`[data-track][data-media]`, each with
// `data-title`, `data-subtitle` and `data-page`, the page that shows it
// playing) are the queue, every playlist of the page one after the other
// in document order (a setlist's sets play through); the page's own
// `<audio>`/`<video>` are not queued there (one played by hand plays
// alone). A tap on a row plays it and the page stays where it is; a
// `[data-playlist-play]` in it plays it from the top; the track playing
// is marked aria-current="true". Songs follow each other without a page
// load. The bar's song opens the now-playing view: a position bar at the
// top, the playing track's `[data-now-playing]` element, the controls
// under it (a view bringing its own: `[data-progress]`, `[data-transport]`
// in it), its lyrics a tap away and its page linked. Back, a swipe down or
// Escape closes it; what plays goes on.
//
// A view from another page is fetched only from a page this link (or
// site) built: a URL under the site's own mount, ending in `/`, no `_`
// segment, no query; answered 200 as `text/html` with the node's
// `X-Architect-Page: built`, which no vault file is served with. What
// comes in goes through an allow-list of elements and attributes (the
// builder's own, `sanitize.rs`: no script, style outside an SVG, meta,
// base, link, form, frame, handler). Without a view the bar's song is a
// link to its page, as it always was.
//
// Pages are separate documents: the player keeps where it is (track,
// time, playing) in sessionStorage and resumes on the next page; a
// browser that refuses to start sound without a click leaves it paused
// at that time. Inert on a page with no media and no queue.
(function () {
  "use strict";
  var doc = document;
  var arch = (window.architect = window.architect || {});
  if (arch.playhead) return;

  function path(url, from) {
    try {
      var base = (from && from.getAttribute && closestBase(from)) || location.href;
      var p = new URL(url, base).pathname;
      return p.replace(/index\.html$/, "");
    } catch (e) {
      return String(url);
    }
  }
  // A fetched page's elements keep the page they came from: their
  // relative URLs are read from there.
  function closestBase(node) {
    var n = node.closest ? node.closest("[data-site-from]") : null;
    return n ? n.getAttribute("data-site-from") : null;
  }
  var here = path(location.href);

  // What the node offers a share page (`architect_sites::island`,
  // studio#445): its web runtime, its sandbox document, its islands. Only
  // the node's own `/_ext/` paths.
  var ext = null;
  (function () {
    var n = doc.querySelector('script#site-islands[type="application/json"]');
    if (!n) return;
    try {
      var x = JSON.parse(n.textContent || "null");
      var own = function (p) { return typeof p === "string" && /^\/_ext\/[A-Za-z0-9._\/-]+$/.test(p) && p.indexOf("..") < 0; };
      if (!x || !own(x.runtime) || !own(x.sandbox)) return;
      x.islands = (Array.isArray(x.islands) ? x.islands : []).filter(function (i) { return i && own(i.folder); });
      ext = x;
    } catch (e) {
      ext = null;
    }
  })();

  // Where the site is mounted: the folder above its `_site/` assets (this
  // script is one), else the link's scope.
  function normal(p) {
    var segs = String(p || "/").split("/").filter(Boolean).map(function (x) {
      try { return decodeURIComponent(x); } catch (e) { return null; }
    });
    if (segs.some(function (x) { return x === null || x === "." || x === ".." || /[\\\/]/.test(x); })) return null;
    // Case kept: the node's paths are (`/S/…` is not `/s/…`).
    return "/" + segs.join("/") + (segs.length ? "/" : "");
  }
  var siteRoot = null;
  (function () {
    var me = doc.currentScript && doc.currentScript.src;
    var at = me ? path(me).indexOf("/_site/") : -1;
    if (at >= 0) siteRoot = normal(path(me).slice(0, at + 1));
  })();
  // `raw` (a row's `data-page`) as a page this site built, or null.
  function builtPage(raw) {
    if (!raw) return null;
    var u;
    try { u = new URL(raw, location.href); } catch (e) { return null; }
    if (u.origin !== location.origin || u.search || u.username || u.password) return null;
    if (u.pathname.slice(-1) !== "/") return null;
    var n = normal(u.pathname);
    var root = siteRoot || normal(scope);
    if (!n || !root || n.indexOf(root) !== 0) return null;
    if (n.split("/").some(function (x) { return x.charAt(0) === "_"; })) return null;
    return u.pathname;
  }

  // The queue: the link's, else this page's media in document order.
  var tracks = [];
  var queueEl = doc.querySelector('script#site-queue[type="application/json"]');
  var scope = here;
  if (queueEl) {
    try {
      var q = JSON.parse(queueEl.textContent || "{}");
      tracks = (q.tracks || []).map(function (t) {
        return { src: path(t.src), page: path(t.page || here), title: t.title || "", kind: t.kind || "audio" };
      });
      scope = q.scope || scope;
    } catch (e) {
      tracks = [];
    }
  }
  var inline = Array.prototype.slice.call(doc.querySelectorAll("[data-site-media]"));
  inline.forEach(function (el) {
    var src = path(el.getAttribute("src") || "");
    if (!tracks.some(function (t) { return t.src === src; })) {
      tracks.push({ src: src, page: here, title: el.textContent.trim() || src.split("/").pop(), kind: el.tagName.toLowerCase() });
    }
  });
  // The page's playlists: their tracks are the queue, the page stays.
  var rows = [];
  var listed = [];
  Array.prototype.forEach.call(doc.querySelectorAll("[data-playlist]"), function (list) {
    Array.prototype.forEach.call(list.querySelectorAll("[data-track][data-media]"), function (row) {
      var src = path(row.getAttribute("data-media"));
      rows.push({ el: row, src: src });
      if (listed.some(function (t) { return t.src === src; })) return;
      var page = builtPage(row.getAttribute("data-page"));
      listed.push({
        src: src,
        page: page,
        view: !!page,
        title: row.getAttribute("data-title") || row.textContent.trim(),
        subtitle: row.getAttribute("data-subtitle") || "",
        set: list.getAttribute("data-title") || list.getAttribute("aria-label") || "",
        kind: "audio",
      });
    });
  });
  var shell = listed.length > 0;
  if (shell) tracks = listed;
  var followers = doc.querySelectorAll("[data-follow-playhead]");
  if (!tracks.length && !followers.length) return;

  var KEY = "architect.player:" + scope;
  function load() {
    try {
      return JSON.parse(sessionStorage.getItem(KEY) || "null") || {};
    } catch (e) {
      return {};
    }
  }
  function store(state) {
    try {
      sessionStorage.setItem(KEY, JSON.stringify(state));
    } catch (e) {}
  }

  var saved = load();
  var follow = saved.follow !== false;
  var cur = -1;
  var el = null;
  // What the bar says over the song's subtitle until it plays again.
  var noteText = null;
  var own = null;
  var listeners = [];
  // The outside clock attached, if any: { of: the source }.
  var source = null;

  function inlineFor(src) {
    for (var i = 0; i < inline.length; i++) {
      if (path(inline[i].getAttribute("src") || "") === src) return inline[i];
    }
    return null;
  }
  function indexOf(src) {
    for (var i = 0; i < tracks.length; i++) if (tracks[i].src === src) return i;
    return -1;
  }
  function ownElement() {
    if (!own) {
      own = doc.createElement("audio");
      own.preload = "metadata";
      own.className = "site-player-audio";
      wire(own);
      (bar || doc.body).appendChild(own);
    }
    return own;
  }

  function read(s, key, fallback) {
    var v = s[key];
    if (typeof v === "function") {
      try { v = v.call(s); } catch (e) { console.error(e); v = undefined; }
    }
    return v === undefined || v === null ? fallback : v;
  }
  function snapshot() {
    if (source) {
      var s = source.of;
      var src = s.src ? path(s.src) : null;
      var d = Number(read(s, "duration", NaN));
      var at = Number(read(s, "time", 0));
      return {
        time: isFinite(at) ? Math.max(0, at) : 0,
        playing: !!read(s, "playing", false),
        rate: Number(read(s, "rate", 1)) || 1,
        duration: isFinite(d) ? d : null,
        src: src,
        page: here,
        here: true,
        index: src ? indexOf(src) : -1,
        tracks: tracks.length,
        source: String(s.name || "source"),
      };
    }
    var t = tracks[cur];
    return {
      time: el ? el.currentTime || 0 : 0,
      playing: !!(el && !el.paused && !el.ended),
      rate: el ? el.playbackRate : 1,
      duration: el && isFinite(el.duration) ? el.duration : null,
      src: t ? t.src : el ? path(el.getAttribute("src") || "") : null,
      page: t ? t.page : null,
      here: t ? t.page === here : !!el,
      index: cur,
      tracks: tracks.length,
      source: null,
    };
  }
  var last = "";
  function emit(force) {
    var s = snapshot();
    var key = [s.time.toFixed(2), s.playing, s.rate, s.src, s.duration].join("|");
    if (!force && key === last) return;
    last = key;
    listeners.slice().forEach(function (fn) {
      try { fn(s); } catch (e) { console.error(e); }
    });
    doc.dispatchEvent(new CustomEvent("architect:playhead", { detail: s }));
    follow_(s);
    paint(s);
  }
  var saving = 0;
  function save(force) {
    var now = Date.now();
    if (!force && now - saving < 1000) return;
    saving = now;
    var s = snapshot();
    // A source standing in for no track of the page keeps what was kept.
    if (source && !s.src) return;
    store({ src: s.src, time: s.time, playing: s.playing, follow: follow });
  }

  var frame = 0;
  function tick() {
    frame = 0;
    emit(false);
    if (source ? snapshot().playing : el && !el.paused) frame = requestAnimationFrame(tick);
  }
  function kick() {
    if (!frame && snapshot().playing) frame = requestAnimationFrame(tick);
  }
  function wire(media) {
    ["play", "pause", "seeked", "ratechange", "durationchange", "timeupdate", "loadedmetadata"].forEach(function (name) {
      media.addEventListener(name, function () {
        if (source) {
          // A source holds the clock: the page's media stays quiet.
          if (name === "play") media.pause();
          return;
        }
        if (media !== el) {
          // A player on the page started by hand: it is the current one.
          if (name === "play") adopt(media);
          return;
        }
        if (name === "play") noteText = null;
        emit(name !== "timeupdate");
        save(name !== "timeupdate");
        if (name === "play" && !frame) frame = requestAnimationFrame(tick);
      });
    });
    media.addEventListener("ended", function () {
      if (media === el && !source && cur >= 0) next(true);
    });
    // A file the node refuses (a link revoked, a layer above the
    // audience's) or that does not play: said in the bar, and the queue
    // stops there.
    media.addEventListener("error", function () {
      if (media !== el || source || !media.getAttribute("src")) return;
      noteText = "This could not be played (the link may have changed)";
      emit(true);
    });
  }
  inline.forEach(wire);

  function adopt(media) {
    var src = path(media.getAttribute("src") || "");
    if (el && el !== media) el.pause();
    el = media;
    cur = indexOf(src);
    emit(true);
    save(true);
  }

  // Make track `i` current, at `time`; play it when `play`. A track of
  // another page, with follow on, is played there: the player goes to
  // that page and resumes.
  function select(i, time, play, stay) {
    var t = tracks[i];
    if (!t) return;
    if (i !== cur) noteText = null;
    var media = inlineFor(t.src);
    if (!media && follow && !stay && !shell && t.page !== here) {
      store({ src: t.src, time: time || 0, playing: play, follow: follow });
      location.href = t.page;
      return;
    }
    if (!media) {
      media = ownElement();
      if (path(media.getAttribute("src") || "") !== t.src) media.setAttribute("src", t.src);
    }
    if (el && el !== media) el.pause();
    el = media;
    cur = i;
    var start = function () {
      if (time) el.currentTime = time;
      if (play) {
        var p = el.play();
        if (p && p.catch) p.catch(function () {
          if (noteText) return;
          noteText = "Press play to go on";
          emit(true);
        });
      }
      emit(true);
    };
    if (el.readyState >= 1) start();
    else el.addEventListener("loadedmetadata", start, { once: true });
    if (el.readyState < 1 && el.preload === "none") el.load();
    emit(true);
    save(true);
  }

  function next(auto) {
    if (cur + 1 < tracks.length) select(cur + 1, 0, true, shell);
    else if (auto) emit(true);
  }
  function previous() {
    if (el && el.currentTime > 3) {
      el.currentTime = 0;
      return;
    }
    if (cur > 0) select(cur - 1, 0, el ? !el.paused : true, shell);
    else if (el) el.currentTime = 0;
  }
  function firstHere() {
    for (var i = 0; i < tracks.length; i++) if (tracks[i].page === here) return i;
    return tracks.length ? 0 : -1;
  }

  // Ask the source to `what` (seek, play, pause), then show where it is.
  function ask(what, arg) {
    var s = source.of;
    var out;
    if (typeof s[what] === "function") {
      try { out = s[what](arg); } catch (e) { console.error(e); }
    }
    emit(true);
    kick();
    return out;
  }

  var playhead = {
    get time() { return snapshot().time; },
    get playing() { return snapshot().playing; },
    get rate() { return snapshot().rate; },
    get duration() { return snapshot().duration; },
    get src() { return snapshot().src; },
    get page() { return snapshot().page; },
    get here() { return snapshot().here; },
    snapshot: snapshot,
    seek: function (t, opts) {
      if (source) return ask("seek", Math.max(0, Number(t) || 0));
      var src = opts && opts.src ? path(opts.src) : null;
      // A playlist's page plays its tracks where it is: a seek with no
      // track named is in the one playing.
      var want = src ? indexOf(src) : cur >= 0 && (shell || tracks[cur].page === here) ? cur : firstHere();
      if (want < 0) return;
      if (want !== cur || !el) select(want, t, opts && opts.play !== undefined ? opts.play : true, true);
      else {
        el.currentTime = Math.max(0, t);
        emit(true);
      }
    },
    play: function () {
      if (source) return ask("play");
      if (el) return el.play();
      if (tracks.length) select(firstHere(), 0, true, true);
    },
    pause: function () {
      if (source) return ask("pause");
      if (el) el.pause();
    },
    next: function () { if (!source) next(false); },
    previous: function () { if (!source) previous(); },
    // An outside clock as the playhead's source (#528): see the top.
    attach: function (spec) {
      if (!spec || spec.time === undefined || typeof spec.seek !== "function") {
        throw new TypeError("a playhead source has a time and a seek(t)");
      }
      var from = snapshot();
      if (source) {
        var old = source.of;
        source = null;
        if (typeof old.detached === "function") {
          try { old.detached(); } catch (e) { console.error(e); }
        }
      }
      if (el && !el.paused) el.pause();
      var mine = { of: spec };
      source = mine;
      yieldBar(true);
      emit(true);
      kick();
      return {
        from: { src: from.src, time: from.time, playing: from.playing },
        changed: function () {
          if (source !== mine) return;
          emit(true);
          save(true);
          kick();
        },
        detach: function () {
          if (source !== mine) return;
          var last = snapshot();
          source = null;
          yieldBar(false);
          var i = last.src ? indexOf(last.src) : -1;
          if (i >= 0) select(i, last.time, false, true);
          else emit(true);
        },
      };
    },
    subscribe: function (fn) {
      listeners.push(fn);
      try { fn(snapshot()); } catch (e) { console.error(e); }
      return function () {
        listeners = listeners.filter(function (f) { return f !== fn; });
      };
    },
  };
  arch.playhead = playhead;

  // Blocks that follow without a script: the page's, and the now-playing
  // view's while it shows another page's.
  function line(box) {
    var items = Array.prototype.slice.call(box.querySelectorAll("[data-at]"));
    var media = box.getAttribute("data-media");
    box.addEventListener("click", function (ev) {
      var item = ev.target.closest("[data-at]");
      if (!item || !box.contains(item)) return;
      var at = parseFloat(item.getAttribute("data-at"));
      if (isFinite(at)) playhead.seek(at, media ? { src: media } : undefined);
    });
    return {
      box: box,
      media: media ? path(media, box) : null,
      items: items,
      at: items.map(function (i) { return parseFloat(i.getAttribute("data-at")) || 0; }),
      current: null,
    };
  }
  var lines = Array.prototype.map.call(followers, line);
  function follows(root, on) {
    var boxes = Array.prototype.slice.call(root.querySelectorAll("[data-follow-playhead]"));
    // The root itself, when it is one (a lyrics list brought in whole).
    if (root.hasAttribute && root.hasAttribute("data-follow-playhead")) boxes.unshift(root);
    lines = lines.filter(function (f) { return boxes.indexOf(f.box) < 0; });
    if (on) lines = lines.concat(boxes.map(line));
    emit(true);
  }
  function follow_(s) {
    lines.forEach(function (f) {
      var mine = f.media ? s.src === f.media : s.here;
      var found = null;
      if (mine) {
        // The last line that has started.
        var lo = 0, hi = f.at.length - 1, best = -1;
        while (lo <= hi) {
          var mid = (lo + hi) >> 1;
          if (f.at[mid] <= s.time + 0.05) { best = mid; lo = mid + 1; } else hi = mid - 1;
        }
        found = best >= 0 ? f.items[best] : null;
      }
      if (found === f.current) return;
      if (f.current) f.current.removeAttribute("aria-current");
      f.current = found;
      if (!found) return;
      found.setAttribute("aria-current", "time");
      intoView(found);
    });
  }

  // The nearest box that scrolls `el`, below the page itself: a lyrics
  // list, or whatever holds an SVG (an SVG is not a scroll box).
  function scroller(el) {
    for (var n = el.parentElement; n && n !== doc.body && n !== doc.documentElement; n = n.parentElement) {
      var st = window.getComputedStyle(n);
      var y = /auto|scroll/.test(st.overflowY) && n.scrollHeight > n.clientHeight;
      var x = /auto|scroll/.test(st.overflowX) && n.scrollWidth > n.clientWidth;
      if (x || y) return n;
    }
    return null;
  }
  // `el` (HTML or SVG: both have a bounding rect, an SVG element has no
  // offsetTop) centred in its scroll box, never moving the page.
  function intoView(el) {
    var box = scroller(el);
    if (!box) return;
    var r = el.getBoundingClientRect(), b = box.getBoundingClientRect();
    // A third of the way down, as Session's chart follows: what comes
    // next under it, and a chart's first lines (its title) at the top.
    var top = box.scrollTop + (r.top - b.top) - box.clientHeight / 3;
    var left = box.scrollLeft + (r.left - b.left) - (box.clientWidth - r.width) / 2;
    box.scrollTo({
      top: Math.max(0, top),
      left: Math.max(0, left),
      behavior: reduced() ? "auto" : "smooth",
    });
  }
  function reduced() {
    return !!(window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  }

  // The bar: shown for a link's queue, a playlist, or two players or more
  // on a page. Its song is a link to the song's page, or opens the
  // now-playing view when there is one to show.
  var ICON = {
    previous: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 5h2v14H6zM20 5v14L9 12z"/></svg>',
    next: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M16 5h2v14h-2zM4 5v14l11-7z"/></svg>',
    play: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 4.5v15L20 12z"/></svg>',
    pause: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 4.5h4v15H6zM14 4.5h4v15h-4z"/></svg>',
    down: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5.3 8.3 12 15l6.7-6.7 1.4 1.4L12 17.8 3.9 9.7z"/></svg>',
  };
  function controls() {
    return '<button type="button" class="site-player-skip" data-act="previous" aria-label="Previous">' + ICON.previous + "</button>" +
      '<button type="button" class="site-player-toggle" data-act="toggle" aria-label="Play">' + ICON.play + "</button>" +
      '<button type="button" class="site-player-skip" data-act="next" aria-label="Next">' + ICON.next + "</button>";
  }
  function act(a) {
    if (a === "previous") previous();
    else if (a === "next") next(false);
    else if (a === "close") close_();
    else if (a === "lyrics") lyrics_();
    else if (snapshot().playing) playhead.pause();
    else playhead.play();
  }
  var bar = null, ui = {};
  if (shell || (queueEl ? tracks.length > 0 : inline.length > 1)) {
    bar = doc.createElement("div");
    bar.className = "site-player";
    bar.setAttribute("role", "region");
    bar.setAttribute("aria-label", "Player");
    bar.innerHTML =
      '<div class="site-player-line" aria-hidden="true"><span></span></div>' +
      '<span class="site-player-what"><a class="site-player-title"></a><span class="site-player-note"></span></span>' +
      '<span class="site-player-time">0:00</span>' +
      controls() +
      (shell ? "" : '<label class="site-player-follow" title="Go to each song\'s page as it plays"><input type="checkbox"> Follow</label>');
    ui.line = bar.querySelector(".site-player-line span");
    ui.title = bar.querySelector(".site-player-title");
    ui.note = bar.querySelector(".site-player-note");
    ui.time = bar.querySelector(".site-player-time");
    ui.follow = bar.querySelector(".site-player-follow input");
    if (ui.follow) {
      ui.follow.checked = follow;
      ui.follow.addEventListener("change", function () {
        follow = ui.follow.checked;
        save(true);
      });
    }
    bar.addEventListener("click", function (ev) {
      var title = ev.target.closest(".site-player-title");
      if (title && hasView(tracks[cur])) {
        // The song's view, not its page.
        ev.preventDefault();
        open_();
        return;
      }
      var a = ev.target.closest("[data-act]");
      if (a) act(a.getAttribute("data-act"));
    });
    doc.body.appendChild(bar);
    doc.body.classList.add("site-has-player");
  }

  // A playlist's rows: a tap plays the row's track where the page is (its
  // links still go where they say); the playing row is marked.
  rows.forEach(function (r) {
    r.el.addEventListener("click", function (ev) {
      // Its links go where they say; a line of a follow box in it seeks
      // there itself (a row written for a page without playlists).
      if (ev.target.closest("a, [data-playlist-play]")) return;
      var line_ = ev.target.closest("[data-at]");
      if (line_ && line_.closest("[data-follow-playhead]") && r.el.contains(line_)) return;
      var i = indexOf(r.src);
      if (i < 0) return;
      if (i === cur && el) act("toggle");
      else select(i, 0, true, true);
    });
  });
  Array.prototype.forEach.call(doc.querySelectorAll("[data-playlist] [data-playlist-play]"), function (b) {
    b.addEventListener("click", function () {
      var first = b.closest("[data-playlist]").querySelector("[data-track][data-media]");
      var i = first ? indexOf(path(first.getAttribute("data-media"))) : -1;
      if (i >= 0) select(i, 0, true, true);
    });
  });

  // ── The now-playing view ──────────────────────────────────────────
  // The playing track's `[data-now-playing]`, in a sheet over the page
  // (a panel beside it from 900 px): from top to bottom where it is (a
  // bar to seek on), the view, the controls, then its lyrics and page.
  // One from this page is moved in with its block's box (which scopes its
  // sheet) and back out on close; one from another page is fetched once,
  // made safe, and kept.
  var sheet = null, view = { src: null, node: null, home: null, lyrics: null };
  // The track whose view is shown or on its way.
  var wanted = null;
  // Each page fetched: a promise of its parsed document (failures are not
  // kept: the next open asks again).
  var fetched = {};
  var opener = null;
  var wide = window.matchMedia ? window.matchMedia("(min-width: 900px)") : { matches: false };
  function hasView(t) {
    return !!(sheet && t && (t.view || viewIn(doc, t.src)));
  }
  if (bar && shell) {
    sheet = doc.createElement("div");
    sheet.className = "site-now";
    sheet.hidden = true;
    sheet.setAttribute("role", "dialog");
    sheet.setAttribute("aria-label", "Now playing");
    sheet.innerHTML =
      '<div class="site-now-head"><button type="button" class="site-now-close" data-act="close" aria-label="Close">' + ICON.down + "</button>" +
      '<div class="site-now-what"><span class="site-now-title"></span><span class="site-now-note"></span></div><span class="site-now-spacer"></span></div>' +
      '<div class="site-now-island site-now-island-top" hidden></div>' +
      '<div class="site-now-progress"><span class="site-now-at">0:00</span><div class="site-now-seek" role="slider" tabindex="0" aria-label="Position" aria-valuemin="0"><span></span></div><span class="site-now-left">–:––</span></div>' +
      '<div class="site-now-body"></div>' +
      '<div class="site-now-island site-now-island-bottom" hidden></div>' +
      '<div class="site-now-controls"><div class="site-now-buttons">' + controls() + "</div></div>" +
      '<div class="site-now-more"><button type="button" class="site-now-lyrics" data-act="lyrics" aria-pressed="false" hidden>Lyrics</button><a class="site-now-page">Song page and downloads</a></div>';
    ui.nowTitle = sheet.querySelector(".site-now-title");
    ui.nowNote = sheet.querySelector(".site-now-note");
    ui.nowPage = sheet.querySelector(".site-now-page");
    ui.body = sheet.querySelector(".site-now-body");
    ui.progress = sheet.querySelector(".site-now-progress");
    ui.islandTop = sheet.querySelector(".site-now-island-top");
    ui.islandBottom = sheet.querySelector(".site-now-island-bottom");
    ui.seek = sheet.querySelector(".site-now-seek");
    ui.seekFill = sheet.querySelector(".site-now-seek span");
    ui.controls = sheet.querySelector(".site-now-controls");
    ui.nowAt = sheet.querySelector(".site-now-at");
    ui.nowLeft = sheet.querySelector(".site-now-left");
    ui.lyrics = sheet.querySelector(".site-now-lyrics");
    ui.close = sheet.querySelector(".site-now-close");
    sheet.addEventListener("click", function (ev) {
      var a = ev.target.closest("[data-act]");
      if (a && !ev.target.closest(".site-now-body")) act(a.getAttribute("data-act"));
    });
    // The position bar: a tap seeks there; arrows step 5 s.
    ui.seek.addEventListener("click", function (ev) {
      var s = snapshot();
      var r = ui.seek.getBoundingClientRect();
      if (!s.duration || !r.width) return;
      playhead.seek(Math.max(0, Math.min(1, (ev.clientX - r.left) / r.width)) * s.duration);
    });
    ui.seek.addEventListener("keydown", function (ev) {
      var step = ev.key === "ArrowRight" ? 5 : ev.key === "ArrowLeft" ? -5 : 0;
      if (!step) return;
      ev.preventDefault();
      playhead.seek(Math.max(0, snapshot().time + step));
    });
    // A swipe down on its head closes it (its head takes no vertical
    // scrolling, the view does not hand its overscroll to the page).
    var startY = null;
    var head = sheet.querySelector(".site-now-head");
    head.addEventListener("touchstart", function (ev) { startY = ev.touches[0].clientY; }, { passive: true });
    head.addEventListener("touchend", function (ev) {
      if (startY !== null && ev.changedTouches[0].clientY - startY > 60) close_();
      startY = null;
    });
    doc.addEventListener("keydown", function (ev) {
      if (sheet.hidden) return;
      if (ev.key === "Escape") close_();
      else if (ev.key === "Tab" && modal()) trap(ev);
    });
    window.addEventListener("popstate", function () {
      if (!sheet.hidden && !(history.state && history.state.siteNowPlaying)) close_(true);
    });
    doc.body.appendChild(sheet);
  }
  // Modal (the page behind made inert) where the view covers the page.
  function modal() {
    return !wide.matches;
  }
  function trap(ev) {
    var all = Array.prototype.filter.call(
      sheet.querySelectorAll('a[href], button, [tabindex="0"], [data-at]'),
      function (n) { return !n.hidden && n.offsetParent !== null; }
    );
    if (!all.length) return;
    var first = all[0], lastOne = all[all.length - 1];
    if (ev.shiftKey && doc.activeElement === first) { ev.preventDefault(); lastOne.focus(); }
    else if (!ev.shiftKey && doc.activeElement === lastOne) { ev.preventDefault(); first.focus(); }
  }
  function inertBehind(on) {
    Array.prototype.forEach.call(doc.body.children, function (n) {
      if (n === sheet) return;
      if (on) n.setAttribute("inert", "");
      else n.removeAttribute("inert");
    });
  }

  // What a view brings in from another page goes through the share
  // pages' own allow-lists again, in this page: `sanitize.js` (the port of
  // `sanitize.rs` every live view is held to, one source), which the
  // site's own script carries as `architect.rules` on every Site and
  // share link. Without it no view is brought in.
  function rules() {
    return Promise.resolve(arch.rules || null);
  }
  // A block's sheet, kept only when every rule is under its box's class
  // (as the builder scoped it) and is inert.
  function scopedSheet(r, text, scope) {
    var css = String(text).replace(/\/\*[\s\S]*?\*\//g, "");
    if (!scope || !r.cssInert(css)) return false;
    return css.split("}").every(function (rule) {
      rule = rule.trim();
      if (!rule) return true;
      var at = rule.indexOf("{");
      if (at < 0) return false;
      return rule.slice(0, at).split(",").every(function (sel) {
        sel = sel.trim();
        return sel.indexOf("." + scope + " ") === 0 || sel === "." + scope;
      });
    });
  }
  function clean(r, node, inSvg, scope) {
    Array.prototype.slice.call(node.children).forEach(function (child) {
      var tag = child.localName;
      var svg = inSvg || tag.toLowerCase() === "svg";
      // An SVG's sheet, as share pages keep it (a live view keeps none):
      // only under its block's box class, inert, and nothing but its text.
      if (tag.toLowerCase() === "style") {
        if (inSvg && scopedSheet(r, child.textContent, scope)) {
          Array.prototype.slice.call(child.attributes).forEach(function (at) { child.removeAttribute(at.name); });
          child.textContent = child.textContent;
        } else child.parentNode.removeChild(child);
        return;
      }
      var rule = r.elementRule(tag, inSvg, false);
      if (rule[0] === "drop") {
        child.parentNode.removeChild(child);
        return;
      }
      if (rule[0] === "unwrap") {
        var plain = doc.createElementNS(inSvg ? "http://www.w3.org/2000/svg" : "http://www.w3.org/1999/xhtml", inSvg ? "g" : "span");
        while (child.firstChild) plain.appendChild(child.firstChild);
        child.parentNode.replaceChild(plain, child);
        clean(r, plain, inSvg, scope);
        return;
      }
      Array.prototype.slice.call(child.attributes).forEach(function (at) {
        var kept = r.attrRule(tag, at.name, at.value, svg && tag.toLowerCase() !== "svg" ? true : svg, false, { prefix: "", urls: [] });
        if (kept === null) child.removeAttribute(at.name);
        else if (kept !== at.value) child.setAttribute(at.name, kept);
      });
      clean(r, child, svg, scope);
    });
    return node;
  }
  function scopeOf(box) {
    var m = /(?:^|\s)(site-block-[0-9a-f]{16})(?:\s|$)/.exec(box.getAttribute("class") || "");
    return m ? m[1] : null;
  }

  // The view on `root` whose media is `src`, else its only one.
  function viewIn(root, src, from) {
    var all = Array.prototype.slice.call(root.querySelectorAll("[data-now-playing]"));
    for (var i = 0; i < all.length; i++) {
      var m = all[i].getAttribute("data-media");
      if (m && pathFrom(m, from) === src) return all[i];
    }
    return all.length === 1 && !all[0].getAttribute("data-media") ? all[0] : null;
  }
  function pathFrom(url, from) {
    try {
      return new URL(url, from || location.href).pathname.replace(/index\.html$/, "");
    } catch (e) {
      return null;
    }
  }
  // The lyrics on `root` that follow `src` (or its page's media).
  function lyricsIn(root, src, from) {
    var all = Array.prototype.slice.call(root.querySelectorAll(".site-lyrics[data-follow-playhead]"));
    for (var i = 0; i < all.length; i++) {
      var m = all[i].getAttribute("data-media");
      if (!m || pathFrom(m, from) === src) return all[i];
    }
    return null;
  }
  // What to show: the view element's block box (it scopes the block's
  // sheet), else the element.
  function boxOf(node) {
    return node.closest(".site-block") || node;
  }
  function unmount() {
    if (!view.node) return;
    if (view.home) {
      view.home.parentNode.insertBefore(view.node, view.home);
      view.home.parentNode.removeChild(view.home);
    } else {
      follows(view.node, false);
      if (view.node.parentNode) view.node.parentNode.removeChild(view.node);
    }
    if (view.lyrics) {
      follows(view.lyrics, false);
      if (view.lyrics.parentNode) view.lyrics.parentNode.removeChild(view.lyrics);
    }
    view = { src: null, node: null, home: null, lyrics: null };
    ui.lyrics.hidden = true;
    ui.lyrics.setAttribute("aria-pressed", "false");
    sheet.classList.remove("site-now-showing-lyrics");
  }
  function mount(node, home, src, lyrics) {
    unmount();
    view = { src: src, node: node, home: home, lyrics: lyrics };
    if (home) node.parentNode.insertBefore(home, node);
    ui.body.textContent = "";
    ui.body.appendChild(node);
    if (!home) follows(node, true);
    if (lyrics) {
      ui.body.appendChild(lyrics);
      follows(lyrics, true);
      ui.lyrics.hidden = false;
    }
    ui.controls.hidden = !!node.querySelector("[data-transport]");
    ui.progress.hidden = !!node.querySelector("[data-progress]");
    island.sections = sectionsOf(node);
    emit(true);
  }
  // A view's sections (`data-sections` on its `[data-now-playing]`: JSON,
  // `[{name, start, end, color}]`, seconds on its recording), as data
  // only: what an island is given.
  function sectionsOf(node) {
    var at = node.matches && node.matches("[data-sections]") ? node : node.querySelector("[data-sections]");
    var raw = at ? at.getAttribute("data-sections") || "" : "";
    if (!raw || raw.length > 65536) return [];
    try {
      var list = JSON.parse(raw);
      if (!Array.isArray(list)) return [];
      return list.slice(0, 200).filter(function (x) {
        return x && isFinite(x.start) && isFinite(x.end) && x.end >= x.start;
      }).map(function (x) {
        return {
          name: String(x.name || "").slice(0, 80),
          start: Number(x.start),
          end: Number(x.end),
          color: /^#[0-9a-fA-F]{3,8}$/.test(String(x.color || "")) ? String(x.color) : null,
        };
      });
    } catch (e) {
      return [];
    }
  }
  function say(text) {
    var note = doc.createElement("p");
    note.className = "site-now-empty";
    note.textContent = text;
    ui.body.textContent = "";
    ui.body.appendChild(note);
  }
  function show(t) {
    if (!t || wanted === t.src) return;
    wanted = t.src;
    var mine = viewIn(doc, t.src);
    if (mine && !sheet.contains(mine)) {
      mount(boxOf(mine), doc.createComment("now-playing"), t.src, null);
      return;
    }
    unmount();
    ui.controls.hidden = false;
    ui.progress.hidden = false;
    if (!t.view || !t.page || t.page === here) {
      say("");
      return;
    }
    say("Loading…");
    var page = t.page;
    var got = fetched[page] || (fetched[page] = fetch(page, { credentials: "same-origin", redirect: "error", headers: { Accept: "text/html" } })
      .then(function (r) {
        var type = r.headers.get("content-type") || "";
        if (!r.ok) throw new Error(r.status === 503 ? "Its page is being built: try again in a moment." : "Its page could not be loaded (" + r.status + ").");
        if (r.headers.get("x-architect-page") !== "built" || !/^text\/html/i.test(type)) throw new Error("Its page is not one this link built.");
        return r.text();
      })
      .then(function (html) { return new DOMParser().parseFromString(html, "text/html"); }));
    Promise.all([got, rules()]).then(function (both) {
      var parsed = both[0], r = both[1];
      if (wanted !== t.src) return;
      var from = new URL(page, location.href).href;
      var found = viewIn(parsed, t.src, from);
      if (!found || !r) {
        say("");
        return;
      }
      var box = boxOf(found);
      var node = clean(r, doc.importNode(box, true), false, scopeOf(box));
      node.setAttribute("data-site-from", from);
      // Its boxes that follow "what plays on their page" follow its track
      // here.
      Array.prototype.forEach.call(node.querySelectorAll("[data-follow-playhead]:not([data-media])"), function (b) {
        b.setAttribute("data-media", t.src);
      });
      var words = lyricsIn(parsed, t.src, from);
      var lyrics = null;
      if (words) {
        lyrics = clean(r, doc.importNode(words, true), false, null);
        lyrics.setAttribute("data-site-from", from);
        lyrics.setAttribute("data-media", t.src);
        lyrics.classList.add("site-now-words");
      }
      mount(node, null, t.src, lyrics);
    }, function (e) {
      delete fetched[page];
      if (wanted === t.src) say(e && e.message ? e.message : "Its page could not be loaded.");
    });
  }
  function lyrics_() {
    var on = !sheet.classList.contains("site-now-showing-lyrics");
    sheet.classList.toggle("site-now-showing-lyrics", on);
    ui.lyrics.setAttribute("aria-pressed", on ? "true" : "false");
  }
  function open_() {
    if (!sheet) return;
    var t = tracks[cur];
    if (!t) return;
    show(t);
    if (sheet.hidden) {
      opener = doc.activeElement && doc.activeElement !== doc.body ? doc.activeElement : ui.title;
      sheet.hidden = false;
      doc.body.classList.add("site-now-open");
      if (modal()) {
        sheet.setAttribute("aria-modal", "true");
        inertBehind(true);
      } else sheet.removeAttribute("aria-modal");
      try { history.pushState({ siteNowPlaying: true }, ""); } catch (e) {}
      ui.close.focus();
      islandOpen();
    }
    emit(true);
    var cur_ = sheet.querySelector('[aria-current="time"]');
    if (cur_) intoView(cur_);
  }
  function close_(popped) {
    if (!sheet || sheet.hidden) return;
    sheet.hidden = true;
    doc.body.classList.remove("site-now-open");
    inertBehind(false);
    wanted = null;
    islandClose();
    unmount();
    if (!popped && history.state && history.state.siteNowPlaying) {
      try { history.back(); } catch (e) {}
    }
    var back = opener && opener.isConnected ? opener : ui.title;
    opener = null;
    if (back && back.focus) back.focus();
  }

  // ── The island (studio#445) ───────────────────────────────────────
  // A linked-in extension's views at the top of the view (where it is)
  // and under it (the controls), drawn by the node's web runtime: the
  // component runs in the sandbox document the node serves
  // (`/_ext/sandbox.<hash>.html`, an opaque origin with no network, its
  // own CSP; this page's is unchanged), its files fetched by this page
  // from the node's `/_ext/` and pinned by their lock, its views drawn
  // into shadow roots by `view.js` under the share pages' rules. It is
  // given data only (where the song is, its sections, its title) and may
  // ask only these, each checked here. The player's own position bar and
  // controls show until its first frame, and stay when it cannot load (a
  // browser without JS Promise Integration, a node without islands).
  var island = { loading: null, ext: null, panes: [], sections: [], fed: 0, sheet: null };
  // At most this many intents a second (a burst of twice that): an island
  // flooding play and pause is refused, not obeyed.
  var INTENTS_PER_SECOND = 8;
  var bucket = { tokens: INTENTS_PER_SECOND * 2, at: Date.now() };
  function intents() {
    var ok = function (fn) {
      return {
        effect: "read_only",
        run: function (args) {
          var now = Date.now();
          bucket.tokens = Math.min(INTENTS_PER_SECOND * 2, bucket.tokens + ((now - bucket.at) / 1000) * INTENTS_PER_SECOND);
          bucket.at = now;
          if (bucket.tokens < 1) throw { kind: "failed", message: "too many requests" };
          bucket.tokens -= 1;
          fn(args);
          return null;
        },
      };
    };
    return {
      PLAYHEAD_SEEK: ok(function (args) {
        var t = args && typeof args === "object" ? Number(args.t) : NaN;
        var d = snapshot().duration;
        if (!isFinite(t) || t < 0 || !d || t > d) throw { kind: "failed", message: "a time within the song" };
        playhead.seek(t);
      }),
      PLAYHEAD_PLAY: ok(function () { playhead.play(); }),
      PLAYHEAD_PAUSE: ok(function () { playhead.pause(); }),
      PLAYHEAD_NEXT: ok(function () { next(false); }),
      PLAYHEAD_PREVIOUS: ok(function () { previous(); }),
    };
  }
  function pinnedSheet(it) {
    var name = String(it.stylesheet);
    var digest = String(it.stylesheetIntegrity || "");
    if (!/^[A-Za-z0-9._-]+(\/[A-Za-z0-9._-]+)*$/.test(name) || name.split("/").some(function (x) { return x === "." || x === ".."; }) ||
        !/^sha256-[A-Za-z0-9+/]{43}=$/.test(digest) || !(window.crypto && window.crypto.subtle)) {
      return Promise.reject(new Error("its stylesheet is not pinned"));
    }
    return fetch(it.folder + name, { credentials: "omit", redirect: "error" })
      .then(function (r) {
        if (!r.ok) throw new Error("its stylesheet did not load");
        return r.arrayBuffer();
      })
      .then(function (bytes) {
        return window.crypto.subtle.digest("SHA-256", bytes).then(function (d) {
          var b = "";
          new Uint8Array(d).forEach(function (x) { b += String.fromCharCode(x); });
          if ("sha256-" + btoa(b) !== digest) throw new Error("its stylesheet is not the one pinned");
          return new TextDecoder().decode(bytes);
        });
      });
  }
  function islandOf() {
    return ext && ext.islands.length ? ext.islands[0] : null;
  }
  function islandLoad(it) {
    if (island.loading) return island.loading;
    var asks = intents();
    island.loading = import(ext.runtime + "index.js")
      .then(function (rt) {
        if (!rt.isSupported()) return null;
        // Its stylesheet, a file of its folder, pinned by its digest as the
        // component's files are by their lock; anything else and the
        // island does not load.
        var sheet_ = it.stylesheet ? pinnedSheet(it) : Promise.resolve(null);
        return Promise.all([
          rt.loadExtension(it.folder, {
            integrity: it.integrity || undefined,
            sandbox: ext.sandbox,
            host: { actions: asks },
            grant: { actions: Object.keys(asks), effect: "read_only" },
          }),
          sheet_,
        ]).then(function (both) {
          island.rt = rt;
          island.ext = both[0];
          island.sheet = both[1];
          return both[0];
        });
      })
      .catch(function (e) {
        island.failed = String((e && e.message) || e);
        return null;
      });
    return island.loading;
  }
  function islandOpen() {
    var it = islandOf();
    if (!it) return;
    clearTimeout(island.idle);
    islandLoad(it).then(function (e) {
      if (!e || sheet.hidden || island.panes.length) return;
      [[it.progress, ui.islandTop, ui.progress], [it.transport, ui.islandBottom, ui.controls]].forEach(function (slot) {
        if (!slot[0]) return;
        var m;
        try {
          m = island.rt.mountView(e, slot[0], slot[1], { sheet: island.sheet, sheetUrls: [it.folder] });
        } catch (err) {
          return;
        }
        island.panes.push(m);
        Promise.resolve(m.rendered).then(function () {
          if (island.panes.indexOf(m) < 0) return;
          // Its first frame: it takes the slot, the player's gives way.
          slot[1].hidden = false;
          slot[2].classList.add("site-now-yielded");
          feed(snapshot(), true);
        }, function () {});
      });
    });
  }
  // Closed for this long, the extension is unloaded (its instance, its
  // workers, its sandbox); the next open loads it again.
  var ISLAND_IDLE_MS = 30000;
  function islandClose() {
    island.panes.forEach(function (m) {
      try { m.unmount(); } catch (e) {}
    });
    island.panes = [];
    clearTimeout(island.idle);
    island.idle = setTimeout(function () {
      if (island.panes.length || !island.ext) return;
      try { island.ext.stop(); } catch (e) {}
      island.ext = null;
      island.loading = null;
    }, ISLAND_IDLE_MS);
    if (!ui.islandTop) return;
    ui.islandTop.hidden = true;
    ui.islandBottom.hidden = true;
    ui.progress.classList.remove("site-now-yielded");
    ui.controls.classList.remove("site-now-yielded");
  }
  // What the island is given, at most a frame a 16 ms: data only.
  function feed(s, force) {
    if (!island.panes.length) return;
    var now = Date.now();
    if (!force && now - island.fed < 16) return;
    island.fed = now;
    var t = tracks[s.index];
    var props = {
      time: s.time,
      playing: !!s.playing,
      duration: s.duration,
      index: s.index,
      tracks: s.tracks,
      title: t ? String(t.title || "") : "",
      sections: island.sections,
    };
    island.panes.forEach(function (m) {
      try { m.input({ claim: "playhead", source: "", props: props }); } catch (e) {}
    });
  }

  // The lock screen and the headset: the song, and its buttons.
  var session = window.navigator && window.navigator.mediaSession;
  var sessionFor = null, sessionAt = 0;
  if (session && bar) {
    var handle = function (name, fn) {
      try { session.setActionHandler(name, fn); } catch (e) {}
    };
    handle("play", function () { playhead.play(); });
    handle("pause", function () { playhead.pause(); });
    handle("previoustrack", function () { previous(); });
    handle("nexttrack", function () { next(false); });
    handle("seekto", function (d) { if (d && isFinite(d.seekTime)) playhead.seek(d.seekTime); });
  }
  function media_(s, t) {
    if (!session) return;
    if (t && sessionFor !== t && window.MediaMetadata) {
      sessionFor = t;
      try {
        session.metadata = new window.MediaMetadata({ title: t.title, artist: t.subtitle || "", album: t.set || doc.title || "" });
      } catch (e) {}
    }
    try { session.playbackState = s.playing ? "playing" : "paused"; } catch (e) {}
    var now = Date.now();
    if (s.duration && session.setPositionState && now - sessionAt > 1000) {
      sessionAt = now;
      try { session.setPositionState({ duration: s.duration, position: Math.min(s.time, s.duration), playbackRate: s.rate || 1 }); } catch (e) {}
    }
  }

  // The bar steps aside while a source holds the clock: the engine
  // brings its own transport.
  function yieldBar(away) {
    if (!bar) return;
    bar.hidden = away;
    doc.body.classList.toggle("site-has-player", !away);
  }
  function clock(t) {
    if (t == null || !isFinite(t)) return "–:––";
    t = Math.max(0, Math.floor(t));
    var s = t % 60;
    return Math.floor(t / 60) + ":" + (s < 10 ? "0" : "") + s;
  }
  // What the toggles show: changed only when the playing state does.
  var shownPlaying = null;
  function paint(s) {
    rows.forEach(function (r) {
      var on = s.src === r.src;
      if (on) {
        r.el.setAttribute("aria-current", "true");
        r.el.setAttribute("data-state", s.playing ? "playing" : "paused");
      } else if (r.el.hasAttribute("aria-current")) {
        r.el.removeAttribute("aria-current");
        r.el.removeAttribute("data-state");
      }
    });
    if (!bar) return;
    var t = tracks[s.index];
    var part = s.duration ? Math.min(1, s.time / s.duration) : 0;
    if (shownPlaying !== s.playing) {
      shownPlaying = s.playing;
      Array.prototype.forEach.call(doc.querySelectorAll('.site-player [data-act="toggle"], .site-now [data-act="toggle"]'), function (b) {
        b.innerHTML = s.playing ? ICON.pause : ICON.play;
        b.setAttribute("aria-label", s.playing ? "Pause" : "Play");
      });
    }
    bar.classList.toggle("site-player-on", !!t);
    ui.title.textContent = t ? t.title : tracks.length === 1 ? "1 track" : tracks.length + " tracks";
    if (t && t.page) ui.title.href = t.page; else ui.title.removeAttribute("href");
    if (hasView(t)) ui.title.setAttribute("aria-haspopup", "dialog");
    else ui.title.removeAttribute("aria-haspopup");
    ui.note.textContent = noteText || (t ? t.subtitle || "" : "");
    ui.note.classList.toggle("site-player-warn", !!noteText);
    ui.time.textContent = clock(s.time) + " / " + clock(s.duration);
    ui.line.style.width = (part * 100).toFixed(2) + "%";
    media_(s, t);
    if (sheet && !sheet.hidden) feed(s, false);
    if (sheet) {
      ui.nowTitle.textContent = t ? t.title : "";
      ui.nowNote.textContent = noteText || (t ? t.subtitle || "" : "");
      if (t && t.page && t.page !== here) {
        ui.nowPage.href = t.page;
        ui.nowPage.hidden = false;
      } else {
        ui.nowPage.removeAttribute("href");
        ui.nowPage.hidden = true;
      }
      ui.seekFill.style.width = (part * 100).toFixed(2) + "%";
      ui.seek.setAttribute("aria-valuemax", s.duration ? String(Math.round(s.duration)) : "0");
      ui.seek.setAttribute("aria-valuenow", String(Math.round(s.time)));
      ui.seek.setAttribute("aria-valuetext", clock(s.time));
      ui.nowAt.textContent = clock(s.time);
      ui.nowLeft.textContent = s.duration ? "-" + clock(s.duration - s.time) : "–:––";
      if (!sheet.hidden && t && wanted !== t.src) show(t);
    }
  }

  // Resume where the last page left off.
  var at = saved.src ? indexOf(path(saved.src)) : -1;
  if (at >= 0) select(at, saved.time || 0, !!saved.playing, true);
  else emit(true);
  window.addEventListener("pagehide", function () { save(true); });
})();
