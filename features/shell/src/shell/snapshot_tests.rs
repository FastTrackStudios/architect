//! The shell's default interface, as Dioxus writes it: every element,
//! attribute, text and listener `AppShell` makes on a desk and a phone,
//! recorded. `default_chrome.golden` was recorded from the shell before
//! its regions became slots (`git show 5528d33:features/shell`): with no
//! chrome above it, and with an empty one, the shell writes exactly that.
//! Recorded again when a page could draw the title row (`use_title_row`):
//! every attribute, text and listener as before; only how Dioxus splits
//! the elements into templates moved (the ribbon's and the sides' optional
//! piece of a page's title row).

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

thread_local! {
    static KIND: std::cell::RefCell<Option<Signal<&'static str>>> = const { std::cell::RefCell::new(None) };
    static SEEN_SHELL: std::cell::RefCell<Option<Shell>> = const { std::cell::RefCell::new(None) };
}

/// Keeps the shell around it for the test to read.
#[component]
fn Seen() -> Element {
    let shell = use_shell();
    SEEN_SHELL.with(|s| *s.borrow_mut() = Some(shell));
    rsx! { div { "the page" } }
}

/// A page with no use for the sidebar.
#[component]
fn Folding() -> Element {
    use_sidebar_layout(SidebarLayout::Rail);
    rsx! { Seen {} }
}

fn switching_shell() -> Element {
    let kind = use_signal(|| "inbox");
    KIND.with(|k| *k.borrow_mut() = Some(kind));
    rsx! {
        AppShell {
            screen: DESK,
            tabs: vec![
                Tab::new("inbox", "Inbox", crate::icon!(Inbox)),
                Tab::new("vault", "Vault", crate::icon!(Check)),
            ],
            current: Some(kind()),
            on_tab: |_| (),
            left: rsx! { div { "the left side" } },
            if kind() == "vault" { Folding {} } else { Seen {} }
        }
    }
}

#[test]
fn a_page_says_how_a_desk_sidebar_opens_and_the_toggle_is_kept_per_page_kind() {
    let mut dom = VirtualDom::new(switching_shell);
    dom.rebuild_in_place();
    let folded = |dom: &mut VirtualDom| {
        dom.render_immediate(&mut dioxus::core::NoOpMutations);
        let shell = SEEN_SHELL.with(|s| s.borrow().clone()).unwrap();
        dom.in_runtime(|| shell.folded())
    };
    let go = |dom: &mut VirtualDom, to: &'static str| {
        let mut kind = KIND.with(|k| k.borrow().unwrap());
        dom.in_runtime(|| kind.set(to));
    };
    let toggle = |dom: &mut VirtualDom| {
        let shell = SEEN_SHELL.with(|s| s.borrow().clone()).unwrap();
        dom.in_runtime(|| shell.toggle(Side::Left));
    };
    assert!(
        !folded(&mut dom),
        "the inbox has a sidebar, open on a wide desk"
    );
    go(&mut dom, "vault");
    assert!(folded(&mut dom), "the vault asks for it folded");
    toggle(&mut dom);
    assert!(!folded(&mut dom), "the person's toggle wins");
    go(&mut dom, "inbox");
    assert!(!folded(&mut dom));
    toggle(&mut dom);
    assert!(folded(&mut dom), "folded on the inbox");
    go(&mut dom, "vault");
    assert!(!folded(&mut dom), "the vault kept its own: open");
    go(&mut dom, "inbox");
    assert!(folded(&mut dom), "the inbox kept its own: folded");
}

fn bare_desk() -> Element {
    rsx! {
        AppShell {
            screen: DESK,
            tabs: vec![Tab::new("inbox", "Inbox", crate::icon!(Inbox))],
            current: Some("inbox"),
            on_tab: |_| (),
            account: Account::new("Cody Wright").menu(Menu::new().item("Settings", crate::icon!(Check), |()| ())),
            workspaces: Workspaces::new("Workspace", |_| ())
                .with(Workspace::new("personal", "Personal"))
                .current(Some("personal")),
            Seen {}
        }
    }
}

#[test]
fn a_desk_page_with_nothing_for_the_sidebar_folds_it_into_the_ribbon() {
    let mut dom = VirtualDom::new(bare_desk);
    let mut out = Record::default();
    dom.rebuild(&mut out);
    let shell = SEEN_SHELL.with(|s| s.borrow().clone()).unwrap();
    assert!(dom.in_runtime(|| shell.folded()), "nothing in it: folded");
    let html = out.0;
    // The switcher and the account as marks on the ribbon, not rows in a
    // sidebar; the sidebar's button still there to open it.
    assert!(
        html.contains("\"Switch Workspace: Personal\""),
        "the switcher's mark"
    );
    assert!(
        html.contains("\"Account: Cody Wright\""),
        "the account's mark"
    );
    assert!(
        !html.contains("attr aria-label None \"Account\"\n"),
        "no account row"
    );
    assert!(html.contains("\"Toggle the sidebar\""));

    // Opened, they stay in the ribbon: a desk's account, switcher and
    // foot buttons never move into the sidebar (#353).
    dom.in_runtime(|| shell.toggle(Side::Left));
    assert!(!dom.in_runtime(|| shell.folded()), "opened");
    let mut out = Record::default();
    dom.render_immediate(&mut out);
    assert!(
        !out.0.contains("attr aria-label None \"Account\"\n"),
        "no account row in the sidebar"
    );
}

/// What an icon named in data draws (`named_icon`), as the mutations.
fn drawn_icon(name: &'static str) -> String {
    thread_local!(static NAME: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") });
    NAME.with(|n| n.set(name));
    record(|| {
        let icon = named_icon(NAME.with(std::cell::Cell::get));
        icon(Icon {
            size: 16,
            color: "currentColor".into(),
        })
    })
}

#[test]
fn lab_outline_s_indent_icons_are_known_not_a_puzzle_piece() {
    let puzzle = drawn_icon("no-such-icon");
    for name in ["indent-increase", "indent-decrease"] {
        assert_ne!(drawn_icon(name), puzzle, "{name}");
    }
    assert_ne!(drawn_icon("indent-increase"), drawn_icon("indent-decrease"));
}

/// A page that prefers two columns.
#[component]
fn TwoColumns() -> Element {
    use_sidebar_layout(SidebarLayout::Two);
    rsx! { Seen {} }
}

fn two_desk() -> Element {
    rsx! {
        AppShell {
            screen: DESK,
            tabs: vec![Tab::new("vault", "Vault", crate::icon!(Check))],
            current: Some("vault"),
            on_tab: |_| (),
            left: rsx! { div { "the navigator" } },
            TwoColumns {}
        }
    }
}

#[test]
fn a_page_prefers_a_layout_and_every_layout_stays_a_press_away() {
    let mut dom = VirtualDom::new(two_desk);
    dom.rebuild_in_place();
    let layout = |dom: &mut VirtualDom| {
        dom.render_immediate(&mut dioxus::core::NoOpMutations);
        let shell = SEEN_SHELL.with(|s| s.borrow().clone()).unwrap();
        dom.in_runtime(|| shell.sidebar_layout())
    };
    let set = |dom: &mut VirtualDom, to: SidebarLayout| {
        let shell = SEEN_SHELL.with(|s| s.borrow().clone()).unwrap();
        dom.in_runtime(|| shell.set_sidebar_layout(to));
    };
    assert_eq!(layout(&mut dom), SidebarLayout::Two, "as the page prefers");
    for to in SidebarLayout::ALL {
        set(&mut dom, to);
        assert_eq!(layout(&mut dom), to);
    }
    set(&mut dom, SidebarLayout::Rail);
    assert_eq!(layout(&mut dom), SidebarLayout::Rail);
    // The toggle opens it as the page prefers.
    let shell = SEEN_SHELL.with(|s| s.borrow().clone()).unwrap();
    dom.in_runtime(|| shell.toggle(Side::Left));
    assert_eq!(layout(&mut dom), SidebarLayout::Two);
    assert_eq!(SidebarLayout::Two.columns(), 2);
}

fn tablet(width: f64, height: f64) -> Screen {
    Screen {
        width,
        height,
        touch: true,
        insets: Insets {
            top: 24.0,
            right: 0.0,
            bottom: 20.0,
            left: 0.0,
        },
    }
}

thread_local! {
    static TABLET: std::cell::Cell<(f64, f64)> = const { std::cell::Cell::new((0.0, 0.0)) };
}

/// A tablet's shell at `TABLET`'s size, Vault current.
fn tablet_shell() -> Element {
    let (w, h) = TABLET.with(std::cell::Cell::get);
    rsx! {
        AppShell {
            screen: tablet(w, h),
            tabs: vec![
                Tab::new("inbox", "Inbox", crate::icon!(Inbox)),
                Tab::new("vault", "Vault", crate::icon!(Check)),
            ],
            current: Some("vault"),
            on_tab: |_| (),
            left: rsx! { div { "the left side" } },
            div { "the page" }
        }
    }
}

/// Every way the shell draws its tabs says which is current
/// (`aria-current`) and which tab each is (`data-tab`): a tablet's rail
/// and its docked sidebar's rows as a phone's bar and a desk's ribbon do.
#[test]
fn every_tab_says_which_it_is_and_whether_it_is_current() {
    let current = |style: &str| {
        format!(
            "attr data-tab None \"vault\"\nattr aria-current None \"page\"\nattr style None \"{style}"
        )
    };
    let other = "attr data-tab None \"inbox\"\nattr aria-current None \"false\"";
    let mut seen = (false, false);
    for size in [
        (1366.0, 1024.0),
        (1180.0, 820.0),
        (820.0, 1180.0),
        (744.0, 1133.0),
    ] {
        TABLET.with(|t| t.set(size));
        let r = record(tablet_shell);
        seen.0 |= r.contains(&current("position: relative; height: 58px;"));
        seen.1 |= r.contains(&current("height: 40px;"));
        if r.contains("height: 58px;") || r.contains("height: 40px;") {
            assert!(
                r.contains(other),
                "the other tab is not current at {size:?}"
            );
        }
    }
    assert!(seen.0, "a rail's tab");
    assert!(seen.1, "a docked sidebar's row");
}
