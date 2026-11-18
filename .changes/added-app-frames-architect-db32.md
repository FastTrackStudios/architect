---
"architect-desktop" = "minor"
"architect-sandbox-sketch" = "minor"
"architect-workspace" = "minor"
---

Added APP_FRAMES (architect_workspace::frames): the window records its own frames (per frame its draw and render times, the display refresh, the input and runtime batches it shows) and writes them with a summary (presented fps, dropped refreshes, frame-time and input-to-frame percentiles) to a JSON file; drive feeds it wheel or drag input through its own event handler. just frames runs the sandbox Sketch timelines with it, linked and as a process; Sketch has a 20,000-clip timeline view, timeline-20k (#839).
