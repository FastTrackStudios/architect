//! The shell's default interface, as Dioxus writes it: every element,
//! attribute, text and listener `AppShell` makes on a desk and a phone,
//! recorded. `default_chrome.golden` was recorded from the shell before
//! its regions became slots (`git show 5528d33:features/shell`): with no
//! chrome above it, and with an empty one, the shell writes exactly that.

// `icon!` casts its closure to the fn pointer, as an app's does.
#![allow(clippy::as_conversions)]

use dioxus::core::{BorrowedAttributeValue, ElementId, VirtualDom, WriteMutations};

use super::*;
use lucide_dioxus::{Check, Inbox, Undo2};

/// Every mutation, one per line.
#[derive(Default)]
struct Record(String);

impl Record {
    fn line(&mut self, s: impl AsRef<str>) {
        self.0.push_str(s.as_ref());
        self.0.push('\n');
    }
}

impl WriteMutations for Record {
    fn push_id(&mut self, id: ElementId) {
        self.line(format!("push {}", id.raw()));
    }
    fn set_id(&mut self, id: ElementId) {
        self.line(format!("id {}", id.raw()));
    }
    fn child(&mut self, index: usize) {
        self.line(format!("child {index}"));
    }
    fn pop(&mut self) {
        self.line("pop");
    }
    fn create_element(&mut self, tag: &str, ns: Option<&str>) {
        self.line(format!("element {tag} {ns:?}"));
    }
    fn create_text(&mut self, value: &str) {
        self.line(format!("text {value:?}"));
    }
    fn clone(&mut self) {
        self.line("clone");
    }
    fn append_children(&mut self, m: usize) {
        self.line(format!("append {m}"));
    }
    fn replace_with(&mut self, m: usize) {
        self.line(format!("replace {m}"));
    }
    fn insert_after(&mut self, m: usize) {
        self.line(format!("after {m}"));
    }
    fn insert_before(&mut self, m: usize) {
        self.line(format!("before {m}"));
    }
    fn set_attribute(&mut self, name: &str, ns: Option<&str>, value: BorrowedAttributeValue<'_>) {
        let v = match value {
            BorrowedAttributeValue::Text(t) => t.to_owned(),
            BorrowedAttributeValue::Float(f) => f.to_string(),
            BorrowedAttributeValue::Int(i) => i.to_string(),
            BorrowedAttributeValue::Bool(b) => b.to_string(),
            BorrowedAttributeValue::None => "none".to_owned(),
            BorrowedAttributeValue::Any(_) => "any".to_owned(),
        };
        self.line(format!("attr {name} {ns:?} {v:?}"));
    }
    fn set_text(&mut self, value: &str) {
        self.line(format!("set-text {value:?}"));
    }
    fn add_event_listener(&mut self, name: &str) {
        self.line(format!("listen {name}"));
    }
    fn remove_event_listener(&mut self, name: &str) {
        self.line(format!("unlisten {name}"));
    }
    fn remove(&mut self) {
        self.line("remove");
    }
}

fn shell(screen: Screen) -> Element {
    rsx! {
        AppShell {
            screen,
            title: "Lab",
            tabs: vec![
                Tab::new("inbox", "Inbox", crate::icon!(Inbox)).badge(3),
                Tab::new("tasks", "Tasks", crate::icon!(Check)),
            ],
            current: Some("inbox"),
            on_tab: |_| (),
            left: rsx! { div { "the left side" } },
            right: rsx! { div { "the details" } },
            status: vec![
                StatusItem::new("sync", "Synced").hint("Every change is saved"),
                StatusItem::new("people", "3 online"),
            ],
            actions: vec![BarAction::new("LAB_UNDO", "Undo", crate::icon!(Undo2))],
            div { "the page" }
        }
    }
}

const DESK: Screen = Screen {
    width: 1400.0,
    height: 900.0,
    touch: false,
    insets: Insets {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    },
};

const PHONE: Screen = Screen {
    width: 402.0,
    height: 874.0,
    touch: true,
    insets: Insets {
        top: 62.0,
        right: 0.0,
        bottom: 34.0,
        left: 0.0,
    },
};

fn record(app: fn() -> Element) -> String {
    let mut dom = VirtualDom::new(app);
    let mut out = Record::default();
    dom.rebuild(&mut out);
    out.0
}

fn plain() -> String {
    record(|| rsx! { {shell(DESK)} {shell(PHONE)} })
}

/// What the shell writes by default, desk then phone.
const GOLDEN: &str = include_str!("default_chrome.golden");

#[test]
fn the_default_shell_writes_what_it_always_did() {
    if std::env::var_os("ARCHITECT_RECORD_SHELL").is_some() {
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shell/default_chrome.golden"
            ),
            plain(),
        )
        .unwrap();
        return;
    }
    assert!(plain() == GOLDEN, "the default shell changed");
}

thread_local! {
    static CHROME: std::cell::RefCell<Chrome> = std::cell::RefCell::new(Chrome::default());
}

/// The shell under a chrome (`CHROME`'s).
fn chromed() -> String {
    record(|| {
        use_context_provider(|| Signal::new(CHROME.with(|c| c.borrow().clone())));
        rsx! { {shell(DESK)} {shell(PHONE)} }
    })
}

#[test]
fn an_empty_chrome_changes_nothing_and_a_filled_slot_replaces_the_shells_own() {
    CHROME.with(|c| *c.borrow_mut() = Chrome::default());
    assert!(chromed() == GOLDEN, "an empty chrome is the default shell");

    CHROME.with(|c| {
        let mut chrome = Chrome::default();
        chrome.fillers.insert(
            Slot::Status,
            Filler::new("ext:lab.board", "lab.board/status-bar", || {
                rsx! { div { "the board's status bar" } }
            }),
        );
        chrome.tabs.push(Tab::new(
            intern("lab.board/board"),
            "Board",
            named_icon("layout-dashboard"),
        ));
        chrome
            .status
            .push(StatusItem::new("lab.board/pins", "2 pins"));
        *c.borrow_mut() = chrome;
    });
    let filled = chromed();
    assert!(filled.contains("the board's status bar"));
    assert!(
        !filled.contains("\"Synced\""),
        "the shell's own status bar is gone"
    );
    assert!(filled.contains("Board"), "the contributed tab");

    CHROME.with(|c| {
        let mut chrome = Chrome::default();
        chrome.hidden.insert(Slot::Ribbon);
        chrome.hidden.insert(Slot::Status);
        *c.borrow_mut() = chrome;
    });
    let hidden = chromed();
    assert!(!hidden.contains("\"Synced\""));
    assert!(hidden.len() < GOLDEN.len());
    assert_eq!(intern("a/b"), intern(&String::from("a/b")));
    assert!(std::ptr::eq(intern("a/b"), intern("a/b")));
}
