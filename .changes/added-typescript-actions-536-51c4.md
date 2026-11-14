---
"architect-app" = "patch"
"architect-extension-archive" = "minor"
"architect-extensions" = "minor"
"architect-pages" = "minor"
---

Added TypeScript actions (#536): `kind = "typescript"` extensions run `[[actions]] script` and `[[hooks]] script` `.ts`/`.js` modules on the pages QuickJS runtime with the whole host service, typed by `architect.d.ts` (`defineAction`, `ActionHandler`, `ActionContext`). `architect_pages::js::action::TsRunner` (feature `actions`) is the host runner for `Kind::TypeScript`, with `ActionLimits` and one undo step per run; `Checker::with_typescript` and `Host::add_runner_arc` let `EXT_CHECK` load them; `Log::with_sink` forwards a runtime log. The app registers the runner from its pages host.

Changed: an extension's page and TypeScript action now run every `actions.run` through the host service (`{op: "run"}` over `EXT_PAGE_CALL`, `AppHost::run`), as Luau scripts and browser pages do: they reach the app's own actions (`VAULT_*`), checked against the extension's capabilities there. A TypeScript action has a total CPU budget (`ActionLimits::cpu`, 20 s), a total time (`time`, 10 min) and a cap on runs at once (`concurrent`, 8). A script action's answer is capped at 1 MB and its error at 4 KB, Luau's too (`host::MAX_ANSWER`, `MAX_ERROR`). `CrdtDocuments` implements `Transactions`: a Luau or TypeScript run is one undo step across documents, taken back when it fails; the app gives both runners the documents as their transactions.
