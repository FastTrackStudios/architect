---
"architect-extensions" = "minor"
---

Added a share link's own extensions (`share_needs`: `LinkNeeds` reusing `Requirements`, signed `[[packages]]` and owner-approved `[hosts]` in `_architect/shares/<code>.extensions.toml`), the guest host's model (`plan_link`, `verify_package`, `guest_capabilities`: read-only share documents, no writes or secrets, approved hosts only) and the owner's choice (`Extensions::set_share_needs`, `EXT_SHARE_NEEDS`, `EXT_SHARE_NEEDS_SET`)
