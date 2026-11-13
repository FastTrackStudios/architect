---
"architect-extensions" = "patch"
---

A process extension's view draws like a linked one: its document starts from what a pane gives a linked view (no margin, the pane's height, the theme's ink and font at 13px), a <style>'s raw CSS (style { dangerous_inner_html: CSS }) is taken as its sheet while raw HTML anywhere else still fails the view, and RemoteView / RemoteEmbed are keyed by extension and view, so a pane that goes on to another process view mounts that view instead of keeping the first one's session.
