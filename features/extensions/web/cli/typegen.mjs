// Typed actions: an extension's manifest (`[[actions]]`, each `params` a
// JSON schema) as a `.d.ts` and a tiny wrapper, so `ext.run("X_Y", args)`
// and `actions.y(args)` are checked at compile time.
//
// The JSON schema subset it reads (what manifests use); anything else is
// `unknown`, never an error:
// - `type`: string, number, integer, boolean, null, array, object, or a
//   list of them (a union);
// - `enum` and `const` (literal types);
// - `properties` with `required` (the rest optional), `additionalProperties`
//   (absent or false: no other keys, so a misspelt one fails to compile;
//   true or a schema: an index signature), `items` (an array of it);
// - `anyOf` / `oneOf` (a union), `allOf` (an intersection);
// No manifest text goes into a comment: ids, keys and literals are
// written with JSON.stringify, method names only from identifiers that
// match a strict pattern, and the manifest is checked first
// (manifest.mjs `checkManifest`).
// Not read: `$ref`, `patternProperties`, tuples (`prefixItems`), `not`,
// formats and numeric or length bounds. An action without `params` takes
// no arguments. Answers are `unknown`: a manifest has no result schema.
import { actionPrefix, checkManifest } from './manifest.mjs';

const IDENT = /^[A-Za-z_$][A-Za-z0-9_$]*$/;
/// A method name: lower camel case, nothing else.
const METHOD = /^[a-z][A-Za-z0-9]{0,63}$/;
const key = (k) => (IDENT.test(k) ? k : JSON.stringify(k));
const lit = (v) => JSON.stringify(v);

/// How much of a schema it reads: past these, the manifest is refused
/// (a `type` list nested in itself grows as a power of its depth).
export const SCHEMA_LIMITS = { depth: 12, nodes: 2000, length: 64 << 10 };

const tooBig = (why) => {
  throw new Error(`the manifest is refused: a params schema ${why}`);
};

/// A JSON schema as a TypeScript type, `depth` its indent.
export function schemaType(s, depth = 0, budget = { nodes: 0 }) {
  if (depth > SCHEMA_LIMITS.depth) tooBig(`nested past ${SCHEMA_LIMITS.depth} levels`);
  if (++budget.nodes > SCHEMA_LIMITS.nodes) tooBig(`of more than ${SCHEMA_LIMITS.nodes} types`);
  const out = typeOf(s, depth, budget);
  if (out.length > SCHEMA_LIMITS.length) tooBig(`whose type is past ${SCHEMA_LIMITS.length} characters`);
  return out;
}

function typeOf(s, depth, budget) {
  if (s === true || s == null) return 'unknown';
  if (s === false) return 'never';
  if (typeof s !== 'object') return 'unknown';
  if ('const' in s) return lit(s.const);
  if (Array.isArray(s.enum)) return s.enum.map(lit).join(' | ') || 'never';
  if (Array.isArray(s.anyOf ?? s.oneOf)) return (s.anyOf ?? s.oneOf).map((x) => wrap(schemaType(x, depth + 1, budget))).join(' | ');
  if (Array.isArray(s.allOf)) return s.allOf.map((x) => wrap(schemaType(x, depth + 1, budget))).join(' & ');
  if (Array.isArray(s.type)) return s.type.map((t) => wrap(schemaType({ ...s, type: t }, depth + 1, budget))).join(' | ');
  switch (s.type) {
    case 'string':
      return 'string';
    case 'number':
    case 'integer':
      return 'number';
    case 'boolean':
      return 'boolean';
    case 'null':
      return 'null';
    case 'array':
      return `Array<${schemaType(s.items, depth + 1, budget)}>`;
    case 'object':
      return objectType(s, depth, budget);
    default:
      return s.properties ? objectType(s, depth, budget) : 'unknown';
  }
}

const wrap = (t) => (/[|&]/.test(t) && !t.startsWith('{') ? `(${t})` : t);

function objectType(s, depth, budget) {
  const props = s.properties ?? {};
  const required = new Set(s.required ?? []);
  const pad = '  '.repeat(depth + 1);
  const lines = Object.entries(props).map(
    ([k, v]) => `${pad}${key(k)}${required.has(k) ? '' : '?'}: ${schemaType(v, depth + 1, budget)};`,
  );
  const extra = s.additionalProperties;
  if (extra === true || (extra && typeof extra === 'object'))
    lines.push(`${pad}[key: string]: ${extra === true ? 'unknown' : schemaType(extra, depth + 1, budget)};`);
  if (!lines.length) return extra === false ? 'Record<string, never>' : 'Record<string, unknown>';
  return `{\n${lines.join('\n')}\n${'  '.repeat(depth)}}`;
}

/// Whether an action's arguments may be left out: no schema, or an
/// object with nothing required.
const optional = (s) => !s || (s.type === 'object' && !(s.required ?? []).length);

/// Each action's method name: its id past the extension's namespace, in
/// lower camel case (`LAB_WASI_COUNTER_ADD` → `add`).
export function methodNames(manifest) {
  const prefix = actionPrefix(manifest.id);
  const used = new Set();
  return (manifest.actions ?? []).map((a) => {
    const rest = a.id.startsWith(prefix) ? a.id.slice(prefix.length) : a.id;
    let name = rest
      .toLowerCase()
      .split('_')
      .filter(Boolean)
      .map((w, i) => (i ? w[0].toUpperCase() + w.slice(1) : w))
      .join('');
    if (!METHOD.test(name)) name = 'action';
    let n = name;
    for (let i = 2; used.has(n); i++) n = `${name}${i}`;
    used.add(n);
    return n;
  });
}

/// `manifest`'s typed actions: `{ dts, js }`, importing the package as
/// `from` (`@fasttrackstudio/architect`).
export function typegen(manifest, from = '@fasttrackstudio/architect', integrity = null) {
  checkManifest(manifest);
  const names = methodNames(manifest);
  const actions = manifest.actions ?? [];
  const head = '// Generated by `architect-ext build-web` from an extension manifest: do not edit.\n';
  const entries = actions.map((a) => {
    const schema = a.params ? JSON.parse(a.params) : null;
    const args = schema ? schemaType(schema, 2) : 'undefined';
    return { a, args: schema && optional(schema) ? `${args} | undefined` : args, none: !schema, opt: optional(schema) };
  });
  const map = entries
    .map(({ a, args }) => `  ${lit(a.id)}: {\n    args: ${args};\n    result: unknown;\n    effect: ${lit(a.effect ?? 'read_only')};\n  };`)
    .join('\n');
  const methods = entries
    .map(({ a, none, opt }, i) => {
      const sig = none ? '' : `args${opt ? '?' : ''}: Actions[${lit(a.id)}]['args']`;
      return `  ${names[i]}(${sig}): Promise<unknown>;`;
    })
    .join('\n');
  const dts = `${head}import type { Extension } from ${lit(from)};

/** Its actions, by id: what each takes. (A type, not an interface, so it is an \`ActionMap\`.) */
export type Actions = {
${map}
};

/** Its id. */
export declare const id: ${lit(manifest.id)};

/** Its version. */
export declare const version: ${lit(manifest.version)};

/** Its lock's digest, to pin its files with: \`loadExtension(url, { integrity })\`. */
export declare const integrity: ${integrity ? lit(integrity) : 'undefined'};

/** Each action as a method. */
export interface Bound {
${methods}
}

/** \`ext\`'s actions as methods. */
export declare function bind(ext: Extension<Actions>): Bound;
`;
  const js = `${head}export const id = ${lit(manifest.id)};

export const version = ${lit(manifest.version)};

export const integrity = ${integrity ? lit(integrity) : 'undefined'};

export function bind(ext) {
  return {
${actions.map((a, i) => `    ${names[i]}: (args) => ext.run(${lit(a.id)}, args),`).join('\n')}
  };
}
`;
  return { dts, js };
}
