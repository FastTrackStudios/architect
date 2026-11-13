---
"architect-scripting" = "patch"
---

A saved script's inner calls (actions.run, state.query) run as the script's, never as whatever started it: a script run from ⌘K no longer hands its calls the person's palette press, so an action that asks first for anything but the person (SHARE_OPEN) asks. Extensions now see where an action came from (palette, key, menu, toolbar, script) in ctx.source where they used to see app.
