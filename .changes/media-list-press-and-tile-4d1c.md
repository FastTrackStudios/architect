---
"architect-media" = "minor"
---

The media list (`ui::MediaList`) takes a press of its caller's: with `on_press` a row is pressed rather than played (told the row's id, nothing goes to the playhead), the pill says `head` and runs `on_head`, `sum` replaces the line beside it and `current` names the row that is current. A row's `tile` (`Tile`: a short text on a colour, a song's key) is drawn where it has no cover and over the cover's corner where it has one. A list outside the shell (an extension's own process) takes its colours from the theme's variables. Studio's launcher mounts it to load a song's session where a setlist's page plays its recording.
