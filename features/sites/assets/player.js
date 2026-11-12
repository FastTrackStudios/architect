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
// and seek there on a click (an item is its nearest box's: a box inside a
// box follows its own). They may be HTML or SVG elements (a chart's
// `<rect data-at>` per bar, keyflow#59).
//
// An item may glide: with `data-glide="d"` the item marked moves `d` along
// x (in the units of its own `x`; px for HTML) as the playhead goes from
// its `data-at` to the next item's, or to its own `data-until` when it has
// one (the last item, or one that holds still before the next), on every
// frame while playing and exactly where the playhead is on a seek or a
// pause: a chart's playhead line gliding across its beats (Session's
// cursor, studio's `follow.rs`). Where the next item is on another line,
// the glide ends where this line does, and the next item shows where it
// is. It moves by a transform of that one element (the SVG `transform`
// attribute, an HTML element's style); the item left goes back to where
// it is drawn. A box with gliding items marks them at the playhead itself
// (a box without marks a line 50 ms early, as lyrics are read ahead).
//
// A playlist (`[data-playlist]`, a block's list of songs) makes the page
// a player: its tracks (`[data-track][data-media]`, each with
// `data-title`, `data-subtitle` and `data-page`, the page that shows it
// playing) are the queue, every playlist of the page one after the other
// in document order; the page's own `<audio>`/`<video>` are not queued
// there (one played by hand plays alone). Playlists one after another
// with nothing but headings between them are ONE list (a setlist's sets):
// each heading becomes a section label in it (`[data-playlist-section]`,
// never a track). A row of a song with no recording (`[data-page]`, no
// `data-track`) opens its page. A tap on a row plays it and the page
// stays where it is; the track playing is marked aria-current="true". A
// row's `[data-track-art]` tile shows the song's art when the link has
// one for its page (the queue's `art`, by page: the node's, from the
// note's `art:` under the link's scope and layer), else stays as drawn.
// Songs follow each other without a page load. The bar (a card: the
// song's art, its title, Click, Loop, play) opens the now-playing view: a
// position bar at the top, the playing track's `[data-now-playing]`
// element, the controls under it (a view bringing its own:
// `[data-progress]`, `[data-transport]` in it). Back, a swipe down or
// Escape closes it; what plays goes on.
//
// Click (a metronome on the song's own beats: its view's
// `[data-at][data-beat]` marks, beat 1 accented, Web Audio scheduled
// ahead against the media clock) and Loop (the section playing, of its
// view's `data-sections`, again and again) are each viewer's, kept with
// the player's place.
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
  var SVG_NS = "http://www.w3.org/2000/svg";
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
      // The web view frame's document, when the node has one (a view of an
      // island's web bundle runs there: no JS Promise Integration needed).
      x.webview = own(x.webview) && /\.html$/.test(x.webview) ? x.webview : null;
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

  // The first element of `node` (itself included) with attribute `attr`.
  function firstOf(node, attr) {
    if (node.hasAttribute && node.hasAttribute(attr)) return node;
    var all = node.querySelectorAll("[" + attr + "]");
    return all && all.length ? all[0] : null;
  }
  function isHeading(n) {
    return !!n && /^H[1-6]$/.test(String(n.tagName || "").toUpperCase());
  }
  // The list a playlist's rows are in.
  function rowsOf(list) {
    var kids = list.children || [];
    for (var i = 0; i < kids.length; i++) {
      if (/^(OL|UL)$/.test(String(kids[i].tagName).toUpperCase())) return kids[i];
    }
    return null;
  }
  // A section label for `heading`: a row of the list that is no track,
  // with the heading's id (a link to it still lands).
  function sectionRow(heading) {
    var li = doc.createElement("li");
    li.setAttribute("data-playlist-section", "");
    // Its words, not its anchor's `#`.
    Array.prototype.slice.call(heading.querySelectorAll(".site-anchor")).forEach(function (a) {
      a.parentNode.removeChild(a);
    });
    li.textContent = String(heading.textContent || "").replace(/\s+/g, " ").trim();
    var id = heading.getAttribute("id");
    if (id) li.setAttribute("id", id);
    return li;
  }
  // Playlists one after another with nothing but headings between them
  // (each in the box the site draws a block in) are one, when each says it
  // may be joined (`data-playlist-join`; a setlist's sets as Studio wrote
  // them before, `.studio-setlist`): the later lists' rows join the
  // first's, each heading between a section label, and the heading right
  // above the first one too. A lone playlist stays as it is, and so does
  // one that does not say so (a list of stems beside a list of references).
  function joins(list) {
    return list.hasAttribute("data-playlist-join") || (list.classList && list.classList.contains("studio-setlist"));
  }
  function joinPlaylists() {
    var host = null, hostBox = null, hostList = null, above = null, joined = false;
    Array.prototype.slice.call(doc.querySelectorAll("[data-playlist]")).filter(joins).forEach(function (list) {
      var box = (list.closest && list.closest(".site-block")) || list;
      var heads = [], n = box.previousElementSibling;
      while (isHeading(n)) { heads.unshift(n); n = n.previousElementSibling; }
      var into = host && hostList && n === hostBox ? hostList : null;
      var mine = rowsOf(list);
      if (!into || !mine) {
        host = list; hostBox = box; hostList = mine; joined = false;
        above = heads.length ? heads[heads.length - 1] : null;
        return;
      }
      if (!joined && above) {
        into.insertBefore(sectionRow(above), into.firstChild);
        above.parentNode.removeChild(above);
      }
      joined = true;
      host.setAttribute("data-playlist-joined", "");
      heads.forEach(function (h) {
        into.appendChild(sectionRow(h));
        h.parentNode.removeChild(h);
      });
      while (mine.firstChild) into.appendChild(mine.firstChild);
      // What follows its rows (a link's files) follows the joined list.
      var after = false;
      Array.prototype.slice.call(list.children).forEach(function (c) {
        if (c === mine) { after = true; return; }
        if (after) host.appendChild(c);
      });
      box.parentNode.removeChild(box);
    });
  }
  // `tile` with the art at `src` over it; the tile as drawn if it fails.
  function showArt(tile, src) {
    var old = firstOf(tile, "data-art");
    if (old && old !== tile && old.getAttribute("src") === src) return old;
    if (old && old !== tile) old.parentNode.removeChild(old);
    var img = doc.createElement("img");
    img.setAttribute("data-art", "");
    img.setAttribute("alt", "");
    img.setAttribute("decoding", "async");
    img.addEventListener("error", function () {
      if (img.parentNode) img.parentNode.removeChild(img);
      tile.removeAttribute("data-has-art");
    });
    img.setAttribute("src", src);
    tile.setAttribute("data-has-art", "");
    tile.appendChild(img);
    return img;
  }

  // `raw` as a file of the link mounted at `mount`: same origin, no
  // query, under the mount; its path, else null.
  function ownFile(raw, mount) {
    if (typeof raw !== "string" || !raw) return null;
    var u;
    try { u = new URL(raw, location.href); } catch (e) { return null; }
    if (u.origin !== location.origin || u.search || u.hash || u.username || u.password) return null;
    return u.pathname.indexOf(mount) === 0 && u.pathname.indexOf("/..") < 0 ? u.pathname : null;
  }
  // A page's art (its path), by the page's path.
  var artBy = {};

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
      // Each page's art, as the node gives it: only a file under the link.
      var linkRoot = normal(scope);
      if (q.art && typeof q.art === "object" && linkRoot) {
        Object.keys(q.art).forEach(function (page) {
          var file = ownFile(q.art[page], linkRoot);
          if (file) artBy[path(page)] = file;
        });
      }
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
  joinPlaylists();
  var rows = [];
  var songRows = [];
  var listed = [];
  Array.prototype.forEach.call(doc.querySelectorAll("[data-playlist]"), function (list) {
    var set = list.getAttribute("data-title") || list.getAttribute("aria-label") || "";
    Array.prototype.forEach.call(list.querySelectorAll("[data-page]"), function (row) {
      var page = builtPage(row.getAttribute("data-page"));
      var tile = firstOf(row, "data-track-art");
      if (tile && page && artBy[page]) showArt(tile, artBy[page]);
      if (row.hasAttribute("data-track") && row.hasAttribute("data-media")) return;
      // A song with no recording: its row opens its page.
      if (page) songRows.push({ el: row, page: page });
    });
    // Each row its own place in the queue: a song listed twice is played
    // and marked where it was tapped, and goes on from there.
    Array.prototype.forEach.call(list.querySelectorAll("[data-track][data-media]"), function (row) {
      var src = path(row.getAttribute("data-media"));
      rows.push({ el: row, src: src, i: listed.length });
      var page = builtPage(row.getAttribute("data-page"));
      listed.push({
        src: src,
        page: page,
        view: !!page,
        title: row.getAttribute("data-title") || row.textContent.trim(),
        subtitle: row.getAttribute("data-subtitle") || "",
        set: set,
        kind: "audio",
        art: page ? artBy[page] || null : null,
        tile: firstOf(row, "data-track-art"),
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
  // Click and Loop, as this viewer left them.
  var prefs = { click: saved.click === true, loop: saved.loop === true };
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
    var key = [s.time.toFixed(3), s.playing, s.rate, s.src, s.duration].join("|");
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
    store({ src: s.src, time: s.time, playing: s.playing, follow: follow, click: prefs.click, loop: prefs.loop });
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
    // Followed each frame from its first play, not each timeupdate.
    kick();
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
      store({ src: t.src, time: time || 0, playing: play, follow: follow, click: prefs.click, loop: prefs.loop });
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
    // Looping a section that ends with the song: back to its start.
    var held = auto ? looped() : null;
    if (held) {
      select(cur, held.start, true, true);
      return;
    }
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
      paintOpts();
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
          paintOpts();
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
    // Its own items: a box inside it (a chart's beats under its bars)
    // follows its own.
    var items = Array.prototype.slice.call(box.querySelectorAll("[data-at]")).filter(function (i) {
      return i.parentElement && i.parentElement.closest("[data-follow-playhead]") === box;
    });
    var media = box.getAttribute("data-media");
    box.addEventListener("click", function (ev) {
      var item = ev.target.closest("[data-at]");
      if (!item || items.indexOf(item) < 0) return;
      var at = parseFloat(item.getAttribute("data-at"));
      if (isFinite(at)) playhead.seek(at, media ? { src: media } : undefined);
    });
    var at = items.map(function (i) { return parseFloat(i.getAttribute("data-at")) || 0; });
    // Each item's glide: how far it moves, and when it is there.
    var glides = items.map(function (item, i) {
      var d = parseFloat(item.getAttribute("data-glide"));
      if (!isFinite(d)) return null;
      var until = parseFloat(item.getAttribute("data-until"));
      if (!isFinite(until)) until = i + 1 < at.length ? at[i + 1] : NaN;
      return { by: d, until: until };
    });
    var glides_ = glides.some(function (g) { return g; });
    return {
      box: box,
      media: media ? path(media, box) : null,
      items: items,
      at: at,
      glides: glides_ ? glides : null,
      current: null,
      // The item moved, and its own transform before it was.
      moved: null,
      base: null,
    };
  }
  var lines = Array.prototype.map.call(followers, line);
  function follows(root, on) {
    var boxes = Array.prototype.slice.call(root.querySelectorAll("[data-follow-playhead]"));
    // The root itself, when it is one (a lyrics list brought in whole).
    if (root.hasAttribute && root.hasAttribute("data-follow-playhead")) boxes.unshift(root);
    lines = lines.filter(function (f) {
      if (boxes.indexOf(f.box) < 0) return true;
      // A box let go of: its glided item back where it is drawn.
      if (f.moved) shift(f.moved, f.base, null);
      return false;
    });
    if (on) lines = lines.concat(boxes.map(line));
    emit(true);
  }
  function follow_(s) {
    lines.forEach(function (f) {
      var mine = f.media ? s.src === f.media : s.here;
      var found = null, best = -1;
      if (mine) {
        // The last line that has started.
        var ahead = f.glides ? 0 : 0.05;
        var lo = 0, hi = f.at.length - 1;
        while (lo <= hi) {
          var mid = (lo + hi) >> 1;
          if (f.at[mid] <= s.time + ahead) { best = mid; lo = mid + 1; } else hi = mid - 1;
        }
        found = best >= 0 ? f.items[best] : null;
      }
      if (f.glides) glide(f, best >= 0 ? best : -1, s.time);
      if (found === f.current) return;
      if (f.current) f.current.removeAttribute("aria-current");
      f.current = found;
      if (!found) return;
      found.setAttribute("aria-current", "time");
      intoView(found);
    });
  }

  // Box `f`'s item `i` (-1: none) where it has glided to at `time`; the
  // item it moved before back where it is drawn.
  function glide(f, i, time) {
    var item = i >= 0 ? f.items[i] : null;
    var g = item ? f.glides[i] : null;
    if (f.moved && f.moved !== (g ? item : null)) {
      shift(f.moved, f.base, null);
      f.moved = null;
    }
    if (!g) return;
    if (f.moved !== item) {
      f.moved = item;
      f.base = item.namespaceURI === SVG_NS ? item.getAttribute("transform") : item.style.transform || "";
    }
    var from = f.at[i];
    var part = g.until > from ? Math.min(1, Math.max(0, (time - from) / (g.until - from))) : 0;
    shift(item, f.base, g.by * part);
  }
  // `el` moved `dx` along its x from where it is drawn (its own transform,
  // `base`, kept); null: back where it is drawn.
  function shift(el, base, dx) {
    if (el.namespaceURI === SVG_NS) {
      if (dx === null) {
        if (base === null || base === undefined) el.removeAttribute("transform");
        else el.setAttribute("transform", base);
      } else el.setAttribute("transform", (base ? base + " " : "") + "translate(" + dx.toFixed(3) + " 0)");
    } else {
      el.style.transform = dx === null ? base || "" : (base ? base + " " : "") + "translateX(" + dx.toFixed(3) + "px)";
    }
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
    // A metronome: its body, its pendulum.
    click: '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill-rule="evenodd" d="M9 2.5h6l4.6 19H4.4zm1.3 1.8-3.5 15.4h10.4L13.7 4.3z"/><path d="m11.2 17.6 6.3-11 1.5.9-6.3 11z"/><path d="M6.3 17.2h11.4v2.5H6.3z"/></svg>',
    loop: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M17 1.5 21.5 6 17 10.5V7.2H7.5A2.5 2.5 0 0 0 5 9.7V12H3V9.7a4.5 4.5 0 0 1 4.5-4.5H17zM7 22.5 2.5 18 7 13.5v3.3h9.5a2.5 2.5 0 0 0 2.5-2.5V12h2v2.3a4.5 4.5 0 0 1-4.5 4.5H7z"/><path d="M11 9.5h2v5h-2z"/></svg>',
    page: '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill-rule="evenodd" d="M5 2h9.4L20 7.6V22H5zm2 2v16h11V9h-5V4zm8 .4V7h2.6z"/></svg>',
    words: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 5h18v2H3zm0 6h18v2H3zm0 6h11v2H3z"/></svg>',
    // A song with no art: Lucide's music note (the page's icon set), never letters.
    note: '<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>',
  };
  // Click and Loop: a toggle each, on when pressed.
  function toggles() {
    return '<button type="button" class="site-player-opt" data-act="click" aria-pressed="false" aria-label="Click" title="Click: a beat with the song">' + ICON.click + "</button>" +
      '<button type="button" class="site-player-opt" data-act="loop" aria-pressed="false" aria-label="Loop section" title="Loop the section playing">' + ICON.loop + "</button>";
  }
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
    else if (a === "click") clickToggle();
    else if (a === "loop") loopToggle();
    else if (snapshot().playing) playhead.pause();
    else playhead.play();
  }
  var bar = null, ui = {};
  if (shell || (queueEl ? tracks.length > 0 : inline.length > 1)) {
    bar = doc.createElement("div");
    bar.className = "site-player";
    bar.setAttribute("role", "region");
    bar.setAttribute("aria-label", "Player");
    // A card: the song's art and title (a tap opens its view), then its
    // buttons; on a playlist's page Click, Loop and play (previous and
    // next are in the view), elsewhere previous, play, next.
    bar.innerHTML =
      '<span class="site-player-art" aria-hidden="true"></span>' +
      '<span class="site-player-what"><a class="site-player-title"></a><span class="site-player-note"></span></span>' +
      '<span class="site-player-time">0:00</span>' +
      (shell ? toggles() + '<button type="button" class="site-player-toggle" data-act="toggle" aria-label="Play">' + ICON.play + "</button>" : controls()) +
      (shell ? "" : '<label class="site-player-follow" title="Go to each song\'s page as it plays"><input type="checkbox"> Follow</label>') +
      '<div class="site-player-line" aria-hidden="true"><span></span></div>';
    if (shell) bar.classList.add("site-player-card");
    ui.art = bar.querySelector(".site-player-art");
    // Its tile from the start: the note, not an empty square, before any
    // song is loaded ("5 tracks").
    if (shell) paintArt(null);
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
      wake();
      var a = ev.target.closest("[data-act]");
      if (a) {
        act(a.getAttribute("data-act"));
        return;
      }
      if (ev.target.closest("label, input")) return;
      // The card's song (anywhere but its buttons): its view, not its page.
      if (hasView(tracks[cur])) {
        ev.preventDefault();
        open_();
      }
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
      if (ev.target.closest("button, [data-track-more]")) return;
      var i = r.i;
      if (!tracks[i]) return;
      wake();
      if (i === cur && el) act("toggle");
      else select(i, 0, true, true);
    });
  });
  // A song with no recording: its row is a way to its page (its chart).
  songRows.forEach(function (r) {
    r.el.addEventListener("click", function (ev) {
      if (ev.target.closest("a, button")) return;
      location.href = r.page;
    });
  });

  // ── A row's menu (its `[data-track-more]`, "…") ───────────────────
  // What there is for that song, nothing that is not: its chart (the
  // now-playing view, the song made current), its lyrics (when its page
  // has them), its page and downloads. A sheet from the foot on a phone,
  // beside the button on a desk; arrows move, Escape closes and gives the
  // button back its focus.
  var menu = null;
  var deskMenu = window.matchMedia ? window.matchMedia("(min-width: 900px)") : { matches: true };
  function closeMenu(back) {
    if (!menu) return;
    var m = menu;
    menu = null;
    [m.box, m.scrim].forEach(function (n) { if (n && n.parentNode) n.parentNode.removeChild(n); });
    m.button.setAttribute("aria-expanded", "false");
    if (back !== false && m.button.focus) m.button.focus();
  }
  // The song in the view (or its lyrics): made current first, playing on
  // if the set was.
  function viewSong(i, words) {
    if (i !== cur) select(i, 0, snapshot().playing, true);
    var view = words ? "player.lyrics" : "player.now";
    if (arch.shell && typeof arch.shell.view === "function" && arch.shell.view(view)) return;
    open_();
    lyrics_(!!words);
  }
  function menuItems(row) {
    var page = builtPage(row.getAttribute("data-page"));
    var r = rows.filter(function (x) { return x.el === row; })[0];
    var t = r ? tracks[r.i] : null;
    var items = [];
    if (t && hasView(t)) {
      items.push({ label: "Open chart", run: function () { viewSong(r.i, false); } });
      if (known[t.src] && known[t.src].lyrics) items.push({ label: "Lyrics", run: function () { viewSong(r.i, true); } });
    }
    if (page) items.push({ label: t ? "Song page and downloads" : "Chart and downloads", href: page });
    return items;
  }
  function fillMenu(box, row) {
    box.textContent = "";
    if (box.classList.contains("site-row-menu-sheet")) {
      var head = doc.createElement("p");
      head.className = "site-row-menu-title";
      head.textContent = row.getAttribute("data-title") || "";
      box.appendChild(head);
    }
    menuItems(row).forEach(function (it) {
      var item = doc.createElement(it.href ? "a" : "button");
      item.className = "site-row-menu-item";
      item.setAttribute("role", "menuitem");
      item.setAttribute("tabindex", "-1");
      if (it.href) item.setAttribute("href", it.href);
      else item.setAttribute("type", "button");
      item.textContent = it.label;
      item.addEventListener("click", function () {
        closeMenu(false);
        if (it.run) it.run();
      });
      box.appendChild(item);
    });
  }
  function menuMove(by) {
    if (!menu) return;
    var items = Array.prototype.slice.call(menu.box.querySelectorAll(".site-row-menu-item"));
    if (!items.length) return;
    var at = items.indexOf(doc.activeElement);
    var next = items[(at + by + items.length) % items.length];
    if (next && next.focus) next.focus();
  }
  function openMenu(button, row) {
    closeMenu(false);
    var sheet_ = !deskMenu.matches;
    var box = doc.createElement("div");
    box.className = "site-row-menu" + (sheet_ ? " site-row-menu-sheet" : "");
    box.setAttribute("role", "menu");
    box.setAttribute("aria-label", row.getAttribute("data-title") || "Song");
    var scrim = null;
    if (sheet_) {
      scrim = doc.createElement("div");
      scrim.className = "site-row-menu-scrim";
      scrim.addEventListener("click", function () { closeMenu(); });
      doc.body.appendChild(scrim);
    }
    fillMenu(box, row);
    box.addEventListener("keydown", function (ev) {
      if (ev.key === "ArrowDown") { ev.preventDefault(); menuMove(1); }
      else if (ev.key === "ArrowUp") { ev.preventDefault(); menuMove(-1); }
      else if (ev.key === "Escape") { ev.preventDefault(); closeMenu(); }
      else if (ev.key === "Tab") closeMenu(false);
    });
    doc.body.appendChild(box);
    if (!sheet_ && button.getBoundingClientRect) {
      var at = button.getBoundingClientRect();
      box.style.top = Math.round(at.bottom + (window.scrollY || 0) + 4) + "px";
      box.style.left = Math.round(Math.max(8, at.right + (window.scrollX || 0) - 224)) + "px";
    }
    menu = { box: box, scrim: scrim, button: button, row: row };
    button.setAttribute("aria-expanded", "true");
    menuMove(1);
    // Whether it has lyrics is known once its page is read.
    var r = rows.filter(function (x) { return x.el === row; })[0];
    var t = r ? tracks[r.i] : null;
    if (t && hasView(t) && !known[t.src]) {
      songOf(t).then(function () {
        if (menu && menu.row === row) fillMenu(box, row);
      });
    }
  }
  Array.prototype.forEach.call(doc.querySelectorAll("[data-playlist] [data-track-more]"), function (b) {
    var row = b.closest("li");
    if (!row) return;
    b.setAttribute("aria-haspopup", "menu");
    b.setAttribute("aria-expanded", "false");
    b.addEventListener("click", function () {
      if (menu && menu.button === b) closeMenu();
      else openMenu(b, row);
    });
  });
  doc.addEventListener("click", function (ev) {
    if (!menu || !ev.target || !ev.target.closest) return;
    if (ev.target.closest(".site-row-menu") || ev.target.closest("[data-track-more]") === menu.button) return;
    closeMenu(false);
  }, true);
  doc.addEventListener("keydown", function (ev) {
    if (menu && ev.key === "Escape") closeMenu();
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
      '<div class="site-now-what"><span class="site-now-title"></span><span class="site-now-note"></span></div>' +
      '<button type="button" class="site-now-lyrics" data-act="lyrics" aria-pressed="false" aria-label="Lyrics" title="Lyrics" hidden>' + ICON.words + "</button>" +
      '<a class="site-now-page" aria-label="Song page and downloads" title="Song page and downloads">' + ICON.page + "</a></div>" +
      '<div class="site-now-island site-now-island-top" hidden></div>' +
      '<div class="site-now-progress"><span class="site-now-at">0:00</span><div class="site-now-seek" role="slider" tabindex="0" aria-label="Position" aria-valuemin="0"><span></span></div><span class="site-now-left">–:––</span></div>' +
      '<div class="site-now-body"></div>' +
      // An island's transport takes the middle of the controls' row: Click
      // and Loop stay at its ends.
      '<div class="site-now-controls"><div class="site-now-buttons">' +
      toggles().replace("</button>", "</button>" + '<div class="site-now-island site-now-island-bottom" hidden></div><div class="site-now-transport">' + controls() + "</div>") + "</div></div>";
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
    var got = pageDoc(page);
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
  // Page `page` (one this link built), fetched once and parsed (inert: a
  // parsed document runs nothing); a failure is not kept.
  function pageDoc(page) {
    return fetched[page] || (fetched[page] = fetch(page, { credentials: "same-origin", redirect: "error", headers: { Accept: "text/html" } })
      .then(function (r) {
        var type = r.headers.get("content-type") || "";
        if (!r.ok) throw new Error(r.status === 503 ? "Its page is being built: try again in a moment." : "Its page could not be loaded (" + r.status + ").");
        if (r.headers.get("x-architect-page") !== "built" || !/^text\/html/i.test(type)) throw new Error("Its page is not one this link built.");
        return r.text();
      })
      .then(function (html) { return new DOMParser().parseFromString(html, "text/html"); }));
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
  // ── An island's web bundle (any browser) ─────────────────────────
  // Its views as a plain web bundle, each in a frame of the node's web
  // view document (`/_ext/webview.<hash>.html`: sandboxed, an opaque
  // origin, nothing to fetch or connect to, its own CSP; this page's is
  // unchanged). This page fetches the bundle's files from the node, checks
  // each against its digest, and hands them in as bytes with its theme and
  // where the song is; the frame may only ask this page's intents (the
  // island's, checked and limited here), say its height and that it is
  // ready. Used whenever the island has one, so its views are the same on
  // every browser; an island without one runs as a component (above).
  var webBytes = null;
  function webOf(it) {
    var w = it && it.web;
    if (!ext || !ext.webview || !w || typeof w.entry !== "string" || !w.files || typeof w.files !== "object") return null;
    var names = Object.keys(w.files);
    var flat = /^web-[A-Za-z0-9._-]+\.(js|wasm)$/;
    var pinned = /^sha256-[A-Za-z0-9+/]{43}=$/;
    if (!names.length || names.length > 64 || names.indexOf(w.entry) < 0 || !/\.js$/.test(w.entry)) return null;
    if (names.some(function (n) { return !flat.test(n) || !pinned.test(String(w.files[n])); })) return null;
    if (names.filter(function (n) { return /\.wasm$/.test(n); }).length !== 1) return null;
    // Its frame's document, as the node chose it (the default, or the one
    // that allows WebRTC for an island declaring and granted it).
    var page = typeof w.webview === "string" ? w.webview : ext.webview;
    if (!page || !/^\/_ext\/webview(-rtc)?\.[0-9a-f]+\.html$/.test(page)) return null;
    w.webview = page;
    return w;
  }
  function webFiles(it, w) {
    if (webBytes) return webBytes;
    if (!(window.crypto && window.crypto.subtle)) return Promise.reject(new Error("no digests here"));
    var names = Object.keys(w.files);
    webBytes = Promise.all(names.map(function (name) {
      return fetch(it.folder + name, { credentials: "omit", redirect: "error" })
        .then(function (r) {
          if (!r.ok) throw new Error(name + " did not load");
          return r.arrayBuffer();
        })
        .then(function (bytes) {
          return window.crypto.subtle.digest("SHA-256", bytes).then(function (d) {
            var b = "";
            new Uint8Array(d).forEach(function (x) { b += String.fromCharCode(x); });
            if ("sha256-" + btoa(b) !== w.files[name]) throw new Error(name + " is not the file pinned");
            return [name, bytes];
          });
        });
    })).then(function (pairs) {
      var out = {};
      pairs.forEach(function (p) { out[p[0]] = p[1]; });
      return out;
    }, function (e) {
      webBytes = null;
      throw e;
    });
    return webBytes;
  }
  // The page's theme as a frame takes it: its custom properties (no URL in
  // any), and light or dark.
  function themeOf() {
    var vars = {}, n = 0;
    try {
      var cs = window.getComputedStyle(doc.documentElement);
      for (var i = 0; i < cs.length && n < 400; i++) {
        var k = cs[i];
        if (String(k).indexOf("--") !== 0) continue;
        var v = String(cs.getPropertyValue(k)).trim();
        if (!v || v.length > 200 || /url\s*\(|expression|[<>]/i.test(v)) continue;
        vars[k] = v;
        n++;
      }
    } catch (e) {}
    var set = doc.documentElement.getAttribute("data-theme");
    var dark = set ? set === "dark" : !!(window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches);
    return { vars: vars, scheme: dark ? "dark" : "light" };
  }
  // How long a frame has to say it is ready, and to answer a heartbeat.
  var WEB_READY_MS = 15000, WEB_BEAT_MS = 2000, WEB_LATE_MS = 6000;
  // The views whose frame died on this page: never booted again here.
  var webDead = {};
  // View `view` of the island's web bundle in `slot`; the player's own
  // `mine` gives way once it is ready, and comes back for good if the
  // frame leaves its document (a second load), fails (an error), is not
  // ready in time or stops answering the heartbeat.
  function webPane(it, w, view, slot, mine) {
    var frame = doc.createElement("iframe");
    frame.setAttribute("sandbox", "allow-scripts");
    frame.setAttribute("title", view);
    frame.className = "site-now-web";
    var chan = null, dead = false, ready = false, loads = 0, hello = false;
    var asks = intents();
    var beat = { sent: 0, answered: 0, n: 0, timer: 0 };
    var pane = {
      input: function (msg) {
        if (chan && !dead) chan.port1.postMessage({ t: "props", props: msg.props });
      },
      unmount: function () {
        dead = true;
        clearTimeout(readyBy);
        clearInterval(beat.timer);
        if (chan) { try { chan.port1.close(); } catch (e) {} }
        if (frame.parentNode) frame.parentNode.removeChild(frame);
      },
    };
    // Gone: the pane out, the slot hidden, the player's own back.
    function die(why) {
      if (dead) return;
      island.failed = why;
      webDead[view] = true;
      pane.unmount();
      island.panes = island.panes.filter(function (p) { return p !== pane; });
      slot.hidden = true;
      mine.classList.remove("site-now-yielded");
    }
    var readyBy = setTimeout(function () { if (!ready) die("not ready in time"); }, WEB_READY_MS);
    frame.addEventListener("load", function () {
      loads += 1;
      // Its one document, loaded once: any other load is the frame gone
      // somewhere else.
      if (loads > 1) { die("it left its document"); return; }
      if (dead || chan || !frame.contentWindow) return;
      chan = new MessageChannel();
      chan.port1.onmessage = function (m) {
        var d = m.data || {};
        if (dead) return;
        if (d.t === "hello") {
          // Answered once: its files are copied and booted once.
          if (hello) return;
          hello = true;
          var th = themeOf();
          chan.port1.postMessage({ t: "theme", vars: th.vars, scheme: th.scheme });
          var sheet_ = it.stylesheet ? pinnedSheet(it).catch(function () { return null; }) : Promise.resolve(null);
          Promise.all([webFiles(it, w), sheet_]).then(function (both) {
            if (dead) return;
            var files = {};
            Object.keys(both[0]).forEach(function (k) { files[k] = both[0][k].slice(0); });
            chan.port1.postMessage({ t: "boot", entry: w.entry, view: view, files: files, sheet: both[1] });
          }, function (e) {
            die(String((e && e.message) || e));
          });
        } else if (d.t === "ready") {
          if (ready) return;
          ready = true;
          clearTimeout(readyBy);
          slot.hidden = false;
          mine.classList.add("site-now-yielded");
          feed(snapshot(), true);
          // The heartbeat, while it shows: unanswered too long, it is gone.
          beat.timer = setInterval(function () {
            if (dead) return;
            if (beat.sent > beat.answered && Date.now() - beat.at > WEB_LATE_MS) { die("it stopped answering"); return; }
            if (beat.sent === beat.answered) {
              beat.sent += 1;
              beat.at = Date.now();
              chan.port1.postMessage({ t: "ping", n: beat.sent });
            }
          }, WEB_BEAT_MS);
        } else if (d.t === "pong") {
          if (Number(d.n) === beat.sent) beat.answered = beat.sent;
        } else if (d.t === "size") {
          var h = Number(d.height);
          if (isFinite(h)) frame.style.height = Math.max(0, Math.min(Math.round(h), 4000)) + "px";
        } else if (d.t === "ask") {
          var a = Object.prototype.hasOwnProperty.call(asks, d.action) ? asks[d.action] : null;
          if (a) { try { a.run(d.args); } catch (e) {} }
        } else if (d.t === "error") {
          die(String(d.message || "it failed"));
        }
      };
      frame.contentWindow.postMessage("architect-web", "*", [chan.port2]);
    });
    frame.setAttribute("src", w.webview);
    slot.appendChild(frame);
    return pane;
  }
  function islandOpen() {
    var it = islandOf();
    if (!it) return;
    clearTimeout(island.idle);
    var w = webOf(it);
    if (w) {
      if (island.panes.length) return;
      [[it.progress, ui.islandTop, ui.progress], [it.transport, ui.islandBottom, ui.controls]].forEach(function (slot) {
        if (slot[0] && !webDead[slot[0]]) island.panes.push(webPane(it, w, slot[0], slot[1], slot[2]));
      });
      return;
    }
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


  // ── A song's beats and sections (Click, Loop) ─────────────────────
  // Read from its view (`[data-now-playing]`), on this page or on its own
  // page (fetched once, as the view is): its beats, the `[data-at]` marks
  // with a `data-beat` (1 is a bar's first), and its `data-sections`.
  // Numbers only: nothing of the fetched page is put on this one.
  var songs = {}, known = {};
  // The closest two beats may be; the shortest section Loop will hold.
  var MIN_BEAT = 0.05, MIN_LOOP = 0.25;
  function readSong(node) {
    var seen = {};
    var beats = Array.prototype.map.call(node.querySelectorAll("[data-at][data-beat]"), function (n) {
      return { at: parseFloat(n.getAttribute("data-at")), one: n.getAttribute("data-beat") === "1" };
    }).filter(function (b) {
      if (!isFinite(b.at) || b.at < 0 || seen[b.at]) return false;
      seen[b.at] = true;
      return true;
    }).sort(function (a, b) { return a.at - b.at; });
    // A beat closer than this to the one before is not a beat: a chart
    // (or a note) marking beats a millisecond apart would ask for a
    // thousand clicks a second.
    var spaced = [], last = -Infinity;
    beats.forEach(function (b) {
      if (b.at - last >= MIN_BEAT) {
        spaced.push(b);
        last = b.at;
      }
    });
    return { beats: spaced.slice(0, 20000), sections: sectionsOf(node), lyrics: false };
  }
  var NONE = { beats: [], sections: [] };
  function songOf(t) {
    if (!t) return Promise.resolve(NONE);
    if (songs[t.src]) return songs[t.src];
    var mine = viewIn(doc, t.src);
    var withWords = function (x, root, from) {
      x.lyrics = !!lyricsIn(root, t.src, from);
      return x;
    };
    var got = mine ? Promise.resolve(withWords(readSong(mine), doc))
      : t.view && t.page && t.page !== here ? pageDoc(t.page).then(function (parsed) {
        var from = new URL(t.page, location.href).href;
        var found = viewIn(parsed, t.src, from);
        return found ? withWords(readSong(found), parsed, from) : NONE;
      }) : Promise.resolve(NONE);
    songs[t.src] = got.then(function (x) {
      known[t.src] = x;
      paintOpts();
      return x;
    }, function () {
      // Its page did not load: no beats, no sections, for this visit
      // (asked again would be every frame).
      known[t.src] = NONE;
      return NONE;
    });
    return songs[t.src];
  }
  function songNow() {
    var t = tracks[cur];
    return (t && known[t.src]) || NONE;
  }

  // ── Click ─────────────────────────────────────────────────────────
  // A tick on each beat, scheduled a little ahead on the Web Audio clock
  // from where the media is (its time, its rate), never a timer a beat.
  // A jump (a seek, a loop back, a rate change, another song) silences
  // what was scheduled and starts again from there.
  var CLICK_AHEAD = 0.15, CLICK_EVERY = 25, CLICK_JUMP = 0.25;
  var metro = { ctx: null, timer: 0, next: -1, at: null, src: null, voices: [] };
  // The audio context, made (and let sound) inside a tap: iOS starts one
  // only there.
  function wake() {
    if (!prefs.click) return;
    var AC = window.AudioContext || window.webkitAudioContext;
    if (!metro.ctx && AC) {
      try { metro.ctx = new AC(); } catch (e) { metro.ctx = null; }
    }
    var c = metro.ctx;
    if (!c) return;
    if (c.state === "suspended" && c.resume) {
      try { c.resume(); } catch (e) {}
    }
    try {
      var b = c.createBufferSource();
      b.buffer = c.createBuffer(1, 1, 22050);
      b.connect(c.destination);
      b.start(0);
    } catch (e) {}
  }
  function hush() {
    metro.voices.forEach(function (v) { try { v.stop(0); } catch (e) {} });
    metro.voices = [];
    metro.next = -1;
    metro.at = null;
  }
  function voice(when, accent) {
    var c = metro.ctx;
    var o = c.createOscillator(), g = c.createGain();
    o.frequency.value = accent ? 1760 : 1175;
    g.gain.setValueAtTime(0.0001, when);
    g.gain.exponentialRampToValueAtTime(accent ? 0.5 : 0.3, when + 0.002);
    g.gain.exponentialRampToValueAtTime(0.0001, when + 0.045);
    o.connect(g);
    g.connect(c.destination);
    o.start(when);
    o.stop(when + 0.05);
    metro.voices.push(o);
    o.onended = function () { metro.voices = metro.voices.filter(function (v) { return v !== o; }); };
  }
  // The index of the first of `beats` at or after `t`.
  function beatFrom(beats, t) {
    var lo = 0, hi = beats.length;
    while (lo < hi) {
      var mid = (lo + hi) >> 1;
      if (beats[mid].at < t - 0.005) lo = mid + 1; else hi = mid;
    }
    return lo;
  }
  function clickTick() {
    var c = metro.ctx, s = snapshot();
    var beats = s.src && known[s.src] ? known[s.src].beats : [];
    if (!prefs.click || !c || c.state !== "running" || !s.playing || !beats.length) {
      if (metro.voices.length || metro.at) hush();
      return;
    }
    var now = c.currentTime, rate = s.rate || 1;
    if (metro.src !== s.src) { hush(); metro.src = s.src; }
    if (metro.at) {
      var expect = metro.at.time + (now - metro.at.ctx) * metro.at.rate;
      if (Math.abs(s.time - expect) > CLICK_JUMP || rate !== metro.at.rate) hush();
    }
    if (metro.next < 0) metro.next = beatFrom(beats, s.time);
    while (metro.next < beats.length) {
      var b = beats[metro.next];
      var dt = (b.at - s.time) / rate;
      if (dt > CLICK_AHEAD) break;
      metro.next++;
      if (dt >= -0.03) voice(now + Math.max(0, dt), b.one);
    }
    metro.at = { time: s.time, ctx: now, rate: rate };
  }
  function clickRun(on) {
    if (on && !metro.timer) metro.timer = setInterval(clickTick, CLICK_EVERY);
    if (!on && metro.timer) {
      clearInterval(metro.timer);
      metro.timer = 0;
    }
    if (!on) hush();
  }
  function clickToggle() {
    prefs.click = !prefs.click;
    if (prefs.click) wake();
    clickRun(prefs.click);
    save(true);
    paintOpts();
  }

  // ── Loop ──────────────────────────────────────────────────────────
  // The section the playhead is in, again and again: at its end, back to
  // its start. A seek out of it loops the one it lands in.
  var loop = { src: null, sec: null };
  function sectionAt(list, t) {
    var found = null;
    for (var i = 0; i < list.length; i++) if (list[i].start <= t + 0.001) found = list[i];
    return found;
  }
  function looped() {
    var t = tracks[cur];
    return prefs.loop && t && loop.src === t.src ? loop.sec : null;
  }
  // The sections Loop can hold: long enough to play (a section of a
  // few milliseconds would seek every frame), in order of their start.
  function loopable(src) {
    var list = src && known[src] ? known[src].sections : [];
    return list.filter(function (x) { return x.end - x.start >= MIN_LOOP; })
      .sort(function (a, b) { return a.start - b.start; });
  }
  var checking = false;
  function loopCheck(s) {
    // Its own seek tells it where the song is again: not from in here.
    if (checking || !prefs.loop || source || !s.src) return;
    var list = loopable(s.src);
    if (!list.length) return;
    checking = true;
    try {
      if (loop.src !== s.src || !loop.sec) loop = { src: s.src, sec: sectionAt(list, s.time) };
      var sec = loop.sec;
      if (!sec) return;
      if (s.time >= sec.end - 0.02 && s.time < sec.end + 0.75) {
        if (s.playing) playhead.seek(sec.start);
        return;
      }
      if (s.time < sec.start - 0.05 || s.time >= sec.end + 0.75) loop.sec = sectionAt(list, s.time);
    } finally {
      checking = false;
    }
  }
  function loopToggle() {
    prefs.loop = !prefs.loop;
    loop = { src: null, sec: null };
    save(true);
    paintOpts();
  }
  playhead.subscribe(loopCheck);

  // The toggles as they are: pressed when on, and of no use (disabled)
  // for a song with no beats (Click) or no sections (Loop).
  function paintOpts() {
    var song = songNow();
    var opts = [
      ["click", prefs.click, song.beats.length > 0, "Click: a beat with the song", "Click: this song's chart has no beats"],
      // Loop holds the page's player: not while an outside clock does.
      ["loop", prefs.loop, !source && loopable((tracks[cur] || {}).src).length > 0, "Loop the section playing", source ? "Loop: the song is played elsewhere" : "Loop: this song's chart has no sections"],
    ];
    opts.forEach(function (o) {
      Array.prototype.forEach.call(doc.querySelectorAll('.site-player [data-act="' + o[0] + '"], .site-now [data-act="' + o[0] + '"]'), function (b) {
        b.setAttribute("aria-pressed", o[1] ? "true" : "false");
        if (o[2]) b.removeAttribute("disabled"); else b.setAttribute("disabled", "");
        b.setAttribute("title", o[2] ? o[3] : o[4]);
      });
    });
  }

  // The card's art: the song's, else its row's tile, else one drawn from
  // its title; the card tinted from it.
  // The track whose art the card shows (`undefined`: none painted yet).
  var artFor;
  function hue(text) {
    var h = 0x811c9dc5;
    var t = String(text || "").toLowerCase();
    for (var i = 0; i < t.length; i++) {
      h ^= t.charCodeAt(i);
      h = Math.imul(h, 0x01000193) >>> 0;
    }
    return h % 360;
  }
  function paintArt(t) {
    if (!ui.art || artFor === t) return;
    artFor = t;
    ui.art.textContent = "";
    ui.art.removeAttribute("style");
    bar.style.removeProperty("--site-player-tint");
    // A music note under any art, on the theme's muted tile with nothing
    // loaded, on the song's colour with one: never letters, never what a
    // row's tile says (its key, on a setlist), never an empty square.
    ui.art.innerHTML = ICON.note;
    ui.art.setAttribute("data-glyph", "");
    if (!t) return;
    var tile = t.tile;
    var h = tile && isFinite(parseInt(tile.getAttribute("data-hue"), 10)) ? parseInt(tile.getAttribute("data-hue"), 10) : hue(t.title);
    ui.art.style.setProperty("background", "hsl(" + h + " 42% 40%)");
    bar.style.setProperty("--site-player-tint", "hsl(" + h + " 50% 45%)");
    var src = t.art || (t.page && artBy[t.page]) || null;
    if (src) {
      var img = showArt(ui.art, src);
      img.addEventListener("load", function () { tintFrom(img); });
    }
  }
  // The card tinted from its art's colours (its own file, same origin).
  function tintFrom(img) {
    try {
      var c = doc.createElement("canvas");
      c.width = c.height = 6;
      var x = c.getContext("2d");
      x.drawImage(img, 0, 0, 6, 6);
      var d = x.getImageData(0, 0, 6, 6).data, r = 0, g = 0, b = 0, n = d.length / 4;
      for (var i = 0; i < d.length; i += 4) { r += d[i]; g += d[i + 1]; b += d[i + 2]; }
      bar.style.setProperty("--site-player-tint", "rgb(" + Math.round(r / n) + " " + Math.round(g / n) + " " + Math.round(b / n) + ")");
    } catch (e) {}
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
      var on = s.index >= 0 ? s.index === r.i : s.src === r.src;
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
    if (shell) {
      paintArt(t);
      if (t && !songs[t.src]) songOf(t);
    }
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

  // Resume where the last page left off (Click waits for a tap to sound).
  if (prefs.click) clickRun(true);
  paintOpts();
  var at = saved.src ? indexOf(path(saved.src)) : -1;
  if (at >= 0) select(at, saved.time || 0, !!saved.playing, true);
  else emit(true);
  window.addEventListener("pagehide", function () { save(true); });
})();
