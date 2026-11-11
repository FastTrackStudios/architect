// What npm would publish of @fasttrackstudio/architect: exactly its
// files, nothing else of the repo's, and no runtime dependency at all.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('..', import.meta.url));

export const PUBLISHED = [
  'LICENSE-APACHE', 'LICENSE-MIT', 'README.md', 'check.js', 'dist/instance-source.js', 'host.js', 'index.d.ts',
  'index.js', 'instance.js', 'node-worker.mjs', 'node.d.ts', 'node.js', 'package.json', 'react.d.ts', 'react.js',
  'runner.js', 'sanitize.js', 'view.js', 'world.js',
];

test('npm pack lists exactly the package’s files', () => {
  const out = execFileSync('npm', ['pack', '--dry-run', '--json', '--ignore-scripts'], { cwd: here, stdio: ['ignore', 'pipe', 'pipe'] });
  const [pack] = JSON.parse(out.toString());
  assert.equal(pack.name, '@fasttrackstudio/architect');
  assert.deepEqual(pack.files.map((f) => f.path).sort(), [...PUBLISHED].sort());
});

// P7 (review of #539): installing it pulled jco (75 packages, 353 MB).
test('P7: the runtime package has no runtime dependency, no bin, and a bounded React peer', () => {
  const p = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(p.private, undefined);
  assert.deepEqual(p.dependencies ?? {}, {});
  assert.equal(p.bin, undefined, 'no generic `architect` command');
  assert.deepEqual(p.publishConfig, { access: 'public', provenance: true });
  assert.equal(p.type, 'module');
  assert.match(p.peerDependencies.react, /<\d/, 'an upper bound');
  for (const [path, e] of Object.entries(p.exports)) if (path !== './package.json') assert.ok(e.types && e.default, path);
});
