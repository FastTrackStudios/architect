---
"architect-extensions" = "patch"
"architect-scene" = "patch"
---

Improved a retained layer (architect_scene::layers::Layers::compose, a remote scene) is drawn into the window by reference (anyrender PaintScene::append_shared) instead of copying every command into a new recording each frame; Blitz repinned to FastTrackStudios/blitz c995d272 (#834).
