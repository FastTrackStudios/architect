//! A share link's phone shell: the tabs at the foot of a share page on a
//! phone, and its round button, as the app's own phone shell draws them.
//!
//! Which tabs: the favourites the share's root note's type declares
//! ([`architect_favourites::TypeFavourites`], an extension's
//! `[[favourites]]`), else the share's own: Home, Search, Browse. A tab
//! whose target this shell does not know is left out; so is a primary
//! action it cannot run. What it knows ([`VIEWS`], [`ACTIONS`]): the page
//! (the share's root), search, browsing the share's pages, the player's
//! now-playing view and its lyrics; and play/pause. A `place` is the root
//! page's URL with it (a heading `#…`, or a path under it).
//!
//! The page gets it as data (`<script type="application/json"
//! id="site-shell">`, [`shell_json`]): each tab's id, label, target and
//! its icon as SVG drawn here from Lucide's shapes (static markup the
//! page's own script puts in place). Nothing of an extension runs.

use std::fmt::Write as _;

use architect_favourites::{Primary, Target, TypeFavourites};
use serde_json::{Value, json};

/// The views a share page's shell opens: its root page, its search, its
/// pages (the navigator), the player's now-playing view and its lyrics.
pub const VIEWS: &[&str] = &["page", "search", "browse", "player.now", "player.lyrics"];

/// The actions it runs: the player's play/pause.
pub const ACTIONS: &[&str] = &["PLAYER_TOGGLE"];

/// The states it knows a primary action's second face by: playing.
pub const STATES: &[&str] = &["playing"];

/// Lucide icon `name` as an SVG of the page's own (stroke in the text's
/// colour); a circle for a name Lucide has not.
#[must_use]
pub fn icon_svg(name: &str) -> String {
    use architect_shell::lucide::{Glyph, glyphs};
    let mut out = String::from(
        "<svg viewBox=\"0 0 24 24\" width=\"22\" height=\"22\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">",
    );
    match glyphs(name) {
        Some(shapes) => {
            for g in shapes {
                let _ = match *g {
                    Glyph::Path(d) => write!(out, "<path d=\"{}\"/>", attr(d)),
                    Glyph::Circle(cx, cy, r) => write!(
                        out,
                        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\"/>",
                        attr(cx),
                        attr(cy),
                        attr(r)
                    ),
                    Glyph::Polyline(p) => write!(out, "<polyline points=\"{}\"/>", attr(p)),
                    Glyph::Polygon(p) => write!(out, "<polygon points=\"{}\"/>", attr(p)),
                };
            }
        }
        None => match RECTS.iter().find(|(n, _)| *n == name.trim()) {
            // Lucide's icons drawn with `<rect>`s, which its table of shapes
            // has not: the same rects as paths.
            Some((_, d)) => {
                let _ = write!(out, "<path d=\"{d}\"/>");
            }
            None => out.push_str("<circle cx=\"12\" cy=\"12\" r=\"9\"/>"),
        },
    }
    out.push_str("</svg>");
    out
}

/// Lucide icons a player's shell uses that are drawn with rects (Lucide
/// 0.4xx `pause`, `square`), as paths: Lucide's shapes table has no rects.
const RECTS: &[(&str, &str)] = &[
    (
        "pause",
        "M15 4h3a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1h-3a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM6 4h3a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z",
    ),
    (
        "square",
        "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z",
    ),
];

/// A Lucide shape's attribute value: numbers and path letters only.
fn attr(v: &str) -> String {
    v.chars()
        .filter(|c| c.is_ascii_alphanumeric() || " .,-".contains(*c))
        .collect()
}

/// `target` as the page's shell follows it, or None when it is not one it
/// knows. `home` is the share's root page (its URL, ending `/`).
fn target_json(target: &Target, home: &str) -> Option<Value> {
    match target {
        Target::View(view) => VIEWS
            .contains(&view.as_str())
            .then(|| json!({ "view": view })),
        Target::Action(action) => ACTIONS
            .contains(&action.as_str())
            .then(|| json!({ "action": action })),
        Target::Place(place) => {
            // Relative to the root page, never above it or elsewhere, and
            // nothing a URL parser reads otherwise (`\` is `/` to it):
            // `TypeFavourites::check` refused those already, checked again.
            if !architect_favourites::place_ok(place) {
                return None;
            }
            Some(json!({ "href": format!("{home}{place}") }))
        }
    }
}

fn primary_json(p: &Primary, home: &str) -> Option<Value> {
    let target = target_json(&p.target, home)?;
    let mut out = serde_json::Map::new();
    out.insert("id".to_owned(), json!(p.id));
    out.insert("label".to_owned(), json!(p.label));
    out.insert("icon".to_owned(), json!(icon_svg(&p.icon)));
    // Its second face, for a state this shell knows (and a target it can
    // run, its own when it names none).
    let second = p
        .state
        .as_deref()
        .filter(|state| STATES.contains(state))
        .zip(p.on.as_ref())
        .and_then(|(state, on)| {
            let on_target = on
                .target
                .as_ref()
                .map_or_else(|| Some(target.clone()), |t| target_json(t, home))?;
            Some((
                state,
                json!({ "label": on.label, "icon": icon_svg(&on.icon), "target": on_target }),
            ))
        });
    if let Some((state, on)) = second {
        out.insert("state".to_owned(), json!(state));
        out.insert("on".to_owned(), on);
    }
    out.insert("target".to_owned(), target);
    Some(Value::Object(out))
}

/// The share's own tabs, for a root note whose type declares none.
#[must_use]
pub fn default_favourites() -> TypeFavourites {
    let tab = |id: &str, label: &str, icon: &str, view: &str| architect_favourites::Favourite {
        id: id.to_owned(),
        label: label.to_owned(),
        icon: icon.to_owned(),
        target: Target::View(view.to_owned()),
    };
    TypeFavourites {
        note_type: String::new(),
        tabs: vec![
            tab("home", "Home", "house", "page"),
            tab("search", "Search", "search", "search"),
            tab("browse", "Browse", "folder-tree", "browse"),
        ],
        primary: None,
    }
}

/// What a share page's shell draws: the tabs of `declared` (the root
/// note's type's favourites; None: [`default_favourites`]) it knows how
/// to open, and its primary action. `home` is the share's root page.
#[must_use]
pub fn shell_json(declared: Option<&TypeFavourites>, home: &str) -> Value {
    let fallback = default_favourites();
    let decl = declared.filter(|d| d.check().is_ok()).unwrap_or(&fallback);
    let tabs: Vec<Value> = decl
        .tabs
        .iter()
        .filter_map(|tab| {
            let target = target_json(&tab.target, home)?;
            Some(json!({
                "id": tab.id,
                "label": tab.label,
                "icon": icon_svg(&tab.icon),
                "target": target,
            }))
        })
        .collect();
    json!({
        "home": home,
        "type": decl.note_type,
        "tabs": tabs,
        "primary": decl.primary.as_ref().and_then(|p| primary_json(p, home)),
    })
}

/// The favourites of `note_type` among `all` (several extensions for one
/// type: the first that declares it).
#[must_use]
pub fn favourites_for<'a>(
    all: &'a [TypeFavourites],
    note_type: &str,
) -> Option<&'a TypeFavourites> {
    if note_type.trim().is_empty() {
        return None;
    }
    all.iter()
        .find(|f| f.is_for(note_type) && f.check().is_ok())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn setlist() -> TypeFavourites {
        let text = r##"
note_type = "setlist"
[[tabs]]
id = "setlist"
label = "Setlist"
icon = "list-music"
target = { view = "page" }
[[tabs]]
id = "charts"
label = "Charts"
icon = "file-music"
target = { view = "player.now" }
[[tabs]]
id = "elsewhere"
label = "Elsewhere"
icon = "globe"
target = { view = "someone.else/view" }
[[tabs]]
id = "notes"
label = "Notes"
icon = "notebook"
target = { place = "#notes" }
[primary]
id = "play"
label = "Play"
icon = "play"
target = { action = "PLAYER_TOGGLE" }
state = "playing"
on = { label = "Pause", icon = "pause" }
"##;
        toml::from_str(text).unwrap()
    }

    #[test]
    fn a_types_favourites_are_the_tabs_it_can_open() {
        let s = shell_json(Some(&setlist()), "/s/abc/");
        let ids: Vec<&str> = s["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["setlist", "charts", "notes"],
            "a view it does not know is left out"
        );
        assert_eq!(s["tabs"][1]["target"], json!({ "view": "player.now" }));
        assert_eq!(s["tabs"][2]["target"], json!({ "href": "/s/abc/#notes" }));
        assert!(
            s["tabs"][0]["icon"]
                .as_str()
                .unwrap()
                .starts_with("<svg viewBox=\"0 0 24 24\"")
        );
        assert!(
            s["tabs"][0]["icon"].as_str().unwrap().contains("<path d="),
            "Lucide's list-music"
        );
        assert_eq!(s["primary"]["target"], json!({ "action": "PLAYER_TOGGLE" }));
        assert_eq!(s["primary"]["state"], "playing");
        assert_eq!(s["primary"]["on"]["label"], "Pause");
        assert_eq!(
            s["primary"]["on"]["target"],
            json!({ "action": "PLAYER_TOGGLE" })
        );
    }

    #[test]
    fn without_a_declaration_the_shares_own_tabs() {
        let s = shell_json(None, "/s/abc/");
        let ids: Vec<&str> = s["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["home", "search", "browse"]);
        assert!(s["primary"].is_null());
        // A declaration that does not check is not used.
        let mut bad = setlist();
        bad.tabs[0].label = String::new();
        assert_eq!(shell_json(Some(&bad), "/s/abc/")["tabs"][0]["id"], "home");
        // By type, any case.
        let all = [setlist()];
        assert!(favourites_for(&all, "Setlist").is_some());
        assert!(favourites_for(&all, "song").is_none());
        assert!(favourites_for(&all, "").is_none());
    }

    #[test]
    fn a_primary_state_it_does_not_know_shows_one_face() {
        let mut f = setlist();
        let p = f.primary.as_mut().unwrap();
        p.state = Some("stored".to_owned());
        let s = shell_json(Some(&f), "/s/abc/");
        assert!(s["primary"]["on"].is_null() && s["primary"]["state"].is_null());
        assert_eq!(s["primary"]["label"], "Play");
        // An action it cannot run: no round button.
        let mut f = setlist();
        f.primary.as_mut().unwrap().target = Target::Action("SHARE_OFFLINE".to_owned());
        assert!(shell_json(Some(&f), "/s/abc/")["primary"].is_null());
    }

    /// A place a browser would read as another site never becomes a tab.
    #[test]
    fn a_place_that_would_leave_the_site_is_no_tab() {
        for evil in [
            "\\evil.example/",
            "/\\evil",
            "%5cevil",
            "//evil.example",
            "a:b",
        ] {
            assert_eq!(
                target_json(&Target::Place(evil.to_owned()), "/"),
                None,
                "{evil}"
            );
        }
        assert_eq!(
            target_json(&Target::Place("#notes".to_owned()), "/s/x/"),
            Some(json!({ "href": "/s/x/#notes" }))
        );
    }

    #[test]
    fn an_icon_is_lucides_shapes_and_nothing_else() {
        let svg = icon_svg("house");
        assert!(svg.ends_with("</svg>") && !svg.contains("script") && !svg.contains(" on"));
        assert!(icon_svg("no-such-icon").contains("<circle cx=\"12\""));
        assert!(
            icon_svg("pause").contains("<path d=\"M15 4h3"),
            "a rect icon as paths"
        );
    }
}
