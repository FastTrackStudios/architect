---
"architect-mobile" = "minor"
---

Added `architect_mobile::run(dist)`: architect-mobile is a library too, the whole `main` of any distribution's iOS app (the counterpart of `architect_desktop::run`), with `Distribution` re-exported as `architect_mobile::distribution`, so a distribution crate links its extensions into the iPhone and iPad app (#640)
