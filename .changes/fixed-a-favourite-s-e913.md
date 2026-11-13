---
"architect-app" = "patch"
"architect-web" = "patch"
---

Fixed: a favourite's declared playhead target (`PLAYHEAD_TOGGLE`) moves its declaring extension's own source, natively and in the browser; the web app's Notes source is `app:notes`; each extension frame has its own ask rate limit beside the shared one (`ext_host::take_ask`); a view at a note's head is capped at 60 % of the window and scrolls inside (`ext_host::head_height`); `ext_host::vault_path` is the playhead's rule; `media_url` carries the #628 media rules.
