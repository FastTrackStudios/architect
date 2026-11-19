// A distribution's linked extensions in the web app (#495, W1), in a real
// browser: the release bundle (ARCHITECT_WEB_BUNDLE), whose wasm is the
// Architect app's web build (`architect_web::run(architect())`), served
// as production serves it (its policy from apps/web/sws.toml), no node.
//
// In `/app/?demo`:
// - the Architect app's linked extensions are on ⌘K: Projects' "Open
//   Projects", the Calendar's "Open the calendar", the Time Tracker's,
//   the theme picker's "Choose a theme…";
// - ⌘K's "Open Calendar" opens the Calendar's own view as a pane of the
//   workspace (`calendar/calendar`), drawn by its Dioxus component in the
//   app's document, not in a frame;
// - "Open Projects" opens Projects' list the same way (its rows need the
//   vault's `BASES_*`, which the browser gets with documents, #495 H1);
// - the settings page lists the extensions' sections (Calendar, Tasks,
//   Day plans) beside Appearance.
//
// With `probe=views` (the second test, below): a linked extension's views
// drawn wherever an extension's view is: a note by its type, a favourite's
// tab, the home, its NOTE_OPEN.
//
// At 390 px and 1280 px, light and dark; SHOTS=<dir> keeps the
// screenshots. Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the
// PATH), muted, in a mount namespace whose runtime dir and /run/pipewire
// are empty (`unshare -r -m`). `just app-linked-extensions-test` builds
// and runs it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { front, firefox, until, sleep, skipWhy, shotDir } from './lib/app-bidi.mjs';

const skip = skipWhy('app-linked-extensions-test');

const CTRL = '';
const ENTER = '';
const ESC = '';
/// Text on show anywhere in the page.
const shows = (b, text) => b.eval(`document.body.innerText.includes(${JSON.stringify(text)})`);

/// Whether the palette's text (spaces taken out) has `name`.
const lists = (rows, name) => rows.includes(name.replace(/\s+/g, ''));

/// ⌘K, `query` typed: the palette's rows' text.
async function palette(b, query) {
  await b.chord(CTRL, 'k');
  await until(() => b.eval('document.activeElement?.tagName === "INPUT"'), 10000, 'the palette');
  await b.keys(query);
  await sleep(400);
  // A row's matched letters are spans of their own, its spaces lost
  // between them: the text without any, as `lists` compares it.
  return (await b.eval('document.body.innerText')).replace(/\s+/g, '');
}

/// ⌘K, `name` typed and run.
async function run(b, name) {
  const rows = await palette(b, name);
  assert.ok(lists(rows, name), `⌘K lists ${name}`);
  await b.keys(ENTER);
  await sleep(500);
}

test("the Architect app's linked extensions run in /app/?demo: ⌘K, their views as panes, their settings", { skip }, async () => {
  const f = await front();
  const shots = shotDir('linked-ext-shots');
  try {
    for (const dark of [false, true]) {
      const b = await firefox(dark);
      const theme = dark ? 'dark' : 'light';
      try {
        await b.go(`${f.origin}/app/?demo`);
        await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 90000, 'the demo');
        // The Architect app's linked extensions are running (the page's
        // `data-extensions`), none refused.
        const ids = await until(async () => { const v = await b.eval('document.documentElement.dataset.extensions || ""'); return v && v.split(' '); }, 20000, 'the extensions to load');
        for (const id of ['projects', 'tasks', 'time', 'calendar', 'dayplan', 'theme.picker']) {
          assert.ok(ids.includes(id), `${id} runs: ${ids.join(' ')}; ${b.logs.filter((l) => l.includes('extensions:')).join(' | ')}`);
        }
        // Their actions on ⌘K.
        for (const name of ['Open Projects', 'Open the calendar', 'Choose a theme…']) {
          const rows = await palette(b, name);
          assert.ok(lists(rows, name), `⌘K lists ${name}: ${rows.slice(-400)}`);
          await b.keys(ESC);
          await sleep(200);
        }
        // The Calendar's view, as a pane, drawn in the app's document.
        // How many places say "Calendar" now (the navigator's folder).
        const labels = `[...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === 'Calendar' && e.getClientRects().length).length`;
        const before = await b.eval(labels);
        // Its view, opened from ⌘K's views (`CALENDAR_OPEN` reads the
        // vault's bases first, which the browser host gets with documents).
        await run(b, 'Open Calendar');
        await until(async () => (await b.eval(labels)) > before, 15000, 'the Calendar pane (its tab)');
        assert.equal(await b.eval('document.querySelectorAll("iframe").length'), 0, 'a linked view is no frame');
        for (const [w, h, tag] of [[1280, 900, 'desk-1280'], [390, 844, 'phone-390']]) {
          await b.size(w, h);
          await sleep(800);
          assert.ok(await b.eval('document.documentElement.scrollWidth <= window.innerWidth'), `${tag}: no sideways scroll`);
          await b.shot(join(shots, `linked-calendar-${tag}-${theme}.png`));
        }
        await b.size(1280, 900);
        // Projects' list, the same way.
        await run(b, 'Open Projects');
        await until(() => shows(b, 'Projects'), 15000, 'the Projects pane');
        await b.shot(join(shots, `linked-projects-desk-1280-${theme}.png`));
        // Their settings sections beside the app's.
        await run(b, 'Settings');
        await until(() => b.eval(`!!document.querySelector('[data-testid=settings-layer]')`), 10000, 'the settings page');
        for (const section of ['Appearance', 'Calendar', 'Tasks']) {
          await until(() => shows(b, section), 10000, `the ${section} section`);
        }
        await b.shot(join(shots, `linked-settings-desk-1280-${theme}.png`));
        const panics = b.logs.filter((l) => /panicked|RuntimeError|unreachable/.test(l));
        assert.deepEqual(panics, [], 'no panic in the page');
      } finally { await b.close(); }
    }
  } finally { f.close(); }
});

// ── A distribution's linked views wherever an extension's view is drawn ──
//
// `/app/?demo&probe=views` loads the view probe
// (apps/web/src/app/view_probe.rs), a linked extension compiled into the
// app as a distribution's are, with no island and no /_ext tree (no node
// at all here), and the demo's notes of its types (`Probe/`):
// - a `probe-note` note is drawn by its `note` view as the note's page,
//   in the app's document (no frame), handed through `use_embed` its text
//   (`source`) and `{note}` in the shape an island's view gets
//   (`ext_host::note_props`); the view sets and plays its own source on
//   the playhead;
// - a `probe-card` note is drawn by its `card` view at the note's head,
//   over its text;
// - the note type's favourites are its tabs; "More" opens its `more`
//   view as a pane, handed the same note;
// - the view's `open` (NOTE_OPEN) opens a note of the vault;
// - `probe=views,home` makes its `home` view the demo's home, handed
//   `{home: …}`, whose button opens the first row's note.

const PROBE = 'document.querySelector';
/// What the probe's element `testid` was handed: its claim and props.
const handed = (b, scope, testid) => b.eval(`(() => { const e = ${PROBE}('${scope} [data-testid=${testid}]'); return e ? JSON.stringify({ claim: e.dataset.claim, props: JSON.parse(e.dataset.props), playing: e.dataset.playing, source: (e.querySelector('[data-testid=probe-source]') || {}).textContent }) : null; })()`).then((v) => v && JSON.parse(v));
/// Presses the last leaf element whose text is `txt` (a navigator row).
const press = (b, txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
const tab = (b, label) => b.eval(`(() => { const t = [...document.querySelectorAll('button[data-tab]')].find((x) => x.getAttribute('aria-label') === ${JSON.stringify(label)}); if (!t) return false; t.click(); return true; })()`);
const tabs = (b) => b.eval(`JSON.stringify([...document.querySelectorAll('button[data-tab]')].map((x) => x.getAttribute('aria-label')))`).then(JSON.parse);
/// A note from the navigator, in a phone's drawer.
async function openNote(b, folder, name) {
  await until(async () => (await tab(b, 'Files')) || tab(b, 'Vault'), 10000, 'a Files tab');
  await sleep(400);
  await until(() => press(b, folder), 10000, `the ${folder} folder`);
  await sleep(300);
  await until(() => press(b, name), 10000, `the note ${name}`);
  await sleep(400);
}
/// One host-service call as the probe (`window.architectViewProbe`).
const ask = (b, request) => b.eval(`window.architectViewProbe(${JSON.stringify(JSON.stringify(request))})`).then(JSON.parse);

test('a linked view draws a note by its type, opens from a tab and a note, and is the home', { skip }, async () => {
  const f = await front();
  const shots = shotDir('linked-ext-shots');
  const b = await firefox(false);
  try {
    await b.size(390, 844);
    await b.go(`${f.origin}/app/?demo&probe=views`);
    await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 90000, 'the demo');
    const ids = await until(async () => { const v = await b.eval('document.documentElement.dataset.extensions || ""'); return v && v.split(' '); }, 20000, 'the extensions to load');
    assert.ok(ids.includes('web.views'), `the probe runs: ${ids.join(' ')}`);

    // A `probe-note`: its linked view is the note's page, handed the note.
    await openNote(b, 'Probe', 'Probe page');
    const page = await until(() => handed(b, '[data-testid=linked-view-pane]', 'probe-note'), 15000, 'the linked view as the note\'s page');
    assert.equal(page.claim, 'note_type:probe-note');
    const note = page.props.note;
    assert.equal(note.path, 'Probe/Probe page.md');
    assert.equal(note.title, 'Probe page');
    assert.equal(note.type, 'probe-note');
    assert.equal(note.properties.type, 'probe-note', 'a member\'s view gets every property');
    assert.match(note.body, /The probe's own words\./);
    assert.ok(!note.body.includes('not for a view'), `no %%comment%%: ${note.body}`);
    assert.ok(Array.isArray(note.files), 'it reads files: `files` is there (none in this note)');
    assert.equal(page.source, note.body, 'use_embed().source is the text it reads');
    assert.equal(await b.eval('document.querySelectorAll("iframe").length'), 0, 'a linked view is no frame');
    await b.shot(join(shots, 'linked-view-note-phone-390.png'));

    // It drives the playhead as its own extension.
    await b.eval(`document.querySelector('[data-testid=probe-play]').click()`);
    await until(async () => (await handed(b, '[data-testid=linked-view-pane]', 'probe-note'))?.playing === 'true', 10000, 'its own source playing');

    // The type's favourites are the tabs; More opens its view in a pane,
    // handed the same note.
    await until(async () => { const t = await tabs(b); return t.includes('Probe') && t.includes('More'); }, 10000, 'the declared tabs');
    assert.ok(await tab(b, 'More'));
    const more = await until(() => handed(b, '[data-testid=linked-view-pane]', 'probe-more'), 15000, 'the More view');
    assert.equal(more.props.note?.path, 'Probe/Probe page.md');
    await b.shot(join(shots, 'linked-view-more-phone-390.png'));

    // A `probe-card`: its view at the note's head, the note's text under it.
    await openNote(b, 'Probe', 'Probe card');
    const card = await until(() => handed(b, '[data-testid=linked-note-embed]', 'probe-note'), 15000, 'the card at the note\'s head');
    assert.equal(card.claim, 'note_type:probe-card');
    assert.equal(card.props.note.title, 'Probe card');
    assert.ok(await b.eval(`!!document.querySelector('[data-testid=note-text]')`), 'the note\'s own text too');

    // Its `open` opens a note of the vault.
    const opened = await ask(b, { op: 'open', path: 'Probe/Probe page.md' });
    assert.equal(opened.ok?.opened, 'Probe/Probe page.md', JSON.stringify(opened));
    await until(() => b.eval(`!!document.querySelector('[data-note="Probe/Probe page.md"]')`), 10000, 'the opened note');
    const nowhere = await ask(b, { op: 'open', path: 'Nowhere/none.md' });
    assert.ok(nowhere.error, JSON.stringify(nowhere));

    // The home: its `home` view, handed `{home}`; its button opens a note.
    await b.go(`${f.origin}/app/?demo&probe=views,home`);
    const home = await until(() => handed(b, '[data-testid=home-page]', 'probe-home'), 90000, 'the linked home');
    assert.equal(home.claim, 'embed_type:home');
    assert.ok(home.props.home, JSON.stringify(home.props).slice(0, 300));
    assert.ok(Number(await b.eval(`document.querySelector('[data-testid=probe-home]').dataset.rows`)) > 0, 'it is handed the vault\'s typed notes');
    await b.shot(join(shots, 'linked-view-home-phone-390.png'));
    await b.eval(`document.querySelector('[data-testid=probe-home-open]').click()`);
    await until(() => b.eval(`!document.querySelector('[data-testid=home-page]') && !!document.querySelector('[data-note]')`), 10000, 'the first row\'s note, the home out of the way');
    const panics = b.logs.filter((l) => /panicked|RuntimeError|unreachable/.test(l));
    assert.deepEqual(panics, [], 'no panic in the page');
  } finally { await b.close(); f.close(); }
});
