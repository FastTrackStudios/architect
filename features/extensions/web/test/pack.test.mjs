// What npm would publish: exactly the package's files, nothing else of
// the repo's (no tests, no node_modules, no Rust).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('..', import.meta.url));

export const PUBLISHED = [
  'LICENSE-APACHE', 'LICENSE-MIT', 'README.md', 'check.js', 'cli.mjs', 'dist/instance-source.js', 'host.js', 'index.d.ts',
  'index.js', 'instance.js', 'node-worker.mjs', 'node.d.ts', 'node.js', 'package.json', 'react.d.ts', 'react.js',
  'runner.js', 'sanitize.js', 'transpile.mjs', 'typegen.mjs', 'view.js', 'world.js',
];

test('npm pack lists exactly the package’s files', () => {
  const out = execFileSync('npm', ['pack', '--dry-run', '--json', '--ignore-scripts'], { cwd: here, stdio: ['ignore', 'pipe', 'pipe'] });
  const [pack] = JSON.parse(out.toString());
  assert.equal(pack.name, '@fasttrackstudio/architect');
  assert.deepEqual(pack.files.map((f) => f.path).sort(), [...PUBLISHED].sort());
});

test('the manifest publishes publicly, with provenance, and every export has its types', () => {
  const p = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(p.private, undefined);
  assert.deepEqual(p.publishConfig, { access: 'public', provenance: true });
  assert.equal(p.type, 'module');
  for (const [path, e] of Object.entries(p.exports)) if (path !== './package.json') assert.ok(e.types && e.default, path);
  assert.equal(p.bin.architect, './cli.mjs');
});
