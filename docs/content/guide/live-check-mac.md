+++
title = "A live check on a Mac"
description = "scripts/mac-live-check.sh: launch the app you built with its control socket and a config folder of its own, drive it through its Action List, shoot it, give its window real input with cua-driver, run an iOS build on a Simulator of its own, and quit all of it."
weight = 96
+++

## What it is for

A test proves what it asserts; a live check shows the change working in
the real app, on the real window, the way a person meets it. On Linux
the fleet's agents start a headless test desktop for that
(`lab-session.sh` on the build server). On a Mac,
`scripts/mac-live-check.sh` is the same loop:

1. **launch** the binary you built, with `FTS_CONTROL=1` (its control
   socket, [CLI and MCP](@/guide/cli-and-mcp.md)), a config folder made
   for the session (`ARCHITECT_LAB_CONFIG_DIR`, so it never reads or
   writes the person's own settings) and a `TMPDIR` of its own (so its
   socket and its one-instance claim are the session's, and a second
   session or the person's own copy of the app is never in the way);
2. **act** through its Action List: the same `actions run` an agent
   uses over MCP;
3. **shot**: `APP_SHOT`, the app painting its own window to a PNG (no
   OS permission needed), with the window as macOS shows it when that
   fails (cua-driver's capture, which has its own Screen Recording grant,
   else `screencapture -l`);
4. **input** that has to be real (a click, a key, the menu bar, typing)
   through **cua-driver**, always naming the app's pid and its window, so
   nothing else on the screen is touched;
5. **ios**: a Simulator made for the session, the app installed and
   launched, a screenshot;
6. **stop**: the app, its child processes and the Simulator, gone; the
   session's folders removed.

## The lab, end to end

```bash
devenv shell -- cargo build -p architect-desktop
S=scripts/mac-live-check.sh
$S launch target/debug/architect-desktop
$S act THEME_SET --arg one=catppuccin-light
$S shot /tmp/lab-light.png
$S input state --png /tmp/lab-state.png     # its tree and a screenshot: where things are
$S input click 22 103                        # the rail's Tasks, in that screenshot's pixels
$S shot /tmp/lab-clicked.png
$S act LAB_STATE                             # {"tab": "tasks", …}: the click landed
$S stop
```

`launch` waits for the control socket (the app is up) and for its
window (cua-driver can see it), and prints the pid, the socket, the
window id and the config folder. `status` prints them again.

## Commands

| | |
|---|---|
| `launch <binary> [args…]` | start it; `MLC_CONFIG_ENV` names another app's config-folder variable |
| `act <ACTION> [--arg k=v…] [--json …]` | `<binary> actions --app <its socket> run …` |
| `cli <args…>` | any other `actions` subcommand: `cli list`, `cli status` |
| `shot <out.png> [--os]` | `APP_SHOT`, a second after the last action (`MLC_SHOT_SETTLE`) so its frame is drawn; `--os` takes the window as macOS composites it instead (the title bar and what macOS draws) |
| `input click <x> <y> [right] [count]` | a click in window-local pixels of `input state`'s screenshot |
| `input element <token> [action]` | an accessibility element from `input state` (`s0000002a:11`) |
| `input type <text>`, `input key <key> [mod…]`, `input hotkey <mod…> <key>` | keys into its window |
| `input menu <label…>` | its menu bar, `input menu Edit Undo` |
| `input scroll <dir> [n] [x y]`, `input drag <x1> <y1> <x2> <y2>` | |
| `input state [query] [--png out.png]` | its accessibility tree, narrowed by `query`, and a screenshot |
| `input call <tool> [json]` | any cua-driver tool, with `pid` and `window_id` overwritten by its own |
| `ios <App.app \| bundle id \| scheme> [args…]` | a Simulator of its own, installed, launched with `FTS_CONTROL=1`, a screenshot after `MLC_IOS_SETTLE` seconds (8) to `MLC_IOS_SHOT` |
| `ios-shot <out.png>` | that Simulator's screen again |
| `status`, `stop` | |

`MLC_SESSION` names a session, so several can run side by side;
`MLC_KEEP=1` makes `stop` keep the config folder and the app's log.

## Rules it keeps

- **Its own window only.** Every input carries the launched pid and
  the window id `launch` found; `input call` overwrites both. A Blitz
  app's window has little accessibility tree (its traffic lights and
  its menu bar), so most clicks are pixel clicks: read the coordinates
  off `input state`'s screenshot, which is the space cua-driver clicks
  in. A pixel click reads the window first, in the session's own
  cua-driver session (`mlc-<session>`): cua-driver refuses one on a
  window that session never read.
- **In front, for a moment.** winit hears no click or key posted to a
  window that is not in front, so clicks, keys, typing, scrolls and
  drags go with cua-driver's `foreground` delivery: it fronts the app's
  window, acts, and gives the front back. `MLC_DELIVERY=background`
  keeps them in the background (for an AppKit app that takes them so).
  The menu bar (`input menu`) and elements (`input element`) go through
  accessibility and never front it.
- **The app's tools first.** An action is exact and needs no window in
  front; a shot through `APP_SHOT` is what the app drew. cua-driver is for
  what only real input proves: a click landing where the eye says it
  is, the native menu bar, the title bar, typing.
- **Nothing of the person's.** The config folder and `TMPDIR` are
  made per session under `/tmp`; the Simulator is created for the
  session and deleted by `stop`, never one already booted.
- **cua-driver's permissions.** It needs Accessibility and Screen
  Recording for itself (`cua-driver permissions status`). Without
  cua-driver, `launch`, `act` and `shot` still work; `input` and the
  `screencapture` fallback do not.

## iOS

`ios` takes a built `.app` (installed, then launched), a bundle id
already installed, or an Xcode scheme in the current folder (built with
`xcodebuild` for the session's Simulator first). The lab's iOS app is
built by dx, not Xcode: `apps/lab/mobile`'s README builds it, then

```bash
scripts/mac-live-check.sh ios target/dx/architect-mobile/debug/ios/ArchitectMobile.app
```

The Simulator is headless (no Simulator.app window needed);
`ios-shot` takes `xcrun simctl io <udid> screenshot`. `stop` shuts it
down and deletes it.
