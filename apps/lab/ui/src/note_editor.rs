//! The lab's note editor (#312): a Vault note's Markdown in the
//! `FastTrackStudios` editor (`editor_view::Editor`, native), worked by
//! extensions through the host service's editor calls
//! (`architect_extensions::editor`).
//!
//! [`Live`] is the lab's `Editors`: what an extension reads is the note
//! being edited, kept in step with it ([`LiveEditor`]); a command an
//! extension sends is made a transaction against it at once (so it answers
//! with the editor after), then applied in the editor through the history
//! the editor was given (`editor_view::dispatch_with_history`): one undo
//! step beside the person's own typing. Its decorations are kept here,
//! moved through every edit, and drawn by the editor
//! (`editor_state::ext::decorations`); a widget mounts the extension's
//! embed (`architect_extensions::embeds::WidgetView`).
//!
//! The editor asks for every key it would handle first: an extension's
//! `[[keymaps]]` entry for it at the cursor runs its action
//! (`editor_state::ext::resolve_key`). Its `[[slash]]` rows are in the
//! slash menu (`\`), and its toolbars of `when = "editor:note"` on the
//! Typing Toolbar while the note is typed in.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, Principal};
use architect_extensions::editor::{DecorationSpec, EditCommand, EditorError, EditorView, Editors};
use architect_extensions::embeds::WidgetView;
use architect_extensions::interface::ExtensionUi;
use dioxus::prelude::*;
use editor_view::editor_state::{EditorState, History, KeySpec, Keymap, TransactionSpec, ext};
use editor_view::palette::{CommandPalette, PaletteState};
use editor_view::{DecorationSource, Editor, TransactionEvent, WidgetMounts};
use facet::Facet;
use futures_util::StreamExt as _;

/// What a note's editor edits: the context a toolbar's `when =
/// "editor:note"` names.
pub const CONTEXT: &str = "note";

#[derive(Default)]
struct Inner {
    /// The note being edited, and what its editor holds.
    editor: Option<String>,
    state: Option<EditorState>,
    version: u64,
    /// Every extension's decorations, by extension.
    decorations: BTreeMap<String, Vec<DecorationSpec>>,
    /// Transactions made here that the editor has yet to apply.
    pending: Vec<TransactionSpec>,
    wake: Option<UnboundedSender<()>>,
}

/// The note being edited, as extensions reach it: one at a time, the one
/// whose editor is mounted.
#[derive(Default)]
pub struct LiveEditor {
    inner: Mutex<Inner>,
}

fn lock(m: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LiveEditor {
    fn view(i: &Inner) -> Option<EditorView> {
        Some(EditorView {
            editor: format!("lab.note/{}", i.editor.as_deref()?),
            context: CONTEXT.to_owned(),
            snapshot: ext::snapshot(i.state.as_ref()?, i.version),
        })
    }

    fn wake(i: &Inner) {
        if let Some(w) = &i.wake {
            let _ = w.unbounded_send(());
        }
    }

    /// Note `id`'s editor is mounted, holding `state`; told of what it is
    /// to apply through `wake`.
    fn attach(&self, id: &str, state: EditorState, wake: UnboundedSender<()>) {
        let mut i = lock(&self.inner);
        i.editor = Some(id.to_owned());
        i.state = Some(state);
        i.version = i.version.wrapping_add(1);
        i.pending.clear();
        i.wake = Some(wake);
    }

    /// Note `id`'s editor went away.
    fn detach(&self, id: &str) {
        let mut i = lock(&self.inner);
        if i.editor.as_deref() == Some(id) {
            *i = Inner {
                decorations: std::mem::take(&mut i.decorations),
                version: i.version,
                ..Inner::default()
            };
        }
    }

    /// The editor applied `event` (the person's typing, a caret move): its
    /// state is `now`.
    fn sync(&self, now: EditorState, event: &TransactionEvent) {
        let mut i = lock(&self.inner);
        if event.is_edit() {
            i.version = i.version.wrapping_add(1);
            for specs in i.decorations.values_mut() {
                *specs = ext::mapped(specs, &event.changes);
            }
        }
        i.state = Some(now);
    }

    /// What the editor is to apply, taken; and every decoration, with its
    /// extension.
    fn take(&self) -> (Vec<TransactionSpec>, Vec<(String, DecorationSpec)>) {
        let mut i = lock(&self.inner);
        let pending = std::mem::take(&mut i.pending);
        (pending, Self::flat(&i))
    }

    fn flat(i: &Inner) -> Vec<(String, DecorationSpec)> {
        i.decorations
            .iter()
            .flat_map(|(owner, specs)| specs.iter().map(move |s| (owner.clone(), s.clone())))
            .collect()
    }

    /// `spec` made against the editor's state now: applied here at once,
    /// and in the editor when it wakes.
    fn push(i: &mut Inner, spec: TransactionSpec) -> Result<(), EditorError> {
        let state = i.state.clone().ok_or(EditorError::NoEditor)?;
        if !spec.changes.is_empty() {
            i.version = i.version.wrapping_add(1);
            for specs in i.decorations.values_mut() {
                *specs = ext::mapped(specs, &spec.changes);
            }
        }
        i.state = Some(state.update(spec.clone()));
        i.pending.push(spec);
        Self::wake(i);
        Ok(())
    }

    /// The caret (or a selection) at `anchor..head`, as a person's click.
    fn select(&self, anchor: usize, head: usize) -> Result<(), EditorError> {
        let mut i = lock(&self.inner);
        let len = i.state.as_ref().ok_or(EditorError::NoEditor)?.doc.len();
        let range = editor_view::editor_state::Range::new(anchor.min(len), head.min(len));
        let spec =
            TransactionSpec::new().selection(editor_view::editor_state::Selection::single(range));
        Self::push(&mut i, spec)
    }
}

static LIVE: LazyLock<LiveEditor> = LazyLock::new(LiveEditor::default);

/// The lab's editors for extensions (`Host::with_editors`): the note being
/// edited.
pub struct Live;

impl Editors for Live {
    fn focused(&self) -> Option<EditorView> {
        LiveEditor::view(&lock(&LIVE.inner))
    }

    fn apply(
        &self,
        edit: &EditCommand,
        step: &str,
        by: &Principal,
    ) -> Result<EditorView, EditorError> {
        let mut i = lock(&LIVE.inner);
        let state = i.state.clone().ok_or(EditorError::NoEditor)?;
        let spec =
            ext::transaction(&state, edit, i.version, &by.describe())?.annotate(ext::STEP, step);
        LiveEditor::push(&mut i, spec)?;
        LiveEditor::view(&i).ok_or(EditorError::NoEditor)
    }

    fn decorate(
        &self,
        extension: &str,
        decorations: Vec<DecorationSpec>,
    ) -> Result<(), EditorError> {
        let mut i = lock(&LIVE.inner);
        let text = i
            .state
            .as_ref()
            .ok_or(EditorError::NoEditor)?
            .doc
            .to_string();
        for d in &decorations {
            d.checked(&text)?;
        }
        if decorations.is_empty() {
            i.decorations.remove(extension);
        } else {
            i.decorations.insert(extension.to_owned(), decorations);
        }
        LiveEditor::wake(&i);
        Ok(())
    }

    fn withdraw(&self, extension: &str) {
        let mut i = lock(&LIVE.inner);
        if i.decorations.remove(extension).is_some() {
            LiveEditor::wake(&i);
        }
    }
}

/// The person's undo and redo, through the editor's history.
fn undo(_: &EditorState) -> Option<TransactionSpec> {
    Some(TransactionSpec::new().user_event("undo"))
}

fn redo(_: &EditorState) -> Option<TransactionSpec> {
    Some(TransactionSpec::new().user_event("redo"))
}

/// The editor's sheet and the lab's look for it, at app level (a `<style>`
/// inside a pane does not apply in Blitz).
const LAYOUT: &str = "
.lab-note-editor { max-width: 640px; box-sizing: border-box; border-radius: 8px;
  border: 1px solid var(--border); background: var(--card); color: var(--foreground);
  padding: 10px 12px 10px 28px; font-family: ui-monospace, Menlo, monospace; font-size: 13px;
  line-height: 1.5; position: relative; }
.lab-note-editor .ext-mark.ext-todo { background: color-mix(in srgb, var(--primary) 22%, transparent); }
.lab-note-editor .editor-gutter { color: var(--muted-foreground); }
.lab-note-editor .ed-menu { position: absolute; z-index: 5; background: var(--popover);
  border: 1px solid var(--border); border-radius: 8px; padding: 4px; min-width: 220px; }
";

/// A Markdown note's editor: `text` to start with; each change of its text
/// given to `on_change`.
#[component]
pub fn NoteEditor(id: String, text: String, on_change: Callback<String>) -> Element {
    let state = use_signal(|| EditorState::new(text.clone()));
    let history = use_signal(History::new);
    let palette = use_signal(|| None::<PaletteState>);
    let mut decos = use_signal(Vec::<(String, DecorationSpec)>::new);
    let ui = try_use_context::<ExtensionUi>();
    let editing = architect_shell::use_editing_actions(CONTEXT, &[]);
    // What extensions made, applied through the editor's history.
    let wake = use_coroutine(move |mut rx: UnboundedReceiver<()>| async move {
        while rx.next().await.is_some() {
            let (pending, flat) = LIVE.take();
            let edited = !pending.is_empty();
            for spec in pending {
                editor_view::dispatch_with_history(state, history, spec, None);
            }
            decos.set(flat);
            if edited {
                on_change.call(state.peek().doc.to_string());
            }
        }
    });
    {
        let id = id.clone();
        use_hook(move || LIVE.attach(&id, state.peek().clone(), wake.tx()));
    }
    {
        let id = id.clone();
        use_drop(move || LIVE.detach(&id));
    }
    // Its slash rows, as extensions come and go.
    let slash: Vec<ext::SlashEntry> = ui
        .as_ref()
        .map(|ui| {
            let _ = ui.revision();
            ui.host()
                .contributions()
                .slash()
                .into_iter()
                .map(|by| by.item)
                .collect()
        })
        .unwrap_or_default();
    let mut registered = use_signal(Vec::<ext::SlashEntry>::new);
    if *registered.peek() != slash {
        editor_view::palette::register_actions(&slash);
        registered.set(slash);
    }
    let run = move |action: String| {
        let Some(ui) = try_consume_context::<ExtensionUi>() else {
            return;
        };
        let list = ui.host().list().clone();
        spawn(async move {
            if let Err(e) = list
                .run_json_async_with(&ActionCaller::app(), &action, b"null")
                .await
            {
                tracing::warn!(action, error = %e, "lab.note_editor.action_failed");
            }
        });
    };
    let on_key = move |press: KeySpec| {
        let Some(ui) = try_consume_context::<ExtensionUi>() else {
            return false;
        };
        let keymaps = ui.host().contributions().keymaps();
        let cur = state.peek().clone();
        let Some(hit) = ext::resolve_key(&keymaps, &press, &cur) else {
            return false;
        };
        run(hit.item.action.clone());
        true
    };
    let on_transaction = move |event: TransactionEvent| {
        LIVE.sync(state.peek().clone(), &event);
        if event.is_edit() {
            decos.set(LIVE.take_flat());
            on_change.call(event.doc_after.to_string());
        }
    };
    let decorations = use_hook(|| {
        DecorationSource::new(move |s: &EditorState| {
            let specs: Vec<DecorationSpec> = decos.read().iter().map(|(_, d)| d.clone()).collect();
            ext::decorations(&specs, &s.doc.to_string())
        })
    });
    let mounts = use_hook(|| {
        WidgetMounts::new(move |mount: &str| {
            let flat = decos.peek();
            match ext::mount_index(mount).and_then(|i| flat.get(i)) {
                Some((owner, DecorationSpec::Widget { view, props, .. })) => rsx! {
                    WidgetView { extension: owner.clone(), id: view.clone(), props: props.clone() }
                },
                _ => rsx! {},
            }
        })
    });
    let keymap = Keymap::new()
        .with("Mod-z", undo)
        .with("Mod-Shift-z", redo)
        .with("Mod-y", redo);
    let css = use_hook(|| format!("{}\n{LAYOUT}", editor_view::EDITOR_CSS));
    rsx! {
        document::Style { {css} }
        div {
            id: "note-source",
            class: "lab-note-editor",
            "data-note-editor": "{id}",
            onfocusin: move |_| editing.focus(),
            onfocusout: move |_| editing.blur(),
            Editor {
                state,
                keymap,
                decorations,
                history,
                on_key,
                mounts,
                palette,
                on_transaction,
                on_action: move |a: String| run(a),
            }
            CommandPalette { state, palette, on_action: move |a: String| run(a) }
        }
    }
}

impl LiveEditor {
    fn take_flat(&self) -> Vec<(String, DecorationSpec)> {
        Self::flat(&lock(&self.inner))
    }
}

/// What `LAB_EDITOR_SELECT` takes.
#[derive(Debug, Clone, Facet)]
struct SelectArgs {
    /// Where the selection starts, a byte offset into the note's text.
    anchor: u64,
    /// Where it ends (the caret): `anchor` when not given.
    head: Option<u64>,
}

static LAB_EDITOR_STATE: ActionMeta = ActionMeta {
    id: "LAB_EDITOR_STATE",
    trait_name: "",
    method_name: "",
    display_name: "The note editor's state",
    description: "The note being edited in the Vault (its Edit mode), as extensions read it: `{editor, context, text, selection, cursor, node, version}` and `decorations` (`[{extension, kind, …}]`); null when no note's editor is open.",
    category: "Lab",
    group: "Editor",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static LAB_EDITOR_SELECT: ActionMeta = ActionMeta {
    id: "LAB_EDITOR_SELECT",
    trait_name: "",
    method_name: "",
    display_name: "Select in the note editor",
    description: "Puts the caret (`anchor`) or a selection (`anchor` to `head`, byte offsets into the text, clamped to it) in the note being edited, as a click would.",
    category: "Lab",
    group: "Editor",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// Puts `LAB_EDITOR_STATE` and `LAB_EDITOR_SELECT` on `list`.
pub fn register(list: &ActionList) {
    let _ = list.register_dynamic(
        &LAB_EDITOR_STATE,
        Effect::ReadOnly,
        None,
        None,
        Arc::new(|_: &[u8]| {
            let i = lock(&LIVE.inner);
            let out = LiveEditor::view(&i).map_or(serde_json::Value::Null, |v| {
                let mut out = serde_json::to_value(v).unwrap_or_default();
                let decorations: Vec<serde_json::Value> = LiveEditor::flat(&i)
                    .into_iter()
                    .map(|(owner, d)| {
                        let mut d = serde_json::to_value(d).unwrap_or_default();
                        if let Some(o) = d.as_object_mut() {
                            o.insert("extension".to_owned(), owner.into());
                        }
                        d
                    })
                    .collect();
                if let Some(o) = out.as_object_mut() {
                    o.insert("decorations".to_owned(), decorations.into());
                }
                out
            });
            Ok(serde_json::to_vec(&out).unwrap_or_default())
        }),
    );
    let _ = list.register_dynamic(
        &LAB_EDITOR_SELECT,
        Effect::Mutating,
        Some(SelectArgs::SHAPE),
        None,
        Arc::new(|bytes: &[u8]| {
            let id = LAB_EDITOR_SELECT.id;
            let args: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
            let at = |k: &str| {
                args.get(k)
                    .and_then(serde_json::Value::as_u64)
                    .map(|v| usize::try_from(v).unwrap_or(usize::MAX))
            };
            let anchor = at("anchor").ok_or_else(|| ActionError::Failed {
                id: id.to_owned(),
                message: "anchor: a byte offset".to_owned(),
            })?;
            let head = at("head").unwrap_or(anchor);
            LIVE.select(anchor, head).map_err(|e| ActionError::Failed {
                id: id.to_owned(),
                message: e.to_string(),
            })?;
            Ok(b"null".to_vec())
        }),
    );
}
