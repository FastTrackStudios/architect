//! A note type's favourites: the few places a shell keeps one tap away
//! while a note of that type is open, and its one primary action.
//!
//! The extension that owns a note type declares them, as data, in its
//! manifest (`[[favourites]]`, read by `architect-extensions`):
//!
//! ```toml
//! [[favourites]]
//! note_type = "setlist"
//!
//! [[favourites.tabs]]
//! id = "setlist"
//! label = "Setlist"
//! icon = "list-music"
//! target = { view = "page" }
//!
//! [[favourites.tabs]]
//! id = "charts"
//! label = "Charts"
//! icon = "file-music"
//! target = { view = "player.now" }
//!
//! [favourites.primary]
//! id = "play"
//! label = "Play"
//! icon = "play"
//! target = { action = "PLAYER_TOGGLE" }
//! state = "playing"
//! on = { label = "Pause", icon = "pause" }
//! ```
//!
//! Whoever draws a shell resolves the targets: a share page's phone shell
//! (the node) its own views and the page, the app's shell its views and
//! its Action List. Nothing here is a site's or a platform's: a target is
//! a view id, an action id, or a place relative to the open note.
//! Nothing runs: a shell that does not know a target leaves its tab out.
//!
//! A declaration names only what its own extension may reach
//! ([`Declarer`]): its own views, and actions in its own namespace or one
//! its capabilities name, never a destructive one. A shell drops any
//! other target, as it drops one it does not know.

use serde::{Deserialize, Serialize};

/// The most tabs one type declares (a phone's bottom bar holds five).
pub const MAX_TABS: usize = 5;
/// The longest label, in characters.
pub const MAX_LABEL: usize = 24;
/// The longest id, icon name or target, in bytes.
pub const MAX_NAME: usize = 96;
/// The longest note type, in characters.
pub const MAX_TYPE: usize = 64;

/// One `[[favourites]]`: the tabs and the primary action of notes of
/// `note_type`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TypeFavourites {
    /// The notes it is about, by their frontmatter `type:` (`setlist`).
    pub note_type: String,
    /// Its tabs, in the order shown.
    pub tabs: Vec<Favourite>,
    /// The round button beside them, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<Primary>,
}

/// One tab.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Favourite {
    /// Unique among the type's tabs: `a-z`, `0-9`, `-`. A shell may name
    /// the tab shown by it (a share page's `#tab=<id>`).
    pub id: String,
    /// What the tab says under its icon.
    pub label: String,
    /// A Lucide icon's name (`list-music`).
    pub icon: String,
    /// What it opens.
    pub target: Target,
}

/// What a tab or an action opens or runs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    /// A view, by id: the shell's own (`page`, `search`, `browse`,
    /// `player.now`, `player.lyrics`) or an extension's (`<extension
    /// id>/<view id>`).
    View(String),
    /// An action, by id (`PLAYER_TOGGLE`).
    Action(String),
    /// A place relative to the open note: a heading (`#songs`) or a
    /// note beside it (`Charts/`), resolved by the shell against the note.
    Place(String),
}

impl Default for Target {
    fn default() -> Self {
        Self::View(String::new())
    }
}

/// The round action button: one face, and a second it shows while a
/// state the shell knows of is on (`playing`; later `stored`, a set kept
/// on the device: Download, then Play).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Primary {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub target: Target,
    /// The state that switches it to `on`, by name; a shell that does
    /// not know it shows the first face only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Its face while `state` is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<Face>,
}

/// A face of the primary action while its state is on: what it says and
/// shows, and what it runs then (none: the same target).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Face {
    pub label: String,
    pub icon: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
}

impl TypeFavourites {
    /// Whether it is about notes of `note_type` (any case, spaces
    /// trimmed).
    #[must_use]
    pub fn is_for(&self, note_type: &str) -> bool {
        self.note_type.trim().eq_ignore_ascii_case(note_type.trim())
    }

    /// Whether a manifest may declare it: a note type, at most
    /// [`MAX_TABS`] tabs with unique ids, bounded names and labels.
    ///
    /// # Errors
    ///
    /// What is wrong, for the manifest's report.
    pub fn check(&self) -> Result<(), String> {
        let ty = self.note_type.trim();
        if ty.is_empty() || ty.chars().count() > MAX_TYPE || ty.chars().any(char::is_control) {
            return Err(format!("a note_type of 1 to {MAX_TYPE} characters"));
        }
        if self.tabs.len() > MAX_TABS {
            return Err(format!("at most {MAX_TABS} tabs"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for tab in &self.tabs {
            check_face(&tab.id, &tab.label, &tab.icon, Some(&tab.target))
                .map_err(|why| format!("tab {:?}: {why}", tab.id))?;
            if !ids.insert(tab.id.as_str()) {
                return Err(format!("two tabs with id {:?}", tab.id));
            }
        }
        if let Some(p) = &self.primary {
            check_face(&p.id, &p.label, &p.icon, Some(&p.target))
                .map_err(|why| format!("primary: {why}"))?;
            if let Some(state) = &p.state {
                if !is_id(state) {
                    return Err(format!("primary: state {state:?} is not a-z, 0-9, -"));
                }
            }
            if let Some(on) = &p.on {
                check_face("on", &on.label, &on.icon, on.target.as_ref())
                    .map_err(|why| format!("primary.on: {why}"))?;
                if p.state.is_none() {
                    return Err("primary: `on` needs the `state` that turns it on".to_owned());
                }
            }
        }
        Ok(())
    }
}

/// How much an action changes, as a manifest's `capabilities.effect` says
/// it (`architect-extensions`' `Reach`, the same three steps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    #[default]
    ReadOnly,
    Mutating,
    Destructive,
}

/// The extension that declared a set of favourites and what its
/// capabilities let it run.
///
/// The same rule as every other contribution that names an action
/// (`architect-extensions`' `Capabilities::may_run`, `host::gate`): its own
/// namespace ([`action_prefix`]), or a prefix its `capabilities.actions`
/// names up to its `capabilities.effect`. A tab's press asks no one, so a
/// destructive action is never a favourite's, even its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Declarer {
    /// Its id (`studio.setlists`).
    pub extension: String,
    /// Its `capabilities.actions`: the prefixes it may run past its own.
    pub actions: Vec<String>,
    /// Its `capabilities.effect`: the most an action it runs may change.
    pub effect: Effect,
}

impl Declarer {
    /// Whether a favourite of its may run action `id`, which does `effect`.
    #[must_use]
    pub fn may_target(&self, id: &str, effect: Effect) -> bool {
        if effect == Effect::Destructive || self.extension.is_empty() {
            return false;
        }
        id.starts_with(&action_prefix(&self.extension))
            || (self
                .actions
                .iter()
                .any(|p| p == "*" || id.starts_with(p.as_str()))
                && effect <= self.effect)
    }

    /// Whether a favourite of its may open a view of `extension`'s: its own
    /// views only.
    #[must_use]
    pub fn may_open_view_of(&self, extension: &str) -> bool {
        !self.extension.is_empty() && self.extension == extension
    }
}

/// One declaration of a node's listing (`/_ext/favourites.json`): who made
/// it and what it may run, and the declaration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Listed {
    pub by: Declarer,
    pub favourites: TypeFavourites,
}

/// The action namespace of extension `id`: its id upper-cased, each run of
/// other characters one `_` (`fts.metronome` → `FTS_METRONOME_`), as
/// `architect-extensions`' `manifest::action_prefix` makes it.
#[must_use]
pub fn action_prefix(id: &str) -> String {
    let mut out = String::with_capacity(id.len().saturating_add(1));
    for c in id.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    if !out.ends_with('_') {
        out.push('_');
    }
    out
}

/// Whether `place` is a place relative to the note.
///
/// Letters, digits, `-`, `_`, `.`, `~`, `/` and one `#` anchor; not
/// starting with `/` or `.`, no `..`, `.` or empty segment but a trailing
/// `/`. Whatever a WHATWG URL parser would normalise into another origin
/// (`\`, `%`, `:`, `//`) is refused here.
#[must_use]
pub fn place_ok(place: &str) -> bool {
    let (path, anchor) = place.split_once('#').unwrap_or((place, ""));
    let plain = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '/'))
    };
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len().saturating_sub(1);
    !place.is_empty()
        && place.len() <= MAX_NAME
        && plain(path)
        && anchor
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !path.starts_with('/')
        && !path.starts_with('.')
        && (path.is_empty()
            || segments
                .iter()
                .enumerate()
                .all(|(i, seg)| *seg != ".." && *seg != "." && (!seg.is_empty() || i == last)))
}

/// Place `place` beside the note at `note`, as the path of a file.
///
/// `Charts/Joy` or `Joy#bar-1` beside `Songs/A.md` is `Songs/Charts/Joy`
/// or `Songs/Joy`; a heading alone (`#notes`), a folder (`Charts/`) or
/// anything a manifest may not declare ([`TypeFavourites::check`]) is
/// `None`. A shell that opens a file from a tab looks for it as given,
/// then with `.md`.
#[must_use]
pub fn place_path(note: &str, place: &str) -> Option<String> {
    if !place_ok(place) {
        return None;
    }
    let path = place.split_once('#').map_or(place, |(p, _)| p);
    if path.is_empty() || path.ends_with('/') {
        return None;
    }
    Some(match note.rsplit_once('/') {
        Some((folder, _)) => format!("{folder}/{path}"),
        None => path.to_owned(),
    })
}

/// `a-z`, `0-9`, `-` (and `.` for a state such as `player.playing`).
fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_NAME
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

fn check_face(id: &str, label: &str, icon: &str, target: Option<&Target>) -> Result<(), String> {
    if !is_id(id) {
        return Err(format!("id {id:?} is not a-z, 0-9, -"));
    }
    let count = label.chars().count();
    if label.trim().is_empty() || count > MAX_LABEL || label.chars().any(char::is_control) {
        return Err(format!("a label of 1 to {MAX_LABEL} characters"));
    }
    if !is_id(icon) {
        return Err(format!("icon {icon:?} is not a Lucide name"));
    }
    if let Some(target) = target {
        let (kind, text) = match target {
            Target::View(v) => ("view", v),
            Target::Action(a) => ("action", a),
            Target::Place(p) => ("place", p),
        };
        let ok = !text.is_empty()
            && text.len() <= MAX_NAME
            && !text.chars().any(|c| c.is_control() || c.is_whitespace())
            && match target {
                Target::View(v) => v
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)),
                Target::Action(a) => a
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                // Relative only, and nothing a URL parser reads otherwise:
                // a plain path and an anchor, never another site (no `\`,
                // which a browser reads as `/`, so `\\host` would be
                // `//host`; no `%`, `:`, `?`, `@`), never above the note.
                Target::Place(p) => place_ok(p),
            };
        if !ok {
            return Err(format!("{kind} target {text:?} is not one"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETLIST: &str = r#"
note_type = "setlist"

[[tabs]]
id = "setlist"
label = "Setlist"
icon = "list-music"
target = { view = "page" }

[[tabs]]
id = "lyrics"
label = "Lyrics"
icon = "mic-vocal"
target = { view = "player.lyrics" }

[primary]
id = "play"
label = "Play"
icon = "play"
target = { action = "PLAYER_TOGGLE" }
state = "playing"
on = { label = "Pause", icon = "pause" }
"#;

    #[test]
    fn a_declaration_reads_and_checks() {
        let f: TypeFavourites = toml::from_str(SETLIST).unwrap();
        assert!(f.is_for(" Setlist "));
        assert_eq!(f.tabs.len(), 2);
        assert_eq!(f.tabs[1].target, Target::View("player.lyrics".into()));
        let p = f.primary.as_ref().unwrap();
        assert_eq!(p.state.as_deref(), Some("playing"));
        assert_eq!(p.on.as_ref().map(|o| o.icon.as_str()), Some("pause"));
        assert_eq!(f.check(), Ok(()));
    }

    #[test]
    fn what_a_manifest_may_not_declare() {
        let ok: TypeFavourites = toml::from_str(SETLIST).unwrap();
        let bad = |edit: &dyn Fn(&mut TypeFavourites)| {
            let mut f = ok.clone();
            edit(&mut f);
            f.check().unwrap_err()
        };
        bad(&|f| f.note_type = " ".into());
        bad(&|f| f.note_type = "x".repeat(MAX_TYPE + 1));
        bad(&|f| f.note_type = "set\nlist".into());
        bad(&|f| f.tabs[1].id = "setlist".into());
        bad(&|f| f.tabs[0].id = "Set list".into());
        bad(&|f| f.tabs[0].label = "x".repeat(MAX_LABEL + 1));
        bad(&|f| f.tabs[0].icon = "<svg>".into());
        bad(&|f| f.tabs[0].target = Target::Place("https://evil.example/".into()));
        bad(&|f| f.tabs[0].target = Target::Place("../../secret".into()));
        // What a browser would read as another site, or a path it rewrites.
        for evil in [
            "\\evil.example/",
            "\\\\evil.example",
            "/\\evil",
            "%5cevil.example",
            "a\\b",
            "//evil.example",
            "a?b=c",
            "a\u{0}b",
            "a\tb",
            "x@evil.example",
            "./a",
            "a//b",
        ] {
            bad(&|f| f.tabs[0].target = Target::Place(evil.into()));
        }
        assert!(place_ok("#notes") && place_ok("Charts/") && place_ok("Charts/Joy#bar-1"));
        // As a file beside the note: a heading alone or a folder is none.
        assert_eq!(
            place_path("Food/Soup.md", "Charts/Joy#bar-1").as_deref(),
            Some("Food/Charts/Joy")
        );
        assert_eq!(place_path("Soup.md", "Joy").as_deref(), Some("Joy"));
        assert_eq!(place_path("Food/Soup.md", "#notes"), None);
        assert_eq!(place_path("Food/Soup.md", "Charts/"), None);
        assert_eq!(place_path("Food/Soup.md", "../secret"), None);
        bad(&|f| f.tabs[0].target = Target::Action("rm -rf".into()));
        bad(&|f| f.tabs = vec![f.tabs[0].clone(); MAX_TABS + 1]);
        bad(&|f| f.primary.as_mut().unwrap().state = None);
        // An unknown key is refused, not ignored.
        assert!(toml::from_str::<TypeFavourites>("note_type = \"x\"\nicons = 1\n").is_err());
    }

    #[test]
    fn a_declarer_names_only_its_own_actions_or_what_its_capabilities_reach() {
        let none = Declarer {
            extension: "acme.charts".into(),
            ..Declarer::default()
        };
        assert_eq!(action_prefix("acme.charts"), "ACME_CHARTS_");
        assert!(none.may_target("ACME_CHARTS_OPEN", Effect::Mutating));
        // Another's, or the app's: not without capabilities.
        assert!(!none.may_target("VAULT_DELETE", Effect::ReadOnly));
        assert!(!none.may_target("ACME_CHARTSX_OPEN", Effect::ReadOnly));
        // Never a destructive one, even its own.
        assert!(!none.may_target("ACME_CHARTS_WIPE", Effect::Destructive));
        let some = Declarer {
            extension: "acme.charts".into(),
            actions: vec!["PLAYER_".into()],
            effect: Effect::Mutating,
        };
        assert!(some.may_target("PLAYER_TOGGLE", Effect::Mutating));
        assert!(!some.may_target("PLAYER_CLEAR", Effect::Destructive));
        assert!(!some.may_target("VAULT_NEW_NOTE", Effect::ReadOnly));
        let read_only = Declarer {
            actions: vec!["*".into()],
            ..none.clone()
        };
        assert!(read_only.may_target("APP_SETTINGS", Effect::ReadOnly));
        assert!(!read_only.may_target("VAULT_NEW_NOTE", Effect::Mutating));
        // Its own views only.
        assert!(none.may_open_view_of("acme.charts"));
        assert!(!none.may_open_view_of("acme.other"));
        assert!(!Declarer::default().may_open_view_of(""));
    }
}
