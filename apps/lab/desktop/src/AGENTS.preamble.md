# {{app}}: the architect lab

The lab is architect's app-shell playground: a desk window (Blitz) with
list pages (Inbox, Tasks, Schedule), the **Vault** (a workspace of panes:
notes, a graph, extension views), the settings page, ⌘K and the theme
picker. This folder, `{{dir}}`, is its home on this machine:

| Path | What it is | Applied |
|---|---|---|
| `themes/*.json` | the lab's themes and yours (the shipped ones are written here on start; an edited one is kept) | live: save and the window follows |
| `dev/<id>/` | extensions in development, and `EXT_CLONE`'s copies | live: a save reloads it |
| `extensions/<id>/` | extensions installed from a registry | on install |
| `surfaces.a2ui`, settings files | the agent-UI surfaces and the device's settings | written by the lab |
| `AGENTS.md`, `actions.schema.json` | this guide and the schema | rewritten from the live list: don't edit |

Reach the running lab with `{{actions}}` (its control socket; the lab
serves it in a debug build or with `FTS_CONTROL=1`). Do not start a
second lab to inspect it: a second launch hands off to the first.

The Vault's workspace actions (`WORKSPACE_*`, `WORKSPACES_*`) are on the
list while the Vault shows: `{{actions}} run LAB_GO --arg tab=vault` first. The
actions at the end of this guide are the ones the lab had when it was
last written (it is written again whenever one comes or goes).

