//! The iPadOS menu bar (feature `native`, iOS): the [`MenuBar`] built
//! into `UIKit`'s main menu with `UIMenuBuilder`, shown when an iPad has a
//! keyboard (and from the top edge on iPadOS 26).
//!
//! `UIKit` asks the app delegate to `buildMenuWithBuilder:` whenever the
//! main menu is rebuilt; winit's delegate does not answer, so this adds
//! that method to its class once. Each row is a `UIAction` whose handler
//! queues its [`Pick`] for the UI's loop to run, so no selector has to
//! find a responder. A new tree asks `UIKit` to rebuild
//! (`UIMenuSystem::setNeedsRebuild`). The platform's own items (Undo,
//! Copy, Hide, Quit) are `UIKit`'s: the app's rows go in at the start of
//! the system menu of the same title (File, Edit, View, Window, Help, the
//! app's), and a menu `UIKit` has none of goes before Window.

use std::cell::RefCell;
use std::ffi::CStr;
use std::ptr::NonNull;
use std::time::Duration;

use block2::RcBlock;
use dioxus::prelude::*;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, ProtocolObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_foundation::{NSArray, NSString};
use objc2_ui_kit::{
    UIAction, UIApplication, UIMenu, UIMenuBuilder, UIMenuElement, UIMenuElementAttributes,
    UIMenuElementState, UIMenuIdentifier, UIMenuOptions, UIMenuSystem,
};

use crate::menu::{Kind, MenuBar, Pick, Tree, without_platform};
use crate::ui::try_use_commands;

/// How often a pick, or an Action List event, is looked for.
const POLL: Duration = Duration::from_millis(30);

/// The prefix of the identifiers of the menus this adds.
const IDS: &str = "app.fasttrackstudio.architect.menu";

/// What `UIKit`'s callback reads: the bar to build, its app menu's title,
/// and the picks waiting for the UI's loop.
#[derive(Default)]
struct State {
    tree: Option<Tree>,
    app: String,
    picks: Vec<Pick>,
    installed: bool,
}

thread_local! {
    // UIKit calls back on the main thread, where the UI runs.
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Builds `bar` into the iPadOS menu bar while mounted (once, at the root).
#[component]
pub fn NativeMenus(bar: MenuBar) -> Element {
    let commands = try_use_commands();
    use_hook(move || {
        if let Some(c) = commands {
            // A phone has no menu bar: the same commands are in the
            // palette, so no in-window bar there either.
            c.set_global_menu(true);
        }
        install();
    });
    if let Some(c) = commands {
        let tree = Tree::new(&without_platform(&bar.resolve(&c.all())));
        let app = bar
            .menus
            .first()
            .map(|m| m.title.clone())
            .unwrap_or_default();
        let changed = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let changed = s.tree.as_ref() != Some(&tree) || s.app != app;
            s.tree = Some(tree);
            s.app = app;
            changed
        });
        if changed && let Some(mtm) = MainThreadMarker::new() {
            UIMenuSystem::mainSystem(mtm).setNeedsRebuild();
        }
    }
    use_future(move || async move {
        let mut heard = commands.map_or(0, |c| c.heard());
        loop {
            futures_timer::Delay::new(POLL).await;
            let Some(c) = commands else {
                continue;
            };
            let picks = STATE.with(|s| std::mem::take(&mut s.borrow_mut().picks));
            for pick in picks {
                let _ = c.run_pick(&pick);
            }
            let now = c.heard();
            if now != heard {
                heard = now;
                c.refresh();
            }
        }
    });
    use_drop(move || {
        if let Some(c) = commands {
            c.set_global_menu(false);
        }
        STATE.with(|s| s.borrow_mut().tree = None);
        if let Some(mtm) = MainThreadMarker::new() {
            UIMenuSystem::mainSystem(mtm).setNeedsRebuild();
        }
    });
    rsx! {}
}

/// Adds `buildMenuWithBuilder:` to the app delegate's class, once. A class
/// that has its own keeps it (the app builds its menus itself), and that
/// is logged.
fn install() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if STATE.with(|s| std::mem::replace(&mut s.borrow_mut().installed, true)) {
        return;
    }
    let app = UIApplication::sharedApplication(mtm);
    // SAFETY: read on the main thread, and only to find its class.
    let Some(delegate) = (unsafe { app.delegate() }) else {
        tracing::warn!(
            app_menu.outcome = "no_delegate",
            "ios menu: no app delegate"
        );
        return;
    };
    let object: &AnyObject = (*delegate).as_ref();
    let class: &AnyClass = object.class();
    let types: &CStr = c"v@:@";
    // SAFETY: `build_menu` has the method's signature (an object, its
    // selector, one object argument, returning nothing), which `types`
    // says; an `Imp` is called through that signature by the runtime.
    let added = unsafe {
        let imp: Imp = std::mem::transmute::<
            unsafe extern "C-unwind" fn(
                &AnyObject,
                Sel,
                NonNull<ProtocolObject<dyn UIMenuBuilder>>,
            ),
            Imp,
        >(build_menu);
        objc2::ffi::class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(buildMenuWithBuilder:),
            imp,
            types.as_ptr(),
        )
    };
    tracing::info!(
        app_menu.outcome = if added.as_bool() {
            "installed"
        } else {
            "delegate_has_its_own"
        },
        app_menu.delegate = class.name().to_string_lossy().as_ref(),
        "ios menu"
    );
    UIMenuSystem::mainSystem(mtm).setNeedsRebuild();
}

/// `UIKit`'s `buildMenuWithBuilder:`: the app's menus into the main menu.
unsafe extern "C-unwind" fn build_menu(
    _this: &AnyObject,
    _cmd: Sel,
    builder: NonNull<ProtocolObject<dyn UIMenuBuilder>>,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: UIKit hands a live builder, for the length of this call.
    let builder = unsafe { builder.as_ref() };
    // Only the main menu bar; a context menu's system is not ours.
    if builder.system() != UIMenuSystem::mainSystem(mtm) {
        return;
    }
    let (tree, app) = STATE.with(|s| {
        let s = s.borrow();
        (s.tree.clone(), s.app.clone())
    });
    let Some(tree) = tree else {
        return;
    };
    for node in &tree.children {
        let Kind::Menu { title } = &node.kind else {
            continue;
        };
        let elements = elements(&node.children, mtm);
        let id = NSString::from_str(&format!("{IDS}.{}", node.id));
        if let Some(system) = system_menu(title, &app) {
            let inline = UIMenu::menuWithTitle_image_identifier_options_children(
                &NSString::new(),
                None,
                Some(&id),
                UIMenuOptions::DisplayInline,
                &elements,
                mtm,
            );
            builder.insertChildMenu_atStartOfMenuForIdentifier(&inline, system);
        } else {
            let menu = UIMenu::menuWithTitle_image_identifier_options_children(
                &NSString::from_str(title),
                None,
                Some(&id),
                UIMenuOptions::empty(),
                &elements,
                mtm,
            );
            // SAFETY: a UIKit identifier constant, read on the main thread.
            let window = unsafe { objc2_ui_kit::UIMenuWindow };
            builder.insertSiblingMenu_beforeMenuForIdentifier(&menu, window);
        }
    }
}

/// The system menu `title` names, if `UIKit` has one (the app's own menu by
/// the app's title).
fn system_menu(title: &str, app: &str) -> Option<&'static UIMenuIdentifier> {
    // SAFETY: UIKit's identifier constants, read on the main thread.
    unsafe {
        Some(match title {
            "File" => objc2_ui_kit::UIMenuFile,
            "Edit" => objc2_ui_kit::UIMenuEdit,
            "View" => objc2_ui_kit::UIMenuView,
            "Window" => objc2_ui_kit::UIMenuWindow,
            "Help" => objc2_ui_kit::UIMenuHelp,
            t if t == app => objc2_ui_kit::UIMenuApplication,
            _ => return None,
        })
    }
}

/// A menu's nodes as `UIKit`'s elements. `UIKit` has no separator: the runs
/// between them become inline menus, which it draws apart.
fn elements(nodes: &[Tree], mtm: MainThreadMarker) -> Retained<NSArray<UIMenuElement>> {
    let runs: Vec<&[Tree]> = nodes
        .split(|n| matches!(n.kind, Kind::Separator))
        .filter(|run| !run.is_empty())
        .collect();
    let out: Vec<Retained<UIMenuElement>> = if runs.len() > 1 {
        runs.into_iter()
            .map(|run| {
                let children = elements_of(run, mtm);
                Retained::into_super(UIMenu::menuWithTitle_image_identifier_options_children(
                    &NSString::new(),
                    None,
                    None,
                    UIMenuOptions::DisplayInline,
                    &NSArray::from_retained_slice(&children),
                    mtm,
                ))
            })
            .collect()
    } else {
        elements_of(nodes, mtm)
    };
    NSArray::from_retained_slice(&out)
}

fn elements_of(nodes: &[Tree], mtm: MainThreadMarker) -> Vec<Retained<UIMenuElement>> {
    nodes
        .iter()
        .filter_map(|node| match &node.kind {
            Kind::Row(row) => {
                let pick = row.pick.clone();
                let handler = RcBlock::new(move |_: NonNull<UIAction>| {
                    let pick = pick.clone();
                    STATE.with(|s| s.borrow_mut().picks.push(pick));
                });
                // SAFETY: a live block; UIKit copies it.
                let action = unsafe {
                    UIAction::actionWithTitle_image_identifier_handler(
                        &NSString::from_str(&row.label),
                        None,
                        None,
                        RcBlock::as_ptr(&handler),
                        mtm,
                    )
                };
                if let Some(on) = row.checked {
                    action.setState(if on {
                        UIMenuElementState::On
                    } else {
                        UIMenuElementState::Off
                    });
                }
                if !row.enabled {
                    action.setAttributes(UIMenuElementAttributes::Disabled);
                }
                Some(Retained::into_super(action))
            }
            Kind::Menu { title } => Some(Retained::into_super(
                UIMenu::menuWithTitle_image_identifier_options_children(
                    &NSString::from_str(title),
                    None,
                    None,
                    UIMenuOptions::empty(),
                    &elements(&node.children, mtm),
                    mtm,
                ),
            )),
            Kind::Root | Kind::Separator | Kind::Standard(_) => None,
        })
        .collect()
}
