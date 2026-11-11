// The sandbox frame's CSP and script: SANDBOX_HASH is the script's, the
// frame's policy lets nothing load or connect.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { SANDBOX_CSP, SANDBOX_HASH, SANDBOX_SCRIPT } from '../runner.js';

test('SANDBOX_HASH is the sandbox script’s', () => {
  assert.equal(SANDBOX_HASH, `sha256-${createHash('sha256').update(SANDBOX_SCRIPT).digest('base64')}`);
});

test('the frame’s own CSP: its script, blob: modules and workers, wasm; nothing to fetch or connect to', () => {
  const d = Object.fromEntries(SANDBOX_CSP.split(';').map((x) => x.trim().split(/\s+/)).map(([k, ...v]) => [k, v]));
  assert.deepEqual(d['default-src'], ["'none'"]);
  assert.deepEqual(d['connect-src'], ["'none'"]);
  assert.deepEqual(d['script-src'], [`'${SANDBOX_HASH}'`, 'blob:', "'wasm-unsafe-eval'"]);
  assert.ok(!SANDBOX_CSP.includes("'self'"), "no 'self': the page's origin is not the frame's");
  assert.ok(!SANDBOX_CSP.includes('unsafe-inline'));
});

test('Architect’s own page that runs components names the current hash in its CSP', async () => {
  const { readFileSync } = await import('node:fs');
  const html = readFileSync(new URL('../../../../apps/sandbox/web/index.html', import.meta.url), 'utf8');
  const csp = /http-equiv="Content-Security-Policy" content="([^"]+)"/.exec(html)[1];
  assert.ok(csp.includes(`'${SANDBOX_HASH}'`), `apps/sandbox/web/index.html: add '${SANDBOX_HASH}' to script-src`);
});
