// A setlist's playlist on a share page: the page's player
// (`features/sites/assets/player.js`) run under node in a small fake DOM.
// Run by `features/sites/tests/player_source.rs` (and `node --test` on
// this file).
//
// - Two playlists with only a heading between them (a setlist's two sets,
//   as an existing note draws them) are one list: the headings become
//   section labels in it, never tracks; numbering runs on; next crosses a
//   section and a song ending goes on to the next set.
// - A song with no recording stays in the list and opens its page.
// - Art: a row's tile shows the art the link gives for its page; none,
//   the tile as drawn; one outside the link, refused.
// - Click: a tick on each of the song's beats, scheduled on the audio
//   clock (a tempo change, a seek, a rate), accented on beat 1.
// - Loop: the section playing again and again; a seek moves the loop.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const PLAYER = readFileSync(process.env.ARCHITECT_PLAYER_JS || join(here, '../assets/player.js'), 'utf8');

// ── A DOM just big enough ────────────────────────────────────────────

class Style {
  constructor() { this.props = {}; }
  setProperty(k, v) { this.props[k] = String(v); }
  removeProperty(k) { delete this.props[k]; }
}

// What has the keyboard's focus (`document.activeElement`).
let focused = null;

class Node {
  constructor(tag, attrs = {}, text = '') {
    this.tagName = tag.toUpperCase();
    this.localName = tag.toLowerCase();
    this.attrs = { ...attrs };
    this.children = [];
    this.parentElement = null;
    this.listeners = {};
    this.ownText = text;
    this.hidden = false;
    this.style = new Style();
    this.stubs = {};
    const self = this;
    this.classList = {
      add(c) { self.classes().includes(c) || self.setAttribute('class', [...self.classes(), c].join(' ')); },
      remove(c) { self.setAttribute('class', self.classes().filter((x) => x !== c).join(' ')); },
      toggle(c, on) { (on ?? !this.contains(c)) ? this.add(c) : this.remove(c); },
      contains(c) { return self.classes().includes(c); },
    };
  }
  classes() { return (this.attrs.class || '').split(/\s+/).filter(Boolean); }
  get className() { return this.attrs.class || ''; }
  set className(v) { this.attrs.class = String(v); }
  get textContent() { return this.ownText + this.children.map((c) => c.textContent).join(''); }
  set textContent(t) { this.ownText = String(t); this.children.forEach((c) => (c.parentElement = null)); this.children = []; }
  get firstChild() { return this.children[0] || null; }
  get parentNode() { return this.parentElement; }
  get previousElementSibling() {
    const p = this.parentElement;
    if (!p) return null;
    const i = p.children.indexOf(this);
    return i > 0 ? p.children[i - 1] : null;
  }
  getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; }
  setAttribute(k, v) { this.attrs[k] = String(v); }
  get attributes() { return Object.entries(this.attrs).map(([name, value]) => ({ name, value })); }
  removeAttribute(k) { delete this.attrs[k]; }
  hasAttribute(k) { return k in this.attrs; }
  appendChild(c) { if (c.parentElement) c.parentElement.removeChild(c); c.parentElement = this; this.children.push(c); return c; }
  insertBefore(c, ref) {
    if (c.parentElement) c.parentElement.removeChild(c);
    c.parentElement = this;
    const at = ref ? this.children.indexOf(ref) : -1;
    this.children.splice(at < 0 ? this.children.length : at, 0, c);
    return c;
  }
  removeChild(c) { this.children = this.children.filter((x) => x !== c); c.parentElement = null; return c; }
  addEventListener(name, fn, opts) { (this.listeners[name] ||= []).push({ fn, once: !!(opts && opts.once) }); }
  dispatch(name, ev = {}) {
    const ls = this.listeners[name] || [];
    this.listeners[name] = ls.filter((l) => !l.once);
    ls.forEach((l) => l.fn({ target: this, preventDefault() {}, ...ev }));
  }
  /// A click on this node, bubbling up.
  click() {
    for (let n = this; n; n = n.parentElement) {
      (n.listeners.click || []).forEach((l) => l.fn({ target: this, preventDefault() {} }));
    }
  }
  contains(n) { for (; n; n = n.parentElement) if (n === this) return true; return false; }
  // One compound selector: `tag`, `.class`, `[attr]`, `[attr="v"]`, together.
  matchesOne(sel) {
    const m = sel.match(/^([a-z0-9]*)((?:\.[\w-]+)*)((?:\[[\w-]+(?:="[^"]*")?\])*)$/i);
    if (!m) return false;
    if (m[1] && m[1].toUpperCase() !== this.tagName) return false;
    for (const c of m[2].split('.').filter(Boolean)) if (!this.classes().includes(c)) return false;
    for (const a of m[3].matchAll(/\[([\w-]+)(?:="([^"]*)")?\]/g)) {
      if (!this.hasAttribute(a[1])) return false;
      if (a[2] !== undefined && this.getAttribute(a[1]) !== a[2]) return false;
    }
    return true;
  }
  matches(sel) { return sel.split(",").some((s) => this.matchesChain(s.trim().split(/\s+(?![^\[]*\])/))); }
  matchesChain(parts) {
    if (!this.matchesOne(parts[parts.length - 1])) return false;
    let from = this.parentElement;
    for (let i = parts.length - 2; i >= 0; i--) {
      while (from && !from.matchesOne(parts[i])) from = from.parentElement;
      if (!from) return false;
      from = from.parentElement;
    }
    return true;
  }
  closest(sel) { for (let n = this; n; n = n.parentElement) if (n.matches(sel)) return n; return null; }
  all() { return this.children.flatMap((c) => [c, ...c.all()]); }
  querySelectorAll(sel) { return this.all().filter((n) => n.matches(sel)); }
  querySelector(sel) {
    // The player's own parts (written with innerHTML, which this DOM does
    // not parse): stand-ins, one per selector.
    return this.querySelectorAll(sel)[0] || (this.stubs[sel] ||= new Node('span'));
  }
  set innerHTML(_) {}
  getBoundingClientRect() { return { top: 0, left: 0, width: 10, height: 10 }; }
  scrollTo() {}
  focus() { focused = this; }
}

class Media extends Node {
  constructor(src) {
    super('audio', src ? { src } : {});
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
  set src(v) { this.setAttribute('src', v); }
}

/// An audio clock that keeps what it was asked to play.
class FakeAudio {
  constructor() { this.currentTime = 100; this.state = 'running'; this.played = []; this.destination = {}; }
  resume() { this.state = 'running'; }
  createBuffer() { return {}; }
  createBufferSource() { return { connect() {}, start() {} }; }
  createGain() {
    return { gain: { setValueAtTime() {}, exponentialRampToValueAtTime() {} }, connect() {} };
  }
  createOscillator() {
    const ctx = this;
    const o = {
      frequency: { value: 0 },
      connect() {},
      start(when) { o.when = when; o.accent = o.frequency.value > 1500; ctx.played.push(o); },
      stop(when) { if (when === 0) o.stopped = true; },
    };
    return o;
  }
  /// What sounds: start times (from `from`) of voices not stopped early.
  heard(from = 100) { return this.played.filter((o) => !o.stopped).map((o) => +(o.when - from).toFixed(3)); }
}

const L = '/s/x/';
const el = (tag, attrs = {}, kids = [], text = '') => {
  const n = new Node(tag, attrs, text);
  kids.forEach((k) => n.appendChild(k));
  return n;
};

/// A row as Studio draws it: a track when it has a recording.
function row(n, title, page, media) {
  const attrs = { 'data-page': page, 'data-title': title };
  if (media) Object.assign(attrs, { 'data-track': '', 'data-media': media, 'data-subtitle': 'Artist' });
  return el('li', attrs, [
    el('span', { 'data-track-mark': '' }, [], String(n)),
    el('span', { 'data-track-art': '', 'data-hue': '200' }, [], title.slice(0, 2).toUpperCase()),
    el('span', { 'data-track-text': '' }, [el('span', { 'data-track-title': '' }, [], title)]),
    el('a', { 'data-track-open': '', href: page }),
  ]);
}
/// A playlist in the box the site draws a block in.
const block = (rows, extra = [], join = { class: 'studio-setlist' }) => el('div', { class: 'site-block site-block-0123456789abcdef' }, [
  el('div', { 'data-playlist': '', ...join }, [el('ol', {}, rows), ...extra]),
]);

/// The setlist's page: two sets under their headings, as an existing note
/// draws them, and (for Click and Loop) the first song's view on the page.
function setlist({ art = null, view = null } = {}) {
  const body = new Node('body');
  const article = body.appendChild(el('article'));
  article.appendChild(el('h2', { id: 'presidents-songs' }, [el('a', { class: 'site-anchor', href: '#presidents-songs' }, [], '#')], 'Presidents Songs'));
  article.appendChild(block([
    row(1, 'Sleigh Ride', `${L}sleigh/`, `${L}Sleigh.mp3`),
    row(2, 'Silent Night', `${L}silent/`, `${L}Silent.mp3`),
    row(3, 'Joy to the World', `${L}joy/`, `${L}Joy.mp3`),
  ]));
  article.appendChild(el('h2', { id: 'tree-lighting' }, [], 'Tree Lighting Concert Set'));
  article.appendChild(block([
    row(4, 'Feliz Navidad', `${L}feliz/`, null),
    row(5, 'This Christmas', `${L}this/`, `${L}This.mp3`),
  ], [el('p', { class: 'files' }, [], 'Files')]));
  if (view) body.appendChild(view);
  const queue = JSON.stringify({ scope: L, tracks: [], art: art || {} });
  return run(body, queue);
}

function run(body, queue, opts = {}) {
  const frames = [];
  const timeouts = [];
  const made = [];
  made.channels = [];
  const clock = { now: 1000 };
  const islandsEl = opts.islands ? Object.assign(new Node('script'), { ownText: JSON.stringify(opts.islands) }) : null;
  const store = new Map();
  const timers = [];
  const audio = new FakeAudio();
  const errors = [];
  const queueEl = queue ? Object.assign(new Node('script'), { ownText: queue }) : null;
  const document = {
    body,
    documentElement: new Node('html'),
    currentScript: null,
    querySelector: (sel) => (sel.startsWith('script#site-queue') ? queueEl : sel.startsWith('script#site-islands') ? islandsEl : null),
    querySelectorAll: (sel) => body.querySelectorAll(sel),
    createElement: (tag) => {
      if (tag === 'audio') return new Media('');
      const n = new Node(tag);
      // A frame: what its parent posts it, kept.
      if (tag === 'iframe') {
        n.contentWindow = { posted: [], postMessage(data, _, ports) { this.posted.push({ data, ports }); } };
        made.push(n);
      }
      return n;
    },
    dispatchEvent: () => {},
    // Shown or hidden (another app, a locked phone), and who listens.
    hidden: false,
    on: {},
    addEventListener(type, fn) { (this.on[type] ||= []).push(fn); },
    removeEventListener(type, fn) { this.on[type] = (this.on[type] || []).filter((f) => f !== fn); },
    createComment: () => new Node('#comment'),
    importNode: (n) => n,
    get activeElement() { return focused; },
  };
  const location = { href: `http://node.test${L}merry/`, origin: 'http://node.test' };
  const window = {
    document,
    location,
    history: { state: null, pushState() {}, back() {} },
    sessionStorage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) },
    requestAnimationFrame: (fn) => frames.push(fn),
    cancelAnimationFrame: () => {},
    setInterval: (fn) => { timers.push(fn); return timers.length; },
    clearInterval: (i) => { timers[i - 1] = null; },
    setTimeout: (fn, ms) => { timeouts.push({ fn, at: clock.now + (ms || 0) }); return timeouts.length; },
    clearTimeout: (i) => { if (timeouts[i - 1]) timeouts[i - 1].fn = null; },
    Date: { now: () => clock.now },
    MessageChannel: class {
      constructor() {
        const a = { sent: [], onmessage: null, postMessage(d) { this.sent.push(d); }, close() { this.closed = true; } };
        this.port1 = a;
        this.port2 = {};
        made.channels.push(this);
      }
    },
    crypto: globalThis.crypto,
    btoa: globalThis.btoa,
    TextDecoder,
    Promise,
    addEventListener: () => {},
    getComputedStyle: () => ({ overflowX: 'visible', overflowY: 'visible' }),
    matchMedia: () => ({ matches: false }),
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    AudioContext: function () { return audio; },
    fetch: opts.fetch || (() => Promise.reject(new Error('no network in this test'))),
    URL,
    console: { ...console, error: (...a) => errors.push(a.map(String).join(' ')) },
    Math,
    JSON,
    // Pages this link built, by path (a song's page, fetched for its view).
    DOMParser: class { parseFromString(key) { return (opts.pages || {})[key]; } },
  };
  if (opts.rules) window.architect = { rules: opts.rules };
  focused = null;
  Object.assign(window, { window });
  vm.runInNewContext(PLAYER, window);
  const playhead = window.architect.playhead;
  const audioEl = () => body.all().find((n) => n instanceof Media);
  /// The media at `t`, its clock moved on with the audio clock's.
  const at = (t) => {
    const a = audioEl();
    audio.currentTime += t - a.currentTime;
    a.currentTime = t;
  };
  return {
    body, playhead, audio, window, location, errors, clock, made,
    /// Run every timeout due by now.
    later: () => timeouts.splice(0).forEach((t) => (t.fn && t.at <= clock.now ? t.fn() : timeouts.push(t))),
    /// A real Loop (or Click) button in the bar (the bar's own are
    /// written as HTML, which this DOM does not parse).
    button: (act) => body.querySelectorAll('.site-player')[0].appendChild(el('button', { 'data-act': act })),
    tick: () => timers.forEach((f) => f && f()),
    /// The page hidden or shown again, as `visibilitychange` says.
    visibility: (hidden) => { document.hidden = hidden; (document.on.visibilitychange || []).slice().forEach((f) => f({ type: 'visibilitychange' })); },
    frame: () => frames.splice(0).forEach((f) => f()),
    audioEl, at,
    list: () => body.querySelectorAll('[data-playlist]'),
  };
}

const plain = (o) => JSON.parse(JSON.stringify(o));

// ── One list, its sections, its order ───────────────────────────────

test('two sets under their headings are one list, the headings its section labels', () => {
  const p = setlist();
  const lists = p.list();
  assert.equal(lists.length, 1, 'one playlist');
  const ol = lists[0].querySelectorAll('ol')[0];
  const kinds = ol.children.map((li) => (li.hasAttribute('data-playlist-section') ? `# ${li.textContent}` : li.querySelectorAll('[data-track-mark]')[0].textContent));
  assert.deepEqual(kinds, ['# Presidents Songs', '1', '2', '3', '# Tree Lighting Concert Set', '4', '5']);
  // The labels keep the headings' ids; the headings are gone; the files
  // line follows the joined list.
  assert.equal(ol.children[0].getAttribute('id'), 'presidents-songs');
  assert.equal(ol.children[4].getAttribute('id'), 'tree-lighting');
  assert.equal(p.body.querySelectorAll('h2').length, 0);
  assert.equal(p.body.querySelectorAll('.site-block').length, 1);
  assert.equal(lists[0].children.at(-1).getAttribute('class'), 'files');
  // A label is no track: the queue is the four recordings.
  assert.equal(p.playhead.snapshot().tracks, 4);
});

test('a lone playlist under a heading stays as it is', () => {
  const body = new Node('body');
  body.appendChild(el('h2', {}, [], 'Encore'));
  body.appendChild(block([row(1, 'Joy', `${L}joy/`, `${L}Joy.mp3`)]));
  const p = run(body, null);
  assert.equal(body.querySelectorAll('h2').length, 1);
  assert.equal(body.querySelectorAll('[data-playlist-section]').length, 0);
  assert.equal(p.playhead.snapshot().tracks, 1);
});

test('next crosses a section, and a song ending goes on into the next set', () => {
  const p = setlist();
  const joy = p.body.querySelectorAll('[data-title="Joy to the World"]')[0];
  joy.click();
  assert.equal(p.playhead.src, `${L}Joy.mp3`);
  assert.equal(joy.getAttribute('aria-current'), 'true');
  p.playhead.next();
  assert.equal(p.playhead.src, `${L}This.mp3`, 'past the label and the song with no recording');
  p.playhead.previous();
  assert.equal(p.playhead.src, `${L}Joy.mp3`);
  p.audioEl().dispatch('ended');
  assert.equal(p.playhead.src, `${L}This.mp3`);
  assert.equal(p.playhead.playing, true);
});

test('a song with no recording stays in the list, is no track, and opens its page', () => {
  const p = setlist();
  const feliz = p.body.querySelectorAll('[data-title="Feliz Navidad"]')[0];
  assert.ok(feliz, 'in the list');
  feliz.querySelectorAll('[data-track-text]')[0].click();
  assert.equal(p.location.href, `${L}feliz/`);
  assert.equal(p.playhead.src, null, 'nothing played');
});

// ── Art ─────────────────────────────────────────────────────────────

test('a row shows the art the link gives for its page, its tile otherwise', () => {
  const p = setlist({
    art: {
      [`${L}sleigh/`]: `${L}Libraries/Sleigh/cover.jpg`,
      // Not the link's: refused.
      [`${L}silent/`]: 'https://evil.example/x.jpg',
      [`${L}feliz/`]: '/s/other/Feliz/cover.jpg',
    },
  });
  const img = (title) => {
    const tile = p.body.querySelectorAll(`[data-title="${title}"]`)[0].querySelectorAll('[data-track-art]')[0];
    return tile.querySelectorAll('img')[0]?.getAttribute('src') ?? null;
  };
  assert.equal(img('Sleigh Ride'), `${L}Libraries/Sleigh/cover.jpg`);
  assert.equal(img('Silent Night'), null, 'another site');
  assert.equal(img('Feliz Navidad'), null, 'another link');
  assert.equal(img('Joy to the World'), null, 'none given: its tile');
  // An art file that does not load: the tile again.
  const tile = p.body.querySelectorAll('[data-title="Sleigh Ride"]')[0].querySelectorAll('[data-track-art]')[0];
  tile.querySelectorAll('img')[0].dispatch('error');
  assert.equal(tile.querySelectorAll('img').length, 0);
  assert.equal(tile.hasAttribute('data-has-art'), false);
});

// ── Click ───────────────────────────────────────────────────────────

/// Joy's view on the page: beats at 120 bpm for two bars, then 150 bpm
/// (a tempo change), 4/4; three sections of its chart.
function joyView() {
  const beats = [0, 0.5, 1, 1.5, 2, 2.5, 3, 3.5, 4, 4.4, 4.8, 5.2, 5.6, 6, 6.4, 6.8];
  const sections = [
    { name: 'Intro', start: 0, end: 4 },
    { name: 'Verse', start: 4, end: 5.6 },
    { name: 'Chorus', start: 5.6, end: 7.2 },
  ];
  const svg = el('svg', {}, beats.map((at, i) => el('rect', { 'data-at': String(at), 'data-beat': String((i % 4) + 1) })));
  return el('div', { 'data-now-playing': '', 'data-media': `${L}Joy.mp3`, 'data-sections': JSON.stringify(sections) }, [svg]);
}

async function playingJoy() {
  const p = setlist({ view: joyView() });
  p.body.querySelectorAll('[data-title="Joy to the World"]')[0].click();
  await new Promise((r) => setImmediate(r));
  return p;
}

test('Click ticks on every beat across a tempo change, beat 1 accented', async () => {
  const p = await playingJoy();
  const button = { getAttribute: () => 'click' };
  // The card's Click, as a tap turns it on (its context made in the tap).
  p.window.architect.playhead.snapshot();
  p.body.querySelectorAll('.site-player')[0].listeners.click[0].fn({ target: { closest: (s) => (s === '[data-act]' ? button : null) }, preventDefault() {} });
  for (let t = 0; t <= 6.0; t += 0.025) {
    p.at(t);
    p.tick();
  }
  assert.deepEqual(p.audio.heard(), [0, 0.5, 1, 1.5, 2, 2.5, 3, 3.5, 4, 4.4, 4.8, 5.2, 5.6, 6]);
  assert.deepEqual(p.audio.played.map((o) => o.accent), [true, false, false, false, true, false, false, false, true, false, false, false, true, false]);
});

test('Click after a seek: what was ahead is silenced, the beats from there sound', async () => {
  const p = await playingJoy();
  const button = { getAttribute: () => 'click' };
  p.body.querySelectorAll('.site-player')[0].listeners.click[0].fn({ target: { closest: (s) => (s === '[data-act]' ? button : null) }, preventDefault() {} });
  for (let t = 0; t <= 0.9; t += 0.025) { p.at(t); p.tick(); }
  // 1.0 is scheduled (within the look-ahead), then a seek to 4.3.
  assert.ok(p.audio.played.some((o) => Math.abs(o.when - 101) < 1e-9));
  p.audioEl().currentTime = 4.3;
  p.tick();
  const stopped = p.audio.played.filter((o) => o.stopped && o.when > 100.95).map((o) => +(o.when - 100).toFixed(3));
  assert.deepEqual(stopped, [1], 'the beat at 1.0 never sounds');
  const after = p.audio.played.filter((o) => !o.stopped && o.when > 100.9);
  assert.deepEqual(after.map((o) => +(o.when - p.audio.currentTime).toFixed(3)), [0.1], 'the beat at 4.4, 0.1 s from the seek');
  // At half speed a beat 0.05 s of the song ahead is 0.1 s of the clock.
  p.audioEl().playbackRate = 0.5;
  p.audioEl().currentTime = 5.15;
  p.tick();
  const last = p.audio.played.filter((o) => !o.stopped).at(-1);
  assert.equal(+(last.when - p.audio.currentTime).toFixed(3), 0.1);
});

test('Click is off with no beats: its button disabled, not hidden', async () => {
  const p = setlist();
  p.body.querySelectorAll('[data-title="Sleigh Ride"]')[0].click();
  await new Promise((r) => setImmediate(r));
  for (let t = 0; t < 2; t += 0.1) { p.at(t); p.tick(); }
  assert.equal(p.audio.played.length, 0);
});

// ── Loop ────────────────────────────────────────────────────────────

test('Loop holds the section playing, and a seek moves it', async () => {
  const p = await playingJoy();
  const button = { getAttribute: () => 'loop' };
  p.body.querySelectorAll('.site-player')[0].listeners.click[0].fn({ target: { closest: (s) => (s === '[data-act]' ? button : null) }, preventDefault() {} });
  const a = p.audioEl();
  const at = (t) => { a.currentTime = t; a.dispatch('timeupdate'); return a.currentTime; };
  assert.equal(at(1), 1);
  assert.equal(at(3.9), 3.9, 'not yet its end');
  assert.equal(at(4.0), 0, 'its end: back to its start');
  // A seek into the chorus loops the chorus.
  assert.equal(at(6.0), 6.0);
  assert.equal(at(7.2), 5.6);
  // The song ending inside the last section: back to it, not on to the next song.
  at(6.5);
  a.dispatch('ended');
  assert.equal(p.playhead.src, `${L}Joy.mp3`);
  assert.equal(a.currentTime, 5.6);
  // Off: it plays on.
  p.body.querySelectorAll('.site-player')[0].listeners.click[0].fn({ target: { closest: (s) => (s === '[data-act]' ? button : null) }, preventDefault() {} });
  assert.equal(at(7.2), 7.2);
  assert.deepEqual(plain(JSON.parse(p.window.sessionStorage.getItem(`architect.player:${L}`))).loop, false);
});

// ── What the review found ─────────────────────────────────────────

test('lists that do not say they join stay apart', () => {
  const body = new Node('body');
  body.appendChild(el('h2', {}, [], 'Stems'));
  body.appendChild(block([row(1, 'Bass', `${L}bass/`, `${L}Bass.mp3`)], [], {}));
  body.appendChild(el('h2', {}, [], 'References'));
  body.appendChild(block([row(1, 'Mix', `${L}mix/`, `${L}Mix.mp3`)], [], {}));
  run(body, JSON.stringify({ scope: L, tracks: [], art: {} }));
  assert.equal(body.querySelectorAll('[data-playlist]').length, 2);
  assert.equal(body.querySelectorAll('h2').length, 2);
  assert.equal(body.querySelectorAll('[data-playlist-section]').length, 0);
});

test('a song listed twice plays and is marked where it was tapped, and goes on from there', () => {
  const body = new Node('body');
  body.appendChild(block([
    row(1, 'Joy', `${L}joy/`, `${L}Joy.mp3`),
    row(2, 'Noel', `${L}noel/`, `${L}Noel.mp3`),
    row(3, 'Joy', `${L}joy/`, `${L}Joy.mp3`),
    row(4, 'Eve', `${L}eve/`, `${L}Eve.mp3`),
  ]));
  const p = run(body, JSON.stringify({ scope: L, tracks: [], art: {} }));
  const rows = body.querySelectorAll('[data-track]');
  rows[2].querySelectorAll('[data-track-title]')[0].click();
  assert.equal(rows[2].getAttribute('aria-current'), 'true');
  assert.equal(rows[0].getAttribute('aria-current'), null, 'not the first Joy');
  p.playhead.next();
  assert.equal(p.playhead.src, `${L}Eve.mp3`, 'on from the second');
});

test('Loop never seeks itself to death on a section too short to play', async () => {
  const sections = [{ name: 'x', start: 2, end: 2 }, { name: 'y', start: 2, end: 2.01 }];
  const view = el('div', { 'data-now-playing': '', 'data-media': `${L}Joy.mp3`, 'data-sections': JSON.stringify(sections) }, [el('svg')]);
  const p = setlist({ view });
  const loopButton = p.button('loop');
  p.body.querySelectorAll('[data-title="Joy to the World"]')[0].click();
  await new Promise((r) => setImmediate(r));
  const button = { getAttribute: () => 'loop' };
  p.body.querySelectorAll('.site-player')[0].listeners.click[0].fn({ target: { closest: (s) => (s === '[data-act]' ? button : null) }, preventDefault() {} });
  const a = p.audioEl();
  a.currentTime = 2;
  a.playbackRate = 1;
  a.dispatch('timeupdate');
  assert.deepEqual(p.errors.filter((e) => /call stack|recursion/i.test(e)), [], 'no seek loop');
  assert.equal(a.currentTime, 2, 'nothing to loop');
  assert.ok(loopButton.hasAttribute('disabled'), 'and Loop says so');
});

test('Loop\'s own seek never re-enters it, even when the media has not moved yet', async () => {
  const sections = [{ name: 'Verse', start: 0, end: 4 }];
  const view = el('div', { 'data-now-playing': '', 'data-media': `${L}Joy.mp3`, 'data-sections': JSON.stringify(sections) }, [el('svg')]);
  const p = setlist({ view });
  p.body.querySelectorAll('[data-title="Joy to the World"]')[0].click();
  await new Promise((r) => setImmediate(r));
  const button = { getAttribute: () => 'loop' };
  p.body.querySelectorAll('.site-player')[0].listeners.click[0].fn({ target: { closest: (s) => (s === '[data-act]' ? button : null) }, preventDefault() {} });
  const a = p.audioEl();
  a.paused = false;
  a.currentTime = 1;
  a.dispatch('timeupdate');
  // At the section's end, and a seek that does not land at once (the
  // media still says where it was): Loop's seek tells it where the song
  // is, synchronously, while it is still deciding.
  const seeks = [];
  Object.defineProperty(a, 'currentTime', { get: () => 3.99, set: (v) => { seeks.push(v); }, configurable: true });
  a.dispatch('timeupdate');
  assert.deepEqual(p.errors.filter((e) => /call stack|recursion/i.test(e)), [], 'no seek loop');
  assert.deepEqual(seeks, [0], 'one seek back to the section\'s start');
});

test('beats closer than 50 ms are one beat, so Click never floods', async () => {
  const beats = Array.from({ length: 200 }, (_, i) => i * 0.001);
  const svg = el('svg', {}, beats.map((at) => el('rect', { 'data-at': String(at), 'data-beat': '1' })));
  const view = el('div', { 'data-now-playing': '', 'data-media': `${L}Joy.mp3` }, [svg]);
  const p = setlist({ view });
  p.body.querySelectorAll('[data-title="Joy to the World"]')[0].click();
  await new Promise((r) => setImmediate(r));
  const button = { getAttribute: () => 'click' };
  p.body.querySelectorAll('.site-player')[0].listeners.click[0].fn({ target: { closest: (s) => (s === '[data-act]' ? button : null) }, preventDefault() {} });
  for (let t = 0; t <= 0.2; t += 0.025) { p.at(t); p.tick(); }
  assert.equal(p.audio.played.length, 4, 'a millisecond apart: one click in 50 ms (0, 0.05, 0.1, 0.15), not 200');
});

test('Loop is off while an outside clock holds the playhead', async () => {
  const p = await (async () => {
    const q = setlist({ view: joyView() });
    q.body.querySelectorAll('[data-title="Joy to the World"]')[0].click();
    await new Promise((r) => setImmediate(r));
    return q;
  })();
  const loopButton = p.button('loop');
  p.audioEl().dispatch('timeupdate');
  await new Promise((r) => setImmediate(r));
  assert.ok(!loopButton.hasAttribute('disabled'), 'a song with sections: Loop on offer');
  const h = p.playhead.attach({ time: 0, playing: false, seek() {} });
  assert.ok(loopButton.hasAttribute('disabled'), 'disabled while the clock is elsewhere');
  h.detach();
  assert.ok(!loopButton.hasAttribute('disabled'));
});

test('a row\'s menu: its chart, its page; Escape closes it and gives the button back', async () => {
  const body = new Node('body');
  const more = el('button', { type: 'button', 'data-track-more': '', 'aria-label': 'More: Joy' });
  const r = row(1, 'Joy', `${L}joy/`, `${L}Joy.mp3`);
  r.appendChild(more);
  const lone = row(2, 'Noel', `${L}noel/`, null);
  const loneMore = el('button', { type: 'button', 'data-track-more': '' });
  lone.appendChild(loneMore);
  body.appendChild(block([r, lone]));
  const p = run(body, JSON.stringify({ scope: L, tracks: [], art: {} }));
  more.click();
  assert.equal(more.getAttribute('aria-expanded'), 'true');
  const menu = body.querySelectorAll('.site-row-menu')[0];
  assert.equal(menu.getAttribute('role'), 'menu');
  const items = menu.querySelectorAll('.site-row-menu-item').map((i) => [i.textContent, i.getAttribute('href')]);
  assert.deepEqual(items, [['Open chart', null], ['Song page and downloads', `${L}joy/`]]);
  menu.dispatch('keydown', { key: 'Escape', preventDefault() {} });
  assert.equal(body.querySelectorAll('.site-row-menu').length, 0);
  assert.equal(more.getAttribute('aria-expanded'), 'false');
  // A song with no recording: its page only, no dead entry.
  loneMore.click();
  const lonely = body.querySelectorAll('.site-row-menu')[0].querySelectorAll('.site-row-menu-item').map((i) => i.textContent);
  assert.deepEqual(lonely, ['Chart and downloads']);
  // The menu button is not a tap on the row.
  assert.equal(p.playhead.src, null);
});

test('the card shows a music note, never letters nor what the tile says (its key), and never an empty square', () => {
  const body = new Node('body');
  const r = row(1, 'Sleigh Ride', `${L}sleigh/`, `${L}Sleigh.mp3`);
  const tile = r.querySelectorAll('[data-track-art]')[0];
  tile.textContent = 'B';
  tile.setAttribute('data-key', '');
  body.appendChild(block([r]));
  const p = run(body, JSON.stringify({ scope: L, tracks: [], art: {} }));
  const art = () => p.window.document.body.querySelectorAll('.site-player')[0].querySelector('.site-player-art');
  // Nothing loaded ("1 track"): the glyph, not an empty tile.
  p.frame();
  assert.ok(art().hasAttribute('data-glyph'), 'a glyph before anything plays');
  assert.equal(art().textContent, '');
  r.querySelectorAll('[data-track-title]')[0].click();
  p.frame();
  assert.ok(art().hasAttribute('data-glyph'), 'a glyph for the song');
  assert.equal(art().textContent, '', 'no initials, no key');
  assert.match(art().style.props.background || '', /^hsl\(200 /, 'on its tile\'s colour');
  assert.ok(!/function initials/.test(PLAYER), 'no initials code path');
});

// ── An island's web view (the review of #608) ───────────────────────

async function sha(bytes) {
  const d = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return 'sha256-' + Buffer.from(d).toString('base64');
}

/// The setlist with an island whose web bundle the node serves, its
/// now-playing view open: the frame made, its port and what it was sent.
async function webview() {
  const files = { 'web-x.js': Buffer.from('export default 1'), 'web-x_bg.wasm': Buffer.from('\0asm') };
  const pinned = {};
  for (const [n, b] of Object.entries(files)) pinned[n] = await sha(b);
  const fetched = [];
  const fetch = (url) => {
    fetched.push(url);
    const name = String(url).split('/').pop();
    if (!files[name]) return Promise.resolve({ ok: false, status: 404, headers: { get: () => null } });
    return Promise.resolve({ ok: true, arrayBuffer: () => Promise.resolve(new Uint8Array(files[name]).buffer) });
  };
  const islands = {
    runtime: '/_ext/runtime.0123/', sandbox: '/_ext/sandbox.0123.html', webview: '/_ext/webview.0123abcd.html',
    islands: [{ id: 'demo', folder: '/_ext/island/demo.0123/', progress: 'bar', web: { entry: 'web-x.js', files: pinned, webview: '/_ext/webview.0123abcd.html' } }],
  };
  const body = new Node('body');
  body.appendChild(block([row(1, 'Joy', `${L}joy/`, `${L}Joy.mp3`)]));
  const p = run(body, JSON.stringify({ scope: L, tracks: [], art: {} }), { islands, fetch });
  body.querySelectorAll('[data-track-title]')[0].click();
  p.body.querySelectorAll('.site-player')[0].listeners.click[0].fn({ target: { closest: () => null }, preventDefault() {} });
  const frame = p.made[0];
  assert.ok(frame, 'a frame for the view');
  assert.equal(frame.getAttribute('src'), '/_ext/webview.0123abcd.html', 'the document the node chose');
  frame.dispatch('load');
  const handed = frame.contentWindow.posted[0];
  assert.equal(handed.data, 'architect-web');
  const port = p.made.channels[0].port1;
  const say = (d) => port.onmessage({ data: d });
  const boots = () => port.sent.filter((m) => m.t === 'boot').length;
  const settle = () => new Promise((r) => setTimeout(r, 20));
  const slot = frame.parentElement;
  return { p, frame, port, say, boots, settle, slot, fetched };
}

test('a frame is answered and booted once, however often it says hello', async () => {
  const w = await webview();
  for (let i = 0; i < 300; i++) w.say({ t: 'hello' });
  await w.settle();
  assert.equal(w.boots(), 1, 'one boot');
  assert.equal(w.fetched.filter((u) => String(u).startsWith('/_ext/')).length, 2, 'its two files fetched once');
});

test('a frame that leaves its document after it is ready is gone, and the player\'s own are back', async () => {
  const w = await webview();
  w.say({ t: 'hello' });
  await w.settle();
  w.say({ t: 'ready' });
  assert.equal(w.slot.hidden, false);
  w.frame.dispatch('load');
  assert.equal(w.frame.parentElement, null, 'the frame is out');
  assert.equal(w.slot.hidden, true);
  // A ready that comes later changes nothing.
  w.say({ t: 'ready' });
  assert.equal(w.slot.hidden, true);
});

test('a frame that is never ready, or stops answering, is given up', async () => {
  const late = await webview();
  late.p.clock.now += 16000;
  late.p.later();
  assert.equal(late.frame.parentElement, null, 'not ready in 15 s');
  const quiet = await webview();
  quiet.say({ t: 'hello' });
  await quiet.settle();
  quiet.say({ t: 'ready' });
  quiet.p.tick();
  const ping = quiet.port.sent.find((m) => m.t === 'ping');
  assert.ok(ping, 'a heartbeat');
  quiet.say({ t: 'pong', n: ping.n });
  quiet.p.clock.now += 2000;
  quiet.p.tick();
  assert.ok(quiet.frame.parentElement, 'answered: still up');
  quiet.p.clock.now += 7000;
  quiet.p.tick();
  assert.equal(quiet.frame.parentElement, null, 'unanswered 6 s: gone');
  // An error after ready: gone too.
  const failing = await webview();
  failing.say({ t: 'hello' });
  await failing.settle();
  failing.say({ t: 'ready' });
  failing.say({ t: 'error', message: 'x' });
  assert.equal(failing.frame.parentElement, null);
});

test('a frame is never given up for the time the page was hidden (an app switch), and is when it stays silent after', async () => {
  // Ready, a ping sent, then the phone switches app: its pong never comes
  // while the page is hidden.
  const w = await webview();
  w.say({ t: 'hello' });
  await w.settle();
  w.say({ t: 'ready' });
  w.p.tick();
  assert.ok(w.port.sent.some((m) => m.t === 'ping'));
  w.p.visibility(true);
  w.p.clock.now += 60000;
  w.p.tick();
  assert.ok(w.frame.parentElement, 'hidden a minute: still up');
  // Shown again: the beat starts over, a new ping, answered: up.
  w.p.visibility(false);
  w.p.tick();
  const pings = w.port.sent.filter((m) => m.t === 'ping');
  assert.equal(pings.length, 2, 'a new beat from being shown');
  w.say({ t: 'pong', n: pings[1].n });
  w.p.clock.now += 2000;
  w.p.tick();
  assert.ok(w.frame.parentElement, 'answered after being shown');
  // Silent while shown: gone, as before.
  w.p.clock.now += 7000;
  w.p.tick();
  assert.equal(w.frame.parentElement, null, 'silent 6 s while shown: gone');

  // Not ready yet when the page is hidden: the deadline waits for it.
  const slow = await webview();
  slow.p.visibility(true);
  slow.p.clock.now += 16000;
  slow.p.later();
  assert.ok(slow.frame.parentElement, 'hidden past 15 s: not given up');
  slow.p.visibility(false);
  slow.p.clock.now += 16000;
  slow.p.later();
  assert.equal(slow.frame.parentElement, null, '15 s shown and not ready: gone');
});

test('the row menu keeps the keyboard where it was when the song\'s page arrives and refills it', async () => {
  const page = `${L}joy/`;
  const joy = new Node('body');
  joy.appendChild(el('div', { 'data-now-playing': '', 'data-media': `${L}Joy.mp3` }, [el('svg')]));
  joy.appendChild(el('ol', { class: 'site-lyrics', 'data-follow-playhead': '', 'data-media': `${L}Joy.mp3` }, [el('li', { 'data-at': '1.000' }, [], 'Joy')]));
  const fetch = (url) => (String(url) === page
    ? Promise.resolve({ ok: true, status: 200, headers: { get: (k) => (k === 'content-type' ? 'text/html' : k === 'x-architect-page' ? 'built' : null) }, text: () => Promise.resolve(page) })
    : Promise.reject(new Error('no network')));
  const rules = { elementRule: () => ['keep'], attrRule: (_t, _n, v) => v, cssInert: () => true };
  const body = new Node('body');
  const more = el('button', { type: 'button', 'data-track-more': '', 'aria-label': 'More: Joy' });
  const r = row(1, 'Joy', page, `${L}Joy.mp3`);
  r.appendChild(more);
  body.appendChild(block([r]));
  const p = run(body, JSON.stringify({ scope: L, tracks: [], art: {} }), { fetch, rules, pages: { [page]: joy } });
  more.click();
  const menu = body.querySelectorAll('.site-row-menu')[0];
  const now = () => p.window.document.activeElement;
  assert.equal(now().textContent, 'Open chart', 'its first item focused');
  menu.dispatch('keydown', { key: 'ArrowDown', preventDefault() {} });
  assert.equal(now().textContent, 'Song page and downloads');
  // The song's page arrives: it has lyrics, so the menu is filled again.
  await new Promise((res) => setTimeout(res, 20));
  const items = menu.querySelectorAll('.site-row-menu-item');
  assert.deepEqual(items.map((i) => i.textContent), ['Open chart', 'Lyrics', 'Song page and downloads'], 'refilled');
  assert.ok(menu.contains(now()), 'the focus is in the menu still, not on the page');
  assert.equal(now().textContent, 'Song page and downloads', 'on the item it was on');
  menu.dispatch('keydown', { key: 'ArrowUp', preventDefault() {} });
  assert.equal(now().textContent, 'Lyrics', 'and the arrows go on from there');
});
