// Typed actions: the counter's manifest made into actions.d.ts, then tsc
// over a use of it. Each wrong argument is marked `@ts-expect-error`, so
// it must fail to compile: tsc fails the run if any of them compiles.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'smol-toml';
import { methodNames, schemaType, typegen } from '../typegen.mjs';

const here = fileURLToPath(new URL('..', import.meta.url));
const web = join(here, '..');
const counter = parse(readFileSync(join(web, '../../../apps/sandbox/wasi-counter/extension.toml'), 'utf8'));

test('the JSON schema subset as TypeScript', () => {
  assert.equal(schemaType({ type: 'string' }), 'string');
  assert.equal(schemaType({ type: 'integer' }), 'number');
  assert.equal(schemaType({ type: ['string', 'null'] }), 'string | null');
  assert.equal(schemaType({ enum: ['a', 'b', 1] }), '"a" | "b" | 1');
  assert.equal(schemaType({ const: 'x' }), '"x"');
  assert.equal(schemaType({ type: 'array', items: { type: 'number' } }), 'Array<number>');
  assert.equal(schemaType({ anyOf: [{ type: 'string' }, { type: 'array', items: { type: 'string' } }] }), 'string | Array<string>');
  assert.equal(
    schemaType({ type: 'object', properties: { a: { type: 'string', description: 'A.' }, 'b-c': { type: 'boolean' } }, required: ['a'] }),
    '{\n  a: string;\n  "b-c"?: boolean;\n}',
  );
  assert.equal(schemaType({ type: 'object', additionalProperties: { type: 'number' } }), '{\n  [key: string]: number;\n}');
  assert.equal(schemaType({ type: 'object' }), 'Record<string, unknown>');
  assert.equal(schemaType({ $ref: '#/x' }), 'unknown', 'outside the subset: unknown, not an error');
  assert.deepEqual(methodNames(counter), ['add', 'ask', 'primes', 'job']);
});

test('the generated .d.ts compiles against a right use and fails against a wrong argument', () => {
  const dir = mkdtempSync(join(tmpdir(), 'architect-typegen-'));
  try {
    const { dts, js } = typegen(counter);
    writeFileSync(join(dir, 'actions.d.ts'), dts);
    writeFileSync(join(dir, 'actions.js'), js);
    writeFileSync(
      join(dir, 'use.ts'),
      `import { loadExtension } from '@fasttrackstudio/architect';
import { bind, id, type Actions } from './actions.js';

export async function use(): Promise<void> {
  const ext = await loadExtension<Actions>('/counter/', { integrity: false });
  await ext.run('LAB_WASI_COUNTER_ADD', { by: 5 });
  await ext.run('LAB_WASI_COUNTER_ADD');
  await ext.run('LAB_WASI_COUNTER_ASK');
  const counter = bind(ext);
  await counter.add({ by: 1 });
  await counter.primes({ n: 1000 });
  await counter.job();
  const which: 'lab.wasi-counter' = id;

  // @ts-expect-error: by is a number
  await ext.run('LAB_WASI_COUNTER_ADD', { by: 'five' });
  // @ts-expect-error: no such action
  await ext.run('LAB_WASI_COUNTER_NOPE');
  // @ts-expect-error: a misspelt argument
  await ext.run('LAB_WASI_COUNTER_ADD', { bye: 5 });
  // @ts-expect-error: ASK takes none
  await ext.run('LAB_WASI_COUNTER_ASK', { by: 1 });
  // @ts-expect-error: by is a number
  await counter.add({ by: '1' });
  // @ts-expect-error: no such method
  await counter.subtract();
  // @ts-expect-error: integrity is required (false says: unchecked)
  await loadExtension('/counter/', {});
  void which;
}
`,
    );
    writeFileSync(
      join(dir, 'tsconfig.json'),
      JSON.stringify({
        compilerOptions: {
          strict: true,
          noEmit: true,
          target: 'ES2022',
          module: 'NodeNext',
          moduleResolution: 'NodeNext',
          skipLibCheck: false,
          types: [],
          paths: { '@fasttrackstudio/architect': [join(web, 'index.d.ts')] },
        },
        files: ['use.ts'],
      }),
    );
    const tsc = join(web, 'node_modules/typescript/bin/tsc');
    try {
      execFileSync(process.execPath, [tsc, '-p', dir], { stdio: 'pipe' });
    } catch (e) {
      assert.fail(`the right use did not compile:\n${e.stdout}`);
    }
    // Without the markers the same file fails, once per wrong use.
    writeFileSync(join(dir, 'use.ts'), readFileSync(join(dir, 'use.ts'), 'utf8').replace(/\/\/ @ts-expect-error.*\n/g, ''));
    let out = '';
    assert.throws(() => {
      try {
        execFileSync(process.execPath, [tsc, '-p', dir], { stdio: 'pipe' });
      } catch (e) {
        out = e.stdout.toString();
        throw e;
      }
    });
    assert.equal(out.match(/error TS/g)?.length, 7, out);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the React wrapper’s types: a use compiles, a wrong prop does not', () => {
  const dir = mkdtempSync(join(tmpdir(), 'architect-react-'));
  try {
    writeFileSync(
      join(dir, 'view.tsx'),
      `import { ArchitectView } from '@fasttrackstudio/architect/react';
import type { Extension } from '@fasttrackstudio/architect';

export function Counter({ ext }: { ext: Extension | null }) {
  return (
    <>
      <ArchitectView extension={ext} view="counter" className="pane" />
      <ArchitectView extension={ext} view="bars" input={{ claim: 'fence:bars', source: '| C | G |' }} />
      {/* @ts-expect-error: view is required */}
      <ArchitectView extension={ext} />
    </>
  );
}
`,
    );
    writeFileSync(
      join(dir, 'tsconfig.json'),
      JSON.stringify({
        compilerOptions: {
          strict: true,
          noEmit: true,
          jsx: 'react-jsx',
          target: 'ES2022',
          module: 'ESNext',
          moduleResolution: 'Bundler',
          typeRoots: [join(web, 'node_modules/@types')],
          types: [],
          paths: {
            '@fasttrackstudio/architect': [join(web, 'index.d.ts')],
            '@fasttrackstudio/architect/react': [join(web, 'react.d.ts')],
            react: [join(web, 'node_modules/@types/react')],
            'react/jsx-runtime': [join(web, 'node_modules/@types/react/jsx-runtime')],
          },
        },
        files: ['view.tsx'],
      }),
    );
    try {
      execFileSync(process.execPath, [join(web, 'node_modules/typescript/bin/tsc'), '-p', dir], { stdio: 'pipe' });
    } catch (e) {
      assert.fail(`it did not compile:\n${e.stdout}`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ---- P5: a manifest never becomes code ---------------------------------------

import vm from 'node:vm';
import ts from 'typescript';
import { checkManifest } from '../manifest.mjs';

/// Strings a hostile manifest could carry (the review of #539's payloads
/// and the classic ways out of a comment, a string or a key).
const HOSTILE = [
  '1\nglobalThis.PWNED=true;//',
  '*/ globalThis.PWNED=true; /*',
  '"; globalThis.PWNED=true; "',
  "'; globalThis.PWNED=true; '",
  '`${globalThis.PWNED=true}`',
  ' globalThis.PWNED=true;//',
  '</script><script>globalThis.PWNED=true</script>',
  '\\u0022; globalThis.PWNED=true; //',
  '\u2029globalThis.PWNED=true;//',
  '__proto__',
  'constructor',
  'x'.repeat(200000),
  '*/'.repeat(5000),
];

function hostileManifests() {
  const base = () => ({
    id: 'lab.wasi-counter',
    name: 'Counter',
    version: '0.1.0',
    actions: [{ id: 'LAB_WASI_COUNTER_ADD', name: 'Add', params: '{"type":"object","properties":{"by":{"type":"integer"}}}' }],
  });
  const out = [];
  // The reviewer's two, as written.
  out.push({ ...base(), version: '1\nglobalThis.PWNED_HEAD=true;//' });
  out.push({ ...base(), actions: [{ id: 'EVIL_EXT_A:(globalThis.PWNED=true),B', name: 'x' }] , id: 'evil.ext' });
  for (const s of HOSTILE) {
    out.push({ ...base(), id: s });
    out.push({ ...base(), version: s });
    out.push({ ...base(), name: s });
    out.push({ ...base(), actions: [{ ...base().actions[0], id: `LAB_WASI_COUNTER_${s}` }] });
    out.push({ ...base(), actions: [{ ...base().actions[0], name: s, description: s }] });
    out.push({ ...base(), actions: [{ ...base().actions[0], effect: s }] });
    out.push({
      ...base(),
      actions: [{ ...base().actions[0], params: JSON.stringify({ type: 'object', description: s, properties: { [s]: { type: 'string', description: s, enum: [s] } } }) }],
    });
  }
  return out;
}

/// The shape of a source file: its statements' kinds, and any syntax error.
function shape(text) {
  const f = ts.createSourceFile('x.d.ts', text, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
  return { kinds: f.statements.map((s) => ts.SyntaxKind[s.kind]), errors: f.parseDiagnostics.length };
}

test('a hostile manifest is refused or emits only data: same AST shape, nothing runs', () => {
  const good = typegen(counter);
  const goodDts = shape(good.dts);
  const goodJs = shape(good.js);
  let refused = 0;
  for (const m of hostileManifests()) {
    let out;
    try {
      checkManifest(m);
      out = typegen(m);
    } catch (e) {
      assert.match(String(e.message), /manifest/, `refused with a reason: ${e.message}`);
      refused++;
      continue;
    }
    const d = shape(out.dts);
    assert.equal(d.errors, 0, `the .d.ts parses: ${JSON.stringify(m)}`);
    assert.deepEqual(d.kinds, goodDts.kinds, `the .d.ts keeps its shape: ${JSON.stringify(m)}\n${out.dts}`);
    const j = shape(out.js);
    assert.equal(j.errors, 0);
    assert.deepEqual(j.kinds, goodJs.kinds, `the .js keeps its shape: ${JSON.stringify(m)}\n${out.js}`);
    // Run it, and bind() it, in a context with a tripwire.
    const tripped = [];
    const context = vm.createContext({ console: { log: () => tripped.push('console.log') } });
    Object.defineProperty(context, 'PWNED', { set: () => tripped.push('PWNED'), get: () => undefined });
    Object.defineProperty(context, 'PWNED_HEAD', { set: () => tripped.push('PWNED_HEAD'), get: () => undefined });
    const js = out.js.replace(/^export /gm, '');
    vm.runInContext(`${js}\nbind({ run: () => null });`, context);
    assert.deepEqual(tripped, [], `nothing ran: ${JSON.stringify(m)}`);
  }
  assert.ok(refused > 0, 'bad ids and versions are refused');
});

// S6 (second review of #539).
test('S6: a schema that grows as a power of its depth is refused in time', () => {
  // `type` lists nested in themselves: 773 bytes took 9 s and crashed.
  let schema = { type: 'string' };
  // Six levels: about 12^6 types unbounded, a small params text.
  for (let i = 0; i < 6; i++) schema = { type: ['object', 'array', 'string', 'null'], properties: { a: schema, b: schema }, items: schema };
  const params = JSON.stringify(schema);
  assert.ok(params.length < 200000, `${params.length} bytes of params`);
  const m = { ...counter, actions: [{ id: 'LAB_WASI_COUNTER_ADD', name: 'Add', params }] };
  const t = performance.now();
  assert.throws(() => typegen(m), /the manifest is refused: a params schema/);
  assert.ok(performance.now() - t < 1000, `${performance.now() - t} ms`);
});

test('S6: ids that collide once lower-cased get methods of their own', () => {
  const m = {
    ...counter,
    actions: [
      { id: 'LAB_WASI_COUNTER_FOO_BAR', name: 'a' },
      { id: 'LAB_WASI_COUNTER_FOO__BAR', name: 'b' },
      { id: 'LAB_WASI_COUNTER_FOOBAR', name: 'c' },
    ],
  };
  const names = methodNames(m);
  assert.equal(new Set(names).size, 3, names.join());
});

test('S6: build-web writes only into a folder it made, and never through a link', async () => {
  const { mkdtempSync, mkdirSync, writeFileSync, existsSync, symlinkSync } = await import('node:fs');
  const { tmpdir } = await import('node:os');
  const { build } = await import('../cli.mjs');
  const dir = mkdtempSync(join(tmpdir(), 'architect-out-'));
  try {
    const out = join(dir, 'mine');
    mkdirSync(join(out, 'interfaces'), { recursive: true });
    writeFileSync(join(out, 'interfaces', 'keep.txt'), 'the embedder’s');
    assert.throws(() => build('/nonexistent.wasm', out, { manifest: counter }), /has files build-web did not write/);
    assert.ok(existsSync(join(out, 'interfaces', 'keep.txt')), 'nothing deleted');
    symlinkSync(out, join(dir, 'link'));
    assert.throws(() => build('/nonexistent.wasm', join(dir, 'link'), { manifest: counter }), /not a folder build-web may write/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
