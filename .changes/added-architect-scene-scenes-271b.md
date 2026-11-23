---
"architect-extensions" = "minor"
"architect-scene" = "minor"
"architect-workspace" = "minor"
---

Added architect-scene, scenes as they cross a boundary: binary fragments named by hash, a pane's retained layers (Layer, Layers) and the operations a view sends (ops::Sender, ops::Receiver, batch version 2); host-run animation on layers (anim: Animation, Clock::Wall and Clock::Playhead, Linear and Steps maps, named Values), hit regions and drags handled by the host, with inertia (region: Region, Drag and Drag::coast, Interaction, RegionEvent), and the engine's values written lock-free (engine::EngineTable, EngineWriter). Added PaintView::layers, PaintView::values and PaintView::region, Painter::with_clocks, Painter::with_engine, Painter::pointer and Painter::failed; the capability ui = ["engine"]; RemoteScene layers, values and onregion; RemoteDocument::set_clocks and set_engine; architect_playhead Source::rate.

New in this release and still moving: the layer (`Layer`, `Layers`, `ops`), region (`region`), animation and engine-value (`engine`) APIs and the batch format may change in later 0.x releases; the bytes on the wire to a browser (#835) and frame pacing (#839) are still being worked on.
