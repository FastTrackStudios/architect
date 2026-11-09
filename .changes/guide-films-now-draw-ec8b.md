---
"architect-guide" = "minor"
"architect-sandbox-media" = "minor"
"architect-workspace" = "minor"
---

Guide films now draw exactly like the app: the runner mounts each window in the desk window's own document (`architect_workspace::native::offscreen_document`, with a head for `document::Style`), so every app stylesheet and `dangerous_inner_html` widget is in the film. Icon-only workspace tabs centre their icon.
