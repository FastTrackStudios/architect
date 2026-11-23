---
"architect-extensions" = "minor"
---

The web app installs and runs extensions (#495): the showcase vault's Install all installs from the registry the app carries, through the one consent prompt, kept in the browser's storage; TypeScript extensions run in Web Workers and their TSX pages draw in the app; Luau is listed as "Can't run here" (#812). The app page's policy allows `blob:` scripts and workers for those pages.
