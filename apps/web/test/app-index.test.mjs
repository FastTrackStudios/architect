// The vault's query service in the browser (#495, #710's data model), in
// a real browser: the release bundle in `/app/?demo&probe`.
//
// - An extension's `query_vault` (the host service, through the demo's
//   probe extension, which may read documents) runs a base over the vault
//   index with the one engine (`vault_query::QueryEngine`), served in the
//   page (`architect::serve_local`): in the demo the index is built here
//   from the demo vault with the one builder. A note an extension makes is
//   in the next answer, the index following the vault.
// - With INDEX_SNAPSHOT=<file> (a 10 000-note index, `cargo test -p
//   architect-web --release --test index_snapshot -- --ignored` with
//   INDEX_SNAPSHOT_OUT), its first load in this browser: fetching it,
//   reading it into a replica and the first two queries, timed in the page
//   (`window.architectProbeIndex`).
//
// `just app-index-test` builds and runs it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { copyFileSync, rmSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { front, firefox, until, skipWhy, BUNDLE } from './lib/app-bidi.mjs';

const skip = skipWhy('app-index-test');
const SNAPSHOT = process.env.INDEX_SNAPSHOT;

/// One host-service call as the probe; its answer, or `{error}` (as text
/// across BiDi).
const call = async (b, request) => JSON.parse(await b.eval(`architectProbe('web.probe', ${JSON.stringify(JSON.stringify(request))}).then((t) => t, (e) => JSON.stringify({ error: String(e) }))`));

const PROJECTS = 'filters:\n  and:\n    - file.inFolder("Projects")\nviews:\n  - type: table\n    name: All\n';
/// Every row's path in an answer's groups.
const paths = (answer) => (answer.rows?.groups || []).flatMap((g) => (g.rows || []).map((r) => r.path));

async function open(b, origin) {
  await b.go(`${origin}/app/?demo&probe`);
  await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 90000, 'the demo');
  await until(() => b.eval(`typeof window.architectProbe === 'function'`), 20000, 'the probe');
}

test("an extension's query_vault runs over the index in the page, and follows the vault", { skip }, async () => {
  const f = await front();
  const b = await firefox(false);
  try {
    await open(b, f.origin);
    const first = await until(async () => {
      const a = await call(b, { op: 'query_vault', source: PROJECTS });
      return a && !a.error && a.total > 0 && a;
    }, 20000, 'the first answer').catch(async (e) => {
      throw new Error(`${e.message}: ${JSON.stringify(await call(b, { op: 'query_vault', source: PROJECTS })).slice(0, 400)}`);
    });
    const listed = paths(first);
    assert.ok(listed.length > 0 && listed.every((p) => p.startsWith('Projects/')), JSON.stringify(listed).slice(0, 300));
    // A note an extension makes: in the next answer.
    const made = await call(b, { op: 'create_document', path: 'Projects/Made by the probe.md', text: '---\nstatus: active\n---\n# Made by the probe\n' });
    assert.equal(made?.error, undefined, JSON.stringify(made));
    await until(async () => {
      const a = await call(b, { op: 'query_vault', source: PROJECTS });
      return a.total === first.total + 1 && paths(a).includes('Projects/Made by the probe.md');
    }, 10000, 'the made note in the next answer');
    // Asked again with its hash: unchanged, nothing run.
    const now = await call(b, { op: 'query_vault', source: PROJECTS });
    const again = await call(b, { op: 'query_vault', source: PROJECTS, if_changed: now.hash });
    assert.equal(again.unchanged, true, JSON.stringify(again).slice(0, 200));
  } finally { await b.close(); f.close(); }
});

test("a 10 000-note index's first load in the browser", { skip: skip || (SNAPSHOT ? false : 'needs INDEX_SNAPSHOT (just app-index-test)') }, async () => {
  const served = join(BUNDLE, 'app', 'index-10k.json');
  copyFileSync(SNAPSHOT, served);
  const f = await front();
  const b = await firefox(false);
  try {
    await open(b, f.origin);
    const base = 'filters:\n  and:\n    - status != "done"\nviews:\n  - type: table\n    name: Open\n    groupBy:\n      property: status\n      direction: ASC\n';
    const runs = [];
    for (let i = 0; i < 3; i++) {
      runs.push(JSON.parse(await b.eval(`architectProbeIndex('/app/index-10k.json', ${JSON.stringify(base)})`)));
    }
    console.log(`10k index in Firefox: ${JSON.stringify(runs)}`);
    assert.equal(runs[0].total, 7500, 'three of every four notes are not done');
  } finally { await b.close(); f.close(); if (existsSync(served)) rmSync(served); }
});
