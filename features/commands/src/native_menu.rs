//! The native menu bar (feature `native`, macOS): a [`MenuBar`] drawn by
//! `AppKit` through `muda`, rebuilt when what it shows changes (a label, a
//! key, whether one can run or is ticked, a live section's rows), each
//! pick running its command. It is resolved again when the Action Lists'
//! events move on ([`Commands::heard`](crate::Commands::heard)) and when
//! anything a section reads (a signal) changes.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, IsMenuItem, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::menu::{Kind, MenuBar, Resolved, Standard, Tree};
use crate::ui::try_use_commands;

/// How often a pick, or an Action List event, is looked for (the menu's
/// events come on a channel; the list's are counted).
const POLL: Duration = Duration::from_millis(30);

/// The bar `AppKit` shows: what it was built from, the numbered tree its
/// item ids name, and the menu, which must outlive each build.
type Held = Rc<RefCell<Option<(Vec<Resolved>, Tree, muda::Menu)>>>;

/// Draws `bar` as the app's menu bar while mounted (once, at the root).
#[component]
pub fn NativeMenus(bar: MenuBar) -> Element {
    let commands = try_use_commands();
    let held: Held = use_hook(|| Rc::new(RefCell::new(None)));
    use_hook(move || {
        if let Some(c) = commands {
            c.set_global_menu(true);
        }
    });
    use_drop(move || {
        if let Some(c) = commands {
            c.set_global_menu(false);
        }
    });
    if let Some(c) = commands {
        let resolved = bar.resolve(&c.all());
        let stale = held
            .borrow()
            .as_ref()
            .is_none_or(|(last, _, _)| *last != resolved);
        if stale {
            let tree = Tree::new(&resolved);
            let menu = build(&tree);
            menu.init_for_nsapp();
            *held.borrow_mut() = Some((resolved, tree, menu));
        }
    }
    // Picks: the channel is read on the UI's own loop, so a command's
    // handler runs where its signals live.
    use_future(move || {
        let held = held.clone();
        async move {
            let mut heard = commands.map_or(0, |c| c.heard());
            loop {
                futures_timer::Delay::new(POLL).await;
                let Some(c) = commands else {
                    continue;
                };
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    let pick = event.id().0.parse::<i32>().ok().and_then(|id| {
                        held.borrow()
                            .as_ref()
                            .and_then(|(_, tree, _)| tree.pick(id).cloned())
                    });
                    if let Some(pick) = pick {
                        let _ = c.run_pick(&pick);
                    }
                }
                let now = c.heard();
                if now != heard {
                    heard = now;
                    c.refresh();
                }
            }
        }
    });
    rsx! {}
}

fn build(tree: &Tree) -> muda::Menu {
    let menu = muda::Menu::new();
    for node in &tree.children {
        if let Kind::Menu { title } = &node.kind {
            let _ = menu.append(&submenu(title, node));
        }
    }
    menu
}

fn submenu(title: &str, node: &Tree) -> Submenu {
    let sub = Submenu::new(title, true);
    for child in &node.children {
        let id = child.id.to_string();
        let _ = match &child.kind {
            Kind::Row(row) => {
                let key = accelerator(&row.shortcut);
                match row.checked {
                    Some(on) => sub.append(&CheckMenuItem::with_id(
                        id,
                        &row.label,
                        row.enabled,
                        on,
                        key,
                    )),
                    None => sub.append(&MenuItem::with_id(id, &row.label, row.enabled, key)),
                }
            }
            Kind::Separator => sub.append(&PredefinedMenuItem::separator()),
            Kind::Standard(s) => sub.append(standard(*s).as_ref()),
            Kind::Menu { title } => sub.append(&submenu(title, child)),
            Kind::Root => Ok(()),
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
