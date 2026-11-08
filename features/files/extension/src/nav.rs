//! Files in the navigator: each org, its projects and its libraries on
//! the left; a project's top folders and files, or a library's assets, on
//! the right. Every item opens its page through `FILES_OPEN`.

use architect_navigator_api::{
    Item, NavigatorProvider, Node, Open, PROJECT_ICON, ProviderFuture, Snapshot,
};
use serde_json::{Value, json};

use crate::{run, summary, text};

/// The provider's actions: `FILES_NAV_SNAPSHOT`, `_OPEN`, `_MOVE`, `_CREATE`.
pub const ACTIONS: &str = "FILES_NAV";

/// The Files provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilesNav;

fn opens(args: Value) -> Open {
    Open::Action {
        id: "FILES_OPEN".to_owned(),
        args,
    }
}

/// A value's `at_us` as the navigator's milliseconds.
fn millis(v: &Value) -> i64 {
    v.get("at_us").and_then(Value::as_i64).unwrap_or(0) / 1000
}

/// The left tree: org → Projects (each project), Assets (each library).
#[must_use]
pub fn tree(roots: &[Value]) -> Vec<Node> {
    let mut orgs: Vec<String> = roots.iter().map(|r| text(r, "org").to_owned()).collect();
    orgs.dedup();
    orgs.sort();
    orgs.dedup();
    orgs.into_iter()
        .map(|org| {
            let mut projects = Node::new(format!("projects:{org}"), "Projects").icon(PROJECT_ICON);
            let mut assets = Node::new(format!("assets:{org}"), "Assets").icon("library");
            for r in roots.iter().filter(|r| text(r, "org") == org) {
                let place = text(r, "place");
                if text(r, "section") == "Assets" {
                    assets = assets
                        .child(Node::new(format!("root:{place}"), text(r, "name")).icon("library"));
                } else {
                    projects = projects.child(
                        Node::new(format!("root:{place}"), text(r, "name")).icon(PROJECT_ICON),
                    );
                }
            }
            let mut node = Node::new(format!("org:{org}"), &org).icon("building");
            if !projects.children.is_empty() {
                node = node.child(projects);
            }
            if !assets.children.is_empty() {
                node = node.child(assets);
            }
            node
        })
        .collect()
}

/// What Files shows with no project folder on this device: how to get one.
///
/// `error` is why the roots could not be read, when it is more than there
/// being nothing to ask (no files engine is a plain empty state).
#[must_use]
pub fn empty_state(error: Option<&str>) -> Snapshot {
    let why = error.filter(|e| !e.contains("no action"));
    let mut connect = Item::new("empty:connect", "", "Connect to a node").preview(
        "A node keeps your org's project files and copies them to your devices. \
         Sign in and enrol this device in the org (Account → Devices).",
    );
    if let Some(why) = why {
        connect = connect.badge("offline").property("error", why);
        connect.preview = Some(format!("The node could not be reached: {why}"));
    }
    Snapshot::new("Files")
        .item(Item::new("empty:add", "", "Add a project folder").preview(
            "A project's sessions, stems and assets live in its folder on a node, \
                 under <org>/Projects/<project>. Make one there and it shows here.",
        ))
        .item(connect)
}

/// Library `place`'s assets, as items in container `container`.
async fn library_items(place: &str, container: &str) -> Vec<Item> {
    let Ok(assets) = run("FILES_ASSETS", json!({ "root": place })).await else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for a in assets.as_array().into_iter().flatten() {
        let name = text(a, "name");
        let props: Vec<&str> = a
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(k, _)| k.as_str() != "type")
            .filter_map(|(_, v)| v.as_str())
            .collect();
        let at = millis(a);
        let mut item = Item::new(format!("{place}|{name}"), container, name)
            .dates(at, at)
            .preview(props.join(" · "))
            .opens(opens(json!({ "root": place, "asset": name })));
        if let Some(ty) = a.pointer("/properties/type").and_then(Value::as_str) {
            item = item.badge(ty);
        }
        out.push(item);
    }
    out
}

/// The asset libraries' provider's actions: `FILES_ASSETS_NAV_SNAPSHOT`, …
pub const ASSETS_ACTIONS: &str = "FILES_ASSETS_NAV";

/// Assets/ in the navigator (#443): every org's asset libraries in one
/// flat list, each marked with its org, its assets on the right.
#[derive(Debug, Clone, Copy, Default)]
pub struct AssetsNav;

/// The libraries of `roots` as nodes, by name, each its org's.
#[must_use]
pub fn libraries(roots: &[Value]) -> Vec<Node> {
    let mut out: Vec<Node> = roots
        .iter()
        .filter(|r| text(r, "section") == "Assets")
        .map(|r| {
            Node::new(format!("root:{}", text(r, "place")), text(r, "name"))
                .icon("library")
                .org(text(r, "org"))
        })
        .collect();
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    out
}

impl NavigatorProvider for AssetsNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async {
            // No files here: no libraries, and nothing to say about it.
            let roots = run("FILES_ROOTS", json!({}))
                .await
                .ok()
                .and_then(|r| r.as_array().cloned())
                .unwrap_or_default();
            let mut snap = Snapshot::new("Assets");
            for node in libraries(&roots) {
                let place = node.id.trim_start_matches("root:").to_owned();
                snap.items.extend(library_items(&place, &node.id).await);
                snap = snap.node(node);
            }
            Ok(snap)
        })
    }

    fn open(&self, item: String) -> ProviderFuture<Open> {
        Box::pin(async move {
            let (root, asset) = item.split_once('|').unwrap_or((&item, ""));
            Ok(opens(json!({ "root": root, "asset": asset })))
        })
    }

    fn move_items(&self, _items: Vec<String>, _to: String) -> ProviderFuture<()> {
        Box::pin(async { Err("move assets in their library's folder".to_owned()) })
    }

    fn create(&self, _container: String, _title: String) -> ProviderFuture<Open> {
        Box::pin(async { Err("add assets to their library's folder".to_owned()) })
    }
}

impl NavigatorProvider for FilesNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async {
            // No files on this device yet (no root, or no files engine to
            // ask): what to do about it, not an error (#443).
            let roots = match run("FILES_ROOTS", json!({})).await {
                Ok(roots) => roots.as_array().cloned().unwrap_or_default(),
                Err(e) => return Ok(empty_state(Some(&e))),
            };
            if roots.is_empty() {
                return Ok(empty_state(None));
            }
            let mut snap = Snapshot::new("Files");
            for node in tree(&roots) {
                snap = snap.node(node);
            }
            let mut orgs: Vec<&str> = roots.iter().map(|r| text(r, "org")).collect();
            orgs.sort_unstable();
            orgs.dedup();
            for org in orgs {
                snap.items.push(
                    Item::new(
                        format!("health:{org}"),
                        format!("org:{org}"),
                        "Storage health",
                    )
                    .preview("copy counts, holders, files under policy")
                    .opens(opens(json!({ "view": "health", "root": org }))),
                );
                snap.items.push(
                    Item::new(format!("find:{org}"), format!("org:{org}"), "Find files")
                        .preview("search every root, offline, and see where each file lives")
                        .opens(opens(json!({ "view": "find" }))),
                );
            }
            for r in &roots {
                let place = text(r, "place").to_owned();
                let org = text(r, "org");
                let pinned = r.get("pinned").and_then(Value::as_bool).unwrap_or(false);
                if text(r, "section") == "Assets" {
                    snap.items
                        .extend(library_items(&place, &format!("root:{place}")).await);
                } else {
                    let entries = run("FILES_BROWSE", json!({ "root": place, "path": "" }))
                        .await
                        .unwrap_or_default();
                    let entries = entries.as_array().cloned().unwrap_or_default();
                    let newest = entries.iter().map(millis).max().unwrap_or(0);
                    // The project itself, in its org's Projects.
                    snap.items.push(
                        Item::new(
                            format!("{place}|"),
                            format!("projects:{org}"),
                            text(r, "name"),
                        )
                        .dates(newest, newest)
                        .preview(summary(r))
                        .badge(if pinned { "pinned" } else { "online" })
                        .opens(opens(json!({ "root": place }))),
                    );
                    for e in &entries {
                        let at = millis(e);
                        let dir = e.get("dir").and_then(Value::as_bool).unwrap_or(false);
                        let path = text(e, "path");
                        snap.items.push(
                            Item::new(
                                format!("{place}|{path}"),
                                format!("root:{place}"),
                                text(e, "name"),
                            )
                            .dates(at, at)
                            .preview(crate::size(
                                e.get("size").and_then(Value::as_u64).unwrap_or(0),
                            ))
                            .opens(opens(json!({ "root": place, "path": path, "dir": dir }))),
                        );
                    }
                }
            }
            Ok(snap)
        })
    }

    fn open(&self, item: String) -> ProviderFuture<Open> {
        Box::pin(async move {
            let (root, rest) = item.split_once('|').unwrap_or((&item, ""));
            Ok(opens(json!({ "root": root, "path": rest })))
        })
    }

    fn move_items(&self, _items: Vec<String>, _to: String) -> ProviderFuture<()> {
        Box::pin(async { Err("move files in the project's own folder, or over WebDAV".to_owned()) })
    }

    fn create(&self, _container: String, _title: String) -> ProviderFuture<Open> {
        Box::pin(async { Err("make a project's files in its folder".to_owned()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_roots_is_an_empty_state_not_an_error() {
        let s = empty_state(Some("FILES_NAV_SNAPSHOT: no action FILES_ROOTS"));
        let titles: Vec<&str> = s.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Add a project folder", "Connect to a node"]);
        assert!(
            s.items.iter().all(|i| i.badges.is_empty()),
            "no engine is no error"
        );
        let down = empty_state(Some("connection refused"));
        assert_eq!(down.items[1].badges, ["offline"]);
    }

    #[test]
    fn every_orgs_libraries_are_one_flat_list_marked_with_their_org() {
        let roots = vec![
            json!({"org": "days-to-praise", "section": "Assets", "place": "days-to-praise/Assets/Tracks", "name": "Tracks"}),
            json!({"org": "codywright", "section": "Projects", "place": "codywright/Projects/Right Here", "name": "Right Here"}),
            json!({"org": "codywright", "section": "Assets", "place": "codywright/Assets/Music", "name": "Music"}),
        ];
        let l = libraries(&roots);
        let names: Vec<(&str, Option<&str>)> = l
            .iter()
            .map(|n| (n.title.as_str(), n.org.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("Music", Some("codywright")),
                ("Tracks", Some("days-to-praise"))
            ]
        );
    }

    #[test]
    fn orgs_hold_their_projects_and_libraries() {
        let roots = vec![
            json!({"org": "codywright", "section": "Projects", "place": "codywright/Projects/Right Here", "name": "Right Here"}),
            json!({"org": "days-to-praise", "section": "Assets", "place": "days-to-praise/Assets/Tracks", "name": "Tracks"}),
            json!({"org": "codywright", "section": "Assets", "place": "codywright/Assets/Music", "name": "Music"}),
        ];
        let t = tree(&roots);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].title, "codywright");
        assert_eq!(t[0].children.len(), 2);
        assert_eq!(
            t[0].children[0].children[0].id,
            "root:codywright/Projects/Right Here"
        );
        assert_eq!(t[1].children[0].title, "Assets");
    }
}
