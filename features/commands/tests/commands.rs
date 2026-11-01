//! The palette's matching and order, openers, custom actions, and the
//! menu bar's model.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::rc::Rc;

use architect_commands::{
    Command, Condition, CustomAction, Item, Kind, Live, MenuBar, OpenIn, OpenItem, Opener, Openers,
    Pick, Query, Recents, Runner, Scope, Standard, Step, Tree, fuzzy, rank, without_platform,
};

#[test]
fn fuzzy_finds_characters_in_order_and_lights_them() {
    let m = fuzzy("ctab", "Close tab").expect("a match");
    assert_eq!(m.positions, vec![0, 6, 7, 8]);
    assert!(fuzzy("tabc", "Close tab").is_none());
    assert!(fuzzy("", "anything").is_some());
}

#[test]
fn a_word_start_beats_the_middle_of_a_word() {
    let start = fuzzy("sp", "Split pane").unwrap().score;
    let middle = fuzzy("sp", "Display").unwrap().score;
    assert!(start > middle, "{start} vs {middle}");
}

#[test]
fn a_query_says_what_it_asks_for() {
    assert_eq!(Query::parse("> close").scope, Scope::Commands);
    assert_eq!(Query::parse("close").text, "close");
    assert_eq!(Query::parse("@kick").scope, Scope::Targets);
    assert_eq!(Query::parse("#buffer").text, "buffer");
    assert_eq!(Query::parse("?").scope, Scope::Help);
}

fn commands() -> Vec<Command> {
    vec![
        Command::new("WORKSPACE_CLOSE_TAB", "Close tab")
            .group("Tabs")
            .shortcut("⌘W"),
        Command::new("WORKSPACE_CLOSE_OTHERS", "Close other tabs").group("Tabs"),
        Command::new("WORKSPACE_SPLIT_RIGHT", "Split right").group("Panes"),
        Command::new("WORKSPACE_REOPEN", "Reopen closed tab")
            .group("Tabs")
            .disabled("nothing was closed"),
    ]
}

#[test]
fn ranking_puts_the_best_enabled_match_first() {
    let rows = rank(&commands(), "close", &Recents::default());
    assert_eq!(rows[0].command.id, "WORKSPACE_CLOSE_TAB");
    assert!(rows.iter().all(|r| r.command.id != "WORKSPACE_SPLIT_RIGHT"));
    // The group and the id match too, less than the label.
    let by_group = rank(&commands(), "panes", &Recents::default());
    assert_eq!(by_group[0].command.id, "WORKSPACE_SPLIT_RIGHT");
}

#[test]
fn an_empty_query_lists_recent_first_and_disabled_last() {
    let mut recents = Recents::default();
    recents.touch("WORKSPACE_SPLIT_RIGHT");
    recents.touch("WORKSPACE_REOPEN");
    let rows = rank(&commands(), "", &recents);
    assert_eq!(rows[0].command.id, "WORKSPACE_SPLIT_RIGHT");
    assert_eq!(rows.last().unwrap().command.id, "WORKSPACE_REOPEN");
}

#[test]
fn recents_keep_the_newest_and_drop_the_oldest() {
    let mut r = Recents::new(2);
    r.touch("a");
    r.touch("b");
    r.touch("a");
    r.touch("c");
    assert_eq!(r.ids, vec!["c", "a"]);
    assert_eq!(r.rank("c"), Some(0));
    assert_eq!(r.rank("b"), None);
}

#[test]
fn openers_search_together_or_one_by_its_prefix() {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let mut openers = Openers::new();
    openers.register(Opener::new(
        "note",
        "Notes",
        |_| {
            vec![
                OpenItem::new("note", "n1", "Welcome").recency(2),
                OpenItem::new("note", "n2", "The workspace").recency(9),
            ]
        },
        {
            let opened = opened.clone();
            move |item, _| opened.borrow_mut().push(item.id.clone())
        },
    ));
    openers.register(Opener::new(
        "song",
        "Songs",
        |_| vec![OpenItem::new("song", "s1", "Way Maker")],
        |_, _| {},
    ));
    // Empty: everything, most recent first among equals.
    let all = openers.search("");
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].item.id, "n2");
    // A prefix narrows to one opener.
    let songs = openers.search("song: way");
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].item.title, "Way Maker");
    // Opening goes to the opener that found it.
    assert!(openers.open(&all[0].item, OpenIn::Tab));
    assert_eq!(*opened.borrow(), vec!["n2"]);
}

struct Log {
    ran: Vec<String>,
    checked: Vec<String>,
    fail: Option<String>,
}

impl Runner for Log {
    fn run(&mut self, action: &str, args: &str) -> Result<(), String> {
        if self.fail.as_deref() == Some(action) {
            return Err("no".into());
        }
        self.ran.push(format!("{action}{args}"));
        Ok(())
    }
    fn checked(&self, action: &str) -> bool {
        self.checked.iter().any(|c| c == action)
    }
    fn enabled(&self, _: &str) -> bool {
        true
    }
}

#[test]
fn a_custom_action_runs_its_steps_in_order_with_conditions_and_repeats() {
    let custom = CustomAction::new("CUSTOM_TIDY", "Tidy")
        .step(Step::new("A"))
        .step(Step::new("B").when(Condition::Checked("SNAP".into())))
        .step(Step::new("C").when(Condition::Not(Box::new(Condition::Checked("SNAP".into())))))
        .step(Step::new("D").args("{\"n\":1}").repeat(2));
    let mut log = Log {
        ran: Vec::new(),
        checked: vec!["SNAP".into()],
        fail: None,
    };
    assert_eq!(custom.run(&mut log), Ok(4));
    assert_eq!(log.ran, vec!["A", "B", "D{\"n\":1}", "D{\"n\":1}"]);
}

#[test]
fn a_custom_action_stops_at_the_step_that_fails() {
    let custom = CustomAction::new("CUSTOM_X", "X")
        .step(Step::new("A"))
        .step(Step::new("B"))
        .step(Step::new("C"));
    let mut log = Log {
        ran: Vec::new(),
        checked: Vec::new(),
        fail: Some("B".into()),
    };
    let err = custom.run(&mut log).unwrap_err();
    assert_eq!(err.step, 1);
    assert_eq!(log.ran, vec!["A"]);
}

#[test]
fn a_menu_bar_resolves_against_the_commands_there_are() {
    let bar = MenuBar::standard("Lab")
        .menu("File", |m| {
            m.command("WORKSPACE_CLOSE_TAB")
                .separator()
                .command("NOT_THERE")
        })
        .menu("View", |m| m.group("Panes").separator())
        .menu("Go", |m| m.command("NOT_THERE"));
    let menus = bar.resolve(&commands());
    let titles: Vec<&str> = menus.iter().map(|m| m.title.as_str()).collect();
    // The app's menu always; an empty "Go" and an empty Help go.
    assert_eq!(titles, vec!["Lab", "File", "Edit", "View", "Window"]);
    let file = &menus[1].items;
    // The missing command and the separator before it are dropped.
    assert_eq!(file.len(), 1);
    assert!(matches!(&file[0], Item::Command(c) if c.shortcut == "⌘W"));
    let view = &menus[3].items;
    assert!(matches!(&view[0], Item::Command(c) if c.id == "WORKSPACE_SPLIT_RIGHT"));
    assert!(menus[2].items.contains(&Item::Standard(Standard::Copy)));
}

#[test]
fn a_new_menu_goes_before_window_and_help() {
    let bar = MenuBar::standard("Lab").menu("Go", |m| m.command("A"));
    let titles: Vec<&str> = bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Lab", "File", "Edit", "View", "Go", "Window", "Help"]
    );
}

#[test]
fn a_toggle_resolves_with_its_tick() {
    let bar = MenuBar::standard("Lab").menu("View", |m| m.command("VIEW_GRID"));
    let on = [Command::new("VIEW_GRID", "Grid").checked(true)];
    let menus = bar.resolve(&on);
    let view = menus.iter().find(|m| m.title == "View").unwrap();
    assert!(matches!(&view.items[0], Item::Command(c) if c.checked == Some(true)));
    let plain = [Command::new("VIEW_GRID", "Grid")];
    let menus = bar.resolve(&plain);
    let view = menus.iter().find(|m| m.title == "View").unwrap();
    assert!(matches!(&view.items[0], Item::Command(c) if c.checked.is_none()));
}

/// A live section is listed each time the bar is resolved; a row whose
/// command is not there goes, one whose command cannot run is disabled
/// with its reason.
#[test]
fn a_live_section_is_listed_when_the_bar_is_resolved() {
    let recent = Rc::new(RefCell::new(vec!["a.rpp".to_owned()]));
    let read = Rc::clone(&recent);
    let bar = MenuBar::standard("Lab").menu("File", |m| {
        m.command("PROJECT_SAVE")
            .separator()
            .section("Recent projects", move || {
                read.borrow()
                    .iter()
                    .map(|p| {
                        Live::new(p.clone(), "PROJECT_OPEN").args(format!("{{\"path\":\"{p}\"}}"))
                    })
                    .chain(std::iter::once(Live::new("Gone", "NOT_THERE")))
                    .collect()
            })
    });
    let commands = vec![
        Command::new("PROJECT_SAVE", "Save"),
        Command::new("PROJECT_OPEN", "Open"),
    ];
    let rows = |commands: &[Command]| -> Vec<Live> {
        bar.resolve(commands)
            .into_iter()
            .find(|m| m.title == "File")
            .map(|m| m.items)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| match i {
                Item::Live(row) => Some(row),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        rows(&commands),
        vec![Live::new("a.rpp", "PROJECT_OPEN").args(r#"{"path":"a.rpp"}"#)]
    );
    recent.borrow_mut().push("b.rpp".into());
    assert_eq!(rows(&commands).len(), 2);

    let closed = vec![
        Command::new("PROJECT_SAVE", "Save"),
        Command::new("PROJECT_OPEN", "Open").disabled("a render is running"),
    ];
    let rows = rows(&closed);
    assert!(rows.iter().all(|r| !r.enabled));
    assert_eq!(
        rows[0].disabled_reason.as_deref(),
        Some("a render is running")
    );
    // A section that lists nothing leaves no separator trailing.
    let empty = MenuBar::standard("Lab").menu("File", |m| {
        m.command("PROJECT_SAVE")
            .separator()
            .section("None", Vec::new)
    });
    let file = empty
        .resolve(&commands)
        .into_iter()
        .find(|m| m.title == "File")
        .unwrap();
    assert_eq!(file.items.len(), 1);
}

/// A bar with the same section (a clone) is equal to itself across
/// renders; another function is another section.
#[test]
fn a_section_is_the_function_it_holds() {
    let bar = MenuBar::standard("Lab").menu("View", |m| m.section("Sets", Vec::new));
    assert_eq!(bar.clone(), bar);
    let other = MenuBar::standard("Lab").menu("View", |m| m.section("Sets", Vec::new));
    assert_ne!(other, bar);
}

/// Where the platform draws nothing, its items go, and so do the menus
/// left empty by that, and the separators left over.
#[test]
fn without_the_platform_its_items_go() {
    let bar = MenuBar::standard("Lab").menu("File", |m| m.command("WORKSPACE_CLOSE_TAB"));
    let menus = without_platform(&bar.resolve(&commands()));
    let titles: Vec<&str> = menus.iter().map(|m| m.title.as_str()).collect();
    // The app menu (About … Quit, no settings command here), Edit and
    // Window were all the platform's.
    assert_eq!(titles, vec!["File"]);
    assert!(
        menus
            .iter()
            .flat_map(|m| &m.items)
            .all(|i| !matches!(i, Item::Standard(_) | Item::Separator))
    );
}

/// A numbered tree: the root is 0, the rest in order depth first; a
/// row's pick runs its command (with a live row's arguments), and a
/// disabled row picks nothing.
#[test]
fn a_tree_numbers_the_bar_and_says_what_a_pick_runs() {
    let bar = MenuBar::default()
        .menu("File", |m| {
            m.command("A")
                .separator()
                .section("Recent", || vec![Live::new("x.rpp", "OPEN").args("{}")])
        })
        .menu("View", |m| m.command("B"));
    let commands = vec![
        Command::new("A", "A").shortcut("⌘A"),
        Command::new("B", "B").disabled("not now").checked(false),
        Command::new("OPEN", "Open"),
    ];
    let tree = Tree::new(&bar.resolve(&commands));
    let ids: Vec<i32> = tree.walk().map(|n| n.id).collect();
    assert_eq!(ids, vec![0, 1, 2, 3, 4, 5, 6]);
    assert!(matches!(&tree.find(1).unwrap().kind, Kind::Menu { title } if title == "File"));
    assert!(matches!(tree.find(3).unwrap().kind, Kind::Separator));
    assert_eq!(
        tree.pick(2),
        Some(&Pick {
            command: "A".into(),
            args: None
        })
    );
    assert_eq!(
        tree.pick(4),
        Some(&Pick {
            command: "OPEN".into(),
            args: Some("{}".into())
        })
    );
    assert!(
        matches!(&tree.find(6).unwrap().kind, Kind::Row(r) if r.checked == Some(false) && !r.enabled)
    );
    assert_eq!(tree.pick(6), None);
    assert_eq!(tree.pick(1), None);
    assert_eq!(tree.pick(99), None);
}
