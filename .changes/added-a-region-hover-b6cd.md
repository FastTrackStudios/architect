---
"architect-scene" = "minor"
"architect-workspace" = "minor"
---

Added a region hover that redraws its content: `Region::hover_layer(key)` and `Region::press_layer(key)` name a layer the view sends once with `visible: false`, which the host draws in the region's layer's space while it is hovered or pressed, with no round trip to the view (`region::shown_layer`; batch `ops::VERSION` 3, written only when a region uses it)
