//! What a person sets for one folder or one tag, over the settings.
//!
//! Its sort, its grouping, its rows (title and preview lines, compact),
//! what its rows show, and whether it lists the notes of folders below.
//!
//! Notebook Navigator keeps these per place (`folderAppearances`,
//! `tagAppearances`; sorts in `folderSortOverrides`, `tagSortOverrides`)
//! and resolves each field as the place's, else the setting's
//! (`resolveListPaneAppearance`, `src/settings/listPaneAppearance.ts`).
//! Notes from folders below are a setting of the whole vault there; here a
//! folder may say so for itself too.

use facet::Facet;

use crate::selection::NavTarget;
use crate::settings::{Grouping, ListMode, NavigatorSettings, SortOption, TextCount};

/// One place's own list: each field `None` follows the settings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct Appearance {
    /// The place: `folder:<path>` or `tag:<tag>` ([`target_key`]).
    pub target: String,
    pub sort: Option<SortOption>,
    pub grouping: Option<Grouping>,
    pub mode: Option<ListMode>,
    /// 1 to 3.
    pub title_rows: Option<u32>,
    /// 0 (none) to 5.
    pub preview_rows: Option<u32>,
    pub descendants: Option<bool>,
    pub show_tags: Option<bool>,
    pub show_properties: Option<bool>,
    pub show_task_progress: Option<bool>,
    pub text_count: Option<TextCount>,
}

impl Appearance {
    /// None of its own: nothing over the settings.
    #[must_use]
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            ..Self::default()
        }
    }

    /// `settings` with this place's own over them.
    #[must_use]
    pub fn over(&self, settings: &NavigatorSettings) -> NavigatorSettings {
        let mut s = settings.clone();
        if let Some(v) = self.sort {
            s.sort = v;
        }
        if let Some(v) = self.grouping {
            s.grouping = v;
        }
        if let Some(v) = self.mode {
            s.mode = v;
        }
        if let Some(v) = self.title_rows {
            s.title_rows = v.clamp(1, 3);
        }
        if let Some(v) = self.preview_rows {
            s.preview_rows = v.min(5);
        }
        if let Some(v) = self.descendants {
            s.include_descendants = v;
        }
        // Tags need the setting's switch on too.
        if let Some(v) = self.show_tags {
            s.show_tags = settings.show_tags && v;
        }
        if let Some(v) = self.show_properties {
            s.show_properties = v;
        }
        if let Some(v) = self.show_task_progress {
            s.show_task_progress = v;
        }
        if let Some(v) = self.text_count {
            s.text_count = v;
        }
        s
    }

    /// Its fields equal to the settings' let go (Notebook Navigator removes
    /// an override equal to the default).
    #[must_use]
    pub fn pruned(mut self, settings: &NavigatorSettings) -> Self {
        fn drop_same<T: PartialEq>(v: &mut Option<T>, same: &T) {
            if v.as_ref() == Some(same) {
                *v = None;
            }
        }
        drop_same(&mut self.sort, &settings.sort);
        drop_same(&mut self.grouping, &settings.grouping);
        drop_same(&mut self.mode, &settings.mode);
        drop_same(&mut self.title_rows, &settings.title_rows);
        drop_same(&mut self.preview_rows, &settings.preview_rows);
        drop_same(&mut self.descendants, &settings.include_descendants);
        drop_same(&mut self.show_tags, &settings.show_tags);
        drop_same(&mut self.show_properties, &settings.show_properties);
        drop_same(&mut self.show_task_progress, &settings.show_task_progress);
        drop_same(&mut self.text_count, &settings.text_count);
        self
    }

    /// Whether it has nothing of its own.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::new(self.target.clone())
    }
}

/// A place's key: `folder:<path>`, `tag:<tag>`.
#[must_use]
pub fn target_key(target: &NavTarget) -> String {
    crate::list::scope_of(target)
}
