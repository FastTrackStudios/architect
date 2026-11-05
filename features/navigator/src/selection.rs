//! What is selected: a folder in the navigation pane, notes in the list,
//! and the history of folders selected, to go back and forward through.
//!
//! Notebook Navigator's `SelectionState` (`src/context/selection/`):
//! `selectedFolder` (or tag, or property), `selectedFiles` and
//! `selectedFile`, the cursor. A plain click selects one note; Cmd/Ctrl
//! toggles one (the last can't be let go, and letting the cursor's go moves
//! the cursor to the first still selected); Shift selects the range from
//! the cursor; select all takes every note. History keeps the last 100
//! navigation selections (`MAX_NAVIGATION_HISTORY_ENTRIES`), not notes.

use std::collections::BTreeSet;

/// What the navigation pane selects: a folder, or a tag (a tag on a note's
/// row goes to it). Properties join it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum NavTarget {
    Folder(String),
    /// A tag without `#`; its list holds the tags below it too.
    Tag(String),
}

impl NavTarget {
    #[must_use]
    pub fn folder(&self) -> Option<&str> {
        match self {
            Self::Folder(p) => Some(p),
            Self::Tag(_) => None,
        }
    }

    #[must_use]
    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Tag(t) => Some(t),
            Self::Folder(_) => None,
        }
    }
}

/// Notebook Navigator's `MAX_NAVIGATION_HISTORY_ENTRIES`.
pub const MAX_HISTORY: usize = 100;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    target: Option<NavTarget>,
    files: BTreeSet<String>,
    cursor: Option<String>,
    history: Vec<NavTarget>,
    /// Where in `history` the selection is.
    at: usize,
}

impl Selection {
    #[must_use]
    pub const fn target(&self) -> Option<&NavTarget> {
        self.target.as_ref()
    }

    /// The selected folder, if a folder is what is selected.
    #[must_use]
    pub fn folder(&self) -> Option<&str> {
        self.target.as_ref().and_then(NavTarget::folder)
    }

    /// The selected notes.
    #[must_use]
    pub const fn files(&self) -> &BTreeSet<String> {
        &self.files
    }

    /// The note the keyboard moves from: the last one chosen.
    #[must_use]
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    #[must_use]
    pub fn is_selected(&self, path: &str) -> bool {
        self.files.contains(path)
    }

    /// Selects `target` in the navigation pane, with `file` the note
    /// selected in it (or none), and records it in the history.
    pub fn select_target(&mut self, target: NavTarget, file: Option<String>) {
        self.record(&target);
        self.set_target(target, file);
    }

    fn set_target(&mut self, target: NavTarget, file: Option<String>) {
        self.target = Some(target);
        self.files = file.iter().cloned().collect();
        self.cursor = file;
    }

    fn record(&mut self, target: &NavTarget) {
        if self.history.get(self.at) == Some(target) {
            return;
        }
        if !self.history.is_empty() {
            self.history.truncate(self.at.saturating_add(1));
        }
        self.history.push(target.clone());
        let over = self.history.len().saturating_sub(MAX_HISTORY);
        self.history.drain(..over);
        self.at = self.history.len().saturating_sub(1);
    }

    /// Whether there is a selection to go back to.
    #[must_use]
    pub const fn can_go_back(&self) -> bool {
        self.at > 0
    }

    #[must_use]
    pub const fn can_go_forward(&self) -> bool {
        self.at.saturating_add(1) < self.history.len()
    }

    /// The selection before, if any (the notes cleared).
    pub fn back(&mut self) -> Option<&NavTarget> {
        if !self.can_go_back() {
            return None;
        }
        self.at = self.at.saturating_sub(1);
        self.go()
    }

    /// The selection after, if any.
    pub fn forward(&mut self) -> Option<&NavTarget> {
        if !self.can_go_forward() {
            return None;
        }
        self.at = self.at.saturating_add(1);
        self.go()
    }

    fn go(&mut self) -> Option<&NavTarget> {
        let target = self.history.get(self.at)?.clone();
        self.set_target(target, None);
        self.target.as_ref()
    }

    /// The history, oldest first, and where in it the selection is.
    #[must_use]
    pub fn history(&self) -> (&[NavTarget], usize) {
        (&self.history, self.at)
    }

    /// A plain click: `path` alone. Returns the note to open.
    pub fn click(&mut self, path: &str) -> String {
        self.files = std::iter::once(path.to_owned()).collect();
        self.cursor = Some(path.to_owned());
        path.to_owned()
    }

    /// Cmd/Ctrl-click: `path` in or out. Returns the note to open, if the
    /// cursor moved. `order` is the list's notes as shown.
    pub fn toggle(&mut self, path: &str, order: &[&str]) -> Option<String> {
        if !self.files.contains(path) {
            self.files.insert(path.to_owned());
            self.cursor = Some(path.to_owned());
            return Some(path.to_owned());
        }
        if self.files.len() == 1 {
            return None;
        }
        self.files.remove(path);
        if self.cursor.as_deref() == Some(path) {
            self.cursor = order
                .iter()
                .find(|p| self.files.contains(**p))
                .map(|p| (*p).to_owned())
                .or_else(|| self.files.iter().next().cloned());
            return self.cursor.clone();
        }
        None
    }

    /// Shift-click: every note from the cursor to `path` added. Returns
    /// the note to open.
    pub fn range(&mut self, path: &str, order: &[&str]) -> String {
        let to = order.iter().position(|p| *p == path);
        let from = self
            .cursor
            .as_deref()
            .and_then(|c| order.iter().position(|p| *p == c))
            .or(to);
        if let (Some(a), Some(b)) = (from, to) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            for p in order
                .iter()
                .skip(lo)
                .take(hi.saturating_sub(lo).saturating_add(1))
            {
                self.files.insert((*p).to_owned());
            }
        } else {
            self.files.insert(path.to_owned());
        }
        self.cursor = Some(path.to_owned());
        path.to_owned()
    }

    /// Every note of the list; the cursor stays (or is the first).
    pub fn select_all(&mut self, order: &[&str]) {
        self.files = order.iter().map(|p| (*p).to_owned()).collect();
        if self
            .cursor
            .as_deref()
            .is_none_or(|c| !self.files.contains(c))
        {
            self.cursor = order.first().map(|p| (*p).to_owned());
        }
    }

    /// Shift+↑/↓: grows the selection by the note `step` away from the
    /// cursor, or shrinks it when that note is selected already. Returns
    /// the new cursor.
    pub fn extend(&mut self, step: isize, order: &[&str]) -> Option<String> {
        let at = self
            .cursor
            .as_deref()
            .and_then(|c| order.iter().position(|p| *p == c))?;
        let to = at.checked_add_signed(step).filter(|t| *t < order.len())?;
        let next = (*order.get(to)?).to_owned();
        if self.files.contains(&next) {
            if let Some(c) = &self.cursor {
                self.files.remove(c);
            }
        } else {
            self.files.insert(next.clone());
        }
        self.cursor = Some(next.clone());
        Some(next)
    }

    /// Shift+Home/End: every note from the cursor to the first or last.
    pub fn extend_to_edge(&mut self, last: bool, order: &[&str]) -> Option<String> {
        let edge = if last { order.last() } else { order.first() };
        let edge = (*edge?).to_owned();
        Some(self.range(&edge, order))
    }

    /// Clears the notes, keeping the folder.
    pub fn clear_files(&mut self) {
        self.files.clear();
        self.cursor = None;
    }

    /// Selects `file` alone without touching the folder (a reveal).
    pub fn set_file(&mut self, file: Option<String>) {
        self.files = file.iter().cloned().collect();
        self.cursor = file;
    }

    /// A note or folder moved or went: the selection follows a move, and
    /// lets go of what is gone (`exists`).
    pub fn follow(
        &mut self,
        moved: &[(String, String)],
        exists: impl Fn(&str) -> bool,
        folder_exists: impl Fn(&str) -> bool,
    ) {
        let rename = |p: &str| {
            moved
                .iter()
                .find(|(from, _)| from == p)
                .map_or_else(|| p.to_owned(), |(_, to)| to.clone())
        };
        self.files = self
            .files
            .iter()
            .map(|p| rename(p))
            .filter(|p| exists(p))
            .collect();
        self.cursor = self.cursor.as_deref().map(rename).filter(|p| exists(p));
        if let Some(NavTarget::Folder(f)) = &self.target {
            let f = rename(f);
            self.target = Some(NavTarget::Folder(if folder_exists(&f) {
                f
            } else {
                // Up to the nearest folder still there.
                crate::source::ancestors(&f)
                    .find(|a| folder_exists(a))
                    .unwrap_or("")
                    .to_owned()
            }));
        }
        let gone = |t: &NavTarget| t.folder().is_some_and(|f| !folder_exists(f));
        let before = self.history.len();
        self.history.retain(|t| !gone(t));
        if self.history.len() != before {
            self.at = self.at.min(self.history.len().saturating_sub(1));
        }
    }
}
