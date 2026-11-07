//! Files in the navigator: each org, its projects and its libraries on
//! the left; a project's top folders and files, or a library's assets, on
//! the right. Every item opens its page through `FILES_OPEN`.

use architect_navigator_api::{Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot};
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
            let mut projects =
                Node::new(format!("projects:{org}"), "Projects").icon("folder-kanban");
            let mut assets = Node::new(format!("assets:{org}"), "Assets").icon("library");
            for r in roots.iter().filter(|r| text(r, "org") == org) {
                let place = text(r, "place");
                if text(r, "section") == "Assets" {
                    assets = assets
                        .child(Node::new(format!("root:{place}"), text(r, "name")).icon("library"));
                } else {
                    projects = projects
                        .child(Node::new(format!("root:{place}"), text(r, "name")).icon("folder"));
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

impl NavigatorProvider for FilesNav {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        Box::pin(async {
            let roots = run("FILES_ROOTS", json!({})).await?;
            let roots = roots.as_array().cloned().unwrap_or_default();
            let mut snap = Snapshot::new("Files");
            for node in tree(&roots) {
                snap = snap.node(node);
            }
            for r in &roots {
                let place = text(r, "place").to_owned();
                let org = text(r, "org");
                let pinned = r.get("pinned").and_then(Value::as_bool).unwrap_or(false);
                if text(r, "section") == "Assets" {
                    let Ok(assets) = run("FILES_ASSETS", json!({ "root": place })).await else {
                        continue;
                    };
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
                        let mut item =
                            Item::new(format!("{place}|{name}"), format!("root:{place}"), name)
                                .dates(at, at)
                                .preview(props.join(" · "))
                                .opens(opens(json!({ "root": place, "asset": name })));
                        if let Some(ty) = a.pointer("/properties/type").and_then(Value::as_str) {
                            item = item.badge(ty);
                        }
                        snap.items.push(item);
                    }
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
