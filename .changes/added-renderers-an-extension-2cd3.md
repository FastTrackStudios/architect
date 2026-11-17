---
"architect-app" = "minor"
"architect-cookbook" = "minor"
"architect-extensions" = "minor"
---

Added [[renderers]]: an extension renders a note type's text in the editor, as markdown has its live preview. While such a note is open the app runs the renderer's action with {text} on each change and draws its answer (architect_extensions::render::Rendered: RenderItem conceal, replace, mark and line, in the app's Look heading, strong, muted, accent, chip, chip-accent, code) over every line but the caret's. A body that is not markdown gets no markdown preview. The Cookbook renders Cooklang (COOKBOOK_RENDER): @flour{250%g} reads as "250 g flour", = Batter as a heading; its card at a recipe note's head leaves the steps to the text.
