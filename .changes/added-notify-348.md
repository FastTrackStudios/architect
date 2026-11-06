---
"architect-notify" = "minor"
"architect-extensions" = "minor"
"architect-app" = "none"
---

Added Notify (`architect-notify`, a built-in extension, id `notify`): persistent notifications with a typed source (`mention`, `thread-reply`, `task-assigned`, `task-due`, `share-opened`, `device-enrolled`, …), each person's kept in `Notifications/<name>.md` so a notification written on one device reaches their others as notes sync. A bell in the status bar with the unread count opens the notification centre (read, read all, clear, "go to" through the source service's `reveal`), a notification that arrives shows as a toast, and every extension's toasts are kept in the centre for the session (the host hands them to the provider of `architect.notify`, `architect_extensions::hostapi::NOTIFY_SERVICE`). Actions `NOTIFY_POST`, `NOTIFY_LIST`, `NOTIFY_UNREAD`, `NOTIFY_READ`, `NOTIFY_READ_ALL`, `NOTIFY_DELETE`, `NOTIFY_CLEAR`, `NOTIFY_OPEN`, `NOTIFY_CENTRE`, `NOTIFY_ME`, `NOTIFY_KINDS`, `NOTIFY_TOASTED`, `NOTIFY_POLL`; service `architect.notify`; the push relay's seam is `architect.push` `send`.
