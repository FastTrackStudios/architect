// The showcase's extensions installed in the web app (#495), in a real
// browser: the release bundle (ARCHITECT_WEB_BUNDLE) served as production
// serves it (its policy from apps/web/sws.toml), no node.
//
// In `/app/?showcase`:
// - the vault's needs (`.architect/extensions.toml`) are listed over it:
//   the Recipe scaler (TypeScript) and the Luau ones to install (#812:
//   none says "Can't run here" any more);
// - "Install all", then "Install" in the one consent prompt, installs them
//   from the registry the app carries, with no network, and they run (the
//   page's `data-extensions`);
// - Luau actions run in Web Workers on luau-web's Luau, run as their
//   extensions through the probe, each host call answered by the app's
//   host service: the Timers sampler's tick (its data kept between runs,
//   its status item), its primes counted in a worker, the Notify
//   sampler's toast, Settings and theme's greeting;
// - its "open" action (`SHOWCASE_RECIPES_OPEN`, run as the extension
//   through the demo's probe, `?showcase&probe`) opens its TSX page as a
//   pane, rendered in a Web Worker on the browser's engine and drawn in
//   the app's document. Its recipe list reads `VAULT_*`, which the web app
//   does not have yet: the page says so (the follow-up moves it to the
//   documents service);
// - after a reload it is still installed and running, with no prompt (the
//   browser's storage keeps the install and the grant).
//
// At 390 px and 1280 px, light and dark; SHOTS=<dir> keeps the
// screenshots. Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the
// PATH), muted, in a mount namespace whose runtime dir and /run/pipewire
// are empty (`unshare -r -m`). `just app-showcase-install-test` builds and
// runs it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { front, firefox, until, sleep, skipWhy, shotDir } from './lib/app-bidi.mjs';

const skip = skipWhy('app-showcase-install-test');

const CTRL = '';
const ESC = '\uE00C';

/// Text on show anywhere in the page.
const shows = (b, text) => b.eval(`document.body.innerText.includes(${JSON.stringify(text)})`);
/// Presses the button whose text is `label` (the last one, if several).
const press = (b, label) => b.eval(`(() => { const all = [...document.querySelectorAll('button')].filter((e) => e.textContent.trim() === ${JSON.stringify(label)}); const e = all[all.length - 1]; if (!e) return false; e.click(); return true; })()`);
/// The extensions running (the page's `data-extensions`).
const running = async (b) => (await b.eval('document.documentElement.dataset.extensions || ""')).split(' ');

test("/app/?showcase installs the showcase's extensions, runs its Luau ones and draws its TSX page", { skip, timeout: 600000 }, async () => {
  const f = await front();
  const shots = shotDir('showcase-install-shots');
  try {
    for (const dark of [false, true]) {
      const b = await firefox(dark);
      const theme = dark ? 'dark' : 'light';
      try {
        await b.size(1280, 900);
        await b.go(`${f.origin}/app/?showcase&probe`);
        // What the vault needs, over it.
        await until(() => shows(b, 'Install all'), 90000, 'the vault\'s needs and Install all');
        assert.ok(await shows(b, 'Recipe scaler'), 'the scaler is offered');
        assert.ok(await shows(b, 'Timers sampler'), 'a Luau extension is offered');
        assert.ok(!(await shows(b, '#812')), 'no Luau extension says it cannot run here');
        assert.ok(!(await running(b)).includes('showcase.recipes'), 'nothing runs before the yes');
        await b.shot(join(shots, `needs-desk-1280-${theme}.png`));
        // One consent for all of it.
        assert.ok(await press(b, 'Install all'));
        await until(() => shows(b, 'Allow on'), 15000, 'the consent prompt');
        // The prompt drawn whole before its button is pressed.
        await sleep(1500);
        await b.shot(join(shots, `consent-desk-1280-${theme}.png`));
        assert.ok(await press(b, 'Install'));
        await until(() => shows(b, 'installed and running'), 20000, 'the install\'s outcome');
        try {
          await until(async () => (await running(b)).includes('showcase.recipes'), 20000, 'the scaler to run');
        } catch (e) {
          await b.shot(join(shots, `failed-install-${theme}.png`));
          throw new Error(`${e.message}; the page says: ${JSON.stringify((await b.eval('document.body.innerText')).slice(0, 1500))}`);
        }
        // Luau: the showcase's run, each action as its extension.
        for (const id of ['showcase.timers', 'showcase.notify', 'showcase.dusk', 'showcase.chart']) {
          assert.ok((await running(b)).includes(id), `${id} runs`);
        }
        const run = async (ext, action, args = {}) => String(await b.eval(`window.architectProbe(${JSON.stringify(ext)}, ${JSON.stringify(JSON.stringify({ op: 'run', action, args }))})`));
        // Its data kept between runs (its `every` hook may tick too).
        const first = Number((await run('showcase.timers', 'SHOWCASE_TIMERS_TICK')).match(/\d+/)?.[0]);
        const second = Number((await run('showcase.timers', 'SHOWCASE_TIMERS_TICK')).match(/\d+/)?.[0]);
        assert.ok(first >= 1 && second > first, `a tick counts on from the last (${first}, ${second})`);
        await until(() => b.eval('/\\b\\d+ ticks?\\b/.test(document.body.innerText)'), 15000, 'its status item');
        // Heavy work in its worker, then a toast.
        assert.match(await run('showcase.timers', 'SHOWCASE_TIMERS_PRIMES', { n: 20000 }), /2262/);
        await until(() => shows(b, '2262 primes below 20000'), 10000, 'the primes toast');
        assert.match(await run('showcase.notify', 'SHOWCASE_NOTIFY_TOAST', { text: 'Luau in the browser' }), /Luau in the browser/);
        await until(() => shows(b, 'Luau in the browser'), 10000, 'the toast');
        assert.match(await run('showcase.dusk', 'SHOWCASE_DUSK_GREET'), /Hello/);
        await b.shot(join(shots, `luau-desk-1280-${theme}.png`));
        await b.size(390, 844);
        await sleep(800);
        await b.shot(join(shots, `luau-phone-390-${theme}.png`));
        await b.size(1280, 900);
        // Its TSX page, from ⌘K, as a pane: a Web Worker renders it, the
        // app's document draws it (no frame).
        // On ⌘K, and opened by its own action, as the extension.
        await b.chord(CTRL, 'k');
        await until(() => b.eval('document.activeElement?.tagName === "INPUT"'), 10000, 'the palette');
        await b.keys('Recipe scaler: open');
        await sleep(400);
        assert.ok((await b.eval('document.body.innerText')).replace(/\s+/g, '').includes('Recipescaler:open'), '⌘K lists it');
        await b.keys(ESC);
        const opened = await b.eval(`window.architectProbe('showcase.recipes', ${JSON.stringify(JSON.stringify({ op: 'run', action: 'SHOWCASE_RECIPES_OPEN', args: {} }))})`);
        assert.match(String(opened), /opened/);
        const page = () => b.eval('[...document.querySelectorAll("[data-extension-page]")].map((e) => e.innerText).join("|")');
        await until(async () => (await page()).includes('Written in TypeScript'), 20000, 'the scaler\'s TSX page');
        assert.match(await page(), /VAULT_LIST/, 'its list says what it lacks here');
        assert.equal(await b.eval('document.querySelectorAll("iframe").length'), 0, 'a page is no frame');
        for (const [w, h, tag] of [[1280, 900, 'desk-1280'], [390, 844, 'phone-390']]) {
          await b.size(w, h);
          await sleep(800);
          assert.ok(await b.eval('document.documentElement.scrollWidth <= window.innerWidth'), `${tag}: no sideways scroll`);
          await b.shot(join(shots, `scaler-${tag}-${theme}.png`));
        }
        await b.size(1280, 900);
        // Kept: after a reload it runs again, and nothing asks again.
        await b.go(`${f.origin}/app/?showcase&probe`);
        await until(async () => (await running(b)).includes('showcase.recipes'), 90000, 'the scaler after a reload');
        await sleep(1000);
        assert.ok(!(await shows(b, 'Allow on')), 'no prompt after a reload');
        const panics = b.logs.filter((l) => /panicked|RuntimeError|unreachable/.test(l));
        assert.deepEqual(panics, [], 'no panic in the page');
      } finally { await b.close(); }
    }
  } finally { f.close(); }
});
