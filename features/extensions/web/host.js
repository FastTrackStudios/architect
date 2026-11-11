// The host an embedder supplies, and the answerer between it and a
// component: every request the component makes of the app arrives as a
// host-service request (`hostcall::HostRequest` JSON), is checked against
// what the extension may reach (check.js, the native `hostcall::check`),
// and only then reaches the embedder's handler for its `op`. An `op` with
// no handler answers "not available in this host": the component gets an
// error it can show, never a throw and never a hang.
import { check, HOST_OPS } from './check.js';

/// The kinds of `hostcall::HostError`.
export const ERROR_KINDS = ['refused', 'confirmation_required', 'not_loaded', 'not_found', 'failed', 'unsupported'];

/// A host-service error (`hostcall::HostError`) as a JS error: `kind`
/// and its fields (`capability` and `why` when refused, `what` when not
/// found or unsupported, `message` when failed), its text as the native
/// host's `Display`.
export class ArchitectError extends Error {
  constructor(error) {
    super(describe(error));
    this.name = 'ArchitectError';
    Object.assign(this, error);
    this.message = describe(error);
  }
  toJSON() {
    const { name: _n, message: _m, stack: _s, ...error } = this;
    return error;
  }
}

/// A host error's words (`HostError`'s `Display`).
export function describe(e) {
  switch (e?.kind) {
    case 'refused':
      return `refused (${e.capability}): ${e.why}`;
    case 'confirmation_required':
      return `${e.action}: a person must confirm it first (ticket ${e.ticket})`;
    case 'not_loaded':
      return `${e.id} is not loaded`;
    case 'not_found':
      return `no ${e.what}`;
    case 'failed':
      return String(e.message);
    case 'unsupported':
      return `unsupported here: ${e.what}`;
    default:
      return String(e?.message ?? e);
  }
}

/// What a handler threw, as a host error.
function asError(e) {
  if (e && ERROR_KINDS.includes(e.kind)) {
    const { name: _n, stack: _s, ...rest } = e;
    if (e instanceof ArchitectError) return e.toJSON();
    return rest.kind === 'failed' ? { kind: 'failed', message: String(rest.message ?? e) } : rest;
  }
  return { kind: 'failed', message: String(e?.message ?? e) };
}

const json = (v) => JSON.stringify(v, (_, x) => (typeof x === 'bigint' ? Number(x) : x));

/// The defaults of a manifest's settings, by key.
function settingDefaults(manifest) {
  const out = new Map();
  for (const s of manifest.sections ?? []) for (const x of s.settings ?? []) out.set(x.key, x.default ?? null);
  return out;
}

/// The answerer of one loaded extension: request JSON in, answer JSON
/// out (`{"ok": …}` or `{"error": {kind, …}}`), never a throw.
///
/// `manifest` its manifest, `may` what it may reach (its capabilities
/// within the grant), `host` the embedder's `Host`, `log(level, source,
/// text)` its log.
export function answerer({ manifest, may, host = {}, log }) {
  const settings = new Map();
  const data = new Map();
  const defaults = settingDefaults(manifest);
  const ctx = { extension: manifest.id, manifest };
  const handlers = host.handle ?? {};

  const builtIn = {
    log: (r) => {
      log(r.level ?? 'info', 'log', String(r.text ?? ''));
      return null;
    },
    data: (r) => (data.has(r.key) ? data.get(r.key) : null),
    set_data: (r) => {
      if (r.value === undefined || r.value === null) data.delete(r.key);
      else data.set(r.key, r.value);
      return null;
    },
    setting: (r) => (settings.has(r.key) ? settings.get(r.key) : defaults.has(r.key) ? defaults.get(r.key) : null),
    set_setting: (r) => {
      if (!defaults.has(r.key)) throw { kind: 'not_found', what: `setting ${r.key} in its manifest` };
      settings.set(r.key, r.value ?? null);
      return null;
    },
    run: async (r) => {
      const action = host.actions?.[r.action];
      if ((action.effect ?? 'mutating') === 'destructive') {
        const yes = host.confirm ? await host.confirm({ id: r.action, effect: 'destructive', by: manifest.id }, r.args ?? null) : false;
        if (!yes) throw { kind: 'confirmation_required', action: r.action, ticket: '' };
      }
      const out = await action.run(r.args ?? null, ctx);
      return out === undefined ? null : out;
    },
  };

  async function answer(request) {
    if (!request || typeof request !== 'object' || !HOST_OPS.includes(request.op))
      return { error: { kind: 'failed', message: `not a host request: ${json(request?.op ?? request)}` } };
    const op = request.op;
    let effect = 'read_only';
    if (op === 'run') {
      const action = host.actions?.[request.action];
      if (!action) return { error: { kind: 'not_found', what: `action ${request.action}` } };
      effect = action.effect ?? 'mutating';
    }
    const refused = check(manifest, may, request, effect);
    if (refused) {
      log('warn', 'host', `${op}: refused (${refused[0]}): ${refused[1]}`);
      return { error: { kind: 'refused', capability: refused[0], why: refused[1] } };
    }
    const handler = handlers[op] ?? builtIn[op];
    if (!handler) {
      const error = { kind: 'unsupported', what: `${op} is not available in this host` };
      log('warn', 'host', `${op}: ${describe(error)}`);
      return { error };
    }
    try {
      const value = await handler(request, ctx);
      return { ok: value === undefined ? null : value };
    } catch (e) {
      const error = asError(e);
      log(error.kind === 'refused' ? 'warn' : 'error', 'host', `${op}: ${describe(error)}`);
      return { error };
    }
  }

  return async (text) => {
    let request;
    try {
      request = JSON.parse(text);
    } catch (e) {
      return json({ error: { kind: 'failed', message: `not a host request: ${e.message}` } });
    }
    try {
      return json(await answer(request));
    } catch (e) {
      return json({ error: asError(e) });
    }
  };
}

// ---- memoryHost --------------------------------------------------------------

const nameOf = (path) => path.slice(path.lastIndexOf('/') + 1);
const info = (path) => {
  const cut = path.indexOf('/');
  return cut < 0 ? { path, name: nameOf(path) } : { path, name: nameOf(path), space: path.slice(0, cut) };
};

function pathOk(path) {
  const p = String(path ?? '');
  if (!p || p.startsWith('/') || p.split('/').some((s) => s === '' || s === '.' || s === '..'))
    throw { kind: 'failed', message: `${JSON.stringify(p)} is not a document path` };
  return p;
}

/// One edit of a document's text (`documents::DocEdit`), by characters.
function applyEdit(text, edit) {
  const chars = [...text];
  if ('insert' in edit) {
    const at = Math.min(edit.insert.at, chars.length);
    return [...chars.slice(0, at), edit.insert.text, ...chars.slice(at)].join('');
  }
  if ('delete' in edit) {
    const { at, len } = edit.delete;
    return [...chars.slice(0, at), ...chars.slice(at + len)].join('');
  }
  if ('append' in edit) return text + edit.append.text;
  if ('replace' in edit) return edit.replace.text;
  throw { kind: 'failed', message: `not a document edit: ${JSON.stringify(edit)}` };
}

/// A host in memory, enough for a demo: documents and files from what the
/// embedder hands it, in `Map`s; the organization and selection it is
/// given; app actions it is given; notifications collected; no network,
/// no clipboard, nothing persisted. The extension still reaches only what
/// its grant lets it.
export function memoryHost(options = {}) {
  const documents = new Map(Object.entries(options.documents ?? {}));
  const files = new Map(Object.entries(options.files ?? {}));
  const notifications = [];
  const doc = (path) => {
    if (!documents.has(path)) throw { kind: 'not_found', what: `document ${path}` };
    return documents.get(path);
  };
  const handle = {
    org: () => options.org ?? null,
    selection: () => options.selection ?? null,
    open: (r) => ({ path: r.path, text: doc(r.path) }),
    read_document: (r) => doc(r.path),
    list_documents: (r) => {
      const under = r.under ? `${String(r.under).replace(/\/+$/, '')}/` : '';
      return [...documents.keys()].filter((p) => p.startsWith(under)).sort().map(info);
    },
    create_document: (r) => {
      const path = pathOk(r.path);
      if (documents.has(path)) throw { kind: 'failed', message: `${path} exists already` };
      documents.set(path, String(r.text ?? ''));
      return info(path);
    },
    edit_document: (r) => {
      const text = (r.edits ?? []).reduce(applyEdit, doc(r.path));
      documents.set(r.path, text);
      return text;
    },
    rename_document: (r) => {
      const text = doc(r.path);
      const to = pathOk(r.to);
      if (documents.has(to)) throw { kind: 'failed', message: `${to} exists already` };
      documents.delete(r.path);
      documents.set(to, text);
      return info(to);
    },
    move_document: (r) => {
      const to = pathOk(`${String(r.folder ?? '').replace(/\/+$/, '')}/${nameOf(r.path)}`.replace(/^\//, ''));
      return handle.rename_document({ path: r.path, to });
    },
    delete_document: (r) => {
      doc(r.path);
      documents.delete(r.path);
      return null;
    },
    read_file: (r) => {
      if (!files.has(r.path)) throw { kind: 'not_found', what: `file ${r.path}` };
      return files.get(r.path);
    },
    write_file: (r) => {
      if (!files.has(r.path))
        throw { kind: 'refused', capability: 'folders', why: `${r.path} is not a file this page handed it` };
      files.set(r.path, String(r.text ?? ''));
      return null;
    },
    notify: (r) => {
      notifications.push({ text: r.text, kind: r.kind ?? null, title: r.title ?? null });
      options.notify?.(r);
      return null;
    },
    ...options.handle,
  };
  return {
    handle,
    actions: options.actions ?? {},
    confirm: options.confirm,
    log: options.log,
    theme: options.theme,
    documents,
    files,
    notifications,
  };
}
