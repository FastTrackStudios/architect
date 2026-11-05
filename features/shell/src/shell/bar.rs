//! A phone's bar at the foot: the action bar, up to six buttons the app
//! and its pages put there by command id, that becomes the Typing Toolbar
//! over the keyboard while a field is typed in (`super::typing`): the
//! focused editor's actions and whatever fills it, sideways when they
//! don't fit, and apart at the right the button that puts the keyboard
//! away. Obsidian's on iOS.
//!
//! The app says only what is on the bar ([`AppShell`]'s `actions`,
//! [`use_bar_action`], [`use_editing_actions`], [`use_typing_scope`],
//! `TypingTools`); how it looks, where it sits and how it rides the
//! keyboard are the shell's.
//!
//! [`AppShell`]: super::AppShell

use dioxus::prelude::*;

use super::bars::BottomBar;
use super::keyboard::use_keyboard_lift;
use super::typing::{KeyboardBar, Typing, TypingContent, TypingItem, TypingTools};
use super::{Action, Icon, IconFn, Screen, Shell, Tab, use_shell};

/// The most buttons the action bar holds.
pub const BAR_MAX: usize = 6;

/// A button on the action bar or the editing bar: the command it runs,
/// by its id on the app's Action List, and how it shows.
#[derive(Clone, PartialEq, Eq)]
pub struct BarAction {
    /// The command it runs (`WORKSPACE_EVERY_PANE`).
    pub id: String,
    /// Its name, for a screen reader and a long press.
    pub label: String,
    pub icon: IconFn,
    /// Something to count on it ("3": the open tabs); `None` for nothing.
    pub badge: Option<String>,
}

impl BarAction {
    pub fn new(id: impl Into<String>, label: impl Into<String>, icon: IconFn) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
            badge: None,
        }
    }

    /// Mark the button with a count.
    #[must_use]
    // By value, so a count reads as `.badge(5)`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn badge(mut self, badge: impl ToString) -> Self {
        self.badge = Some(badge.to_string());
        self
    }
}

/// Runs a bar button's command by its id.
///
/// `architect-commands` puts one in context (`use_commands_provider`), so
/// a button runs what the palette, the keys and the menus run; an app on
/// commands of its own provides one itself.
#[derive(Clone, Copy, PartialEq)]
pub struct CommandRunner {
    /// Runs the command; false when there is none, or it cannot run now.
    pub run: Callback<String, bool>,
    /// Whether it can run now; `None` when there is no such command.
    pub enabled: Callback<String, Option<bool>>,
    /// Whether a toggle is on now; `None` when it is not a toggle, or
    /// there is no such command. A Typing Toolbar button follows it.
    pub checked: Callback<String, Option<bool>>,
}

/// What the pages and the editors have put on the bar: the model under
/// [`use_bar_action`] and [`use_editing_actions`].
#[derive(Clone, Default, PartialEq)]
pub(super) struct BarModel {
    page: Vec<(u64, BarAction)>,
    editors: Vec<Editor>,
    /// The pages and views that say whether the focus is in them.
    scopes: Vec<Scope>,
    serial: u64,
}

/// A page or view the Typing Toolbar follows the focus into
/// ([`use_typing_scope`]).
#[derive(Clone, PartialEq)]
struct Scope {
    serial: u64,
    /// `view:note`, `view:lab.board/chart`.
    context: String,
    focused: bool,
}

/// An editor's actions, for while it is typed in.
#[derive(Clone, PartialEq)]
struct Editor {
    serial: u64,
    /// What it edits (`markdown`), named on the bar.
    context: String,
    actions: Vec<BarAction>,
    focused: bool,
}

impl BarModel {
    const fn next(&mut self) -> u64 {
        self.serial = self.serial.saturating_add(1);
        self.serial
    }

    fn add(&mut self, action: BarAction) -> u64 {
        let serial = self.next();
        self.page.push((serial, action));
        serial
    }

    /// Keeps a registration current; whether it changed.
    fn update(&mut self, serial: u64, action: &BarAction) -> bool {
        self.page
            .iter_mut()
            .find(|(s, a)| *s == serial && a != action)
            .map(|(_, a)| *a = action.clone())
            .is_some()
    }

    fn remove(&mut self, serial: u64) {
        self.page.retain(|(s, _)| *s != serial);
        self.editors.retain(|e| e.serial != serial);
        self.scopes.retain(|e| e.serial != serial);
    }

    fn add_scope(&mut self, context: &str) -> u64 {
        let serial = self.next();
        self.scopes.push(Scope {
            serial,
            context: context.to_owned(),
            focused: false,
        });
        serial
    }

    /// A scope says the focus is in it, or has left: whether it changed.
    /// Scopes nest (a field in a view in a pane), so each keeps its own.
    fn focus_scope(&mut self, serial: u64, on: bool) -> bool {
        self.scopes
            .iter_mut()
            .find(|s| s.serial == serial && s.focused != on)
            .map(|s| s.focused = on)
            .is_some()
    }

    fn add_editor(&mut self, context: &str, actions: Vec<BarAction>) -> u64 {
        let serial = self.next();
        self.editors.push(Editor {
            serial,
            context: context.to_owned(),
            actions,
            focused: false,
        });
        serial
    }

    fn update_editor(&mut self, serial: u64, context: &str, actions: &[BarAction]) -> bool {
        self.editors
            .iter_mut()
            .find(|e| e.serial == serial && (e.context != context || e.actions != actions))
            .map(|e| {
                context.clone_into(&mut e.context);
                e.actions = actions.to_vec();
            })
            .is_some()
    }

    /// The editor says it has the text focus, or has let it go.
    fn focus_editor(&mut self, serial: u64, on: bool) -> bool {
        let mut changed = false;
        for e in &mut self.editors {
            let now = if e.serial == serial {
                on
            } else {
                // One editor has the focus at a time.
                e.focused && !on
            };
            changed |= e.focused != now;
            e.focused = now;
        }
        changed
    }

    /// The action bar: the app's buttons, a page's in place of the app's
    /// of the same id, then the pages' others in the order they came. Past
    /// [`BAR_MAX`] the app's give way from the end, then the latest page's.
    fn shown(&self, app: &[BarAction]) -> Vec<BarAction> {
        // A later registration of an id wins over an earlier one.
        let mut page: Vec<&BarAction> = Vec::new();
        for (_, a) in &self.page {
            page.retain(|p| p.id != a.id);
            page.push(a);
        }
        // (from the app, the button)
        let mut row: Vec<(bool, BarAction)> = Vec::new();
        for a in app {
            if row.iter().any(|(_, r)| r.id == a.id) {
                continue;
            }
            row.push(
                page.iter()
                    .find(|p| p.id == a.id)
                    .map_or_else(|| (true, a.clone()), |p| (false, (*p).clone())),
            );
        }
        for p in page {
            if !row.iter().any(|(_, r)| r.id == p.id) {
                row.push((false, p.clone()));
            }
        }
        while row.len() > BAR_MAX {
            match row.iter().rposition(|(from_app, _)| *from_app) {
                Some(i) => {
                    row.remove(i);
                }
                None => {
                    row.pop();
                }
            }
        }
        row.into_iter().map(|(_, a)| a).collect()
    }

    /// The editing bar's actions and what they edit: the editor with the
    /// text focus, else (when no page or view says it has the focus) the
    /// one registered last; nothing with no editor.
    fn editing(&self) -> Option<(&str, &[BarAction])> {
        let focused = self.editors.iter().rev().find(|e| e.focused);
        let scoped = self.scopes.iter().any(|s| s.focused);
        focused
            .or_else(|| (!scoped).then(|| self.editors.last()).flatten())
            .map(|e| (e.context.as_str(), e.actions.as_slice()))
    }

    /// What is typed in now: the editor's context and actions, and the
    /// scopes the focus is in.
    fn typing(&self) -> Typing {
        let (editor, actions) = self
            .editing()
            .map(|(c, a)| (Some(c.to_owned()), a.to_vec()))
            .unwrap_or_default();
        Typing {
            editor,
            actions,
            scopes: self
                .scopes
                .iter()
                .filter(|s| s.focused)
                .map(|s| s.context.clone())
                .collect(),
        }
    }
}

/// Takes registration `serial` off the bar, if the shell is still there
/// (a pane in a window can go after the shell it was told about).
fn forget(bars: Option<Signal<BarModel>>, serial: Option<u64>) {
    if let (Some(mut b), Some(serial)) = (bars, serial)
        && let Ok(mut w) = b.try_write()
    {
        w.remove(serial);
    }
}

fn bars() -> Option<Signal<BarModel>> {
    try_use_context::<Signal<Shell>>().map(|s| s.peek().bars)
}

/// Puts `action` on a phone's action bar while the calling component is
/// mounted. Call it every render: its label and badge are kept current.
/// A page's button takes the place of the app's of the same id.
// By value: a button is built where it is registered (`BarAction::new(…)`).
#[allow(clippy::needless_pass_by_value)]
pub fn use_bar_action(action: BarAction) {
    let bars = bars();
    let first = action.clone();
    let serial = use_hook(move || bars.map(|mut b| b.write().add(first)));
    if let (Some(mut b), Some(serial)) = (bars, serial) {
        let stale = b
            .peek()
            .page
            .iter()
            .any(|(s, a)| *s == serial && *a != action);
        if stale {
            b.write().update(serial, &action);
        }
    }
    use_drop(move || forget(bars, serial));
}

/// An editor's say on the editing bar ([`use_editing_actions`]): that it
/// has the text focus, or has let it go.
#[derive(Clone, Copy)]
pub struct EditingFocus {
    bars: Option<Signal<BarModel>>,
    serial: Option<u64>,
    /// A page's or view's ([`use_typing_scope`]), not an editor's.
    scope: bool,
}

impl EditingFocus {
    /// The editor has the text focus: its actions are the bar's.
    pub fn focus(&self) {
        self.set(true);
    }

    /// The editor let the focus go.
    pub fn blur(&self) {
        self.set(false);
    }

    fn set(&self, on: bool) {
        if self.scope {
            if let (Some(mut b), Some(serial)) = (self.bars, self.serial) {
                let changes = b
                    .peek()
                    .scopes
                    .iter()
                    .any(|s| s.serial == serial && s.focused != on);
                if changes {
                    b.write().focus_scope(serial, on);
                }
            }
            return;
        }
        if let (Some(mut b), Some(serial)) = (self.bars, self.serial) {
            // Told on every focus and blur: written only when it changes.
            let changes = b
                .peek()
                .editors
                .iter()
                .any(|e| e.focused != (e.serial == serial && on) && (e.serial == serial || on));
            if changes {
                b.write().focus_editor(serial, on);
            }
        }
    }
}

/// Puts an editor's editing actions on the bar over the keyboard while
/// the editor is mounted.
///
/// Bold, list, link, indent, undo…, each by command id; `context` names
/// what it edits (`markdown`). With more than one editor on the page, the
/// one that says it has the focus (the returned [`EditingFocus`], from its
/// `onfocus` and `onblur`) is the bar's, else the one mounted last. Call
/// it every render.
#[must_use = "the editor tells it when it has the focus"]
pub fn use_editing_actions(context: &str, actions: &[BarAction]) -> EditingFocus {
    let bars = bars();
    let first = (context.to_owned(), actions.to_vec());
    let serial = use_hook(move || bars.map(|mut b| b.write().add_editor(&first.0, first.1)));
    if let (Some(mut b), Some(serial)) = (bars, serial) {
        let stale = b
            .peek()
            .editors
            .iter()
            .any(|e| e.serial == serial && (e.context != context || e.actions != actions));
        if stale {
            b.write().update_editor(serial, context, actions);
        }
    }
    use_drop(move || forget(bars, serial));
    EditingFocus {
        bars,
        serial,
        scope: false,
    }
}

/// Lets the Typing Toolbar follow the focus into the calling page or view.
///
/// `context` names it (`view:note`, `view:lab.board/chart`), the
/// context a toolbar's `when` names. Wire the returned [`EditingFocus`] to
/// its box's `onfocusin` and `onfocusout` (they bubble from its fields,
/// in Blitz too); while a field in it is typed in, the context is in
/// force ([`Typing::scopes`]). The workspace's panes do it for their view
/// kind, an extension's view for its own.
#[must_use = "the page tells it when the focus comes and goes"]
pub fn use_typing_scope(context: &str) -> EditingFocus {
    let bars = bars();
    let first = context.to_owned();
    let serial = use_hook(move || bars.map(|mut b| b.write().add_scope(&first)));
    if let (Some(mut b), Some(serial)) = (bars, serial) {
        let stale = b
            .peek()
            .scopes
            .iter()
            .any(|s| s.serial == serial && s.context != context);
        if stale {
            let mut w = b.write();
            if let Some(s) = w.scopes.iter_mut().find(|s| s.serial == serial) {
                context.clone_into(&mut s.context);
            }
        }
    }
    use_drop(move || forget(bars, serial));
    EditingFocus {
        bars,
        serial,
        scope: true,
    }
}

/// The action bar's buttons' height, points.
const BAR_ROW: f64 = 48.0;
/// A button's side, points: a finger's.
const BUTTON: f64 = 44.0;

/// A phone's foot (`Frame::Bottom`): the action bar over the tabs, or,
/// while a field is typed in, the editing bar riding the keyboard.
#[component]
pub(super) fn PhoneFoot(
    tabs: Vec<Tab>,
    current: Option<&'static str>,
    on_tab: EventHandler<&'static str>,
    separate: Option<Action>,
    screen: Screen,
) -> Element {
    let shell = use_shell();
    let keyboard = use_keyboard_lift();
    let now = (keyboard.now)();
    let lift = keyboard.lift();
    let ins = screen.insets;
    let model = (shell.bars)();
    if now.editing() {
        let typing = model.typing();
        // Filled by whoever put `TypingTools` in context (the toolbars of
        // place `typing`); else the editor's own actions.
        let filled = try_consume_context::<TypingTools>().and_then(|f| (f.0)(&typing));
        let TypingContent { items, above } = filled.unwrap_or_else(|| TypingContent {
            items: typing
                .actions
                .iter()
                .cloned()
                .map(TypingItem::action)
                .collect(),
            above: None,
        });
        let context = typing.editor;
        return rsx! {
            KeyboardBar { context, items, above, screen, on_dismiss: move |()| keyboard.dismissed() }
            div { style: "flex-shrink: 0; height: {lift.max(ins.bottom)}px;" }
        };
    }
    let items = model.shown(&shell.bar_actions);
    let tab_bar = !tabs.is_empty() || separate.is_some();
    rsx! {
        if !items.is_empty() {
            ActionBar { items, bottom: if tab_bar { 0.0 } else { ins.bottom }, screen }
        }
        if tab_bar {
            BottomBar { tabs, current, on_tab, separate, screen }
        }
        // The keyboard on its way down: the bars ride it.
        if lift > 0.5 {
            div { style: "flex-shrink: 0; height: {lift}px;" }
        }
    }
}

/// Runs `id` through the app's [`CommandRunner`].
fn run(id: &str) {
    if let Some(runner) = try_consume_context::<CommandRunner>() {
        let _ = runner.run.call(id.to_owned());
    }
}

/// The action bar: its buttons spread along a row over the tabs (or the
/// home indicator, with no tabs).
#[component]
fn ActionBar(items: Vec<BarAction>, bottom: f64, screen: Screen) -> Element {
    let t = use_shell().theme;
    let ins = screen.insets;
    let runner = try_use_context::<CommandRunner>();
    rsx! {
        div {
            "data-bar": "actions",
            style: "flex-shrink: 0; height: {BAR_ROW + bottom}px; box-sizing: border-box; display: flex; \
                    align-items: center; justify-content: space-around; \
                    padding: 0 {12.0 + ins.right}px {bottom}px {12.0 + ins.left}px; \
                    background: {t.desk}; border-top: 1px solid {t.rule};",
            for item in items {
                BarButton {
                    key: "{item.id}",
                    enabled: runner.and_then(|r| r.enabled.call(item.id.clone())).unwrap_or(true),
                    action: item,
                }
            }
        }
    }
}

#[component]
fn BarButton(action: BarAction, enabled: bool) -> Element {
    let t = use_shell().theme;
    let colour = if enabled { &*t.ink_2 } else { &*t.ink_3 };
    let opacity = if enabled { 1.0 } else { 0.45 };
    let id = action.id.clone();
    rsx! {
        button {
            class: "lp",
            "aria-label": "{action.label}",
            "aria-disabled": if enabled { "false" } else { "true" },
            "data-bar-item": "{action.id}",
            style: "position: relative; width: {BUTTON}px; height: {BUTTON}px; flex-shrink: 0; padding: 0; border: none; \
                    border-radius: 12px; background: transparent; display: flex; align-items: center; \
                    justify-content: center; opacity: {opacity};",
            onclick: move |_| {
                if enabled {
                    run(&id);
                }
            },
            {(action.icon)(Icon { size: 22, color: colour.into() })}
            if let Some(badge) = action.badge {
                // Obsidian's tab count: a number in a rounded square at
                // the icon's corner.
                span {
                    style: "position: absolute; top: 4px; right: 3px; min-width: 16px; height: 16px; padding: 0 4px; \
                            box-sizing: border-box; border-radius: 5px; display: flex; align-items: center; \
                            justify-content: center; background: {t.pill}; border: 1px solid {t.ink_3}; \
                            color: {t.ink}; font-family: {t.font}; font-size: 10px; font-weight: 700;",
                    "{badge}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: Icon) -> Element {
        VNode::empty()
    }

    fn a(id: &str) -> BarAction {
        BarAction::new(id, id, none)
    }

    fn ids(row: &[BarAction]) -> Vec<&str> {
        row.iter().map(|a| a.id.as_str()).collect()
    }

    #[test]
    fn the_app_s_buttons_then_the_pages_in_the_order_they_came() {
        let mut m = BarModel::default();
        let app = [a("BACK"), a("SEARCH")];
        assert_eq!(ids(&m.shown(&app)), ["BACK", "SEARCH"]);
        let tabs = m.add(a("TABS"));
        let _menu = m.add(a("MENU"));
        assert_eq!(ids(&m.shown(&app)), ["BACK", "SEARCH", "TABS", "MENU"]);
        m.remove(tabs);
        assert_eq!(ids(&m.shown(&app)), ["BACK", "SEARCH", "MENU"]);
    }

    #[test]
    fn a_page_s_button_takes_the_place_of_the_app_s_of_its_id() {
        let mut m = BarModel::default();
        let app = [a("BACK"), a("SEARCH"), a("MENU")];
        let s = m.add(a("SEARCH").badge(2));
        let row = m.shown(&app);
        assert_eq!(ids(&row), ["BACK", "SEARCH", "MENU"]);
        assert_eq!(row[1].badge.as_deref(), Some("2"));
        // A later registration of an id wins over an earlier one.
        let _again = m.add(a("SEARCH").badge(5));
        assert_eq!(m.shown(&app)[1].badge.as_deref(), Some("5"));
        // Kept current: a new badge is a change, the same one is not.
        assert!(m.update(s, &a("SEARCH").badge(3)));
        assert!(!m.update(s, &a("SEARCH").badge(3)));
    }

    #[test]
    fn six_at_most_and_the_app_s_give_way_first() {
        let mut m = BarModel::default();
        let app: Vec<BarAction> = ["A1", "A2", "A3", "A4", "A5", "A6"]
            .into_iter()
            .map(a)
            .collect();
        assert_eq!(m.shown(&app).len(), BAR_MAX);
        let _tabs = m.add(a("TABS"));
        // The page's tabs button is kept; the app's last goes.
        assert_eq!(ids(&m.shown(&app)), ["A1", "A2", "A3", "A4", "A5", "TABS"]);
        for id in ["P1", "P2", "P3", "P4", "P5", "P6"] {
            let _ = m.add(a(id));
        }
        // Only the pages' now, the latest past six let go.
        assert_eq!(ids(&m.shown(&app)), ["TABS", "P1", "P2", "P3", "P4", "P5"]);
        // The app's own duplicates count once.
        let twice = [a("BACK"), a("BACK")];
        assert_eq!(ids(&BarModel::default().shown(&twice)), ["BACK"]);
    }

    #[test]
    fn the_editing_bar_is_the_focused_editor_s_else_the_last_one_s() {
        let mut m = BarModel::default();
        assert!(m.editing().is_none(), "no editor, no editing actions");
        let title = m.add_editor("plain", vec![a("UNDO")]);
        let body = m.add_editor("markdown", vec![a("BOLD"), a("LIST")]);
        let (context, actions) = m.editing().unwrap_or(("", &[]));
        assert_eq!((context, ids(actions)), ("markdown", vec!["BOLD", "LIST"]));

        assert!(m.focus_editor(title, true));
        assert_eq!(m.editing().map(|(c, _)| c), Some("plain"));
        // One editor has the focus at a time.
        assert!(m.focus_editor(body, true));
        assert_eq!(m.editing().map(|(c, _)| c), Some("markdown"));
        assert!(!m.focus_editor(body, true), "no change, no redraw");
        assert!(m.focus_editor(body, false));
        assert_eq!(
            m.editing().map(|(c, _)| c),
            Some("markdown"),
            "the last one"
        );

        // Kept current, and gone with its editor.
        assert!(m.update_editor(body, "markdown", &[a("BOLD")]));
        assert!(!m.update_editor(body, "markdown", &[a("BOLD")]));
        m.remove(body);
        assert_eq!(m.editing().map(|(c, _)| c), Some("plain"));
        m.remove(title);
        assert!(m.editing().is_none());
        // The action bar never shows an editor's actions.
        assert!(m.shown(&[]).is_empty());
    }

    #[test]
    fn a_scope_the_focus_is_in_is_in_force_and_takes_the_unfocused_editor_s_place() {
        let mut m = BarModel::default();
        let capture = m.add_editor("capture", vec![a("TAG")]);
        let pane = m.add_scope("view:lab.board/chart");
        // Nothing says it has the focus: the editor mounted last, as before.
        assert_eq!(m.typing().editor.as_deref(), Some("capture"));
        assert!(m.typing().scopes.is_empty());

        // A field in the view has it: the view's context, no editor's.
        assert!(m.focus_scope(pane, true));
        assert!(!m.focus_scope(pane, true), "no change, no redraw");
        let t = m.typing();
        assert_eq!(t.editor, None);
        assert!(t.actions.is_empty());
        assert_eq!(t.contexts(), ["view:lab.board/chart"]);

        // An editor in it takes the focus: both are in force.
        assert!(m.focus_editor(capture, true));
        assert_eq!(
            m.typing().contexts(),
            ["editor:capture", "view:lab.board/chart"]
        );

        // The focus leaves; the scope goes with its view.
        assert!(m.focus_scope(pane, false));
        m.remove(pane);
        assert!(m.typing().scopes.is_empty());
        assert_eq!(m.typing().editor.as_deref(), Some("capture"));
    }
}
