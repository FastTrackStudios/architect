# {{app}}

This folder is **{{app}}'s home**: the files it keeps on this machine
(settings, themes, extensions in development, its workspace) and this
guide. The guide and `actions.schema.json` beside it are written by the
app from its live Action List: what follows is what the running app can
do now, not what it could do once.

Everything the app can do is an **action** on its Action List: a key, a
menu, the command palette, a script, an extension and you all run the
same ids. You reach the running app with:

```sh
{{actions}} list                 # every action: id, effect, what it does
{{actions}} describe ID          # what one takes and gives (JSON Schema)
{{actions}} run ID --json '{…}'  # run one; `--arg key=text` works too
{{actions}} events --follow      # what changes, as it happens
```

or over MCP, the same five tools (`actions`, `describe`, `run`, `batch`,
`watch`).

## Inspect before acting

```sh
{{actions}} run APP_STATUS       # windows, focus, layout, theme, open sheets and every problem
{{actions}} shot /tmp/app.png    # the window as the app drew it (APP_SHOT)
```

`APP_STATUS` is one read of what is on screen and what is wrong: every
problem from every source (theme files, extension loads, layouts), each
with its source, file, field and message. Read it before you change
anything and after. A screen that still works is not proof an edit
worked: a broken file keeps its last good version running, and only
`APP_STATUS` says it is broken.

`APP_SHOT` renders the window (or one pane: `--pane <id>`) to a PNG and
answers its path and size. Look at the picture before you say a visual
change is done.

## Changing things safely

- **Read-only, mutating, destructive.** Every action says which (its
  effect, below). Read-only ones are safe any time. A destructive one run
  from outside the app waits for the person: the answer is
  `confirmation_required` with a `ticket` (exit code 7 on the command
  line). Nothing hangs and no dialog opens for you: tell the person, and
  run the same call again with `--ticket` once they have approved it.
- **Arguments are checked.** A name an action does not take is refused
  with the names it does take. Read `describe` rather than guessing.
- **Last good.** A file that does not read (a theme, a manifest) keeps
  the last version that did, and the problem is in `APP_STATUS`. The app
  does not write `<name>.error` files yet.
- **Clone to edit.** What the app ships is read-only to you: copy it
  first and edit the copy. `WORKSPACES_CLONE {name, as?}` copies a
  built-in workspace layout (saving over a built-in one from outside the
  app is refused); `EXT_CLONE {id, as?}` copies a built-in or installed
  extension into the development folder under a new id.
- **Escape hatch.** {{escape}} always works and nobody can rebind it: it
  sets every built-in workspace layout back as shipped (a changed one is
  kept as a copy), shows the first, and drops the person's key changes.
  Tell the person about it when you change their layout or keys.
- Make only the change you were asked for.

## Themes

`THEME_LIST`, `THEME_GET`, `THEME_SET {one | light, dark, appearance}`,
`THEME_TOKEN_SET` and `THEME_PROBLEMS`. Theme files live in this folder's
`themes/`: save one and the app follows; a file that does not read keeps
its last good theme and shows in `APP_STATUS`.

## Workspaces

The window's panes are a revisioned layout tree: `WORKSPACE_LAYOUT`
reads it (its `revision` too), every `WORKSPACE_*` write takes an
`if_revision` and answers `stale` when the layout moved under you.
Saved layouts are `WORKSPACES_*`: list, load, save, clone.

## Extensions

`EXT_LIST`, `EXT_INSTALL`, `EXT_RELOAD`. An extension in development
lives in a folder (`extension.toml` and its scripts or program); the app
reloads it when its files change.

The dev loop, after every change to an extension:

```sh
{{actions}} run EXT_CHECK --arg folder=/path/to/extension
{{actions}} run APP_STATUS
{{actions}} shot /tmp/ext.png
```

`EXT_CHECK` loads the folder once on its own (manifest, scripts or
program), renders each of its views once and answers `ok` or the exact
error: the manifest field, the script and line, the view that did not
render. Never call an extension done without a passing check.
