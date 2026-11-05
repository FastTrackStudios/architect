//! The lab's settings: what it registers (Workspace, Editor, Keyboard;
//! the Vault's navigator's section is Navigator's, the built-in extension;
//! Appearance is architect's, `architect_settings::use_appearance_section`,
//! in `theme.rs`). The metronome extension adds its own section while it is
//! loaded (`extensions.rs`). Editor is a struct (`#[derive(Settings)]`);
//! the rest are written out as data.

use architect_settings::{Choice, Control, Registry, Section, Setting, Settings, Value};
use architect_shell::{ShellStyle, StatusStyle};
use facet::Facet;

/// Every section the lab itself declares.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    for section in [
        workspace(),
        Editor::section(),
        keyboard(),
    ] {
        let _ = r.register(section);
    }
    r
}

fn workspace() -> Section {
    Section::new("workspace", "Workspace")
        .icon("panel-bottom")
        .block(
            Some("Layout"),
            vec![
                Setting::new(
                    "workspace.shell",
                    "Navigation on a phone",
                    Control::Select(vec![
                        Choice::text("pill", "Floating pill"),
                        Choice::text("bar", "Tab bar"),
                        Choice::text("rail", "Rail only"),
                    ]),
                    Value::Text("pill".into()),
                )
                .describe("How the sections show at the foot of a phone."),
                Setting::new(
                    "workspace.details",
                    "Details on the right",
                    Control::Switch,
                    Value::Bool(true),
                )
                .describe("A pane with what is selected, beside the page."),
                Setting::new(
                    "workspace.status",
                    "Status line",
                    Control::Segmented(vec![
                        Choice::text("bar", "Bar"),
                        Choice::text("badge", "Badge"),
                    ]),
                    Value::Text("bar".into()),
                )
                .describe("Along the window's foot, or in its corner."),
            ],
        )
        .block(
            Some("Motion"),
            vec![
                Setting::new(
                    "appearance.reduce_motion",
                    "Reduce motion",
                    Control::Switch,
                    Value::Bool(false),
                )
                .describe(
                    "Sidebars, tabs, splits and drawers go straight to where they end. \
                     Off, the lab follows the Mac's own setting.",
                )
                .aliases(["animation"]),
            ],
        )
        .block(
            Some("Extensions"),
            vec![
                Setting::new(
                    "workspace.metronome",
                    "Load the metronome extension",
                    Control::Switch,
                    Value::Bool(false),
                )
                .describe(
                    "A real one, from the lab's registry: loaded, its settings are on this page \
                     and its action in ⌘K; save its files and it reloads.",
                ),
            ],
        )
}

/// Editor
#[derive(Facet, Settings, Clone, PartialEq)]
#[settings(icon = "type")]
pub struct Editor {
    /// Vim keys
    /// Move and edit with Vim's keys.
    #[setting(aliases = ["modal", "hjkl"])]
    pub vim: bool,
    /// Line width
    /// How wide a line of a note runs.
    #[setting(control = slider(40..=120, step = 1), unit = " ch")]
    pub width: f64,
    /// A new note's title
    #[setting(placeholder = "Untitled")]
    pub new_title: String,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            vim: false,
            width: 72.0,
            new_title: "Untitled".to_owned(),
        }
    }
}

fn keyboard() -> Section {
    Section::new("keyboard", "Keyboard").icon("keyboard").block(
        None,
        vec![
            Setting::new(
                "keyboard.palette",
                "Show every command",
                Control::Shortcut,
                Value::Text("⌘K".into()),
            )
            .describe("Opens the command palette."),
        ],
    )
}

/// A shell style's setting id.
pub const fn shell_id(s: ShellStyle) -> &'static str {
    match s {
        ShellStyle::FloatingPill => "pill",
        ShellStyle::TabBar => "bar",
        ShellStyle::RailOnly => "rail",
    }
}

pub fn shell_style(id: &str) -> ShellStyle {
    match id {
        "bar" => ShellStyle::TabBar,
        "rail" => ShellStyle::RailOnly,
        _ => ShellStyle::FloatingPill,
    }
}

pub fn status_style(id: &str) -> StatusStyle {
    if id == "badge" {
        StatusStyle::Badge
    } else {
        StatusStyle::Bar
    }
}
