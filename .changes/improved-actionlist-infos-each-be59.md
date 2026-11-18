---
"architect" = "patch"
---

Improved ActionList::infos: each action parameter and result schema is worked out once per type and kept (schema::json_schema), not on every call; a menu resolved after each action run spent about a tenth of a 5120x1440 window frame thread on it during playback
