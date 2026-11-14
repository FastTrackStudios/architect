---
"architect-mobile" = "patch"
---

iOS: the app no longer crashes on every launch without a link; a scene connecting with no URL contexts or user activities (nil, though the bindings declare them non-null) is read as none.
