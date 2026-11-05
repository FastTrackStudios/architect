//! Toolbars: rows of buttons, separators, groups and widgets, each button
//! an action (`docs/content/architecture/extensions.md`, "Toolbars").
//!
//! A toolbar is a document: an extension's manifest ships it
//! (`[[toolbars]]`), a person's file (`toolbars/<name>.toml` in the app's
//! folder, [`ToolbarDecl::parse`]) adds one or overrides one of the same
//! key (`<extension>/<id>`). One flat item type ([`ToolItem`], its `kind`
//! a button by default) and one menu-row type ([`MenuItem`]) are the whole
//! format, so a visual editor reads and writes it as it is.
//!
//! ```toml
//! id = "transport"
//! title = "Transport"
//! where = "dock:right"          # dock:left|right|bottom, float, window, pane, view:<kind>, typing
//! when = "view:lab.board/chart" # the context it is for (a `typing` toolbar's: where the focus is)
//!
//! [[items]]                     # a button: an action, a label and/or an icon
//! action = "FTS_METRONOME_CLICK"
//! icon = "timer"
//! label = "Click"
//! right_click = "arm"           # menu, arm, none, or another action's id
//! colors = { on_fill = "accent-soft", on_ink = "accent" }
//!
//! [[items]]
//! kind = "separator"
//!
//! [[items]]                     # buttons drawn as one, with a label
//! kind = "group"
//! label = "Loop"
//! items = [{ action = "A" }, { action = "B" }]
//!
//! [[items]]                     # a dropdown: no action, a menu
//! label = "More"
//! menu = [{ kind = "heading", label = "Count" }, { action = "C" },
//!         { kind = "submenu", label = "Bars", items = [{ action = "D" }] }]
//!
//! [[items]]                     # a live control: an extension's view
//! kind = "widget"
//! view = "meter"                # its own view, or <extension>/<view>
//! width = 120
//! newline = true                # starts a new row
//!
//! [[items]]                     # an image file for its icon
//! action = "E"
//! image = "icons/star.svg"      # beside the file (a person's), or in the package
//!
//! [style]                       # how it is drawn, all optional
//! size = 28                     # a button's height, points
//! spacing = 4                   # between items
//! rounding = 6                  # a button's corners
//! preset = "accent"             # colours every button starts from (PRESETS)
//! ```
//!
//! The model is pure data, wasm-clean; `crate::toolbar_edit` edits it by
//! item ([`ToolItem`] at a place, `3` or `2.1`).

use serde::{Deserialize, Serialize};

/// The Typing Toolbar's place (`where = "typing"`): a phone's bar over the
/// keyboard.
pub const TYPING: &str = "typing";

/// The docks a toolbar can be in (`where = "dock:<side>"`).
pub const DOCKS: [&str; 3] = ["left", "right", "bottom"];

/// What a right-click on a button does when it says nothing: nothing.
pub const RIGHT_CLICKS: [&str; 3] = ["menu", "arm", "none"];

/// The context-menu targets toolbars own (`[[context]] target = …`):
/// a toolbar's empty space (`toolbar`, its key the id) and an item while
/// editing (`toolbar-item`, `<key>#<place>` the id: `user/mine#2.1`).
pub const CONTEXT_TARGETS: [(&str, &str); 2] = [
    (
        "toolbar",
        "a toolbar's empty space; its id the toolbar's key",
    ),
    (
        "toolbar-item",
        "an item of a toolbar being edited; its id `<key>#<place>` (`user/mine#2.1`)",
    ),
];

/// A toolbar's colour presets (`style.preset`): what every button's
/// colours start from, each a theme token. `theme` is the theme's own.
pub const PRESETS: [&str; 5] = ["theme", "accent", "soft", "contrast", "danger"];

/// The colours preset `name` gives (none for `theme` or one there is not).
#[must_use]
pub fn preset(name: &str) -> ToolColors {
    let c = |fill: &str, ink: &str, on_fill: &str, on_ink: &str| ToolColors {
        fill: fill.to_owned(),
        ink: ink.to_owned(),
        on_fill: on_fill.to_owned(),
        on_ink: on_ink.to_owned(),
        ..ToolColors::default()
    };
    match name {
        "accent" => c(
            "primary",
            "primary-foreground",
            "accent",
            "accent-foreground",
        ),
        "soft" => c(
            "secondary",
            "secondary-foreground",
            "primary",
            "primary-foreground",
        ),
        "contrast" => c("foreground", "background", "primary", "primary-foreground"),
        "danger" => c(
            "destructive",
            "destructive-foreground",
            "foreground",
            "background",
        ),
        _ => ToolColors::default(),
    }
}

/// The image files an item's `image` may be, by extension.
pub const IMAGE_TYPES: [(&str, &str); 6] = [
    ("png", "image/png"),
    ("svg", "image/svg+xml"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];

/// The media type of image file `path`, by its extension.
#[must_use]
pub fn image_type(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    IMAGE_TYPES
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, mime)| *mime)
}

/// Where a toolbar goes (its `where`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolbarPlace {
    /// A dock's (`dock:left`, `dock:right`, `dock:bottom`): a pane of
    /// its own there, opened when it loads.
    Dock(String),
    /// Floating over the main window (`float`), a pane of its own.
    Float,
    /// A window of its own (`window`), as a popped-out pane is.
    Window,
    /// Every pane's header (`pane`).
    Pane,
    /// The headers of one view kind's panes (`view:<kind>`, the
    /// workspace's kind: `note`, `lab.board/board`).
    View(String),
    /// Nowhere (`hidden`): kept, not shown (another shows in its place).
    Hidden,
    /// A phone's Typing Toolbar (`typing`), over the keyboard while a field
    /// is typed in: shown while its `when` is in force (a page's or view's
    /// kind the focus is in, the focused editor's context), after the
    /// editor's own items.
    Typing,
}

impl ToolbarPlace {
    /// Whether it is a pane of its own (a dock's, a float, a window).
    #[must_use]
    pub const fn is_pane(&self) -> bool {
        matches!(self, Self::Dock(_) | Self::Float | Self::Window)
    }
}

/// A toolbar.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ToolbarDecl {
    /// Its id: its extension's own (`transport`), or in a person's file
    /// the key of the one it overrides (`fts.metronome/transport`) or a
    /// name of its own.
    pub id: String,
    /// Its name: a pane toolbar's tab, a header toolbar's label for a
    /// screen reader.
    pub title: String,
    /// `dock:left`, `dock:right`, `dock:bottom`, `float`, `window`,
    /// `pane`, `view:<kind>`, `typing` or `hidden`.
    #[serde(rename = "where")]
    pub at: String,
    /// Every button's label hidden (an icon alone where it has one).
    #[serde(skip_serializing_if = "is_false")]
    pub hide_labels: bool,
    /// The context it is for: a view kind (`view:note`), an editor's
    /// context (`editor:markdown`); empty, every one ([`Self::applies_to`]).
    /// A place that follows what has the focus shows the toolbars whose
    /// context it is in.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub when: String,
    /// How it is drawn: sizes, spacing, rounding, a colour preset.
    #[serde(skip_serializing_if = "ToolbarStyle::is_empty")]
    pub style: ToolbarStyle,
    pub items: Vec<ToolItem>,
}

/// How a toolbar is drawn; unset, the app's own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ToolbarStyle {
    /// A button's height (and an icon button's width), points:
    /// [`SIZES`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    /// The space between items, points (`0..=24`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<u32>,
    /// A button's corner radius, points (`0..=20`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounding: Option<u32>,
    /// What every button's colours start from: one of [`PRESETS`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub preset: String,
}

/// The button sizes a toolbar may have, points.
pub const SIZES: std::ops::RangeInclusive<u32> = 16..=64;

impl ToolbarStyle {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Checks it: sizes in range, a preset there is.
    ///
    /// # Errors
    ///
    /// The first thing it gets wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.size.is_some_and(|s| !SIZES.contains(&s)) {
            return Err(format!(
                "style.size is {}..={} points",
                SIZES.start(),
                SIZES.end()
            ));
        }
        if self.spacing.is_some_and(|s| s > 24) {
            return Err("style.spacing is 0..=24 points".to_owned());
        }
        if self.rounding.is_some_and(|s| s > 20) {
            return Err("style.rounding is 0..=20 points".to_owned());
        }
        if !self.preset.is_empty() && !PRESETS.contains(&self.preset.as_str()) {
            return Err(format!(
                "style.preset {:?} is not one ({})",
                self.preset,
                PRESETS.join(", ")
            ));
        }
        Ok(())
    }
}

/// What an item is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// A button: runs its `action`, or opens its `menu` (a dropdown).
    #[default]
    Button,
    /// A thin rule between items.
    Separator,
    /// Its `items` (buttons) drawn as one, with an optional `label`.
    Group,
    /// A live control: an extension's `view`, `width` × `height` points.
    Widget,
}

/// One item of a toolbar: a button, a separator, a group or a widget
/// (`kind`). One flat type, its fields read by kind, so an editor writes
/// any item the same way.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ToolItem {
    #[serde(skip_serializing_if = "is_button")]
    pub kind: ToolKind,
    /// A button's action; none: its `menu` opens on a press.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub action: String,
    /// What it says (a group's name); a button's action's name when unset.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// A Lucide icon's name (`timer`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// An image file drawn as its icon, in place of `icon` (a PNG, SVG,
    /// JPEG, GIF or WebP: [`IMAGE_TYPES`]): absolute, or relative to the
    /// person's toolbar folder (an extension's: its package).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub image: String,
    /// Its label hidden (its icon alone, the label its tooltip).
    #[serde(skip_serializing_if = "is_false")]
    pub hide_label: bool,
    /// Its colours: theme tokens (`accent`) or CSS colours.
    #[serde(skip_serializing_if = "ToolColors::is_empty")]
    pub colors: ToolColors,
    /// What a right-click does: `menu` (its `menu`), `arm` (arms its
    /// action), `none`, or another action's id; nothing when unset.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub right_click: String,
    /// A dropdown's rows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub menu: Vec<MenuItem>,
    /// A group's buttons.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Self>,
    /// A widget's view: one of its extension's (`meter`), or another's
    /// (`<extension>/<view>`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// It starts a new row (a multi-row toolbar).
    #[serde(skip_serializing_if = "is_false")]
    pub newline: bool,
}

/// A button's colours, each a theme token's name (`accent`, `card`) or a
/// CSS colour (`#e11d48`); unset, the theme's own. `on_*` while its action
/// is on, `armed_*` while it is armed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ToolColors {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fill: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ink: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub on_fill: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub on_ink: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub armed_fill: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub armed_ink: String,
}

impl ToolColors {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Each of its colours, or `under`'s where it sets none (a button's
    /// over its toolbar's preset).
    #[must_use]
    pub fn over(&self, under: &Self) -> Self {
        let pick = |a: &String, b: &String| if a.is_empty() { b.clone() } else { a.clone() };
        Self {
            fill: pick(&self.fill, &under.fill),
            ink: pick(&self.ink, &under.ink),
            on_fill: pick(&self.on_fill, &under.on_fill),
            on_ink: pick(&self.on_ink, &under.on_ink),
            armed_fill: pick(&self.armed_fill, &under.armed_fill),
            armed_ink: pick(&self.armed_ink, &under.armed_ink),
        }
    }
}

/// What a dropdown's row is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuKind {
    /// Runs its action; ticked while a toggle is on.
    #[default]
    Action,
    /// A title over the rows after it.
    Heading,
    Separator,
    /// Its `items`, one level in.
    Submenu,
}

/// A row of a dropdown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct MenuItem {
    #[serde(skip_serializing_if = "is_action")]
    pub kind: MenuKind,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub action: String,
    /// What it says; an action's name when unset.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Self>,
}

// `skip_serializing_if` takes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(b: &bool) -> bool {
    !*b
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_button(k: &ToolKind) -> bool {
    *k == ToolKind::Button
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_action(k: &MenuKind) -> bool {
    *k == MenuKind::Action
}

/// How deep a dropdown's submenus go.
pub const MAX_MENU_DEPTH: usize = 4;

impl ToolbarDecl {
    /// A person's toolbar file (`toolbars/<name>.toml`).
    ///
    /// # Errors
    ///
    /// What is wrong with its TOML.
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// It as a person's toolbar file.
    ///
    /// # Errors
    ///
    /// It cannot be written as TOML (it always can).
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }

    /// Where it goes, when its `where` is a place there is.
    #[must_use]
    pub fn place(&self) -> Option<ToolbarPlace> {
        match self.at.as_str() {
            "pane" => return Some(ToolbarPlace::Pane),
            "hidden" => return Some(ToolbarPlace::Hidden),
            "float" => return Some(ToolbarPlace::Float),
            "window" => return Some(ToolbarPlace::Window),
            TYPING => return Some(ToolbarPlace::Typing),
            _ => {}
        }
        if let Some(side) = self.at.strip_prefix("dock:") {
            return DOCKS
                .contains(&side)
                .then(|| ToolbarPlace::Dock(side.to_owned()));
        }
        self.at
            .strip_prefix("view:")
            .filter(|kind| !kind.is_empty())
            .map(|kind| ToolbarPlace::View(kind.to_owned()))
    }

    /// Whether it is for context `context` (`view:note`, `editor:markdown`):
    /// its `when` is empty, or that context, or the kind of it (`editor`
    /// for `editor:markdown`).
    #[must_use]
    pub fn applies_to(&self, context: &str) -> bool {
        self.when.is_empty()
            || self.when == context
            || context
                .split_once(':')
                .is_some_and(|(kind, _)| kind == self.when)
    }

    /// Whether it is for any of `contexts` (the Typing Toolbar's: the
    /// focused editor's, the pages and views the focus is in).
    #[must_use]
    pub fn applies_to_any(&self, contexts: &[String]) -> bool {
        self.when.is_empty() || contexts.iter().any(|c| self.applies_to(c))
    }

    /// Whether it is in the header of a pane of view kind `kind`.
    #[must_use]
    pub fn in_header_of(&self, kind: &str) -> bool {
        match self.place() {
            Some(ToolbarPlace::Pane) => true,
            Some(ToolbarPlace::View(k)) => k == kind,
            _ => false,
        }
    }

    /// Every action it runs: its buttons', their right-clicks', their
    /// menus' (what a toggle's state is read for).
    #[must_use]
    pub fn actions(&self) -> Vec<String> {
        fn menu(rows: &[MenuItem], out: &mut Vec<String>) {
            for r in rows {
                if !r.action.is_empty() {
                    out.push(r.action.clone());
                }
                menu(&r.items, out);
            }
        }
        fn items(all: &[ToolItem], out: &mut Vec<String>) {
            for i in all {
                if !i.action.is_empty() {
                    out.push(i.action.clone());
                }
                if !i.right_click.is_empty() && !RIGHT_CLICKS.contains(&i.right_click.as_str()) {
                    out.push(i.right_click.clone());
                }
                menu(&i.menu, out);
                items(&i.items, out);
            }
        }
        let mut out = Vec::new();
        items(&self.items, &mut out);
        out
    }

    /// Checks it: an id and a title, a place there is, at least one item,
    /// each item whole for its kind (a button an action or a menu, a group
    /// buttons alone, a widget a view), every action one `action` accepts
    /// and every widget's view one `view` does.
    ///
    /// # Errors
    ///
    /// The first thing it gets wrong.
    pub fn check(
        &self,
        action: impl Fn(&str) -> Result<(), String>,
        view: impl Fn(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.items.is_empty() {
            return Err("it has no items ([[toolbars.items]])".to_owned());
        }
        self.check_draft(action, view)
    }

    /// [`Self::check`], an empty toolbar allowed: a person's, built from
    /// nothing in the editor.
    ///
    /// # Errors
    ///
    /// The first thing it gets wrong.
    pub fn check_draft(
        &self,
        action: impl Fn(&str) -> Result<(), String>,
        view: impl Fn(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.id.is_empty() || self.title.is_empty() {
            return Err("it needs an id and a title".to_owned());
        }
        if self.place().is_none() {
            return Err(format!(
                "{:?} is not a place (dock:{}, float, window, pane, view:<kind>, typing, hidden)",
                self.at,
                DOCKS.join(" | dock:")
            ));
        }
        self.style.check()?;
        if !self.when.is_empty() && !good_context(&self.when) {
            return Err(format!(
                "when {:?} is not a context (`view:note`, `editor:markdown`, `editor`)",
                self.when
            ));
        }
        for (n, item) in self.items.iter().enumerate() {
            check_item(item, false, &action, &view).map_err(|why| format!("item {n}: {why}"))?;
        }
        Ok(())
    }
}

/// A context's name: a kind (lower case words joined by `-`), or
/// `<kind>:<which>`.
fn good_context(c: &str) -> bool {
    let (kind, which) = c.split_once(':').map_or((c, None), |(k, w)| (k, Some(w)));
    !kind.is_empty()
        && kind
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        && which.is_none_or(|w| !w.is_empty())
}

fn check_item(
    item: &ToolItem,
    in_group: bool,
    action: &impl Fn(&str) -> Result<(), String>,
    view: &impl Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    match item.kind {
        ToolKind::Button => {
            if item.action.is_empty() && item.menu.is_empty() {
                return Err("a button runs an action or opens a menu".to_owned());
            }
            if !item.action.is_empty() {
                action(&item.action)?;
            }
            if item.action.is_empty()
                && item.label.is_empty()
                && item.icon.is_empty()
                && item.image.is_empty()
            {
                return Err("a dropdown needs a label or an icon".to_owned());
            }
            match item.right_click.as_str() {
                "menu" if item.menu.is_empty() => {
                    return Err("right_click = \"menu\" with no menu".to_owned());
                }
                "arm" if item.action.is_empty() => {
                    return Err("right_click = \"arm\" with no action to arm".to_owned());
                }
                "" | "none" | "menu" | "arm" => {}
                other => action(other)?,
            }
            check_menu(&item.menu, 0, action)?;
            if !item.image.is_empty() && image_type(&item.image).is_none() {
                return Err(format!(
                    "image {:?} is not an image file ({})",
                    item.image,
                    IMAGE_TYPES.map(|(e, _)| e).join(", ")
                ));
            }
            if !item.items.is_empty() || !item.view.is_empty() {
                return Err("a button has no items or view".to_owned());
            }
        }
        ToolKind::Separator => {}
        ToolKind::Group => {
            if in_group {
                return Err("a group inside a group".to_owned());
            }
            if item.items.is_empty() {
                return Err("an empty group".to_owned());
            }
            for (n, i) in item.items.iter().enumerate() {
                if i.kind != ToolKind::Button {
                    return Err(format!("its item {n}: a group holds buttons"));
                }
                check_item(i, true, action, view).map_err(|why| format!("its item {n}: {why}"))?;
            }
        }
        ToolKind::Widget => {
            if item.view.is_empty() {
                return Err("a widget names its view".to_owned());
            }
            view(&item.view)?;
        }
    }
    Ok(())
}

fn check_menu(
    rows: &[MenuItem],
    depth: usize,
    action: &impl Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    if depth > MAX_MENU_DEPTH {
        return Err(format!("submenus deeper than {MAX_MENU_DEPTH}"));
    }
    for r in rows {
        match r.kind {
            MenuKind::Action => {
                if r.action.is_empty() {
                    return Err("a menu row names no action".to_owned());
                }
                action(&r.action)?;
            }
            MenuKind::Heading if r.label.is_empty() => {
                return Err("a heading with no label".to_owned());
            }
            MenuKind::Submenu => {
                if r.label.is_empty() || r.items.is_empty() {
                    return Err("a submenu needs a label and rows".to_owned());
                }
                check_menu(&r.items, depth.saturating_add(1), action)?;
            }
            MenuKind::Heading | MenuKind::Separator => {}
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const FILE: &str = r##"
id = "fts.metronome/transport"
title = "Transport"
where = "float"

[[items]]
action = "A_PLAY"
icon = "play"
right_click = "arm"
colors = { on_fill = "accent-soft", armed_ink = "#e11d48" }

[[items]]
kind = "separator"

[[items]]
kind = "group"
label = "Loop"
items = [{ action = "A_LOOP" }, { action = "A_SET", newline = true }]

[[items]]
label = "More"
menu = [
    { kind = "heading", label = "Count" },
    { action = "A_COUNT" },
    { kind = "separator" },
    { kind = "submenu", label = "Bars", items = [{ action = "A_BARS" }] },
]

[[items]]
kind = "widget"
view = "lab.meter/meter"
width = 120
newline = true
"##;

    // The shape `check` takes.
    #[allow(clippy::unnecessary_wraps)]
    fn ok(_: &str) -> Result<(), String> {
        Ok(())
    }

    #[test]
    fn a_file_reads_every_kind_of_item_and_writes_back_the_same() {
        let t = ToolbarDecl::parse(FILE).unwrap();
        assert_eq!(t.place(), Some(ToolbarPlace::Float));
        assert_eq!(t.items.len(), 5);
        assert_eq!(t.items[1].kind, ToolKind::Separator);
        assert_eq!(t.items[2].items[1].action, "A_SET");
        assert_eq!(t.items[3].menu[3].items[0].action, "A_BARS");
        assert_eq!(t.items[4].width, Some(120));
        assert_eq!(
            t.actions(),
            ["A_PLAY", "A_LOOP", "A_SET", "A_COUNT", "A_BARS"]
        );
        assert!(t.check(ok, ok).is_ok());
        // An editor writes what it read.
        let again = ToolbarDecl::parse(&t.to_toml().unwrap()).unwrap();
        assert_eq!(again, t);
    }

    #[test]
    fn a_draft_may_be_empty_and_styled_and_a_preset_is_under_a_buttons_colours() {
        let mut t = ToolbarDecl {
            id: "mine".into(),
            title: "Mine".into(),
            at: "hidden".into(),
            ..ToolbarDecl::default()
        };
        assert_eq!(t.place(), Some(ToolbarPlace::Hidden));
        assert!(t.check(ok, ok).unwrap_err().contains("no items"));
        assert!(t.check_draft(ok, ok).is_ok());
        t.style = ToolbarStyle {
            size: Some(32),
            rounding: Some(10),
            preset: "accent".into(),
            ..ToolbarStyle::default()
        };
        t.items.push(ToolItem {
            action: "A".into(),
            image: "icons/Star.SVG".into(),
            colors: ToolColors {
                fill: "#e11d48".into(),
                ..ToolColors::default()
            },
            ..ToolItem::default()
        });
        assert!(t.check(ok, ok).is_ok());
        assert_eq!(image_type(&t.items[0].image), Some("image/svg+xml"));
        let drawn = t.items[0].colors.over(&preset(&t.style.preset));
        assert_eq!(
            (drawn.fill.as_str(), drawn.ink.as_str()),
            ("#e11d48", "primary-foreground")
        );
        assert!(preset("theme").is_empty());
        // Scoped to a context: its kind, or one of it.
        assert!(t.applies_to("view:note"));
        t.when = "editor".into();
        assert!(t.applies_to("editor:markdown") && !t.applies_to("view:note"));
        t.when = "editor:markdown".into();
        assert!(t.applies_to("editor:markdown") && !t.applies_to("editor:code"));
        assert!(t.check(ok, ok).is_ok());
        t.when = "Editor:".into();
        assert!(t.check(ok, ok).unwrap_err().contains("not a context"));
        t.when = "editor:markdown".into();
        let again = ToolbarDecl::parse(&t.to_toml().unwrap()).unwrap();
        assert_eq!(again, t);
    }

    #[test]
    fn a_toolbar_is_somewhere_there_is() {
        let at = |w: &str| {
            ToolbarDecl {
                at: w.into(),
                ..ToolbarDecl::default()
            }
            .place()
        };
        assert_eq!(at("dock:bottom"), Some(ToolbarPlace::Dock("bottom".into())));
        assert_eq!(at("pane"), Some(ToolbarPlace::Pane));
        assert_eq!(at("window"), Some(ToolbarPlace::Window));
        assert_eq!(at("view:note"), Some(ToolbarPlace::View("note".into())));
        assert_eq!(at("typing"), Some(ToolbarPlace::Typing));
        assert!(!ToolbarPlace::Typing.is_pane());
        for bad in ["dock:top", "view:", "header", ""] {
            assert_eq!(at(bad), None, "{bad}");
        }
        let header = ToolbarDecl {
            at: "view:note".into(),
            ..ToolbarDecl::default()
        };
        assert!(header.in_header_of("note") && !header.in_header_of("files"));
        // The Typing Toolbar's: for the contexts in force.
        let typing = ToolbarDecl {
            at: TYPING.into(),
            when: "view:lab.board/chart".into(),
            ..ToolbarDecl::default()
        };
        assert!(!typing.in_header_of("note"));
        let chords = ["view:lab.board/chart".to_owned()];
        let capture = ["editor:capture".to_owned()];
        assert!(typing.applies_to_any(&chords) && !typing.applies_to_any(&capture));
        assert!(!typing.applies_to_any(&[]), "no focus, not for it");
        let every = ToolbarDecl {
            when: String::new(),
            ..typing
        };
        assert!(every.applies_to_any(&capture) && every.applies_to_any(&[]));
    }

    #[test]
    fn a_bad_item_says_which_and_why() {
        let base = ToolbarDecl::parse(FILE).unwrap();
        let bad = |edit: &dyn Fn(&mut ToolbarDecl)| {
            let mut t = base.clone();
            edit(&mut t);
            t.check(ok, ok).unwrap_err()
        };
        assert!(bad(&|t| t.items.clear()).contains("no items"));
        assert!(bad(&|t| t.at = "dock:top".into()).contains("not a place"));
        assert!(bad(&|t| t.items[0].action.clear()).contains("item 0: a button runs"));
        assert!(bad(&|t| t.items[2].items[0].kind = ToolKind::Widget).contains("holds buttons"));
        assert!(bad(&|t| t.items[3].menu[0].label.clear()).contains("heading with no label"));
        assert!(bad(&|t| t.items[4].view.clear()).contains("names its view"));
        assert!(bad(&|t| t.items[3].right_click = "arm".into()).contains("no action to arm"));
        assert!(bad(&|t| t.items[0].image = "star.txt".into()).contains("not an image file"));
        assert!(bad(&|t| t.style.size = Some(200)).contains("style.size"));
        assert!(bad(&|t| t.style.preset = "neon".into()).contains("style.preset"));
        // An action the owner may not run is refused by its rule.
        let t = base.clone();
        let refuse = |id: &str| {
            if id == "A_BARS" {
                Err(format!("{id} is not yours"))
            } else {
                Ok(())
            }
        };
        assert_eq!(
            t.check(refuse, ok).unwrap_err(),
            "item 3: A_BARS is not yours"
        );
    }
}
