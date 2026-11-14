//! A note's head, at the top of its pane (#710): its one title and its
//! Properties.
//!
//! A vault note says its name once, in the bar at the top: a desk's pane
//! header (between back/forward and `…`), a phone's page bar. It is
//! renamed there (press the name, Enter keeps it, Escape leaves it), a
//! rename `NOTE_RENAME_UNDO` takes back. Its Properties are a button in the
//! same bar, its count on it: a popover under it on a desk, a sheet on a
//! phone (`NOTE_PROPERTIES_TOGGLE`). The body starts with the note's text;
//! the inline title is the Editor's "Show inline title" setting.
//!
//! The note (`crate::vault` `VaultNote`) puts its head here ([`use_note_head`]);
//! the bars draw from it.

use std::collections::BTreeMap;
use std::rc::Rc;

use architect_shell::{Action, layer};
use architect_workspace::{HeaderParts, PaneId, ViewHeader, Workspace};
use dioxus::prelude::*;
use lucide_dioxus::TableProperties;

use crate::properties::{PropertiesBar, Types};

/// The view kind a vault note is (`crate::vault`'s `VAULT_NOTE`).
pub const NOTE_VIEW: &str = "vault.note";

/// Shows or hides the active note's Properties.
pub const PROPERTIES_TOGGLE: &str = "NOTE_PROPERTIES_TOGGLE";
/// Renames the active note in the bar at the top.
pub const RENAME: &str = "NOTE_RENAME";
/// Takes back the last rename made there.
pub const RENAME_UNDO: &str = "NOTE_RENAME_UNDO";

/// What a note open in a pane gives the bars at the top.
#[derive(Clone, PartialEq)]
pub struct NoteHead {
    /// Its name (its file's, without `.md`).
    pub title: String,
    /// Its pane's state: where it is (`@<org>/…` for another org's).
    pub state: String,
    /// Its frontmatter, as the Properties editor writes it.
    pub block: String,
    pub types: Types,
    /// Keys its own list draws (a collection's), not shown as rows.
    pub hidden: Vec<String>,
    /// The frontmatter written again.
    pub on_change: Callback<String>,
    /// A property's link followed.
    pub on_link: Callback<String>,
    /// Renamed to a name (not a path); `true` puts it on the undo list.
    /// Refused with why: a name [`plan_rename`] refuses, or the vault's.
    pub rename: Callback<(String, bool), Result<(), String>>,
}

impl NoteHead {
    /// The count on its Properties button.
    #[must_use]
    pub fn count(&self) -> usize {
        crate::properties::shown_count(&self.block, &self.hidden)
    }
}

/// How a note at `from` is renamed to `name`: where it goes, and the
/// name it passes through on the way for a change of case alone (a
/// case-insensitive disk would see the same file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    pub to: String,
    pub via: Option<String>,
}

/// Plans renaming the note at `from` to `name`, as the navigator checks a
/// name (`architect_navigator::state::check_name`: none, a leading
/// period, `:` or `/` refused); `taken` says whether a path is a note
/// already (any case). `None` when the name is the one it has.
///
/// # Errors
///
/// A name that cannot be, or another note's.
pub fn plan_rename(
    from: &str,
    name: &str,
    taken: impl Fn(&str) -> bool,
) -> Result<Option<RenamePlan>, String> {
    let name = name.trim();
    if let Some(e) = architect_navigator::state::check_name(name).error {
        return Err(e);
    }
    let folder = from.rsplit_once('/').map(|(f, _)| f);
    let at = |stem: &str| folder.map_or_else(|| format!("{stem}.md"), |f| format!("{f}/{stem}.md"));
    let to = at(name);
    if to == from {
        return Ok(None);
    }
    let case_only = to.eq_ignore_ascii_case(from);
    if !case_only && taken(&to) {
        return Err(format!("“{name}” is there already"));
    }
    let via = case_only.then(|| {
        (1_u32..1000)
            .map(|n| at(&format!("{name} (renaming {n})")))
            .find(|p| !taken(p))
            .unwrap_or_default()
    });
    Ok(Some(RenamePlan { to, via }))
}

/// Renames the note at `from` in `disk` to `name` ([`plan_rename`]): its
/// new path, or `None` when it already had that name.
///
/// # Errors
///
/// What [`plan_rename`] refuses, or the vault's own error.
pub fn rename_note(
    disk: &architect_navigator::VaultSource,
    from: &str,
    name: &str,
) -> Result<Option<String>, String> {
    let taken = |p: &str| {
        disk.vault().with(|v| {
            v.pages
                .iter()
                .any(|page| page.rel_path.eq_ignore_ascii_case(p))
        })
    };
    let Some(plan) = plan_rename(from, name, taken)? else {
        return Ok(None);
    };
    if let Some(via) = &plan.via {
        disk.rename(from, via)?;
        if let Err(e) = disk.rename(via, &plan.to) {
            let _ = disk.rename(via, from);
            return Err(e);
        }
    } else {
        disk.rename(from, &plan.to)?;
    }
    Ok(Some(plan.to))
}

/// A rename made in the bar: the note's state before and after.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Renamed {
    pub from: String,
    pub to: String,
}

/// Every open note's head by its pane, and what the bars are doing with
/// them: one at the app's root.
#[derive(Clone, Copy)]
pub struct NoteHeads {
    heads: Signal<BTreeMap<String, NoteHead>>,
    /// The pane whose title is being typed.
    pub editing: Signal<Option<String>>,
    /// The pane whose Properties are open.
    pub open: Signal<Option<String>>,
    renames: Signal<Vec<Renamed>>,
}

impl NoteHeads {
    /// The head of the note in `pane`, if it is one.
    #[must_use]
    pub fn of(&self, pane: &str) -> Option<NoteHead> {
        self.heads.read().get(pane).cloned()
    }

    /// Opens or closes `pane`'s Properties.
    pub fn toggle(mut self, pane: &str) {
        let now = self.open.peek().as_deref() == Some(pane);
        self.open.set((!now).then(|| pane.to_owned()));
    }

    /// Says a rename was made, for [`RENAME_UNDO`].
    pub fn renamed(mut self, from: String, to: String) {
        self.renames.write().push(Renamed { from, to });
    }

    /// The last rename, to take back.
    #[must_use]
    pub fn last_rename(&self) -> Option<Renamed> {
        self.renames.read().last().cloned()
    }

    /// Takes the last rename off the list.
    pub fn pop_rename(mut self) -> Option<Renamed> {
        self.renames.write().pop()
    }

    /// The open note's head whose state is `state`.
    #[must_use]
    pub fn by_state(&self, state: &str) -> Option<NoteHead> {
        self.heads
            .peek()
            .values()
            .find(|h| h.state == state)
            .cloned()
    }
}

/// The app's [`NoteHeads`].
pub fn use_note_heads() -> NoteHeads {
    use_root_context(|| NoteHeads {
        heads: Signal::new_in_scope(BTreeMap::new(), ScopeId::ROOT),
        editing: Signal::new_in_scope(None, ScopeId::ROOT),
        open: Signal::new_in_scope(None, ScopeId::ROOT),
        renames: Signal::new_in_scope(Vec::new(), ScopeId::ROOT),
    })
}

/// Puts the note in `pane` in the bars at the top while it is open.
#[allow(clippy::needless_pass_by_value)] // kept as the effect's dependency
pub fn use_note_head(pane: String, head: NoteHead) {
    let heads = use_note_heads();
    let key = pane.clone();
    use_effect(use_reactive!(|head| {
        let mut map = heads.heads;
        if map.peek().get(&key) != Some(&head) {
            map.write().insert(key.clone(), head);
        }
    }));
    use_drop(move || {
        let (mut map, mut open, mut editing) = (heads.heads, heads.open, heads.editing);
        map.write().remove(&pane);
        if open.peek().as_deref() == Some(pane.as_str()) {
            open.set(None);
        }
        if editing.peek().as_deref() == Some(pane.as_str()) {
            editing.set(None);
        }
    });
}

/// The workspace's [`HeaderParts`] for a vault note: its title, renamed
/// there, and its Properties button. Call it above the workspace.
pub fn use_note_header_parts() {
    use_context_provider(|| {
        HeaderParts::new(|kind: &str, pane: &PaneId| {
            (kind == NOTE_VIEW).then(|| {
                let pane = pane.0.clone();
                ViewHeader {
                    title: Some(rsx! { NoteTitle { pane: pane.clone() } }),
                    tools: Some(rsx! { PropertiesButton { pane } }),
                }
            })
        })
    });
}

/// The active pane's id, when it shows a vault note.
fn active_note(workspace: Signal<Workspace>) -> Option<String> {
    let w = workspace.read();
    let id = w.active.as_ref()?;
    let pane = w.pane(id)?;
    (pane.view.0 == NOTE_VIEW).then(|| id.0.clone())
}

/// ⌘K's and the keys' note commands, on the active note:
/// [`PROPERTIES_TOGGLE`], [`RENAME`] and [`RENAME_UNDO`].
///
/// `rename_closed` renames a note no pane has open (state from, state to).
pub fn use_note_commands(workspace: Signal<Workspace>, rename_closed: Callback<Renamed>) {
    let heads = use_note_heads();
    let active = active_note(workspace);
    let on_note = |c: architect_commands::Command| {
        if active.is_some() {
            c
        } else {
            c.disabled("no note is open")
        }
    };
    architect_commands::use_command(
        on_note(
            architect_commands::Command::new(
                PROPERTIES_TOGGLE,
                "Show or hide the note's properties",
            )
            .group("Note"),
        ),
        move || {
            if let Some(pane) = active_note(workspace) {
                heads.toggle(&pane);
            }
        },
    );
    architect_commands::use_command(
        on_note(architect_commands::Command::new(RENAME, "Rename the note").group("Note")),
        move || {
            let mut editing = heads.editing;
            if let Some(pane) = active_note(workspace) {
                editing.set(Some(pane));
            }
        },
    );
    let last = heads.last_rename();
    let undo = architect_commands::Command::new(
        RENAME_UNDO,
        last.as_ref().map_or_else(
            || "Undo the rename".to_owned(),
            |r| format!("Undo the rename to “{}”", name_of(&r.to)),
        ),
    )
    .group("Note");
    architect_commands::use_command(
        if last.is_some() {
            undo
        } else {
            undo.disabled("nothing renamed yet")
        },
        move || {
            let Some(r) = heads.pop_rename() else { return };
            // The note open: renamed by its pane, which follows its file.
            if let Some(head) = heads.by_state(&r.to) {
                let _ = head.rename.call((name_of(&r.from), false));
            } else {
                rename_closed.call(r);
            }
        },
    );
}

/// A note state's name: its file's, without its folder and `.md`.
#[must_use]
pub fn name_of(state: &str) -> String {
    let file = state.rsplit('/').next().unwrap_or(state);
    file.strip_suffix(".md").unwrap_or(file).to_owned()
}

/// A phone's page bar's Properties button, when the active pane is a
/// note: its count on it.
pub fn phone_properties(workspace: Signal<Workspace>) -> Option<Action> {
    let heads = use_note_heads();
    let pane = active_note(workspace)?;
    let head = heads.of(&pane)?;
    Some(
        Action::new(
            "Properties",
            |i| rsx! { TableProperties { size: i.size, color: i.color } },
            move |()| heads.toggle(&pane),
        )
        .with_badge(head.count()),
    )
}

/// A note's title in the bar at the top: press it to rename the note.
#[component]
pub fn NoteTitle(
    pane: String,
    /// A phone's page bar's: large, at the left.
    #[props(default)]
    large: bool,
) -> Element {
    let t = crate::properties::theme();
    let heads = use_note_heads();
    let head = heads.of(&pane);
    let mut draft = use_signal(|| None::<String>);
    let Some(head) = head else {
        return rsx! {};
    };
    let editing = heads.editing.read().as_deref() == Some(pane.as_str());
    let (size, weight, align) = if large {
        (28, 750, "left")
    } else {
        (13, 500, "center")
    };
    let font = format!(
        "font-size: {size}px; font-weight: {weight}; line-height: 1.2; letter-spacing: {};",
        if large { "-0.02em" } else { "0" }
    );
    let rename = head.rename;
    let mut error = use_signal(|| None::<String>);
    // Enter: renamed, or the field stays open saying why not.
    let commit = move || {
        let (mut editing, mut draft, mut error) = (heads.editing, draft, error);
        let Some(name) = draft.peek().clone() else {
            editing.set(None);
            return;
        };
        match rename.call((name, true)) {
            Ok(()) => {
                draft.set(None);
                error.set(None);
                editing.set(None);
            }
            Err(e) => error.set(Some(e)),
        }
    };
    // Away from the field: renamed when it can be, else left as it was.
    let leave = move || {
        let (mut editing, mut draft, mut error) = (heads.editing, draft, error);
        if let Some(name) = draft.take() {
            let _ = rename.call((name, true));
        }
        error.set(None);
        editing.set(None);
    };
    if editing {
        let shown = draft().unwrap_or_else(|| head.title.clone());
        // Said as it is typed, as the navigator says it; a taken name on Enter.
        let why = error()
            .or_else(|| draft().and_then(|d| architect_navigator::state::check_name(&d).error));
        return rsx! {
            div { style: "position: relative; flex: 1 1 auto; min-width: 0; width: 100%; display: flex;",
            input {
                "data-testid": "note-title-input",
                "aria-label": "Note title",
                style: "{font} flex: 1 1 auto; min-width: 0; width: 100%; box-sizing: border-box; margin: 0; padding: 2px 6px; \
                        border: 1px solid {t.accent}; border-radius: 6px; outline: none; background: {t.sheet}; \
                        color: {t.ink}; text-align: {align};",
                value: "{shown}",
                onmounted: move |e| async move {
                    let _ = e.set_focus(true).await;
                },
                oninput: move |e| {
                    error.set(None);
                    draft.set(Some(e.value()));
                },
                onkeydown: move |e: KeyboardEvent| match e.key() {
                    Key::Named(NamedKey::Enter) => {
                        e.prevent_default();
                        commit();
                    }
                    Key::Named(NamedKey::Escape) => {
                        e.prevent_default();
                        e.stop_propagation();
                        draft.set(None);
                        error.set(None);
                        let mut editing = heads.editing;
                        editing.set(None);
                    }
                    _ => {}
                },
                onblur: move |_| leave(),
            }
            if let Some(why) = why {
                div {
                    "data-testid": "note-title-error",
                    role: "alert",
                    style: "position: absolute; top: 100%; left: 0; right: 0; margin-top: 4px; z-index: {layer::MENU}; \
                            padding: 4px 8px; border-radius: 6px; background: {t.pill}; border: 1px solid {t.rule}; \
                            color: {t.ink}; font-size: 12px; font-weight: 500; text-align: left; white-space: normal;",
                    "{why}"
                }
            }
            }
        };
    }
    let ink = if large { &t.ink } else { &t.ink_2 };
    let edit_pane = pane.clone();
    rsx! {
        div {
            "data-testid": "note-title",
            role: "button",
            tabindex: "0",
            title: "Rename",
            "aria-label": "{head.title}, rename",
            style: "{font} min-width: 0; max-width: 100%; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; \
                    padding: 2px 6px; border-radius: 6px; cursor: text; color: {ink}; text-align: {align};",
            onclick: move |_| {
                let mut editing = heads.editing;
                editing.set(Some(edit_pane.clone()));
            },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Enter) {
                    let mut editing = heads.editing;
                    editing.set(Some(pane.clone()));
                }
            },
            "{head.title}"
        }
    }
}

/// A note's Properties button in its pane's header, its count on it; its
/// popover opens under it.
#[component]
pub fn PropertiesButton(pane: String) -> Element {
    let t = crate::properties::theme();
    let heads = use_note_heads();
    let mut button = use_signal(|| None::<Rc<MountedData>>);
    let open = heads.open.read().as_deref() == Some(pane.as_str());
    // Closed (Escape, the button, the command, a press outside): the
    // keys go back to the button.
    let mut was = use_signal(|| false);
    use_effect(use_reactive!(|open| {
        if *was.peek()
            && !open
            && let Some(b) = button.peek().clone()
        {
            spawn(async move {
                let _ = b.set_focus(true).await;
            });
        }
        was.set(open);
    }));
    let Some(head) = heads.of(&pane) else {
        return rsx! {};
    };
    let count = head.count();
    let fill = if open { &*t.active } else { "transparent" };
    let toggle = pane.clone();
    let close = pane.clone();
    rsx! {
        div { style: "position: relative; flex: 0 0 auto; display: flex;",
            button {
                "data-testid": "note-properties-button",
                "data-count": "{count}",
                "aria-label": "Properties",
                "aria-expanded": "{open}",
                title: "Properties",
                style: "height: 28px; min-width: 28px; box-sizing: border-box; padding: 0 6px; border: none; border-radius: 6px; \
                        background: {fill}; color: {t.ink_2}; display: flex; align-items: center; justify-content: center; gap: 4px; \
                        font-size: 11px; font-weight: 650;",
                onmounted: move |e| button.set(Some(e.data())),
                onclick: move |_| heads.toggle(&toggle),
                TableProperties { size: 16, color: t.ink_2.to_string() }
                if count > 0 {
                    span {
                        style: "min-width: 16px; height: 16px; padding: 0 4px; box-sizing: border-box; border-radius: 8px; \
                                display: flex; align-items: center; justify-content: center; \
                                background: {t.pill}; color: {t.ink_2};",
                        "{count}"
                    }
                }
            }
            if open {
                // A press outside closes it; far past every edge (Blitz
                // has no `position: fixed`).
                div {
                    style: "position: absolute; top: -2000px; left: -4000px; width: 8000px; height: 8000px; z-index: {layer::MENU};",
                    onclick: move |_| heads.toggle(&close),
                }
                div {
                    style: "position: absolute; top: 34px; right: -40px; z-index: {layer::MENU}; width: 340px; max-height: 420px; \
                            overflow-y: scroll; box-sizing: border-box; padding: 10px 12px; border-radius: 12px; \
                            background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float};",
                    PropertiesPanel { pane, head }
                }
            }
        }
    }
}

/// A phone's Properties sheet, over the page's foot. Put it last in the
/// page's body, which it lays against.
#[component]
pub fn PropertiesSheet() -> Element {
    let t = crate::properties::theme();
    let heads = use_note_heads();
    let Some(pane) = heads.open.read().clone() else {
        return rsx! {};
    };
    let Some(head) = heads.of(&pane) else {
        return rsx! {};
    };
    let close = pane.clone();
    let done = pane.clone();
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {layer::SHEET}; background: rgba(0,0,0,0.28);",
            onclick: move |_| heads.toggle(&close),
        }
        div {
            "data-testid": "note-properties-sheet",
            style: "position: absolute; left: 0; right: 0; bottom: 0; z-index: {layer::SHEET}; max-height: 72%; overflow-y: scroll; \
                    box-sizing: border-box; padding: 8px 16px 28px; border-radius: 16px 16px 0 0; \
                    background: {t.sheet}; border-top: 1px solid {t.rule}; box-shadow: {t.shadow_float};",
            div { style: "display: flex; justify-content: center; padding-bottom: 6px;",
                div { style: "width: 36px; height: 5px; border-radius: 3px; background: {t.rule};" }
            }
            div { style: "display: flex; align-items: center; justify-content: space-between; padding: 0 2px 6px;",
                span { style: "font-size: 17px; font-weight: 650; color: {t.ink};", "Properties" }
                button {
                    class: "lp",
                    style: "height: 32px; padding: 0 2px 0 10px; border: none; background: transparent; color: {t.accent}; \
                            font-size: 16px; font-weight: 650;",
                    onclick: move |_| heads.toggle(&done),
                    "Done"
                }
            }
            PropertiesPanel { pane, head, heading: false }
        }
    }
}

/// The Properties editor inside the popover or the sheet: focused as it
/// opens, Escape closes it.
#[component]
fn PropertiesPanel(
    pane: String,
    head: NoteHead,
    /// Says "Properties" over the rows (the sheet says it in its bar).
    #[props(default = true)]
    heading: bool,
) -> Element {
    let t = crate::properties::theme();
    let heads = use_note_heads();
    rsx! {
        div {
            "data-testid": "note-properties-panel",
            tabindex: "0",
            "aria-label": "Properties",
            style: "outline: none; display: flex; flex-direction: column; gap: 4px;",
            onmounted: move |e| async move {
                let _ = e.set_focus(true).await;
            },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    e.stop_propagation();
                    heads.toggle(&pane);
                }
            },
            if heading {
                div { style: "font-size: 12px; font-weight: 650; color: {t.ink_3}; padding: 0 2px 4px;", "Properties" }
            }
            PropertiesBar {
                block: head.block,
                types: head.types,
                hidden: head.hidden,
                on_change: head.on_change,
                on_link: head.on_link,
                bare: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::cell::RefCell;

    use blitz_dom::{Document as _, DocumentConfig};
    use blitz_traits::events::{
        BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, Code, KeyState, Location, Modifiers,
        MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails, UiEvent,
    };
    use blitz_traits::shell::{ColorScheme, Viewport};
    use dioxus_native::DioxusDocument;

    use super::*;

    thread_local! {
        /// The names the header renamed the note to.
        static RENAMED: RefCell<Vec<(String, bool)>> = const { RefCell::new(Vec::new()) };
    }

    #[test]
    fn a_states_name_is_its_files_without_folder_or_md() {
        assert_eq!(name_of("Showcase.md"), "Showcase");
        assert_eq!(name_of("Projects/Launch plan.md"), "Launch plan");
        assert_eq!(name_of("@org/Notes/A.md"), "A");
    }

    /// A note's head as a pane's header draws it: its title, then its
    /// Properties button, over the pane.
    #[component]
    fn Header() -> Element {
        let block = use_signal(|| "---\ntitle: Showcase\ntags: [showcase]\n---\n".to_owned());
        let on_change = use_callback(move |b: String| {
            let mut block = block;
            block.set(b);
        });
        // As the note does: a name the plan refuses, refused; "Taken" is
        // another note's.
        let rename = use_callback(|r: (String, bool)| -> Result<(), String> {
            plan_rename("Showcase.md", &r.0, |p| p == "Taken.md")?;
            RENAMED.with(|n| n.borrow_mut().push(r));
            Ok(())
        });
        use_note_head(
            "p1".to_owned(),
            NoteHead {
                title: "Showcase".to_owned(),
                state: "Showcase.md".to_owned(),
                block: block(),
                types: Types::default(),
                hidden: Vec::new(),
                on_change,
                on_link: use_callback(|_: String| {}),
                rename,
            },
        );
        rsx! {
            div { style: "position: absolute; left: 0; top: 0; width: 800px; height: 600px;",
                div { style: "height: 40px; display: flex; align-items: center; padding: 0 8px;",
                    div { style: "flex: 1 1 0; min-width: 0; display: flex; justify-content: center;",
                        NoteTitle { pane: "p1" }
                    }
                    PropertiesButton { pane: "p1" }
                }
            }
        }
    }

    fn mount() -> DioxusDocument {
        let mut doc = DioxusDocument::new(
            VirtualDom::new(Header),
            DocumentConfig {
                viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
                ..Default::default()
            },
        );
        doc.initial_build();
        settle(&mut doc);
        doc
    }

    fn settle(doc: &mut DioxusDocument) {
        for _ in 0..6 {
            doc.poll(None);
            doc.inner.borrow_mut().resolve(0.0);
        }
    }

    fn count(doc: &DioxusDocument, selector: &str) -> usize {
        doc.inner
            .borrow()
            .query_selector_all(selector)
            .unwrap()
            .len()
    }

    fn text(doc: &DioxusDocument, selector: &str) -> String {
        let d = doc.inner.borrow();
        let id = d
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no {selector}"));
        d.get_node(id).unwrap().text_content()
    }

    fn pointer(x: f32, y: f32, buttons: MouseEventButtons) -> BlitzPointerEvent {
        BlitzPointerEvent {
            id: BlitzPointerId::Mouse,
            is_primary: true,
            coords: PointerCoords {
                page_x: x,
                page_y: y,
                screen_x: x,
                screen_y: y,
                client_x: x,
                client_y: y,
            },
            button: MouseEventButton::Main,
            buttons,
            mods: Modifiers::empty(),
            details: PointerDetails::default(),
            element: blitz_traits::events::Point { x: 0.0, y: 0.0 },
            active_pointers: std::sync::Arc::default(),
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    fn click(doc: &mut DioxusDocument, selector: &str) {
        let (x, y) = {
            let d = doc.inner.borrow();
            let id = d
                .query_selector(selector)
                .unwrap()
                .unwrap_or_else(|| panic!("no {selector}"));
            let r = d.get_client_bounding_rect(id).unwrap();
            ((r.x + r.width / 2.0) as f32, (r.y + r.height / 2.0) as f32)
        };
        doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, MouseEventButtons::None)));
        doc.handle_ui_event(UiEvent::PointerDown(pointer(
            x,
            y,
            MouseEventButtons::Primary,
        )));
        doc.handle_ui_event(UiEvent::PointerUp(pointer(x, y, MouseEventButtons::None)));
        settle(doc);
    }

    /// A key by its name (`Enter`, `Escape`) or the character it types.
    fn key(doc: &mut DioxusDocument, name: &str) {
        let key = name
            .parse::<blitz_traits::events::Key>()
            .unwrap_or_else(|_| blitz_traits::events::Key::Character(name.into()));
        let text = (name.chars().count() == 1).then_some(name);
        let event = |state| BlitzKeyEvent {
            key: key.clone(),
            code: Code::Unidentified,
            modifiers: Modifiers::empty(),
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state,
            text: text.map(Into::into),
        };
        doc.handle_ui_event(UiEvent::KeyDown(event(KeyState::Pressed)));
        doc.handle_ui_event(UiEvent::KeyUp(event(KeyState::Released)));
        settle(doc);
    }

    #[test]
    fn the_title_is_drawn_once_and_the_properties_button_opens_the_editor() {
        let mut doc = mount();
        assert_eq!(count(&doc, "[data-testid=note-title]"), 1);
        assert_eq!(text(&doc, "[data-testid=note-title]"), "Showcase");
        // Its count on it; the editor closed.
        assert_eq!(text(&doc, "[data-testid=note-properties-button]"), "2");
        assert_eq!(count(&doc, "[data-testid=note-properties-panel]"), 0);
        click(&mut doc, "[data-testid=note-properties-button]");
        assert_eq!(
            count(&doc, "[data-testid=note-properties-panel]"),
            1,
            "opened"
        );
        assert_eq!(
            count(&doc, "[data-testid=note-property-add]"),
            1,
            "Add property"
        );
        // Escape closes it.
        key(&mut doc, "Escape");
        assert_eq!(
            count(&doc, "[data-testid=note-properties-panel]"),
            0,
            "closed"
        );
        // The command opens it as the button does.
        doc.vdom.in_scope(ScopeId::ROOT, || {
            consume_context::<NoteHeads>().toggle("p1");
        });
        settle(&mut doc);
        assert_eq!(
            count(&doc, "[data-testid=note-properties-panel]"),
            1,
            "toggled"
        );
    }

    #[test]
    fn the_header_renames_on_enter_and_leaves_it_on_escape() {
        RENAMED.with(|n| n.borrow_mut().clear());
        let mut doc = mount();
        // Escape: nothing renamed, the title back.
        click(&mut doc, "[data-testid=note-title]");
        assert_eq!(count(&doc, "[data-testid=note-title-input]"), 1, "editing");
        key(&mut doc, "X");
        key(&mut doc, "Escape");
        assert_eq!(count(&doc, "[data-testid=note-title-input]"), 0);
        assert_eq!(count(&doc, "[data-testid=note-title]"), 1);
        assert!(
            RENAMED.with(|n| n.borrow().is_empty()),
            "Escape renames nothing"
        );
        // Enter: renamed, for undo.
        click(&mut doc, "[data-testid=note-title]");
        key(&mut doc, "X");
        key(&mut doc, "Enter");
        let renamed = RENAMED.with(|n| n.borrow().clone());
        assert_eq!(renamed.len(), 1, "{renamed:?}");
        let (name, record) = &renamed[0];
        assert!(*record, "on the undo list");
        assert_eq!(name.len(), "ShowcaseX".len(), "{name}");
        assert!(
            name.contains('X') && name.replace('X', "") == "Showcase",
            "{name}"
        );
    }

    #[test]
    fn a_rename_is_checked_as_the_navigator_checks_a_name() {
        let none = |_: &str| false;
        for bad in [".draft", "a:b", "a/b", "  "] {
            assert!(
                plan_rename("Notes/A.md", bad, none).is_err(),
                "{bad:?} refused"
            );
        }
        assert_eq!(
            plan_rename("Notes/A.md", "B", |p| p == "Notes/B.md"),
            Err("“B” is there already".to_owned())
        );
        assert_eq!(plan_rename("Notes/A.md", "A", none), Ok(None));
        assert_eq!(
            plan_rename("Notes/A.md", "B", none),
            Ok(Some(RenamePlan {
                to: "Notes/B.md".to_owned(),
                via: None
            }))
        );
        // A change of case alone passes through another name.
        let plan = plan_rename("Notes/a.md", "A", |p| p.eq_ignore_ascii_case("Notes/a.md"))
            .unwrap()
            .unwrap();
        assert_eq!(plan.to, "Notes/A.md");
        assert_eq!(plan.via.as_deref(), Some("Notes/A (renaming 1).md"));
    }

    #[test]
    fn a_note_on_disk_is_renamed_refused_and_renamed_back() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notes")).unwrap();
        std::fs::write(dir.path().join("Notes/Plan.md"), "body\n").unwrap();
        std::fs::write(dir.path().join("Notes/Other.md"), "other\n").unwrap();
        let disk = architect_navigator::VaultSource::new(
            vault::actions::OpenVault::open(dir.path()).unwrap(),
        );
        // Refused: a leading period, a colon, another note's name.
        assert!(rename_note(&disk, "Notes/Plan.md", ".draft").is_err());
        assert!(rename_note(&disk, "Notes/Plan.md", "a:b").is_err());
        assert!(
            rename_note(&disk, "Notes/Plan.md", "other").is_err(),
            "taken in any case"
        );
        assert!(dir.path().join("Notes/Plan.md").is_file());
        // A change of case alone.
        assert_eq!(
            rename_note(&disk, "Notes/Plan.md", "PLAN")
                .unwrap()
                .as_deref(),
            Some("Notes/PLAN.md")
        );
        let names: Vec<String> = std::fs::read_dir(dir.path().join("Notes"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"PLAN.md".to_owned()), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("renaming")), "{names:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Notes/PLAN.md")).unwrap(),
            "body\n"
        );
        // Renamed back, as NOTE_RENAME_UNDO renames a note no pane has open.
        assert_eq!(
            rename_note(&disk, "Notes/PLAN.md", &name_of("Notes/Plan.md"))
                .unwrap()
                .as_deref(),
            Some("Notes/Plan.md")
        );
    }

    #[test]
    fn a_refused_name_keeps_the_title_open_saying_why() {
        RENAMED.with(|n| n.borrow_mut().clear());
        let mut doc = mount();
        click(&mut doc, "[data-testid=note-title]");
        // A colon is refused wherever the caret put it.
        key(&mut doc, ":");
        assert_eq!(
            count(&doc, "[data-testid=note-title-error]"),
            1,
            "said as it is typed"
        );
        key(&mut doc, "Enter");
        assert_eq!(
            count(&doc, "[data-testid=note-title-input]"),
            1,
            "still open"
        );
        assert_eq!(count(&doc, "[data-testid=note-title-error]"), 1);
        assert!(RENAMED.with(|n| n.borrow().is_empty()), "nothing renamed");
    }
}
