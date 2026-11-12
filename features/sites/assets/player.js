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
// playing) in document order are the queue; a tap on one plays it and
// the page stays where it is; a `[data-playlist-play]` in it plays it from
// the top; the track playing is marked aria-current="true". Songs follow
// each other without a page load. The bar at the bottom (the song, its
// position, play / pause) opens the now-playing view: the playing track's
// `[data-now-playing]` element (from its page, fetched when it is
// another), with the player's controls under it unless it brings its own
// (`[data-transport]` in it). Back, a swipe down or Escape closes it;
// what plays goes on.
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
      var page = row.getAttribute("data-page");
      listed.push({
        src: src,
        page: page ? path(page) : here,
        title: row.getAttribute("data-title") || row.textContent.trim(),
        subtitle: row.getAttribute("data-subtitle") || "",
        kind: "audio",
      });
    });
  });
  var shell = listed.length > 0;
  if (shell) {
    inline.forEach(function (m) {
      var src = path(m.getAttribute("src") || "");
      if (!listed.some(function (t) { return t.src === src; })) {
        listed.push({ src: src, page: here, title: m.textContent.trim() || src.split("/").pop(), subtitle: "", kind: m.tagName.toLowerCase() });
      }
    });
    tracks = listed;
  }
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
      src: t ? t.src : null,
      page: t ? t.page : null,
      here: !!(t && t.page === here),
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
        emit(name !== "timeupdate");
        save(name !== "timeupdate");
        if (name === "play" && !frame) frame = requestAnimationFrame(tick);
      });
    });
    media.addEventListener("ended", function () {
      if (media === el && !source) next(true);
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
        if (p && p.catch) p.catch(function () { paint(snapshot(), "Press play to go on"); });
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
    var top = box.scrollTop + (r.top - b.top) - (box.clientHeight - r.height) / 2;
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
  // on a page. Its song opens the now-playing view.
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
    else if (a === "open") open_();
    else if (a === "close") close_();
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
      '<button type="button" class="site-player-what" data-act="open" aria-label="Now playing"><span class="site-player-title"></span><span class="site-player-note"></span></button>' +
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

  // The now-playing view: the playing track's `[data-now-playing]`, in a
  // sheet over the page. One from this page is moved in (with its block's
  // box, which scopes its sheet) and back out on close; one from another
  // page is fetched once and kept.
  var sheet = null, view = { src: null, node: null, home: null };
  // The track whose view is shown or on its way.
  var wanted = null;
  var fetched = {};
  if (bar) {
    sheet = doc.createElement("div");
    sheet.className = "site-now";
    sheet.hidden = true;
    sheet.setAttribute("role", "dialog");
    sheet.setAttribute("aria-modal", "true");
    sheet.setAttribute("aria-label", "Now playing");
    sheet.innerHTML =
      '<div class="site-now-head"><button type="button" class="site-now-close" data-act="close" aria-label="Close">' + ICON.down + "</button>" +
      '<div class="site-now-what"><span class="site-now-title"></span><span class="site-now-note"></span></div>' +
      '<a class="site-now-page">Song page</a></div>' +
      '<div class="site-now-body"></div>' +
      '<div class="site-now-controls"><div class="site-player-line" aria-hidden="true"><span></span></div><div class="site-now-buttons"><span class="site-now-at">0:00</span>' + controls() + '<span class="site-now-left">–:––</span></div></div>';
    ui.nowTitle = sheet.querySelector(".site-now-title");
    ui.nowNote = sheet.querySelector(".site-now-note");
    ui.nowPage = sheet.querySelector(".site-now-page");
    ui.body = sheet.querySelector(".site-now-body");
    ui.controls = sheet.querySelector(".site-now-controls");
    ui.nowLine = sheet.querySelector(".site-now-controls .site-player-line span");
    ui.nowAt = sheet.querySelector(".site-now-at");
    ui.nowLeft = sheet.querySelector(".site-now-left");
    sheet.addEventListener("click", function (ev) {
      var a = ev.target.closest("[data-act]");
      if (a && !ev.target.closest(".site-now-body")) act(a.getAttribute("data-act"));
    });
    // A swipe down on its head closes it.
    var startY = null;
    var head = sheet.querySelector(".site-now-head");
    head.addEventListener("touchstart", function (ev) { startY = ev.touches[0].clientY; }, { passive: true });
    head.addEventListener("touchend", function (ev) {
      if (startY !== null && ev.changedTouches[0].clientY - startY > 60) close_();
      startY = null;
    });
    doc.addEventListener("keydown", function (ev) {
      if (ev.key === "Escape" && !sheet.hidden) close_();
    });
    window.addEventListener("popstate", function () {
      if (!sheet.hidden && !(history.state && history.state.siteNowPlaying)) close_(true);
    });
    doc.body.appendChild(sheet);
  }
  // The view on this page whose media is `src`, else its first.
  function viewIn(root, src) {
    var all = Array.prototype.slice.call(root.querySelectorAll("[data-now-playing]"));
    for (var i = 0; i < all.length; i++) {
      var m = all[i].getAttribute("data-media");
      if (m && path(m, all[i]) === src) return all[i];
    }
    return all.length === 1 && !all[0].getAttribute("data-media") ? all[0] : null;
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
    view = { src: null, node: null, home: null };
  }
  function mount(node, home, src) {
    unmount();
    view = { src: src, node: node, home: home };
    if (home) node.parentNode.insertBefore(home, node);
    ui.body.textContent = "";
    ui.body.appendChild(node);
    if (!home) follows(node, true);
    ui.controls.hidden = !!node.querySelector("[data-transport]");
    sheet.classList.toggle("site-now-own", ui.controls.hidden);
    emit(true);
  }
  function show(t) {
    if (!t || wanted === t.src) return;
    wanted = t.src;
    var mine = viewIn(doc, t.src);
    if (mine && !sheet.contains(mine)) {
      var box = boxOf(mine);
      mount(box, doc.createComment("now-playing"), t.src);
      return;
    }
    unmount();
    ui.controls.hidden = false;
    var note = doc.createElement("p");
    note.className = "site-now-empty";
    note.textContent = "Loading…";
    ui.body.textContent = "";
    ui.body.appendChild(note);
    if (t.page === here) {
      note.textContent = "";
      return;
    }
    var page = t.page;
    var got = fetched[page] || (fetched[page] = fetch(page, { credentials: "same-origin" })
      .then(function (r) { return r.ok ? r.text() : ""; })
      .then(function (html) {
        var parsed = new DOMParser().parseFromString(html, "text/html");
        return parsed;
      }));
    got.then(function (parsed) {
      if (wanted !== t.src) return;
      var found = parsed ? viewIn(withBase(parsed, page), t.src) : null;
      if (!found) {
        note.textContent = "";
        return;
      }
      var node = doc.importNode(boxOf(found), true);
      node.setAttribute("data-site-from", new URL(page, location.href).href);
      mount(node, null, t.src);
    }).catch(function () { note.textContent = ""; });
  }
  // `parsed` read as from `page`: its views' relative URLs resolve there.
  function withBase(parsed, page) {
    Array.prototype.forEach.call(parsed.querySelectorAll("[data-now-playing]"), function (n) {
      boxOf(n).setAttribute("data-site-from", new URL(page, location.href).href);
    });
    return parsed;
  }
  function open_() {
    if (!sheet) return;
    var t = tracks[cur];
    if (!t) {
      if (tracks.length) select(0, 0, false, true);
      t = tracks[cur];
    }
    if (!t) return;
    show(t);
    if (sheet.hidden) {
      sheet.hidden = false;
      doc.body.classList.add("site-now-open");
      try { history.pushState({ siteNowPlaying: true }, ""); } catch (e) {}
    }
    emit(true);
    var cur_ = sheet.querySelector('[aria-current="time"]');
    if (cur_) intoView(cur_);
  }
  function close_(popped) {
    if (!sheet || sheet.hidden) return;
    sheet.hidden = true;
    doc.body.classList.remove("site-now-open");
    wanted = null;
    unmount();
    if (!popped && history.state && history.state.siteNowPlaying) {
      try { history.back(); } catch (e) {}
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
  function paint(s, note) {
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
    Array.prototype.forEach.call(doc.querySelectorAll('.site-player [data-act="toggle"], .site-now [data-act="toggle"]'), function (b) {
      b.innerHTML = s.playing ? ICON.pause : ICON.play;
      b.setAttribute("aria-label", s.playing ? "Pause" : "Play");
    });
    bar.classList.toggle("site-player-on", !!t);
    ui.title.textContent = t ? t.title : tracks.length === 1 ? "1 track" : tracks.length + " tracks";
    if (note !== undefined) ui.note.textContent = note;
    else ui.note.textContent = t ? t.subtitle || "" : "";
    ui.time.textContent = clock(s.time) + " / " + clock(s.duration);
    ui.line.style.width = (part * 100).toFixed(2) + "%";
    if (sheet) {
      ui.nowTitle.textContent = t ? t.title : "";
      ui.nowNote.textContent = t ? t.subtitle || "" : "";
      if (t && t.page && t.page !== here) ui.nowPage.href = t.page; else ui.nowPage.removeAttribute("href");
      ui.nowPage.hidden = !(t && t.page && t.page !== here);
      ui.nowLine.style.width = (part * 100).toFixed(2) + "%";
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
