---
"architect-extensions" = "none"
"architect-lab-ui" = "none"
"architect-lab-media" = "none"
"architect-lab-board" = "none"
"architect-lab-clicker" = "none"
---

The lab's note editor backs its fence registry with `Embeds::fences` (#180): a fenced block in a language an extension claims (the Board's ```` ```kf ````) keeps its source and mounts the extension's `EmbedView` under it as a block widget, gone with the extension. Tests close out #178 (an extension's principal reaching a vox-served `PermissionsGate`, `features/extensions/tests/host_gate.rs`), #181 (the metronome and the clicker installed, updated, turned off and on and removed by clicks on the manager's page, and `MissingExtensions` "Install all" through one consent prompt, `apps/lab/clicker/tests/manager_page.rs`), #183 (the Board's views, opener and embed going and coming back with it) and #180 (a ```kf fence resizing with its content on Blitz, `apps/lab/board/tests/kf_fence.rs`).
