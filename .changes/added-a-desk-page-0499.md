---
"architect-app" = "minor"
"architect-shell" = "minor"
"architect-workspace" = "minor"
---

Added a desk page that draws the title row itself (`use_title_row`; the workspace's `WorkspaceView` claims it with a `TitleRow`): its pane tab bars sit in the 40pt title row beside the traffic lights and the sidebar button, one row instead of two. Added `SidebarDefault`, `use_sidebar_default` and `Page`'s `sidebar`: a page says whether a desk's sidebar opens folded; a page with nothing for the sidebar folds it; the person's toggle wins and is kept per page kind (the current tab). With the sidebar folded, the workspace switcher and the account fold into the ribbon's foot as marks that open the same menus. The lab's Vault opens with its sidebar folded. A pane popped out to its own window on a Mac has its tab bar in the window's title row too (transparent title bar, traffic lights placed by `fit_traffic_lights`), not under a second row.
