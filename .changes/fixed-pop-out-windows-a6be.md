---
"architect-workspace" = "patch"
---

Fixed pop-out windows keeping the theme they opened with: a pop-out now shares the main window's `ShellTheme` signal and carries the theme store (`ThemeHandle`) and the `:root` variables (`architect_theme::RootCss`) through its `Bridge`, so every `THEME_SET` reaches it.
