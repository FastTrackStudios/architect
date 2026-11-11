// @fasttrackstudio/architect: an Architect extension (a WASI component)
// in any web app. `loadExtension` starts one from what `architect
// build-web` wrote; its actions run in a Web Worker under memory and time
// limits; what it asks of the app reaches the embedder's `Host` only
// within what the embedder granted; `mountView` draws one of its views
// into an element. docs/content/guide/extensions-in-your-web-app.md.
import { Component, LIMITS, WORKER_LIMITS } from './runner.js';
import INSTANCE_SOURCE from './dist/instance-source.js';
import { actionPrefix, capabilities, noCapabilities, within } from './check.js';
import { answerer, ArchitectError } from './host.js';
import { Pane, ViewError } from './view.js';
import { refusal } from './world.js';

export { ArchitectError, describe } from './host.js';
export { memoryHost } from './host.js';
export { actionPrefix, check, within, noCapabilities } from './check.js';
export { WORLD, API } from './world.js';
export { LIMITS, WORKER_LIMITS } from './runner.js';

/// What `architect build-web` writes beside the transpiled module.
export const METADATA = 'architect.json';

async function readJson(url) {
  if (url.protocol === 'file:') {
    // Under node only: named so a page's bundler leaves it out.
    const fsModule = 'node:fs/promises';
    const fs = await import(/* @vite-ignore */ /* webpackIgnore: true */ fsModule);
    return JSON.parse(await fs.readFile(url, 'utf8'));
  }
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: ${r.status} ${r.statusText}`);
  return r.json();
}

function base() {
  return globalThis.location?.href ?? import.meta.url;
}

/// `source` as `{ url, metadata }`: a folder `build-web` wrote (its URL,
/// with or without the trailing slash), its `architect.json`, or the
/// metadata itself with `module` an absolute URL.
async function resolve(source) {
  if (source && typeof source === 'object' && !(source instanceof URL)) {
    if (!source.manifest || !source.module) throw new TypeError('loadExtension: a source object needs `manifest` and `module`');
    return { url: new URL(source.module, base()).href, metadata: source };
  }
  let at = new URL(String(source), base());
  if (!at.pathname.endsWith('.json')) at = new URL(`${at.pathname.replace(/\/?$/, '/')}${METADATA}`, at);
  const metadata = await readJson(at);
  return { url: new URL(metadata.module, at).href, metadata };
}

/// A loaded extension: its manifest, its actions, `run`, `stop`, and its
/// views for `mountView`.
export class Extension {
  constructor({ manifest, metadata, component, may, host, logs }) {
    this.manifest = manifest;
    this.world = metadata.world;
    this.metadata = metadata;
    /// What it may reach: its manifest's capabilities within the grant.
    this.capabilities = may;
    this.host = host;
    this.logs = logs;
    this.component = component;
    this.panes = new Map();
    this.nextPane = 0;
    this.stopped = false;
    const runs = new Set(component.info.actions);
    this.actions = (manifest.actions ?? [])
      .filter((a) => runs.has(a.id))
      .map((a) => ({
        id: a.id,
        name: a.name,
        description: a.description ?? '',
        effect: a.effect ?? 'read_only',
        schema: a.params ? JSON.parse(a.params) : null,
      }));
    this.views = (manifest.views ?? [])
      .filter((v) => component.info.views.includes(v.id))
      .map((v) => ({ id: v.id, title: v.title ?? v.id }));
  }

  /// Runs its action `id` with `args` (JSON): its answer. A destructive
  /// one runs only when the host's `confirm` says yes.
  async run(id, args) {
    if (this.stopped) throw new ArchitectError({ kind: 'not_loaded', id: this.manifest.id });
    const action = this.actions.find((a) => a.id === id);
    if (!action) throw new ArchitectError({ kind: 'not_found', what: `action ${id} of ${this.manifest.id}` });
    if (action.effect === 'destructive') {
      const yes = this.host.confirm ? await this.host.confirm({ id, effect: 'destructive', by: 'embedder' }, args ?? null) : false;
      if (!yes)
        throw new ArchitectError({
          kind: 'refused',
          capability: 'confirm',
          why: `${id} is destructive: it runs only when the host's confirm says yes`,
        });
    }
    const text = args === undefined || args === null ? '' : JSON.stringify(args);
    let out;
    try {
      out = await this.component.invoke(id, text, JSON.stringify({ source: 'app' }));
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
    return this.component.report();
  }

  /// Unloads it: its instance, its workers and its views end.
  stop() {
    if (this.stopped) return;
    this.stopped = true;
    for (const [, p] of this.panes) p.pane.element.replaceChildren();
    this.panes.clear();
    this.component.stop();
  }

  frame(pane, f) {
    const p = this.panes.get(pane);
    if (!p) return;
    if (f.failed !== undefined) return p.pane.failed(f.failed);
    try {
      p.pane.apply(f.edits ?? []);
    } catch (e) {
      const why = e instanceof ViewError ? e.message : String(e?.message ?? e);
      p.pane.failed(`its edits could not be applied: ${why}`);
      this.logs('warn', 'view', why);
    }
  }

  remount() {
    for (const [pane, p] of this.panes) {
      p.pane.reset();
      this.component.render(pane, p.view);
    }
  }
}

/// Loads the extension `architect build-web` wrote to `source` and starts
/// it. `options.host` answers what it asks of the app; `options.grant`
/// is what the embedder lets it reach (nothing by default: its
/// manifest's `[capabilities]` are what it asks for, not what it gets).
export async function loadExtension(source, options = {}) {
  const { url, metadata } = await resolve(source);
  const manifest = metadata.manifest;
  const why = refusal(metadata.world, manifest.api, manifest.id);
  if (why) throw new ArchitectError({ kind: 'unsupported', what: why });
  const host = options.host ?? {};
  const may = within(capabilities(manifest.capabilities), options.grant ?? noCapabilities());
  const logs = (level, source, text) => {
    (options.log ?? host.log)?.({ level, source, text });
  };
  const ask = answerer({ manifest, may, host, log: logs });
  const workers = {};
  for (const w of manifest.workers ?? []) {
    workers[w.id] = { entry: w.entry ?? w.id };
    const wurl = metadata.workers?.[w.id];
    if (wurl) workers[w.id].url = new URL(wurl, url).href;
  }
  let ext = null;
  const component = new Component({
    url,
    instance: options.instance ?? INSTANCE_SOURCE,
    limits: options.limits,
    workerLimits: options.workerLimits,
    workers,
    host: ask,
    log: logs,
    theme: host.theme ? () => host.theme() : undefined,
    frame: (pane, f) => ext?.frame(pane, f),
    restarted: () => ext?.remount(),
    ...(options.spawn ? { spawn: options.spawn } : {}),
    ...(options.load ? { load: options.load } : {}),
  });
  try {
    await component.start();
  } catch (e) {
    component.stop();
    throw new ArchitectError({ kind: 'failed', message: `${manifest.id} did not start: ${e?.message ?? e}` });
  }
  ext = new Extension({ manifest, metadata, component, may, host, logs });
  for (const a of component.info.actions)
    if (!(manifest.actions ?? []).some((d) => d.id === a)) logs('warn', 'wasi', `it runs ${a}, which its manifest does not declare`);
  return ext;
}

/// Draws `extension`'s view `viewId` into `element` (emptied first): its
/// edits applied as they come, events on it sent back. `unmount()` ends it.
export function mountView(extension, viewId, element) {
  if (!extension.views.some((v) => v.id === viewId))
    throw new ArchitectError({ kind: 'not_found', what: `view ${viewId} of ${extension.manifest.id}` });
  const id = ++extension.nextPane;
  const pane = new Pane(element, (event) => extension.component.event(id, event));
  extension.panes.set(id, { pane, view: viewId });
  const rendered = extension.component.render(id, viewId);
  return {
    id,
    rendered,
    /// Sends what the view's embed input is (an inline embed's
    /// `architect-embed` event: `{claim, source, props}`).
    input: (input) =>
      extension.component.event(id, { element: 0, name: 'architect-embed', bubbles: false, data: { tag: 'form', val: JSON.stringify(input) } }),
    unmount: () => {
      if (!extension.panes.delete(id)) return;
      element.replaceChildren();
      extension.component.close(id);
    },
  };
}
