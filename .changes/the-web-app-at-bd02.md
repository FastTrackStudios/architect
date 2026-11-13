---
"architect-web" = "minor"
---

The web app at `/app/?demo` (the landing page's Demo button) is Architect itself on dioxus-web: the shell, the Vault's workspace, the navigator, ⌘K and ⌘O and the settings page, on the demo vault in the tab's memory, with no sign-in and nothing sent. The page's CSP keeps scripts to its own and the wasm (no eval) and now allows inline styles. The wasm is built with the new `web` profile (opt-level z, fat LTO).
