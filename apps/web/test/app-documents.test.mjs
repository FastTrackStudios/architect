// The document API, document events and kept data in the web app (#495
// H1, H2, H4), in a real browser: the release bundle in `/app/?demo&probe`,
// where the demo's probe extension (`apps/web/src/app/probe.toml`: it may
// read and write documents, and records each document event it hears) is
// loaded and `window.architectProbe(id, request)` makes one host-service
// call as it (`EXT_PAGE_CALL`, checked as any extension's).
//
// - H1: it lists, reads, creates, edits, renames and deletes the demo
//   vault's notes; what it makes and changes is what the app's navigator,
//   ⌘O and note view show.
// - H2: the person's typing in a note is heard as `doc.saved`; the
//   probe's own create is heard once (the host raises it, the vault's
//   bridge leaves it out).
// - H4: a setting it sets and data it keeps are there again after a
//   reload (the settings page's store, and `localStorage` per scope).
//
// At 390 px and 1280 px, light and dark; SHOTS=<dir> keeps the
// screenshots. `just app-documents-test` builds and runs it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { front, firefox, until, sleep, skipWhy, shotDir } from './lib/app-bidi.mjs';

const skip = skipWhy('app-documents-test');
const CTRL = '';
const ENTER = '';
const ESC = '';

/// One host-service call as the probe; its answer, or `{error}`.
// (As text across BiDi, whose objects arrive as typed remote values.)
const call = async (b, request) => JSON.parse(await b.eval(`architectProbe('web.probe', ${JSON.stringify(JSON.stringify(request))}).then((t) => t, (e) => JSON.stringify({ error: String(e) }))`));
/// What the probe heard, in order: each `{event, path | from + to}`.
const heard = async (b) => JSON.parse(await b.eval('architectProbeHeard()')).map((h) => ({ event: h.from.event, ...JSON.parse(h.from.detail) }));
/// The open note's text.
const noteText = (b) => b.eval(`document.querySelector('[data-testid=note-text]')?.value ?? ''`);

/// The demo with its probe, its extensions loaded.
async function open(b, origin) {
  await b.go(`${origin}/app/?demo&probe`);
  await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 90000, 'the demo');
  await until(async () => (await b.eval('document.documentElement.dataset.extensions || ""')).split(' ').includes('web.probe'), 20000, 'the probe');
  await until(() => b.eval(`typeof window.architectProbe === 'function'`), 10000, 'the probe call');
}

/// ⌘O, `name` typed, the first row opened.
async function quickOpen(b, name) {
  await b.chord(CTRL, 'o');
  await until(() => b.eval('document.activeElement?.tagName === "INPUT"'), 10000, 'quick open');
  await b.keys(name);
  await sleep(400);
  await b.keys(ENTER);
  await sleep(500);
}

const MADE = 'Notes/From an extension.md';
const MOVED = 'Notes/Renamed by an extension.md';

test('an extension reads and writes the web vault, hears the person, and keeps its data', { skip }, async () => {
  const f = await front();
  const shots = shotDir('documents-shots');
  try {
    for (const dark of [false, true]) {
      const b = await firefox(dark);
      const theme = dark ? 'dark' : 'light';
      try {
        await open(b, f.origin);
        // H1: list and read.
        const listed = await call(b, { op: 'list_documents', under: 'Projects' });
        assert.ok(Array.isArray(listed) && listed.length > 0, `lists Projects: ${JSON.stringify(listed).slice(0, 300)}`);
        assert.ok(listed.every((d) => d.path.startsWith('Projects/')), JSON.stringify(listed).slice(0, 300));
        const welcome = await call(b, { op: 'read_document', path: 'Welcome.md' });
        assert.match(String(welcome), /# Welcome/);
        // Create: the app's ⌘O finds it, its view shows its text.
        const made = await call(b, { op: 'create_document', path: MADE, text: '# From an extension\n\nMade through the document API.\n' });
        assert.equal(made?.error, undefined, JSON.stringify(made));
        await quickOpen(b, 'From an extension');
        await until(async () => (await noteText(b)).includes('Made through the document API.'), 10000, 'the made note open');
        // Edit: one transaction, the open note shows it.
        const after = await call(b, { op: 'edit_document', path: MADE, edits: [{ append: { text: '\nAnd edited by it.\n' } }], name: 'Probe edit' });
        assert.match(String(after), /And edited by it\./, JSON.stringify(after));
        await until(async () => (await noteText(b)).includes('And edited by it.'), 10000, 'the edit in the open note');
        for (const [w, h, tag] of [[1280, 900, 'desk-1280'], [390, 844, 'phone-390']]) {
          await b.size(w, h);
          await sleep(800);
          assert.ok(await b.eval('document.documentElement.scrollWidth <= window.innerWidth'), `${tag}: no sideways scroll`);
          await b.shot(join(shots, `documents-${tag}-${theme}.png`));
        }
        await b.size(1280, 900);
        // An edit outside the text changes nothing.
        const bad = await call(b, { op: 'edit_document', path: MADE, edits: [{ delete: { at: 100000, len: 3 } }] });
        assert.ok(bad?.error, 'an edit outside the text is refused');
        // Rename, then delete.
        const moved = await call(b, { op: 'rename_document', path: MADE, to: MOVED });
        assert.equal(moved?.error, undefined, JSON.stringify(moved));
        const notes = (await call(b, { op: 'list_documents', under: 'Notes' })).map((d) => d.path);
        assert.ok(notes.includes(MOVED) && !notes.includes(MADE), notes.join(', '));
        assert.equal((await call(b, { op: 'delete_document', path: MOVED }))?.error, undefined);
        assert.ok((await call(b, { op: 'read_document', path: MOVED }))?.error, 'gone once deleted');
        // H2: the person's typing is heard as `doc.saved`.
        await quickOpen(b, 'Welcome');
        await until(async () => (await noteText(b)).includes('# Welcome'), 10000, 'Welcome open');
        await b.eval(`document.querySelector('[data-testid=note-text]').focus(); true`);
        await b.keys(' typed');
        await until(async () => (await heard(b)).some((e) => e.event === 'doc.saved' && e.path === 'Welcome.md'), 10000, "the person's edit heard as doc.saved");
        // Each of the probe's own changes is heard once (the host raises
        // them; the vault's bridge leaves them out).
        const own = (await heard(b)).filter((e) => [MADE, MOVED].includes(e.path) || [MADE, MOVED].includes(e.to));
        const created = own.filter((e) => e.event === 'doc.created' && e.path === MADE);
        assert.equal(created.length, 1, `its create heard once: ${JSON.stringify(own)}`);
        const keys = own.map((e) => JSON.stringify(e));
        assert.equal(new Set(keys).size, keys.length, `nothing heard twice: ${keys.join(' | ')}`);
        // H4: a setting and data, kept past a reload.
        const word = `kept-${theme}`;
        assert.equal((await call(b, { op: 'set_setting', key: 'web.probe.word', value: word }))?.error, undefined);
        assert.equal((await call(b, { op: 'set_data', key: 'count', value: { n: dark ? 2 : 1 } }))?.error, undefined);
        await sleep(600); // the settings page takes the write on its next beat
        await open(b, f.origin);
        assert.equal(await call(b, { op: 'setting', key: 'web.probe.word' }), word);
        assert.deepEqual(await call(b, { op: 'data', key: 'count' }), { n: dark ? 2 : 1 });
      } finally { await b.close(); }
    }
  } finally { f.close(); }
});
