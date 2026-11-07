---
"architect-app" = "minor"
"architect-sandbox-web" = "minor"
"architect-web" = "minor"
---

Added the web front end (#431): `apps/web` is the web app's shell (OIDC redirect sign-in with PKCE as client `architect`, the node's status), bundled with a landing page and the docs as one static site (`scripts/web-bundle.sh`, `just web`, `just web-test`) and the `architect-web-image` flake output; the sandbox's browser demo moved to `apps/sandbox/web` (`architect-sandbox-web`, `just sandbox-web`); the desktop app's issuer client id is now lowercase `architect`
