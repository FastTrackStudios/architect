---
"architect-extensions" = "minor"
"architect-sandbox-sketch" = "minor"
---

Fixed a process view missing what a linked view gets from the app: its paint view's `ViewCtx::actions` lists the app's actions it may run, each run through its host (`guest::app_actions`, new host call `actions`); every view a process serves has the app's theme store (`architect_theme::try_use_theme`, `use_resolved`) holding the host's theme, and `document::Style` applies in its document
