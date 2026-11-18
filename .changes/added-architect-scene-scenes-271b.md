---
"architect-extensions" = "minor"
"architect-playhead" = "minor"
"architect-scene" = "minor"
"architect-workspace" = "minor"
---

Added architect-scene, scenes as they cross a boundary: binary fragments named by hash, a pane's retained layers (Layer, Layers) and the operations a view sends (ops::Sender, ops::Receiver, batch version 2); host-run animation on layers (anim: Animation, Clock::Wall and Clock::Playhead, Linear and Steps maps, named Values), hit regions and drags handled by the host (region: Region, Drag, Interaction, RegionEvent), and the engine's values written lock-free (engine::EngineTable, EngineWriter). Added PaintView::layers, PaintView::values and PaintView::region, Painter::with_clocks, Painter::with_engine, Painter::pointer and Painter::failed; the capability ui = ["engine"]; RemoteScene layers, values and onregion; RemoteDocument::set_clocks and set_engine; architect_playhead Source::rate.
