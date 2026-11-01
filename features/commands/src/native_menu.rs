//! The native menu bar (feature `native`, macOS): a [`MenuBar`] drawn by
//! `AppKit` through `muda`, rebuilt when the commands change (a label, a
//! key, whether one can run), each pick running its command.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{IsMenuItem, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::menu::{Item, MenuBar, Resolved, Standard};
use crate::ui::try_use_commands;

/// How often a pick is looked for (the menu's events come on a channel).
const POLL: Duration = Duration::from_millis(30);

/// Draws `bar` as the app's menu bar while mounted (once, at the root).
#[component]
pub fn NativeMenus(bar: MenuBar) -> Element {
    let commands = try_use_commands();
    // The menu AppKit shows must outlive each build; the last one built.
    let held = use_hook(|| Rc::new(RefCell::new(None::<(Vec<Resolved>, muda::Menu)>)));
    if let Some(c) = commands {
        let resolved = bar.resolve(&c.all());
        let stale = held
            .borrow()
            .as_ref()
            .is_none_or(|(last, _)| *last != resolved);
        if stale {
            let menu = build(&resolved);
            #[cfg(target_os = "macos")]
            menu.init_for_nsapp();
            *held.borrow_mut() = Some((resolved, menu));
        }
    }
    // Picks: the channel is read on the UI's own loop, so a command's
    // handler runs where its signals live.
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(POLL).await;
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if let Some(c) = commands {
                    let _ = c.run(&event.id().0);
                }
            }
        }
    });
    rsx! {}
}

fn build(resolved: &[Resolved]) -> muda::Menu {
    let menu = muda::Menu::new();
    for r in resolved {
        let sub = submenu(r);
        let _ = menu.append(&sub);
    }
    menu
}

fn submenu(r: &Resolved) -> Submenu {
    let sub = Submenu::new(&r.title, true);
    for item in &r.items {
        let _ = match item {
            Item::Command(c) => {
                let entry =
                    MenuItem::with_id(c.id.clone(), &c.label, c.enabled, accelerator(&c.shortcut));
                sub.append(&entry)
            }
            Item::Separator => sub.append(&PredefinedMenuItem::separator()),
            Item::Standard(s) => sub.append(standard(*s).as_ref()),
            Item::Submenu(inner) => sub.append(&submenu(inner)),
        };
    }
    sub
}

fn standard(s: Standard) -> Box<dyn IsMenuItem> {
    Box::new(match s {
        Standard::About => PredefinedMenuItem::about(None, None),
        Standard::Services => PredefinedMenuItem::services(None),
        Standard::Hide => PredefinedMenuItem::hide(None),
        Standard::HideOthers => PredefinedMenuItem::hide_others(None),
        Standard::ShowAll => PredefinedMenuItem::show_all(None),
        Standard::Quit => PredefinedMenuItem::quit(None),
        Standard::Undo => PredefinedMenuItem::undo(None),
        Standard::Redo => PredefinedMenuItem::redo(None),
        Standard::Cut => PredefinedMenuItem::cut(None),
        Standard::Copy => PredefinedMenuItem::copy(None),
        Standard::Paste => PredefinedMenuItem::paste(None),
        Standard::SelectAll => PredefinedMenuItem::select_all(None),
        Standard::Minimize => PredefinedMenuItem::minimize(None),
        Standard::Zoom => PredefinedMenuItem::maximize(None),
        Standard::FullScreen => PredefinedMenuItem::fullscreen(None),
        Standard::BringAllToFront => PredefinedMenuItem::bring_all_to_front(None),
        Standard::CloseWindow => PredefinedMenuItem::close_window(None),
    })
}

/// A shortcut as commands show it (`⇧⌘T`, `⌃Tab`, `⌥⌘←`) as the menu's
/// key equivalent; `None` when it has none or one the menu cannot say.
fn accelerator(shortcut: &str) -> Option<Accelerator> {
    if shortcut.is_empty() {
        return None;
    }
    let mut mods = Modifiers::empty();
    let mut rest = shortcut;
    loop {
        let mut chars = rest.chars();
        let flag = match chars.next() {
            Some('⌃') => Modifiers::CONTROL,
            Some('⌥') => Modifiers::ALT,
            Some('⇧') => Modifiers::SHIFT,
            Some('⌘') => Modifiers::SUPER,
            _ => break,
        };
        mods |= flag;
        rest = chars.as_str();
    }
    let code = match rest {
        "Tab" => Code::Tab,
        "Space" => Code::Space,
        "Esc" => Code::Escape,
        "⌫" => Code::Backspace,
        "↵" | "Return" => Code::Enter,
        "←" => Code::ArrowLeft,
        "→" => Code::ArrowRight,
        "↑" => Code::ArrowUp,
        "↓" => Code::ArrowDown,
        "[" => Code::BracketLeft,
        "]" => Code::BracketRight,
        "\\" => Code::Backslash,
        "," => Code::Comma,
        "." => Code::Period,
        "/" => Code::Slash,
        ";" => Code::Semicolon,
        "'" => Code::Quote,
        "-" => Code::Minus,
        "=" => Code::Equal,
        "`" => Code::Backquote,
        key => letter_or_digit(key)?,
    };
    Some(Accelerator::new((!mods.is_empty()).then_some(mods), code))
}

fn letter_or_digit(key: &str) -> Option<Code> {
    let mut chars = key.chars();
    let c = chars.next()?.to_ascii_uppercase();
    if chars.next().is_some() {
        return None;
    }
    Some(match c {
        'A' => Code::KeyA,
        'B' => Code::KeyB,
        'C' => Code::KeyC,
        'D' => Code::KeyD,
        'E' => Code::KeyE,
        'F' => Code::KeyF,
        'G' => Code::KeyG,
        'H' => Code::KeyH,
        'I' => Code::KeyI,
        'J' => Code::KeyJ,
        'K' => Code::KeyK,
        'L' => Code::KeyL,
        'M' => Code::KeyM,
        'N' => Code::KeyN,
        'O' => Code::KeyO,
        'P' => Code::KeyP,
        'Q' => Code::KeyQ,
        'R' => Code::KeyR,
        'S' => Code::KeyS,
        'T' => Code::KeyT,
        'U' => Code::KeyU,
        'V' => Code::KeyV,
        'W' => Code::KeyW,
        'X' => Code::KeyX,
        'Y' => Code::KeyY,
        'Z' => Code::KeyZ,
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        _ => return None,
    })
}
