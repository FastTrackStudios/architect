//! The shell's palette slot (#381), over a mounted `Commands`: ⌘K is not
//! the shell's but a key an extension binds; an overlay shows only when a
//! view draws it; a contributed source's rows join the commands once
//! something is typed, and picking one runs its source's action.
#![cfg(feature = "ui")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_commands::engine::Item;
use architect_commands::kit::Pick;
use architect_commands::{
    Commands, FnSource, Overlay, PaletteView, Query, Ranking, try_use_commands, use_action_list,
    use_commands_provider,
};
use dioxus::prelude::*;

static OPEN: ActionMeta = ActionMeta {
    id: "LAB_PALETTE_OPEN",
    trait_name: "",
    method_name: "",
    display_name: "Show every command",
    description: "",
    category: "",
    group: "Help",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static OPENED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static COMMANDS: RefCell<Option<Commands>> = const { RefCell::new(None) };
}

#[allow(clippy::unwrap_used)]
fn actions() -> Arc<ActionList> {
    let list = ActionList::new();
    list.register(&OPEN, Effect::ReadOnly, |(): ()| {
        OPENED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    Arc::new(list)
}

#[allow(non_snake_case)]
fn App() -> Element {
    let _ = use_commands_provider();
    use_action_list(use_hook(actions));
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

#[test]
#[allow(clippy::too_many_lines)]
fn the_palette_is_a_slot_its_keys_and_sources_are_contributed() {
    let (dom, commands) = mount();
    dom.in_runtime(|| {
        // No palette extension: ⌘K is nobody's, and nothing draws it.
        assert!(!commands.handle_chord("⌘K"), "⌘K is not the shell's");
        assert!(!commands.has_palette(Overlay::Palette));
        assert_eq!(commands.palette_owner(Overlay::Palette), None);

        // An extension binds it (`[[keys]]`), ⌘K before ⌃K: both run its
        // action, ⌘K is the one shown.
        commands.set_bound_keys(vec![
            ("LAB_PALETTE_OPEN".to_owned(), "⌘K".to_owned()),
            ("LAB_PALETTE_OPEN".to_owned(), "⌃K".to_owned()),
        ]);
        assert!(commands.handle_chord("⌘K"));
        assert!(commands.handle_chord("⌃K"));
        assert_eq!(OPENED.load(Ordering::SeqCst), 2);
        let shown = commands
            .all()
            .into_iter()
            .find(|c| c.id == "LAB_PALETTE_OPEN")
            .map(|c| c.shortcut);
        assert_eq!(shown.as_deref(), Some("⌘K"));

        // Its view draws the overlay: now it shows; Esc closes it.
        let draw: Rc<dyn Fn() -> Element> = Rc::new(|| rsx! {});
        commands.set_palette_views(vec![PaletteView {
            overlay: Overlay::Palette,
            owner: "command-palette".to_owned(),
            draw,
        }]);
        assert!(commands.has_palette(Overlay::Palette));
        assert!(!commands.has_palette(Overlay::QuickOpen));
        assert_eq!(
            commands.palette_owner(Overlay::Palette).as_deref(),
            Some("command-palette")
        );
        commands.show(Overlay::Palette);
        assert_eq!(*commands.overlay.peek(), Some(Overlay::Palette));
        assert!(commands.handle_chord("Esc"));
        assert_eq!(*commands.overlay.peek(), None);

        // A contributed source: its rows join once something is typed.
        let ran = Rc::new(RefCell::new(Vec::<String>::new()));
        let kept = Rc::clone(&ran);
        commands.register_source(
            FnSource::new("tours", "Tours", |_| {
                vec![Item::new("welcome", "Welcome tour").detail("the basics")]
            })
            .ranking(Ranking::Open),
            move |item: &Item| kept.borrow_mut().push(item.id.clone()),
        );
        assert_eq!(commands.source_ids(), ["tours"]);
        let empty = commands.palette_rows(&Query::parse(""));
        assert!(
            empty.iter().all(|(r, _)| r.item.source.is_empty()),
            "an empty palette is the commands"
        );
        let rows = commands.palette_rows(&Query::parse("welcome"));
        let (row, pick) = rows
            .iter()
            .find(|(r, _)| r.item.source == "tours")
            .expect("the tour");
        assert_eq!(row.item.group, "Tours", "its source's title beside it");
        // Its prefix asks only it.
        let only = commands.palette_rows(&Query::parse("tours: wel"));
        assert_eq!(only.len(), 1);
        commands.pick(pick);
        assert_eq!(*ran.borrow(), ["welcome"]);
        assert!(matches!(pick, Pick::Item(_)));
        commands.remove_source("tours");
        assert!(commands.source_ids().is_empty());

        // The extension goes: its keys and views with it.
        commands.set_bound_keys(Vec::new());
        commands.set_palette_views(Vec::new());
        assert!(!commands.handle_chord("⌘K"));
        assert!(!commands.has_palette(Overlay::Palette));
        assert_eq!(Overlay::parse("quick-open"), Some(Overlay::QuickOpen));
        assert_eq!(Overlay::QuickOpen.name(), "quick-open");
    });
}
