---
"architect-extensions" = "minor"
---

Changed: an extension's preset target, hook action and hook script now run as that extension (`Principal::extension(owner, for whoever ran it)`, `host::as_extension`) instead of as the app, so a service that checks whose call it is (`architect.playhead`'s own-source rule) sees the extension: a preset or hook can no longer reach another extension's state by running as the app. An action a preset or hook names now gets `caller` in its context and is checked against the list's authorizer, if one is set. Added `Host::raise_app_each`: an app event whose detail is made for each extension that hears it.
