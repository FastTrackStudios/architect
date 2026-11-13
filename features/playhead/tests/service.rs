//! The playhead as the app hosts it: linked in beside two extensions that
//! use `architect.playhead` through the host service, as a linked or a
//! process extension does. Each puts a queue on the deck and moves it; one
//! cannot move the other's, whatever it names, because who calls is the
//! host's to say (the action context's `caller`); the app (the bar, the
//! Action List) moves the active one and switches between them.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::linked::{Builder, Extension};
use architect_playhead::{ID, Playhead, SERVICE};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

/// An extension that plays through the service: `LAB_<ID>_CALL {method,
/// args}` calls `architect.playhead`'s `method`.
struct Player(&'static str);

impl Extension for Player {
    fn manifest(&self) -> &'static str {
        self.0
    }

    fn build(&self, ext: &mut Builder) {
        let handle = ext.handle();
        let id = if self.0.contains("id = \"lab.alpha\"") {
            "LAB_ALPHA_CALL"
        } else {
            "LAB_BETA_CALL"
        };
        ext.action(id, move |args: Value| {
            let handle = handle.clone();
            async move {
                let method = args["method"].as_str().unwrap_or_default().to_owned();
                handle
                    .service(SERVICE, &method, args["args"].clone())
                    .await
                    .map_err(|e| e.to_string())
            }
        });
    }
}

fn manifest(id: &str, action: &str) -> &'static str {
    Box::leak(
        format!(
            r#"id = "{id}"
name = "{id}"
version = "0.1.0"
kind = "process"
api = "0.1"

[entry]
path = "x"

[uses]
"architect.playhead" = "^0.1"

[capabilities]
effect = "mutating"

[[actions]]
id = "{action}"
name = "call"
effect = "mutating"
params = '{{"type":"object","properties":{{"method":{{"type":"string"}},"args":{{"type":"object"}}}}}}'
"#
        )
        .into_boxed_str(),
    )
}

fn run(list: &ActionList, id: &str, args: Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn call(list: &ActionList, who: &str, method: &str, args: Value) -> Result<Value, String> {
    run(list, who, json!({ "method": method, "args": args }))
}

#[test]
fn two_extensions_each_with_a_source_and_the_app_between_them() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Playhead)
        .unwrap()
        .with_linked(&Player(manifest("lab.alpha", "LAB_ALPHA_CALL")))
        .unwrap()
        .with_linked(&Player(manifest("lab.beta", "LAB_BETA_CALL")))
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    assert_eq!(
        ext.host()
            .provider(SERVICE, &"^0.1".parse().unwrap())
            .unwrap()
            .0,
        ID
    );

    let queue = |title: &str| {
        json!({ "name": title, "queue": [
            { "id": "1", "title": format!("{title} one"), "duration": 60 },
            { "id": "2", "title": format!("{title} two"), "duration": 60 }
        ]})
    };
    // Each puts its queue in: the source is its extension's, by the host.
    let a = call(&list, "LAB_ALPHA_CALL", "set", queue("Alpha")).unwrap();
    assert_eq!(
        (a["source"].clone(), a["active"].clone()),
        (json!("lab.alpha"), json!(true))
    );
    let b = call(&list, "LAB_BETA_CALL", "set", queue("Beta")).unwrap();
    assert_eq!(
        (b["source"].clone(), b["active"].clone()),
        (json!("lab.beta"), json!(false))
    );

    // Beta cannot move Alpha's, naming it or not.
    assert!(
        call(
            &list,
            "LAB_BETA_CALL",
            "pause",
            json!({ "source": "lab.alpha" })
        )
        .unwrap_err()
        .contains("not your source")
    );
    assert!(
        call(
            &list,
            "LAB_BETA_CALL",
            "activate",
            json!({ "source": "lab.beta" })
        )
        .unwrap_err()
        .contains("only the app")
    );
    // Its own: play takes the bar.
    let b = call(&list, "LAB_BETA_CALL", "play", json!({})).unwrap();
    assert_eq!(
        (b["playing"].clone(), b["active"].clone()),
        (json!(true), json!(true))
    );
    let b = call(&list, "LAB_BETA_CALL", "next", json!({})).unwrap();
    assert_eq!(b["title"], "Beta two");
    // What Alpha reads is its own, paused now.
    let a = call(&list, "LAB_ALPHA_CALL", "state", json!({})).unwrap();
    assert_eq!(
        (a["playing"].clone(), a["active"].clone()),
        (json!(false), json!(false))
    );

    // The app: the active source, or the one it names.
    let now = run(&list, "PLAYHEAD_STATE", json!({})).unwrap();
    assert_eq!(now["source"], "lab.beta");
    run(&list, "PLAYHEAD_PAUSE", json!({})).unwrap();
    run(&list, "PLAYHEAD_ACTIVATE", json!({ "source": "lab.alpha" })).unwrap();
    let listed = run(&list, "PLAYHEAD_SOURCES", json!({})).unwrap();
    assert_eq!(listed["active"], "lab.alpha");
    assert_eq!(listed["sources"].as_array().unwrap().len(), 2);
    run(&list, "PLAYHEAD_ITEM", json!({ "index": 1, "play": true })).unwrap();
    let a = call(&list, "LAB_ALPHA_CALL", "state", json!({})).unwrap();
    assert_eq!(
        (a["title"].clone(), a["playing"].clone()),
        (json!("Alpha two"), json!(true))
    );

    // Media is a vault path, never a URL.
    assert!(
        call(
            &list,
            "LAB_ALPHA_CALL",
            "set",
            json!({ "queue": [
                { "id": "x", "title": "X", "media": "https://evil.test/x.mp3" }
            ]})
        )
        .unwrap_err()
        .contains("vault path")
    );
    // Gone: an empty queue.
    call(&list, "LAB_ALPHA_CALL", "set", json!({ "queue": [] })).unwrap();
    call(&list, "LAB_BETA_CALL", "set", json!({ "queue": [] })).unwrap();
    assert_eq!(
        run(&list, "PLAYHEAD_STATE", json!({})).unwrap(),
        Value::Null
    );
}
