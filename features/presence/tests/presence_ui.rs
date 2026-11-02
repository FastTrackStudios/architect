//! `AppPresence` mounted over a recording platform: the badge follows the
//! app's state, the Dock menu is drawn from the commands, the window's
//! close and a Dock click go through the state machine, and every part of
//! it is on the Action List.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::ActionList;
use architect_commands::{Command, Menu, use_command, use_commands_provider};
use architect_presence::{
    AppPresence, Badge, Call, Effect, Handoff, Heard, Lifecycle, Presence, Recorder, SHOW_WINDOW,
    STATE, TOGGLE_BACKGROUND, TOGGLE_LOGIN, UsePlatform, close_requested,
};
use dioxus::core::NoOpMutations;
use dioxus::prelude::*;

thread_local! {
    static RECORDER: RefCell<Recorder> = RefCell::new(Recorder::default());
    static LIST: Arc<ActionList> = Arc::new(ActionList::new());
    static INBOX: RefCell<Option<Signal<usize>>> = const { RefCell::new(None) };
    /// Why this copy cannot open at login, when it cannot.
    static REFUSE: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[allow(non_snake_case)]
fn App() -> Element {
    let mut recorder = RECORDER.with(|r| r.borrow().clone());
    recorder.refuse_login = REFUSE.with(|r| r.borrow().clone());
    RECORDER.with(|r| {
        r.borrow_mut()
            .refuse_login
            .clone_from(&recorder.refuse_login);
    });
    use_context_provider(|| UsePlatform(Rc::new(recorder)));
    let list = LIST.with(Arc::clone);
    use_context_provider(|| list.clone());
    let _ = use_commands_provider();
    use_command(Command::new("LAB_CAPTURE", "Capture"), || {});
    let inbox = use_signal(|| 2_usize);
    use_hook(|| INBOX.with(|s| *s.borrow_mut() = Some(inbox)));
    let presence = use_hook(|| {
        Presence::new("app.test.presence", "Test")
            .dock_menu(
                Menu::new("Dock")
                    .command("LAB_CAPTURE")
                    .separator()
                    .command(TOGGLE_BACKGROUND),
            )
            .background(true)
            .open_at_login()
    });
    rsx! {
        AppPresence { presence, badge: Badge::count(inbox()), background: false }
    }
}

fn settle(dom: &mut VirtualDom) {
    for _ in 0..4 {
        dom.process_events();
        dom.render_immediate(&mut NoOpMutations);
    }
}

fn badges(r: &Recorder) -> Vec<Option<String>> {
    r.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Badge(b) => Some(b),
            _ => None,
        })
        .collect()
}

fn effects(r: &Recorder) -> Vec<Effect> {
    r.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Apply(e) => Some(e),
            _ => None,
        })
        .collect()
}

#[test]
fn the_presence_binds_the_badge_the_menus_the_window_and_the_action_list() {
    let mut dom = VirtualDom::new(App);
    dom.rebuild_in_place();
    settle(&mut dom);
    let r = RECORDER.with(|r| r.borrow().clone());

    // Started, in the window, the Dock icon on.
    assert_eq!(r.calls().first(), Some(&Call::Start));
    assert_eq!(effects(&r), vec![Effect::DockIcon(true)]);

    // The badge follows the Inbox's count, and only when it changes.
    assert_eq!(badges(&r), vec![Some("2".to_owned())]);
    let mut inbox = INBOX.with(|s| *s.borrow()).unwrap();
    dom.in_runtime(|| inbox.set(5));
    settle(&mut dom);
    dom.in_runtime(|| inbox.set(0));
    settle(&mut dom);
    assert_eq!(badges(&r), vec![Some("2".into()), Some("5".into()), None]);

    // The Dock menu, from the commands: the view's own and the presence's
    // toggle, ticked as the setting is.
    let dock = r
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::DockMenu(Some(m)) => Some(m),
            _ => None,
        })
        .next_back()
        .unwrap();
    let rows: Vec<(String, Option<bool>)> = dock
        .rows()
        .iter()
        .map(|r| (r.label.clone(), r.checked))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Capture".to_owned(), None),
            ("Keep running when the window closes".to_owned(), Some(true)),
        ]
    );

    // Every part of it is on the Action List.
    let list = LIST.with(Arc::clone);
    for id in [SHOW_WINDOW, TOGGLE_BACKGROUND, STATE] {
        assert!(list.info(id).is_some(), "{id} is listed");
    }
    assert_eq!(
        list.info(TOGGLE_BACKGROUND).and_then(|i| i.checked),
        Some(true)
    );
    let state = String::from_utf8(list.run_json(STATE, b"null").unwrap()).unwrap();
    assert!(state.contains("\"place\":\"window\""), "{state}");
    assert!(state.contains("\"keep_running\":true"), "{state}");

    // Closing the window hides it: no Dock icon; a Dock click brings both
    // back.
    let before = effects(&r).len();
    let life = RefCell::new(Lifecycle::new(true));
    let events = r.events.borrow().clone().unwrap();
    assert!(close_requested(&life, &r, &events));
    settle(&mut dom);
    assert_eq!(
        effects(&r).get(before..),
        Some(&[Effect::HideWindow, Effect::DockIcon(false)][..])
    );

    // The presence's own state machine: hidden by the Action List, back
    // by a second launch.
    let before = effects(&r).len();
    list.run(architect_presence::HIDE_WINDOW).unwrap();
    settle(&mut dom);
    let state = String::from_utf8(list.run_json(STATE, b"null").unwrap()).unwrap();
    assert!(state.contains("\"place\":\"background\""), "{state}");
    r.hear(Heard::Handoff(Handoff {
        args: vec!["lab://vault".into()],
        ..Handoff::default()
    }));
    settle(&mut dom);
    assert_eq!(
        effects(&r).get(before..),
        Some(
            &[
                Effect::HideWindow,
                Effect::DockIcon(false),
                Effect::DockIcon(true),
                Effect::ShowWindow,
                Effect::Focus,
            ][..]
        )
    );
    let state = String::from_utf8(list.run_json(STATE, b"null").unwrap()).unwrap();
    assert!(state.contains("\"place\":\"window\""), "{state}");

    // The toggle turns background mode off: the menu's tick follows.
    list.run(TOGGLE_BACKGROUND).unwrap();
    settle(&mut dom);
    assert_eq!(
        list.info(TOGGLE_BACKGROUND).and_then(|i| i.checked),
        Some(false)
    );
    // A Dock click with the window on screen is AppKit's to do.
    let before = effects(&r).len();
    r.hear(Heard::Reopen { visible: true });
    settle(&mut dom);
    assert_eq!(effects(&r).len(), before);

    // A pick from the Dock menu runs its command: Hide, from the list.
    let hide = Menu::new("Dock").command(architect_presence::HIDE_WINDOW);
    let tree = architect_presence::MenuTree::resolve(
        &hide,
        &[Command::new(architect_presence::HIDE_WINDOW, "Hide")],
    );
    let id = tree.items().first().map(|n| n.id).unwrap();
    r.hear(Heard::Pick(tree.tree.pick(id).cloned().unwrap()));
    settle(&mut dom);
    let state = String::from_utf8(list.run_json(STATE, b"null").unwrap()).unwrap();
    assert!(state.contains("\"place\":\"background\""), "{state}");
}

#[test]
fn a_copy_that_cannot_open_at_login_greys_the_toggle_with_why() {
    REFUSE.with(|r| *r.borrow_mut() = Some("move the app to Applications".into()));
    let mut dom = VirtualDom::new(App);
    dom.rebuild_in_place();
    settle(&mut dom);
    let list = LIST.with(Arc::clone);
    let info = list.info(TOGGLE_LOGIN).unwrap();
    assert!(!info.enabled);
    assert_eq!(
        info.disabled_reason.as_deref(),
        Some("move the app to Applications")
    );
    assert_eq!(info.checked, Some(false));
    // Off, the item is removed (there is nothing to remove): asked once.
    let r = RECORDER.with(|r| r.borrow().clone());
    assert_eq!(
        r.calls()
            .iter()
            .filter(|c| matches!(c, Call::Login { .. }))
            .count(),
        1
    );
}
