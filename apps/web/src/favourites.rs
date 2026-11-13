//! The note types' favourites in a browser (#607): the node's listing
//! read, and what each declared target opens in the web app.
//!
//! The node lists its extensions' `[[favourites]]` at [`LISTING`]
//! (`apps/desktop/src/node/shares.rs` `FAVOURITES_LISTING`); the app
//! believes a declaration only whole and only when it keeps the
//! manifest's rules ([`listing`]). The shell turns each into its bar
//! (`architect_shell::declare`), each target read by [`opens`]: a tab the
//! web app cannot open is left out, never shown dead.

use std::collections::BTreeSet;

use architect_favourites::{Target, TypeFavourites, place_path};
use serde_json::Value;

/// Where the node lists the note types' favourites.
pub const LISTING: &str = "/_ext/favourites.json";

/// The most declarations the app reads from a listing.
pub const MAX_DECLARATIONS: usize = 64;

/// The declarations of the node's listing this app believes: each read
/// alone, one of the wrong shape or that breaks the manifest's rules
/// (`TypeFavourites::check`) left out, never trusted in part.
#[must_use]
pub fn listing(v: &Value) -> Vec<TypeFavourites> {
    v.get("favourites")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_DECLARATIONS)
                .filter_map(|d| serde_json::from_value::<TypeFavourites>(d.clone()).ok())
                .filter(|d| d.check().is_ok())
                .collect()
        })
        .unwrap_or_default()
}

/// What a declared favourite opens in the web app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opens {
    /// The open note itself (`page`).
    Note,
    /// Quick open (`search`).
    QuickOpen,
    /// The navigator (`browse`).
    Files,
    /// An island's view in a frame of its own, on the open note.
    View { island: String, view: String },
    /// A command of the app's, by id.
    Action(String),
    /// Another note of the vault, by its path (a `{place}`).
    Path(String),
}

/// What the web app has to open a target with: the node's islands and
/// their views (`(id, views)`), the commands it has, the open note's path,
/// and whether a path is a file of the vault.
pub struct Reach<'a> {
    pub islands: &'a [(String, Vec<String>)],
    pub actions: &'a BTreeSet<String>,
    pub note: &'a str,
    pub exists: &'a dyn Fn(&str) -> bool,
}

/// Quick open, as the Command Palette's views run it.
pub const QUICK_OPEN: &str = "COMMAND_PALETTE_OPEN_ANYTHING";

/// What `target` opens here; `None` for what the web app cannot open (a
/// share page's `player.*` views, a view no island of the node's has, a
/// command the app has not, a heading or a folder).
#[must_use]
pub fn opens(target: &Target, reach: &Reach<'_>) -> Option<Opens> {
    match target {
        Target::View(v) => match v.as_str() {
            "page" => Some(Opens::Note),
            "search" => reach
                .actions
                .contains(QUICK_OPEN)
                .then_some(Opens::QuickOpen),
            "browse" => Some(Opens::Files),
            _ => {
                // `<extension id>/<view>`, or a view of any island's (the
                // listing names no declaring extension).
                let (island, view) = match v.split_once('/') {
                    Some((i, w)) => (Some(i), w),
                    None => (None, v.as_str()),
                };
                reach
                    .islands
                    .iter()
                    .find(|(id, views)| {
                        island.is_none_or(|i| i == id) && views.iter().any(|w| w == view)
                    })
                    .map(|(id, _)| Opens::View {
                        island: id.clone(),
                        view: view.to_owned(),
                    })
            }
        },
        Target::Action(a) => reach.actions.contains(a).then(|| Opens::Action(a.clone())),
        Target::Place(p) => {
            let path = place_path(reach.note, p)?;
            let md = format!("{path}.md");
            if (reach.exists)(&path) {
                Some(Opens::Path(path))
            } else if (reach.exists)(&md) {
                Some(Opens::Path(md))
            } else {
                None
            }
        }
    }
}

/// A frame pane's state: the island, its view and the note it shows, one
/// a line ([`frame_state`] reads it back).
#[must_use]
pub fn frame_state(island: &str, view: &str, note: &str) -> String {
    format!("{island}\n{view}\n{note}")
}

/// A frame pane's island, view and note, from its state.
#[must_use]
pub fn read_frame_state(state: &str) -> Option<(&str, &str, &str)> {
    let mut parts = state.splitn(3, '\n');
    let (i, v, n) = (parts.next()?, parts.next()?, parts.next()?);
    (!i.is_empty() && !v.is_empty() && !n.is_empty()).then_some((i, v, n))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decl() -> Value {
        json!({
            "note_type": "recipe",
            "tabs": [
                {"id": "page", "label": "Recipe", "icon": "notebook-text", "target": {"view": "page"}},
                {"id": "now", "label": "Now", "icon": "play", "target": {"view": "player.now"}},
                {"id": "steps", "label": "Steps", "icon": "list", "target": {"view": "lab.cook/steps"}},
                {"id": "words", "label": "Words", "icon": "mic-vocal", "target": {"view": "lyrics"}},
                {"id": "other", "label": "Other", "icon": "list", "target": {"view": "lab.other/steps"}},
                {"id": "shop", "label": "Shopping", "icon": "list", "target": {"place": "Shopping"}},
                {"id": "go", "label": "Go", "icon": "play", "target": {"action": "COOK_GO"}}
            ]
        })
    }

    /// A declaration a manifest may make (five tabs at most).
    fn ok_decl() -> Value {
        let mut d = decl();
        d["tabs"].as_array_mut().unwrap().truncate(2);
        d
    }

    #[test]
    fn a_listing_is_believed_one_declaration_at_a_time() {
        let decl = ok_decl;
        let mut bad = decl();
        bad["note_type"] = json!("bad");
        bad["tabs"][0]["target"] = json!({"place": "https://evil.example/"});
        let mut unknown = decl();
        unknown["sneaky"] = json!(1);
        let got = listing(&json!({"favourites": [decl(), bad, unknown, "x"]}));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].note_type, "recipe");
        assert!(listing(&json!({})).is_empty());
        assert!(listing(&json!({"favourites": {}})).is_empty());
        let flood: Vec<Value> = (0..200).map(|_| decl()).collect();
        assert_eq!(
            listing(&json!({ "favourites": flood })).len(),
            MAX_DECLARATIONS
        );
    }

    #[test]
    fn each_target_opens_what_the_web_app_has_and_nothing_else() {
        let d: TypeFavourites = serde_json::from_value(decl()).unwrap();
        let islands = vec![(
            "lab.cook".to_owned(),
            vec![
                "progress".to_owned(),
                "steps".to_owned(),
                "lyrics".to_owned(),
            ],
        )];
        let actions: BTreeSet<String> = [QUICK_OPEN.to_owned()].into();
        let exists = |p: &str| p == "Food/Shopping.md";
        let reach = Reach {
            islands: &islands,
            actions: &actions,
            note: "Food/Soup.md",
            exists: &exists,
        };
        let got: Vec<Option<Opens>> = d.tabs.iter().map(|t| opens(&t.target, &reach)).collect();
        let view = |v: &str| {
            Some(Opens::View {
                island: "lab.cook".into(),
                view: v.into(),
            })
        };
        assert_eq!(
            got,
            [
                Some(Opens::Note),
                None,
                view("steps"),
                view("lyrics"),
                None,
                Some(Opens::Path("Food/Shopping.md".into())),
                None,
            ]
        );
        assert_eq!(
            opens(&Target::View("search".into()), &reach),
            Some(Opens::QuickOpen)
        );
        assert_eq!(
            opens(&Target::View("browse".into()), &reach),
            Some(Opens::Files)
        );
    }

    #[test]
    fn a_frame_panes_state_reads_back() {
        let s = frame_state("lab.cook", "lyrics", "Songs/A b.md");
        assert_eq!(
            read_frame_state(&s),
            Some(("lab.cook", "lyrics", "Songs/A b.md"))
        );
        assert_eq!(read_frame_state("Songs/A.md"), None);
        assert_eq!(read_frame_state("a\n\nb"), None);
    }
}
