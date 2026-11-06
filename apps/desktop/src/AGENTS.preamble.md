# {{app}}: Architect

Architect on this machine: a desk window (Blitz) with the **Vault** (a
workspace of panes over a folder of markdown: notes, the navigator,
extension views), the settings page, ⌘K and the theme picker. The dev
sandbox (`architect-sandbox`) adds list pages (Inbox, Tasks, Schedule) and
example extensions, on made-up data. This folder, `{{dir}}`, is its home
on this machine:

| Path | What it is | Applied |
|---|---|---|
| `themes/*.json` | the app's themes and yours (the shipped ones are written here on start; an edited one is kept) | live: save and the window follows |
| `dev/<id>/` | extensions in development, and `EXT_CLONE`'s copies | live: a save reloads it |
| `extensions/<id>/` | extensions installed from a registry | on install |
| `surfaces.a2ui`, settings files | the agent-UI surfaces and the device's settings | written by the app |
| `AGENTS.md`, `actions.schema.json` | this guide and the schema | rewritten from the live list: don't edit |

Reach the running app with `{{actions}}` (its control socket; the app
serves it in a debug build or with `FTS_CONTROL=1`). Do not start a
second app to inspect it: a second launch hands off to the first.

The Vault's workspace actions (`WORKSPACE_*`, `WORKSPACES_*`) are on the
list while the Vault shows: `{{actions}} run LAB_GO --arg tab=vault` first. The
actions at the end of this guide are the ones the app had when it was
last written (it is written again whenever one comes or goes).

