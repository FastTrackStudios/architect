// The host an embedder supplies, and the answerer between it and a
// component: every request the component makes of the app arrives as a
// host-service request (`hostcall::HostRequest` JSON), is checked against
// what the extension may reach (check.js, the native `hostcall::check`),
// and only then reaches the embedder's handler for its `op`. An `op` with
// no handler answers "not available in this host": the component gets an
// error it can show, never a throw and never a hang.
import { check, HOST_OPS, inFolders } from './check.js';

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
  const frozen = (o) => {
    for (const v of Object.values(o)) if (v && typeof v === 'object') frozen(v);
    return Object.freeze(o);
  };
  const ctx = Object.freeze({ extension: manifest.id, manifest, capabilities: frozen(structuredClone(may)) });
  const handlers = host.handle ?? {};
  const own = (o, k) => o != null && Object.hasOwn(o, k);

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
      const out = await host.actions[r.action].run(r.args ?? null, ctx);
      return out === undefined ? null : out;
    },
  };

  async function answer(request) {
    if (!request || typeof request !== 'object' || !HOST_OPS.includes(request.op))
      return { error: { kind: 'failed', message: `not a host request: ${json(request?.op ?? request)}` } };
    const op = request.op;
    let effect = 'read_only';
    if (op === 'run') {
      // Only an action the embedder offers, and never the extension's own
      // whatever its name: an extension `vault` does not own the page's
      // VAULT_EMPTY_TRASH.
      if (!own(host.actions, request.action) && !own(handlers, 'run'))
        return { error: { kind: 'not_found', what: `action ${request.action}` } };
      effect = own(host.actions, request.action) ? (host.actions[request.action].effect ?? 'mutating') : 'mutating';
    }
    const refused = check(manifest, may, request, effect, true);
    if (refused) {
      log('warn', 'host', `${op}: refused (${refused[0]}): ${refused[1]}`);
      return { error: { kind: 'refused', capability: refused[0], why: refused[1] } };
    }
    // A destructive app action needs a yes, whoever answers it.
    if (op === 'run' && effect === 'destructive') {
      const yes = host.confirm ? await host.confirm({ id: request.action, effect, by: manifest.id }, request.args ?? null) : false;
      if (yes !== true) {
        const error = { kind: 'confirmation_required', action: request.action, ticket: '' };
        log('warn', 'host', `run: ${describe(error)}`);
        return { error };
      }
    }
    const handler = own(handlers, op) ? handlers[op] : own(builtIn, op) ? builtIn[op] : null;
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

function folderOk(path, ctx) {
  if (!inFolders(path, ctx.capabilities.folders))
    throw { kind: 'refused', capability: 'folders', why: `${path} is not in a folder this extension may reach (capabilities.folders)` };
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
    // Files: only under a folder it was granted (`capabilities.folders`),
    // and only those the page handed it.
    read_file: (r, ctx) => {
      folderOk(r.path, ctx);
      if (!files.has(r.path)) throw { kind: 'not_found', what: `file ${r.path}` };
      return files.get(r.path);
    },
    write_file: (r, ctx) => {
      folderOk(r.path, ctx);
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

// ---- fetchHandler ------------------------------------------------------------

const b64 = (bytes) => {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
};

/// Handlers for `fetch`, `read_body` and `close_body` that make an
/// extension's requests the safe way: never with the page's credentials
/// (`credentials: "omit"`), never to the page's own origin, only to
/// `allow` (host names, `*.domain`, `host:port`; on top of the
/// extension's `capabilities.hosts`, which the host checked already), no
/// redirect followed (where it goes is hidden from a page, so it cannot be
/// checked), a time and a size limit. Spread them into a host's `handle`.
export function fetchHandler({ allow = [], maxBytes = 16 << 20, timeoutMs = 30000, fetch: doFetch = globalThis.fetch } = {}) {
  const streams = new Map();
  let next = 0;
  const allowed = (url) => {
    const host = url.hostname.toLowerCase();
    const port = url.port || (url.protocol === 'https:' ? '443' : '80');
    return allow.some((a) => {
      a = a.toLowerCase();
      return a === host || a === `${host}:${port}` || (a.startsWith('*.') && (host === a.slice(2) || host.endsWith(a.slice(1))));
    });
  };
  return {
    fetch: async (r) => {
      let url;
      try {
        url = new URL(r.url);
      } catch {
        throw { kind: 'failed', message: `${r.url} is not a URL` };
      }
      if (url.protocol !== 'https:' && url.protocol !== 'http:') throw { kind: 'refused', capability: 'hosts', why: `${r.url}: only http and https` };
      if (globalThis.location && url.origin === globalThis.location.origin)
        throw { kind: 'refused', capability: 'hosts', why: `${url.host} is this page's own origin: an extension never reaches it` };
      if (!allowed(url)) throw { kind: 'refused', capability: 'hosts', why: `${url.host} is not a host this page lets extensions reach` };
      const signal = AbortSignal.timeout(Math.min(Number(r.timeout_ms ?? timeoutMs), 300000));
      let res;
      try {
        res = await doFetch(url.href, {
          method: String(r.method ?? 'GET').toUpperCase(),
          headers: r.headers ?? {},
          body: r.body ?? undefined,
          credentials: 'omit',
          redirect: 'manual',
          referrerPolicy: 'no-referrer',
          mode: 'cors',
          cache: 'no-store',
          signal,
        });
      } catch (e) {
        throw { kind: 'unsupported', what: `the browser refused to fetch ${url.href} (CORS, or the network): ${e?.message ?? e}` };
      }
      if (res.type === 'opaqueredirect' || (res.status >= 300 && res.status < 400))
        throw { kind: 'unsupported', what: `${url.href} redirects: it is not followed` };
      const cap = Math.min(Number(r.max_bytes ?? maxBytes), maxBytes);
      const bytes = new Uint8Array(await res.arrayBuffer());
      if (bytes.length > cap) throw { kind: 'failed', message: `the response is past the most it takes (${cap} bytes)` };
      const headers = {};
      res.headers.forEach((v, k) => (headers[k.toLowerCase()] = v));
      const out = { status: res.status, url: url.href, headers };
      if (r.stream) {
        const id = `s${++next}`;
        streams.set(id, { bytes, at: 0, binary: !!r.binary, decoder: new TextDecoder() });
        out.stream = id;
      } else out.body = r.binary ? b64(bytes) : new TextDecoder().decode(bytes);
      return out;
    },
    read_body: (r) => {
      const s = streams.get(r.stream);
      if (!s) throw { kind: 'not_found', what: `stream ${r.stream}` };
      const end = Math.min(s.bytes.length, s.at + Math.max(1, Number(r.max ?? 65536)));
      const part = s.bytes.subarray(s.at, end);
      s.at = end;
      const done = s.at >= s.bytes.length;
      if (done) streams.delete(r.stream);
      return { chunk: s.binary ? b64(part) : s.decoder.decode(part, { stream: !done }), done };
    },
    close_body: (r) => streams.delete(r.stream),
  };
}
