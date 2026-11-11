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
// Pages are separate documents: the player keeps where it is (track,
// time, playing) in sessionStorage and resumes on the next page; a
// browser that refuses to start sound without a click leaves it paused
// at that time. Inert on a page with no media and no queue.
(function () {
  "use strict";
  var doc = document;
  var arch = (window.architect = window.architect || {});
  if (arch.playhead) return;

  function path(url) {
    try {
      var p = new URL(url, location.href).pathname;
      return p.replace(/index\.html$/, "");
    } catch (e) {
      return String(url);
    }
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
    if (!media && follow && !stay && t.page !== here) {
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
    if (cur + 1 < tracks.length) select(cur + 1, 0, true);
    else if (auto) emit(true);
  }
  function previous() {
    if (el && el.currentTime > 3) {
      el.currentTime = 0;
      return;
    }
    if (cur > 0) select(cur - 1, 0, el ? !el.paused : true);
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
      var want = src ? indexOf(src) : cur >= 0 && tracks[cur].page === here ? cur : firstHere();
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

  // Blocks that follow without a script.
  var lines = Array.prototype.map.call(followers, function (box) {
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
      media: media ? path(media) : null,
      items: items,
      at: items.map(function (i) { return parseFloat(i.getAttribute("data-at")) || 0; }),
      current: null,
    };
  });
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

  // The bar: shown for a link's queue, or two players or more on a page.
  var bar = null, ui = {};
  if (queueEl ? tracks.length > 0 : inline.length > 1) {
    bar = doc.createElement("div");
    bar.className = "site-player";
    bar.setAttribute("role", "region");
    bar.setAttribute("aria-label", "Player");
    bar.innerHTML =
      '<button type="button" data-act="previous" aria-label="Previous">⏮</button>' +
      '<button type="button" data-act="toggle" aria-label="Play">▶</button>' +
      '<button type="button" data-act="next" aria-label="Next">⏭</button>' +
      '<span class="site-player-what"><a class="site-player-title"></a><span class="site-player-note"></span></span>' +
      '<span class="site-player-time">0:00</span>' +
      '<label class="site-player-follow" title="Go to each song\'s page as it plays"><input type="checkbox"> Follow</label>';
    ui.toggle = bar.querySelector('[data-act="toggle"]');
    ui.title = bar.querySelector(".site-player-title");
    ui.note = bar.querySelector(".site-player-note");
    ui.time = bar.querySelector(".site-player-time");
    ui.follow = bar.querySelector(".site-player-follow input");
    ui.follow.checked = follow;
    ui.follow.addEventListener("change", function () {
      follow = ui.follow.checked;
      save(true);
    });
    bar.addEventListener("click", function (ev) {
      var act = ev.target.closest("[data-act]");
      if (!act) return;
      var a = act.getAttribute("data-act");
      if (a === "previous") previous();
      else if (a === "next") next(false);
      else if (el && !el.paused) el.pause();
      else playhead.play();
    });
    doc.body.appendChild(bar);
    doc.body.classList.add("site-has-player");
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
    if (!bar) return;
    var t = tracks[s.index];
    ui.toggle.textContent = s.playing ? "⏸" : "▶";
    ui.toggle.setAttribute("aria-label", s.playing ? "Pause" : "Play");
    ui.title.textContent = t ? t.title : tracks.length + " tracks";
    if (t) ui.title.href = t.page; else ui.title.removeAttribute("href");
    ui.time.textContent = clock(s.time) + " / " + clock(s.duration);
    if (note !== undefined) ui.note.textContent = note;
    else if (s.playing) ui.note.textContent = "";
  }

  // Resume where the last page left off.
  var at = saved.src ? indexOf(path(saved.src)) : -1;
  if (at >= 0) select(at, saved.time || 0, !!saved.playing, true);
  else emit(true);
  window.addEventListener("pagehide", function () { save(true); });
})();
