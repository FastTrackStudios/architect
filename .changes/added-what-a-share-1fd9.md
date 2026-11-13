---
"architect-desktop" = "minor"
"architect-extensions" = "minor"
"architect-sites" = "minor"
"architect-spaces" = "minor"
"architect-web" = "minor"
---

Added what a share link guest view reads (#648, #669): ShareGuest::read fills GuestNote::source with the share build text (comments, raw HTML and anything past the link gone) only when the node island for the note type reads the note, and the new GuestNote::files with the link images and lyrics only when its extension declares reads = ["files"] (new capability architect_extensions::capabilities::FILES, architect_sites::island::note_reads, READS_FILES); note_props hands a guest the body and either audience its files (ViewFile, capped by architect_spaces::view: 2 MiB a file, 8 MiB and 32 files a note); without_comments moved to architect_spaces::view. Security: a share build no longer reads or copies a note or attachment that resolves outside the vault (a symlink out of it), and a page _note.json is no longer served as a file of the link.
