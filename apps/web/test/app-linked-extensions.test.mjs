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
