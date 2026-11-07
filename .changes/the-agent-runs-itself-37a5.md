---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-files" = "minor"
---

The agent runs itself: architect-desktop agent install|uninstall|start|status writes agent.toml with its defaults (drive ~/Architect, store <data>/roots, bind 127.0.0.1:18471) and a systemd user unit (ExecStopPost unmounts the drive) or a launchd agent; the window joins the installed agent, starting it when it is down.
