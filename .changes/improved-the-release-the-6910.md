---
"architect-web" = "patch"
---

Improved the release: the web bundle (architect-web-<X.Y.Z>.tar.gz, what the server deploys) now waits only for the release job, not the desktop and node builds. The site's /download names each file of the release by convention from the tag (links that work with scripts off) and asks GitHub's API once in the browser which files are up, with their sizes and sha256; a file not up yet (or the optional macOS build) shows as not available, and if the API cannot be reached the links stay. The page also shows how to check a download against the signed SHA256SUMS. A release's site build still fails unless the release and its notes exist.
