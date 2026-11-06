---
"architect-navigator" = "minor"
---

Added VaultSource (feature vault): architect's vault (vault::actions::OpenVault) as a NavigatorSource, live: folders, notes with properties, tags, preview, feature image and task counts, bases and attachments; save, create_note and rename (backlinks follow) emit their SourceEvents at once, and watch hears edits made outside the app. Added feature images on the list's rows, NavigatorView's columns (the shell sidebar's: one pane or two) and on_new_note (New note in the list's header).
