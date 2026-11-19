//! In-process extensions (#183): one trait a crate implements runs linked
//! into the app as a built-in — its actions here, told where they were
//! invoked from, its views Dioxus components, its builder's contributions
//! beside its manifest's — is turned off and on at run time, and is
//! overridden by a development copy of its id.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect::action::{ActionCaller, ActionList, Invocation, InvocationSource};
use architect_extensions::Origin;
use architect_extensions::contrib::{Contribution, StatusDecl};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::linked::{Builder, Extension};
use architect_settings::Registry;
use dioxus::prelude::*;
use futures::executor::block_on;
use serde_json::{Value, json};

const MANIFEST: &str = r#"
id = "lab.linked"
name = "Linked"
version = "0.1.0"
kind = "process"

[entry]
path = "linked-bin"

[[actions]]
id = "LAB_LINKED_WHERE"
name = "Where was I run from"

[[views]]
id = "panel"
title = "Panel"

[[tabs]]
id = "panel"
title = "Panel"
view = "panel"
"#;

#[component]
fn Panel() -> Element {
    rsx! { "the panel" }
}

struct Linked;

impl Extension for Linked {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        ext.view("panel", Panel)
            .action_in("LAB_LINKED_WHERE", |_, cx| async move { Ok(cx) })
            .contribute(Contribution {
                status: vec![StatusDecl {
                    id: "linked".into(),
                    text: "Linked in".into(),
                    ..StatusDecl::default()
                }],
                ..Contribution::default()
            });
    }
}

/// The same id as a development copy, in Luau.
const DEV: &str = r#"
id = "lab.linked"
name = "Linked (dev)"
version = "0.2.0-dev"
kind = "luau"

[[actions]]
id = "LAB_LINKED_WHERE"
name = "Where was I run from"
script = "where.luau"

[[views]]
id = "panel"
title = "Panel"
"#;

#[test]
fn a_linked_extension_runs_in_process_and_a_dev_copy_overrides_it() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Linked)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();

    let loaded = ext.host().get("lab.linked").unwrap();
    assert_eq!(loaded.origin, Origin::Builtin);
    // Its view is its component; its action runs here, told where from.
    assert!(ext.host().linked_view("lab.linked", "panel").is_some());
    let caller = ActionCaller::app().with_invocation(Invocation::new(InvocationSource::Palette));
    let out = block_on(list.run_json_async_with(&caller, "LAB_LINKED_WHERE", b"")).unwrap();
    let out: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(out["source"], "palette");
    // The manifest's tab and the builder's status item, together.
    let reg = ext.host().contributions().clone();
    let keys = reg.of("lab.linked").unwrap().keys();
    assert!(keys.contains("tab:panel") && keys.contains("status:linked"));

    // Off and on at run time, like any extension.
    let _ = block_on(list.run_json_async("EXT_DISABLE", br#"{"id":"lab.linked"}"#)).unwrap();
    assert!(ext.host().get("lab.linked").is_none());
    assert!(reg.tabs().is_empty());
    assert!(list.info("LAB_LINKED_WHERE").is_none());
    let _ = block_on(list.run_json_async("EXT_ENABLE", br#"{"id":"lab.linked"}"#)).unwrap();
    assert!(list.info("LAB_LINKED_WHERE").is_some());

    // A development copy of its id overrides it: not linked any more.
    let dev = tmp.path().join("dev");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::write(dev.join("extension.toml"), DEV).unwrap();
    std::fs::write(
        dev.join("where.luau"),
        "return { source = ctx.source, dev = true }",
    )
    .unwrap();
    let _ = ext.dev(&dev);
    assert_eq!(ext.host().get("lab.linked").unwrap().origin, Origin::Dev);
    assert!(ext.host().linked_view("lab.linked", "panel").is_none());
    let out = block_on(list.run_json_async_with(&caller, "LAB_LINKED_WHERE", b"")).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&out).unwrap(),
        json!({ "source": "palette", "dev": true })
    );
    assert!(
        !reg.of("lab.linked")
            .unwrap()
            .keys()
            .contains("status:linked"),
        "the dev copy's own contributions"
    );
    // Its session over, the linked one is back.
    let _ = ext.end_dev(&dev);
    assert_eq!(
        ext.host().get("lab.linked").unwrap().origin,
        Origin::Builtin
    );
}

#[test]
fn a_linked_extension_that_does_not_match_its_manifest_is_refused() {
    struct Wrong;
    impl Extension for Wrong {
        fn manifest(&self) -> &'static str {
            MANIFEST
        }
        fn build(&self, ext: &mut Builder) {
            ext.view("panel", Panel);
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let host = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("i")))
        .with_linked(&Wrong)
        .unwrap();
    let events = ext.sync();
    assert!(
        format!("{events:?}").contains("does not run its action LAB_LINKED_WHERE"),
        "{events:?}"
    );
}

/// One that keeps its handle and asks the host through it: in process,
/// the host's own session, checked as every other way is.
struct Asking;

static ASKING: std::sync::OnceLock<architect_extensions::linked::Handle> =
    std::sync::OnceLock::new();

const ASKING_MANIFEST: &str = r#"
id = "lab.asking"
name = "Asking"
version = "0.1.0"
kind = "process"

[entry]
path = "asking-bin"

[[actions]]
id = "LAB_ASKING_GO"
name = "Ask the host"
"#;

impl Extension for Asking {
    fn manifest(&self) -> &'static str {
        ASKING_MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        let _ = ASKING.set(ext.handle());
        ext.action("LAB_ASKING_GO", |_| async {
            let handle = ASKING.get().cloned().ok_or("no handle")?;
            handle
                .host(architect_extensions::hostcall::HostRequest::SetData {
                    key: "seen".to_owned(),
                    value: Some(json!(true)),
                })
                .await
                .map_err(|e| e.to_string())?;
            let seen = handle
                .host(architect_extensions::hostcall::HostRequest::Data {
                    key: "seen".to_owned(),
                })
                .await
                .map_err(|e| e.to_string())?;
            // Not its namespace, and it asks for none.
            let refused = handle
                .host(architect_extensions::hostcall::HostRequest::Run {
                    action: "EXT_LIST".to_owned(),
                    args: Value::Null,
                    ticket: None,
                })
                .await
                .map_err(|e| e.to_string())
                .unwrap_err();
            Ok(json!({ "seen": seen, "refused": refused }))
        });
    }
}

#[test]
fn a_linked_extension_asks_the_host_through_its_handle() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Asking)
        .unwrap();
    ext.install_actions().unwrap();
    let _ = ext.sync();
    let out = block_on(list.run_json_async("LAB_ASKING_GO", b"")).unwrap();
    let out: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(out["seen"], true, "{out}");
    assert!(
        out["refused"]
            .as_str()
            .unwrap()
            .starts_with("refused (actions)"),
        "{out}"
    );
}

/// A linked extension that draws notes of a type, declares that type's
/// favourites, and asks its host what a guest may not have.
struct Noted;

/// The handle of the copy built last (each host builds its own).
static NOTED: std::sync::Mutex<Option<architect_extensions::linked::Handle>> =
    std::sync::Mutex::new(None);

const NOTED_MANIFEST: &str = r#"
id = "lab.noted"
name = "Noted"
version = "0.1.0"
kind = "process"

[entry]
path = "noted-bin"

[capabilities]
reads = ["documents", "files"]
writes = ["documents"]

[[views]]
id = "card"
title = "Card"

[[views]]
id = "more"
title = "More"

[[embeds]]
id = "card"
view = "card"
claims = { note_type = ["Recipe", "menu"] }
fill = true
max_height = 600

[[embeds]]
id = "fence"
view = "more"
claims = { fence = ["cook"] }

[[favourites]]
note_type = "recipe"

[[favourites.tabs]]
id = "card"
label = "Card"
icon = "notebook-text"
target = { view = "card" }
"#;

impl Extension for Noted {
    fn manifest(&self) -> &'static str {
        NOTED_MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        *NOTED.lock().unwrap() = Some(ext.handle());
        ext.view("card", Panel).view("more", Panel);
    }
}

/// The app's own linked extensions say which note types they draw, what
/// they read and which favourites they declare, in the app's process.
#[test]
fn a_linked_extensions_note_views_and_favourites_are_read_in_process() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Noted)
        .unwrap()
        .with_linked(&Linked)
        .unwrap();
    let _ = ext.sync();
    let host = ext.host();
    // Only a claim of a note type by a linked view: the fence is none.
    let views = host.embeds().linked_note_views();
    assert_eq!(views.len(), 1, "{views:?}");
    assert_eq!(views[0].extension, "lab.noted");
    assert_eq!(views[0].view, "card");
    assert_eq!(views[0].note_types, ["Recipe", "menu"]);
    assert_eq!(views[0].reads, ["documents", "files"]);
    assert!(views[0].fill);
    assert_eq!(views[0].max_height, 600);
    let favourites = host.linked_favourites();
    assert_eq!(favourites.len(), 1);
    assert_eq!(favourites[0].by.extension, "lab.noted");
    assert!(favourites[0].favourites.is_for("Recipe"));
    let ids = host.linked_view_ids();
    assert!(
        ids.contains(&(
            "lab.noted".to_owned(),
            vec!["card".to_owned(), "more".to_owned()]
        )),
        "{ids:?}"
    );
    assert!(ids.contains(&("lab.linked".to_owned(), vec!["panel".to_owned()])));

    // A development copy of its id overrides it: its views are not the
    // app's own any more, and nothing here names them.
    let dev = tmp.path().join("dev");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::write(
        dev.join("extension.toml"),
        "id = \"lab.noted\"\nname = \"Noted (dev)\"\nversion = \"0.2.0-dev\"\nkind = \"luau\"\n",
    )
    .unwrap();
    let _ = ext.dev(&dev);
    assert!(host.embeds().linked_note_views().is_empty());
    assert!(host.linked_favourites().is_empty());
    assert!(
        host.linked_view_ids()
            .iter()
            .all(|(id, _)| id != "lab.noted")
    );
}

/// A guest's host: a linked extension that may read and write documents
/// is refused every vault request, and a path it opens goes to the app's
/// own opener alone.
#[test]
fn a_guests_host_refuses_a_linked_extension_the_vault() {
    use architect_extensions::embeds::GuestEmbedPolicy;

    // One host-service call as the extension, through its handle: what
    // its linked view makes.
    let ask = |request: Value| -> Value {
        let handle = NOTED.lock().unwrap().clone().unwrap();
        let request = serde_json::from_value(request).unwrap();
        match block_on(handle.host(request)) {
            Ok(answer) => json!({ "ok": answer }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    };
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let make = |guest: bool| {
        let tmp = tempfile::tempdir().unwrap();
        let host = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
        let host = if guest { host.for_guest() } else { host };
        let seen = std::sync::Arc::clone(&opened);
        // Only what the link lists.
        host.set_path_opener(move |path| {
            if path == "Songs/Joy.md" {
                seen.lock().unwrap().push(path.to_owned());
                Ok(json!({ "opened": path }))
            } else {
                Err("not a note the viewer can open here".to_owned())
            }
        });
        let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
            .with_linked(&Noted)
            .unwrap();
        let _ = ext.sync();
        (tmp, ext)
    };

    let (_tmp, ext) = make(true);
    assert!(ext.host().is_guest());
    for request in [
        json!({ "op": "read_document", "path": "Songs/Joy.md" }),
        json!({ "op": "list_documents" }),
        json!({ "op": "query_vault", "source": "views: []" }),
        json!({ "op": "create_document", "path": "a.md", "text": "x" }),
        json!({ "op": "data", "key": "k" }),
        json!({ "op": "set_data", "key": "k", "value": 1 }),
        json!({ "op": "run", "action": "EXT_LIST" }),
    ] {
        let answer = ask(request.clone());
        let error = answer["error"].as_str().unwrap_or_default();
        assert!(error.starts_with("refused (guest)"), "{request}: {answer}");
    }
    // A listed note opens through the app's opener; an unlisted one does not.
    let answer = ask(json!({ "op": "open", "path": "Songs/Joy.md" }));
    assert_eq!(answer["ok"]["opened"], "Songs/Joy.md", "{answer}");
    let answer = ask(json!({ "op": "open", "path": "Private/Plan.md" }));
    assert!(
        answer["error"]
            .as_str()
            .unwrap_or_default()
            .contains("not a note"),
        "{answer}"
    );
    assert_eq!(*opened.lock().unwrap(), ["Songs/Joy.md"]);
    // The app's linked views draw a guest's content only on this host,
    // and only when the app says so.
    assert!(GuestEmbedPolicy::own_linked().draws_linked(ext.host()));
    assert!(!GuestEmbedPolicy::pages_only().draws_linked(ext.host()));
    assert!(!GuestEmbedPolicy::default().draws_linked(ext.host()));

    // A person's host answers the same extension as before, and never
    // draws a guest's content with a linked view, whatever the policy.
    let (_tmp, ext) = make(false);
    let answer = ask(json!({ "op": "set_data", "key": "k", "value": 1 }));
    assert_eq!(answer, json!({ "ok": null }));
    assert!(!GuestEmbedPolicy::own_linked().draws_linked(ext.host()));
}
