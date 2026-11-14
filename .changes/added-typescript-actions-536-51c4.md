---
"architect-app" = "none"
"architect-extension-archive" = "minor"
"architect-extensions" = "minor"
"architect-pages" = "minor"
---

Added TypeScript actions (#536): `kind = "typescript"` extensions run `[[actions]] script` and `[[hooks]] script` `.ts`/`.js` modules on the pages QuickJS runtime with the whole host service, typed by `architect.d.ts` (`defineAction`, `ActionHandler`, `ActionContext`). `architect_pages::js::action::TsRunner` (feature `actions`) is the host runner for `Kind::TypeScript`, with `ActionLimits` and one undo step per run; `Checker::with_typescript` and `Host::add_runner_arc` let `EXT_CHECK` load them; `Log::with_sink` forwards a runtime log. The app registers the runner from its pages host.
