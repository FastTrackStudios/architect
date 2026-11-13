---
"architect-pages" = "minor"
---

Added AppHost::guest and Pages::open_guest_embed: a page drawing a guest's content (a share link's note) runs no action, makes no host-service call and keeps its settings and shared state apart, reaching nothing of the person's (#629)
