---
"architect" = "minor"
"architect-permissions" = "major"
"architect-extensions" = "major"
"architect-settings" = "minor"
"architect-workspace" = "minor"
"architect-a2ui" = "minor"
"architect-files" = "minor"
"architect-spaces" = "minor"
"architect-desktop" = "minor"
"architect-app" = "minor"
"architect-sandbox-clicker" = "none"
---

Added the host service, consent and the extensions manager (#178, #181, #239 X5). architect-permissions: `Principal::Extension { id, on_behalf_of }` (`Principal::extension`, `acting_for`, `extension_id`); `RoleEngine` gives it at most the rules of `on_behalf_of`. architect: an extension acting for the app itself is not asked to confirm; one acting for an agent asks as the agent would. architect-extensions: the host service (`hostcall::HostRequest` / `HostAnswer`, `hostapi::ExtensionApi` from `Host::api`, `AppContext`, `SettingValues`, `MemoryValues`, `Host::with_app_context`, `with_setting_values`, `with_data_root`, `set_app_list`, `capabilities`, `capabilities_of`; over a process's connection `host_proto::ExtensionHost` and `guest::host`; Luau's `host` global; `linked::Handle::host`), with `perm.*` on each call's span; `Capabilities::reads` and `capabilities` (`asks`, `grows_from`, `within`, `may_run`); consent (`consent::Grants`, `Scope`, `Host::with_grants`); logs (`logs::Logs`, `Host::logs`); `manage` (`Extensions::preview`, `install_with`, `update`, `pin`, `versions`, `Preview`, `Installed`); actions `EXT_PREVIEW`, `EXT_INFO`, `EXT_UPDATE`, `EXT_PIN`, `EXT_GRANT`, `EXT_REVOKE`, `EXT_GRANTS`, `EXT_LOGS`, `EXT_HOST_CALL`, and `EXT_INSTALL`'s `grant` and `pin` (`Extensions::confirm_on`); `browse::Browse`; the extensions manager (feature `manager`: `manager::Manager`, `ItemKind`, `Item`, `Details`, `Op`, the kinds `ExtensionKind`, `ScriptKind`, `WorkerKind`, `ThemeKind`, `LayoutKind`, and `ManagerExtension` with `MANAGER_KINDS`, `MANAGER_LIST`, `MANAGER_GET`, `MANAGER_RUN`); the page (feature `page`: `ManagerPage`, `ManagerView`, `use_manager_page`, `ConsentPrompt`, `MissingExtensions`). architect-settings: `use_section_page`. architect-workspace: layout and `keys.json` problems in `APP_STATUS` (`Home::app_problems`).

Migration: match `Principal::Extension` where you match every principal. A runner's `Context` has `api` and `logs`. A manifest naming an unknown `capabilities.reads` is refused. An app that keeps consent (`Host::with_grants`) runs an installed extension with only what a person granted: grant what existing installs ask (`EXT_GRANT`). `HOME_PROBLEMS` answers `{source, file, field, message}` with the file's full path.
