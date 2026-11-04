---
"architect-lab-ui" = "minor"
---

Added the lab's Embedded views screenset (⌘3): a Dioxus, a Luau ImGui, a Frame surface and an A2UI pane, each naming its kind. The Frame surface shows `apps/lab/surfaces/drive.frame`, reloads it on save, and binds its knob to the lab's drive with its meter following (LAB_FRAME_SET, LAB_FRAME_STATE). The A2UI pane opens on a release demo (LAB_A2UI_DEMO): a form, a task list, and a Ship it button whose LAB_RELEASE_SHIP waits for confirmation; it can be patched live with A2UI_PATCH. LAB_A2UI_DATA reads a surface's data.
