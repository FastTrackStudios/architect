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
const counter = parse(readFileSync(join(here, '../../../apps/sandbox/wasi-counter/extension.toml'), 'utf8'));

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
    '{\n  /** A. */\n  a: string;\n  "b-c"?: boolean;\n}',
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
  const ext = await loadExtension<Actions>('/counter/');
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
          paths: { '@fasttrackstudio/architect': [join(here, 'index.d.ts')] },
        },
        files: ['use.ts'],
      }),
    );
    const tsc = join(here, 'node_modules/typescript/bin/tsc');
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
    assert.equal(out.match(/error TS/g)?.length, 6, out);
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
          typeRoots: [join(here, 'node_modules/@types')],
          types: [],
          paths: {
            '@fasttrackstudio/architect': [join(here, 'index.d.ts')],
            '@fasttrackstudio/architect/react': [join(here, 'react.d.ts')],
            react: [join(here, 'node_modules/@types/react')],
            'react/jsx-runtime': [join(here, 'node_modules/@types/react/jsx-runtime')],
          },
        },
        files: ['view.tsx'],
      }),
    );
    try {
      execFileSync(process.execPath, [join(here, 'node_modules/typescript/bin/tsc'), '-p', dir], { stdio: 'pipe' });
    } catch (e) {
      assert.fail(`it did not compile:\n${e.stdout}`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
