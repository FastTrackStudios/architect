// `architect.playhead` with an outside clock attached (#528): the page's
// player (`features/sites/assets/player.js`) run under node in a small
// fake DOM, an engine standing in as a fake source. Run by
// `features/sites/tests/player_source.rs` (and `node --test` on this file).
//
// - Without a source, the page's media drives the playhead (as today).
// - Attached, the source's clock drives it: the chart's and the lyrics'
//   `[data-at]` lines follow it each frame, the page's media is paused
//   and its bar hidden, and seeks, plays and pauses go to the source.
// - Detached, the page's player takes over where the source was.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const PLAYER = readFileSync(process.env.ARCHITECT_PLAYER_JS || join(here, '../assets/player.js'), 'utf8');

// ── A DOM just big enough for the player ────────────────────────────

class Node {
  constructor(tag, attrs = {}) {
    this.tagName = tag.toUpperCase();
    this.attrs = { ...attrs };
    this.children = [];
    this.parentElement = null;
    this.listeners = {};
    this.textContent = '';
    this.hidden = false;
    this.style = {};
    const classes = new Set();
    this.classList = {
      add: (c) => classes.add(c),
      remove: (c) => classes.delete(c),
      toggle: (c, on) => ((on ?? !classes.has(c)) ? classes.add(c) : classes.delete(c)),
      contains: (c) => classes.has(c),
    };
    this.stubs = {};
  }
  getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; }
  setAttribute(k, v) { this.attrs[k] = String(v); }
  removeAttribute(k) { delete this.attrs[k]; }
  hasAttribute(k) { return k in this.attrs; }
  appendChild(c) { if (c.parentElement) c.parentElement.removeChild(c); c.parentElement = this; this.children.push(c); return c; }
  get parentNode() { return this.parentElement; }
  insertBefore(c, ref) {
    if (c.parentElement) c.parentElement.removeChild(c);
    c.parentElement = this;
    const at = this.children.indexOf(ref);
    this.children.splice(at < 0 ? this.children.length : at, 0, c);
    return c;
  }
  removeChild(c) { this.children = this.children.filter((x) => x !== c); c.parentElement = null; return c; }
  addEventListener(name, fn, opts) {
    (this.listeners[name] ||= []).push({ fn, once: !!(opts && opts.once) });
  }
  dispatch(name, ev = {}) {
    const ls = this.listeners[name] || [];
    this.listeners[name] = ls.filter((l) => !l.once);
    ls.forEach((l) => l.fn({ target: this, ...ev }));
  }
  contains(n) { for (; n; n = n.parentElement) if (n === this) return true; return false; }
  // Selectors: `[a]`, `[a][b]`, and those joined by a space (inside).
  matches(compound) {
    const attrs = compound.match(/^(\[[\w-]+\])+$/) ? [...compound.matchAll(/\[([\w-]+)\]/g)].map((m) => m[1]) : null;
    return !!attrs && attrs.every((a) => this.hasAttribute(a));
  }
  closest(sel) {
    for (let n = this; n; n = n.parentElement) if (n.matches(sel)) return n;
    return null;
  }
  all() { return this.children.flatMap((c) => [c, ...c.all()]); }
  querySelectorAll(sel) {
    const parts = sel.trim().split(/\s+/);
    const last = parts.pop();
    return this.all().filter((n) => {
      if (!n.matches(last)) return false;
      let from = n.parentElement;
      for (let i = parts.length - 1; i >= 0; i--) {
        while (from && from !== this && !from.matches(parts[i])) from = from.parentElement;
        if (!from || from === this) return false;
        from = from.parentElement;
      }
      return true;
    });
  }
  querySelector(sel) {
    // The bar's parts: stand-ins, one per selector.
    return (this.stubs[sel] ||= new Node('span'));
  }
  set innerHTML(_) {}
  getBoundingClientRect() { return { top: 0, left: 0, width: 10, height: 10 }; }
  scrollTo() {}
}

class Media extends Node {
  constructor(src) {
    super('audio', { src, 'data-site-media': '' });
    this.currentTime = 0;
    this.paused = true;
    this.ended = false;
    this.playbackRate = 1;
    this.duration = 60;
    this.readyState = 1;
    this.preload = 'metadata';
  }
  play() { this.paused = false; this.dispatch('play'); return Promise.resolve(); }
  pause() { if (!this.paused) { this.paused = true; this.dispatch('pause'); } }
  load() {}
}

/// A page at `/s/x/joy/` with `media` and a chart following the
/// playhead (a bar every 2 s), run through the player.
function page({ media = ['/s/x/joy.mp3'], chartMedia = null, build = null } = {}) {
  const body = new Node('body');
  if (build) build(body);
  const players = media.map((src) => body.appendChild(new Media(src)));
  const chart = body.appendChild(new Node('div', { 'data-follow-playhead': '' }));
  if (chartMedia) chart.setAttribute('data-media', chartMedia);
  const bars = [0, 2, 4, 6, 8].map((at) => chart.appendChild(new Node('rect', { 'data-at': String(at) })));
  const events = [];
  const frames = [];
  const store = new Map();
  const document = {
    body,
    documentElement: new Node('html'),
    querySelector: () => null,
    querySelectorAll: (sel) => body.querySelectorAll(sel),
    createElement: (tag) => (tag === 'audio' ? new Media('') : new Node(tag)),
    dispatchEvent: (ev) => events.push(ev.detail),
    addEventListener: () => {},
    createComment: () => new Node('#comment'),
  };
  const window = {
    document,
    location: { href: 'http://node.test/s/x/joy/' },
    history: { state: null, pushState() {}, back() {} },
    sessionStorage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) },
    requestAnimationFrame: (fn) => frames.push(fn),
    cancelAnimationFrame: () => {},
    addEventListener: () => {},
    getComputedStyle: () => ({ overflowX: 'visible', overflowY: 'visible' }),
    matchMedia: () => ({ matches: true }),
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    URL,
    console,
  };
  Object.assign(window, { window, sessionStorage: window.sessionStorage });
  vm.runInNewContext(PLAYER, window);
  const frame = () => frames.splice(0).forEach((f) => f());
  const current = () => bars.findIndex((b) => b.getAttribute('aria-current') === 'time');
  return { window, playhead: window.architect.playhead, players, chart, bars, events, frame, current, body };
}

/// An engine's clock: a time it is told, playing or not, recording what
/// the page asks of it.
function engine(extra = {}) {
  const asked = [];
  const e = {
    name: 'test.engine',
    at: 0,
    on: false,
    asked,
    time() { return e.at; },
    playing() { return e.on; },
    duration: 120,
    seek(t) { asked.push(['seek', t]); e.at = t; },
    play() { asked.push(['play']); e.on = true; },
    pause() { asked.push(['pause']); e.on = false; },
    ...extra,
  };
  return e;
}

/// An object of the page's realm as plain data.
const plain = (o) => JSON.parse(JSON.stringify(o));

// ── The tests ────────────────────────────────────────────────────────

test('without a source the page media drives the playhead', () => {
  const p = page();
  const [audio] = p.players;
  audio.play();
  audio.currentTime = 4.2;
  audio.dispatch('timeupdate');
  assert.equal(p.current(), 2);
  assert.equal(p.playhead.snapshot().source, null);
});

test('an attached source drives the followers each frame, and the page media yields', () => {
  const p = page();
  const [audio] = p.players;
  audio.play();
  audio.currentTime = 2.5;
  audio.dispatch('timeupdate');
  const e = engine();
  const handle = p.playhead.attach(e);
  assert.equal(audio.paused, true, 'the page media is paused');
  assert.deepEqual(plain(handle.from), { src: '/s/x/joy.mp3', time: 2.5, playing: true }, 'where the page was');
  e.at = 6.1;
  e.on = true;
  handle.changed();
  assert.equal(p.current(), 3);
  let s = p.playhead.snapshot();
  assert.equal(s.source, 'test.engine');
  assert.equal(s.playing, true);
  assert.equal(s.duration, 120);
  assert.equal(p.playhead.time, 6.1);
  // The clock moves on with no call: read each frame while it plays.
  e.at = 8.0;
  p.frame();
  assert.equal(p.current(), 4);
  assert.equal(p.events.at(-1).time, 8);
  // The page media no longer drives it, and played by hand is paused.
  audio.currentTime = 0.5;
  audio.dispatch('timeupdate');
  assert.equal(p.current(), 4);
  audio.play();
  assert.equal(audio.paused, true);
  assert.equal(p.current(), 4);
});

test('seeks, plays and pauses go to the source; a click on a bar too', () => {
  const p = page();
  const e = engine();
  p.playhead.attach(e);
  p.playhead.play();
  p.playhead.seek(3);
  p.playhead.pause();
  p.chart.dispatch('click', { target: p.bars[2] });
  p.playhead.next();
  p.playhead.previous();
  assert.deepEqual(e.asked, [['play'], ['seek', 3], ['pause'], ['seek', 4]]);
  assert.equal(p.players[0].paused, true, 'the page media was never started');
  assert.equal(p.current(), 2);
});

test('a block for one media file follows a source that stands in for it alone', () => {
  const other = page({ chartMedia: '/s/x/other.mp3' });
  const h = other.playhead.attach(engine({ at: 4 }));
  h.changed();
  assert.equal(other.current(), -1);
  const mine = page({ chartMedia: '/s/x/joy-reference.ogg' });
  const m = mine.playhead.attach(engine({ at: 4, src: '/s/x/joy-reference.ogg' }));
  m.changed();
  assert.equal(mine.current(), 2);
  assert.equal(mine.playhead.src, '/s/x/joy-reference.ogg');
});

test('detached, the page player takes over where the source was', () => {
  const p = page();
  const e = engine({ src: '/s/x/joy.mp3' });
  const handle = p.playhead.attach(e);
  e.at = 4.5;
  e.on = true;
  handle.changed();
  handle.detach();
  const s = p.playhead.snapshot();
  assert.equal(s.source, null);
  assert.equal(p.players[0].currentTime, 4.5);
  assert.equal(s.playing, false, 'paused: the engine stopped, nothing restarts on its own');
  // The page media drives it again.
  p.players[0].currentTime = 0.1;
  p.players[0].dispatch('timeupdate');
  assert.equal(p.current(), 0);
  // A stale handle does nothing.
  e.at = 8;
  handle.changed();
  assert.equal(p.current(), 0);
});

test('a second source replaces the first, which is told', () => {
  const p = page();
  let told = 0;
  const first = p.playhead.attach(engine({ detached() { told += 1; } }));
  const second = engine({ at: 2 });
  const h = p.playhead.attach(second);
  h.changed();
  assert.equal(told, 1);
  assert.equal(p.current(), 1);
  first.detach();
  assert.equal(p.playhead.snapshot().source, 'test.engine', 'the old handle detaches nothing');
});

test('the bar hides while a source holds the clock', () => {
  const p = page({ media: ['/s/x/a.mp3', '/s/x/b.mp3'] });
  const bar = p.body.children.find((c) => c.className === 'site-player');
  assert.ok(bar, 'two players on a page: a bar');
  const h = p.playhead.attach(engine());
  assert.equal(bar.hidden, true);
  assert.equal(p.body.classList.contains('site-has-player'), false);
  h.detach();
  assert.equal(bar.hidden, false);
  assert.equal(p.body.classList.contains('site-has-player'), true);
});

test('a source needs a clock and a seek', () => {
  const p = page();
  // (A TypeError of the page's realm.)
  assert.throws(() => p.playhead.attach({ time: 0 }), /a time and a seek/);
  assert.throws(() => p.playhead.attach({ seek() {} }), /a time and a seek/);
});

// ── A playlist: the page is the player ──────────────────────────────

/// A setlist's page: a playlist of three songs, each row naming its
/// recording and the page that shows it.
function setlist() {
  const rows = [];
  const p = page({
    media: [],
    build: (body) => {
      const list = body.appendChild(new Node('div', { 'data-playlist': '' }));
      for (const name of ['sleigh', 'silent', 'joy']) {
        rows.push(list.appendChild(new Node('li', {
          'data-track': '', 'data-media': `/s/x/${name}.mp4`, 'data-page': `/s/x/${name}/`, 'data-title': name,
        })));
      }
    },
  });
  p.window.location.href = 'http://node.test/s/x/setlist/';
  return { ...p, rows };
}

test('a playlist is the queue: a row plays where the page is, the next follows without leaving it', () => {
  const { playhead, rows, window, events } = setlist();
  const at = window.location.href;
  rows[1].dispatch('click', { target: rows[1] });
  let s = events.at(-1);
  assert.equal(s.src, '/s/x/silent.mp4', 'the row tapped plays');
  assert.equal(s.tracks, 3, 'its rows are the queue');
  assert.equal(rows[1].getAttribute('aria-current'), 'true', 'and is marked');
  assert.equal(rows[0].getAttribute('aria-current'), null);
  // A seek naming no track is in the one playing, not the page's first.
  playhead.seek(30);
  assert.equal(playhead.src, '/s/x/silent.mp4');
  playhead.next();
  assert.equal(playhead.src, '/s/x/joy.mp4', 'the next row');
  assert.equal(window.location.href, at, 'no page load');
  assert.equal(rows[2].getAttribute('aria-current'), 'true');
  playhead.previous();
  s = events.at(-1);
  assert.ok(['/s/x/joy.mp4', '/s/x/silent.mp4'].includes(s.src));
  assert.equal(window.location.href, at);
});


test('a row that is also a follow box (written for a page without playlists) plays once, in place', () => {
  const rows = [];
  const lines = [];
  const p = page({
    media: [],
    build: (body) => {
      const list = body.appendChild(new Node('div', { 'data-playlist': '' }));
      for (const name of ['sleigh', 'silent']) {
        const row = list.appendChild(new Node('li', {
          'data-track': '', 'data-follow-playhead': '', 'data-media': `/s/x/${name}.mp4`, 'data-page': `/s/x/${name}/`, 'data-title': name,
        }));
        rows.push(row);
        lines.push(row.appendChild(new Node('span', { 'data-at': '0' })));
      }
    },
  });
  p.window.location.href = 'http://node.test/s/x/setlist/';
  // The click reaches the line's follow box (the row) and the row: one
  // of them plays it, and it is not toggled back off.
  rows[1].dispatch('click', { target: lines[1] });
  assert.equal(p.playhead.src, '/s/x/silent.mp4');
  assert.equal(p.playhead.playing, true, 'playing, not paused again');
  assert.equal(rows[1].getAttribute('aria-current'), 'true');
});
