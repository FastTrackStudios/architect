---
"architect-lab-media" = "patch"
"architect-shell" = "patch"
"architect-workspace" = "patch"
---

Fixed a page that draws the desk's title row (`use_title_row`): its tabs past the first pane took no press (the shell's row was hit instead), now the page is laid out from the window's top and the shell draws its row only beside it. Fixed a tab dropped while the shell's sidebar is folded: the workspace's left dock is left out of the drop targets while it is not on screen.
