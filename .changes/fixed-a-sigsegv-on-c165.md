---
"architect-workspace" = "minor"
---

Fixed a SIGSEGV on docking a popped-out dock back (NVIDIA on Wayland): a closed pop-out's GPU device is kept until the app ends. Fixed a dock popping out into its own window from a press on its grip that never moved past the slop (a framed window's own move released outside). Added Reset layout: WORKSPACES_RESET falls back to the first built-in screenset when none is active (Screensets::reset_target, Screensets::is_resettable), a Reset layout button in the manage sheet and WORKSPACE_RESET_LAYOUT on the palette; every popped-out dock and pane docks back and its window closes. Added WorkspaceView's empty: the main area with no tab says No file is open, with the app's commands, Go to file and Close. A desk's pop-out and main windows load file: URLs from disk.
