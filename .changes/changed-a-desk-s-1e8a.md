---
"architect-commands" = "major"
"architect-shell" = "major"
---

Changed a desk's left side to three layouts, SidebarLayout::{Rail, One, Two}, every one available on every page: SidebarDefault and use_sidebar_default become SidebarLayout and use_sidebar_layout (a page says which it prefers; Page's sidebar takes a SidebarLayout), with Shell::sidebar_layout, Shell::set_sidebar_layout and Shell::preferred_sidebar; two columns start 480pt wide. Added architect_commands::use_sidebar_commands (Sidebar: Rail / One column / Two columns on the palette, SHELL_SIDEBAR_*). A desk's foot buttons (Theme, Settings), workspace switcher and account now always sit in the ribbon's foot, never in the sidebar. Removed the sidebar's 440pt maximum width: dragged wider it stops where the page keeps 240pt. Migration: use_sidebar_default(SidebarDefault::Collapsed) is use_sidebar_layout(SidebarLayout::Rail), Open is One.
