// What the package ships that is made from its sources: instance.js as a
// string (an instance's Web Worker is started from it as a blob, so an
// embedder's bundler never has to know about it, as the Rust side's
// `include_str!` does), and the repo's licence files beside it.
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = join(dirname(fileURLToPath(import.meta.url)), '..');
mkdirSync(join(here, 'dist'), { recursive: true });
const source = readFileSync(join(here, 'instance.js'), 'utf8');
writeFileSync(
  join(here, 'dist/instance-source.js'),
  `// Generated from instance.js by scripts/build.mjs: do not edit.\nexport default ${JSON.stringify(source)};\n`,
);
for (const f of ['LICENSE-MIT', 'LICENSE-APACHE']) copyFileSync(join(here, '../../..', f), join(here, f));
