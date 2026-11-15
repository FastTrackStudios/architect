// TypeScript extensions (#536): `architect-ext new --typescript`, `types`,
// `check`, `pack` and `keygen` (ts.mjs). The archive is also opened by the
// Rust side: features/extensions/archive/tests/npm_pack.rs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';
import { actionTypes, check, keygen, pack, readManifest, scaffold } from '../ts.mjs';

const cli = fileURLToPath(new URL('../cli.mjs', import.meta.url));
const recipes = fileURLToPath(new URL('../../../../../examples/showcase/recipe-scaler', import.meta.url));
const tmp = () => mkdtempSync(join(tmpdir(), 'architect-ts-'));

/// The entries of a gzipped tar: `[path, bytes]`.
function untar(gz) {
  const tar = gunzipSync(gz);
  const out = [];
  for (let at = 0; at + 512 <= tar.length; ) {
    const h = tar.subarray(at, at + 512);
    if (h.every((b) => b === 0)) break;
    const field = (from, len) => h.subarray(from, from + len).toString().replace(/\0.*$/s, '');
    const name = field(0, 100);
    const prefix = field(345, 155);
    const size = parseInt(field(124, 12), 8);
    out.push([prefix ? `${prefix}/${name}` : name, tar.subarray(at + 512, at + 512 + size)]);
    at += 512 + Math.ceil(size / 512) * 512;
  }
  return out;
}

test('new --typescript writes one that type-checks, and a mistake fails the check', () => {
  const dir = join(tmp(), 'hello');
  const files = scaffold('lab.hello', dir);
  assert.ok(files.includes('extension.toml') && files.includes('actions/hello.ts') && files.includes('pages/main.tsx'));
  const m = readManifest(dir);
  assert.equal(m.kind, 'typescript');
  assert.equal(m.actions[0].id, 'LAB_HELLO_HELLO');
  check(dir);
  // The manifest's params type the handler: a misspelt field fails.
  const action = join(dir, 'actions/hello.ts');
  writeFileSync(action, readFileSync(action, 'utf8').replace('args?.name', 'args?.nam'));
  assert.throws(() => check(dir), /does not type-check[\s\S]*actions\/hello\.ts[\s\S]*nam/);
  assert.throws(() => scaffold('Bad Id', join(tmp(), 'x')), /lower case/);
});

test('the Recipe scaler type-checks against the app\'s types and its manifest\'s', () => {
  const dir = join(tmp(), 'recipes');
  cpSync(recipes, dir, { recursive: true });
  check(dir);
  const dts = readFileSync(join(dir, 'types/actions.d.ts'), 'utf8');
  assert.match(dts, /"SHOWCASE_RECIPES_SCALE": \{\n\s+path\?: string;\n\s+servings\?: number;/);
});

test('types: each action\'s arguments from its params, as ActionArgs', () => {
  const dts = actionTypes({
    id: 'lab.x',
    actions: [
      { id: 'LAB_X_A', params: '{"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}' },
      { id: 'LAB_X_B' },
    ],
  });
  assert.match(dts, /declare module "architect" \{\n {2}interface ActionArgs \{/);
  assert.match(dts, /"LAB_X_A": \{\n\s+n: number;\n\s+\};/);
  assert.match(dts, /"LAB_X_B": null \| undefined;/);
});

test('a manifest that is not a typescript extension\'s is refused', () => {
  const dir = tmp();
  writeFileSync(join(dir, 'extension.toml'), 'id = "lab.x"\nname = "X"\nversion = "0.1.0"\nkind = "typescript"\n[[actions]]\nid = "LAB_X_A"\nname = "A"\nscript = "a.luau"\n');
  assert.throws(() => readManifest(dir), /runs .ts or .js modules/);
  writeFileSync(join(dir, 'extension.toml'), 'id = "lab.x"\nname = "X"\nversion = "0.1.0"\nkind = "typescript"\n[[actions]]\nid = "LAB_X_A"\nname = "A"\nscript = "gone.ts"\n');
  assert.throws(() => readManifest(dir), /gone\.ts is not in the folder/);
  writeFileSync(join(dir, 'extension.toml'), 'id = "lab.x"\nname = "X"\nversion = "0.1.0"\nkind = "luau"\n');
  assert.throws(() => readManifest(dir), /kind = "typescript"/);
});

test('pack: a gzipped tar of the index, its minisign signature and the files', () => {
  const pair = keygen();
  const { bytes, index } = pack(recipes, pair.secret);
  const entries = untar(bytes);
  assert.deepEqual(entries.slice(0, 2).map(([p]) => p), ['archext.json', 'archext.json.minisig']);
  const paths = entries.slice(2).map(([p]) => p);
  assert.deepEqual(paths, index.files.map((f) => f.path));
  assert.ok(paths.includes('extension.toml') && paths.includes('actions/scale.ts') && paths.includes('pages/lib/recipe.ts'));
  assert.ok(!paths.some((p) => p.startsWith('types/') || p.includes('node_modules')));
  assert.equal(index.kind, 'typescript');
  for (const [path, body] of entries.slice(2)) {
    const f = index.files.find((x) => x.path === path);
    assert.equal(f.sha256, createHash('sha256').update(body).digest('hex'), path);
  }
  // The signature: Ed25519 over BLAKE2b-512 of the index, then over the
  // signature and the trusted comment, by the public key's id.
  const [, sigLine, trusted, globalLine] = entries[1][1].toString().split('\n');
  assert.equal(trusted, `trusted comment: archext ${index.id} ${index.version}`);
  const pub = Buffer.from(pair.public.split('\n')[1], 'base64');
  const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), pub.subarray(10)]), format: 'der', type: 'spki' });
  const sig = Buffer.from(sigLine, 'base64');
  assert.equal(sig.subarray(0, 2).toString(), 'ED');
  assert.ok(sig.subarray(2, 10).equals(pub.subarray(2, 10)), 'its key id');
  const prehash = createHash('blake2b512').update(entries[0][1]).digest();
  assert.ok(verify(null, prehash, key, sig.subarray(10)));
  const global = Buffer.from(globalLine, 'base64');
  assert.ok(verify(null, Buffer.concat([sig.subarray(10), Buffer.from(`archext ${index.id} ${index.version}`)]), key, global));
  // Unsigned without a key.
  assert.deepEqual(untar(pack(recipes).bytes).map(([p]) => p)[1], 'actions/list.ts');
});

test('the command line: new, check, keygen and pack', () => {
  const dir = tmp();
  const run = (...args) => execFileSync(process.execPath, [cli, ...args], { cwd: dir, stdio: ['ignore', 'pipe', 'pipe'] }).toString();
  assert.match(run('new', '--typescript', 'lab.cli', 'ext'), /lab\.cli in ext/);
  assert.equal(run('check', 'ext').trim(), 'ok');
  run('keygen', 'keys');
  assert.match(run('pack', 'ext', '--out', 'x.archext', '--key', 'keys/architect-ext.key'), /lab\.cli 0\.1\.0: 4 files → x\.archext, signed/);
  assert.throws(() => run('new', 'lab.cli2'), /--typescript/);
});
