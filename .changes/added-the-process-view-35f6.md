---
"architect-extensions" = "minor"
"architect-process-view" = "minor"
"architect-workspace" = "minor"
---

Added the `process` view kind for every extension (#838): a `[[views]] kind = "process"` names a `program` (and `args`) in its package, asked for in `capabilities.programs` and granted like any capability (never to a share-link guest), and the app draws it with the process-view pane whichever copy is loaded (built in, installed or in development), restarting it when it dies or a development copy is rebuilt and stopping it when its pane closes or the extension unloads; on Linux and macOS (Windows: its pane says why). New: `architect_workspace::PROCESS` and `KindSpec::process`, `architect_extensions::process_view` (`launch`, `not_here`, `package_file`), `interface::ProcessViews` and `ProcessAsk`, `Capabilities::programs` and `may_run_program`, `ViewDecl::program` and `args`; `architect-process-view` feature `extension` (`extension::use_process_views`, `open`, `reap`), `host::Restart`, `host::park`/`unpark`/`end`/`reap`/`find`, `Program::args`/`dir`/`stamp`/`restart`, `Session::greet_without_window` and `take_frames_without_window`; the app registers the `process` kind on every platform.

New in this release and still moving: the process view kind, its wire and `architect-process-view`'s API may change in later 0.x releases.
