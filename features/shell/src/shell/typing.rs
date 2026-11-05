//! The Typing Toolbar: a phone's bar over the keyboard while a field is
//! typed in (Obsidian's on iOS). Its items in a pill that scrolls
//! sideways when they don't fit, and apart at the right the round button
//! that puts the keyboard away, which is the shell's and never an item.
//!
//! What is on it is the focused editor's editing actions
//! ([`use_editing_actions`]), unless something fills it: [`TypingTools`] in
//! context, given the [`Typing`] in force (the editor's context and
//! actions, the pages and views the focus is in, [`use_typing_scope`]),
//! answers its items. `architect-extensions` fills it with the toolbars
//! of place `typing` (`docs/content/architecture/app-shell.md`, "The
//! action bar and the keyboard bar"). The shell draws every item, so they
//! look alike whoever put them there, and keeps the field's focus through
//! a press on any of them.
//!
//! [`use_editing_actions`]: super::use_editing_actions

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use lucide_dioxus::KeyboardOff;

use super::bar::{BarAction, CommandRunner};
use super::{Icon, IconFn, Screen, use_shell};

/// What is typed in now, for whoever fills the Typing Toolbar.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Typing {
    /// The editor whose field has the focus: its context (`markdown`).
    pub editor: Option<String>,
    /// Its editing actions, in its order: the bar's built-in items.
    pub actions: Vec<BarAction>,
    /// The pages and views the focus is in, outermost first
    /// (`view:note`, `view:lab.board/chart`): [`use_typing_scope`].
    pub scopes: Vec<String>,
}

impl Typing {
    /// Every context in force: `editor:<context>` for the editor, then the
    /// scopes, each once.
    #[must_use]
    pub fn contexts(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in self
            .editor
            .iter()
            .map(|e| format!("editor:{e}"))
            .chain(self.scopes.iter().cloned())
        {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }
}

/// One item of the Typing Toolbar.
#[derive(Clone, PartialEq)]
pub enum TypingItem {
    /// A button running a command; a toggle shows on while it is.
    Button(TypingButton),
    /// A thin rule between items.
    Separator,
    /// A button opening rows above the bar.
    Menu(TypingMenu),
    /// A live control, drawn by whoever put it there.
    Widget(TypingWidget),
}

impl TypingItem {
    /// A button for `action`, run through the app's [`CommandRunner`].
    #[must_use]
    pub const fn action(action: BarAction) -> Self {
        Self::Button(TypingButton {
            action,
            checked: None,
            text: false,
            on_press: None,
        })
    }

    /// How wide it is in the pill, points: the pill scrolls by these.
    #[must_use]
    pub fn width(&self) -> f64 {
        match self {
            Self::Button(b) if b.text => text_width(&b.action.label),
            Self::Button(_) => BUTTON,
            Self::Separator => SEPARATOR,
            Self::Menu(m) if m.icon.is_none() => text_width(&m.label) + CHEVRON,
            Self::Menu(_) => BUTTON + CHEVRON,
            Self::Widget(w) => w.width.max(1.0),
        }
    }
}

/// A button on the Typing Toolbar.
#[derive(Clone, PartialEq)]
pub struct TypingButton {
    /// The command it runs, its name and its icon.
    pub action: BarAction,
    /// On or off, a toggle's; `None` asks the app's [`CommandRunner`].
    pub checked: Option<bool>,
    /// Its name drawn in place of its icon (it has none).
    pub text: bool,
    /// Runs in place of the command by id (an extension's toolbar button,
    /// told where it was pressed).
    pub on_press: Option<Callback<()>>,
}

/// A dropdown on the Typing Toolbar: its rows open above the bar.
#[derive(Clone)]
pub struct TypingMenu {
    /// Which it is, among the bar's menus.
    pub key: String,
    pub label: String,
    /// Its icon; none, its label.
    pub icon: Option<IconFn>,
    /// Draws its rows, given what closes it.
    pub rows: Rc<dyn Fn(Callback<()>) -> Element>,
}

impl PartialEq for TypingMenu {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.label == other.label
            && match (self.icon, other.icon) {
                (Some(a), Some(b)) => std::ptr::fn_addr_eq(a, b),
                (a, b) => a.is_none() && b.is_none(),
            }
            && Rc::ptr_eq(&self.rows, &other.rows)
    }
}

/// A live control on the Typing Toolbar.
#[derive(Clone)]
pub struct TypingWidget {
    /// Which it is (`lab.board/tempo`).
    pub key: String,
    /// Its width, points; it is the bar's buttons' height.
    pub width: f64,
    pub render: Rc<dyn Fn() -> Element>,
}

impl PartialEq for TypingWidget {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.width.to_bits() == other.width.to_bits()
            && Rc::ptr_eq(&self.render, &other.render)
    }
}

/// What fills the Typing Toolbar.
#[derive(Clone, Default, PartialEq)]
pub struct TypingContent {
    /// Its items, in order.
    pub items: Vec<TypingItem>,
    /// Drawn above the bar, over the page (the toolbar editor's strips
    /// while it is on).
    pub above: Option<Element>,
}

/// Fills the Typing Toolbar: put one in context above the shell. Asked on
/// every draw with what is typed in; `None` leaves the editor's own
/// actions.
#[derive(Clone)]
pub struct TypingTools(pub Rc<dyn Fn(&Typing) -> Option<TypingContent>>);

impl TypingTools {
    pub fn new(fill: impl Fn(&Typing) -> Option<TypingContent> + 'static) -> Self {
        Self(Rc::new(fill))
    }
}

/// The button's side, points: a finger's.
pub(super) const BUTTON: f64 = 44.0;
/// The pill's height, and its round button's side.
pub(super) const PILL: f64 = 48.0;
/// The space around the bar.
pub(super) const EDGE: f64 = 8.0;
/// Between the pill's items, and inside its ends.
const PILL_GAP: f64 = 2.0;
const PILL_PAD: f64 = 4.0;
/// A separator's room.
const SEPARATOR: f64 = 9.0;
/// A dropdown's ▾ beside its icon or name.
const CHEVRON: f64 = 12.0;
/// How far a press moves before it is a scroll, not a tap.
const SLOP: f64 = 6.0;
/// A dropdown's width.
const MENU: f64 = 220.0;

/// A name drawn on a button, about as wide as it is.
fn text_width(label: &str) -> f64 {
    let chars = f64::from(u32::try_from(label.chars().count()).unwrap_or(u32::MAX));
    7.5f64.mul_add(chars, 24.0).max(BUTTON)
}

fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Where each item starts in the pill's row, from its first item's left
/// edge.
fn starts(widths: &[f64]) -> Vec<f64> {
    let mut at = 0.0;
    widths
        .iter()
        .map(|w| {
            let here = at;
            at += w + PILL_GAP;
            here
        })
        .collect()
}

/// A press on the pill: where it began, and the offset then.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Slide {
    x0: f64,
    from: f64,
    moved: bool,
    /// Still pressed: a pointer moving with no press (a mouse's hover)
    /// scrolls nothing.
    down: bool,
}

/// The Typing Toolbar: its items in a pill that scrolls sideways, and
/// apart at the right the button that puts the keyboard away.
///
/// A press on the pill, or on a dropdown's rows, keeps the field's focus
/// (its default is prevented, and its click's, so the caret and the
/// keyboard stay), which also keeps Blitz from panning it: the pill
/// follows the finger itself.
#[allow(clippy::too_many_lines)]
#[component]
pub(super) fn KeyboardBar(
    context: Option<String>,
    items: Vec<TypingItem>,
    above: Option<Element>,
    screen: Screen,
    on_dismiss: EventHandler<()>,
) -> Element {
    let t = use_shell().theme;
    let ins = screen.insets;
    let mut offset = use_signal(|| 0.0_f64);
    let slide = use_hook(|| Rc::new(Cell::new(None::<Slide>)));
    // The dropdown open, by its key, and where its button's left edge is.
    let mut open = use_signal(|| None::<(String, f64)>);
    let runner = try_use_context::<CommandRunner>();
    let widths: Vec<f64> = items.iter().map(TypingItem::width).collect();
    let at = starts(&widths);
    // Laid out from the items' fixed sizes: how far the pill can scroll.
    let content = 2.0f64.mul_add(
        PILL_PAD,
        PILL_GAP.mul_add(
            (count(items.len()) - 1.0).max(0.0),
            widths.iter().sum::<f64>(),
        ),
    );
    // The bar's two edges, the pill's gap to the round button, the button.
    let room = 3.0f64.mul_add(-EDGE, screen.width - ins.left - ins.right - PILL);
    let reach = (content - room).max(0.0);
    let x = offset().clamp(-reach, 0.0);
    // Only the items in the pill's window are drawn: Blitz hits what
    // `overflow: hidden` hides, and one scrolled past the pill's end would
    // take a press meant for the round button.
    let (first, end) = in_view(&widths, x, room);
    let before = at.get(first).copied().unwrap_or(0.0);
    let (down, mv, up) = (slide.clone(), slide.clone(), slide.clone());
    let context = context.unwrap_or_default();
    let empty = widths.is_empty();
    // An open dropdown that is still on the bar.
    let menu = open().and_then(|(key, left)| {
        items.iter().find_map(|i| match i {
            TypingItem::Menu(m) if m.key == key => Some((m.clone(), left)),
            _ => None,
        })
    });
    let menu_left = |left: f64| {
        left.clamp(
            EDGE + ins.left,
            (screen.width - ins.right - EDGE - MENU).max(EDGE),
        )
    };
    let shown: Vec<(usize, TypingItem)> = items
        .into_iter()
        .enumerate()
        .take(end)
        .skip(first)
        .collect();
    rsx! {
        div {
            "data-bar": "editing",
            "data-editing": "{context}",
            style: "position: relative; flex-shrink: 0; display: flex; align-items: center; gap: {EDGE}px; \
                    padding: 6px {EDGE + ins.right}px {EDGE}px {EDGE + ins.left}px;",
            if let Some(above) = above {
                div {
                    "data-typing-above": "true",
                    style: "position: absolute; left: {EDGE + ins.left}px; right: {EDGE + ins.right}px; bottom: {PILL + EDGE + 10.0}px; \
                            z-index: 40; display: flex; flex-direction: column; gap: 6px; padding: 8px; border-radius: 14px; \
                            background: {t.sheet}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float};",
                    onpointerdown: move |e: PointerEvent| {
                        e.prevent_default();
                        e.stop_propagation();
                    },
                    onmousedown: move |e: MouseEvent| e.prevent_default(),
                    {above}
                }
            }
            if let Some((m, left)) = menu {
                div {
                    role: "menu",
                    "data-typing-menu": "{m.key}",
                    style: "position: absolute; left: {menu_left(left)}px; bottom: {PILL + EDGE + 10.0}px; z-index: 50; \
                            width: {MENU}px; display: flex; flex-direction: column; padding: 4px; border-radius: 12px; \
                            background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float};",
                    // Its rows keep the field's focus, as the pill's do.
                    onpointerdown: move |e: PointerEvent| {
                        e.prevent_default();
                        e.stop_propagation();
                    },
                    onmousedown: move |e: MouseEvent| e.prevent_default(),
                    onclick: move |e: MouseEvent| e.prevent_default(),
                    {(m.rows)(Callback::new(move |()| open.set(None)))}
                }
            }
            if empty {
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
                        down.set(Some(Slide { x0: e.client_coordinates().x, from: x, moved: false, down: true }));
                    },
                    onmousedown: move |e: MouseEvent| e.prevent_default(),
                    onpointermove: move |e: PointerEvent| {
                        let Some(mut s) = mv.get().filter(|s| s.down) else { return };
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
                            up.set(Some(Slide { x0: 0.0, from: 0.0, moved: true, down: false }));
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
                        for (n, item) in shown {
                            match item {
                                TypingItem::Button(b) => {
                                    let id = b.action.id.clone();
                                    rsx! {
                                        EditButton {
                                            key: "{n}:{id}",
                                            enabled: runner.and_then(|r| r.enabled.call(id.clone())).unwrap_or(true),
                                            checked: b.checked.or_else(|| runner.and_then(|r| r.checked.call(id.clone()))),
                                            text: b.text,
                                            on_press: b.on_press,
                                            slid: slide.clone(),
                                            action: b.action,
                                        }
                                    }
                                }
                                TypingItem::Separator => rsx! {
                                    div {
                                        key: "{n}:|",
                                        role: "separator",
                                        style: "width: 1px; height: 24px; flex-shrink: 0; margin: 0 4px; background: {t.rule};",
                                    }
                                },
                                TypingItem::Menu(m) => {
                                    let left = EDGE + ins.left + PILL_PAD + x + at.get(n).copied().unwrap_or(0.0);
                                    let key = m.key.clone();
                                    let is_open = open.peek().as_ref().is_some_and(|(k, _)| *k == key);
                                    rsx! {
                                        MenuButton {
                                            key: "{n}:menu:{key}",
                                            menu: m,
                                            open: is_open,
                                            slid: slide.clone(),
                                            on_toggle: move |()| {
                                                let again = open.peek().as_ref().is_some_and(|(k, _)| *k == key);
                                                open.set((!again).then(|| (key.clone(), left)));
                                            },
                                        }
                                    }
                                }
                                TypingItem::Widget(w) => {
                                    rsx! {
                                        div {
                                            key: "{n}:widget:{w.key}",
                                            "data-bar-item": "widget:{w.key}",
                                            style: "width: {w.width}px; height: {BUTTON}px; flex-shrink: 0; display: flex; \
                                                    align-items: center; overflow: hidden; border-radius: {BUTTON / 2.0}px;",
                                            // Its own buttons keep the field's focus too.
                                            onclick: move |e: MouseEvent| e.prevent_default(),
                                            {(w.render)()}
                                        }
                                    }
                                }
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
                    open.set(None);
                    let _ = document::eval("document.activeElement && document.activeElement.blur();");
                    on_dismiss.call(());
                },
                KeyboardOff { size: 20, color: t.ink_2.to_string() }
            }
        }
    }
}

/// Which of the items `widths` wide the pill shows at offset `x` in
/// `room` points: from the first that reaches into it to the one before
/// the first past its end.
pub(super) fn in_view(widths: &[f64], x: f64, room: f64) -> (usize, usize) {
    let at = starts(widths);
    let left = |i: usize| PILL_PAD + x + at.get(i).copied().unwrap_or(0.0);
    let right = |i: usize| left(i) + widths.get(i).copied().unwrap_or(0.0);
    let n = widths.len();
    let first = (0..n).find(|&i| right(i) > 0.0).unwrap_or(n);
    let end = (first..n).find(|&i| left(i) >= room).unwrap_or(n);
    (first, end)
}

/// A press on an item: not when it scrolled the pill.
fn pressed(slid: &Rc<Cell<Option<Slide>>>) -> bool {
    !slid.take().is_some_and(|s| s.moved)
}

#[component]
fn EditButton(
    action: BarAction,
    enabled: bool,
    checked: Option<bool>,
    text: bool,
    on_press: Option<Callback<()>>,
    slid: Rc<Cell<Option<Slide>>>,
) -> Element {
    let t = use_shell().theme;
    let on = checked == Some(true);
    let colour = if !enabled {
        &*t.ink_3
    } else if on {
        &*t.accent
    } else {
        &*t.ink
    };
    let fill = if on { &*t.accent_soft } else { "transparent" };
    let opacity = if enabled { 1.0 } else { 0.45 };
    let id = action.id.clone();
    let width = if text {
        text_width(&action.label)
    } else {
        BUTTON
    };
    rsx! {
        button {
            tabindex: "-1",
            "aria-label": "{action.label}",
            "aria-disabled": if enabled { "false" } else { "true" },
            "aria-pressed": checked.map(|on| on.to_string()),
            "data-bar-item": "{action.id}",
            style: "width: {width}px; height: {BUTTON}px; flex-shrink: 0; padding: 0; border: none; \
                    border-radius: {BUTTON / 2.0}px; background: {fill}; display: flex; \
                    align-items: center; justify-content: center; opacity: {opacity};",
            onclick: move |e: MouseEvent| {
                // Blitz takes the focus off the field for a click on
                // nothing focussable: not this one.
                e.prevent_default();
                // The press that scrolled the pill presses nothing.
                if !pressed(&slid) || !enabled {
                    return;
                }
                if let Some(press) = on_press {
                    press.call(());
                } else {
                    run(&id);
                }
            },
            if text {
                span { style: "font-family: {t.font}; font-size: 14px; font-weight: 600; color: {colour}; white-space: nowrap;",
                    "{action.label}"
                }
            } else {
                {(action.icon)(Icon { size: 20, color: colour.into() })}
            }
        }
    }
}

#[component]
fn MenuButton(
    menu: TypingMenu,
    open: bool,
    slid: Rc<Cell<Option<Slide>>>,
    on_toggle: EventHandler<()>,
) -> Element {
    let t = use_shell().theme;
    let colour = if open { &*t.accent } else { &*t.ink };
    let fill = if open { &*t.accent_soft } else { "transparent" };
    let width = TypingItem::Menu(menu.clone()).width();
    rsx! {
        button {
            tabindex: "-1",
            "aria-label": "{menu.label}",
            "aria-haspopup": "menu",
            "aria-expanded": "{open}",
            "data-bar-item": "menu:{menu.key}",
            style: "width: {width}px; height: {BUTTON}px; flex-shrink: 0; padding: 0; border: none; \
                    border-radius: {BUTTON / 2.0}px; background: {fill}; display: flex; gap: 2px; \
                    align-items: center; justify-content: center;",
            onclick: move |e: MouseEvent| {
                e.prevent_default();
                if pressed(&slid) {
                    on_toggle.call(());
                }
            },
            if let Some(icon) = menu.icon {
                {icon(Icon { size: 20, color: colour.into() })}
            } else {
                span { style: "font-family: {t.font}; font-size: 14px; font-weight: 600; color: {colour}; white-space: nowrap;",
                    "{menu.label}"
                }
            }
            span { style: "font-size: 9px; color: {colour}; opacity: 0.7;", "▾" }
        }
    }
}

/// Runs `id` through the app's [`CommandRunner`].
fn run(id: &str) {
    if let Some(runner) = try_consume_context::<CommandRunner>() {
        let _ = runner.run.call(id.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_draws_only_the_items_in_its_window() {
        let nine = [BUTTON; 9];
        // 9 buttons, 46 points apart from 4 in: 0–48, 50–94, …, 372–416.
        assert_eq!(in_view(&nine, 0.0, 330.0), (0, 8));
        assert_eq!(in_view(&nine, -90.0, 330.0), (1, 9), "scrolled to the end");
        assert_eq!(in_view(&nine, -50.0, 330.0), (1, 9));
        assert_eq!(in_view(&nine, -47.0, 330.0), (0, 9), "a sliver still shows");
        assert_eq!(in_view(&nine[..3], 0.0, 330.0), (0, 3), "all fit");
        assert_eq!(in_view(&[], 0.0, 330.0), (0, 0));
        // Items of their own widths: a 200-point widget after a button.
        let mixed = [BUTTON, 200.0, BUTTON, BUTTON];
        // 4–48, 50–250, 252–296, 298–342.
        assert_eq!(in_view(&mixed, 0.0, 280.0), (0, 3));
        assert_eq!(
            in_view(&mixed, -60.0, 280.0),
            (1, 4),
            "the widget still shows"
        );
    }

    #[test]
    fn the_contexts_in_force_are_the_editor_s_then_the_scopes() {
        let t = Typing {
            editor: Some("markdown".to_owned()),
            actions: Vec::new(),
            scopes: vec!["view:note".to_owned(), "view:note".to_owned()],
        };
        assert_eq!(t.contexts(), ["editor:markdown", "view:note"]);
        assert!(Typing::default().contexts().is_empty());
    }

    #[test]
    fn a_label_is_about_as_wide_as_it_is_and_never_narrower_than_a_finger() {
        assert!((text_width("C") - BUTTON).abs() < f64::EPSILON);
        assert!(text_width("Insert a chord") > 100.0);
    }
}
