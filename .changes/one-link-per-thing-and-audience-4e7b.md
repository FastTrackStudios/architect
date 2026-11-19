---
"architect-spaces" = "major"
"architect-accounts" = "minor"
"architect-desktop" = "minor"
---

Changed sharing so a thing has one link an audience, for good: `SHARE_CREATE` on a thing already shared with that audience returns that link with what the thing holds now and mints nothing (`Share::is_link_for`, `ShareScope::root`); another audience is another link. Added a link's name for its owner (`Share::name`, never shown to a guest), pause and resume (`ShareChange::Paused`, `ShareState::Paused`: closed to everyone, the same code after; revoking is the only thing that ends a code), and when it was last opened (`Share::opened_at`, since the node started, never saved). `SHARE_UPDATE` takes `name` and `paused`; `SHARE_LIST` gives each link's name, root and last opening. The Share sheet opened on a thing lists that thing's links first, and Revoke asks first.

Migration: `architect_spaces::Share` has three new fields (`name`, `paused_at`, `opened_at`), so a struct literal adds them (`None`); `ShareState` has a new variant, `Paused`, so an exhaustive `match` adds an arm.
