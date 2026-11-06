---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-mobile" = "minor"
"architect-sandbox" = "minor"
"architect-sandbox-board" = "minor"
"architect-sandbox-clicker" = "minor"
"architect-sandbox-media" = "minor"
"architect-sandbox-wasi-counter" = "minor"
"architect-web" = "minor"
---

Changed the lab into the Architect app (#383): `apps/lab/{desktop,mobile,web,ui}` are `apps/desktop`, `apps/mobile`, `apps/web` and `apps/ui`, the UI crate renamed `architect-lab-ui` → `architect-app` and the web one `architect-lab-web` → `architect-web`. Added `architect_app::Mode` (Architect or Sandbox, set by each binary) and `architect_desktop::run`. The Architect app (`architect-desktop`, `just app`) is "Architect" (bundle id `app.fasttrackstudio.architect`), keeps its folder in `architect` (`ARCHITECT_CONFIG_DIR`; `ARCHITECT_LAB_CONFIG_DIR` still read), copies an existing `architect-lab` folder into it once, and on first run opens a folder, creates a vault or explores the built-in demo vault, then offers the tour. Added the dev sandbox `architect-sandbox` (`apps/sandbox`, `just sandbox`): the made-up data and example extensions, with its own folder (`architect-sandbox`, `ARCHITECT_SANDBOX_CONFIG_DIR`); the example extensions, scenarios and media crate moved beside it, renamed `architect-sandbox-board`, `-clicker`, `-wasi-counter` and `-media` (its binary `sandbox-media`). `ARCHITECT_LAB_VAULT`/`_SEED`/`_MCP_PORT`/`_MCP_TOKEN` are `ARCHITECT_VAULT`, `ARCHITECT_VAULT_SEED`, `ARCHITECT_MCP_PORT` and `ARCHITECT_MCP_TOKEN`, the old names still read.
