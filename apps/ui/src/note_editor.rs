//! The lab's note editor (#312): a Vault note's Markdown in the
//! `FastTrackStudios` editor (`architect_editor::ui::Editor`, native), worked by
//! extensions through the host service's editor calls
//! (`architect_extensions::editor`).
//!
//! [`Live`] is the lab's `Editors`: what an extension reads is the note
//! being edited, kept in step with it ([`LiveEditor`]); a command an
//! extension sends is made a transaction against it at once (so it answers
//! with the editor after), then applied in the editor through the history
//! the editor was given (`architect_editor::ui::dispatch_with_history`): one undo
//! step beside the person's own typing. Its decorations are kept here,
//! moved through every edit, and drawn by the editor
//! (`architect_editor::state::ext::decorations`); a widget mounts the extension's
//! embed (`architect_extensions::embeds::WidgetView`).
//!
//! The editor asks for every key it would handle first: an extension's
//! `[[keymaps]]` entry for it at the cursor runs its action
//! (`architect_editor::state::ext::resolve_key`). Its `[[slash]]` rows are in the
//! slash menu (`\`), and its toolbars of `when = "editor:note"` on the
//! Typing Toolbar while the note is typed in.
//!
//! The note is drawn as the editor draws Markdown
//! (`architect_editor::state::markdown::live_preview_with`, Obsidian's live preview:
//! a line's markup shows while the caret is on it), its `[[links]]`
//! resolved against the vault ([`NoteVault`]) and its embedded images
//! read from it. Before #442 it was given only the extensions'
//! decorations, so a note read as its raw source in the editor's own
//! 480px scroll box (`--ed-scroll-height`); now it grows with the note,
//! and the pane scrolls.
//!
//! Its fence registry is the extensions' (`Embeds::fences`): a fenced
//! block whose language an extension claims (the Board's ```` ```kf ````)
//! keeps its source, and under it the extension's embed is mounted as a
//! block widget (`architect_extensions::embeds::EmbedView`), given the
//! fence's body as it is typed, sized to what it draws; it goes when the
//! extension does ([`claimed_fences`]).

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, Principal};
use architect_editor::state::markdown::{VaultBlockHit, VaultLookup, VaultPageHit};
use architect_editor::state::{
    DecoratedRange, DecorationKind, EditorState, History, KeySpec, Keymap, TransactionSpec, ext,
};
use architect_editor::ui::palette::{CommandPalette, PaletteState};
use architect_editor::ui::{DecorationSource, Editor, TransactionEvent, WidgetMounts};
use architect_extensions::contrib::Claim;
use architect_extensions::editor::{DecorationSpec, EditCommand, EditorError, EditorView, Editors};
use architect_extensions::embeds::{EmbedView, WidgetView};
use architect_extensions::interface::ExtensionUi;
use dioxus::prelude::*;
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
        let flat = Self::flat(&i);
        drop(i);
        (pending, flat)
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
        let range = architect_editor::state::Range::new(anchor.min(len), head.min(len));
        let spec =
            TransactionSpec::new().selection(architect_editor::state::Selection::single(range));
        let out = Self::push(&mut i, spec);
        drop(i);
        out
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
        let view = LiveEditor::view(&i);
        drop(i);
        view.ok_or(EditorError::NoEditor)
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
        drop(i);
        Ok(())
    }

    fn withdraw(&self, extension: &str) {
        let mut i = lock(&LIVE.inner);
        if i.decorations.remove(extension).is_some() {
            LiveEditor::wake(&i);
        }
    }
}

/// The person's undo and redo, through the editor's history (a keymap's
/// command: `architect_editor::state::Command` returns an `Option`).
#[expect(
    clippy::unnecessary_wraps,
    reason = "an architect_editor::state::Command"
)]
fn undo(_: &EditorState) -> Option<TransactionSpec> {
    Some(TransactionSpec::new().user_event("undo"))
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "an architect_editor::state::Command"
)]
fn redo(_: &EditorState) -> Option<TransactionSpec> {
    Some(TransactionSpec::new().user_event("redo"))
}

/// The editor's sheet and the lab's look for it, at app level (a `<style>`
/// inside a pane does not apply in Blitz). A mark's background goes on its
/// inner span too: Blitz paints an inline background only on the span that
/// owns the glyphs, and a mark's text is in a tile span inside it.
///
/// The editor sits flush on the page (the editor's own `editor-frame
/// editor-frame--flush`: no card, no line numbers) and is as tall as the
/// note: `--ed-scroll-height: auto`, so the pane scrolls it, not a box of
/// its own (every line is built then; a note is not a 100k-line file).
const LAYOUT: &str = "
.lab-note-editor { box-sizing: border-box; color: var(--foreground); position: relative;
  max-width: 760px; width: 100%; padding: 0; min-height: 0; }
.lab-note-editor .editor-root { --ed-scroll-height: auto; padding-left: 0; min-height: 120px;
  font-size: 15px; line-height: 1.65; }
.lab-note-editor .editor-root.ed-render-native { height: auto; overflow: visible; }
.lab-note-editor .md-embed-image { max-width: 100%; height: auto; border-radius: 6px; }
.lab-note-editor .ext-mark.ext-todo,
.lab-note-editor .ext-mark.ext-todo span { background: var(--accent); color: var(--accent-foreground); }
.lab-note-editor .editor-gutter { color: var(--muted-foreground); }
.lab-note-editor .ed-menu { position: absolute; z-index: 5; background: var(--popover);
  border: 1px solid var(--border); border-radius: 8px; padding: 4px; min-width: 220px; }
";

/// The edit that makes `old` read as `new`: the bytes between their common
/// start and end replaced, so the caret and the selection around it map
/// through (an outside change reloads without moving them). `None` when
/// they are the same.
#[must_use]
pub fn replace_spec(old: &str, new: &str) -> Option<TransactionSpec> {
    if old == new {
        return None;
    }
    // Compared a character at a time, so neither end is inside one.
    let same = |(a, b): &(char, char)| a == b;
    let start: usize = old
        .chars()
        .zip(new.chars())
        .take_while(same)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let (old_rest, new_rest) = (old.get(start..)?, new.get(start..)?);
    let tail: usize = old_rest
        .chars()
        .rev()
        .zip(new_rest.chars().rev())
        .take_while(same)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let old_end = old.len().saturating_sub(tail).max(start);
    let new_end = new.len().saturating_sub(tail).max(start);
    let inserted = new.get(start..new_end)?;
    Some(
        TransactionSpec::new()
            .changes(architect_editor::state::Changes::replace(
                start..old_end,
                inserted,
            ))
            .user_event("reload"),
    )
}

/// The vault a note's editor reads: its `[[links]]` resolved, its images
/// loaded from it.
#[derive(Clone)]
pub struct NoteVault(pub architect_navigator::VaultSource);

impl PartialEq for NoteVault {
    /// One vault for the run: a new handle to it is no new vault.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl NoteVault {
    /// The vault's file `name` names (`Page`, `Folder/Page`, `Page.md`, a
    /// `.base`, an attachment): its path.
    #[must_use]
    pub fn file_path(&self, name: &str) -> Option<String> {
        self.0.vault().with(|v| file_of(v, name))
    }

    /// The live preview's decorations for `s`, the vault's.
    fn preview(&self, s: &EditorState) -> Vec<DecoratedRange> {
        use architect_navigator::NavigatorSource as _;
        let mut out = self
            .0
            .vault()
            .with(|v| architect_editor::state::markdown::live_preview_with(s, Some(&Lookup(v))));
        for d in &mut out {
            if let DecorationKind::Widget { html } = &mut d.kind
                && html.contains("md-embed-image")
                && let Some(next) = with_src(html, |src| self.0.resource_url(src))
            {
                *html = next;
            }
        }
        out
    }
}

/// `html`'s first `src="…"` resolved by `resolve`, unless it is a URL
/// already; `None` when it has none or it is not the vault's.
fn with_src(html: &str, resolve: impl Fn(&str) -> Option<String>) -> Option<String> {
    let (head, rest) = html.split_once("src=\"")?;
    let (src, tail) = rest.split_once('"')?;
    let raw = src
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    if raw.contains("://") || raw.starts_with("data:") {
        return None;
    }
    let url = resolve(&raw)?.replace('"', "%22");
    Some(format!("{head}src=\"{url}\"{tail}"))
}

/// The vault's note `name` names: by its path, else by its file's name.
fn page_of<'v>(
    v: &'v vault::obsidian::Vault,
    name: &str,
) -> Option<&'v vault::obsidian::VaultPage> {
    let name = name.trim();
    let name = name.strip_suffix(".md").unwrap_or(name);
    let by_path = v.pages.iter().find(|p| {
        p.rel_path
            .strip_suffix(".md")
            .is_some_and(|r| r.eq_ignore_ascii_case(name))
    });
    by_path.or_else(|| {
        let base = name.rsplit('/').next().unwrap_or(name);
        v.pages
            .iter()
            .find(|p| p.basename.eq_ignore_ascii_case(base))
    })
}

/// The vault's file `name` names: a note, else a `.base` or an attachment
/// by its path or its file's name (Obsidian links any file).
fn file_of(v: &vault::obsidian::Vault, name: &str) -> Option<String> {
    if let Some(p) = page_of(v, name) {
        return Some(p.rel_path.clone());
    }
    let name = name.trim();
    let file = name.rsplit('/').next().unwrap_or(name);
    let by = |rel: &str| {
        rel.eq_ignore_ascii_case(name)
            || rel
                .rsplit('/')
                .next()
                .is_some_and(|f| f.eq_ignore_ascii_case(file))
    };
    v.bases
        .iter()
        .map(|b| b.rel_path.as_str())
        .chain(v.attachments.iter().map(|a| a.rel_path.as_str()))
        .find(|rel| by(rel))
        .map(str::to_owned)
}

/// The vault, as the editor's live preview asks it about links.
struct Lookup<'v>(&'v vault::obsidian::Vault);

impl VaultLookup for Lookup<'_> {
    fn lookup_block(&self, _: &str) -> Option<VaultBlockHit> {
        None
    }

    fn lookup_page(&self, name: &str) -> Option<VaultPageHit> {
        let Some(p) = page_of(self.0, name) else {
            // A `.base` or an attachment: resolved, nothing to preview.
            return file_of(self.0, name).map(|_| VaultPageHit {
                preview: String::new(),
            });
        };
        let body = crate::properties::split(&p.raw).1;
        Some(VaultPageHit {
            preview: body.chars().take(200).collect(),
        })
    }

    fn lookup_section(&self, page: &str, heading: &str) -> Option<String> {
        let p = page_of(self.0, page)?;
        section(&p.raw, heading)
    }

    fn lookup_block_short(&self, _: &str, _: &str) -> Option<String> {
        None
    }
}

/// The section of `raw` under heading `name`: its heading line and what
/// follows, to the next heading as high.
fn section(raw: &str, name: &str) -> Option<String> {
    let level = |l: &str| {
        let n = l.chars().take_while(|c| *c == '#').count();
        (n > 0 && l.chars().nth(n) == Some(' ')).then_some(n)
    };
    let mut out = String::new();
    let mut at: Option<usize> = None;
    for line in raw.lines() {
        match (at, level(line)) {
            (None, Some(n))
                if line
                    .trim_start_matches('#')
                    .trim()
                    .eq_ignore_ascii_case(name.trim()) =>
            {
                at = Some(n);
            }
            (Some(n), Some(m)) if m <= n => break,
            _ => {}
        }
        if at.is_some() {
            out.push_str(line);
            out.push('\n');
        }
    }
    at.map(|_| out)
}

/// A Markdown note's editor: `text` to start with; each change of its text
/// given to `on_change`. `incoming` is the text to show instead, from
/// outside the editor (the file changed on disk, merged with the typing),
/// counted so the same text twice is two reloads: applied as one undo
/// step, the caret kept where it was.
#[component]
pub fn NoteEditor(
    id: String,
    text: String,
    on_change: Callback<String>,
    #[props(default)] incoming: Option<(u64, String)>,
    #[props(default)] vault: Option<NoteVault>,
    #[props(default)] on_link: Option<Callback<String>>,
) -> Element {
    let state = use_signal(|| EditorState::new(text.clone()));
    let history = use_signal(History::new);
    let applied = use_hook(|| CopyValue::new(0_u64));
    use_effect(use_reactive!(|incoming| {
        let mut applied = applied;
        if let Some((n, new)) = incoming
            && n != *applied.peek()
        {
            applied.set(n);
            let now = state.peek().doc.to_string();
            if let Some(spec) = replace_spec(&now, &new) {
                architect_editor::ui::dispatch_with_history(state, history, spec, None);
            }
        }
    }));
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
                architect_editor::ui::dispatch_with_history(state, history, spec, None);
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
        architect_editor::ui::palette::register_actions(&slash);
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
    // The fence languages extensions claim now: the editor's fence
    // registry, as they load and unload.
    let claimed = move || ui.map(|ui| ui.host().embeds().fences()).unwrap_or_default();
    // The editor's languages: read below, so a fence in a language that
    // came or went (an extension's `[[languages]]`) is drawn again.
    let languages = architect_editor::ui::use_languages_revision();
    let decorations = use_hook(|| {
        DecorationSource::new(move |s: &EditorState| {
            let _ = *languages.read();
            let specs: Vec<DecorationSpec> = decos.read().iter().map(|(_, d)| d.clone()).collect();
            let text = s.doc.to_string();
            let mut out = vault.as_ref().map_or_else(
                || architect_editor::state::markdown::live_preview(s),
                |v| v.preview(s),
            );
            out.extend(ext::decorations(&specs, &text));
            out.extend(
                claimed_fences(&text, &claimed())
                    .iter()
                    .enumerate()
                    .map(|(i, f)| DecoratedRange::mount(f.at, &fence_mount(i), true)),
            );
            out.sort_by_key(|d| d.from);
            out
        })
    });
    let mounts = use_hook(|| {
        WidgetMounts::new(move |mount: &str| {
            if let Some(i) = fence_index(mount) {
                let text = state.peek().doc.to_string();
                return match claimed_fences(&text, &claimed()).into_iter().nth(i) {
                    Some(f) => rsx! {
                        div { class: "lab-note-fence-embed", "data-fence": "{f.info}",
                            EmbedView {
                                claim: f.claim,
                                source: f.source,
                                props: serde_json::json!({ "info": f.info }),
                            }
                        }
                    },
                    None => rsx! {},
                };
            }
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
    // The editor's sheet, then its token classes pointed at the theme's
    // `syntax.*` tokens, so a fence's highlighting follows the theme.
    let css = use_hook(|| {
        format!(
            "{}\n{}\n{LAYOUT}",
            architect_editor::ui::EDITOR_CSS,
            architect_editor::theme::css()
        )
    });
    // Gone (Done, another note): typing is what the host says again.
    use_drop(|| architect_shell::set_typing(dioxus_native::text_input_focused()));
    rsx! {
        document::Style { {css} }
        div {
            id: "note-source",
            class: "lab-note-editor editor-frame editor-frame--flush",
            "data-note-editor": "{id}",
            // The editor is a text field the host does not count as one
            // (Blitz reports an `<input>` or a `<textarea>` only, and moves
            // the focus on a click only into those, so a click here fires
            // no `focusin`): it says so itself, on a click (its text's
            // spans keep their `mousedown` to themselves). A phone's bar
            // becomes the Typing Toolbar, its `editor:note` items with it,
            // and global keys stand aside while it has the keys.
            onclick: move |_| {
                editing.focus();
                architect_shell::set_typing(true);
            },
            onfocusin: move |_| {
                editing.focus();
                architect_shell::set_typing(true);
            },
            onfocusout: move |_| {
                editing.blur();
                architect_shell::set_typing(dioxus_native::text_input_focused());
            },
            Editor {
                state,
                keymap,
                decorations,
                history,
                on_key,
                mounts,
                palette,
                on_transaction,
                on_link_click: move |href: String| {
                    if let Some(go) = on_link {
                        go.call(href);
                    }
                },
                on_action: move |a: String| run(a),
            }
            CommandPalette { state, palette, on_action: move |a: String| run(a) }
        }
    }
}

/// A fenced block an extension claims, in a note's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedFence {
    /// Where its embed is mounted: the end of its closing fence line (of
    /// the text, for a fence left open).
    pub at: usize,
    /// Its info string (```` ```kf chorus ```` → `kf chorus`).
    pub info: String,
    /// What it claims (`fence:kf`).
    pub claim: Claim,
    /// Its body.
    pub source: String,
}

/// Each fenced block of `text` whose language is one of `claimed` (lower
/// case, as `Embeds::fences` lists them), in order.
#[must_use]
pub fn claimed_fences(text: &str, claimed: &[String]) -> Vec<ClaimedFence> {
    let mut out = Vec::new();
    // The fence open: its info, where its body starts.
    let mut open: Option<(&str, usize)> = None;
    let mut line_start = 0_usize;
    for line in text.split_inclusive('\n') {
        let end = line_start.saturating_add(line.trim_end_matches(['\n', '\r']).len());
        let trimmed = line.trim();
        match open {
            Some((info, body)) if trimmed.starts_with("```") => {
                out.extend(claimed_fence(text, claimed, info, body..line_start, end));
                open = None;
            }
            Some(_) => {}
            None => {
                if let Some(info) = trimmed.strip_prefix("```") {
                    open = Some((info.trim(), line_start.saturating_add(line.len())));
                }
            }
        }
        line_start = line_start.saturating_add(line.len());
    }
    // An open fence runs to the end, as the editor shows it.
    if let Some((info, body)) = open {
        out.extend(claimed_fence(
            text,
            claimed,
            info,
            body..text.len(),
            text.len(),
        ));
    }
    out
}

fn claimed_fence(
    text: &str,
    claimed: &[String],
    info: &str,
    body: std::ops::Range<usize>,
    at: usize,
) -> Option<ClaimedFence> {
    let claim = Claim::fence(info)?;
    let Claim::Fence(lang) = &claim else {
        return None;
    };
    if !claimed.contains(&lang.to_ascii_lowercase()) {
        return None;
    }
    let source = text.get(body.start.min(text.len())..body.end.max(body.start).min(text.len()))?;
    Some(ClaimedFence {
        at,
        info: info.to_owned(),
        claim,
        source: source.trim_end_matches(['\n', '\r']).to_owned(),
    })
}

/// The mount id of the `i`th claimed fence's embed.
fn fence_mount(i: usize) -> String {
    format!("fence-{i}")
}

/// Which claimed fence a mount id names.
fn fence_index(mount: &str) -> Option<usize> {
    mount.strip_prefix("fence-")?.parse().ok()
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
            drop(i);
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

#[cfg(test)]
mod tests {
    use super::{LAYOUT, claimed_fences, fence_index, fence_mount};

    /// The editor's fences are the extensions': a fence in a claimed
    /// language is found with its body and where its embed goes; one in
    /// a language nobody claims, or once its claim is gone, is left as
    /// source.
    #[test]
    fn a_claimed_fence_is_found_with_its_body_and_where_its_embed_goes() {
        let text = "# Song\n\n```kf chorus\n[Verse]\n| C | G |\n```\n\n```rust\nfn main() {}\n```\n```KF\n| F |";
        let claimed = vec!["kf".to_owned()];
        let found = claimed_fences(text, &claimed);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].info, "kf chorus");
        assert_eq!(found[0].claim.to_string(), "fence:kf");
        assert_eq!(found[0].source, "[Verse]\n| C | G |");
        let closing = text.find("| C | G |\n```").unwrap() + "| C | G |\n```".len();
        assert_eq!(found[0].at, closing, "the end of its closing line");
        // Left open: to the end of the note.
        assert_eq!(found[1].source, "| F |");
        assert_eq!(found[1].at, text.len());
        // Nobody claims it: none.
        assert!(claimed_fences(text, &[]).is_empty());
        assert_eq!(claimed_fences(text, &["rust".to_owned()]).len(), 1);
        assert_eq!(fence_index(&fence_mount(3)), Some(3));
        assert_eq!(fence_index("ext-3"), None);
    }

    /// Blitz paints an inline background only on the span that owns the
    /// glyphs: a TODO mark's text is in a tile span inside the mark, so a
    /// background on the mark alone showed nothing on the desktop.
    #[test]
    fn a_todo_mark_s_background_is_on_the_span_that_holds_its_text() {
        assert!(
            LAYOUT
                .contains(".lab-note-editor .ext-mark.ext-todo span { background: var(--accent);"),
            "{LAYOUT}"
        );
    }
}

#[cfg(test)]
mod reload_tests {
    use super::replace_spec;

    #[test]
    fn a_reload_replaces_only_what_differs() {
        let spec = replace_spec("One two three", "One 2 three").unwrap();
        let c = &spec.changes;
        assert!(format!("{c:?}").contains("from: 4"), "{c:?}");
        // A run of the same character: the edit is one character, not two.
        let spec = replace_spec("aaa", "aa").unwrap();
        assert!(format!("{:?}", spec.changes).contains("inserted: \"\""));
        assert!(replace_spec("same", "same").is_none());
        // Never inside a character.
        assert!(replace_spec("café", "cafè").is_some());
        assert!(replace_spec("aé", "aéé").is_some());
    }
}
