//! macOS (feature `native`): the Dock tile (badge, progress, attention),
//! the Dock menu, reopen on a Dock click and URLs to open (the app's
//! `NSApplicationDelegate`: winit registers none, so this one is the
//! app's), the Dock icon on and off (the activation policy), and the
//! status item in the menu bar with a template image.
//!
//! Everything runs on the main thread, inside `AppKit`'s run loop. A pick,
//! a Dock click or a URL goes to the window's thread as a
//! [`Heard`](crate::Heard). The menus are built from the latest resolved
//! tree when they open (`applicationDockMenu:`, `menuNeedsUpdate:`), so
//! they are never stale and nothing polls.

use std::cell::RefCell;

use architect_commands::{Kind, Tree};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSCellImagePosition,
    NSControlStateValueOff, NSControlStateValueOn, NSEventMask, NSEventType, NSImage, NSImageView,
    NSMenu, NSMenuDelegate, NSMenuItem, NSProgressIndicator, NSProgressIndicatorStyle,
    NSRequestUserAttentionType, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{
    NSArray, NSData, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};

use crate::lifecycle::Effect;
use crate::login::Applied;
use crate::model::{Attention, MenuTree, TrayClick};
use crate::native::WindowOps;
use crate::platform::{Events, Heard, Platform, TrayView};

/// Which menu a target serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Dock,
    Tray,
}

/// What the delegate and targets read, on the main thread.
#[derive(Default)]
struct State {
    events: Option<Events>,
    dock: Option<MenuTree>,
    /// The tree the Dock menu on show was built from (its tags name it).
    dock_built: Option<Tree>,
    tray: Option<TrayView>,
    tray_built: Option<Tree>,
    delegate: Option<Retained<Delegate>>,
    dock_target: Option<Retained<Target>>,
    tray_target: Option<Retained<Target>>,
    status: Option<Retained<NSStatusItem>>,
    tray_menu: Option<Retained<NSMenu>>,
    /// The tile's progress bar, while there is one.
    progress: Option<Retained<NSProgressIndicator>>,
    /// What the tile shows, put back when the Dock icon comes back (the
    /// Dock forgets a tile it took away).
    badge: Option<String>,
    value: Option<f64>,
    attention: Option<isize>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn send(heard: Heard) {
    let events = STATE.with(|s| s.borrow().events.clone());
    if let Some(tx) = events {
        let _ = tx.unbounded_send(heard);
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the class adds
    // the application delegate's methods and no ivars.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ArchitectPresenceDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        /// A click on the Dock icon: the window back when there is none on
        /// screen. `true`: AppKit then does its own (brings a minimised
        /// one back).
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, visible: bool) -> bool {
            send(Heard::Reopen { visible });
            true
        }

        /// The Dock menu, built now from the latest tree.
        #[unsafe(method_id(applicationDockMenu:))]
        fn dock_menu(&self, _app: &NSApplication) -> Option<Retained<NSMenu>> {
            dock_menu(MainThreadMarker::from(self))
        }

        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &NSArray<NSURL>) {
            let urls: Vec<String> = urls
                .iter()
                .filter_map(|u| u.absoluteString().map(|s| s.to_string()))
                .collect();
            send(Heard::Handoff(crate::Handoff {
                urls,
                ..crate::Handoff::default()
            }));
        }
    }

    impl Delegate {
        #[unsafe(method(restoreTile:))]
        fn restore_tile(&self, _sender: Option<&AnyObject>) {
            restore_tile(MainThreadMarker::from(self));
        }
    }
);

/// The Dock menu, built from the latest tree; `None` for the Dock's own
/// items alone.
fn dock_menu(mtm: MainThreadMarker) -> Option<Retained<NSMenu>> {
    let (tree, target) = STATE.with(|s| {
        let s = s.borrow();
        (s.dock.clone(), s.dock_target.clone())
    });
    let (tree, target) = (tree?, target?);
    if tree.is_empty() {
        return None;
    }
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    fill(&menu, tree.items(), &target, mtm);
    STATE.with(|s| s.borrow_mut().dock_built = Some(tree.tree));
    Some(menu)
}

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: `init` on a freshly allocated NSObject subclass.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the ivar is plain
    // data.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ArchitectPresenceTarget"]
    #[ivars = Which]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    unsafe impl NSMenuDelegate for Target {
        /// The tray's menu, filled as it opens from the latest tree.
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            let mtm = MainThreadMarker::from(self);
            let tree = STATE.with(|s| s.borrow().tray.as_ref().map(|t| t.menu.clone()));
            let Some(tree) = tree else {
                return;
            };
            fill(menu, tree.items(), self, mtm);
            STATE.with(|s| s.borrow_mut().tray_built = Some(tree.tree));
        }
    }

    impl Target {
        /// A row picked: its tag is its node in the tree it was built from.
        #[unsafe(method(pick:))]
        fn pick(&self, sender: Option<&NSMenuItem>) {
            let Some(id) = sender.and_then(|i| i32::try_from(i.tag()).ok()) else {
                return;
            };
            let which = *self.ivars();
            let pick = STATE.with(|s| {
                let s = s.borrow();
                let built = match which {
                    Which::Dock => s.dock_built.as_ref(),
                    Which::Tray => s.tray_built.as_ref(),
                };
                built.and_then(|t| t.pick(id).cloned())
            });
            if let Some(pick) = pick {
                send(Heard::Pick(pick));
            }
        }

        /// The status item clicked, where a click runs a command: the
        /// secondary button (or ⌃-click) opens the menu instead.
        #[unsafe(method(trayClicked:))]
        fn tray_clicked(&self, _sender: Option<&AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let app = NSApplication::sharedApplication(mtm);
            let secondary = app.currentEvent().is_some_and(|e| {
                e.r#type() == NSEventType::RightMouseUp
                    || e.modifierFlags().contains(objc2_app_kit::NSEventModifierFlags::Control)
            });
            if !secondary {
                send(Heard::TrayClicked);
                return;
            }
            let (status, menu) = STATE.with(|s| {
                let s = s.borrow();
                (s.status.clone(), s.tray_menu.clone())
            });
            let (Some(status), Some(menu)) = (status, menu) else {
                return;
            };
            // The menu for this click only: with a menu, the item opens it
            // instead of sending its action. No borrow is held: the menu
            // tracks in a loop of its own, and its picks read the state.
            status.setMenu(Some(&menu));
            if let Some(button) = status.button(mtm) {
                // SAFETY: a click on the status item's own button, on the
                // main thread.
                unsafe { button.performClick(None) };
            }
            status.setMenu(None);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, which: Which) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(which);
        // SAFETY: NSObject's `init` on a freshly allocated subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// `menu`'s items from `items`, each row picking through `target`.
fn fill(menu: &NSMenu, items: &[Tree], target: &Target, mtm: MainThreadMarker) {
    menu.removeAllItems();
    for node in items {
        match &node.kind {
            Kind::Row(row) => {
                // SAFETY: standard NSMenuItem construction on the main thread.
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(&row.label),
                        Some(sel!(pick:)),
                        &NSString::from_str(""),
                    )
                };
                // SAFETY: the target lives as long as the app's state does.
                unsafe { item.setTarget(Some(target)) };
                item.setTag(isize::try_from(node.id).unwrap_or(0));
                item.setEnabled(row.enabled);
                if let Some(on) = row.checked {
                    item.setState(if on {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                }
                menu.addItem(&item);
            }
            Kind::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Kind::Menu { title } => {
                let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
                sub.setAutoenablesItems(false);
                fill(&sub, &node.children, target, mtm);
                let item = NSMenuItem::new(mtm);
                item.setTitle(&NSString::from_str(title));
                item.setSubmenu(Some(&sub));
                menu.addItem(&item);
            }
            Kind::Root | Kind::Standard(_) => {}
        }
    }
}

/// The Mac's presence.
pub struct Mac {
    window: WindowOps,
}

impl Mac {
    pub const fn new(window: WindowOps) -> Self {
        Self { window }
    }
}

impl Platform for Mac {
    fn start(&self, events: Events) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let delegate = Delegate::new(mtm);
        if app.delegate().is_some() {
            tracing::warn!(
                presence.delegate = "taken",
                "presence: the app has a delegate already: no reopen, Dock menu or URLs"
            );
        } else {
            app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.events = Some(events);
            s.delegate = Some(delegate);
            s.dock_target = Some(Target::new(mtm, Which::Dock));
            s.tray_target = Some(Target::new(mtm, Which::Tray));
        });
    }

    fn set_dock_menu(&self, menu: Option<MenuTree>) {
        STATE.with(|s| s.borrow_mut().dock = menu);
    }

    fn set_tray(&self, tray: Option<TrayView>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(view) = tray else {
            let status = STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.tray = None;
                s.tray_menu = None;
                s.status.take()
            });
            if let Some(status) = status {
                NSStatusBar::systemStatusBar().removeStatusItem(&status);
            }
            return;
        };
        let (status, target, menu, before) = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let before = s.tray.as_ref().map(|t| t.tray.clone());
            let status = s
                .status
                .get_or_insert_with(|| {
                    NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)
                })
                .clone();
            let menu = s
                .tray_menu
                .get_or_insert_with(|| {
                    let m = NSMenu::new(mtm);
                    m.setAutoenablesItems(false);
                    m
                })
                .clone();
            s.tray = Some(view.clone());
            (status, s.tray_target.clone(), menu, before)
        });
        let Some(target) = target else {
            return;
        };
        if before.as_ref() == Some(&view.tray) {
            return;
        }
        let tray = &view.tray;
        menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        if let Some(button) = status.button(mtm) {
            if let Some(image) = icon(&tray.icon, &tray.tooltip) {
                button.setImage(Some(&image));
            }
            button.setTitle(&NSString::from_str(
                &tray
                    .title
                    .as_ref()
                    .map(|t| format!(" {t}"))
                    .unwrap_or_default(),
            ));
            button.setImagePosition(NSCellImagePosition::ImageLeft);
            button.setToolTip(Some(&NSString::from_str(&tray.tooltip)));
            match &tray.click {
                TrayClick::Menu => {
                    status.setMenu(Some(&menu));
                }
                TrayClick::Run(_) => {
                    status.setMenu(None);
                    // SAFETY: the target lives as long as the app's state.
                    unsafe {
                        button.setTarget(Some(&target));
                        button.setAction(Some(sel!(trayClicked:)));
                    }
                    button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
                }
            }
        }
    }

    fn set_badge(&self, label: Option<String>) {
        if let Some(mtm) = MainThreadMarker::new() {
            badge(mtm, label);
        }
    }

    fn set_progress(&self, value: Option<f64>) {
        if let Some(mtm) = MainThreadMarker::new() {
            progress(mtm, value);
        }
    }

    fn request_attention(&self, attention: Attention) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        if let Some(id) = STATE.with(|s| s.borrow_mut().attention.take()) {
            app.cancelUserAttentionRequest(id);
        }
        let id = app.requestUserAttention(match attention {
            Attention::Informational => NSRequestUserAttentionType::InformationalRequest,
            Attention::Critical => NSRequestUserAttentionType::CriticalRequest,
        });
        STATE.with(|s| s.borrow_mut().attention = Some(id));
    }

    fn apply(&self, effect: Effect) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        match effect {
            Effect::DockIcon(on) => {
                app.setActivationPolicy(if on {
                    NSApplicationActivationPolicy::Regular
                } else {
                    NSApplicationActivationPolicy::Accessory
                });
                tracing::info!(presence.dock_icon = on, "presence: the Dock icon");
                // The tile is a new one: its badge and progress again, once
                // the Dock has made it (a badge set in this turn is lost).
                let delegate = STATE.with(|s| s.borrow().delegate.clone());
                if let (true, Some(delegate)) = (on, delegate) {
                    // SAFETY: NSObject's delayed perform, of a method the
                    // delegate has, on the main thread.
                    unsafe {
                        let () = msg_send![
                            &*delegate,
                            performSelector: sel!(restoreTile:),
                            withObject: None::<&AnyObject>,
                            afterDelay: 0.25_f64
                        ];
                    };
                }
            }
            Effect::Focus => {
                #[allow(deprecated)]
                app.activateIgnoringOtherApps(true);
                let _ = self.window.apply(effect);
            }
            Effect::Quit => {
                if !self.window.apply(effect) {
                    app.terminate(None);
                }
            }
            other => {
                let _ = self.window.apply(other);
            }
        }
    }

    fn login_available(&self) -> Result<(), String> {
        crate::login::available()
    }

    fn set_login(&self, id: &str, name: &str, on: bool) -> Result<Applied, String> {
        crate::native::apply_login(id, name, on)
    }
}

/// The badge over the Dock icon.
fn badge(mtm: MainThreadMarker, label: Option<String>) {
    let tile = NSApplication::sharedApplication(mtm).dockTile();
    let ns = label.as_deref().map(NSString::from_str);
    tile.setBadgeLabel(ns.as_deref());
    STATE.with(|s| s.borrow_mut().badge = label);
}

/// Progress across the Dock tile's foot.
fn progress(mtm: MainThreadMarker, value: Option<f64>) {
    let app = NSApplication::sharedApplication(mtm);
    let tile = app.dockTile();
    STATE.with(|s| s.borrow_mut().value = value);
    let Some(value) = value else {
        if STATE.with(|s| s.borrow_mut().progress.take()).is_some() {
            tile.setContentView(None);
            tile.display();
        }
        return;
    };
    let bar = STATE.with(|s| s.borrow().progress.clone());
    let bar = bar.unwrap_or_else(|| {
        // The app's icon, a bar across its foot.
        let size = tile.size();
        let view = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), size),
        );
        view.setImage(app.applicationIconImage().as_deref());
        let bar = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(mtm),
            NSRect::new(
                NSPoint::new(size.width * 0.12, size.height * 0.06),
                NSSize::new(size.width * 0.76, size.height * 0.12),
            ),
        );
        bar.setStyle(NSProgressIndicatorStyle::Bar);
        bar.setIndeterminate(false);
        bar.setMinValue(0.0);
        bar.setMaxValue(1.0);
        view.addSubview(&bar);
        tile.setContentView(Some(&view));
        STATE.with(|s| s.borrow_mut().progress = Some(bar.clone()));
        bar
    });
    bar.setDoubleValue(value);
    tile.display();
}

/// The tile's badge and progress again, on a tile the Dock made anew.
fn restore_tile(mtm: MainThreadMarker) {
    let (label, value) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.progress = None;
        (s.badge.clone(), s.value)
    });
    // The tile keeps its label but the Dock draws a new tile without it,
    // and the same label again is not drawn: clear it first.
    badge(mtm, None);
    badge(mtm, label);
    progress(mtm, value);
    NSApplication::sharedApplication(mtm).dockTile().display();
}

/// The tray's picture as a template image (it follows the menu bar's
/// light or dark): the SF Symbol, else the PNG.
fn icon(icon: &crate::TrayIcon, description: &str) -> Option<Retained<NSImage>> {
    let symbol = icon.symbol.as_ref().and_then(|name| {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(name),
            Some(&NSString::from_str(description)),
        )
    });
    let image = symbol.or_else(|| {
        let png = icon.template_png?;
        let data = NSData::with_bytes(png);
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        // 18pt tall, as wide as it is drawn.
        let size = image.size();
        if size.height > 0.0 {
            image.setSize(NSSize::new(size.width * 18.0 / size.height, 18.0));
        }
        Some(image)
    })?;
    image.setTemplate(true);
    Some(image)
}
