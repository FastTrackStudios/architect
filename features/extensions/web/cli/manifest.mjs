// An extension's manifest as build-web reads it: checked as the native
// `Manifest::check` checks what this tool uses (features/extensions/src/manifest.rs),
// and refused when it is not, so nothing a manifest says can become code.

/// `id` as its actions' namespace (`lab.wasi-counter` → `LAB_WASI_COUNTER_`;
/// `manifest::action_prefix`, and the runtime's check.js `actionPrefix`:
/// test/check.test.mjs keeps them equal).
export function actionPrefix(id) {
  let out = '';
  for (const c of id) {
    if (/[A-Za-z0-9]/.test(c)) out += c.toUpperCase();
    else if (!out.endsWith('_')) out += '_';
  }
  if (!out.endsWith('_')) out += '_';
  return out;
}

const ID = /^[a-z0-9._-]+$/;
const ACTION = /^[A-Z][A-Z0-9_]*$/;
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const API = /^\d+\.\d+$/;
const EFFECTS = ['read_only', 'mutating', 'destructive'];

const bad = (why) => {
  throw new Error(`the manifest is refused: ${why}`);
};
// eslint-disable-next-line no-control-regex
const plain = (s) => typeof s === 'string' && s.length <= 1000 && !/[\u0000-\u001f\u007f\u2028\u2029]/.test(s);

/// `m`, when it is a manifest build-web may use: its id, version and api
/// in their grammars (`Manifest::check`; semver), each action's id in
/// its namespace (or `reserved`) and upper case, its effect one of three,
/// its `params` a JSON object; names and descriptions plain text. Throws
/// why it is not.
export function checkManifest(m) {
  if (!m || typeof m !== 'object') bad('not a table');
  if (typeof m.id !== 'string' || !ID.test(m.id)) bad(`id ${JSON.stringify(m.id)}: lower case letters, digits, . - _`);
  if (typeof m.version !== 'string' || !SEMVER.test(m.version)) bad(`version ${JSON.stringify(m.version)}: not a semantic version`);
  if (m.api !== undefined && (typeof m.api !== 'string' || !API.test(m.api))) bad(`api ${JSON.stringify(m.api)}: <major>.<minor>`);
  if (!plain(m.name) || !m.name) bad('name: plain text, one line');
  if (m.description !== undefined && typeof m.description !== 'string') bad('description: text');
  const prefix = actionPrefix(m.id);
  const reserved = Array.isArray(m.reserved) ? m.reserved : [];
  if (m.actions !== undefined && !Array.isArray(m.actions)) bad('actions: a list');
  const seen = new Set();
  for (const a of m.actions ?? []) {
    if (!a || typeof a !== 'object') bad('an action that is not a table');
    if (typeof a.id !== 'string' || !ACTION.test(a.id)) bad(`action ${JSON.stringify(a.id)}: A-Z, 0-9 and _`);
    if (!a.id.startsWith(prefix) && !reserved.includes(a.id)) bad(`action ${a.id}: not in its namespace ${prefix}`);
    if (seen.has(a.id)) bad(`action ${a.id}: declared twice`);
    seen.add(a.id);
    if (!plain(a.name) || !a.name) bad(`action ${a.id}: its name is plain text, one line`);
    if (a.effect !== undefined && !EFFECTS.includes(a.effect)) bad(`action ${a.id}: effect ${JSON.stringify(a.effect)}`);
    if (a.params !== undefined) {
      let schema;
      try {
        schema = JSON.parse(a.params);
      } catch (e) {
        bad(`action ${a.id}: params is not JSON (${e.message})`);
      }
      if (!schema || typeof schema !== 'object' || Array.isArray(schema)) bad(`action ${a.id}: params is not a JSON schema object`);
    }
  }
  return m;
}
