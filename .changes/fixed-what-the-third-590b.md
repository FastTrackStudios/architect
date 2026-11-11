---
"architect-extensions" = "minor"
---

Fixed what the third security review of #539 found before the npm package's first publish. A set_data key of another type than string is refused (as HostRequest types it), so the data cap cannot be bypassed. A memory64 module is refused. A view's edit cannot remove a link's rel/target, a field's autocomplete or a button's type, and web::Pane now forces rel/target on links too (wire::forced_attribute, wire::LINK_FORCED). The idle watchdog counts only the embedder's calls, and a view's timers do not run with no view mounted. fetchHandler caps open streams and their bytes per extension. Core-module reads stop at moduleBytes and each component's cache is emptied on stop. A site's root is refused as an extension's folder. integrity: false warns on the console when no log is given. The npm-pack job packs before anything else runs and npm-publish checks the tarballs' digests.
