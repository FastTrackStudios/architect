//! The note types' favourites in a browser (#607): the node's listing
//! read, and what each declared target opens in the web app.
//!
//! The app's own linked extensions declare theirs in its process
//! (`architect_extensions::host::Host::linked_favourites`); those come
//! first, and a node's declaration for the same extension and type is
//! left out ([`merged`]).
//!
//! The node lists its extensions' `[[favourites]]` at [`LISTING`]
//! (`apps/desktop/src/node/shares.rs` `FAVOURITES_LISTING`), each with the
//! extension that declared it and what its capabilities let it run
//! (`architect_favourites::Listed`); the app believes a declaration only
//! whole and only when it keeps the manifest's rules ([`listing`]). The
//! shell turns each into its bar (`architect_shell::declare`), each target
//! read by [`opens`]: a tab the web app cannot open, or that names what
//! its extension may not (another extension's view, an action outside its
//! reach, a destructive one), is left out, never shown dead.

use std::collections::BTreeMap;

use architect_favourites::{Declarer, Effect, Listed, Target, TypeFavourites, place_path};
use serde_json::Value;

/// Where the node lists the note types' favourites.
pub const LISTING: &str = "/_ext/favourites.json";

/// The most declarations the app reads from a listing.
pub const MAX_DECLARATIONS: usize = 64;

/// The largest listing the app reads, in bytes; a larger one is none.
pub const MAX_LISTING_BYTES: usize = 256 * 1024;

/// The declarations of the node's listing this app believes.
///
/// Each is read alone: one of the wrong shape, with no declaring
/// extension, or that breaks the manifest's rules
/// (`TypeFavourites::check`) is left out, never trusted in part.
#[must_use]
pub fn listing(v: &Value) -> Vec<Listed> {
    v.get("favourites")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_DECLARATIONS)
                .filter_map(|d| serde_json::from_value::<Listed>(d.clone()).ok())
                .filter(|d| !d.by.extension.is_empty() && d.favourites.check().is_ok())
                .collect()
        })
        .unwrap_or_default()
}

/// [`listing`] of the node's answer, `bytes`: none when it is larger than
/// [`MAX_LISTING_BYTES`] or not JSON.
#[must_use]
pub fn listing_bytes(bytes: &[u8]) -> Vec<Listed> {
    if bytes.len() > MAX_LISTING_BYTES {
        return Vec::new();
    }
    serde_json::from_slice::<Value>(bytes)
        .map(|v| listing(&v))
        .unwrap_or_default()
}

/// Every declaration the app shows from.
///
/// Its linked extensions' own (`linked`, read in its process) first, then
/// the node's listing (`node`), without what a linked extension already
/// declares for the same note type as that same extension (the code
/// compiled in is what runs; its listing on a node may be of another
/// release).
#[must_use]
pub fn merged(linked: &[Listed], node: &[Listed]) -> Vec<Listed> {
    let shadowed = |d: &Listed| {
        linked.iter().any(|l| {
            l.by.extension == d.by.extension && l.favourites.is_for(&d.favourites.note_type)
        })
    };
    linked
        .iter()
        .filter(|d| !d.by.extension.is_empty() && d.favourites.check().is_ok())
        .chain(node.iter().filter(|d| !shadowed(d)))
        .cloned()
        .collect()
}

/// The label a frame opened by `target` of `decl` is titled with: its
/// tab's, else its primary's (either face).
#[must_use]
pub fn label_for(decl: &TypeFavourites, target: &Target) -> String {
    if let Some(tab) = decl.tabs.iter().find(|f| &f.target == target) {
        return tab.label.clone();
    }
    decl.primary
        .as_ref()
        .and_then(|p| {
            if &p.target == target {
                return Some(p.label.clone());
            }
            p.on.as_ref()
                .filter(|on| on.target.as_ref() == Some(target))
                .map(|on| on.label.clone())
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
    /// A linked extension's view, in the app's own document, on the open
    /// note.
    Linked { extension: String, view: String },
    /// A command of the app's, by id.
    Action(String),
    /// Another note of the vault, by its path (a `{place}`).
    Path(String),
}

/// What the web app has to open a target with.
///
/// Its linked extensions and the views their code draws (`(id, views)`),
/// the node's islands and their views (the same), the commands it has
/// and how much each changes, the open note's path, and whether a path is
/// a file of the vault.
pub struct Reach<'a> {
    pub linked: &'a [(String, Vec<String>)],
    pub islands: &'a [(String, Vec<String>)],
    pub actions: &'a BTreeMap<String, Effect>,
    pub note: &'a str,
    pub exists: &'a dyn Fn(&str) -> bool,
}

/// Quick open, as the Command Palette's views run it.
pub const QUICK_OPEN: &str = "COMMAND_PALETTE_OPEN_ANYTHING";

/// What `target`, declared by `by`, opens here.
///
/// A view is its declaring extension's own: the one its linked code
/// draws when the extension is compiled in, else the one its island on
/// the node lists (a linked view wins where both have it).
///
/// `None` for what the web app cannot open (a share page's `player.*`
/// views, a view neither a linked extension draws nor an island of the
/// node's lists, a command the app has not, a heading or a folder) or
/// `by` may not name (another extension's view, an action outside its
/// reach or a destructive one: `Declarer`).
#[must_use]
pub fn opens(by: &Declarer, target: &Target, reach: &Reach<'_>) -> Option<Opens> {
    match target {
        // A tab's menu is drawn by the bar that shows it (a link's guest
        // page has one); the member's bar has none yet.
        Target::Menu(_) => None,
        Target::View(v) => match v.as_str() {
            "page" => Some(Opens::Note),
            "search" => reach
                .actions
                .contains_key(QUICK_OPEN)
                .then_some(Opens::QuickOpen),
            "browse" => Some(Opens::Files),
            _ => {
                // `<its own id>/<view>`, or a bare view of its own.
                let (island, view) = v.split_once('/').unwrap_or((&by.extension, v));
                if !by.may_open_view_of(island) {
                    return None;
                }
                let has = |list: &[(String, Vec<String>)]| {
                    list.iter()
                        .any(|(id, views)| id == island && views.iter().any(|w| w == view))
                };
                if has(reach.linked) {
                    return Some(Opens::Linked {
                        extension: island.to_owned(),
                        view: view.to_owned(),
                    });
                }
                reach
                    .islands
                    .iter()
                    .find(|(id, views)| id == island && views.iter().any(|w| w == view))
                    .map(|(id, _)| Opens::View {
                        island: id.clone(),
                        view: view.to_owned(),
                    })
            }
        },
        Target::Action(a) => {
            let effect = *reach.actions.get(a)?;
            by.may_target(a, effect).then(|| Opens::Action(a.clone()))
        }
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
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
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
            ],
            "primary": {"id": "timer", "label": "Timer", "icon": "timer", "target": {"view": "timer"}}
        })
    }

    fn by() -> Declarer {
        Declarer {
            extension: "lab.cook".into(),
            ..Declarer::default()
        }
    }

    /// A declaration a manifest may make (five tabs at most), as listed.
    fn ok_listed() -> Value {
        let mut d = decl();
        d["tabs"].as_array_mut().unwrap().truncate(2);
        json!({"by": {"extension": "lab.cook"}, "favourites": d})
    }

    #[test]
    fn a_listing_is_believed_one_declaration_at_a_time() {
        let mut bad = ok_listed();
        bad["favourites"]["note_type"] = json!("bad");
        bad["favourites"]["tabs"][0]["target"] = json!({"place": "https://evil.example/"});
        let mut unknown = ok_listed();
        unknown["favourites"]["sneaky"] = json!(1);
        let mut nobody = ok_listed();
        nobody["by"] = json!({});
        let bare = ok_listed()["favourites"].clone();
        let mut long = ok_listed();
        long["favourites"]["note_type"] = json!("x".repeat(architect_favourites::MAX_TYPE + 1));
        let got =
            listing(&json!({"favourites": [ok_listed(), bad, unknown, nobody, bare, long, "x"]}));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].favourites.note_type, "recipe");
        assert_eq!(got[0].by.extension, "lab.cook");
        assert!(listing(&json!({})).is_empty());
        assert!(listing(&json!({"favourites": {}})).is_empty());
        let flood: Vec<Value> = (0..200).map(|_| ok_listed()).collect();
        assert_eq!(
            listing(&json!({ "favourites": flood })).len(),
            MAX_DECLARATIONS
        );
    }

    #[test]
    fn a_listing_larger_than_the_limit_is_none() {
        let one = serde_json::to_vec(&json!({"favourites": [ok_listed()]})).unwrap();
        assert_eq!(listing_bytes(&one).len(), 1);
        // The same declaration, padded past the limit: not read at all.
        let mut padded = json!({"favourites": [ok_listed()]});
        padded["pad"] = json!("x".repeat(MAX_LISTING_BYTES));
        let big = serde_json::to_vec(&padded).unwrap();
        assert!(big.len() > MAX_LISTING_BYTES);
        assert!(listing_bytes(&big).is_empty());
        assert!(listing_bytes(b"not json").is_empty());
    }

    fn reach_with<'a>(
        islands: &'a [(String, Vec<String>)],
        actions: &'a BTreeMap<String, Effect>,
        exists: &'a dyn Fn(&str) -> bool,
    ) -> Reach<'a> {
        Reach {
            linked: &[],
            islands,
            actions,
            note: "Food/Soup.md",
            exists,
        }
    }

    fn islands() -> Vec<(String, Vec<String>)> {
        vec![
            (
                "lab.cook".to_owned(),
                vec![
                    "progress".to_owned(),
                    "steps".to_owned(),
                    "lyrics".to_owned(),
                ],
            ),
            (
                "lab.other".to_owned(),
                vec!["steps".to_owned(), "board".to_owned()],
            ),
        ]
    }

    fn actions() -> BTreeMap<String, Effect> {
        [
            (QUICK_OPEN, Effect::ReadOnly),
            ("LAB_COOK_GO", Effect::Mutating),
            ("LAB_COOK_BURN", Effect::Destructive),
            ("COOK_GO", Effect::ReadOnly),
            ("VAULT_NEW_NOTE", Effect::Mutating),
            ("APP_SETTINGS", Effect::ReadOnly),
        ]
        .into_iter()
        .map(|(a, e)| (a.to_owned(), e))
        .collect()
    }

    #[test]
    fn each_target_opens_what_the_web_app_has_and_nothing_else() {
        let d: TypeFavourites = serde_json::from_value(decl()).unwrap();
        let (islands, actions) = (islands(), actions());
        let exists = |p: &str| p == "Food/Shopping.md";
        let reach = reach_with(&islands, &actions, &exists);
        let got: Vec<Option<Opens>> = d
            .tabs
            .iter()
            .map(|t| opens(&by(), &t.target, &reach))
            .collect();
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
                // A bare view: its own island's.
                view("lyrics"),
                // Another extension's view, though the node lists it.
                None,
                Some(Opens::Path("Food/Shopping.md".into())),
                // An action of the app's outside its namespace.
                None,
            ]
        );
        assert_eq!(
            opens(&by(), &Target::View("search".into()), &reach),
            Some(Opens::QuickOpen)
        );
        assert_eq!(
            opens(&by(), &Target::View("browse".into()), &reach),
            Some(Opens::Files)
        );
        // A bare view only its own island has, never another's.
        assert_eq!(opens(&by(), &Target::View("board".into()), &reach), None);
    }

    /// A favourite's view is its extension's linked view when its code is
    /// compiled in, else its island's; never another extension's.
    #[test]
    fn a_view_target_is_the_linked_view_before_an_islands() {
        let (islands, actions) = (islands(), actions());
        let exists = |_: &str| false;
        let linked = vec![
            (
                "lab.cook".to_owned(),
                vec!["steps".to_owned(), "pantry".to_owned()],
            ),
            ("lab.other".to_owned(), vec!["board".to_owned()]),
        ];
        let reach = Reach {
            linked: &linked,
            ..reach_with(&islands, &actions, &exists)
        };
        let view = |v: &str| opens(&by(), &Target::View(v.into()), &reach);
        let own = |v: &str| {
            Some(Opens::Linked {
                extension: "lab.cook".into(),
                view: v.into(),
            })
        };
        // Both have `steps`: the linked one, by either spelling.
        assert_eq!(view("steps"), own("steps"));
        assert_eq!(view("lab.cook/steps"), own("steps"));
        // Only its linked code draws `pantry`; only its island `lyrics`.
        assert_eq!(view("pantry"), own("pantry"));
        assert_eq!(
            view("lyrics"),
            Some(Opens::View {
                island: "lab.cook".into(),
                view: "lyrics".into()
            })
        );
        // Another extension's linked view, bare or named: never.
        assert_eq!(view("board"), None);
        assert_eq!(view("lab.other/board"), None);
        // The app's own views are still the app's.
        assert_eq!(view("page"), Some(Opens::Note));
        // No linked extension at all: an island's, as before.
        let reach = reach_with(&islands, &actions, &exists);
        assert_eq!(
            opens(&by(), &Target::View("steps".into()), &reach),
            Some(Opens::View {
                island: "lab.cook".into(),
                view: "steps".into()
            })
        );
    }

    /// The app's linked extensions' declarations come first; the node's
    /// for the same extension and type is left out, others stay.
    #[test]
    fn linked_declarations_come_before_the_nodes_listing() {
        let listed = |extension: &str, ty: &str, label: &str| {
            let mut d = ok_listed();
            d["by"]["extension"] = json!(extension);
            d["favourites"]["note_type"] = json!(ty);
            d["favourites"]["tabs"][0]["label"] = json!(label);
            serde_json::from_value::<Listed>(d).unwrap()
        };
        let linked = [listed("lab.cook", "recipe", "Linked")];
        let node = [
            listed("lab.cook", "Recipe", "Node"),
            listed("lab.cook", "menu", "Node menu"),
            listed("lab.other", "recipe", "Other"),
        ];
        let got: Vec<String> = merged(&linked, &node)
            .iter()
            .map(|d| d.favourites.tabs[0].label.clone())
            .collect();
        assert_eq!(got, ["Linked", "Node menu", "Other"]);
        // With no linked extension, the node's listing as it is.
        assert_eq!(merged(&[], &node), node);
        // A linked declaration that breaks the manifest's rules is none.
        let mut bad = listed("lab.cook", "recipe", "Bad");
        bad.favourites.note_type = String::new();
        assert_eq!(merged(&[bad], &node).len(), 3);
    }

    #[test]
    fn an_action_is_one_its_extension_may_run_and_never_destructive() {
        let (islands, actions) = (islands(), actions());
        let exists = |_: &str| false;
        let reach = reach_with(&islands, &actions, &exists);
        let act = |by: &Declarer, a: &str| opens(by, &Target::Action(a.into()), &reach);
        assert_eq!(
            act(&by(), "LAB_COOK_GO"),
            Some(Opens::Action("LAB_COOK_GO".into()))
        );
        assert_eq!(act(&by(), "VAULT_NEW_NOTE"), None);
        assert_eq!(act(&by(), "APP_SETTINGS"), None);
        assert_eq!(act(&by(), "LAB_COOK_BURN"), None, "destructive");
        assert_eq!(act(&by(), "NOT_HERE"), None);
        let vault = Declarer {
            actions: vec!["VAULT_".into()],
            effect: Effect::ReadOnly,
            ..by()
        };
        assert_eq!(act(&vault, "VAULT_NEW_NOTE"), None, "past its effect");
        let vault = Declarer {
            effect: Effect::Mutating,
            ..vault
        };
        assert_eq!(
            act(&vault, "VAULT_NEW_NOTE"),
            Some(Opens::Action("VAULT_NEW_NOTE".into()))
        );
    }

    #[test]
    fn a_frame_is_titled_by_its_tab_or_its_primary() {
        let d: TypeFavourites = serde_json::from_value(decl()).unwrap();
        assert_eq!(
            label_for(&d, &Target::View("lab.cook/steps".into())),
            "Steps"
        );
        assert_eq!(label_for(&d, &Target::View("timer".into())), "Timer");
        assert_eq!(label_for(&d, &Target::View("nothing".into())), "");
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
