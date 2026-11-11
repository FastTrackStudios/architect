// Which `architect:extension` world this package runs, and the check that
// refuses a component built for a newer one
// (features/extensions/wit/extension.wit, its `package` line; the
// manifest's `api`, features/extensions/src/manifest.rs `API`).
//
// The world's version is in the component itself (each interface it
// imports and exports is `architect:extension/<name>@<version>`):
// `architect build-web` reads it from the transpiled module and writes it
// into `architect.json`, and `loadExtension` checks it before it starts
// anything. Native hosts do not check it yet (#507).

/// The world's version this package's host implements (test/world.test.mjs
/// keeps it equal to extension.wit's).
export const WORLD = '0.1.0';

/// The manifest `api` this package's host implements.
export const API = '0.1';

const parts = (v) => String(v).split('.').map((n) => Number.parseInt(n, 10) || 0);

/// Whether version `a` is newer than `b` (dotted numbers).
export function newer(a, b) {
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  }
  return false;
}

/// The world versions a transpiled module names (jco keeps them in its
/// import and export names).
export function worldsIn(moduleText) {
  const found = new Set();
  for (const m of moduleText.matchAll(/architect:extension\/[a-z][a-z0-9-]*@(\d+\.\d+\.\d+)/g)) found.add(m[1]);
  return [...found];
}

/// Why a component built for world `world` (manifest `api`) cannot run
/// here, or `null`.
export function refusal(world, api, id = 'the extension') {
  if (!world)
    return `${id} does not say which architect:extension world it was built for: build it with \`architect build-web\``;
  if (newer(world, WORLD))
    return `${id} was built for architect:extension@${world}, newer than this host's ${WORLD}: update @fasttrackstudio/architect`;
  if (api && newer(api, API))
    return `${id} was written for extension API ${api}, newer than this host's ${API}: update @fasttrackstudio/architect`;
  return null;
}
