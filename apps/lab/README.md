# The lab: `architect-desktop` and `architect-mobile`

Two thin apps that mount one made-up app on `architect_shell`
(`apps/lab/ui`), for designing the shell on a phone, a tablet
and a desk. Both paint through Blitz, as the FastTrackStudio apps do, and
both are meant to be run with `dx serve`: an `rsx!` edit in
`apps/lab/ui` or in `features/shell` shows in the
running app without a rebuild (a change to Rust logic rebuilds).

The lab is Task's shape: Inbox, Tasks, Schedule and Vault as the tabs,
Capture as the separate button, projects on the left, details on the
right. The lab's own menu (the flask beside a page's ⋯) switches the
shell's style (Floating pill, Tab bar, Rail
only) and the right side live. The Vault tab is the workspace host
(`architect-workspace`'s `ui`), Obsidian's shape: on a desk its tabs are
the window's title row; drag a tab to a group's middle or edge or the
window's edge, hold W to grab a pane without tabs, right-click a tab (or
⋯) for the rest.

The lab's lists are live. A click selects a row (the details follow,
with what it can do and the keys for it), a double click or Return
opens it; ↑ and ↓ (or ⌃N and ⌃P) select. Capture (⇧⌘N, or the feather)
and a list's + write a new row (Return or Esc keeps it). A row's circle,
Done or ⌘↵ completes one: its circle fills, it fades, and the next row
is selected. Move to, or ⇧⌘M and a few letters of the project, files a
capture and selects the next one. Edit › Undo and Redo (⌘Z, ⇧⌘Z) step
through the last twenty changes. An item's page keeps notes. ⌘/ shows
every key and where things are. The lab's face, Inter, is
bundled (`ui/assets/fonts`, OFL) and handed to Blitz at launch
(`FONTS`): Blitz does not load an `@font-face` from a `data:` URI. What
else Blitz does differently is in the skill's
`skills/architect/references/blitz-quirks.md`.

## On a desk

```bash
cd apps/lab/desktop
dx serve --platform desktop --renderer native
# or open it standing in for a device:
ARCHITECT_DEVICE=iphone dx serve --platform desktop --renderer native
```

`ARCHITECT_DEVICE` is `iphone`, `iphone-side`, `ipad` or `ipad-side`: the
window takes that device's size, touch and safe area, with its status bar,
camera housing and home indicator drawn over the page. The lab's menu (the
flask) changes it live. A mouse drag stands in for a swipe (the shell hears pointer
events).

The Vault's Clicker tab is a process extension's view (`apps/lab/clicker`):
a counter whose `VirtualDom` runs in a process of its own, its edits
applied in the pane over vox. It runs when its program is built beside
the lab's (`cargo build --workspace`, then `target/debug/architect-desktop`).
Kill that process and the pane says why, with a button to start it again;
the lab runs on.

## On the iOS simulator

```bash
cd apps/lab/mobile
env -u DEVELOPER_DIR -u SDKROOT \
  PATH="$PWD/ios:$PATH" SIM_UDID=<udid> SIM_SET=<device set, if not the default> \
  dx serve --ios
```

- `ios/Info.plist` is the app's whole plist (dx takes it verbatim): iOS
  27 stops an app that has not adopted the UIScene lifecycle, and
  `src/scene.rs` is the delegate it names.
- `ios/xcrun` sends dx's `xcrun simctl … booted` to `SIM_UDID` in
  `SIM_SET`: dx assumes one booted simulator in the default device set.
- The iOS targets must be installed for the toolchain cargo uses
  (`rustup target add aarch64-apple-ios-sim`).

With no Simulator.app at hand (a headless Mac), watch and drive the
simulator with `xcrun simctl io <udid> screenshot --mask=black` and `idb ui
tap` / `idb ui swipe`.
