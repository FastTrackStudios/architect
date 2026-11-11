#!/usr/bin/env node
// The repo's path to the transpiler (justfile, test/harness.mjs,
// docs): it lives in the CLI package, cli/transpile.mjs.
//
//   node features/extensions/web/transpile.mjs <component.wasm> <out-dir> [name]
import { fileURLToPath } from 'node:url';
import { transpile } from './cli/transpile.mjs';

export * from './cli/transpile.mjs';

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [component, out, name] = process.argv.slice(2);
  if (!component || !out) {
    console.error('usage: transpile.mjs <component.wasm> <out-dir> [name]');
    process.exit(2);
  }
  console.log(transpile(component, out, name));
}
