---
"architect-desktop" = "patch"
"architect-sites" = "patch"
---

Fixed a share's phone shell: it takes over only at a phone's width (on a desk the card opens the player's own view, the hash is left alone), a tab that would leave the share or the site is never drawn (shell.js and shell_json check it), #tab=constructor no longer throws, the page behind a tab of the player is inert, the tab bar has colour fallbacks for Safari 16.0/16.1, and a share page shows one title: the page's, at the top on a phone
