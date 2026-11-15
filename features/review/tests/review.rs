//! Review against a real vault, as the app hosts it: the app's list
//! holds the vault's and Bases' actions over a folder; Threads, the
//! playhead and Review are linked in, and Review reaches the others only
//! through the host. A song with a light-show video in two versions and a
//! recording: comments at a time, over a range and on a part of the
//! frame, per version, the playhead's marks, a new version, carrying the
//! open comments over, each read back from the markdown.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use std::path::Path;
use std::sync::Arc;

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const SONG: &str = "Songs/Joy/Joy.md";
const V1: &str = "Songs/Joy/Joy - Lights v1.mp4";
const V2: &str = "Songs/Joy/Joy - Lights v2.mp4";
const V3: &str = "Songs/Joy/Joy - Lights v3.mp4";
const MIX: &str = "Songs/Joy/Joy.ogg";

fn app(dir: &Path) -> Arc<ActionList> {
    let list = Arc::new(ActionList::new());
    let open = vault::actions::OpenVault::open(dir).unwrap();
    vault::actions::register(&list, &open).unwrap();
    let types: &'static architect_bases::model::ViewTypes =
        Box::leak(Box::new(architect_bases::model::ViewTypes::new()));
    architect_bases::actions::register(
        &list,
        &open,
        types,
        architect_bases::actions::local_queries(&open),
    )
    .unwrap();
    list
}

fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

fn fails(list: &ActionList, id: &str, args: Value) -> String {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .err()
        .unwrap_or_else(|| panic!("{id} {args} should fail"))
        .to_string()
}

#[test]
fn a_bands_review_of_a_light_show_lives_in_threads() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    std::fs::create_dir_all(dir.join("Songs/Joy")).unwrap();
    std::fs::write(
        dir.join(SONG),
        "---\ntitle: Joy to the World\nartist: The Band\nduration: \"3:20\"\n\
         review:\n  - name: Lights\n    versions:\n      - \"[[Joy - Lights v1.mp4]]\"\n      - \"[[Joy - Lights v2.mp4]]\"\n\
         \x20 - name: Mix\n    versions:\n      - \"[[Joy.ogg]]\"\n---\n# Joy\n",
    )
    .unwrap();
    let app = app(&dir);
    run(&app, "VAULT_RELOAD", Value::Null);

    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.set_app_list((*app).clone());
    host.forward_actions_to(&app);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&architect_threads::Threads)
        .unwrap()
        .with_linked(&architect_playhead::Playhead)
        .unwrap()
        .with_linked(&architect_review::Review)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    assert_eq!(
        ext.host()
            .provider(architect_review::SERVICE, &"^0.1".parse().unwrap())
            .unwrap()
            .0,
        architect_review::ID
    );

    // The item: two tracks, the light show in two versions.
    let item = run(&list, "REVIEW_ITEM", json!({ "path": SONG }));
    assert_eq!(item["title"], "Joy to the World");
    assert_eq!(item["duration"], 200.0);
    assert_eq!(item["tracks"][0]["name"], "Lights");
    assert_eq!(item["tracks"][0]["versions"][1]["file"], V2);
    assert_eq!(item["tracks"][0]["versions"][1]["kind"], "video");
    assert_eq!(item["tracks"][1]["versions"][0]["file"], MIX);

    // Open: the newest light show, cued on the playhead, paused.
    let opened = run(&list, "REVIEW_OPEN", json!({ "path": SONG, "open": false }));
    assert_eq!(opened["file"], V2);
    let state = run(&list, "PLAYHEAD_STATE", json!({ "source": "review" }));
    assert_eq!(state["item"]["id"], V2);
    assert_eq!(state["item"]["media"], V2);
    assert_eq!(state["playing"], false);

    // Comments on v1: a time, a range on a part of the frame; one on v2.
    let at = run(
        &list,
        "REVIEW_COMMENT",
        json!({ "path": SONG, "file": V1, "start": "0:33", "body": "Strobe too early @alex", "author": "cody" }),
    );
    let at_id = at["thread"]["id"].as_str().unwrap().to_owned();
    assert_eq!(at["thread"]["moment"], json!({ "start": 33.0 }));
    assert_eq!(at["thread"]["anchorPath"], format!("media/{V1}"));
    let span = run(
        &list,
        "REVIEW_COMMENT",
        json!({ "path": SONG, "file": V1, "start": 60, "end": 75.5,
                "rect": { "x": 0.5, "y": 0.0, "w": 0.5, "h": 0.5 },
                "body": "Bridge drags; the top right wash is too blue", "author": "alex" }),
    );
    let span_id = span["thread"]["id"].as_str().unwrap().to_owned();
    run(
        &list,
        "REVIEW_COMMENT",
        json!({ "path": SONG, "file": V2, "start": 10, "body": "Love the intro", "kind": "praise", "author": "sam" }),
    );
    // A reply on the range, then the time resolved.
    let first = span["comment"]["id"].as_str().unwrap().to_owned();
    run(
        &list,
        "THREADS_REPLY",
        json!({ "thread": span_id, "comment": first, "body": "Will fix in v3", "author": "cody" }),
    );

    // Refused: a file not a version, a bad range, a time not a time.
    assert!(
        fails(
            &list,
            "REVIEW_COMMENT",
            json!({ "path": SONG, "file": "Songs/Joy/other.mp4", "start": 1, "body": "x" })
        )
        .contains("not a version")
    );
    assert!(
        !fails(
            &list,
            "REVIEW_COMMENT",
            json!({ "path": SONG, "file": V1, "start": 9, "end": 3, "body": "x" })
        )
        .is_empty()
    );
    assert!(
        fails(
            &list,
            "REVIEW_COMMENT",
            json!({ "path": SONG, "file": V1, "start": "soon", "body": "x" })
        )
        .contains("start")
    );

    // Per version, by time; the note's markdown holds them.
    let on_v1 = run(&list, "REVIEW_LIST", json!({ "path": SONG, "file": V1 }));
    let ids: Vec<&str> = on_v1["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![at_id.as_str(), span_id.as_str()]);
    assert_eq!(
        on_v1["comments"][1]["comments"].as_array().unwrap().len(),
        2
    );
    assert_eq!(on_v1["comments"][1]["moment"]["rect"]["w"], 0.5);
    let thread_path = run(&list, "THREADS_GET", json!({ "thread": at_id }))["path"]
        .as_str()
        .unwrap()
        .to_owned();
    let text = std::fs::read_to_string(dir.join(&thread_path)).unwrap();
    for line in [
        "entityId: Songs/Joy/Joy.md",
        "anchorPath: media/Songs/Joy/Joy - Lights v1.mp4",
        "moment:",
        "start: 33",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    // Threads' own counts see them on the song's note.
    let counts = run(&list, "THREADS_COUNTS", json!({}));
    assert_eq!(counts["note:Songs/Joy/Joy.md"]["open"], 3);
    let item = run(&list, "REVIEW_ITEM", json!({ "path": SONG }));
    assert_eq!(item["tracks"][0]["versions"][0]["open"], 2);
    assert_eq!(item["tracks"][0]["versions"][1]["open"], 1);

    // Playing v1 puts its comments on the playhead as marks.
    run(
        &list,
        "REVIEW_PLAY",
        json!({ "path": SONG, "file": V1, "t": 30, "play": false }),
    );
    let state = run(&list, "PLAYHEAD_STATE", json!({ "source": "review" }));
    assert_eq!(state["item"]["id"], V1);
    let marks: Vec<f64> = state["item"]["marks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["at"].as_f64().unwrap())
        .collect();
    assert_eq!(marks, vec![33.0, 60.0]);
    assert!(
        (state["time"].as_f64().unwrap() - 30.0).abs() < 0.5,
        "{state}"
    );

    // A third version; then v1's open comments carried to v2.
    let added = run(
        &list,
        "REVIEW_ADD_VERSION",
        json!({ "path": SONG, "file": "Joy - Lights v3.mp4", "track": "Lights" }),
    );
    assert_eq!(added["file"], V3);
    let props = run(&app, "VAULT_READ", json!({ "path": SONG }))["properties"].clone();
    assert_eq!(
        props["review"][0]["versions"][2]["file"],
        format!("[[{V3}]]")
    );
    assert_eq!(props["review"][0]["versions"][2]["label"], "v3");
    assert!(
        fails(
            &list,
            "REVIEW_ADD_VERSION",
            json!({ "path": SONG, "file": V3 })
        )
        .contains("already")
    );

    run(
        &list,
        "THREADS_RESOLVE",
        json!({ "thread": at_id, "by": "cody" }),
    );
    let carried = run(&list, "REVIEW_CARRY", json!({ "path": SONG, "from": V1 }));
    assert_eq!(carried["to"], V2, "the version after v1");
    assert_eq!(
        carried["carried"].as_array().unwrap().len(),
        1,
        "only the open one"
    );
    let on_v2 = run(&list, "REVIEW_LIST", json!({ "path": SONG, "file": V2 }));
    let moved = on_v2["comments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["moment"]["start"] == 60.0)
        .unwrap()
        .clone();
    assert_eq!(moved["moment"]["end"], 75.5);
    assert_eq!(moved["moment"]["rect"]["x"], 0.5);
    let bodies: Vec<(&str, &str)> = moved["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["author"].as_str().unwrap(), c["body"].as_str().unwrap()))
        .collect();
    assert_eq!(
        bodies,
        vec![
            ("alex", "Bridge drags; the top right wash is too blue"),
            ("cody", "Will fix in v3")
        ]
    );
    assert!(
        moved["comments"][1]["replyTo"] == moved["comments"][0]["id"],
        "a reply stays a reply"
    );
    let old = run(&list, "THREADS_GET", json!({ "thread": span_id }));
    assert_eq!(old["resolved"], true);
    assert_eq!(
        old["comments"].as_array().unwrap().last().unwrap()["body"],
        "Carried to v2."
    );
    // Nothing left open on v1.
    assert_eq!(
        run(
            &list,
            "REVIEW_LIST",
            json!({ "path": SONG, "file": V1, "resolved": false })
        )["open"],
        0
    );
}

#[test]
fn a_note_without_a_review_list_reviews_its_video_and_recording() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    std::fs::create_dir_all(dir.join("Songs")).unwrap();
    std::fs::write(
        dir.join("Songs/Grace.md"),
        "---\ntitle: Amazing Grace\nvideo: \"[[Grace - Lights.mp4]]\"\nrecording: \"[[Recordings/Grace.mp3]]\"\n---\n",
    )
    .unwrap();
    let app = app(&dir);
    run(&app, "VAULT_RELOAD", Value::Null);
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.set_app_list((*app).clone());
    host.forward_actions_to(&app);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&architect_threads::Threads)
        .unwrap()
        .with_linked(&architect_review::Review)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();

    let item = run(&list, "REVIEW_ITEM", json!({ "path": "Songs/Grace.md" }));
    assert_eq!(item["tracks"][0]["name"], "Video");
    assert_eq!(
        item["tracks"][0]["versions"][0]["file"],
        "Songs/Grace - Lights.mp4"
    );
    assert_eq!(item["tracks"][1]["name"], "Recording");
    assert_eq!(
        item["tracks"][1]["versions"][0]["file"],
        "Recordings/Grace.mp3"
    );
    // No playhead loaded: opening and commenting still work.
    let opened = run(
        &list,
        "REVIEW_OPEN",
        json!({ "path": "Songs/Grace.md", "open": false }),
    );
    assert_eq!(opened["file"], "Songs/Grace - Lights.mp4");
    run(
        &list,
        "REVIEW_COMMENT",
        json!({ "path": "Songs/Grace.md", "file": "Recordings/Grace.mp3", "start": 95, "end": 110, "body": "Bar 33: guitars too loud" }),
    );
    let l = run(
        &list,
        "REVIEW_LIST",
        json!({ "path": "Songs/Grace.md", "file": "Recordings/Grace.mp3" }),
    );
    assert_eq!(l["open"], 1);
    // Adding a version to a note with no list writes the list it had.
    run(
        &list,
        "REVIEW_ADD_VERSION",
        json!({ "path": "Songs/Grace.md", "file": "Recordings/Grace v2.mp3", "track": "Recording" }),
    );
    let item = run(&list, "REVIEW_ITEM", json!({ "path": "Songs/Grace.md" }));
    assert_eq!(
        item["tracks"][1]["versions"][1]["file"],
        "Recordings/Grace v2.mp3"
    );
    assert_eq!(
        item["tracks"][0]["versions"][0]["file"],
        "Songs/Grace - Lights.mp4"
    );
}
