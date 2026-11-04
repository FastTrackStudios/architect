---
"architect-extensions" = "minor"
"architect-pages" = "minor"
"architect-workspace" = "minor"
---

Added the app's interface for extensions (#306): notifications, clipboard read and write, file and folder pickers and save dialogs, prompts (text, confirm, pick-one), a context menu of an extension's own items at a point, and icons and badges on file-tree rows and tabs. Each is a host-service call (HostRequest::Notify, ClipboardRead, ClipboardWrite, Pick, Prompt, Menu, Decorate) gated by the new `capabilities.ui`, the same from Luau (`host.prompt`…), the WIT world's typed imports (wasi-guest `host::pick`…), processes, linked extensions and pages (`ui` and `clipboard`, PageHost::host_call, EXT_PAGE_CALL). The app answers through a HostUi (Host::set_ui): NoUi by default, FakeUi for tests and headless hosts, and the window's WindowUi drawn by ExtensionOverlays (rfd pickers with feature `pickers`). Badges are the `app.decorations` point plus Host::decorations, drawn by host_ui_view::Decorations and the workspace's TabBadges; EXT_DECORATIONS lists them. API breaks: `Capabilities` has a `ui` field, `HostError` an `Unsupported` variant, and a Luau `host.*` call that answers `null` now returns `nil`.
