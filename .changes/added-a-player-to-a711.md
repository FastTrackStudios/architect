---
"architect-desktop" = "minor"
"architect-sites" = "minor"
---

Added a player to share links and Sites (#511): audio and video embeds drawn as native players (`architect_sites::media`), served by the node with HTTP ranges (206, `Content-Range`, 416) behind the link's audience checks; each share page carries its link's queue in list order and plays it across pages; synced lyrics from an `lrc` fence or an embedded `.lrc` (`architect_sites::lyrics::parse_lrc`, `LyricsBlocks`) follow the playhead; every block follows `architect.playhead` or the `data-follow-playhead`/`data-at` attributes
