---
"architect-extensions" = "minor"
"architect-workspace" = "minor"
---

Added architect_workspace::pacing: a runtime view's frames (each batch, and each change its document makes, through remote::HostShell::pacer) are asked of the window at most once a refresh after a frame that waited for its swapchain image, started just before the next image comes free, instead of each starting a frame that waits most of a refresh with the window thread blocked. ARCHITECT_FRAME_PACING=0 turns it off. APP_FRAMES records how each frame was asked for (#840).
