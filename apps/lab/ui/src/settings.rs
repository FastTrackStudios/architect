//! The lab's settings: what it registers (Appearance, Workspace, Editor,
//! Keyboard). The metronome extension adds its own section while it is
//! loaded (`extensions.rs`).

use architect_settings::{Choice, Control, Registry, Section, Setting, Value};
use architect_shell::{ShellStyle, StatusStyle};

/// Every section the lab itself declares.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    for section in [appearance(), workspace(), editor(), keyboard()] {
        let _ = r.register(section);
    }
    r
}

fn appearance() -> Section {
    Section::new("appearance", "Appearance")
        .icon("palette")
        .block(
            None,
            vec![
                Setting::new(
                    "appearance.theme",
                    "Theme",
                    Control::Cards(vec![
                        Choice::text("system", "System").detail("Follow the Mac"),
                        Choice::text("dark", "Dark"),
                        Choice::text("light", "Light"),
                    ]),
                    Value::Text("system".into()),
                )
                .describe("Follow the system, or always light or dark.")
                .aliases(["dark mode", "light mode", "colour scheme"]),
                Setting::new(
                    "appearance.text_size",
                    "Text size",
                    Control::Stepper {
                        min: 80,
                        max: 140,
                        step: 5,
                        unit: "%".into(),
                    },
                    Value::Int(100),
                )
                .describe("How large text is everywhere.")
                .aliases(["font", "zoom"]),
                Setting::new(
                    "appearance.reduce_motion",
                    "Reduce motion",
                    Control::Switch,
                    Value::Bool(false),
                )
                .describe("Drawers and menus appear without moving.")
                .aliases(["animation"]),
            ],
        )
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

fn editor() -> Section {
    Section::new("editor", "Editor").icon("type").block(
        None,
        vec![
            Setting::new(
                "editor.vim",
                "Vim keys",
                Control::Switch,
                Value::Bool(false),
            )
            .describe("Move and edit with Vim's keys.")
            .aliases(["modal", "hjkl"]),
            Setting::new(
                "editor.width",
                "Line width",
                Control::Slider {
                    min: 40.0,
                    max: 120.0,
                    step: 1.0,
                    unit: " ch".into(),
                },
                Value::Float(72.0),
            )
            .describe("How wide a line of a note runs."),
            Setting::new(
                "editor.new_title",
                "A new note's title",
                Control::Text {
                    placeholder: "Untitled".into(),
                },
                Value::Text("Untitled".into()),
            ),
        ],
    )
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
