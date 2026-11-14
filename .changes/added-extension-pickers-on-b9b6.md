---
"architect-extension-wasi" = "minor"
"architect-extensions" = "minor"
"architect-pages" = "minor"
---

Added extension pickers on iOS (#708): `pick` opens UIKit's document picker (files as the app's own copy, a folder through its security scope, a save as a folder and the suggested name) and the new `PickKind::Photo` / `PickKind::Photos` kinds (`{"op":"pick","kind":"photo"}`), `PHPickerViewController` on iOS (no photo-library permission; each image copied for the extension) and a file picker for images (`hostcall::image_filter`) on a desk; gated by `capabilities.ui = ["pickers"]` like every pick
