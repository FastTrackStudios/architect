---
"architect-scene" = "patch"
---

Improved architect_scene::ops::Sender::frame_with: a layer shown with the same content, placement, animations and regions costs one lookup and a pointer comparison (its transform written in place when it moved), and the sweeps for removed layers and unused fragments run only when a layer came, went or took other content. A frame moving 1,851 layers: 423 to 69 µs (p50, release).
