//! The navigator's colours: its own theme roles (`navigator.*`), defaults
//! over architect's tokens, so they follow every theme (Notebook
//! Navigator's `--nn-theme-*` roles, `docs/theming-guide.md`).
//!
//! An app adds [`TOKENS`] to its `App` layer (`architect_theme::Layer::token`);
//! a theme file or a person overrides any of them. Without them (no store,
//! or an app that left them out) each role falls back to the plain token
//! its default is made from.

use architect_theme::{FromResolved, Resolved};

/// Every role and its default: (name, expression, the token it falls back
/// to when the app's layer lacks it).
pub const ROLES: &[(&str, &str, &str)] = &[
    // The panes' fills (`nav-bg`, `list-bg`).
    ("navigator.background", "$sheet", "sheet"),
    ("navigator.list-background", "$sheet", "sheet"),
    // A row's name, its preview, its date and folder, a count, a chevron.
    ("navigator.text", "$ink", "ink"),
    ("navigator.muted", "$ink-2", "ink-2"),
    ("navigator.faint", "$ink-3", "ink-3"),
    // Under the pointer (`navitem-hover-bg`).
    ("navigator.hover", "alpha($ink, 0.06)", "active"),
    // Selected, in the pane with the keyboard (`navitem-selected-bg`,
    // `file-selected-bg`), and in the other (`…-selected-inactive-bg`).
    ("navigator.selected", "$brand-soft", "brand-soft"),
    ("navigator.selected-inactive", "$active", "active"),
    // A group's heading in the list (`list-group-header-color`).
    ("navigator.heading", "$ink-3", "ink-3"),
    // The line between panes and rows (`divider-border-color`), and the
    // resize handle under the pointer.
    ("navigator.divider", "$rule", "rule"),
    ("navigator.handle", "$brand", "brand"),
    // The note open in the workspace, marked in the list; a task bar.
    ("navigator.open", "$brand", "brand"),
    // A tag's or a property's pill without a colour of its own, and the
    // box behind a note's image (`file-tag-bg`, `feature-image-bg`).
    ("navigator.pill", "alpha($ink, 0.07)", "active"),
    ("navigator.thumb", "alpha($ink, 0.05)", "active"),
    // The calendar: today (`calendar-day-today-bg`, `-color`), the
    // weekend's shade, a day's note dot (`calendar-note-indicator-color`)
    // and its tasks-left ring (`calendar-task-indicator-color`).
    ("navigator.calendar-today", "$brand-soft", "brand-soft"),
    ("navigator.calendar-today-text", "$brand", "brand"),
    ("navigator.calendar-weekend", "alpha($ink, 0.04)", "active"),
    ("navigator.calendar-dot", "$ink-3", "ink-3"),
    ("navigator.calendar-task", "$brand", "brand"),
    // A day's number over its feature image, and the shade under it
    // (`calendar-feature-image-overlay-color`).
    ("navigator.calendar-image-text", "#ffffff", "sheet"),
    ("navigator.calendar-image-shade", "alpha(#000000, 0.3)", "active"),
];

/// The roles as `(name, default)`, for an app's `App` layer.
pub const TOKENS: &[(&str, &str)] = &[
    ("navigator.background", "$sheet"),
    ("navigator.list-background", "$sheet"),
    ("navigator.text", "$ink"),
    ("navigator.muted", "$ink-2"),
    ("navigator.faint", "$ink-3"),
    ("navigator.hover", "alpha($ink, 0.06)"),
    ("navigator.selected", "$brand-soft"),
    ("navigator.selected-inactive", "$active"),
    ("navigator.heading", "$ink-3"),
    ("navigator.divider", "$rule"),
    ("navigator.handle", "$brand"),
    ("navigator.open", "$brand"),
    ("navigator.pill", "alpha($ink, 0.07)"),
    ("navigator.thumb", "alpha($ink, 0.05)"),
    ("navigator.calendar-today", "$brand-soft"),
    ("navigator.calendar-today-text", "$brand"),
    ("navigator.calendar-weekend", "alpha($ink, 0.04)"),
    ("navigator.calendar-dot", "$ink-3"),
    ("navigator.calendar-task", "$brand"),
    ("navigator.calendar-image-text", "#ffffff"),
    ("navigator.calendar-image-shade", "alpha(#000000, 0.3)"),
];

/// `layer` with the navigator's roles added.
#[must_use]
pub fn with_tokens(layer: architect_theme::Layer) -> architect_theme::Layer {
    TOKENS
        .iter()
        .fold(layer, |layer, (name, value)| layer.token(*name, *value))
}

/// The navigator's slice of the theme: each role as a CSS colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigatorColours(Vec<(String, String)>);

impl FromResolved for NavigatorColours {
    fn from_resolved(r: &Resolved) -> Self {
        Self(
            ROLES
                .iter()
                .map(|(name, _, fallback)| {
                    let value = if r.get(name).is_some() {
                        r.color(name)
                    } else {
                        r.color(fallback)
                    };
                    let var = name.strip_prefix("navigator.").unwrap_or(name);
                    (format!("--nv-{var}"), value.to_string())
                })
                .collect(),
        )
    }
}

impl NavigatorColours {
    /// The roles as custom properties (`--nv-text: …;`), for the root.
    #[must_use]
    pub fn vars(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.0 {
            out.push_str(k);
            out.push(':');
            out.push_str(v);
            out.push(';');
        }
        out
    }
}
