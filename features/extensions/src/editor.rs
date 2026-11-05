//! The editor being typed in, for extensions (#312, gap `G-editor`).
//!
//! The editor's extension surface is data (`editor:crates/editor-ext`,
//! re-exported here): a [`Snapshot`] of the live editor, an
//! [`EditCommand`] applied as one transaction, [`KeymapEntry`]s scoped to
//! the editor or a node kind, [`DecorationSpec`]s and [`SlashEntry`]s. So
//! every runtime reaches it the same way, through the host service
//! (`crate::hostcall`):
//!
//! | call | does | needs |
//! |---|---|---|
//! | `editor_state` | the focused editor ([`EditorView`]) | `reads` or `writes = ["editor"]` |
//! | `edit_editor {changes, selection?, name?, base?}` | one transaction, one undo step, made as the extension | `writes = ["editor"]` |
//! | `decorate_editor {decorations}` | its decorations, in place of the last | `writes = ["editor"]` |
//!
//! and from its manifest (or contributed at run time), keys and slash rows
//! that name its actions:
//!
//! ```toml
//! [capabilities]
//! writes = ["editor"]
//!
//! [[keymaps]]
//! key = "Tab"
//! when = "node:list-item"     # or "editor", "node:table", …
//! action = "ACME_INDENT"
//!
//! [[slash]]
//! label = "Table"
//! action = "ACME_TABLE"
//! ```
//!
//! The app gives the host its editors by implementing [`Editors`]
//! ([`crate::host::Host::with_editors`]): which editor has the focus, and
//! applying a command to it as the extension — through its own history, so
//! the edit undoes in one step beside the person's typing, and, for a
//! document backed by `crdt`, as a transaction that merges with it. It runs
//! the keymaps ([`key_action`]) where it handles a key, draws the
//! decorations (`editor_state::ext::decorations`; a widget mounts the
//! extension's embed, `crate::embeds::WidgetView`), and puts the slash
//! rows in its menu. [`MemoryEditor`] is one over a string, for a headless
//! host and for tests.

#[cfg(feature = "host")]
use std::collections::BTreeMap;
#[cfg(feature = "host")]
use std::sync::{Mutex, PoisonError};

#[cfg(feature = "host")]
use architect::action::Principal;
use serde::{Deserialize, Serialize};

pub use editor_ext::{
    Cell, DecorationSpec, EditCommand, ExtError, KeyChord, KeymapEntry, NodeAt, NodeKind, Scope,
    SelRange, SlashEntry, Snapshot, TextChange, node_at,
};

use crate::contrib::{By, Contributions};
use crate::hostcall::HostError;

/// The focused editor, as an extension reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct EditorView {
    /// The app's id for it (`lab.note/Inbox.md`).
    pub editor: String,
    /// What it edits (`note`, `markdown`): the context a toolbar's
    /// `when = "editor:<context>"` names.
    pub context: String,
    /// Its text, selection, cursor, the block at the cursor, its version.
    #[serde(flatten)]
    pub snapshot: Snapshot,
}

/// Why an editor call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorError {
    /// No editor has the focus.
    NoEditor,
    /// The command or a decoration does not fit the text.
    Refused(ExtError),
    /// The app could not do it.
    Failed(String),
}

impl std::fmt::Display for EditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoEditor => f.write_str("no editor has the focus"),
            Self::Refused(e) => write!(f, "{e}"),
            Self::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for EditorError {}

impl From<ExtError> for EditorError {
    fn from(e: ExtError) -> Self {
        Self::Refused(e)
    }
}

impl From<EditorError> for HostError {
    fn from(e: EditorError) -> Self {
        Self::Failed {
            message: e.to_string(),
        }
    }
}

#[cfg(feature = "host")]
/// The app's editors, as extensions reach them.
pub trait Editors: Send + Sync {
    /// The editor being typed in, if one is.
    fn focused(&self) -> Option<EditorView>;

    /// `edit` on the focused editor as one transaction — one undo step
    /// named `step` in its history, beside the person's own — made as
    /// `by`. The editor after.
    ///
    /// # Errors
    ///
    /// No editor has the focus, or `edit` does not fit its text (stale,
    /// out of bounds, overlapping).
    fn apply(
        &self,
        edit: &EditCommand,
        step: &str,
        by: &Principal,
    ) -> Result<EditorView, EditorError>;

    /// `decorations` of extension `extension` in the focused editor, in
    /// place of the ones it set before; moved through later edits.
    ///
    /// # Errors
    ///
    /// No editor has the focus, or one does not fit its text.
    fn decorate(
        &self,
        extension: &str,
        decorations: Vec<DecorationSpec>,
    ) -> Result<(), EditorError>;

    /// Takes away everything `extension` added: it unloaded.
    fn withdraw(&self, extension: &str) {
        let _ = self.decorate(extension, Vec::new());
    }
}

impl AsRef<KeymapEntry> for By<KeymapEntry> {
    fn as_ref(&self) -> &KeymapEntry {
        &self.item
    }
}

/// The keymap entry `press` runs in an editor whose cursor is in `node`,
/// of every extension's (`[[keymaps]]`): its owner and action. A node's
/// entry before the editor's, then the first loaded.
#[must_use]
pub fn key_action(
    contributions: &Contributions,
    press: &KeyChord,
    node: &NodeAt,
) -> Option<By<KeymapEntry>> {
    let all = contributions.keymaps();
    editor_ext::resolve(&all, press, node).cloned()
}

/// The widgets of `decorations` that name neither one of `embeds` nor of
/// `views` (the extension's own): the first, refused.
///
/// # Errors
///
/// A widget naming something the extension does not have.
pub fn check_widgets(
    decorations: &[DecorationSpec],
    embeds: &[String],
    views: &[String],
) -> Result<(), String> {
    for d in decorations {
        if let DecorationSpec::Widget { view, .. } = d
            && !embeds.iter().chain(views).any(|v| v == view)
        {
            return Err(format!(
                "widget {view:?}: not one of its own [[embeds]] or [[views]]"
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "host")]
/// One step of a [`MemoryEditor`]'s undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Its name.
    pub name: String,
    /// Who made it ([`Principal`]'s words).
    pub by: String,
    text: String,
    selection: Vec<SelRange>,
}

#[cfg(feature = "host")]
#[derive(Debug, Default)]
struct Memory {
    view: EditorView,
    undo: Vec<Step>,
    decorations: BTreeMap<String, Vec<DecorationSpec>>,
    focused: bool,
}

#[cfg(feature = "host")]
/// An editor over a string, with its own undo and decorations: for a host
/// with no editor on screen, and for tests. The person's typing is
/// [`Self::type_text`]; an extension's command one undo step.
#[derive(Debug, Default)]
pub struct MemoryEditor {
    inner: Mutex<Memory>,
}

#[cfg(feature = "host")]
impl MemoryEditor {
    /// Editor `editor` of context `context` holding `text`, the caret at
    /// its end, focused.
    #[must_use]
    pub fn new(editor: &str, context: &str, text: &str) -> Self {
        let caret = [SelRange::caret(text.len())];
        Self {
            inner: Mutex::new(Memory {
                view: EditorView {
                    editor: editor.to_owned(),
                    context: context.to_owned(),
                    snapshot: Snapshot::of(text, &caret, 0, 0),
                },
                focused: true,
                ..Memory::default()
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Its text.
    #[must_use]
    pub fn text(&self) -> String {
        self.lock().view.snapshot.text.clone()
    }

    /// Its view.
    #[must_use]
    pub fn view(&self) -> EditorView {
        self.lock().view.clone()
    }

    /// Whether it has the focus (an unfocused one is not
    /// [`Editors::focused`]).
    pub fn set_focused(&self, on: bool) {
        self.lock().focused = on;
    }

    /// The caret at `pos` (clamped to the text).
    pub fn put_caret(&self, pos: usize) {
        let mut m = self.lock();
        let s = &m.view.snapshot;
        let pos = pos.min(s.text.len());
        m.view.snapshot = Snapshot::of(&s.text, &[SelRange::caret(pos)], 0, s.version);
    }

    fn set(m: &mut Memory, text: &str, selection: &[SelRange], changes: &[TextChange]) {
        let version = m.view.snapshot.version.saturating_add(1);
        m.view.snapshot = Snapshot::of(text, selection, 0, version);
        for specs in m.decorations.values_mut() {
            *specs = specs.iter().filter_map(|d| d.mapped(changes)).collect();
        }
    }

    /// The person types `text` at the caret (no undo step of its own:
    /// typing is not what this tests).
    pub fn type_text(&self, text: &str) {
        let mut m = self.lock();
        let s = &m.view.snapshot;
        let change = [TextChange::insert(s.cursor, text)];
        let cmd = EditCommand::changes(change.to_vec());
        if let Ok((after, sel)) = cmd.apply_to(&s.text, &s.selection, s.version) {
            Self::set(&mut m, &after, &sel, &change);
        }
        drop(m);
    }

    /// Its undo's steps, oldest first.
    #[must_use]
    pub fn steps(&self) -> Vec<Step> {
        self.lock().undo.clone()
    }

    /// Takes back the last step: its name.
    pub fn undo(&self) -> Option<String> {
        let mut m = self.lock();
        let step = m.undo.pop()?;
        let version = m.view.snapshot.version.saturating_add(1);
        m.view.snapshot = Snapshot::of(&step.text, &step.selection, 0, version);
        drop(m);
        Some(step.name)
    }

    /// Every extension's decorations, by extension.
    #[must_use]
    pub fn decorations(&self) -> BTreeMap<String, Vec<DecorationSpec>> {
        self.lock().decorations.clone()
    }
}

#[cfg(feature = "host")]
impl Editors for MemoryEditor {
    fn focused(&self) -> Option<EditorView> {
        let m = self.lock();
        m.focused.then(|| m.view.clone())
    }

    fn apply(
        &self,
        edit: &EditCommand,
        step: &str,
        by: &Principal,
    ) -> Result<EditorView, EditorError> {
        let mut m = self.lock();
        if !m.focused {
            return Err(EditorError::NoEditor);
        }
        let s = m.view.snapshot.clone();
        let changes = edit.checked(&s.text, s.version)?;
        let (after, sel) = edit.apply_to(&s.text, &s.selection, s.version)?;
        m.undo.push(Step {
            name: step.to_owned(),
            by: by.describe(),
            text: s.text,
            selection: s.selection,
        });
        Self::set(&mut m, &after, &sel, &changes);
        Ok(m.view.clone())
    }

    fn decorate(
        &self,
        extension: &str,
        decorations: Vec<DecorationSpec>,
    ) -> Result<(), EditorError> {
        let mut m = self.lock();
        if !m.focused {
            return Err(EditorError::NoEditor);
        }
        for d in &decorations {
            d.checked(&m.view.snapshot.text)?;
        }
        if decorations.is_empty() {
            m.decorations.remove(extension);
        } else {
            m.decorations.insert(extension.to_owned(), decorations);
        }
        drop(m);
        Ok(())
    }
}
