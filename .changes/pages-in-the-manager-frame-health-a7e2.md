---
"architect-extensions" = "minor"
"architect-pages" = "minor"
"architect-control" = "minor"
"architect-lab-ui" = "minor"
"architect-desktop" = "minor"
---

Joined the extension seams: pages are a kind in the extensions manager (`architect-pages` feature `manager`, `PagesKind`: list page files and overrides with status and problems, check, clone to edit, remove the override to restore); a slow or busy view is an `APP_STATUS` problem until it recovers (`views::ViewProblems` for `Meters::with_report`), and an extension's manager page shows its views' p50, p99 and slow count (`ExtensionKind::with_meters`); pages' problems feed `APP_STATUS` (`Pages::report_problems`) and `EXT_CHECK` renders an extension's page files (`Checker::with_page_checker`, `Host::set_page_checker`, `Pages::page_checker`); the generated guide writes and names files beside it (`Guide::file`), the lab's `architect.d.ts` and `architect.d.luau` among them (no more `write_types` call); the manager fills a workspace pane, its list and item page scrolling on their own, every item listed, and a narrow pane shows the list then the item. `CheckReport` problems may now have the stage `page`; `ManagerPage` takes `fill` and `narrow`.
