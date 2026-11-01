//! The palette's matching and order, openers, and custom actions.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::rc::Rc;

use architect_commands::{
    Command, Condition, CustomAction, OpenIn, OpenItem, Opener, Openers, Query, Recents, Runner,
    Scope, Step, fuzzy, rank,
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
