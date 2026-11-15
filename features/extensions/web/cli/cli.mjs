#!/usr/bin/env node
// `architect-ext build-web <component.wasm> <out> [--manifest extension.toml]`
// (@fasttrackstudio/architect-cli): an extension's component made ready
// for a web page. Its manifest is checked (manifest.mjs); jco transpiles
// it (transpile.mjs); its manifest and the world it was built for go into
// `<out>/architect.json` (what `loadExtension` reads); a SHA-256 of every
// file into `<out>/architect.lock.json`, whose own digest is printed to
// pin it with (`loadExtension(…, { integrity })`); its typed actions into
// `actions.d.ts` and `actions.js` (typegen.mjs); and the size of
// everything a page downloads is printed.
import { createHash } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { brotliCompressSync, gzipSync, constants } from 'node:zlib';
import { parse } from 'smol-toml';
import { transpile } from './transpile.mjs';
import { typegen } from './typegen.mjs';
import { checkManifest } from './manifest.mjs';
import { check as checkTs, keygen, pack, scaffold, writeTypes } from './ts.mjs';

const here = dirname(fileURLToPath(import.meta.url));

/// A transfer size over this (brotli, KiB) is warned about.
export const BUDGET_KIB = 1024;

/// What @fasttrackstudio/architect's own code adds to a page (before the
/// embedder's bundler minifies it).
const RUNTIME = ['index.js', 'runner.js', 'check.js', 'host.js', 'sanitize.js', 'view.js', 'world.js', 'dist/instance-source.js'];

/// The world's versions a transpiled module names (jco keeps them in its
/// import and export names); the runtime's world.js `worldsIn`.
export function worldsIn(moduleText) {
  const found = new Set();
  for (const m of moduleText.matchAll(/architect:extension\/[a-z][a-z0-9-]*@(\d+\.\d+\.\d+)/g)) found.add(m[1]);
  return [...found];
}

/// Where the runtime package is: installed beside this one, or (in the
/// repo) this folder's parent.
function runtimeDir() {
  try {
    return dirname(fileURLToPath(import.meta.resolve('@fasttrackstudio/architect')));
  } catch {
    const repo = join(here, '..');
    return existsSync(join(repo, 'runner.js')) ? repo : null;
  }
}

/// `sha256-<base64>` (Subresource Integrity's form).
export const sri = (bytes) => `sha256-${createHash('sha256').update(bytes).digest('base64')}`;

const sizes = (bytes) => ({
  raw: bytes.length,
  gzip: gzipSync(bytes, { level: 9 }).length,
  brotli: brotliCompressSync(bytes, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length,
});

/// Every file under `dir`, never through a link.
function walk(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    const st = lstatSync(p);
    if (st.isSymbolicLink()) return [];
    return st.isDirectory() ? walk(p) : [p];
  });
}

/// Folder names that are a site's root (what a dev server or host serves
/// at `/`).
const SITE_ROOTS = ['public', 'dist', 'www', 'static', 'build', 'out', 'site', '_site', 'htdocs'];

/// What marks a folder build-web made (and may write and delete in).
export const MARKER = '.architect-build';

/// `out`, made or already build-web's: a folder with something else in
/// it, or a link, is refused, so build-web never deletes what it did not
/// write.
function outFolder(out) {
  if (existsSync(out)) {
    const st = lstatSync(out);
    if (st.isSymbolicLink() || !st.isDirectory()) throw new Error(`${out} is not a folder build-web may write (a link, or a file)`);
    if (readdirSync(out).length && !existsSync(join(out, MARKER)))
      throw new Error(`${out} has files build-web did not write (no ${MARKER}): give it an empty or new folder`);
  }
  mkdirSync(out, { recursive: true });
  writeFileSync(join(out, MARKER), 'written by architect-ext build-web: it may replace what is in this folder\n');
}

/// The size of each file a page downloads from `out`, and the package's
/// runtime's: `{ files: [{ file, raw, gzip, brotli }], total, runtime }`.
export function sizeReport(out) {
  const files = walk(out)
    .filter((p) => /\.(js|wasm|json)$/.test(p) && !/actions\.js$|architect\.lock\.json$/.test(p))
    .sort()
    .map((p) => ({ file: relative(out, p), ...sizes(readFileSync(p)) }));
  const sum = (list) => list.reduce((t, f) => ({ raw: t.raw + f.raw, gzip: t.gzip + f.gzip, brotli: t.brotli + f.brotli }), { raw: 0, gzip: 0, brotli: 0 });
  const dir = runtimeDir();
  const runtime = dir ? sum(RUNTIME.filter((f) => existsSync(join(dir, f))).map((f) => sizes(readFileSync(join(dir, f))))) : null;
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
    ...(runtime ? [row('@fasttrackstudio/architect (unminified)', runtime)] : []),
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
  checkManifest(manifest);
  name ??= basename(component, '.wasm');
  // A site's root as the extension's folder: everything served there
  // would be a file beside its module (the runtime refuses a folder at
  // `/`). A folder of its own, inside the site.
  if (SITE_ROOTS.includes(basename(resolve(out)).toLowerCase()))
    throw new Error(`${out} looks like a site's root: build into a folder of its own (${join(out, manifest.id)})`);
  outFolder(out);
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
  const metadata = { module: basename(module), world: worlds[0], manifest };
  writeFileSync(join(out, 'architect.json'), `${JSON.stringify(metadata, null, 2)}\n`);
  // Every file a page loads, pinned: the lock, and the lock's own digest.
  const files = {};
  for (const p of walk(out).sort()) {
    const rel = relative(out, p).split('\\').join('/');
    if (rel === 'architect.lock.json' || rel === MARKER || /^actions\.(js|d\.ts)$/.test(rel)) continue;
    files[rel] = sri(readFileSync(p));
  }
  const lockText = `${JSON.stringify({ lockfileVersion: 1, files }, null, 2)}\n`;
  writeFileSync(join(out, 'architect.lock.json'), lockText);
  const integrity = sri(Buffer.from(lockText));
  const { dts, js } = typegen(manifest, '@fasttrackstudio/architect', integrity);
  const typesDir = types ?? out;
  mkdirSync(typesDir, { recursive: true });
  writeFileSync(join(typesDir, 'actions.d.ts'), dts);
  writeFileSync(join(typesDir, 'actions.js'), js);
  return { metadata, integrity, report: sizeReport(out) };
}

const USAGE = `usage: architect-ext build-web <component.wasm> <out-dir> [--manifest extension.toml] [--types dir] [--name name] [--budget KiB]

Transpiles an Architect extension's WASI component for a web page (jco), writes
<out-dir>/architect.json (its manifest and world, what loadExtension reads),
<out-dir>/architect.lock.json (a SHA-256 of each file; its digest is printed: pin it),
<out-dir>/actions.d.ts and actions.js (its typed actions; --types puts them in
your source instead), and prints the size of everything a page downloads.`;

const TS_USAGE = `TypeScript extensions (kind = "typescript", #536): actions as .ts files beside TSX pages.

  architect-ext new --typescript <id> [dir]   a new one: a manifest, an action, a page, its types
  architect-ext types [dir]                   types/architect.d.ts and types/actions.d.ts (each
                                              action's arguments, from its manifest's params)
  architect-ext check [dir]                   the manifest and its files, then tsc --noEmit
  architect-ext pack [dir] [--out F] [--key F] its .archext, signed with a minisign secret key
                                              (--key, or ARCHITECT_EXT_SIGNING_KEY: the key's text)
  architect-ext keygen <dir>                  a throwaway key pair, for trials (never a release key)`;

/// The TypeScript commands; `null` when `command` is none of them.
function tsMain(command, rest) {
  const flag = (name) => {
    const i = rest.indexOf(name);
    if (i < 0) return undefined;
    const v = rest[i + 1];
    rest.splice(i, 2);
    return v;
  };
  try {
    switch (command) {
      case 'new': {
        if (!rest.includes('--typescript')) throw new Error('new makes TypeScript extensions: architect-ext new --typescript <id> [dir]');
        rest.splice(rest.indexOf('--typescript'), 1);
        const [id, dir = id] = rest;
        if (!id) throw new Error('new --typescript <id> [dir]');
        const files = scaffold(id, dir);
        console.log(`${id} in ${dir}: ${files.join(', ')}, types/`);
        console.log(`load it as a development copy (EXT_DEV ${resolve(dir)}), then: architect-ext check ${dir}`);
        return 0;
      }
      case 'types':
        for (const f of writeTypes(rest[0] ?? '.')) console.log(f);
        return 0;
      case 'check':
        checkTs(rest[0] ?? '.');
        console.log('ok');
        return 0;
      case 'pack': {
        const out = flag('--out');
        const keyFile = flag('--key');
        const dir = rest[0] ?? '.';
        const key = keyFile ? readFileSync(keyFile, 'utf8') : process.env.ARCHITECT_EXT_SIGNING_KEY;
        const { bytes, index } = pack(dir, key);
        const path = out ?? `${index.id}-${index.version}.archext`;
        writeFileSync(path, bytes);
        console.log(`${index.id} ${index.version}: ${index.files.length} files → ${path}${key ? ', signed' : ', unsigned (an app installs only signed archives: --key)'}`);
        return 0;
      }
      case 'keygen': {
        const dir = rest[0];
        if (!dir) throw new Error('keygen <dir>');
        mkdirSync(dir, { recursive: true });
        const pair = keygen();
        writeFileSync(join(dir, 'architect-ext.key'), pair.secret, { mode: 0o600 });
        writeFileSync(join(dir, 'architect-ext.pub'), pair.public);
        console.log(`${join(dir, 'architect-ext.key')} and .pub: a throwaway pair, never a release key`);
        return 0;
      }
      default:
        return null;
    }
  } catch (e) {
    console.error(`architect-ext ${command}: ${e.message}`);
    return 1;
  }
}

function main(argv) {
  const [command, ...rest] = argv;
  if (!rest.includes('--help') && !rest.includes('-h')) {
    const done = tsMain(command, [...rest]);
    if (done !== null) return done;
  }
  if (command !== 'build-web' || rest.includes('--help') || rest.includes('-h')) {
    console.error(USAGE);
    console.error(`\n${TS_USAGE}`);
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
    const { metadata, integrity, report } = build(pos[0], pos[1], opts);
    console.log(`${metadata.manifest.id} ${metadata.manifest.version}, architect:extension@${metadata.world} → ${pos[1]}`);
    console.log(`integrity: ${integrity}   (loadExtension(url, { integrity: '${integrity}' }))`);
    console.log(formatReport(report, opts.budget ?? BUDGET_KIB));
    return 0;
  } catch (e) {
    console.error(`architect-ext build-web: ${e.message}`);
    return 1;
  }
}

// Run as a command (npm's bin is a link to this file), not imported.
if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) process.exitCode = main(process.argv.slice(2));
