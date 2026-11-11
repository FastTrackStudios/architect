// The decision every host makes, in JS: whether an extension may make a
// host-service request, from its manifest's `[capabilities]` within what
// the embedder granted. A port of features/extensions/src/hostcall.rs
// `check` and features/extensions/src/capabilities.rs `Capabilities`
// (`within`, `may_run`, `may_read`, …), with the same capability names and
// the same words, so a refusal reads the same in any host. The cases are
// in test/check.test.mjs; a change to either side changes both.

/// How much an action may change, least first (`manifest::Reach`).
export const REACH = ['read_only', 'mutating', 'destructive'];

const rank = (r) => {
  const i = REACH.indexOf(r ?? 'read_only');
  return i < 0 ? 0 : i;
};

const reachName = (r) => REACH[rank(r)];

/// What an extension may reach when nothing is granted: nothing past its
/// own actions, data and log.
export function noCapabilities() {
  return {
    actions: [],
    effect: 'read_only',
    hosts: [],
    folders: [],
    reads: [],
    writes: [],
    ui: [],
    events: [],
    ports: [],
    languages: [],
  };
}

/// `c` with every field (a manifest's `[capabilities]` may leave any out).
export function capabilities(c = {}) {
  const out = noCapabilities();
  for (const k of Object.keys(out)) if (c[k] !== undefined) out[k] = k === 'effect' ? reachName(c[k]) : [...c[k]];
  return out;
}

/// `manifest.id` as its actions' namespace (`lab.wasi-counter` →
/// `LAB_WASI_COUNTER_`; `manifest::action_prefix`).
export function actionPrefix(id) {
  let out = '';
  for (const c of id) {
    if (/[A-Za-z0-9]/.test(c)) out += c.toUpperCase();
    else if (!out.endsWith('_')) out += '_';
  }
  if (!out.endsWith('_')) out += '_';
  return out;
}

const prefixIn = (p, of) => of.some((g) => g === '*' || p.startsWith(g));

const hostIn = (h, of) =>
  of.some((g) => g === '*' || g === h || (g.startsWith('*.') && (h.endsWith(`.${g.slice(2)}`) || h === g.slice(2))));

const folderIn = (f, of) => {
  f = f.replace(/\/+$/, '');
  return of.some((g) => {
    g = g.replace(/\/+$/, '');
    return f === g || f.startsWith(`${g}/`);
  });
};

/// What it may do when `granted` is what the embedder said yes to: each
/// part the narrower of the two (`Capabilities::within`).
export function within(asked, granted) {
  asked = capabilities(asked);
  granted = capabilities(granted);
  const actions = asked.actions.filter((p) => prefixIn(p, granted.actions));
  for (const g of granted.actions)
    if (g !== '*' && prefixIn(g, asked.actions) && !prefixIn(g, actions)) actions.push(g);
  const hosts = asked.hosts.filter((h) => hostIn(h, granted.hosts));
  for (const g of granted.hosts) if (g !== '*' && hostIn(g, asked.hosts) && !hostIn(g, hosts)) hosts.push(g);
  const folders = asked.folders.filter((f) => folderIn(f, granted.folders));
  for (const g of granted.folders) if (folderIn(g, asked.folders) && !folderIn(g, folders)) folders.push(g);
  const both = (k) => asked[k].filter((x) => granted[k].includes(x));
  return {
    actions,
    effect: REACH[Math.min(rank(asked.effect), rank(granted.effect))],
    hosts,
    folders,
    reads: both('reads'),
    writes: both('writes'),
    ui: both('ui'),
    events: both('events'),
    ports: both('ports'),
    languages: both('languages'),
  };
}

/// Whether it may run action `id` of `effect` past its own namespace
/// `own`: `null`, or `[capability, why]` (`Capabilities::may_run`). `own`
/// null: the action is not its own whatever its name (an app action the
/// embedder offers).
export function mayRun(may, own, id, effect) {
  if (own && id.startsWith(own)) return null;
  if (!prefixIn(id, may.actions)) return ['actions', `${id} is not in a namespace this extension may run`];
  if (rank(effect) > rank(may.effect))
    return ['effect', `${id} is ${reachName(effect)}: this extension may run ${reachName(may.effect)} actions at most`];
  return null;
}

function reachesHost(of, host, port) {
  host = host.replace(/\.+$/, '').toLowerCase();
  if (!host) return false;
  const lower = of.map((h) => h.toLowerCase());
  return hostIn(host, lower) || (port != null && lower.some((g) => g === `${host}:${port}`));
}

const SECRET_OPEN = '{{secret:';

/// The secrets a value names (`{{secret:NAME}}`).
export function secretRefs(value) {
  const out = [];
  let rest = String(value ?? '');
  for (;;) {
    const at = rest.indexOf(SECRET_OPEN);
    if (at < 0) break;
    const after = rest.slice(at + SECRET_OPEN.length);
    const end = after.indexOf('}}');
    if (end < 0) break;
    out.push(after.slice(0, end).trim());
    rest = after.slice(end + 2);
  }
  return out;
}

const DEFAULT_PORTS = { 'http:': 80, 'https:': 443 };

/// Whether `path` is `folders`' or under one (`capabilities::folder_in`).
export function inFolders(path, folders) {
  return folderIn(String(path), folders);
}

function mayReachUrl(may, url) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch (e) {
    return [null, ['hosts', `${url} is not a URL: ${e.message}`]];
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:')
    return [null, ['hosts', `${url}: only http and https are fetched`]];
  // An IPv6 host keeps its brackets, as `url::Url::host_str` does.
  const host = parsed.hostname;
  const port = parsed.port ? Number(parsed.port) : DEFAULT_PORTS[parsed.protocol];
  if (reachesHost(may.hosts, host, port)) return [parsed, null];
  return [null, ['hosts', `this extension may not reach ${host} (capabilities.hosts)`]];
}

function fetchCheck(manifest, may, fetch) {
  const [url, refused] = mayReachUrl(may, fetch.url);
  if (refused) return refused;
  if (secretRefs(fetch.url).length || secretRefs(fetch.body).length)
    return ['secrets', 'a secret goes in a header\'s value, not the URL or the body'];
  for (const name of Object.values(fetch.headers ?? {}).flatMap(secretRefs)) {
    const secret = (manifest.secrets ?? []).find((s) => s.id === name);
    if (!secret) return ['secrets', `${name} is not one of its secrets ([[secrets]])`];
    const host = url.hostname;
    const port = url.port ? Number(url.port) : DEFAULT_PORTS[url.protocol];
    // One of its hosts, or of `capabilities.hosts` when it names none
    // (`Manifest::secret_reaches`).
    const asked = manifest.capabilities?.hosts ?? [];
    const hosts = secret.hosts?.length ? secret.hosts : asked;
    if (!reachesHost(hosts, host, port) || !reachesHost(asked, host, port))
      return ['secrets', `secret ${name} is not sent to ${host} (its [[secrets]] hosts)`];
  }
  return null;
}

const UI = { notify: 'notify', clipboard_read: 'clipboard', clipboard_write: 'clipboard', pick: 'pickers', prompt: 'prompts', menu: 'menus', decorate: 'badges' };
const DOC_READS = ['open', 'read_document', 'list_documents', 'watch_documents'];
const DOC_WRITES = ['create_document', 'edit_document', 'rename_document', 'move_document', 'delete_document'];

/// Whether event pattern `pattern` covers app event `name`
/// (`events::covers`).
function covers(pattern, name) {
  return (
    pattern === '*' ||
    pattern === name ||
    (pattern.endsWith('.*') && name.startsWith(pattern.slice(0, -2)) && name.slice(pattern.length - 2).startsWith('.'))
  );
}

const DOC_EVENTS = ['doc.created', 'doc.saved', 'doc.renamed', 'doc.deleted'];

/// Whether it may open `kind` port `name`, hearing on `listen`
/// (`capabilities::opens_port`).
function opensPort(of, kind, name, listen) {
  const entries = of.filter((e) => e.startsWith(`${kind}:`)).map((e) => e.slice(kind.length + 1));
  let named = false;
  if (kind === 'midi') named = entries.some((g) => g === '*' || g === name || (g.endsWith('*') && name.startsWith(g.slice(0, -1))));
  else if (kind === 'osc') {
    const lower = name.toLowerCase();
    const cut = lower.lastIndexOf(':');
    const host = cut < 0 ? lower : lower.slice(0, cut);
    named = entries.some((g) => {
      g = g.toLowerCase();
      return !g.startsWith('listen:') && (g === '*' || g === lower || (g.endsWith(':*') && g.slice(0, -2) === host));
    });
  }
  const hears =
    listen == null || (kind === 'osc' && entries.some((g) => g === '*' || g === 'listen:*' || g === `listen:${listen}`));
  return named && hears;
}

/// Whether an extension of `manifest` that may reach `may` may make
/// `request` (a `hostcall::HostRequest`, its `op` in snake case), `effect`
/// the reach of the action a `run` names: `null`, or `[capability, why]`.
/// `appAction`: a `run` names an action of the app's (never its own, even
/// in its namespace).
export function check(manifest, may, request, effect = 'read_only', appAction = false) {
  const reads = (what) =>
    may.reads.includes(what) ? null : [`reads.${what}`, `this extension may not read the app's ${what} (capabilities.reads)`];
  const writes = (what) =>
    may.writes.includes(what) ? null : [`writes.${what}`, `this extension may not change the app's ${what} (capabilities.writes)`];
  const uses = (what) =>
    may.ui.includes(what) ? null : [`ui.${what}`, `this extension may not use the app's ${what} (capabilities.ui)`];
  const op = request.op;
  if (op === 'run') return mayRun(may, appAction ? null : actionPrefix(manifest.id), String(request.action ?? ''), effect);
  if (op === 'org') return reads('org');
  if (op === 'selection') return reads('selection');
  if (DOC_READS.includes(op)) return reads('documents');
  if (DOC_WRITES.includes(op)) return writes('documents');
  if (op === 'editor_state')
    return may.reads.includes('editor') || may.writes.includes('editor') ? null : reads('editor');
  if (op === 'edit_editor' || op === 'decorate_editor') return writes('editor');
  if (op === 'setting' || op === 'set_setting') {
    const key = String(request.key ?? '');
    return key.startsWith(`${manifest.id}.`)
      ? null
      : ['settings', `${key} is not one of its settings: an extension reads and writes only its own (${manifest.id}.…)`];
  }
  if (op in UI) return uses(UI[op]);
  if (op === 'subscribe') {
    const on = String(request.on ?? '');
    if (on.startsWith('app:')) {
      const name = on.slice(4);
      if (!may.events.some((p) => covers(p, name)))
        return [`events.${name}`, `this extension may not hear the app's ${name} events (capabilities.events)`];
      return null;
    }
    return DOC_EVENTS.includes(on) ? reads('documents') : null;
  }
  if (op === 'fetch') return fetchCheck(manifest, may, request);
  if (op === 'secret' || op === 'ask_secret') {
    const name = String(request.name ?? '');
    return (manifest.secrets ?? []).some((s) => s.id === name)
      ? null
      : ['secrets', `${name} is not one of its secrets ([[secrets]])`];
  }
  if (op === 'ports')
    return may.ports.length ? null : ['ports', 'this extension may open no MIDI port or OSC endpoint (capabilities.ports)'];
  if (op === 'open_port') {
    const { kind, name, listen } = request;
    if (kind !== 'midi' && kind !== 'osc') return ['ports', `${JSON.stringify(kind)} is not a kind of port: midi or osc`];
    const ok = opensPort(may.ports, kind, String(name ?? ''), listen);
    if (ok) return null;
    const hearing = listen != null ? `, hearing on ${listen},` : '';
    return [`ports.${kind}:${name}`, `this extension may not open ${kind} port ${name}${hearing} (capabilities.ports)`];
  }
  // Its own data, log, streams, events, ports and surfaces; services,
  // points and windows checked by the host that answers them.
  return null;
}

/// Every `op` a host-service request may have (`hostcall::HostRequest`).
export const HOST_OPS = [
  'run', 'org', 'selection', 'open', 'read_document', 'list_documents', 'create_document', 'edit_document',
  'rename_document', 'move_document', 'delete_document', 'watch_documents', 'editor_state', 'edit_editor',
  'decorate_editor', 'read_file', 'write_file', 'setting', 'set_setting', 'data', 'set_data', 'log', 'service',
  'point', 'window', 'notify', 'clipboard_read', 'clipboard_write', 'pick', 'prompt', 'menu', 'decorate',
  'subscribe', 'unsubscribe', 'events', 'raise', 'fetch', 'read_body', 'close_body', 'secret', 'ask_secret',
  'ports', 'open_port', 'port_send', 'port_receive', 'close_port', 'targets', 'surfaces', 'surface_bank',
];
