---
"architect-workspace" = "minor"
---

Fixed markup vanishing in desk windows: `architect_workspace::native` window documents (the main window and every pop-out) now have Blitz's HTML parser, so `dangerous_inner_html` makes nodes (the Keyflow editor's painted caret, a widget's markup). Added `native::document_config(net, fonts)`, what each window's document is made with.
