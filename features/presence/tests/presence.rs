//! The model: Dock and tray menus resolved from the commands, the
//! window's state machine, the login items' files, a second launch's
//! hand-off.

use std::path::Path;

use architect_commands::{Command, Kind, Live, Menu, Pick, Standard};
use architect_presence::login::{installed, plan};
use architect_presence::{
    Effect, Event, Lifecycle, LoginItem, MenuTree, Os, Place, Plan, Presence, QUIT, SHOW_WINDOW,
    TOGGLE_BACKGROUND, TOGGLE_LOGIN,
};

fn commands() -> Vec<Command> {
    vec![
        Command::new("LAB_CAPTURE", "Capture").shortcut("⇧⌘N"),
        Command::new(TOGGLE_BACKGROUND, "Keep running").checked(true),
        Command::new(SHOW_WINDOW, "Open the window"),
        Command::new(QUIT, "Quit"),
        Command::new("LAB_GO_INBOX", "Go to Inbox"),
        Command::new("LAB_GO_VAULT", "Go to Vault").disabled("no vault open"),
    ]
}

#[test]
fn a_dock_menu_is_an_action_list_menu_ticked_live_and_greyed() {
    let menu = Menu::new("Dock")
        .command("LAB_CAPTURE")
        .command("NOT_THERE")
        .separator()
        .section("Tabs", || {
            vec![
                Live::new("Inbox", "LAB_GO_INBOX").checked(true),
                Live::new("Vault", "LAB_GO_VAULT").checked(false),
                Live::new("Gone", "LAB_GO_NOWHERE"),
            ]
        })
        .separator()
        .standard(Standard::Quit)
        .command(TOGGLE_BACKGROUND);
    let tree = MenuTree::resolve(&menu, &commands());
    let rows: Vec<(&str, bool, Option<bool>)> = tree
        .rows()
        .iter()
        .map(|r| (r.label.as_str(), r.enabled, r.checked))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Capture", true, None),
            ("Inbox", true, Some(true)),
            ("Vault", false, Some(false)),
            ("Keep running", true, Some(true)),
        ],
        "a missing command and a row whose command is gone are left out; the platform's Quit too"
    );
    let kinds: Vec<bool> = tree
        .items()
        .iter()
        .map(|n| matches!(n.kind, Kind::Separator))
        .collect();
    assert_eq!(kinds, vec![false, true, false, false, true, false]);
    // Picking a row runs its command; a greyed one runs nothing.
    let inbox = tree.items().get(2).map(|n| n.id).unwrap_or_default();
    assert_eq!(
        tree.tree.pick(inbox),
        Some(&Pick {
            command: "LAB_GO_INBOX".into(),
            args: None
        })
    );
    let vault = tree.items().get(3).map(|n| n.id).unwrap_or_default();
    assert_eq!(tree.tree.pick(vault), None);
}

#[test]
fn the_standard_menu_lists_what_the_presence_declares() {
    let ids = |p: &Presence| -> Vec<String> {
        p.standard_menu()
            .entries
            .iter()
            .filter_map(|e| match e {
                architect_commands::Entry::Command(id) => Some(id.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        ids(&Presence::new("a.b", "App")),
        vec![SHOW_WINDOW.to_owned(), QUIT.to_owned()]
    );
    let all = Presence::new("a.b", "App").background(true).open_at_login();
    assert!(ids(&all).contains(&TOGGLE_BACKGROUND.to_owned()));
    assert!(ids(&all).contains(&TOGGLE_LOGIN.to_owned()));
    let empty = MenuTree::resolve(&Menu::new("Dock").command("NOT_THERE"), &commands());
    assert!(empty.is_empty());
}

#[test]
fn closing_hides_in_background_mode_and_quits_without_it() {
    let mut on = Lifecycle::new(true);
    assert_eq!(
        on.handle(Event::Launched { background: false }),
        vec![Effect::DockIcon(true)]
    );
    assert_eq!(
        on.handle(Event::CloseRequested),
        vec![Effect::HideWindow, Effect::DockIcon(false)]
    );
    assert_eq!(on.place, Place::Background);
    // The Dock icon is gone, so a reopen is the tray's Open or a launch.
    assert_eq!(
        on.handle(Event::Open),
        vec![Effect::DockIcon(true), Effect::ShowWindow, Effect::Focus]
    );
    assert_eq!(on.place, Place::Window);
    assert_eq!(on.handle(Event::Quit), vec![Effect::Quit]);

    let mut off = Lifecycle::new(false);
    assert_eq!(off.handle(Event::CloseRequested), vec![Effect::Close]);
    assert_eq!(off.place, Place::Window);
    // Turned on later, the next close hides.
    assert!(off.handle(Event::KeepRunning(true)).is_empty());
    assert_eq!(
        off.handle(Event::CloseRequested),
        vec![Effect::HideWindow, Effect::DockIcon(false)]
    );
}

#[test]
fn a_login_launch_starts_in_the_background_and_a_dock_click_brings_it_back() {
    let mut life = Lifecycle::new(false);
    assert_eq!(
        life.handle(Event::Launched { background: true }),
        vec![Effect::HideWindow, Effect::DockIcon(false)]
    );
    assert_eq!(
        life.handle(Event::Reopen { visible: false }),
        vec![Effect::DockIcon(true), Effect::ShowWindow, Effect::Focus]
    );
    // With the window on screen, AppKit's own reopen does it.
    assert!(life.handle(Event::Reopen { visible: true }).is_empty());
    // Hide is asked for: it hides whatever the setting.
    assert_eq!(
        life.handle(Event::Hide),
        vec![Effect::HideWindow, Effect::DockIcon(false)]
    );
}

#[test]
fn the_login_items_start_the_installed_app_in_the_background() {
    let item = LoginItem {
        id: "app.fasttrackstudio.task".into(),
        name: "Task".into(),
        exe: "/Applications/Task & Co.app/Contents/MacOS/Task".into(),
    };
    let plist = item.launch_agent();
    assert!(plist.contains("<string>app.fasttrackstudio.task</string>"));
    assert!(plist.contains("<string>/Applications/Task &amp; Co.app/Contents/MacOS/Task</string>"));
    assert!(plist.contains("<string>--background</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));

    let linux = LoginItem {
        exe: "/opt/My App/task".into(),
        ..item.clone()
    };
    let entry = linux.autostart_entry();
    assert!(entry.contains("Exec=\"/opt/My App/task\" --background\n"));
    assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=Task\n"));

    let windows = LoginItem {
        exe: r"C:\Program Files\Task\task.exe".into(),
        ..item
    };
    assert_eq!(
        windows.run_value(),
        r#""C:\Program Files\Task\task.exe" --background"#
    );
}

#[test]
fn only_an_installed_copy_opens_at_login() {
    let home = Path::new("/Users/me");
    let mac = |exe: &str| installed(Os::MacOs, Path::new(exe), home, None, None);
    assert!(mac("/Applications/Task.app/Contents/MacOS/Task").is_ok());
    assert!(mac("/Users/me/Applications/Task.app/Contents/MacOS/Task").is_ok());
    assert!(mac("/Volumes/Task/Task.app/Contents/MacOS/Task").is_err());
    assert!(mac("/Users/me/dev/target/debug/task").is_err());

    let home = Path::new("/home/me");
    let linux = |exe: &str, image: Option<&str>| {
        installed(Os::Linux, Path::new(exe), home, image.map(Path::new), None)
    };
    assert!(linux("/usr/bin/task", None).is_ok());
    assert!(linux("/home/me/.local/bin/task", None).is_ok());
    assert!(linux("/home/me/dev/target/debug/task", None).is_err());
    assert_eq!(
        linux("/tmp/.mount_x/task", Some("/home/me/Task.AppImage")),
        Ok("/home/me/Task.AppImage".into()),
        "an AppImage starts the image, not its mount"
    );

    let local = Path::new(r"C:\Users\me\AppData\Local");
    let win = |exe: &str| installed(Os::Windows, Path::new(exe), home, None, Some(local));
    assert!(win(r"C:\Program Files\Task\task.exe").is_ok());
    assert!(win(r"C:\dev\target\debug\task.exe").is_err());
}

#[test]
fn a_login_item_is_rewritten_when_the_app_moves() {
    assert_eq!(plan(Some("a"), None), Plan::Write("a".into()));
    assert_eq!(plan(Some("a"), Some("a")), Plan::Keep);
    assert_eq!(plan(Some("b"), Some("a")), Plan::Write("b".into()));
    assert_eq!(plan(None, Some("a")), Plan::Remove);
    assert_eq!(plan(None, None), Plan::Keep);
}

/// A second launch hands its arguments to the first over the instance
/// socket, and ends.
#[cfg(all(unix, feature = "native"))]
#[test]
fn a_second_launch_hands_over_to_the_first() {
    use architect_presence::Handoff;
    use architect_presence::instance::{Claim, claim_at};
    use futures_util::StreamExt as _;

    let path = std::env::temp_dir().join(format!("presence-test-{}.instance", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let first = claim_at(&path, &Handoff::default()).unwrap();
    let Claim::First { mut handoffs } = first else {
        panic!("the first launch claims it");
    };
    let second = Handoff {
        args: vec!["lab://vault".into()],
        cwd: Some("/".into()),
        urls: vec!["lab://vault".into()],
    };
    assert!(matches!(claim_at(&path, &second).unwrap(), Claim::Running));
    let heard = futures_executor_block_on(handoffs.next());
    assert_eq!(heard, Some(second));

    // A socket left by a copy that died is taken over.
    drop(handoffs);
    let _ = std::fs::remove_file(&path);
    std::os::unix::net::UnixListener::bind(&path)
        .map(drop)
        .unwrap();
    assert!(matches!(
        claim_at(&path, &Handoff::default()).unwrap(),
        Claim::First { .. }
    ));
    let _ = std::fs::remove_file(&path);
}

/// A future run to its end on this thread.
#[cfg(all(unix, feature = "native"))]
fn futures_executor_block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park_timeout(std::time::Duration::from_secs(5));
    }
}
