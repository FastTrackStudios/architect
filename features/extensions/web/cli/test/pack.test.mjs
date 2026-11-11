// What npm would publish of @fasttrackstudio/architect-cli.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('..', import.meta.url));

test('the CLI package: its files, its bin, jco as its dependency', () => {
  const out = execFileSync('npm', ['pack', '--dry-run', '--json', '--ignore-scripts'], { cwd: here, stdio: ['ignore', 'pipe', 'pipe'] });
  const [pack] = JSON.parse(out.toString());
  assert.equal(pack.name, '@fasttrackstudio/architect-cli');
  assert.deepEqual(
    pack.files.map((f) => f.path).sort(),
    ['LICENSE-APACHE', 'LICENSE-MIT', 'README.md', 'cli.mjs', 'manifest.mjs', 'package.json', 'transpile.mjs', 'typegen.mjs'],
  );
  const p = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.deepEqual(p.bin, { 'architect-ext': './cli.mjs' });
  assert.ok(p.dependencies['@bytecodealliance/jco']);
  const runtime = JSON.parse(readFileSync(new URL('../../package.json', import.meta.url), 'utf8'));
  assert.equal(runtime.devDependencies['@bytecodealliance/jco'], p.dependencies['@bytecodealliance/jco'], 'one jco pin');
  assert.equal(runtime.devDependencies['smol-toml'], p.dependencies['smol-toml']);
});
