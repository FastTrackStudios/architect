//! `use_input_processor`'s handle, driven from outside the Dioxus runtime
//! the way a capture-phase document listener drives it.
#![cfg(feature = "dioxus")]

use std::cell::RefCell;
use std::time::Duration;

use architect_input::dioxus::{InputHandle, use_input_processor};
use architect_input::{
    InputCommand, InputContext, InputEvent, InputProcessor, KeyChord, KeyCode, KeyEvent, KeyTrie,
    ModeId, Modifiers,
};
use dioxus::core::NoOpMutations;
use dioxus::prelude::{Element, VNode, VirtualDom, use_hook};

thread_local! {
    static HANDLE: RefCell<Option<InputHandle>> = const { RefCell::new(None) };
}

fn space() -> KeyChord {
    KeyChord::plain(KeyCode::char(' '))
}

fn press(chord: &KeyChord) -> InputEvent {
    InputEvent::Key(KeyEvent {
        key: chord.key.clone(),
        modifiers: Modifiers::NONE,
    })
}

/// `space space` searches, `g g` goes to the top; a short timeout.
fn keys() -> InputProcessor {
    let g = KeyChord::plain(KeyCode::char('g'));
    let mut normal = KeyTrie::new();
    normal.bind_sequence(&[space(), space()], "SEARCH_ALL".into());
    normal.bind_sequence(&[g.clone(), g], "CURSOR_TOP".into());
    let mut proc = InputProcessor::new();
    proc.set_keymap(ModeId::normal(), normal);
    proc.set_timeout(Duration::from_millis(20));
    proc
}

#[allow(non_snake_case)]
fn App() -> Element {
    let handle = use_input_processor(keys);
    use_hook(move || HANDLE.with(|slot| *slot.borrow_mut() = Some(handle)));
    VNode::empty()
}

/// Mount the app and hand back its handle; every call after this runs
/// outside the runtime.
// `allow-expect-in-tests` reaches `#[test]` fns only, not this fixture.
#[allow(clippy::expect_used)]
fn mount() -> (VirtualDom, InputHandle) {
    let mut dom = VirtualDom::new(App);
    dom.rebuild_in_place();
    let handle = HANDLE
        .with(|slot| slot.borrow_mut().take())
        .expect("mounted");
    (dom, handle)
}

fn actions(commands: &[InputCommand]) -> Vec<&str> {
    commands
        .iter()
        .filter_map(|cmd| cmd.action().map(String::as_str))
        .collect()
}

#[test]
fn space_space_with_releases_fires_once_outside_the_runtime() {
    let (_dom, keys) = mount();
    let ctx = InputContext::new();

    assert!(actions(&keys.process_event(press(&space()), &ctx)).is_empty());
    assert!(keys.pending_display().is_some(), "the leader is pending");
    assert!(!keys.notify_key_release(&space()));
    assert_eq!(
        actions(&keys.process_event(press(&space()), &ctx)),
        vec!["SEARCH_ALL"]
    );
    assert!(!keys.notify_key_release(&space()));
    assert_eq!(keys.pending_display(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn the_sequence_times_out_when_armed_outside_the_runtime() {
    let (mut dom, keys) = mount();
    let g = KeyChord::plain(KeyCode::char('g'));
    let _ = keys.process_event(press(&g), &InputContext::new());
    assert_eq!(keys.pending_display(), Some("g".to_owned()));

    tokio::time::timeout(Duration::from_secs(5), async {
        // Nothing renders the mirror, so `wait_for_work` may not return
        // once the timer task is done: poll it in short slices.
        while keys.pending_display().is_some() {
            let _ = tokio::time::timeout(Duration::from_millis(10), dom.wait_for_work()).await;
            dom.render_immediate(&mut NoOpMutations);
        }
    })
    .await
    .expect("the timer expires the pending `g`");
    assert!(!keys.processor().needs_timeout());
}
