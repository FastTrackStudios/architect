---
"architect-accounts" = "minor"
"architect-desktop" = "minor"
"architect-spaces" = "minor"
---

Changed share links to live under `/s/<code>/` (`architect_spaces::share::LINKS`, `link_path`, `Share::path`); the node's share sign-in moves from `/_share/signin` to `/s/_signin` and `/_share/signout` to `/s/_signout`

Migration: a link handed out as `<node>/<code>/` is now `<node>/s/<code>/`; the old path is no longer a share. A reverse proxy in front of the node routes `/s` to it (and no longer needs every root path). A share with a domain of its own is unchanged.
