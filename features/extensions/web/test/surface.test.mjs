// The public surface is deliberate: what index.js exports is what
// index.d.ts declares, and nothing internal (the check, the answerer,
// a component's insides) is reachable from an import or an Extension.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

function declared(file) {
  const src = ts.createSourceFile(file, readFileSync(new URL(`../${file}`, import.meta.url), 'utf8'), ts.ScriptTarget.ES2022, true);
  const names = [];
  for (const st of src.statements) {
    const exported = ts.getCombinedModifierFlags(st) & ts.ModifierFlags.Export;
    if (!exported) continue;
    if (ts.isVariableStatement(st)) st.declarationList.declarations.forEach((d) => names.push(d.name.getText()));
    else if ((ts.isFunctionDeclaration(st) || ts.isClassDeclaration(st)) && st.name) names.push(st.name.text);
  }
  return names.sort();
}

test('index.js exports exactly the values index.d.ts declares', async () => {
  const js = Object.keys(await import('../index.js')).sort();
  assert.deepEqual(js, declared('index.d.ts'));
  for (const internal of ['check', 'within', 'noCapabilities', 'actionPrefix', 'describe', 'answerer', 'Component', 'Pane'])
    assert.ok(!js.includes(internal), internal);
});

test('react.js and node.js too', async () => {
  assert.deepEqual(Object.keys(await import('../node.js')).sort(), declared('node.d.ts'));
  const react = readFileSync(new URL('../react.js', import.meta.url), 'utf8');
  assert.deepEqual([...react.matchAll(/^export function (\w+)/gm)].map((m) => m[1]), declared('react.d.ts'));
});

test('an Extension cannot be made by hand', async () => {
  const { Extension } = await import('../index.js');
  assert.throws(() => new Extension(), /comes from loadExtension/);
});
