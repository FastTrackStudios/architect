//! A phone's bar at the foot: the action bar, up to six buttons the app
//! and its pages put there by command id, that becomes the editing bar
//! over the keyboard while a field is typed in — the focused editor's
//! actions, sideways when they don't fit, and apart at the right the
//! button that puts the keyboard away. Obsidian's on iOS.
//!
//! The app says only what is on the bar ([`AppShell`]'s `actions`,
//! [`use_bar_action`], [`use_editing_actions`]); how it looks, where it
//! sits and how it rides the keyboard are the shell's.
//!
//! [`AppShell`]: super::AppShell

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use lucide_dioxus::KeyboardOff;

use super::bars::BottomBar;
use super::keyboard::use_keyboard_lift;
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
}

/// What the pages and the editors have put on the bar: the model under
/// [`use_bar_action`] and [`use_editing_actions`].
#[derive(Clone, Default, PartialEq)]
pub(super) struct BarModel {
    page: Vec<(u64, BarAction)>,
    editors: Vec<Editor>,
    serial: u64,
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
    /// text focus, else the one registered last; nothing with no editor.
    fn editing(&self) -> Option<(&str, &[BarAction])> {
        self.editors
            .iter()
            .rev()
            .find(|e| e.focused)
            .or_else(|| self.editors.last())
            .map(|e| (e.context.as_str(), e.actions.as_slice()))
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
    use_drop(move || {
        if let (Some(mut b), Some(serial)) = (bars, serial) {
            b.write().remove(serial);
        }
    });
}

/// An editor's say on the editing bar ([`use_editing_actions`]): that it
/// has the text focus, or has let it go.
#[derive(Clone, Copy)]
pub struct EditingFocus {
    bars: Option<Signal<BarModel>>,
    serial: Option<u64>,
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
    use_drop(move || {
        if let (Some(mut b), Some(serial)) = (bars, serial) {
            b.write().remove(serial);
        }
    });
    EditingFocus { bars, serial }
}

/// The action bar's buttons' height, points.
const BAR_ROW: f64 = 48.0;
/// A button's side, points: a finger's.
const BUTTON: f64 = 44.0;
/// The editing bar's pill's height, and its round button's side.
const PILL: f64 = 48.0;
/// The space around the editing bar.
const EDGE: f64 = 8.0;
/// Between the pill's buttons, and inside its ends.
const PILL_GAP: f64 = 2.0;
const PILL_PAD: f64 = 4.0;
/// How far a press moves before it is a scroll, not a tap.
const SLOP: f64 = 6.0;

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
        let (context, actions) = model
            .editing()
            .map(|(c, a)| (Some(c.to_owned()), a.to_vec()))
            .unwrap_or_default();
        return rsx! {
            KeyboardBar { context, actions, screen, on_dismiss: move |()| keyboard.dismissed() }
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
    let colour = if enabled { t.ink_2 } else { t.ink_3 };
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
            {(action.icon)(Icon { size: 22, color: colour })}
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

/// A press on the editing bar's pill: where it began, and the offset then.
#[derive(Clone, Copy, PartialEq)]
struct Slide {
    x0: f64,
    from: f64,
    moved: bool,
}

/// The editing bar: the editor's actions in a pill that scrolls sideways,
/// and apart at the right the button that puts the keyboard away.
///
/// A press on the pill keeps the field's focus (its default is prevented,
/// and its click's, so the caret and the keyboard stay), which also keeps
/// Blitz from panning it: the pill follows the finger itself.
#[component]
fn KeyboardBar(
    context: Option<String>,
    actions: Vec<BarAction>,
    screen: Screen,
    on_dismiss: EventHandler<()>,
) -> Element {
    let t = use_shell().theme;
    let ins = screen.insets;
    let mut offset = use_signal(|| 0.0_f64);
    let slide = use_hook(|| Rc::new(Cell::new(None::<Slide>)));
    let runner = try_use_context::<CommandRunner>();
    // Laid out from the buttons' fixed sizes: how far the pill can scroll.
    let count = f64::from(u32::try_from(actions.len()).unwrap_or(u32::MAX));
    let content = 2.0f64.mul_add(
        PILL_PAD,
        BUTTON.mul_add(count, PILL_GAP * (count - 1.0).max(0.0)),
    );
    // The bar's two edges, the pill's gap to the round button, the button.
    let room = 3.0f64.mul_add(-EDGE, screen.width - ins.left - ins.right - PILL);
    let reach = (content - room).max(0.0);
    let x = offset().clamp(-reach, 0.0);
    // Only the buttons in the pill's window are drawn: Blitz hits what
    // `overflow: hidden` hides, and one scrolled past the pill's end would
    // take a press meant for the round button.
    let (first, end) = in_view(actions.len(), x, room);
    let before = f64::from(u32::try_from(first).unwrap_or(u32::MAX)) * (BUTTON + PILL_GAP);
    let (down, mv, up) = (slide.clone(), slide.clone(), slide.clone());
    let context = context.unwrap_or_default();
    rsx! {
        div {
            "data-bar": "editing",
            "data-editing": "{context}",
            style: "flex-shrink: 0; display: flex; align-items: center; gap: {EDGE}px; \
                    padding: 6px {EDGE + ins.right}px {EDGE}px {EDGE + ins.left}px;",
            if actions.is_empty() {
                span { style: "flex: 1;" }
            } else {
                div {
                    style: "position: relative; flex: 1; min-width: 0; height: {PILL}px; box-sizing: border-box; \
                            overflow: hidden; border-radius: {PILL / 2.0}px; background: {t.pill}; \
                            border: 1px solid {t.rule}; box-shadow: {t.shadow_lift};",
                    // The field keeps the focus, and no drawer opens under
                    // a sideways scroll.
                    onpointerdown: move |e: PointerEvent| {
                        e.prevent_default();
                        e.stop_propagation();
                        down.set(Some(Slide { x0: e.client_coordinates().x, from: x, moved: false }));
                    },
                    onmousedown: move |e: MouseEvent| e.prevent_default(),
                    onpointermove: move |e: PointerEvent| {
                        let Some(mut s) = mv.get() else { return };
                        let dx = e.client_coordinates().x - s.x0;
                        if s.moved || dx.abs() >= SLOP {
                            s.moved = true;
                            mv.set(Some(s));
                            offset.set((s.from + dx).clamp(-reach, 0.0));
                        }
                    },
                    onpointerup: move |_| {
                        // A scroll is not a press: its click is let go.
                        let s = up.take();
                        if s.is_some_and(|s| s.moved) {
                            up.set(Some(Slide { x0: 0.0, from: 0.0, moved: true }));
                        }
                    },
                    div {
                        // `left`, not a transform: Blitz moves a transform
                        // by device pixels, a third as far on a 3x phone.
                        style: "position: absolute; top: 0; bottom: 0; left: {x}px; display: flex; align-items: center; \
                                gap: {PILL_GAP}px; padding: 0 {PILL_PAD}px;",
                        if first > 0 {
                            span { style: "width: {before - PILL_GAP}px; flex-shrink: 0;" }
                        }
                        for action in actions.into_iter().take(end).skip(first) {
                            EditButton {
                                key: "{action.id}",
                                enabled: runner.and_then(|r| r.enabled.call(action.id.clone())).unwrap_or(true),
                                slid: slide.clone(),
                                action,
                            }
                        }
                    }
                }
            }
            // Not focusable: a press on it takes the focus off the field,
            // which puts the keyboard away (Blitz, and a browser); a web
            // view is told as well. Over the pill's edge button, should one
            // reach under it.
            button {
                class: "lp",
                tabindex: "-1",
                "aria-label": "Hide the keyboard",
                "data-bar-item": "hide-keyboard",
                style: "position: relative; z-index: 1; width: {PILL}px; height: {PILL}px; flex-shrink: 0; \
                        box-sizing: border-box; padding: 0; \
                        border-radius: {PILL / 2.0}px; display: flex; align-items: center; justify-content: center; \
                        background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_lift};",
                onclick: move |_| {
                    let _ = document::eval("document.activeElement && document.activeElement.blur();");
                    on_dismiss.call(());
                },
                KeyboardOff { size: 20, color: t.ink_2 }
            }
        }
    }
}

/// Which of `count` buttons the pill shows at offset `x` in `room`
/// points: from the first that reaches into it to the one before the
/// first past its end.
fn in_view(count: usize, x: f64, room: f64) -> (usize, usize) {
    let left = |i: usize| {
        f64::from(u32::try_from(i).unwrap_or(u32::MAX)).mul_add(BUTTON + PILL_GAP, PILL_PAD + x)
    };
    let first = (0..count)
        .find(|&i| left(i) + BUTTON > 0.0)
        .unwrap_or(count);
    let end = (first..count).find(|&i| left(i) >= room).unwrap_or(count);
    (first, end)
}

#[component]
fn EditButton(action: BarAction, enabled: bool, slid: Rc<Cell<Option<Slide>>>) -> Element {
    let t = use_shell().theme;
    let colour = if enabled { t.ink } else { t.ink_3 };
    let opacity = if enabled { 1.0 } else { 0.45 };
    let id = action.id.clone();
    rsx! {
        button {
            tabindex: "-1",
            "aria-label": "{action.label}",
            "aria-disabled": if enabled { "false" } else { "true" },
            "data-bar-item": "{action.id}",
            style: "width: {BUTTON}px; height: {BUTTON}px; flex-shrink: 0; padding: 0; border: none; \
                    border-radius: {BUTTON / 2.0}px; background: transparent; display: flex; \
                    align-items: center; justify-content: center; opacity: {opacity};",
            onclick: move |e: MouseEvent| {
                // Blitz takes the focus off the field for a click on
                // nothing focussable: not this one.
                e.prevent_default();
                // The press that scrolled the pill presses nothing.
                if slid.take().is_some_and(|s| s.moved) {
                    return;
                }
                if enabled {
                    run(&id);
                }
            },
            {(action.icon)(Icon { size: 20, color: colour })}
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
    fn the_pill_draws_only_the_buttons_in_its_window() {
        // 9 buttons, 46 points apart from 4 in: 0–48, 50–94, …, 372–416.
        assert_eq!(in_view(9, 0.0, 330.0), (0, 8));
        assert_eq!(in_view(9, -90.0, 330.0), (1, 9), "scrolled to the end");
        assert_eq!(in_view(9, -50.0, 330.0), (1, 9));
        assert_eq!(in_view(9, -47.0, 330.0), (0, 9), "a sliver still shows");
        assert_eq!(in_view(3, 0.0, 330.0), (0, 3), "all fit");
        assert_eq!(in_view(0, 0.0, 330.0), (0, 0));
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
}
