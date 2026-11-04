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
