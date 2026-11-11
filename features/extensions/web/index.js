// @fasttrackstudio/architect: an Architect extension (a WASI component)
// in any web app. `loadExtension` starts one from what `architect-ext
// build-web` wrote; in a page its JS runs in an opaque-origin sandbox frame
// (runner.js `Sandbox`), its actions in Web Workers under memory and time
// limits; what it asks of the app reaches the embedder's `Host` only within
// what the embedder granted; `mountView` draws one of its views into an
// element. docs/content/guide/extensions-in-your-web-app.md.
import { Component, LIMITS, WORKER_LIMITS, SANDBOX_HASH, SANDBOX_CSP } from './runner.js';
import INSTANCE_SOURCE from './dist/instance-source.js';
import { capabilities, noCapabilities, within } from './check.js';
import { answerer, ArchitectError } from './host.js';
import { Pane, ViewError } from './view.js';
import { refusal } from './world.js';

export { ArchitectError, memoryHost, fetchHandler } from './host.js';
export { WORLD, API } from './world.js';
export { LIMITS, WORKER_LIMITS, SANDBOX_HASH, SANDBOX_CSP };

/// What `build-web` writes beside the transpiled module.
export const METADATA = 'architect.json';
/// Its lock: a SHA-256 of every file it wrote.
export const LOCK = 'architect.lock.json';

/// Whether this browser (or node) can run an extension: JS Promise
/// Integration (`WebAssembly.Suspending`, `WebAssembly.promising`).
export function isSupported() {
  return typeof WebAssembly === 'object' && typeof WebAssembly.Suspending === 'function' && typeof WebAssembly.promising === 'function';
}

const UNSUPPORTED =
  'this browser cannot run Architect extensions: it lacks JS Promise Integration (WebAssembly.Suspending). Chromium 137+, Firefox and node 24+ have it; Safari does not yet.';

// ---- files -------------------------------------------------------------------

/// `path` when it names a file inside the extension's folder: relative,
/// no scheme, no `..`, `.` or empty part, no `\`, `?`, `#` or `%`.
function insideFolder(path) {
  const p = String(path ?? '');
  // eslint-disable-next-line no-control-regex
  if (!p || p.length > 256 || /^[a-z][a-z0-9+.-]*:/i.test(p) || p.startsWith('/') || /[\\?#%\u0000-\u001f]/.test(p)) return null;
  const parts = p.split('/');
  if (parts.some((s) => s === '' || s === '.' || s === '..')) return null;
  return parts.join('/');
}

async function readBytes(url) {
  if (url.protocol === 'file:') {
    // Under node only: named so a page's bundler leaves it out.
    const fsModule = 'node:fs/promises';
    const fs = await import(/* @vite-ignore */ /* webpackIgnore: true */ fsModule);
    return new Uint8Array(await fs.readFile(url));
  }
  const r = await fetch(url, { credentials: 'same-origin' });
  if (!r.ok) throw new Error(`${url}: ${r.status} ${r.statusText}`);
  return new Uint8Array(await r.arrayBuffer());
}

/// `sha256-<base64>` of `bytes` (Subresource Integrity's form).
async function digest(bytes) {
  const d = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  let s = '';
  for (const b of d) s += String.fromCharCode(b);
  return `sha256-${btoa(s)}`;
}

const refuse = (why) => new ArchitectError({ kind: 'refused', capability: 'integrity', why });

/// The files of the folder at `folder`: each read once, checked against
/// `lock` (when there is one) before anything uses it.
function folderReader(folder, lock) {
  const cache = new Map();
  return (path) => {
    const rel = insideFolder(path);
    if (!rel) return Promise.reject(refuse(`${JSON.stringify(path)} is not a file inside the extension's folder`));
    if (!cache.has(rel)) {
      cache.set(
        rel,
        (async () => {
          const want = lock?.files?.[rel];
          if (lock && !want) throw refuse(`${rel} is not in its lock`);
          const bytes = await readBytes(new URL(rel, folder));
          if (want) {
            const got = await digest(bytes);
            if (got !== want) throw refuse(`${rel} is not the file its lock names (${got}, not ${want})`);
          }
          return bytes;
        })(),
      );
    }
    return cache.get(rel);
  };
}

const json = (bytes) => JSON.parse(new TextDecoder().decode(bytes));

function base() {
  return globalThis.location?.href ?? import.meta.url;
}

/// `source` as `{ folder, metadata, read }`.
async function resolve(source, integrity) {
  if (source && typeof source === 'object' && !(source instanceof URL)) {
    if (!source.manifest || !source.module || !source.folder)
      throw new TypeError('loadExtension: a source object needs `folder`, `manifest` and `module`');
    const folder = new URL(String(source.folder).replace(/\/?$/, '/'), base());
    const lock = typeof integrity === 'object' && integrity ? integrity : null;
    return { folder, metadata: source, read: folderReader(folder, lock) };
  }
  let at = new URL(String(source), base());
  if (at.pathname.endsWith(`/${METADATA}`)) at = new URL('.', at);
  const folder = new URL(at.pathname.replace(/\/?$/, '/'), at);
  let lock = null;
  if (typeof integrity === 'string') {
    const bytes = await readBytes(new URL(LOCK, folder));
    const got = await digest(bytes);
    if (got !== integrity) throw refuse(`its lock is not the one pinned (${got}, not ${integrity})`);
    lock = json(bytes);
  } else if (integrity && typeof integrity === 'object') lock = integrity;
  const read = folderReader(folder, lock);
  return { folder, metadata: json(await read(METADATA)), read };
}

// ---- an extension --------------------------------------------------------------

/// What the package keeps of each extension, out of the embedder's reach.
const inner = new WeakMap();

const deepFreeze = (o) => {
  for (const v of Object.values(o)) if (v && typeof v === 'object') deepFreeze(v);
  return Object.freeze(o);
};

/// A loaded extension: its manifest, its actions, `run`, `stop`, and its
/// views for `mountView`.
export class Extension {
  constructor(token) {
    if (token !== inner) throw new TypeError('an Extension comes from loadExtension');
  }

  /// Runs its action `id` with `args` (JSON): its answer. A destructive
  /// one runs only when the host's `confirm` says yes.
  async run(id, args) {
    const me = inner.get(this);
    if (me.stopped) throw new ArchitectError({ kind: 'not_loaded', id: this.manifest.id });
    const action = this.actions.find((a) => a.id === id);
    if (!action) throw new ArchitectError({ kind: 'not_found', what: `action ${id} of ${this.manifest.id}` });
    if (action.effect === 'destructive') {
      const yes = me.host.confirm ? await me.host.confirm({ id, effect: 'destructive', by: 'embedder' }, args ?? null) : false;
      if (yes !== true)
        throw new ArchitectError({
          kind: 'refused',
          capability: 'confirm',
          why: `${id} is destructive: it runs only when the host's confirm says yes`,
        });
    }
    const text = args === undefined || args === null ? '' : JSON.stringify(args);
    let out;
    try {
      out = await me.component.invoke(id, text, JSON.stringify({ source: 'app' }));
    } catch (e) {
      throw new ArchitectError({ kind: 'failed', message: String(e?.message ?? e) });
    }
    if (typeof out !== 'string') return out;
    try {
      return JSON.parse(out);
    } catch {
      return out;
    }
  }

  /// What it runs now: calls, traps, its generation, its workers.
  report() {
    return inner.get(this).component.report();
  }

  /// Unloads it: its instance, its workers, its sandbox and its views end.
  /// Under node, a process with an extension running exits only once it
  /// is stopped or idle.
  stop() {
    const me = inner.get(this);
    if (me.stopped) return;
    me.stopped = true;
    for (const [, p] of me.panes) p.pane.unmount();
    me.panes.clear();
    me.component.stop();
  }
}

function frame(me, pane, f) {
  const p = me.panes.get(pane);
  if (!p) return;
  if (f.failed !== undefined) return p.pane.failed(f.failed);
  try {
    p.pane.apply(f.edits ?? []);
  } catch (e) {
    const why = e instanceof ViewError ? e.message : String(e?.message ?? e);
    p.pane.failed(`its edits could not be applied: ${why}`);
    me.log('warn', 'view', why);
  }
}

function remount(me) {
  for (const [pane, p] of me.panes) {
    p.pane.reset();
    me.component.render(pane, p.view);
  }
}

/// Loads the extension `architect-ext build-web` wrote to `source` and
/// starts it. `options.integrity` pins its files (the lock's digest
/// `build-web` printed, or the lock itself); `options.host` answers what
/// it asks of the app; `options.grant` is what the embedder lets it reach
/// (nothing by default: its manifest's `[capabilities]` are what it asks
/// for, not what it gets).
export async function loadExtension(source, options = {}) {
  if (!isSupported()) throw new ArchitectError({ kind: 'unsupported', what: UNSUPPORTED });
  const { folder, metadata, read } = await resolve(source, options.integrity);
  const manifest = metadata.manifest;
  const why = refusal(metadata.world, manifest?.api, manifest?.id);
  if (why) throw new ArchitectError({ kind: 'unsupported', what: why });
  const module = insideFolder(metadata.module);
  if (!module) throw refuse(`its module ${JSON.stringify(metadata.module)} is not a file inside its folder`);
  const host = options.host ?? {};
  const may = deepFreeze(within(capabilities(manifest.capabilities), options.grant ?? noCapabilities()));
  const limits = { ...LIMITS, ...options.limits };
  const log = (level, src, text) => (options.log ?? host.log)?.({ level, source: src, text });
  const me = { host, panes: new Map(), nextPane: 0, stopped: false, log, limits, component: null };
  const ask = answerer({ manifest, may, host, log });
  const url = new URL(module, folder).href;
  const load = (href) => {
    if (!href.startsWith(folder.href)) return Promise.reject(refuse(`${href} is outside its folder`));
    return read(href.slice(folder.href.length));
  };
  const workers = {};
  for (const w of manifest.workers ?? []) {
    workers[w.id] = { entry: w.entry ?? w.id };
    const wurl = metadata.workers?.[w.id];
    if (wurl) {
      const rel = insideFolder(wurl);
      if (!rel) throw refuse(`worker ${w.id}'s module ${JSON.stringify(wurl)} is not a file inside its folder`);
      workers[w.id].url = new URL(rel, folder).href;
    }
  }
  const component = new Component({
    url,
    instance: options.instance ?? INSTANCE_SOURCE,
    limits,
    workerLimits: options.workerLimits,
    workers,
    host: ask,
    log,
    load,
    inlineGlue: true,
    theme: host.theme ? () => host.theme() : undefined,
    frame: (pane, f) => frame(me, pane, f),
    restarted: () => remount(me),
    ...(options.spawn ? { spawn: options.spawn } : {}),
  });
  me.component = component;
  try {
    await component.start();
  } catch (e) {
    component.stop();
    throw new ArchitectError({ kind: 'failed', message: `${manifest.id} did not start: ${e?.message ?? e}` });
  }
  const ext = new Extension(inner);
  inner.set(ext, me);
  const runs = new Set(component.info.actions);
  Object.defineProperties(ext, {
    manifest: { value: deepFreeze(structuredClone(manifest)), enumerable: true },
    world: { value: metadata.world, enumerable: true },
    capabilities: { value: may, enumerable: true },
    actions: {
      value: deepFreeze(
        (manifest.actions ?? [])
          .filter((a) => runs.has(a.id))
          .map((a) => ({
            id: a.id,
            name: a.name,
            description: a.description ?? '',
            effect: a.effect ?? 'read_only',
            schema: a.params ? JSON.parse(a.params) : null,
          })),
      ),
      enumerable: true,
    },
    views: {
      value: deepFreeze((manifest.views ?? []).filter((v) => component.info.views.includes(v.id)).map((v) => ({ id: v.id, title: v.title ?? v.id }))),
      enumerable: true,
    },
  });
  for (const a of component.info.actions)
    if (!(manifest.actions ?? []).some((d) => d.id === a)) log('warn', 'wasi', `it runs ${a}, which its manifest does not declare`);
  return ext;
}

/// Draws `extension`'s view `viewId` into `element` (emptied first), in a
/// shadow root of its own: its edits applied as they come, events on it
/// sent back. `options.urls`: URL prefixes its images and media may load
/// from (none by default). `unmount()` ends it.
export function mountView(extension, viewId, element, options = {}) {
  const me = inner.get(extension);
  if (!me) throw new TypeError('mountView: not an extension loadExtension made');
  if (!extension.views.some((v) => v.id === viewId))
    throw new ArchitectError({ kind: 'not_found', what: `view ${viewId} of ${extension.manifest.id}` });
  const id = ++me.nextPane;
  const pane = new Pane(element, (event) => me.component.event(id, event), {
    urls: options.urls ?? [],
    limits: me.limits,
  });
  me.panes.set(id, { pane, view: viewId });
  const rendered = me.component.render(id, viewId);
  return {
    id,
    rendered,
    /// Sends what the view's embed input is (an inline embed's
    /// `architect-embed` event: `{claim, source, props}`).
    input: (input) =>
      me.component.event(id, { element: 0, name: 'architect-embed', bubbles: false, data: { tag: 'form', val: JSON.stringify(input) } }),
    unmount: () => {
      if (!me.panes.delete(id)) return;
      pane.unmount();
      me.component.close(id);
    },
  };
}
