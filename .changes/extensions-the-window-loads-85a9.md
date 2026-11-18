---
"architect-app" = "minor"
"architect-desktop" = "minor"
---

Extensions the window loads, development copies (dev --extension, EXT_DEV, EXT_CLONE) among them, register their [[doc_kinds]] with the app's node, and take them off when they unload or reload without them, unless another source still declares them (node::kinds::LoadedKinds, follow; architect_app::on_extension_contributions)
