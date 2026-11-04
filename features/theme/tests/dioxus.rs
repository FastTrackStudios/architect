//! The store in a Dioxus app: a change reaches `use_themed` slices, and a
//! slice that did not change does not rerun its readers.

// Test helpers fail the test: that is their assertion.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::cell::RefCell;

use architect_theme::{
    FromResolved, Layer, LayerKind, Resolved, ThemeChoice, ThemeStore, use_theme_store, use_themed,
};
use dioxus::prelude::*;

#[derive(Debug, Clone, PartialEq)]
struct Ink(String);

impl FromResolved for Ink {
    fn from_resolved(r: &Resolved) -> Self {
        Self(r.value("ink").to_owned())
    }
}

thread_local! {
    static STORE: RefCell<Option<ThemeStore>> = const { RefCell::new(None) };
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn app() -> Element {
    use_theme_store(|| STORE.with(|s| s.borrow().clone().unwrap()));
    rsx! { Reader {} }
}

#[component]
fn Reader() -> Element {
    let ink = use_themed::<Ink>();
    SEEN.with(|s| s.borrow_mut().push(ink().0));
    rsx! { div { "{ink().0}" } }
}

fn settle(dom: &mut VirtualDom) {
    futures::executor::block_on(async {
        let _ = futures::future::select(
            Box::pin(dom.wait_for_work()),
            Box::pin(futures_timer_free_yield()),
        )
        .await;
    });
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
}

/// A few turns of yielding (each wakes itself), so a ready channel is
/// drained before giving up on more work.
async fn futures_timer_free_yield() {
    for _ in 0..64 {
        let mut yielded = false;
        futures::future::poll_fn(|cx| {
            if yielded {
                std::task::Poll::Ready(())
            } else {
                yielded = true;
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        })
        .await;
    }
}

#[test]
fn a_store_change_reaches_the_slice_and_only_a_real_one_reruns_it() {
    let store = ThemeStore::default();
    STORE.with(|s| *s.borrow_mut() = Some(store.clone()));
    let mut dom = VirtualDom::new(app);
    dom.rebuild_in_place();
    assert_eq!(SEEN.with(|s| s.borrow().clone()), ["#e4e4e7"]);

    // A change outside the slice: the reader does not run again.
    assert!(store.set_token(LayerKind::Person, "radius", "3px"));
    settle(&mut dom);
    assert_eq!(
        SEEN.with(|s| s.borrow().len()),
        1,
        "radius is not the reader's"
    );

    store.set(
        LayerKind::Pick,
        Layer::default().choice(ThemeChoice::One("fasttrackstudio-light".into())),
    );
    settle(&mut dom);
    assert_eq!(
        SEEN.with(|s| s.borrow().last().cloned()).unwrap(),
        "#18181b"
    );
}
