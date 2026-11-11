// The WASI Counter extension in a plain page, through the package's public
// API only: load it, run its actions with their generated types, mount
// its views. `npm run extension` writes public/counter and src/counter.
import { loadExtension, memoryHost, mountView, type Extension } from '@fasttrackstudio/architect';
import { bind, integrity, type Actions } from './counter/actions.js';

import { isSupported } from '@fasttrackstudio/architect';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function main(): Promise<Extension<Actions> | null> {
  if (!isSupported()) {
    $('status').textContent = 'This browser cannot run Architect extensions yet (it lacks WebAssembly JS Promise Integration: Safari, for now).';
    document.body.dataset.ready = 'unsupported';
    return null;
  }
  const host = memoryHost({
    // An app action the page offers extensions: they run it only if granted.
    actions: { THEME_LIST: { effect: 'read_only', run: () => ['light', 'dark', 'sepia'] } },
    log: (line) => console.debug(`[${line.source}] ${line.text}`),
  });
  const ext = await loadExtension<Actions>('/counter/', {
    // Its files pinned: the lock build-web wrote, by its digest.
    integrity,
    host,
    // Its manifest asks for THEME_LIST; the page grants it. Nothing else.
    grant: { actions: ['THEME_LIST'] },
  });
  const counter = bind(ext);

  $('add').onclick = async () => {
    $('answer').textContent = String(await ext.run('LAB_WASI_COUNTER_ADD', { by: 5 }));
  };
  $('ask').onclick = async () => {
    $('answer').textContent = JSON.stringify(await counter.ask());
  };

  await mountView(ext, 'counter', $('counter')).rendered;
  const bars = mountView(ext, 'bars', $('bars'));
  await bars.rendered;
  await bars.input({ claim: 'fence:bars', source: '| C | G | Am | F |' });

  $('status').textContent = `Loaded ${ext.manifest.name} ${ext.manifest.version} (architect:extension@${ext.world}): ${ext.actions.length} actions, ${ext.views.length} views.`;
  document.body.dataset.ready = 'true';
  return ext;
}

main().catch((e: unknown) => {
  $('status').textContent = `It did not load: ${e instanceof Error ? e.message : String(e)}`;
  document.body.dataset.ready = 'failed';
});
