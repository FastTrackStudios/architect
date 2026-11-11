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

// N4 (second review of #539): the token is the `npm` environment's
// (restricted to v* tags), read by npm-publish only; the npm jobs' actions
// are pinned by commit.
test('N4: release.yml reads NPM_TOKEN only in npm-publish, in the npm environment; its actions pinned', () => {
  const yml = readFileSync(new URL('../../../../.github/workflows/release.yml', import.meta.url), 'utf8');
  const jobs = yml.slice(yml.indexOf('\n  npm-pack:'));
  const publish = jobs.slice(jobs.indexOf('\n  npm-publish:'));
  const before = jobs.slice(0, jobs.indexOf('\n  npm-publish:'));
  assert.match(publish, /\n    environment: npm\n/);
  assert.ok(!before.includes('NPM_TOKEN') && !before.includes('id-token'), 'npm-pack and npm-test have no token');
  assert.equal(yml.split('secrets.NPM_TOKEN').length - 1, 1, 'read once, in one step');
  assert.ok(publish.includes('secrets.NPM_TOKEN'));
  for (const uses of jobs.matchAll(/uses: ([^\s]+)/g)) assert.match(uses[1], /^[\w.-]+\/[\w.-]+@[0-9a-f]{40}$/, uses[1]);
  assert.ok(!/continue-on-error/.test(publish), 'a failed publish shows red');
});

// SF5 (third review of #539): the tarballs were packed after tests and
// dev dependencies had run; nothing tied what was published to what was
// packed.
test('SF5: the tarballs are packed before anything else runs, and published only if their digests match', () => {
  const yml = readFileSync(new URL('../../../../.github/workflows/release.yml', import.meta.url), 'utf8');
  const jobs = yml.slice(yml.indexOf('\n  npm-pack:'));
  const pack = jobs.slice(0, jobs.indexOf('\n  npm-test:'));
  const publish = jobs.slice(jobs.indexOf('\n  npm-publish:'));
  assert.ok(pack.includes('npm pack --ignore-scripts'));
  assert.ok(!/npm (ci|install|test|run)\b/.test(pack), 'nothing installed or run before the pack');
  assert.match(pack, /outputs:\n\s+runtime_sha256: .*\n\s+cli_sha256:/);
  assert.match(publish, /needs: \[npm-pack, npm-test\]/);
  assert.match(publish, /RUNTIME_SHA256: \$\{\{ needs\.npm-pack\.outputs\.runtime_sha256 \}\}/);
  assert.match(publish, /sha256sum -c -[\s\S]*npm publish/, 'checked before it is published');
  assert.ok(!/npm (ci|install|run)\b/.test(publish), 'nothing installed or run where the token is');
});
