// The Rust browser applier (features/extensions/src/web.rs `Pane`) in
// headless Firefox (#720 H4): a browser sends `click` and `contextmenu` as
// `PointerEvent`s, and they must reach a view as mouse data — pointer data
// there panics a guest's `onclick`. Builds apps/sandbox/pane-events for
// wasm32 and runs it.
//
// `env -u LD_LIBRARY_PATH node --test features/extensions/web/test/rust-pane.test.mjs`
// (in the dev shell: cargo, the wasm32 target and wasm-bindgen), or
// `just ext-pane-browser`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { firefox, open, serve } from './firefox.mjs';

const has = (bin) => {
  try {
    execFileSync('sh', ['-c', `command -v ${bin}`]);
    return true;
  } catch {
    return false;
  }
};
const skip = !firefox() ? 'needs a Firefox (FIREFOX=…)' : !has('wasm-bindgen') ? 'needs wasm-bindgen' : false;
const repo = fileURLToPath(new URL('../../../..', import.meta.url));

test('a click is mouse data and a pointer press pointer data, in a real browser', { skip, timeout: 900000 }, async () => {
  const target = process.env.CARGO_TARGET_DIR || join(repo, 'target');
  execFileSync(
    'cargo',
    ['build', '-p', 'architect-sandbox-pane-events', '--target', 'wasm32-unknown-unknown'],
    { cwd: repo, stdio: 'inherit' },
  );
  const site = mkdtempSync(join(tmpdir(), 'rust-pane-'));
  try {
    execFileSync('wasm-bindgen', [
      '--target', 'web', '--no-typescript', '--out-dir', site,
      join(target, 'wasm32-unknown-unknown/debug/architect_sandbox_pane_events.wasm'),
    ]);
    writeFileSync(
      join(site, 'index.html'),
      `<!doctype html><meta charset="utf-8"><body><script type="module">
import init from './architect_sandbox_pane_events.js';
await init();
const b = document.getElementById('b');
b.dispatchEvent(new PointerEvent('click', { bubbles: true, pointerType: 'mouse' }));
b.dispatchEvent(new PointerEvent('contextmenu', { bubbles: true, pointerType: 'mouse' }));
b.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'pen' }));
b.dispatchEvent(new WheelEvent('wheel', { bubbles: true, deltaY: 40 }));
window.results = window.paneEvents;
</script>`,
    );
    const server = await serve(site);
    const { tab, close } = await open();
    try {
      await tab.go(`${server.origin}/index.html`);
      const seen = JSON.parse(await tab.until('window.results && window.results.length >= 4 && JSON.stringify(window.results)', 'the events'));
      assert.deepEqual(seen, ['click:mouse', 'contextmenu:mouse', 'pointerdown:pointer', 'wheel:wheel']);
    } finally {
      close();
      server.close();
    }
  } finally {
    rmSync(site, { recursive: true, force: true });
  }
});
