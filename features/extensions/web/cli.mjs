#!/usr/bin/env node
// `architect build-web <component.wasm> <out> [--manifest extension.toml]`:
// an extension's component made ready for a web page. jco transpiles it
// (transpile.mjs), its manifest and the world it was built for go into
// `<out>/architect.json` (what `loadExtension` reads), its typed actions
// into `<out>/actions.d.ts` and `<out>/actions.js` (typegen.mjs), and the
// size of everything a page downloads is printed.
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { brotliCompressSync, gzipSync, constants } from 'node:zlib';
import { parse } from 'smol-toml';
import { transpile } from './transpile.mjs';
import { typegen } from './typegen.mjs';
import { refusal, worldsIn } from './world.js';

const here = dirname(fileURLToPath(import.meta.url));

/// A transfer size over this (brotli, KiB) is warned about.
export const BUDGET_KIB = 1024;

/// What the package's own code adds to a page (before the embedder's
/// bundler minifies it).
const RUNTIME = ['index.js', 'runner.js', 'check.js', 'host.js', 'sanitize.js', 'view.js', 'world.js', 'dist/instance-source.js'];

const sizes = (bytes) => ({
  raw: bytes.length,
  gzip: gzipSync(bytes, { level: 9 }).length,
  brotli: brotliCompressSync(bytes, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length,
});

function walk(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

/// The size of each file a page downloads from `out`, and the package's
/// runtime's: `{ files: [{ file, raw, gzip, brotli }], total, runtime }`.
export function sizeReport(out) {
  const files = walk(out)
    .filter((p) => /\.(js|wasm|json)$/.test(p) && !/actions\.js$/.test(p))
    .sort()
    .map((p) => ({ file: relative(out, p), ...sizes(readFileSync(p)) }));
  const sum = (list) => list.reduce((t, f) => ({ raw: t.raw + f.raw, gzip: t.gzip + f.gzip, brotli: t.brotli + f.brotli }), { raw: 0, gzip: 0, brotli: 0 });
  const runtime = sum(RUNTIME.filter((f) => existsSync(join(here, f))).map((f) => sizes(readFileSync(join(here, f)))));
  return { files, total: sum(files), runtime };
}

const kib = (n) => `${(n / 1024).toFixed(1)} KiB`;

export function formatReport({ files, total, runtime }, budget = BUDGET_KIB) {
  const w = Math.max(...files.map((f) => f.file.length), 30);
  const row = (name, s) => `${name.padEnd(w)}  ${kib(s.raw).padStart(11)}  ${kib(s.gzip).padStart(11)}  ${kib(s.brotli).padStart(11)}`;
  const lines = [
    `${'what a page downloads'.padEnd(w)}  ${'raw'.padStart(11)}  ${'gzip'.padStart(11)}  ${'brotli'.padStart(11)}`,
    ...files.map((f) => row(f.file, f)),
    row('the extension, in all', total),
    row('@fasttrackstudio/architect (unminified)', runtime),
  ];
  if (total.brotli > budget * 1024)
    lines.push(`warning: the extension is ${kib(total.brotli)} over the wire (brotli), past the budget of ${budget} KiB: build it with --release, strip it, or split its heavy work into a worker component`);
  return lines.join('\n');
}

/// The manifest beside `component`, or in the working folder.
function findManifest(component) {
  for (const p of [join(dirname(component), 'extension.toml'), resolve('extension.toml')]) if (existsSync(p)) return p;
  return null;
}

/// Builds `component` into `out` for the web: its metadata, typed actions
/// and size report. `manifest` its parsed manifest (else read from
/// `manifestPath`).
export function build(component, out, { manifest, manifestPath, name, types } = {}) {
  if (!manifest) {
    const path = manifestPath ?? findManifest(component);
    if (!path) throw new Error(`no extension.toml beside ${component} or here: pass --manifest <extension.toml>`);
    manifest = parse(readFileSync(path, 'utf8'));
  }
  for (const k of ['id', 'name', 'version']) if (!manifest[k]) throw new Error(`the manifest has no ${k}`);
  name ??= basename(component, '.wasm');
  const module = transpile(component, out, name);
  // jco's own declarations describe its raw interface, not this package's.
  rmSync(join(out, `${name}.d.ts`), { force: true });
  rmSync(join(out, 'interfaces'), { recursive: true, force: true });
  const worlds = worldsIn(readFileSync(module, 'utf8'));
  if (worlds.length !== 1)
    throw new Error(
      worlds.length
        ? `${component} names more than one architect:extension world (${worlds.join(', ')})`
        : `${component} is not an Architect extension: it exports no architect:extension interface`,
    );
  const why = refusal(worlds[0], manifest.api, manifest.id);
  if (why) throw new Error(why);
  const metadata = { module: basename(module), world: worlds[0], manifest };
  writeFileSync(join(out, 'architect.json'), `${JSON.stringify(metadata, null, 2)}\n`);
  const { dts, js } = typegen(manifest);
  const typesDir = types ?? out;
  mkdirSync(typesDir, { recursive: true });
  writeFileSync(join(typesDir, 'actions.d.ts'), dts);
  writeFileSync(join(typesDir, 'actions.js'), js);
  return { metadata, report: sizeReport(out) };
}

const USAGE = `usage: architect build-web <component.wasm> <out-dir> [--manifest extension.toml] [--types dir] [--name name] [--budget KiB]

Transpiles an Architect extension's WASI component for a web page (jco), writes
<out-dir>/architect.json (its manifest and world, what loadExtension reads),
<out-dir>/actions.d.ts and actions.js (its typed actions; --types puts them in
your source instead), and prints the size of everything a page downloads.`;

function main(argv) {
  const [command, ...rest] = argv;
  if (command !== 'build-web' || rest.includes('--help') || rest.includes('-h')) {
    console.error(USAGE);
    return command === '--help' || command === '-h' || rest.includes('--help') ? 0 : 2;
  }
  const opts = {};
  const pos = [];
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i];
    if (a === '--manifest') opts.manifestPath = rest[++i];
    else if (a === '--name') opts.name = rest[++i];
    else if (a === '--types') opts.types = rest[++i];
    else if (a === '--budget') opts.budget = Number(rest[++i]);
    else pos.push(a);
  }
  if (pos.length !== 2) {
    console.error(USAGE);
    return 2;
  }
  try {
    const { metadata, report } = build(pos[0], pos[1], opts);
    console.log(`${metadata.manifest.id} ${metadata.manifest.version}, architect:extension@${metadata.world} → ${pos[1]}`);
    console.log(formatReport(report, opts.budget ?? BUDGET_KIB));
    return 0;
  } catch (e) {
    console.error(`architect build-web: ${e.message}`);
    return 1;
  }
}

// Run as a command (npm's bin is a link to this file), not imported.
if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) process.exitCode = main(process.argv.slice(2));
