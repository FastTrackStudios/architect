---
"architect-app" = "minor"
"architect-extensions" = "minor"
"architect-navigator" = "minor"
"architect-navigator-extension" = "minor"
"architect-settings" = "minor"
---

Added Navigator 6's settings and API (#339): a synced setting can be kept on one device (architect_settings::ScopedValues::set_local, scope_of, is_local, Values::local, SettingsHandle::set_local, a toggle on its row); the navigator's section is synced (the Architect app keeps the synced scope in settings-synced.json and moves the navigator values it kept per device there once), has its own zoom (NavigatorSettings::zoom, Navigator::zoom) and a vault profile editor on the settings page (ui::profiles_page, NavigatorState::edit_profile_in, Profiles::find_mut); extensions raise their own events (HostRequest::Raise, Host::raise_own), and Navigator raises navigator.nav-item-changed, selection-changed, pinned-files-changed and metadata-changed; NAVIGATOR_METADATA reads a target's icon, colour, background and pins
