+++
title = "App presence"
description = "How a desk app lives when its window is not on screen: the Dock icon (badge, menu, progress, attention), a menu-bar or tray item, background mode, reopen, opening at login and one instance, all on the Action List."
weight = 56
+++

> **Status: landed (2026-11-01)** in `architect-presence`
> (`features/presence`): the model (`features/presence/src/model.rs`
> `Presence`, `Tray`, `Badge`, `MenuTree`; `features/presence/src/lifecycle.rs`
> `Lifecycle`; `features/presence/src/login.rs` `LoginItem`, `plan`;
> `features/presence/src/instance.rs` `Handoff`), the component that binds
> it (`features/presence/src/ui.rs` `AppPresence`) and the platforms'
> side: macOS (`features/presence/src/mac.rs`, proven in the real Dock and
> menu bar with the lab), Linux (`features/presence/src/linux.rs`:
> StatusNotifierItem, Unity's launcher entry, XDG autostart; checked to
> build, not yet run on a desktop) and Windows (the window's life and the
> Run key; no tray yet, and not compiled here). The lab uses it
> (`apps/lab/ui/src/presence.rs`).

Task and Studio each hand-rolled how their app lives off screen: Task's
tray menu and its LaunchAgent with `--background`
(`task:apps/desktop/src/tray.rs`, `task:apps/desktop/src/menu_bar.rs`),
Studio's accessory process with a status item listing its engines
(`studio:apps/studio/desktop/src/menubar.rs`). Each item in those menus
was a selector or a match arm, nothing ⌘K or a model could reach. An app's
presence is a view of what it can do, so architect builds it from the
Action List, as it builds the [menu bar](@/architecture/app-menus.md).

(Not people's presence in a document: that is `crdt::presence`.)

## What an app says

```rust
let presence = Presence::new("app.fasttrackstudio.task", "Task")
    .dock_menu(Menu::new("Dock").command("TASK_CAPTURE").section("Projects", projects))
    .tray(
        Tray::new(TrayIcon::symbol("checklist").icon_name("task-due"), tray_menu)
            .tooltip("Task")
            .title((due > 0).then(|| due.to_string())),
    )
    .background(true)   // closing the window keeps it running (a setting, on by default)
    .open_at_login();   // a setting, off until the person turns it on
rsx! { AppPresence { presence, badge: Badge::count(inbox.len()) } }
```

and in `main`, before anything opens:

```rust
if architect_presence::single_instance(ID) == Launch::HandedOff {
    return;
}
let window = window.with_visible(!architect_presence::launched_in_background());
```

- **The menus are Action List menus** (`architect_commands::Menu`):
  commands by id, groups, separators and live sections, resolved against
  the commands as the menu bar is, so a toggle is ticked, a command that
  cannot run is greyed with its reason, and a live section is read as the
  menu opens. The platform's own items are left out: the Dock adds
  Options, Show All Windows, Hide and Quit itself; a tray quits through
  `PRESENCE_QUIT`.
- **The badge is bound:** the app gives the badge it has now each time it
  draws; the platform hears only a change. A count over 999 reads `999+`;
  zero is no badge. Progress (`0.0`–`1.0`) is the same; attention is
  asked of `PresenceHandle::request_attention`.
- **Every part is an action:** `PRESENCE_SHOW_WINDOW`,
  `PRESENCE_HIDE_WINDOW`, `PRESENCE_QUIT`, `PRESENCE_TOGGLE_BACKGROUND`
  and `PRESENCE_TOGGLE_LOGIN` (toggles, ticked as their settings are) and
  the read-only `PRESENCE_STATE` (where the app is, its badge, its
  settings). `AppPresence` lists its Action List as commands, so a Dock
  or tray menu, ⌘K, MCP and a tour reach them.
- **Settings › App** holds "Keep running when the window closes" and
  "Open at login" (`settings_section`), each only when declared.

## The window's life

`Lifecycle` is the state machine, pure and tested; the platform carries
out its `Effect`s.

| Event | In the window | In the background |
|---|---|---|
| launch | Dock icon on | `--background`: window hidden, Dock icon off |
| close the window | background mode on: hide it and the Dock icon; off: it closes and the app quits | — |
| Dock click (`applicationShouldHandleReopen`), tray Open, a second launch | to the front (a minimised window back) | the window back where it was, the Dock icon back, in front |
| Quit | quits | quits |

The close is the workspace launcher's
(`features/workspace/src/native.rs` `MainWindow::on_close`): it asks the
app before it lets the main window go, and `MainWindow::quit` ends the
loop. With the Dock icon off the app is an accessory (no Dock tile, no
⌘Tab, no menu bar until it is shown again).

## Where it shows

| | macOS | Linux | Windows |
|---|---|---|---|
| Badge, progress, attention | the Dock tile (`NSDockTile`; progress a bar across the icon's foot) | Unity's `LauncherEntry` (Ubuntu's Dock, KDE's task manager); attention also winit's urgency hint | winit's attention only |
| Dock menu | `applicationDockMenu:`, built as it opens | none (a launcher's menu is its desktop file's) | none |
| Reopen | `applicationShouldHandleReopen`, and Launch Services opening the running bundle again | a second launch | a second launch |
| Tray item | `NSStatusItem`, a template image (SF Symbol or PNG) that follows light and dark, an optional title; its menu filled as it opens (`menuNeedsUpdate:`); `TrayClick::Run` runs a command on the primary click and opens the menu on the secondary | StatusNotifierItem, its menu the commands' dbusmenu object (`architect_commands::DbusMenu`) | not built: says so in the log |
| Open at login | a LaunchAgent with `--background` (Task's), in `~/Library/LaunchAgents/<id>.plist` | XDG autostart, `$XDG_CONFIG_HOME/autostart/<id>.desktop` | the `HKCU\…\Run` key, through `reg.exe` |
| One instance | a socket in the person's temporary folder | a socket in `$XDG_RUNTIME_DIR` | not built: every launch runs |

**Opening at login** is written only for an installed copy (a bundle in
`/Applications` or `~/Applications`; on Linux under `/usr`, `/opt`,
`~/.local`, a Nix profile or an AppImage; on Windows under Program Files
or `%LOCALAPPDATA%\Programs`), so a build in `target/` or an app on a disk
image never becomes what starts at login. Elsewhere the toggle is greyed
with why. The file is written again at each launch while the setting is
on, so it follows the app when it moves, and removed when it is turned
off. A LaunchAgent rather than `SMAppService` because it can pass
`--background`; macOS shows its "background activity" notification when
the agent appears.

**One instance:** `single_instance` connects to the running copy's socket
and hands it a `Handoff` (its arguments, its folder, the URLs among them)
as one JSON line; the running copy comes forward and gives it to
`AppPresence`'s `on_handoff`. A socket nothing answers (a copy that
crashed) is taken over. On macOS, URLs opened by Launch Services arrive
through `application:openURLs:` the same way (the bundle declares the
scheme).

## Proven on macOS

The lab, bundled as `Architect Lab.app` and run on an M4 Mac mini: the
Dock tile badged with the Inbox count; the Dock menu listing Capture, the
tabs (the one on show ticked) and the background toggle, a pick running
its command; closing the window leaving the lab in the menu bar as an
accessory with no Dock tile; the tray menu's Show the window, a second
launch, and `open` on the bundle each bringing the window back where it
was, the badge with it; Quit quitting; Open at login writing the
LaunchAgent from `~/Applications` and removing it.

## From what exists

- Task's `menu_bar.rs` and `tray.rs` become a `Presence` declaration: its
  tray items become actions (mount the folders, read what has not been
  read, sync as the signed-in account, check for updates), its
  `dock_icon` and `ensure_login_item` are this crate's. Task is on
  dioxus-desktop (a web view), so it needs the window side
  (`MainWindow`) from its own launcher.
- Studio's menubar process becomes a `Presence` with a live section of its
  engines, each row its stop action.

## Open questions

1. **Windows' notification area and one instance** (a named pipe): built
   when an app ships on Windows.
2. **Reopen with no window at all** (an app whose last window was closed
   for real): today the main window is hidden, never dropped, so it comes
   back where it was.
