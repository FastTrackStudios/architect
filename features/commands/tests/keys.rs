//! Keys through the keymap, end to end over a mounted `Commands`: a
//! command's shortcut is its binding, the person rebinds it (and the
//! change goes out to be kept and synced, and comes back from another
//! device), sequences, contexts, held-letter tools, custom actions.
#![cfg(feature = "ui")]

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_commands::{
    Commands, CustomAction, EVERYWHERE, PersonKeymap, Step, try_use_commands, use_action_list,
    use_commands_provider, use_keymap,
};
use architect_input::{InputCommand, InputEvent, KeymapConfig, Modifiers, MouseButton, MouseEvent};
use dioxus::prelude::{Callback, Element, VNode, VirtualDom, use_hook};

const fn meta(id: &'static str, label: &'static str, shortcut: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: "",
        display_name: label,
        description: "",
        category: "",
        group: "Tabs",
        shortcut,
        toggleable: false,
        undo: false,
    }
}

static CLOSE: ActionMeta = meta("WORKSPACE_CLOSE_TAB", "Close tab", "⌘W");
static SAVE: ActionMeta = meta("FILE_SAVE_ALL", "Save all", "");
static DUPLICATE: ActionMeta = meta("ARRANGE_DUPLICATE", "Duplicate", "");

static CLOSED: AtomicUsize = AtomicUsize::new(0);
static SAVED: AtomicUsize = AtomicUsize::new(0);
static DUPLICATED: AtomicUsize = AtomicUsize::new(0);

/// The app's own keymap: a sequence, a context's key, a held-letter tool.
const APP_KEYMAP: &str = r#"{
    "keymap": { "normal": { "Meta+g Meta+s": "FILE_SAVE_ALL" } },
    "contexts": {
        "session.arrange": {
            "keys": { "Meta+d": "ARRANGE_DUPLICATE" },
            "mouse": { "Z+Left.Drag": "VIEW_ZOOM_BOX" }
        }
    }
}"#;

thread_local! {
    static COMMANDS: RefCell<Option<Commands>> = const { RefCell::new(None) };
    /// What the app was told to keep (and sync).
    static KEPT: RefCell<Vec<PersonKeymap>> = const { RefCell::new(Vec::new()) };
}

#[allow(clippy::unwrap_used)]
fn actions() -> Arc<ActionList> {
    let list = ActionList::new();
    list.register(&CLOSE, Effect::Mutating, |(): ()| {
        CLOSED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    list.register(&SAVE, Effect::Mutating, |(): ()| {
        SAVED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    list.register(&DUPLICATE, Effect::Mutating, |(): ()| {
        DUPLICATED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    Arc::new(list)
}

#[allow(non_snake_case, clippy::unwrap_used)]
fn App() -> Element {
    let _ = use_commands_provider();
    use_action_list(use_hook(actions));
    use_keymap(
        || KeymapConfig::from_json_str(APP_KEYMAP).unwrap(),
        PersonKeymap::default,
        Some(Callback::new(|person: PersonKeymap| {
            KEPT.with(|kept| kept.borrow_mut().push(person));
        })),
    );
    let commands = try_use_commands();
    use_hook(move || COMMANDS.with(|slot| *slot.borrow_mut() = commands));
    VNode::empty()
}

#[allow(clippy::expect_used)]
fn mount() -> (VirtualDom, Commands) {
    let mut dom = VirtualDom::new(App);
    dom.rebuild_in_place();
    let commands = COMMANDS
        .with(|slot| slot.borrow_mut().take())
        .expect("mounted");
    (dom, commands)
}

fn count(n: &AtomicUsize) -> usize {
    n.load(Ordering::SeqCst)
}

/// One test: the counters are process-wide.
#[test]
fn keys_run_through_the_keymap_and_the_person_rebinds_them() {
    let (dom, commands) = mount();
    dom.in_runtime(|| {
        // The declared shortcut is the binding that runs it.
        let shown = commands
            .all()
            .into_iter()
            .find(|c| c.id == "WORKSPACE_CLOSE_TAB")
            .map(|c| c.shortcut);
        assert_eq!(shown.as_deref(), Some("⌘W"));
        assert!(commands.handle_chord("⌘W"));
        assert_eq!(count(&CLOSED), 1);

        // The person rebinds ⌘W to ⌘E in the action list.
        commands.edit_keymap(|k| {
            k.unbind(EVERYWHERE, "⌘W").unwrap();
            k.bind("WORKSPACE_CLOSE_TAB", EVERYWHERE, "⌘E").unwrap();
        });
        assert!(!commands.handle_chord("⌘W"), "⌘W no longer closes");
        assert!(commands.handle_chord("⌘E"));
        assert_eq!(count(&CLOSED), 2);
        let shown = commands
            .all()
            .into_iter()
            .find(|c| c.id == "WORKSPACE_CLOSE_TAB")
            .map(|c| c.shortcut);
        assert_eq!(
            shown.as_deref(),
            Some("⌘E"),
            "the palette shows the new key"
        );

        // The change went out to be kept, as data that travels.
        let kept = KEPT.with(|kept| kept.borrow().last().cloned()).unwrap();
        assert_eq!(kept.rebinds.len(), 2);
        let synced = PersonKeymap::from_json(&kept.to_json().unwrap()).unwrap();
        assert_eq!(synced, kept);

        // Another device: its layer arrives, and keys follow it.
        commands.set_person_keymap(PersonKeymap::default());
        assert!(commands.handle_chord("⌘W"));
        assert_eq!(count(&CLOSED), 3);
        commands.set_person_keymap(synced);
        assert!(commands.handle_chord("⌘E"));
        assert_eq!(count(&CLOSED), 4);

        // A sequence waits for its second key.
        assert!(commands.handle_chord("⌘G"), "the first key is taken");
        assert_eq!(count(&SAVED), 0);
        assert!(commands.handle_chord("⌘S"));
        assert_eq!(count(&SAVED), 1);

        // A context's key holds only where it is live, and inside it.
        assert!(!commands.handle_chord("⌘D"));
        commands.set_context("session.arrange.item");
        assert!(commands.handle_chord("⌘D"));
        assert_eq!(count(&DUPLICATED), 1);

        // Held Z is the zoom tool over the arrange: the drag comes back
        // with its data for the app's action.
        commands.set_context("session.arrange");
        let down = commands.handle_input(InputEvent::Key(architect_input::KeyEvent {
            key: architect_input::KeyCode::char('z'),
            modifiers: Modifiers::NONE,
        }));
        assert!(down.is_empty(), "the press is the tool's");
        let drag = commands.handle_input(InputEvent::Mouse(MouseEvent::drag(
            MouseButton::Left,
            (0.0, 0.0),
            (120.0, 40.0),
            Modifiers::NONE,
        )));
        assert!(matches!(
            drag.first(),
            Some(InputCommand::Gesture { action, .. }) if action == "VIEW_ZOOM_BOX"
        ));
        assert!(commands.handle_key_up("Z").is_empty());

        // A custom action is an action: run, and bound.
        commands.edit_keymap(|k| {
            k.save_custom(
                CustomAction::new("CUSTOM_CLOSE_TWO", "Close two")
                    .step(Step::new("WORKSPACE_CLOSE_TAB").repeat(2)),
            );
            k.bind("CUSTOM_CLOSE_TWO", EVERYWHERE, "⌘⇧W").unwrap();
        });
        assert!(commands.handle_chord("⇧⌘W"));
        assert_eq!(count(&CLOSED), 6);

        // Reset puts the app's ⌘W back.
        commands.edit_keymap(|k| k.reset("WORKSPACE_CLOSE_TAB"));
        assert!(commands.handle_chord("⌘W"));
        assert_eq!(count(&CLOSED), 7);
    });
}
