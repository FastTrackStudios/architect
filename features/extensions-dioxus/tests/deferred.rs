//! A deferred linked extension (`architect_extensions_dioxus::deferred`): linked by its
//! manifest alone, its views and actions there from the start, its code
//! built the first time one of them is used, once.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};

use architect_extensions::linked::{Builder, Extension};
use architect_extensions_dioxus::deferred::{Defer, Deferred, Loading};
use dioxus::prelude::*;
use futures::executor::block_on;
use serde_json::json;

const MANIFEST: &str = r#"
id = "lab.deferred"
name = "Deferred"
version = "0.1.0"
kind = "process"

[entry]
path = "deferred-bin"

[[actions]]
id = "LAB_DEFERRED_ECHO"
name = "Echo"

[[views]]
id = "panel"
title = "Panel"

[[views]]
id = "other"
title = "Other"
"#;

static BUILT: AtomicUsize = AtomicUsize::new(0);

#[component]
fn Panel() -> Element {
    rsx! { "the panel" }
}

struct Real;

impl Extension for Real {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        BUILT.fetch_add(1, Ordering::SeqCst);
        ext.view("panel", Panel)
            .view("other", Panel)
            .action("LAB_DEFERRED_ECHO", |args| async move { Ok(args) });
    }
}

struct Lazy;

impl Defer for Lazy {
    fn manifest() -> &'static str {
        MANIFEST
    }

    fn load() -> Loading {
        Box::pin(async { Builder::of(&Real) })
    }
}

#[test]
fn its_views_and_actions_are_there_before_its_code_and_its_code_is_built_once_on_first_use() {
    let linked = Builder::of(&Deferred::<Lazy>::NEW);
    assert_eq!(BUILT.load(Ordering::SeqCst), 0, "linking builds nothing");
    assert!(linked.view_root("panel").is_some());
    assert!(linked.view_root("other").is_some());
    let echo = linked.action_fn("LAB_DEFERRED_ECHO").expect("its action");
    let answer = block_on(echo(json!({"say": "hi"}), json!({})));
    assert_eq!(answer.unwrap(), json!({"say": "hi"}));
    assert_eq!(BUILT.load(Ordering::SeqCst), 1, "built on first use");
    let _ = block_on(echo(json!(1), json!({})));
    assert_eq!(BUILT.load(Ordering::SeqCst), 1, "and only once");
}
