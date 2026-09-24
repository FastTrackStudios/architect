# Browser tests for the auth pages

```
npm install
npx playwright install chromium
npm test
```

Playwright starts the server itself: `AUTH_DEV_SEED=1` against an
in-memory database, so every run begins from the same fixed cast and
there is nothing to tear down. The first run builds the server, which
takes a few minutes; after that it is seconds.

## Why these are cheap

The pages are server-rendered with no JavaScript of their own. Every
action is a form POST answered by a redirect, so there is no hydration
to wait for and no "is it ready yet" polling anywhere in the suite —
the next page is complete when it arrives. That is what makes it
reasonable to run this on every push rather than nightly.

## What is asserted, and what is not

These check the things only a browser can: that a form's fields are
reachable by their labels, that a redirect lands where it should, that
a control is *absent* for somebody who may not use it. The rules
themselves — who may remove whom, when a link stops admitting people —
are pinned by the Rust tests, which are faster and do not need a
browser:

- `features/auth/auth/src/flows.rs` — the engine's own rules
- `apps/auth-server/tests/org_pages.rs` — the same journeys over HTTP
- `apps/auth-server/tests/dev_server.rs` — seeding and snapshots

A rule should be tested there and merely *reflected* here.

## The fixture

`tests/fixtures.ts` hard-codes the seeded people and organizations,
which is only safe because the seed is deterministic — see
`apps/auth-server/src/dev.rs`. If you add to `DEV_PEOPLE` or
`DEV_ORGS`, add it here too.

Tests run in parallel against one shared server, so anything that
mutates shared state (roles, memberships) can collide. Where a test
must change something, it changes it in an organization no other test
in this suite touches, or creates its own.
