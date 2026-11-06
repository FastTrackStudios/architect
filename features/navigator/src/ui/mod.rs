//! The navigator on architect's shell (feature `ui`): [`use_navigator`]
//! keeps it, [`NavigatorView`] draws it.
//!
//! Notebook Navigator's view (`src/components/NotebookNavigatorComponent.tsx`):
//! a split container holding the navigation pane and the list pane, side by
//! side, stacked, or one at a time sliding over the other
//! (`styles/sections/layout-panes.css`). Mapped onto architect: the app
//! mounts the view as its left sidebar's content and opens what
//! [`NavigatorView`]'s `on_open` asks for in its workspace; the commands
//! are on the palette (`NAVIGATOR_*`), the settings a section of the
//! settings page, the colours theme roles ([`theme`]).

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use architect_commands::{Command, use_command};
use architect_settings::{UseSettings, use_settings};
use dioxus::prelude::*;
use futures_util::StreamExt as _;

use crate::index::VaultIndex;
use crate::layout::{self, Pane, PaneLayout};
use crate::list::Clock;
use crate::period::PeriodKind;
use crate::settings::{CalendarPlacement, Grouping, ListMode, NavigatorSettings, Orientation};
use crate::source::{NavigatorSource, SourceEvent, Subscription};
use crate::state::{Key as NavKey, Mods, NavigatorState, OpenIn, Outcome};

mod calendar;
mod panes;
pub mod theme;
mod virtual_pane;

pub use theme::{NavigatorColours, TOKENS, with_tokens};

/// Notebook Navigator's `DEBOUNCE_KEYBOARD_FILE_OPEN`: a note the keyboard
/// moved to opens once the keys rest this long.
pub const KEY_OPEN_SETTLE: Duration = Duration::from_millis(500);

/// A note to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub path: String,
    pub how: OpenIn,
    /// The arrow keys moved to it: the person is browsing, so an app may
    /// show it in place of the last rather than open one more tab.
    pub browsing: bool,
}

/// The navigator, kept by the component that called [`use_navigator`]:
/// pass it to [`NavigatorView`], or read and drive it.
#[derive(Clone, Copy)]
pub struct Navigator {
    pub state: Signal<NavigatorState>,
    pub settings: UseSettings<NavigatorSettings>,
    source: CopyValue<Arc<dyn NavigatorSource>>,
    /// The view's width, once measured.
    width: Signal<Option<f64>>,
    /// Bumped by each key that moves to a note: an older wait to open is
    /// stale.
    settle: CopyValue<u64>,
    /// A finger dragged the pane pressed: its release is not a tap.
    pub(crate) dragged: CopyValue<bool>,
    /// A word for the person under the list's header: what a command did
    /// ("Pinned 3 folder notes"), or why it could not.
    pub notice: Signal<Option<String>>,
    /// The list's menu edits the selected place's own list, not the
    /// settings.
    pub own: Signal<bool>,
    /// A periodic note waiting to be made, and its path: the person is
    /// asked first (`calendarConfirmBeforeCreate`).
    pub confirm: Signal<Option<(Outcome, String)>>,
}

/// The navigator over `source`, kept while the caller is mounted: its
/// settings from the settings page (registered on first use), the source's
/// changes taken in as they come.
pub fn use_navigator(source: impl FnOnce() -> Arc<dyn NavigatorSource>) -> Navigator {
    let settings = use_settings::<NavigatorSettings>();
    let source = use_hook(|| CopyValue::new(source()));
    let state = use_signal(|| {
        let s = source.read();
        NavigatorState::new(
            VaultIndex::read(s.as_ref()),
            settings.value.peek().clone(),
            clock_now(),
        )
    });
    let nav = Navigator {
        state,
        settings,
        source,
        width: use_signal(|| None),
        settle: use_hook(|| CopyValue::new(0)),
        dragged: use_hook(|| CopyValue::new(false)),
        notice: use_signal(|| None),
        own: use_signal(|| false),
        confirm: use_signal(|| None),
    };
    // The settings page's changes.
    use_effect(move || {
        let s = settings.read();
        let mut state = state;
        state.write().set_settings(s);
    });
    // The source's changes: heard on its thread, applied here in batches.
    let events = use_hook(move || {
        let (tx, rx) = futures_channel::mpsc::unbounded::<SourceEvent>();
        let sub = source.read().subscribe(Box::new(move |e| {
            let _ = tx.unbounded_send(e.clone());
        }));
        (Rc::new(sub), CopyValue::new(Some(rx)))
    });
    let _kept: &Rc<Subscription> = &events.0;
    let rx = events.1;
    use_future(move || async move {
        let Some(mut rx) = rx.write_unchecked().take() else {
            return;
        };
        while let Some(first) = rx.next().await {
            let mut batch = vec![first];
            while let Ok(e) = rx.try_recv() {
                batch.push(e);
            }
            let mut state = state;
            if batch.iter().any(|e| matches!(e, SourceEvent::Reset)) {
                let index = VaultIndex::read(source.read().as_ref());
                state.write().replace_index(index);
            } else {
                let mut s = state.write();
                for e in &batch {
                    s.apply(e);
                }
            }
        }
    });
    // The day turns over: the date groups follow.
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(Duration::from_secs(60)).await;
            let mut state = state;
            state.write().set_clock(clock_now());
        }
    });
    nav
}

/// Now, in the person's time zone.
fn clock_now() -> Clock {
    let now = chrono::Local::now();
    Clock {
        now: now.timestamp_millis(),
        offset: i64::from(now.offset().local_minus_utc()).saturating_mul(1000),
    }
}

impl PartialEq for Navigator {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl Navigator {
    /// The note open in the workspace (lit, and revealed when the
    /// settings say so).
    pub fn set_active(&self, path: Option<&str>) {
        if self.state.peek().active() != path {
            let mut state = self.state;
            state.write().set_active(path);
        }
    }

    /// The view's [`crate::ViewState`], to keep between runs.
    #[must_use]
    pub fn view_state(&self) -> crate::ViewState {
        self.state.peek().view_state()
    }

    pub fn restore(&self, view: &crate::ViewState) {
        let mut state = self.state;
        state.write().restore(view);
    }

    /// The source it reads.
    #[must_use]
    pub fn source(&self) -> Arc<dyn NavigatorSource> {
        Arc::clone(&self.source.read())
    }

    /// The panes' layout now.
    #[must_use]
    pub fn layout(&self, phone: bool) -> PaneLayout {
        let s = self.state.read();
        layout::resolve(s.settings(), phone, *self.width.read(), s.nav_width)
    }

    /// Changes the navigator with `f`.
    pub fn update<R>(&self, f: impl FnOnce(&mut NavigatorState) -> R) -> R {
        let mut state = self.state;
        f(&mut state.write())
    }

    /// Says `text` under the list's header.
    pub fn say(&self, text: impl Into<String>) {
        let mut notice = self.notice;
        notice.set(Some(text.into()));
    }

    /// Changes the list's look with `f`: the selected place's own list
    /// while [`own`](Self::own) is on (or it has one), else the settings.
    pub fn edit_list(&self, f: impl Fn(&mut crate::Appearance, &mut NavigatorSettings)) {
        let target = self.state.peek().target();
        let has_own = self.state.peek().appearance(&target).is_some();
        if *self.own.peek() || has_own {
            let settings = self.state.peek().settings().clone();
            self.update(|s| {
                s.edit_appearance(|a| {
                    let mut shown = a.over(&settings);
                    f(a, &mut shown);
                    *a = crate::Appearance {
                        target: a.target.clone(),
                        sort: Some(shown.sort),
                        grouping: Some(shown.grouping),
                        mode: Some(shown.mode),
                        title_rows: Some(shown.title_rows),
                        preview_rows: Some(shown.preview_rows),
                        descendants: Some(shown.include_descendants),
                        show_tags: a.show_tags,
                        show_properties: a.show_properties,
                        show_task_progress: a.show_task_progress,
                        text_count: a.text_count,
                    };
                });
            });
        } else {
            self.settings.update(|s| {
                let mut unused = crate::Appearance::default();
                f(&mut unused, s);
            });
        }
    }

    /// Does what an input asked of the app (an [`Outcome`] from the
    /// model): opens a note through `on_open`, or writes to the source.
    pub fn act(&self, out: Option<Outcome>, on_open: EventHandler<Open>) {
        self.outcome(out, on_open, None);
    }

    /// Opens a periodic note, or makes it: asking first when it is new
    /// and the settings say to (`calendarConfirmBeforeCreate`), saying why
    /// not when it cannot be.
    pub fn open_or_create(&self, out: Result<Outcome, String>, on_open: EventHandler<Open>) {
        match out {
            Ok(Outcome::Write {
                ops,
                open: Some((path, how)),
            }) if self.state.peek().settings().calendar_confirm_create => {
                let mut confirm = self.confirm;
                confirm.set(Some((
                    Outcome::Write {
                        ops,
                        open: Some((path.clone(), how)),
                    },
                    path,
                )));
            }
            Ok(out) => self.act(Some(out), on_open),
            Err(e) => self.say(e),
        }
    }

    /// Does what an input asked of the app.
    fn outcome(
        &self,
        out: Option<Outcome>,
        on_open: EventHandler<Open>,
        on_editor: Option<EventHandler<()>>,
    ) {
        match out {
            Some(Outcome::Open {
                path,
                how,
                settle: false,
            }) => {
                let mut settle = self.settle;
                settle.set(settle.cloned().wrapping_add(1));
                on_open.call(Open {
                    path,
                    how,
                    browsing: false,
                });
            }
            Some(Outcome::Open {
                path,
                how,
                settle: true,
            }) => {
                let mut settle = self.settle;
                let n = settle.cloned().wrapping_add(1);
                settle.set(n);
                spawn(async move {
                    futures_timer::Delay::new(KEY_OPEN_SETTLE).await;
                    if settle.cloned() == n {
                        on_open.call(Open {
                            path,
                            how,
                            browsing: true,
                        });
                    }
                });
            }
            Some(Outcome::FocusEditor) => {
                if let Some(e) = on_editor {
                    e.call(());
                }
            }
            Some(Outcome::Write { ops, open }) => match self.source.read().write(&ops) {
                Ok(()) => {
                    if let Some((path, how)) = open {
                        on_open.call(Open {
                            path,
                            how,
                            browsing: false,
                        });
                    }
                }
                Err(e) => self.say(e),
            },
            None => {}
        }
    }
}

/// The key a keyboard event is to the navigator.
fn nav_key(e: &KeyboardEvent) -> Option<(NavKey, Mods)> {
    let m = e.modifiers();
    let command = if cfg!(target_os = "macos") {
        m.meta()
    } else {
        m.ctrl()
    };
    let key = match e.key() {
        Key::Named(NamedKey::ArrowUp) => NavKey::Up,
        Key::Named(NamedKey::ArrowDown) => NavKey::Down,
        Key::Named(NamedKey::ArrowLeft) => NavKey::Left,
        Key::Named(NamedKey::ArrowRight) => NavKey::Right,
        Key::Named(NamedKey::Home) => NavKey::Home,
        Key::Named(NamedKey::End) => NavKey::End,
        Key::Named(NamedKey::PageUp) => NavKey::PageUp,
        Key::Named(NamedKey::PageDown) => NavKey::PageDown,
        Key::Named(NamedKey::Enter) => NavKey::Enter,
        Key::Named(NamedKey::Tab) => NavKey::Tab,
        Key::Named(NamedKey::Escape) => NavKey::Escape,
        Key::Character(c) if c.eq_ignore_ascii_case("a") && command => NavKey::A,
        Key::Character(c) if c.eq_ignore_ascii_case("p") && command && m.shift() => NavKey::Pin,
        _ => return None,
    };
    Some((
        key,
        Mods {
            shift: m.shift(),
            command,
        },
    ))
}

/// `command`, or not now (`why`).
fn when(command: Command, ok: bool, why: &str) -> Command {
    if ok { command } else { command.disabled(why) }
}

/// The palette's `NAVIGATOR_*` commands, while the view is mounted.
fn use_commands(nav: &Navigator) {
    let nav = *nav;
    let group = "Navigator";
    let s = nav.state.read();
    use_command(
        when(
            Command::new("NAVIGATOR_REVEAL", "Reveal the open note").group(group),
            s.active().is_some(),
            "No note is open",
        ),
        move || nav.update(NavigatorState::reveal_active),
    );
    let any = s.any_expanded();
    use_command(
        Command::new(
            "NAVIGATOR_COLLAPSE_OR_EXPAND",
            if any {
                "Collapse all folders"
            } else {
                "Expand all folders"
            },
        )
        .group(group),
        move || nav.update(NavigatorState::collapse_or_expand_all),
    );
    use_command(
        when(
            Command::new("NAVIGATOR_BACK", "Back to the folder before").group(group),
            s.selection().can_go_back(),
            "No folder before this one",
        ),
        move || nav.update(NavigatorState::back),
    );
    use_command(
        when(
            Command::new("NAVIGATOR_FORWARD", "Forward to the next folder").group(group),
            s.selection().can_go_forward(),
            "No folder after this one",
        ),
        move || nav.update(NavigatorState::forward),
    );
    use_command(
        Command::new("NAVIGATOR_FOCUS_FOLDERS", "Go to the folders").group(group),
        move || nav.update(|s| s.focus_pane(Pane::Navigation)),
    );
    use_command(
        Command::new("NAVIGATOR_FOCUS_NOTES", "Go to the notes").group(group),
        move || nav.update(|s| s.focus_pane(Pane::Files)),
    );
    use_command(
        Command::new("NAVIGATOR_SELECT_ALL", "Select every note in the list").group(group),
        move || nav.update(NavigatorState::select_all),
    );
    let settings = s.settings().clone();
    use_command(
        Command::new("NAVIGATOR_TOGGLE_DUAL_PANE", "Two panes")
            .group(group)
            .checked(settings.dual_pane),
        move || nav.settings.update(|s| s.dual_pane = !s.dual_pane),
    );
    use_command(
        Command::new("NAVIGATOR_TOGGLE_ORIENTATION", "Stack the panes")
            .group(group)
            .checked(settings.orientation == Orientation::Vertical),
        move || {
            nav.settings.update(|s| {
                s.orientation = match s.orientation {
                    Orientation::Horizontal => Orientation::Vertical,
                    Orientation::Vertical => Orientation::Horizontal,
                };
            });
        },
    );
    use_command(
        Command::new("NAVIGATOR_TOGGLE_DESCENDANTS", "Notes from folders below")
            .group(group)
            .checked(settings.include_descendants),
        move || {
            nav.settings
                .update(|s| s.include_descendants = !s.include_descendants);
        },
    );
    use_command(
        Command::new("NAVIGATOR_REORDER_ROOTS", "Reorder the top folders")
            .group(group)
            .checked(s.reordering),
        move || nav.update(|s| s.reordering = !s.reordering),
    );
    let shown = s.effective().clone();
    use_command(
        Command::new("NAVIGATOR_TOGGLE_COMPACT", "Compact rows")
            .group(group)
            .checked(shown.mode == ListMode::Compact),
        move || {
            nav.edit_list(|_, s| {
                s.mode = match s.mode {
                    ListMode::Standard => ListMode::Compact,
                    ListMode::Compact => ListMode::Standard,
                };
            });
        },
    );
    for g in Grouping::ALL {
        let id = format!("NAVIGATOR_GROUP_{}", g.id().to_uppercase());
        use_command(
            Command::new(id, format!("Group notes: {}", g.label()))
                .group(group)
                .checked(shown.grouping == g),
            move || nav.edit_list(|_, s| s.grouping = g),
        );
    }
    let cursor = s.selection().cursor().map(str::to_owned);
    let any_picked = !s.selection().files().is_empty();
    use_command(
        when(
            Command::new("NAVIGATOR_PIN", "Pin or unpin the selected notes").group(group),
            any_picked,
            "No note is selected",
        ),
        move || {
            let picked: Vec<String> = nav
                .state
                .peek()
                .selection()
                .files()
                .iter()
                .cloned()
                .collect();
            nav.update(|s| s.toggle_pin(&picked));
        },
    );
    let folder = s.selection().folder().map(str::to_owned);
    let has_note = folder.as_deref().and_then(|f| s.folder_note(f)).is_some();
    let notes_on = s.settings().folder_notes;
    use_command(
        when(
            Command::new("NAVIGATOR_CONVERT_TO_FOLDER_NOTE", "Convert to folder note").group(group),
            notes_on && cursor.is_some(),
            "Turn folder notes on, and select a note",
        ),
        move || {
            let Some(p) = nav.state.peek().selection().cursor().map(str::to_owned) else {
                return;
            };
            match nav.update(|s| s.convert_to_folder_note(&p)) {
                Ok(out) => nav.outcome(Some(out), nav_open(), None),
                Err(e) => nav.say(e),
            }
        },
    );
    use_command(
        when(
            Command::new("NAVIGATOR_SET_AS_FOLDER_NOTE", "Set as folder note").group(group),
            notes_on && cursor.is_some(),
            "Turn folder notes on, and select a note",
        ),
        move || {
            let Some(p) = nav.state.peek().selection().cursor().map(str::to_owned) else {
                return;
            };
            match nav.update(|s| s.set_as_folder_note(&p)) {
                Ok(out) => nav.outcome(Some(out), nav_open(), None),
                Err(e) => nav.say(e),
            }
        },
    );
    use_command(
        when(
            Command::new("NAVIGATOR_DETACH_FOLDER_NOTE", "Detach folder note").group(group),
            has_note,
            "The folder has no note",
        ),
        move || {
            let Some(f) = nav.state.peek().selection().folder().map(str::to_owned) else {
                return;
            };
            match nav.update(|s| s.detach_folder_note(&f, None)) {
                Ok(out) => nav.outcome(Some(out), nav_open(), None),
                Err(e) => nav.say(e),
            }
        },
    );
    use_command(
        when(
            Command::new("NAVIGATOR_PIN_FOLDER_NOTES", "Pin all folder notes").group(group),
            notes_on,
            "Folder notes are off",
        ),
        move || {
            let n = nav.update(NavigatorState::pin_folder_notes);
            nav.say(format!("Pinned {n} folder notes"));
        },
    );
    use_command(
        when(
            Command::new(
                "NAVIGATOR_OPEN_FOLDER_NOTE_TAB",
                "Open the folder note in a new tab",
            )
            .group(group),
            has_note,
            "The folder has no note",
        ),
        move || {
            let out = folder
                .as_deref()
                .and_then(|f| nav.state.peek().open_folder_note(f, true));
            nav.outcome(out, nav_open(), None);
        },
    );
    let calendar_on = s.settings().calendar;
    use_command(
        when(
            Command::new("NAVIGATOR_TOGGLE_CALENDAR", "Calendar")
                .group(group)
                .checked(s.calendar_open),
            calendar_on,
            "The calendar is off in Navigator's settings",
        ),
        move || nav.update(NavigatorState::toggle_calendar),
    );
    for kind in PeriodKind::ALL {
        let adjective = kind.adjective();
        use_command(
            Command::new(
                format!("NAVIGATOR_OPEN_{}_NOTE", adjective.to_uppercase()),
                format!("Open the {adjective} note"),
            )
            .group(group),
            move || {
                let today = nav.state.peek().today();
                let out = nav.state.peek().open_periodic(kind, today, OpenIn::Here);
                nav.open_or_create(out, nav_open());
            },
        );
    }
    let periodic = s.active_period().is_some();
    for (id, label, keys, by) in [
        (
            "NAVIGATOR_NEXT_PERIOD",
            "Open the next periodic note",
            "⌃⌥→",
            1,
        ),
        (
            "NAVIGATOR_PREVIOUS_PERIOD",
            "Open the previous periodic note",
            "⌃⌥←",
            -1,
        ),
        (
            "NAVIGATOR_PARENT_PERIOD",
            "Open the parent periodic note",
            "⌃⌥↑",
            0,
        ),
    ] {
        use_command(
            when(
                Command::new(id, label).group(group).shortcut(keys),
                periodic,
                "The note open is not a periodic note",
            ),
            move || {
                let s = nav.state.peek();
                let out = if by == 0 {
                    s.open_parent_period(OpenIn::Here)
                } else {
                    s.open_adjacent_period(by, OpenIn::Here)
                };
                drop(s);
                nav.open_or_create(out, nav_open());
            },
        );
    }
}

/// The `on_open` the mounted view gave (commands run outside its props).
fn nav_open() -> EventHandler<Open> {
    try_consume_context::<OpenWith>().map_or_else(|| EventHandler::new(|_| {}), |o| o.0)
}

/// The view's `on_open`, in context for its commands.
#[derive(Clone, Copy)]
struct OpenWith(EventHandler<Open>);

/// The navigator drawn: both panes, in the layout its settings and the
/// room give. Mount it as a sidebar's content (it fills its parent).
///
/// `on_open` opens a note in the app's workspace; `on_context_menu` is a
/// right-click on a note (its path); `on_focus_editor` gives the keyboard
/// to the note on show (→ from the list); `phone` is one pane at a time,
/// with a phone's measures. `columns` is the sidebar's when the app's
/// shell sets it (`architect_shell::SidebarLayout::columns`): one is one
/// pane at a time, two is folders beside notes, whatever the settings say.
/// `on_new_note` puts New note in the list's header: it is given the
/// folder on show (`""` for the root), for the app to make a note in.
#[component]
pub fn NavigatorView(
    navigator: Navigator,
    on_open: EventHandler<Open>,
    #[props(default)] on_context_menu: Option<EventHandler<(String, MouseEvent)>>,
    #[props(default)] on_focus_editor: Option<EventHandler<()>>,
    #[props(default)] phone: bool,
    /// Its commands on the palette (`NAVIGATOR_*`); off when an extension
    /// declares them (Navigator's `extension.toml`).
    #[props(default = true)]
    commands: bool,
    #[props(default)] columns: Option<u8>,
    #[props(default)] on_new_note: Option<EventHandler<String>>,
) -> Element {
    let nav = navigator;
    use_context_provider(|| OpenWith(on_open));
    // Hooks run in the same order every time: the choice is made once.
    let with_commands = use_hook(|| commands);
    if with_commands {
        use_commands(&nav);
    }
    let colours = architect_theme::use_themed::<NavigatorColours>();
    let mut root: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    // A phone's taller rows.
    use_effect(move || {
        let mut state = nav.state;
        if state.peek().settings().dual_pane || phone {
            state.write().set_touch(phone);
        }
    });
    // The width, measured again while mounted (Blitz fires no resize).
    use_future(move || async move {
        loop {
            if let Some(m) = root.peek().clone()
                && let Ok(r) = m.get_client_rect().await
            {
                let w = r.size.width;
                if w > 0.0 && nav.width.peek().is_none_or(|old| (old - w).abs() > 0.5) {
                    let mut width = nav.width;
                    width.set(Some(w));
                }
            }
            futures_timer::Delay::new(Duration::from_millis(250)).await;
        }
    });
    let layout = match (columns, phone) {
        (_, true) | (None, false) => nav.layout(phone),
        (Some(0 | 1), false) => PaneLayout::Single,
        (Some(_), false) => PaneLayout::Dual(nav.state.read().settings().orientation),
    };
    let (focus, single, transition_ms, nav_width, nav_height) = {
        let s = nav.state.read();
        (
            s.focus,
            s.single,
            s.settings().transition_ms,
            s.nav_width,
            s.nav_height,
        )
    };
    let slide = use_slide(single, transition_ms);
    let mut drag: Signal<Option<(f64, f64)>> = use_signal(|| None);
    let vars = colours.read().vars();
    let focus_attr = match focus {
        Pane::Navigation => "navigation",
        Pane::Files => "files",
    };
    let on_key = move |e: KeyboardEvent| {
        if let Some((key, mods)) = nav_key(&e) {
            e.prevent_default();
            let out = nav.state.clone().write().key(key, mods);
            nav.outcome(out, on_open, on_focus_editor);
        }
    };
    // The calendar's place (Notebook Navigator's `calendarLeftPlacement`
    // and its vertical split): under the folders side by side, under the
    // notes stacked, and with one pane at a time under the folders or
    // under both.
    let (cal_nav, cal_list, cal_below) = {
        let s = nav.state.read();
        let on = s.calendar_visible();
        match layout {
            PaneLayout::Dual(Orientation::Horizontal) => (on, false, false),
            PaneLayout::Dual(Orientation::Vertical) => (false, on, false),
            PaneLayout::Single => match s.settings().calendar_placement {
                CalendarPlacement::Navigation => (on, false, false),
                CalendarPlacement::Below => (false, false, on),
            },
        }
    };
    let nav_pane = rsx! {
        panes::NavPane { nav, on_open, calendar: cal_nav }
    };
    let list_pane = rsx! {
        panes::ListPane { nav, on_open, on_context_menu, single: layout == PaneLayout::Single, on_new_note, calendar: cal_list }
    };
    let body = match layout {
        PaneLayout::Single => {
            // 0: the folders on show; 1: the notes.
            let p = slide();
            let nav_left = -p * 100.0;
            let list_left = (1.0 - p) * 100.0;
            rsx! {
                div { style: "position:absolute; inset:0; display:flex; flex-direction:column;",
                    div { class: "nv-split nv-single", "data-pane": if single == Pane::Navigation { "navigation" } else { "files" },
                        style: "flex:1 1 0; min-height:0; position:relative; overflow:hidden;",
                        div { class: "nv-slot", style: "position:absolute; top:0; bottom:0; width:100%; left:{nav_left}%;", {nav_pane} }
                        div { class: "nv-slot", style: "position:absolute; top:0; bottom:0; width:100%; left:{list_left}%;", {list_pane} }
                    }
                    if cal_below {
                        calendar::Calendar { nav, on_open }
                    }
                }
            }
        }
        PaneLayout::Dual(orientation) => {
            let horizontal = orientation == Orientation::Horizontal;
            let size = if horizontal { nav_width } else { nav_height };
            let (dir, nav_style, handle_style) = if horizontal {
                (
                    "row",
                    format!("flex:0 0 {size}px; min-width:0; position:relative;"),
                    "flex:0 0 5px; margin:0 -2px; cursor:col-resize; z-index:3; position:relative;",
                )
            } else {
                (
                    "column",
                    format!("flex:0 0 {size}px; min-height:0; position:relative;"),
                    "flex:0 0 5px; margin:-2px 0; cursor:row-resize; z-index:3; position:relative;",
                )
            };
            let room = nav.width.read().filter(|_| horizontal);
            rsx! {
                div { class: "nv-split nv-dual", "data-orientation": if horizontal { "horizontal" } else { "vertical" },
                    style: "position:absolute; inset:0; display:flex; flex-direction:{dir};",
                    div { class: "nv-slot", style: "{nav_style}", {nav_pane} }
                    div {
                        class: "nv-resize",
                        role: "separator",
                        "aria-label": "Resize the folders",
                        style: "{handle_style} touch-action:none;",
                        onpointerdown: move |e: PointerEvent| {
                            e.stop_propagation();
                            let p = e.data().client_coordinates();
                            let at = if horizontal { p.x } else { p.y };
                            drag.set(Some((at, size)));
                        },
                        // Blitz sends the moves and the release to the
                        // element pressed.
                        onpointermove: move |e: PointerEvent| {
                            let Some((start, from)) = *drag.peek() else { return };
                            let p = e.data().client_coordinates();
                            let at = if horizontal { p.x } else { p.y };
                            let to = layout::clamp_size(orientation, from + at - start, room);
                            nav.update(|s| if horizontal { s.nav_width = to } else { s.nav_height = to });
                        },
                        onpointerup: move |_| drag.set(None),
                        div { class: "nv-resize-line", style: if horizontal { "position:absolute; top:0; bottom:0; left:2px; width:1px;" } else { "position:absolute; left:0; right:0; top:2px; height:1px;" } }
                    }
                    div { class: "nv-slot", style: "flex:1 1 0; min-width:0; min-height:0; position:relative;", {list_pane} }
                }
            }
        }
    };
    let confirm = nav.confirm.read().clone();
    rsx! {
        document::Style { {CSS} }
        document::Style { {calendar::CSS} }
        div {
            class: "nv-root",
            "data-navigator": "",
            "data-focus-pane": "{focus_attr}",
            style: "{vars} position:absolute; inset:0; overflow:hidden; background:var(--nv-background); color:var(--nv-text); font-size:13px;",
            onmounted: move |e| root.set(Some(e.data())),
            onkeydown: on_key,
            {body}
            if let Some((out, path)) = confirm {
                ConfirmCreate { nav, on_open, out, path }
            }
        }
    }
}

/// Notebook Navigator's "New daily note" dialog, as a bar over the
/// navigator's foot: the note's path, Cancel and Create.
#[component]
fn ConfirmCreate(
    nav: Navigator,
    on_open: EventHandler<Open>,
    out: Outcome,
    path: String,
) -> Element {
    let mut confirm = nav.confirm;
    rsx! {
        div { class: "nv-confirm", role: "alertdialog", "aria-label": "New note",
            style: "position:absolute; left:8px; right:8px; bottom:8px; z-index:20; padding:10px; border-radius:10px; \
                    background:var(--nv-background); border:1px solid var(--nv-divider); display:flex; flex-direction:column; gap:8px;",
            div { style: "font-weight:600;", "New note" }
            div { style: "color:var(--nv-muted); font-size:12px; overflow-wrap:anywhere;",
                "File {path} does not exist. Would you like to create it?"
            }
            div { style: "display:flex; justify-content:flex-end; gap:6px;",
                button { class: "nv-confirm-cancel", "data-testid": "nv-confirm-cancel",
                    style: "height:26px; padding:0 10px; border-radius:6px; border:1px solid var(--nv-divider); background:transparent; color:var(--nv-text); cursor:pointer;",
                    onclick: move |_| confirm.set(None),
                    "Cancel"
                }
                button { class: "nv-confirm-create", "data-testid": "nv-confirm-create",
                    style: "height:26px; padding:0 10px; border-radius:6px; border:none; background:var(--nv-open); color:var(--nv-background); cursor:pointer; font-weight:600;",
                    onclick: move |_| {
                        confirm.set(None);
                        nav.act(Some(out.clone()), on_open);
                    },
                    "Create"
                }
            }
        }
    }
}

/// The single pane's slide, 0 (the folders) to 1 (the notes), eased over
/// `ms` each time `single` changes (Notebook Navigator's
/// `paneTransitionDuration`, ease-in-out); at once with reduced motion.
fn use_slide(single: Pane, ms: u32) -> Signal<f64> {
    let target: f64 = if single == Pane::Files { 1.0 } else { 0.0 };
    let mut at = use_signal(|| target);
    let mut aim = use_signal(|| target);
    if (*aim.peek() - target).abs() > f64::EPSILON {
        aim.set(target);
        let from = *at.peek();
        let length = f64::from(ms.max(1)) / 1000.0;
        if architect_shell::motion::reduced_motion() {
            at.set(target);
        } else {
            spawn(async move {
                let start = architect_shell::motion::now();
                loop {
                    architect_shell::motion::next_frame().await;
                    if (*aim.peek() - target).abs() > f64::EPSILON {
                        return;
                    }
                    let t = ((architect_shell::motion::now() - start) / length).clamp(0.0, 1.0);
                    at.set((target - from).mul_add(ease_in_out(t), from));
                    if t >= 1.0 {
                        return;
                    }
                }
            });
        }
    }
    at
}

/// CSS's `ease-in-out`, near enough: a smoothstep.
fn ease_in_out(t: f64) -> f64 {
    t * t * 2.0f64.mul_add(-t, 3.0)
}

/// The navigator's sheet: rows read the theme's roles from the root.
const CSS: &str = "
.nv-root .nv-row { cursor: default; }
.nv-root .nv-row:hover { background: var(--nv-hover); }
.nv-root .nv-row.nv-selected { background: var(--nv-selected-inactive); }
.nv-root[data-focus-pane=navigation] .nv-nav .nv-row.nv-selected { background: var(--nv-selected); }
.nv-root[data-focus-pane=files] .nv-list .nv-row.nv-selected { background: var(--nv-selected); }
.nv-root .nv-resize:hover .nv-resize-line { background: var(--nv-handle); }
.nv-root .nv-resize-line { background: var(--nv-divider); }
.nv-root .nv-tool { display:flex; align-items:center; justify-content:center; width:26px; height:26px; border:none; border-radius:6px; background:transparent; color:var(--nv-faint); cursor:pointer; padding:0; }
.nv-root .nv-tool:hover { background: var(--nv-hover); color: var(--nv-text); }
.nv-root .nv-tool.nv-on { color: var(--nv-open); }
.nv-root .nv-menu-row:hover { background: var(--nv-hover); }
.nv-root .nv-thumb { background: var(--nv-faint); opacity: 0.35; }
.nv-root .nv-thumb:hover { opacity: 0.6; }
";
