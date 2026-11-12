---
"architect" = "none"
---

Fixed clippy::unused_async on macOS and Windows: blob::place's reflink is an async fn on Linux only, a ready future elsewhere.
