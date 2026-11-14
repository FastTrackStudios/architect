// The web view SDK's types (webview.d.ts, webview-host.d.ts) compile, and
// accept one request per host-service op (test/host-requests.ts, the same
// JSON serde reads in features/extensions/src/webview.rs) while refusing a
// wrong one.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, copyFileSync, mkdirSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const web = fileURLToPath(new URL('..', import.meta.url));
const tsc = join(web, 'node_modules/typescript/bin/tsc');

function compile(files) {
  const dir = mkdtempSync(join(tmpdir(), 'webview-types-'));
  mkdirSync(join(dir, 'test'));
  for (const f of ['webview.d.ts', 'webview-host.d.ts']) copyFileSync(join(web, f), join(dir, f));
  for (const [name, text] of Object.entries(files)) writeFileSync(join(dir, name), text);
  writeFileSync(join(dir, 'tsconfig.json'), JSON.stringify({
    compilerOptions: { strict: true, noEmit: true, module: 'nodenext', moduleResolution: 'nodenext', target: 'es2022', lib: ['es2022', 'dom'], skipLibCheck: false },
    include: ['**/*.ts'],
  }));
  execFileSync(process.execPath, [tsc, '-p', dir], { stdio: 'pipe' });
}

test('every host request type-checks against the SDK', () => {
  const fixture = readFileSync(join(web, 'test/host-requests.ts'), 'utf8');
  compile({ 'test/host-requests.ts': fixture });
});

test('a wrong request does not', () => {
  assert.throws(() =>
    compile({
      'wrong.ts': "import type { HostRequest } from './webview.js';\nexport const r: HostRequest = { op: 'store_secret', name: 'token' };\n",
    }));
});
